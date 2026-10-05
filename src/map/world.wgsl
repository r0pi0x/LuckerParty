// World surfaces the way Source's LightmappedGeneric draws them at LDR:
// texture x baked light, no dynamic lighting. With a normal map, the light
// is Valve's radiosity normal mapping ("Shading in Valve's Source Engine",
// SIGGRAPH 2006): three directional lightmaps, weighted per pixel by the
// squared, normalised alignment of the tangent-space normal with three
// fixed basis directions.

#import bevy_pbr::{
    forward_io::VertexOutput,
    view_transformations::position_world_to_view,
}

struct WorldParams {
    base_color: vec4<f32>,
    light_scale: f32,
    // 0 = flat lightmap only, 1 = radiosity normal mapping.
    bumped: f32,
    // Sign applied to the normal map's green channel.
    normal_g_sign: f32,
    alpha_cutoff: f32,
    // 0 normal, 1 lighting only, 2 albedo only.
    debug_view: f32,
    // Sign applied to the normal map's red channel.
    normal_x_sign: f32,
    // Multiplier on sampled lightmap values.
    lightmap_scale: f32,
    // Second texture blended in by the vertex alpha (WorldVertexTransition).
    blend: f32,
    blend_masked: f32,
    blend_normal: f32,
    // Detail texture: 0 none, 1 mod2x, 2 additive.
    detail: f32,
    detail_factor: f32,
    detail_scale: vec2<f32>,
    fog_color: vec4<f32>,
    fog_range: vec4<f32>,
}

// Source range fog: toward the fog color by f^2, f from view depth.
fn apply_fog(color: vec3<f32>, world: vec3<f32>) -> vec3<f32> {
    if params.fog_color.w < 0.5 {
        return color;
    }
    let depth = -position_world_to_view(world).z;
    let f = clamp(min(params.fog_range.z, (depth - params.fog_range.x) / (params.fog_range.y - params.fog_range.x)), 0.0, 1.0);
    return mix(color, params.fog_color.rgb, f * f);
}

@group(#{MATERIAL_BIND_GROUP}) @binding(0) var<uniform> params: WorldParams;
@group(#{MATERIAL_BIND_GROUP}) @binding(1) var base_texture: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(2) var base_sampler: sampler;
@group(#{MATERIAL_BIND_GROUP}) @binding(3) var normal_texture: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(4) var normal_sampler: sampler;
@group(#{MATERIAL_BIND_GROUP}) @binding(5) var lightmap: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(6) var lightmap_sampler: sampler;
@group(#{MATERIAL_BIND_GROUP}) @binding(7) var lightmap_b0: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(8) var lightmap_b1: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(9) var lightmap_b2: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(10) var base2_texture: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(11) var normal2_texture: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(12) var blend_mask: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(13) var detail_texture: texture_2d<f32>;

// Basis directions of the three directional lightmap pages, in order
// (specs/cs_source/shaders.md). Tangent space: x along texture u, y along
// v (image-down), z out of the surface.
const BASIS_0 = vec3<f32>(0.81649658, 0.0, 0.57735027);
const BASIS_1 = vec3<f32>(-0.40824829, 0.70710678, 0.57735027);
const BASIS_2 = vec3<f32>(-0.40824829, -0.70710678, 0.57735027);

@fragment
fn fragment(in: VertexOutput) -> @location(0) vec4<f32> {
    var albedo = textureSample(base_texture, base_sampler, in.uv) * params.base_color;
    // WorldVertexTransition: blend toward the second texture by the vertex
    // alpha, optionally shaped by the mask (green: transition point, red:
    // softness). Colors blend in linear space; alpha stays the first's.
    var b = 0.0;
#ifdef VERTEX_COLORS
    b = in.color.a;
#endif
    if params.blend > 0.5 {
        if params.blend_masked > 0.5 {
            let m = textureSample(blend_mask, normal_sampler, in.uv);
            let lo = saturate(m.g - m.r);
            let hi = saturate(m.g + m.r);
            let t = select(step(lo, b), saturate((b - lo) / (hi - lo)), hi > lo);
            b = t * t * (3.0 - 2.0 * t);
        }
        let second = textureSample(base2_texture, base_sampler, in.uv) * params.base_color;
        albedo = vec4<f32>(mix(albedo.rgb, second.rgb, b), albedo.a);
    }
    // $detail: mod2x multiplies by twice the raw texel (mid-grey = no
    // change); additive adds the decoded texel.
    if params.detail > 0.5 {
        let d = textureSample(detail_texture, base_sampler, in.uv * params.detail_scale);
        if params.detail < 1.5 {
            albedo = vec4<f32>(albedo.rgb * mix(vec3<f32>(1.0), 2.0 * d.rgb, params.detail_factor), albedo.a);
        } else {
            albedo = vec4<f32>(albedo.rgb + params.detail_factor * d.rgb, albedo.a);
        }
    }
    if params.alpha_cutoff > 0.0 && albedo.a < params.alpha_cutoff {
        discard;
    }
    var light = textureSample(lightmap, lightmap_sampler, in.uv_b).rgb;
    if params.bumped > 0.5 {
        var n = textureSample(normal_texture, normal_sampler, in.uv).xyz * 2.0 - 1.0;
        if params.blend > 0.5 && params.blend_normal > 0.5 {
            let n2 = textureSample(normal2_texture, normal_sampler, in.uv).xyz * 2.0 - 1.0;
            n = mix(n, n2, b);
        }
        n.x = n.x * params.normal_x_sign;
        n.y = n.y * params.normal_g_sign;
        var w = vec3<f32>(
            saturate(dot(n, BASIS_0)),
            saturate(dot(n, BASIS_1)),
            saturate(dot(n, BASIS_2)),
        );
        w = w * w;
        w = w / max(w.x + w.y + w.z, 1e-4);
        light = w.x * textureSample(lightmap_b0, lightmap_sampler, in.uv_b).rgb
            + w.y * textureSample(lightmap_b1, lightmap_sampler, in.uv_b).rgb
            + w.z * textureSample(lightmap_b2, lightmap_sampler, in.uv_b).rgb;
    }
    light = light * params.lightmap_scale;
    if params.debug_view == 1.0 {
        albedo = vec4<f32>(1.0, 1.0, 1.0, albedo.a);
        light = light * 0.25;
        return vec4<f32>(albedo.rgb * light, albedo.a);
    }
    if params.debug_view == 2.0 {
        return albedo;
    }
    return vec4<f32>(apply_fog(albedo.rgb * light * params.light_scale, in.world_position.xyz), albedo.a);
}
