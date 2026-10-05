// Prop shadows on the world (multiplied over it): the caster's coverage
// blurred over 5 taps, less the distance fade, darkens toward the shadow
// colour; fog fades the shadow out (specs/cs_source/shadows_sky.md D).

#import bevy_pbr::{
    forward_io::VertexOutput,
    view_transformations::position_world_to_view,
}

struct ShadowParams {
    color: vec4<f32>,
    texel: vec2<f32>,
    fog_color: vec4<f32>,
    fog_range: vec4<f32>,
}

@group(#{MATERIAL_BIND_GROUP}) @binding(0) var<uniform> params: ShadowParams;
@group(#{MATERIAL_BIND_GROUP}) @binding(1) var atlas: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(2) var atlas_sampler: sampler;

@fragment
fn fragment(in: VertexOutput) -> @location(0) vec4<f32> {
    let t = params.texel;
    let uv = in.uv;
    let sum = textureSample(atlas, atlas_sampler, uv).r
        + textureSample(atlas, atlas_sampler, uv + t).r
        + textureSample(atlas, atlas_sampler, uv - t).r
        + textureSample(atlas, atlas_sampler, uv + vec2<f32>(t.x, -t.y)).r
        + textureSample(atlas, atlas_sampler, uv + vec2<f32>(-t.x, t.y)).r;
    var fade = 0.0;
#ifdef VERTEX_COLORS
    fade = in.color.r;
#endif
    let coverage = clamp(sum / 5.0 - fade, 0.0, 1.0);
    var result = mix(vec3<f32>(1.0), params.color.rgb, coverage);
    if params.fog_color.w > 0.5 {
        let depth = -position_world_to_view(in.world_position.xyz).z;
        let f = clamp(min(params.fog_range.z, (depth - params.fog_range.x) / (params.fog_range.y - params.fog_range.x)), 0.0, 1.0);
        // The raw fog factor (not the squared blend amount).
        let k = pow(1.0 - f, 4.0);
        result = vec3<f32>(1.0) - (vec3<f32>(1.0) - result) * k;
    }
    return vec4<f32>(result, 1.0);
}
