//! The material for ropes (`MapRope`): the mesh holds each rope point twice
//! (U 0 and 1, the rope's direction as its normal), and the vertex shader
//! pushes the pair apart sideways, facing the camera. Widths follow
//! specs/cs_source/ropes.md, including the fake anti-aliasing pair: a
//! translucent "back" strip at least a fraction of a pixel wide, and the
//! main strip narrowed with distance. See rope.wgsl.

use bevy::{
    asset::{RenderAssetUsages, embedded_asset},
    mesh::Indices,
    prelude::*,
    reflect::TypePath,
    render::render_resource::{AsBindGroup, PrimitiveTopology},
    shader::ShaderRef,
};

use super::MapRope;

#[derive(Clone, Copy, Debug, Default, bevy::render::render_resource::ShaderType)]
pub struct RopeParams {
    /// Full width, meters.
    pub width: f32,
    /// 1 for the translucent back strip, 0 for the rope itself.
    pub back: f32,
    pub light_scale: f32,
    pub has_normal_map: f32,
    /// World and water fog (`fog::SceneFog`).
    pub fog: super::fog::FogUniform,
}

#[derive(Asset, TypePath, AsBindGroup, Debug, Clone)]
#[bind_group_data(RopeMaterialKey)]
pub struct RopeMaterial {
    #[uniform(0)]
    pub params: RopeParams,
    #[texture(1)]
    #[sampler(2)]
    pub base: Option<Handle<Image>>,
    #[texture(3)]
    #[sampler(4)]
    pub normal: Option<Handle<Image>>,
    pub blend: bool,
}

impl Material for RopeMaterial {
    fn vertex_shader() -> ShaderRef {
        "embedded://mashup/map/rope.wgsl".into()
    }

    fn fragment_shader() -> ShaderRef {
        "embedded://mashup/map/rope.wgsl".into()
    }

    fn alpha_mode(&self) -> AlphaMode {
        if self.blend {
            AlphaMode::Blend
        } else {
            AlphaMode::Opaque
        }
    }

    fn specialize(
        _pipeline: &bevy::pbr::MaterialPipeline,
        descriptor: &mut bevy::render::render_resource::RenderPipelineDescriptor,
        _layout: &bevy::mesh::MeshVertexBufferLayoutRef,
        _key: bevy::pbr::MaterialPipelineKey<Self>,
    ) -> Result<(), bevy::render::render_resource::SpecializedMeshPipelineError> {
        // Ropes are $nocull.
        descriptor.primitive.cull_mode = None;
        Ok(())
    }
}

#[repr(C)]
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct RopeMaterialKey {
    blend: bool,
}

impl From<&RopeMaterial> for RopeMaterialKey {
    fn from(m: &RopeMaterial) -> Self {
        Self { blend: m.blend }
    }
}

/// Two vertices per point, with the rope's direction in the normal slot.
pub fn rope_mesh(rope: &MapRope) -> Mesh {
    let n = rope.points.len();
    let mut positions = Vec::with_capacity(n * 2);
    let mut directions = Vec::with_capacity(n * 2);
    let mut uvs = Vec::with_capacity(n * 2);
    let mut colors = Vec::with_capacity(n * 2);
    for i in 0..n {
        let before = rope.points[i.saturating_sub(1)];
        let after = rope.points[(i + 1).min(n - 1)];
        let dir = (after - before).normalize_or(Vec3::X);
        let l = rope.light.get(i).copied().unwrap_or(Vec3::ONE);
        for u in [0.0, 1.0] {
            positions.push(rope.points[i].to_array());
            directions.push(dir.to_array());
            uvs.push([u, rope.v.get(i).copied().unwrap_or(0.0)]);
            colors.push([l.x, l.y, l.z, 1.0]);
        }
    }
    let mut indices = Vec::with_capacity(n.saturating_sub(1) * 6);
    for i in 0..n.saturating_sub(1) as u32 {
        let (a, b, c, d) = (i * 2, i * 2 + 1, i * 2 + 2, i * 2 + 3);
        indices.extend([a, c, b, b, c, d]);
    }
    let mut mesh = Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::default());
    mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, positions);
    mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, directions);
    mesh.insert_attribute(Mesh::ATTRIBUTE_UV_0, uvs);
    mesh.insert_attribute(Mesh::ATTRIBUTE_COLOR, colors);
    mesh.insert_indices(Indices::U32(indices));
    mesh
}

pub struct RopeMaterialPlugin;

impl Plugin for RopeMaterialPlugin {
    fn build(&self, app: &mut App) {
        embedded_asset!(app, "rope.wgsl");
        app.add_plugins((super::fog::FogShaderPlugin, MaterialPlugin::<RopeMaterial>::default()));
    }
}
