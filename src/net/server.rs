//! The server side: listening, the join handshake, a character per
//! client, intents from clients, and what characters replicate.

use std::{
    net::{Ipv4Addr, SocketAddr, UdpSocket},
    time::SystemTime,
};

use bevy::prelude::*;
use bevy_replicon::{prelude::*, server::server_tick::ServerTick, shared::backend::connected_client::NetworkId};
use bevy_replicon_renet::{
    RenetServer,
    netcode::{ServerAuthentication, ServerConfig},
};

use super::{
    HOST_ID, Join, NET_VERSION, NetBody, NetCharacter, NetCmd, NetEvent, NetSettings, NetVersion, OwnState,
    PROTOCOL_ID, Refused, UserCmds, body_flags,
};
use crate::{
    character::character_bundle,
    core::{
        FreezeTime, Health, Intent, LocalPlayer, MovementState, NetRole, PredictedComponents, Seed, SimClock, SimSet,
        SimTick, Team, Velocity,
    },
    objectives::hostages::Hostage,
    rules::{
        Dead,
        rounds::{Phase, RoundState},
    },
    slots::{Loadout, MovementSlot, set_movement},
    weapon::{
        Armor, Inventory,
        economy::{DefuseKit, Money},
    },
};

pub(super) fn plugin(app: &mut App) {
    use crate::console::ConsoleAppExt;
    app.console_command(
        "kick",
        "kick <name>: drop a player (or a bot) from the server.",
        |w, a| {
            let name = a.join(" ");
            if name.is_empty() {
                return Err("kick <name>".into());
            }
            kick_by(w, |n, _| n.eq_ignore_ascii_case(&name), "Kicked by Console")
                .ok_or_else(|| format!("No player named \"{name}\""))
                .map(Some)
        },
    )
    .console_command(
        "kickid",
        "kickid <userid> [message]: drop the player with that id (status lists them).",
        |w, a| {
            let id: u64 = a
                .first()
                .and_then(|i| i.parse().ok())
                .ok_or("kickid <userid> [message]")?;
            let message = a[1..].join(" ");
            let reason = if message.is_empty() {
                "Kicked by Console".to_string()
            } else {
                format!("Kicked by Console : {message}")
            };
            kick_by(w, |_, i| i == Some(id), &reason)
                .ok_or_else(|| format!("No player with userid {id}"))
                .map(Some)
        },
    );
    app.add_observer(join)
        .add_observer(left)
        .add_systems(Update, drop_refused.run_if(in_state(ServerState::Running)))
        .add_systems(
            PreUpdate,
            (receive_commands, receive_loaded)
                .after(ServerSystems::Receive)
                .run_if(in_state(ServerState::Running)),
        )
        // Before the rules, which may hold the intent (the dead, freeze
        // time), as bots write theirs.
        .add_systems(
            FixedUpdate,
            apply_commands
                .before(SimSet::Rules)
                .run_if(resource_equals(NetRole::Server)),
        )
        // The tick's outcome, from the simulation's values (a listen
        // server draws eased transforms after the fixed loop).
        .add_systems(
            FixedLast,
            (canonical_positions, write_bodies, capture_own_states)
                .chain()
                .run_if(resource_equals(NetRole::Server)),
        )
        // Replicon's tick is the simulation's: a client keys snapshots by
        // the tick their message is from (`interp`).
        .add_systems(
            FixedPostUpdate,
            lock_replication_tick
                .after(ServerSystems::IncrementTick)
                .run_if(resource_equals(NetRole::Server)),
        )
        .add_systems(
            PostUpdate,
            (replicate_characters, send_own_states)
                .before(ServerSystems::Send)
                .run_if(resource_equals(NetRole::Server)),
        );
}

/// On a client's server-side entity (replicon's `ConnectedClient`): its
/// player's name and character.
#[derive(Component, Debug)]
pub struct Player {
    pub name: String,
    pub character: Entity,
}

/// On a refused client's entity: when it was told (`drop_refused`).
#[derive(Component, Debug)]
struct RefusedAt(f64);

/// How long a refused client has to hear why before it is dropped, s.
const REFUSED_GRACE: f64 = 1.0;

/// Kick the first player (or bot: no id) whose name and id `pick` takes:
/// a client is told why and dropped a moment later (as a refusal); a bot
/// leaves at once and the quota goes down by one. What happened, or None
/// when nobody matched (or no game is served).
fn kick_by(world: &mut World, pick: impl Fn(&str, Option<u64>) -> bool, reason: &str) -> Option<String> {
    if world.get_resource::<NetRole>() != Some(&NetRole::Server) {
        return None;
    }
    let client = world
        .query::<(Entity, &Player, &NetworkId)>()
        .iter(world)
        .find(|(_, p, id)| pick(&p.name, Some(id.get())))
        .map(|(c, p, _)| (c, p.name.clone()));
    if let Some((client, name)) = client {
        info!("kicked {name}: {reason}");
        world.commands().server_trigger(ToClients {
            targets: SendTargets::Single(ClientId::Client(client)),
            message: Refused {
                reason: reason.to_string(),
            },
        });
        let at = world.resource::<Time<Real>>().elapsed_secs_f64();
        world.entity_mut(client).insert(RefusedAt(at));
        return Some(format!("kicked {name}"));
    }
    let bot = world
        .query_filtered::<(Entity, &Name), With<crate::bot::Bot>>()
        .iter(world)
        .find(|(_, n)| pick(n.as_str(), None))
        .map(|(e, n)| (e, n.as_str().to_string()));
    let (bot, name) = bot?;
    crate::bot::kick_bot(world, bot);
    if let Some(mut q) = world.get_resource_mut::<crate::bot::BotQuota>() {
        q.quota = q.quota.saturating_sub(1);
    }
    Some(format!("kicked {name}"))
}

/// Drop refused clients that didn't hang up.
fn drop_refused(q: Query<(Entity, &RefusedAt)>, time: Res<Time<Real>>, mut out: MessageWriter<DisconnectRequest>) {
    let now = time.elapsed_secs_f64();
    for (client, at) in &q {
        if now - at.0 >= REFUSED_GRACE {
            out.write(DisconnectRequest { client });
        }
    }
}

/// On a remote player's character: the commands its client sent for
/// ticks to come (a jitter buffer, plan §1 "bind command number to server
/// tick"), the last one applied, and how they arrive.
#[derive(Component, Debug, Default)]
pub struct CommandBuffer {
    pub queued: std::collections::BTreeMap<u64, NetCmd>,
    /// The command run last (repeated for a tick without one).
    pub last: Option<NetCmd>,
    /// The last tick a command was applied for.
    pub applied: u64,
    /// The smallest lead heard since the last `OwnState` (see
    /// `OwnState::lead`), and the newest tick heard.
    pub lead: Option<i32>,
    pub newest: u64,
    /// The newest clock epoch heard (`UserCmds::epoch`): `lead` is that
    /// epoch's.
    pub epoch: u32,
    /// Ticks run on a repeated command; commands that came too late (after
    /// their tick) or too early (beyond `MAX_AHEAD`).
    pub missed: u32,
    pub late: u32,
    pub early: u32,
}

/// Commands further ahead of the server than this (ticks) are dropped: a
/// client can't queue up more than this, whatever its clock says.
pub const MAX_AHEAD: u64 = 64;

/// On a remote player's character: its state after the latest tick, for
/// its client (`send_own_states`).
#[derive(Component, Debug, Default)]
pub struct OwnStateOut {
    state: Option<OwnState>,
    sent: u64,
}

/// Start serving the loaded map on `hostport` (a listen server: this
/// process's own player plays too). Fails if a network game is running
/// or the port is taken.
pub fn listen(world: &mut World) -> Result<SocketAddr, String> {
    let port = world.resource::<NetSettings>().hostport;
    let socket = UdpSocket::bind((Ipv4Addr::UNSPECIFIED, port)).map_err(|e| format!("UDP port {port}: {e}"))?;
    let addr = socket.local_addr().map_err(|e| e.to_string())?;
    let max_clients = remote_slots(world);
    let config = ServerConfig {
        current_time: SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap_or_default(),
        max_clients,
        protocol_id: PROTOCOL_ID,
        // Direct IP and LAN, as Source's game traffic: no encryption.
        // Secure connect tokens come with a matchmaker (plan slice 9).
        public_addresses: vec![addr],
        authentication: ServerAuthentication::Unsecure,
    };
    // Answers server queries on the same port (`query`).
    let secret = world.resource::<super::query::InstanceId>().0 ^ config.current_time.as_nanos() as u64;
    let transport = super::udp::UdpServer::new(config, socket, secret).map_err(|e| e.to_string())?;
    start(world)?;
    world.insert_resource(transport);
    world.write_message(NetEvent::Listening(addr));
    Ok(addr)
}

/// Become the server with renet's `RenetServer` (a transport is added by
/// the caller: netcode in `listen`, memory in tests).
pub fn start(world: &mut World) -> Result<(), String> {
    if super::active(world) {
        return Err("already in a network game (disconnect first)".into());
    }
    let config = super::connection_config(world);
    world.insert_resource(RenetServer::new(config));
    world.insert_resource(NetRole::Server);
    // Bots added before serving stay (`bot_quota` counts them).
    crate::bot::sync_quota(world);
    Ok(())
}

/// Remote players the server takes: `maxplayers`, less the host's own
/// slot on a listen server (one with a local player).
fn remote_slots(world: &mut World) -> usize {
    let max = world.resource::<NetSettings>().maxplayers.max(1) as usize;
    let host = world.query_filtered::<(), With<LocalPlayer>>().iter(world).count();
    max.saturating_sub(host).max(1)
}

/// Stop serving: clients are told and dropped, their characters go.
pub(super) fn stop(world: &mut World) {
    if let Some(mut transport) = world.remove_resource::<super::udp::UdpServer>() {
        if let Some(mut server) = world.get_resource_mut::<RenetServer>() {
            transport.disconnect_all(&mut server);
        }
    }
    if let Some(memory) = world.remove_resource::<super::memory::MemoryServer>()
        && let Some(server) = world.get_resource::<RenetServer>()
    {
        memory.hang_up(server);
    }
    world.remove_resource::<RenetServer>();
    let clients: Vec<Entity> = world
        .query_filtered::<Entity, With<ConnectedClient>>()
        .iter(world)
        .collect();
    for c in clients {
        despawn_player(world, c);
        world.despawn(c);
    }
    // What stays (the host, bots) stops replicating.
    let replicated: Vec<Entity> = world.query_filtered::<Entity, With<Replicated>>().iter(world).collect();
    for e in replicated {
        world
            .entity_mut(e)
            .remove::<(
                Replicated,
                NetCharacter,
                NetBody,
                super::NetMover,
                super::NetProp,
                super::NetHeld,
                super::NetItem,
                super::NetSmoke,
                super::NetScore,
                super::NetBomb,
                super::NetHostage,
            )>();
    }
    // The round's entity goes.
    let rounds: Vec<Entity> = world
        .query_filtered::<Entity, With<super::NetRound>>()
        .iter(world)
        .collect();
    for e in rounds {
        world.despawn(e);
    }
}

/// A connected client says who it is: refused (another version or
/// protocol, the server full) or let in with a character.
fn join(join: On<FromClient<Join>>, mut commands: Commands) {
    let Some(client) = join.client_id.entity() else {
        return;
    };
    let msg = join.message.clone();
    commands.queue(move |world: &mut World| admit(world, client, msg));
}

fn admit(world: &mut World, client: Entity, msg: Join) {
    let ours = world.resource::<NetVersion>().0.clone();
    let protocol = *world.resource::<ProtocolHash>();
    let refuse = if msg.version != ours {
        Some(format!(
            "Your game is version {} but the server's is {ours}: update the one that's older.",
            msg.version
        ))
    } else if msg.protocol != protocol {
        Some(format!(
            "Your game's network protocol differs from the server's ({NET_VERSION})."
        ))
    } else if world.get::<Player>(client).is_some() {
        return;
    } else if !password_ok(world, client) {
        // Source's words.
        Some("Bad password.".to_string())
    } else {
        let taken = world.query_filtered::<(), With<Player>>().iter(world).count();
        (taken >= remote_slots(world)).then(|| "Server is full.".to_string())
    };
    let id = world.get::<NetworkId>(client).map_or(0, |n| n.get());
    if let Some(reason) = refuse {
        info!("refused client {id} ({}): {reason}", msg.name);
        world.commands().server_trigger(ToClients {
            targets: SendTargets::Single(ClientId::Client(client)),
            message: Refused { reason },
        });
        // Dropped a moment later: the client hangs up when it hears why,
        // and a disconnect sent now could overtake the reason.
        let at = world.resource::<Time<Real>>().elapsed_secs_f64();
        world.entity_mut(client).insert(RefusedAt(at));
        return;
    }
    let name = clean_name(&msg.name, id);
    let character = spawn_player(world, id, &name);
    set_userinfo(world, character, &msg.userinfo);
    world.entity_mut(client).insert((
        AuthorizedClient,
        Player {
            name: name.clone(),
            character,
        },
    ));
    // The served map (its offer: name, hash, size, where to download it).
    let welcome = super::maps::welcome(world, id);
    world.commands().server_trigger(ToClients {
        targets: SendTargets::Single(ClientId::Client(client)),
        message: welcome,
    });
    // The server's replicated cvars (movement, rules), before it plays.
    super::cvars::send_all(world, client);
    info!("{name} joined (client {id})");
}

/// Whether a client gave the server's password (`sv_password`; any
/// without one). It rides in netcode's user data (`query::user_data`);
/// a client without a UDP transport (tests' memory link) gives none.
fn password_ok(world: &World, client: Entity) -> bool {
    let want = &world.resource::<super::query::Hosting>().password;
    if want.is_empty() {
        return true;
    }
    let id = world.get::<NetworkId>(client).map(|n| n.get());
    let data = id.and_then(|id| world.get_resource::<super::udp::UdpServer>()?.user_data(id));
    super::query::password_of(data.as_ref()) == *want
}

/// A remote player's userinfo (`core::UserInfo`), bounded: at most 64
/// keys of 64 bytes, values of 256.
pub(super) fn set_userinfo(world: &mut World, character: Entity, values: &[(String, String)]) {
    let info = crate::core::UserInfo(
        values
            .iter()
            .filter(|(k, v)| k.len() <= 64 && v.len() <= 256)
            .take(64)
            .map(|(k, v)| (k.to_lowercase(), v.clone()))
            .collect(),
    );
    if let Ok(mut e) = world.get_entity_mut(character) {
        e.insert(info);
    }
}

/// A printable name of at most 32 characters, or "Player <id>".
pub(super) fn clean_name(name: &str, id: u64) -> String {
    let n: String = name.chars().filter(|c| !c.is_control()).take(32).collect();
    let n = n.trim();
    if n.is_empty() {
        format!("Player {id}")
    } else {
        n.to_string()
    }
}

/// A remote player's character: on the team with fewer players, out of
/// the game (`core::Connecting`) until its client has loaded the map,
/// then spawned as the rules allow (`rules::enter_game`).
fn spawn_player(world: &mut World, id: u64, name: &str) -> Entity {
    let mut counts = [0usize; 2];
    for (team, hostage) in world.query::<(&Team, Has<Hostage>)>().iter(world) {
        if !hostage && (1..=2).contains(&team.0) {
            counts[team.0 as usize - 1] += 1;
        }
    }
    // Counter-terrorists (2) on a tie, as the local player starts.
    let team = if counts[0] < counts[1] { Team(1) } else { Team(2) };
    let movement = world
        .get_resource::<Loadout>()
        .map_or(crate::movement::placeholder::ID, |l| l.movement);
    let e = world
        .spawn((
            character_bundle(Transform::default(), team),
            // Command numbers and shared randoms differ per player.
            Seed(0x5EED_0000_0000_0000 ^ id),
            Dead { since: f64::MIN },
            CommandBuffer::default(),
            OwnStateOut::default(),
            crate::weapon::lagcomp::ViewTick::default(),
            crate::core::RemotePlayer,
            Replicated,
            NetCharacter {
                owner: Some(id),
                name: name.to_string(),
            },
            NetBody::default(),
        ))
        .id();
    world.entity_mut(e).insert(Name::new(name.to_string()));
    set_movement(e, movement).apply(world);
    // Out of the game until its client has the map (`receive_loaded`).
    crate::rules::hold_out(world, e);
    e
}

/// A client has the map loaded: its player comes into the game, if it's
/// the map the server is on (a report from before a map change counts
/// for nothing).
fn receive_loaded(mut loaded: MessageReader<FromClient<super::Loaded>>, players: Query<&Player>, mut commands: Commands) {
    for l in loaded.read() {
        let Some(client) = l.client_id.entity() else { continue };
        let Ok(p) = players.get(client) else { continue };
        let (character, map) = (p.character, l.message.map.clone());
        commands.queue(move |world: &mut World| {
            let serving = world
                .get_resource::<super::maps::ServedMap>()
                .map_or_else(|| super::current_map(world), |s| s.map.clone());
            if !map.eq_ignore_ascii_case(&serving) || world.get_entity(character).is_err() {
                info!("a client reported {map} loaded; the server is on {serving}");
                return;
            }
            crate::rules::enter_game(world, character);
            // What was shot up before it came.
            super::decals::send(world, client);
        });
    }
}

/// A client left (or was dropped): its character and weapons go.
fn left(remove: On<Remove, ConnectedClient>, players: Query<&Player>, mut commands: Commands) {
    let Ok(p) = players.get(remove.entity) else { return };
    info!("{} left", p.name);
    let character = p.character;
    commands.queue(move |world: &mut World| despawn_character(world, character));
}

fn despawn_player(world: &mut World, client: Entity) {
    if let Some(character) = world.get::<Player>(client).map(|p| p.character) {
        despawn_character(world, character);
    }
}

/// A character and the weapons it carries.
fn despawn_character(world: &mut World, character: Entity) {
    let weapons = world
        .get::<Inventory>(character)
        .map(|i| i.weapons.clone())
        .unwrap_or_default();
    for w in weapons {
        if let Ok(e) = world.get_entity_mut(w) {
            e.despawn();
        }
    }
    if let Ok(e) = world.get_entity_mut(character) {
        e.despawn();
    }
}

/// Queue each remote player's commands by tick, and note how early they
/// came (the clock sync's lead). Commands for ticks already run are late
/// (counted, dropped); ones more than `MAX_AHEAD` ahead are dropped. A
/// command already queued for a tick is replaced (the client moved its
/// clock back and re-predicted that tick); repeats from `CMD_BACKUP` are
/// the same command.
fn receive_commands(
    mut messages: MessageReader<FromClient<UserCmds>>,
    players: Query<&Player>,
    mut buffers: Query<&mut CommandBuffer>,
    tick: Res<SimTick>,
) {
    let now = tick.0;
    for msg in messages.read() {
        let Some(client) = msg.client_id.entity() else {
            continue;
        };
        let Ok(p) = players.get(client) else { continue };
        let Ok(mut b) = buffers.get_mut(p.character) else {
            continue;
        };
        let Some(newest) = msg.message.cmds.iter().map(|c| c.tick).max() else {
            continue;
        };
        let lead = (newest as i64 - now as i64).clamp(i32::MIN as i64, i32::MAX as i64) as i32;
        // Leads of the client's present clock only: commands sent before
        // it jumped say nothing of where it is now.
        if msg.message.epoch > b.epoch {
            b.epoch = msg.message.epoch;
            b.lead = None;
        }
        if msg.message.epoch == b.epoch {
            b.lead = Some(b.lead.map_or(lead, |l| l.min(lead)));
        }
        b.newest = b.newest.max(newest);
        if newest <= b.applied {
            // Even its newest command's tick has run.
            b.late += 1;
        }
        for cmd in &msg.message.cmds {
            if cmd.tick <= b.applied {
                continue;
            }
            if cmd.tick > now + MAX_AHEAD {
                b.early += 1;
                continue;
            }
            b.queued.insert(cmd.tick, cmd.clone());
        }
    }
}

/// Each remote player's command for this tick into its `Intent`, made
/// safe (`NetCmd::apply`): exactly one command per tick whatever the
/// client sends, so no client moves faster than the tick allows. Without
/// one (lost, late) the last repeats, and the client gets corrected.
fn apply_commands(
    mut q: Query<(&mut CommandBuffer, &mut Intent, Option<&mut crate::weapon::lagcomp::ViewTick>)>,
    clock: Res<SimClock>,
) {
    let tick = clock.tick;
    for (mut b, mut intent, view) in &mut q {
        // Commands for ticks gone by never run.
        let stale: Vec<u64> = b.queued.range(..tick).map(|(t, _)| *t).collect();
        for t in stale {
            b.queued.remove(&t);
        }
        let cmd = match b.queued.remove(&tick) {
            Some(c) => Some(c),
            None => {
                if b.last.is_some() {
                    b.missed += 1;
                }
                b.last.clone()
            }
        };
        b.applied = tick;
        if let Some(c) = cmd {
            c.apply(&mut intent);
            // Where it saw others: its shots are traced there.
            if let Some(mut view) = view {
                let v = crate::weapon::lagcomp::ViewTick(c.view(tick));
                if *view != v {
                    *view = v;
                }
            }
            b.last = Some(c);
        }
    }
}

/// Remote players' positions with their zeros as +0.0 (`canonical_position`).
fn canonical_positions(world: &mut World) {
    let players: Vec<Entity> = world
        .query_filtered::<Entity, With<OwnStateOut>>()
        .iter(world)
        .collect();
    for e in players {
        super::canonical_position(world, e);
    }
}

/// After each tick: every remote player's predicted state for its client.
fn capture_own_states(world: &mut World) {
    let clock = *world.resource::<SimClock>();
    let tick_nanos = world.resource::<Time<Fixed>>().timestep().as_nanos() as u64;
    let time_nanos = world.resource::<Time<Fixed>>().elapsed().as_nanos() as u64;
    let frozen = world.get_resource::<FreezeTime>().is_some_and(|f| f.0);
    // The freeze ends on the first tick at its end time (`rules::rounds`).
    let frozen_until = match world.get_resource::<RoundState>().map(|r| r.phase) {
        Some(Phase::Freeze { until }) if frozen => {
            let dt = clock.delta.as_secs_f64().max(1e-6);
            let k = ((until - clock.now) / dt - 1e-6).ceil().max(1.0);
            Some(clock.tick + k as u64)
        }
        _ if frozen => Some(u64::MAX),
        _ => None,
    };
    let characters: Vec<Entity> = world
        .query_filtered::<Entity, With<OwnStateOut>>()
        .iter(world)
        .collect();
    world.resource_scope(|world, registry: Mut<PredictedComponents>| {
        for e in characters {
            let state = registry.encode(world, e);
            let movement = world.get::<MovementSlot>(e).map_or("", |m| m.0).to_string();
            let seed = world.get::<Seed>(e).map_or(0, |s| s.0);
            let held = world.get::<Dead>(e).is_some();
            let money = world.get::<Money>(e).map(|m| m.0);
            let armor = world.get::<Armor>(e).map(|a| (a.amount, a.helmet));
            let kit = world.get::<DefuseKit>(e).is_some();
            let view = world
                .get::<crate::core::MapView>(e)
                .map(|v| (v.origin.to_array(), v.rotation.to_array()));
            let ground = world
                .get::<MovementState>(e)
                .and_then(|s| s.ground)
                .and_then(|g| world.get::<crate::map::MapBrushEntity>(g))
                .map(|m| m.0 as u32);
            let mut out = world.get_mut::<OwnStateOut>(e).expect("queried");
            out.state = Some(OwnState {
                tick: clock.tick,
                time_nanos,
                tick_nanos,
                state,
                movement,
                seed,
                held,
                ground,
                frozen_until,
                money,
                armor,
                kit,
                view,
                ..default()
            });
        }
    });
}

/// The newest state of each remote player to its client, with how its
/// commands are arriving.
fn send_own_states(
    players: Query<(Entity, &Player)>,
    mut characters: Query<(&mut OwnStateOut, &mut CommandBuffer)>,
    mut out: MessageWriter<ToClients<OwnState>>,
) {
    for (client, p) in &players {
        let Ok((mut own, mut b)) = characters.get_mut(p.character) else {
            continue;
        };
        let Some(mut state) = own.state.clone() else { continue };
        if state.tick <= own.sent {
            continue;
        }
        own.sent = state.tick;
        state.lead = b.lead.take();
        state.newest = b.newest;
        state.epoch = b.epoch;
        state.buffered = b.queued.len().min(u16::MAX as usize) as u16;
        state.missed = b.missed;
        state.late = b.late;
        out.write(ToClients {
            targets: SendTargets::Single(ClientId::Client(client)),
            message: state,
        });
    }
}
/// Replicon's tick (the tick replication messages carry) kept equal to
/// the simulation's (`SimClock::tick`), so a value a client receives is
/// keyed by the server tick it is from. Both count once per fixed tick;
/// this only moves replicon's forward to it once, when serving starts.
fn lock_replication_tick(clock: Res<SimClock>, mut tick: ResMut<ServerTick>) {
    let want = clock.tick as u32;
    let diff = want.wrapping_sub(tick.get());
    if diff != 0 && (diff as i32) > 0 {
        tick.increment_by(diff);
    }
}

/// Characters this server didn't spawn for a client (the host's, bots)
/// replicate too.
#[allow(clippy::type_complexity)]
fn replicate_characters(
    q: Query<
        (Entity, Option<&Name>, Has<LocalPlayer>),
        (With<Intent>, With<Health>, Without<Replicated>, Without<Hostage>),
    >,
    settings: Res<NetSettings>,
    mut commands: Commands,
) {
    for (e, name, local) in &q {
        let name = if local {
            settings.name.clone()
        } else {
            name.map_or_else(|| "Character".into(), |n| n.as_str().to_string())
        };
        commands.entity(e).insert((
            Replicated,
            NetCharacter {
                owner: local.then_some(HOST_ID),
                name,
            },
            NetBody::default(),
        ));
    }
}

/// The simulation's state of each character into what replicates.
fn write_bodies(mut q: Query<(&Transform, &Velocity, &MovementState, &Intent, Has<Dead>, &mut NetBody)>) {
    for (t, v, s, i, dead, mut body) in &mut q {
        let mut flags = 0;
        for (on, bit) in [
            (s.on_ground, body_flags::ON_GROUND),
            (s.crouching, body_flags::CROUCHING),
            (s.on_ladder, body_flags::ON_LADDER),
            (dead, body_flags::DEAD),
        ] {
            if on {
                flags |= bit;
            }
        }
        let now = NetBody {
            origin: t.translation.to_array(),
            velocity: v.0.to_array(),
            yaw: i.yaw,
            pitch: i.pitch,
            eye: s.eye_offset.to_array(),
            flags,
        };
        body.set_if_neq(now);
    }
}

/// A connected player, for `status` and the scoreboard.
#[derive(Clone, Debug)]
pub struct PlayerInfo {
    pub id: u64,
    pub name: String,
    /// Round trip, ms (None for the host).
    pub ping_ms: Option<f64>,
    pub loss: f64,
    /// Commands waiting for later ticks, and ticks run without one
    /// (`CommandBuffer`); 0 for the host.
    pub buffered: usize,
    pub missed: u32,
}

/// The host (on a listen server) and every joined client.
pub fn players(world: &mut World) -> Vec<PlayerInfo> {
    let mut out = Vec::new();
    let host = world.resource::<NetSettings>().name.clone();
    if world
        .query_filtered::<(), With<LocalPlayer>>()
        .iter(world)
        .next()
        .is_some()
    {
        out.push(PlayerInfo {
            id: HOST_ID,
            name: host,
            ping_ms: None,
            loss: 0.0,
            buffered: 0,
            missed: 0,
        });
    }
    let mut clients = Vec::new();
    for (p, id, stats) in world
        .query::<(&Player, &NetworkId, Option<&ConnectedClientStats>)>()
        .iter(world)
    {
        clients.push((
            p.character,
            PlayerInfo {
                id: id.get(),
                name: p.name.clone(),
                ping_ms: stats.map(|s| s.rtt * 1000.0),
                loss: stats.map_or(0.0, |s| s.packet_loss),
                buffered: 0,
                missed: 0,
            },
        ));
    }
    for (character, mut info) in clients {
        if let Some(b) = world.get::<CommandBuffer>(character) {
            info.buffered = b.queued.len();
            info.missed = b.missed;
        }
        out.push(info);
    }
    out
}
