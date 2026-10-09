//! A small HTTP/1.1 client for map downloads from `sv_downloadurl`
//! (docs/plans/active/custom-maps.md, "Distribution"): plain HTTP only, as
//! the game's own downloader ("Only HTTP is supported"), GET with
//! `Content-Length` or chunked bodies, a few redirects, timeouts, progress
//! and cancelling from another thread.

use std::{
    io::{BufRead, BufReader, Read, Write},
    net::{TcpStream, ToSocketAddrs},
    sync::atomic::{AtomicBool, AtomicU64, Ordering},
    time::Duration,
};

/// How long connecting and each read may take.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const READ_TIMEOUT: Duration = Duration::from_secs(20);
/// Redirects followed, at most.
const MAX_REDIRECTS: usize = 4;

/// Why a download failed, as the game's download errors name them
/// (`GameUI_DownloadFailed*`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HttpError {
    /// Not an `http://host/path` URL.
    BadUrl,
    /// Another scheme (https, ftp).
    BadProtocol,
    CantConnect(String),
    /// No status line or headers came back.
    NoHeaders,
    /// 404 (or another 4xx).
    NotFound,
    /// The connection closed before the body was complete.
    Closed,
    ZeroLength,
    /// Bigger than the caller allows.
    TooBig(u64),
    Cancelled,
    /// Another status, or a read error.
    Other(String),
}

impl std::fmt::Display for HttpError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            HttpError::BadUrl => write!(f, "Invalid URL"),
            HttpError::BadProtocol => write!(f, "Only HTTP is supported"),
            HttpError::CantConnect(e) => write!(f, "Cannot connect to server ({e})"),
            HttpError::NoHeaders => write!(f, "Cannot get file info from server"),
            HttpError::NotFound => write!(f, "File does not exist"),
            HttpError::Closed => write!(f, "Connection closed by remote host"),
            HttpError::ZeroLength => write!(f, "File has no data"),
            HttpError::TooBig(n) => write!(f, "File is too big ({n} bytes)"),
            HttpError::Cancelled => write!(f, "Cancelled"),
            HttpError::Other(e) => write!(f, "{e}"),
        }
    }
}

/// A download's progress, shared with the thread doing it.
#[derive(Debug, Default)]
pub struct Progress {
    pub done: AtomicU64,
    /// 0 while unknown.
    pub total: AtomicU64,
    pub cancel: AtomicBool,
}

/// `http://host[:port]/path` split up.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Url {
    pub host: String,
    pub port: u16,
    pub path: String,
}

pub fn parse_url(url: &str) -> Result<Url, HttpError> {
    let url = url.trim();
    let rest = match url.split_once("://") {
        Some((scheme, rest)) if scheme.eq_ignore_ascii_case("http") => rest,
        Some(_) => return Err(HttpError::BadProtocol),
        // Source's sv_downloadurl is always written with http://; a bare
        // host is taken as one.
        None => url,
    };
    let (authority, path) = match rest.find('/') {
        Some(i) => (&rest[..i], &rest[i..]),
        None => (rest, "/"),
    };
    if authority.is_empty() || authority.contains('@') {
        return Err(HttpError::BadUrl);
    }
    let (host, port) = match authority.rsplit_once(':') {
        Some((h, p)) if !h.ends_with(']') || h.starts_with('[') => {
            (h.to_string(), p.parse::<u16>().map_err(|_| HttpError::BadUrl)?)
        }
        _ => (authority.to_string(), 80),
    };
    if host.is_empty() || path.chars().any(|c| c.is_whitespace() || c.is_control()) {
        return Err(HttpError::BadUrl);
    }
    Ok(Url {
        host: host.trim_matches(|c| c == '[' || c == ']').to_string(),
        port,
        path: path.to_string(),
    })
}

/// GET `url` (following redirects), at most `max` bytes; `progress`
/// counts the body's bytes as they come and can cancel.
pub fn get(url: &str, max: u64, progress: &Progress) -> Result<Vec<u8>, HttpError> {
    let mut url = url.to_string();
    for _ in 0..=MAX_REDIRECTS {
        match get_once(&url, max, progress)? {
            Reply::Body(b) => return Ok(b),
            Reply::Redirect(to) => {
                url = if to.starts_with('/') {
                    let u = parse_url(&url)?;
                    format!("http://{}:{}{to}", u.host, u.port)
                } else {
                    to
                };
            }
        }
    }
    Err(HttpError::Other("too many redirects".into()))
}

enum Reply {
    Body(Vec<u8>),
    Redirect(String),
}

fn get_once(url: &str, max: u64, progress: &Progress) -> Result<Reply, HttpError> {
    let u = parse_url(url)?;
    let addr = (u.host.as_str(), u.port)
        .to_socket_addrs()
        .map_err(|e| HttpError::CantConnect(e.to_string()))?
        .next()
        .ok_or_else(|| HttpError::CantConnect("no address".into()))?;
    let stream =
        TcpStream::connect_timeout(&addr, CONNECT_TIMEOUT).map_err(|e| HttpError::CantConnect(e.to_string()))?;
    stream.set_read_timeout(Some(READ_TIMEOUT)).ok();
    stream.set_write_timeout(Some(READ_TIMEOUT)).ok();
    let mut writer = stream.try_clone().map_err(|e| HttpError::CantConnect(e.to_string()))?;
    let host = if u.port == 80 {
        u.host.clone()
    } else {
        format!("{}:{}", u.host, u.port)
    };
    let request = format!(
        "GET {} HTTP/1.1\r\nHost: {host}\r\nUser-Agent: mashup\r\nAccept-Encoding: identity\r\nConnection: close\r\n\r\n",
        u.path
    );
    writer
        .write_all(request.as_bytes())
        .map_err(|e| HttpError::CantConnect(e.to_string()))?;
    let mut r = BufReader::new(stream);
    let mut line = String::new();
    r.read_line(&mut line).map_err(|_| HttpError::NoHeaders)?;
    let mut parts = line.split_whitespace();
    let (Some(version), Some(code)) = (parts.next(), parts.next()) else {
        return Err(HttpError::NoHeaders);
    };
    if !version.starts_with("HTTP/") {
        return Err(HttpError::NoHeaders);
    }
    let code: u16 = code.parse().map_err(|_| HttpError::NoHeaders)?;
    let mut length: Option<u64> = None;
    let mut chunked = false;
    let mut location = None;
    loop {
        line.clear();
        if r.read_line(&mut line).map_err(|_| HttpError::NoHeaders)? == 0 {
            return Err(HttpError::NoHeaders);
        }
        let l = line.trim_end();
        if l.is_empty() {
            break;
        }
        let Some((k, v)) = l.split_once(':') else { continue };
        let v = v.trim();
        match k.trim().to_ascii_lowercase().as_str() {
            "content-length" => length = v.parse().ok(),
            "transfer-encoding" => chunked = v.to_ascii_lowercase().contains("chunked"),
            "location" => location = Some(v.to_string()),
            _ => {}
        }
    }
    match code {
        200 => {}
        301 | 302 | 303 | 307 | 308 => {
            return location.map(Reply::Redirect).ok_or(HttpError::NoHeaders);
        }
        404 | 410 => return Err(HttpError::NotFound),
        400..=499 => return Err(HttpError::NotFound),
        c => return Err(HttpError::Other(format!("HTTP {c}"))),
    }
    if let Some(n) = length {
        if n > max {
            return Err(HttpError::TooBig(n));
        }
        progress.total.store(n, Ordering::Relaxed);
    }
    let mut body = Vec::with_capacity(length.unwrap_or(0).min(max) as usize);
    if chunked {
        loop {
            line.clear();
            if r.read_line(&mut line).map_err(|_| HttpError::Closed)? == 0 {
                return Err(HttpError::Closed);
            }
            let size = u64::from_str_radix(line.trim().split(';').next().unwrap_or("").trim(), 16)
                .map_err(|_| HttpError::Other("bad chunk".into()))?;
            if size == 0 {
                break;
            }
            read_n(&mut r, size, max, &mut body, progress)?;
            line.clear();
            r.read_line(&mut line).map_err(|_| HttpError::Closed)?;
        }
    } else if let Some(n) = length {
        read_n(&mut r, n, max, &mut body, progress)?;
    } else {
        // To the end of the connection.
        read_n(&mut r, u64::MAX, max, &mut body, progress)?;
    }
    if body.is_empty() {
        return Err(HttpError::ZeroLength);
    }
    Ok(Reply::Body(body))
}

/// Read `n` bytes (or to the end when `n` is `u64::MAX`) into `out`.
fn read_n(r: &mut impl Read, n: u64, max: u64, out: &mut Vec<u8>, progress: &Progress) -> Result<(), HttpError> {
    let mut left = n;
    let mut buf = vec![0u8; 64 * 1024];
    while left > 0 {
        if progress.cancel.load(Ordering::Relaxed) {
            return Err(HttpError::Cancelled);
        }
        let want = left.min(buf.len() as u64) as usize;
        let got = match r.read(&mut buf[..want]) {
            Ok(0) if n == u64::MAX => return Ok(()),
            Ok(0) => return Err(HttpError::Closed),
            Ok(k) => k,
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(HttpError::Other(e.to_string())),
        };
        out.extend_from_slice(&buf[..got]);
        if out.len() as u64 > max {
            return Err(HttpError::TooBig(out.len() as u64));
        }
        progress.done.fetch_add(got as u64, Ordering::Relaxed);
        if n != u64::MAX {
            left -= got as u64;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls() {
        assert_eq!(
            parse_url("http://fastdl.example.com/cstrike").unwrap(),
            Url {
                host: "fastdl.example.com".into(),
                port: 80,
                path: "/cstrike".into()
            }
        );
        assert_eq!(parse_url("http://127.0.0.1:8080/a/b.bsp").unwrap().port, 8080);
        assert_eq!(parse_url("127.0.0.1:81").unwrap().path, "/");
        assert_eq!(parse_url("https://x/y"), Err(HttpError::BadProtocol));
        assert_eq!(parse_url("http:///y"), Err(HttpError::BadUrl));
        assert_eq!(parse_url("http://h:port/"), Err(HttpError::BadUrl));
    }

    /// One reply from a local listener, chunked, then a 404.
    #[test]
    fn chunked_bodies_and_missing_files() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = std::thread::spawn(move || {
            for reply in [
                "HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n5\r\nhello\r\n6\r\n world\r\n0\r\n\r\n",
                "HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\n\r\n",
            ] {
                let (mut s, _) = listener.accept().unwrap();
                let mut buf = Vec::new();
                let mut byte = [0u8; 1];
                while !buf.ends_with(b"\r\n\r\n") && s.read(&mut byte).is_ok_and(|n| n == 1) {
                    buf.push(byte[0]);
                }
                s.write_all(reply.as_bytes()).unwrap();
            }
        });
        let p = Progress::default();
        let url = format!("http://127.0.0.1:{port}/x");
        assert_eq!(get(&url, 1 << 20, &p).unwrap(), b"hello world");
        assert_eq!(p.done.load(Ordering::Relaxed), 11);
        assert_eq!(get(&url, 1 << 20, &Progress::default()), Err(HttpError::NotFound));
        server.join().unwrap();
    }
}
