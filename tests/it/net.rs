//! Network play, slice 1 (docs/plans/active/multiplayer.md): a server and
//! clients in one process (`harness::NetSim`, an in-memory link with
//! seeded latency, jitter and loss) on the greybox. Clients join, the
//! server spawns their characters, everyone sees everyone move; a
//! disconnect removes the character; another version is refused; a
//! different copy of the map is refused.

use std::time::Duration;

use bevy::prelude::*;
use mashup::{
    core::{Health, LocalPlayer, NetRole, Team},
    games::cs_source::movement::{self as source, SourceMovementPlugin},
    greybox::GreyboxMapPlugin,
    harness::NetSim,
    map::{LoadedMapName, MapFile},
    net::{self, LastDisconnect, NetCharacter, NetSettings, NetVersion, client::Joined, memory::LinkConditions},
    slots::Loadout,
};

fn greybox(app: &mut App) {
    app.add_plugins((GreyboxMapPlugin, SourceMovementPlugin))
        .insert_resource(Loadout { movement: source::ID });
}

fn lan() -> LinkConditions {
    LinkConditions {
        latency: Duration::from_millis(30),
        jitter: Duration::from_millis(10),
        loss: 0.0,
    }
}

/// The listen server's own player, as the game's local player.
fn host(sim: &mut NetSim) -> Entity {
    let e = sim.server.spawn_character(Vec3::new(6.0, 1.0, 0.0), source::ID);
    // A counter-terrorist, as the game's player starts.
    sim.server.app.world_mut().entity_mut(e).insert((LocalPlayer, Team(2)));
    e
}

fn pos(world: &World, e: Entity) -> Vec3 {
    world.get::<Transform>(e).unwrap().translation
}

/// Client `viewer`'s copy of the character owned by `owner`.
fn seen_by(sim: &mut NetSim, viewer: usize, owner: u64) -> Option<Entity> {
    NetSim::owned_by(&mut sim.clients[viewer], Some(owner))
}

#[test]
fn clients_join_and_see_each_other_move() {
    let mut sim = NetSim::new(lan(), 1, 2, greybox);
    let host = host(&mut sim);
    sim.until_joined(200);

    // The server made a character per client, on both teams (the host is
    // a counter-terrorist: the first client joins the terrorists).
    let a = sim.character_of(0).expect("server character for client 1");
    let b = sim.character_of(1).expect("server character for client 2");
    let teams = |e| *sim.server.app.world().get::<Team>(e).unwrap();
    assert_eq!(teams(a), Team(1));
    assert_eq!(teams(b), Team(2));
    // Respawned by the rules at a spawn point once its client said it has
    // the map (a round trip), and drawn there by the others (in the past).
    sim.ticks(40);
    assert!(sim.server.app.world().get::<mashup::rules::Dead>(a).is_none());

    // Each client sees all three, its own as its local player.
    for i in 0..2 {
        let world = sim.clients[i].app.world_mut();
        let n = world.query::<&NetCharacter>().iter(world).count();
        assert_eq!(n, 3, "client {i} sees {n} characters");
        let me = sim.local_player(i).unwrap();
        let owner = sim.clients[i].app.world().get::<NetCharacter>(me).unwrap().owner;
        assert_eq!(owner, Some(sim.client_id(i)));
    }
    // Team and health replicate.
    let a_on_2 = seen_by(&mut sim, 1, 1).unwrap();
    assert_eq!(*sim.clients[1].app.world().get::<Team>(a_on_2).unwrap(), Team(1));
    assert_eq!(sim.clients[1].app.world().get::<Health>(a_on_2).unwrap().current, 1.0);

    // Client 1 walks forward: the server moves its character, client 2
    // sees it move, and so does client 1 (drawn from the server's state).
    let a_local = sim.local_player(0).unwrap();
    let start_server = pos(sim.server.app.world(), a);
    let start_seen = pos(sim.clients[1].app.world(), a_on_2);
    assert!(
        start_server.distance(start_seen) < 0.01,
        "{start_server} vs {start_seen}"
    );
    sim.clients[0]
        .app
        .world_mut()
        .get_mut::<mashup::core::Intent>(a_local)
        .unwrap()
        .move_axis = Vec2::Y;
    sim.ticks(64);
    sim.clients[0]
        .app
        .world_mut()
        .get_mut::<mashup::core::Intent>(a_local)
        .unwrap()
        .move_axis = Vec2::ZERO;
    // Stopped (friction), and others are drawn 0.1 s and the latency in
    // the past (`net::interp`).
    sim.ticks(80);
    let moved = pos(sim.server.app.world(), a).distance(start_server);
    assert!(moved > 2.0, "the server moved client 1's character {moved} m");
    let end_server = pos(sim.server.app.world(), a);
    for (viewer, e) in [(0, a_local), (1, a_on_2)] {
        let seen = pos(sim.clients[viewer].app.world(), e);
        assert!(
            seen.distance(end_server) < 0.01,
            "client {} sees it at {seen}, the server has {end_server}",
            viewer + 1
        );
    }

    // The host walks: both clients see it.
    let host_start = pos(sim.server.app.world(), host);
    sim.server.intent(host).move_axis = Vec2::X;
    sim.ticks(64);
    sim.server.intent(host).move_axis = Vec2::ZERO;
    sim.ticks(80);
    let host_end = pos(sim.server.app.world(), host);
    assert!(host_end.distance(host_start) > 2.0);
    for viewer in 0..2 {
        let e = seen_by(&mut sim, viewer, net::HOST_ID).unwrap();
        assert!(pos(sim.clients[viewer].app.world(), e).distance(host_end) < 0.01);
    }
}

#[test]
fn looks_replicate_to_others() {
    let mut sim = NetSim::new(lan(), 2, 2, greybox);
    sim.until_joined(200);
    let me = sim.local_player(0).unwrap();
    {
        let world = sim.clients[0].app.world_mut();
        let mut i = world.get_mut::<mashup::core::Intent>(me).unwrap();
        i.yaw = 1.0;
        i.pitch = -0.25;
        i.crouch = true;
    }
    sim.ticks(30);
    let other = seen_by(&mut sim, 1, 1).unwrap();
    let world = sim.clients[1].app.world();
    let i = world.get::<mashup::core::Intent>(other).unwrap();
    assert!(
        (i.yaw - 1.0).abs() < 1e-6 && (i.pitch + 0.25).abs() < 1e-6,
        "{} {}",
        i.yaw,
        i.pitch
    );
    assert!(world.get::<mashup::core::MovementState>(other).unwrap().crouching);
}

#[test]
fn a_disconnect_removes_the_character_everywhere() {
    let mut sim = NetSim::new(lan(), 3, 2, greybox);
    sim.until_joined(200);
    assert!(sim.character_of(1).is_some());
    net::disconnect(sim.clients[1].app.world_mut(), "Disconnect by user.");
    sim.ticks(30);
    assert!(sim.character_of(1).is_none(), "the server dropped client 2's character");
    assert!(seen_by(&mut sim, 0, 2).is_none(), "client 1 no longer sees it");
    assert!(seen_by(&mut sim, 0, 1).is_some(), "client 1 still sees itself");
    // The leaver is back to single player with nothing from the server.
    let world = sim.clients[1].app.world_mut();
    assert_eq!(*world.resource::<NetRole>(), NetRole::Standalone);
    assert_eq!(world.query::<&NetCharacter>().iter(world).count(), 0);

    // The server stopping drops the rest.
    net::disconnect(sim.server.app.world_mut(), "Server shutting down.");
    sim.ticks(10);
    let world = sim.clients[0].app.world_mut();
    assert_eq!(*world.resource::<NetRole>(), NetRole::Standalone);
    assert_eq!(world.query::<&NetCharacter>().iter(world).count(), 0);
}

#[test]
fn another_version_is_refused_with_a_reason() {
    let mut sim = NetSim::new(lan(), 4, 0, greybox);
    sim.add_client(|w| w.insert_resource(NetVersion("0.0.1/net0".into())));
    sim.ticks(60);
    let world = sim.clients[0].app.world();
    assert_eq!(*world.resource::<NetRole>(), NetRole::Standalone);
    let reason = &world.resource::<LastDisconnect>().0;
    assert!(reason.contains("version 0.0.1/net0"), "{reason}");
    assert!(!world.contains_resource::<Joined>());
    assert!(sim.character_of(0).is_none());
}

#[test]
fn a_full_server_refuses() {
    let mut sim = NetSim::new(lan(), 5, 0, greybox);
    host(&mut sim);
    // The host and one more.
    sim.server.app.world_mut().resource_mut::<NetSettings>().maxplayers = 2;
    sim.add_client(|_| {});
    sim.until_joined(200);
    sim.add_client(|_| {});
    sim.ticks(60);
    let reason = &sim.clients[1].app.world().resource::<LastDisconnect>().0;
    assert_eq!(reason, "Server is full.");
    assert!(sim.character_of(0).is_some() && sim.character_of(1).is_none());
}

/// Both sides "loaded" `cs_source:test_map` with these file hashes.
fn on_map(world: &mut World, hash: u8) {
    world.insert_resource(LoadedMapName("cs_source:test_map".into()));
    world.insert_resource(MapFile {
        name: "test_map".into(),
        hash: Some([hash; 32]),
    });
}

#[test]
fn the_map_must_be_the_same_file() {
    let mut sim = NetSim::new(lan(), 6, 0, greybox);
    on_map(sim.server.app.world_mut(), 7);
    sim.add_client(|w| on_map(w, 7));
    sim.add_client(|w| on_map(w, 8));
    sim.ticks(60);
    assert!(sim.clients[0].app.world().contains_resource::<Joined>());
    let world = sim.clients[1].app.world();
    assert_eq!(*world.resource::<NetRole>(), NetRole::Standalone);
    let reason = &world.resource::<LastDisconnect>().0;
    assert!(reason.contains("differs from the server's"), "{reason}");
    sim.ticks(10);
    assert!(sim.character_of(1).is_none());
}

#[test]
fn a_lossy_link_still_converges() {
    let conditions = LinkConditions {
        latency: Duration::from_millis(60),
        jitter: Duration::from_millis(30),
        loss: 0.2,
    };
    let mut sim = NetSim::new(conditions, 7, 2, greybox);
    sim.until_joined(600);
    let me = sim.local_player(0).unwrap();
    sim.clients[0]
        .app
        .world_mut()
        .get_mut::<mashup::core::Intent>(me)
        .unwrap()
        .move_axis = Vec2::Y;
    sim.ticks(64);
    sim.clients[0]
        .app
        .world_mut()
        .get_mut::<mashup::core::Intent>(me)
        .unwrap()
        .move_axis = Vec2::ZERO;
    sim.ticks(100);
    let a = sim.character_of(0).unwrap();
    let truth = pos(sim.server.app.world(), a);
    let seen = seen_by(&mut sim, 1, 1).unwrap();
    let there = pos(sim.clients[1].app.world(), seen);
    assert!(there.distance(truth) < 0.01, "{there} vs {truth}");
    assert!(sim.link.lock().unwrap().lost > 0, "the link dropped packets");
}

/// The real transport: netcode over UDP on loopback, on a port the system
/// picks (never 27015: a CS:S server may hold it).
#[test]
fn netcode_over_loopback() {
    let make = || {
        mashup::harness::Sim::with(|app| {
            app.add_plugins(net::NetPlugin);
            greybox(app);
        })
    };
    let mut server = make();
    {
        let world = server.app.world_mut();
        let mut s = world.resource_mut::<NetSettings>();
        s.hostport = 0;
        s.maxplayers = 4;
    }
    let addr = net::server::listen(server.app.world_mut()).expect("listen");
    assert_ne!(addr.port(), 0);
    let mut client = make();
    let to = std::net::SocketAddr::from(([127, 0, 0, 1], addr.port()));
    net::client::connect(client.app.world_mut(), to).expect("connect");
    let mut joined = false;
    for _ in 0..400 {
        server.app.update();
        client.app.update();
        std::thread::sleep(Duration::from_millis(1));
        let world = client.app.world_mut();
        let mine = world.query_filtered::<(), With<LocalPlayer>>().iter(world).count();
        if world.contains_resource::<Joined>() && mine == 1 {
            joined = true;
            break;
        }
    }
    assert!(joined, "joined over UDP");
    let status = net::status(server.app.world_mut());
    assert!(status.contains("players : 1 (4 max)"), "{status}");
    // Leaving reaches the server (netcode's disconnect packet).
    net::disconnect(client.app.world_mut(), "Disconnect by user.");
    let mut gone = false;
    for _ in 0..200 {
        server.app.update();
        std::thread::sleep(Duration::from_millis(1));
        let world = server.app.world_mut();
        if world.query::<&NetCharacter>().iter(world).count() == 0 {
            gone = true;
            break;
        }
    }
    assert!(gone, "the server dropped the character");
    net::disconnect(server.app.world_mut(), "done");
}

/// `net_fakelag` on a client's UDP transport: the round trip grows by it,
/// and the game still joins and predicts.
#[test]
fn fake_lag_delays_what_the_client_receives() {
    let make = || {
        mashup::harness::Sim::with(|app| {
            app.add_plugins(net::NetPlugin);
            greybox(app);
        })
    };
    let mut server = make();
    {
        let world = server.app.world_mut();
        let mut s = world.resource_mut::<NetSettings>();
        s.hostport = 0;
        s.maxplayers = 4;
    }
    let addr = net::server::listen(server.app.world_mut()).expect("listen");
    let mut client = make();
    client.app.world_mut().resource_mut::<net::udp::FakeLag>().lag = 80.0;
    let to = std::net::SocketAddr::from(([127, 0, 0, 1], addr.port()));
    net::client::connect(client.app.world_mut(), to).expect("connect");
    let mut rtt = 0.0;
    let start = std::time::Instant::now();
    while start.elapsed() < Duration::from_secs(6) {
        server.app.update();
        client.app.update();
        std::thread::sleep(Duration::from_millis(2));
        let world = client.app.world();
        let predicting = world.resource::<net::predict::CommandClock>().tick.is_some();
        rtt = world
            .get_resource::<bevy_replicon_renet::RenetClient>()
            .map_or(0.0, |c| c.rtt());
        if predicting && rtt > 0.08 {
            break;
        }
    }
    assert!(rtt > 0.08, "round trip {rtt} s with 80 ms of fake lag");
    assert!(
        client
            .app
            .world()
            .resource::<net::predict::CommandClock>()
            .tick
            .is_some()
    );
    net::disconnect(client.app.world_mut(), "done");
    net::disconnect(server.app.world_mut(), "done");
}
