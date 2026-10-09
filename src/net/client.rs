//! The client side: connecting, the join handshake, characters the
//! server sends (ours marked `LocalPlayer`). Commands and prediction:
//! `predict`.

use std::{
    net::{Ipv4Addr, SocketAddr, UdpSocket},
    time::SystemTime,
};

use bevy::prelude::*;
use bevy_replicon::prelude::*;
use bevy_replicon_renet::{RenetClient, netcode::ClientAuthentication};

use super::{
    Join, NetBody, NetCharacter, NetEvent, NetSettings, NetVersion, PROTOCOL_ID, Refused, Welcome, body_flags,
};
use crate::{
    character::character_bundle,
    map::interp::NetDrawn,
    core::{Intent, LocalPlayer, MovementState, NetRole, Team, Velocity},
    rules::Dead,
    weapon::Inventory,
};

pub(super) fn plugin(app: &mut App) {
    app.add_observer(refused)
        .add_observer(welcome)
        .add_observer(character_arrived)
        .add_systems(OnEnter(ClientState::Connected), send_join)
        .add_systems(
            PreUpdate,
            (watch_connection, apply_bodies)
                .chain()
                .after(ClientSystems::Receive)
                .run_if(resource_equals(NetRole::Client)),
        )
        .add_systems(Update, check_map.run_if(resource_exists::<Handshake>));
}

/// This client's id (its characters' `NetCharacter::owner`).
#[derive(Resource, Clone, Copy, Debug, PartialEq, Eq)]
pub struct LocalClientId(pub u64);

/// The server this client connects to (for `status`).
#[derive(Resource, Clone, Copy, Debug)]
pub struct ServerAddress(pub SocketAddr);

/// The server's welcome, until its map is checked.
#[derive(Resource, Clone, Debug)]
pub struct Handshake {
    pub welcome: Welcome,
    /// `NetEvent::LoadMap` already asked for.
    asked: bool,
}

/// In the game: the map matches the server's.
#[derive(Resource, Clone, Debug)]
pub struct Joined {
    pub map: String,
}

/// Why the server refused us, to report when it drops us.
#[derive(Resource, Clone, Debug)]
struct RefusedReason(String);

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
    let auth = ClientAuthentication::Unsecure {
        client_id: id,
        protocol_id: PROTOCOL_ID,
        server_addr: server,
        user_data: None,
    };
    let transport = super::udp::UdpClient::new(now, auth, socket).map_err(|e| e.to_string())?;
    start(world, id)?;
    world.insert_resource(transport);
    world.insert_resource(ServerAddress(server));
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
    super::predict::reset(world);
    super::interp::reset(world);
    let remote: Vec<Entity> = world.query_filtered::<Entity, With<Remote>>().iter(world).collect();
    for e in remote {
        if let Ok(e) = world.get_entity_mut(e) {
            e.despawn();
        }
    }
}

/// Connected: say who we are.
fn send_join(
    mut commands: Commands,
    version: Res<NetVersion>,
    protocol: Res<ProtocolHash>,
    settings: Res<NetSettings>,
    role: Res<NetRole>,
) {
    if *role != NetRole::Client {
        return;
    }
    commands.client_trigger(Join {
        version: version.0.clone(),
        protocol: *protocol,
        name: settings.name.clone(),
    });
}

fn refused(r: On<Refused>, mut commands: Commands) {
    warn!("the server refused us: {}", r.reason);
    commands.insert_resource(RefusedReason(r.reason.clone()));
}

fn welcome(w: On<Welcome>, mut commands: Commands) {
    info!("server: map {}, tick {:.4} s", w.map, w.tick_nanos as f64 * 1e-9);
    commands.insert_resource(Handshake {
        welcome: w.event().clone(),
        asked: false,
    });
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
    super::disconnect(world, &reason);
}

/// The server's map loaded here and the same file: in the game. Another
/// map: ask the client layer to load it (`NetEvent::LoadMap`); a different
/// file by the same name: leave.
fn check_map(world: &mut World) {
    let Some(h) = world.get_resource::<Handshake>().cloned() else {
        return;
    };
    let map = h.welcome.map.clone();
    match super::map_matches(world, &map, h.welcome.map_hash) {
        Some(true) => {
            world.insert_resource(Time::<Fixed>::from_duration(std::time::Duration::from_nanos(
                h.welcome.tick_nanos,
            )));
            world.remove_resource::<Handshake>();
            world.insert_resource(Joined { map: map.clone() });
            world.write_message(NetEvent::Joined { map });
        }
        Some(false) => {
            super::disconnect(
                world,
                &format!("Your copy of {map} differs from the server's (another version of the map)."),
            );
        }
        None if !h.asked => {
            world.resource_mut::<Handshake>().asked = true;
            world.write_message(NetEvent::LoadMap(map));
        }
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
