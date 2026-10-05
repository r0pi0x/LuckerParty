// Props lit by a light probe: texture x the light baked into vertex
// colors, then Source's range fog (f^2), as for world surfaces.

#import bevy_pbr::{
    forward_io::VertexOutput,
    view_transformations::position_world_to_view,
}

struct PropParams {
    base_color: vec4<f32>,
    alpha_cutoff: f32,
    fog_color: vec4<f32>,
    fog_range: vec4<f32>,
    translucent: f32,
}

@group(#{MATERIAL_BIND_GROUP}) @binding(0) var<uniform> params: PropParams;
@group(#{MATERIAL_BIND_GROUP}) @binding(1) var base_texture: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(2) var base_sampler: sampler;

@fragment
fn fragment(in: VertexOutput) -> @location(0) vec4<f32> {
    var color = textureSample(base_texture, base_sampler, in.uv) * params.base_color;
    if params.alpha_cutoff > 0.0 && color.a < params.alpha_cutoff {
        discard;
    }
#ifdef VERTEX_COLORS
    color = vec4<f32>(color.rgb * in.color.rgb, color.a);
#endif
    var rgb = color.rgb;
    if params.fog_color.w > 0.5 {
        let depth = -position_world_to_view(in.world_position.xyz).z;
        let f = clamp(min(params.fog_range.z, (depth - params.fog_range.x) / (params.fog_range.y - params.fog_range.x)), 0.0, 1.0);
        rgb = mix(rgb, params.fog_color.rgb, f * f);
    }
    return vec4<f32>(rgb, select(1.0, color.a, params.translucent > 0.5));
}
