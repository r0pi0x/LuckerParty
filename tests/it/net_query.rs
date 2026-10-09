//! Finding games (multiplayer slice 8): server queries answered on a
//! server's own UDP port, next to the game connection; LAN discovery
//! over broadcast and this machine; the server browser's saved lists.
//! Real sockets on loopback, ports 27031 and up or the system's own
//! (never 27015: a CS:S server may hold it).

use std::{
    net::{Ipv4Addr, SocketAddr, UdpSocket},
    time::{Duration, Instant},
};

use bevy::prelude::*;
use mashup::{
    bot::Bot,
    client::server_browser::ServerBrowser,
    core::{LocalPlayer, Team},
    games::cs_source::movement::{self as source, SourceMovementPlugin},
    greybox::GreyboxMapPlugin,
    harness::Sim,
    net::{
        self, LastDisconnect, NET_VERSION, NetSettings,
        client::Joined,
        query::{GAME_FOLDER, Hosting, JoinPassword, QueryState, ServerQueries},
    },
    slots::Loadout,
};

fn server(hostname: &str, port: u16, maxplayers: u32) -> Result<(Sim, SocketAddr), String> {
    let mut sim = Sim::with(|app| {
        app.add_plugins((net::NetPlugin, GreyboxMapPlugin, SourceMovementPlugin))
            .insert_resource(Loadout { movement: source::ID });
    });
    {
        let world = sim.app.world_mut();
        let mut s = world.resource_mut::<NetSettings>();
        s.hostport = port;
        s.maxplayers = maxplayers;
        world.resource_mut::<Hosting>().hostname = hostname.into();
    }
    let addr = net::server::listen(sim.app.world_mut())?;
    Ok((sim, SocketAddr::from((Ipv4Addr::LOCALHOST, addr.port()))))
}

/// Step the servers and poll the queries until `done` or `secs` pass.
fn run(
    servers: &mut [&mut Sim],
    queries: &mut ServerQueries,
    secs: f64,
    mut done: impl FnMut(&ServerQueries) -> bool,
) -> bool {
    let start = Instant::now();
    while start.elapsed().as_secs_f64() < secs {
        for s in servers.iter_mut() {
            s.app.update();
        }
        queries.poll();
        if done(queries) {
            return true;
        }
        std::thread::sleep(Duration::from_millis(1));
    }
    false
}

#[test]
fn a_server_answers_queries_with_its_info() {
    let (mut sv, addr) = server("Query test", 0, 6).expect("listen");
    // Two players: a bot and someone else's character.
    let bot = sv.spawn_character(Vec3::new(4.0, 1.0, 0.0), source::ID);
    sv.app.world_mut().entity_mut(bot).insert((Team(1), Bot::default()));
    let other = sv.spawn_character(Vec3::new(-4.0, 1.0, 0.0), source::ID);
    sv.app.world_mut().entity_mut(other).insert(Team(2));
    sv.app.world_mut().resource_mut::<Hosting>().password = "letmein".into();

    let mut q = ServerQueries::default();
    q.query(addr);
    assert!(
        run(&mut [&mut sv], &mut q, 5.0, |q| !q.busy()),
        "the server answered within 5 s"
    );
    let Some(QueryState::Answered { info, ping_ms }) = q.result(addr).map(|r| r.state.clone()) else {
        panic!("answered: {:?}", q.results);
    };
    assert_eq!(info.name, "Query test");
    assert_eq!(info.map, "greybox");
    assert_eq!(info.folder, GAME_FOLDER);
    assert_eq!((info.players, info.bots, info.max_players), (2, 1, 6));
    assert!(info.dedicated, "no player of its own: dedicated");
    assert!(info.password);
    assert_eq!(info.version, NET_VERSION);
    assert_eq!(info.port, Some(addr.port()));
    assert!(ping_ms < 1000, "a round trip on loopback: {ping_ms} ms");
    let status = net::status(sv.app.world_mut());
    assert!(status.contains("hostname: Query test"), "{status}");
    assert!(status.contains("queries : 1 answered, 1 challenged"), "{status}");

    // A host's own player: a listen server.
    let host = sv.spawn_character(Vec3::new(0.0, 1.0, 4.0), source::ID);
    sv.app.world_mut().entity_mut(host).insert(LocalPlayer);
    q.query(addr);
    run(&mut [&mut sv], &mut q, 5.0, |q| !q.busy());
    let Some(QueryState::Answered { info, .. }) = q.result(addr).map(|r| r.state.clone()) else {
        panic!("answered again");
    };
    assert!(!info.dedicated && info.players == 3);

    // The game connection on the same port: the password is checked.
    let join = |password: &str, sv: &mut Sim| -> Result<(), String> {
        let mut client = Sim::with(|app| {
            app.add_plugins((net::NetPlugin, GreyboxMapPlugin, SourceMovementPlugin))
                .insert_resource(Loadout { movement: source::ID });
        });
        client.app.world_mut().resource_mut::<JoinPassword>().0 = password.into();
        net::client::connect(client.app.world_mut(), addr)?;
        let start = Instant::now();
        while start.elapsed() < Duration::from_secs(5) {
            sv.app.update();
            client.app.update();
            std::thread::sleep(Duration::from_millis(1));
            let w = client.app.world_mut();
            if w.contains_resource::<Joined>() {
                net::disconnect(w, "done");
                return Ok(());
            }
            if let Some(why) = w.get_resource::<LastDisconnect>() {
                return Err(why.0.clone());
            }
        }
        Err("timed out".into())
    };
    let refused = join("wrong", &mut sv).expect_err("a wrong password is refused");
    assert!(refused.contains("Bad password"), "{refused}");
    join("letmein", &mut sv).expect("the right password joins");
}

/// Two ports next to each other, 27031 and up, free now.
fn free_pair() -> u16 {
    let base = 27031 + (std::process::id() % 20) as u16 * 2;
    (0..60)
        .map(|k| base + k * 2)
        .find(|p| {
            UdpSocket::bind((Ipv4Addr::UNSPECIFIED, *p)).is_ok()
                && UdpSocket::bind((Ipv4Addr::UNSPECIFIED, p + 1)).is_ok()
        })
        .expect("two free ports")
}

#[test]
fn lan_discovery_finds_two_servers_on_their_ports() {
    let (mut a, mut b) = loop {
        let p = free_pair();
        match (server("LAN one", p, 4), server("LAN two", p + 1, 8)) {
            (Ok(a), Ok(b)) => break (a, b),
            // Taken between the check and the bind: another pair.
            _ => continue,
        }
    };
    let ports = (a.1.port(), b.1.port());
    let mut q = ServerQueries::default();
    q.scan_lan(ports, &["255.255.255.255".parse().unwrap()]);
    let done = run(&mut [&mut a.0, &mut b.0], &mut q, 6.0, |q| !q.busy());
    assert!(done, "the scan ended");
    let mut found: Vec<(String, u16, u8)> = q
        .results
        .iter()
        .filter(|r| r.lan)
        .filter_map(|r| match &r.state {
            QueryState::Answered { info, .. } => Some((info.name.clone(), r.addr.port(), info.max_players)),
            _ => None,
        })
        .collect();
    found.sort();
    assert_eq!(
        found,
        [("LAN one".to_string(), ports.0, 4), ("LAN two".to_string(), ports.1, 8)],
        "each server once (at its LAN address or loopback): {:?}",
        q.results
    );
    // Not one of ours on a scanned port: left out. A plain socket that
    // answers like a Source server, with another game folder.
    let other = UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
    other.set_nonblocking(true).unwrap();
    let port = other.local_addr().unwrap().port();
    let mut q = ServerQueries::default();
    q.scan_lan((port, port), &[]);
    let fake = mashup::net::query::ServerInfo {
        name: "Some Source server".into(),
        folder: "cstrike".into(),
        ..default()
    };
    let mut buf = [0u8; 1400];
    let start = Instant::now();
    while q.busy() && start.elapsed() < Duration::from_secs(4) {
        if let Ok((n, from)) = other.recv_from(&mut buf) {
            let reply = match mashup::net::query::parse_request(&buf[..n]) {
                Some(None) => mashup::net::query::challenge_reply(7),
                _ => fake.encode(),
            };
            other.send_to(&reply, from).unwrap();
        }
        q.poll();
        std::thread::sleep(Duration::from_millis(1));
    }
    assert!(
        q.results.iter().all(|r| !r.lan),
        "another game's server isn't listed: {:?}",
        q.results
    );
}

#[test]
fn favorites_and_history_persist() {
    let dir = std::env::temp_dir().join(format!("mashup-server-browser-{}", std::process::id()));
    let path = dir.join("serverbrowser.vdf");
    let mut b = ServerBrowser::default();
    let a: SocketAddr = "192.168.1.20:27031".parse().unwrap();
    let c: SocketAddr = "10.1.2.3:27032".parse().unwrap();
    b.add_favorite(a, "Friday night");
    b.played(c, "Somebody's game", 1_760_000_000);
    b.save_to(&path).expect("saved");
    let mut loaded = ServerBrowser::default();
    loaded.load_from(&path);
    assert_eq!(loaded.favorites, b.favorites);
    assert_eq!(loaded.history, b.history);
    assert_eq!(loaded.favorites[0].name, "Friday night");
    assert_eq!(loaded.history[0].last_played, 1_760_000_000);
    let _ = std::fs::remove_dir_all(dir);
}
