//! Network play (docs/plans/active/multiplayer.md): an authoritative
//! server (a listen server in the game, or the dedicated `mashup_server`)
//! and clients, over bevy_replicon with renet's netcode UDP transport.
//!
//! So far (slices 1 to 7): connect and disconnect (`connect`,
//! `disconnect`, `listen`, `status`; `hostport`, `maxplayers`, `name`),
//! a join handshake (build version, protocol hash, then the map by name
//! and file hash), a character spawned on the server per client, and
//! characters replicated to every client (`NetCharacter`, `NetBody`,
//! `Team`, `Health`). A client sends a user command per server tick
//! (`UserCmds`), runs its clock ahead of the server's so each arrives in
//! time, predicts its own player and corrects it from the server's state
//! (`OwnState`): `predict`. Others are drawn in the past from a buffer of
//! snapshots keyed by server tick (`cl_interp`): `interp`. Moving brushes
//! replicate their motion state (`NetMover`) and a client steps them to
//! the tick it predicts, carrying its own player as the server does:
//! `movers`. Physics props are drawn from snapshots like others: `props`.
//! Weapons: the client predicts its own with its movement (what it
//! carries goes in `OwnState`), the server hits with lag compensation,
//! others' shots, items, grenades and deaths come from the server:
//! `weapons`.
//!
//! Rounds, money, chat and cvars: `game`, `chat`, `cvars`. Bots are the
//! server's like any character (`bot_quota`). Maps (`maps`): the server
//! offers its map (name, hash, size); a client without that file fetches
//! it over the connection or from `sv_downloadurl` into the content
//! cache, and a map change on the server takes every client along.
//! Joining's stages, for the loading dialog: `client::JoinProgress`.
//!
//! `NetRole` (core) says what this process is; `authoritative` systems
//! (rules, bots, damage, logic) don't run on a client.

pub mod chat;
pub mod client;
pub mod cvars;
pub mod decals;
pub mod game;
pub mod http;
pub mod interp;
pub mod maps;
pub mod memory;
pub mod movers;
pub mod predict;
pub mod props;
pub mod server;
pub mod udp;
pub mod weapons;

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
pub const NET_VERSION: &str = concat!(env!("CARGO_PKG_VERSION"), "/net8");

/// The owner id of the listen server's own player (`NetCharacter::owner`).
/// Remote clients' ids are never 0.
pub const HOST_ID: u64 = 0;

/// The name of the greybox map in the handshake (and `map greybox`).
pub const GREYBOX: &str = "greybox";

/// A predicted player's position at the end of a tick (server and client)
/// and of each replayed one, its zeros written as +0.0. The physics'
/// transform sync after a live tick turns a -0.0 there into 0.0 or not,
/// depending on where the physics last had the body (avian's tolerance),
/// which a replay can't know; the two are the same position, but the
/// state is compared bit for bit and feeds the next tick (the movement's
/// own state keeps the sign). Network games only: single player keeps
/// its values as they were.
pub fn canonical_position(world: &mut World, e: Entity) {
    if let Some(mut t) = world.get_mut::<Transform>(e) {
        let canonical = t.translation + Vec3::ZERO;
        if canonical.to_array().map(f32::to_bits) != t.translation.to_array().map(f32::to_bits) {
            t.translation = canonical;
        }
    }
}

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
    /// The server's name, shown to joining players (Source's `hostname`).
    pub hostname: String,
}

impl Default for NetSettings {
    fn default() -> Self {
        Self {
            hostport: DEFAULT_PORT,
            maxplayers: 1,
            name: "Player".into(),
            hostname: "Lucker Party".into(),
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
    /// (the client layer; a test's `harness::serve_maps`), from `file`
    /// when given (a downloaded copy in the content cache), else by name;
    /// then the handshake checks its hash.
    LoadMap { map: String, file: Option<std::path::PathBuf> },
    /// The server is changing level (to this map): the loading dialog
    /// shows it; the new map comes as `LoadMap` once the server has it.
    ChangingLevel(String),
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
/// `cs_source:<name>`), its file's hash and size, which the client
/// checks (and fetches, `maps`) before it plays, where to download it
/// from, the tick the server runs at, and what the loading dialog shows
/// of the server.
#[derive(Event, Serialize, Deserialize, Clone, Debug, Default)]
pub struct Welcome {
    pub map: String,
    pub map_hash: Option<[u8; 32]>,
    /// The map file's size, bytes (0: none, the greybox).
    pub map_size: u64,
    /// `sv_downloadurl`: an HTTP folder laid out like the game's
    /// (`<url>/maps/<name>.bsp.bz2` or `.bsp`); empty: none.
    pub download_url: String,
    /// `sv_allowdownload`: the server sends the map over the connection.
    pub allow_download: bool,
    /// Length of a server tick, ns (exact: the client's fixed tick must
    /// be the same `Duration`).
    pub tick_nanos: u64,
    /// The client's id (`NetCharacter::owner` of its character).
    pub you: u64,
    /// `hostname`, players in the game (bots included) and `maxplayers`.
    pub server_name: String,
    pub players: u32,
    pub max_players: u32,
}

/// Server -> clients: a map change started (`changelevel`, `map`); the
/// new map follows as `ChangeLevel` once the server has loaded it.
#[derive(Event, Serialize, Deserialize, Clone, Debug, Default)]
pub struct ChangingLevel {
    pub map: String,
}

/// Server -> clients: the server is on another map now: load it (fetch
/// it first if needed, as on joining) and play on; the connection and
/// the characters stay.
#[derive(Event, Serialize, Deserialize, Clone, Debug, Default)]
pub struct ChangeLevel {
    pub map: String,
    pub map_hash: Option<[u8; 32]>,
    pub map_size: u64,
    pub download_url: String,
    pub allow_download: bool,
    pub tick_nanos: u64,
}

/// Client -> server: I have the map loaded (`map`, the id the server
/// offered): put me in the game (`rules::enter_game`). Until then the
/// server keeps the player out of it (`core::Connecting`).
#[derive(Message, Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
pub struct Loaded {
    pub map: String,
}

/// Client -> server: send me your map's file (`sv_allowdownload`).
#[derive(Message, Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
pub struct MapRequest {
    pub map: String,
}

/// Server -> client: a piece of the map file it asked for, at `offset`
/// of `total` bytes. Reliable and in order.
#[derive(Message, Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
pub struct MapChunk {
    pub offset: u64,
    pub total: u64,
    pub data: Vec<u8>,
}

/// Client -> server: bytes of the map received so far (the server keeps
/// a window of `maps::WINDOW` bytes in flight beyond it).
#[derive(Message, Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
pub struct MapAck {
    pub received: u64,
}

/// Server -> client: it won't send the map, and why.
#[derive(Message, Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
pub struct MapDenied {
    pub reason: String,
}

/// One user command (Source's usercmd): the intent for one server tick,
/// with the look angles as the exact `f32`s the client simulated with.
/// Its command number is the tick plus the character's `Seed` offset
/// (`core::number_commands`) on both sides.
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
pub struct NetCmd {
    /// The server tick this command is for.
    pub tick: u64,
    pub move_axis: [f32; 2],
    pub yaw: f32,
    pub pitch: f32,
    /// `buttons::*` bits.
    pub buttons: u16,
    pub select: Option<u8>,
    /// The server tick (fractional) the client drew others at when it made
    /// this command (`interp::InterpClock::render_tick`; 0: none, or
    /// `cl_lagcompensation 0`): the server traces its shots against others
    /// where they were then (`weapon::lagcomp`).
    pub view_tick: f64,
}

/// `NetCmd::buttons` bits.
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

impl NetCmd {
    pub fn of(tick: u64, i: &crate::core::Intent) -> Self {
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
            tick,
            move_axis: i.move_axis.to_array(),
            yaw: i.yaw,
            pitch: i.pitch,
            buttons: b,
            select: i.select,
            view_tick: 0.0,
        }
    }

    /// The view tick, made safe: finite, not ahead of `tick` (the
    /// command's own), None when not given.
    pub fn view(&self, tick: u64) -> Option<f64> {
        (self.view_tick.is_finite() && self.view_tick > 0.0).then(|| self.view_tick.min(tick as f64))
    }

    /// Write into an intent, made safe first (never trust a client): the
    /// move axis at most 1 long, finite angles, yaw in [0, 2 pi), pitch
    /// within straight up and down, a weapon slot key 0-9. The command
    /// number stays (stamped each tick). A client puts its own intent
    /// through this too before it simulates (`normalize`), so both sides
    /// simulate the same values.
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

    /// The intent as the server will simulate it (`of`, then `apply`),
    /// written back in place; returns the command for `tick`.
    pub fn normalize(tick: u64, i: &mut crate::core::Intent) -> Self {
        Self::of(tick, i).apply(i);
        Self::of(tick, i)
    }
}

/// Client -> server, every frame it ran ticks: its new commands and the
/// `CMD_BACKUP` before them again (Source's `cl_cmdbackup`), so a lost
/// packet costs nothing. Unreliable.
#[derive(Message, Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
pub struct UserCmds {
    /// Oldest first.
    pub cmds: Vec<NetCmd>,
}

/// Commands each `UserCmds` repeats besides the new ones.
pub const CMD_BACKUP: usize = 3;

/// Server -> the client a character belongs to, each frame it ran ticks:
/// the server's state of that client's own player after a tick (its
/// predicted components, `core::PredictedComponents::encode`), which the
/// client compares with what it predicted for that tick, and how the
/// client's commands are arriving (the clock sync: `lead`). Unreliable;
/// only the newest matters.
#[derive(Message, Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
pub struct OwnState {
    /// The server tick this state is after.
    pub tick: u64,
    /// The server's simulation time at that tick (`SimClock::now`) and its
    /// tick length, ns: the client runs each command at the same time.
    pub time_nanos: u64,
    pub tick_nanos: u64,
    /// The predicted components, encoded.
    pub state: Vec<u8>,
    /// The character's movement implementation (`slots::MovementSlot`).
    pub movement: String,
    /// The character's `Seed` (its command number offset).
    pub seed: u64,
    /// The rules hold it (dead): its commands do nothing but look.
    pub held: bool,
    /// The smallest lead of the commands heard since the last `OwnState`:
    /// the newest command's tick less the server's last tick when it
    /// arrived (1: just in time). None: none heard.
    pub lead: Option<i32>,
    /// The newest command tick heard (the client counts only the leads of
    /// commands it sent after it last moved its clock).
    pub newest: u64,
    /// The map entity index (`map::MapBrushEntity`) of the mover the
    /// player stands on (`MovementState::ground`, an entity, isn't in
    /// `state`: ids differ), so a correction keeps it riding.
    pub ground: Option<u32>,
    /// Commands waiting for later ticks.
    pub buffered: u16,
    /// Freeze time holds the player until this tick (it may look and pick
    /// a weapon); `held` is the dead's hold (only the look).
    pub frozen_until: Option<u64>,
    /// What the HUD shows of the player that isn't predicted: its money
    /// (`weapon::economy::Money`), armour (amount, helmet) and whether it
    /// has a defusal kit. (Arming and defusing the bomb are predicted:
    /// `objectives::bomb::Arming`, `Defusing` in `state`.)
    pub money: Option<u32>,
    pub armor: Option<(f32, bool)>,
    pub kit: bool,
    /// Ticks run without this client's command (its last one repeated),
    /// and commands that came after their tick, since it joined.
    pub missed: u32,
    pub late: u32,
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
/// eye height and movement flags. A client keeps them by server tick
/// (`interp::Snapshots`) and draws others between two of them.
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

/// A moving brush's motion state (doors, platforms, trains, rotating and
/// parented brushes; `logic::movers::Pusher`), entity space (angles in
/// degrees), written by the server on the mover's node after each tick
/// (`movers::write_movers`). A client steps its own copy of the node
/// (`map::MapBrushEntity` with this `index`) to the tick it predicts the
/// way the logic does (`movers::place`).
#[derive(Component, Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
pub struct NetMover {
    /// The map entity index (`map::MapBrushEntity`).
    pub index: u32,
    pub origin: [f32; 3],
    pub angles: [f32; 3],
    pub velocity: [f32; 3],
    pub avelocity: [f32; 3],
    /// The pusher's local time, its move's (or wait's) end, and where the
    /// move ends.
    pub ltime: f64,
    pub move_done: Option<f64>,
    pub goal_origin: Option<[f32; 3]>,
    pub goal_angles: Option<[f32; 3]>,
    /// `mover_flags::*` bits.
    pub flags: u16,
}

/// `NetMover::flags` bits.
pub mod mover_flags {
    pub const VISIBLE: u16 = 1;
    pub const SOLID: u16 = 1 << 1;
    /// Steps land on whole ticks (func_rotating's spin).
    pub const INEXACT: u16 = 1 << 2;
    /// Angles kept in [0, 360) (func_rotating).
    pub const SPIN: u16 = 1 << 3;
    /// Rotations push by the box's leading corner (model doors).
    pub const PHYSICS_SOLID: u16 = 1 << 4;
    /// Players are moved through (trains flag 512).
    pub const UNBLOCKABLE: u16 = 1 << 5;
    /// Shots hit it (a broken breakable's aren't).
    pub const SHOOTABLE: u16 = 1 << 6;
    /// Its logic entity is gone (killed, broken) until a round restart.
    pub const GONE: u16 = 1 << 7;
    /// Moves with a parent (parented brushes, breakables): not stepped on
    /// its own.
    pub const ATTACHED: u16 = 1 << 8;
}

/// A physics prop's pose (engine space), velocity and whether it is there,
/// written by the server on the prop's node after each tick
/// (`props::write_props`); a client draws its own copy (the node with the
/// same `map::PropIndex`) from these at the render time (`props`).
#[derive(Component, Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
pub struct NetProp {
    /// `map::PropIndex`.
    pub index: u32,
    pub origin: [f32; 3],
    pub rotation: [f32; 4],
    pub velocity: [f32; 3],
    /// `prop_flags::*` bits.
    pub flags: u16,
}

/// `NetProp::flags` bits.
pub mod prop_flags {
    pub const VISIBLE: u16 = 1;
    pub const SOLID: u16 = 1 << 1;
}

/// What a character holds, for others to draw (`weapons`): the active
/// weapon's registry index (`weapon::WeaponRegistry::index`), its
/// `AltModes` mode (a silencer on) and whether a grenade's pin is out.
/// Written by the server after each tick.
#[derive(Component, Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
pub struct NetHeld {
    pub weapon: Option<u16>,
    pub mode: u8,
    pub primed: bool,
}

/// Something small the server simulates and clients draw from snapshots
/// (`weapons`): a loose weapon or a grenade in flight, by its held-model
/// key (`map::loose::ShownItem`), pose and velocity.
#[derive(Component, Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
pub struct NetItem {
    pub model: String,
    pub origin: [f32; 3],
    pub rotation: [f32; 4],
    pub velocity: [f32; 3],
}

/// A smoke cloud (`weapon::grenade::SmokeCloud`): where, and the grenade
/// whose rule it follows (registry index). A client starts its own copy
/// when it hears of it.
#[derive(Component, Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
pub struct NetSmoke {
    pub centre: [f32; 3],
    pub weapon: u16,
    /// Seconds it had been there when the server last wrote it.
    pub age: f32,
}

/// Server -> every client but the shooter's: one round someone fired
/// (`weapon::WeaponEventKind::Fired`), for the client to trace again and
/// draw (Source's "fire bullets" message): who, which weapon (registry
/// index), from where, the angles, the spread seed and the spread.
/// Unreliable, like Source's temporary entities.
#[derive(Message, Serialize, Deserialize, Clone, Debug, PartialEq, bevy::ecs::entity::MapEntities)]
pub struct FireBullets {
    #[entities]
    pub shooter: Entity,
    pub weapon: u16,
    pub origin: [f32; 3],
    pub yaw: f32,
    pub pitch: f32,
    pub seed: u32,
    /// `weapon::SpreadShape`: a disc (inaccuracy, spread), else the
    /// template's scale in `inaccuracy`.
    pub disc: bool,
    pub inaccuracy: f32,
    pub spread: f32,
    pub mode: u8,
}

/// Server -> every client but the doer's: something else a weapon did,
/// for the body's animation (`weapon_fx::*`).
#[derive(Message, Serialize, Deserialize, Clone, Debug, PartialEq, bevy::ecs::entity::MapEntities)]
pub struct WeaponFx {
    #[entities]
    pub owner: Entity,
    pub kind: u8,
}

/// `WeaponFx::kind`.
pub mod weapon_fx {
    pub const RELOAD: u8 = 1;
    pub const SHELL: u8 = 2;
    pub const RELOADED: u8 = 3;
    pub const SWING: u8 = 4;
    pub const SWING2: u8 = 5;
    pub const PIN: u8 = 6;
    pub const THROWN: u8 = 7;
}

/// Server -> clients: a grenade went off (`weapon::grenade::Detonated`):
/// what kind, which grenade (registry index), where and the ground under
/// it, for the explosion's look.
#[derive(Message, Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Detonation {
    pub kind: u8,
    pub weapon: u16,
    pub at: [f32; 3],
    pub ground: Option<([f32; 3], [f32; 3])>,
}

/// Server -> the client a character belongs to: its player was blinded
/// (alpha, seconds held, seconds fading: `core::Blinded` from now on) or
/// its hearing hit (`core::Deafened`).
#[derive(Message, Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Senses {
    pub blind: Option<(f32, f32, f32)>,
    pub hearing: Option<crate::core::HearingEffect>,
}

/// Server -> clients: a character died (`core::Died`): who, by whom, and
/// the killing hit (its weapon's registry index when it isn't what the
/// killer holds, hitgroup and kind as `net::weapons` numbers them, where
/// and which way), for the kill feed and the client's ragdoll.
#[derive(Message, Serialize, Deserialize, Clone, Debug, PartialEq, bevy::ecs::entity::MapEntities)]
pub struct Killed {
    #[entities]
    pub victim: Entity,
    #[entities]
    pub attacker: Option<Entity>,
    pub weapon: Option<u16>,
    pub hitgroup: u8,
    pub kind: u8,
    pub point: [f32; 3],
    pub dir: [f32; 3],
    pub force: [f32; 3],
}

/// Server -> the shooter's client: its shot hit (the server's
/// confirmation: the hit marker), how hard and where on the body.
#[derive(Message, Serialize, Deserialize, Clone, Debug, PartialEq, bevy::ecs::entity::MapEntities)]
pub struct HitConfirm {
    #[entities]
    pub target: Entity,
    pub amount: f32,
    pub hitgroup: u8,
}

/// Client -> server: drop the weapon I hold (Source's `drop`).
#[derive(Message, Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
pub struct DropRequest;

// --- Game rules (slice 5): `game`, `chat`, `cvars`.

/// The round and the objectives' totals, on one replicated entity the
/// server keeps (`game::write_round`). Times are server ticks
/// (fractional), which a client turns into its own clock
/// (`game::ServerTime`).
#[derive(Component, Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
pub struct NetRound {
    /// `round_phase::*`.
    pub phase: u8,
    /// Live: when it went live. Freeze: when it ends. Live: when time is
    /// up. Over: when the next round starts.
    pub since: f64,
    pub until: f64,
    /// Over: the winner's team number (None: a draw).
    pub winner: Option<u8>,
    pub number: u32,
    /// Rounds won by the attackers and the defenders.
    pub wins: [u32; 2],
    /// Why buying is closed now (the rules' words); None: open.
    pub buy_closed: Option<String>,
    /// This round's bomb: `bomb_outcome::*`.
    pub bomb: u8,
    /// Hostages this round: total, rescued, killed.
    pub hostages: [u32; 3],
}

/// `NetRound::phase`.
pub mod round_phase {
    pub const OFF: u8 = 0;
    pub const FREEZE: u8 = 1;
    pub const LIVE: u8 = 2;
    pub const OVER: u8 = 3;
}

/// `NetRound::bomb`.
pub mod bomb_outcome {
    pub const NONE: u8 = 0;
    pub const PLANTED: u8 = 1;
    pub const EXPLODED: u8 = 2;
    pub const DEFUSED: u8 = 3;
}

/// A character's line on the scoreboard (`game::write_scores`): kills,
/// deaths, ping (ms, from the server's measure of its client's round
/// trip; 0 for the host and bots) and `score_flags::*`.
#[derive(Component, Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
pub struct NetScore {
    pub kills: u32,
    pub deaths: u32,
    pub ping: u16,
    pub flags: u8,
}

/// `NetScore::flags`.
pub mod score_flags {
    pub const BOT: u8 = 1;
    /// Carries the bomb (shown to teammates).
    pub const BOMB: u8 = 1 << 1;
    /// Has a defusal kit.
    pub const KIT: u8 = 1 << 2;
}

/// The planted bomb (`objectives::bomb::PlantedBomb`), on its own
/// replicated entity: where, its model, and its clock in server ticks.
#[derive(Component, Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
pub struct NetBomb {
    pub origin: [f32; 3],
    pub rotation: [f32; 4],
    pub model: Option<String>,
    pub site: Option<u32>,
    pub planted: f64,
    pub explode: f64,
    pub timer: f32,
    /// Who defuses it, since and until when (ticks), with a kit.
    #[entities]
    pub defuser: Option<Entity>,
    pub defuse_since: f64,
    pub defuse_until: f64,
    pub defuse_kit: bool,
    pub defused: bool,
}

/// A hostage (`objectives::hostages::Hostage`): which one, its model and
/// whom it follows. With `NetCharacter` and `NetBody` like anyone.
#[derive(Component, Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
pub struct NetHostage {
    pub index: u32,
    pub model: Option<String>,
    #[entities]
    pub leader: Option<Entity>,
}

/// Client -> server: buy this (`buy <what>`, the buy menu, `buyammo1`).
#[derive(Message, Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
pub struct BuyRequest {
    pub what: String,
}

/// Client -> server: put me on this team (`jointeam`: our team number,
/// 1 terrorists, 2 counter-terrorists).
#[derive(Message, Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
pub struct TeamRequest {
    pub team: u8,
}

/// Client -> server: say this to everyone or the team (`say`,
/// `say_team`).
#[derive(Message, Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
pub struct SayRequest {
    pub text: String,
    pub team_only: bool,
}

/// Client -> server: a radio call (`coverme`, ...).
#[derive(Message, Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
pub struct RadioRequest {
    pub command: String,
}

/// Server -> a client: why its request was refused (a buy, a team), as
/// the game's hint.
#[derive(Message, Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
pub struct Notice {
    pub text: String,
}

/// Server -> each player who reads it (`map::radio::sees_say`): a chat
/// line, for the client to put in the game's format. A listen server's
/// host gets its copy as this message too.
#[derive(Message, Serialize, Deserialize, Clone, Debug, Default, PartialEq, bevy::ecs::entity::MapEntities)]
pub struct ChatMessage {
    #[entities]
    pub sender: Option<Entity>,
    pub name: String,
    /// The sender's team number, if any; whether alive; where (the nav
    /// mesh's place name).
    pub team: Option<u8>,
    pub alive: bool,
    pub team_only: bool,
    pub place: Option<String>,
    pub text: String,
}

/// Server -> each player who hears it (`map::radio::hears`): a radio
/// call, played as `core::Radio` here.
#[derive(Message, Serialize, Deserialize, Clone, Debug, PartialEq, bevy::ecs::entity::MapEntities)]
pub struct RadioCall {
    #[entities]
    pub sender: Entity,
    pub command: String,
}

/// Server -> clients: an objective event (`objectives::ObjectiveEvent`,
/// `hostages::HostagePenalty`) for the HUD's words and sounds: its kind
/// (`game` numbers them), who, the other party, where, and the rest.
#[derive(Message, Serialize, Deserialize, Clone, Debug, Default, PartialEq, bevy::ecs::entity::MapEntities)]
pub struct ObjectiveNews {
    pub kind: u8,
    #[entities]
    pub who: Option<Entity>,
    #[entities]
    pub other: Option<Entity>,
    pub at: [f32; 3],
    pub flag: bool,
    pub site: Option<u32>,
    pub amount: f32,
    pub reason: u8,
}

/// Server -> clients: the round ended (`rules::rounds::RoundEnded`):
/// the winner's team number and why (`game` numbers the reasons).
#[derive(Message, Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
pub struct RoundOver {
    pub winner: Option<u8>,
    pub reason: u8,
}

/// Server -> clients: a sound of the game's rules (`map::GameSound`: the
/// bomb, hostages), played where the server played it.
#[derive(Message, Serialize, Deserialize, Clone, Debug, Default, PartialEq, bevy::ecs::entity::MapEntities)]
pub struct ServerSound {
    pub entry: String,
    pub at: Option<[f32; 3]>,
    pub volume: Option<f32>,
    pub pitch: Option<f32>,
    #[entities]
    pub source: Option<Entity>,
    pub channel: Option<u8>,
}

/// Server -> clients: replicated cvars' values (`console::CvarScope::
/// Replicated`): all of them when a client joins, then those that
/// changed. A client sets them and refuses local changes while connected.
#[derive(Message, Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
pub struct CvarValues {
    pub values: Vec<(String, String)>,
}

/// Client -> server: my name is now this (the `name` cvar changed while
/// connected; Source's setinfo).
#[derive(Message, Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
pub struct NameRequest {
    pub name: String,
}

/// Server -> every player: someone changed their name (the game's
/// "* %s1 changed name to %s2").
#[derive(Message, Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
pub struct NameChanged {
    pub old: String,
    pub new: String,
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
            RepliconPlugins
                .set(RepliconSharedPlugin {
                    // Our own handshake (`Join`): a version message before the
                    // protocol hash.
                    auth_method: AuthMethod::Custom,
                })
                .set(ServerPlugin {
                    // A mutate message every tick, empty or not: a client
                    // learns that what didn't change held still at that
                    // tick (`interp`), and when ticks arrive (its clock).
                    track_mutate_messages: true,
                    ..ServerPlugin::new(FixedPostUpdate)
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
        // Maps (slice 7).
        .add_server_event::<ChangingLevel>(Channel::Ordered)
        .make_event_independent::<ChangingLevel>()
        .add_server_event::<ChangeLevel>(Channel::Ordered)
        .make_event_independent::<ChangeLevel>()
        .add_client_message::<Loaded>(Channel::Ordered)
        .add_client_message::<MapRequest>(Channel::Ordered)
        .add_client_message::<MapAck>(Channel::Unreliable)
        .add_server_message::<MapChunk>(Channel::Ordered)
        .make_message_independent::<MapChunk>()
        .add_server_message::<MapDenied>(Channel::Ordered)
        .make_message_independent::<MapDenied>()
        .add_client_message::<UserCmds>(Channel::Unreliable)
        .add_server_message::<OwnState>(Channel::Unreliable)
        // Holds no entities: no need to wait for replication.
        .make_message_independent::<OwnState>()
        .replicate::<NetCharacter>()
        .replicate::<NetBody>()
        .replicate::<Team>()
        .replicate::<Health>()
        .replicate::<NetMover>()
        // Every value a client receives also goes into its snapshot
        // buffer, by the server tick it is from.
        .set_receive_fns::<NetBody>(interp::write_snapshot::<NetBody>, interp::remove_snapshots::<NetBody>)
        .set_receive_fns::<NetMover>(interp::write_snapshot::<NetMover>, interp::remove_snapshots::<NetMover>)
        .replicate::<NetProp>()
        .set_receive_fns::<NetProp>(interp::write_snapshot::<NetProp>, interp::remove_snapshots::<NetProp>)
        // Weapons (slice 4).
        .replicate::<NetHeld>()
        .replicate::<NetItem>()
        .set_receive_fns::<NetItem>(interp::write_snapshot::<NetItem>, interp::remove_snapshots::<NetItem>)
        .replicate::<NetSmoke>()
        .add_mapped_server_message::<FireBullets>(Channel::Unreliable)
        .add_mapped_server_message::<WeaponFx>(Channel::Unreliable)
        .add_server_message::<Detonation>(Channel::Unreliable)
        .make_message_independent::<Detonation>()
        .add_server_message::<Senses>(Channel::Ordered)
        .make_message_independent::<Senses>()
        .add_client_message::<DropRequest>(Channel::Ordered)
        .add_mapped_server_message::<Killed>(Channel::Ordered)
        .add_mapped_server_message::<HitConfirm>(Channel::Unordered)
        // Game rules (slice 5).
        .replicate::<NetRound>()
        .replicate::<NetScore>()
        .replicate::<NetBomb>()
        .replicate::<NetHostage>()
        .add_client_message::<BuyRequest>(Channel::Ordered)
        .add_client_message::<TeamRequest>(Channel::Ordered)
        .add_client_message::<SayRequest>(Channel::Ordered)
        .add_client_message::<RadioRequest>(Channel::Ordered)
        .add_server_message::<Notice>(Channel::Ordered)
        .make_message_independent::<Notice>()
        .add_mapped_server_message::<ChatMessage>(Channel::Ordered)
        .add_mapped_server_message::<RadioCall>(Channel::Ordered)
        .add_mapped_server_message::<ObjectiveNews>(Channel::Ordered)
        .add_server_message::<RoundOver>(Channel::Ordered)
        .add_mapped_server_message::<ServerSound>(Channel::Unreliable)
        .add_server_message::<CvarValues>(Channel::Ordered)
        .make_message_independent::<CvarValues>()
        .add_client_message::<NameRequest>(Channel::Ordered)
        .add_server_message::<NameChanged>(Channel::Ordered)
        .make_message_independent::<NameChanged>()
        .init_resource::<NetSettings>()
        .init_resource::<NetVersion>()
        .add_message::<NetEvent>()
        .add_systems(Update, log_events);
        app.world_mut().resource_mut::<ProtocolHasher>().add_custom(NET_VERSION);
        server::plugin(app);
        client::plugin(app);
        predict::plugin(app);
        interp::plugin(app);
        movers::plugin(app);
        props::plugin(app);
        weapons::plugin(app);
        game::plugin(app);
        chat::plugin(app);
        cvars::plugin(app);
        maps::plugin(app);
        decals::plugin(app);
        udp::plugin(app);
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
            NetEvent::LoadMap { map, file } => match file {
                Some(f) => info!("the server is on {map}: loading it from {}", f.display()),
                None => info!("the server is on {map}: loading it"),
            },
            NetEvent::ChangingLevel(m) => info!("the server is changing level to {m}"),
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
    // The loaded file's name, with or without its game (`cs_source:x`).
    let have = file.name.rsplit(':').next().unwrap_or(&file.name);
    if !have.eq_ignore_ascii_case(want_name) || current_map(world) != map {
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
        NetRole::Server => {
            server::stop(world);
            maps::stop_serving(world);
        }
        NetRole::Client => {
            client::stop(world);
            client::ended(world, reason);
        }
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
    resource_cvar::<NetSettings, String>(
        app,
        "hostname",
        "The server's name, shown to players joining it.",
        |s| &mut s.hostname,
    );
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
            if let Some(g) = world.get_resource::<predict::NetGraph>() {
                out.push(format!("predict : {}", g.prediction_line()));
                out.push(format!("interp  : {}", g.interp_line()));
            }
        }
        NetRole::Server => {
            let settings = world.resource::<NetSettings>().clone();
            out.push(format!("udp/ip  : 0.0.0.0:{}", settings.hostport));
            out.push(format!("map     : {map}"));
            let players = server::players(world);
            out.push(format!("players : {} ({} max)", players.len(), settings.maxplayers));
            out.push("# id               name                 ping  loss   cmds missed".into());
            for p in players {
                out.push(format!(
                    "# {:<16} {:<20} {:>4}  {:>4.1}%  {:>4} {:>6}",
                    p.id,
                    p.name,
                    p.ping_ms.map_or("-".into(), |p| format!("{p:.0}")),
                    p.loss * 100.0,
                    p.buffered,
                    p.missed
                ));
            }
            let step = world.resource::<Time<Fixed>>().timestep().as_secs_f64();
            let lag = world.resource::<crate::weapon::lagcomp::LagCompStats>();
            out.push(format!(
                "lagcomp : {} shots rewound, {} held at sv_maxunlag, {} not across a teleport{}",
                lag.rewinds,
                lag.clamped,
                lag.teleports,
                lag.last.as_ref().map_or(String::new(), |r| format!(
                    "; last {:.0} ms back, {} moved",
                    r.seconds_back(step) * 1000.0,
                    r.moved
                ))
            ));
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
        NetCmd {
            tick: 7,
            move_axis: [3.0, f32::NAN],
            yaw: f32::INFINITY,
            pitch: 7.0,
            buttons: buttons::JUMP | buttons::FIRE,
            select: Some(200),
            view_tick: f64::NAN,
        }
        .apply(&mut i);
        assert_eq!(i.move_axis, Vec2::new(1.0, 0.0));
        assert_eq!(i.yaw, 0.0);
        assert_eq!(i.pitch, std::f32::consts::FRAC_PI_2);
        assert!(i.jump && i.fire && !i.crouch);
        assert_eq!(i.select, None);
        // A view tick that isn't one, or one ahead of its command: none,
        // or the command's own.
        let c = |v: f64| NetCmd {
            tick: 7,
            view_tick: v,
            ..default()
        };
        assert_eq!(c(f64::NAN).view(7), None);
        assert_eq!(c(f64::INFINITY).view(7), None);
        assert_eq!(c(0.0).view(7), None);
        assert_eq!(c(5.5).view(7), Some(5.5));
        assert_eq!(c(90.0).view(7), Some(7.0));
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
        NetCmd::of(1, &src).apply(&mut back);
        assert_eq!(NetCmd::of(1, &back), NetCmd::of(1, &src));
    }

    #[test]
    fn greybox_names() {
        assert_eq!(normalize_map(""), GREYBOX);
        assert_eq!(normalize_map("mashup:greybox"), GREYBOX);
        assert_eq!(normalize_map("cs_source:de_dust2"), "cs_source:de_dust2");
    }
}
