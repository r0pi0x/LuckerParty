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
        physics: None,
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
    // A unit above the ground, to land on it.
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

/// A kz booster pad: `!activator AddOutput basevelocity 0 0 400` when the
/// player walks in. The keyvalue is the base velocity (replaced); the
/// player's next move turns it into velocity x (1 + dt/2) (triggers.md,
/// trigger_push step 1 and open question 9), which at over 250 takes it
/// off the ground: 403 up, less that tick's gravity (12).
#[test]
fn basevelocity_booster_launches_by_the_base_velocity_rule() {
    let pad = entity(
        &[
            ("classname", "trigger_multiple"),
            ("spawnflags", "1"),
            ("wait", "1"),
            ("OnStartTouch", "!activator,AddOutput,basevelocity 0 0 400,0,-1"),
        ],
        vec![hull(Vec3::new(200.0, -64.0, 0.0), Vec3::new(264.0, 64.0, 16.0))],
        false,
    );
    let (mut sim, p) = sim(vec![pad], Vec3::ZERO, 0.0);
    sim.app.world_mut().get_mut::<Transform>(p).unwrap().translation = to_engine(Vec3::new(232.0, 0.0, 37.0));
    let mut peak = 0.0f32;
    let mut rise = 0.0f32;
    for _ in 0..30 {
        sim.ticks(1);
        peak = peak.max(to_source(sim.velocity(p)).z);
        rise = rise.max(feet(&sim, p).z);
    }
    let dt = cs_source::TICK_INTERVAL as f32;
    let want = 400.0 * (1.0 + dt / 2.0) - 800.0 * dt;
    assert!((peak - want).abs() < 0.5, "launch speed {peak}, want {want}");
    assert!(rise > 80.0, "rose {rise}");
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
        force: bevy::math::Vec3::ZERO,
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

/// Every contact pair avian links into an island once: a pair still
/// waiting to switch its constraints on must not be in one yet, and a
/// collider of an enabled body must not carry a "body disabled" proxy.
/// Either one, left over from a restored prop, later links a contact
/// twice and trips avian's island assertion (`contact.island.is_none()`).
fn island_bookkeeping_errors(world: &World) -> Vec<String> {
    use avian3d::{
        collider_tree::{ColliderTreeProxyFlags, ColliderTreeProxyKey, ColliderTrees},
        prelude::*,
    };
    let mut errors = Vec::new();
    let graph = world.resource::<ContactGraph>();
    for p in graph.active_pairs().iter().chain(graph.sleeping_pairs()) {
        let Some(edge) = graph.get_edge_by_id(p.contact_id) else { continue };
        if edge.island.is_some() && p.flags.contains(ContactPairFlags::STARTED_GENERATING_CONSTRAINTS) {
            errors.push(format!("contact {} / {} is in an island and about to join one", p.collider1, p.collider2));
        }
    }
    let trees = world.resource::<ColliderTrees>();
    for (e, key, of) in world
        .try_query::<(Entity, &ColliderTreeProxyKey, &ColliderOf)>()
        .unwrap()
        .iter(world)
    {
        let disabled = world.get::<RigidBodyDisabled>(of.body).is_some();
        if let Some(proxy) = trees.get_proxy(*key)
            && proxy.flags.contains(ColliderTreeProxyFlags::BODY_DISABLED) != disabled
        {
            errors.push(format!("collider {e}: proxy says body disabled {}, body {disabled}", !disabled));
        }
    }
    errors
}

#[test]
fn restored_prop_contacts_join_islands_once() {
    use avian3d::prelude::{LinearVelocity, RigidBody, RigidBodyDisabled};
    use mashup::{
        logic::Value,
        map::{MapCollision, MapConvex, MapModel, MapPhysics, MapProp, PropEntity, PropSolid, PushAway},
    };
    // A physics crate resting on the floor, away from the player.
    let crate_ = entity(&[("classname", "prop_physics"), ("targetname", "crate"), ("origin", "300 0 0")], Vec::new(), false);
    let mut data = map(vec![crate_]);
    let half = 16.0 * SCALE;
    let (lo, hi) = (Vec3::new(-half, 0.0, -half), Vec3::new(half, 2.0 * half, half));
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
        model: 0,
        translation: to_engine(Vec3::new(300.0, 0.0, 0.0)),
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
        entity: Some(0),
        skin: 0,
        body: 0,
    });
    let mut sim = Sim::new((MapPlugin::new(data), SourceMovementPlugin));
    sim.set_tick_interval(cs_source::TICK_INTERVAL);
    sim.spawn_character(to_engine(Vec3::new(-300.0, 0.0, 37.0)), movement::ID);
    sim.ticks(30);
    let world = sim.app.world_mut();
    let node = world
        .query::<(Entity, &PropEntity)>()
        .iter(world)
        .find(|(_, p)| p.0 == 0)
        .expect("crate node")
        .0;
    assert!(world.get::<RigidBody>(node).is_some_and(|b| b.is_dynamic()));
    assert_eq!(island_bookkeeping_errors(sim.app.world()), Vec::<String>::new());

    // Killed (as when broken): its body stops.
    sim.app
        .world_mut()
        .resource_mut::<Logic>()
        .world
        .queue_input("crate", "Kill", Value::Void, 0.0, None);
    sim.ticks(5);
    assert!(!exists(&sim, 0), "killed");
    assert!(sim.app.world().get::<RigidBodyDisabled>(node).is_some());

    // A new round brings it back where it was, on the floor.
    sim.app.world_mut().resource_mut::<RoundRestarts>().0 += 1;
    for i in 0..120 {
        sim.ticks(1);
        if i % 20 == 10 {
            // Shoved about: its contacts with the floor change.
            sim.app.world_mut().get_mut::<LinearVelocity>(node).unwrap().0 = Vec3::new(1.5, 0.5, -0.5);
        }
        let errors = island_bookkeeping_errors(sim.app.world());
        assert!(errors.is_empty(), "tick {i} after the restart: {errors:?}");
    }
    assert!(exists(&sim, 0), "re-created");
    assert!(sim.app.world().get::<RigidBodyDisabled>(node).is_none());
}

/// A map's point_servercommand sets server settings for as long as the
/// map is loaded (specs/source/entity_io.md, "point_servercommand"; the
/// rule in `logic::classes::check_server_command`): a known setting is
/// applied and put back when another map loads or the map unloads
/// (disconnect); commands and settings mashup lacks are ignored.
#[test]
fn map_server_settings_last_until_the_map_unloads() {
    use mashup::console::Console;
    let maxvelocity = |sim: &mut Sim| {
        let world = sim.app.world_mut();
        let cvar = world.resource::<Console>().cvar("sv_maxvelocity").cloned().unwrap();
        (cvar.get)(world).unwrap()
    };
    let lines = |sim: &Sim| -> Vec<String> {
        let c = sim.app.world().resource::<Console>();
        c.output.iter().map(|l| l.text.clone()).filter(|t| t.starts_with("point_servercommand")).collect()
    };
    let entities = vec![
        entity(&[("classname", "point_servercommand"), ("targetname", "server")], Vec::new(), false),
        entity(
            &[
                ("classname", "logic_auto"),
                ("OnMapSpawn", "server,Command,sv_maxvelocity 5000,0,-1"),
                ("OnMapSpawn", "server,Command,sv_cheats 1,0,-1"),
                ("OnMapSpawn", "server,Command,quit,0,-1"),
                ("OnMapSpawn", "server,Command,sv_maxvelocity 6000,0.1,-1"),
            ],
            Vec::new(),
            false,
        ),
    ];
    let (mut sim, _) = sim(entities.clone(), Vec3::ZERO, 0.0);
    sim.ticks(40);
    assert_eq!(maxvelocity(&mut sim), "6000", "the map's last value");
    let l = lines(&sim);
    assert!(l.iter().any(|t| t == "point_servercommand: sv_maxvelocity 5000 (was 3500)"), "{l:?}");
    assert!(l.iter().any(|t| t == "point_servercommand: sv_maxvelocity 6000"), "{l:?}");
    assert!(l.iter().any(|t| t.contains("refused 'sv_cheats 1' (a setting maps may not change)")), "{l:?}");
    assert!(l.iter().any(|t| t.contains("refused 'quit' (not a game setting)")), "{l:?}");

    // Another map: the setting goes back to what it was before the first.
    mashup::map::change_map(sim.app.world_mut(), map(Vec::new()), mashup::map::MapDebugView::Normal);
    sim.ticks(5);
    assert_eq!(maxvelocity(&mut sim), "3500");
    assert!(lines(&sim).iter().any(|t| t == "point_servercommand: map unloaded, sv_maxvelocity back to 3500"));

    // The map again, then unloaded (disconnect).
    mashup::map::change_map(sim.app.world_mut(), map(entities), mashup::map::MapDebugView::Normal);
    sim.ticks(40);
    assert_eq!(maxvelocity(&mut sim), "6000");
    mashup::map::unload_map(sim.app.world_mut());
    sim.ticks(5);
    assert_eq!(maxvelocity(&mut sim), "3500");
}
