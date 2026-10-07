//! Map logic in the running simulation (specs/source/entity_io.md,
//! triggers.md, doors_buttons.md): a small map built in Source units with
//! entities, a Source player, the logic layer and movement together. The
//! logic's own spec tables are unit tests in src/logic/tests.rs.

use bevy::prelude::*;
use mashup::{
    core::{Damage, DamageKind, Damageable, Hitgroup, Intent, MovingSolid, RoundRestarts},
    games::cs_source::{
        self,
        movement::{self, SourceMovementPlugin, to_engine, to_source},
    },
    harness::Sim,
    logic::{HudMessages, Logic},
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

fn node(sim: &mut Sim, index: usize) -> Entity {
    let world = sim.app.world_mut();
    let mut q = world.query::<(Entity, &MapBrushEntity)>();
    q.iter(world).find(|(_, n)| n.0 == index).expect("mover node").0
}

/// Whether map entity `index` exists in the logic.
fn exists(sim: &Sim, index: usize) -> bool {
    let logic = sim.app.world().resource::<Logic>();
    logic
        .world
        .ids()
        .into_iter()
        .any(|id| logic.world.get(id).unwrap().map_index == Some(index))
}

#[test]
fn round_restart_puts_the_map_back() {
    use avian3d::prelude::ColliderDisabled;
    let entities = vec![
        // 0: a door the trigger opens and that stays open.
        entity(
            &[("classname", "func_door"), ("targetname", "door"), ("origin", "100 0 64"), ("lip", "4"), ("speed", "200"), ("wait", "-1")],
            vec![hull(Vec3::new(-32.0, -4.0, -64.0), Vec3::new(32.0, 4.0, 64.0))],
            true,
        ),
        // 1: a trigger_once around the player's start.
        entity(
            &[("classname", "trigger_once"), ("spawnflags", "1"), ("OnTrigger", "door,Open,,0,-1")],
            vec![hull(Vec3::new(-64.0, -64.0, 0.0), Vec3::new(64.0, 64.0, 72.0))],
            false,
        ),
        // 2: a metal vent, health 1.
        entity(
            &[("classname", "func_breakable"), ("material", "2"), ("health", "1"), ("origin", "0 300 32")],
            vec![hull(Vec3::new(-32.0, -4.0, -16.0), Vec3::new(32.0, 4.0, 16.0))],
            true,
        ),
        // 3, 4: a HUD message at every round start.
        entity(&[("classname", "game_text"), ("targetname", "msg"), ("message", "go"), ("holdtime", "100"), ("spawnflags", "1")], Vec::new(), false),
        entity(&[("classname", "logic_auto"), ("OnMapSpawn", "msg,Display,,0,-1")], Vec::new(), false),
    ];
    let (mut sim, p) = sim(entities, Vec3::ZERO, 0.0);
    sim.ticks(40);
    let hud = |sim: &Sim| sim.app.world().resource::<HudMessages>().channels.iter().flatten().count();
    assert_eq!(hud(&sim), 1, "logic_auto showed the message");
    assert!(!exists(&sim, 1), "the trigger fired and went");
    let open = mover_origin(&mut sim, 0);
    assert!((open.x - 158.0).abs() < 0.01, "door open: {open}");

    // Break the vent.
    let vent = node(&mut sim, 2);
    assert!(sim.app.world().get::<Damageable>(vent).is_some());
    sim.app.world_mut().write_message(Damage {
        target: vent,
        attacker: None,
        amount: 0.5,
        point: to_engine(Vec3::new(0.0, 296.0, 32.0)),
        dir: Vec3::NEG_Z,
        hitgroup: Hitgroup::Generic,
        kind: DamageKind::Bullet,
        weapon: None,
    });
    sim.ticks(10);
    assert!(!exists(&sim, 2), "broken and removed");
    let world = sim.app.world();
    assert_eq!(world.get::<Visibility>(vent), Some(&Visibility::Hidden), "hidden, not despawned");
    assert!(world.get::<ColliderDisabled>(vent).is_some());
    assert!(!world.get::<MovingSolid>(vent).unwrap().solid);
    assert!(world.get::<Damageable>(vent).is_none());

    // Out of the trigger, then a new round.
    sim.app.world_mut().get_mut::<Transform>(p).unwrap().translation = to_engine(Vec3::new(-300.0, 0.0, 37.0));
    sim.ticks(5);
    sim.app.world_mut().resource_mut::<RoundRestarts>().0 += 1;
    sim.ticks(1);
    assert_eq!(hud(&sim), 0, "the last round's HUD message is cleared");
    assert!(exists(&sim, 1) && exists(&sim, 2), "trigger and vent re-created");
    assert_eq!(mover_origin(&mut sim, 0), Vec3::new(100.0, 0.0, 64.0), "door back closed");
    assert_eq!(node(&mut sim, 2), vent, "the same node");
    let world = sim.app.world();
    assert_eq!(world.get::<Visibility>(vent), Some(&Visibility::Inherited));
    assert!(world.get::<ColliderDisabled>(vent).is_none());
    assert!(world.get::<MovingSolid>(vent).unwrap().solid);
    assert!(world.get::<Damageable>(vent).is_some());
    sim.ticks(14);
    assert_eq!(hud(&sim), 1, "logic_auto fired again");
    assert!(exists(&sim, 1), "the player is out of the trigger: still armed");
}
