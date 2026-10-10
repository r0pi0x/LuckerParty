//! The client side: connecting, the join handshake, following the
//! server's map changes, characters the server sends (ours marked
//! `LocalPlayer`). Commands and prediction: `predict`; getting the map:
//! `maps`. How far joining has come (for the loading dialog):
//! `JoinProgress`.

use std::{
    net::{Ipv4Addr, SocketAddr, UdpSocket},
    time::SystemTime,
};

use bevy::prelude::*;
use bevy_replicon::prelude::*;
use bevy_replicon_renet::{RenetClient, netcode::ClientAuthentication};

use super::{
    ChangeLevel, ChangingLevel, Join, NetBody, NetCharacter, NetEvent, NetSettings, NetVersion, PROTOCOL_ID, Refused,
    Welcome, body_flags,
};
use crate::{
    character::character_bundle,
    core::{Intent, LocalPlayer, MovementState, NetRole, Team, Velocity},
    map::interp::NetDrawn,
    rules::Dead,
    weapon::Inventory,
};

pub(super) fn plugin(app: &mut App) {
    app.init_resource::<JoinProgress>()
        .add_observer(refused)
        .add_observer(welcome)
        .add_observer(changing_level)
        .add_observer(change_level)
        .add_observer(character_arrived)
        .add_systems(OnEnter(ClientState::Connected), send_join)
        .add_systems(
            PreUpdate,
            (watch_connection, apply_bodies)
                .chain()
                .after(ClientSystems::Receive)
                .run_if(resource_equals(NetRole::Client)),
        )
        .add_systems(Update, watch_join.run_if(resource_equals(NetRole::Client)))
        .add_systems(Update, check_map.run_if(resource_exists::<Handshake>));
}

/// This client's id (its characters' `NetCharacter::owner`).
#[derive(Resource, Clone, Copy, Debug, PartialEq, Eq)]
pub struct LocalClientId(pub u64);

/// The server this client connects to (for `status`).
#[derive(Resource, Clone, Copy, Debug)]
pub struct ServerAddress(pub SocketAddr);

/// The server's welcome (or its map change), until its map is in here:
/// how far getting it has come (`maps::Fetch`).
#[derive(Resource, Debug)]
pub struct Handshake {
    pub welcome: Welcome,
    pub fetch: super::maps::Fetch,
    /// A map change: load the map even if it's the one here (the server
    /// loaded it again: a fresh game, CS:S's `changelevel` to the same
    /// map).
    pub reload: bool,
    /// The map loads counted (`rules::MapLoads`) when we asked the game
    /// to load it: in once there has been one more.
    pub loads_before: Option<u32>,
}

/// In the game: the map matches the server's.
#[derive(Resource, Clone, Debug)]
pub struct Joined {
    pub map: String,
}

/// The server's welcome while connected (its name, the map's offer).
#[derive(Resource, Clone, Debug)]
pub struct ServerWelcome(pub Welcome);

/// Why the server refused us, to report when it drops us.
#[derive(Resource, Clone, Debug)]
struct RefusedReason(String);

/// Where joining a server (or following its map change) has come, for
/// the loading dialog and its detailed view. Kept after a disconnect,
/// with why (`failure`), until the dialog is closed (`dismiss`).
#[derive(Resource, Clone, Debug, Default)]
pub struct JoinProgress {
    pub stage: JoinStage,
    /// The server being joined.
    pub address: Option<SocketAddr>,
    /// What the server said of itself (`Welcome`).
    pub server: Option<ServerInfo>,
    /// The map being fetched or loaded.
    pub map: Option<String>,
    pub download: Option<DownloadInfo>,
    /// Each stage reached and when (real time, s), oldest first; the
    /// detailed view times them.
    pub log: Vec<(JoinStage, f64)>,
    /// Why the game ended before or after joining (not by the player's
    /// own `disconnect`), its kind (the dialog's wording) and the server's
    /// or our words.
    pub failure: Option<(JoinFailure, String)>,
}

impl JoinProgress {
    /// The dialog has something to show: joining, or why it failed.
    pub fn showing(&self) -> bool {
        !matches!(self.stage, JoinStage::Idle | JoinStage::Joined) || self.failure.is_some()
    }

    /// The dialog closed: nothing to show until the next join.
    pub fn dismiss(&mut self) {
        *self = Self::default();
    }

    /// When `stage` was last reached, real s.
    pub fn reached(&self, stage: JoinStage) -> Option<f64> {
        self.log.iter().rev().find(|(s, _)| *s == stage).map(|(_, t)| *t)
    }
}

/// A stage of joining (Source's loading progress names: `token`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum JoinStage {
    #[default]
    Idle,
    /// Connecting (netcode's handshake).
    Connecting,
    /// Connected; waiting for the server's welcome.
    ServerInfo,
    /// The server started a map change.
    ChangingLevel,
    /// Comparing the map files here with the server's.
    Verifying,
    Downloading,
    /// The game loads the map (`map::loading` has its stages).
    LoadingMap,
    Joined,
}

impl JoinStage {
    /// The game's localization token for the stage, and its English.
    pub fn token(self) -> (&'static str, &'static str) {
        match self {
            JoinStage::Idle | JoinStage::Connecting => ("LoadingProgress_Connecting", "Connecting to server..."),
            JoinStage::ServerInfo => ("LoadingProgress_ProcessServerInfo", "Retrieving server info..."),
            JoinStage::ChangingLevel => ("LoadingProgress_Changelevel", "Server is changing level..."),
            JoinStage::Verifying | JoinStage::Downloading => (
                "GameUI_VerifyingAndDownloading",
                "Verifying and downloading resources...",
            ),
            JoinStage::LoadingMap => ("LoadingProgress_LoadMap", "Loading world..."),
            JoinStage::Joined => ("LoadingProgress_SignonData", "Retrieving game data..."),
        }
    }

    /// Our own name for the detailed view.
    pub fn name(self) -> &'static str {
        match self {
            JoinStage::Idle => "idle",
            JoinStage::Connecting => "connecting",
            JoinStage::ServerInfo => "server info",
            JoinStage::ChangingLevel => "server changing level",
            JoinStage::Verifying => "checking the map",
            JoinStage::Downloading => "downloading the map",
            JoinStage::LoadingMap => "loading the map",
            JoinStage::Joined => "in the game",
        }
    }
}

/// What the server said of itself.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ServerInfo {
    pub name: String,
    pub map: String,
    pub players: u32,
    pub max_players: u32,
}

/// A map download under way.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct DownloadInfo {
    /// As the game names it (`maps/x.bsp.bz2`).
    pub file: String,
    /// "server" or "HTTP".
    pub from: String,
    pub done: u64,
    /// 0 while unknown.
    pub total: u64,
}

/// Why joining (or the game) ended, for the dialog's wording (the game's
/// own messages where it has one).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum JoinFailure {
    /// The server didn't answer, or stopped answering.
    Timeout,
    Full,
    /// The server runs an older or a newer build.
    OldServer,
    NewServer,
    /// Another network protocol (same version string).
    Protocol,
    /// The map here is another version, and none could be fetched.
    MapDiffers,
    /// No copy of the map here, and the server sends none.
    MapMissing,
    /// A download failed (the file, why).
    Download { file: String, error: String },
    /// The server ended the game (shut down, kicked).
    Dropped,
    Other,
}

/// The reason a player gives when they leave (`disconnect`, Cancel): not
/// a failure to show.
pub const BY_USER: &str = "Disconnect by user.";

/// Seconds without a packet before either end of a UDP connection gives
/// up (Source's `cl_timeout` default).
pub const TIMEOUT_SECONDS: i32 = 30;

/// Connect to a server over UDP (netcode).
pub fn connect(world: &mut World, server: SocketAddr) -> Result<(), String> {
    let socket = UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0)).map_err(|e| format!("UDP socket: {e}"))?;
    let now = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_default();
    // A random nonzero id (0 is the listen server's host).
    let id = loop {
        let n = random_u64(now.as_nanos() as u64 ^ std::process::id() as u64);
        if n != super::HOST_ID {
            break n;
        }
    };
    // netcode's "unsecure" connection as `ClientAuthentication::Unsecure`
    // makes it (an unencrypted token), but with Source's 30 s timeout
    // (`cl_timeout`) instead of its fixed 15 s: a client putting a big map
    // in on a slow or busy machine went quiet for longer and was dropped
    // by both ends (the live soak, multiplayer.md "Soak"). The server
    // takes each client's timeout from its token.
    // The password (`password`) for a server that wants one.
    let user_data = super::query::user_data(&world.resource::<super::query::JoinPassword>().0);
    let token = renetcode::ConnectToken::generate(
        now,
        PROTOCOL_ID,
        300,
        id,
        TIMEOUT_SECONDS,
        vec![server],
        Some(&user_data),
        &[0; renetcode::NETCODE_KEY_BYTES],
    )
    .map_err(|e| e.to_string())?;
    let auth = ClientAuthentication::Secure { connect_token: token };
    let transport = super::udp::UdpClient::new(now, auth, socket).map_err(|e| e.to_string())?;
    start(world, id)?;
    world.insert_resource(transport);
    world.insert_resource(ServerAddress(server));
    world.resource_mut::<JoinProgress>().address = Some(server);
    world.write_message(NetEvent::Connecting(server));
    Ok(())
}

/// splitmix64 of a seed: a client id.
fn random_u64(seed: u64) -> u64 {
    let mut z = seed.wrapping_add(0x9E37_79B9_7F4A_7C15);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

/// Become a client with renet's `RenetClient` (the caller adds the
/// transport: netcode in `connect`, memory in tests). What this process
/// simulated on its own (its player, bots and their weapons) goes: the
/// server's characters replace them.
pub fn start(world: &mut World, id: u64) -> Result<(), String> {
    if super::active(world) {
        return Err("already in a network game (disconnect first)".into());
    }
    let local: Vec<Entity> = world
        .query_filtered::<Entity, (With<Intent>, Without<Remote>)>()
        .iter(world)
        .collect();
    for e in local {
        let weapons = world.get::<Inventory>(e).map(|i| i.weapons.clone()).unwrap_or_default();
        for w in weapons {
            if let Ok(w) = world.get_entity_mut(w) {
                w.despawn();
            }
        }
        if let Ok(e) = world.get_entity_mut(e) {
            e.despawn();
        }
    }
    let config = super::connection_config(world);
    world.insert_resource(RenetClient::new(config));
    world.insert_resource(LocalClientId(id));
    world.insert_resource(NetRole::Client);
    world.resource_mut::<JoinProgress>().dismiss();
    set_stage(world, JoinStage::Connecting);
    Ok(())
}

/// Leave: tell the server (netcode sends its disconnect packets), drop
/// what it sent.
pub(super) fn stop(world: &mut World) {
    if let Some(mut transport) = world.remove_resource::<super::udp::UdpClient>() {
        transport.disconnect();
    }
    if let Some(memory) = world.remove_resource::<super::memory::MemoryClient>() {
        memory.hang_up();
    }
    world.remove_resource::<RenetClient>();
    world.remove_resource::<LocalClientId>();
    world.remove_resource::<ServerAddress>();
    world.remove_resource::<Handshake>();
    world.remove_resource::<Joined>();
    world.remove_resource::<RefusedReason>();
    world.remove_resource::<ServerWelcome>();
    super::predict::reset(world);
    super::interp::reset(world);
    super::weapons::reset(world);
    super::cvars::restore(world);
    let remote: Vec<Entity> = world.query_filtered::<Entity, With<Remote>>().iter(world).collect();
    for e in remote {
        // The weapons built here for what it carries go with it.
        let weapons = world.get::<Inventory>(e).map(|i| i.weapons.clone()).unwrap_or_default();
        for w in weapons {
            if let Ok(w) = world.get_entity_mut(w) {
                w.despawn();
            }
        }
        if let Ok(e) = world.get_entity_mut(e) {
            e.despawn();
        }
    }
}

/// Connected: say who we are.
fn send_join(world: &mut World) {
    if *world.resource::<NetRole>() != NetRole::Client {
        return;
    }
    let join = Join {
        version: world.resource::<NetVersion>().0.clone(),
        protocol: *world.resource::<ProtocolHash>(),
        name: world.resource::<NetSettings>().name.clone(),
        userinfo: crate::console::userinfo(world).into_iter().collect(),
    };
    world.commands().client_trigger(join);
    world.flush();
}

fn refused(r: On<Refused>, mut commands: Commands) {
    warn!("the server refused us: {}", r.reason);
    commands.insert_resource(RefusedReason(r.reason.clone()));
}

fn welcome(w: On<Welcome>, mut commands: Commands) {
    info!(
        "server \"{}\": map {} ({} bytes), tick {:.4} s, {}/{} players",
        w.server_name,
        w.map,
        w.map_size,
        w.tick_nanos as f64 * 1e-9,
        w.players,
        w.max_players
    );
    let welcome = w.event().clone();
    commands.queue(move |world: &mut World| {
        {
            let mut p = world.resource_mut::<JoinProgress>();
            p.server = Some(ServerInfo {
                name: welcome.server_name.clone(),
                map: welcome.map.clone(),
                players: welcome.players,
                max_players: welcome.max_players,
            });
            p.map = Some(welcome.map.clone());
        }
        world.insert_resource(ServerWelcome(welcome.clone()));
        world.insert_resource(Handshake {
            welcome,
            fetch: default(),
            reload: false,
            loads_before: None,
        });
    });
}

/// The server started changing level: the loading dialog says so while
/// the game plays on until the new map comes (`change_level`).
fn changing_level(c: On<ChangingLevel>, mut commands: Commands) {
    let map = c.map.clone();
    commands.queue(move |world: &mut World| {
        if world.get_resource::<NetRole>() != Some(&NetRole::Client) || !world.contains_resource::<Joined>() {
            return;
        }
        world.resource_mut::<JoinProgress>().map = Some(map.clone());
        set_stage(world, JoinStage::ChangingLevel);
        world.write_message(NetEvent::ChangingLevel(map));
    });
}

/// The server is on a new map: stop predicting, get the map as when
/// joining (`maps`), play on there once it's in.
fn change_level(c: On<ChangeLevel>, mut commands: Commands) {
    let c = c.event().clone();
    commands.queue(move |world: &mut World| {
        if world.get_resource::<NetRole>() != Some(&NetRole::Client) {
            return;
        }
        info!("the server changed level to {}", c.map);
        let mut welcome = world
            .get_resource::<ServerWelcome>()
            .map(|w| w.0.clone())
            .unwrap_or_default();
        welcome.map = c.map.clone();
        welcome.map_hash = c.map_hash;
        welcome.map_size = c.map_size;
        welcome.download_url = c.download_url.clone();
        welcome.allow_download = c.allow_download;
        welcome.tick_nanos = c.tick_nanos;
        world.insert_resource(ServerWelcome(welcome.clone()));
        world.remove_resource::<Joined>();
        // What was predicted and drawn was on the old map.
        super::predict::reset(world);
        super::interp::reset(world);
        {
            let mut p = world.resource_mut::<JoinProgress>();
            p.map = Some(c.map.clone());
            p.download = None;
            if let Some(s) = &mut p.server {
                s.map = c.map.clone();
            }
        }
        if world.resource::<JoinProgress>().stage != JoinStage::ChangingLevel {
            set_stage(world, JoinStage::ChangingLevel);
            world.write_message(NetEvent::ChangingLevel(c.map.clone()));
        }
        world.insert_resource(Handshake {
            welcome,
            fetch: default(),
            reload: true,
            loads_before: None,
        });
    });
}

/// Move joining on to `stage` (logged with the time, for the detailed
/// view).
pub fn set_stage(world: &mut World, stage: JoinStage) {
    let now = world
        .get_resource::<Time<Real>>()
        .map_or(0.0, |t| t.elapsed_secs_f64());
    let mut p = world.resource_mut::<JoinProgress>();
    if p.stage != stage {
        p.stage = stage;
        p.log.push((stage, now));
    }
}

/// Joining failed (or the game ended) for `reason`: disconnect; the
/// dialog shows why, in the game's words for `kind`.
pub fn fail(world: &mut World, kind: JoinFailure, reason: &str) {
    world.insert_resource(FailureKind(kind));
    super::disconnect(world, reason);
}

/// The kind of the failure `fail` is reporting (read by `ended`).
#[derive(Resource)]
struct FailureKind(JoinFailure);

/// The game ended (`net::disconnect` on a client): the dialog keeps why,
/// unless the player left.
pub(super) fn ended(world: &mut World, reason: &str) {
    let kind = world.remove_resource::<FailureKind>().map(|k| k.0);
    let own = world.resource::<NetVersion>().0.clone();
    let mut p = world.resource_mut::<JoinProgress>();
    let joined = p.stage == JoinStage::Joined;
    p.stage = JoinStage::Idle;
    p.download = None;
    p.failure = if reason == BY_USER || reason == "Quit." {
        None
    } else {
        Some((kind.unwrap_or_else(|| classify(reason, &own, joined)), reason.to_string()))
    };
}

/// The kind of a disconnect's reason: the server's refusals (their
/// words, `server::admit`), netcode's timeouts and denials.
pub fn classify(reason: &str, own_version: &str, joined: bool) -> JoinFailure {
    if reason == "Server is full." || reason.contains("ConnectionDenied") {
        JoinFailure::Full
    } else if let Some(rest) = reason.strip_prefix("Your game is version ") {
        // "Your game is version A but the server's is B: ..."
        let server = rest
            .split("the server's is ")
            .nth(1)
            .and_then(|r| r.split(": ").next())
            .unwrap_or("");
        match compare_versions(own_version, server) {
            Some(std::cmp::Ordering::Greater) => JoinFailure::OldServer,
            Some(std::cmp::Ordering::Less) => JoinFailure::NewServer,
            _ => JoinFailure::Other,
        }
    } else if reason.contains("network protocol differs") {
        JoinFailure::Protocol
    } else if reason.contains("TimedOut") || reason.contains("timed out") {
        JoinFailure::Timeout
    } else if reason.contains("differs from the server's") {
        JoinFailure::MapDiffers
    } else if joined || reason.contains("DisconnectedByServer") || reason.contains("shutting down") {
        JoinFailure::Dropped
    } else {
        JoinFailure::Other
    }
}

/// Order two build versions (`0.1.0/net7`): the numbers in them, left to
/// right. None if either has none.
pub fn compare_versions(a: &str, b: &str) -> Option<std::cmp::Ordering> {
    let nums = |s: &str| -> Vec<u64> {
        s.split(|c: char| !c.is_ascii_digit())
            .filter(|p| !p.is_empty())
            .filter_map(|p| p.parse().ok())
            .collect()
    };
    let (x, y) = (nums(a), nums(b));
    (!x.is_empty() && !y.is_empty()).then(|| x.cmp(&y))
}

/// Connecting and waiting for the welcome take at most this long, s.
const JOIN_TIMEOUT: f64 = 20.0;

/// Joining's stages as the connection goes: connected (waiting for the
/// welcome); a server that never welcomes us timed out.
fn watch_join(world: &mut World) {
    let connected = world.get_resource::<RenetClient>().is_some_and(|c| c.is_connected());
    let stage = world.resource::<JoinProgress>().stage;
    if connected && stage == JoinStage::Connecting {
        set_stage(world, JoinStage::ServerInfo);
    }
    if matches!(stage, JoinStage::Connecting | JoinStage::ServerInfo)
        && !world.contains_resource::<Handshake>()
        && !world.contains_resource::<Joined>()
        && let Some(since) = world.resource::<JoinProgress>().reached(JoinStage::Connecting)
    {
        let now = world.resource::<Time<Real>>().elapsed_secs_f64();
        if now - since > JOIN_TIMEOUT {
            fail(world, JoinFailure::Timeout, "Connection to server timed out.");
        }
    }
}

/// Dropped (refused, kicked, timed out, the server gone): back to single
/// player with the reason.
fn watch_connection(world: &mut World) {
    // Refused: hang up at once with the server's reason (it drops us a
    // moment later anyway).
    if let Some(r) = world.get_resource::<RefusedReason>() {
        let reason = r.0.clone();
        super::disconnect(world, &reason);
        return;
    }
    let Some(client) = world.get_resource::<RenetClient>() else {
        return;
    };
    if !client.is_disconnected() {
        return;
    }
    let renet_reason = client.disconnect_reason().map(|r| r.to_string());
    let netcode_reason = world
        .get_resource::<super::udp::UdpClient>()
        .and_then(|t| t.disconnect_reason())
        .map(|r| format!("{r:?}"));
    let reason = netcode_reason
        .or(renet_reason)
        .unwrap_or_else(|| "connection lost".into());
    let reason = match reason.as_str() {
        // Netcode's names, in the game's words.
        "ConnectionRequestTimedOut" | "ConnectionResponseTimedOut" => "Connection to server timed out.".to_string(),
        "ConnectionTimedOut" => "Connection to server timed out (ConnectionTimedOut).".to_string(),
        "ConnectionDenied" => "Server is full.".to_string(),
        "DisconnectedByServer" => "The server ended the game (DisconnectedByServer).".to_string(),
        _ => reason,
    };
    super::disconnect(world, &reason);
}

/// Get the server's map (`maps::step`: the loaded one, a copy here, a
/// download) and, once the game has it loaded and it's the same file,
/// play: in the game. A different file by the same name: leave.
fn check_map(world: &mut World) {
    if !matches!(
        world.get_resource::<Handshake>().map(|h| &h.fetch),
        Some(super::maps::Fetch::Loading)
    ) {
        super::maps::step(world);
    }
    let Some(h) = world.get_resource::<Handshake>() else {
        return;
    };
    if !matches!(h.fetch, super::maps::Fetch::Loading) {
        return;
    }
    let (map, hash, tick_nanos) = (h.welcome.map.clone(), h.welcome.map_hash, h.welcome.tick_nanos);
    // Asked for a load: wait for it (the map here may be the one asked
    // for, loaded before).
    let loads = world.get_resource::<crate::rules::MapLoads>().map_or(0, |l| l.0);
    if h.loads_before.is_some_and(|before| loads <= before) {
        return;
    }
    match super::map_matches(world, &map, hash) {
        Some(true) => {
            world.insert_resource(Time::<Fixed>::from_duration(std::time::Duration::from_nanos(tick_nanos)));
            world.remove_resource::<Handshake>();
            world.insert_resource(Joined { map: map.clone() });
            // The server puts us in the game now.
            world.write_message(super::Loaded { map: map.clone() });
            set_stage(world, JoinStage::Joined);
            world.write_message(NetEvent::Joined { map });
        }
        Some(false) => fail(world, JoinFailure::MapDiffers, &super::maps::differs(&map)),
        None => {}
    }
}

/// A character from the server: the components the client draws it with
/// (no movement: the server moves it), ours marked `LocalPlayer`, others
/// drawn from snapshots (`NetDrawn`, `interp`).
fn character_arrived(
    add: On<Add, NetCharacter>,
    q: Query<(&NetCharacter, Option<&NetBody>, Option<&Team>)>,
    me: Option<Res<LocalClientId>>,
    role: Option<Res<NetRole>>,
    mut commands: Commands,
) {
    if role.as_deref() != Some(&NetRole::Client) {
        return;
    }
    let Ok((c, body, team)) = q.get(add.entity) else { return };
    let at = body.map_or(Vec3::ZERO, |b| Vec3::from_array(b.origin));
    let mut e = commands.entity(add.entity);
    e.insert_if_new(character_bundle(
        Transform::from_translation(at),
        team.copied().unwrap_or_default(),
    ))
    .insert(Name::new(c.name.clone()));
    if me.is_some_and(|me| c.owner == Some(me.0)) {
        e.insert(LocalPlayer);
    } else {
        e.insert(NetDrawn);
    }
}

/// Our own character where the server says it is until it is predicted
/// (others are drawn from snapshots: `interp::draw_others`).
#[allow(clippy::type_complexity)]
fn apply_bodies(
    mut q: Query<
        (
            Entity,
            Ref<NetBody>,
            &mut Transform,
            &mut Velocity,
            &mut MovementState,
            Has<Dead>,
            Has<crate::slots::MovementSlot>,
        ),
        (With<Remote>, Without<NetDrawn>),
    >,
    time: Res<Time<Fixed>>,
    mut commands: Commands,
) {
    for (e, body, mut t, mut v, mut s, dead, predicted) in &mut q {
        let dead_now = body.flags & body_flags::DEAD != 0;
        if dead_now != dead {
            if dead_now {
                commands.entity(e).insert(Dead {
                    since: time.elapsed_secs_f64(),
                });
            } else {
                commands.entity(e).remove::<Dead>();
            }
        }
        // Our own player, once predicted, moves by our commands
        // (`predict`): the server's state of it comes as `OwnState`.
        if !body.is_changed() || predicted {
            continue;
        }
        let origin = Vec3::from_array(body.origin);
        if t.translation != origin {
            t.translation = origin;
        }
        v.0 = Vec3::from_array(body.velocity);
        let on = |bit| body.flags & bit != 0;
        s.on_ground = on(body_flags::ON_GROUND);
        s.crouching = on(body_flags::CROUCHING);
        s.on_ladder = on(body_flags::ON_LADDER);
        s.eye_offset = Vec3::from_array(body.eye);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn refusals_are_told_apart() {
        let own = "0.1.0/net7";
        assert_eq!(classify("Server is full.", own, false), JoinFailure::Full);
        let r = |v: &str| {
            format!("Your game is version {own} but the server's is {v}: update the one that's older.")
        };
        assert_eq!(classify(&r("0.1.0/net5"), own, false), JoinFailure::OldServer);
        assert_eq!(classify(&r("0.2.0/net7"), own, false), JoinFailure::NewServer);
        assert_eq!(classify("Connection to server timed out.", own, false), JoinFailure::Timeout);
        assert_eq!(
            classify("Your map [maps/x.bsp] differs from the server's.", own, false),
            JoinFailure::MapDiffers
        );
        assert_eq!(classify("whatever", own, true), JoinFailure::Dropped);
        assert_eq!(compare_versions("0.1.0/net7", "0.1.0/net10"), Some(std::cmp::Ordering::Less));
    }
}
