//! Prop damage and breaking on real CS:S maps, headless
//! (specs/source/prop_damage.md): cs_office's monitors change skin when
//! shot and its computer cases switch their monitor's skin when they
//! break; cs_italy's drawer breaks the radio's breakable, which stops the
//! opera; a wood crate breaks into its model's pieces; cs_militia's gas
//! can explodes and hurts who stands next to it; a crate dropped from
//! height takes impact damage; cs_office's sleeping phones and keyboards
//! stay whole until woken. Skipped without an install.

use avian3d::prelude::*;
use bevy::{
    ecs::message::{MessageCursor, Messages},
    prelude::*,
};
use mashup::{
    core::{Damage, DamageKind, Health, Hitgroup},
    games::{
        self, cs_source,
        cs_source::{
            movement::{SourceMovementPlugin, to_engine},
            weapons::CsWeaponsPlugin,
        },
    },
    harness::Sim,
    logic::{EntId, Logic, Value},
    map::{BreakProp, LiveSounds, MapData, MapPlugin, PropEntity, PropLook, SpawnGibs},
    mount::config::LocalConfig,
    movement::noclip,
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

fn sim(map: MapData) -> Sim {
    let mut sim = Sim::new((MapPlugin::new(map), SourceMovementPlugin, CsWeaponsPlugin));
    sim.set_tick_interval(cs_source::TICK_INTERVAL);
    sim.ticks(2);
    sim
}

fn model_is(map: &MapData, i: usize, model: &str) -> bool {
    map.entities[i]
        .get("model")
        .is_some_and(|m| m.eq_ignore_ascii_case(model))
}

/// Map entities whose model is `model`.
fn with_model(map: &MapData, model: &str) -> Vec<usize> {
    (0..map.entities.len()).filter(|i| model_is(map, *i, model)).collect()
}

fn id_of(sim: &Sim, index: usize) -> Option<EntId> {
    let logic = sim.app.world().resource::<Logic>();
    logic
        .world
        .ids()
        .into_iter()
        .find(|id| logic.world.get(*id).unwrap().map_index == Some(index))
}

fn node(sim: &mut Sim, index: usize) -> Entity {
    let world = sim.app.world_mut();
    world
        .query::<(Entity, &PropEntity)>()
        .iter(world)
        .find(|(_, p)| p.0 == index)
        .map(|(e, _)| e)
        .unwrap_or_else(|| panic!("no node for entity {index}"))
}

fn health(sim: &Sim, index: usize) -> Option<(i32, i32)> {
    let id = id_of(sim, index)?;
    sim.app.world().resource::<Logic>().world.prop_health(id)
}

/// A bullet of `amount` health points on prop entity `index`.
fn shoot(sim: &mut Sim, index: usize, amount: f32, attacker: Option<Entity>) {
    let target = node(sim, index);
    let point = sim.app.world().get::<Transform>(target).unwrap().translation;
    sim.app.world_mut().write_message(Damage {
        target,
        attacker,
        amount: amount / 100.0,
        point,
        dir: Vec3::NEG_Z,
        hitgroup: Hitgroup::Generic,
        kind: DamageKind::Bullet,
        weapon: None,
        force: Vec3::ZERO,
    });
    sim.ticks(1);
}

fn send(sim: &mut Sim, target: &str, input: &str) {
    sim.app
        .world_mut()
        .resource_mut::<Logic>()
        .world
        .queue_input(target, input, Value::Void, 0.0, None);
    sim.ticks(2);
}

fn skin(sim: &mut Sim, index: usize) -> i32 {
    let n = node(sim, index);
    sim.app.world().get::<PropLook>(n).map_or(0, |l| l.skin)
}

/// Every message of type `M` still held.
fn messages<M: Message + Clone>(sim: &Sim) -> Vec<M> {
    let m = sim.app.world().resource::<Messages<M>>();
    MessageCursor::<M>::default().read(m).cloned().collect()
}

#[test]
fn cs_office_monitors_and_computer_cases_change_skins() {
    let Some(map) = load("cs_office") else { return };
    // Monitors whose OnHealthChanged skins themselves.
    let monitors: Vec<usize> = (0..map.entities.len())
        .filter(|i| {
            let e = &map.entities[*i];
            let name = e.get("targetname").unwrap_or("");
            !name.is_empty()
                && e.keyvalues.iter().any(|(k, v)| {
                    k.eq_ignore_ascii_case("OnHealthChanged")
                        && v.to_ascii_lowercase()
                            .contains(&format!("{},skin", name.to_ascii_lowercase()))
                })
        })
        .collect();
    // Computer cases whose OnBreak skins a monitor.
    let cases: Vec<(usize, String)> = (0..map.entities.len())
        .filter_map(|i| {
            let v = map.entities[i]
                .keyvalues
                .iter()
                .find(|(k, v)| k.eq_ignore_ascii_case("OnBreak") && v.to_ascii_lowercase().contains(",skin,"))?
                .1
                .clone();
            Some((i, v.split([',', '\u{1b}']).next().unwrap().to_string()))
        })
        .collect();
    eprintln!("monitors {monitors:?}, cases {cases:?}");
    assert_eq!(monitors.len(), 2, "two self-skinning monitors");
    assert_eq!(cases.len(), 2, "two computer cases");
    let mut sim = sim(map.clone());
    let m = monitors[0];
    assert_eq!(health(&sim, m), Some((10, 10)), "Plastic.break");
    assert_eq!(skin(&mut sim, m), 0);
    shoot(&mut sim, m, 5.0, None);
    sim.ticks(2);
    assert_eq!(health(&sim, m), Some((5, 10)));
    assert_eq!(skin(&mut sim, m), 1, "OnHealthChanged -> Skin 1");
    // A case breaks: its monitor shows skin 1.
    let shot = map.entities[m].get("targetname").unwrap().to_ascii_lowercase();
    let (case, target) = cases
        .iter()
        .find(|(_, t)| t.to_ascii_lowercase() != shot)
        .expect("another monitor's case");
    let monitor = map
        .entities
        .iter()
        .position(|e| e.get("targetname").is_some_and(|n| n.eq_ignore_ascii_case(target)))
        .expect("the case's monitor");
    assert_eq!(skin(&mut sim, monitor), 0);
    shoot(&mut sim, *case, 500.0, None);
    sim.ticks(3);
    assert!(id_of(&sim, *case).is_none(), "the case broke");
    assert_eq!(skin(&mut sim, monitor), 1);
}

#[test]
fn cs_office_sleeping_props_break_only_when_woken() {
    let Some(map) = load("cs_office") else { return };
    let sleepers: Vec<usize> = (0..map.entities.len())
        .filter(|i| {
            map.entities[*i]
                .keyvalues
                .iter()
                .any(|(k, _)| k.eq_ignore_ascii_case("OnAwakened"))
        })
        .collect();
    assert!(sleepers.len() >= 20, "{}", sleepers.len());
    let mut sim = sim(map.clone());
    sim.seconds(3.0);
    let whole = sleepers.iter().filter(|i| id_of(&sim, **i).is_some()).count();
    assert_eq!(whole, sleepers.len(), "nothing wakes them by itself");
    // Wake one: it breaks.
    let one = sleepers[0];
    let name = map.entities[one].get("targetname").unwrap().to_string();
    send(&mut sim, &name, "Wake");
    sim.ticks(4);
    assert!(id_of(&sim, one).is_none(), "{name} broke when woken");
}

#[test]
fn cs_italy_drawer_breaks_the_radio() {
    let Some(map) = load("cs_italy") else { return };
    let drawer = (0..map.entities.len())
        .find(|i| {
            map.entities[*i]
                .keyvalues
                .iter()
                .any(|(k, v)| k.eq_ignore_ascii_case("OnBreak") && v.to_ascii_lowercase().starts_with("killradio"))
        })
        .expect("the drawer");
    let radio = map
        .entities
        .iter()
        .position(|e| e.get("targetname") == Some("killradio"))
        .expect("killradio");
    let opera = map
        .entities
        .iter()
        .find(|e| e.get("targetname").is_some_and(|n| n.eq_ignore_ascii_case("operaWav")))
        .and_then(|e| e.get("message"))
        .expect("the opera")
        .to_ascii_lowercase();
    let mut sim = sim(map);
    let playing = |sim: &Sim| {
        sim.app
            .world()
            .resource::<LiveSounds>()
            .sounds
            .values()
            .any(|s| s.entry.eq_ignore_ascii_case(&opera))
    };
    assert!(playing(&sim), "the opera plays");
    assert_eq!(health(&sim, drawer), Some((30, 30)), "Wooden.Medium");
    // 20 x 0.75 = 15, twice.
    shoot(&mut sim, drawer, 20.0, None);
    assert_eq!(health(&sim, drawer), Some((15, 30)));
    shoot(&mut sim, drawer, 20.0, None);
    sim.ticks(3);
    assert!(id_of(&sim, drawer).is_none(), "the drawer broke");
    sim.seconds(1.5);
    assert!(id_of(&sim, radio).is_none(), "and the radio's breakable");
    assert!(!playing(&sim), "the opera stopped");
}

/// A wood crate's entity on some map: (map, index).
fn a_wood_crate() -> Option<(MapData, usize)> {
    for name in ["de_port", "cs_militia", "de_chateau", "cs_compound", "de_train"] {
        let map = load(name)?;
        let found = (0..map.entities.len()).find(|i| {
            let e = &map.entities[*i];
            e.get("model")
                .is_some_and(|m| m.to_ascii_lowercase().contains("wood_crate001a"))
                && e.classname().starts_with("prop_physics")
                // Neither asleep nor frozen: it falls when lifted.
                && e.get("spawnflags")
                    .and_then(|s| s.parse::<u32>().ok())
                    .is_some_and(|f| f & 9 == 0)
        });
        if let Some(i) = found {
            eprintln!("{name}: crate {i} at {}", map.entities[i].origin());
            return Some((map, i));
        }
    }
    panic!("no loose wood_crate001a found");
}

#[test]
fn wood_crate_breaks_into_its_pieces() {
    let Some((map, crate_)) = a_wood_crate() else { return };
    assert_eq!(
        map.entities[crate_].get(mashup::map::entities::PROP_PIECES_KEY),
        Some("7"),
        "wood_crate001a's .phy break pieces"
    );
    let mut sim = sim(map);
    assert_eq!(health(&sim, crate_), Some((30, 30)));
    shoot(&mut sim, crate_, 36.0, None);
    assert_eq!(health(&sim, crate_), Some((3, 30)));
    shoot(&mut sim, crate_, 36.0, None);
    sim.ticks(1);
    assert!(id_of(&sim, crate_).is_none(), "broken and removed");
    let breaks = messages::<BreakProp>(&sim);
    assert_eq!(breaks.len(), 1);
    let gibs = messages::<SpawnGibs>(&sim);
    let pieces: usize = gibs.iter().filter(|g| g.prop).map(|g| g.pieces.len()).sum();
    assert_eq!(pieces, 7, "7 pieces");
    // Flying outward at 100 units/s on top of the crate's velocity.
    for g in gibs.iter().filter(|g| g.prop).flat_map(|g| &g.pieces) {
        let speed = g.velocity.length() / 0.0254;
        assert!((90.0..115.0).contains(&speed), "{speed}");
    }
    // Its node no longer blocks.
    let n = node(&mut sim, crate_);
    assert!(sim.app.world().get::<ColliderDisabled>(n).is_some());
}

#[test]
fn cs_militia_gas_can_explodes() {
    let Some(map) = load("cs_militia") else { return };
    let cans = with_model(&map, "models/props_junk/gascan001a.mdl");
    assert_eq!(cans.len(), 1, "one gas can");
    let can = cans[0];
    let e = &map.entities[can];
    eprintln!("gas can: {:?}", e.keyvalues);
    assert_eq!(e.get(mashup::map::entities::PROP_EXPLODE_KEY), Some("25 80"));
    let origin = e.origin();
    let mut sim = sim(map);
    let (h, _) = health(&sim, can).expect("a gas can");
    assert!(h > 0);
    // Someone standing 40 units away.
    let victim = sim.spawn_character(to_engine(origin + Vec3::new(40.0, 0.0, 0.0)), noclip::ID);
    sim.ticks(2);
    let before = sim.app.world().get::<Health>(victim).unwrap().current;
    shoot(&mut sim, can, 500.0, None);
    sim.ticks(3);
    assert!(id_of(&sim, can).is_none(), "it broke");
    let after = sim.app.world().get::<Health>(victim).unwrap().current;
    eprintln!("victim health {before} -> {after}");
    assert!(after < before, "the blast hurt the victim");
    assert!(before - after <= 0.26, "at most the can's 25");
}

#[test]
fn dropped_crate_takes_impact_damage() {
    let Some((map, crate_)) = a_wood_crate() else { return };
    let mut sim = sim(map);
    sim.seconds(1.0);
    assert_eq!(health(&sim, crate_), Some((30, 30)), "settling does no damage");
    // de_port's loose crate has physdamagescale 1. Lift it 60 units and
    // let it fall: about 310 units/s at the floor, E' = 96000: 10.
    let n = node(&mut sim, crate_);
    let up = Vec3::Y * 60.0 * 0.0254;
    {
        let world = sim.app.world_mut();
        let mut e = world.entity_mut(n);
        e.get_mut::<Position>().unwrap().0 += up;
        e.get_mut::<Transform>().unwrap().translation += up;
        e.get_mut::<LinearVelocity>().unwrap().0 = Vec3::ZERO;
        e.remove::<Sleeping>();
    }
    sim.seconds(2.0);
    let (h, _) = health(&sim, crate_).expect("still there");
    eprintln!("crate health after the drop: {h}");
    assert!(h < 30, "the landing hurt it");
    // 10 by the default table for a clean landing on the floor (unit
    // tests); its neighbours here take some of it.
    assert!((20..30).contains(&h), "{h}");
}

#[test]
fn cs_militia_roof_boards_break_under_a_player() {
    let Some(map) = load("cs_militia") else { return };
    let boards: Vec<usize> = (0..map.entities.len())
        .filter(|i| {
            let e = &map.entities[*i];
            e.classname().starts_with("prop_physics")
                && e.get("spawnflags")
                    .and_then(|f| f.parse::<u32>().ok())
                    .is_some_and(|f| f & 32 != 0)
        })
        .collect();
    assert_eq!(boards.len(), 1, "the pressure roof boards");
    let board = boards[0];
    let prop = map.props.iter().find(|p| p.entity == Some(board)).expect("its prop");
    let (lo, hi) = map.models[prop.model].bounds;
    let corners: Vec<Vec3> = (0..8)
        .map(|k| {
            let c = Vec3::new(
                if k & 1 == 0 { lo.x } else { hi.x },
                if k & 2 == 0 { lo.y } else { hi.y },
                if k & 4 == 0 { lo.z } else { hi.z },
            );
            cs_source::movement::to_source(prop.translation + prop.rotation * c)
        })
        .collect();
    let (a, b) = corners
        .iter()
        .fold((Vec3::MAX, Vec3::MIN), |(a, b), c| (a.min(*c), b.max(*c)));
    eprintln!("board bounds {a} .. {b}");
    let origin = ((a + b) / 2.0).with_z(b.z);
    let mut sim = sim(map);
    assert!(health(&sim, board).is_some());
    // Dropped onto it from a little above.
    let p = sim.spawn_character(to_engine(origin + Vec3::new(0.0, 0.0, 40.0)), cs_source::movement::ID);
    sim.seconds(0.5);
    let on = sim.state(p).ground;
    eprintln!(
        "player at {:?}, standing on {on:?} (board {:?})",
        cs_source::movement::to_source(sim.position(p)),
        node(&mut sim, board)
    );
    sim.seconds(1.5);
    assert!(id_of(&sim, board).is_none(), "the boards broke under the player");
}
