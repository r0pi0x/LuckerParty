//! Drawn-only entities (sprites, dust, switchable lights), prop skins,
//! body groups and sequences, and env_global, on the logic world.

use super::*;
use crate::logic::visuals::PartKind;

fn map_entity(pairs: &[(&str, &str)]) -> crate::map::MapEntity {
    crate::map::MapEntity {
        keyvalues: kv(pairs),
        ..Default::default()
    }
}

fn part(w: &LogicWorld, index: usize) -> Option<(PartKind, bool)> {
    w.part_states().into_iter().find(|p| p.0 == index).map(|p| (p.1, p.2))
}

#[test]
fn sprites_show_hide_toggle_and_die() {
    let entities = vec![
        // cs_assault's traffic lights: named, not "Start on".
        map_entity(&[("classname", "env_sprite"), ("targetname", "red"), ("spawnflags", "0")]),
        // cs_office's projector glow: named, "Start on".
        map_entity(&[("classname", "env_sprite"), ("targetname", "glow"), ("spawnflags", "1")]),
        // Unnamed: always shown.
        map_entity(&[("classname", "env_sprite"), ("spawnflags", "0")]),
        map_entity(&[
            ("classname", "logic_relay"),
            ("targetname", "go"),
            ("OnTrigger", "red,ShowSprite,,0,-1"),
            ("OnTrigger", "glow,Kill,,0,-1"),
            ("OnTrigger", "red,ToggleSprite,,1,-1"),
        ]),
    ];
    let mut w = world();
    w.load_map(&entities);
    assert_eq!(part(&w, 0), Some((PartKind::Sprite, false)));
    assert_eq!(part(&w, 1), Some((PartKind::Sprite, true)));
    assert_eq!(part(&w, 2), Some((PartKind::Sprite, true)));
    w.queue_input("go", "Trigger", Value::Void, 0.0, None);
    run_to(&mut w, 1);
    assert_eq!(part(&w, 0), Some((PartKind::Sprite, true)), "ShowSprite");
    assert_eq!(part(&w, 1), None, "killed: gone");
    run_to(&mut w, 70);
    assert_eq!(part(&w, 0), Some((PartKind::Sprite, false)), "ToggleSprite");
    // A round restart brings them back as they spawned.
    w.round_restart(&entities);
    assert_eq!(part(&w, 0), Some((PartKind::Sprite, false)));
    assert_eq!(part(&w, 1), Some((PartKind::Sprite, true)));
}

#[test]
fn steam_turns_on_off_and_toggles() {
    // env_steam: InitialState 1 starts it on; TurnOn, TurnOff, Toggle.
    let entities = vec![
        map_entity(&[("classname", "env_steam"), ("targetname", "steam"), ("InitialState", "0")]),
        map_entity(&[("classname", "env_steamjet"), ("InitialState", "1")]),
    ];
    let mut w = world();
    w.load_map(&entities);
    assert_eq!(part(&w, 0), Some((PartKind::Steam, false)));
    assert_eq!(part(&w, 1), Some((PartKind::Steam, true)));
    w.queue_input("steam", "TurnOn", Value::Void, 0.0, None);
    run_to(&mut w, 1);
    assert_eq!(part(&w, 0), Some((PartKind::Steam, true)));
    w.queue_input("steam", "Toggle", Value::Void, 0.0, None);
    run_to(&mut w, 2);
    assert_eq!(part(&w, 0), Some((PartKind::Steam, false)));
    w.queue_input("steam", "Toggle", Value::Void, 0.0, None);
    w.queue_input("steam", "TurnOff", Value::Void, 0.0, None);
    run_to(&mut w, 3);
    assert_eq!(part(&w, 0), Some((PartKind::Steam, false)));
}

#[test]
fn soundscapes_enable_disable_and_toggle() {
    // env_soundscape(_proxy): StartDisabled; Enable, Disable,
    // ToggleEnabled (specs/cs_source/sounds.md 6).
    let entities = vec![
        map_entity(&[("classname", "env_soundscape"), ("targetname", "a"), ("StartDisabled", "1")]),
        map_entity(&[("classname", "env_soundscape_proxy"), ("targetname", "b")]),
    ];
    let mut w = world();
    w.load_map(&entities);
    assert_eq!(part(&w, 0), Some((PartKind::Soundscape, false)));
    assert_eq!(part(&w, 1), Some((PartKind::Soundscape, true)));
    w.queue_input("a", "Enable", Value::Void, 0.0, None);
    w.queue_input("b", "Disable", Value::Void, 0.0, None);
    run_to(&mut w, 1);
    assert_eq!(part(&w, 0), Some((PartKind::Soundscape, true)));
    assert_eq!(part(&w, 1), Some((PartKind::Soundscape, false)));
    w.queue_input("a", "ToggleEnabled", Value::Void, 0.0, None);
    run_to(&mut w, 2);
    assert_eq!(part(&w, 0), Some((PartKind::Soundscape, false)));
}

#[test]
fn dust_turns_on_and_off() {
    // de_train's bomb-site dust clouds start disabled.
    let entities = vec![
        map_entity(&[
            ("classname", "func_dustcloud"),
            ("targetname", "dust"),
            ("StartDisabled", "1"),
        ]),
        map_entity(&[("classname", "func_dustmotes"), ("StartDisabled", "0")]),
    ];
    let mut w = world();
    w.load_map(&entities);
    assert_eq!(part(&w, 0), Some((PartKind::Dust, false)));
    assert_eq!(part(&w, 1), Some((PartKind::Dust, true)));
    w.queue_input("dust", "TurnOn", Value::Void, 0.0, None);
    run_to(&mut w, 1);
    assert_eq!(part(&w, 0), Some((PartKind::Dust, true)));
    w.queue_input("dust", "TurnOff", Value::Void, 0.0, None);
    run_to(&mut w, 2);
    assert_eq!(part(&w, 0), Some((PartKind::Dust, false)));
}

#[test]
fn tonemap_controller_inputs_fire_after_map_start() {
    // surf_botanica's controller setup, then a trigger changing it later.
    let entities = vec![
        map_entity(&[("classname", "env_tonemap_controller"), ("targetname", "tonemap")]),
        map_entity(&[
            ("classname", "logic_auto"),
            ("OnMapSpawn", "tonemap,SetAutoExposureMin,0.6,0,-1"),
            ("OnMapSpawn", "tonemap,SetAutoExposureMax,2,0,-1"),
            ("OnMapSpawn", "tonemap,SetBloomScale,0.15,0,-1"),
            ("OnMapSpawn", "tonemap,SetTonemapRate,1,0,-1"),
        ]),
        map_entity(&[
            ("classname", "logic_relay"),
            ("targetname", "cave"),
            ("OnTrigger", "tonemap,SetAutoExposureMax,4,0,-1"),
            ("OnTrigger", "tonemap,SetTonemapRate,0.25,1,-1"),
        ]),
    ];
    let mut w = world();
    w.load_map(&entities);
    run_to(&mut w, 30);
    let t = w.tonemap().clone();
    assert_eq!(
        (t.exposure_min, t.exposure_max, t.bloom_scale, t.rate),
        (Some(0.6), Some(2.0), Some(0.15), Some(1.0))
    );
    w.queue_input("cave", "Trigger", Value::Void, 0.0, None);
    run_to(&mut w, 32);
    assert_eq!(w.tonemap().exposure_max, Some(4.0));
    assert_eq!(w.tonemap().rate, Some(1.0), "the rate comes a second later");
    run_to(&mut w, 110);
    assert_eq!(w.tonemap().rate, Some(0.25));
    // Back to the cvars' bounds.
    w.queue_input("tonemap", "UseDefaultAutoExposure", Value::Void, 0.0, None);
    run_to(&mut w, 112);
    let t = w.tonemap().clone();
    assert!(t.default_exposure && t.exposure_min.is_none() && t.exposure_max.is_none());
    let hdr = crate::map::MapHdr {
        exposure: Some((0.6, 2.0)),
        bloom_scale: 0.15,
        rate: 1.0,
    };
    assert_eq!(t.apply(&hdr).exposure, Some(crate::map::DEFAULT_AUTO_EXPOSURE));
    // Kept over a round restart.
    w.round_restart(&entities);
    assert!(w.tonemap().default_exposure);
}

#[test]
fn lights_switch_their_style() {
    // de_prodigy: bomb-site lights share style 32 and go off when the bomb
    // explodes; fire lights (style 33) start dark and come on.
    let entities = vec![
        map_entity(&[("classname", "light"), ("targetname", "site_lights"), ("style", "32")]),
        map_entity(&[
            ("classname", "light_spot"),
            ("targetname", "site_lights"),
            ("style", "32"),
        ]),
        map_entity(&[
            ("classname", "light"),
            ("targetname", "fire_light"),
            ("style", "33"),
            ("spawnflags", "1"),
        ]),
        // Unnamed lights have no switchable style.
        map_entity(&[("classname", "light"), ("style", "0")]),
    ];
    let mut w = world();
    w.load_map(&entities);
    assert_eq!(w.light_styles(), &[(32, true), (33, false)]);
    w.queue_input("site_lights", "TurnOff", Value::Void, 0.0, None);
    w.queue_input("fire_light", "TurnOn", Value::Void, 0.0, None);
    run_to(&mut w, 1);
    assert!(!w.light_style(32) && w.light_style(33));
    w.queue_input("fire_light", "Toggle", Value::Void, 0.0, None);
    run_to(&mut w, 2);
    assert!(!w.light_style(33));
    // Re-created at a new round, as they spawned.
    w.round_restart(&entities);
    assert_eq!(w.light_styles(), &[(32, true), (33, false)]);
}

#[test]
fn prop_skin_body_and_sequences() {
    // de_nuke's computer screens and cars.
    let entities = vec![
        map_entity(&[
            ("classname", "prop_dynamic"),
            ("targetname", "computer01"),
            ("model", "models/props/de_prodigy/wall_console1.mdl"),
            ("skin", "0"),
        ]),
        map_entity(&[
            ("classname", "prop_dynamic"),
            ("targetname", "car01"),
            ("model", "models/props/de_nuke/car_nuke_animation.mdl"),
            ("DefaultAnim", "hide"),
        ]),
        map_entity(&[
            ("classname", "logic_timer"),
            ("RefireTime", "1"),
            ("OnTimer", "computer01,Skin,2,0,-1"),
            ("OnTimer", "car01,SetAnimation,run,0,-1"),
            ("OnTimer", "computer01,SetBodyGroup,1,0,-1"),
        ]),
    ];
    let mut w = world();
    let ids = w.load_map(&entities);
    let state = |w: &LogicWorld, id: EntId| w.prop_states().into_iter().find(|s| s.id == id).unwrap();
    let (screen, car) = (ids[0], ids[1]);
    assert_eq!(state(&w, screen).skin, 0);
    assert_eq!(state(&w, car).sequence, None);
    assert_eq!(state(&w, car).default_sequence, "hide");
    run_to(&mut w, 67);
    assert_eq!(state(&w, screen).skin, 2, "Skin 2 after the timer");
    assert_eq!(state(&w, screen).body_group, Some(1));
    assert_eq!(state(&w, car).sequence, Some(("run".to_string(), 1)));
    // Each SetAnimation restarts it: a new serial.
    run_to(&mut w, 134);
    assert_eq!(state(&w, car).sequence, Some(("run".to_string(), 2)));
    w.queue_input("car01", "SetDefaultAnimation", Value::Str("idle".into()), 0.0, None);
    run_to(&mut w, 135);
    assert_eq!(state(&w, car).default_sequence, "idle");
}

#[test]
fn env_global_gates_logic_auto() {
    let entities = vec![
        map_entity(&[
            ("classname", "env_global"),
            ("targetname", "g"),
            ("globalstate", "power_on"),
            ("initialstate", "1"),
            ("spawnflags", "1"),
        ]),
        map_entity(&[("classname", "math_counter"), ("targetname", "ctr")]),
        map_entity(&[
            ("classname", "logic_auto"),
            ("globalstate", "power_on"),
            ("OnMapSpawn", "ctr,Add,1,0,-1"),
        ]),
        map_entity(&[
            ("classname", "logic_auto"),
            ("globalstate", "never_set"),
            ("OnMapSpawn", "ctr,Add,100,0,-1"),
        ]),
    ];
    let mut w = world();
    let ids = w.load_map(&entities);
    run_to(&mut w, 20);
    assert_eq!(w.global("power_on"), Some(GlobalState::On));
    assert_eq!(counter(&w, ids[1]), 1.0, "only the auto whose global is on fires");
    // Turned off, it stays off through a round restart (globals outlive
    // entities), so the auto doesn't fire again.
    w.queue_input("g", "TurnOff", Value::Void, 0.0, None);
    w.queue_input("g", "AddToCounter", Value::Int(5), 0.0, None);
    run_to(&mut w, 21);
    assert_eq!(w.global("power_on"), Some(GlobalState::Off));
    assert_eq!(w.global_counter("power_on"), 5);
    let ids = w.round_restart(&entities);
    let now = w.tick;
    run_to(&mut w, now + 20);
    assert_eq!(w.global("power_on"), Some(GlobalState::Off));
    assert_eq!(counter(&w, ids[1]), 0.0);
    w.queue_input("g", "Toggle", Value::Void, 0.0, None);
    run_to(&mut w, now + 21);
    assert_eq!(w.global("power_on"), Some(GlobalState::On));
    w.queue_input("g", "Remove", Value::Void, 0.0, None);
    run_to(&mut w, now + 22);
    assert_eq!(w.global("power_on"), Some(GlobalState::Dead));
}
