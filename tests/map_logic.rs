//! Map logic in the running simulation (specs/source/entity_io.md,
//! triggers.md, doors_buttons.md): a small map built in Source units with
//! entities, a Source player, the logic layer and movement together. The
//! logic's own spec tables are unit tests in src/logic/tests.rs.

use bevy::prelude::*;
use mashup::{
    core::{Intent, MovingSolid},
    games::cs_source::{
        self,
        movement::{self, SourceMovementPlugin, to_engine, to_source},
    },
    harness::Sim,
    map::{MapBrush, MapBrushEntity, MapData, MapEntity, MapHull, MapPlugin},
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
    }
}

/// A floor at z = 0 plus `entities`.
fn map(entities: Vec<MapEntity>) -> MapData {
    let (a, b) = (to_engine(Vec3::new(-4096.0, -4096.0, -64.0)), to_engine(Vec3::new(4096.0, 4096.0, 0.0)));
    MapData {
        name: "test:logic".into(),
        collision_brushes: vec![MapBrush::from_box(a.min(b), a.max(b))],
        entities,
        entity_scale: SCALE,
        ..default()
    }
}

/// A Source player standing at `feet`, looking along Source yaw `yaw`
/// (degrees), after 10 ticks.
fn sim(entities: Vec<MapEntity>, feet: Vec3, yaw: f32) -> (Sim, Entity) {
    let mut sim = Sim::new((MapPlugin::new(map(entities)), SourceMovementPlugin));
    sim.set_tick_interval(cs_source::TICK_INTERVAL);
    // A unit above the ground, to land on it (exact contact reads as
    // inside the brush).
    let p = sim.spawn_character(to_engine(feet + Vec3::Z * 37.0), movement::ID);
    sim.intent(p).yaw = (yaw - 90.0).to_radians();
    sim.ticks(10);
    (sim, p)
}

fn feet(sim: &Sim, p: Entity) -> Vec3 {
    to_source(sim.position(p)) - Vec3::Z * 36.0
}

/// The Source origin of the mover node for map entity `index`.
fn mover_origin(sim: &mut Sim, index: usize) -> Vec3 {
    let world = sim.app.world_mut();
    let mut q = world.query::<(&MapBrushEntity, &Transform)>();
    let t = q.iter(world).find(|(n, _)| n.0 == index).expect("mover node").1;
    to_source(t.translation)
}

#[test]
fn door_opens_on_use_and_closes_after_its_wait() {
    let door = entity(
        &[
            ("classname", "func_door"),
            ("origin", "100 0 64"),
            ("movedir", "0 0 0"),
            ("lip", "4"),
            ("speed", "100"),
            ("wait", "4"),
            ("spawnflags", "256"),
        ],
        vec![hull(Vec3::new(-32.0, -4.0, -64.0), Vec3::new(32.0, 4.0, 64.0))],
        true,
    );
    let (mut sim, p) = sim(vec![door], Vec3::new(40.0, 0.0, 0.0), 0.0);
    assert!((mover_origin(&mut sim, 0).x - 100.0).abs() < 0.01);
    // The closed door is solid to movement: walking into it stops short.
    sim.intent(p).move_axis = Vec2::Y;
    sim.ticks(40);
    sim.intent(p).move_axis = Vec2::ZERO;
    assert!(feet(&sim, p).x < 100.0 - 4.0 - 16.0 + 0.1, "walked into the door: {}", feet(&sim, p));
    assert!(sim.app.world_mut().query::<&MovingSolid>().iter(sim.app.world()).any(|m| m.solid));

    // One press of +use: opens 58 units in 0.58 s, waits 4 s, closes.
    sim.intent(p).use_key = true;
    sim.ticks(1);
    sim.intent(p).use_key = false;
    sim.ticks(45);
    assert!((mover_origin(&mut sim, 0).x - 158.0).abs() < 0.01, "open: {}", mover_origin(&mut sim, 0));
    sim.ticks(250);
    assert!((mover_origin(&mut sim, 0).x - 158.0).abs() < 0.01, "still waiting");
    sim.ticks(60);
    assert!(mover_origin(&mut sim, 0).x < 158.0, "closing after its wait");
    sim.ticks(60);
    // The player stood clear (x 40 + 16 < 68): back closed.
    assert!((mover_origin(&mut sim, 0).x - 100.0).abs() < 0.01, "closed: {}", mover_origin(&mut sim, 0));
}

#[test]
fn player_rides_a_rising_platform() {
    // An elevator 128x128x8 whose top is at z = 8, rising 100 units at
    // 100 units/s when the map starts (logic_auto -> Open).
    let lift = entity(
        &[
            ("classname", "func_door"),
            ("targetname", "lift"),
            ("origin", "0 0 8"),
            ("movedir", "-90 0 0"),
            ("lip", "-94"),
            ("speed", "100"),
            ("wait", "-1"),
        ],
        vec![hull(Vec3::new(-64.0, -64.0, -8.0), Vec3::ZERO)],
        true,
    );
    let auto = entity(&[("classname", "logic_auto"), ("OnMapSpawn", "lift,Open,,0,-1")], Vec::new(), false);
    let (mut sim, p) = sim(vec![lift, auto], Vec3::new(0.0, 0.0, 8.0), 0.0);
    sim.ticks(30);
    let rising = mover_origin(&mut sim, 0).z;
    assert!(rising > 8.0 && rising < 108.0, "rising: {rising}");
    assert!((feet(&sim, p).z - rising).abs() < 2.0, "rides it: feet {} lift {rising}", feet(&sim, p).z);
    assert!(sim.state(p).on_ground);
    sim.ticks(100);
    let top = mover_origin(&mut sim, 0).z;
    assert!((top - 108.0).abs() < 0.01, "lift at the top: {top}");
    let f = feet(&sim, p);
    // (Within the 2-unit ground probe: it may hover the unit it spawned at.)
    assert!((f.z - 108.0).abs() < 1.5 && f.truncate().length() < 1.0, "on top of it: {f}");
}

#[test]
fn trigger_teleport_moves_and_turns_the_player() {
    let teleport = entity(
        &[("classname", "trigger_teleport"), ("spawnflags", "1"), ("target", "dest"), ("origin", "0 0 0")],
        vec![hull(Vec3::new(-64.0, -64.0, 0.0), Vec3::new(64.0, 64.0, 64.0))],
        false,
    );
    let dest = entity(
        &[("classname", "info_teleport_destination"), ("targetname", "dest"), ("origin", "1000 200 0"), ("angles", "0 90 0")],
        Vec::new(),
        false,
    );
    let (mut sim, p) = sim(vec![teleport, dest], Vec3::new(0.0, 0.0, 0.0), 0.0);
    sim.ticks(3);
    let f = feet(&sim, p);
    assert!((f.truncate() - Vec2::new(1000.0, 200.0)).length() < 1.0, "teleported to {f}");
    assert!(f.z.abs() < 1.0);
    // Facing the destination's yaw 90 (Intent yaw 0).
    let yaw = sim.app.world().get::<Intent>(p).unwrap().yaw;
    assert!(yaw.abs() < 1e-3 || (yaw - std::f32::consts::TAU).abs() < 1e-3, "yaw {yaw}");
}

#[test]
fn trigger_push_is_a_conveyor_then_momentum() {
    // Push +X at 300 units/s over x -64..200.
    let push = entity(
        &[("classname", "trigger_push"), ("spawnflags", "1"), ("pushdir", "0 0 0"), ("speed", "300")],
        vec![hull(Vec3::new(-64.0, -64.0, 0.0), Vec3::new(200.0, 64.0, 64.0))],
        false,
    );
    let (mut sim, p) = sim(vec![push], Vec3::ZERO, 0.0);
    sim.ticks(3);
    let a = feet(&sim, p).x;
    sim.ticks(1);
    let b = feet(&sim, p).x;
    assert!((b - a - 4.5).abs() < 0.05, "moves 4.5 units a tick: {}", b - a);
    assert!(to_source(sim.velocity(p)).x.abs() < 1.0, "own velocity stays ~0: {}", sim.velocity(p));
    // Leaving the push: its speed becomes momentum (x 1.0075), then
    // friction.
    let mut peak = 0.0f32;
    for _ in 0..80 {
        sim.ticks(1);
        peak = peak.max(to_source(sim.velocity(p)).x);
    }
    assert!(feet(&sim, p).x > 216.0, "left the push: {}", feet(&sim, p));
    assert!(peak > 250.0 && peak < 303.0, "momentum peak {peak}");
}
