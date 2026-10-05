// World surfaces the way Source's LightmappedGeneric draws them at LDR:
// texture x baked light, no dynamic lighting. With a normal map, the light
// is Valve's radiosity normal mapping ("Shading in Valve's Source Engine",
// SIGGRAPH 2006): three directional lightmaps, weighted per pixel by the
// squared, normalised alignment of the tangent-space normal with three
// fixed basis directions.

#import bevy_pbr::forward_io::VertexOutput

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

const BASIS_0 = vec3<f32>(-0.40824829, 0.70710678, 0.57735027);
const BASIS_1 = vec3<f32>(-0.40824829, -0.70710678, 0.57735027);
const BASIS_2 = vec3<f32>(0.81649658, 0.0, 0.57735027);

@fragment
fn fragment(in: VertexOutput) -> @location(0) vec4<f32> {
    var albedo = textureSample(base_texture, base_sampler, in.uv) * params.base_color;
    if params.alpha_cutoff > 0.0 && albedo.a < params.alpha_cutoff {
        discard;
    }
    var light = textureSample(lightmap, lightmap_sampler, in.uv_b).rgb;
    if params.bumped > 0.5 {
        var n = textureSample(normal_texture, normal_sampler, in.uv).xyz * 2.0 - 1.0;
        n.x = n.x * params.normal_x_sign;
        n.y = n.y * params.normal_g_sign;
        n = normalize(n);
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
    if params.debug_view == 1.0 {
        albedo = vec4<f32>(1.0, 1.0, 1.0, albedo.a);
        light = light * 0.25;
        return vec4<f32>(albedo.rgb * light, albedo.a);
    }
    if params.debug_view == 2.0 {
        return albedo;
    }
    return vec4<f32>(albedo.rgb * light * params.light_scale, albedo.a);
}
