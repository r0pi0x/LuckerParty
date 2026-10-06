// World surfaces the way Source's LightmappedGeneric draws them at LDR:
// texture x baked light, plus the scene's point lights (muzzle flashes). With a normal map, the light
// is Valve's radiosity normal mapping ("Shading in Valve's Source Engine",
// SIGGRAPH 2006): three directional lightmaps, weighted per pixel by the
// squared, normalised alignment of the tangent-space normal with three
// fixed basis directions.

#import bevy_pbr::{
    forward_io::VertexOutput,
    mesh_view_bindings::view,
    mesh_view_bindings as view_bindings,
    clustered_forward as clustering,
    view_transformations::position_world_to_view,
}

// The scene's point lights (muzzle flashes; map::DynamicLight), in
// lightmap units: colour x (1 - d^2/r^2) x cosine. Same as prop.wgsl.
fn dynamic_light(p: vec3<f32>, n: vec3<f32>, frag_coord: vec2<f32>) -> vec3<f32> {
    let view_z = dot(vec4<f32>(
        view.view_from_world[0].z,
        view.view_from_world[1].z,
        view.view_from_world[2].z,
        view.view_from_world[3].z
    ), vec4<f32>(p, 1.0));
    let cluster = clustering::view_fragment_cluster_index(frag_coord, view_z, false);
    let ranges = clustering::unpack_clusterable_object_index_ranges(cluster);
    var sum = vec3<f32>(0.0);
    for (var i = ranges.first_point_light_index_offset; i < ranges.first_spot_light_index_offset; i = i + 1u) {
        let light = &view_bindings::clustered_lights.data[clustering::get_clusterable_object_id(i)];
        let to = (*light).position_radius.xyz - p;
        let falloff = saturate(1.0 - dot(to, to) * (*light).color_inverse_square_range.w);
        sum = sum + (*light).color_inverse_square_range.rgb * falloff * max(dot(n, normalize(to)), 0.0);
    }
    return sum;
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
    // Detail texture: 0 none, 1 mod2x, 2 additive, 3 alpha blend.
    detail: f32,
    detail_factor: f32,
    detail_scale: vec2<f32>,
    fog_color: vec4<f32>,
    fog_range: vec4<f32>,
    bicubic: f32,
    translucent: f32,
    envmap: f32,
    // 0 none, 1 normal-map alpha, 2 one minus base alpha, 3 mask texture.
    envmap_mask: f32,
    has_normal: f32,
    envmap_contrast: f32,
    envmap_saturation: f32,
    envmap_fresnel: f32,
    envmap_tint: vec4<f32>,
}

// A lightmap page, bilinear or bicubic B-spline (4 bilinear taps; the
// sampler has already decoded sRGB, as the GPU does in CS:S).
fn sample_lightmap(t: texture_2d<f32>, uv: vec2<f32>) -> vec3<f32> {
    if params.bicubic < 0.5 {
        return textureSampleLevel(t, lightmap_sampler, uv, 0.0).rgb;
    }
    let size = vec2<f32>(textureDimensions(t));
    let p = uv * size - 0.5;
    let i = floor(p);
    let f = p - i;
    let f2 = f * f;
    let f3 = f2 * f;
    let w0 = (-f3 + 3.0 * f2 - 3.0 * f + 1.0) / 6.0;
    let w1 = (3.0 * f3 - 6.0 * f2 + 4.0) / 6.0;
    let w2 = (-3.0 * f3 + 3.0 * f2 + 3.0 * f + 1.0) / 6.0;
    let w3 = f3 / 6.0;
    let g0 = w0 + w1;
    let g1 = w2 + w3;
    let h0 = (i - 1.0 + w1 / g0 + 0.5) / size;
    let h1 = (i + 1.0 + w3 / g1 + 0.5) / size;
    let a = textureSampleLevel(t, lightmap_sampler, vec2<f32>(h0.x, h0.y), 0.0).rgb;
    let b = textureSampleLevel(t, lightmap_sampler, vec2<f32>(h1.x, h0.y), 0.0).rgb;
    let c = textureSampleLevel(t, lightmap_sampler, vec2<f32>(h0.x, h1.y), 0.0).rgb;
    let d = textureSampleLevel(t, lightmap_sampler, vec2<f32>(h1.x, h1.y), 0.0).rgb;
    return g0.y * (g0.x * a + g1.x * b) + g1.y * (g0.x * c + g1.x * d);
}

// Source range fog: toward the fog color by f^2, f from view depth.
// Opaque surfaces write alpha 1 (see `translucent`).
fn out_alpha(a: f32) -> f32 {
    // Additive (2): alpha 0 under premultiplied blending adds the colour.
    if params.translucent > 1.5 {
        return 0.0;
    }
    return select(1.0, a, params.translucent > 0.5);
}

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
@group(#{MATERIAL_BIND_GROUP}) @binding(14) var envmap_texture: texture_cube<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(15) var envmap_sampler: sampler;
@group(#{MATERIAL_BIND_GROUP}) @binding(16) var envmap_mask_texture: texture_2d<f32>;

const LUMA = vec3<f32>(0.299, 0.587, 0.114);

// $envmap (specs/cs_source/shaders.md): the cubemap in the reflected view
// direction, x mask x tint, then contrast, saturation and fresnel. The
// normal follows the normal map through a screen-space tangent frame
// (u and v directions), since world meshes carry no tangents.
fn envmap_term(in: VertexOutput, n_ts: vec3<f32>, base_alpha: f32, normal_alpha: f32) -> vec3<f32> {
    let p = in.world_position.xyz;
    var n = normalize(in.world_normal);
    if params.has_normal > 0.5 {
        let dp1 = dpdx(p);
        let dp2 = dpdy(p);
        let duv1 = dpdx(in.uv);
        let duv2 = dpdy(in.uv);
        let dp2perp = cross(dp2, n);
        let dp1perp = cross(n, dp1);
        let t = dp2perp * duv1.x + dp1perp * duv2.x;
        let b = dp2perp * duv1.y + dp1perp * duv2.y;
        let inv = inverseSqrt(max(max(dot(t, t), dot(b, b)), 1e-12));
        n = normalize(n_ts.x * t * inv + n_ts.y * b * inv + n_ts.z * n);
    }
    let v = view.world_position - p;
    let r = 2.0 * dot(n, v) * n - dot(n, n) * v;
    // Source's cubemap axes: x, -z, y of ours (Z up).
    var spec = textureSample(envmap_texture, envmap_sampler, vec3<f32>(r.x, -r.z, r.y)).rgb;
    var mask = vec3<f32>(1.0);
    if params.envmap_mask > 0.5 && params.envmap_mask < 1.5 {
        mask = vec3<f32>(normal_alpha);
    } else if params.envmap_mask > 1.5 && params.envmap_mask < 2.5 {
        mask = vec3<f32>(1.0 - base_alpha);
    } else if params.envmap_mask > 2.5 {
        mask = textureSample(envmap_mask_texture, base_sampler, in.uv).rgb;
    }
    spec = spec * mask * params.envmap_tint.rgb;
    spec = mix(spec, spec * spec, params.envmap_contrast);
    spec = mix(vec3<f32>(dot(spec, LUMA)), spec, params.envmap_saturation);
    let r0 = params.envmap_fresnel;
    let ndv = saturate(dot(n, normalize(v)));
    spec = spec * (pow(1.0 - ndv, 5.0) * (1.0 - r0) + r0);
    return spec;
}

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
    // change); additive adds the decoded texel; translucent blends over.
    if params.detail > 0.5 {
        let d = textureSample(detail_texture, base_sampler, in.uv * params.detail_scale);
        if params.detail < 1.5 {
            albedo = vec4<f32>(albedo.rgb * mix(vec3<f32>(1.0), 2.0 * d.rgb, params.detail_factor), albedo.a);
        } else if params.detail < 2.5 {
            albedo = vec4<f32>(albedo.rgb + params.detail_factor * d.rgb, albedo.a);
        } else if params.detail < 3.5 {
            // Translucent detail (mode 2): the decoded detail over the base
            // by its own alpha.
            albedo = vec4<f32>(mix(albedo.rgb, d.rgb, d.a * params.detail_factor), albedo.a);
        } else if params.detail < 4.5 {
            // WorldTwoTextureBlend, detail over base: the raw detail texel.
            albedo = vec4<f32>(mix(albedo.rgb, d.rgb, d.a), albedo.a);
        } else {
            // WorldTwoTextureBlend's 2x grime mask: the detail is the
            // surface, darkened by twice the decoded base where the
            // detail's alpha says so. Alpha stays the base's.
            let k = saturate(2.0 * albedo.rgb);
            let m = saturate(k * d.a + (1.0 - d.a));
            albedo = vec4<f32>(m * d.rgb, albedo.a);
        }
    }
    // WorldTwoTextureBlend lights with one bilinear lightmap tap times a
    // fixed 2.0, not 2^2.2; its bump uses the detail coordinates and
    // unsquared weights.
    let two_texture = params.detail > 3.5;
    if params.alpha_cutoff > 0.0 && albedo.a < params.alpha_cutoff {
        discard;
    }
    var light = sample_lightmap(lightmap, in.uv_b);
    if two_texture {
        light = textureSampleLevel(lightmap, lightmap_sampler, in.uv_b, 0.0).rgb;
    }
    // The normal map (tangent space, signs fixed), for bump lighting and
    // reflections; its alpha can mask reflections.
    let normal_texel = textureSample(normal_texture, normal_sampler, in.uv);
    var n_ts = normal_texel.xyz * 2.0 - 1.0;
    n_ts.x = n_ts.x * params.normal_x_sign;
    n_ts.y = n_ts.y * params.normal_g_sign;
    if params.bumped > 0.5 {
        let bump_uv = select(in.uv, in.uv * params.detail_scale, two_texture);
        var n = textureSample(normal_texture, normal_sampler, bump_uv).xyz * 2.0 - 1.0;
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
        if !two_texture {
            w = w * w;
        }
        w = w / max(w.x + w.y + w.z, 1e-4);
        light = w.x * sample_lightmap(lightmap_b0, in.uv_b)
            + w.y * sample_lightmap(lightmap_b1, in.uv_b)
            + w.z * sample_lightmap(lightmap_b2, in.uv_b);
    }
    light = light * params.lightmap_scale;
    if two_texture {
        // The usual scale is 2^2.2 for Source's LDR lightmaps; this shader
        // uses 2.0.
        light = light * (2.0 / 4.594793);
    }
    if params.debug_view == 1.0 {
        albedo = vec4<f32>(1.0, 1.0, 1.0, albedo.a);
        light = light * 0.25;
        return vec4<f32>(albedo.rgb * light, out_alpha(albedo.a));
    }
    if params.debug_view == 2.0 {
        return albedo;
    }
    light = light + dynamic_light(in.world_position.xyz, normalize(in.world_normal), in.position.xy);
    var color = albedo.rgb * light * params.light_scale;
    if params.envmap > 0.5 {
        color = color + envmap_term(in, n_ts, albedo.a, normal_texel.a);
    }
    return vec4<f32>(apply_fog(color, in.world_position.xyz), out_alpha(albedo.a));
}
