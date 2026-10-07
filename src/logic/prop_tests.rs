//! Model doors (specs/source/doors_buttons.md, "prop_door_rotating" and
//! its test cases) and prop damage and inputs, on the logic world.

use super::*;
use crate::core::DamageKind;
use crate::logic::props::PropDoor;

/// A door leaf 52 wide on the hinge's left (+Y), 2 thick, 100 high.
fn leaf() -> MapHull {
    hull(Vec3::new(-1.0, 0.0, 0.0), Vec3::new(1.0, 52.0, 100.0))
}

/// cs_assault's door_lower at the origin: returndelay 20, distance 90,
/// speed 100, "Use closes".
fn assault_door(w: &mut LogicWorld, extra: &[(&str, &str)]) -> EntId {
    let mut pairs = vec![
        ("classname", "prop_door_rotating"),
        ("targetname", "door"),
        ("origin", "0 0 0"),
        ("angles", "0 0 0"),
        ("spawnflags", "8192"),
        ("returndelay", "20"),
        ("speed", "100"),
        ("distance", "90"),
        ("hardware", "1"),
        (crate::map::entities::DOOR_MOVE_KEY, "Doors.Move1"),
        (crate::map::entities::DOOR_OPEN_KEY, "Doors.FullOpen1"),
        (crate::map::entities::DOOR_CLOSE_KEY, "Doors.FullClose1"),
    ];
    pairs.extend_from_slice(extra);
    w.spawn(&kv(&pairs), vec![leaf()])
}

fn state(w: &LogicWorld, id: EntId) -> DoorState {
    w.prop_door(id).unwrap().state
}

fn sounds(w: &mut LogicWorld) -> Vec<String> {
    w.effects
        .drain(..)
        .filter_map(|e| match e {
            Effect::Sound { entry, .. } => Some(entry),
            _ => None,
        })
        .collect()
}

#[test]
fn cs_assault_door_opens_away_and_closes_after_its_delay() {
    let mut w = world();
    let d = assault_door(&mut w, &[]);
    w.activate();
    // In front of the door (+X), looking at it.
    player_at(&mut w, 0, Vec3::new(40.0, 26.0, 0.0));
    w.players[0].view = Vec3::new(0.0, 180.0, 0.0);
    w.players[0].use_key = true;
    run_to(&mut w, 0);
    w.players[0].use_key = false;
    assert_eq!(fired(&w, d, "OnOpen"), vec![0], "the use opens it");
    assert_eq!(sounds(&mut w), vec!["Doors.Move1"]);
    // Away from the user: back, +90, over 0.9 s.
    run_to(&mut w, 59);
    let a = angles_of(&w, d).y;
    assert!(a > 85.0 && a < 90.0, "{a}");
    run_to(&mut w, 60);
    assert_eq!(angles_of(&w, d).y, 90.0);
    assert_eq!(fired(&w, d, "OnFullyOpen"), vec![60]);
    assert_eq!(sounds(&mut w), vec!["Doors.FullOpen1"]);
    // Closes 20.1 s (1340 ticks) after fully open.
    run_to(&mut w, 60 + 1339);
    assert!(fired(&w, d, "OnClose").is_empty());
    run_to(&mut w, 60 + 1340);
    assert_eq!(fired(&w, d, "OnClose"), vec![1400]);
    run_to(&mut w, 1460);
    assert_eq!(fired(&w, d, "OnFullyClosed"), vec![1460]);
    assert_eq!(angles_of(&w, d).y, 0.0);
    assert_eq!(state(&w, d), DoorState::Closed);
}

#[test]
fn second_use_closes_an_open_door() {
    let mut w = world();
    let d = assault_door(&mut w, &[]);
    w.activate();
    let p = player_at(&mut w, 0, Vec3::new(40.0, 26.0, 0.0));
    w.queue_input("door", "Use", Value::Void, 0.0, Some(Who::Player(p)));
    run_to(&mut w, 99);
    assert_eq!(state(&w, d), DoorState::Open);
    w.queue_input("door", "Use", Value::Void, 0.0, Some(Who::Player(p)));
    run_to(&mut w, 100);
    assert_eq!(fired(&w, d, "OnClose"), vec![100]);
    run_to(&mut w, 160);
    assert_eq!(angles_of(&w, d).y, 0.0);
    assert_eq!(fired(&w, d, "OnFullyClosed"), vec![160]);

    // Without "Use closes", a use on the open door does nothing.
    let mut w = world();
    let d = assault_door(&mut w, &[("spawnflags", "0")]);
    w.activate();
    let p = player_at(&mut w, 0, Vec3::new(40.0, 26.0, 0.0));
    w.queue_input("door", "Use", Value::Void, 0.0, Some(Who::Player(p)));
    run_to(&mut w, 99);
    w.queue_input("door", "Use", Value::Void, 0.0, Some(Who::Player(p)));
    run_to(&mut w, 120);
    assert!(fired(&w, d, "OnClose").is_empty());
    assert_eq!(state(&w, d), DoorState::Open);
}

#[test]
fn opens_forward_for_a_user_behind_it_and_obeys_opendir() {
    let mut w = world();
    let d = assault_door(&mut w, &[]);
    w.activate();
    let p = player_at(&mut w, 0, Vec3::new(-40.0, 26.0, 0.0));
    w.queue_input("door", "Use", Value::Void, 0.0, Some(Who::Player(p)));
    run_to(&mut w, 70);
    assert_eq!(angles_of(&w, d).y, -90.0);

    // opendir 2: back only, whoever opens it.
    let mut w = world();
    let d = assault_door(&mut w, &[("opendir", "2")]);
    w.activate();
    // Behind it but out of the swing.
    let p = player_at(&mut w, 0, Vec3::new(-200.0, 26.0, 0.0));
    w.queue_input("door", "Use", Value::Void, 0.0, Some(Who::Player(p)));
    run_to(&mut w, 70);
    assert_eq!(angles_of(&w, d).y, 90.0);

    // A leaf on the hinge's right turns the other way for the same side.
    let mut w = world();
    let d = w.spawn(
        &kv(&[("classname", "prop_door_rotating"), ("targetname", "door"), ("returndelay", "-1")]),
        vec![hull(Vec3::new(-1.0, -52.0, 0.0), Vec3::new(1.0, 0.0, 100.0))],
    );
    w.activate();
    let p = player_at(&mut w, 0, Vec3::new(40.0, -26.0, 0.0));
    w.queue_input("door", "Use", Value::Void, 0.0, Some(Who::Player(p)));
    run_to(&mut w, 70);
    assert_eq!(angles_of(&w, d).y, -90.0, "swings away from the user");
}

#[test]
fn open_input_fires_on_open_twice() {
    let mut w = world();
    let d = assault_door(&mut w, &[("OnOpen", "c,Add,1,0,-1"), ("OnClose", "c,Add,10,0,-1")]);
    let c = spawn(&mut w, &[("classname", "math_counter"), ("targetname", "c")]);
    w.activate();
    w.queue_input("door", "Open", Value::Void, 0.0, None);
    run_to(&mut w, 0);
    assert_eq!(fired(&w, d, "OnOpen"), vec![0, 0]);
    assert_eq!(counter(&w, c), 2.0, "OnOpen delivered twice");
    // Opened by input with no user: forward.
    run_to(&mut w, 60);
    assert_eq!(angles_of(&w, d).y, -90.0);
    w.queue_input("door", "Close", Value::Void, 0.0, None);
    run_to(&mut w, 61);
    assert_eq!(counter(&w, c), 22.0, "OnClose delivered twice");
    // Open on an open door: nothing.
    run_to(&mut w, 200);
    w.queue_input("door", "Open", Value::Void, 0.0, None);
    run_to(&mut w, 201);
    w.queue_input("door", "Open", Value::Void, 0.0, None);
    run_to(&mut w, 202);
    assert_eq!(fired(&w, d, "OnOpen").len(), 4);
}

#[test]
fn open_away_from_and_toggle() {
    let mut w = world();
    let d = assault_door(&mut w, &[("returndelay", "-1")]);
    spawn(&mut w, &[("classname", "info_target"), ("targetname", "spot"), ("origin", "-50 0 0")]);
    w.activate();
    w.queue_input("door", "OpenAwayFrom", Value::Str("spot".into()), 0.0, None);
    run_to(&mut w, 70);
    assert_eq!(angles_of(&w, d).y, -90.0, "away from a spot behind it");
    w.queue_input("door", "Toggle", Value::Void, 0.0, None);
    run_to(&mut w, 140);
    assert_eq!(state(&w, d), DoorState::Closed);
    w.queue_input("door", "Toggle", Value::Void, 0.0, None);
    run_to(&mut w, 210);
    assert_eq!(state(&w, d), DoorState::Open);
}

#[test]
fn locked_door_fires_on_locked_use() {
    let mut w = world();
    let d = assault_door(&mut w, &[("spawnflags", "10240"), ("soundlockedoverride", "Doors.Locked")]);
    w.activate();
    let p = player_at(&mut w, 0, Vec3::new(40.0, 26.0, 0.0));
    w.queue_input("door", "Use", Value::Void, 0.0, Some(Who::Player(p)));
    w.queue_input("door", "Open", Value::Void, 0.0, None);
    run_to(&mut w, 10);
    assert_eq!(fired(&w, d, "OnLockedUse"), vec![0]);
    assert!(fired(&w, d, "OnOpen").is_empty());
    assert!(sounds(&mut w).contains(&"Doors.Locked".to_string()));
    w.queue_input("door", "Unlock", Value::Void, 0.0, None);
    run_to(&mut w, 11);
    w.queue_input("door", "Use", Value::Void, 0.0, Some(Who::Player(p)));
    run_to(&mut w, 12);
    assert_eq!(fired(&w, d, "OnOpen"), vec![12]);
}

#[test]
fn ignore_use_and_spawn_positions() {
    // "Ignore +use": a player's use does nothing; inputs still work.
    let mut w = world();
    let d = assault_door(&mut w, &[("spawnflags", "32768")]);
    w.activate();
    player_at(&mut w, 0, Vec3::new(40.0, 26.0, 0.0));
    w.players[0].view = Vec3::new(0.0, 180.0, 0.0);
    w.players[0].use_key = true;
    run_to(&mut w, 5);
    assert!(fired(&w, d, "OnOpen").is_empty());

    // Starts open (de_port: flag 1; spawnpos 1).
    for extra in [[("spawnflags", "8193"), ("spawnpos", "0")], [("spawnflags", "8192"), ("spawnpos", "1")]] {
        let mut w = world();
        let d = assault_door(&mut w, &extra);
        w.activate();
        assert_eq!(state(&w, d), DoorState::Open);
        assert_eq!(angles_of(&w, d).y, -90.0);
    }
}

#[test]
fn slaves_move_with_the_master() {
    let mut w = world();
    let a = assault_door(&mut w, &[("targetname", "a"), ("slavename", "b")]);
    let b = assault_door(&mut w, &[("targetname", "b"), ("origin", "0 200 0")]);
    w.activate();
    w.queue_input("a", "Open", Value::Void, 0.0, None);
    run_to(&mut w, 60);
    assert_eq!(angles_of(&w, a).y.abs(), 90.0);
    assert_eq!(angles_of(&w, b).y.abs(), 90.0);
    assert_eq!(fired(&w, b, "OnFullyOpen"), vec![60]);
    w.queue_input("a", "Close", Value::Void, 0.0, None);
    run_to(&mut w, 130);
    assert_eq!(angles_of(&w, b).y, 0.0);
}

#[test]
fn blocked_door_stops_and_resumes() {
    let mut w = world();
    let d = assault_door(&mut w, &[("returndelay", "-1")]);
    w.activate();
    // Opening forward (-90) sweeps x > 0: a player there, pinned against
    // a wall.
    let p = player_at(&mut w, 0, Vec3::new(30.0, 30.0, 0.0));
    let wall = BrushCollision(vec![MapBrush::from_box(Vec3::new(47.0, -200.0, -10.0), Vec3::new(100.0, 200.0, 200.0))]);
    w.queue_input("door", "Open", Value::Void, 0.0, None);
    run_to_with(&mut w, 40, &wall);
    assert_eq!(fired(&w, d, "OnBlockedOpening").len(), 1);
    let held = angles_of(&w, d).y;
    assert!(held > -90.0 && held < 0.0, "{held}");
    run_to_with(&mut w, 50, &wall);
    assert_eq!(angles_of(&w, d).y, held, "holds still");
    assert!(w.effects.iter().all(|e| !matches!(e, Effect::Damage { .. })), "no damage");
    // The player steps away: it carries on.
    w.player_mut(p).unwrap().origin = Vec3::new(300.0, 300.0, 0.0);
    run_to_with(&mut w, 120, &wall);
    assert_eq!(fired(&w, d, "OnUnblockedOpening").len(), 1);
    assert_eq!(angles_of(&w, d).y, -90.0);
    assert!(matches!(&w.get(d).unwrap().class, Class::PropDoor(x) if x.state == DoorState::Open));
}

#[test]
fn auto_close_waits_for_a_clear_swing() {
    let mut w = world();
    let d = assault_door(&mut w, &[("returndelay", "1")]);
    w.activate();
    w.queue_input("door", "Open", Value::Void, 0.0, None);
    run_to(&mut w, 60);
    assert_eq!(angles_of(&w, d).y, -90.0);
    // Someone stands where it would close through.
    let p = player_at(&mut w, 0, Vec3::new(30.0, 30.0, 0.0));
    run_to(&mut w, 60 + 73 + 5);
    assert!(fired(&w, d, "OnClose").is_empty(), "waits");
    w.player_mut(p).unwrap().origin = Vec3::new(300.0, 300.0, 0.0);
    // Retries after the return delay (1 s).
    run_to(&mut w, 60 + 73 + 67 + 1);
    assert_eq!(fired(&w, d, "OnClose"), vec![60 + 73 + 67]);
}

#[test]
fn model_door_never_turns_through_a_player() {
    let mut w = world();
    let d = assault_door(&mut w, &[("returndelay", "-1")]);
    w.activate();
    let PropDoor { push, .. } = w.prop_door(d).unwrap();
    assert!(push.solid);
    // Forward (-90) sweeps x > 0: a player standing in the swing, out in
    // the open. The leaf's tip outruns the box's corner, so it stops at
    // them (HL2's doors bump into you).
    let p = player_at(&mut w, 0, Vec3::new(20.0, 40.0, 0.0));
    w.queue_input("door", "Open", Value::Void, 0.0, None);
    run_to(&mut w, 70);
    let a = angles_of(&w, d).y;
    assert!(a > -90.0 && a < 0.0, "{a}");
    assert_eq!(fired(&w, d, "OnBlockedOpening").len(), 1);
    let pl = w.player(p).unwrap().clone();
    let rot = crate::map::entities::entity_rotation(angles_of(&w, d));
    let leaf = place_hull(&leaf(), rot, Vec3::ZERO);
    let (half, centre) = ((pl.maxs - pl.mins) / 2.0, pl.origin + (pl.mins + pl.maxs) / 2.0);
    assert!(!leaf.overlaps_box(centre, half, SOLID_SKIN), "not inside the player");
}

// ---------------------------------------------------------------- props

fn extinguisher(w: &mut LogicWorld, health: &str) -> EntId {
    spawn(
        w,
        &[
            ("classname", "prop_physics_multiplayer"),
            ("targetname", "ext"),
            ("spawnflags", "264"),
            (crate::map::entities::PROP_HEALTH_KEY, health),
            ("OnHealthChanged", "steam,TurnOn,,0,1"),
            ("OnHealthChanged", "steamsound,PlaySound,,0,1"),
        ],
    )
}

#[test]
fn shot_prop_fires_on_health_changed_once() {
    let mut w = world();
    let p = extinguisher(&mut w, "0");
    let steam = spawn(&mut w, &[("classname", "info_target"), ("targetname", "steam")]);
    let sound = spawn(&mut w, &[("classname", "info_target"), ("targetname", "steamsound")]);
    w.activate();
    run_to(&mut w, 1);
    assert!(w.damage(p, 36.0, DamageKind::Bullet, None, Vec3::ZERO, Vec3::X));
    run_to(&mut w, 2);
    assert_eq!(fired(&w, p, "OnHealthChanged"), vec![2]);
    assert_eq!(fired(&w, p, "OnTakeDamage"), vec![2]);
    assert_eq!(got(&w, steam, "TurnOn"), vec![2]);
    assert_eq!(got(&w, sound, "PlaySound"), vec![2]);
    // Fire count 1: the next hit fires the output but reaches nobody.
    w.damage(p, 36.0, DamageKind::Bullet, None, Vec3::ZERO, Vec3::X);
    run_to(&mut w, 3);
    assert_eq!(got(&w, sound, "PlaySound").len(), 1);
    assert!(w.get(p).is_some(), "no health: never breaks");
}

#[test]
fn prop_health_breaks_and_fires_outputs() {
    let mut w = world();
    let p = spawn(
        &mut w,
        &[
            ("classname", "prop_physics_multiplayer"),
            ("targetname", "crate"),
            (crate::map::entities::PROP_HEALTH_KEY, "50"),
        ],
    );
    w.activate();
    let shot = |w: &mut LogicWorld| {
        w.damage(p, 36.0, DamageKind::Bullet, None, Vec3::ZERO, Vec3::X);
    };
    shot(&mut w);
    assert_eq!(w.prop_health(p), Some((32, 50)));
    shot(&mut w);
    assert_eq!(w.prop_health(p), Some((14, 50)));
    assert!(fired(&w, p, "OnBreak").is_empty());
    shot(&mut w);
    assert_eq!(fired(&w, p, "OnBreak").len(), 1);
    assert_eq!(fired(&w, p, "OnHealthChanged").len(), 3);
    w.frame(&NoCollision);
    assert!(w.get(p).is_none(), "removed");

    // The map's own health keyvalue wins; the Break input breaks it.
    let mut w = world();
    let p = spawn(
        &mut w,
        &[("classname", "prop_dynamic"), ("targetname", "box"), ("health", "5"), (crate::map::entities::PROP_HEALTH_KEY, "50")],
    );
    w.activate();
    assert_eq!(w.prop_health(p), Some((5, 5)));
    w.queue_input("box", "Break", Value::Void, 0.0, None);
    w.frame(&NoCollision);
    assert_eq!(fired(&w, p, "OnBreak").len(), 1);
    assert!(w.get(p).is_none());
}

#[test]
fn prop_visibility_and_collision_inputs() {
    let mut w = world();
    let p = spawn(&mut w, &[("classname", "prop_dynamic"), ("targetname", "screen")]);
    w.activate();
    let shown = |w: &LogicWorld| w.prop_states().iter().find(|s| s.0 == p).map(|s| (s.2, s.3));
    // Not a map entity: no map index, so not listed.
    assert_eq!(shown(&w), None);
    w.get_mut(p).unwrap().map_index = Some(0);
    assert_eq!(shown(&w), Some((true, true)));
    w.queue_input("screen", "Disable", Value::Void, 0.0, None);
    w.queue_input("screen", "DisableCollision", Value::Void, 0.0, None);
    w.frame(&NoCollision);
    assert_eq!(shown(&w), Some((false, false)));
    w.queue_input("screen", "TurnOn", Value::Void, 0.0, None);
    w.frame(&NoCollision);
    assert_eq!(shown(&w), Some((true, false)));
    w.queue_input("screen", "Kill", Value::Void, 0.0, None);
    w.frame(&NoCollision);
    assert!(w.get(p).is_none());
}
