//! de_port from a real CS:S install, headless: its `trigger_hurt` (the sea
//! under the map) hurts as measured (specs/cs_source/fall_damage.md).
//! Skipped without an install.

use bevy::prelude::*;
use mashup::{
    core::Health,
    games::{
        self, cs_source,
        cs_source::movement::{self, SourceMovementPlugin},
    },
    harness::Sim,
    map::{MapData, MapPlugin},
    mount::config::LocalConfig,
};

fn port() -> Option<MapData> {
    let installed = LocalConfig::load()
        .ok()?
        .game_path(cs_source::GAME)
        .is_some_and(|p| p.join("cstrike").is_dir());
    if !installed {
        eprintln!("skipping: no CS:S install configured");
        return None;
    }
    Some(games::load_map("cs_source:de_port").expect("load de_port"))
}

#[test]
fn trigger_hurt_bites_every_half_second() {
    let Some(map) = port() else { return };
    assert_eq!(map.hurt.len(), 1, "de_port has one trigger_hurt");
    let volume = &map.hurt[0];
    for b in &volume.brushes {
        eprintln!("brush {} .. {} planes {}", movement::to_source(b.min), movement::to_source(b.max), b.planes.len());
    }
    assert_eq!(volume.damage_per_second, 50.0);
    // Its brushes span the measured model bounds (Source units).
    let lo = volume.brushes.iter().map(|b| b.min).fold(Vec3::MAX, Vec3::min);
    let hi = volume.brushes.iter().map(|b| b.max).fold(Vec3::MIN, Vec3::max);
    let (a, b) = (movement::to_source(lo), movement::to_source(hi));
    let (lo, hi) = (a.min(b), a.max(b));
    assert!(
        lo.distance(Vec3::new(-7424.0, -6816.0, -128.0)) < 1.0 && hi.distance(Vec3::new(7936.0, 2144.0, 128.0)) < 1.0,
        "bounds {lo} .. {hi}"
    );

    let mut sim = Sim::new((MapPlugin::new(map), SourceMovementPlugin));
    sim.set_tick_interval(cs_source::TICK_INTERVAL);
    // Where the probe bot stood inside it: the floor at (256, 2000, 0).
    let feet = movement::to_engine(Vec3::new(256.0, 2000.0, 1.0));
    let p = sim.spawn_character(feet + Vec3::Y * 36.0 * 0.0254, movement::ID);
    let mut bites = Vec::new();
    let mut last = 1.0;
    for tick in 0..150 {
        sim.ticks(1);
        let h = sim.app.world().get::<Health>(p).unwrap().current;
        if h < last {
            assert!((last - h - 0.25).abs() < 1e-5 || h == 0.0, "bit {} at tick {tick}", last - h);
            bites.push(tick);
            last = h;
        }
    }
    // 25 a bite, 33 ticks apart: dead on the fourth.
    assert_eq!(bites.len(), 4, "bites at {bites:?}");
    assert!(bites[0] <= 4, "first bite at tick {}", bites[0]);
    assert!(bites.windows(2).all(|w| w[1] - w[0] == 33), "bites at {bites:?}");
    assert_eq!(last, 0.0);
}
