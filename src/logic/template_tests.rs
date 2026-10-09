//! Test cases from specs/source/viewcontrol_and_templates.md
//! (point_template, env_entity_maker, point_viewcontrol) and
//! physics_brushes.md (phys_thruster, phys_keepupright on the logic
//! side), and entities parented to a placed weapon following it.

use super::*;
use crate::logic::templates::{apply_suffix, prepare_fixup, Member};

fn map_entity(pairs: &[(&str, &str)]) -> crate::map::MapEntity {
    crate::map::MapEntity {
        keyvalues: kv(pairs),
        ..Default::default()
    }
}

fn named_all(w: &LogicWorld, prefix: &str) -> Vec<(String, Vec3, Vec3)> {
    let mut out: Vec<(String, Vec3, Vec3)> = w
        .ids()
        .into_iter()
        .filter_map(|id| w.get(id))
        .filter(|e| e.targetname.to_ascii_lowercase().starts_with(prefix))
        .map(|e| (e.targetname.clone(), e.origin, e.angles))
        .collect();
    out.sort_by(|a, b| a.0.cmp(&b.0));
    out
}

// ------------------------------------------------------------ templates

#[test]
fn template_removes_members_and_spawns_copies_in_place() {
    let entities = vec![
        map_entity(&[("classname", "point_template"), ("targetname", "t"), ("Template01", "box"), ("OnEntitySpawned", "done,Trigger,,0,-1")]),
        map_entity(&[("classname", "prop_physics"), ("targetname", "box"), ("origin", "64 0 0")]),
        map_entity(&[("classname", "logic_relay"), ("targetname", "done")]),
    ];
    let mut w = world();
    w.load_map(&entities);
    assert!(w.find("box").is_none(), "the original is out of the map");
    w.queue_input("t", "ForceSpawn", Value::Void, 0.0, None);
    run_to(&mut w, 0);
    let boxes = named_all(&w, "box");
    assert_eq!(boxes.len(), 1);
    assert!(boxes[0].1.distance(Vec3::new(64.0, 0.0, 0.0)) < 1e-3);
    assert_eq!(got(&w, w.find("done").unwrap(), "Trigger"), vec![0]);
    assert!(w.effects.iter().any(|e| matches!(e, Effect::Spawned { source: 1, .. })));
    // The template moved and turned: the copy keeps its place around it.
    let t = w.find("t").unwrap();
    w.get_mut(t).unwrap().origin = Vec3::new(100.0, 0.0, 0.0);
    w.get_mut(t).unwrap().angles = Vec3::new(0.0, 90.0, 0.0);
    w.queue_input("t", "ForceSpawn", Value::Void, 0.0, None);
    run_to(&mut w, 1);
    let boxes = named_all(&w, "box");
    assert_eq!(boxes.len(), 2);
    assert!(boxes.iter().any(|b| b.1.distance(Vec3::new(100.0, 64.0, 0.0)) < 1e-3 && (b.2.y - 90.0).abs() < 1e-3));
}

#[test]
fn template_keeps_members_with_flag_1() {
    let entities = vec![
        map_entity(&[("classname", "point_template"), ("targetname", "t"), ("Template01", "box"), ("spawnflags", "1")]),
        map_entity(&[("classname", "prop_physics"), ("targetname", "box"), ("origin", "64 0 0")]),
    ];
    let mut w = world();
    w.load_map(&entities);
    assert!(w.find("box").is_some());
    w.queue_input("t", "ForceSpawn", Value::Void, 0.0, None);
    run_to(&mut w, 0);
    assert_eq!(named_all(&w, "box").len(), 2);
}

/// A button and the door it opens, in a template (with or without name
/// fix-up), spawned twice.
fn button_door(flags: &str) -> LogicWorld {
    let entities = vec![
        map_entity(&[
            ("classname", "point_template"),
            ("targetname", "t"),
            ("Template01", "btn"),
            ("Template02", "door"),
            ("spawnflags", flags),
        ]),
        map_entity(&[("classname", "logic_relay"), ("targetname", "btn"), ("OnTrigger", "door,Trigger,,0,-1")]),
        map_entity(&[("classname", "logic_relay"), ("targetname", "door")]),
    ];
    let mut w = world();
    w.load_map(&entities);
    w.queue_input("t", "ForceSpawn", Value::Void, 0.0, None);
    w.queue_input("t", "ForceSpawn", Value::Void, 0.0, None);
    run_to(&mut w, 0);
    w
}

#[test]
fn name_fixup_pairs_each_copy() {
    let mut w = button_door("0");
    let names: Vec<String> = named_all(&w, "").into_iter().map(|n| n.0).collect();
    let buttons: Vec<&String> = names.iter().filter(|n| n.starts_with("btn")).collect();
    let doors: Vec<&String> = names.iter().filter(|n| n.starts_with("door")).collect();
    assert_eq!(buttons.len(), 2);
    assert_eq!(doors.len(), 2);
    assert_ne!(doors[0], doors[1], "each copy has its own name");
    assert!(doors.iter().all(|d| d.len() == "door&0000".len() && d.contains('&')));
    // The button isn't referenced inside the group: its copies share
    // "btn" (2.1 step 4; the spec's test table suffixes it too: open
    // question). Each copy aims at its own door.
    assert!(buttons.iter().all(|b| *b == "btn"));
    let mine = w.find(doors[0]).unwrap();
    let other = w.find(doors[1]).unwrap();
    let my_button = w
        .ids()
        .into_iter()
        .find(|id| {
            w.get(*id).is_some_and(|e| {
                e.targetname == "btn"
                    && e.outputs.iter().any(|(_, c)| c.iter().any(|c| c.target.eq_ignore_ascii_case(doors[0])))
            })
        })
        .expect("the button of the first door");
    w.deliver(Who::Ent(my_button), "Trigger", Value::Void, None, None);
    run_to(&mut w, 1);
    assert_eq!(got(&w, mine, "Trigger"), vec![1]);
    assert!(got(&w, other, "Trigger").is_empty());
    // Outside logic aiming at "door" finds none; "door*" finds both.
    w.queue_input("door", "Trigger", Value::Void, 0.0, None);
    w.queue_input("door*", "Trigger", Value::Void, 0.0, None);
    run_to(&mut w, 2);
    assert_eq!(got(&w, mine, "Trigger"), vec![1, 2]);
    assert_eq!(got(&w, other, "Trigger"), vec![2]);
}

#[test]
fn no_fixup_shares_names() {
    let mut w = button_door("2");
    let names: Vec<String> = named_all(&w, "").into_iter().map(|n| n.0).collect();
    assert_eq!(names.iter().filter(|n| *n == "door").count(), 2);
    w.queue_input("btn", "Trigger", Value::Void, 0.0, None);
    run_to(&mut w, 1);
    // Either button opens both doors (each button reaches both).
    for id in w.ids() {
        if w.get(id).unwrap().targetname == "door" {
            assert_eq!(got(&w, id, "Trigger"), vec![1, 1]);
        }
    }
}

#[test]
fn fixup_marks_only_whole_names_in_the_first_field() {
    let mut members = vec![
        Member {
            keyvalues: kv(&[
                ("targetname", "lamp"),
                ("parentname", "cart"),
                ("OnUser1", "cart*,Kill,,0,-1"),
                ("OnUser2", "!activator,SetParent,cart,0,-1"),
            ]),
            hulls: Vec::new(),
            source: 0,
            offset: Vec3::ZERO,
            rotation: Quat::IDENTITY,
            fixup: false,
        },
        Member {
            keyvalues: kv(&[("targetname", "cart")]),
            hulls: Vec::new(),
            source: 1,
            offset: Vec3::ZERO,
            rotation: Quat::IDENTITY,
            fixup: false,
        },
    ];
    prepare_fixup(&mut members);
    let get = |m: &Member, k: &str| m.keyvalues.iter().find(|(x, _)| x == k).unwrap().1.clone();
    assert_eq!(get(&members[0], "parentname"), "cart&0000");
    assert_eq!(get(&members[0], "OnUser1"), "cart*,Kill,,0,-1", "wildcards left alone");
    assert_eq!(get(&members[0], "OnUser2"), "!activator,SetParent,cart,0,-1", "parameters left alone");
    assert_eq!(get(&members[1], "targetname"), "cart&0000");
    assert_eq!(get(&members[0], "targetname"), "lamp", "not referenced: keeps its name");
    assert_eq!(apply_suffix("cart&0000,Kill", "&0012"), "cart&0012,Kill");
}

// ---------------------------------------------------------------- maker

fn maker_world(maker_flags: &str) -> LogicWorld {
    let entities = vec![
        map_entity(&[("classname", "point_template"), ("targetname", "t"), ("Template01", "box")]),
        map_entity(&[("classname", "info_target"), ("targetname", "box"), ("origin", "64 0 0")]),
        map_entity(&[
            ("classname", "env_entity_maker"),
            ("targetname", "m"),
            ("EntityTemplate", "t"),
            ("origin", "0 0 100"),
            ("angles", "0 180 0"),
            ("spawnflags", maker_flags),
            ("OnEntitySpawned", "made,Trigger,,0,-1"),
            ("OnEntityFailedSpawn", "failed,Trigger,,0,-1"),
        ]),
        map_entity(&[("classname", "logic_relay"), ("targetname", "made")]),
        map_entity(&[("classname", "logic_relay"), ("targetname", "failed")]),
    ];
    let mut w = world();
    w.load_map(&entities);
    w
}

#[test]
fn maker_spawns_at_itself() {
    let mut w = maker_world("0");
    w.queue_input("m", "ForceSpawn", Value::Void, 0.0, None);
    run_to(&mut w, 0);
    let boxes = named_all(&w, "box");
    assert_eq!(boxes.len(), 1);
    assert!(boxes[0].1.distance(Vec3::new(-64.0, 0.0, 100.0)) < 1e-3, "{:?}", boxes[0].1);
    assert_eq!(got(&w, w.find("made").unwrap(), "Trigger"), vec![0]);
}

#[test]
fn maker_needs_room() {
    let mut w = maker_world("8");
    w.queue_input("m", "ForceSpawn", Value::Void, 0.0, None);
    run_to(&mut w, 0);
    // The copy still sits where it was made: no room.
    w.queue_input("m", "ForceSpawn", Value::Void, 0.0, None);
    run_to(&mut w, 1);
    assert_eq!(named_all(&w, "box").len(), 1);
    assert_eq!(got(&w, w.find("failed").unwrap(), "Trigger"), vec![1]);
}

#[test]
fn maker_waits_until_nobody_looks() {
    let mut w = maker_world("16");
    // A player facing the maker (through walls counts).
    player_at(&mut w, 1, Vec3::new(-300.0, 0.0, 36.0));
    w.queue_input("m", "ForceSpawn", Value::Void, 0.0, None);
    run_to(&mut w, 0);
    assert_eq!(got(&w, w.find("failed").unwrap(), "Trigger"), vec![0]);
    w.players[0].view = Vec3::new(0.0, 180.0, 0.0);
    w.queue_input("m", "ForceSpawn", Value::Void, 0.0, None);
    run_to(&mut w, 1);
    assert_eq!(named_all(&w, "box").len(), 1);
}

#[test]
fn maker_autospawns_after_the_copy_is_gone() {
    let mut w = maker_world("7");
    run_to(&mut w, 0);
    assert_eq!(named_all(&w, "box").len(), 1, "one at once");
    run_to(&mut w, 100);
    assert_eq!(named_all(&w, "box").len(), 1, "waits while it exists");
    w.queue_input("box", "Kill", Value::Void, 0.0, None);
    run_to(&mut w, 101);
    run_to(&mut w, 140);
    assert_eq!(named_all(&w, "box").len(), 1, "a new one within 0.5 s");
}

// --------------------------------------------------------------- camera

fn camera_world(pairs: &[(&str, &str)]) -> (LogicWorld, EntId, Entity) {
    let mut w = world();
    let p = player_at(&mut w, 1, Vec3::ZERO);
    let mut all = vec![
        ("classname", "point_viewcontrol"),
        ("targetname", "cam"),
        ("OnEndFollow", "ended,Trigger,,0,-1"),
    ];
    all.extend_from_slice(pairs);
    let cam = spawn(&mut w, &all);
    spawn(&mut w, &[("classname", "info_target"), ("targetname", "look"), ("origin", "0 100 0")]);
    relay(&mut w, "ended", &[]);
    w.activate();
    (w, cam, p)
}

#[test]
fn camera_needs_a_player() {
    let (mut w, _, _) = camera_world(&[("target", "look"), ("wait", "3")]);
    w.queue_input("cam", "Enable", Value::Void, 0.0, None);
    run_to(&mut w, 0);
    assert!(w.camera_views().is_empty());
    assert!(!w.effects.iter().any(|e| matches!(e, Effect::ViewControl { .. })));
}

#[test]
fn camera_holds_then_gives_the_view_back() {
    let (mut w, cam, p) = camera_world(&[("target", "look"), ("wait", "3")]);
    w.queue_input("cam", "Enable", Value::Void, 0.0, Some(Who::Player(p)));
    run_to(&mut w, 0);
    assert_eq!(w.camera_views().len(), 1);
    assert!(w.effects.contains(&Effect::ViewControl {
        target: p,
        camera: Some(cam),
        freeze: false
    }));
    run_to(&mut w, 199);
    assert_eq!(w.camera_views().len(), 1, "3 s");
    run_to(&mut w, 202);
    assert!(w.camera_views().is_empty());
    assert!(w.effects.contains(&Effect::ViewControl {
        target: p,
        camera: None,
        freeze: false
    }));
    assert_eq!(got(&w, w.find("ended").unwrap(), "Trigger").len(), 1);
    // Disable on a camera that is off still fires OnEndFollow.
    w.queue_input("cam", "Disable", Value::Void, 0.0, None);
    run_to(&mut w, 203);
    assert_eq!(got(&w, w.find("ended").unwrap(), "Trigger").len(), 2);
}

#[test]
fn camera_turns_toward_its_target() {
    // Yaw 0 to a target 90° to the left: 0.9 % of the error per tick.
    let (mut w, cam, p) = camera_world(&[("target", "look"), ("wait", "100")]);
    w.queue_input("cam", "Enable", Value::Void, 0.0, Some(Who::Player(p)));
    run_to(&mut w, 1);
    let yaw = w.get(cam).unwrap().angles.y;
    assert!((yaw - 0.81).abs() < 0.01, "{yaw}");
    run_to(&mut w, 167);
    let left = 90.0 - w.get(cam).unwrap().angles.y;
    assert!((left - 90.0 * 0.991f32.powi(167)).abs() < 1.0, "{left}");
    // Snapping: at once.
    let (mut w, cam, p) = camera_world(&[("target", "look"), ("wait", "100"), ("spawnflags", "16")]);
    w.queue_input("cam", "Enable", Value::Void, 0.0, Some(Who::Player(p)));
    run_to(&mut w, 1);
    assert!((w.get(cam).unwrap().angles.y - 90.0).abs() < 1e-3);
}

#[test]
fn camera_without_target_never_times_out() {
    let (mut w, _, p) = camera_world(&[("wait", "3")]);
    w.queue_input("cam", "Enable", Value::Void, 0.0, Some(Who::Player(p)));
    run_to(&mut w, 700);
    assert_eq!(w.camera_views().len(), 1);
}

#[test]
fn interruptable_camera_stops_on_a_key() {
    let (mut w, _, p) = camera_world(&[("target", "look"), ("wait", "100"), ("spawnflags", "64")]);
    w.queue_input("cam", "Enable", Value::Void, 0.0, Some(Who::Player(p)));
    run_to(&mut w, 5);
    w.players[0].buttons = crate::core::buttons::JUMP;
    run_to(&mut w, 7);
    assert!(w.camera_views().is_empty());
}

#[test]
fn a_second_camera_ends_the_first() {
    let (mut w, cam, p) = camera_world(&[("target", "look"), ("wait", "100")]);
    let other = spawn(&mut w, &[("classname", "point_viewcontrol"), ("targetname", "cam2"), ("target", "look")]);
    w.activate();
    w.queue_input("cam", "Enable", Value::Void, 0.0, Some(Who::Player(p)));
    run_to(&mut w, 1);
    w.queue_input("cam2", "Enable", Value::Void, 0.0, Some(Who::Player(p)));
    run_to(&mut w, 2);
    assert_eq!(w.views, vec![(p, other)]);
    assert_eq!(got(&w, w.find("ended").unwrap(), "Trigger").len(), 1);
    let _ = cam;
}

// ---------------------------------------------------- thrusters, upright

#[test]
fn thruster_turns_on_scales_and_times_out() {
    let mut w = world();
    spawn(&mut w, &[("classname", "prop_physics"), ("targetname", "box"), ("origin", "0 0 0")]);
    spawn(
        &mut w,
        &[
            ("classname", "phys_thruster"),
            ("targetname", "th"),
            ("attach1", "box"),
            ("force", "1000"),
            ("forcetime", "2"),
            ("spawnflags", "2"),
        ],
    );
    w.activate();
    assert!(w.controls().is_empty());
    w.queue_input("th", "scale", Value::Str("2".into()), 0.0, None);
    run_to(&mut w, 0);
    let c = w.controls();
    assert_eq!(c.len(), 1);
    let crate::logic::physics::ControlKind::Thrust { force, scale, serial, .. } = &c[0].kind else {
        panic!()
    };
    assert_eq!((*force, *scale, *serial), (Vec3::new(1000.0, 0.0, 0.0), 2.0, 1));
    // Activate while on: ignored (no new turn-on).
    w.queue_input("th", "Activate", Value::Void, 0.0, None);
    run_to(&mut w, 1);
    assert!(matches!(w.controls()[0].kind, crate::logic::physics::ControlKind::Thrust { serial: 1, .. }));
    // forcetime 2 s: off by tick 133.
    run_to(&mut w, 134);
    assert!(w.controls().is_empty());
}

#[test]
fn keepupright_without_a_body_removes_itself() {
    let mut w = world();
    let u = spawn(&mut w, &[("classname", "phys_keepupright"), ("attach1", "nothing")]);
    let box_ = spawn(&mut w, &[("classname", "prop_physics"), ("targetname", "b")]);
    let v = spawn(&mut w, &[("classname", "phys_keepupright"), ("targetname", "up"), ("attach1", "b")]);
    w.activate();
    run_to(&mut w, 0);
    assert!(w.get(u).is_none());
    assert_eq!(w.controls().len(), 1);
    assert_eq!(w.controls()[0].body, box_);
    w.queue_input("up", "TurnOff", Value::Void, 0.0, None);
    run_to(&mut w, 1);
    assert!(w.controls().is_empty());
    let _ = v;
}

// --------------------------------------------- children of a placed weapon

#[test]
fn children_of_a_weapon_follow_it() {
    let entities = vec![
        map_entity(&[("classname", "weapon_knife"), ("targetname", "item"), ("origin", "100 0 0")]),
        map_entity(&[("classname", "info_target"), ("targetname", "car"), ("parentname", "item"), ("origin", "100 32 0")]),
        map_entity(&[("classname", "info_target"), ("targetname", "wheel"), ("parentname", "car"), ("origin", "100 64 0")]),
    ];
    let mut w = world();
    w.load_map(&entities);
    let item = w.find("item").unwrap();
    // Picked up: the weapon is at its owner, turned to its yaw.
    w.set_anchor(item, Vec3::new(0.0, 0.0, 10.0), Vec3::new(0.0, 90.0, 0.0));
    w.follow_anchors();
    let car = w.get(w.find("car").unwrap()).unwrap().origin;
    let wheel = w.get(w.find("wheel").unwrap()).unwrap().origin;
    assert!(car.distance(Vec3::new(-32.0, 0.0, 10.0)) < 1e-3, "{car}");
    assert!(wheel.distance(Vec3::new(-64.0, 0.0, 10.0)) < 1e-3, "{wheel}");
}
