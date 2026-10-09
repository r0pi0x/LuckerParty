//! Server queries and LAN discovery (plan slice 8): a server answers a
//! small info request on its own UDP port, separate from the game
//! connection, so a client can list servers without joining them.
//!
//! The packets mirror Source's `A2S_INFO` as the Valve Developer Wiki's
//! "Server queries" page documents it (public): a 4-byte `FF FF FF FF`
//! header (no netcode packet starts with `FF`, so the server's socket
//! tells them apart: `udp::UdpServer`), the request `T` with
//! `"Source Engine Query\0"`, a 4-byte challenge reply `A` first (the
//! request is sent again with the challenge appended: a spoofed source
//! address gets back no more than it sent), then the info reply `I`:
//! protocol, name, map, folder, game, app id, players, max players, bots,
//! server type (`d` dedicated, `l` listen), environment (`l`, `w`, `m`),
//! visibility (1: password), VAC (0), version, and the extra data flag
//! with the port (`0x80`), an instance id in the SteamID field (`0x10`,
//! random per server process: one server reached at two addresses is
//! listed once) and keywords (`0x20`). Our servers say folder `mashup`;
//! replies from anything else (a Source server on the LAN) are left out.
//! Answers are rate-limited per address and in total (`Responder`).
//!
//! The client side (`ServerQueries`) sends requests from its own socket,
//! measures the ping as the round trip of the challenged request, retries
//! once, and scans the LAN: the request broadcast to each port of
//! `net_lan_ports` (27015-27020, as Source's LAN tab) at the
//! `net_lan_broadcast` addresses and to this machine (127.0.0.1).

use std::{
    collections::HashMap,
    hash::{Hash, Hasher},
    net::{IpAddr, Ipv4Addr, SocketAddr, UdpSocket},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

use bevy::prelude::*;

use super::{NET_VERSION, NetSettings};
use crate::{
    console::{ConsoleAppExt, resource_cvar},
    core::{Intent, LocalPlayer, NetRole},
};

/// Every query packet starts with this (a connectionless packet in
/// Source's terms).
pub const HEADER: [u8; 4] = [0xFF; 4];
/// Request: server info.
pub const A2S_INFO: u8 = b'T';
/// Reply: server info.
pub const S2A_INFO: u8 = b'I';
/// Reply: send the request again with this challenge.
pub const S2C_CHALLENGE: u8 = b'A';
/// The info request's payload.
pub const INFO_PAYLOAD: &[u8] = b"Source Engine Query\0";
/// The protocol byte of the info reply (Source's).
pub const PROTOCOL: u8 = 17;
/// Our servers' game folder: replies naming another are not ours.
pub const GAME_FOLDER: &str = "mashup";
/// The game's description in the reply (the browser's Game column).
pub const GAME_NAME: &str = "Lucker Party";
/// Ports the LAN tab scans unless `net_lan_ports` says otherwise
/// (Source's: 27015-27020).
pub const LAN_PORTS: (u16, u16) = (27015, 27020);

/// Extra data flags of the info reply.
mod edf {
    pub const PORT: u8 = 0x80;
    pub const STEAM_ID: u8 = 0x10;
    pub const KEYWORDS: u8 = 0x20;
    pub const SOURCE_TV: u8 = 0x40;
    pub const GAME_ID: u8 = 0x01;
}

/// What a server says about itself.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ServerInfo {
    pub protocol: u8,
    pub name: String,
    pub map: String,
    pub folder: String,
    pub game: String,
    pub app_id: u16,
    /// Players in the game, bots included (as Source counts them).
    pub players: u8,
    pub max_players: u8,
    pub bots: u8,
    pub dedicated: bool,
    /// `l` Linux, `w` Windows, `m` macOS.
    pub os: char,
    pub password: bool,
    pub secure: bool,
    pub version: String,
    /// The game port (the same as the query port for ours).
    pub port: Option<u16>,
    /// Random per server process (Source's SteamID field).
    pub instance: Option<u64>,
    pub keywords: Option<String>,
}

impl ServerInfo {
    /// Whether it's one of ours (not a Source server that answered).
    pub fn is_ours(&self) -> bool {
        self.folder.eq_ignore_ascii_case(GAME_FOLDER)
    }

    /// The info reply.
    pub fn encode(&self) -> Vec<u8> {
        let mut out = HEADER.to_vec();
        out.push(S2A_INFO);
        out.push(self.protocol);
        for s in [&self.name, &self.map, &self.folder, &self.game] {
            put_str(&mut out, s);
        }
        out.extend_from_slice(&self.app_id.to_le_bytes());
        out.extend_from_slice(&[
            self.players,
            self.max_players,
            self.bots,
            if self.dedicated { b'd' } else { b'l' },
            self.os as u8,
            self.password as u8,
            self.secure as u8,
        ]);
        put_str(&mut out, &self.version);
        let mut flags = 0;
        if self.port.is_some() {
            flags |= edf::PORT;
        }
        if self.instance.is_some() {
            flags |= edf::STEAM_ID;
        }
        if self.keywords.is_some() {
            flags |= edf::KEYWORDS;
        }
        out.push(flags);
        if let Some(p) = self.port {
            out.extend_from_slice(&p.to_le_bytes());
        }
        if let Some(id) = self.instance {
            out.extend_from_slice(&id.to_le_bytes());
        }
        if let Some(k) = &self.keywords {
            put_str(&mut out, k);
        }
        out
    }
}

/// A string as the protocol writes it: its bytes (no NUL inside) and a
/// NUL, at most 255 bytes.
fn put_str(out: &mut Vec<u8>, s: &str) {
    let mut n = 0;
    for c in s.chars().filter(|c| *c != '\0') {
        if n + c.len_utf8() > 255 {
            break;
        }
        let mut b = [0; 4];
        out.extend_from_slice(c.encode_utf8(&mut b).as_bytes());
        n += c.len_utf8();
    }
    out.push(0);
}

/// Reads a reply front to back.
struct Reader<'a>(&'a [u8]);

impl Reader<'_> {
    fn u8(&mut self) -> Option<u8> {
        let (&b, rest) = self.0.split_first()?;
        self.0 = rest;
        Some(b)
    }

    fn bytes<const N: usize>(&mut self) -> Option<[u8; N]> {
        let b: [u8; N] = self.0.get(..N)?.try_into().ok()?;
        self.0 = &self.0[N..];
        Some(b)
    }

    fn str(&mut self) -> Option<String> {
        let end = self.0.iter().position(|&b| b == 0)?;
        let s = String::from_utf8_lossy(&self.0[..end]).into_owned();
        self.0 = &self.0[end + 1..];
        Some(s)
    }
}

/// The info request, with the challenge the server gave (None: the first).
pub fn info_request(challenge: Option<i32>) -> Vec<u8> {
    let mut out = HEADER.to_vec();
    out.push(A2S_INFO);
    out.extend_from_slice(INFO_PAYLOAD);
    if let Some(c) = challenge {
        out.extend_from_slice(&c.to_le_bytes());
    }
    out
}

/// Whether a packet is a query (not netcode's).
pub fn is_query(packet: &[u8]) -> bool {
    packet.starts_with(&HEADER)
}

/// A request a server understands: the info request and its challenge
/// (None without one, or `-1`, Source's "none yet").
pub fn parse_request(packet: &[u8]) -> Option<Option<i32>> {
    let rest = packet.strip_prefix(&HEADER)?;
    let rest = rest.strip_prefix(&[A2S_INFO])?;
    let rest = rest.strip_prefix(INFO_PAYLOAD)?;
    match rest.len() {
        0 => Some(None),
        4.. => {
            let c = i32::from_le_bytes(rest[..4].try_into().ok()?);
            Some((c != -1).then_some(c))
        }
        _ => None,
    }
}

/// A reply a client understands.
#[derive(Clone, Debug, PartialEq)]
pub enum Reply {
    Challenge(i32),
    Info(ServerInfo),
}

pub fn challenge_reply(challenge: i32) -> Vec<u8> {
    let mut out = HEADER.to_vec();
    out.push(S2C_CHALLENGE);
    out.extend_from_slice(&challenge.to_le_bytes());
    out
}

pub fn parse_reply(packet: &[u8]) -> Option<Reply> {
    let rest = packet.strip_prefix(&HEADER)?;
    let mut r = Reader(rest);
    match r.u8()? {
        S2C_CHALLENGE => Some(Reply::Challenge(i32::from_le_bytes(r.bytes()?))),
        S2A_INFO => {
            let mut info = ServerInfo {
                protocol: r.u8()?,
                name: r.str()?,
                map: r.str()?,
                folder: r.str()?,
                game: r.str()?,
                app_id: u16::from_le_bytes(r.bytes()?),
                players: r.u8()?,
                max_players: r.u8()?,
                bots: r.u8()?,
                dedicated: r.u8()? == b'd',
                os: r.u8()? as char,
                password: r.u8()? != 0,
                secure: r.u8()? != 0,
                version: r.str()?,
                ..default()
            };
            // The extra data is optional (old servers stop here).
            if let Some(flags) = r.u8() {
                if flags & edf::PORT != 0 {
                    info.port = Some(u16::from_le_bytes(r.bytes()?));
                }
                if flags & edf::STEAM_ID != 0 {
                    info.instance = Some(u64::from_le_bytes(r.bytes()?));
                }
                if flags & edf::SOURCE_TV != 0 {
                    let _port: [u8; 2] = r.bytes()?;
                    let _name = r.str()?;
                }
                if flags & edf::KEYWORDS != 0 {
                    info.keywords = Some(r.str()?);
                }
                if flags & edf::GAME_ID != 0 {
                    let _id: [u8; 8] = r.bytes()?;
                }
            }
            Some(Reply::Info(info))
        }
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// The server side.

/// A token bucket: `rate` a second, at most `burst` saved up.
#[derive(Clone, Copy, Debug)]
struct Bucket {
    tokens: f32,
    at: Duration,
}

impl Bucket {
    fn take(&mut self, now: Duration, rate: f32, burst: f32) -> bool {
        let dt = now.saturating_sub(self.at).as_secs_f32();
        self.tokens = (self.tokens + dt * rate).min(burst);
        self.at = now;
        if self.tokens >= 1.0 {
            self.tokens -= 1.0;
            true
        } else {
            false
        }
    }
}

/// Answers per address a second (and saved up); in total.
pub const PER_ADDRESS: (f32, f32) = (8.0, 16.0);
pub const TOTAL: (f32, f32) = (200.0, 400.0);
/// Addresses remembered for their buckets before the oldest are forgotten.
const MAX_ADDRESSES: usize = 4096;
/// How long a challenge holds, s (the current window and the one before).
const CHALLENGE_WINDOW: u64 = 30;

/// A server's query answers: challenges from a secret (nothing kept per
/// request), rate limits, counts (`status`).
#[derive(Debug)]
pub struct Responder {
    secret: u64,
    buckets: HashMap<IpAddr, Bucket>,
    total: Bucket,
    /// Info replies sent, challenges sent, requests dropped (rate limit or
    /// malformed).
    pub answered: u64,
    pub challenged: u64,
    pub dropped: u64,
}

impl Responder {
    pub fn new(secret: u64) -> Self {
        Self {
            secret,
            buckets: HashMap::new(),
            total: Bucket {
                tokens: TOTAL.1,
                at: Duration::ZERO,
            },
            answered: 0,
            challenged: 0,
            dropped: 0,
        }
    }

    /// The challenge for an address in a time window.
    fn challenge(&self, from: SocketAddr, window: u64) -> i32 {
        let mut h = std::collections::hash_map::DefaultHasher::new();
        (self.secret, from, window).hash(&mut h);
        // Never -1 (Source's "no challenge yet").
        let c = h.finish() as i32;
        if c == -1 { 0 } else { c }
    }

    /// The answer to a query packet from `from` at `now` (a monotonic
    /// clock): a challenge, the info, or nothing (not a request, or over
    /// the rate limits).
    pub fn handle(&mut self, from: SocketAddr, packet: &[u8], now: Duration, info: &ServerInfo) -> Option<Vec<u8>> {
        let Some(challenge) = parse_request(packet) else {
            self.dropped += 1;
            return None;
        };
        if self.buckets.len() >= MAX_ADDRESSES && !self.buckets.contains_key(&from.ip()) {
            // Forget those that have been quiet longest.
            let mut ats: Vec<Duration> = self.buckets.values().map(|b| b.at).collect();
            ats.sort();
            let cut = ats[ats.len() / 2];
            self.buckets.retain(|_, b| b.at > cut);
        }
        let bucket = self.buckets.entry(from.ip()).or_insert(Bucket {
            tokens: PER_ADDRESS.1,
            at: now,
        });
        if !bucket.take(now, PER_ADDRESS.0, PER_ADDRESS.1) || !self.total.take(now, TOTAL.0, TOTAL.1) {
            self.dropped += 1;
            return None;
        }
        let window = now.as_secs() / CHALLENGE_WINDOW;
        let valid = challenge.is_some_and(|c| {
            c == self.challenge(from, window) || (window > 0 && c == self.challenge(from, window - 1))
        });
        if valid {
            self.answered += 1;
            Some(info.encode())
        } else {
            self.challenged += 1;
            Some(challenge_reply(self.challenge(from, window)))
        }
    }
}

/// A server's name and password (`hostname`, `sv_password`).
#[derive(Resource, Clone, Debug)]
pub struct Hosting {
    pub hostname: String,
    pub password: String,
}

impl Default for Hosting {
    fn default() -> Self {
        Self {
            hostname: GAME_NAME.into(),
            password: String::new(),
        }
    }
}

/// The password this client joins with (`password`, as Source's).
#[derive(Resource, Clone, Debug, Default)]
pub struct JoinPassword(pub String);

/// The bytes of netcode's user data a client sends: a tag, the
/// password's length, the password (at most 250 bytes).
pub fn user_data(password: &str) -> [u8; 256] {
    let mut out = [0u8; 256];
    out[0] = 1;
    let bytes = &password.as_bytes()[..password.len().min(250)];
    out[1] = bytes.len() as u8;
    out[2..2 + bytes.len()].copy_from_slice(bytes);
    out
}

/// The password in a client's user data ("" without).
pub fn password_of(user_data: Option<&[u8; 256]>) -> String {
    match user_data {
        Some(d) if d[0] == 1 => {
            let n = (d[1] as usize).min(250);
            String::from_utf8_lossy(&d[2..2 + n]).into_owned()
        }
        _ => String::new(),
    }
}

/// This server's info as queries get it, kept current while serving.
#[derive(Resource, Clone, Debug, Default)]
pub struct LocalServerInfo(pub ServerInfo);

/// This process's server instance id (random, per process).
#[derive(Resource, Clone, Copy, Debug)]
pub struct InstanceId(pub u64);

impl Default for InstanceId {
    fn default() -> Self {
        let mut h = std::collections::hash_map::DefaultHasher::new();
        (std::process::id(), Instant::now(), std::time::SystemTime::now()).hash(&mut h);
        Self(h.finish())
    }
}

/// The server's info now: name, map, players and bots, its limits.
fn update_info(
    role: Res<NetRole>,
    settings: Res<NetSettings>,
    hosting: Res<Hosting>,
    instance: Res<InstanceId>,
    characters: Query<
        (Has<crate::bot::Bot>, Has<LocalPlayer>),
        (With<Intent>, Without<crate::objectives::hostages::Hostage>),
    >,
    map: Option<Res<crate::map::LoadedMapName>>,
    port: Option<Res<super::udp::UdpServer>>,
    mut info: ResMut<LocalServerInfo>,
) {
    if *role != NetRole::Server {
        return;
    }
    let (mut players, mut bots, mut host) = (0u32, 0u32, false);
    for (bot, local) in &characters {
        players += 1;
        bots += bot as u32;
        host |= local;
    }
    let name = map.as_ref().map(|m| m.0.clone()).unwrap_or_default();
    let map = super::normalize_map(&name);
    let next = ServerInfo {
        protocol: PROTOCOL,
        name: hosting.hostname.clone(),
        map: map.rsplit(':').next().unwrap_or(&map).to_string(),
        folder: GAME_FOLDER.into(),
        game: GAME_NAME.into(),
        app_id: 0,
        players: players.min(255) as u8,
        max_players: settings.maxplayers.min(255) as u8,
        bots: bots.min(255) as u8,
        dedicated: !host,
        os: if cfg!(windows) {
            'w'
        } else if cfg!(target_os = "macos") {
            'm'
        } else {
            'l'
        },
        password: !hosting.password.is_empty(),
        secure: false,
        version: NET_VERSION.into(),
        port: port.and_then(|p| p.local_addr()).map(|a| a.port()),
        instance: Some(instance.0),
        keywords: Some(
            map.split(':')
                .next()
                .filter(|g| *g != map)
                .unwrap_or("mashup")
                .to_string(),
        ),
    };
    if info.0 != next {
        info.0 = next;
    }
}

// ---------------------------------------------------------------------------
// The client side.

/// How long to wait for a reply before asking again, and how many times
/// to ask.
pub const QUERY_TIMEOUT: Duration = Duration::from_millis(1200);
pub const QUERY_TRIES: u8 = 2;
/// How long a LAN scan listens for servers.
pub const LAN_SCAN_TIME: Duration = Duration::from_millis(1500);

/// A server's state in the list.
#[derive(Clone, Debug, PartialEq)]
pub enum QueryState {
    /// Asked, no answer yet.
    Pending,
    /// It answered: its info and the ping, ms.
    Answered { info: ServerInfo, ping_ms: u32 },
    /// No answer after every try.
    NoResponse,
}

/// A server queried (or found on the LAN).
#[derive(Clone, Debug, PartialEq)]
pub struct QueryResult {
    pub addr: SocketAddr,
    pub state: QueryState,
    /// Found by a LAN scan (not asked for by address).
    pub lan: bool,
}

#[derive(Clone, Debug)]
struct Asking {
    /// When the last request went out (the ping is from it).
    sent: Instant,
    tries: u8,
    challenge: Option<i32>,
}

/// A LAN scan under way.
#[derive(Clone, Debug)]
struct Scan {
    ports: (u16, u16),
    until: Instant,
}

/// The client's queries: one socket, servers asked and their answers
/// (`results`, by address). `query` asks one server, `scan_lan` asks
/// every server on the local network; `poll` (each frame) reads replies,
/// asks again on a challenge or a timeout.
#[derive(Resource, Default)]
pub struct ServerQueries {
    socket: Option<UdpSocket>,
    /// Replies a reader thread took off the socket, with when they came
    /// (the ping doesn't wait for the next frame).
    received: Arc<Mutex<Vec<(Instant, SocketAddr, Vec<u8>)>>>,
    stop: Arc<AtomicBool>,
    asking: HashMap<SocketAddr, Asking>,
    scan: Option<Scan>,
    pub results: Vec<QueryResult>,
    /// Bumped whenever `results` change (the browser redraws).
    pub generation: u64,
    /// Why the last send failed (no network), for the status line.
    pub error: Option<String>,
}

impl Drop for ServerQueries {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

impl ServerQueries {
    fn socket(&mut self) -> Result<&UdpSocket, String> {
        if self.socket.is_none() {
            let s = UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0)).map_err(|e| format!("UDP socket: {e}"))?;
            s.set_broadcast(true).map_err(|e| e.to_string())?;
            s.set_read_timeout(Some(Duration::from_millis(100)))
                .map_err(|e| e.to_string())?;
            let reader = s.try_clone().map_err(|e| e.to_string())?;
            let (received, stop) = (self.received.clone(), self.stop.clone());
            std::thread::Builder::new()
                .name("server queries".into())
                .spawn(move || {
                    let mut buf = [0u8; 1400];
                    while !stop.load(Ordering::Relaxed) {
                        match reader.recv_from(&mut buf) {
                            Ok((n, from)) => {
                                if let Ok(mut r) = received.lock() {
                                    r.push((Instant::now(), from, buf[..n].to_vec()));
                                }
                            }
                            Err(e)
                                if matches!(
                                    e.kind(),
                                    std::io::ErrorKind::WouldBlock
                                        | std::io::ErrorKind::TimedOut
                                        | std::io::ErrorKind::Interrupted
                                        // Windows: an earlier send's "port unreachable".
                                        | std::io::ErrorKind::ConnectionReset
                                ) => {}
                            Err(e) => {
                                debug!("server queries: {e}");
                                std::thread::sleep(Duration::from_millis(50));
                            }
                        }
                    }
                })
                .map_err(|e| e.to_string())?;
            self.socket = Some(s);
        }
        Ok(self.socket.as_ref().unwrap())
    }

    fn send(&mut self, to: SocketAddr, packet: &[u8]) {
        let r = self
            .socket()
            .and_then(|s| s.send_to(packet, to).map_err(|e| format!("{to}: {e}")));
        if let Err(e) = r {
            debug!("server query: {e}");
            self.error = Some(e);
        }
    }

    /// The entry for `addr` (made pending if new).
    fn entry(&mut self, addr: SocketAddr, lan: bool) -> &mut QueryResult {
        let i = match self.results.iter().position(|r| r.addr == addr) {
            Some(i) => i,
            None => {
                self.results.push(QueryResult {
                    addr,
                    state: QueryState::Pending,
                    lan,
                });
                self.results.len() - 1
            }
        };
        &mut self.results[i]
    }

    /// Ask a server for its info (again, if asked before).
    pub fn query(&mut self, addr: SocketAddr) {
        self.ask(addr, false, Instant::now());
    }

    fn ask(&mut self, addr: SocketAddr, lan: bool, now: Instant) {
        // What it said last stays shown until it answers again.
        let e = self.entry(addr, lan);
        if !matches!(e.state, QueryState::Answered { .. }) {
            e.state = QueryState::Pending;
        }
        e.lan |= lan;
        self.asking.insert(
            addr,
            Asking {
                sent: now,
                tries: 1,
                challenge: None,
            },
        );
        self.send(addr, &info_request(None));
        self.generation += 1;
    }

    /// Ask every server on the local network: the request broadcast to
    /// each port in `ports` at each of `broadcast`, and sent to this
    /// machine's. Servers found before are forgotten.
    pub fn scan_lan(&mut self, ports: (u16, u16), broadcast: &[IpAddr]) {
        let now = Instant::now();
        self.results.retain(|r| !r.lan);
        self.error = None;
        self.scan = Some(Scan {
            ports,
            until: now + LAN_SCAN_TIME,
        });
        let request = info_request(None);
        for port in ports.0..=ports.1.max(ports.0) {
            for ip in broadcast.iter().chain(&[IpAddr::V4(Ipv4Addr::LOCALHOST)]) {
                self.send(SocketAddr::new(*ip, port), &request);
            }
        }
        self.generation += 1;
    }

    /// Whether a LAN scan is still listening, or a server still asked.
    pub fn busy(&self) -> bool {
        self.scan.is_some() || !self.asking.is_empty()
    }

    /// Forget a server (and stop asking it).
    pub fn forget(&mut self, addr: SocketAddr) {
        self.asking.remove(&addr);
        self.results.retain(|r| r.addr != addr);
        self.generation += 1;
    }

    pub fn result(&self, addr: SocketAddr) -> Option<&QueryResult> {
        self.results.iter().find(|r| r.addr == addr)
    }

    /// Read replies, answer challenges, ask again or give up on servers
    /// that don't answer.
    pub fn poll(&mut self) {
        let replies = self
            .received
            .lock()
            .map(|mut r| std::mem::take(&mut *r))
            .unwrap_or_default();
        for (at, from, packet) in replies {
            self.reply(from, &packet, at);
        }
        let now = Instant::now();
        // Timeouts.
        let mut again = Vec::new();
        let mut gave_up = Vec::new();
        for (addr, a) in &self.asking {
            if now.duration_since(a.sent) < QUERY_TIMEOUT {
                continue;
            }
            if a.tries < QUERY_TRIES {
                again.push(*addr);
            } else {
                gave_up.push(*addr);
            }
        }
        for addr in again {
            let challenge = {
                let a = self.asking.get_mut(&addr).unwrap();
                a.tries += 1;
                a.sent = now;
                a.challenge
            };
            self.send(addr, &info_request(challenge));
        }
        for addr in gave_up {
            self.asking.remove(&addr);
            let lan = self.result(addr).is_some_and(|r| r.lan);
            if lan {
                // A LAN server that went quiet: not listed.
                self.results.retain(|r| r.addr != addr);
            } else {
                self.entry(addr, false).state = QueryState::NoResponse;
            }
            self.generation += 1;
        }
        if self.scan.as_ref().is_some_and(|s| now >= s.until) {
            self.scan = None;
            self.generation += 1;
        }
    }

    fn reply(&mut self, from: SocketAddr, packet: &[u8], now: Instant) {
        let Some(reply) = parse_reply(packet) else { return };
        let scanning = self
            .scan
            .as_ref()
            .is_some_and(|s| (s.ports.0..=s.ports.1).contains(&from.port()));
        if !self.asking.contains_key(&from) {
            // A server answering the LAN broadcast: ask it as any other.
            if !(scanning && matches!(reply, Reply::Challenge(_))) {
                return;
            }
            if self
                .result(from)
                .is_some_and(|r| matches!(r.state, QueryState::Answered { .. }))
            {
                return;
            }
            self.entry(from, true);
            self.asking.insert(
                from,
                Asking {
                    sent: now,
                    tries: 1,
                    challenge: None,
                },
            );
        }
        match reply {
            Reply::Challenge(c) => {
                let a = self.asking.get_mut(&from).unwrap();
                a.challenge = Some(c);
                // The round trip is from this send.
                a.sent = Instant::now();
                self.send(from, &info_request(Some(c)));
            }
            Reply::Info(info) => {
                let a = self.asking.remove(&from).unwrap();
                let lan = self.result(from).is_some_and(|r| r.lan);
                if lan && !info.is_ours() {
                    // Another game's server on a scanned port.
                    self.results.retain(|r| r.addr != from);
                    self.generation += 1;
                    return;
                }
                // One server at two addresses (its LAN address and
                // loopback): listed once, at the LAN address.
                if lan
                    && let Some(id) = info.instance
                    && let Some(other) = self.results.iter().position(|r| {
                        r.addr != from
                            && r.lan
                            && matches!(&r.state, QueryState::Answered { info, .. } if info.instance == Some(id))
                    })
                {
                    if from.ip().is_loopback() {
                        self.results.retain(|r| r.addr != from);
                        self.generation += 1;
                        return;
                    }
                    self.results.remove(other);
                }
                let ping_ms = now.saturating_duration_since(a.sent).as_millis().min(9999) as u32;
                self.entry(from, lan).state = QueryState::Answered { info, ping_ms };
                self.generation += 1;
            }
        }
    }
}

/// `net_lan_ports` and `net_lan_broadcast`.
#[derive(Resource, Clone, Debug)]
pub struct LanSettings {
    /// `first-last` or one port.
    pub ports: String,
    /// Broadcast addresses, space separated.
    pub broadcast: String,
}

impl Default for LanSettings {
    fn default() -> Self {
        Self {
            ports: format!("{}-{}", LAN_PORTS.0, LAN_PORTS.1),
            broadcast: "255.255.255.255".into(),
        }
    }
}

impl LanSettings {
    /// The port range (Source's when unreadable).
    pub fn port_range(&self) -> (u16, u16) {
        let t = self.ports.trim();
        let parse = |s: &str| s.trim().parse::<u16>().ok();
        let range = match t.split_once('-') {
            Some((a, b)) => parse(a).zip(parse(b)),
            None => parse(t).map(|p| (p, p)),
        };
        match range {
            Some((a, b)) if a > 0 && b >= a && b - a <= 64 => (a, b),
            _ => LAN_PORTS,
        }
    }

    pub fn broadcast_addresses(&self) -> Vec<IpAddr> {
        self.broadcast
            .split_whitespace()
            .filter_map(|s| s.parse().ok())
            .collect()
    }
}

/// One line about a server, for the console.
pub fn describe(r: &QueryResult) -> String {
    match &r.state {
        QueryState::Pending => format!("{}  (asking...)", r.addr),
        QueryState::NoResponse => format!("{}  < not responding >", r.addr),
        QueryState::Answered { info, ping_ms } => format!(
            "{}  \"{}\"  {}  {}/{} players ({} bots)  {} ms  {}{}{}",
            r.addr,
            info.name,
            info.map,
            info.players,
            info.max_players,
            info.bots,
            ping_ms,
            if info.dedicated { "dedicated" } else { "listen" },
            if info.password { ", password" } else { "" },
            if info.version != NET_VERSION {
                format!(", version {}", info.version)
            } else {
                String::new()
            }
        ),
    }
}

/// Pending console printouts of `serverinfo`/`lanscan`: printed once the
/// queries settle.
#[derive(Resource, Default)]
struct ConsoleQuery {
    /// Addresses asked by `serverinfo` (None: the LAN scan's results).
    addrs: Option<Vec<SocketAddr>>,
    waiting: bool,
}

fn poll(mut queries: ResMut<ServerQueries>) {
    if queries.socket.is_some() {
        queries.poll();
    }
}

fn print_console_query(
    mut pending: ResMut<ConsoleQuery>,
    queries: Res<ServerQueries>,
    mut console: ResMut<crate::console::Console>,
) {
    if !pending.waiting || queries.busy() {
        return;
    }
    pending.waiting = false;
    let lines: Vec<String> = match &pending.addrs {
        Some(addrs) => addrs.iter().filter_map(|a| queries.result(*a)).map(describe).collect(),
        None => {
            let found: Vec<String> = queries.results.iter().filter(|r| r.lan).map(describe).collect();
            if found.is_empty() {
                vec!["No servers on the local network.".into()]
            } else {
                found
            }
        }
    };
    for l in lines {
        console.info(l);
    }
}

pub(super) fn plugin(app: &mut App) {
    app.init_resource::<Hosting>()
        .init_resource::<JoinPassword>()
        .init_resource::<LocalServerInfo>()
        .init_resource::<InstanceId>()
        .init_resource::<ServerQueries>()
        .init_resource::<LanSettings>()
        .init_resource::<ConsoleQuery>()
        .add_systems(PreUpdate, update_info.before(bevy_replicon_renet::RenetReceive))
        .add_systems(Update, (poll, print_console_query).chain());
    resource_cvar::<Hosting, String>(app, "hostname", "The server's name in server lists.", |h| {
        &mut h.hostname
    });
    resource_cvar::<Hosting, String>(
        app,
        "sv_password",
        "Players must give this password to join (`password`); empty: none.",
        |h| &mut h.password,
    );
    resource_cvar::<JoinPassword, String>(app, "password", "The password to join a server with.", |p| &mut p.0);
    resource_cvar::<LanSettings, String>(
        app,
        "net_lan_ports",
        "UDP ports the LAN tab and `lanscan` look for servers on: first-last (Source's 27015-27020).",
        |l| &mut l.ports,
    );
    resource_cvar::<LanSettings, String>(
        app,
        "net_lan_broadcast",
        "Addresses the LAN scan broadcasts to, space separated (this machine is always asked too).",
        |l| &mut l.broadcast,
    );
    app.console_command(
        "serverinfo",
        "serverinfo <ip[:port]> ...: ask servers for their name, map, players and ping.",
        |w, a| {
            if a.is_empty() {
                return Err("serverinfo <ip[:port]> ...".into());
            }
            let addrs = a
                .iter()
                .map(|s| super::parse_address(s))
                .collect::<Result<Vec<_>, _>>()?;
            let mut q = w.resource_mut::<ServerQueries>();
            for addr in &addrs {
                q.query(*addr);
            }
            *w.resource_mut::<ConsoleQuery>() = ConsoleQuery {
                addrs: Some(addrs),
                waiting: true,
            };
            Ok(Some("Asking...".into()))
        },
    )
    .console_command(
        "lanscan",
        "List servers on the local network (`net_lan_ports`).",
        |w, _| {
            let lan = w.resource::<LanSettings>().clone();
            let (ports, broadcast) = (lan.port_range(), lan.broadcast_addresses());
            w.resource_mut::<ServerQueries>().scan_lan(ports, &broadcast);
            *w.resource_mut::<ConsoleQuery>() = ConsoleQuery {
                addrs: None,
                waiting: true,
            };
            Ok(Some(format!(
                "Looking for servers on UDP ports {}-{}...",
                ports.0, ports.1
            )))
        },
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    fn info() -> ServerInfo {
        ServerInfo {
            protocol: PROTOCOL,
            name: "Test server".into(),
            map: "de_dust2".into(),
            folder: GAME_FOLDER.into(),
            game: GAME_NAME.into(),
            app_id: 0,
            players: 3,
            max_players: 8,
            bots: 2,
            dedicated: true,
            os: 'l',
            password: true,
            secure: false,
            version: NET_VERSION.into(),
            port: Some(27031),
            instance: Some(0x1122_3344_5566_7788),
            keywords: Some("cs_source".into()),
        }
    }

    #[test]
    fn info_reply_round_trips() {
        let i = info();
        assert_eq!(parse_reply(&i.encode()), Some(Reply::Info(i.clone())));
        let bare = ServerInfo {
            port: None,
            instance: None,
            keywords: None,
            ..i
        };
        assert_eq!(parse_reply(&bare.encode()), Some(Reply::Info(bare)));
    }

    #[test]
    fn the_layout_is_a2s_info() {
        // The documented byte layout, front to back.
        let mut i = info();
        i.instance = None;
        i.keywords = None;
        let b = i.encode();
        assert_eq!(&b[..6], &[0xFF, 0xFF, 0xFF, 0xFF, b'I', 17]);
        let mut at = 6;
        for s in ["Test server", "de_dust2", "mashup", "Lucker Party"] {
            assert_eq!(&b[at..at + s.len()], s.as_bytes());
            assert_eq!(b[at + s.len()], 0);
            at += s.len() + 1;
        }
        assert_eq!(&b[at..at + 9], &[0, 0, 3, 8, 2, b'd', b'l', 1, 0]);
        at += 9;
        at += NET_VERSION.len() + 1;
        assert_eq!(b[at], 0x80, "the extra data flag: port");
        assert_eq!(&b[at + 1..], &27031u16.to_le_bytes());
    }

    #[test]
    fn requests_and_challenges() {
        let r = info_request(None);
        assert_eq!(&r[..5], &[0xFF, 0xFF, 0xFF, 0xFF, b'T']);
        assert_eq!(&r[5..], b"Source Engine Query\0");
        assert_eq!(parse_request(&r), Some(None));
        assert_eq!(parse_request(&info_request(Some(-1))), Some(None));
        assert_eq!(parse_request(&info_request(Some(1234))), Some(Some(1234)));
        assert_eq!(parse_request(b"\xFF\xFF\xFF\xFFTSource"), None);
        assert_eq!(parse_reply(&challenge_reply(-5)), Some(Reply::Challenge(-5)));
        assert!(parse_reply(b"\xFF\xFF\xFF\xFFI\x11abc").is_none(), "truncated");
        assert!(is_query(&r) && !is_query(&[0x01, 0xFF, 0xFF, 0xFF]));
    }

    #[test]
    fn info_only_with_the_right_challenge() {
        let mut r = Responder::new(42);
        let from: SocketAddr = "10.0.0.5:50000".parse().unwrap();
        let t = Duration::from_secs(100);
        let Some(Reply::Challenge(c)) = r
            .handle(from, &info_request(None), t, &info())
            .and_then(|b| parse_reply(&b))
        else {
            panic!("a challenge first");
        };
        // A challenge reply is never bigger than the request.
        assert!(challenge_reply(c).len() <= info_request(None).len());
        // A wrong one: challenged again.
        let wrong = r.handle(from, &info_request(Some(c ^ 1)), t, &info()).unwrap();
        assert!(matches!(parse_reply(&wrong), Some(Reply::Challenge(_))));
        // Another address can't use it.
        let other: SocketAddr = "10.0.0.6:50000".parse().unwrap();
        let theirs = r.handle(other, &info_request(Some(c)), t, &info()).unwrap();
        assert!(matches!(parse_reply(&theirs), Some(Reply::Challenge(_))));
        let ok = r.handle(from, &info_request(Some(c)), t, &info()).unwrap();
        assert_eq!(parse_reply(&ok), Some(Reply::Info(info())));
        // Still good in the next window; not two windows on.
        let later = t + Duration::from_secs(CHALLENGE_WINDOW);
        assert!(matches!(
            r.handle(from, &info_request(Some(c)), later, &info())
                .and_then(|b| parse_reply(&b)),
            Some(Reply::Info(_))
        ));
        let much_later = t + Duration::from_secs(3 * CHALLENGE_WINDOW);
        assert!(matches!(
            r.handle(from, &info_request(Some(c)), much_later, &info())
                .and_then(|b| parse_reply(&b)),
            Some(Reply::Challenge(_))
        ));
        assert!(
            r.handle(from, b"\xFF\xFF\xFF\xFFVxxxx", t, &info()).is_none(),
            "not a request"
        );
    }

    #[test]
    fn answers_are_rate_limited() {
        let mut r = Responder::new(7);
        let from: SocketAddr = "10.0.0.5:50000".parse().unwrap();
        let t = Duration::from_secs(5);
        let answered = (0..100)
            .filter(|_| r.handle(from, &info_request(None), t, &info()).is_some())
            .count();
        assert_eq!(answered, PER_ADDRESS.1 as usize, "a burst, then nothing");
        // A second later: the rate's worth more.
        let more = (0..100)
            .filter(|_| {
                r.handle(from, &info_request(None), t + Duration::from_secs(1), &info())
                    .is_some()
            })
            .count();
        assert_eq!(more, PER_ADDRESS.0 as usize);
        // In total, across many addresses.
        let mut r = Responder::new(7);
        let total = (0..2000u32)
            .filter(|i| {
                let a = SocketAddr::from((Ipv4Addr::from(0x0A00_0000 + i), 1000));
                r.handle(a, &info_request(None), t, &info()).is_some()
            })
            .count();
        assert_eq!(total, TOTAL.1 as usize);
    }

    #[test]
    fn passwords_ride_in_user_data() {
        assert_eq!(password_of(Some(&user_data("hunter2"))), "hunter2");
        assert_eq!(password_of(Some(&user_data(""))), "");
        assert_eq!(password_of(None), "");
        assert_eq!(password_of(Some(&[0; 256])), "");
        let long = "x".repeat(400);
        assert_eq!(password_of(Some(&user_data(&long))).len(), 250);
    }

    #[test]
    fn lan_port_ranges() {
        let mut l = LanSettings::default();
        assert_eq!(l.port_range(), (27015, 27020));
        l.ports = "27031-27032".into();
        assert_eq!(l.port_range(), (27031, 27032));
        l.ports = "27040".into();
        assert_eq!(l.port_range(), (27040, 27040));
        l.ports = "9-2".into();
        assert_eq!(l.port_range(), LAN_PORTS);
        l.broadcast = "255.255.255.255 192.168.1.255 junk".into();
        assert_eq!(l.broadcast_addresses().len(), 2);
    }
}
