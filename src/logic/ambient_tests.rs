//! ambient_generic cases from specs/cs_source/sounds.md ("Test cases"
//! and the ambient_generic rules). dt = 0.05 s, so 0.1 s is 2 ticks and
//! a 0.2 s step 4.

use bevy::prelude::*;

use super::*;
use crate::logic::world::{Effect, EntId, LogicWorld, NoCollision};

const DT: f32 = 0.05;

fn kv(pairs: &[(&str, &str)]) -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = vec![
        ("classname".into(), "ambient_generic".into()),
        ("origin".into(), "10 20 30".into()),
        ("message".into(), "ambient/test_loop.wav".into()),
        ("radius".into(), "1250".into()),
        ("health".into(), "10".into()),
        ("pitch".into(), "100".into()),
        ("pitchstart".into(), "100".into()),
    ];
    for (k, v) in pairs {
        match out.iter_mut().find(|(o, _)| o.eq_ignore_ascii_case(k)) {
            Some(old) => old.1 = v.to_string(),
            None => out.push((k.to_string(), v.to_string())),
        }
    }
    out
}

/// A world with one ambient_generic (activated, as at map load).
fn world(pairs: &[(&str, &str)]) -> (LogicWorld, EntId) {
    let mut w = LogicWorld::new(DT);
    let id = w.spawn(&kv(pairs), Vec::new());
    w.activate();
    (w, id)
}

fn frames(w: &mut LogicWorld, n: usize) {
    for _ in 0..n {
        w.frame(&NoCollision);
    }
}

fn input(w: &mut LogicWorld, id: EntId, name: &str, value: &str) {
    let v = if value.is_empty() {
        crate::logic::Value::Void
    } else {
        crate::logic::Value::Str(value.into())
    };
    w.deliver(crate::logic::Who::Ent(id), name, v, None, None);
}

/// The ambient effects since the last call.
fn sounds(w: &mut LogicWorld) -> Vec<Effect> {
    std::mem::take(&mut w.effects)
        .into_iter()
        .filter(|e| {
            matches!(
                e,
                Effect::AmbientStart { .. } | Effect::AmbientChange { .. } | Effect::AmbientStop { .. }
            )
        })
        .collect()
}

fn state(w: &LogicWorld, id: EntId) -> &Ambient {
    match &w.get(id).unwrap().class {
        crate::logic::classes::Class::Ambient(a) => a,
        other => panic!("not an ambient: {other:?}"),
    }
}

fn volumes(effects: &[Effect]) -> Vec<f32> {
    effects
        .iter()
        .filter_map(|e| match e {
            Effect::AmbientChange { volume, .. } => *volume,
            _ => None,
        })
        .collect()
}

#[test]
fn level_from_radius() {
    let cases = [(36.0, 40.0), (360.0, 60.0), (1250.0, 70.0), (3600.0, 80.0), (10000.0, 88.0)];
    for (r, l) in cases {
        assert_eq!(radius_level(r, false), l, "radius {r}");
    }
    assert_eq!(radius_level(1250.0, true), 0.0, "play everywhere");
    assert_eq!(radius_level(0.0, false), 0.0, "no radius");
    let (w, id) = world(&[("radius", "256")]);
    assert_eq!(state(&w, id).level, 57.0);
}

#[test]
fn looping_sound_starts_at_map_load_with_the_entitys_volume_and_pitch() {
    let (mut w, id) = world(&[("health", "5"), ("pitch", "120")]);
    assert_eq!(
        sounds(&mut w),
        vec![Effect::AmbientStart {
            id,
            entry: "ambient/test_loop.wav".into(),
            at: Vec3::new(10.0, 20.0, 30.0),
            source: None,
            volume: Some(0.5),
            pitch: Some(120.0),
            level: Some(70.0),
        }]
    );
    assert!(state(&w, id).active && state(&w, id).playing);
}

#[test]
fn start_silent_not_looping_and_silent_ones_wait() {
    for flags in ["16", "32", "48"] {
        let (mut w, _) = world(&[("spawnflags", flags)]);
        assert!(sounds(&mut w).is_empty(), "spawnflags {flags}");
    }
    let (mut w, _) = world(&[("health", "0")]);
    assert!(sounds(&mut w).is_empty(), "volume 0");
    let (w, id) = world(&[("message", "")]);
    assert!(w.get(id).is_none_or(|e| e.killed), "no message: removed");
}

#[test]
fn script_entries_use_their_own_level_and_on_input_their_own_volume() {
    let (mut w, id) = world(&[("message", "fire_medium"), ("health", "5")]);
    match &sounds(&mut w)[..] {
        [Effect::AmbientStart { volume, level, .. }] => {
            assert_eq!((*volume, *level), (Some(0.5), None), "map start: the entity's volume");
        }
        other => panic!("{other:?}"),
    }
    input(&mut w, id, "StopSound", "");
    input(&mut w, id, "PlaySound", "");
    let e = sounds(&mut w);
    assert!(matches!(e[0], Effect::AmbientStop { .. }));
    match &e[1] {
        Effect::AmbientStart {
            volume, pitch, level, ..
        } => assert_eq!((*volume, *pitch, *level), (None, None, None), "input: the entry's own"),
        other => panic!("{other:?}"),
    }
}

#[test]
fn one_shots_restart_on_each_play_and_ignore_stop() {
    let (mut w, id) = world(&[("spawnflags", "48"), ("message", "ambient/office/coinslot1.wav")]);
    input(&mut w, id, "PlaySound", "");
    input(&mut w, id, "PlaySound", "");
    let starts = sounds(&mut w)
        .iter()
        .filter(|e| matches!(e, Effect::AmbientStart { .. }))
        .count();
    assert_eq!(starts, 2);
    input(&mut w, id, "StopSound", "");
    assert!(sounds(&mut w).is_empty(), "StopSound only stops an active (looping) sound");
}

#[test]
fn toggle_and_stop_a_loop() {
    let (mut w, id) = world(&[("spawnflags", "16")]);
    input(&mut w, id, "ToggleSound", "");
    assert!(matches!(sounds(&mut w)[..], [Effect::AmbientStart { .. }]));
    input(&mut w, id, "PlaySound", "");
    assert!(sounds(&mut w).is_empty(), "already on");
    input(&mut w, id, "ToggleSound", "");
    assert_eq!(sounds(&mut w), vec![Effect::AmbientStop { id }]);
    assert!(!state(&w, id).active);
}

#[test]
fn fade_in_over_two_seconds() {
    // fadeinsecs 2 (rate 2560), health 10, volstart 0; PlaySound at t = 0.
    let (mut w, id) = world(&[("spawnflags", "16"), ("fadeinsecs", "2"), ("volstart", "0")]);
    assert_eq!(state(&w, id).m.fadein, 2560);
    input(&mut w, id, "PlaySound", "");
    match &sounds(&mut w)[..] {
        [Effect::AmbientStart { volume, .. }] => assert_eq!(*volume, Some(0.0), "raw: starts at volume 0"),
        other => panic!("{other:?}"),
    }
    // Steps at t = 0.1, 0.3, ... (ticks 2, 6, 10, ...).
    frames(&mut w, 1);
    assert!(sounds(&mut w).is_empty());
    frames(&mut w, 2);
    assert_eq!(volumes(&sounds(&mut w)), vec![0.1]);
    frames(&mut w, 4 * 9);
    let v = volumes(&sounds(&mut w));
    assert_eq!(v.len(), 9);
    for (k, v) in v.iter().enumerate() {
        assert!((v - 0.1 * (k + 2) as f32).abs() < 1e-6, "step {}: {v}", k + 2);
    }
    // Step 11 ends the fade (clamped at 1.0); nothing after.
    frames(&mut w, 4);
    assert_eq!(volumes(&sounds(&mut w)), vec![1.0]);
    assert_eq!(state(&w, id).m.fade, 0);
    frames(&mut w, 40);
    assert!(sounds(&mut w).is_empty());
}

#[test]
fn volume_and_pitch_inputs() {
    let (mut w, id) = world(&[]);
    sounds(&mut w);
    for (x, want) in [("5.5", 0.55), ("0.55", 0.06), ("12", 1.0)] {
        input(&mut w, id, "Volume", x);
        assert_eq!(volumes(&sounds(&mut w)), vec![want], "Volume {x}");
    }
    for (x, want) in [("300", 255), ("-5", 0)] {
        input(&mut w, id, "Pitch", x);
        assert_eq!(state(&w, id).m.pitch, want, "Pitch {x}");
        sounds(&mut w);
    }
}

#[test]
fn spin_up_from_pitchstart() {
    // spinup 1 (rate 6400 = 25 per step), pitchstart 50, pitch 100.
    let (mut w, id) = world(&[("spawnflags", "16"), ("spinup", "1"), ("pitchstart", "50")]);
    input(&mut w, id, "PlaySound", "");
    match &sounds(&mut w)[..] {
        [Effect::AmbientStart { pitch, .. }] => assert_eq!(*pitch, Some(50.0)),
        other => panic!("{other:?}"),
    }
    let mut pitches = Vec::new();
    for _ in 0..3 {
        frames(&mut w, if pitches.is_empty() { 3 } else { 4 });
        pitches.push(state(&w, id).m.pitch);
        sounds(&mut w);
    }
    assert_eq!(pitches, [75, 100, 100], "the third step clamps and ends the spin-up");
    assert_eq!(state(&w, id).m.spin, 0);
    frames(&mut w, 20);
    assert!(sounds(&mut w).is_empty());
}

#[test]
fn legacy_fade_out_on_stop() {
    // de_nuke's steam: fadeout 1 (rate 6400 = 25 per step) from volume
    // 100: 75, 50, 25, (0 →) 1, then below volstart 0: stopped.
    let (mut w, id) = world(&[("spawnflags", "16"), ("fadeout", "1")]);
    input(&mut w, id, "PlaySound", "");
    sounds(&mut w);
    input(&mut w, id, "StopSound", "");
    assert!(sounds(&mut w).is_empty(), "fades first");
    frames(&mut w, 3 + 4 * 4);
    let e = sounds(&mut w);
    assert_eq!(volumes(&e), vec![0.75, 0.5, 0.25, 0.01]);
    assert_eq!(e.last(), Some(&Effect::AmbientStop { id }));
    assert!(!state(&w, id).playing);
}

#[test]
fn fade_out_input() {
    let (mut w, id) = world(&[]);
    sounds(&mut w);
    input(&mut w, id, "FadeOut", "1");
    // 1 s: 20 volume per step; 100 → 80, 60, 40, 20, 0 (→ 1), stop.
    frames(&mut w, 3 + 4 * 5);
    let e = sounds(&mut w);
    assert_eq!(volumes(&e), vec![0.8, 0.6, 0.4, 0.2, 0.01]);
    assert_eq!(e.last(), Some(&Effect::AmbientStop { id }));
}

#[test]
fn triangle_lfo_moves_the_volume() {
    // lforate 64: the LFO position moves 64 per step and bounces at 255.
    let (mut w, id) = world(&[("health", "5"), ("lfotype", "2"), ("lforate", "64"), ("lfomodvol", "10")]);
    sounds(&mut w);
    frames(&mut w, 3 + 4 * 4);
    let v: Vec<i32> = volumes(&sounds(&mut w)).iter().map(|v| (v * 100.0).round() as i32).collect();
    // Positions 64, 128, 192, 255 (bounce), 191: volume += (pos − 128)/10.
    assert_eq!(v, vec![50 - 6, 44, 44 + 6, 50 + 12, 62 + 6]);
    assert_eq!(state(&w, id).m.lfo_rate, -64 * 256, "falling after the bounce");
}

#[test]
fn source_entity_and_removal() {
    let mut w = LogicWorld::new(DT);
    let radio = w.spawn(
        &[("classname".into(), "prop_physics_multiplayer".into()), ("targetname".into(), "radio".into())],
        Vec::new(),
    );
    let id = w.spawn(&kv(&[("SourceEntityName", "radio")]), Vec::new());
    w.activate();
    match &sounds(&mut w)[..] {
        [Effect::AmbientStart { source, .. }] => assert_eq!(*source, Some(radio)),
        other => panic!("{other:?}"),
    }
    input(&mut w, id, "Kill", "");
    frames(&mut w, 1);
    assert_eq!(sounds(&mut w), vec![Effect::AmbientStop { id }]);
}

#[test]
fn round_restart_starts_it_again() {
    let mut w = LogicWorld::new(DT);
    let entities = vec![crate::map::MapEntity {
        keyvalues: kv(&[]),
        hulls: Vec::new(),
        mover: false,
    }];
    w.load_map(&entities);
    assert_eq!(sounds(&mut w).len(), 1);
    let ids = w.round_restart(&entities);
    let e = sounds(&mut w);
    assert!(
        matches!(&e[..], [Effect::AmbientStart { id, .. }] if *id == ids[0]),
        "{e:?}"
    );
}
