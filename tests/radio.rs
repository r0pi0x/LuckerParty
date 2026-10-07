//! The team radio: "Fire in the hole!" on a grenade throw, bots' calls
//! (enemy spotted, enemy down) with their cooldowns, and, with a real
//! CS:S install (skipped without one), that every call has a loaded sound
//! entry and the game's text.

use bevy::{
    ecs::message::{MessageCursor, Messages},
    prelude::*,
};
use mashup::{
    bot::{Bot, BotConfig},
    core::{Health, Radio, Team},
    games::{
        self, cs_source,
        cs_source::{
            TICK_INTERVAL,
            grenades::{DRAW_TIME, HEGRENADE},
            weapons::CsWeaponsPlugin,
        },
    },
    greybox::{self, GreyboxMapPlugin},
    harness::Sim,
    map::MapData,
    mount::config::LocalConfig,
    movement::placeholder,
    weapon::give,
};

fn dust2() -> Option<MapData> {
    let installed = LocalConfig::load()
        .ok()?
        .game_path(cs_source::GAME)
        .is_some_and(|p| p.join("cstrike").is_dir());
    if !installed {
        eprintln!("skipping: no CS:S install configured");
        return None;
    }
    Some(games::load_map("cs_source:de_dust2").expect("load de_dust2"))
}

#[test]
fn every_radio_call_has_a_loaded_sound_and_the_games_text() {
    let Some(map) = dust2() else { return };
    let radio = map.radio.as_ref().expect("CS:S has a radio");
    assert_eq!(radio.menus.len(), 3);
    for (i, m) in radio.menus.iter().enumerate() {
        assert!(!m.title.is_empty(), "menu {i} has no title");
        let calls = radio
            .commands
            .iter()
            .filter(|c| c.menu.is_some_and(|x| x.0 == i))
            .count();
        assert_eq!(m.items.len(), calls, "menu {i}: {:?}", m.items);
    }
    assert_eq!(radio.menus[2].items[0].1, "Affirmative/Roger");
    for c in &radio.commands {
        for (sound, text) in &c.variants {
            let entry = map
                .sounds
                .entry(sound)
                .unwrap_or_else(|| panic!("{}: no sound entry {sound}", c.command));
            assert!(!entry.waves.is_empty(), "{sound} has no decoded wave");
            assert!(
                !text.contains('_'),
                "{}: {text:?} is a key, not the game's text",
                c.command
            );
        }
    }
    assert_eq!(radio.get("fireinhole").unwrap().variants[0].1, "Fire in the hole!");
    assert!(radio.format.contains("(RADIO)"));
}

fn sim() -> Sim {
    let mut sim = Sim::new((GreyboxMapPlugin, CsWeaponsPlugin));
    sim.set_tick_interval(TICK_INTERVAL);
    sim
}

/// Radio calls since the cursor last read.
fn calls(sim: &Sim, cursor: &mut MessageCursor<Radio>) -> Vec<Radio> {
    let messages = sim.app.world().resource::<Messages<Radio>>();
    cursor.read(messages).cloned().collect()
}

/// Step `ticks` ticks, collecting radio calls.
fn run(sim: &mut Sim, ticks: u64, cursor: &mut MessageCursor<Radio>) -> Vec<Radio> {
    let mut out = Vec::new();
    for _ in 0..ticks {
        sim.ticks(1);
        out.extend(calls(sim, cursor));
    }
    out
}

fn throw(ignore: bool) -> usize {
    let mut sim = sim();
    if ignore {
        sim.app
            .world_mut()
            .resource_mut::<mashup::weapon::grenade::GrenadeRadio>()
            .0 = 1;
    }
    let p = sim.spawn_character(greybox::SPAWNS[0], placeholder::ID);
    sim.app.world_mut().entity_mut(p).insert(Team(2));
    sim.ticks(1);
    give(sim.app.world_mut(), p, HEGRENADE).unwrap();
    sim.seconds(DRAW_TIME as f64 + 0.05);
    let mut cursor = sim.app.world().resource::<Messages<Radio>>().get_cursor_current();
    sim.intent(p).fire = true;
    let mut heard = run(&mut sim, 5, &mut cursor);
    assert!(heard.is_empty(), "nothing said with the pin pulled: {heard:?}");
    sim.intent(p).fire = false;
    heard.extend(run(&mut sim, 20, &mut cursor));
    heard
        .iter()
        .filter(|c| c.sender == p && c.command == "fireinhole")
        .count()
}

#[test]
fn a_grenade_throw_calls_fire_in_the_hole() {
    assert_eq!(throw(false), 1);
    assert_eq!(throw(true), 0, "sv_ignoregrenaderadio 1");
}

#[test]
fn bots_call_enemy_spotted_once_and_enemy_down_after_a_kill() {
    let mut sim = sim();
    sim.app.world_mut().resource_mut::<BotConfig>().grenades = 0;
    sim.app.world_mut().resource_mut::<BotConfig>().dont_shoot = 1;
    let bot = sim.spawn_character(Vec3::new(0.0, 1.0, 12.0), placeholder::ID);
    sim.app.world_mut().entity_mut(bot).insert((Team(1), Bot::default()));
    let enemy = sim.spawn_character(Vec3::new(0.0, 1.0, 4.0), placeholder::ID);
    sim.app.world_mut().entity_mut(enemy).insert(Team(2));
    let mut cursor = sim.app.world().resource::<Messages<Radio>>().get_cursor_current();
    let heard = run(&mut sim, 60, &mut cursor);
    let said = |heard: &[Radio], what: &str| heard.iter().filter(|c| c.sender == bot && c.command == what).count();
    assert_eq!(said(&heard, "enemyspot"), 1, "{heard:?}");
    // Out of sight and back within the cooldown: quiet.
    sim.app.world_mut().get_mut::<Transform>(enemy).unwrap().translation = Vec3::new(30.0, 1.0, -30.0);
    let mut heard = run(&mut sim, 30, &mut cursor);
    sim.app.world_mut().get_mut::<Transform>(enemy).unwrap().translation = Vec3::new(0.0, 1.0, 4.0);
    heard.extend(run(&mut sim, 30, &mut cursor));
    assert_eq!(said(&heard, "enemyspot"), 0, "{heard:?}");
    // The bot kills the enemy: "Enemy down".
    sim.app.world_mut().write_message(mashup::core::Damage {
        target: enemy,
        attacker: Some(bot),
        amount: 2.0,
        point: Vec3::ZERO,
        dir: Vec3::Z,
        hitgroup: mashup::core::Hitgroup::Chest,
        kind: mashup::core::DamageKind::Bullet,
        weapon: None,
    });
    let heard = run(&mut sim, 5, &mut cursor);
    assert!(sim.app.world().get::<Health>(enemy).unwrap().current <= 0.0);
    assert_eq!(said(&heard, "enemydown"), 1, "{heard:?}");
}
