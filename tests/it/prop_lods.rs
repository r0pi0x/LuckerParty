//! Props' levels of detail (`map::lod`, Video > Advanced's model detail):
//! a prop whose model has a lower level draws it once the camera is far
//! enough (its screen-size metric past the level's switch point), the
//! finer one up close; `r_rootlod` keeps it from the finer level at any
//! distance, `r_lod` forces one. Headless: the props' meshes swap handles.

use bevy::prelude::*;
use mashup::{
    harness::Sim,
    map::{
        MapData, MapMesh, MapModel, MapPlugin, MapProp, PropSolid, ViewModelAnchor,
        lod::{LodMeshes, MapLod, ModelSettings},
        prop_material::PropMaterial,
        rope_material::RopeMaterial,
        shadows::ShadowMaterial,
        sprite_material::SpriteMaterial,
        world_material::WorldMaterial,
    },
};

fn quad(material: &str, half: f32) -> MapMesh {
    MapMesh {
        material: material.into(),
        positions: vec![
            [-half, 0.0, 0.0],
            [half, 0.0, 0.0],
            [half, 2.0 * half, 0.0],
            [-half, 2.0 * half, 0.0],
        ],
        normals: vec![[0.0, 0.0, 1.0]; 4],
        uvs: vec![[0.0, 0.0]; 4],
        indices: vec![0, 1, 2, 0, 2, 3],
        color: [128, 128, 128],
        ..default()
    }
}

/// A 1 m prop with one lower level taking over at metric 20 (0.75 d / r
/// with r = 0.87 m: 23 m away at fov 90).
fn map() -> MapData {
    let mut data = MapData {
        name: "test:prop_lods".into(),
        entity_scale: 0.0254,
        ..default()
    };
    let half = 0.5;
    data.models.push(MapModel {
        meshes: vec![quad("test/pot", half)],
        bounds: (Vec3::new(-half, 0.0, -half), Vec3::new(half, 2.0 * half, half)),
        lods: vec![MapLod {
            switch: 20.0,
            meshes: vec![Some(quad("test/pot", half))],
        }],
        ..default()
    });
    data.props.push(MapProp {
        pose: None,
        ragdoll: None,
        model: 0,
        translation: Vec3::ZERO,
        rotation: Quat::IDENTITY,
        solid: PropSolid::None,
        skybox: false,
        lighting: None,
        vertex_light: None,
        casts_shadow: false,
        physics: None,
        fade: None,
        parent: None,
        entity: None,
        skin: 0,
        body: 0,
        detail: false,
    });
    data
}

/// Which level the prop's mesh draws (index into its `LodMeshes`).
fn level(sim: &mut Sim) -> usize {
    let world = sim.app.world_mut();
    let (levels, mesh) = world
        .query::<(&LodMeshes, &Mesh3d)>()
        .iter(world)
        .next()
        .map(|(l, m)| (l.0.clone(), m.0.clone()))
        .expect("the prop's mesh has levels");
    levels.iter().position(|h| *h == mesh).expect("one of its levels")
}

#[test]
fn far_props_draw_their_lower_level_and_model_detail_caps_the_finest() {
    let mut sim = Sim::with(|app| {
        app.init_asset::<Image>()
            .init_asset::<StandardMaterial>()
            .init_asset::<WorldMaterial>()
            .init_asset::<PropMaterial>()
            .init_asset::<SpriteMaterial>()
            .init_asset::<RopeMaterial>()
            .init_asset::<ShadowMaterial>()
            .init_asset::<bevy::mesh::skinning::SkinnedMeshInverseBindposes>()
            .add_plugins(MapPlugin::new(map()));
    });
    // The eye: fov 90 at 4:3 (vertical tangent 0.75).
    let fov = 2.0 * 0.75f32.atan();
    let eye = sim
        .app
        .world_mut()
        .spawn((
            ViewModelAnchor,
            Projection::Perspective(PerspectiveProjection { fov, ..default() }),
            Transform::from_xyz(0.0, 0.5, 5.0),
        ))
        .id();
    let at = |sim: &mut Sim, z: f32| {
        sim.app
            .world_mut()
            .entity_mut(eye)
            .insert(Transform::from_xyz(0.0, 0.5, z));
        sim.ticks(2);
    };
    at(&mut sim, 5.0);
    assert_eq!(level(&mut sim), 0, "5 m away: LOD 0");
    at(&mut sim, 30.0);
    assert_eq!(level(&mut sim), 1, "30 m away: the lower level");
    at(&mut sim, 5.0);
    assert_eq!(level(&mut sim), 0, "back near");
    // Model detail Medium (r_rootlod 1): the lower level up close too.
    sim.app.insert_resource(ModelSettings { root_lod: 1, lod: -1 });
    sim.ticks(2);
    assert_eq!(level(&mut sim), 1);
    // High with r_lod 0: LOD 0 even far off.
    sim.app.insert_resource(ModelSettings { root_lod: 0, lod: 0 });
    at(&mut sim, 30.0);
    assert_eq!(level(&mut sim), 0);
}
