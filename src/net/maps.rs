//! Maps in a network game (docs/plans/active/multiplayer.md, slice 7;
//! docs/plans/active/custom-maps.md, "Distribution").
//!
//! - **The offer.** A server serves the map it has loaded (`ServedMap`:
//!   its id, the file's SHA-256 and size, the file itself to send); a
//!   client learns it in `Welcome`, and in `ChangeLevel` when the server
//!   moves to another map (`changelevel`, `map`): `watch_map` notices the
//!   new map once it is in, starts a fresh game for everyone there (scores
//!   and money anew, as CS:S's map change) and takes every client along
//!   (`ChangingLevel` first, while it loads, for the loading dialog).
//! - **The client's copy.** `Fetch` (driven by `client::check_map`): the
//!   loaded map if it is the same file; else the copy this process would
//!   load (`MapFiles::read`: the install, its downloads, the content
//!   cache) or the cache's own, compared by hash in the background; else a
//!   download: from `sv_downloadurl` over HTTP (`<url>/maps/<name>.bsp.bz2`,
//!   then `.bsp`), or over the connection (`MapRequest`, `MapChunk`s with a
//!   window of acknowledged bytes, `sv_allowdownload`, `net_maxfilesize`).
//!   What arrives is checked against the offer's hash and size, written to
//!   the content cache (`<cache>/maps/<name>.bsp`, noted in `index.toml`)
//!   and loaded from there (`NetEvent::LoadMap` with the file). Anything
//!   that fails disconnects with the reason, the game's words where it has
//!   them (`client::JoinFailure`).
//!
//! `net` can't reach the mounts, so the game, the dedicated server and
//! tests say how this process reads map files (`MapFiles`).

use std::{
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::Ordering,
    },
};

use bevy::{
    prelude::*,
    tasks::{AsyncComputeTaskPool, Task, block_on, poll_once},
};
use bevy_replicon::prelude::*;
use sha2::Digest;

use super::{
    ChangeLevel, ChangingLevel, GREYBOX, MapAck, MapChunk, MapDenied, MapRequest, NetEvent, Welcome,
    client::{Handshake, JoinFailure, JoinProgress, JoinStage},
    current_map,
    server::{CommandBuffer, Player},
};
use crate::{
    console::resource_cvar,
    core::{Intent, NetRole},
};

/// Bytes in each piece of a map sent over the connection.
pub const CHUNK: usize = 16 * 1024;
/// Bytes a server keeps in flight to a client beyond what it acknowledged.
pub const WINDOW: u64 = 256 * 1024;
/// Bytes a server queues per client per frame, at most.
const PER_FRAME: usize = 4 * CHUNK;

/// How this process reads map files: the game and the dedicated server
/// read through the game's mounts, tests from a folder of their own.
#[derive(Resource, Clone)]
pub struct MapFiles {
    /// The bytes of map `id`'s file as this process loads it (the
    /// install, its downloads, the content cache), None without one.
    pub read: Arc<dyn Fn(&str) -> Option<Vec<u8>> + Send + Sync>,
    /// The content cache (`content_dir`): downloads go to
    /// `<cache>/maps/<name>.bsp`. None: this process can't download.
    pub cache: Option<PathBuf>,
}

/// `sv_allowdownload`, `sv_downloadurl`, `net_maxfilesize` (Source's).
#[derive(Resource, Clone, Debug)]
pub struct DownloadSettings {
    /// 1: clients may download the map over the connection.
    pub allow: u8,
    /// An HTTP folder laid out like the game's (`maps/<name>.bsp.bz2`).
    pub url: String,
    /// Largest file sent over the connection, MB.
    pub max_mb: f32,
}

impl Default for DownloadSettings {
    fn default() -> Self {
        Self {
            allow: 1,
            url: String::new(),
            // CS:S's default is 16 MB; stock and community maps are often
            // bigger, and a client here has no other way without a URL.
            max_mb: 64.0,
        }
    }
}

/// On a server: the map it serves (what `Welcome` and `ChangeLevel` say)
/// and its file, for clients that download it.
#[derive(Resource, Clone, Debug, Default)]
pub struct ServedMap {
    pub map: String,
    pub hash: Option<[u8; 32]>,
    pub size: u64,
    pub bytes: Option<Arc<Vec<u8>>>,
    /// A map whose load clients were told of (`ChangingLevel`).
    announced: Option<String>,
}

/// On a client's server-side entity: the map file being sent to it.
#[derive(Component, Debug)]
pub struct Upload {
    bytes: Arc<Vec<u8>>,
    pub sent: u64,
    pub acked: u64,
}

pub(super) fn plugin(app: &mut App) {
    let server = || resource_equals(NetRole::Server);
    app.init_resource::<DownloadSettings>()
        .add_systems(Update, watch_map.run_if(server()))
        .add_systems(
            PreUpdate,
            (receive_requests, receive_acks)
                .after(ServerSystems::Receive)
                .run_if(in_state(ServerState::Running)),
        )
        .add_systems(PostUpdate, send_uploads.before(ServerSystems::Send).run_if(server()))
        .add_systems(
            PreUpdate,
            receive_chunks
                .after(ClientSystems::Receive)
                .run_if(resource_equals(NetRole::Client)),
        )
        .add_systems(
            PostUpdate,
            send_acks
                .before(ClientSystems::Send)
                .run_if(resource_equals(NetRole::Client)),
        );
    resource_cvar::<DownloadSettings, u8>(
        app,
        "sv_allowdownload",
        "1: clients without the map download it from the server.",
        |s| &mut s.allow,
    );
    resource_cvar::<DownloadSettings, String>(
        app,
        "sv_downloadurl",
        "An HTTP folder clients download the map from (<url>/maps/<name>.bsp.bz2 or .bsp); empty: none.",
        |s| &mut s.url,
    );
    resource_cvar::<DownloadSettings, f32>(
        app,
        "net_maxfilesize",
        "Largest map the server sends over the connection, MB.",
        |s| &mut s.max_mb,
    );
}

/// SHA-256 of a file's bytes (the handshake's map hash).
pub fn sha256(bytes: &[u8]) -> [u8; 32] {
    sha2::Sha256::digest(bytes).into()
}

/// A hash as hex (logs, the cache's index).
pub fn hex(hash: &[u8; 32]) -> String {
    hash.iter().map(|b| format!("{b:02x}")).collect()
}

/// A map id's name (`cs_source:de_dust2` -> `de_dust2`).
pub fn map_name(map: &str) -> &str {
    map.rsplit(':').next().unwrap_or(map)
}

/// The file a map is in, as the game names it (`maps/de_dust2.bsp`).
pub fn map_path(map: &str) -> String {
    format!("maps/{}.bsp", map_name(map))
}

/// A name safe to use as a file name in the cache.
fn safe_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 128
        && name != "."
        && name != ".."
        && name.chars().all(|c| c.is_ascii_alphanumeric() || "_-.+()[]".contains(c))
}

/// The loaded map (its id and file hash) once it is in; None while a load
/// is under way (the name set, the file not spawned yet).
pub fn loaded_map(world: &World) -> Option<(String, Option<[u8; 32]>)> {
    let id = current_map(world);
    if id == GREYBOX {
        return Some((id, None));
    }
    let file = world.get_resource::<crate::map::MapFile>()?;
    // The file's name with or without its game (`cs_source:de_dust2`).
    map_name(&file.name)
        .eq_ignore_ascii_case(map_name(&id))
        .then(|| (id.clone(), file.hash))
}

/// The map to serve: `id` with its file from `MapFiles` (none for the
/// greybox, or without a provider).
fn serve(world: &World, id: &str, hash: Option<[u8; 32]>) -> ServedMap {
    let bytes = if id == GREYBOX {
        None
    } else {
        world
            .get_resource::<MapFiles>()
            .and_then(|f| (f.read)(id))
            .map(Arc::new)
    };
    // The file read must be the one loaded (the hash the map spawned with).
    let bytes = bytes.filter(|b| hash.is_none_or(|h| sha256(b) == h));
    let hash = hash.or_else(|| bytes.as_ref().map(|b| sha256(b)));
    ServedMap {
        map: id.to_string(),
        hash,
        size: bytes.as_ref().map_or(0, |b| b.len() as u64),
        bytes,
        announced: None,
    }
}

/// The served map: the loaded one (set now if it wasn't yet).
pub fn served(world: &mut World) -> ServedMap {
    if let Some(s) = world.get_resource::<ServedMap>() {
        return s.clone();
    }
    let (id, hash) = loaded_map(world).unwrap_or_else(|| (current_map(world), None));
    let s = serve(world, &id, hash);
    world.insert_resource(s.clone());
    s
}

/// The welcome for a joining client: the served map and how to get it.
pub(super) fn welcome(world: &mut World, you: u64) -> Welcome {
    let s = served(world);
    let dl = world.resource::<DownloadSettings>().clone();
    let settings = world.resource::<super::NetSettings>().clone();
    let players = super::server::players(world).len()
        + world.query_filtered::<(), With<crate::bot::Bot>>().iter(world).count();
    Welcome {
        map: s.map,
        map_hash: s.hash,
        map_size: s.size,
        download_url: dl.url.trim().to_string(),
        allow_download: dl.allow != 0,
        tick_nanos: world.resource::<Time<Fixed>>().timestep().as_nanos() as u64,
        you,
        server_name: settings.hostname.clone(),
        players: players as u32,
        max_players: settings.maxplayers,
    }
}

/// Stop serving: nothing stays for the next time.
pub(super) fn stop_serving(world: &mut World) {
    world.remove_resource::<ServedMap>();
}

/// The server: a map load started (tell clients, once) or a new map is in
/// (a fresh game there, every client taken along).
fn watch_map(world: &mut World) {
    let Some((id, hash)) = loaded_map(world) else {
        let target = current_map(world);
        let tell = world
            .get_resource::<ServedMap>()
            .is_some_and(|s| s.map != target && s.announced.as_deref() != Some(target.as_str()));
        if tell {
            world.resource_mut::<ServedMap>().announced = Some(target.clone());
            info!("changing level to {target}");
            world.commands().server_trigger(ToClients {
                targets: SendTargets::All,
                message: ChangingLevel { map: target },
            });
        }
        return;
    };
    let (first, cancelled) = match world.get_resource::<ServedMap>() {
        // A change that never came (the load failed): clients told of it
        // go back to the map they're on (`ChangeLevel` with it).
        Some(s) if s.map == id && s.hash == hash && s.announced.is_some() => (false, true),
        Some(s) if s.map == id && s.hash == hash => return,
        Some(_) => (false, false),
        None => (true, false),
    };
    let s = if cancelled {
        let mut s = world.resource::<ServedMap>().clone();
        s.announced = None;
        s
    } else {
        serve(world, &id, hash)
    };
    world.insert_resource(s.clone());
    if first {
        return;
    }
    if cancelled {
        info!("the change of level was called off: still on {id}");
    } else {
        info!(
            "now serving {id} ({} bytes{})",
            s.size,
            s.hash.map_or(String::new(), |h| format!(", sha256 {}", hex(&h)))
        );
        new_level(world);
    }
    let dl = world.resource::<DownloadSettings>().clone();
    let tick_nanos = world.resource::<Time<Fixed>>().timestep().as_nanos() as u64;
    world.commands().server_trigger(ToClients {
        targets: SendTargets::All,
        message: ChangeLevel {
            map: s.map,
            map_hash: s.hash,
            map_size: s.size,
            download_url: dl.url.trim().to_string(),
            allow_download: dl.allow != 0,
            tick_nanos,
        },
    });
}

/// The server's side of a map change, after the new map spawned and the
/// rules started a new game (`rules::new_game`): remote players' queued
/// commands and intents dropped (their clients load the map and start
/// sending again), everyone's score back to nothing (CS:S's map change
/// is a new game), uploads of the old map stopped.
fn new_level(world: &mut World) {
    let players: Vec<Entity> = world
        .query_filtered::<Entity, With<CommandBuffer>>()
        .iter(world)
        .collect();
    for e in players {
        if let Some(mut b) = world.get_mut::<CommandBuffer>(e) {
            b.queued.clear();
            b.last = None;
            b.lead = None;
        }
        if let Some(mut i) = world.get_mut::<Intent>(e) {
            let (yaw, pitch) = (i.yaw, i.pitch);
            *i = Intent {
                yaw,
                pitch,
                ..default()
            };
        }
    }
    let scored: Vec<Entity> = world
        .query_filtered::<Entity, With<crate::rules::Score>>()
        .iter(world)
        .collect();
    for e in scored {
        world.entity_mut(e).remove::<crate::rules::Score>();
    }
    let uploads: Vec<Entity> = world.query_filtered::<Entity, With<Upload>>().iter(world).collect();
    for e in uploads {
        world.entity_mut(e).remove::<Upload>();
    }
}

/// Clients asking for the map: sent if allowed, else refused with why.
fn receive_requests(
    mut requests: MessageReader<FromClient<MapRequest>>,
    served: Option<Res<ServedMap>>,
    settings: Res<DownloadSettings>,
    players: Query<&Player>,
    mut denied: MessageWriter<ToClients<MapDenied>>,
    mut commands: Commands,
) {
    for r in requests.read() {
        let Some(client) = r.client_id.entity() else { continue };
        if players.get(client).is_err() {
            continue;
        }
        let file = map_path(&r.message.map);
        let refuse = match served.as_deref() {
            _ if settings.allow == 0 => Some(format!(
                "The server doesn't send maps (sv_allowdownload 0): you need {file}."
            )),
            None => Some("The server has no map yet.".to_string()),
            Some(s) if s.map != r.message.map => Some(format!("The server is on {} now.", s.map)),
            Some(s) if s.bytes.is_none() => Some(format!("The server has no file to send for {}.", s.map)),
            Some(s) if s.size as f64 > settings.max_mb as f64 * 1024.0 * 1024.0 => Some(format!(
                "{file} is {:.1} MB, more than the server sends (net_maxfilesize {}).",
                s.size as f64 / (1024.0 * 1024.0),
                settings.max_mb
            )),
            Some(_) => None,
        };
        if let Some(reason) = refuse {
            info!("map download refused: {reason}");
            denied.write(ToClients {
                targets: SendTargets::Single(ClientId::Client(client)),
                message: MapDenied { reason },
            });
            continue;
        }
        let s = served.as_deref().expect("checked");
        info!("sending {file} ({} bytes)", s.size);
        commands.entity(client).insert(Upload {
            bytes: s.bytes.clone().expect("checked"),
            sent: 0,
            acked: 0,
        });
    }
}

fn receive_acks(mut acks: MessageReader<FromClient<MapAck>>, mut uploads: Query<&mut Upload>) {
    for a in acks.read() {
        let Some(client) = a.client_id.entity() else { continue };
        if let Ok(mut u) = uploads.get_mut(client) {
            u.acked = u.acked.max(a.message.received.min(u.sent));
        }
    }
}

/// Map pieces to each downloading client, a window ahead of its acks.
fn send_uploads(
    mut uploads: Query<(Entity, &mut Upload)>,
    mut out: MessageWriter<ToClients<MapChunk>>,
    mut commands: Commands,
) {
    for (client, mut u) in &mut uploads {
        let total = u.bytes.len() as u64;
        let mut budget = PER_FRAME;
        while u.sent < total && u.sent - u.acked < WINDOW && budget > 0 {
            let start = u.sent as usize;
            let end = (start + CHUNK).min(total as usize);
            out.write(ToClients {
                targets: SendTargets::Single(ClientId::Client(client)),
                message: MapChunk {
                    offset: u.sent,
                    total,
                    data: u.bytes[start..end].to_vec(),
                },
            });
            u.sent = end as u64;
            budget = budget.saturating_sub(end - start);
        }
        if u.sent >= total && u.acked >= total {
            commands.entity(client).remove::<Upload>();
        }
    }
}

// --- The client.

/// Where a client is in getting the server's map (`Handshake::fetch`).
#[derive(Default)]
pub enum Fetch {
    /// Nothing done yet.
    #[default]
    Start,
    /// Comparing the copies here with the offer (in the background).
    Checking(Task<LocalCopy>),
    /// Downloading over HTTP (a thread).
    Http(HttpFetch),
    /// Downloading over the connection.
    Server { buf: Vec<u8>, acked: u64 },
    /// Checking and writing what arrived (in the background).
    Saving(Task<Result<PathBuf, String>>),
    /// Asked the game to load it (`NetEvent::LoadMap`).
    Loading,
}

impl std::fmt::Debug for Fetch {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Fetch::Start => "start",
            Fetch::Checking(_) => "checking",
            Fetch::Http(_) => "http",
            Fetch::Server { .. } => "server",
            Fetch::Saving(_) => "saving",
            Fetch::Loading => "loading",
        })
    }
}

/// An HTTP download running on its own thread.
pub struct HttpFetch {
    pub progress: Arc<super::http::Progress>,
    result: Arc<Mutex<Option<Result<(Vec<u8>, String), (super::http::HttpError, String)>>>>,
}

impl Drop for HttpFetch {
    fn drop(&mut self) {
        // Left (cancelled, disconnected): the thread stops at its next read.
        self.progress.cancel.store(true, Ordering::Relaxed);
    }
}

/// What the client has of the offered map.
#[derive(Debug, Clone, PartialEq)]
pub enum LocalCopy {
    /// The same file: load it by name (None) or from this file.
    Found(Option<PathBuf>),
    /// A file by that name, another version.
    Differs,
    Missing,
}

/// Compare the copies of `map` here with `hash`: the one this process
/// loads by name, then the cache's.
pub fn check_local(files: &MapFiles, map: &str, hash: [u8; 32]) -> LocalCopy {
    let mut seen = false;
    if let Some(bytes) = (files.read)(map) {
        seen = true;
        if sha256(&bytes) == hash {
            return LocalCopy::Found(None);
        }
    }
    if let Some(cache) = &files.cache {
        let path = cache_file(cache, map);
        if let Ok(bytes) = std::fs::read(&path) {
            seen = true;
            if sha256(&bytes) == hash {
                return LocalCopy::Found(Some(path));
            }
        }
    }
    if seen { LocalCopy::Differs } else { LocalCopy::Missing }
}

/// Where the cache keeps `map`.
pub fn cache_file(cache: &Path, map: &str) -> PathBuf {
    cache.join("maps").join(format!("{}.bsp", map_name(map)))
}

/// Check a downloaded file against the offer and put it in the cache
/// (written aside, then moved in place, and noted in `index.toml`).
pub fn save_download(cache: &Path, map: &str, hash: [u8; 32], size: u64, bytes: &[u8], from: &str) -> Result<PathBuf, String> {
    let file = map_path(map);
    if size != 0 && bytes.len() as u64 != size {
        return Err(format!(
            "The downloaded {file} is {} bytes, not the server's {size}: refused.",
            bytes.len()
        ));
    }
    let got = sha256(bytes);
    if got != hash {
        return Err(format!(
            "The downloaded {file} differs from the server's (sha256 {} instead of {}): refused.",
            &hex(&got)[..16],
            &hex(&hash)[..16]
        ));
    }
    if !safe_name(map_name(map)) {
        return Err(format!("{file}: not a name to save"));
    }
    let target = cache_file(cache, map);
    let dir = target.parent().expect("has a folder");
    std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let part = target.with_extension("bsp.part");
    std::fs::write(&part, bytes).map_err(|e| format!("{}: {e}", part.display()))?;
    std::fs::rename(&part, &target).map_err(|e| format!("{}: {e}", target.display()))?;
    let entry = format!(
        "[[map]]\nname = {:?}\nsize = {}\nsha256 = \"{}\"\nfrom = {:?}\n\n",
        map_name(map),
        bytes.len(),
        hex(&hash),
        from
    );
    use std::io::Write;
    let _ = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(cache.join("index.toml"))
        .and_then(|mut f| f.write_all(entry.as_bytes()));
    Ok(target)
}

/// Download `<url>/maps/<name>.bsp.bz2` (unpacked) or else `.bsp`, at most
/// `size` bytes unpacked, on a thread.
fn start_http(url: &str, map: &str, size: u64) -> HttpFetch {
    let progress = Arc::new(super::http::Progress::default());
    let result = Arc::new(Mutex::new(None));
    let base = url.trim().trim_end_matches('/').to_string();
    let name = map_name(map).to_string();
    let (p, r) = (progress.clone(), result.clone());
    let max = size.max(1) + 1024 * 1024;
    std::thread::spawn(move || {
        let bz = format!("{base}/maps/{name}.bsp.bz2");
        let out = match super::http::get(&bz, max, &p) {
            Ok(packed) => {
                use std::io::Read;
                let mut out = Vec::new();
                match bzip2::read::BzDecoder::new(&packed[..]).take(max).read_to_end(&mut out) {
                    Ok(_) => Ok((out, bz)),
                    Err(e) => Err((super::http::HttpError::Other(format!("not a bzip2 file ({e})")), bz)),
                }
            }
            Err(super::http::HttpError::NotFound) => {
                let plain = format!("{base}/maps/{name}.bsp");
                p.done.store(0, Ordering::Relaxed);
                p.total.store(0, Ordering::Relaxed);
                super::http::get(&plain, max, &p).map(|b| (b, plain.clone())).map_err(|e| (e, plain))
            }
            Err(e) => Err((e, bz)),
        };
        *r.lock().unwrap_or_else(|e| e.into_inner()) = Some(out);
    });
    HttpFetch { progress, result }
}

/// Drive the client's fetch of the server's map one step (from
/// `client::check_map`).
pub(super) fn step(world: &mut World) {
    let Some(h) = world.get_resource::<Handshake>() else { return };
    let welcome = h.welcome.clone();
    let map = welcome.map.clone();
    let fetch = std::mem::take(&mut world.resource_mut::<Handshake>().fetch);
    let next = match fetch {
        Fetch::Start => start(world, &welcome),
        Fetch::Checking(mut task) => match block_on(poll_once(&mut task)) {
            None => Fetch::Checking(task),
            Some(LocalCopy::Found(file)) => load(world, &map, file),
            Some(copy) => download(world, &welcome, copy),
        },
        Fetch::Http(http) => {
            show_download(world, &map, "HTTP", &http.progress);
            let done = http.result.lock().unwrap_or_else(|e| e.into_inner()).take();
            match done {
                None => Fetch::Http(http),
                Some(Ok((bytes, from))) => {
                    // Wrong bytes from the web: the server's own copy, if it sends it.
                    let (hash, size) = (welcome.map_hash.unwrap_or_default(), welcome.map_size);
                    if sha256(&bytes) != hash && welcome.allow_download {
                        warn!("{from} isn't the server's {}: asking the server", map_path(&map));
                        request(world, &map)
                    } else {
                        save(world, &map, hash, size, bytes, from)
                    }
                }
                Some(Err((e, from))) => {
                    warn!("downloading {from}: {e}");
                    if welcome.allow_download {
                        request(world, &map)
                    } else {
                        let file = from.rsplit('/').next().unwrap_or(&from).to_string();
                        super::client::fail(
                            world,
                            JoinFailure::Download {
                                file: format!("maps/{file}"),
                                error: e.to_string(),
                            },
                            &format!("Could not download maps/{file}:\n{e}"),
                        );
                        return;
                    }
                }
            }
        }
        Fetch::Server { buf, acked } => {
            let total = welcome.map_size;
            if let Some(p) = world.get_resource_mut::<JoinProgress>().as_deref_mut()
                && let Some(d) = &mut p.download
            {
                d.done = buf.len() as u64;
                d.total = total;
            }
            if total > 0 && buf.len() as u64 >= total {
                let hash = welcome.map_hash.unwrap_or_default();
                let from = format!("the server ({})", world.get_resource::<super::client::ServerAddress>().map_or("local".into(), |a| a.0.to_string()));
                save(world, &map, hash, total, buf, from)
            } else {
                Fetch::Server { buf, acked }
            }
        }
        Fetch::Saving(mut task) => match block_on(poll_once(&mut task)) {
            None => Fetch::Saving(task),
            Some(Ok(path)) => {
                info!("saved {} to {}", map_path(&map), path.display());
                load(world, &map, Some(path))
            }
            Some(Err(e)) => {
                super::client::fail(world, JoinFailure::MapDiffers, &e);
                return;
            }
        },
        Fetch::Loading => Fetch::Loading,
    };
    if let Some(mut h) = world.get_resource_mut::<Handshake>() {
        h.fetch = next;
    }
}

/// The first step: the loaded map if it's the server's; the copies here
/// checked in the background; without a way to read files, load it by
/// name and let the hash decide.
fn start(world: &mut World, welcome: &Welcome) -> Fetch {
    let map = welcome.map.clone();
    match super::map_matches(world, &map, welcome.map_hash) {
        // Already here: nothing to load (`client::check_map` joins).
        Some(true) => {
            super::client::set_stage(world, JoinStage::LoadingMap);
            return Fetch::Loading;
        }
        Some(false) if world.get_resource::<MapFiles>().is_none() => {
            super::client::fail(world, JoinFailure::MapDiffers, &differs(&map));
            return Fetch::Start;
        }
        _ => {}
    }
    let (Some(files), Some(hash)) = (world.get_resource::<MapFiles>().cloned(), welcome.map_hash) else {
        // The greybox, or no files to compare: the game loads it by name.
        return load(world, &map, None);
    };
    super::client::set_stage(world, JoinStage::Verifying);
    let task = AsyncComputeTaskPool::get().spawn(async move { check_local(&files, &map, hash) });
    Fetch::Checking(task)
}

/// CS:S's words for another version of the map.
pub fn differs(map: &str) -> String {
    format!("Your map [{}] differs from the server's.", map_path(map))
}

/// Ask the game to load the map (from `file`), and wait for it.
fn load(world: &mut World, map: &str, file: Option<PathBuf>) -> Fetch {
    super::client::set_stage(world, JoinStage::LoadingMap);
    world.write_message(NetEvent::LoadMap {
        map: map.to_string(),
        file,
    });
    Fetch::Loading
}

/// No copy here (or another version): download it, from the web first.
fn download(world: &mut World, welcome: &Welcome, copy: LocalCopy) -> Fetch {
    let map = welcome.map.clone();
    let file = map_path(&map);
    let can_save = world
        .get_resource::<MapFiles>()
        .is_some_and(|f| f.cache.is_some());
    if !can_save || (welcome.download_url.is_empty() && !welcome.allow_download) {
        let (kind, reason) = match copy {
            LocalCopy::Differs => (JoinFailure::MapDiffers, differs(&map)),
            // CS:S's words.
            _ => (JoinFailure::MapMissing, format!("Missing map {file}, disconnecting")),
        };
        super::client::fail(world, kind, &reason);
        return Fetch::Start;
    }
    if !welcome.download_url.is_empty() {
        super::client::set_stage(world, JoinStage::Downloading);
        let http = start_http(&welcome.download_url, &map, welcome.map_size);
        show_download(world, &map, "HTTP", &http.progress);
        Fetch::Http(http)
    } else {
        request(world, &map)
    }
}

/// Ask the server for the file.
fn request(world: &mut World, map: &str) -> Fetch {
    super::client::set_stage(world, JoinStage::Downloading);
    let total = world.get_resource::<Handshake>().map_or(0, |h| h.welcome.map_size);
    if let Some(mut p) = world.get_resource_mut::<JoinProgress>() {
        p.download = Some(super::client::DownloadInfo {
            file: map_path(map),
            from: "server".into(),
            done: 0,
            total,
        });
    }
    world.write_message(MapRequest { map: map.to_string() });
    Fetch::Server {
        buf: Vec::with_capacity(total.min(512 << 20) as usize),
        acked: 0,
    }
}

fn show_download(world: &mut World, map: &str, from: &str, progress: &super::http::Progress) {
    if let Some(mut p) = world.get_resource_mut::<JoinProgress>() {
        let total = progress.total.load(Ordering::Relaxed);
        let done = progress.done.load(Ordering::Relaxed);
        p.download = Some(super::client::DownloadInfo {
            file: format!("{}.bz2", map_path(map)),
            from: from.into(),
            done,
            total,
        });
    }
}

/// Check and cache what arrived (in the background).
fn save(world: &mut World, map: &str, hash: [u8; 32], size: u64, bytes: Vec<u8>, from: String) -> Fetch {
    let Some(cache) = world.get_resource::<MapFiles>().and_then(|f| f.cache.clone()) else {
        super::client::fail(world, JoinFailure::MapMissing, "no content cache to save the map in");
        return Fetch::Start;
    };
    let map = map.to_string();
    let task = AsyncComputeTaskPool::get().spawn(async move { save_download(&cache, &map, hash, size, &bytes, &from) });
    Fetch::Saving(task)
}

/// The server's pieces of the map, in order; a refusal ends the join.
fn receive_chunks(world: &mut World) {
    let chunks: Vec<MapChunk> = world.resource_mut::<Messages<MapChunk>>().drain().collect();
    let denied: Vec<MapDenied> = world.resource_mut::<Messages<MapDenied>>().drain().collect();
    if let Some(d) = denied.into_iter().next() {
        let downloading = world
            .get_resource::<Handshake>()
            .is_some_and(|h| matches!(h.fetch, Fetch::Server { .. }));
        if downloading {
            let file = world
                .get_resource::<Handshake>()
                .map(|h| map_path(&h.welcome.map))
                .unwrap_or_default();
            super::client::fail(
                world,
                JoinFailure::Download {
                    file,
                    error: d.reason.clone(),
                },
                &d.reason,
            );
            return;
        }
    }
    let Some(mut h) = world.get_resource_mut::<Handshake>() else { return };
    let Fetch::Server { buf, .. } = &mut h.fetch else { return };
    for c in chunks {
        if c.offset == buf.len() as u64 {
            buf.extend_from_slice(&c.data);
        }
    }
}

/// How much of the map arrived, to the server (each frame it grew).
fn send_acks(handshake: Option<ResMut<Handshake>>, mut out: MessageWriter<MapAck>) {
    let Some(mut h) = handshake else { return };
    if let Fetch::Server { buf, acked } = &mut h.fetch
        && buf.len() as u64 > *acked
    {
        *acked = buf.len() as u64;
        out.write(MapAck { received: *acked });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_and_paths() {
        assert_eq!(map_name("cs_source:de_dust2"), "de_dust2");
        assert_eq!(map_path("cs_source:de_dust2"), "maps/de_dust2.bsp");
        assert!(safe_name("mg_swag_multigames_v1"));
        assert!(!safe_name("../x") && !safe_name("a/b") && !safe_name(".."));
    }

    #[test]
    fn downloads_are_checked_before_they_are_kept() {
        let dir = std::env::temp_dir().join(format!("mashup-net-save-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let bytes = b"VBSP not really a map".to_vec();
        let hash = sha256(&bytes);
        assert!(save_download(&dir, "test:m", [0; 32], 0, &bytes, "x").is_err());
        assert!(save_download(&dir, "test:m", hash, 3, &bytes, "x").is_err());
        assert!(!cache_file(&dir, "test:m").exists(), "nothing kept from a bad download");
        let path = save_download(&dir, "test:m", hash, bytes.len() as u64, &bytes, "x").unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), bytes);
        let files = MapFiles {
            read: Arc::new(|_| None),
            cache: Some(dir.clone()),
        };
        assert_eq!(check_local(&files, "test:m", hash), LocalCopy::Found(Some(path)));
        assert_eq!(check_local(&files, "test:m", [1; 32]), LocalCopy::Differs);
        assert_eq!(check_local(&files, "test:other", hash), LocalCopy::Missing);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
