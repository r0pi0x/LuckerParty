//! Physics props' shadows on the world (Source's render-to-texture
//! shadows, specs/cs_source/shadows_sky.md) while the prop moves, headless:
//! the shadow follows the prop every frame through one mesh asset that is
//! rewritten in place. A new mesh handle each frame left a moving prop's
//! shadow undrawn until it came to rest (Bevy re-specializes an entity
//! whose `Mesh3d` changed, and skipped it while the new mesh wasn't
//! prepared on the GPU yet). The render world's extraction is emulated
//! (`harness::emulate_mesh_extraction`): every rewrite must reach it once,
//! in the frame it was made.

use avian3d::prelude::LinearVelocity;
use bevy::{camera::primitives::MeshAabb, prelude::*};
use mashup::{
    harness::{MeshExtraction, Sim},
    map::{
        MapCollision, MapConvex, MapData, MapMesh, MapModel, MapPhysics, MapPlugin, MapProp, MapShadows, PropIndex,
        PropShadow, PropSolid, PushAway,
        prop_material::PropMaterial,
        rope_material::RopeMaterial,
        shadows::ShadowMaterial,
        sprite_material::SpriteMaterial,
        world_material::WorldMaterial,
    },
};

/// A box's 12 triangles (corners `lo`..`hi`).
fn box_mesh(lo: Vec3, hi: Vec3) -> MapMesh {
    let p = |k: usize| [[lo.x, hi.x][k & 1], [lo.y, hi.y][(k >> 1) & 1], [lo.z, hi.z][(k >> 2) & 1]];
    let positions: Vec<[f32; 3]> = (0..8).map(p).collect();
    let indices = vec![
        0, 1, 3, 0, 3, 2, 4, 6, 7, 4, 7, 5, 0, 4, 5, 0, 5, 1, 2, 3, 7, 2, 7, 6, 0, 2, 6, 0, 6, 4, 1, 5, 7, 1, 7, 3,
    ];
    MapMesh {
        material: "test/box".into(),
        normals: vec![[0.0, 1.0, 0.0]; 8],
        uvs: vec![[0.0, 0.0]; 8],
        positions,
        indices,
        color: [128, 128, 128],
        ..default()
    }
}

/// A 20 m floor (world mesh and collision) with a 0.6 m physics crate
/// standing on it, casting a shadow straight down and a little sideways.
fn map() -> MapData {
    let mut data = MapData {
        name: "test:prop_shadows".into(),
        entity_scale: 0.0254,
        shadows: Some(MapShadows {
            direction: Vec3::new(0.2, -1.0, 0.1).normalize(),
            color: [100, 100, 100],
            distance: 2.0,
        }),
        ..default()
    };
    let (a, b) = (Vec3::new(-10.0, -1.0, -10.0), Vec3::new(10.0, 0.0, 10.0));
    data.collision_brushes.push(mashup::map::MapBrush::from_box(a, b));
    data.collision_hulls.push(
        (0..8)
            .map(|k| [[a.x, b.x][k & 1], [a.y, b.y][(k >> 1) & 1], [a.z, b.z][(k >> 2) & 1]])
            .collect(),
    );
    // The floor's top face, facing up.
    data.meshes.push(MapMesh {
        material: "test/floor".into(),
        positions: vec![[-10.0, 0.0, -10.0], [-10.0, 0.0, 10.0], [10.0, 0.0, 10.0], [10.0, 0.0, -10.0]],
        normals: vec![[0.0, 1.0, 0.0]; 4],
        uvs: vec![[0.0, 0.0]; 4],
        lightmap_uvs: vec![[0.0, 0.0]; 4],
        indices: vec![0, 1, 2, 0, 2, 3],
        color: [200, 180, 150],
        ..default()
    });
    let half = 0.3;
    let (lo, hi) = (Vec3::splat(-half), Vec3::splat(half));
    let corners = (0..8)
        .map(|k| Vec3::new([lo.x, hi.x][k & 1], [lo.y, hi.y][(k >> 1) & 1], [lo.z, hi.z][(k >> 2) & 1]))
        .collect();
    data.models.push(MapModel {
        meshes: vec![box_mesh(lo, hi)],
        bounds: (lo, hi),
        collision: Some(MapCollision {
            pieces: vec![MapConvex {
                points: corners,
                planes: Vec::new(),
            }],
            mass: 20.0,
            ..default()
        }),
        ..default()
    });
    data.props.push(MapProp {
        pose: None,
        model: 0,
        translation: Vec3::new(0.0, half + 0.01, 0.0),
        rotation: Quat::IDENTITY,
        solid: PropSolid::Mesh,
        skybox: false,
        lighting: None,
        vertex_light: None,
        casts_shadow: true,
        physics: Some(MapPhysics {
            mass: 20.0,
            friction: 0.0,
            elasticity: 0.0,
            damping: 0.0,
            rotdamping: 0.0,
            push: PushAway::Collide,
            frozen: false,
            asleep: false,
        }),
        fade: None,
        parent: None,
        entity: None,
        skin: 0,
        body: 0,
    });
    data
}

/// The map's render assets (a headless `Sim` has meshes only).
fn sim() -> Sim {
    Sim::with(|app| {
        app.init_asset::<Image>()
            .init_asset::<StandardMaterial>()
            .init_asset::<WorldMaterial>()
            .init_asset::<PropMaterial>()
            .init_asset::<SpriteMaterial>()
            .init_asset::<RopeMaterial>()
            .init_asset::<ShadowMaterial>()
            .init_asset::<bevy::mesh::skinning::SkinnedMeshInverseBindposes>()
            .add_plugins(MapPlugin::new(map()));
        mashup::harness::emulate_mesh_extraction(app);
    })
}

/// The shadow's entity, mesh asset and its centre.
fn shadow(sim: &mut Sim) -> (Entity, AssetId<Mesh>, Vec3) {
    let world = sim.app.world_mut();
    let (e, mesh) = world
        .query::<(Entity, &PropShadow, &Mesh3d)>()
        .iter(world)
        .find(|(_, s, _)| s.prop == 0)
        .map(|(e, _, m)| (e, m.id()))
        .expect("the crate casts a shadow");
    // As the render world holds it (the shadow mesh lives there only).
    let aabb = world
        .resource::<MeshExtraction>()
        .extracted
        .get(&mesh)
        .and_then(|m| m.compute_aabb())
        .expect("a shadow mesh with positions");
    (e, mesh, Vec3::from(aabb.center))
}

fn crate_at(sim: &mut Sim) -> (Entity, Vec3) {
    let world = sim.app.world_mut();
    world
        .query::<(Entity, &PropIndex, &Transform)>()
        .iter(world)
        .find(|(_, i, _)| i.0 == 0)
        .map(|(e, _, t)| (e, t.translation))
        .expect("the crate")
}

#[test]
fn a_moving_prop_keeps_its_shadow_mesh_and_moves_it() {
    let mut sim = sim();
    sim.ticks(5);
    let (entity, mesh, start) = shadow(&mut sim);
    let (node, at) = crate_at(&mut sim);
    assert!(start.xz().distance(at.xz()) < 0.5, "under the crate: {start} {at}");
    sim.app.world_mut().entity_mut(node).insert(LinearVelocity(Vec3::X * 2.0));
    let mut last = start;
    for tick in 0..20 {
        sim.ticks(1);
        let (e, m, centre) = shadow(&mut sim);
        let (_, at) = crate_at(&mut sim);
        assert_eq!((e, m), (entity, mesh), "tick {tick}: the same shadow entity and mesh asset");
        assert!(
            centre.xz().distance(at.xz()) < 0.5,
            "tick {tick}: the shadow follows the crate: {centre} {at}"
        );
        last = centre;
    }
    assert!(last.x - start.x > 0.3, "the shadow moved with the crate: {start} -> {last}");
    // Coming to rest: no shadow update is announced after the render
    // world took its data (a rewrite announced a frame late, after
    // `AssetEventSystems`, was extracted twice: Bevy logged "RenderMesh
    // with RenderAssetUsages == RENDER_WORLD cannot be extracted" every
    // time a moving prop stopped).
    sim.app.world_mut().entity_mut(node).insert(LinearVelocity(Vec3::ZERO));
    for _ in 0..30 {
        sim.ticks(1);
        // Stop and go: moves on alternate stretches of frames.
        let v = if sim.tick() % 6 < 3 { Vec3::X } else { Vec3::ZERO };
        sim.app.world_mut().entity_mut(node).insert(LinearVelocity(v));
    }
    let failures = &sim.app.world().resource::<MeshExtraction>().failures;
    assert!(failures.is_empty(), "meshes announced changed after extraction: {failures:?}");
}
