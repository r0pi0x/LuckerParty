//! Maps with many brush entities (doors, breakables), headless with the
//! render assets the map spawns: brush entities' meshes with the same
//! material share one material asset (so they draw in batches), and they
//! are culled by the map's visibility like the world, following them as
//! they move (docs/performance.md); while they stay put and whole they
//! are drawn merged per material and chunk (`map::merge`), and drawn on
//! their own again once they move, break or are removed. On the minigame
//! map
//! `mg_lego_multigames_v2` (578 func_breakable, 170 func_door; skipped
//! when it isn't in the content cache) and de_nuke (skipped without an
//! install).

use std::collections::{HashMap, HashSet};

use bevy::prelude::*;
use mashup::{
    games::{self, cs_source},
    harness::Sim,
    core::RoundRestarts,
    logic::{Logic, Value},
    map::{
        BrushEntityBounds, MapBrushEntity, MapData, MapPlugin,
        merge::{MergedBrush, MergedChunk, MergedPiece},
        prop_material::PropMaterial,
        rope_material::RopeMaterial,
        sprite_material::SpriteMaterial,
        vis::{self, ActiveVisibility, LogicHidden, VisClusters, VisStats},
        world_material::WorldMaterial,
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
    match games::load_map(&format!("cs_source:{name}")) {
        Ok(m) => Some(m),
        Err(e) => {
            eprintln!("skipping {name}: {e}");
            None
        }
    }
}

/// The render assets the map spawns its meshes with (headless `Sim` has
/// meshes only).
fn render_assets(app: &mut App) {
    app.init_asset::<Image>()
        .init_asset::<StandardMaterial>()
        .init_asset::<WorldMaterial>()
        .init_asset::<PropMaterial>()
        .init_asset::<SpriteMaterial>()
        .init_asset::<RopeMaterial>()
        .init_asset::<bevy::mesh::skinning::SkinnedMeshInverseBindposes>();
}

/// The lego map's slow view (the room with the breakable blocks): eye
/// position, engine space (setpos 110.02 -795.16 -140.17, eye 64 units up).
fn lego_eye() -> Vec3 {
    Vec3::new(110.02, -140.17 + 64.0, 795.16) * cs_source::bsp::METERS_PER_UNIT
}

/// Brush entity meshes: (material name, world material asset) per mesh.
fn brush_entity_materials(sim: &mut Sim) -> Vec<(String, AssetId<WorldMaterial>)> {
    let world = sim.app.world_mut();
    let nodes: HashSet<Entity> = world
        .query_filtered::<Entity, With<MapBrushEntity>>()
        .iter(world)
        .collect();
    world
        .query::<(&Name, &MeshMaterial3d<WorldMaterial>, &ChildOf)>()
        .iter(world)
        .filter(|(_, _, p)| nodes.contains(&p.parent()))
        .map(|(n, m, _)| (n.as_str().to_string(), m.0.id()))
        .collect()
}

#[test]
fn brush_entities_share_materials() {
    for name in ["mg_lego_multigames_v2", "de_nuke"] {
        let Some(map) = load(name) else { continue };
        let mut sim = Sim::new((render_assets, MapPlugin::new(map)));
        let meshes = brush_entity_materials(&mut sim);
        let handles: HashSet<_> = meshes.iter().map(|(_, h)| *h).collect();
        let mut per_name: HashMap<&str, HashSet<AssetId<WorldMaterial>>> = HashMap::new();
        for (n, h) in &meshes {
            per_name.entry(n.as_str()).or_default().insert(*h);
        }
        eprintln!(
            "{name}: {} brush entity meshes, {} materials, {} material names",
            meshes.len(),
            handles.len(),
            per_name.len()
        );
        assert!(!meshes.is_empty(), "{name}: lightmapped brush entity meshes");
        // A material's variants: in the 3D skybox or not, and its nearest
        // cubemap when it reflects.
        for (n, hs) in &per_name {
            assert!(hs.len() <= 4, "{name}: {n} has {} material assets", hs.len());
        }
        assert!(handles.len() * 2 < meshes.len(), "{name}: materials aren't shared");
    }
}

#[test]
fn brush_entities_are_culled_and_follow_movers() {
    let Some(map) = load("mg_lego_multigames_v2") else {
        return;
    };
    let v = map.visibility.clone().expect("visibility");
    // The camera's cluster is found (the view once drew everything: the
    // water reflection's mirrored eye was outside the map).
    let eye = lego_eye();
    let cluster = v.cluster_at(eye).expect("the slow view's eye is in a cluster");
    let mut sim = Sim::new((render_assets, MapPlugin::new(map)));
    sim.set_tick_interval(cs_source::TICK_INTERVAL);
    let yaw = (153.31f32 - 90.0).to_radians();
    sim.app.world_mut().spawn((
        Camera3d::default(),
        Transform::from_translation(eye).with_rotation(Quat::from_rotation_y(yaw)),
    ));
    sim.ticks(2);
    assert_eq!(sim.app.world().resource::<VisStats>().cluster, Some(cluster));
    // Every brush entity with meshes is tagged; it is drawn exactly when one
    // of its clusters is potentially visible.
    let world = sim.app.world_mut();
    let mut q = world.query_filtered::<(&VisClusters, &Visibility), (With<MapBrushEntity>, Without<LogicHidden>)>();
    let (mut shown, mut hidden) = (0, 0);
    for (c, visibility) in q.iter(world) {
        assert!(!c.clusters.is_empty(), "a brush entity with no clusters");
        let seen = v.sees_any(cluster, &c.clusters);
        assert_eq!(*visibility != Visibility::Hidden, seen, "clusters {:?}", c.clusters);
        if seen {
            shown += 1;
        } else {
            hidden += 1;
        }
    }
    eprintln!("brush entities from the slow view: {shown} drawn, {hidden} culled");
    assert!(
        hidden > 100 && shown > 100,
        "the wall of blocks drawn, other rooms culled"
    );

    // Movers (its func_rotating turns all the time) take their clusters
    // along.
    let before: Vec<(Entity, GlobalTransform)> = {
        let world = sim.app.world_mut();
        world
            .query_filtered::<(Entity, &GlobalTransform), With<BrushEntityBounds>>()
            .iter(world)
            .map(|(e, t)| (e, *t))
            .collect()
    };
    sim.seconds(2.0);
    let world = sim.app.world_mut();
    let vis = world.resource::<ActiveVisibility>().0.clone();
    let mut moved = 0;
    let mut q = world.query::<(Entity, &GlobalTransform, &BrushEntityBounds, &VisClusters)>();
    for (e, at, b, tag) in q.iter(world) {
        let Some((_, was)) = before.iter().find(|(n, _)| *n == e) else {
            continue;
        };
        if was.translation().distance(at.translation()) < 0.01 && was.rotation().angle_between(at.rotation()) < 0.01 {
            continue;
        }
        moved += 1;
        let corners = (0..8).map(|i| {
            at.transform_point(Vec3::new(
                if i & 1 == 0 { b.min.x } else { b.max.x },
                if i & 2 == 0 { b.min.y } else { b.max.y },
                if i & 4 == 0 { b.min.z } else { b.max.z },
            ))
        });
        let (lo, hi) = corners.fold((Vec3::MAX, Vec3::MIN), |(lo, hi), p| (lo.min(p), hi.max(p)));
        let want = vis::box_clusters(&vis, lo - Vec3::splat(1e-3), hi + Vec3::splat(1e-3));
        for c in tag.clusters.iter() {
            assert!(want.contains(c), "a moved brush entity keeps cluster {c} it left");
        }
    }
    assert!(moved > 0, "a brush entity moved");
}

/// Every combined mesh draws exactly the triangles of its merged nodes,
/// and every node's own meshes are hidden exactly while it is merged.
/// Returns (merged nodes, split nodes, combined meshes).
fn check_merged(sim: &mut Sim) -> (usize, usize, usize) {
    let world = sim.app.world_mut();
    let nodes: HashMap<Entity, bool> = world
        .query::<(Entity, &MergedBrush)>()
        .iter(world)
        .map(|(e, m)| (e, m.merged))
        .collect();
    let chunks: Vec<(Entity, MergedChunk, Handle<Mesh>, bool)> = world
        .query::<(Entity, &MergedChunk, &Mesh3d, Has<LogicHidden>)>()
        .iter(world)
        .map(|(e, c, m, h)| (e, c.clone(), m.0.clone(), h))
        .collect();
    let meshes = world.resource::<Assets<Mesh>>();
    for (e, c, mesh, hidden) in &chunks {
        let want: Vec<u32> = c
            .parts
            .iter()
            .filter(|(n, _)| nodes[n])
            .flat_map(|(_, r)| c.indices[r.clone()].iter().copied())
            .collect();
        if want.is_empty() {
            assert!(hidden, "chunk {e} has nothing merged and is hidden");
            continue;
        }
        assert!(!hidden, "chunk {e} with merged nodes is drawn");
        let got: Vec<u32> = meshes.get(mesh).unwrap().indices().unwrap().iter().map(|i| i as u32).collect();
        assert_eq!(got, want, "chunk {e} draws its merged nodes' triangles");
    }
    let mut q = world.query_filtered::<(&Visibility, &ChildOf), With<MergedPiece>>();
    for (v, parent) in q.iter(world) {
        let merged = nodes[&parent.parent()];
        assert_eq!(*v == Visibility::Hidden, merged, "a node's own mesh shows exactly when it is not merged");
    }
    let merged = nodes.values().filter(|m| **m).count();
    (merged, nodes.len() - merged, chunks.len())
}

fn merged(sim: &mut Sim, index: usize) -> bool {
    merge_state(sim, index).expect("a merged brush entity")
}

/// Whether map entity `index` is drawn merged; None when it never is.
fn merge_state(sim: &mut Sim, index: usize) -> Option<bool> {
    let world = sim.app.world_mut();
    world
        .query::<(&MapBrushEntity, &MergedBrush)>()
        .iter(world)
        .find(|(b, _)| b.0 == index)
        .map(|(_, m)| m.merged)
}

fn input(sim: &mut Sim, target: &str, input: &str) {
    sim.app
        .world_mut()
        .resource_mut::<Logic>()
        .world
        .queue_input(target, input, Value::Void, 0.0, None);
}

#[test]
fn brush_entities_draw_merged_until_they_change() {
    if let Some(map) = load("mg_lego_multigames_v2") {
        let breakables = map.entities.iter().filter(|e| e.classname() == "func_breakable").count();
        let mut sim = Sim::new((render_assets, MapPlugin::new(map)));
        sim.set_tick_interval(cs_source::TICK_INTERVAL);
        sim.ticks(2);
        let (merged, split, chunks) = check_merged(&mut sim);
        eprintln!("lego: {merged} merged, {split} split (movers), {chunks} combined meshes");
        assert!(merged >= breakables / 2, "the blocks are merged ({merged} of {breakables} breakables)");
        let own = {
            let world = sim.app.world_mut();
            world.query_filtered::<(), With<MergedPiece>>().iter(world).count()
        };
        assert!(chunks * 4 < own, "{chunks} combined meshes for {own} brush entity meshes");
        // Break them all: each leaves its combined mesh.
        input(&mut sim, "func_breakable", "Break");
        sim.seconds(1.0);
        let (after, _, _) = check_merged(&mut sim);
        assert!(after + breakables / 2 <= merged, "broken blocks left: {merged} -> {after}");
        sim.app.world_mut().resource_mut::<RoundRestarts>().0 += 1;
        sim.ticks(3);
        let (back, _, _) = check_merged(&mut sim);
        assert!(back >= merged, "a new round merges them again: {back} of {merged}");
    }

    let Some(map) = load("de_nuke") else { return };
    let vents: Vec<usize> = map
        .entities
        .iter()
        .enumerate()
        .filter(|(_, e)| e.classname() == "func_breakable" && e.get("material") == Some("2"))
        .map(|(i, _)| i)
        .collect();
    let doors: Vec<(usize, String)> = map
        .entities
        .iter()
        .enumerate()
        .filter(|(_, e)| e.classname() == "func_door_rotating" && e.get("targetname").is_some_and(|n| !n.is_empty()))
        .map(|(i, e)| (i, e.get("targetname").unwrap().to_string()))
        .collect();
    let mut sim = Sim::new((render_assets, MapPlugin::new(map)));
    sim.set_tick_interval(cs_source::TICK_INTERVAL);
    sim.ticks(2);
    let (start, _, chunks) = check_merged(&mut sim);
    eprintln!("de_nuke: {start} merged brush entities in {chunks} combined meshes");
    // Meshes that are blended (see-through grates) stay on their own.
    let vents: Vec<usize> = vents.into_iter().filter(|v| merge_state(&mut sim, *v).is_some()).collect();
    assert!(!vents.is_empty(), "some vents are drawn merged");
    assert!(vents.iter().all(|v| merged(&mut sim, *v)), "the vents start merged");
    let door = doors
        .into_iter()
        .find(|(i, _)| merge_state(&mut sim, *i).is_some())
        .expect("a merged named door");
    assert!(merged(&mut sim, door.0), "the door starts merged");

    // Broken vents leave; an opening door leaves and rejoins once shut.
    input(&mut sim, "func_breakable", "Break");
    input(&mut sim, &door.1, "Open");
    sim.seconds(0.3);
    check_merged(&mut sim);
    assert!(vents.iter().all(|v| !merged(&mut sim, *v)), "broken vents are split out");
    assert!(!merged(&mut sim, door.0), "a moving door is split out");
    sim.seconds(6.0);
    check_merged(&mut sim);
    assert!(merged(&mut sim, door.0), "the door shut again rejoins");
    sim.app.world_mut().resource_mut::<RoundRestarts>().0 += 1;
    sim.ticks(3);
    let (back, _, _) = check_merged(&mut sim);
    assert_eq!(back, start, "a new round merges the vents again");
}
