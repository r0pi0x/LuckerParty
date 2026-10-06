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
    /// 1 when a second texture is blended in by the vertex alpha.
    pub blend: f32,
    /// 1 when `blend_mask` shapes that blend.
    pub blend_masked: f32,
    /// 1 when the second layer has its own normal map.
    pub blend_normal: f32,
    /// Detail texture: 0 none, 1 mod2x, 2 additive, 3 alpha blend.
    pub detail: f32,
    pub detail_factor: f32,
    pub detail_scale: Vec2,
    /// Range fog: linear color (w = 1 when on), and start, end (meters),
    /// max density.
    pub fog_color: Vec4,
    pub fog_range: Vec4,
    /// 1: bicubic lightmap sampling.
    pub bicubic: f32,
    /// 1 when alpha blended, 2 when additive (output alpha 0, which adds
    /// under premultiplied blending). Otherwise the output alpha is 1: textures
    /// often keep other data in alpha (env map masks), and the camera's
    /// output is composited over the sky camera by alpha.
    pub translucent: f32,
    /// 1 when the surface reflects `envmap` (Source `$envmap`).
    pub envmap: f32,
    /// What scales the reflection: 0 nothing, 1 normal-map alpha, 2 one
    /// minus base alpha, 3 `envmap_mask` colour.
    pub envmap_mask: f32,
    /// 1 when `normal` is bound (reflections follow the normal map).
    pub has_normal: f32,
    pub envmap_contrast: f32,
    pub envmap_saturation: f32,
    /// Fresnel R0; 1 = none.
    pub envmap_fresnel: f32,
    pub envmap_tint: Vec4,
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
    /// Second layer (WorldVertexTransition), sampled like `base`.
    #[texture(10)]
    pub base2: Option<Handle<Image>>,
    #[texture(11)]
    pub normal2: Option<Handle<Image>>,
    #[texture(12)]
    pub blend_mask: Option<Handle<Image>>,
    #[texture(13)]
    pub detail: Option<Handle<Image>>,
    /// Cube texture sampled with Source-frame (Z-up) directions.
    #[texture(14, dimension = "cube")]
    #[sampler(15)]
    pub envmap: Option<Handle<Image>>,
    #[texture(16)]
    pub envmap_mask: Option<Handle<Image>>,
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
