//! The material for props lit by a light probe: texture x the per-vertex
//! light baked into the mesh's vertex colors, with Source's range fog
//! (the same f^2 curve as the world, see world.wgsl). See prop.wgsl.

use bevy::{
    asset::embedded_asset, prelude::*, reflect::TypePath, render::render_resource::AsBindGroup, shader::ShaderRef,
};

#[derive(Clone, Copy, Debug, Default, bevy::render::render_resource::ShaderType)]
pub struct PropParams {
    pub base_color: Vec4,
    /// Cut out below this alpha (0: off).
    pub alpha_cutoff: f32,
    /// Linear fog color, w = 1 when fog is on.
    pub fog_color: Vec4,
    /// Fog start, end (meters), max density.
    pub fog_range: Vec4,
    /// 1 when alpha blended; otherwise the output alpha is 1 (see
    /// `WorldParams::translucent`).
    pub translucent: f32,
    /// 1 when the prop reflects `envmap` (Source VertexLitGeneric
    /// `$envmap`: no fresnel).
    pub envmap: f32,
    /// 0 no mask, 2 one minus base alpha, 3 `envmap_mask` colour.
    pub envmap_mask: f32,
    pub envmap_contrast: f32,
    pub envmap_saturation: f32,
    /// Linear tint (the model shader converts the material's gamma value).
    pub envmap_tint: Vec4,
}

#[derive(Asset, TypePath, AsBindGroup, Debug, Clone)]
#[bind_group_data(PropMaterialKey)]
pub struct PropMaterial {
    #[uniform(0)]
    pub params: PropParams,
    #[texture(1)]
    #[sampler(2)]
    pub base: Option<Handle<Image>>,
    /// Cube texture sampled with Source-frame (Z-up) directions.
    #[texture(3, dimension = "cube")]
    #[sampler(4)]
    pub envmap: Option<Handle<Image>>,
    #[texture(5)]
    pub envmap_mask: Option<Handle<Image>>,
    pub alpha_mode: AlphaMode,
    pub double_sided: bool,
}

impl Material for PropMaterial {
    fn fragment_shader() -> ShaderRef {
        "embedded://mashup/map/prop.wgsl".into()
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

#[repr(C)]
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct PropMaterialKey {
    double_sided: bool,
}

impl From<&PropMaterial> for PropMaterialKey {
    fn from(m: &PropMaterial) -> Self {
        Self {
            double_sided: m.double_sided,
        }
    }
}

pub struct PropMaterialPlugin;

impl Plugin for PropMaterialPlugin {
    fn build(&self, app: &mut App) {
        embedded_asset!(app, "prop.wgsl");
        app.add_plugins(MaterialPlugin::<PropMaterial>::default());
    }
}
