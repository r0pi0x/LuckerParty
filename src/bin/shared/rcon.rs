//! Source RCON client (Valve's documented TCP protocol), shared by the dev
//! tools that drive CS:S (`refcmp`, `movecmp`).

use std::{
    io::{Read, Write},
    net::TcpStream,
    time::Duration,
};

pub struct Rcon {
    stream: TcpStream,
    next_id: i32,
}

impl Rcon {
    pub fn connect(addr: &str, password: &str) -> std::io::Result<Self> {
        let addr = addr.parse().map_err(std::io::Error::other)?;
        let stream = TcpStream::connect_timeout(&addr, Duration::from_secs(2))?;
        stream.set_read_timeout(Some(Duration::from_secs(10)))?;
        let mut r = Self { stream, next_id: 1 };
        r.send(3, password)?;
        loop {
            let (id, kind, _) = r.recv()?;
            if kind == 2 {
                if id == -1 {
                    return Err(std::io::Error::other("RCON authentication failed"));
                }
                return Ok(r);
            }
        }
    }

    fn send(&mut self, kind: i32, body: &str) -> std::io::Result<i32> {
        let id = self.next_id;
        self.next_id += 1;
        let mut packet = Vec::new();
        packet.extend((10 + body.len() as i32).to_le_bytes());
        packet.extend(id.to_le_bytes());
        packet.extend(kind.to_le_bytes());
        packet.extend(body.as_bytes());
        packet.extend([0, 0]);
        self.stream.write_all(&packet)?;
        Ok(id)
    }

    fn recv(&mut self) -> std::io::Result<(i32, i32, String)> {
        let mut len = [0u8; 4];
        self.stream.read_exact(&mut len)?;
        let mut data = vec![0u8; i32::from_le_bytes(len).max(10) as usize];
        self.stream.read_exact(&mut data)?;
        let id = i32::from_le_bytes(data[0..4].try_into().unwrap());
        let kind = i32::from_le_bytes(data[4..8].try_into().unwrap());
        Ok((id, kind, String::from_utf8_lossy(&data[8..data.len() - 2]).into_owned()))
    }

    /// Run a command and return its output (an empty marker packet after it
    /// tells us the response is complete).
    pub fn exec(&mut self, cmd: &str) -> std::io::Result<String> {
        self.send(2, cmd)?;
        let marker = self.send(0, "")?;
        let mut out = String::new();
        loop {
            let (id, _, body) = self.recv()?;
            if id == marker {
                return Ok(out);
            }
            out.push_str(&body);
        }
    }
}
