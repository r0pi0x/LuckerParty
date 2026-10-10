//! Map logic that community maps (docs/plans/active/community-maps.md,
//! "Map logic audit") rely on, from small built-in entity lists. dt =
//! 0.015 s.

use bevy::prelude::*;

use super::Value;
use super::classes::Class;
use super::movers::pusher;
use super::tests::{fired, got, kv, origin_of, player_at, run_to, spawn, spawn_brush, world};
use super::world::*;

/// A func_movelinear lift "m" going up 100 units at 100 units/s.
fn lift(w: &mut LogicWorld) -> EntId {
    spawn_brush(
        w,
        &[
            ("classname", "func_movelinear"),
            ("targetname", "m"),
            ("movedir", "-90 0 0"),
            ("MoveDistance", "100"),
            ("speed", "100"),
        ],
        Vec3::new(-32.0, -32.0, -4.0),
        Vec3::new(32.0, 32.0, 0.0),
    )
}

#[test]
fn movelinear_set_speed_zero_pauses() {
    // mg_swag_multigames_v1's lifts: Open, SetSpeed 0 a while later
    // (a stop between floors), SetSpeed 100 to go on. Speed 0 makes the
    // move's time infinite (doors_buttons.md, linear move): it holds.
    let mut w = world();
    let m = lift(&mut w);
    w.activate();
    w.queue_input("m", "Open", Value::Void, 0.0, None);
    w.queue_input("m", "SetSpeed", Value::Str("0".into()), 0.5, None);
    run_to(&mut w, 100);
    let z = origin_of(&w, m).z;
    assert!((45.0..=55.0).contains(&z), "paused near half way, at {z}");
    assert!(fired(&w, m, "OnFullyOpen").is_empty(), "no arrival while paused");
    w.queue_input("m", "SetSpeed", Value::Str("100".into()), 0.0, None);
    run_to(&mut w, 200);
    assert_eq!(origin_of(&w, m).z, 100.0);
    assert_eq!(fired(&w, m, "OnFullyOpen").len(), 1);
}

#[test]
fn children_of_a_mover_ride_it() {
    // A lift with a trigger_teleport, a teleport destination and a door
    // parented to it (mg_swag_multigames_v1, mg_boatrace_scramble,
    // mg_kommando): they move with it.
    let mut w = world();
    let m = lift(&mut w);
    let t = spawn_brush(
        &mut w,
        &[
            ("classname", "trigger_teleport"),
            ("parentname", "m"),
            ("target", "dest"),
            ("spawnflags", "1"),
        ],
        Vec3::new(200.0, -16.0, 0.0),
        Vec3::new(232.0, 16.0, 32.0),
    );
    let dest = spawn(
        &mut w,
        &[
            ("classname", "info_teleport_destination"),
            ("targetname", "dest"),
            ("parentname", "m"),
            ("origin", "0 0 10"),
        ],
    );
    let door = spawn_brush(
        &mut w,
        &[
            ("classname", "func_door"),
            ("targetname", "door"),
            ("parentname", "m"),
            ("movedir", "0 90 0"),
            ("speed", "100"),
            ("lip", "0"),
            ("wait", "-1"),
        ],
        Vec3::new(-8.0, -32.0, 0.0),
        Vec3::new(8.0, 32.0, 64.0),
    );
    w.activate();
    w.link_anchored();
    w.queue_input("m", "Open", Value::Void, 0.0, None);
    run_to(&mut w, 150);
    assert_eq!(origin_of(&w, m).z, 100.0);
    assert!(
        (w.get(dest).unwrap().origin.z - 110.0).abs() < 1e-3,
        "destination rode up"
    );
    assert!((origin_of(&w, door).z - 100.0).abs() < 1e-3, "door rode up");
    // The door's own motion goes on in the lift's frame.
    w.queue_input("door", "Open", Value::Void, 0.0, None);
    run_to(&mut w, 300);
    let d = origin_of(&w, door);
    assert!((d.z - 100.0).abs() < 1e-3 && d.y > 60.0, "door opened up there: {d}");
    // The trigger's volume is up there too: a player standing in it at
    // the top is teleported to the destination.
    let Class::Trigger(trig) = &w.get(t).unwrap().class else {
        panic!()
    };
    assert!(trig.brushes[0].min.z > 99.0, "trigger rode up");
    let p = player_at(&mut w, 1, Vec3::new(216.0, 0.0, 100.0));
    run_to(&mut w, 305);
    let pl = w.player(p).unwrap();
    assert!(
        pl.teleported && (pl.origin.z - 110.0).abs() < 1e-3,
        "teleported to {}",
        pl.origin
    );
    let _ = kv(&[]);
}

#[test]
fn a_mover_child_carries_its_parents_velocity_for_riders() {
    let mut w = world();
    let m = lift(&mut w);
    let door = spawn_brush(
        &mut w,
        &[
            ("classname", "func_door"),
            ("parentname", "m"),
            ("movedir", "0 90 0"),
            ("speed", "100"),
        ],
        Vec3::new(-8.0, -32.0, 0.0),
        Vec3::new(8.0, 32.0, 64.0),
    );
    w.activate();
    w.link_anchored();
    w.queue_input("m", "Open", Value::Void, 0.0, None);
    run_to(&mut w, 20);
    let (.., velocity, _, _) = w.mover_poses().into_iter().find(|p| p.0 == door).unwrap();
    assert!(
        (velocity.z - 100.0).abs() < 1e-3,
        "rides at the lift's speed: {velocity}"
    );
    assert!(pusher(&w.get(m).unwrap().class).is_some());
}

#[test]
fn physics_bodies_touch_physics_triggers() {
    // mg_boatrace_scramble: func_physbox boats cross trigger_push
    // boosters and a trigger_once finish line (spawnflags 8, a class
    // filter for func_physbox).
    let mut w = world();
    spawn(
        &mut w,
        &[
            ("classname", "filter_activator_class"),
            ("targetname", "boats"),
            ("filterclass", "func_physbox"),
        ],
    );
    let boat = spawn_brush(
        &mut w,
        &[("classname", "func_physbox"), ("targetname", "boat")],
        Vec3::splat(-16.0),
        Vec3::splat(16.0),
    );
    spawn_brush(
        &mut w,
        &[
            ("classname", "trigger_once"),
            ("spawnflags", "8"),
            ("filtername", "boats"),
            ("OnStartTouch", "!activator,FireUser1,,0,-1"),
        ],
        Vec3::new(100.0, -64.0, -64.0),
        Vec3::new(132.0, 64.0, 64.0),
    );
    spawn_brush(
        &mut w,
        &[
            ("classname", "trigger_push"),
            ("spawnflags", "8"),
            ("filtername", "boats"),
            ("speed", "900"),
            ("pushdir", "0 90 0"),
        ],
        Vec3::new(-64.0, -64.0, -64.0),
        Vec3::new(64.0, 64.0, 64.0),
    );
    spawn_brush(
        &mut w,
        &[
            ("classname", "trigger_teleport"),
            ("spawnflags", "8"),
            ("target", "far"),
        ],
        Vec3::new(300.0, -64.0, -64.0),
        Vec3::new(332.0, 64.0, 64.0),
    );
    spawn(
        &mut w,
        &[
            ("classname", "info_teleport_destination"),
            ("targetname", "far"),
            ("origin", "0 5000 0"),
            ("angles", "0 90 0"),
        ],
    );
    // A player-only trigger the boat never touches.
    spawn_brush(
        &mut w,
        &[
            ("classname", "trigger_multiple"),
            ("spawnflags", "1"),
            ("OnStartTouch", "!activator,FireUser2,,0,-1"),
        ],
        Vec3::splat(-64.0),
        Vec3::splat(64.0),
    );
    w.activate();
    let mass = match &w.get(boat).unwrap().class {
        Class::Prop(p) => {
            assert!(p.physics, "a physics brush is a physics body");
            p.mass
        }
        _ => panic!("func_physbox is a prop"),
    };
    // In the booster: pushed every tick, by speed x 100 x dt / mass.
    w.set_prop_bounds(boat, (Vec3::splat(-16.0), Vec3::splat(16.0)));
    w.frame(&NoCollision);
    let dv = w
        .effects
        .iter()
        .find_map(|e| match e {
            Effect::BodyVelocity { id, velocity } if *id == boat => Some(*velocity),
            _ => None,
        })
        .expect("the booster pushes the boat");
    let want = 900.0 * 100.0 * DT / mass.max(1.0);
    assert!((dv.y - want).abs() < 1e-3 && dv.x.abs() < 1e-3, "{dv} vs {want}");
    assert!(
        got(&w, boat, "FireUser2").is_empty(),
        "player-only trigger ignores bodies"
    );
    // Across the finish line: OnStartTouch with the boat as activator.
    w.effects.clear();
    w.set_prop_bounds(boat, (Vec3::new(100.0, -16.0, -16.0), Vec3::new(132.0, 16.0, 16.0)));
    w.frame(&NoCollision);
    w.frame(&NoCollision);
    assert_eq!(got(&w, boat, "FireUser1").len(), 1);
    // Into the teleport: to the destination with its angles.
    w.effects.clear();
    w.set_prop_bounds(boat, (Vec3::new(300.0, -16.0, -16.0), Vec3::new(332.0, 16.0, 16.0)));
    w.frame(&NoCollision);
    assert!(w.effects.iter().any(|e| matches!(e,
        Effect::BodyTeleport { id, origin, angles: Some(a) }
            if *id == boat && *origin == Vec3::new(0.0, 5000.0, 0.0) && a.y == 90.0)));
}

#[test]
fn players_classname_marks_their_stage() {
    // kz_bhop_izanami: a stage's start trigger sets the toucher's
    // classname; the stage's fall teleport is filtered by that class.
    let mut w = world();
    spawn(
        &mut w,
        &[
            ("classname", "filter_activator_class"),
            ("targetname", "f_a1"),
            ("filterclass", "A1"),
        ],
    );
    spawn_brush(
        &mut w,
        &[
            ("classname", "trigger_multiple"),
            ("spawnflags", "1"),
            ("wait", "1"),
            ("OnStartTouch", "!activator,AddOutput,classname A1,0,-1"),
        ],
        Vec3::new(-16.0, -16.0, 0.0),
        Vec3::new(16.0, 16.0, 16.0),
    );
    spawn_brush(
        &mut w,
        &[
            ("classname", "trigger_teleport"),
            ("spawnflags", "1"),
            ("filtername", "f_a1"),
            ("target", "cp1"),
        ],
        Vec3::new(500.0, -16.0, 0.0),
        Vec3::new(532.0, 16.0, 16.0),
    );
    spawn(
        &mut w,
        &[
            ("classname", "info_teleport_destination"),
            ("targetname", "cp1"),
            ("origin", "0 0 300"),
        ],
    );
    w.activate();
    let p = player_at(&mut w, 0, Vec3::new(516.0, 0.0, 0.0));
    run_to(&mut w, 3);
    assert!(
        !w.player(p).unwrap().teleported,
        "a plain player fails the stage filter"
    );
    assert_eq!(w.class_of(Who::Player(p)), "player");
    w.player_mut(p).unwrap().origin = Vec3::ZERO;
    run_to(&mut w, 6);
    assert_eq!(w.class_of(Who::Player(p)), "A1");
    w.player_mut(p).unwrap().origin = Vec3::new(516.0, 0.0, 0.0);
    run_to(&mut w, 8);
    let pl = w.player(p).unwrap();
    assert!(
        pl.teleported && pl.origin.z == 300.0,
        "back to the stage start: {}",
        pl.origin
    );
    // Found by its new classname, no longer as "player".
    assert_eq!(w.resolve("A1", None, None), vec![Who::Player(p)]);
    assert!(w.resolve("player", None, None).is_empty());
}

#[test]
fn addoutput_keyvalues_that_change_behaviour() {
    let mut w = world();
    let r = spawn_brush(
        &mut w,
        &[
            ("classname", "func_rotating"),
            ("targetname", "fan"),
            ("maxspeed", "100"),
        ],
        Vec3::splat(-8.0),
        Vec3::splat(8.0),
    );
    let equip = spawn(
        &mut w,
        &[
            ("classname", "game_player_equip"),
            ("targetname", "eq"),
            ("spawnflags", "1"),
        ],
    );
    let prop = spawn(&mut w, &[("classname", "info_target"), ("targetname", "thing")]);
    spawn(
        &mut w,
        &[
            ("classname", "filter_activator_class"),
            ("targetname", "f"),
            ("filterclass", "crashed"),
        ],
    );
    w.activate();
    w.queue_input("fan", "AddOutput", Value::Str("maxspeed 400".into()), 0.0, None);
    w.queue_input("fan", "Start", Value::Void, 0.0, None);
    w.queue_input("eq", "AddOutput", Value::Str("weapon_knife 1".into()), 0.0, None);
    w.queue_input("thing", "AddOutput", Value::Str("classname crashed".into()), 0.0, None);
    run_to(&mut w, 30);
    let Class::Rotating(rot) = &w.get(r).unwrap().class else {
        panic!()
    };
    assert_eq!(rot.speed, 400.0, "spins up to the new max speed");
    let Class::Equip(q) = &w.get(equip).unwrap().class else {
        panic!()
    };
    assert_eq!(q.items, vec![("weapon_knife".to_string(), 1)]);
    let f = w.find("f").unwrap();
    assert!(
        super::classes::filter_passes(&w, f, Some(Who::Ent(prop))),
        "filters see the new classname"
    );
    assert!(!w.log.iter().any(|l| l.contains("has no effect")), "{:?}", w.log);
}

fn angles_now(w: &LogicWorld, id: EntId) -> Vec3 {
    pusher(&w.get(id).unwrap().class).unwrap().angles
}

#[test]
fn rotating_buttons_turn() {
    // func_rot_button (mg_swag_multigames_v1's walls): Press turns it
    // "distance" degrees (yaw), it comes back after "wait".
    let mut w = world();
    let b = spawn_brush(
        &mut w,
        &[
            ("classname", "func_rot_button"),
            ("targetname", "wall"),
            ("distance", "90"),
            ("speed", "90"),
            ("wait", "1"),
            ("OnPressed", "!self,FireUser1,,0,-1"),
        ],
        Vec3::splat(-8.0),
        Vec3::splat(8.0),
    );
    // momentary_rot_button (surf_boreas, mg_boatrace_scramble):
    // SetPosition turns it at its speed; arriving at 1 fires OnFullyOpen
    // and Position; SetPositionImmediately jumps without outputs.
    let m = spawn_brush(
        &mut w,
        &[
            ("classname", "momentary_rot_button"),
            ("targetname", "wheel"),
            ("distance", "-360"),
            ("speed", "180"),
            ("spawnflags", "33"),
            ("OnFullyOpen", "!self,FireUser2,,0,-1"),
            ("OnFullyClosed", "!self,FireUser3,,0,-1"),
        ],
        Vec3::splat(-8.0),
        Vec3::splat(8.0),
    );
    w.activate();
    w.queue_input("wall", "Press", Value::Void, 0.0, None);
    w.queue_input("wheel", "SetPosition", Value::Str("1".into()), 0.0, None);
    run_to(&mut w, 40);
    assert!((angles_now(&w, b).y - 54.0).abs() < 2.0, "turning: {}", angles_now(&w, b));
    assert_eq!(got(&w, b, "FireUser1").len(), 1);
    run_to(&mut w, 100);
    assert_eq!(angles_now(&w, b).y, 90.0, "pressed in");
    run_to(&mut w, 300);
    assert_eq!(angles_now(&w, b).y, 0.0, "back out after its wait");
    assert_eq!(angles_now(&w, m).y, -360.0, "a full turn backwards in 2 s");
    assert_eq!(got(&w, m, "FireUser2").len(), 1);
    assert!(w.fired.iter().any(|(_, e, o)| *e == m && o == "Position"));
    w.queue_input("wheel", "SetPositionImmediately", Value::Str("0".into()), 0.0, None);
    run_to(&mut w, 305);
    assert_eq!(angles_now(&w, m).y, 0.0);
    assert!(got(&w, m, "FireUser3").is_empty(), "no outputs when set at once");
}

#[test]
fn measure_movement_mirrors_the_player() {
    // A turret moved by the player's eye relative to a zone, scaled
    // (mg_creative_multigames_v8_ns's paint turret).
    let mut w = world();
    spawn(&mut w, &[("classname", "info_target"), ("targetname", "zone"), ("origin", "100 0 0")]);
    spawn(&mut w, &[("classname", "info_target"), ("targetname", "base"), ("origin", "1000 0 0")]);
    let t = spawn_brush(
        &mut w,
        &[("classname", "func_movelinear"), ("targetname", "turret"), ("origin", "1000 0 0")],
        Vec3::splat(-4.0),
        Vec3::splat(4.0),
    );
    spawn(
        &mut w,
        &[
            ("classname", "logic_measure_movement"),
            ("targetname", "mm"),
            ("MeasureTarget", "pilot"),
            ("MeasureReference", "zone"),
            ("Target", "turret"),
            ("TargetReference", "base"),
            ("TargetScale", "2"),
            ("MeasureType", "0"),
        ],
    );
    w.activate();
    let p = player_at(&mut w, 0, Vec3::new(110.0, 5.0, 0.0));
    w.player_mut(p).unwrap().view = Vec3::new(0.0, 90.0, 0.0);
    w.player_names.push((p, "pilot".into()));
    run_to(&mut w, 5);
    let o = origin_of(&w, t);
    assert!((o - Vec3::new(1020.0, 10.0, 0.0)).length() < 1e-3, "{o}");
    assert!((angles_now(&w, t).y - 90.0).abs() < 1e-3);
    w.queue_input("mm", "Disable", Value::Void, 0.0, None);
    run_to(&mut w, 7);
    w.player_mut(p).unwrap().origin = Vec3::new(150.0, 0.0, 0.0);
    run_to(&mut w, 12);
    assert!((origin_of(&w, t) - Vec3::new(1020.0, 10.0, 0.0)).length() < 1e-3, "disabled: stays");
}

#[test]
fn point_teleport_multicompare_and_shake() {
    let mut w = world();
    // point_teleport moves its target (mg_lt_galaxy_v5 moves a spinning
    // brush) and the activator by name.
    let r = spawn_brush(
        &mut w,
        &[("classname", "func_rotating"), ("targetname", "sky"), ("origin", "0 0 0")],
        Vec3::splat(-8.0),
        Vec3::splat(8.0),
    );
    spawn(
        &mut w,
        &[("classname", "point_teleport"), ("targetname", "tp"), ("target", "sky"), ("origin", "500 0 64")],
    );
    spawn(
        &mut w,
        &[("classname", "point_teleport"), ("targetname", "tp2"), ("target", "!activator"), ("origin", "0 900 0"), ("angles", "0 180 0")],
    );
    // logic_multicompare with no values compares equal (mg_starwars_v1
    // fires its lasers that way).
    spawn(
        &mut w,
        &[("classname", "logic_multicompare"), ("targetname", "mc"), ("OnEqual", "!self,FireUser1,,0,-1"), ("OnNotEqual", "!self,FireUser2,,0,-1")],
    );
    spawn(
        &mut w,
        &[("classname", "env_shake"), ("targetname", "quake"), ("amplitude", "4"), ("radius", "500"), ("duration", "2"), ("frequency", "20")],
    );
    w.activate();
    let p = player_at(&mut w, 0, Vec3::ZERO);
    w.queue_input("tp", "Teleport", Value::Void, 0.0, None);
    w.queue_input("tp2", "Teleport", Value::Void, 0.0, Some(Who::Player(p)));
    w.queue_input("mc", "CompareValues", Value::Void, 0.0, None);
    w.queue_input("mc", "UpdateValue", Value::Str("1".into()), 0.01, None);
    w.queue_input("mc", "UpdateValue", Value::Str("2".into()), 0.01, None);
    w.queue_input("mc", "CompareValues", Value::Void, 0.02, None);
    w.queue_input("quake", "StartShake", Value::Void, 0.0, None);
    run_to(&mut w, 1);
    assert_eq!(origin_of(&w, r), Vec3::new(500.0, 0.0, 64.0));
    let pl = w.player(p).unwrap();
    assert!(pl.teleported && pl.origin == Vec3::new(0.0, 900.0, 0.0) && pl.view.y == 180.0);
    assert!(w.effects.iter().any(|e| matches!(e, Effect::Shake { amplitude, radius, .. } if *amplitude == 4.0 && *radius == 500.0)));
    run_to(&mut w, 5);
    let mc = w.find("mc").unwrap();
    assert_eq!(got(&w, mc, "FireUser1").len(), 1);
    assert_eq!(got(&w, mc, "FireUser2").len(), 1);
}

#[test]
fn color_input_sets_the_render_colour() {
    // mg_creative_multigames_v8_ns: yellow buttons turn red once pressed
    // (OnPressed !self Color "255 0 0"); a round restart puts the map's
    // colour back.
    let mut w = world();
    let b = spawn_brush(
        &mut w,
        &[("classname", "func_button"), ("targetname", "l"), ("rendercolor", "255 255 0"), ("spawnflags", "1024")],
        Vec3::splat(-8.0),
        Vec3::splat(8.0),
    );
    let wall = spawn(&mut w, &[("classname", "func_wall"), ("targetname", "w")]);
    w.activate();
    assert_eq!(w.get(b).unwrap().render_color, [255, 255, 0]);
    assert_eq!(w.get(wall).unwrap().render_color, [255, 255, 255]);
    w.queue_input("l", "Color", Value::Str("255 0 0".into()), 0.0, None);
    w.queue_input("w", "AddOutput", Value::Str("rendercolor 0 128 255".into()), 0.0, None);
    w.queue_input("w", "Alpha", Value::Str("300".into()), 0.0, None);
    run_to(&mut w, 2);
    assert_eq!(w.get(b).unwrap().render_color, [255, 0, 0]);
    assert_eq!(w.get(wall).unwrap().render_color, [0, 128, 255]);
    assert_eq!(w.get(wall).unwrap().kv("renderamt"), Some("255"));
    assert!(!w.log.iter().any(|l| l.contains("unhandled") || l.contains("no effect")), "{:?}", w.log);
}

const DT: f32 = 0.015;

#[test]
fn set_parent_at_run_time_follows_and_clear_parent_lets_go() {
    // mg_creative_multigames_v8_ns parents a trail and a spinner to a
    // box when it spawns (SetParent); mg_crazykart_v1_1 its particles.
    let mut w = world();
    let m = lift(&mut w);
    let t = spawn(
        &mut w,
        &[("classname", "info_target"), ("targetname", "t"), ("origin", "10 0 0")],
    );
    w.activate();
    w.link_anchored();
    w.queue_input("t", "SetParent", Value::Str("m".into()), 0.0, None);
    w.queue_input("m", "Open", Value::Void, 0.0, None);
    for _ in 0..80 {
        w.frame(&NoCollision);
        w.follow_anchors();
    }
    let lifted = origin_of(&w, m).z;
    assert!(lifted > 50.0, "{lifted}");
    assert!((w.get(t).unwrap().origin - Vec3::new(10.0, 0.0, lifted)).length() < 1e-3);
    w.queue_input("t", "ClearParent", Value::Void, 0.0, None);
    w.frame(&NoCollision);
    let left = w.get(t).unwrap().origin;
    for _ in 0..20 {
        w.frame(&NoCollision);
        w.follow_anchors();
    }
    assert_eq!(w.get(t).unwrap().origin, left, "stays where it was let go");
    // SetParentAttachment snaps to the parent (its origin here).
    w.queue_input("t", "SetParent", Value::Str("m".into()), 0.0, None);
    w.queue_input("t", "SetParentAttachment", Value::Str("primary".into()), 0.0, None);
    w.frame(&NoCollision);
    w.follow_anchors();
    assert!((w.get(t).unwrap().origin - origin_of(&w, m)).length() < 1e-3);
    assert!(!w.log.iter().any(|l| l.contains("unhandled")), "{:?}", w.log);
}

#[test]
fn a_player_set_parent_rides_its_parent() {
    // mg_crazykart_v1_1: a kart's starter trigger parents the player to
    // the kart's seat (!activator SetParent, SetParentAttachment); the
    // player rides along, and dying lets go.
    let mut w = world();
    let m = lift(&mut w);
    let p = player_at(&mut w, 1, Vec3::new(0.0, 0.0, 0.0));
    w.activate();
    w.link_anchored();
    w.queue_input(
        "!activator",
        "SetParent",
        Value::Str("m".into()),
        0.0,
        Some(Who::Player(p)),
    );
    w.queue_input(
        "!activator",
        "SetParentAttachment",
        Value::Str("primary".into()),
        0.1,
        Some(Who::Player(p)),
    );
    w.queue_input("m", "Open", Value::Void, 0.2, None);
    for _ in 0..60 {
        w.frame(&NoCollision);
        w.follow_anchors();
    }
    assert_eq!(w.player_parent(p), Some(Who::Ent(m)));
    let pl = w.player(p).unwrap();
    assert!(
        (pl.origin - origin_of(&w, m)).length() < 1e-3,
        "{} vs {}",
        pl.origin,
        origin_of(&w, m)
    );
    assert!(pl.moved);
    assert!(pl.velocity.z > 0.0, "carried up with the lift");
    w.queue_input("!activator", "ClearParent", Value::Void, 0.0, Some(Who::Player(p)));
    w.frame(&NoCollision);
    assert_eq!(w.player_parent(p), None);
    assert!(!w.log.iter().any(|l| l.contains("unhandled")), "{:?}", w.log);
}

#[test]
fn render_inputs_on_players_and_brushes() {
    // Invisibility power-ups (mg_swag_multigames_v1, surf_halloween_tf2,
    // mg_lt_galaxy_v5): AddOutput rendermode/renderamt/rendercolor and the
    // Alpha and Color inputs on !activator; brushes fade by Alpha.
    use crate::map::tint::RenderLook;
    let mut w = world();
    let p = player_at(&mut w, 1, Vec3::ZERO);
    let wall = spawn(
        &mut w,
        &[
            ("classname", "func_brush"),
            ("targetname", "w"),
            ("rendermode", "1"),
            ("renderamt", "120"),
        ],
    );
    w.activate();
    let me = Some(Who::Player(p));
    w.queue_input("!activator", "AddOutput", Value::Str("rendermode 1".into()), 0.0, me);
    w.queue_input("!activator", "AddOutput", Value::Str("renderamt 30".into()), 0.0, me);
    w.queue_input("!activator", "Color", Value::Str("255 0 0".into()), 0.0, me);
    w.queue_input("!activator", "AddOutput", Value::Str("renderfx 0".into()), 0.0, me);
    w.queue_input("w", "Alpha", Value::Str("60".into()), 0.0, None);
    run_to(&mut w, 2);
    let look = w.player_looks.iter().find(|(e, _)| *e == p).unwrap().1;
    assert_eq!(
        look,
        RenderLook {
            color: [255, 0, 0],
            alpha: 30,
            mode: 1
        }
    );
    assert!(look.blend().is_some());
    let b = w.get(wall).unwrap().render_look();
    assert_eq!((b.mode, b.alpha), (1, 60));
    w.queue_input("!activator", "Alpha", Value::Str("255".into()), 0.0, me);
    w.queue_input("!activator", "AddOutput", Value::Str("rendermode 0".into()), 0.0, me);
    run_to(&mut w, 4);
    assert!(w.player_looks[0].1.blend().is_none(), "visible again");
    assert!(
        !w.log
            .iter()
            .any(|l| l.contains("unhandled") || l.contains("not supported")),
        "{:?}",
        w.log
    );
}

#[test]
fn trails_smoke_stacks_and_particle_systems() {
    // env_spritetrail (always drawn), env_smokestack (TurnOn/TurnOff,
    // its shape inputs kept), info_particle_system (Start/Stop), and a
    // trail parented to a lift reports where it is for drawing.
    use crate::logic::visuals::PartKind;
    let mut w = world();
    lift(&mut w);
    let trail = spawn(
        &mut w,
        &[
            ("classname", "env_spritetrail"),
            ("parentname", "m"),
            ("origin", "5 0 0"),
        ],
    );
    let stack = spawn(
        &mut w,
        &[
            ("classname", "env_smokestack"),
            ("targetname", "s"),
            ("InitialState", "0"),
        ],
    );
    let fx = spawn(
        &mut w,
        &[
            ("classname", "info_particle_system"),
            ("targetname", "fx"),
            ("start_active", "1"),
        ],
    );
    for (i, id) in [trail, stack, fx].into_iter().enumerate() {
        w.get_mut(id).unwrap().map_index = Some(i + 1);
    }
    w.activate();
    w.link_anchored();
    let state = |w: &LogicWorld, i: usize| w.part_states().into_iter().find(|s| s.0 == i).map(|s| (s.1, s.2));
    assert_eq!(state(&w, 1), Some((PartKind::Trail, true)));
    assert_eq!(state(&w, 2), Some((PartKind::SmokeStack, false)));
    assert_eq!(state(&w, 3), Some((PartKind::Particles, true)));
    w.queue_input("s", "TurnOn", Value::Void, 0.0, None);
    w.queue_input("s", "Rate", Value::Str("40".into()), 0.0, None);
    w.queue_input("fx", "Stop", Value::Void, 0.0, None);
    w.queue_input("m", "Open", Value::Void, 0.0, None);
    for _ in 0..30 {
        w.frame(&NoCollision);
        w.follow_anchors();
    }
    assert_eq!(state(&w, 2), Some((PartKind::SmokeStack, true)));
    assert_eq!(state(&w, 3), Some((PartKind::Particles, false)));
    let poses = w.part_poses();
    assert_eq!(poses.len(), 1, "{poses:?}");
    assert_eq!(poses[0].0, 1);
    assert!(poses[0].1.z > 1.0, "the trail rides the lift: {}", poses[0].1);
    assert!(!w.log.iter().any(|l| l.contains("unhandled")), "{:?}", w.log);
}

#[test]
fn constraints_and_motors_find_their_bodies() {
    // gg_bk_warehouse_v1's lamps on ball sockets (attach1 only: held to
    // the world), surf_surreal's motors with "Hinge Object".
    use crate::logic::physics::JointKind;
    let mut w = world();
    let lamp = spawn(
        &mut w,
        &[
            ("classname", "prop_physics"),
            ("targetname", "lamp"),
            ("origin", "0 0 100"),
        ],
    );
    spawn(
        &mut w,
        &[
            ("classname", "phys_ballsocket"),
            ("targetname", "s"),
            ("attach1", "lamp"),
            ("origin", "0 0 132"),
        ],
    );
    spawn(
        &mut w,
        &[
            ("classname", "phys_motor"),
            ("targetname", "mo"),
            ("attach1", "lamp"),
            ("origin", "0 0 100"),
            ("axis", "0 0 200"),
            ("speed", "50"),
            ("spawnflags", "7"),
        ],
    );
    spawn(&mut w, &[("classname", "phys_constraint"), ("attach1", "nothing")]);
    w.activate();
    let joints = w.joints();
    assert_eq!(joints.len(), 2, "{joints:?}");
    assert_eq!(joints[0].kind, JointKind::Ball);
    assert_eq!((joints[0].body1, joints[0].body2), (None, lamp));
    assert_eq!(joints[0].anchor, Vec3::new(0.0, 0.0, 132.0));
    assert_eq!(joints[1].kind, JointKind::Hinge { axis: Vec3::Z });
    let motors: Vec<_> = w.controls();
    assert_eq!(motors.len(), 1);
    w.queue_input("s", "TurnOff", Value::Void, 0.0, None);
    w.queue_input("mo", "TurnOff", Value::Void, 0.0, None);
    run_to(&mut w, 2);
    assert!(!w.joints()[0].on);
    assert!(w.controls().is_empty());
    w.queue_input("s", "Break", Value::Void, 0.0, None);
    run_to(&mut w, 4);
    assert_eq!(w.joints().len(), 1, "broken: gone");
}

#[test]
fn set_parent_attachment_snaps_to_the_models_attachment() {
    // The loader gives a parent prop its model's attachments
    // (`$attachment <name>` keys, model space): SetParentAttachment puts
    // the child (a player on a kart seat) there, turned with the parent.
    let mut w = world();
    spawn(
        &mut w,
        &[
            ("classname", "info_target"),
            ("targetname", "seat"),
            ("origin", "100 0 0"),
            ("angles", "0 90 0"),
            ("$attachment primary", "10 0 30"),
        ],
    );
    let p = player_at(&mut w, 1, Vec3::ZERO);
    w.activate();
    let me = Some(Who::Player(p));
    w.queue_input("!activator", "SetParent", Value::Str("seat".into()), 0.0, me);
    w.queue_input("!activator", "SetParentAttachment", Value::Str("primary".into()), 0.0, me);
    w.frame(&NoCollision);
    w.follow_anchors();
    let at = w.player(p).unwrap().origin;
    // (10, 0, 30) turned 90 degrees: (0, 10, 30) from (100, 0, 0).
    assert!((at - Vec3::new(100.0, 10.0, 30.0)).length() < 1e-3, "{at}");
}

#[test]
fn shooters_pushes_toggles_overlays_and_cameras() {
    // env_shooter (mg_kommando's shell casings), point_push (mg_lt_galaxy's
    // black hole), env_texturetoggle (mg_crazykart's item boxes),
    // env_screenoverlay (mg_swag's team screens), point_camera with a
    // func_monitor (mg_lt_galaxy's spectator screens), func_water_analog
    // and func_tanktrain.
    let mut w = world();
    spawn(
        &mut w,
        &[
            ("classname", "env_shooter"),
            ("targetname", "shells"),
            ("shootmodel", "models/shells/shell_762nato.mdl"),
            ("m_iGibs", "3"),
            ("m_flVelocity", "100"),
            ("m_flGibLife", "2"),
        ],
    );
    let toggled = spawn(&mut w, &[("classname", "func_door"), ("targetname", "items")]);
    spawn(
        &mut w,
        &[
            ("classname", "env_texturetoggle"),
            ("targetname", "tt"),
            ("target", "items"),
        ],
    );
    spawn(
        &mut w,
        &[
            ("classname", "env_screenoverlay"),
            ("targetname", "ov"),
            ("OverlayName1", "a/one"),
            ("OverlayTime1", "1"),
            ("OverlayName2", "a/two"),
            ("OverlayTime2", "-1"),
        ],
    );
    spawn(
        &mut w,
        &[
            ("classname", "point_camera"),
            ("targetname", "cam"),
            ("origin", "1 2 3"),
            ("FOV", "70"),
            ("spawnflags", "1"),
        ],
    );
    spawn(
        &mut w,
        &[("classname", "func_monitor"), ("targetname", "mon"), ("target", "cam")],
    );
    spawn(&mut w, &[("classname", "func_water_analog"), ("targetname", "water")]);
    spawn(&mut w, &[("classname", "func_tanktrain"), ("targetname", "tank")]);
    w.activate();
    assert_eq!(w.monitor_camera(), None, "the camera starts off");
    w.queue_input("shells", "Shoot", Value::Void, 0.0, None);
    w.queue_input("tt", "SetTextureIndex", Value::Str("2".into()), 0.0, None);
    w.queue_input("ov", "StartOverlays", Value::Void, 0.0, None);
    w.queue_input("cam", "SetOn", Value::Void, 0.0, None);
    w.queue_input("water", "Open", Value::Void, 0.0, None);
    w.queue_input("tank", "Toggle", Value::Void, 0.0, None);
    run_to(&mut w, 2);
    let gibs: Vec<_> = w
        .effects
        .iter()
        .filter_map(|e| match e {
            Effect::Gibs { set, pieces, .. } => Some((set.clone(), pieces.len())),
            _ => None,
        })
        .collect();
    assert_eq!(gibs, vec![("models/shells/shell_762nato.mdl".to_string(), 3)]);
    assert_eq!(w.get(toggled).unwrap().texture_frame, 2);
    let overlays = |w: &LogicWorld| -> Vec<String> {
        w.effects
            .iter()
            .filter_map(|e| match e {
                Effect::Hud {
                    what: crate::logic::hud::HudShow::Overlay { material, .. },
                    ..
                } => Some(material.clone()),
                _ => None,
            })
            .collect()
    };
    assert_eq!(overlays(&w), vec!["a/one".to_string()]);
    assert_eq!(w.monitor_camera(), Some((Vec3::new(1.0, 2.0, 3.0), Vec3::ZERO, 70.0)));
    w.effects.clear();
    // After its second, the next overlay; it stays.
    run_to(&mut w, 80);
    assert_eq!(overlays(&w), vec!["a/two".to_string()]);
    w.queue_input("tt", "IncrementTextureIndex", Value::Void, 0.0, None);
    w.queue_input("mon", "Disable", Value::Void, 0.0, None);
    run_to(&mut w, 82);
    assert_eq!(w.get(toggled).unwrap().texture_frame, 3);
    assert_eq!(w.monitor_camera(), None, "no monitor shows it");
    assert!(!w.log.iter().any(|l| l.contains("unhandled")), "{:?}", w.log);
}

#[test]
fn point_push_and_tesla_think_while_on() {
    let mut w = world();
    let p = player_at(&mut w, 1, Vec3::new(100.0, 0.0, 0.0));
    spawn(
        &mut w,
        &[
            ("classname", "point_push"),
            ("targetname", "push"),
            ("magnitude", "200"),
            ("radius", "512"),
            ("spawnflags", "8"),
            ("enabled", "0"),
        ],
    );
    let tesla = spawn(
        &mut w,
        &[
            ("classname", "point_tesla"),
            ("targetname", "tesla"),
            ("interval_min", "0.1"),
            ("interval_max", "0.1"),
            ("m_SoundName", "DoSpark"),
        ],
    );
    w.get_mut(tesla).unwrap().map_index = Some(7);
    w.activate();
    run_to(&mut w, 10);
    assert_eq!(w.player(p).unwrap().velocity, Vec3::ZERO, "off at spawn");
    w.queue_input("push", "Enable", Value::Void, 0.0, None);
    w.queue_input("tesla", "TurnOn", Value::Void, 0.0, None);
    w.effects.clear();
    run_to(&mut w, 80);
    // Pushed away from it (+X), less than the full 200 u/s² at 100 of 512.
    let v = w.player(p).unwrap().velocity;
    assert!(v.x > 50.0 && v.y.abs() < 1e-3, "{v}");
    let sparks = w
        .effects
        .iter()
        .filter(|e| matches!(e, Effect::Tesla { entity: 7 }))
        .count();
    assert!((8..=12).contains(&sparks), "{sparks}");
    assert!((crate::logic::community::push_strength(200.0, 512.0, 256.0, true) - 100.0).abs() < 1e-3);
}

#[test]
fn intended_gaps_are_noted_once_per_map() {
    // VScript inputs (CS:GO), server plugin commands, triggers naming a
    // missing filter, inputs to names nothing has: noted once each (not
    // complaints), across round restarts too.
    let mut w = world();
    let p = player_at(&mut w, 1, Vec3::ZERO);
    spawn(
        &mut w,
        &[("classname", "point_servercommand"), ("targetname", "server")],
    );
    spawn_brush(
        &mut w,
        &[
            ("classname", "trigger_multiple"),
            ("filtername", "filter_red"),
            ("spawnflags", "1"),
        ],
        Vec3::splat(-8.0),
        Vec3::splat(8.0),
    );
    w.activate();
    let me = Some(Who::Player(p));
    for _ in 0..3 {
        w.queue_input("!activator", "RunScriptCode", Value::Str("x = 1".into()), 0.0, me);
        w.queue_input("server", "Command", Value::Str("sm_say hello".into()), 0.0, None);
        w.queue_input("nobody", "Trigger", Value::Void, 0.0, None);
    }
    run_to(&mut w, 3);
    let notes: Vec<&String> = w.log.iter().filter(|l| l.starts_with("note: ")).collect();
    assert_eq!(notes.len(), 4, "{notes:?}");
    assert!(notes.iter().any(|l| l.contains("player.runscriptcode")));
    assert!(notes.iter().any(|l| l.contains("sm_say hello")));
    assert!(notes.iter().any(|l| l.contains("filter_red")));
    assert!(notes.iter().any(|l| l.contains("'nobody'")));
    assert!(w.log.iter().all(|l| l.starts_with("note: ")), "{:?}", w.log);
    assert!(crate::logic::classes::is_plugin_command("ma_csay hi"));
    assert!(!crate::logic::classes::is_plugin_command("sv_gravity 800"));
}

/// Round 3's classes: env_viewpunch kicks players within its radius on
/// the ground (surf_surreal's crash), env_muzzleflash's Fire flashes
/// (mg_kommando's gun), color_correction and env_embers switch, and
/// classes CS:S doesn't have are noted once and do nothing.
#[test]
fn view_punch_muzzle_flash_color_correction_and_noted_classes() {
    let mut w = world();
    spawn(
        &mut w,
        &[
            ("classname", "env_viewpunch"),
            ("targetname", "vp"),
            ("origin", "0 0 0"),
            ("radius", "100"),
            ("punchangle", "10 5 90"),
        ],
    );
    let mf = spawn(&mut w, &[("classname", "env_muzzleflash"), ("targetname", "mf")]);
    w.get_mut(mf).unwrap().map_index = Some(3);
    let cc = spawn(
        &mut w,
        &[("classname", "color_correction"), ("targetname", "cc"), ("StartDisabled", "1")],
    );
    let embers = spawn(&mut w, &[("classname", "env_embers"), ("targetname", "em"), ("spawnflags", "1")]);
    spawn(&mut w, &[("classname", "logic_script"), ("vscripts", "x.nut")]);
    spawn(&mut w, &[("classname", "ai_changetarget"), ("targetname", "c1")]);
    let near = player_at(&mut w, 0, Vec3::new(50.0, 0.0, 0.0));
    player_at(&mut w, 1, Vec3::new(500.0, 0.0, 0.0));
    for p in &mut w.players {
        p.on_ground = true;
    }
    w.activate();
    let on = |w: &LogicWorld, id: EntId| matches!(&w.get(id).unwrap().class, Class::Part(p) if p.on);
    assert!(!on(&w, cc), "StartDisabled");
    assert!(on(&w, embers), "Start On");
    w.queue_input("vp", "ViewPunch", Value::Void, 0.0, None);
    w.queue_input("mf", "Fire", Value::Void, 0.0, None);
    w.queue_input("cc", "Enable", Value::Void, 0.0, None);
    w.queue_input("em", "TurnOff", Value::Void, 0.0, None);
    run_to(&mut w, 2);
    let punches: Vec<(Entity, Vec3)> = w
        .effects
        .iter()
        .filter_map(|e| match e {
            Effect::ViewPunch { player, angles } => Some((*player, *angles)),
            _ => None,
        })
        .collect();
    assert_eq!(punches, vec![(near, Vec3::new(10.0, 5.0, 90.0))], "only the player within the radius");
    assert!(w.effects.iter().any(|e| matches!(e, Effect::MuzzleFlash { entity: 3 })));
    assert!(on(&w, cc) && !on(&w, embers));
    let notes = w.log.iter().filter(|l| l.starts_with("note: entity classes CS:S doesn't have")).count();
    assert_eq!(notes, 1, "noted once: {:?}", w.log);
    assert!(!w.log.iter().any(|l| l.contains("unhandled")), "{:?}", w.log);
}
