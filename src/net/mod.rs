//! Network play (docs/plans/active/multiplayer.md): an authoritative
//! server (a listen server in the game, or the dedicated `mashup_server`)
//! and clients, over bevy_replicon with renet's netcode UDP transport.
//!
//! Slice 1 (this much so far): connect and disconnect (`connect`,
//! `disconnect`, `listen`, `status`; `hostport`, `maxplayers`, `name`),
//! a join handshake (build version, protocol hash, then the map by name
//! and file hash), a character spawned on the server per client, and
//! characters replicated to every client (`NetCharacter`, `NetBody`,
//! `Team`, `Health`), drawn where the server last put them. A client
//! sends its latest `Intent` every frame (`NetIntent`) and the server
//! moves its character with it; no prediction or interpolation buffer yet
//! (slices 2 and 3).
//!
//! `NetRole` (core) says what this process is; `authoritative` systems
//! (rules, bots, damage, logic) don't run on a client.

pub mod client;
pub mod memory;
pub mod server;

use std::net::SocketAddr;

use bevy::{prelude::*, state::app::StatesPlugin};
use bevy_replicon::prelude::*;
use bevy_replicon_renet::{RenetChannelsExt, RenetClient, RenetServer, RepliconRenetPlugins, renet::ConnectionConfig};
use serde::{Deserialize, Serialize};

use crate::{
    console::{ConsoleAppExt, resource_cvar},
    core::{Health, NetRole, Team},
};

/// Source's default port; the game and the dedicated server listen here
/// unless `hostport` says otherwise.
pub const DEFAULT_PORT: u16 = 27015;

/// The netcode protocol id: Lucker Party's, the same for every build, so a
/// client of another build still reaches the server and hears why it was
/// refused (`Join::version`) instead of timing out.
pub const PROTOCOL_ID: u64 = 0x4C55_434B_4552_5059;

/// This build's network version. A server refuses clients of another
/// version. Bump the suffix when the protocol changes in a way the
/// replicon protocol hash can't see (a field added to a message).
pub const NET_VERSION: &str = concat!(env!("CARGO_PKG_VERSION"), "/net1");

/// The owner id of the listen server's own player (`NetCharacter::owner`).
/// Remote clients' ids are never 0.
pub const HOST_ID: u64 = 0;

/// The name of the greybox map in the handshake (and `map greybox`).
pub const GREYBOX: &str = "greybox";

/// Network settings (console: `hostport`, `maxplayers`, `name`).
#[derive(Resource, Clone, Debug)]
pub struct NetSettings {
    /// UDP port a server listens on.
    pub hostport: u16,
    /// Players a server takes, the host's own included on a listen
    /// server (Source's `maxplayers`; 1 is single player).
    pub maxplayers: u32,
    /// This player's name (Source's `name`).
    pub name: String,
}

impl Default for NetSettings {
    fn default() -> Self {
        Self {
            hostport: DEFAULT_PORT,
            maxplayers: 1,
            name: "Player".into(),
        }
    }
}

/// The version this process says it is in `Join` (`NET_VERSION`; tests
/// set another to be refused).
#[derive(Resource, Clone, Debug)]
pub struct NetVersion(pub String);

impl Default for NetVersion {
    fn default() -> Self {
        Self(NET_VERSION.into())
    }
}

/// What happened on the network, for the client layer (menus, map loads)
/// and logs.
#[derive(Message, Clone, Debug, PartialEq)]
pub enum NetEvent {
    /// Listening on this address (`listen`, `map` with `maxplayers` > 1, or
    /// the dedicated server).
    Listening(SocketAddr),
    /// Connecting to this server.
    Connecting(SocketAddr),
    /// The server is on this map and this client hasn't loaded it: load it
    /// (the client layer runs `map`), then the handshake checks its hash.
    LoadMap(String),
    /// Joined: the map matches, the server spawns our character.
    Joined { map: String },
    /// Left the game (or stopped serving): why.
    Disconnected(String),
}

// --- Protocol: messages and replicated components. Registered in the same
// order on every side (`NetPlugin::build`).

/// Client -> server, first thing after connecting: who we are.
#[derive(Event, Serialize, Deserialize, Clone, Debug)]
pub struct Join {
    /// `NET_VERSION` of the client's build.
    pub version: String,
    /// Replicon's hash of everything registered (types and order).
    pub protocol: ProtocolHash,
    pub name: String,
}

/// Server -> client: refused to join (version, full); the server
/// disconnects it right after.
#[derive(Event, Serialize, Deserialize, Clone, Debug)]
pub struct Refused {
    pub reason: String,
}

/// Server -> client: joined. The map (`map` id: `greybox` or
/// `cs_source:<name>`) and its file hash, which the client checks before
/// it plays, and the tick the server runs at.
#[derive(Event, Serialize, Deserialize, Clone, Debug)]
pub struct Welcome {
    pub map: String,
    pub map_hash: Option<[u8; 32]>,
    /// Seconds per server tick.
    pub tick_interval: f64,
    /// The client's id (`NetCharacter::owner` of its character).
    pub you: u64,
}

/// Client -> server, every frame: the client's latest intent. Slice 2
/// replaces this with numbered user commands bound to server ticks.
#[derive(Message, Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
pub struct NetIntent {
    pub move_axis: [f32; 2],
    pub yaw: f32,
    pub pitch: f32,
    /// `buttons::*` bits.
    pub buttons: u16,
    pub select: Option<u8>,
}

/// `NetIntent::buttons` bits.
pub mod buttons {
    pub const JUMP: u16 = 1;
    pub const CROUCH: u16 = 1 << 1;
    pub const SPRINT: u16 = 1 << 2;
    pub const WALK: u16 = 1 << 3;
    pub const FIRE: u16 = 1 << 4;
    pub const SECONDARY: u16 = 1 << 5;
    pub const RELOAD: u16 = 1 << 6;
    pub const LAST_WEAPON: u16 = 1 << 7;
    pub const USE: u16 = 1 << 8;
}

impl NetIntent {
    pub fn of(i: &crate::core::Intent) -> Self {
        use buttons::*;
        let mut b = 0;
        for (on, bit) in [
            (i.jump, JUMP),
            (i.crouch, CROUCH),
            (i.sprint, SPRINT),
            (i.walk, WALK),
            (i.fire, FIRE),
            (i.secondary, SECONDARY),
            (i.reload, RELOAD),
            (i.last_weapon, LAST_WEAPON),
            (i.use_key, USE),
        ] {
            if on {
                b |= bit;
            }
        }
        Self {
            move_axis: i.move_axis.to_array(),
            yaw: i.yaw,
            pitch: i.pitch,
            buttons: b,
            select: i.select,
        }
    }

    /// Write into an intent, made safe first (never trust a client): the
    /// move axis at most 1 long, finite angles, pitch within straight up
    /// and down, a weapon slot key 0-9. The command number stays (the
    /// server stamps it).
    pub fn apply(&self, i: &mut crate::core::Intent) {
        use buttons::*;
        let finite = |v: f32| if v.is_finite() { v } else { 0.0 };
        let axis = Vec2::new(finite(self.move_axis[0]), finite(self.move_axis[1]));
        i.move_axis = axis.clamp_length_max(1.0);
        i.yaw = finite(self.yaw).rem_euclid(std::f32::consts::TAU);
        let half_pi = std::f32::consts::FRAC_PI_2;
        i.pitch = finite(self.pitch).clamp(-half_pi, half_pi);
        let on = |bit: u16| self.buttons & bit != 0;
        i.jump = on(JUMP);
        i.crouch = on(CROUCH);
        i.sprint = on(SPRINT);
        i.walk = on(WALK);
        i.fire = on(FIRE);
        i.secondary = on(SECONDARY);
        i.reload = on(RELOAD);
        i.last_weapon = on(LAST_WEAPON);
        i.use_key = on(USE);
        i.select = self.select.filter(|s| *s < 10);
    }
}

/// A character the server replicates: whose it is and its name. On a
/// client, the character's local components are added when it arrives
/// (`client::character_arrived`).
#[derive(Component, Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct NetCharacter {
    /// The player's client id (`HOST_ID` for a listen server's host), or
    /// None for a bot.
    pub owner: Option<u64>,
    pub name: String,
}

/// What clients draw a character from, written by the server every tick
/// from the simulation (`server::write_bodies`): position, velocity, look,
/// eye height and movement flags. Slice 3 buffers these by server tick.
#[derive(Component, Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
pub struct NetBody {
    pub origin: [f32; 3],
    pub velocity: [f32; 3],
    pub yaw: f32,
    pub pitch: f32,
    pub eye: [f32; 3],
    /// `body_flags::*` bits.
    pub flags: u8,
}

/// `NetBody::flags` bits.
pub mod body_flags {
    pub const ON_GROUND: u8 = 1;
    pub const CROUCHING: u8 = 1 << 1;
    pub const ON_LADDER: u8 = 1 << 2;
    pub const DEAD: u8 = 1 << 3;
}

/// The network's systems, protocol and console commands. With
/// `RepliconPlugins` and renet's; nothing runs until `listen` or
/// `connect` (single player stays `NetRole::Standalone`).
pub struct NetPlugin;

impl Plugin for NetPlugin {
    fn build(&self, app: &mut App) {
        if !app.is_plugin_added::<StatesPlugin>() {
            app.add_plugins(StatesPlugin);
        }
        app.add_plugins((
            RepliconPlugins.set(RepliconSharedPlugin {
                // Our own handshake (`Join`): a version message before the
                // protocol hash.
                auth_method: AuthMethod::Custom,
            }),
            RepliconRenetPlugins,
        ))
        // The protocol, in the same order everywhere. `Join` and
        // `Refused` come first so another build can still read them.
        .add_client_event::<Join>(Channel::Ordered)
        .add_server_event::<Refused>(Channel::Ordered)
        .make_event_independent::<Refused>()
        .add_server_event::<Welcome>(Channel::Ordered)
        .make_event_independent::<Welcome>()
        .add_client_message::<NetIntent>(Channel::Unreliable)
        .replicate::<NetCharacter>()
        .replicate::<NetBody>()
        .replicate::<Team>()
        .replicate::<Health>()
        .init_resource::<NetSettings>()
        .init_resource::<NetVersion>()
        .add_message::<NetEvent>()
        .add_systems(Update, log_events);
        app.world_mut().resource_mut::<ProtocolHasher>().add_custom(NET_VERSION);
        server::plugin(app);
        client::plugin(app);
        memory::plugin(app);
        commands(app);
    }
}

/// Network events in the log (and so the game's console).
fn log_events(mut events: MessageReader<NetEvent>) {
    for e in events.read() {
        match e {
            NetEvent::Listening(a) => info!("listening on UDP {a}"),
            NetEvent::Connecting(a) => info!("connecting to {a}..."),
            NetEvent::LoadMap(m) => info!("the server is on {m}: loading it"),
            NetEvent::Joined { map } => info!("joined the game on {map}"),
            NetEvent::Disconnected(r) => info!("disconnected: {r}"),
        }
    }
}

/// Renet's connection settings for replicon's channels.
pub(crate) fn connection_config(world: &World) -> ConnectionConfig {
    let channels = world.resource::<RepliconChannels>();
    ConnectionConfig {
        server_channels_config: channels.server_configs(),
        client_channels_config: channels.client_configs(),
        ..default()
    }
}

/// The loaded map's id as the handshake names it: `greybox` or the
/// `game:name` the map was loaded as (`map::LoadedMapName`).
pub fn current_map(world: &World) -> String {
    let name = world
        .get_resource::<crate::map::LoadedMapName>()
        .map(|n| n.0.clone())
        .unwrap_or_default();
    normalize_map(&name)
}

/// `greybox` for the greybox's names (none, `greybox`, `mashup:greybox`);
/// other ids as they are.
pub fn normalize_map(name: &str) -> String {
    if name.is_empty() || name.eq_ignore_ascii_case(GREYBOX) || name.eq_ignore_ascii_case("mashup:greybox") {
        GREYBOX.into()
    } else {
        name.into()
    }
}

/// Whether the loaded map is `map` and its file hash is `hash`: Some(true)
/// or Some(false) once a map by that name is loaded, None while it isn't
/// (still loading, or another one).
pub fn map_matches(world: &World, map: &str, hash: Option<[u8; 32]>) -> Option<bool> {
    if map == GREYBOX {
        return (current_map(world) == GREYBOX).then_some(true);
    }
    let file = world.get_resource::<crate::map::MapFile>()?;
    let want_name = map.rsplit(':').next().unwrap_or(map);
    if !file.name.eq_ignore_ascii_case(want_name) || current_map(world) != map {
        return None;
    }
    Some(file.hash == hash)
}

/// Stop serving or leave the server, back to single player
/// (`NetRole::Standalone`): remote players' characters and everything
/// the server sent go. Reports `NetEvent::Disconnected(reason)`.
pub fn disconnect(world: &mut World, reason: &str) {
    let role = world.get_resource::<NetRole>().copied().unwrap_or_default();
    match role {
        NetRole::Standalone => return,
        NetRole::Server => server::stop(world),
        NetRole::Client => client::stop(world),
    }
    world.insert_resource(NetRole::Standalone);
    world.insert_resource(LastDisconnect(reason.into()));
    world.write_message(NetEvent::Disconnected(reason.into()));
}

/// Why the last network game ended (`disconnect`).
#[derive(Resource, Clone, Debug, PartialEq)]
pub struct LastDisconnect(pub String);

/// Whether a server or client transport is up.
pub fn active(world: &World) -> bool {
    world.contains_resource::<RenetServer>() || world.contains_resource::<RenetClient>()
}

/// Parse `ip[:port]` (or `host[:port]`), port `DEFAULT_PORT` when left out.
pub fn parse_address(text: &str) -> Result<SocketAddr, String> {
    use std::net::ToSocketAddrs;
    let text = text.trim();
    if text.is_empty() {
        return Err("connect <ip[:port]>".into());
    }
    // A bare IPv6 address, or one in brackets with a port.
    if let Ok(ip) = text.parse::<std::net::IpAddr>() {
        return Ok(SocketAddr::new(ip, DEFAULT_PORT));
    }
    if let Ok(a) = text.parse::<SocketAddr>() {
        return Ok(a);
    }
    let with_port = if text.rsplit_once(':').is_some_and(|(_, p)| p.parse::<u16>().is_ok()) {
        text.to_string()
    } else {
        format!("{text}:{DEFAULT_PORT}")
    };
    with_port
        .to_socket_addrs()
        .map_err(|e| format!("{text}: {e}"))?
        .find(|a| a.is_ipv4())
        .or_else(|| with_port.to_socket_addrs().ok()?.next())
        .ok_or_else(|| format!("{text}: no address"))
}

fn commands(app: &mut App) {
    resource_cvar::<NetSettings, u16>(
        app,
        "hostport",
        "UDP port a server listens on (Source's default 27015).",
        |s| &mut s.hostport,
    );
    resource_cvar::<NetSettings, u32>(
        app,
        "maxplayers",
        "Players a server takes, the host included (1: single player). Above 1, `map` hosts a listen server.",
        |s| &mut s.maxplayers,
    );
    resource_cvar::<NetSettings, String>(app, "name", "Your player name.", |s| &mut s.name);
    app.console_command(
        "connect",
        "connect <ip[:port]>: join a server (port 27015 unless given).",
        |w, a| {
            let addr = parse_address(&a.join(" "))?;
            client::connect(w, addr)?;
            Ok(Some(format!("Connecting to {addr}...")))
        },
    )
    .console_command(
        "listen",
        "Host the loaded map as a listen server on hostport (maxplayers raised to 8 if 1).",
        |w, _| {
            if w.resource::<NetSettings>().maxplayers <= 1 {
                w.resource_mut::<NetSettings>().maxplayers = 8;
            }
            let addr = server::listen(w)?;
            Ok(Some(format!("Listening on UDP {addr} ({})", current_map(w))))
        },
    )
    .console_command("status", "Server, map and players (with ping).", |w, _| {
        Ok(Some(status(w)))
    });
}

/// `status`: Source's layout, roughly.
pub fn status(world: &mut World) -> String {
    let role = world.get_resource::<NetRole>().copied().unwrap_or_default();
    let map = current_map(world);
    let mut out = vec![format!("version : {NET_VERSION}")];
    match role {
        NetRole::Standalone => {
            out.push(format!("map     : {map}"));
            out.push("not connected (single player)".into());
        }
        NetRole::Client => {
            let addr = world.get_resource::<client::ServerAddress>().map(|a| a.0.to_string());
            out.push(format!("server  : {}", addr.unwrap_or_else(|| "(local)".into())));
            out.push(format!("map     : {map}"));
            if let Some(c) = world.get_resource::<RenetClient>() {
                out.push(format!(
                    "state   : {}",
                    if c.is_connected() {
                        "connected"
                    } else if c.is_connecting() {
                        "connecting"
                    } else {
                        "disconnected"
                    }
                ));
                out.push(format!(
                    "ping    : {:.0} ms, loss {:.1}%",
                    c.rtt() * 1000.0,
                    c.packet_loss() * 100.0
                ));
            }
        }
        NetRole::Server => {
            let settings = world.resource::<NetSettings>().clone();
            out.push(format!("udp/ip  : 0.0.0.0:{}", settings.hostport));
            out.push(format!("map     : {map}"));
            let players = server::players(world);
            out.push(format!("players : {} ({} max)", players.len(), settings.maxplayers));
            out.push("# id               name                 ping  loss".into());
            for p in players {
                out.push(format!(
                    "# {:<16} {:<20} {:>4}  {:>4.1}%",
                    p.id,
                    p.name,
                    p.ping_ms.map_or("-".into(), |p| format!("{p:.0}")),
                    p.loss * 100.0
                ));
            }
        }
    }
    out.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn addresses_default_to_the_source_port() {
        assert_eq!(parse_address("127.0.0.1").unwrap(), "127.0.0.1:27015".parse().unwrap());
        assert_eq!(
            parse_address("127.0.0.1:27016").unwrap(),
            "127.0.0.1:27016".parse().unwrap()
        );
        assert_eq!(parse_address("localhost:27020").unwrap().port(), 27020);
        assert_eq!(parse_address("::1").unwrap(), "[::1]:27015".parse().unwrap());
        assert!(parse_address("").is_err());
    }

    #[test]
    fn intents_from_clients_are_made_safe() {
        let mut i = crate::core::Intent::default();
        NetIntent {
            move_axis: [3.0, f32::NAN],
            yaw: f32::INFINITY,
            pitch: 7.0,
            buttons: buttons::JUMP | buttons::FIRE,
            select: Some(200),
        }
        .apply(&mut i);
        assert_eq!(i.move_axis, Vec2::new(1.0, 0.0));
        assert_eq!(i.yaw, 0.0);
        assert_eq!(i.pitch, std::f32::consts::FRAC_PI_2);
        assert!(i.jump && i.fire && !i.crouch);
        assert_eq!(i.select, None);
        // A round trip keeps what a real intent says.
        let src = crate::core::Intent {
            move_axis: Vec2::new(0.6, -0.8),
            yaw: 1.25,
            pitch: -0.3,
            crouch: true,
            use_key: true,
            select: Some(2),
            ..default()
        };
        let mut back = crate::core::Intent::default();
        NetIntent::of(&src).apply(&mut back);
        assert_eq!(NetIntent::of(&back), NetIntent::of(&src));
    }

    #[test]
    fn greybox_names() {
        assert_eq!(normalize_map(""), GREYBOX);
        assert_eq!(normalize_map("mashup:greybox"), GREYBOX);
        assert_eq!(normalize_map("cs_source:de_dust2"), "cs_source:de_dust2");
    }
}
