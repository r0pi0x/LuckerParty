//! Network play, slice 7 (docs/plans/active/multiplayer.md): maps.
//! `NetSim` with maps built in code (`harness::TestMaps`: a folder of map
//! files standing for an install, any bytes, hashed as the game hashes a
//! .bsp). A map change on the server takes two clients along (they keep
//! their connection and play the new map's game); a client joining
//! mid-game gets the whole state (round, scores, money, an opened door,
//! a broken breakable); a client without the map downloads it over the
//! connection or from `sv_downloadurl` (HTTP, a local server in the test,
//! `.bsp.bz2`), checks its hash, caches and loads it; a wrong file and a
//! server that sends nothing are refused with reasons.
//!
//! `cargo test --features dev --test it net_maps -- --nocapture` prints
//! what each client went through.

use std::{
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::Duration,
};

use bevy::prelude::*;
use mashup::{
    console::{execute, parse},
    core::{NetRole, Team},
    games::cs_source::{
        movement::{self as source, SourceMovementPlugin, to_engine},
        weapons::CsWeaponsPlugin,
    },
    greybox::GreyboxMapPlugin,
    harness::{self, NetSim, TestMaps},
    map::{MapBrush, MapData, MapEntity, MapFile, MapHull, MapPlugin},
    net::{
        LastDisconnect, NetCharacter, NetMover, NetScore, NetVersion,
        client::{JoinFailure, JoinProgress, JoinStage, Joined, LocalClientId},
        maps::MapFiles,
        memory::LinkConditions,
        mover_flags,
    },
    rules::{Score, rounds::RoundState},
    slots::Loadout,
    weapon::economy::Money,
};

const SCALE: f32 = 0.0254;

fn hull(lo: Vec3, hi: Vec3) -> MapHull {
    let b = MapBrush::from_box(lo, hi);
    let points = (0..8)
        .map(|i| {
            Vec3::new(
                if i & 1 == 0 { lo.x } else { hi.x },
                if i & 2 == 0 { lo.y } else { hi.y },
                if i & 4 == 0 { lo.z } else { hi.z },
            )
        })
        .collect();
    MapHull { planes: b.planes, points }
}

fn entity(pairs: &[(&str, &str)], hulls: Vec<MapHull>, mover: bool) -> MapEntity {
    MapEntity {
        keyvalues: pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect(),
        hulls,
        mover,
        physics: None,
    }
}

/// The test maps: a floor with team spawns; `beta` also has a door that
/// opens for good as the map starts (entity 0) and a vent broken half a
/// second in (entity 1).
fn build(name: &str) -> MapData {
    let (a, b) = (to_engine(Vec3::new(-2048.0, -2048.0, -64.0)), to_engine(Vec3::new(2048.0, 2048.0, 0.0)));
    let spawn = |x: f32, y: f32, team: u8| (to_engine(Vec3::new(x, y, 37.0)), Some(Team(team)));
    let mut d = MapData {
        collision_brushes: vec![MapBrush::from_box(a.min(b), a.max(b))],
        entity_scale: SCALE,
        ..default()
    };
    if name == "beta" {
        d.spawns = vec![spawn(-128.0, -768.0, 1), spawn(128.0, -768.0, 1), spawn(-128.0, 768.0, 2), spawn(128.0, 768.0, 2)];
        d.entities = vec![
            entity(
                &[
                    ("classname", "func_door"),
                    ("targetname", "door"),
                    ("origin", "600 0 64"),
                    ("movedir", "0 0 0"),
                    ("lip", "4"),
                    ("speed", "200"),
                    ("wait", "-1"),
                ],
                vec![hull(Vec3::new(-32.0, -4.0, -64.0), Vec3::new(32.0, 4.0, 64.0))],
                true,
            ),
            entity(
                &[
                    ("classname", "func_breakable"),
                    ("targetname", "vent"),
                    ("material", "2"),
                    ("health", "1"),
                    ("origin", "-600 0 32"),
                ],
                vec![hull(Vec3::new(-32.0, -4.0, -16.0), Vec3::new(32.0, 4.0, 16.0))],
                true,
            ),
            entity(
                &[
                    ("classname", "logic_auto"),
                    ("OnMapSpawn", "door,Open,,0,-1"),
                    ("OnMapSpawn", "vent,Break,,0.5,-1"),
                ],
                Vec::new(),
                false,
            ),
        ];
    } else {
        d.spawns = vec![spawn(-512.0, 0.0, 1), spawn(-512.0, 128.0, 1), spawn(512.0, 0.0, 2), spawn(512.0, 128.0, 2)];
    }
    d
}

/// A map file's bytes: a VBSP-looking header, the name and a version, then
/// `len` bytes from a generator (so it doesn't pack to nothing).
fn file_bytes(name: &str, version: u8, len: usize) -> Vec<u8> {
    let mut out = b"VBSP".to_vec();
    out.extend_from_slice(name.as_bytes());
    out.push(version);
    let mut x = 0x9E37_79B9u32 ^ version as u32 ^ (name.len() as u32) << 8;
    while out.len() < len {
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        // Some runs, as real maps have: bz2 packs it somewhat.
        out.push(if x % 3 == 0 { 0 } else { (x >> 24) as u8 });
    }
    out
}

/// A scratch folder for a test, emptied.
fn scratch(test: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("mashup-net-maps-{test}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn write_map(install: &Path, name: &str, bytes: &[u8]) {
    let maps = install.join("maps");
    std::fs::create_dir_all(&maps).unwrap();
    std::fs::write(maps.join(format!("{name}.bsp")), bytes).unwrap();
}

fn maps(install: &Path, cache: Option<&Path>) -> TestMaps {
    TestMaps {
        install: install.to_path_buf(),
        cache: cache.map(Path::to_path_buf),
        build: Arc::new(build),
    }
}

/// The server's install: alpha and beta (~200 KB each).
fn server_install(dir: &Path) -> PathBuf {
    let install = dir.join("server");
    write_map(&install, "alpha", &file_bytes("alpha", 1, 150_000));
    write_map(&install, "beta", &file_bytes("beta", 1, 220_000));
    install
}

fn link() -> LinkConditions {
    LinkConditions {
        latency: Duration::from_millis(30),
        jitter: Duration::from_millis(5),
        loss: 0.0,
    }
}

/// A server on `map` from `install`, with no clients yet.
fn server(install: PathBuf, map: &str, seed: u64) -> NetSim {
    let setup_install = install.clone();
    let mut sim = NetSim::new(link(), seed, 0, move |app| {
        app.add_plugins((GreyboxMapPlugin, MapPlugin::empty(), SourceMovementPlugin, CsWeaponsPlugin))
            .insert_resource(Loadout { movement: source::ID });
        harness::serve_maps(app, maps(&setup_install, None));
    });
    harness::load_level(sim.server.app.world_mut(), map, None).expect("server map");
    sim.ticks(2);
    sim
}

/// A client whose install is `install` and content cache `cache`.
fn add_client(sim: &mut NetSim, install: &Path, cache: Option<&Path>) -> usize {
    let m = maps(install, cache);
    sim.add_client(move |w| {
        let read = {
            let m = m.clone();
            Arc::new(move |id: &str| std::fs::read(m.file(id)).ok())
        };
        w.insert_resource(MapFiles {
            read,
            cache: m.cache.clone(),
        });
        w.insert_resource(m);
    })
}

fn run(world: &mut World, line: &str) {
    for words in parse(line) {
        execute(world, &words, 0);
    }
}

fn progress(sim: &NetSim, i: usize) -> JoinProgress {
    sim.clients[i].app.world().resource::<JoinProgress>().clone()
}

fn stages(p: &JoinProgress) -> Vec<JoinStage> {
    p.log.iter().map(|(s, _)| *s).collect()
}

fn loaded(sim: &NetSim, i: usize) -> Option<MapFile> {
    sim.clients[i].app.world().get_resource::<MapFile>().cloned()
}

#[test]
fn changelevel_takes_two_clients_along() {
    let dir = scratch("changelevel");
    let install = server_install(&dir);
    let mut sim = server(install.clone(), "test:alpha", 71);
    run(sim.server.app.world_mut(), "mashup_rounds 1");
    for _ in 0..2 {
        add_client(&mut sim, &install, None);
    }
    sim.until_joined(600);
    let ids: Vec<u64> = (0..2)
        .map(|i| sim.clients[i].app.world().resource::<LocalClientId>().0)
        .collect();
    for i in 0..2 {
        assert_eq!(loaded(&sim, i).unwrap().name, "alpha");
    }
    run(sim.server.app.world_mut(), "mp_restartgame 1");
    sim.ticks(200);
    // Some score to lose at the map change.
    let a = sim.character_of(0).unwrap();
    sim.server.app.world_mut().entity_mut(a).insert((Score { kills: 2, deaths: 1 }, Money(4321)));
    sim.ticks(20);
    let me = sim.local_player(0).unwrap();
    assert_eq!(sim.clients[0].app.world().get::<NetScore>(me).unwrap().kills, 2);

    // The server starts changing level: its clients hear it.
    harness::begin_level_change(sim.server.app.world_mut(), "test:beta");
    sim.ticks(20);
    for i in 0..2 {
        assert_eq!(progress(&sim, i).stage, JoinStage::ChangingLevel, "client {i} told");
    }
    // In: every client loads it and plays on, on the same connection.
    harness::load_level(sim.server.app.world_mut(), "test:beta", None).unwrap();
    let ok = sim.until(600, |s| {
        (0..2).all(|i| {
            s.clients[i]
                .app
                .world()
                .get_resource::<Joined>()
                .is_some_and(|j| j.map == "test:beta")
        })
    });
    assert!(ok, "both clients followed to beta");
    for i in 0..2 {
        let w = sim.clients[i].app.world();
        assert_eq!(*w.resource::<NetRole>(), NetRole::Client);
        assert!(w.get_resource::<LastDisconnect>().is_none(), "client {i} never dropped");
        assert_eq!(w.resource::<LocalClientId>().0, ids[i], "the same connection");
        assert_eq!(loaded(&sim, i).unwrap().name, "beta");
        let p = progress(&sim, i);
        println!("client {i}: {:?}", stages(&p));
        assert!(stages(&p).ends_with(&[JoinStage::ChangingLevel, JoinStage::Verifying, JoinStage::LoadingMap, JoinStage::Joined]));
    }
    sim.ticks(120);
    // Predicting again at once: the client started its own states over
    // (`predict::OwnStateBases`), and says so even before it has commands
    // to send, so the server sends it a whole state rather than deltas
    // against one it dropped.
    for i in 0..2 {
        let g = sim.clients[i].app.world().resource::<mashup::net::predict::NetGraph>().clone();
        assert!(g.checked >= 10, "client {i} predicts on beta: {} states compared", g.checked);
    }
    // A new game on beta: the round restarted, scores gone, everyone at
    // beta's spawns (|y| ~ 768 units).
    let round = sim.server.app.world().resource::<RoundState>().clone();
    println!("server round after the change: {round:?}");
    assert_eq!(round.wins, [0, 0]);
    for i in 0..2 {
        let c = sim.character_of(i).unwrap();
        let at = sim.server.app.world().get::<Transform>(c).unwrap().translation;
        assert!(((at.z / SCALE).abs() - 768.0).abs() < 64.0, "client {i}'s character at a beta spawn: {at}");
        let me = sim.local_player(i).unwrap();
        let w = sim.clients[i].app.world();
        assert_eq!(w.get::<NetScore>(me).unwrap().kills, 0, "scores start over");
        assert_eq!(w.resource::<RoundState>().number, round.number, "the client's round is the server's");
        assert_eq!(w.resource::<RoundState>().wins, [0, 0]);
    }
    // Money starts over too (CS:S's map change is a new game).
    for i in 0..2 {
        let c = sim.character_of(i).unwrap();
        let start = sim.server.app.world().resource::<mashup::rules::rounds::RoundSettings>().start_money;
        assert_eq!(sim.server.app.world().get::<Money>(c).map(|m| m.0), Some(start), "client {i}'s money");
    }
    // And it plays there: client 0 walks, the server moves it.
    let c = sim.character_of(0).unwrap();
    let before = sim.server.app.world().get::<Transform>(c).unwrap().translation;
    run(sim.server.app.world_mut(), "mp_restartgame 1");
    let ok = sim.until(800, |s| {
        let w = s.clients[0].app.world();
        !w.get_resource::<mashup::core::FreezeTime>().is_some_and(|f| f.0)
            && w.get_resource::<RoundState>().is_some_and(|r| matches!(r.phase, mashup::rules::rounds::Phase::Live { .. }))
    });
    assert!(ok, "the round went live");
    let me = sim.local_player(0).unwrap();
    sim.clients[0].app.world_mut().get_mut::<mashup::core::Intent>(me).unwrap().move_axis = Vec2::Y;
    sim.ticks(64);
    sim.clients[0].app.world_mut().get_mut::<mashup::core::Intent>(me).unwrap().move_axis = Vec2::ZERO;
    sim.ticks(30);
    let after = sim.server.app.world().get::<Transform>(c).unwrap().translation;
    let graph = sim.clients[0].app.world().resource::<mashup::net::predict::NetGraph>().clone();
    println!(
        "client 0 walked {:.2} m on beta; prediction: {} of {} wrong",
        after.distance(before),
        graph.errors,
        graph.checked
    );
    assert!(after.distance(before) > 1.0, "the server moved it");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_late_joiner_gets_the_whole_state() {
    let dir = scratch("late");
    let install = server_install(&dir);
    let mut sim = server(install.clone(), "test:beta", 72);
    run(sim.server.app.world_mut(), "mashup_rounds 1; bot_add 1; bot_add 2");
    sim.ticks(200);
    // A game under way: the round score, a bot's score.
    {
        let w = sim.server.app.world_mut();
        w.resource_mut::<RoundState>().wins = [3, 2];
        let bot = w
            .query_filtered::<Entity, With<mashup::bot::Bot>>()
            .iter(w)
            .next()
            .unwrap();
        w.entity_mut(bot).insert(Score { kills: 4, deaths: 1 });
    }
    sim.ticks(10);
    // The door opened and the vent broke on the server.
    let server_movers: Vec<NetMover> = {
        let w = sim.server.app.world_mut();
        w.query::<&NetMover>().iter(w).cloned().collect()
    };
    let door = server_movers.iter().find(|m| m.index == 0).expect("the door replicates").clone();
    let vent = server_movers.iter().find(|m| m.index == 1).expect("the vent replicates").clone();
    assert!(door.origin[0] > 600.0 + 50.0, "the door opened: {:?}", door.origin);
    assert!(vent.flags & mover_flags::GONE != 0, "the vent broke");

    let i = add_client(&mut sim, &install, None);
    sim.until_joined(600);
    let c = sim.character_of(i).unwrap();
    sim.server.app.world_mut().entity_mut(c).insert(Money(4321));
    sim.ticks(40);
    let w = sim.clients[i].app.world_mut();
    let wins = w.resource::<RoundState>().wins;
    let money = w.query_filtered::<&Money, With<mashup::core::LocalPlayer>>().single(w).unwrap().0;
    let movers: Vec<NetMover> = w.query::<&NetMover>().iter(w).cloned().collect();
    let scores: Vec<(String, i32, u32)> = w
        .query::<(&NetCharacter, &NetScore)>()
        .iter(w)
        .map(|(c, s)| (c.name.clone(), s.kills, s.deaths))
        .collect();
    println!("late joiner: wins {wins:?}, money {money}, scores {scores:?}, movers {movers:?}");
    assert_eq!(wins, [3, 2], "the round score");
    assert_eq!(money, 4321, "its own money");
    assert!(scores.iter().any(|s| s.1 == 4 && s.2 == 1), "the bot's score");
    assert_eq!(scores.len(), 3, "both bots and itself");
    let door_seen = movers.iter().find(|m| m.index == 0).expect("the door");
    assert_eq!(door_seen.origin, door.origin, "the door where the server has it");
    assert!(
        movers.iter().find(|m| m.index == 1).is_some_and(|m| m.flags & mover_flags::GONE != 0),
        "the vent broken"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// Client 0 has no maps at all (an empty install, an empty cache).
fn joins_without_the_map(sim: &mut NetSim, dir: &Path) -> (usize, PathBuf) {
    let empty = dir.join("client");
    std::fs::create_dir_all(&empty).unwrap();
    let cache = dir.join("cache");
    let i = add_client(sim, &empty, Some(&cache));
    (i, cache)
}

#[test]
fn a_client_without_the_map_downloads_it_over_the_connection() {
    let dir = scratch("download");
    let install = server_install(&dir);
    let mut sim = server(install.clone(), "test:beta", 73);
    let (i, cache) = joins_without_the_map(&mut sim, &dir);
    let mut most = 0;
    let ok = sim.until(2000, |s| {
        if let Some(d) = &s.clients[i].app.world().resource::<JoinProgress>().download {
            most = most.max(d.done);
        }
        s.clients[i].app.world().contains_resource::<Joined>()
    });
    let p = progress(&sim, i);
    println!("client: {:?}, failure {:?}", stages(&p), p.failure);
    assert!(ok, "joined after downloading");
    assert!(stages(&p).contains(&JoinStage::Downloading));
    let file = cache.join("maps/beta.bsp");
    assert_eq!(std::fs::read(&file).unwrap(), std::fs::read(install.join("maps/beta.bsp")).unwrap());
    assert_eq!(loaded(&sim, i).unwrap().name, "beta");
    assert!(std::fs::read_to_string(cache.join("index.toml")).unwrap().contains("sha256"));
    println!("downloaded {} bytes over the connection", most);
    // Joining again: the cached copy, no download.
    mashup::net::disconnect(sim.clients[i].app.world_mut(), "Disconnect by user.");
    sim.ticks(20);
    let j = add_client(&mut sim, &dir.join("client"), Some(&cache));
    let ok = sim.until(600, |s| s.clients[j].app.world().contains_resource::<Joined>());
    assert!(ok, "joined from the cache");
    assert!(!stages(&progress(&sim, j)).contains(&JoinStage::Downloading), "no second download");
    let _ = std::fs::remove_dir_all(&dir);
}

/// A tiny HTTP server for the test: answers each request from `files`
/// (path -> bytes, else 404) and notes the paths asked for.
fn http_server(files: Vec<(String, Vec<u8>)>) -> (u16, Arc<Mutex<Vec<String>>>) {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let asked = Arc::new(Mutex::new(Vec::new()));
    let log = asked.clone();
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut s) = stream else { continue };
            let mut buf = Vec::new();
            let mut byte = [0u8; 1];
            while !buf.ends_with(b"\r\n\r\n") && s.read(&mut byte).is_ok_and(|n| n == 1) {
                buf.push(byte[0]);
            }
            let req = String::from_utf8_lossy(&buf).to_string();
            let path = req.split_whitespace().nth(1).unwrap_or("").to_string();
            log.lock().unwrap().push(path.clone());
            match files.iter().find(|(p, _)| *p == path) {
                Some((_, body)) => {
                    let _ = write!(s, "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len());
                    let _ = s.write_all(body);
                }
                None => {
                    let _ = s.write_all(b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
                }
            }
        }
    });
    (port, asked)
}

fn bz2(bytes: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    let mut enc = bzip2::write::BzEncoder::new(&mut out, bzip2::Compression::default());
    enc.write_all(bytes).unwrap();
    enc.finish().unwrap();
    out
}

#[test]
fn a_client_downloads_the_map_from_sv_downloadurl() {
    let dir = scratch("http");
    let install = server_install(&dir);
    let beta = std::fs::read(install.join("maps/beta.bsp")).unwrap();
    let packed = bz2(&beta);
    let (port, asked) = http_server(vec![("/fastdl/maps/beta.bsp.bz2".into(), packed.clone())]);
    let mut sim = server(install.clone(), "test:beta", 74);
    // HTTP only: the server itself sends nothing.
    run(
        sim.server.app.world_mut(),
        &format!("sv_allowdownload 0; sv_downloadurl \"http://127.0.0.1:{port}/fastdl\""),
    );
    let (i, cache) = joins_without_the_map(&mut sim, &dir);
    let ok = sim.until(3000, |s| {
        std::thread::sleep(Duration::from_micros(200));
        s.clients[i].app.world().contains_resource::<Joined>()
    });
    let p = progress(&sim, i);
    println!(
        "client: {:?}, asked {:?}, {} bytes packed for {}, failure {:?}",
        stages(&p),
        asked.lock().unwrap(),
        packed.len(),
        beta.len(),
        p.failure
    );
    assert!(ok, "joined after downloading over HTTP");
    assert_eq!(asked.lock().unwrap().as_slice(), ["/fastdl/maps/beta.bsp.bz2"]);
    assert_eq!(std::fs::read(cache.join("maps/beta.bsp")).unwrap(), beta, "unpacked and cached");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn wrong_or_refused_downloads_end_the_join_with_reasons() {
    let dir = scratch("refused");
    let install = server_install(&dir);
    // A web copy that isn't the server's (another version, plain .bsp).
    let other = file_bytes("beta", 2, 220_000);
    let (port, asked) = http_server(vec![("/dl/maps/beta.bsp".into(), other)]);
    let mut sim = server(install.clone(), "test:beta", 75);
    run(
        sim.server.app.world_mut(),
        &format!("sv_allowdownload 0; sv_downloadurl \"http://127.0.0.1:{port}/dl\""),
    );
    let (i, cache) = joins_without_the_map(&mut sim, &dir);
    sim.until(3000, |s| {
        std::thread::sleep(Duration::from_micros(200));
        *s.clients[i].app.world().resource::<NetRole>() == NetRole::Standalone
    });
    let p = progress(&sim, i);
    println!("wrong file: asked {:?}, failure {:?}", asked.lock().unwrap(), p.failure);
    let (kind, reason) = p.failure.clone().expect("a reason");
    assert_eq!(kind, JoinFailure::MapDiffers);
    assert!(reason.contains("differs from the server's") && reason.contains("refused"), "{reason}");
    assert_eq!(
        asked.lock().unwrap().as_slice(),
        ["/dl/maps/beta.bsp.bz2", "/dl/maps/beta.bsp"],
        "the packed file first, then the plain one"
    );
    assert!(!cache.join("maps/beta.bsp").exists(), "nothing kept");
    sim.ticks(30);
    assert!(sim.character_of(i).is_none(), "the server let it go");

    // No URL and no downloads: CS:S's words.
    run(sim.server.app.world_mut(), "sv_downloadurl \"\"");
    let (j, _) = joins_without_the_map(&mut sim, &dir);
    sim.until(600, |s| *s.clients[j].app.world().resource::<NetRole>() == NetRole::Standalone);
    let p = progress(&sim, j);
    println!("no download: {:?}", p.failure);
    assert_eq!(
        p.failure.clone().unwrap(),
        (JoinFailure::MapMissing, "Missing map maps/beta.bsp, disconnecting".to_string())
    );

    // Downloads on, but the map is over net_maxfilesize: the server says why.
    run(sim.server.app.world_mut(), "sv_allowdownload 1; net_maxfilesize 0.1");
    let (k, _) = joins_without_the_map(&mut sim, &dir);
    sim.until(600, |s| *s.clients[k].app.world().resource::<NetRole>() == NetRole::Standalone);
    let p = progress(&sim, k);
    println!("too big: {:?}", p.failure);
    match p.failure.clone().unwrap() {
        (JoinFailure::Download { file, error }, _) => {
            assert_eq!(file, "maps/beta.bsp");
            assert!(error.contains("net_maxfilesize"), "{error}");
        }
        other => panic!("{other:?}"),
    }

    // Another build: told apart (the server's is newer than "0.0.1/net0").
    let v = sim.add_client(|w| w.insert_resource(NetVersion("0.0.1/net0".into())));
    sim.until(300, |s| *s.clients[v].app.world().resource::<NetRole>() == NetRole::Standalone);
    assert_eq!(progress(&sim, v).failure.unwrap().0, JoinFailure::NewServer);
    let _ = std::fs::remove_dir_all(&dir);
}

/// The server's view of client `i`'s character: (connecting, dead, solid,
/// health).
fn in_game(sim: &mut NetSim, i: usize) -> (bool, bool, bool, f32) {
    let c = sim.character_of(i).expect("a character");
    let w = sim.server.app.world();
    (
        w.get::<mashup::core::Connecting>(c).is_some(),
        w.get::<mashup::rules::Dead>(c).is_some(),
        w.get::<avian3d::prelude::ColliderDisabled>(c).is_none(),
        w.get::<mashup::core::Health>(c).unwrap().current,
    )
}

/// Hit client `i`'s character on the server; whether it took the damage.
fn hurts(sim: &mut NetSim, i: usize) -> bool {
    let c = sim.character_of(i).unwrap();
    let before = sim.server.app.world().get::<mashup::core::Health>(c).unwrap().current;
    sim.server.app.world_mut().write_message(mashup::core::Damage {
        force: Vec3::ZERO,
        target: c,
        attacker: None,
        amount: 0.05,
        point: Vec3::ZERO,
        dir: Vec3::X,
        hitgroup: mashup::core::Hitgroup::Chest,
        kind: default(),
        weapon: None,
    });
    sim.ticks(2);
    let after = sim.server.app.world().get::<mashup::core::Health>(c).unwrap().current;
    after < before
}

#[test]
fn a_client_loading_the_map_is_out_of_the_game_until_it_has_it() {
    let dir = scratch("loading");
    let install = server_install(&dir);
    let mut sim = server(install.clone(), "test:alpha", 73);
    // Deathmatch: a slow client (its load takes 150 frames).
    let slow = add_client(&mut sim, &install, None);
    sim.clients[slow].app.insert_resource(harness::MapLoadDelay(150));
    let loading = sim.until(300, |s| progress(s, slow).stage == JoinStage::LoadingMap);
    assert!(loading, "the client loads the map");
    sim.ticks(60);
    let state = in_game(&mut sim, slow);
    println!("while loading (deathmatch): connecting, dead, solid, health = {state:?}");
    assert_eq!(state, (true, true, false, 0.0), "not in the game while loading");
    assert!(!hurts(&mut sim, slow), "nothing hits a loading player");
    sim.until_joined(400);
    sim.ticks(10);
    let state = in_game(&mut sim, slow);
    println!("loaded (deathmatch): {state:?}");
    assert_eq!(state, (false, false, true, 1.0), "spawned at once in deathmatch");
    assert!(hurts(&mut sim, slow));

    // Rounds, then a map change: one client loads quickly (in the new
    // game's freeze: it plays the first round), the other slowly (after
    // the freeze: out until the next round).
    run(sim.server.app.world_mut(), "mashup_rounds 1; mp_freezetime 3; mp_roundtime 1");
    let quick = add_client(&mut sim, &install, None);
    sim.until_joined(600);
    run(sim.server.app.world_mut(), "mp_restartgame 1");
    sim.ticks(300);
    sim.clients[quick].app.insert_resource(harness::MapLoadDelay(30));
    sim.clients[slow].app.insert_resource(harness::MapLoadDelay(400));
    harness::begin_level_change(sim.server.app.world_mut(), "test:beta");
    sim.ticks(5);
    harness::load_level(sim.server.app.world_mut(), "test:beta", None).unwrap();
    sim.ticks(3);
    for i in [quick, slow] {
        let state = in_game(&mut sim, i);
        println!("client {i} right after the change: {state:?}");
        assert!(state.0 && state.1 && !state.2, "client {i} out of the game while it loads");
    }
    let joined = |s: &NetSim, i: usize| {
        s.clients[i]
            .app
            .world()
            .get_resource::<Joined>()
            .is_some_and(|j| j.map == "test:beta")
    };
    assert!(sim.until(200, |s| joined(s, quick)), "the quick client has the map");
    sim.ticks(10);
    let phase = sim.server.app.world().resource::<RoundState>().phase;
    assert!(matches!(phase, mashup::rules::rounds::Phase::Freeze { .. }), "{phase:?}");
    let state = in_game(&mut sim, quick);
    println!("the quick client in the freeze: {state:?}");
    assert_eq!(state, (false, false, true, 1.0), "loaded in the freeze: plays this round");
    assert!(in_game(&mut sim, slow).0, "the slow one still loading");
    assert!(!hurts(&mut sim, slow));
    assert!(sim.until(600, |s| joined(s, slow)), "the slow client has the map");
    sim.ticks(10);
    let phase = sim.server.app.world().resource::<RoundState>().phase;
    let state = in_game(&mut sim, slow);
    println!("the slow client after the freeze ({phase:?}): {state:?}");
    assert!(!state.0, "in the game");
    assert!(
        matches!(phase, mashup::rules::rounds::Phase::Live { .. }),
        "the slow client came in after the freeze, the round goes on: {phase:?}"
    );
    // Alone on its team: it plays now (coming in dead would hand the round
    // to the others).
    assert_eq!(state, (false, false, true, 1.0), "alone on its team: in at once");
    // A third client on the quick one's team, loading past the freeze of
    // the next game: its team is alive, so it waits for the next round.
    let late = add_client(&mut sim, &install, None);
    sim.clients[late].app.insert_resource(harness::MapLoadDelay(10));
    let team_of = |sim: &mut NetSim, i: usize| {
        let c = sim.character_of(i).unwrap();
        *sim.server.app.world().get::<Team>(c).unwrap()
    };
    sim.until_joined(600);
    sim.ticks(10);
    let phase = sim.server.app.world().resource::<RoundState>().phase;
    let mates = (0..3).filter(|i| *i != late).any(|i| team_of(&mut sim, i) == team_of(&mut sim, late));
    let state = in_game(&mut sim, late);
    println!("a third client mid-round ({phase:?}, teammates: {mates}): {state:?}");
    if mates && matches!(phase, mashup::rules::rounds::Phase::Live { .. }) {
        assert!(!state.0 && state.1, "mid-round with living teammates: out until the next round");
    }
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn changelevel_to_the_same_map_makes_clients_reload_it() {
    let dir = scratch("samemap");
    let install = server_install(&dir);
    let mut sim = server(install.clone(), "test:beta", 74);
    run(sim.server.app.world_mut(), "mashup_rounds 1");
    add_client(&mut sim, &install, None);
    sim.until_joined(600);
    run(sim.server.app.world_mut(), "mp_restartgame 1");
    sim.ticks(200);
    let a = sim.character_of(0).unwrap();
    sim.server.app.world_mut().entity_mut(a).insert(Score { kills: 3, deaths: 0 });
    sim.ticks(20);
    let loads = |w: &World| w.get_resource::<mashup::rules::MapLoads>().map_or(0, |l| l.0);
    let before = loads(sim.clients[0].app.world());
    let id = sim.clients[0].app.world().resource::<LocalClientId>().0;
    // `changelevel beta` on beta: the server loads it again.
    harness::load_level(sim.server.app.world_mut(), "test:beta", None).unwrap();
    let ok = sim.until(600, |s| {
        let w = s.clients[0].app.world();
        loads(w) > before && w.get_resource::<Joined>().is_some_and(|j| j.map == "test:beta")
    });
    assert!(ok, "the client loaded beta again and is back in");
    let p = progress(&sim, 0);
    println!("client: {:?}, map loads {before} -> {}", stages(&p), loads(sim.clients[0].app.world()));
    assert!(stages(&p).ends_with(&[JoinStage::ChangingLevel, JoinStage::Verifying, JoinStage::LoadingMap, JoinStage::Joined]));
    assert_eq!(loads(sim.clients[0].app.world()), before + 1, "loaded once more");
    let w = sim.clients[0].app.world();
    assert_eq!(w.resource::<LocalClientId>().0, id, "the same connection");
    assert!(w.get_resource::<LastDisconnect>().is_none());
    sim.ticks(60);
    // A fresh game: the score gone, the player in it.
    let me = sim.local_player(0).unwrap();
    assert_eq!(sim.clients[0].app.world().get::<NetScore>(me).unwrap().kills, 0);
    let c = sim.character_of(0).unwrap();
    assert!(sim.server.app.world().get::<mashup::core::Connecting>(c).is_none(), "back in the game");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_late_joiner_gets_the_decals() {
    use mashup::map::decal::{DecalGroup, PlaceDecal};
    let dir = scratch("decals");
    let install = server_install(&dir);
    let mut sim = server(install.clone(), "test:alpha", 75);
    // Shots marked the walls before anyone joined (as the impact effects
    // ask for decals); one hit a prop-less spot on the world, one is a
    // knife slash.
    for k in 0..3 {
        sim.server.app.world_mut().write_message(PlaceDecal {
            target: None,
            group: DecalGroup::Material('C'),
            point: Vec3::new(k as f32, 1.0, 5.0),
            normal: Vec3::NEG_Z,
            dir: Vec3::Z,
            spin: false,
        });
    }
    sim.server.app.world_mut().write_message(PlaceDecal {
        target: None,
        group: DecalGroup::Named("ManhackCut".into()),
        point: Vec3::new(0.0, 1.5, 5.0),
        normal: Vec3::NEG_Z,
        dir: Vec3::Z,
        spin: false,
    });
    sim.ticks(5);
    assert_eq!(sim.server.app.world().resource::<mashup::net::decals::DecalLog>().decals.len(), 4);
    let i = add_client(&mut sim, &install, None);
    let mut cursor = sim.clients[i].app.world().resource::<bevy::ecs::message::Messages<PlaceDecal>>().get_cursor_current();
    let mut placed = Vec::new();
    for _ in 0..300 {
        sim.step();
        placed.extend(cursor.read(sim.clients[i].app.world().resource::<bevy::ecs::message::Messages<PlaceDecal>>()).cloned());
        if placed.len() >= 4 {
            break;
        }
    }
    println!("the joiner placed {} decals", placed.len());
    assert_eq!(placed.len(), 4, "the joiner put the server's decals on its world");
    assert_eq!(placed[0].point, Vec3::new(0.0, 1.0, 5.0));
    assert!(matches!(&placed[3].group, DecalGroup::Named(n) if n == "ManhackCut"));
    // A new map starts the log over.
    harness::load_level(sim.server.app.world_mut(), "test:beta", None).unwrap();
    sim.ticks(3);
    assert!(sim.server.app.world().resource::<mashup::net::decals::DecalLog>().decals.is_empty());
    let _ = std::fs::remove_dir_all(&dir);
}
