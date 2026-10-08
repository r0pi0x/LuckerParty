//! Fire on de_dust2 (specs/source/fire.md test cases): the bomb blows at
//! site A, its `BombExplode` lights the nine `firea` fires 0.2 s later
//! (eight at A, one at B) and leaves `fireb` dark; a player standing in
//! the 512 fire at (864, 2744) takes 26 burn a second once it is full.
//! Skipped without a CS:S install.

use bevy::prelude::*;
use mashup::{
    console::Console,
    core::{Health, Team},
    games::{
        self, cs_source,
        cs_source::{
            TICK_INTERVAL,
            movement::{self, SourceMovementPlugin, to_engine},
            objectives::C4,
            weapons::CsWeaponsPlugin,
        },
    },
    harness::Sim,
    logic::Logic,
    map::{MapData, MapPlugin, fire::MapFires},
    mount::config::LocalConfig,
    objectives::bomb::{BombOutcome, BombState},
    weapon::give,
};

const UNIT: f32 = 0.0254;
/// The standing hull's centre above the feet, m (36 units).
const LIFT: f32 = 36.0 * UNIT;

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

fn character(sim: &mut Sim, feet_units: Vec3, team: u8) -> Entity {
    let e = sim.spawn_character(to_engine(feet_units) + Vec3::Y * (LIFT + 0.05), movement::ID);
    sim.app.world_mut().entity_mut(e).insert(Team(team));
    e
}

/// Lit fires by name, with where they burn (entity space).
fn lit(sim: &Sim, name: &str) -> Vec<Vec3> {
    let logic = sim.app.world().resource::<Logic>();
    logic
        .world
        .fire_looks()
        .into_iter()
        .filter(|f| logic.world.get(f.id).is_some_and(|e| e.targetname.eq_ignore_ascii_case(name)))
        .map(|f| f.at)
        .collect()
}

#[test]
fn dust2_a_bomb_lights_the_a_fires_and_they_burn() {
    let Some(map) = load("de_dust2") else { return };
    let mut sim = Sim::new((MapPlugin::new(map), SourceMovementPlugin, CsWeaponsPlugin));
    sim.set_tick_interval(TICK_INTERVAL);
    sim.ticks(2);
    // Plant at A (as tests/it/heavy/map_objectives.rs) with a 10 s timer.
    let t = character(&mut sim, Vec3::new(1160.0, 2480.0, 100.0), 1);
    sim.seconds(0.5);
    give(sim.app.world_mut(), t, C4).unwrap();
    sim.seconds(1.2);
    sim.app.world_mut().resource_mut::<Console>().submit("mp_c4timer 10");
    sim.ticks(1);
    sim.intent(t).fire = true;
    sim.seconds(3.1);
    sim.intent(t).fire = false;
    sim.app.world_mut().resource_mut::<Logic>().world.record = true;
    assert!(lit(&sim, "firea").is_empty() && lit(&sim, "fireb").is_empty(), "dark before");
    // Run to the explosion.
    let mut exploded = None;
    for _ in 0..1000 {
        sim.ticks(1);
        let logic = sim.app.world().resource::<Logic>();
        if let Some((tick, ..)) = logic.world.fired.iter().find(|(_, _, o)| o == "BombExplode") {
            exploded = Some(*tick);
            break;
        }
    }
    let exploded = exploded.expect("the bomb blew");
    assert_eq!(
        sim.app.world().resource::<BombState>().outcome,
        Some(BombOutcome::Exploded)
    );
    // StartFire 0.2 s later (14 ticks; the bomb fires its output after
    // this tick's queue, so it is delivered on the 15th tick from here).
    let mut waited = 0;
    while lit(&sim, "firea").is_empty() && waited < 100 {
        sim.ticks(1);
        waited += 1;
    }
    assert_eq!(waited, 15, "ticks from the explosion to the fires");
    let _ = exploded;
    let a = lit(&sim, "firea");
    assert_eq!(a.len(), 9, "nine firea fires");
    assert!(lit(&sim, "fireb").is_empty(), "site B's stay dark");
    assert!(
        a.iter().any(|p| (p.x + 1988.0).abs() < 1.0 && (p.y - 1704.0).abs() < 1.0),
        "one firea burns at B (quirk)"
    );
    // The 512 fire at (864, 2744) landed on the floor at 96.
    let big = a
        .iter()
        .find(|p| (p.x - 864.0).abs() < 1.0 && (p.y - 2744.0).abs() < 1.0)
        .expect("the 512 fire");
    assert!((big.z - 96.0).abs() < 1.0, "on the floor: {big}");
    assert_eq!(sim.app.world().resource::<MapFires>().fires.len(), 9, "drawn");

    // Full after 4.2 s; a CT standing at (1064, 2744) then takes 26 a
    // damage tick, every 1.05 s.
    sim.seconds(4.5);
    let ct = character(&mut sim, Vec3::new(1064.0, 2744.0, 100.0), 2);
    let health = |sim: &Sim| sim.app.world().get::<Health>(ct).unwrap().current;
    let mut drops = Vec::new();
    let mut last = health(&sim);
    for _ in 0..(2.2 / TICK_INTERVAL) as usize {
        sim.ticks(1);
        let h = health(&sim);
        if h < last {
            drops.push(((last - h) * 100.0).round());
        }
        last = h;
    }
    assert_eq!(drops, vec![26.0, 26.0], "burn per damage tick");
}
