// Depth prepass for alpha-tested props: drop the texels the main pass
// drops (prop.wgsl), including those dithered out in a fade band.

#import bevy_pbr::prepass_io::VertexOutput
#import mashup::dither::dithered_out

// The start of PropParams.
struct PropParamsHead {
    base_color: vec4<f32>,
    alpha_cutoff: f32,
    base_uv_u: vec4<f32>,
    base_uv_v: vec4<f32>,
}

@group(#{MATERIAL_BIND_GROUP}) @binding(0) var<uniform> params: PropParamsHead;
@group(#{MATERIAL_BIND_GROUP}) @binding(1) var base_texture: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(2) var base_sampler: sampler;

@fragment
fn fragment(in: VertexOutput) {
#ifdef VISIBILITY_RANGE_DITHER
    if dithered_out(in.position, in.visibility_range_dither) {
        discard;
    }
#endif
#ifdef VERTEX_UVS_A
    // $basetexturetransform without its scroll (the prepass has no time).
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
