//! A network map change to a real map (docs/plans/active/multiplayer.md,
//! slice 7): a server on the greybox with a client and two bots changes
//! level to de_dust2 from the install; the client follows on the same
//! connection (its copy of the file checked by hash), the bots come
//! along, everyone spawns on de_dust2. Skipped without a CS:S install.

use std::{sync::Arc, time::Duration};

use bevy::{
    ecs::message::{MessageCursor, Messages},
    prelude::*,
};
use mashup::{
    games::{
        self,
        cs_source::{
            self,
            movement::{self as source, SourceMovementPlugin},
            weapons::CsWeaponsPlugin,
        },
    },
    greybox::GreyboxMapPlugin,
    harness::NetSim,
    map::{MapFile, MapPlugin},
    mount::config::LocalConfig,
    net::{
        NetEvent,
        client::{JoinStage, Joined, JoinProgress},
        maps::MapFiles,
        memory::LinkConditions,
    },
    slots::Loadout,
};

const DUST2: &str = "cs_source:de_dust2";

fn installed() -> bool {
    let ok = LocalConfig::load()
        .ok()
        .and_then(|c| c.game_path(cs_source::GAME))
        .is_some_and(|p| p.join("cstrike").is_dir());
    if !ok {
        eprintln!("skipping: no CS:S install configured");
    }
    ok
}

/// What the game's client layer does with `NetEvent::LoadMap`, at once.
fn load_requested(world: &mut World, mut cursor: Local<MessageCursor<NetEvent>>) {
    let events: Vec<NetEvent> = cursor.read(world.resource::<Messages<NetEvent>>()).cloned().collect();
    for e in events {
        if let NetEvent::LoadMap { map, file } = e {
            let data = match file {
                Some(f) => games::load_map_file(&map, &f, 0),
                None => games::load_map(&map),
            }
            .expect("the client loads the map");
            mashup::swap_map(world, &map, Some(data), Duration::from_secs_f64(cs_source::TICK_INTERVAL));
        }
    }
}

#[test]
fn changelevel_to_de_dust2_takes_a_client_and_bots_along() {
    if !installed() {
        return;
    }
    let link = LinkConditions {
        latency: Duration::from_millis(40),
        ..default()
    };
    let mut sim = NetSim::new(link, 81, 1, |app| {
        app.add_plugins((GreyboxMapPlugin, MapPlugin::empty(), SourceMovementPlugin, CsWeaponsPlugin))
            .insert_resource(Loadout { movement: source::ID })
            .insert_resource(MapFiles {
                read: Arc::new(games::map_file_bytes),
                cache: None,
            })
            .add_systems(Update, load_requested);
    });
    sim.until_joined(400);
    let id = sim.client_id(0);
    mashup::console::execute(sim.server.app.world_mut(), &["bot_add".into(), "1".into()], 0);
    mashup::console::execute(sim.server.app.world_mut(), &["bot_add".into(), "2".into()], 0);
    sim.ticks(20);

    let data = games::load_map(DUST2).expect("load de_dust2");
    mashup::swap_map(
        sim.server.app.world_mut(),
        DUST2,
        Some(data),
        Duration::from_secs_f64(cs_source::TICK_INTERVAL),
    );
    let ok = sim.until(2000, |s| {
        s.clients[0]
            .app
            .world()
            .get_resource::<Joined>()
            .is_some_and(|j| j.map == DUST2)
    });
    let p: JoinProgress = sim.clients[0].app.world().resource::<JoinProgress>().clone();
    println!("client: {:?}, failure {:?}", p.log.iter().map(|l| l.0).collect::<Vec<_>>(), p.failure);
    assert!(ok, "the client followed to de_dust2");
    assert!(p.log.iter().any(|l| l.0 == JoinStage::Verifying), "its copy checked by hash");
    let server_file = sim.server.app.world().resource::<MapFile>().clone();
    let client_file = sim.clients[0].app.world().resource::<MapFile>().clone();
    assert_eq!(server_file.hash, client_file.hash);
    assert_eq!(sim.client_id(0), id);
    sim.ticks(200);
    // The bots came along and everyone stands on de_dust2 (above its floor,
    // not falling).
    let w = sim.server.app.world_mut();
    let bots = w.query_filtered::<(), With<mashup::bot::Bot>>().iter(w).count();
    assert_eq!(bots, 2);
    let me = sim.local_player(0).unwrap();
    let v = sim.clients[0].app.world().get::<mashup::core::Velocity>(me).unwrap().0;
    assert!(v.y.abs() < 1.0, "standing: {v}");
}
