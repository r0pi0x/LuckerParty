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
        &kv(&[
            ("classname", "prop_door_rotating"),
            ("targetname", "door"),
            ("returndelay", "-1"),
        ]),
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
    spawn(
        &mut w,
        &[
            ("classname", "info_target"),
            ("targetname", "spot"),
            ("origin", "-50 0 0"),
        ],
    );
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
    let d = assault_door(
        &mut w,
        &[("spawnflags", "10240"), ("soundlockedoverride", "Doors.Locked")],
    );
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
    for extra in [
        [("spawnflags", "8193"), ("spawnpos", "0")],
        [("spawnflags", "8192"), ("spawnpos", "1")],
    ] {
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
    let wall = BrushCollision(vec![MapBrush::from_box(
        Vec3::new(47.0, -200.0, -10.0),
        Vec3::new(100.0, 200.0, 200.0),
    )]);
    w.queue_input("door", "Open", Value::Void, 0.0, None);
    run_to_with(&mut w, 40, &wall);
    assert_eq!(fired(&w, d, "OnBlockedOpening").len(), 1);
    let held = angles_of(&w, d).y;
    assert!(held > -90.0 && held < 0.0, "{held}");
    run_to_with(&mut w, 50, &wall);
    assert_eq!(angles_of(&w, d).y, held, "holds still");
    assert!(
        w.effects.iter().all(|e| !matches!(e, Effect::Damage { .. })),
        "no damage"
    );
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
// specs/source/prop_damage.md, its test cases.

use crate::logic::prop_damage::{Hit, Motion, PropExplosion};
use crate::map::entities::{
    PROP_BREAK_SOUND_KEY, PROP_CLIENT_KEY, PROP_DAMAGE_KEY, PROP_EXPLODE_KEY, PROP_EXPLODE_SOUND_KEY, PROP_HEALTH_KEY,
    PROP_INTERACTIONS_KEY, PROP_PIECES_KEY,
};

fn extinguisher(w: &mut LogicWorld, health: &str) -> EntId {
    spawn(
        w,
        &[
            ("classname", "prop_physics_multiplayer"),
            ("targetname", "ext"),
            ("spawnflags", "264"),
            (PROP_HEALTH_KEY, health),
            ("OnHealthChanged", "steam,TurnOn,,0,1"),
            ("OnHealthChanged", "steamsound,PlaySound,,0,1"),
        ],
    )
}

/// `wood_crate001a`: Wooden.Medium (health 30, 0.75/2/1.5), 7 pieces.
fn wood_crate(w: &mut LogicWorld, extra: &[(&str, &str)]) -> EntId {
    let mut pairs = vec![
        ("classname", "prop_physics_multiplayer"),
        ("targetname", "crate"),
        (PROP_HEALTH_KEY, "30"),
        (PROP_DAMAGE_KEY, "0.75 2 1.5"),
        (PROP_PIECES_KEY, "7"),
        (PROP_BREAK_SOUND_KEY, "Wood_Crate.Break"),
    ];
    pairs.extend_from_slice(extra);
    spawn(w, &pairs)
}

fn hit(amount: f32, kind: DamageKind, attacker: Option<Who>) -> Hit {
    Hit {
        amount,
        kind,
        attacker,
        point: Vec3::ZERO,
        dir: Vec3::X,
        force: 0.0,
        direct: false,
    }
}

/// The outputs `id` fired, in order.
fn outputs(w: &LogicWorld, id: EntId) -> Vec<String> {
    w.fired.iter().filter(|f| f.1 == id).map(|f| f.2.clone()).collect()
}

fn breaks(w: &LogicWorld) -> Vec<(EntId, Option<String>, Option<PropExplosion>)> {
    w.effects
        .iter()
        .filter_map(|e| match e {
            Effect::PropBreak { id, sound, explode } => Some((*id, sound.clone(), explode.clone())),
            _ => None,
        })
        .collect()
}

fn last_value(w: &LogicWorld, output_target: EntId, input: &str) -> Option<Value> {
    w.deliveries
        .iter()
        .rev()
        .find(|d| d.target == Who::Ent(output_target) && d.input.eq_ignore_ascii_case(input))
        .map(|d| d.value.clone())
}

#[test]
fn events_only_prop_fires_on_health_changed_with_zero() {
    let mut w = world();
    let p = extinguisher(&mut w, "0");
    let steam = spawn(&mut w, &[("classname", "info_target"), ("targetname", "steam")]);
    let sound = spawn(&mut w, &[("classname", "info_target"), ("targetname", "steamsound")]);
    w.activate();
    run_to(&mut w, 1);
    assert!(w.damage(p, 36.0, DamageKind::Bullet, None, Vec3::ZERO, Vec3::X));
    assert_eq!(outputs(&w, p), vec!["OnHealthChanged", "OnTakeDamage"]);
    run_to(&mut w, 2);
    assert_eq!(got(&w, steam, "TurnOn"), vec![2]);
    assert_eq!(got(&w, sound, "PlaySound"), vec![2]);
    assert_eq!(
        last_value(&w, steam, "TurnOn"),
        Some(Value::Float(0.0)),
        "events only: 0"
    );
    // Fire count 1: the next hit fires the output but reaches nobody.
    w.damage(p, 36.0, DamageKind::Bullet, None, Vec3::ZERO, Vec3::X);
    run_to(&mut w, 3);
    assert_eq!(got(&w, sound, "PlaySound").len(), 1);
    assert!(w.get(p).is_some(), "no health: never breaks");
}

#[test]
fn wood_crate_takes_scaled_integer_damage_and_breaks() {
    let mut w = world();
    let p = wood_crate(&mut w, &[("OnHealthChanged", "log,Show,,0,-1")]);
    let log = spawn(&mut w, &[("classname", "info_target"), ("targetname", "log")]);
    let shooter = Who::Player(player_at(&mut w, 0, Vec3::ZERO));
    w.activate();
    run_to(&mut w, 1);
    assert_eq!(w.prop_health(p), Some((30, 30)));
    w.prop_hit(p, hit(36.0, DamageKind::Bullet, Some(shooter)));
    // 36 x 0.75 = 27.
    assert_eq!(w.prop_health(p), Some((3, 30)));
    run_to(&mut w, 2);
    match last_value(&w, log, "Show") {
        Some(Value::Float(v)) => assert!((v - 0.1).abs() < 1e-6, "{v}"),
        v => panic!("{v:?}"),
    }
    w.fired.clear();
    w.prop_hit(p, hit(36.0, DamageKind::Bullet, Some(shooter)));
    // OnBreak first, then the damage outputs.
    assert_eq!(outputs(&w, p), vec!["OnBreak", "OnHealthChanged", "OnTakeDamage"]);
    assert_eq!(breaks(&w), vec![(p, Some("Wood_Crate.Break".to_string()), None)]);
    assert!(w.get(p).unwrap().targetname.is_empty(), "name cleared at once");
    // A second hit the same tick: ignored.
    w.prop_hit(p, hit(36.0, DamageKind::Bullet, Some(shooter)));
    assert_eq!(outputs(&w, p).len(), 3);
    run_to(&mut w, 3);
    assert!(w.get(p).is_none(), "removed");
    // A club hit of 16: 32 >= 30.
    let mut w = world();
    let p = wood_crate(&mut w, &[]);
    w.activate();
    run_to(&mut w, 1);
    w.prop_hit(p, hit(16.0, DamageKind::Melee, None));
    assert_eq!(breaks(&w).len(), 1);
}

#[test]
fn health_needs_something_to_break_into() {
    // chair_office: Cloth.Large (health 100), no pieces: events only.
    let mut w = world();
    let p = spawn(
        &mut w,
        &[
            ("classname", "prop_physics_multiplayer"),
            (PROP_HEALTH_KEY, "100"),
            (PROP_PIECES_KEY, "0"),
        ],
    );
    w.activate();
    run_to(&mut w, 1);
    assert_eq!(w.prop_health(p), Some((0, 1)));
    w.prop_hit(p, hit(500.0, DamageKind::Bullet, None));
    assert!(breaks(&w).is_empty());
    // A flammable or exploding one breaks without pieces.
    let mut w = world();
    let p = spawn(
        &mut w,
        &[
            ("classname", "prop_physics_multiplayer"),
            (PROP_HEALTH_KEY, "20"),
            (PROP_INTERACTIONS_KEY, "explode_fire flammable"),
        ],
    );
    w.activate();
    assert_eq!(w.prop_health(p), Some((20, 20)));
}

#[test]
fn integer_truncation_breaks_a_file_box() {
    // Cardboard.break: health 10, bullets 0.5; 19 -> 9.5 -> 0.5 -> 0.
    let mut w = world();
    let p = spawn(
        &mut w,
        &[
            ("classname", "prop_physics_multiplayer"),
            (PROP_HEALTH_KEY, "10"),
            (PROP_DAMAGE_KEY, "0.5 1.25 1.5"),
            (PROP_PIECES_KEY, "3"),
        ],
    );
    w.activate();
    run_to(&mut w, 1);
    w.prop_hit(p, hit(19.0, DamageKind::Bullet, None));
    assert_eq!(breaks(&w).len(), 1);
}

#[test]
fn minhealthdmg_ignores_small_hits_entirely() {
    // de_train's toilet: Pottery.Medium 40, minhealthdmg 50.
    let mut w = world();
    let p = spawn(
        &mut w,
        &[
            ("classname", "prop_physics_multiplayer"),
            ("minhealthdmg", "50"),
            (PROP_HEALTH_KEY, "40"),
            (PROP_DAMAGE_KEY, "1 1.25 1.5"),
            (PROP_PIECES_KEY, "4"),
        ],
    );
    w.activate();
    run_to(&mut w, 1);
    w.prop_hit(p, hit(36.0, DamageKind::Bullet, None));
    assert!(outputs(&w, p).is_empty(), "no outputs at all");
    assert_eq!(w.prop_health(p), Some((40, 40)));
    w.prop_hit(p, hit(60.0, DamageKind::Bullet, None));
    assert_eq!(breaks(&w).len(), 1);
}

#[test]
fn map_health_counts_only_on_override_classes() {
    let mut w = world();
    let plain = spawn(
        &mut w,
        &[
            ("classname", "prop_physics"),
            ("health", "5"),
            (PROP_HEALTH_KEY, "50"),
            (PROP_PIECES_KEY, "2"),
        ],
    );
    let over = spawn(
        &mut w,
        &[
            ("classname", "prop_dynamic_override"),
            ("targetname", "box"),
            ("health", "5"),
            (PROP_HEALTH_KEY, "50"),
            (PROP_PIECES_KEY, "2"),
        ],
    );
    w.activate();
    assert_eq!(w.prop_health(plain), Some((50, 50)));
    assert_eq!(w.prop_health(over), Some((5, 5)));
    // The Break input breaks it (and an events-only prop too).
    let ev = spawn(&mut w, &[("classname", "prop_physics"), ("targetname", "ext")]);
    w.queue_input("box", "Break", Value::Void, 0.0, None);
    w.queue_input("ext", "Break", Value::Void, 0.0, None);
    w.frame(&NoCollision);
    assert_eq!(fired(&w, over, "OnBreak").len(), 1);
    assert_eq!(fired(&w, ev, "OnBreak").len(), 1);
    assert!(w.get(over).is_none() && w.get(ev).is_none());
}

#[test]
fn hits_on_the_spawn_tick_are_events_only() {
    let mut w = world();
    let p = wood_crate(&mut w, &[]);
    w.activate();
    w.prop_hit(p, hit(100.0, DamageKind::Bullet, None));
    assert_eq!(w.prop_health(p), Some((30, 30)));
    assert_eq!(outputs(&w, p), vec!["OnHealthChanged", "OnTakeDamage"]);
}

#[test]
fn gas_can_explodes_as_the_last_attacker() {
    let mut w = world();
    let p = spawn(
        &mut w,
        &[
            ("classname", "prop_physics_multiplayer"),
            (PROP_HEALTH_KEY, "20"),
            (PROP_INTERACTIONS_KEY, "explode_fire flammable"),
            (PROP_EXPLODE_KEY, "25 80"),
            (PROP_EXPLODE_SOUND_KEY, "PropaneTank.Burst"),
            (PROP_BREAK_SOUND_KEY, "Metal_Barrel.Break"),
        ],
    );
    let shooter = Who::Player(player_at(&mut w, 0, Vec3::ZERO));
    w.activate();
    run_to(&mut w, 1);
    w.prop_hit(p, hit(19.0, DamageKind::Bullet, Some(shooter)));
    assert_eq!(w.prop_health(p), Some((1, 20)));
    // Whoever finishes it, the blast counts for the last player.
    w.prop_hit(p, hit(5.0, DamageKind::Blast, None));
    let b = breaks(&w);
    assert_eq!(b.len(), 1);
    assert_eq!(
        b[0].2,
        Some(PropExplosion {
            damage: 25.0,
            radius: 80.0,
            attacker: Some(shooter),
            sound: Some("PropaneTank.Burst".into()),
        })
    );
}

#[test]
fn client_props_take_flat_damage_and_have_no_outputs() {
    // glassjug01: Glass.Small 5, mode 3.
    let mut w = world();
    let p = spawn(
        &mut w,
        &[
            ("classname", "prop_physics_multiplayer"),
            ("targetname", "jug"),
            (PROP_HEALTH_KEY, "5"),
            (PROP_CLIENT_KEY, "1"),
            (PROP_BREAK_SOUND_KEY, "Glass.Break"),
            ("OnBreak", "x,Show,,0,-1"),
        ],
    );
    w.activate();
    assert!(w.get(p).unwrap().targetname.is_empty(), "no name");
    run_to(&mut w, 1);
    w.prop_hit(p, hit(1.0, DamageKind::Bullet, None));
    assert_eq!(breaks(&w), vec![(p, None, None)], "no sound");
    assert!(outputs(&w, p).is_empty());
    // A blast 300 units out (14.286 raw): |(14.286 along, 32 up)| = 35.
    let mut w = world();
    let p = spawn(
        &mut w,
        &[
            ("classname", "prop_physics_multiplayer"),
            (PROP_HEALTH_KEY, "40"),
            (PROP_CLIENT_KEY, "1"),
        ],
    );
    w.activate();
    run_to(&mut w, 1);
    w.prop_hit(p, hit(100.0 - 300.0 * 100.0 / 350.0, DamageKind::Blast, None));
    assert_eq!(w.prop_health(p), Some((5, 40)));
}

#[test]
fn force_to_enable_motion() {
    let mut w = world();
    let p = spawn(
        &mut w,
        &[
            ("classname", "prop_physics_multiplayer"),
            ("targetname", "ext"),
            ("forcetoenablemotion", "2250"),
            ("OnMotionEnabled", "steam,TurnOn,,0,-1"),
        ],
    );
    let steam = spawn(&mut w, &[("classname", "info_target"), ("targetname", "steam")]);
    w.activate();
    run_to(&mut w, 1);
    let push = |w: &mut LogicWorld, force: f32| {
        w.prop_hit(
            p,
            Hit {
                force,
                ..hit(10.0, DamageKind::Bullet, None)
            },
        );
    };
    push(&mut w, 2000.0);
    assert_eq!(outputs(&w, p), vec!["OnHealthChanged", "OnTakeDamage"], "stays frozen");
    push(&mut w, 2400.0);
    assert_eq!(fired(&w, p, "OnMotionEnabled").len(), 1);
    assert!(w.effects.contains(&Effect::PropMotion {
        id: p,
        motion: Motion::Enable
    }));
    run_to(&mut w, 2);
    assert_eq!(got(&w, steam, "TurnOn").len(), 1);
    // Once: the threshold is cleared.
    push(&mut w, 5000.0);
    assert_eq!(fired(&w, p, "OnMotionEnabled").len(), 1);
}

#[test]
fn asleep_keyboard_breaks_when_woken() {
    // cs_office's keyboard1: start asleep (sf 257), OnAwakened -> own Break.
    let mut w = world();
    let p = spawn(
        &mut w,
        &[
            ("classname", "prop_physics_multiplayer"),
            ("targetname", "keyboard1"),
            ("spawnflags", "257"),
            (PROP_HEALTH_KEY, "10"),
            (PROP_PIECES_KEY, "6"),
            ("OnAwakened", "keyboard1,Break,,0,-1"),
        ],
    );
    w.activate();
    run_to(&mut w, 1);
    assert!(w.prop_states().is_empty() || w.prop(p).unwrap().asleep);
    w.prop_awakened(p);
    w.prop_awakened(p);
    assert_eq!(fired(&w, p, "OnAwakened").len(), 1, "once");
    run_to(&mut w, 2);
    assert_eq!(fired(&w, p, "OnBreak").len(), 1);
    let activator = w
        .deliveries
        .iter()
        .find(|d| d.input.eq_ignore_ascii_case("Break"))
        .and_then(|d| d.activator);
    assert_eq!(activator, Some(Who::Ent(p)), "the keyboard itself");
}

#[test]
fn pressure_breaks_after_its_delay() {
    // cs_militia's roof boards: sf 296, PressureDelay 1.
    let mut w = world();
    let p = spawn(
        &mut w,
        &[
            ("classname", "prop_physics"),
            ("spawnflags", "296"),
            ("PressureDelay", "1"),
            (PROP_HEALTH_KEY, "30"),
            (PROP_PIECES_KEY, "4"),
        ],
    );
    w.activate();
    let pl = player_at(&mut w, 0, Vec3::ZERO);
    run_to(&mut w, 10);
    assert!(fired(&w, p, "OnBreak").is_empty());
    w.players[0].ground = Some(p);
    run_to(&mut w, 11);
    // 1 s = 67 ticks (ceil(1/0.015 - 1e-4)) later.
    run_to(&mut w, 76);
    assert!(fired(&w, p, "OnBreak").is_empty());
    run_to(&mut w, 79);
    assert_eq!(fired(&w, p, "OnBreak").len(), 1);
    let _ = pl;
}

#[test]
fn breaking_removes_what_is_parented_to_it() {
    let mut w = world();
    let p = wood_crate(&mut w, &[("targetname", "mon")]);
    let spark = spawn(
        &mut w,
        &[
            ("classname", "env_sprite"),
            ("targetname", "spark"),
            ("parentname", "mon"),
        ],
    );
    let deeper = spawn(&mut w, &[("classname", "info_target"), ("parentname", "spark,attach")]);
    let other = spawn(&mut w, &[("classname", "info_target"), ("parentname", "elsewhere")]);
    w.activate();
    w.queue_input("mon", "Break", Value::Void, 0.0, None);
    w.frame(&NoCollision);
    assert!(w.get(p).is_none() && w.get(spark).is_none() && w.get(deeper).is_none());
    assert!(w.get(other).is_some());
}

#[test]
fn prop_visibility_and_collision_inputs() {
    let mut w = world();
    let p = spawn(&mut w, &[("classname", "prop_dynamic"), ("targetname", "screen")]);
    w.activate();
    let shown = |w: &LogicWorld| w.prop_states().iter().find(|s| s.id == p).map(|s| (s.visible, s.solid));
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
