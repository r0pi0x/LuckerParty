//! Areaportals on stock CS:S maps (map::vis areas, logic::visuals
//! areaportals), headless: the maps' areas and portals load, every portal
//! has its entity, and a door linked to an areaportal hides the area
//! behind it while closed and shows it once opened. Skipped without an
//! install.

use bevy::{camera::CameraProjection, prelude::*};
use mashup::{
    games::{self, cs_source},
    harness::Sim,
    logic::{Logic, Value},
    map::{
        MapData, MapPlugin,
        vis::{self, AreaPortalStates, FadeDistance, LogicHidden, MapVisibility, VisClusters, WindowBrush},
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

const DOORS: &[&str] = &["func_door", "func_door_rotating", "prop_door_rotating"];

#[test]
fn areas_and_portals_load_with_their_entities() {
    for name in ["de_nuke", "cs_militia"] {
        let Some(map) = load(name) else { return };
        let v = map.visibility.as_deref().expect("visibility");
        let areas = &v.areas;
        assert!(areas.area_count() > 2, "{name}: {} areas", areas.area_count());
        assert!(!areas.portals.is_empty(), "{name}: no areaportals");
        assert_eq!(areas.leaf_areas.len(), v.leaf_clusters.len());
        let keys: Vec<u16> = map
            .entities
            .iter()
            .filter(|e| e.classname().starts_with("func_areaportal"))
            .filter_map(|e| e.get("portalnumber")?.trim().parse().ok())
            .collect();
        for p in &areas.portals {
            assert!(keys.contains(&p.key), "{name}: portal {} has no entity", p.key);
            assert!(p.polygon.len() >= 3, "{name}: portal {} has {} vertices", p.key, p.polygon.len());
            assert!(p.areas[0] != p.areas[1] && p.areas[0] != 0, "{name}: portal {} joins {:?}", p.key, p.areas);
        }
        let shared = areas.cluster_areas.iter().filter(|a| a.len() > 1).count();
        let linked = map
            .entities
            .iter()
            .filter(|e| e.classname() == "func_areaportal" && e.get("target").is_some_and(|t| !t.is_empty()))
            .count();
        eprintln!(
            "{name}: {} areas, {} portals ({} windows, {} see-through, {linked} linked to doors), {} of {} clusters in more than one area",
            areas.area_count(),
            areas.portals.len(),
            areas.portals.iter().filter(|p| p.fade.is_some()).count(),
            areas.portals.iter().filter(|p| p.see_through).count(),
            shared,
            v.cluster_count
        );
    }
}

/// A door-linked portal, a camera spot 1 m on its near side looking at
/// its middle from which the far area is out of sight while the doors are
/// shut, and the door's name.
struct Setup {
    key: u16,
    door: String,
    eye: Vec3,
    look_at: Vec3,
    far_area: u16,
}

fn find_setup(map: &MapData, v: &MapVisibility) -> Option<Setup> {
    let areas = &v.areas;
    // Portals linked to doors: all closed at the start (the doors are).
    let linked: Vec<u16> = map
        .entities
        .iter()
        .filter(|e| e.classname() == "func_areaportal" && e.get("target").is_some_and(|t| !t.is_empty()))
        .filter_map(|e| e.get("portalnumber")?.trim().parse().ok())
        .collect();
    for e in map.entities.iter().filter(|e| e.classname() == "func_areaportal") {
        let (Some(key), Some(door)) = (
            e.get("portalnumber").and_then(|k| k.trim().parse::<u16>().ok()),
            e.get("target").filter(|t| !t.is_empty()),
        ) else {
            continue;
        };
        if !map
            .entities
            .iter()
            .any(|d| DOORS.contains(&d.classname()) && d.get("targetname") == Some(door))
        {
            continue;
        }
        let Some(portal) = areas.portals.iter().find(|p| p.key == key) else { continue };
        let poly = &portal.polygon;
        let centre = poly.iter().copied().sum::<Vec3>() / poly.len() as f32;
        let normal = (poly[1] - poly[0]).cross(poly[2] - poly[0]).normalize_or_zero();
        for side in [1.0, -1.0] {
            let eye = centre + normal * side;
            let near = v.area_at(eye);
            if v.cluster_at(eye).is_none() || !portal.areas.contains(&near) {
                continue;
            }
            let far = if portal.areas[0] == near { portal.areas[1] } else { portal.areas[0] };
            // What the test camera sees (Camera3d's default projection):
            // through the door's opening only, so open portals elsewhere
            // (the house's windows) are out of view.
            let view = Transform::from_translation(eye).looking_at(centre, Vec3::Y);
            let clip = PerspectiveProjection::default().get_clip_from_view() * view.to_matrix().inverse();
            let reached = areas.flood(near, &|p| !linked.contains(&p.key), Some(clip));
            if !reached[far as usize] {
                return Some(Setup {
                    key,
                    door: door.to_string(),
                    eye,
                    look_at: centre,
                    far_area: far,
                });
            }
        }
    }
    None
}

#[test]
fn a_closed_door_hides_the_area_behind_its_portal() {
    // cs_militia's doors are func_door_rotating, cs_assault's
    // prop_door_rotating.
    for name in ["cs_militia", "cs_assault"] {
        let Some(map) = load(name) else { return };
        let v = map.visibility.clone().expect("visibility");
        let s = find_setup(&map, &v).unwrap_or_else(|| panic!("{name}: no door-linked portal that alone leads somewhere"));
        let cluster = v.cluster_at(s.eye).unwrap();
        // Map parts wholly in the far area that the camera's cluster can
        // potentially see.
        let behind = |clusters: &[u32]| {
            !clusters.is_empty()
                && clusters.iter().all(|&c| v.areas.cluster_areas[c as usize] == [s.far_area])
                && clusters.iter().any(|&c| v.sees(cluster, c))
        };
        let mut sim = Sim::new(MapPlugin::new(map));
        sim.set_tick_interval(cs_source::TICK_INTERVAL);
        sim.app
            .world_mut()
            .spawn((Camera3d::default(), Transform::from_translation(s.eye).looking_at(s.look_at, Vec3::Y)));
        sim.ticks(2);
        let parts = |sim: &mut Sim| -> Vec<bool> {
            let world = sim.app.world_mut();
            // Fading props may be beyond their fade distance; logic-hidden
            // ones stay hidden.
            let mut q = world.query_filtered::<(&VisClusters, &Visibility), (Without<FadeDistance>, Without<LogicHidden>)>();
            q.iter(world)
                .filter(|(c, _)| behind(&c.clusters))
                .map(|(_, vis)| *vis != Visibility::Hidden)
                .collect()
        };
        let closed = |sim: &Sim| {
            !sim.app
                .world()
                .get_resource::<AreaPortalStates>()
                .is_some_and(|s2| s2.is_open(s.key))
        };
        assert!(closed(&sim), "{name}: portal {} starts closed with door {}", s.key, s.door);
        let shut = parts(&mut sim);
        assert!(!shut.is_empty(), "{name}: map parts behind portal {}", s.key);
        assert!(shut.iter().all(|shown| !shown), "{name}: {} parts behind the shut door drawn", shut.iter().filter(|x| **x).count());
        // Open the door: the portal opens with it.
        sim.app
            .world_mut()
            .resource_mut::<Logic>()
            .world
            .queue_input(&s.door, "Open", Value::Void, 0.0, None);
        sim.ticks(3);
        assert!(!closed(&sim), "{name}: portal {} opens with its door", s.key);
        let open = parts(&mut sim);
        assert!(open.iter().all(|shown| *shown), "{name}: {} of {} parts behind the open door hidden", open.iter().filter(|x| !**x).count(), open.len());
        let dir = s.look_at - s.eye;
        eprintln!(
            "{name}: portal {} (door {}): {} parts behind it; camera at {:.2}, yaw {:.0}",
            s.key,
            s.door,
            open.len(),
            s.eye,
            (-dir.x).atan2(-dir.z).to_degrees()
        );
    }
}

/// de_nuke's windows (func_areaportalwindow, TranslucencyLimit 0): near
/// the opening the window brush isn't drawn and the portal is open; past
/// its FadeDist the brush is drawn and the portal closed.
#[test]
fn window_brushes_fade_in_and_close_their_portal_with_distance() {
    let Some(map) = load("de_nuke") else { return };
    let v = map.visibility.clone().expect("visibility");
    let windows: Vec<(usize, &vis::AreaPortal)> =
        v.areas.portals.iter().enumerate().filter(|(_, p)| p.fade.is_some_and(|f| f.brush.is_some())).collect();
    assert_eq!(windows.len(), 4, "de_nuke's four windows with brushes");
    // A window and two spots looking at its opening: one within its
    // start distance, one past its end (each in a cluster).
    let mut setup = None;
    'search: for (i, p) in &windows {
        let fade = p.fade.unwrap();
        let poly = &p.polygon;
        let centre = poly.iter().copied().sum::<Vec3>() / poly.len() as f32;
        let normal = (0..poly.len()).map(|k| poly[k].cross(poly[(k + 1) % poly.len()])).sum::<Vec3>().normalize();
        for side in [1.0, -1.0] {
            let near = centre + normal * side * 1.0;
            if v.cluster_at(near).is_none() || fade.alpha(p.distance(near)) > 0.0 {
                continue;
            }
            for extra in [1.0, 3.0, 6.0] {
                let far = centre + normal * side * (fade.end + extra);
                if v.cluster_at(far).is_some() && p.window_closed(far) {
                    setup = Some((*i, centre, near, far));
                    break 'search;
                }
            }
        }
    }
    let (portal, centre, near, far) = setup.expect("a window with spots near and far");
    let mut sim = Sim::new(MapPlugin::new(map));
    sim.set_tick_interval(cs_source::TICK_INTERVAL);
    let camera = sim
        .app
        .world_mut()
        .spawn((Camera3d::default(), Transform::from_translation(near).looking_at(centre, Vec3::Y)))
        .id();
    // The brush's alpha and, when rendering spawned its meshes, whether
    // they are drawn.
    let brush = |sim: &mut Sim| -> (Option<f32>, Vec<bool>) {
        let world = sim.app.world_mut();
        let mut q = world.query::<(&WindowBrush, Option<&Children>)>();
        let (alpha, children): (Option<f32>, Vec<Entity>) = q
            .iter(world)
            .find(|(w, _)| w.portal == portal)
            .map(|(w, c)| (w.alpha, c.map(|c| c.iter().collect()).unwrap_or_default()))
            .expect("the window's brush node");
        let drawn = children.iter().filter_map(|c| world.get::<Visibility>(*c)).map(|v| *v != Visibility::Hidden).collect();
        (alpha, drawn)
    };
    sim.ticks(2);
    let (alpha, drawn) = brush(&mut sim);
    assert_eq!(alpha, Some(0.0), "window {portal}: transparent up close");
    assert!(drawn.iter().all(|d| !d), "window {portal}: brush hidden up close");
    *sim.app.world_mut().get_mut::<Transform>(camera).unwrap() = Transform::from_translation(far).looking_at(centre, Vec3::Y);
    sim.ticks(2);
    let (alpha, drawn) = brush(&mut sim);
    assert_eq!(alpha, Some(1.0), "window {portal}: opaque from afar");
    assert!(drawn.iter().all(|d| *d), "window {portal}: brush drawn from afar");
    eprintln!("de_nuke window {portal}: hidden at {near:.2}, drawn and closed at {far:.2}");
}
