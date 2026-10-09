//! Optional (`mashup_skyocclude 1`; off by default): sky faces hide what
//! lies behind them. Sky faces are never drawn
//! (specs/cs_source/shadows_sky.md); whether the game still keeps what
//! lies behind one (in the view's PVS) from showing through it is engine
//! side and unchecked, so by default nothing does, and what Source's PVS
//! culls is culled (`vis`: on mg_creative_multigames_v8_ns other rooms'
//! weapons and water showed through the sky only because they weren't).
//! When on, the playable world's sky faces (`MapData::sky_surfaces`) are
//! drawn into depth only: no colour (the sky camera's picture stays) and
//! no shadows. They go in the main view's depth prepass, which the main
//! camera then gets, so they are in the depth buffer before anything they
//! hide is drawn. A CS:S capture of a prop in the PVS behind a sky face
//! decides the default (docs/backlog.md).

use bevy::{
    asset::embedded_asset,
    camera::visibility::RenderLayers,
    mesh::{Indices, PrimitiveTopology},
    prelude::*,
    reflect::TypePath,
    render::render_resource::{AsBindGroup, ColorWrites},
    shader::ShaderRef,
};

/// A depth-only surface: writes depth, no colour.
#[derive(Asset, TypePath, AsBindGroup, Debug, Clone, Default)]
pub struct SkyOccluderMaterial {}

impl Material for SkyOccluderMaterial {
    fn fragment_shader() -> ShaderRef {
        "embedded://mashup/map/sky_occluder.wgsl".into()
    }

    fn specialize(
        _pipeline: &bevy::pbr::MaterialPipeline,
        descriptor: &mut bevy::render::render_resource::RenderPipelineDescriptor,
        _layout: &bevy::mesh::MeshVertexBufferLayoutRef,
        _key: bevy::pbr::MaterialPipelineKey<Self>,
    ) -> Result<(), bevy::render::render_resource::SpecializedMeshPipelineError> {
        // Seen from either side, and never coloured.
        descriptor.primitive.cull_mode = None;
        if let Some(fragment) = descriptor.fragment.as_mut() {
            for target in fragment.targets.iter_mut().flatten() {
                target.write_mask = ColorWrites::empty();
                target.blend = None;
            }
        }
        Ok(())
    }
}

/// The loaded map's sky surfaces (see the module docs).
#[derive(Component)]
pub struct SkyOccluder;

/// `mashup_skyocclude`: 1 sky faces hide what lies behind them; 0 (the
/// default) they don't.
#[derive(Resource, Default)]
pub struct SkyOcclusion(pub u8);

/// Show or hide the sky surfaces by `SkyOcclusion`.
fn apply_setting(setting: Option<Res<SkyOcclusion>>, mut occluders: Query<&mut Visibility, With<SkyOccluder>>) {
    let on = setting.is_some_and(|s| s.0 != 0);
    for mut v in &mut occluders {
        v.set_if_neq(if on { Visibility::Inherited } else { Visibility::Hidden });
    }
}

/// The one material all sky surfaces share.
#[derive(Resource)]
struct OccluderMaterial(Handle<SkyOccluderMaterial>);

/// The sky surfaces as a mesh (a triangle list, three positions each);
/// None without any.
pub fn mesh(positions: &[[f32; 3]]) -> Option<Mesh> {
    let tris = positions.len() / 3;
    if tris == 0 {
        return None;
    }
    let positions = positions[..tris * 3].to_vec();
    let normals: Vec<[f32; 3]> = positions
        .chunks_exact(3)
        .flat_map(|t| {
            let [a, b, c] = [t[0], t[1], t[2]].map(Vec3::from);
            [(b - a).cross(c - a).normalize_or_zero().to_array(); 3]
        })
        .collect();
    let mut mesh = Mesh::new(PrimitiveTopology::TriangleList, default());
    mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, positions);
    mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, normals);
    mesh.insert_indices(Indices::U32((0..tris as u32 * 3).collect()));
    Some(mesh)
}

/// Give new sky surfaces their material.
fn attach_material(
    mut commands: Commands,
    added: Query<Entity, Added<SkyOccluder>>,
    material: Option<Res<OccluderMaterial>>,
    mut materials: Option<ResMut<Assets<SkyOccluderMaterial>>>,
) {
    if added.is_empty() {
        return;
    }
    let Some(materials) = materials.as_mut() else { return };
    let handle = match material {
        Some(m) => m.0.clone(),
        None => {
            let h = materials.add(SkyOccluderMaterial::default());
            commands.insert_resource(OccluderMaterial(h.clone()));
            h
        }
    };
    for e in &added {
        commands.entity(e).insert((
            MeshMaterial3d(handle.clone()),
            // The main view only (not the 3D skybox's camera).
            RenderLayers::layer(0),
            bevy::light::NotShadowCaster,
            bevy::light::NotShadowReceiver,
        ));
    }
}

/// While sky surfaces hide what's behind them, main views draw a depth
/// prepass (the surfaces must be in depth before what they hide).
#[allow(clippy::type_complexity)]
fn prepass_on_main_cameras(
    mut commands: Commands,
    setting: Option<Res<SkyOcclusion>>,
    occluders: Query<(), With<SkyOccluder>>,
    main: Query<
        Entity,
        (
            super::water::MainCameraFilter,
            Without<bevy::core_pipeline::prepass::DepthPrepass>,
        ),
    >,
) {
    if occluders.is_empty() || !setting.is_some_and(|s| s.0 != 0) {
        return;
    }
    for e in &main {
        commands.entity(e).insert(bevy::core_pipeline::prepass::DepthPrepass);
    }
}

pub struct SkyOccluderPlugin;

impl Plugin for SkyOccluderPlugin {
    fn build(&self, app: &mut App) {
        embedded_asset!(app, "sky_occluder.wgsl");
        app.add_plugins(MaterialPlugin::<SkyOccluderMaterial>::default())
            .add_systems(Update, (attach_material, prepass_on_main_cameras, apply_setting));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sky_faces_become_a_triangle_mesh() {
        assert!(mesh(&[]).is_none());
        let quad = [
            [0.0, 0.0, 0.0],
            [1.0, 0.0, 0.0],
            [1.0, 1.0, 0.0],
            [0.0, 0.0, 0.0],
            [1.0, 1.0, 0.0],
            [0.0, 1.0, 0.0],
        ];
        let m = mesh(&quad).unwrap();
        assert_eq!(m.count_vertices(), 6);
        assert_eq!(m.indices().unwrap().len(), 6);
    }
}
