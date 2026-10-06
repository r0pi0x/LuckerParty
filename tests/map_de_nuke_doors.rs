//! de_nuke's doors from a real CS:S install, headless
//! (specs/source/doors_buttons.md, "func_door_rotating"): brush doors in
//! pairs linked by chainstodoor (spawnflags 1280, distance 90, speed 200,
//! wait 4); using one opens both 90 degrees in 0.45 s (30 ticks), and they
//! close after their wait; a round restart closes them. Skipped without
//! an install.

use bevy::prelude::*;
use mashup::{
    games::{
        self, cs_source,
        cs_source::movement::{self, SourceMovementPlugin, to_engine},
    },
    core::RoundRestarts,
    harness::Sim,
    logic::{Logic, movers::pusher},
    map::{MapData, MapEntity, MapPlugin},
    mount::config::LocalConfig,
};

fn nuke() -> Option<MapData> {
    let installed = LocalConfig::load()
        .ok()?
        .game_path(cs_source::GAME)
        .is_some_and(|p| p.join("cstrike").is_dir());
    if !installed {
        eprintln!("skipping: no CS:S install configured");
        return None;
    }
    Some(games::load_map("cs_source:de_nuke").expect("load de_nuke"))
}

/// The door's world bounds (Source units) where it spawns.
fn bounds(e: &MapEntity) -> (Vec3, Vec3) {
    let rot = mashup::map::entities::entity_rotation(e.angles());
    let pts = e.hulls.iter().flat_map(|h| &h.points).map(|p| rot * *p + e.origin());
    pts.fold((Vec3::MAX, Vec3::MIN), |(lo, hi), p| (lo.min(p), hi.max(p)))
}

/// Yaw (degrees) of map entity `index` in the running logic.
fn yaw(sim: &Sim, index: usize) -> f32 {
    let logic = sim.app.world().resource::<Logic>();
    let id = logic
        .world
        .ids()
        .into_iter()
        .find(|id| logic.world.get(*id).unwrap().map_index == Some(index))
        .unwrap();
    pusher(&logic.world.get(id).unwrap().class).unwrap().angles.y
}

#[test]
fn doors_are_chained_rotating_pairs_that_open_on_use() {
    let Some(map) = nuke() else { return };
    let doors: Vec<(usize, &MapEntity)> = map
        .entities
        .iter()
        .enumerate()
        .filter(|(_, e)| e.classname() == "func_door_rotating")
        .collect();
    let name = |e: &MapEntity| e.get("targetname").unwrap_or("").to_string();
    let pairs: Vec<(usize, usize)> = doors
        .iter()
        .filter_map(|(i, d)| {
            let chain = d.get("chainstodoor")?;
            let (j, _) = doors
                .iter()
                .find(|(j, o)| j != i && name(o) == chain && o.get("chainstodoor") == Some(&name(d)))?;
            (i < j).then_some((*i, *j))
        })
        .collect();
    assert!(!pairs.is_empty(), "no chained func_door_rotating pairs among {} doors", doors.len());
    for (i, j) in &pairs {
        for e in [&map.entities[*i], &map.entities[*j]] {
            assert!(e.mover && !e.hulls.is_empty(), "door {} moves and has volumes", name(e));
            assert_eq!(e.get("spawnflags"), Some("1280"), "door {}", name(e));
            assert_eq!(e.get("distance").map(|v| v.parse::<f32>().unwrap()), Some(90.0));
            assert_eq!(e.get("speed").map(|v| v.parse::<f32>().unwrap()), Some(200.0));
            assert_eq!(e.get("wait").map(|v| v.parse::<f32>().unwrap()), Some(4.0));
        }
    }

    // The handles (prop_dynamic parented to the doors) ride them.
    let riders = map
        .props
        .iter()
        .filter(|p| p.parent.is_some_and(|i| map.entities[i].mover))
        .count();
    assert!(riders >= 2 * pairs.len(), "{riders} props ride the {} door pairs", pairs.len());

    // Stand 40 units in front of the first pair's first door, at its
    // foot, looking at its middle; press +use once.
    let (a, b) = pairs[0];
    let (lo, hi) = bounds(&map.entities[a]);
    let size = hi - lo;
    let centre = (lo + hi) / 2.0;
    let out = if size.x < size.y { Vec3::X } else { Vec3::Y } * (size.x.min(size.y) / 2.0 + 40.0);
    let feet = Vec3::new(centre.x, centre.y, lo.z) + out;
    let look = -out;
    let source_yaw = look.y.atan2(look.x).to_degrees();

    let mut sim = Sim::new((MapPlugin::new(map), SourceMovementPlugin));
    sim.set_tick_interval(cs_source::TICK_INTERVAL);
    let p = sim.spawn_character(to_engine(feet + Vec3::Z * 37.0), movement::ID);
    sim.intent(p).yaw = (source_yaw - 90.0).to_radians();
    sim.ticks(10);
    let (ya, yb) = (yaw(&sim, a), yaw(&sim, b));
    sim.intent(p).use_key = true;
    sim.ticks(1);
    sim.intent(p).use_key = false;
    // Opening starts with the door's next step: 90 degrees after 30 ticks.
    sim.ticks(29);
    let turned = |sim: &Sim| ((yaw(sim, a) - ya).abs(), (yaw(sim, b) - yb).abs());
    let (ta, tb) = turned(&sim);
    assert!(ta > 80.0 && ta < 90.0 && tb > 80.0 && tb < 90.0, "after 29 ticks: {ta}, {tb}");
    sim.ticks(1);
    assert_eq!(turned(&sim), (90.0, 90.0), "both open after 0.45 s");
    // Wait 4 s (267 ticks), then 30 ticks closing.
    sim.ticks(266);
    assert_eq!(turned(&sim), (90.0, 90.0), "still open during the wait");
    sim.ticks(32);
    assert_eq!(turned(&sim), (0.0, 0.0), "closed again");

    // Open them again, then a round restart: closed at once.
    sim.intent(p).use_key = true;
    sim.ticks(1);
    sim.intent(p).use_key = false;
    sim.ticks(20);
    assert!(turned(&sim).0 > 30.0, "opening");
    sim.app.world_mut().resource_mut::<RoundRestarts>().0 += 1;
    sim.ticks(1);
    assert_eq!(turned(&sim), (0.0, 0.0), "a new round starts with them closed");
}
