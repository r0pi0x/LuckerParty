// Depth prepass for alpha-tested world surfaces: drop the texels the main
// pass drops (world.wgsl), so the depth behind fences and grates is right.

#import bevy_pbr::prepass_io::VertexOutput

// The start of WorldParams.
struct WorldParamsHead {
    base_color: vec4<f32>,
    light_scale: f32,
    bumped: f32,
    normal_g_sign: f32,
    alpha_cutoff: f32,
}

@group(#{MATERIAL_BIND_GROUP}) @binding(0) var<uniform> params: WorldParamsHead;
@group(#{MATERIAL_BIND_GROUP}) @binding(1) var base_texture: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(2) var base_sampler: sampler;

@fragment
fn fragment(in: VertexOutput) {
#ifdef VERTEX_UVS_A
    let a = textureSample(base_texture, base_sampler, in.uv).a * params.base_color.a;
    if params.alpha_cutoff > 0.0 && a < params.alpha_cutoff {
        discard;
    }
#endif
}
