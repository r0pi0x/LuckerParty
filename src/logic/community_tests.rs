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

const DT: f32 = 0.015;
