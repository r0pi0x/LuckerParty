// Props lit by a light probe: texture x the light baked into vertex
// colors, then Source's range fog (f^2), as for world surfaces.

#import bevy_pbr::{
    forward_io::VertexOutput,
    mesh_view_bindings::view,
    view_transformations::position_world_to_view,
}

struct PropParams {
    base_color: vec4<f32>,
    alpha_cutoff: f32,
    fog_color: vec4<f32>,
    fog_range: vec4<f32>,
    translucent: f32,
    envmap: f32,
    envmap_mask: f32,
    envmap_contrast: f32,
    envmap_saturation: f32,
    envmap_tint: vec4<f32>,
}

@group(#{MATERIAL_BIND_GROUP}) @binding(0) var<uniform> params: PropParams;
@group(#{MATERIAL_BIND_GROUP}) @binding(1) var base_texture: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(2) var base_sampler: sampler;
@group(#{MATERIAL_BIND_GROUP}) @binding(3) var envmap_texture: texture_cube<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(4) var envmap_sampler: sampler;
@group(#{MATERIAL_BIND_GROUP}) @binding(5) var envmap_mask_texture: texture_2d<f32>;

const LUMA = vec3<f32>(0.299, 0.587, 0.114);

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
    // $envmap (specs/cs_source/shaders.md 4): the cubemap in the reflected
    // view direction x mask x tint, contrast and saturation; no fresnel.
    if params.envmap > 0.5 {
        let n = normalize(in.world_normal);
        let v = view.world_position - in.world_position.xyz;
        let r = 2.0 * dot(n, v) * n - dot(n, n) * v;
        var spec = textureSample(envmap_texture, envmap_sampler, vec3<f32>(r.x, -r.z, r.y)).rgb;
        if params.envmap_mask > 1.5 && params.envmap_mask < 2.5 {
            spec = spec * (1.0 - color.a);
        } else if params.envmap_mask > 2.5 {
            spec = spec * textureSample(envmap_mask_texture, base_sampler, in.uv).rgb;
        }
        spec = spec * params.envmap_tint.rgb;
        spec = mix(spec, spec * spec, params.envmap_contrast);
        spec = mix(vec3<f32>(dot(spec, LUMA)), spec, params.envmap_saturation);
        rgb = rgb + spec;
    }
    if params.fog_color.w > 0.5 {
        let depth = -position_world_to_view(in.world_position.xyz).z;
        let f = clamp(min(params.fog_range.z, (depth - params.fog_range.x) / (params.fog_range.y - params.fog_range.x)), 0.0, 1.0);
        rgb = mix(rgb, params.fog_color.rgb, f * f);
    }
    return vec4<f32>(rgb, select(1.0, color.a, params.translucent > 0.5));
}
