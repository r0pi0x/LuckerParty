//! Map logic meeting physics bodies in the running simulation, on a small
//! map in Source units. Breakables (specs/source/breakables.md, "Damage"
//! and "Breaking"): a physics prop falling on a glass func_breakable breaks
//! it by impact damage (unless the brush takes no physics damage), and a
//! breakable that explodes when it breaks breaks the one next to it. Model
//! doors (specs/source/doors_buttons.md, prop_door_rotating "Blocked"): a
//! crate pinned against a wall stops a closing door, unless it is
//! forceclosed.

use avian3d::prelude::Position;
use bevy::prelude::*;
use mashup::{
    games::cs_source::{
        self,
        movement::{SourceMovementPlugin, to_engine, to_source},
        weapons::CsWeaponsPlugin,
    },
    harness::Sim,
    logic::{Logic, Value, classes::Class},
    map::{
        MapBrush, MapCollision, MapConvex, MapData, MapEntity, MapHull, MapModel, MapPhysics, MapPlugin, MapProp,
        PropEntity, PropSolid, PushAway,
    },
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

/// A floor at z = 0 plus `entities`.
fn map(entities: Vec<MapEntity>) -> MapData {
    let mut data = MapData {
        name: "test:logic_physics".into(),
        entities,
        entity_scale: SCALE,
        ..default()
    };
    solid(&mut data, Vec3::new(-4096.0, -4096.0, -64.0), Vec3::new(4096.0, 4096.0, 0.0));
    data
}

/// A solid box of the world (Source units): for players and the logic,
/// and for shots and physics bodies.
fn solid(data: &mut MapData, lo: Vec3, hi: Vec3) {
    let (a, b) = (to_engine(lo), to_engine(hi));
    let (a, b) = (a.min(b), a.max(b));
    data.collision_brushes.push(MapBrush::from_box(a, b));
    data.collision_hulls.push(
        (0..8)
            .map(|k| [[a.x, b.x][k & 1], [a.y, b.y][(k >> 1) & 1], [a.z, b.z][(k >> 2) & 1]])
            .collect(),
    );
}

/// A 20 kg, 32-unit physics crate (map entity `index`) at `at`.
fn add_crate(data: &mut MapData, index: usize, at: Vec3) {
    let half = 16.0 * SCALE;
    let (lo, hi) = (Vec3::splat(-half), Vec3::splat(half));
    let corners = (0..8)
        .map(|k| Vec3::new([lo.x, hi.x][k & 1], [lo.y, hi.y][(k >> 1) & 1], [lo.z, hi.z][(k >> 2) & 1]))
        .collect();
    data.models.push(MapModel {
        bounds: (lo, hi),
        collision: Some(MapCollision {
            pieces: vec![MapConvex { points: corners, planes: Vec::new() }],
            mass: 20.0,
            ..default()
        }),
        ..default()
    });
    data.props.push(MapProp {
        pose: None,
        model: data.models.len() - 1,
        translation: to_engine(at),
        rotation: Quat::IDENTITY,
        solid: PropSolid::Mesh,
        skybox: false,
        lighting: None,
        vertex_light: None,
        casts_shadow: false,
        physics: Some(MapPhysics {
            mass: 20.0,
            friction: 0.8,
            elasticity: 0.0,
            damping: 0.0,
            rotdamping: 0.0,
            push: PushAway::Collide,
            frozen: false,
            asleep: false,
        }),
        fade: None,
        parent: None,
        entity: Some(index),
        skin: 0,
        body: 0,
    });
}

fn sim(data: MapData) -> Sim {
    let mut sim = Sim::new((MapPlugin::new(data), SourceMovementPlugin, CsWeaponsPlugin));
    sim.set_tick_interval(cs_source::TICK_INTERVAL);
    sim
}

/// Whether map entity `index` is a broken breakable (or gone).
fn broken(sim: &Sim, index: usize) -> bool {
    let logic = sim.app.world().resource::<Logic>();
    let id = logic
        .world
        .ids()
        .into_iter()
        .find(|id| logic.world.get(*id).unwrap().map_index == Some(index));
    match id.map(|id| &logic.world.get(id).unwrap().class) {
        None => true,
        Some(Class::Breakable(b)) => b.broken,
        Some(_) => panic!("not a breakable"),
    }
}

/// Where the crate is (Source units).
fn crate_at(sim: &mut Sim) -> Vec3 {
    let world = sim.app.world_mut();
    let node = world.query::<(Entity, &PropEntity)>().iter(world).next().unwrap().0;
    to_source(world.get::<Position>(node).unwrap().0)
}

/// A glass pane lying flat 64 units up, a crate dropped from 160: the
/// fall breaks it (the glass table at 2 kg), and the crate lands on the
/// floor. With "Don't take physics damage" (1024) it holds the crate.
#[test]
fn a_falling_crate_breaks_a_glass_pane() {
    for (flags, breaks) in [("0", true), ("1024", false)] {
        let pane = entity(
            &[
                ("classname", "func_breakable"),
                ("origin", "0 0 64"),
                ("material", "0"),
                ("health", "1"),
                ("spawnflags", flags),
            ],
            vec![hull(Vec3::new(-48.0, -48.0, -1.0), Vec3::new(48.0, 48.0, 1.0))],
            true,
        );
        let box_ = entity(&[("classname", "prop_physics"), ("origin", "0 0 160")], Vec::new(), false);
        let mut data = map(vec![pane, box_]);
        add_crate(&mut data, 1, Vec3::new(0.0, 0.0, 160.0));
        let mut sim = sim(data);
        sim.seconds(2.0);
        assert_eq!(broken(&sim, 0), breaks, "flags {flags}");
        let z = crate_at(&mut sim).z;
        if breaks {
            assert!(z < 20.0, "fell through to the floor: {z}");
        } else {
            assert!((z - 81.0).abs() < 2.0, "rests on the pane: {z}");
        }
    }
}

/// "Break immediately on physics" (512): a slow touch breaks even a
/// sturdy wooden board.
#[test]
fn break_on_physics_breaks_at_the_first_impact() {
    let board = entity(
        &[
            ("classname", "func_breakable"),
            ("origin", "0 0 40"),
            ("material", "1"),
            ("health", "500"),
            ("spawnflags", "512"),
        ],
        vec![hull(Vec3::new(-48.0, -48.0, -2.0), Vec3::new(48.0, 48.0, 2.0))],
        true,
    );
    let box_ = entity(&[("classname", "prop_physics"), ("origin", "0 0 60")], Vec::new(), false);
    let mut data = map(vec![board, box_]);
    add_crate(&mut data, 1, Vec3::new(0.0, 0.0, 60.0));
    let mut sim = sim(data);
    sim.seconds(1.5);
    assert!(broken(&sim, 0));
}

/// A breakable with explodemagnitude explodes as it breaks (the HE
/// grenade's blast) and breaks the wooden one beside it.
#[test]
fn an_exploding_breakable_breaks_its_neighbour() {
    let barrel = entity(
        &[
            ("classname", "func_breakable"),
            ("targetname", "barrel"),
            ("origin", "0 0 32"),
            ("material", "2"),
            ("health", "10"),
            ("explodemagnitude", "100"),
        ],
        vec![hull(Vec3::splat(-16.0), Vec3::splat(16.0))],
        true,
    );
    let crate_ = entity(
        &[
            ("classname", "func_breakable"),
            ("origin", "80 0 32"),
            ("material", "1"),
            ("health", "40"),
        ],
        vec![hull(Vec3::splat(-16.0), Vec3::splat(16.0))],
        true,
    );
    let far = entity(
        &[
            ("classname", "func_breakable"),
            ("origin", "1000 0 32"),
            ("material", "1"),
            ("health", "40"),
        ],
        vec![hull(Vec3::splat(-16.0), Vec3::splat(16.0))],
        true,
    );
    let mut sim = sim(map(vec![barrel, crate_, far]));
    sim.ticks(5);
    sim.app
        .world_mut()
        .resource_mut::<Logic>()
        .world
        .queue_input("barrel", "Break", Value::Void, 0.0, None);
    sim.ticks(10);
    assert!(broken(&sim, 0));
    assert!(broken(&sim, 1), "the blast broke its neighbour");
    assert!(!broken(&sim, 2), "out of reach");
}

/// A model door open back (its leaf along -X) closing on a crate that
/// has a wall behind it: the door stops at the crate, while a forceclosed
/// one shoves it through and closes.
#[test]
fn a_pinned_crate_blocks_a_closing_model_door_unless_forceclosed() {
    use mashup::logic::movers::DoorState;
    for (forceclosed, closes) in [("0", false), ("1", true)] {
        let door = entity(
            &[
                ("classname", "prop_door_rotating"),
                ("targetname", "door"),
                ("origin", "0 0 1"),
                ("spawnpos", "2"),
                ("returndelay", "-1"),
                ("speed", "100"),
                ("distance", "90"),
                ("forceclosed", forceclosed),
            ],
            vec![hull(Vec3::new(-1.0, 0.0, 0.0), Vec3::new(1.0, 52.0, 100.0))],
            true,
        );
        let box_ = entity(&[("classname", "prop_physics"), ("origin", "-30 17.5 17")], Vec::new(), false);
        let mut data = map(vec![door, box_]);
        add_crate(&mut data, 1, Vec3::new(-30.0, 17.5, 17.0));
        // The wall above the crate, clear of the leaf's swing ends.
        solid(&mut data, Vec3::new(-60.0, 34.0, 0.0), Vec3::new(-5.0, 60.0, 100.0));
        let mut sim = sim(data);
        sim.ticks(10);
        sim.app
            .world_mut()
            .resource_mut::<Logic>()
            .world
            .queue_input("door", "Close", Value::Void, 0.0, None);
        sim.seconds(2.0);
        let logic = sim.app.world().resource::<Logic>();
        let id = logic
            .world
            .ids()
            .into_iter()
            .find(|id| logic.world.get(*id).unwrap().map_index == Some(0))
            .unwrap();
        let d = logic.world.prop_door(id).unwrap();
        eprintln!("forceclosed {forceclosed}: {:?} at {}", d.state, d.push.angles.y);
        if closes {
            assert_eq!((d.state, d.push.angles.y), (DoorState::Closed, 0.0));
        } else {
            assert_eq!(d.state, DoorState::Closing);
            assert!(d.push.angles.y > 10.0, "stopped at the crate: {}", d.push.angles.y);
        }
    }
}

/// Water (a 128-unit-deep pool): a light crate dropped in it comes up and
/// floats at the surface (map::buoyancy); out of the water the same crate
/// rests on the floor.
#[test]
fn a_crate_floats_in_water() {
    for (water, floats) in [(true, true), (false, false)] {
        let box_ = entity(
            &[("classname", "prop_physics"), ("origin", "0 0 40")],
            Vec::new(),
            false,
        );
        let mut data = map(vec![box_]);
        add_crate(&mut data, 0, Vec3::new(0.0, 0.0, 40.0));
        if water {
            let (a, b) = (
                to_engine(Vec3::new(-512.0, -512.0, 0.0)),
                to_engine(Vec3::new(512.0, 512.0, 128.0)),
            );
            data.water.push(mashup::core::MapWaterVolume {
                brush: MapBrush::from_box(a.min(b), a.max(b)),
                slime: false,
            });
        }
        let mut sim = sim(data);
        sim.seconds(5.0);
        let z = crate_at(&mut sim).z;
        if floats {
            // 20 kg in 32768 cubic inches: mostly above the surface.
            assert!(z > 110.0 && z < 150.0, "floats at the surface: {z}");
        } else {
            assert!(z < 20.0, "rests on the floor: {z}");
        }
    }
}

/// phys_ballsocket holding a crate to the world 32 units above it: it
/// hangs there instead of falling (physics_constraints.md); Break lets go.
#[test]
fn a_ball_socket_holds_a_crate_until_it_breaks() {
    let box_ = entity(
        &[
            ("classname", "prop_physics"),
            ("targetname", "lamp"),
            ("origin", "0 0 200"),
        ],
        Vec::new(),
        false,
    );
    let socket = entity(
        &[
            ("classname", "phys_ballsocket"),
            ("targetname", "socket"),
            ("origin", "0 0 232"),
            ("attach1", "lamp"),
        ],
        Vec::new(),
        false,
    );
    let mut data = map(vec![box_, socket]);
    add_crate(&mut data, 0, Vec3::new(0.0, 0.0, 200.0));
    let mut sim = sim(data);
    sim.seconds(2.0);
    let z = crate_at(&mut sim).z;
    assert!((z - 200.0).abs() < 8.0, "hangs from the socket: {z}");
    sim.app
        .world_mut()
        .resource_mut::<Logic>()
        .world
        .queue_input("socket", "Break", Value::Void, 0.0, None);
    sim.seconds(2.0);
    let z = crate_at(&mut sim).z;
    assert!(z < 20.0, "fell once broken: {z}");
}

/// phys_motor with "Hinge Object" (spawnflags 7, as surf_surreal's
/// flowers): the crate turns about the motor's vertical axis at its speed
/// and stays where it is.
#[test]
fn a_motor_spins_its_body_on_a_hinge() {
    use avian3d::prelude::AngularVelocity;
    let box_ = entity(
        &[
            ("classname", "prop_physics"),
            ("targetname", "flower"),
            ("origin", "0 0 100"),
        ],
        Vec::new(),
        false,
    );
    let motor = entity(
        &[
            ("classname", "phys_motor"),
            ("origin", "0 0 100"),
            ("axis", "0 0 140"),
            ("speed", "90"),
            ("spinup", "1"),
            ("spawnflags", "7"),
            ("attach1", "flower"),
        ],
        Vec::new(),
        false,
    );
    let mut data = map(vec![box_, motor]);
    add_crate(&mut data, 0, Vec3::new(0.0, 0.0, 100.0));
    let mut sim = sim(data);
    sim.seconds(3.0);
    let at = crate_at(&mut sim);
    assert!(
        (at.z - 100.0).abs() < 4.0 && at.x.abs() < 4.0,
        "held on its hinge: {at}"
    );
    let world = sim.app.world_mut();
    let node = world.query::<(Entity, &PropEntity)>().iter(world).next().unwrap().0;
    let w = world.get::<AngularVelocity>(node).unwrap().0;
    // 90 deg/s about Source +Z, engine +Y.
    assert!(
        (w.y.abs() - 90f32.to_radians()).abs() < 0.2,
        "spins at the motor's speed: {w}"
    );
}
