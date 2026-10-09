//! Player and game entities in the running simulation, on a small map in
//! Source units with a Source player: player_speedmod's movement clock,
//! game_ui holding the player while its keys drive outputs, func_conveyor
//! carrying it, point_viewcontrol's view, freeze and invulnerability,
//! env_fade on the local HUD (specs/source/game_entities.md,
//! viewcontrol_and_templates.md), and phys_thruster pushing a
//! func_physbox (physics_brushes.md). The logic's own spec tables are
//! unit tests in src/logic/game_tests.rs and template_tests.rs.

use bevy::prelude::*;
use mashup::{
    core::{Damage, DamageKind, Health, Hitgroup, LocalPlayer, MapControls, MapView},
    games::cs_source::{
        self,
        movement::{self, SourceMovementPlugin, to_engine, to_source},
    },
    harness::Sim,
    logic::{Logic, ScreenFades, Value, Who},
    map::{MapBrush, MapData, MapEntity, MapHull, MapPhysics, MapPlugin, PushAway},
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

fn entity(pairs: &[(&str, &str)]) -> MapEntity {
    MapEntity {
        keyvalues: pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect(),
        ..default()
    }
}

/// A floor at z = 0 plus `entities`.
fn map(entities: Vec<MapEntity>) -> MapData {
    let (a, b) = (to_engine(Vec3::new(-4096.0, -4096.0, -64.0)), to_engine(Vec3::new(4096.0, 4096.0, 0.0)));
    let (a, b) = (a.min(b), a.max(b));
    MapData {
        name: "test:game_entities".into(),
        collision_brushes: vec![MapBrush::from_box(a, b)],
        collision_hulls: vec![
            (0..8)
                .map(|k| [[a.x, b.x][k & 1], [a.y, b.y][(k >> 1) & 1], [a.z, b.z][(k >> 2) & 1]])
                .collect(),
        ],
        entities,
        entity_scale: SCALE,
        ..default()
    }
}

/// A Source player standing at `feet` (the local player), after 10 ticks.
fn sim(entities: Vec<MapEntity>, feet: Vec3) -> (Sim, Entity) {
    let mut sim = Sim::new((MapPlugin::new(map(entities)), SourceMovementPlugin));
    sim.set_tick_interval(cs_source::TICK_INTERVAL);
    let p = sim.spawn_character(to_engine(feet + Vec3::Z * 37.0), movement::ID);
    sim.app.world_mut().entity_mut(p).insert(LocalPlayer);
    sim.intent(p).yaw = -std::f32::consts::FRAC_PI_2;
    sim.ticks(10);
    (sim, p)
}

fn feet(sim: &Sim, p: Entity) -> Vec3 {
    to_source(sim.position(p)) - Vec3::Z * 36.0
}

fn input(sim: &mut Sim, target: &str, input: &str, value: &str, p: Option<Entity>) {
    let value = if value.is_empty() { Value::Void } else { Value::Str(value.into()) };
    sim.app
        .world_mut()
        .resource_mut::<Logic>()
        .world
        .queue_input(target, input, value, 0.0, p.map(Who::Player));
}

/// Units moved per tick along +X while running.
fn step(sim: &mut Sim, p: Entity) -> f32 {
    let a = feet(sim, p);
    sim.ticks(1);
    feet(sim, p).x - a.x
}

#[test]
fn speedmod_scales_the_movement_clock() {
    let (mut sim, p) = sim(vec![entity(&[("classname", "player_speedmod"), ("targetname", "speed")])], Vec3::ZERO);
    sim.intent(p).move_axis = Vec2::Y;
    sim.ticks(80);
    let normal = step(&mut sim, p);
    let v = sim.velocity(p).length();
    assert!(normal > 3.0, "running: {normal} units a tick");
    // From logic with no player: nothing.
    input(&mut sim, "speed", "ModifySpeed", "2", None);
    sim.ticks(2);
    assert!((step(&mut sim, p) - normal).abs() < 1e-3);
    input(&mut sim, "speed", "ModifySpeed", "2", Some(p));
    sim.ticks(3);
    let fast = step(&mut sim, p);
    assert!((fast - 2.0 * normal).abs() < 0.05, "{fast} vs 2 x {normal}");
    assert!((sim.velocity(p).length() - v).abs() < 1e-3, "the velocity itself isn't scaled");
    input(&mut sim, "speed", "ModifySpeed", "0", Some(p));
    sim.ticks(2);
    assert_eq!(step(&mut sim, p), 0.0, "0 holds it still");
}

#[test]
fn speedmod_takes_the_jump_away_until_restored() {
    let (mut sim, p) = sim(vec![entity(&[("classname", "player_speedmod"), ("targetname", "speed"), ("spawnflags", "4")])], Vec3::ZERO);
    input(&mut sim, "speed", "ModifySpeed", "0.9", Some(p));
    sim.ticks(2);
    let ground = feet(&sim, p).z;
    sim.intent(p).jump = true;
    sim.ticks(10);
    assert!((feet(&sim, p).z - ground).abs() < 0.5, "no jump");
    sim.intent(p).jump = false;
    input(&mut sim, "speed", "ModifySpeed", "1", Some(p));
    sim.ticks(2);
    sim.intent(p).jump = true;
    sim.ticks(10);
    assert!(feet(&sim, p).z > ground + 10.0, "jumps again");
}

#[test]
fn game_ui_holds_the_player_and_reports_keys() {
    let (mut sim, p) = sim(
        vec![
            entity(&[
                ("classname", "game_ui"),
                ("targetname", "ui"),
                ("spawnflags", "32"),
                ("FieldOfView", "-1"),
                ("PressedForward", "fwd,Trigger,,0,-1"),
            ]),
            entity(&[("classname", "logic_relay"), ("targetname", "fwd")]),
        ],
        Vec3::ZERO,
    );
    sim.app.world_mut().resource_mut::<Logic>().world.record = true;
    input(&mut sim, "ui", "Activate", "", Some(p));
    sim.ticks(3);
    let at = feet(&sim, p);
    sim.intent(p).move_axis = Vec2::Y;
    sim.ticks(30);
    assert!(feet(&sim, p).distance(at) < 0.1, "held still");
    let logic = sim.app.world().resource::<Logic>();
    let fwd = logic.world.find("fwd").unwrap();
    assert!(logic.world.deliveries.iter().any(|d| d.target == Who::Ent(fwd)), "PressedForward");
}

#[test]
fn conveyor_carries_whoever_stands_on_it() {
    let belt = MapEntity {
        hulls: vec![hull(Vec3::new(-128.0, -128.0, -8.0), Vec3::new(128.0, 128.0, 0.0))],
        mover: true,
        ..entity(&[
            ("classname", "func_conveyor"),
            ("targetname", "belt"),
            ("origin", "0 0 8"),
            ("movedir", "0 90 0"),
            ("speed", "100"),
        ])
    };
    let (mut sim, p) = sim(vec![belt], Vec3::new(0.0, 0.0, 8.0));
    sim.ticks(5);
    let a = feet(&sim, p);
    sim.ticks(1);
    let d = feet(&sim, p) - a;
    assert!((d.y - 1.5).abs() < 0.05 && d.x.abs() < 0.01, "{d}");
    assert!(sim.velocity(p).length() < 1e-3, "its own velocity stays 0");
}

#[test]
fn viewcontrol_takes_the_view_freezes_and_protects() {
    let (mut sim, p) = sim(
        vec![
            entity(&[
                ("classname", "point_viewcontrol"),
                ("targetname", "cam"),
                ("origin", "0 -200 100"),
                ("target", "look"),
                ("wait", "2"),
                ("spawnflags", "4"),
            ]),
            entity(&[("classname", "info_target"), ("targetname", "look"), ("origin", "0 0 0")]),
        ],
        Vec3::ZERO,
    );
    input(&mut sim, "cam", "Enable", "", None);
    sim.ticks(3);
    assert!(sim.app.world().get::<MapView>(p).is_none(), "no player activator: nothing");
    input(&mut sim, "cam", "Enable", "", Some(p));
    sim.ticks(3);
    let view = *sim.app.world().get::<MapView>(p).expect("viewing through the camera");
    assert!(view.origin.distance(to_engine(Vec3::new(0.0, -200.0, 100.0))) < 0.05);
    assert!(sim.app.world().get::<MapControls>(p).is_some_and(|c| c.frozen.is_some() && c.invulnerable));
    let at = feet(&sim, p);
    sim.intent(p).move_axis = Vec2::Y;
    sim.ticks(20);
    assert!(feet(&sim, p).distance(at) < 0.1, "frozen");
    let health = sim.app.world().get::<Health>(p).unwrap().current;
    sim.app.world_mut().write_message(Damage {
        target: p,
        attacker: None,
        amount: 0.5,
        point: Vec3::ZERO,
        dir: Vec3::NEG_Y,
        hitgroup: Hitgroup::Generic,
        kind: DamageKind::Generic,
        weapon: None,
        force: Vec3::ZERO,
    });
    sim.ticks(2);
    assert_eq!(sim.app.world().get::<Health>(p).unwrap().current, health, "invulnerable");
    // 2 s later the view comes back.
    sim.seconds(2.1);
    assert!(sim.app.world().get::<MapView>(p).is_none());
    assert!(sim.app.world().get::<MapControls>(p).is_some_and(|c| c.frozen.is_none() && !c.invulnerable));
}

#[test]
fn fade_reaches_the_local_screen() {
    let (mut sim, _) = sim(
        vec![entity(&[
            ("classname", "env_fade"),
            ("targetname", "fade"),
            ("duration", "1"),
            ("holdtime", "1"),
            ("renderamt", "255"),
            ("rendercolor", "0 0 0"),
        ])],
        Vec3::ZERO,
    );
    input(&mut sim, "fade", "Fade", "", None);
    sim.ticks(2);
    assert_eq!(sim.app.world().resource::<ScreenFades>().fades.len(), 1);
}

#[test]
fn thruster_pushes_a_physbox() {
    let physbox = MapEntity {
        hulls: vec![hull(Vec3::splat(-16.0), Vec3::splat(16.0))],
        mover: true,
        physics: Some(MapPhysics {
            mass: 50.0,
            friction: 0.0,
            elasticity: 0.0,
            damping: 0.0,
            rotdamping: 0.0,
            push: PushAway::Collide,
            frozen: false,
            asleep: false,
        }),
        ..entity(&[
            ("classname", "func_physbox"),
            ("targetname", "box"),
            ("origin", "0 0 600"),
            ("mashup_prop_mass", "50"),
        ])
    };
    let thruster = entity(&[
        ("classname", "phys_thruster"),
        ("targetname", "th"),
        ("attach1", "box"),
        ("origin", "0 0 600"),
        ("force", "1000"),
        ("spawnflags", "2"),
    ]);
    let (mut sim, _) = sim(vec![physbox, thruster], Vec3::new(500.0, 500.0, 0.0));
    let vx = |sim: &mut Sim| {
        let world = sim.app.world_mut();
        world
            .query::<(&mashup::map::MapBrushEntity, &avian3d::prelude::LinearVelocity)>()
            .iter(world)
            .next()
            .map(|(_, v)| v.0.x)
            .expect("the physbox's body")
    };
    let before = vx(&mut sim);
    input(&mut sim, "th", "Activate", "", None);
    sim.seconds(0.5);
    // a = F/m = 20 units/s² along +X: 10 units/s after half a second.
    let gained = (vx(&mut sim) - before) / SCALE;
    assert!((gained - 10.0).abs() < 1.5, "{gained} units/s");
}
