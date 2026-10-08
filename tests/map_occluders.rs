//! Occluders on stock CS:S maps (map::vis occluders, logic::visuals
//! func_occluder), headless: the occlusion lump loads with its entities,
//! and a map part fully behind an active occluder is hidden from a camera
//! in front of it, shown once the occluder is deactivated and hidden again
//! when it is activated. Skipped without an install.

use bevy::prelude::*;
use mashup::{
    games::{self, cs_source},
    harness::Sim,
    logic::{Logic, Value},
    map::{
        MapData, MapPlugin,
        vis::{Occludee, VisStats},
    },
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
fn occluders_load_with_their_entities() {
    // (map, occluders, of which with polygons); de_aztec's were compiled
    // without any.
    for (name, count, with_polygons) in [("cs_assault", 16, 16), ("de_port", 16, 16), ("cs_compound", 14, 14), ("de_aztec", 7, 0)] {
        let Some(map) = load(name) else { return };
        let v = map.visibility.as_deref().expect("visibility");
        assert_eq!(v.occluders.len(), count, "{name}");
        assert_eq!(v.occluders.iter().filter(|o| !o.polygons.is_empty()).count(), with_polygons, "{name}");
        let keys: Vec<u16> = map
            .entities
            .iter()
            .filter(|e| e.classname() == "func_occluder")
            .filter_map(|e| e.get("occludernumber")?.trim().parse().ok())
            .collect();
        for o in &v.occluders {
            assert!(keys.contains(&o.key), "{name}: occluder {} has no entity", o.key);
            assert!(o.start_active, "{name}: occluder {} starts active", o.key);
            for p in &o.polygons {
                assert!(p.len() >= 3, "{name}: occluder {} polygon of {}", o.key, p.len());
                // Planar (corners may be collinear: Newell's normal).
                let n = (0..p.len())
                    .map(|i| p[i].cross(p[(i + 1) % p.len()]))
                    .sum::<Vec3>()
                    .normalize();
                assert!(p.iter().all(|c| n.dot(*c - p[0]).abs() < 0.01), "{name}: occluder {} not planar", o.key);
            }
        }
        let polygons: usize = v.occluders.iter().map(|o| o.polygons.len()).sum();
        eprintln!("{name}: {} occluders, {polygons} polygons", v.occluders.len());
    }
}

#[test]
fn deactivating_an_occluder_shows_what_it_hid() {
    let Some(map) = load("cs_assault") else { return };
    let v = map.visibility.clone().expect("visibility");
    // Named occluders and camera spots in front of each of their polygons,
    // looking at it.
    let names: Vec<(u16, String)> = map
        .entities
        .iter()
        .filter(|e| e.classname() == "func_occluder")
        .filter_map(|e| Some((e.get("occludernumber")?.trim().parse().ok()?, e.get("targetname")?.to_string())))
        .collect();
    let mut spots: Vec<(String, Vec3, Vec3)> = Vec::new();
    for o in &v.occluders {
        let Some((_, name)) = names.iter().find(|(k, _)| *k == o.key) else { continue };
        for p in &o.polygons {
            let centre = p.iter().copied().sum::<Vec3>() / p.len() as f32;
            let n = (0..p.len())
                .map(|i| p[i].cross(p[(i + 1) % p.len()]))
                .sum::<Vec3>()
                .normalize();
            for d in [3.0, 6.0, -3.0, -6.0] {
                let eye = centre + n * d + Vec3::Y * 0.2;
                if v.cluster_at(eye).is_some() {
                    spots.push((name.clone(), eye, centre));
                }
            }
        }
    }
    assert!(!spots.is_empty(), "no camera spots in front of cs_assault's occluders");
    let mut sim = Sim::new(MapPlugin::new(map));
    sim.set_tick_interval(cs_source::TICK_INTERVAL);
    let camera = sim.app.world_mut().spawn((Camera3d::default(), Transform::default())).id();
    let hidden = |sim: &mut Sim| -> Vec<Entity> {
        let world = sim.app.world_mut();
        let mut q = world.query::<(Entity, &Occludee, &Visibility)>();
        q.iter(world).filter(|(_, _, v)| **v == Visibility::Hidden).map(|(e, _, _)| e).collect()
    };
    let occluded = |sim: &Sim| sim.app.world().resource::<VisStats>().occluded_parts;
    // The first spot where the occluder hides something.
    let mut found = None;
    for (name, eye, at) in &spots {
        *sim.app.world_mut().get_mut::<Transform>(camera).unwrap() = Transform::from_translation(*eye).looking_at(*at, Vec3::Y);
        sim.ticks(1);
        if occluded(&sim) > 0 {
            found = Some((name.clone(), *eye));
            break;
        }
    }
    let (name, eye) = found.expect("an occluder hiding something from a spot in front of it");
    let active = hidden(&mut sim);
    let count = occluded(&sim);
    let send = |sim: &mut Sim, input: &str| {
        sim.app
            .world_mut()
            .resource_mut::<Logic>()
            .world
            .queue_input(&name, input, Value::Void, 0.0, None);
        sim.ticks(3);
    };
    send(&mut sim, "Deactivate");
    let off = hidden(&mut sim);
    let shown: Vec<Entity> = active.iter().copied().filter(|e| !off.contains(e)).collect();
    assert!(!shown.is_empty(), "{name}: deactivating shows the {count} parts it hid");
    assert!(occluded(&sim) < count);
    send(&mut sim, "Activate");
    let on = hidden(&mut sim);
    assert!(shown.iter().all(|e| on.contains(e)), "{name}: activated again, they hide again");
    send(&mut sim, "Toggle");
    let toggled = hidden(&mut sim);
    assert!(shown.iter().all(|e| !toggled.contains(e)), "{name}: toggled off");
    eprintln!("{name}: hides {} map parts from {eye:.2}", shown.len());
}
