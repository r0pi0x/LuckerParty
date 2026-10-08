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
    base_uv_u: vec4<f32>,
    base_uv_v: vec4<f32>,
}

@group(#{MATERIAL_BIND_GROUP}) @binding(0) var<uniform> params: WorldParamsHead;
@group(#{MATERIAL_BIND_GROUP}) @binding(1) var base_texture: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(2) var base_sampler: sampler;

@fragment
fn fragment(in: VertexOutput) {
#ifdef VERTEX_UVS_A
    // $basetexturetransform without its scroll (the prepass has no time;
    // scrolling alpha-tested surfaces are rare).
    let uv = vec2<f32>(
        dot(params.base_uv_u.xyz, vec3<f32>(in.uv, 1.0)),
        dot(params.base_uv_v.xyz, vec3<f32>(in.uv, 1.0)),
    );
    let a = textureSample(base_texture, base_sampler, uv).a * params.base_color.a;
    if params.alpha_cutoff > 0.0 && a < params.alpha_cutoff {
        discard;
    }
#endif
}
