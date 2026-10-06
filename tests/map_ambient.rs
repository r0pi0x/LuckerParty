//! Map ambience from ambient_generic on real CS:S maps, headless
//! (specs/cs_source/sounds.md 6, "ambient_generic"): the sounds are
//! loaded with the map, which ones start at map load and which wait for
//! their triggers, at what level and volume, from where, and what a round
//! restart does. Skipped without an install.

use bevy::prelude::*;
use mashup::{
    core::RoundRestarts,
    games::{self, cs_source, cs_source::movement::SourceMovementPlugin},
    harness::Sim,
    logic::{Effect, Logic, LogicWorld, NoCollision, Value},
    map::{LiveSounds, MapData, MapPlugin, PropEntity, SoundLevel},
    mount::config::LocalConfig,
};

fn load(name: &str) -> Option<MapData> {
    let installed = LocalConfig::load()
        .ok()?
        .game_path(cs_source::GAME)
        .is_some_and(|p| p.join("cstrike").is_dir());
    if !installed {
        eprintln!("skipping: no CS:S install configured");
        return None;
    }
    Some(games::load_map(&format!("cs_source:{name}")).expect("load map"))
}

/// Ambient starts the logic asks for: (entity's targetname, entry,
/// volume, level).
fn starts(w: &mut LogicWorld) -> Vec<(String, String, Option<f32>, Option<f32>)> {
    let effects = std::mem::take(&mut w.effects);
    effects
        .into_iter()
        .filter_map(|e| match e {
            Effect::AmbientStart {
                id,
                entry,
                volume,
                level,
                ..
            } => Some((w.get(id).unwrap().targetname.clone(), entry, volume, level)),
            _ => None,
        })
        .collect()
}

#[test]
fn de_nuke_ambients_are_loaded_and_wait_for_their_triggers() {
    let Some(map) = load("de_nuke") else { return };
    let count = map
        .entities
        .iter()
        .filter(|e| e.classname() == "ambient_generic")
        .count();
    assert_eq!(count, 9);
    // Loaded with the map: three raw waves; the steam and the alarm loop.
    let (entries, raw) = cs_source::sound::ambient_messages(&map.entities);
    assert!(entries.is_empty(), "{entries:?}");
    assert_eq!(raw.len(), 3, "{raw:?}");
    for name in &raw {
        let entry = map.sounds.entry(name).unwrap_or_else(|| panic!("{name} loaded"));
        let clip = &map.sounds.clips[entry.waves[0]];
        let looping = clip.loop_start.is_some();
        eprintln!("{name}: {} Hz, {} channels, looping {looping}", clip.rate, clip.channels);
        assert_eq!(looping, name.contains("loop"), "{name}");
    }

    // All nine start silent: steam (fire extinguisher damage), the
    // radiation alarm (bomb) and the cars (a random timer).
    let mut w = LogicWorld::new(cs_source::TICK_INTERVAL as f32);
    w.load_map(&map.entities);
    assert!(starts(&mut w).is_empty());
    // The car timer fires every 8–15 s and picks a car: car1.wav, once,
    // from car01_sound (radius 1759: level 73) or car02_sound (1471: 72).
    let mut heard = Vec::new();
    for _ in 0..(16.0 / cs_source::TICK_INTERVAL) as usize {
        w.frame(&NoCollision);
        heard.extend(starts(&mut w));
    }
    eprintln!("de_nuke in 16 s: {heard:?}");
    assert!(!heard.is_empty(), "a car went by");
    for (name, entry, volume, level) in &heard {
        assert_eq!(entry, "ambient/misc/car1.wav");
        assert_eq!(*volume, Some(1.0));
        match name.as_str() {
            "car01_sound" => assert_eq!(*level, Some(73.0)),
            "car02_sound" => assert_eq!(*level, Some(72.0)),
            other => panic!("{other}"),
        }
    }
    // The steam plays while a fire extinguisher is hit, then fades out.
    w.queue_input("steamsound1", "PlaySound", Value::Void, 0.0, None);
    w.frame(&NoCollision);
    assert_eq!(starts(&mut w).len(), 1);
}

#[test]
fn cs_office_projector_plays_and_the_radio_follows_its_prop() {
    let Some(map) = load("cs_office") else { return };
    let radio_index = map
        .entities
        .iter()
        .position(|e| e.get("targetname") == Some("radio"))
        .expect("the radio prop");
    let mut sim = Sim::new((MapPlugin::new(map), SourceMovementPlugin));
    sim.set_tick_interval(cs_source::TICK_INTERVAL);
    sim.ticks(2);

    // Only the projector starts at map load: health 5 (volume 0.5),
    // radius 256 (level 57), a looping wave.
    let playing = |sim: &Sim| {
        let mut v: Vec<_> = sim
            .app
            .world()
            .resource::<LiveSounds>()
            .sounds
            .values()
            .map(|s| (s.entry.clone(), s.volume, s.level, s.looping, s.follow, s.at))
            .collect();
        v.sort_by(|a, b| a.0.cmp(&b.0));
        v
    };
    let now = playing(&sim);
    eprintln!("cs_office at load: {now:?}");
    assert_eq!(now.len(), 1);
    let (entry, volume, level, looping, ..) = &now[0];
    assert_eq!(entry, "ambient/tones/projector.wav");
    assert_eq!((*volume, *level, *looping), (0.5, SoundLevel::Db(57.0), true));

    // The radio news (a timer's PlaySound) plays from the radio prop.
    sim.app
        .world_mut()
        .resource_mut::<Logic>()
        .world
        .queue_input("radiosound", "PlaySound", Value::Void, 0.0, None);
    sim.ticks(2);
    let radio = {
        let world = sim.app.world_mut();
        let mut q = world.query::<(Entity, &PropEntity, &GlobalTransform)>();
        q.iter(world)
            .find(|(_, p, _)| p.0 == radio_index)
            .map(|(e, _, t)| (e, t.translation()))
            .expect("radio prop node")
    };
    let now = playing(&sim);
    let news = now
        .iter()
        .find(|s| s.0 == "ambient/office/officenews.wav")
        .expect("news playing");
    assert_eq!(news.4, Some(radio.0), "follows the radio");
    assert!(news.5.unwrap().distance(radio.1) < 1e-3, "at the radio");
    assert_eq!(news.2, SoundLevel::Db(51.0), "radius 140");

    // A new round: the radio stops, the projector starts again.
    sim.app.world_mut().resource_mut::<RoundRestarts>().0 += 1;
    sim.ticks(2);
    let now = playing(&sim);
    assert_eq!(now.len(), 1, "{now:?}");
    assert_eq!(now[0].0, "ambient/tones/projector.wav");
}
