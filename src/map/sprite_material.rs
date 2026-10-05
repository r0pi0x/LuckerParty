//! The material for sprites (`MapSprite`): every vertex sits at the
//! sprite's centre and the vertex shader spreads the corners along the
//! view's right and up axes, so the quad stays parallel to the view plane.
//! Added to the image (one, one); glows skip the depth test. See sprite.wgsl.

use bevy::{
    asset::{RenderAssetUsages, embedded_asset},
    mesh::Indices,
    prelude::*,
    reflect::TypePath,
    render::render_resource::{
        AsBindGroup, BlendComponent, BlendFactor, BlendOperation, BlendState, CompareFunction, PrimitiveTopology,
    },
    shader::ShaderRef,
};

#[derive(Clone, Copy, Debug, Default, bevy::render::render_resource::ShaderType)]
pub struct SpriteParams {
    /// RGB scale and alpha weight.
    pub color: Vec4,
    /// Full width and height, meters.
    pub size: Vec2,
}

#[derive(Asset, TypePath, AsBindGroup, Debug, Clone)]
#[bind_group_data(SpriteMaterialKey)]
pub struct SpriteMaterial {
    #[uniform(0)]
    pub params: SpriteParams,
    #[texture(1)]
    #[sampler(2)]
    pub texture: Option<Handle<Image>>,
    /// Drawn over everything (glows); otherwise depth tested.
    pub glow: bool,
}

impl Material for SpriteMaterial {
    fn vertex_shader() -> ShaderRef {
        "embedded://mashup/map/sprite.wgsl".into()
    }

    fn fragment_shader() -> ShaderRef {
        "embedded://mashup/map/sprite.wgsl".into()
    }

    fn alpha_mode(&self) -> AlphaMode {
        AlphaMode::Add
    }

    fn specialize(
        _pipeline: &bevy::pbr::MaterialPipeline,
        descriptor: &mut bevy::render::render_resource::RenderPipelineDescriptor,
        _layout: &bevy::mesh::MeshVertexBufferLayoutRef,
        key: bevy::pbr::MaterialPipelineKey<Self>,
    ) -> Result<(), bevy::render::render_resource::SpecializedMeshPipelineError> {
        descriptor.primitive.cull_mode = None;
        let add = BlendComponent {
            src_factor: BlendFactor::One,
            dst_factor: BlendFactor::One,
            operation: BlendOperation::Add,
        };
        if let Some(fragment) = descriptor.fragment.as_mut() {
            for target in fragment.targets.iter_mut().flatten() {
                target.blend = Some(BlendState { color: add, alpha: add });
            }
        }
        if let Some(depth) = descriptor.depth_stencil.as_mut() {
            depth.depth_write_enabled = Some(false);
            if key.bind_group_data.glow {
                depth.depth_compare = Some(CompareFunction::Always);
            }
        }
        Ok(())
    }
}

#[repr(C)]
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct SpriteMaterialKey {
    glow: bool,
}

impl From<&SpriteMaterial> for SpriteMaterialKey {
    fn from(m: &SpriteMaterial) -> Self {
        Self { glow: m.glow }
    }
}

/// A quad whose four vertices sit at the origin; UV_1 holds each corner
/// (-0.5..0.5) for the vertex shader.
pub fn sprite_mesh(texture_size: UVec2) -> Mesh {
    // Texture coordinates inset by half a texel, as the game does.
    let (du, dv) = (0.5 / texture_size.x.max(1) as f32, 0.5 / texture_size.y.max(1) as f32);
    let corners = [[-0.5, -0.5], [-0.5, 0.5], [0.5, 0.5], [0.5, -0.5]];
    let uvs = [[du, 1.0 - dv], [du, dv], [1.0 - du, dv], [1.0 - du, 1.0 - dv]];
    let mut mesh = Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::default());
    mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, vec![[0.0f32; 3]; 4]);
    mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, vec![[0.0f32, 1.0, 0.0]; 4]);
    mesh.insert_attribute(Mesh::ATTRIBUTE_UV_0, uvs.to_vec());
    mesh.insert_attribute(Mesh::ATTRIBUTE_UV_1, corners.to_vec());
    mesh.insert_indices(Indices::U32(vec![0, 1, 2, 0, 2, 3]));
    mesh
}

pub struct SpriteMaterialPlugin;

impl Plugin for SpriteMaterialPlugin {
    fn build(&self, app: &mut App) {
        embedded_asset!(app, "sprite.wgsl");
        app.add_plugins(MaterialPlugin::<SpriteMaterial>::default());
    }
}
