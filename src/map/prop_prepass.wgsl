// Depth prepass for alpha-tested props: drop the texels the main pass
// drops (prop.wgsl), including those dithered out in a fade band.

#import bevy_pbr::prepass_io::VertexOutput
#import mashup::dither::dithered_out

// The start of PropParams.
struct PropParamsHead {
    base_color: vec4<f32>,
    alpha_cutoff: f32,
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
    let a = textureSample(base_texture, base_sampler, in.uv).a * params.base_color.a;
    if params.alpha_cutoff > 0.0 && a < params.alpha_cutoff {
        discard;
    }
#endif
}
