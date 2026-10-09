//! Sound and steam effects on real CS:S maps, headless: the sound entries
//! scrapes and gib bounces use are loaded with the map; de_nuke's steam
//! jets load from their env_steam keys and puff once the logic turns them
//! on; env_soundscape_proxy points copy their main soundscape; the room
//! settings soundscapes name. Skipped without an install.

use bevy::prelude::*;
use mashup::{
    games::{self, cs_source, cs_source::movement::SourceMovementPlugin},
    harness::Sim,
    logic::{Logic, Value},
    map::{EntityPart, MapData, MapPlugin, steam::SteamEmitter},
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

#[test]
fn scrape_and_gib_bounce_entries_load_with_the_map() {
    let Some(map) = load("de_nuke") else { return };
    let s = &map.sounds;
    // Gib bounces by break flag (logic::breakables::Material).
    for e in [
        "Bounce.Glass",
        "Bounce.Wood",
        "Bounce.Metal",
        "Bounce.Flesh",
        "Bounce.Concrete",
    ] {
        let entry = s.entry(e).unwrap_or_else(|| panic!("{e} not loaded"));
        assert!(!entry.waves.is_empty(), "{e} has no waves");
    }
    // Surfaces name their scrapes, and the entries are there.
    let mut scraping = 0;
    for (name, surface) in &s.surfaces {
        for e in [&surface.scrape_rough, &surface.scrape_smooth].into_iter().flatten() {
            assert!(s.entry(e).is_some(), "{name}: {e} not loaded");
            scraping += 1;
        }
    }
    assert!(scraping > 10, "{scraping} scrape entries");
    let default = s.surface("default").unwrap();
    assert!(default.scrape_rough.is_some());
    eprintln!(
        "default: rough {:?} smooth {:?} roughness {} threshold {}",
        default.scrape_rough, default.scrape_smooth, default.roughness, default.rough_threshold
    );
}

#[test]
fn de_nuke_steam_jets_puff_when_turned_on() {
    let Some(map) = load("de_nuke") else { return };
    assert!(!map.steam.is_empty(), "de_nuke's env_steam jets");
    for j in &map.steam {
        let e = &map.entities[j.entity.unwrap()];
        eprintln!(
            "{} {:?} at {:?} dir {:?}: on {} speed {:.2} m/s length {:.2} m rate {} sizes {:.3}-{:.3} m alpha {:.2} ramp {:?} texture {}",
            e.classname(),
            e.get("targetname"),
            j.origin,
            j.forward,
            j.start_on,
            j.speed,
            j.length,
            j.rate,
            j.start_size,
            j.end_size,
            j.alpha,
            j.ramp[0],
            j.texture.is_some()
        );
        assert!(j.life().is_some(), "every stock jet makes puffs");
        assert!(j.texture.is_some(), "particle/particle_smokegrenade");
        assert!(j.forward.is_normalized());
    }
    let mut sim = Sim::new((MapPlugin::new(map.clone()), SourceMovementPlugin));
    sim.set_tick_interval(cs_source::TICK_INTERVAL);
    sim.seconds(1.0);
    let counts = |sim: &mut Sim| -> Vec<(usize, bool, usize)> {
        let world = sim.app.world_mut();
        world
            .query::<(&SteamEmitter, &EntityPart)>()
            .iter(world)
            .map(|(s, p)| (p.entity, p.on, s.count()))
            .collect()
    };
    let before = counts(&mut sim);
    assert_eq!(before.len(), map.steam.len());
    for (entity, on, n) in &before {
        assert_eq!(*n > 0, *on, "entity {entity}: puffs only while on");
    }
    // Turn every jet on through the logic.
    let names: Vec<String> = map
        .steam
        .iter()
        .filter_map(|j| map.entities[j.entity.unwrap()].get("targetname").map(str::to_string))
        .collect();
    for n in &names {
        let mut logic = sim.app.world_mut().resource_mut::<Logic>();
        logic.world.queue_input(n, "TurnOn", Value::Void, 0.0, None);
    }
    sim.seconds(1.0);
    for (entity, on, n) in counts(&mut sim) {
        let named = map.entities[entity].get("targetname").is_some();
        if named {
            assert!(on && n > 0, "entity {entity}: on after TurnOn, {n} puffs");
        }
    }
    // Off again: the puffs live out their life and none follow.
    for n in &names {
        let mut logic = sim.app.world_mut().resource_mut::<Logic>();
        logic.world.queue_input(n, "TurnOff", Value::Void, 0.0, None);
    }
    sim.seconds(3.0);
    for (entity, on, n) in counts(&mut sim) {
        if map.entities[entity].get("targetname").is_some() {
            assert!(!on && n == 0, "entity {entity}: off, {n} puffs left");
        }
    }
}

#[test]
fn stock_soundscapes_proxies_and_room_settings() {
    let mut proxies = 0;
    for name in [
        "de_dust2",
        "de_nuke",
        "de_aztec",
        "de_inferno",
        "de_train",
        "de_prodigy",
        "de_piranesi",
        "de_port",
        "de_cbble",
        "de_chateau",
        "de_tides",
        "cs_office",
        "cs_italy",
        "cs_militia",
        "cs_assault",
        "cs_compound",
        "cs_havana",
    ] {
        let Some(map) = load(name) else { return };
        let s = &map.sounds;
        for e in &s.soundscape_emitters {
            let ent = &map.entities[e.entity.expect("emitters know their entity")];
            assert_eq!(
                e.start_disabled,
                ent.get("StartDisabled").is_some_and(|v| v.starts_with('1'))
            );
            if ent.classname() == "env_soundscape_proxy" {
                proxies += 1;
                let main = ent.get("MainSoundscapeName").unwrap_or("");
                let main = map
                    .entities
                    .iter()
                    .find(|m| m.classname() == "env_soundscape" && m.get("targetname") == Some(main))
                    .expect("its main soundscape");
                let scape = &s.soundscapes[e.scape];
                assert!(
                    main.get("soundscape")
                        .is_some_and(|n| n.eq_ignore_ascii_case(&scape.name)),
                    "{name}: proxy plays its main's soundscape"
                );
            }
        }
        for sc in &s.soundscapes {
            if sc.dsp_player.is_some() || sc.dsp_volume.is_some() || sc.mixer.is_some() {
                eprintln!(
                    "{name}: {} dsp_player {:?} dsp_volume {:?} mixer {:?}",
                    sc.name, sc.dsp_player, sc.dsp_volume, sc.mixer
                );
            }
        }
    }
    eprintln!("{proxies} env_soundscape_proxy entities in the stock maps");
}
