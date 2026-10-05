//! The material for lightmapped world surfaces: texture x baked light, with
//! radiosity normal mapping where the surface has directional lightmaps.
//! See world.wgsl.

use bevy::{
    asset::embedded_asset, prelude::*, reflect::TypePath, render::render_resource::AsBindGroup, shader::ShaderRef,
};

#[derive(Clone, Copy, Debug, Default, bevy::render::render_resource::ShaderType)]
pub struct WorldParams {
    pub base_color: Vec4,
    pub light_scale: f32,
    pub bumped: f32,
    pub normal_g_sign: f32,
    pub alpha_cutoff: f32,
    pub debug_view: f32,
    pub normal_x_sign: f32,
    /// Multiplier on sampled lightmap values (decodes Source's LDR encoding).
    pub lightmap_scale: f32,
}

#[derive(Asset, TypePath, AsBindGroup, Debug, Clone)]
#[bind_group_data(WorldMaterialKey)]
pub struct WorldMaterial {
    #[uniform(0)]
    pub params: WorldParams,
    #[texture(1)]
    #[sampler(2)]
    pub base: Option<Handle<Image>>,
    #[texture(3)]
    #[sampler(4)]
    pub normal: Option<Handle<Image>>,
    #[texture(5)]
    #[sampler(6)]
    pub lightmap: Option<Handle<Image>>,
    #[texture(7)]
    pub lightmap_b0: Option<Handle<Image>>,
    #[texture(8)]
    pub lightmap_b1: Option<Handle<Image>>,
    #[texture(9)]
    pub lightmap_b2: Option<Handle<Image>>,
    pub alpha_mode: AlphaMode,
    pub double_sided: bool,
}

impl Material for WorldMaterial {
    fn fragment_shader() -> ShaderRef {
        "embedded://mashup/map/world.wgsl".into()
    }

    fn alpha_mode(&self) -> AlphaMode {
        self.alpha_mode
    }

    fn specialize(
        _pipeline: &bevy::pbr::MaterialPipeline,
        descriptor: &mut bevy::render::render_resource::RenderPipelineDescriptor,
        _layout: &bevy::mesh::MeshVertexBufferLayoutRef,
        key: bevy::pbr::MaterialPipelineKey<Self>,
    ) -> Result<(), bevy::render::render_resource::SpecializedMeshPipelineError> {
        if key.bind_group_data.double_sided {
            descriptor.primitive.cull_mode = None;
        }
        Ok(())
    }
}

/// Pipeline key data: what changes the pipeline rather than uniforms.
#[repr(C)]
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct WorldMaterialKey {
    double_sided: bool,
}

impl From<&WorldMaterial> for WorldMaterialKey {
    fn from(m: &WorldMaterial) -> Self {
        Self {
            double_sided: m.double_sided,
        }
    }
}

/// Registers the world material and its shader. Needs rendering; maps
/// still load headless without it.
pub struct WorldMaterialPlugin;

impl Plugin for WorldMaterialPlugin {
    fn build(&self, app: &mut App) {
        embedded_asset!(app, "world.wgsl");
        app.add_plugins(MaterialPlugin::<WorldMaterial>::default());
    }
}
