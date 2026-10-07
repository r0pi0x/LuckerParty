//! The material for props lit by a light probe: texture x the per-vertex
//! light baked into the mesh's vertex colors (static props), or x the
//! probe evaluated per pixel from uniforms (moving models: view models),
//! plus the scene's point lights (muzzle flashes), with Source's range fog
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
    /// 1 when alpha blended, 2 additive; otherwise the output alpha is 1 (see
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
    /// 1: add the scene's point lights (`DynamicLight`s), see
    /// `dynamic_light` in prop.wgsl.
    pub dynamic: f32,
    /// 1: lit per pixel by the probe below instead of vertex colours.
    pub probe: f32,
    /// `LightProbe::cube` (+X -X +Y -Y +Z -Z), lightmap units.
    pub probe_cube: [Vec4; 6],
    /// Up to four directional lights: direction toward the light (xyz)
    /// and colour; unused ones are zero.
    pub probe_light_dir: [Vec4; 4],
    pub probe_light_color: [Vec4; 4],
    /// Under water (map::water): the water's range fog, linear colour (w = 1
    /// when on), for points below `water_fog_range.w` (the surface plus
    /// the fudge); start, end (meters), max density in xyz.
    pub water_fog_color: Vec4,
    pub water_fog_range: Vec4,
}

impl PropParams {
    /// Light the material per pixel with `probe` (times `scale`).
    pub fn set_probe(&mut self, probe: &super::LightProbe, scale: f32) {
        self.probe = 1.0;
        for (c, v) in self.probe_cube.iter_mut().zip(probe.cube) {
            *c = (v * scale).extend(0.0);
        }
        let mut lights: Vec<&(Vec3, Vec3)> = probe.lights.iter().collect();
        lights.sort_by(|a, b| b.1.max_element().total_cmp(&a.1.max_element()));
        for i in 0..4 {
            let (dir, color) = lights.get(i).map_or((Vec3::ZERO, Vec3::ZERO), |l| (l.0, l.1 * scale));
            self.probe_light_dir[i] = dir.extend(0.0);
            self.probe_light_color[i] = color.extend(0.0);
        }
    }
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
    /// Cull front faces instead of back ones (drawn mirrored).
    pub cull_front: bool,
}

impl Material for PropMaterial {
    fn fragment_shader() -> ShaderRef {
        "embedded://mashup/map/prop.wgsl".into()
    }

    /// Alpha-tested surfaces count as masked, so the depth prepass (main
    /// views with water) runs `prop_prepass.wgsl` and drops the same texels.
    fn alpha_mode(&self) -> AlphaMode {
        if self.alpha_mode == AlphaMode::Opaque && self.params.alpha_cutoff > 0.0 {
            return AlphaMode::Mask(self.params.alpha_cutoff);
        }
        self.alpha_mode
    }

    fn prepass_fragment_shader() -> ShaderRef {
        "embedded://mashup/map/prop_prepass.wgsl".into()
    }

    fn specialize(
        _pipeline: &bevy::pbr::MaterialPipeline,
        descriptor: &mut bevy::render::render_resource::RenderPipelineDescriptor,
        _layout: &bevy::mesh::MeshVertexBufferLayoutRef,
        key: bevy::pbr::MaterialPipelineKey<Self>,
    ) -> Result<(), bevy::render::render_resource::SpecializedMeshPipelineError> {
        // Opaque props get a depth-only prepass (no fragment stage); inside
        // a fade band it must drop the dithered-out pixels too.
        if descriptor.fragment.is_none()
            && key
                .mesh_key
                .contains(bevy::pbr::MeshPipelineKey::VISIBILITY_RANGE_DITHER)
        {
            descriptor.fragment = Some(bevy::render::render_resource::FragmentState {
                shader: DITHER_PREPASS_SHADER,
                shader_defs: descriptor.vertex.shader_defs.clone(),
                entry_point: Some("fragment".into()),
                targets: Vec::new(),
            });
        }
        if key.bind_group_data.double_sided {
            descriptor.primitive.cull_mode = None;
        } else if key.bind_group_data.cull_front {
            descriptor.primitive.cull_mode = Some(bevy::render::render_resource::Face::Front);
        }
        Ok(())
    }
}

#[repr(C)]
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct PropMaterialKey {
    double_sided: bool,
    cull_front: bool,
}

impl From<&PropMaterial> for PropMaterialKey {
    fn from(m: &PropMaterial) -> Self {
        Self {
            double_sided: m.double_sided,
            cull_front: m.cull_front,
        }
    }
}

/// `prop_dither_prepass.wgsl`: the depth prepass fragment stage for opaque
/// props in a fade band (see `specialize`).
const DITHER_PREPASS_SHADER: Handle<Shader> = bevy::asset::uuid_handle!("6f3d2b8e-5c1a-4e7f-9a42-1d8b7c3e5f60");

pub struct PropMaterialPlugin;

impl Plugin for PropMaterialPlugin {
    fn build(&self, app: &mut App) {
        embedded_asset!(app, "prop.wgsl");
        embedded_asset!(app, "prop_prepass.wgsl");
        bevy::shader::load_shader_library!(app, "dither.wgsl");
        bevy::asset::load_internal_asset!(app, DITHER_PREPASS_SHADER, "prop_dither_prepass.wgsl", Shader::from_wgsl);
        app.add_plugins((super::fog::FogShaderPlugin, MaterialPlugin::<PropMaterial>::default()));
    }
}
