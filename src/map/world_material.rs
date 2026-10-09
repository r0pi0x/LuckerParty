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
    /// `$basetexturetransform` rows (m0, m1, m2, scroll), see
    /// `MapUvTransform::shader_rows`; world_prepass.wgsl reads them too.
    pub base_uv_u: Vec4,
    pub base_uv_v: Vec4,
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
    /// Detail texture: 0 none, 1 + Source's `$detailblendmode`, 20 and 21
    /// WorldTwoTextureBlend's modes (`DetailMode::shader_value`).
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
    /// 1 when the surface reflects `envmap` (Source `$envmap`); 2 when it
    /// shows it in the view direction, unlit (WindowImposter).
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
    /// Under water (map::water): the water's range fog, linear colour (w = 1
    /// when on), for points below `water_fog_range.w` (the surface plus
    /// the fudge); start, end (meters), max density in xyz.
    pub water_fog_color: Vec4,
    pub water_fog_range: Vec4,
    /// `$basetexturetransform2` rows (the second layer's coordinates).
    pub base2_uv_u: Vec4,
    pub base2_uv_v: Vec4,
    /// `$detailtint` (linear multiplier on the detail texel).
    pub detail_tint: Vec4,
    /// 1 with `$selfillum`: the base alpha lerps toward `selfillum_tint` x
    /// albedo, unlit.
    pub selfillum: f32,
    pub selfillum_tint: Vec4,
    /// 1: drawn at the texture's own brightness times `unlit_tint` (an
    /// UnlitGeneric material on a brush, or the `$emissiveblend` stand-in),
    /// not lit by the lightmap.
    pub unlit: f32,
    pub unlit_tint: Vec4,
}

impl WorldParams {
    /// Identity texture transforms and a white detail tint, for params
    /// built field by field.
    pub fn identity_uv() -> Self {
        let [u, v] = super::MapUvTransform::IDENTITY.shader_rows();
        Self {
            base_uv_u: u,
            base_uv_v: v,
            base2_uv_u: u,
            base2_uv_v: v,
            detail_tint: Vec4::ONE,
            selfillum_tint: Vec4::ONE,
            unlit_tint: Vec4::ONE,
            ..Default::default()
        }
    }
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
    /// A map overlay or decal lying on a surface: drawn right after the
    /// opaque world (alpha-mask phase) with its own blend and no depth
    /// writes, so everything see-through (particles, smoke, glows, muzzle
    /// flashes) draws over it rather than being sorted against it.
    pub decal: bool,
}

impl Material for WorldMaterial {
    fn fragment_shader() -> ShaderRef {
        "embedded://mashup/map/world.wgsl".into()
    }

    /// Alpha-tested surfaces count as masked, so the depth prepass (main
    /// views with water) runs `world_prepass.wgsl` and drops the same texels.
    fn alpha_mode(&self) -> AlphaMode {
        if self.decal {
            return AlphaMode::Mask(0.001);
        }
        if self.alpha_mode == AlphaMode::Opaque && self.params.alpha_cutoff > 0.0 {
            return AlphaMode::Mask(self.params.alpha_cutoff);
        }
        self.alpha_mode
    }

    fn prepass_fragment_shader() -> ShaderRef {
        "embedded://mashup/map/world_prepass.wgsl".into()
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
        // Decals: blended in the alpha-mask phase (see `decal`).
        let blend = match key.bind_group_data.decal_blend {
            1 => Some(bevy::render::render_resource::BlendState::ALPHA_BLENDING),
            2 => Some(bevy::render::render_resource::BlendState {
                color: bevy::render::render_resource::BlendComponent {
                    src_factor: bevy::render::render_resource::BlendFactor::SrcAlpha,
                    dst_factor: bevy::render::render_resource::BlendFactor::One,
                    operation: bevy::render::render_resource::BlendOperation::Add,
                },
                alpha: bevy::render::render_resource::BlendComponent::OVER,
            }),
            _ => None,
        };
        if blend.is_some() {
            if let Some(fragment) = descriptor.fragment.as_mut() {
                for target in fragment.targets.iter_mut().flatten() {
                    target.blend = blend;
                }
            }
            if let Some(depth) = descriptor.depth_stencil.as_mut() {
                depth.depth_write_enabled = Some(false);
            }
        }
        Ok(())
    }
}

/// Pipeline key data: what changes the pipeline rather than uniforms.
#[repr(C)]
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct WorldMaterialKey {
    double_sided: bool,
    /// Decals only: 1 alpha blend, 2 additive (0: not a decal).
    decal_blend: u8,
}

impl From<&WorldMaterial> for WorldMaterialKey {
    fn from(m: &WorldMaterial) -> Self {
        Self {
            double_sided: m.double_sided,
            decal_blend: match (m.decal, m.alpha_mode) {
                (false, _) => 0,
                (true, AlphaMode::Add) => 2,
                (true, _) => 1,
            },
        }
    }
}

/// Registers the world material and its shader. Needs rendering; maps
/// still load headless without it.
pub struct WorldMaterialPlugin;

impl Plugin for WorldMaterialPlugin {
    fn build(&self, app: &mut App) {
        embedded_asset!(app, "world.wgsl");
        embedded_asset!(app, "world_prepass.wgsl");
        app.add_plugins((super::fog::FogShaderPlugin, MaterialPlugin::<WorldMaterial>::default()));
    }
}
