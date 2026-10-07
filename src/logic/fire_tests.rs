//! Fire (specs/source/fire.md "Test cases"): env_fire growth, damage per
//! tick by size, the damage box and its line of sight, heat passing to
//! neighbours, extinguishing, the heat helpers and entity flames, on the
//! logic world (dt 0.015 s: a 0.1 s think is 7 ticks).

use std::sync::Arc;

use super::*;
use crate::core::DamageKind;
use crate::logic::fire::*;
use crate::map::entities::{PROP_EXPLODE_KEY, PROP_HEALTH_KEY, PROP_INTERACTIONS_KEY, PROP_PIECES_KEY};

/// A stock fire (every stock env_fire but its size): infinite, ignition
/// point 32, fuel 30 (ignored), attack 4, damage scale 1, not dropping
/// unless `drop`.
fn stock_fire(w: &mut LogicWorld, size: &str, origin: Vec3, flags: u32) -> EntId {
    let origin = format!("{} {} {}", origin.x, origin.y, origin.z);
    let flags = flags.to_string();
    spawn(
        w,
        &[
            ("classname", "env_fire"),
            ("targetname", "fire"),
            ("origin", &origin),
            ("firesize", size),
            ("fireattack", "4"),
            ("health", "30"),
            ("ignitionpoint", "32"),
            ("damagescale", "1.0"),
            ("spawnflags", &flags),
        ],
    )
}

fn fire_state(w: &LogicWorld, id: EntId) -> Fire {
    w.fire(id).unwrap().clone()
}

/// Run one frame; the burn damage it dealt to players.
fn step(w: &mut LogicWorld) -> Vec<(i64, Entity, f32)> {
    let tick = w.tick;
    w.frame(&NoCollision);
    let mut out = Vec::new();
    w.effects.retain(|e| match e {
        Effect::Burn { target, amount } => {
            out.push((tick, *target, *amount));
            false
        }
        _ => true,
    });
    out
}

fn floor_at(z: f32) -> Option<Arc<dyn Collision + Send + Sync>> {
    Some(Arc::new(BrushCollision(vec![MapBrush::from_box(
        Vec3::new(-4096.0, -4096.0, z - 64.0),
        Vec3::new(4096.0, 4096.0, z),
    )])))
}

fn close(a: f32, b: f32) -> bool {
    (a - b).abs() < 1e-3
}

#[test]
fn spawned_fire_is_unlit_with_its_absorb() {
    let mut w = world();
    let f = stock_fire(&mut w, "256", Vec3::new(0.0, 0.0, 104.0), SF_FIRE_INFINITE);
    w.activate();
    let s = fire_state(&w, f);
    assert_eq!(s.heat, 0.0);
    assert!(close(s.absorb, 1.6));
    assert_eq!(s.max_heat, 64.0);
    assert!(!s.lit);
    assert!(w.fire_looks().is_empty(), "no effect");
}

/// StartFire drops it, lights it, runs think 0 in the same tick; then
/// heat grows by 1.6 a think to 64 at think 40, and damage ticks every 10
/// thinks deal 1, 1, 2, 4, 7, 7 (size 256).
#[test]
fn growth_and_damage_per_size() {
    // (size, floor heat at thinks 0..40 by 10, damage at thinks 0..50 by 10)
    let cases: [(&str, [f32; 5], [f32; 6]); 3] = [
        ("256", [1.0667, 17.0667, 33.0667, 49.0667, 64.0], [1.0, 1.0, 2.0, 4.0, 7.0, 7.0]),
        ("512", [2.6667, 34.6667, 66.6667, 98.6667, 128.0], [1.0, 2.0, 7.0, 15.0, 26.0, 26.0]),
        ("128", [0.2667, 8.2667, 16.2667, 24.2667, 32.0], [1.0, 1.0, 1.0, 1.0, 2.0, 2.0]),
    ];
    for (size, heats, damage) in cases {
        let mut w = world();
        w.collision = floor_at(96.0);
        let f = stock_fire(&mut w, size, Vec3::new(864.0, 2744.0, 104.0), SF_FIRE_INFINITE);
        // A player standing right by the fire's centre, on the floor.
        let p = player_at(&mut w, 0, Vec3::new(874.0, 2744.0, 96.0));
        w.activate();
        step(&mut w);
        w.queue_input("fire", "StartFire", Value::Void, 0.0, None);
        let t0 = w.tick;
        let mut burns = step(&mut w);
        let s = fire_state(&w, f);
        assert!(s.lit);
        assert_eq!(fired(&w, f, "OnIgnited"), vec![t0]);
        let o = w.get(f).unwrap().origin;
        assert!((o.z - 96.0).abs() < 0.05, "dropped to the floor: {o}");
        assert!(close(s.heat, heats[0]), "{size} think 0: {}", s.heat);
        for k in 1..=50i64 {
            while w.tick <= t0 + 7 * k {
                burns.extend(step(&mut w));
            }
            let h = fire_state(&w, f).heat;
            if k % 10 == 0 && k <= 40 {
                assert!(close(h, heats[(k / 10) as usize]), "{size} think {k}: {h}");
            }
            if k < 40 {
                let want = heats[0] + (heats[1] - heats[0]) / 10.0 * k as f32;
                assert!(close(h, want), "{size} think {k}: {h} vs {want}");
            }
        }
        let ticks: Vec<i64> = burns.iter().map(|b| b.0).collect();
        let want_ticks: Vec<i64> = (0..6).map(|i| t0 + 70 * i).collect();
        assert_eq!(ticks, want_ticks, "{size}: a damage tick every 10 thinks (1.05 s)");
        assert!(burns.iter().all(|b| b.1 == p));
        let amounts: Vec<f32> = burns.iter().map(|b| b.2).collect();
        assert_eq!(amounts, damage.to_vec(), "{size}");
    }
}

/// A full 512 fire at de_dust2's (864, 2744) on the floor at 96: its
/// box reaches x 1120.
#[test]
fn players_in_the_damage_box() {
    for (x, hit) in [(1064.0, true), (1130.0, true), (1140.0, false)] {
        let mut w = world();
        w.collision = floor_at(96.0);
        let f = stock_fire(
            &mut w,
            "512",
            Vec3::new(864.0, 2744.0, 104.0),
            SF_FIRE_INFINITE | SF_FIRE_START_FULL,
        );
        player_at(&mut w, 0, Vec3::new(x, 2744.0, 96.0));
        w.activate();
        step(&mut w);
        w.queue_input("fire", "StartFire", Value::Void, 0.0, None);
        let burns = step(&mut w);
        assert_eq!(fire_state(&w, f).last_damage, 26);
        assert_eq!(burns.iter().map(|b| b.2).collect::<Vec<_>>(), if hit { vec![26.0] } else { vec![] }, "player at x {x}");
    }
}

/// Objects must touch the half-size box: a crate 100 in out is missed,
/// one 80 in out takes 7.
#[test]
fn objects_must_touch_the_object_box() {
    for (d, hit) in [(100.0, false), (80.0, true)] {
        let mut w = world();
        let f = stock_fire(
            &mut w,
            "256",
            Vec3::ZERO,
            SF_FIRE_INFINITE | SF_FIRE_START_FULL | SF_FIRE_DONT_DROP,
        );
        let c = spawn(
            &mut w,
            &[
                ("classname", "prop_physics_multiplayer"),
                (PROP_HEALTH_KEY, "100"),
                (PROP_PIECES_KEY, "1"),
            ],
        );
        let half = Vec3::new(40.5, 40.3, 40.2) / 2.0;
        let centre = Vec3::new(d, 0.0, half.z);
        w.set_prop_bounds(c, (centre - half, centre + half));
        w.activate();
        step(&mut w);
        w.queue_input("fire", "StartFire", Value::Void, 0.0, None);
        step(&mut w);
        assert_eq!(fire_state(&w, f).last_damage, 7);
        let want = if hit { 93 } else { 100 };
        assert_eq!(w.prop_health(c), Some((want, 100)), "crate {d} in out");
    }
}

/// The line-of-sight ray from the fire's centre: a wall between blocks
/// it, and a centre inside a brush hurts nobody.
#[test]
fn walls_block_burns() {
    let wall = MapBrush::from_box(Vec3::new(40.0, -100.0, 0.0), Vec3::new(48.0, 100.0, 400.0));
    let buried = MapBrush::from_box(Vec3::new(-300.0, -300.0, 100.0), Vec3::new(-200.0, -200.0, 200.0));
    let mut w = world();
    w.collision = Some(Arc::new(BrushCollision(vec![wall, buried])));
    let flags = SF_FIRE_INFINITE | SF_FIRE_START_FULL | SF_FIRE_DONT_DROP;
    stock_fire(&mut w, "256", Vec3::ZERO, flags);
    // Centre 128 up, at (-250, -250, 128): inside `buried`.
    let inside = spawn(
        &mut w,
        &[
            ("classname", "env_fire"),
            ("targetname", "buried"),
            ("origin", "-250 -250 0"),
            ("firesize", "256"),
            ("fireattack", "4"),
            ("damagescale", "1"),
            ("spawnflags", &flags.to_string()),
        ],
    );
    let behind = player_at(&mut w, 0, Vec3::new(80.0, 0.0, 0.0));
    let clear = player_at(&mut w, 1, Vec3::new(-80.0, 0.0, 0.0));
    let near_buried = player_at(&mut w, 2, Vec3::new(-250.0, -180.0, 0.0));
    w.activate();
    step(&mut w);
    w.queue_input("fire", "StartFire", Value::Void, 0.0, None);
    w.queue_input("buried", "StartFire", Value::Void, 0.0, None);
    let burns = step(&mut w);
    assert!(fire_state(&w, inside).lit, "it still burns");
    let hit: Vec<Entity> = burns.iter().map(|b| b.1).collect();
    assert!(hit.contains(&clear));
    assert!(!hit.contains(&behind), "the wall blocks it");
    assert!(!hit.contains(&near_buried), "rays from inside a brush hit nothing");
}

#[test]
fn a_lit_fire_lights_an_unlit_neighbour() {
    let mut w = world();
    let lit = stock_fire(
        &mut w,
        "256",
        Vec3::ZERO,
        SF_FIRE_INFINITE | SF_FIRE_START_ON | SF_FIRE_START_FULL | SF_FIRE_DONT_DROP,
    );
    let other = stock_fire(&mut w, "256", Vec3::new(50.0, 0.0, 0.0), SF_FIRE_INFINITE | SF_FIRE_DONT_DROP);
    w.activate();
    assert!(fire_state(&w, lit).lit);
    let s = fire_state(&w, other);
    assert!(s.lit, "lit by 6.4 heat");
    assert_eq!(fired(&w, other, "OnIgnited"), vec![0]);
    // 6.4 - 1.6/3 = 5.867, then its own first growth of 1.6.
    assert!(close(s.heat, 5.8667 + 1.6), "{}", s.heat);
    // Two full fires keep each other at full heat.
    run_to(&mut w, 1000);
    assert_eq!(fire_state(&w, lit).heat, 64.0);
    assert_eq!(fire_state(&w, other).heat, 64.0);
}

#[test]
fn extinguish_then_relight_burns_its_fuel() {
    let mut w = world();
    let f = stock_fire(
        &mut w,
        "256",
        Vec3::ZERO,
        // "Start on" lights it at full heat.
        SF_FIRE_INFINITE | SF_FIRE_START_ON | SF_FIRE_DONT_DROP,
    );
    player_at(&mut w, 0, Vec3::new(20.0, 0.0, 0.0));
    w.activate();
    step(&mut w);
    w.queue_input("fire", "Extinguish", Value::Str("2".into()), 0.0, None);
    let t = w.tick;
    let mut burns = Vec::new();
    while w.tick <= t + 140 {
        burns.extend(step(&mut w));
    }
    assert!(burns.is_empty(), "no damage while fading");
    assert_eq!(fired(&w, f, "OnExtinguished"), vec![t + 133]);
    let s = fire_state(&w, f);
    assert!(!s.lit);
    assert_eq!(s.heat, 0.0, "min(64 - 20, 0)");
    assert!(!w.get(f).unwrap().has_flag(SF_FIRE_INFINITE), "Extinguish clears 'infinite'");
    // Relit: 30 s of fuel, no ignition delay; out after 300 thinks and a
    // 1 s fade, then deleted.
    w.queue_input("fire", "StartFire", Value::Void, 0.0, None);
    let t1 = w.tick;
    step(&mut w);
    assert!(close(fire_state(&w, f).heat, 1.6));
    run_to(&mut w, t1 + 7 * 301 + 67 + 1);
    let out = fired(&w, f, "OnExtinguished");
    assert_eq!(out.len(), 2);
    let think = (out[1] - 67 - t1) as f32 / 7.0;
    assert!((298.0..=301.0).contains(&think), "fuel ran out at think {think}");
    assert!(w.get(f).is_none(), "a fire with fuel is deleted when out");
}

#[test]
fn extinguish_unlit_and_disable() {
    let mut w = world();
    let f = stock_fire(&mut w, "256", Vec3::ZERO, SF_FIRE_INFINITE | SF_FIRE_DONT_DROP);
    w.activate();
    w.queue_input("fire", "Extinguish", Value::Str("0".into()), 0.0, None);
    let t = w.tick;
    run_to(&mut w, t + 1);
    assert_eq!(fired(&w, f, "OnExtinguished"), vec![t + 1]);
    assert_eq!(fire_state(&w, f).heat, -20.0);

    // Start disabled: StartFire does nothing until enabled.
    let mut w = world();
    let f = spawn(
        &mut w,
        &[
            ("classname", "env_fire"),
            ("targetname", "fire"),
            ("firesize", "256"),
            ("spawnflags", "17"),
            ("StartDisabled", "1"),
        ],
    );
    w.activate();
    w.queue_input("fire", "StartFire", Value::Void, 0.0, None);
    step(&mut w);
    assert!(!fire_state(&w, f).lit);
    w.queue_input("fire", "Enable", Value::Void, 0.0, None);
    w.queue_input("fire", "StartFire", Value::Void, 0.0, None);
    step(&mut w);
    assert!(fire_state(&w, f).lit);
    // Disable while burning: out at once, no fade.
    w.queue_input("fire", "Disable", Value::Void, 0.0, None);
    let t = w.tick;
    step(&mut w);
    assert_eq!(fired(&w, f, "OnExtinguished"), vec![t]);
    assert!(fire_state(&w, f).heat <= 0.0);
}

fn gas_can(w: &mut LogicWorld, at: Vec3) -> EntId {
    let p = spawn(
        w,
        &[
            ("classname", "prop_physics_multiplayer"),
            ("targetname", "can"),
            (PROP_HEALTH_KEY, "20"),
            (PROP_INTERACTIONS_KEY, "flammable explode_fire firstimpact_break"),
            (PROP_EXPLODE_KEY, "25 80"),
        ],
    );
    let lo = Vec3::new(-4.28, -10.11, -15.11);
    let hi = Vec3::new(4.28, 10.05, 15.11);
    w.set_prop_bounds(p, (at + lo, at + hi));
    p
}

fn flames(w: &LogicWorld) -> Vec<EntId> {
    w.flame_looks().into_iter().map(|f| f.id).collect()
}

#[test]
fn ignited_gas_can_burns_down_and_breaks() {
    let mut w = world();
    let can = gas_can(&mut w, Vec3::ZERO);
    w.activate();
    step(&mut w);
    w.queue_input("can", "Ignite", Value::Void, 0.0, None);
    let t0 = w.tick;
    step(&mut w);
    assert_eq!(fired(&w, can, "OnIgnite"), vec![t0]);
    let fl = flames(&w);
    assert_eq!(fl.len(), 1);
    let flame = w.flame(fl[0]).unwrap().clone();
    assert_eq!(flame.size, 16.0, "max(16, 14.37)");
    assert!((flame.life_end - (w.now64() - 0.015 + 30.0)).abs() < 1e-3);
    assert!(w.effects.contains(&Effect::FlameStart {
        id: fl[0],
        target: Who::Ent(can),
    }));
    // Direct hits at +7, +20, +33 ... ticks: the 20th breaks it.
    run_to(&mut w, t0 + 7);
    assert_eq!(w.prop_health(can), Some((19, 20)));
    run_to(&mut w, t0 + 20);
    assert_eq!(w.prop_health(can), Some((18, 20)));
    run_to(&mut w, t0 + 7 + 18 * 13);
    assert_eq!(w.prop_health(can), Some((1, 20)));
    run_to(&mut w, t0 + 7 + 19 * 13);
    assert_eq!(fired(&w, can, "OnBreak"), vec![t0 + 7 + 19 * 13]);
    assert!(w.get(can).is_none() && w.get(fl[0]).is_none(), "the flame goes with it");
    assert!(w.effects.contains(&Effect::AmbientStop { id: fl[0] }));
    // A second Ignite on a burning can does nothing.
}

#[test]
fn burning_can_lights_a_fire_next_to_it() {
    let mut w = world();
    let f = stock_fire(&mut w, "256", Vec3::new(5.0, 0.0, 0.0), SF_FIRE_INFINITE | SF_FIRE_DONT_DROP);
    let can = gas_can(&mut w, Vec3::ZERO);
    w.activate();
    w.ignite(Who::Ent(can), 30.0);
    let t0 = w.tick;
    run_to(&mut w, t0 + 6);
    assert!(!fire_state(&w, f).lit);
    run_to(&mut w, t0 + 7);
    // 2 heat - 1.6/3 = 1.467: lit, and its first update adds 1.6.
    let s = fire_state(&w, f);
    assert!(s.lit);
    assert!(close(s.heat, 1.4667 + 1.6), "{}", s.heat);
}

#[test]
fn env_fire_burn_ignites_the_can_once() {
    let mut w = world();
    stock_fire(
        &mut w,
        "256",
        Vec3::new(0.0, 60.0, 0.0),
        SF_FIRE_INFINITE | SF_FIRE_START_FULL | SF_FIRE_DONT_DROP,
    );
    let can = gas_can(&mut w, Vec3::new(0.0, 0.0, 16.0));
    w.activate();
    step(&mut w);
    w.queue_input("fire", "StartFire", Value::Void, 0.0, None);
    step(&mut w);
    // 7 burn on 20 hp: not deadly, so it ignites (and takes the 7).
    assert_eq!(w.prop_health(can), Some((13, 20)));
    assert!(w.prop(can).unwrap().burning);
    assert_eq!(flames(&w).len(), 1);
    // The next damage tick (70 ticks on) is ignored; only the flame's
    // own 1 a think counts.
    let t = w.tick;
    run_to(&mut w, t + 69);
    let h = w.prop_health(can).unwrap().0;
    assert_eq!(h, 13 - 5, "five flame thinks");
}

#[test]
fn ignite_rules() {
    let mut w = world();
    // Not flammable: no flame, no OnIgnite.
    let c = spawn(
        &mut w,
        &[("classname", "prop_physics"), ("targetname", "crate"), (PROP_HEALTH_KEY, "30"), (PROP_PIECES_KEY, "1")],
    );
    let p = player_at(&mut w, 0, Vec3::new(500.0, 0.0, 0.0));
    w.activate();
    w.queue_input("crate", "Ignite", Value::Void, 0.0, None);
    step(&mut w);
    assert!(flames(&w).is_empty());
    assert!(fired(&w, c, "OnIgnite").is_empty());
    // A player takes one (Q3: the SDK's Ignite input): 1 burn a think.
    w.deliver(Who::Player(p), "Ignite", Value::Void, None, None);
    assert_eq!(flames(&w).len(), 1);
    let t = w.tick;
    let mut burns = Vec::new();
    while w.tick <= t + 7 + 13 {
        burns.extend(step(&mut w));
    }
    assert_eq!(burns.iter().map(|b| (b.0 - t, b.2)).collect::<Vec<_>>(), vec![(7, 1.0), (20, 1.0)]);
}

#[test]
fn a_flame_lifetime_ends_and_the_prop_stays_marked() {
    let mut w = world();
    let p = spawn(
        &mut w,
        &[
            ("classname", "prop_physics"),
            ("targetname", "barrel"),
            (PROP_HEALTH_KEY, "100"),
            (PROP_INTERACTIONS_KEY, "flammable"),
        ],
    );
    w.activate();
    step(&mut w);
    w.queue_input("barrel", "IgniteLifetime", Value::Str("2".into()), 0.0, None);
    let t0 = w.tick;
    step(&mut w);
    let fl = flames(&w)[0];
    run_to(&mut w, t0 + 7 + 10 * 13);
    assert_eq!(w.prop_health(p), Some((90, 100)), "10 direct hits");
    assert!(w.effects.contains(&Effect::AmbientStop { id: fl }), "StopBurning");
    assert!(w.get(fl).is_some());
    run_to(&mut w, t0 + 7 + 10 * 13 + 33);
    assert!(w.get(fl).is_none(), "removed 0.5 s later");
    w.queue_input("barrel", "Ignite", Value::Void, 0.0, None);
    step(&mut w);
    assert!(flames(&w).is_empty(), "props stay marked burning");
}

#[test]
fn fire_source_and_sensor() {
    let mut w = world();
    let f = stock_fire(&mut w, "256", Vec3::new(50.0, 0.0, 0.0), SF_FIRE_INFINITE | SF_FIRE_DONT_DROP);
    spawn(
        &mut w,
        &[("classname", "env_firesource"), ("fireradius", "100"), ("firedamage", "10"), ("spawnflags", "1")],
    );
    w.activate();
    step(&mut w);
    // 2.5 heat - 0.533 = 1.967, lit, then its own growth.
    let s = fire_state(&w, f);
    assert!(s.lit);
    assert!(close(s.heat, 1.9667 + 1.6), "{}", s.heat);

    let mut w = world();
    let flags = SF_FIRE_INFINITE | SF_FIRE_START_ON | SF_FIRE_START_FULL | SF_FIRE_DONT_DROP;
    stock_fire(&mut w, "256", Vec3::new(1000.0, 0.0, 0.0), flags);
    let b = stock_fire(&mut w, "256", Vec3::new(-1000.0, 0.0, 0.0), flags);
    if let Some(e) = w.get_mut(b) {
        e.targetname = "fire_b".into();
    }
    let s = spawn(
        &mut w,
        &[
            ("classname", "env_firesensor"),
            ("fireradius", "1100"),
            ("heatlevel", "100"),
            ("heattime", "2"),
            ("spawnflags", "1"),
        ],
    );
    w.activate();
    // Thinks every 0.5 s (33 ticks): the 4th reaches 2 s.
    run_to(&mut w, 33 * 4 - 1);
    assert!(fired(&w, s, "OnHeatLevelStart").is_empty());
    run_to(&mut w, 33 * 4);
    assert_eq!(fired(&w, s, "OnHeatLevelStart"), vec![33 * 4]);
    w.queue_input("fire_b", "Disable", Value::Void, 0.0, None);
    run_to(&mut w, 33 * 5);
    assert_eq!(fired(&w, s, "OnHeatLevelEnd"), vec![33 * 5]);
}

#[test]
fn round_restart_puts_fires_out() {
    let entities = vec![crate::map::MapEntity {
        keyvalues: kv(&[
            ("classname", "env_fire"),
            ("targetname", "fire"),
            ("firesize", "256"),
            ("fireattack", "4"),
            ("ignitionpoint", "32"),
            ("spawnflags", "17"),
        ]),
        hulls: Vec::new(),
        mover: false,
    }];
    let mut w = world();
    w.load_map(&entities);
    w.queue_input("fire", "StartFire", Value::Void, 0.0, None);
    run_to(&mut w, 100);
    assert_eq!(w.fire_looks().len(), 1);
    w.round_restart(&entities);
    assert!(w.fire_looks().is_empty(), "re-created unlit (spec Q4)");
    let f = w.find("fire").unwrap();
    assert!(close(fire_state(&w, f).absorb, 1.6));
    let _ = DamageKind::Burn;
}
