//! Test cases from specs/source/entity_io.md, triggers.md and
//! doors_buttons.md, run on the logic world directly. dt = 0.015 s.

use bevy::prelude::*;

use super::classes::{Class, ServerLine, allowed_client_command, check_server_command};
use super::hud::HudMessage;
use super::movers::{DoorState, pusher};
use super::world::*;
use super::{Value, Who};
use crate::map::{MapBrush, MapHull};

const DT: f32 = 0.015;

/// Keyvalues; for plain keys a later pair replaces an earlier one (so
/// helpers can take overrides), outputs ("On...") are all kept.
pub(super) fn kv(pairs: &[(&str, &str)]) -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = Vec::new();
    for (k, v) in pairs {
        if !is_output_key(k)
            && let Some(old) = out.iter_mut().find(|(o, _)| o.eq_ignore_ascii_case(k))
        {
            old.1 = v.to_string();
            continue;
        }
        out.push((k.to_string(), v.to_string()));
    }
    out
}

pub(super) fn world() -> LogicWorld {
    let mut w = LogicWorld::new(DT);
    w.record = true;
    w
}

/// An axis-aligned box hull (entity space).
pub(super) fn hull(lo: Vec3, hi: Vec3) -> MapHull {
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

pub(super) fn spawn(w: &mut LogicWorld, pairs: &[(&str, &str)]) -> EntId {
    w.spawn(&kv(pairs), Vec::new())
}

pub(super) fn spawn_brush(w: &mut LogicWorld, pairs: &[(&str, &str)], lo: Vec3, hi: Vec3) -> EntId {
    w.spawn(&kv(pairs), vec![hull(lo, hi)])
}

/// Run frames up to and including tick `last`.
pub(super) fn run_to(w: &mut LogicWorld, last: i64) {
    while w.tick <= last {
        w.frame(&NoCollision);
    }
}

fn run_to_with(w: &mut LogicWorld, last: i64, col: &dyn Collision) {
    while w.tick <= last {
        w.frame(col);
    }
}

/// Ticks at which `id` received `input`.
pub(super) fn got(w: &LogicWorld, id: EntId, input: &str) -> Vec<i64> {
    w.deliveries
        .iter()
        .filter(|d| d.target == Who::Ent(id) && d.input.eq_ignore_ascii_case(input))
        .map(|d| d.tick)
        .collect()
}

/// Ticks at which `id` fired `output`.
pub(super) fn fired(w: &LogicWorld, id: EntId, output: &str) -> Vec<i64> {
    w.fired
        .iter()
        .filter(|(_, e, o)| *e == id && o.eq_ignore_ascii_case(output))
        .map(|(t, ..)| *t)
        .collect()
}

pub(super) fn relay(w: &mut LogicWorld, name: &str, outputs: &[&str]) -> EntId {
    let mut pairs = vec![("classname", "logic_relay"), ("targetname", name)];
    for o in outputs {
        pairs.push(("OnTrigger", o));
    }
    spawn(w, &pairs)
}

pub(super) fn player_at(w: &mut LogicWorld, n: u32, origin: Vec3) -> Entity {
    let e = Entity::from_raw_u32(100 + n).unwrap();
    let mut p = Player::new(e, origin);
    p.maxs = Vec3::new(16.0, 16.0, 62.0);
    w.players.push(p);
    e
}

fn counter(w: &LogicWorld, id: EntId) -> f32 {
    match &w.get(id).unwrap().class {
        Class::Counter(c) => c.value,
        _ => panic!("not a counter"),
    }
}

// ---------------------------------------------------------------- I/O

#[test]
fn connections_fire_in_reverse_order() {
    let mut w = world();
    let r = relay(&mut w, "r", &["a,Trigger,,0,-1", "b,Trigger,,0,-1", "c,Trigger,,0,-1"]);
    let a = relay(&mut w, "a", &[]);
    let b = relay(&mut w, "b", &[]);
    let c = relay(&mut w, "c", &[]);
    w.activate();
    run_to(&mut w, 99);
    w.queue_input("r", "Trigger", Value::Void, 0.0, None);
    run_to(&mut w, 100);
    let order: Vec<Who> = w
        .deliveries
        .iter()
        .filter(|d| d.target != Who::Ent(r))
        .map(|d| d.target)
        .collect();
    assert_eq!(order, vec![Who::Ent(c), Who::Ent(b), Who::Ent(a)]);
    assert!(w.deliveries.iter().all(|d| d.tick == 100));
}

#[test]
fn delays_round_up_to_ticks() {
    for (delay, expect) in [("0.5", 134), ("0.02", 102)] {
        let mut w = world();
        relay(&mut w, "r", &[&format!("b,Trigger,,{delay},-1")]);
        let b = relay(&mut w, "b", &[]);
        w.activate();
        run_to(&mut w, 99);
        w.queue_input("r", "Trigger", Value::Void, 0.0, None);
        run_to(&mut w, 200);
        assert_eq!(got(&w, b, "Trigger"), vec![expect], "delay {delay}");
    }
}

#[test]
fn zero_delay_chains_finish_in_one_tick() {
    let mut w = world();
    let a = relay(&mut w, "a", &["b,Trigger,,0,-1"]);
    let b = relay(&mut w, "b", &["c,Trigger,,0,-1"]);
    let c = relay(&mut w, "c", &[]);
    w.activate();
    run_to(&mut w, 49);
    w.queue_input("a", "Trigger", Value::Void, 0.0, None);
    run_to(&mut w, 50);
    for id in [a, b, c] {
        assert_eq!(fired(&w, id, "OnTrigger"), vec![50]);
    }
}

#[test]
fn times_to_fire() {
    for (times, expect) in [("0", 5), ("2", 2)] {
        let mut w = world();
        let r = spawn(&mut w, &[("classname", "logic_branch"), ("targetname", "r"), ("OnTrue", &format!("t,Use,,0,{times}"))]);
        let t = spawn(&mut w, &[("classname", "info_target"), ("targetname", "t")]);
        w.activate();
        for _ in 0..5 {
            w.deliver(Who::Ent(r), "SetValueTest", Value::Str("1".into()), None, None);
            w.frame(&NoCollision);
        }
        assert_eq!(got(&w, t, "Use").len(), expect, "times {times}");
    }
}

#[test]
fn math_counter_typing_and_start_value() {
    let mut w = world();
    let r = relay(&mut w, "r", &["ctr,Add,,0,-1"]);
    let c = spawn(&mut w, &[("classname", "math_counter"), ("targetname", "ctr")]);
    let c2 = spawn(&mut w, &[("classname", "math_counter"), ("startvalue", "2.5")]);
    w.activate();
    w.deliver(Who::Ent(r), "Trigger", Value::Void, None, None);
    run_to(&mut w, 0);
    assert_eq!(counter(&w, c), 0.0);
    assert!(fired(&w, c, "OutValue").is_empty(), "no value into Add is rejected");
    w.deliver(Who::Ent(c), "Add", Value::Str("2.7".into()), None, None);
    assert!((counter(&w, c) - 2.7).abs() < 1e-6);
    assert_eq!(counter(&w, c2), 2.0);
}

#[test]
fn branch_true_string_is_false() {
    let mut w = world();
    let b = spawn(&mut w, &[("classname", "logic_branch")]);
    w.deliver(Who::Ent(b), "SetValueTest", Value::Str("true".into()), None, None);
    assert_eq!(fired(&w, b, "OnFalse").len(), 1);
    assert!(fired(&w, b, "OnTrue").is_empty());
}

#[test]
fn wildcards_and_classname_fallback() {
    let mut w = world();
    let ids: Vec<EntId> = ["door", "Door_1", "doorway", "mydoor", "dox"]
        .iter()
        .map(|n| spawn(&mut w, &[("classname", "info_target"), ("targetname", n)]))
        .collect();
    let hit = |w: &LogicWorld, t: &str| w.resolve(t, None, None);
    let r = hit(&w, "door*");
    assert_eq!(r, vec![Who::Ent(ids[0]), Who::Ent(ids[1]), Who::Ent(ids[2])]);
    assert_eq!(hit(&w, "do*r"), vec![Who::Ent(ids[0]), Who::Ent(ids[1]), Who::Ent(ids[2]), Who::Ent(ids[4])]);

    let mut w = world();
    let d1 = spawn(&mut w, &[("classname", "func_door")]);
    let d2 = spawn(&mut w, &[("classname", "func_door")]);
    assert_eq!(w.resolve("func_door", None, None), vec![Who::Ent(d1), Who::Ent(d2)]);
    let named = spawn(&mut w, &[("classname", "info_target"), ("targetname", "func_door")]);
    assert_eq!(w.resolve("func_door", None, None), vec![Who::Ent(named)]);
}

fn touch_trigger(w: &mut LogicWorld, output: &str) -> EntId {
    spawn_brush(
        w,
        &[("classname", "trigger_multiple"), ("spawnflags", "1"), ("OnStartTouch", output)],
        Vec3::new(0.0, 0.0, 0.0),
        Vec3::new(100.0, 100.0, 100.0),
    )
}

#[test]
fn activator_targets_the_toucher() {
    let mut w = world();
    touch_trigger(&mut w, "!activator,SetHealth,50,0,-1");
    w.activate();
    let p = player_at(&mut w, 0, Vec3::new(50.0, 50.0, 0.0));
    run_to(&mut w, 0);
    assert_eq!(w.effects, vec![Effect::SetHealth { target: p, health: 50.0 }]);

    // Delayed 1 s; the player is gone after 0.5 s: nothing receives it.
    let mut w = world();
    touch_trigger(&mut w, "!activator,SetHealth,50,1,-1");
    w.activate();
    player_at(&mut w, 0, Vec3::new(50.0, 50.0, 0.0));
    run_to(&mut w, 33);
    w.players.clear();
    run_to(&mut w, 100);
    assert!(w.effects.is_empty());
}

#[test]
fn self_fireuser_and_addoutput() {
    let mut w = world();
    let r = relay(&mut w, "r", &["!self,FireUser1,,0,-1"]);
    w.activate();
    w.deliver(Who::Ent(r), "Trigger", Value::Void, None, None);
    run_to(&mut w, 0);
    assert_eq!(got(&w, r, "FireUser1"), vec![0]);
    assert_eq!(fired(&w, r, "OnUser1"), vec![0]);

    let mut w = world();
    let e = spawn(&mut w, &[("classname", "info_target"), ("targetname", "e")]);
    let r2 = relay(&mut w, "relay2", &[]);
    w.activate();
    w.deliver(Who::Ent(e), "AddOutput", Value::Str("OnUser1 relay2:Trigger::0.5:1".into()), None, None);
    w.queue_input("e", "FireUser1", Value::Void, 0.0, None);
    w.queue_input("e", "FireUser1", Value::Void, 2.0, None);
    run_to(&mut w, 300);
    assert_eq!(got(&w, r2, "Trigger"), vec![34]);

    w.deliver(Who::Ent(e), "AddOutput", Value::Str("targetname newname".into()), None, None);
    assert_eq!(w.resolve("newname", None, None), vec![Who::Ent(e)]);
    assert!(w.resolve("e", None, None).is_empty());
}

#[test]
fn fireuser_keeps_activator_with_self_as_caller() {
    let mut w = world();
    let e = spawn(&mut w, &[("classname", "info_target"), ("targetname", "e"), ("OnUser1", "t,Use,,0,-1")]);
    let t = spawn(&mut w, &[("classname", "info_target"), ("targetname", "t")]);
    let r = relay(&mut w, "r", &["e,FireUser1,,0,-1"]);
    w.activate();
    let p = player_at(&mut w, 0, Vec3::splat(-1000.0));
    w.deliver(Who::Ent(r), "Trigger", Value::Void, Some(Who::Player(p)), None);
    run_to(&mut w, 0);
    let d = w.deliveries.iter().find(|d| d.target == Who::Ent(t)).unwrap();
    assert_eq!(d.activator, Some(Who::Player(p)));
    assert_eq!(d.caller, Some(Who::Ent(e)));
}

#[test]
fn killed_entity_still_takes_inputs_this_tick() {
    let mut w = world();
    let r = relay(&mut w, "r", &[]);
    w.activate();
    run_to(&mut w, 9);
    w.queue_input("r", "Kill", Value::Void, 0.0, None);
    w.queue_input("r", "Trigger", Value::Void, 0.0, None);
    run_to(&mut w, 10);
    assert_eq!(fired(&w, r, "OnTrigger"), vec![10]);
    assert!(w.get(r).is_none(), "removed at the end of the frame");
}

#[test]
fn logic_auto_and_relay_spawn() {
    let mut w = world();
    let a = spawn(&mut w, &[("classname", "logic_auto"), ("OnMapSpawn", "x,Use,,0,-1")]);
    let r = spawn(&mut w, &[("classname", "logic_relay"), ("OnSpawn", "x,Use,,0,-1")]);
    w.activate();
    run_to(&mut w, 20);
    assert_eq!(fired(&w, a, "OnMapSpawn"), vec![13]);
    assert_eq!(fired(&w, r, "OnSpawn"), vec![1]);
}

#[test]
fn relay_refire_lock() {
    let mut w = world();
    relay(&mut w, "r", &["b,Trigger,,1,-1"]);
    let b = relay(&mut w, "b", &[]);
    w.activate();
    for t in [0, 34, 68] {
        run_to(&mut w, t - 1);
        w.queue_input("r", "Trigger", Value::Void, 0.0, None);
    }
    run_to(&mut w, 200);
    assert_eq!(got(&w, b, "Trigger"), vec![67, 135]);

    let mut w = world();
    spawn(&mut w, &[("classname", "logic_relay"), ("targetname", "r"), ("spawnflags", "2"), ("OnTrigger", "b,Trigger,,1,-1")]);
    let b = relay(&mut w, "b", &[]);
    w.activate();
    for t in [0, 34] {
        run_to(&mut w, t - 1);
        w.queue_input("r", "Trigger", Value::Void, 0.0, None);
    }
    run_to(&mut w, 200);
    assert_eq!(got(&w, b, "Trigger"), vec![67, 101]);

    let mut w = world();
    let r = spawn(&mut w, &[("classname", "logic_relay"), ("targetname", "r"), ("spawnflags", "1")]);
    w.activate();
    w.queue_input("r", "Trigger", Value::Void, 0.0, None);
    w.queue_input("r", "Trigger", Value::Void, 0.0, None);
    run_to(&mut w, 2);
    assert_eq!(fired(&w, r, "OnTrigger").len(), 1);
    assert!(w.get(r).is_none());

    let mut w = world();
    let r = relay(&mut w, "r", &["b,Trigger,,5,-1"]);
    let b = relay(&mut w, "b", &[]);
    w.activate();
    w.queue_input("r", "Trigger", Value::Void, 0.0, None);
    run_to(&mut w, 66);
    w.queue_input("r", "CancelPending", Value::Void, 0.0, None);
    run_to(&mut w, 500);
    assert!(got(&w, b, "Trigger").is_empty());
    assert!(matches!(&w.get(r).unwrap().class, Class::Relay(x) if !x.locked));
}

fn timer(pairs: &[(&str, &str)]) -> (LogicWorld, EntId) {
    let mut w = world();
    let mut all = vec![("classname", "logic_timer")];
    all.extend_from_slice(pairs);
    let t = spawn(&mut w, &all);
    w.activate();
    (w, t)
}

#[test]
fn timers() {
    let (mut w, t) = timer(&[("RefireTime", "1.0")]);
    run_to(&mut w, 210);
    assert_eq!(fired(&w, t, "OnTimer"), vec![67, 134, 201]);
    for r in ["0", "0.02"] {
        let (mut w, t) = timer(&[("RefireTime", r)]);
        run_to(&mut w, 5);
        assert_eq!(fired(&w, t, "OnTimer"), vec![1, 2, 3, 4, 5], "refire {r}");
    }
    let (mut w, t) = timer(&[("RefireTime", "1"), ("spawnflags", "1")]);
    run_to(&mut w, 210);
    assert_eq!(fired(&w, t, "OnTimerLow"), vec![67, 201]);
    assert_eq!(fired(&w, t, "OnTimerHigh"), vec![134]);
    for seed in 1..20u64 {
        let mut w = world();
        w.seed(seed * 7919);
        let t = spawn(
            &mut w,
            &[("classname", "logic_timer"), ("UseRandomTime", "1"), ("LowerRandomBound", "5"), ("UpperRandomBound", "30")],
        );
        w.activate();
        run_to(&mut w, 2001);
        let first = fired(&w, t, "OnTimer")[0];
        assert!((333..=2000).contains(&first), "first fire {first}");
    }
}

#[test]
fn timer_add_and_subtract() {
    let (mut w, t) = timer(&[("RefireTime", "2")]);
    run_to(&mut w, 99);
    w.get_mut(t).unwrap().next_think = Some(200);
    w.deliver(Who::Ent(t), "AddToTimer", Value::Str("1.0".into()), None, None);
    assert_eq!(w.get(t).unwrap().next_think, Some(267));
    w.get_mut(t).unwrap().next_think = Some(200);
    w.deliver(Who::Ent(t), "SubtractFromTimer", Value::Str("2.0".into()), None, None);
    run_to(&mut w, 101);
    let f = fired(&w, t, "OnTimer");
    assert!(f == vec![100] || f == vec![101], "fired {f:?}");
}

#[test]
fn math_counter_cases() {
    let mut w = world();
    let c = spawn(&mut w, &[("classname", "math_counter"), ("min", "0"), ("max", "5")]);
    for _ in 0..5 {
        w.deliver(Who::Ent(c), "Add", Value::Str("1".into()), None, None);
    }
    let outs: Vec<String> = w.fired.iter().map(|(_, _, o)| o.clone()).collect();
    assert_eq!(outs, ["OutValue", "OutValue", "OutValue", "OutValue", "OnHitMax", "OutValue"]);
    w.fired.clear();
    w.deliver(Who::Ent(c), "Add", Value::Str("1".into()), None, None);
    assert_eq!(counter(&w, c), 5.0);
    assert!(fired(&w, c, "OnHitMax").is_empty());
    w.deliver(Who::Ent(c), "Subtract", Value::Str("1".into()), None, None);
    w.deliver(Who::Ent(c), "Add", Value::Str("1".into()), None, None);
    assert_eq!(fired(&w, c, "OnHitMax").len(), 1);

    let mut w = world();
    let c = spawn(&mut w, &[("classname", "math_counter"), ("min", "0"), ("max", "10"), ("startvalue", "4")]);
    w.deliver(Who::Ent(c), "Divide", Value::Str("0".into()), None, None);
    assert_eq!(counter(&w, c), 4.0);
    assert_eq!(fired(&w, c, "OutValue").len(), 1);

    let mut w = world();
    let c = spawn(&mut w, &[("classname", "math_counter"), ("startvalue", "3"), ("StartDisabled", "1"), ("OnGetValue", "x,Use,,0,-1")]);
    spawn(&mut w, &[("classname", "info_target"), ("targetname", "x")]);
    w.deliver(Who::Ent(c), "Add", Value::Str("1".into()), None, None);
    w.deliver(Who::Ent(c), "GetValue", Value::Void, None, None);
    assert_eq!(counter(&w, c), 3.0);
    run_to(&mut w, 0);
    assert_eq!(w.deliveries.last().unwrap().value, Value::Float(3.0));

    let mut w = world();
    let c = spawn(&mut w, &[("classname", "math_counter")]);
    for _ in 0..3 {
        w.deliver(Who::Ent(c), "Add", Value::Str("5".into()), None, None);
    }
    assert_eq!(counter(&w, c), 15.0);
}

#[test]
fn counter_into_case_and_case_rules() {
    let mut w = world();
    let c = spawn(&mut w, &[("classname", "math_counter"), ("OutValue", "case,InValue,,0,-1")]);
    let case = spawn(&mut w, &[("classname", "logic_case"), ("targetname", "case"), ("Case01", "3"), ("Case02", "3.0")]);
    w.deliver(Who::Ent(c), "SetValue", Value::Str("3".into()), None, None);
    run_to(&mut w, 0);
    assert_eq!(fired(&w, case, "OnCase01").len(), 1);
    assert!(fired(&w, case, "OnCase02").is_empty());

    let mut w = world();
    let case = spawn(&mut w, &[("classname", "logic_case"), ("Case01", "abc"), ("Case02", "abc"), ("OnDefault", "x,Use,,0,-1")]);
    spawn(&mut w, &[("classname", "info_target"), ("targetname", "x")]);
    w.deliver(Who::Ent(case), "InValue", Value::Str("ABC".into()), None, None);
    assert_eq!(fired(&w, case, "OnCase01").len(), 1);
    assert!(fired(&w, case, "OnCase02").is_empty());
    w.deliver(Who::Ent(case), "InValue", Value::Str("zz".into()), None, None);
    run_to(&mut w, 0);
    assert_eq!(w.deliveries.last().unwrap().value, Value::Str("zz".into()));
}

#[test]
fn case_random_picks() {
    let mut w = world();
    let case = spawn(
        &mut w,
        &[("classname", "logic_case"), ("OnCase02", "x,Use,,0,-1"), ("OnCase05", "x,Use,,0,-1"), ("OnCase09", "x,Use,,0,-1")],
    );
    let mut counts = [0; 17];
    for _ in 0..3000 {
        w.fired.clear();
        w.deliver(Who::Ent(case), "PickRandom", Value::Void, None, None);
        let n: usize = w.fired[0].2[6..].parse().unwrap();
        counts[n] += 1;
    }
    for (n, c) in counts.iter().enumerate() {
        if [2, 5, 9].contains(&n) {
            assert!((800..1200).contains(c), "case {n}: {c}");
        } else {
            assert_eq!(*c, 0);
        }
    }
    let mut picks = Vec::new();
    for _ in 0..39 {
        w.fired.clear();
        w.deliver(Who::Ent(case), "PickRandomShuffle", Value::Void, None, None);
        picks.push(w.fired[0].2.clone());
    }
    for round in picks.chunks(3) {
        let mut r = round.to_vec();
        r.sort();
        assert_eq!(r, ["OnCase02", "OnCase05", "OnCase09"]);
    }
    assert!(picks.windows(2).all(|p| p[0] != p[1]), "no immediate repeat: {picks:?}");
}

#[test]
fn compare() {
    let mut w = world();
    let c = spawn(&mut w, &[("classname", "logic_compare"), ("CompareValue", "5")]);
    w.deliver(Who::Ent(c), "SetValueCompare", Value::Str("5".into()), None, None);
    let outs: Vec<String> = w.fired.iter().map(|(_, _, o)| o.clone()).collect();
    assert_eq!(outs, ["OnEqualTo"]);
    w.fired.clear();
    w.deliver(Who::Ent(c), "SetValueCompare", Value::Str("7".into()), None, None);
    let outs: Vec<String> = w.fired.iter().map(|(_, _, o)| o.clone()).collect();
    assert_eq!(outs, ["OnNotEqualTo", "OnGreaterThan"]);
}

#[test]
fn filters() {
    let mut w = world();
    let f = spawn(&mut w, &[("classname", "filter_activator_name"), ("filtername", "bob*")]);
    let fneg = spawn(&mut w, &[("classname", "filter_activator_name"), ("filtername", "bob*"), ("Negated", "1")]);
    let fp = spawn(&mut w, &[("classname", "filter_activator_name"), ("filtername", "!player")]);
    let bob = spawn(&mut w, &[("classname", "info_target"), ("targetname", "Bob1")]);
    let p = player_at(&mut w, 0, Vec3::ZERO);
    w.players[0].team = 3;
    let t = player_at(&mut w, 1, Vec3::ZERO);
    w.players[1].team = 2;
    w.player_names.push((p, "b".into()));
    let pass = |w: &LogicWorld, f: EntId, x: Who| super::classes::filter_passes(w, f, Some(x));
    assert!(pass(&w, f, Who::Ent(bob)));
    assert!(!pass(&w, fneg, Who::Ent(bob)));
    assert!(pass(&w, fp, Who::Player(p)));
    let team = spawn(&mut w, &[("classname", "filter_activator_team"), ("filterteam", "3")]);
    assert!(pass(&w, team, Who::Player(p)));
    assert!(!pass(&w, team, Who::Player(t)));
    spawn(&mut w, &[("classname", "filter_activator_name"), ("targetname", "fa"), ("filtername", "a")]);
    spawn(&mut w, &[("classname", "filter_activator_class"), ("targetname", "fc"), ("filterclass", "player")]);
    let and0 = spawn(&mut w, &[("classname", "filter_multi")]);
    let or0 = spawn(&mut w, &[("classname", "filter_multi"), ("FilterType", "1")]);
    let or = spawn(
        &mut w,
        &[("classname", "filter_multi"), ("FilterType", "1"), ("Filter01", "fa"), ("Filter02", "fc"), ("Negated", "1")],
    );
    w.activate();
    assert!(pass(&w, and0, Who::Ent(bob)));
    assert!(!pass(&w, or0, Who::Ent(bob)));
    assert!(!pass(&w, or, Who::Player(p)));
}

#[test]
fn game_text_rules() {
    let mut w = world();
    spawn(&mut w, &[("classname", "game_text"), ("targetname", "t"), ("message", "hi")]);
    spawn(&mut w, &[("classname", "logic_auto"), ("OnMapSpawn", "t,Display,,0,-1")]);
    w.activate();
    player_at(&mut w, 0, Vec3::ZERO);
    run_to(&mut w, 20);
    assert!(w.effects.is_empty(), "no activator, no 'all players': nobody sees it");

    let m = HudMessage {
        text: "x".into(),
        x: -1.0,
        y: 0.25,
        channel: 7,
        effect: 0,
        color: [255; 3],
        color2: [255; 3],
        fade_in: 1.5,
        fade_out: 0.5,
        hold: 2.0,
        fx_time: 0.25,
    };
    assert_eq!(m.slot(), 1);
    assert!((m.lifetime() - 4.0).abs() < 1e-6);
    assert!((m.opacity(0.75) - 0.5).abs() < 1e-6);
    assert_eq!(m.opacity(2.0), 1.0);
    assert!((m.opacity(3.8) - 0.4).abs() < 1e-5);
    let p = m.place(Vec2::new(1920.0, 1080.0), 300.0, 300.0, 24.0);
    assert_eq!(p, Vec2::new(810.0, 270.0));
    let scan = HudMessage {
        text: "0123456789".into(),
        effect: 2,
        fade_in: 0.05,
        hold: 2.0,
        fade_out: 1.0,
        ..m
    };
    assert!(scan.scan_color(9, 0.49).is_none());
    assert!(scan.scan_color(9, 0.5).is_some());
    assert!((scan.lifetime() - 3.5).abs() < 1e-5);
}

#[test]
fn command_allowlists() {
    let set = |n: &str, v: &str| Ok(ServerLine::Set { name: n.into(), value: v.into() });
    assert_eq!(check_server_command("sv_gravity 400"), set("sv_gravity", "400"));
    assert_eq!(check_server_command("rcon_password x"), Err("not a game setting"));
    assert_eq!(check_server_command("sv_gravity 400; quit"), Err("more than one command"));
    assert_eq!(check_server_command("sv_gravity 99999"), set("sv_gravity", "4000"));
    assert_eq!(check_server_command("say hello there"), Ok(ServerLine::Say("hello there".into())));
    assert_eq!(check_server_command("say \"hi\""), Ok(ServerLine::Say("hi".into())));
    // Game-rule settings by prefix, as community maps send them; values
    // quoted or not, names in any case.
    assert_eq!(check_server_command(" SV_MaxVelocity  \"5000\" "), set("sv_maxvelocity", "5000"));
    assert_eq!(check_server_command("sv_maxvelocity 1e9"), set("sv_maxvelocity", "100000"));
    assert_eq!(check_server_command("sv_enablebunnyhopping 1"), set("sv_enablebunnyhopping", "1"));
    // The cheats gate is the server operator's alone.
    assert_eq!(check_server_command("sv_cheats 1"), Err("a setting maps may not change"));
    assert_eq!(check_server_command("mp_freezetime 0"), set("mp_freezetime", "0"));
    assert_eq!(check_server_command("phys_pushscale 50"), set("phys_pushscale", "50"));
    assert_eq!(check_server_command("bot_stop 1"), set("bot_stop", "1"));
    assert_eq!(check_server_command("ammo_50ae_max 999"), set("ammo_50ae_max", "999"));
    assert_eq!(check_server_command("mp_restartgame 3"), set("mp_restartgame", "3"));
    // Commands that do something other than set a game rule.
    for line in [
        "quit",
        "exit",
        "exec server.cfg",
        "bind w quit",
        "unbindall",
        "connect 1.2.3.4",
        "rcon quit",
        "host_writeconfig",
        "writeip",
        "changelevel de_dust2",
        "map de_dust2",
        "kick Bot",
        "banid 0 x",
        "sm_say hi",
        "ma_csay hi",
        "alias a quit",
        "host_timescale 10",
    ] {
        assert_eq!(check_server_command(line), Err("not a game setting"), "{line}");
    }
    // Settings with a fitting prefix that maps may still not touch.
    for line in [
        "sv_password secret",
        "sv_rcon_banpenalty 1",
        "sv_allowdownload 1",
        "sv_allowupload 1",
        "sv_downloadurl http://x",
        "sv_logfile 1",
        "mp_logdetail 3",
        "sv_lan 0",
        "sv_pure 0",
        "sv_allow_point_servercommand always",
    ] {
        assert_eq!(check_server_command(line), Err("a setting maps may not change"), "{line}");
    }
    assert_eq!(check_server_command("sv_gravity"), Err("no value"));
    assert_eq!(check_server_command("sv_gravity fast"), Err("bad value"));
    assert_eq!(check_server_command("mp_x a\"b"), Err("bad value"));
    assert!(allowed_client_command("play buttons/button1.wav").is_some());
    assert!(allowed_client_command("bind w quit").is_none());

    let mut w = world();
    let s = spawn(&mut w, &[("classname", "point_servercommand")]);
    w.deliver(Who::Ent(s), "Command", Value::Str("sv_gravity 400".into()), None, None);
    w.deliver(Who::Ent(s), "Command", Value::Str("rcon_password x".into()), None, None);
    assert_eq!(w.effects, vec![Effect::ServerCommand(ServerLine::Set { name: "sv_gravity".into(), value: "400".into() })]);
    assert!(w.log.iter().any(|l| l.contains("refused")));

    let c = spawn(&mut w, &[("classname", "point_clientcommand")]);
    let p = player_at(&mut w, 0, Vec3::ZERO);
    w.effects.clear();
    w.deliver(Who::Ent(c), "Command", Value::Str("play buttons/button1.wav".into()), Some(Who::Player(p)), None);
    w.deliver(Who::Ent(c), "Command", Value::Str("play x".into()), None, None);
    assert_eq!(
        w.effects,
        vec![Effect::ClientCommand {
            player: p,
            command: "play buttons/button1.wav".into()
        }]
    );
}

#[test]
fn zero_delay_loops_are_capped() {
    let mut w = world();
    relay(&mut w, "a", &["b,Trigger,,0,-1"]);
    spawn(&mut w, &[("classname", "logic_relay"), ("targetname", "b"), ("spawnflags", "2"), ("OnTrigger", "a,Trigger,,0,-1"), ("OnTrigger", "a,EnableRefire,,0,-1")]);
    w.activate();
    w.queue_input("a", "Trigger", Value::Void, 0.0, None);
    run_to(&mut w, 1);
    assert!(w.log.iter().any(|l| l.contains("deliveries in one tick")));
}

// ----------------------------------------------------------- triggers

#[test]
fn touch_needs_overlap() {
    for (x, touches) in [(-17.0, false), (-15.0, true)] {
        let mut w = world();
        let t = spawn_brush(
            &mut w,
            &[("classname", "trigger_multiple"), ("spawnflags", "1"), ("OnStartTouch", "x,Use,,0,-1")],
            Vec3::ZERO,
            Vec3::new(10.0, 10.0, 100.0),
        );
        w.activate();
        let p = player_at(&mut w, 0, Vec3::new(x, 5.0, 0.0));
        w.players[0].mins = Vec3::new(-16.0, -16.0, 0.0);
        let _ = p;
        run_to(&mut w, 3);
        assert_eq!(fired(&w, t, "OnStartTouch").len(), touches as usize, "x {x}");
    }
}

#[test]
fn dead_players_and_flags_dont_touch() {
    let mut w = world();
    let t = touch_trigger(&mut w, "x,Use,,0,-1");
    w.activate();
    player_at(&mut w, 0, Vec3::splat(50.0));
    w.players[0].alive = false;
    run_to(&mut w, 3);
    assert!(fired(&w, t, "OnStartTouch").is_empty());
}

#[test]
fn trigger_multiple_refires_while_standing() {
    for (wait, expect) in [("1", vec![0, 67, 134, 201, 268]), ("0", (0..300).step_by(13).collect())] {
        let mut w = world();
        let t = spawn_brush(
            &mut w,
            &[("classname", "trigger_multiple"), ("spawnflags", "1"), ("wait", wait), ("OnTrigger", "x,Use,,0,-1"), ("OnStartTouch", "x,Use,,0,-1")],
            Vec3::ZERO,
            Vec3::splat(100.0),
        );
        w.activate();
        player_at(&mut w, 0, Vec3::splat(50.0));
        run_to(&mut w, 299);
        assert_eq!(fired(&w, t, "OnStartTouch"), vec![0]);
        assert_eq!(fired(&w, t, "OnTrigger"), expect, "wait {wait}");
    }
}

#[test]
fn trigger_once() {
    let mut w = world();
    let t = spawn_brush(
        &mut w,
        &[("classname", "trigger_once"), ("spawnflags", "1"), ("OnTrigger", "x,Use,,0,-1"), ("OnStartTouch", "x,Use,,0,-1")],
        Vec3::ZERO,
        Vec3::splat(100.0),
    );
    w.activate();
    player_at(&mut w, 0, Vec3::splat(50.0));
    run_to(&mut w, 2);
    player_at(&mut w, 1, Vec3::splat(50.0));
    run_to(&mut w, 6);
    assert!(w.get(t).is_some());
    run_to(&mut w, 7);
    assert_eq!(fired(&w, t, "OnTrigger"), vec![0]);
    assert_eq!(fired(&w, t, "OnStartTouch"), vec![0, 3]);
    assert!(w.get(t).is_none(), "gone at tick 7");
}

#[test]
fn end_touch_all_prunes_the_dead() {
    let mut w = world();
    let t = spawn_brush(
        &mut w,
        &[("classname", "trigger_multiple"), ("spawnflags", "1"), ("OnEndTouch", "x,Use,,0,-1"), ("OnEndTouchAll", "x,Use,,0,-1")],
        Vec3::ZERO,
        Vec3::splat(100.0),
    );
    w.activate();
    player_at(&mut w, 0, Vec3::splat(50.0));
    player_at(&mut w, 1, Vec3::splat(50.0));
    run_to(&mut w, 5);
    w.players[0].origin = Vec3::splat(500.0);
    run_to(&mut w, 10);
    assert_eq!(fired(&w, t, "OnEndTouch"), vec![6]);
    assert!(fired(&w, t, "OnEndTouchAll").is_empty());
    w.players[1].alive = false;
    run_to(&mut w, 15);
    assert_eq!(fired(&w, t, "OnEndTouchAll"), vec![11]);
}

fn damage_ticks(w: &mut LogicWorld, until: i64, leave: Option<i64>) -> Vec<(i64, f32)> {
    let mut out = Vec::new();
    while w.tick <= until {
        if Some(w.tick) == leave {
            w.players[0].origin = Vec3::splat(500.0);
        }
        let t = w.tick;
        w.frame(&NoCollision);
        for e in w.effects.drain(..) {
            match e {
                Effect::Damage { amount, .. } => out.push((t, amount)),
                Effect::Heal { amount, .. } => out.push((t, -amount)),
                _ => {}
            }
        }
    }
    out
}

fn hurt(damage: &str, extra: &[(&str, &str)]) -> LogicWorld {
    let mut w = world();
    let mut pairs = vec![("classname", "trigger_hurt"), ("spawnflags", "1"), ("damage", damage)];
    pairs.extend_from_slice(extra);
    spawn_brush(&mut w, &pairs, Vec3::ZERO, Vec3::splat(100.0));
    w.activate();
    player_at(&mut w, 0, Vec3::splat(50.0));
    w
}

#[test]
fn trigger_hurt_passes() {
    // Entered at tick 0: passes every 33 ticks (the first at the next
    // think opportunity, tick 1 here; open question 8).
    let mut w = hurt("50", &[]);
    let hits = damage_ticks(&mut w, 120, None);
    assert_eq!(hits, vec![(1, 25.0), (34, 25.0), (67, 25.0), (100, 25.0)]);

    // Leaves after being hurt in the last pass: no exit hit.
    let mut w = hurt("20", &[]);
    assert_eq!(damage_ticks(&mut w, 100, Some(40)), vec![(1, 10.0), (34, 10.0)]);
    let mut w = hurt("20", &[]);
    assert_eq!(damage_ticks(&mut w, 100, Some(10)), vec![(1, 10.0)]);

    // A dash through before the first pass: the exit half-hit.
    let mut w = hurt("20", &[]);
    assert_eq!(damage_ticks(&mut w, 20, Some(1)), vec![(1, 10.0)]);

    // Healing.
    let mut w = hurt("-10", &[]);
    assert_eq!(damage_ticks(&mut w, 40, None), vec![(1, -5.0), (34, -5.0)]);

    // Doubling up to the cap; forgiven after 3 s away.
    let mut w = hurt("10", &[("damagemodel", "1"), ("damagecap", "50")]);
    let hits: Vec<f32> = damage_ticks(&mut w, 140, None).into_iter().map(|(_, d)| d).collect();
    assert_eq!(hits, vec![5.0, 10.0, 20.0, 25.0, 25.0]);
    let mut w = hurt("10", &[("damagemodel", "1"), ("damagecap", "50")]);
    damage_ticks(&mut w, 40, Some(40));
    damage_ticks(&mut w, 40 + 207, None);
    w.players[0].origin = Vec3::splat(50.0);
    let hits = damage_ticks(&mut w, 300, None);
    assert_eq!(hits[0].1, 5.0, "back to the original damage: {hits:?}");
}

#[test]
fn trigger_push_sets_base_velocity() {
    let mut w = world();
    spawn_brush(&mut w, &[("classname", "trigger_push"), ("spawnflags", "1"), ("pushdir", "0 0 0"), ("speed", "100")], Vec3::ZERO, Vec3::splat(100.0));
    spawn_brush(&mut w, &[("classname", "trigger_push"), ("spawnflags", "1"), ("pushdir", "0 90 0"), ("speed", "100")], Vec3::ZERO, Vec3::splat(100.0));
    w.activate();
    player_at(&mut w, 0, Vec3::splat(50.0));
    run_to(&mut w, 0);
    let p = &w.players[0];
    assert!((p.base_velocity - Vec3::new(100.0, 100.0, 0.0)).length() < 1e-3, "{}", p.base_velocity);
    assert!(p.base_touched);

    let mut w = world();
    let t = spawn_brush(
        &mut w,
        &[("classname", "trigger_push"), ("spawnflags", "129"), ("pushdir", "-90 0 0"), ("speed", "500")],
        Vec3::ZERO,
        Vec3::splat(100.0),
    );
    w.activate();
    player_at(&mut w, 0, Vec3::splat(50.0));
    run_to(&mut w, 0);
    assert!((w.players[0].velocity.z - 500.0).abs() < 1e-3);
    assert!(w.get(t).is_none());
}

fn teleport(pairs: &[(&str, &str)]) -> LogicWorld {
    let mut w = world();
    let mut all = vec![("classname", "trigger_teleport"), ("spawnflags", "1")];
    all.extend_from_slice(pairs);
    spawn_brush(&mut w, &all, Vec3::ZERO, Vec3::splat(100.0));
    spawn(&mut w, &[("classname", "info_teleport_destination"), ("targetname", "d"), ("origin", "1000 0 64"), ("angles", "0 90 0")]);
    spawn(&mut w, &[("classname", "info_target"), ("targetname", "l"), ("origin", "0 0 0")]);
    w.activate();
    player_at(&mut w, 0, Vec3::new(5.0, 3.0, 0.0));
    w.players[0].velocity = Vec3::new(400.0, 0.0, 0.0);
    w.players[0].view = Vec3::new(30.0, 0.0, 0.0);
    run_to(&mut w, 0);
    w
}

#[test]
fn trigger_teleport() {
    let w = teleport(&[("target", "d")]);
    let p = &w.players[0];
    assert_eq!(p.origin, Vec3::new(1000.0, 0.0, 64.0));
    assert_eq!(p.velocity, Vec3::new(400.0, 0.0, 0.0));
    assert_eq!(p.view, Vec3::new(0.0, 90.0, 0.0));
    let w = teleport(&[("target", "d"), ("landmark", "l")]);
    let p = &w.players[0];
    assert_eq!(p.origin, Vec3::new(1005.0, 3.0, 64.0));
    assert_eq!(p.view, Vec3::new(30.0, 0.0, 0.0));
    let w = teleport(&[("target", "missing")]);
    assert_eq!(w.players[0].origin, Vec3::new(5.0, 3.0, 0.0));
    let w = teleport(&[("target", "d"), ("spawnflags", "33")]);
    assert_eq!(w.players[0].origin, Vec3::new(5.0, 3.0, 0.0));
    let w = teleport(&[("target", "!activator")]);
    assert_eq!(w.players[0].origin, Vec3::new(5.0, 3.0, 0.0));
}

/// The triggers a player overlaps are listed before their touches run:
/// a trigger_multiple sharing a teleport's volume fires although the
/// teleport (first in the list) moved the player away (mg_wipeout2's
/// stage teleports score this way); at the destination the player
/// re-links (a trigger there fires the same tick).
#[test]
fn teleport_keeps_the_other_touches() {
    let mut w = world();
    spawn_brush(&mut w, &[("classname", "trigger_teleport"), ("spawnflags", "1"), ("target", "d")], Vec3::ZERO, Vec3::splat(100.0));
    let score = spawn_brush(
        &mut w,
        &[("classname", "trigger_multiple"), ("spawnflags", "1"), ("OnTrigger", "x,Use,,0,-1")],
        Vec3::ZERO,
        Vec3::splat(100.0),
    );
    let there = spawn_brush(
        &mut w,
        &[("classname", "trigger_multiple"), ("spawnflags", "1"), ("OnStartTouch", "x,Use,,0,-1")],
        Vec3::new(950.0, -50.0, 0.0),
        Vec3::new(1050.0, 50.0, 128.0),
    );
    spawn(&mut w, &[("classname", "info_teleport_destination"), ("targetname", "d"), ("origin", "1000 0 64")]);
    w.activate();
    player_at(&mut w, 0, Vec3::new(5.0, 3.0, 0.0));
    run_to(&mut w, 0);
    assert_eq!(w.players[0].origin, Vec3::new(1000.0, 0.0, 64.0));
    assert_eq!(fired(&w, score, "OnTrigger"), vec![0], "the volume shared with the teleport");
    assert_eq!(fired(&w, there, "OnStartTouch"), vec![0], "the destination's, the same tick");
}

#[test]
fn trigger_gravity_and_filtered_trigger() {
    let mut w = world();
    spawn_brush(&mut w, &[("classname", "trigger_gravity"), ("spawnflags", "1"), ("gravity", "0.5")], Vec3::ZERO, Vec3::splat(100.0));
    w.activate();
    player_at(&mut w, 0, Vec3::splat(50.0));
    run_to(&mut w, 0);
    w.players[0].origin = Vec3::splat(500.0);
    run_to(&mut w, 3);
    assert_eq!(w.players[0].gravity, 0.5);

    let mut w = world();
    spawn(&mut w, &[("classname", "filter_activator_team"), ("targetname", "ts"), ("filterteam", "2")]);
    let t = spawn_brush(
        &mut w,
        &[("classname", "trigger_multiple"), ("spawnflags", "1"), ("filtername", "ts"), ("OnStartTouch", "x,Use,,0,-1"), ("OnTrigger", "x,Use,,0,-1")],
        Vec3::ZERO,
        Vec3::splat(100.0),
    );
    w.activate();
    player_at(&mut w, 0, Vec3::splat(50.0));
    w.players[0].team = 3;
    run_to(&mut w, 3);
    assert!(fired(&w, t, "OnStartTouch").is_empty() && fired(&w, t, "OnTrigger").is_empty());
}

// ------------------------------------------------------------- movers

pub(super) fn origin_of(w: &LogicWorld, id: EntId) -> Vec3 {
    pusher(&w.get(id).unwrap().class).unwrap().origin
}

fn angles_of(w: &LogicWorld, id: EntId) -> Vec3 {
    pusher(&w.get(id).unwrap().class).unwrap().angles
}

/// A func_door 64x8x128 at (100, 0, 64), the player at (40, 0, 0)
/// looking +X.
fn door_world(extra: &[(&str, &str)]) -> (LogicWorld, EntId) {
    let mut w = world();
    let mut pairs = vec![
        ("classname", "func_door"),
        ("targetname", "door"),
        ("origin", "100 0 64"),
        ("movedir", "0 0 0"),
        ("lip", "4"),
        ("speed", "100"),
        ("wait", "4"),
        ("spawnflags", "256"),
    ];
    pairs.extend_from_slice(extra);
    let d = spawn_brush(&mut w, &pairs, Vec3::new(-32.0, -4.0, -64.0), Vec3::new(32.0, 4.0, 64.0));
    w.activate();
    player_at(&mut w, 0, Vec3::new(40.0, 0.0, 0.0));
    (w, d)
}

#[test]
fn door_opens_on_use_and_closes_after_wait() {
    let (mut w, d) = door_world(&[]);
    w.players[0].use_key = true;
    run_to(&mut w, 0);
    w.players[0].use_key = false;
    assert_eq!(fired(&w, d, "OnOpen"), vec![0]);
    run_to(&mut w, 38);
    assert!((origin_of(&w, d).x - 157.0).abs() < 1e-3, "{}", origin_of(&w, d));
    run_to(&mut w, 39);
    assert_eq!(origin_of(&w, d).x, 158.0);
    assert_eq!(fired(&w, d, "OnFullyOpen"), vec![39]);
    run_to(&mut w, 400);
    assert_eq!(fired(&w, d, "OnClose"), vec![39 + 267]);
    assert_eq!(fired(&w, d, "OnFullyClosed"), vec![39 + 267 + 39]);
    assert_eq!(origin_of(&w, d).x, 100.0);
}

#[test]
fn holding_use_activates_once() {
    let (mut w, d) = door_world(&[("wait", "-1"), ("spawnflags", "288")]);
    w.players[0].use_key = true;
    run_to(&mut w, 133);
    assert_eq!(fired(&w, d, "OnOpen"), vec![0]);
    assert!(fired(&w, d, "OnClose").is_empty());
}

#[test]
fn sliding_door_up_never_returns() {
    let mut w = world();
    let d = spawn_brush(
        &mut w,
        &[("classname", "func_door"), ("targetname", "door"), ("movedir", "-90 0 0"), ("lip", "16"), ("wait", "-1"), ("speed", "100")],
        Vec3::new(-32.0, -4.0, -64.0),
        Vec3::new(32.0, 4.0, 64.0),
    );
    w.activate();
    w.queue_input("door", "Open", Value::Void, 0.0, None);
    run_to(&mut w, 73);
    assert!(fired(&w, d, "OnFullyOpen").is_empty());
    run_to(&mut w, 74);
    assert_eq!(fired(&w, d, "OnFullyOpen"), vec![74]);
    assert_eq!(origin_of(&w, d).z, 110.0);
    w.queue_input("door", "Open", Value::Void, 0.0, None);
    run_to(&mut w, 600);
    assert_eq!(origin_of(&w, d).z, 110.0);
    assert_eq!(fired(&w, d, "OnOpen").len(), 1);
}

#[test]
fn locked_doors() {
    let (mut w, d) = door_world(&[("spawnflags", "2304")]);
    w.queue_input("door", "Open", Value::Void, 0.0, None);
    run_to(&mut w, 5);
    assert!(fired(&w, d, "OnOpen").is_empty());
    w.players[0].use_key = true;
    run_to(&mut w, 6);
    assert_eq!(fired(&w, d, "OnLockedUse"), vec![6]);
    assert!(w.effects.iter().any(|e| matches!(e, Effect::Sound { entry, .. } if entry == "DoorSound.DefaultLocked")));
    w.queue_input("door", "Unlock", Value::Void, 0.0, None);
    w.queue_input("door", "Open", Value::Void, 0.0, None);
    run_to(&mut w, 8);
    assert_eq!(fired(&w, d, "OnOpen"), vec![7]);

    let (mut w, d) = door_world(&[("spawnflags", "0")]);
    w.players[0].use_key = true;
    run_to(&mut w, 3);
    assert!(fired(&w, d, "OnOpen").is_empty());
    assert!(w.effects.iter().any(|e| matches!(e, Effect::Sound { entry, .. } if entry == "DoorSound.DefaultLocked")));
}

/// A door starting open (x 158) closing onto a player pinned against a
/// wall, so the push can't move them.
fn blocked_door(extra: &[(&str, &str)]) -> (LogicWorld, EntId, BrushCollision) {
    let mut w = world();
    let mut pairs = vec![
        ("classname", "func_door"),
        ("targetname", "door"),
        ("origin", "158 0 64"),
        ("movedir", "0 0 0"),
        ("lip", "4"),
        ("speed", "100"),
        ("spawnpos", "1"),
        ("dmg", "10"),
    ];
    pairs.extend_from_slice(extra);
    let d = spawn_brush(&mut w, &pairs, Vec3::new(-32.0, -4.0, -64.0), Vec3::new(32.0, 4.0, 64.0));
    w.activate();
    // Spawned at the open spot; P1 is the closed spot 58 back.
    if let Some(Class::Door(door)) = w.get_mut(d).map(|e| &mut e.class) {
        door.p1 = Vec3::new(100.0, 0.0, 64.0);
        door.p2 = Vec3::new(158.0, 0.0, 64.0);
    }
    player_at(&mut w, 0, Vec3::new(110.0, 0.0, 0.0));
    let wall = BrushCollision(vec![MapBrush::from_box(Vec3::new(60.0, -100.0, 0.0), Vec3::new(93.0, 100.0, 200.0))]);
    (w, d, wall)
}

fn total_damage(w: &mut LogicWorld) -> f32 {
    w.effects
        .drain(..)
        .map(|e| match e {
            Effect::Damage { amount, .. } => amount,
            _ => 0.0,
        })
        .sum()
}

#[test]
fn blocked_doors() {
    let (mut w, d, wall) = blocked_door(&[("wait", "4")]);
    w.queue_input("door", "Close", Value::Void, 0.0, None);
    run_to_with(&mut w, 60, &wall);
    let blocked = fired(&w, d, "OnBlockedClosing");
    assert_eq!(blocked.len(), 1, "{blocked:?}");
    assert_eq!(total_damage(&mut w), 10.0);
    assert!(fired(&w, d, "OnOpen").len() == 1, "reversed");

    let (mut w, _, wall) = blocked_door(&[("wait", "-1")]);
    w.queue_input("door", "Close", Value::Void, 0.0, None);
    run_to_with(&mut w, 0, &wall);
    // Closing at 1.5/tick: it reaches the player (x 126 face at 110+16)
    // after a few ticks, then pushes and damages every tick.
    let mut first = None;
    for _ in 0..60 {
        let t = w.tick;
        run_to_with(&mut w, t, &wall);
        if first.is_none() && w.effects.iter().any(|e| matches!(e, Effect::Damage { .. })) {
            first = Some(t);
            w.effects.clear();
            let pos = origin_of(&w, d);
            run_to_with(&mut w, t + 10, &wall);
            assert_eq!(total_damage(&mut w), 100.0);
            assert_eq!(origin_of(&w, d), pos, "doesn't move or reverse");
            break;
        }
    }
    assert!(first.is_some());

    let (mut w, d, wall) = blocked_door(&[("wait", "4"), ("dmg", "0"), ("forceclosed", "1")]);
    w.queue_input("door", "Close", Value::Void, 0.0, None);
    run_to_with(&mut w, 60, &wall);
    assert_eq!(total_damage(&mut w), 0.0);
    assert_eq!(fired(&w, d, "OnOpen").len(), 0);
    assert!(matches!(&w.get(d).unwrap().class, Class::Door(x) if x.state == DoorState::Closing));
}

fn rotating_door(flags: &str, user: Vec3) -> (LogicWorld, EntId) {
    let mut w = world();
    let d = spawn_brush(
        &mut w,
        &[("classname", "func_door_rotating"), ("targetname", "door"), ("distance", "90"), ("speed", "100"), ("wait", "-1"), ("spawnflags", flags)],
        Vec3::new(0.0, -2.0, 0.0),
        Vec3::new(64.0, 2.0, 100.0),
    );
    w.activate();
    let p = player_at(&mut w, 0, user);
    w.queue_input("door", "Use", Value::Void, 0.0, Some(Who::Player(p)));
    run_to(&mut w, 100);
    (w, d)
}

#[test]
fn rotating_doors_open_away() {
    let (w, d) = rotating_door("256", Vec3::new(32.0, -40.0, 0.0));
    assert_eq!(angles_of(&w, d).y, 90.0);
    let (w, d) = rotating_door("256", Vec3::new(32.0, 40.0, 0.0));
    assert_eq!(angles_of(&w, d).y, -90.0);
    let (w, d) = rotating_door("272", Vec3::new(200.0, 40.0, 0.0)); // one-way; out of the swing
    assert_eq!(angles_of(&w, d).y, 90.0);

    // Opened by input after that user: swings using the old user.
    let (mut w, d) = rotating_door("256", Vec3::new(32.0, 40.0, 0.0));
    w.players[0].origin = Vec3::new(500.0, 500.0, 0.0);
    w.queue_input("door", "Close", Value::Void, 0.0, None);
    run_to(&mut w, 300);
    w.players[0].origin = Vec3::new(32.0, 40.0, 0.0);
    w.queue_input("door", "Open", Value::Void, 0.0, None);
    run_to(&mut w, 400);
    assert_eq!(angles_of(&w, d).y, -90.0);
}

#[test]
fn de_nuke_door_pair() {
    let mut w = world();
    let pair = |w: &mut LogicWorld, name: &str, other: &str, y: f32| {
        let origin = format!("0 {y} 0");
        spawn_brush(
            w,
            &[
                ("classname", "func_door_rotating"),
                ("targetname", name),
                ("chainstodoor", other),
                ("origin", &origin),
                ("distance", "90"),
                ("speed", "200"),
                ("wait", "4"),
                ("spawnflags", "1280"),
            ],
            Vec3::new(0.0, -2.0, 0.0),
            Vec3::new(64.0, 2.0, 100.0),
        )
    };
    let a = pair(&mut w, "a", "b", 0.0);
    let b = pair(&mut w, "b", "a", 200.0);
    w.activate();
    let p = player_at(&mut w, 0, Vec3::new(32.0, -100.0, 0.0));
    w.queue_input("a", "Use", Value::Void, 0.0, Some(Who::Player(p)));
    run_to(&mut w, 29);
    assert!(angles_of(&w, a).y.abs() < 90.0);
    run_to(&mut w, 30);
    assert_eq!(angles_of(&w, a).y.abs(), 90.0);
    assert_eq!(angles_of(&w, b).y.abs(), 90.0);
    run_to(&mut w, 30 + 267 + 30);
    assert_eq!(angles_of(&w, a).y, 0.0);
    assert_eq!(angles_of(&w, b).y, 0.0);
}

fn button_world(pairs: &[(&str, &str)]) -> (LogicWorld, EntId) {
    let mut w = world();
    let mut all = vec![("classname", "func_button"), ("targetname", "b"), ("origin", "100 0 64"), ("movedir", "0 0 0")];
    all.extend_from_slice(pairs);
    // 8 deep along the move.
    let b = spawn_brush(&mut w, &all, Vec3::new(-4.0, -8.0, -8.0), Vec3::new(4.0, 8.0, 8.0));
    w.activate();
    player_at(&mut w, 0, Vec3::new(40.0, 0.0, 0.0));
    (w, b)
}

fn use_at(w: &mut LogicWorld, ticks: &[i64], until: i64) {
    while w.tick <= until {
        w.players[0].use_key = ticks.contains(&w.tick);
        w.frame(&NoCollision);
    }
}

#[test]
fn buttons() {
    let (mut w, b) = button_world(&[("spawnflags", "1024"), ("speed", "40"), ("lip", "4"), ("wait", "1")]);
    use_at(&mut w, &[0], 100);
    assert_eq!(fired(&w, b, "OnPressed"), vec![0]);
    assert_eq!(fired(&w, b, "OnIn"), vec![4]);
    assert_eq!(fired(&w, b, "OnOut"), vec![4 + 67 + 4]);

    let (mut w, b) = button_world(&[("spawnflags", "1025"), ("wait", "3.5")]);
    use_at(&mut w, &[0, 100, 240], 260);
    assert_eq!(fired(&w, b, "OnPressed"), vec![0, 240]);
    assert_eq!(fired(&w, b, "OnIn"), vec![0, 240]);
    assert_eq!(fired(&w, b, "OnOut"), vec![233]);

    let (mut w, b) = button_world(&[("spawnflags", "1056")]);
    use_at(&mut w, &[0, 20], 40);
    assert_eq!(fired(&w, b, "OnPressed"), vec![0, 20]);
    assert_eq!(fired(&w, b, "OnIn"), vec![4]);
    assert_eq!(fired(&w, b, "OnOut"), vec![24]);

    let (mut w, b) = button_world(&[("spawnflags", "3072")]);
    use_at(&mut w, &[0, 8, 16], 20);
    assert_eq!(fired(&w, b, "OnUseLocked"), vec![0]);
}

/// "Touch Activates" (256): walking into the button presses it, again
/// once it is back out; a locked one does nothing; without the flag a
/// touch does nothing (mg_escape_prison_beta's gas trap).
#[test]
fn touch_activated_buttons() {
    let (mut w, b) = button_world(&[("spawnflags", "257"), ("wait", "1")]);
    run_to(&mut w, 2);
    assert!(fired(&w, b, "OnPressed").is_empty(), "not touching yet");
    // Against its face (x 96..104, z 56..72; the player box 32 wide).
    player_at(&mut w, 0, Vec3::new(80.0, 0.0, 30.0));
    run_to(&mut w, 100);
    // Out again after its wait; still touched: pressed on the next tick.
    assert_eq!(fired(&w, b, "OnPressed"), vec![3, 3 + 67 + 1]);
    let (mut w, b) = button_world(&[("spawnflags", "1"), ("wait", "1")]);
    player_at(&mut w, 0, Vec3::new(80.0, 0.0, 30.0));
    run_to(&mut w, 20);
    assert!(fired(&w, b, "OnPressed").is_empty(), "no touch flag");
    let (mut w, b) = button_world(&[("spawnflags", "2305"), ("wait", "1")]);
    player_at(&mut w, 0, Vec3::new(80.0, 0.0, 30.0));
    run_to(&mut w, 20);
    assert!(fired(&w, b, "OnPressed").is_empty(), "locked");
}

#[test]
fn use_reach() {
    // Eye at (0,0,64) looking +X; a button face at x = 79 / 81.
    for (face, used) in [(79.0, true), (81.0, false)] {
        let mut w = world();
        let origin = format!("{} 0 64", face + 4.0);
        let b = spawn_brush(
            &mut w,
            &[("classname", "func_button"), ("origin", &origin), ("spawnflags", "1025")],
            Vec3::new(-4.0, -8.0, -8.0),
            Vec3::new(4.0, 8.0, 8.0),
        );
        w.activate();
        player_at(&mut w, 0, Vec3::ZERO);
        use_at(&mut w, &[0], 2);
        assert_eq!(fired(&w, b, "OnPressed").len(), used as usize, "face at {face}");
    }
    // On the floor 30 ahead: found by an angled trace.
    let mut w = world();
    let b = spawn_brush(
        &mut w,
        &[("classname", "func_button"), ("origin", "38 0 1"), ("spawnflags", "1025")],
        Vec3::new(-8.0, -8.0, -1.0),
        Vec3::new(8.0, 8.0, 1.0),
    );
    w.activate();
    player_at(&mut w, 0, Vec3::ZERO);
    use_at(&mut w, &[0], 2);
    assert_eq!(fired(&w, b, "OnPressed").len(), 1);
}

/// +use on a brush parented to a button presses the button
/// (doors_buttons.md "+use: finding what to use": a hit entity that isn't
/// usable tries its parent, grandparent...).
#[test]
fn use_reaches_a_usable_parent() {
    let mut w = world();
    // The button itself out of reach (200 ahead), a panel parented to it
    // 60 ahead.
    let b = spawn_brush(
        &mut w,
        &[
            ("classname", "func_button"),
            ("targetname", "b"),
            ("origin", "204 0 64"),
            ("spawnflags", "1025"),
        ],
        Vec3::new(-4.0, -8.0, -8.0),
        Vec3::new(4.0, 8.0, 8.0),
    );
    spawn_brush(
        &mut w,
        &[("classname", "func_brush"), ("parentname", "b"), ("origin", "64 0 64")],
        Vec3::new(-4.0, -16.0, -16.0),
        Vec3::new(4.0, 16.0, 16.0),
    );
    w.activate();
    player_at(&mut w, 0, Vec3::ZERO);
    use_at(&mut w, &[0], 2);
    assert_eq!(fired(&w, b, "OnPressed").len(), 1);
}

/// func_water_analog moves like func_movelinear (mg_jacks_multigames_v1's
/// rising flood: movedir up, 635 units at 11 units/s).
#[test]
fn water_analog_moves() {
    let mut w = world();
    let water = spawn(
        &mut w,
        &[
            ("classname", "func_water_analog"),
            ("targetname", "water"),
            ("movedir", "-90 0 0"),
            ("movedistance", "635"),
            ("speed", "11"),
        ],
    );
    w.activate();
    w.queue_input("water", "Open", Value::Void, 0.0, None);
    run_to(&mut w, 660);
    let z = origin_of(&w, water).z;
    assert!((z - 110.0).abs() < 2.0, "10 s up at 11 units/s: {z}");
}

#[test]
fn movelinear() {
    let mut w = world();
    let m = spawn_brush(
        &mut w,
        &[("classname", "func_movelinear"), ("targetname", "m"), ("movedir", "0 0 0"), ("MoveDistance", "100"), ("speed", "100")],
        Vec3::splat(-4.0),
        Vec3::splat(4.0),
    );
    w.activate();
    w.queue_input("m", "SetPosition", Value::Str("0.5".into()), 0.0, None);
    run_to(&mut w, 40);
    assert_eq!(origin_of(&w, m).x, 50.0);
    assert!(fired(&w, m, "OnFullyOpen").is_empty() && fired(&w, m, "OnFullyClosed").is_empty());
    w.queue_input("m", "SetPosition", Value::Str("2".into()), 0.0, None);
    run_to(&mut w, 300);
    assert_eq!(origin_of(&w, m).x, 200.0);
    w.queue_input("m", "Open", Value::Void, 0.0, None);
    run_to(&mut w, 500);
    w.queue_input("m", "Close", Value::Void, 0.0, None);
    run_to(&mut w, 700);
    assert_eq!(fired(&w, m, "OnFullyOpen").len(), 1);
    assert_eq!(fired(&w, m, "OnFullyClosed").len(), 1);
}

fn spin(w: &LogicWorld, id: EntId) -> f32 {
    match &w.get(id).unwrap().class {
        Class::Rotating(r) => r.speed,
        _ => panic!(),
    }
}

#[test]
fn func_rotating() {
    let mut w = world();
    let r = spawn_brush(
        &mut w,
        &[("classname", "func_rotating"), ("targetname", "r"), ("maxspeed", "180"), ("fanfriction", "20"), ("spawnflags", "80")],
        Vec3::splat(-4.0),
        Vec3::splat(4.0),
    );
    w.activate();
    w.queue_input("r", "Start", Value::Void, 0.0, None);
    run_to(&mut w, 7);
    assert!((spin(&w, r) - 7.2).abs() < 1e-3, "{}", spin(&w, r));
    run_to(&mut w, 200);
    assert_eq!(spin(&w, r), 180.0);
    // 25 steps of 0.1 s.
    w.queue_input("r", "Stop", Value::Void, 0.0, None);
    let start = w.tick;
    while spin(&w, r) > 0.0 {
        w.frame(&NoCollision);
    }
    let secs = (w.tick - start) as f32 * DT;
    assert!((secs - 5.0).abs() < 0.1, "stopped after {secs} s");

    let mut w = world();
    let r = spawn_brush(&mut w, &[("classname", "func_rotating"), ("targetname", "r"), ("maxspeed", "100")], Vec3::splat(-4.0), Vec3::splat(4.0));
    w.activate();
    w.queue_input("r", "SetSpeed", Value::Str("-0.5".into()), 0.0, None);
    run_to(&mut w, 0);
    assert_eq!(spin(&w, r), -50.0);
    w.queue_input("r", "SetSpeed", Value::Str("-1".into()), 0.0, None);
    w.queue_input("r", "Toggle", Value::Void, 0.0, None);
    run_to(&mut w, 1);
    assert_eq!(spin(&w, r), 100.0, "negative counts as stopped; starts in its direction");
}

#[test]
fn tracktrain_on_a_straight_path() {
    let mut w = world();
    let a = spawn(&mut w, &[("classname", "path_track"), ("targetname", "a"), ("target", "b"), ("origin", "0 0 0")]);
    let b = spawn(&mut w, &[("classname", "path_track"), ("targetname", "b"), ("origin", "1000 0 0")]);
    let t = spawn_brush(
        &mut w,
        &[("classname", "func_tracktrain"), ("targetname", "t"), ("target", "a"), ("startspeed", "200"), ("height", "10")],
        Vec3::splat(-8.0),
        Vec3::splat(8.0),
    );
    w.activate();
    run_to(&mut w, 2);
    assert_eq!(origin_of(&w, t), Vec3::new(0.0, 0.0, 10.0));
    w.queue_input("t", "StartForward", Value::Void, 0.0, None);
    let mut pass_x = None;
    for _ in 0..400 {
        w.frame(&NoCollision);
        if pass_x.is_none() && !fired(&w, b, "OnPass").is_empty() {
            pass_x = Some(origin_of(&w, t).x);
        }
    }
    let _ = a;
    let x = pass_x.unwrap();
    // Fired when the look-ahead point (0.1 s, 20 units) passes B, before
    // that tick's step of 3 units.
    assert!((980.0..=986.0).contains(&x), "OnPass at x {x}");
    assert_eq!(origin_of(&w, t), Vec3::new(1000.0, 0.0, 10.0));
}

#[test]
fn riders_are_carried() {
    let mut w = world();
    let d = spawn_brush(
        &mut w,
        &[("classname", "func_door"), ("targetname", "lift"), ("movedir", "-90 0 0"), ("speed", "100"), ("wait", "-1"), ("lip", "-1000")],
        Vec3::new(-64.0, -64.0, -8.0),
        Vec3::new(64.0, 64.0, 0.0),
    );
    w.activate();
    player_at(&mut w, 0, Vec3::ZERO);
    w.players[0].ground = Some(d);
    w.queue_input("lift", "Open", Value::Void, 0.0, None);
    run_to(&mut w, 10);
    assert!((w.players[0].origin.z - 15.0).abs() < 1e-3, "{}", w.players[0].origin);
    assert_eq!(w.players[0].velocity, Vec3::ZERO);

    // A ceiling 10 units above the head blocks it.
    let ceiling = BrushCollision(vec![MapBrush::from_box(Vec3::new(-100.0, -100.0, 87.0), Vec3::new(100.0, 100.0, 120.0))]);
    run_to_with(&mut w, 30, &ceiling);
    assert!(w.players[0].origin.z <= 87.0 - 62.0 + 0.1, "{}", w.players[0].origin.z);
    assert!(origin_of(&w, d).z < 26.0);

    // A spinning platform turns its rider.
    let mut w = world();
    let r = spawn_brush(
        &mut w,
        &[("classname", "func_rotating"), ("targetname", "r"), ("maxspeed", "90"), ("spawnflags", "1")],
        Vec3::new(-64.0, -64.0, -8.0),
        Vec3::new(64.0, 64.0, 0.0),
    );
    w.activate();
    player_at(&mut w, 0, Vec3::new(32.0, 0.0, 0.0));
    w.players[0].ground = Some(r);
    run_to(&mut w, 14);
    let view = w.players[0].view.y;
    run_to(&mut w, 15);
    assert!((w.players[0].view.y - view - 1.35).abs() < 1e-3);
    let p = w.players[0].origin;
    assert!((p.truncate().length() - 32.0).abs() < 1e-2, "carried on the arc: {p}");
}

#[test]
fn func_brush_toggles() {
    let mut w = world();
    let b = spawn_brush(&mut w, &[("classname", "func_brush"), ("targetname", "wall"), ("StartDisabled", "1")], Vec3::splat(-4.0), Vec3::splat(4.0));
    w.activate();
    assert!(w.mover_solid(b).is_none());
    w.queue_input("wall", "Enable", Value::Void, 0.0, None);
    run_to(&mut w, 0);
    assert!(w.mover_solid(b).is_some());
    assert!(w.mover_poses()[0].4);
}

#[test]
fn parented_brushes_follow_their_mover() {
    let mut w = world();
    let d = spawn_brush(
        &mut w,
        &[("classname", "func_door_rotating"), ("targetname", "door"), ("distance", "90"), ("speed", "100"), ("wait", "-1")],
        Vec3::new(0.0, -2.0, 0.0),
        Vec3::new(64.0, 2.0, 100.0),
    );
    let glass = spawn_brush(
        &mut w,
        &[("classname", "func_breakable"), ("parentname", "door"), ("origin", "32 0 50")],
        Vec3::new(-8.0, -1.0, -8.0),
        Vec3::new(8.0, 1.0, 8.0),
    );
    w.activate();
    w.queue_input("door", "Open", Value::Void, 0.0, None);
    run_to(&mut w, 100);
    assert_eq!(angles_of(&w, d).y, 90.0);
    let o = origin_of(&w, glass);
    assert!((o - Vec3::new(0.0, 32.0, 50.0)).length() < 1e-3, "window at {o}");
    assert_eq!(angles_of(&w, glass).y, 90.0);
}

#[path = "restart_tests.rs"]
mod restart;

#[path = "prop_tests.rs"]
mod prop_cases;

#[path = "visual_tests.rs"]
mod visual_cases;

#[path = "fire_tests.rs"]
mod fire_cases;

#[test]
fn use_presses_say_whether_something_was_found() {
    let (mut w, _) = door_world(&[]);
    let me = w.players[0].entity;
    w.players[0].use_key = true;
    run_to(&mut w, 0);
    assert_eq!(w.use_presses, vec![(me, true)]);
    run_to(&mut w, 1);
    assert!(w.use_presses.is_empty(), "held, not pressed again");
    w.players[0].use_key = false;
    run_to(&mut w, 2);
    // Far from the door: a press that finds nothing.
    w.players[0].origin = Vec3::new(-2000.0, 0.0, 0.0);
    w.players[0].use_key = true;
    run_to(&mut w, 3);
    assert_eq!(w.use_presses, vec![(me, false)]);
}

// ------------------------------------------------- community map inputs

/// What bhop, kz and surf maps send their players (found by the map sweep,
/// docs/plans/active/community-maps.md): boosters add a base velocity once,
/// gravity and origin through AddOutput, a fall-damage filter.
#[test]
fn player_addoutput_and_damage_filter() {
    let mut w = world();
    spawn(
        &mut w,
        &[("classname", "filter_damage_type"), ("targetname", "nofall"), ("damagetype", "32"), ("Negated", "1")],
    );
    spawn(&mut w, &[("classname", "filter_activator_team"), ("targetname", "team"), ("filterteam", "2")]);
    w.activate();
    let p = player_at(&mut w, 0, Vec3::new(50.0, 50.0, 0.0));
    w.player_mut(p).unwrap().on_ground = true;
    let me = Some(Who::Player(p));
    w.deliver(Who::Player(p), "AddOutput", Value::Str("basevelocity 0 0 325".into()), me, None);
    let pl = w.player(p).unwrap();
    assert_eq!(pl.velocity, Vec3::new(0.0, 0.0, 325.0));
    assert!(pl.unground && !pl.on_ground);
    w.deliver(Who::Player(p), "AddOutput", Value::Str("gravity 0.5".into()), me, None);
    assert_eq!(w.player(p).unwrap().gravity, 0.5);
    w.deliver(Who::Player(p), "AddOutput", Value::Str("origin 10 20 30".into()), me, None);
    assert_eq!(w.player(p).unwrap().origin, Vec3::new(10.0, 20.0, 30.0));
    assert!(w.player(p).unwrap().teleported);
    w.deliver(Who::Player(p), "AddOutput", Value::Str("health 50".into()), me, None);
    w.deliver(Who::Player(p), "SetDamageFilter", Value::Str("nofall".into()), me, None);
    // Not a damage-type filter: ignored.
    w.deliver(Who::Player(p), "SetDamageFilter", Value::Str("team".into()), me, None);
    w.deliver(Who::Player(p), "SetDamageFilter", Value::Str(String::new()), me, None);
    assert_eq!(
        w.effects,
        vec![
            Effect::SetHealth { target: p, health: 50.0 },
            Effect::DamageFilter {
                target: p,
                filter: Some((32, true))
            },
            Effect::DamageFilter { target: p, filter: None },
        ]
    );
}

/// game_player_equip gives its items to whoever uses it (stripping first
/// with spawnflag 2); player_weaponstrip's Strip takes the activator's
/// weapons.
#[test]
fn equip_and_strip_the_activator() {
    let mut w = world();
    spawn(
        &mut w,
        &[
            ("classname", "game_player_equip"),
            ("targetname", "eq"),
            ("spawnflags", "3"),
            ("weapon_scout", "1"),
            ("weapon_hegrenade", "2"),
            ("item_kevlar", "1"),
        ],
    );
    spawn(&mut w, &[("classname", "player_weaponstrip"), ("targetname", "strip")]);
    w.activate();
    let p = player_at(&mut w, 0, Vec3::ZERO);
    let me = Some(Who::Player(p));
    w.queue_input("eq", "Use", Value::Void, 0.0, me);
    w.queue_input("strip", "Strip", Value::Void, 0.0, me);
    // Nobody to give to.
    w.queue_input("eq", "Use", Value::Void, 0.0, None);
    run_to(&mut w, 1);
    let items = vec![
        ("weapon_scout".to_string(), 1),
        ("weapon_hegrenade".to_string(), 2),
        ("item_kevlar".to_string(), 1),
    ];
    assert_eq!(
        w.effects,
        vec![
            Effect::Equip {
                target: p,
                items,
                strip: true
            },
            Effect::Equip {
                target: p,
                items: Vec::new(),
                strip: true
            },
        ]
    );
}

#[path = "game_tests.rs"]
mod game_cases;

#[path = "template_tests.rs"]
mod template_cases;
