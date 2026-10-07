// Water surfaces the way Source's Water shader draws them at DX9 LDR
// (specs/cs_source/water.md). Pass 1 (refraction and/or planar
// reflection, Fresnel-blended) and pass 2 (the cheap cubemap reflection,
// blended over in gamma space) are computed here in one go.
//
// The refraction texture is the main view's opaque scene (Bevy's
// transmission copy); its fog alpha (the original's height fog written by
// the refraction view) is rebuilt from the depth prepass: the point behind
// each pixel, how much of the eye's path to it is under water and how far.

#import bevy_pbr::{
    forward_io::VertexOutput,
    mesh_view_bindings as vb,
    mesh_view_bindings::{view, globals},
    view_transformations::{position_world_to_view, position_ndc_to_world},
}

struct WaterParams {
    fog_linear: vec4<f32>,
    fog_gamma: vec4<f32>,
    reflect_tint_linear: vec4<f32>,
    reflect_tint_raw: vec4<f32>,
    scene_fog_linear: vec4<f32>,
    scene_fog_gamma: vec4<f32>,
    scene_fog_range: vec4<f32>,
    // First layer scroll (xy), frame rate (z), frame count (w).
    bump: vec4<f32>,
    // scroll1 (xy), scroll2 (zw).
    scrolls: vec4<f32>,
    refract_amount: f32,
    reflect_amount: f32,
    fog_start: f32,
    fog_end: f32,
    cheap_start: f32,
    cheap_end: f32,
    above_water: f32,
    refract: f32,
    reflect: f32,
    cheap_pass: f32,
    force_cheap: f32,
    fixed_weight: f32,
    cheap_mode: f32,
    fudge: f32,
    sky_env: f32,
    prefog_plane: f32,
    refract_fog_only: f32,
}

@group(#{MATERIAL_BIND_GROUP}) @binding(0) var<uniform> params: WaterParams;
@group(#{MATERIAL_BIND_GROUP}) @binding(1) var normal_texture: texture_2d_array<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(2) var normal_sampler: sampler;
@group(#{MATERIAL_BIND_GROUP}) @binding(3) var reflection_texture: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(4) var reflection_sampler: sampler;
@group(#{MATERIAL_BIND_GROUP}) @binding(5) var envmap_texture: texture_cube<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(6) var envmap_sampler: sampler;

fn srgb_encode(c: vec3<f32>) -> vec3<f32> {
    let x = max(c, vec3<f32>(0.0));
    return select(1.055 * pow(x, vec3<f32>(1.0 / 2.4)) - 0.055, 12.92 * x, x <= vec3<f32>(0.0031308));
}

fn srgb_decode(c: vec3<f32>) -> vec3<f32> {
    let x = max(c, vec3<f32>(0.0));
    return select(pow((x + 0.055) / 1.055, vec3<f32>(2.4)), x / 12.92, x <= vec3<f32>(0.04045));
}

// Depth behind a screen position from the depth prepass: 0 where nothing
// was drawn, -1 without a prepass.
fn scene_depth(uv: vec2<f32>) -> f32 {
#ifdef DEPTH_PREPASS
    let size = vec2<f32>(textureDimensions(vb::depth_prepass_texture));
    let p = vec2<i32>(clamp(uv * size, vec2<f32>(0.0), size - 1.0));
    return textureLoad(vb::depth_prepass_texture, p, 0);
#else
    return -1.0;
#endif
}

// The refraction texture at `uv`: colour (linear) already moved toward the
// water fog by the height fog, and that fog factor (spec section 8). Where
// nothing below the surface was drawn, the clear: fog colour, alpha 1.
fn refraction(uv: vec2<f32>, surface: f32) -> vec4<f32> {
    if params.refract_fog_only > 0.5 {
        return vec4<f32>(params.fog_linear.rgb, 1.0);
    }
    let c = textureSampleLevel(vb::view_transmission_texture, vb::view_transmission_sampler, uv, 0.0).rgb;
    if params.above_water < 0.5 {
        return vec4<f32>(c, 1.0);
    }
    let d = scene_depth(uv);
    if d < 0.0 {
        return vec4<f32>(c, 1.0);
    }
    if d <= 0.0 {
        return vec4<f32>(params.fog_linear.rgb, 1.0);
    }
    let p = position_ndc_to_world(vec3<f32>(uv.x * 2.0 - 1.0, 1.0 - uv.y * 2.0, d));
    let plane = surface + params.fudge;
    if p.y > plane {
        return vec4<f32>(params.fog_linear.rgb, 1.0);
    }
    let z = -position_world_to_view(p).z;
    let k = 1.0 / max(params.fog_end - params.fog_start, 1e-4);
    let path = view.world_position.y - p.y;
    let h = select(1.0, saturate((plane - p.y) / path), path > 0.0);
    let f = saturate(h * z * k);
    // The near plane crosses this surface: what lies below was drawn with
    // this same fog already (map::fog, the intersection view).
    if abs(surface - params.prefog_plane) < 0.02 {
        return vec4<f32>(c, f);
    }
    return vec4<f32>(mix(c, params.fog_linear.rgb, f), f);
}

// Source range fog on the surface: toward the fog colour by f^2.
fn range_fog(depth: f32) -> f32 {
    if params.scene_fog_linear.w < 0.5 {
        return 0.0;
    }
    let r = params.scene_fog_range;
    let f = clamp(min(r.z, (depth - r.x) / (r.y - r.x)), 0.0, 1.0);
    return f * f;
}

@fragment
fn fragment(in: VertexOutput) -> @location(0) vec4<f32> {
    let p = in.world_position.xyz;
    let t = globals.time;
    // Tangent frame from the texture coordinates (water brushes carry no
    // tangents): x along u, y along v, normalised (spec open question 11).
    let geo_n = normalize(in.world_normal);
    let dp1 = dpdx(p);
    let dp2 = dpdy(p);
    let duv1 = dpdx(in.uv);
    let duv2 = dpdy(in.uv);
    let dp2perp = cross(dp2, geo_n);
    let dp1perp = cross(geo_n, dp1);
    let tangent = normalize(dp2perp * duv1.x + dp1perp * duv2.x + vec3<f32>(1e-8, 0.0, 0.0));
    let bitangent = normalize(dp2perp * duv1.y + dp1perp * duv2.y + vec3<f32>(0.0, 0.0, 1e-8));

    // Normal samples (section 4): the animated frame; pass 1 scrolls the
    // first layer, pass 2 doesn't.
    let count = max(params.bump.w, 1.0);
    let layer = i32(floor(max(t * params.bump.z, 0.0)) % count);
    let shift = fract(params.bump.xy * t);
    var s1 = textureSample(normal_texture, normal_sampler, in.uv + shift, layer);
    var s2 = textureSample(normal_texture, normal_sampler, in.uv, layer);
    let uv1 = vec2<f32>(0.1 * (in.uv.x + in.uv.y), 0.1 * (in.uv.y - in.uv.x)) + t * params.scrolls.xy;
    let uv2 = vec2<f32>(0.45 * in.uv.y, 0.45 * in.uv.x) + t * params.scrolls.zw;
    let l1 = textureSample(normal_texture, normal_sampler, uv1, layer);
    let l2 = textureSample(normal_texture, normal_sampler, uv2, layer);
    if abs(params.scrolls.x) > 0.0 {
        s1 = 0.33 * (s1 + l1 + l2);
        s2 = 0.33 * (s2 + l1 + l2);
    }
    let n1 = s1.xyz * 2.0 - 1.0;
    let n2 = s2.xyz * 2.0 - 1.0;
    // Not renormalised (spec).
    let nw1 = n1.x * tangent + n1.y * bitangent + n1.z * geo_n;
    let nw2 = n2.x * tangent + n2.y * bitangent + n2.z * geo_n;

    let eye = view.world_position;
    let to_eye = eye - p;
    let dist = length(to_eye);
    let e = to_eye / max(dist, 1e-6);
    let depth = -position_world_to_view(p).z;
    let fog_f2 = range_fog(depth);

    // Screen positions: refraction v down, reflection v up (section 5).
    let screen = (in.position.xy - view.viewport.xy) / view.viewport.zw;
    let refract_base = screen;
    let reflect_base = vec2<f32>(screen.x, 1.0 - screen.y);

    // Fog alpha behind this pixel, undistorted.
    var a0 = 1.0;
    if params.above_water > 0.5 && params.refract > 0.5 && params.cheap_mode < 0.5 {
        a0 = refraction(refract_base, p.y).w;
    }

    let pass1 = params.force_cheap < 0.5 && (params.reflect > 0.5 || params.refract > 0.5);
    var color = vec3<f32>(0.0);
    if pass1 {
        let offset = s1.a * n1.xy * a0;
        var refr = vec4<f32>(0.0, 0.0, 0.0, 1.0);
        if params.refract > 0.5 {
            refr = refraction(refract_base + params.refract_amount * offset, p.y);
        }
        var a = refr.w;
        if params.above_water < 0.5 {
            a = 1.0;
        }
        var refl = vec3<f32>(0.0);
        if params.reflect > 0.5 {
            refl = textureSampleLevel(reflection_texture, reflection_sampler, reflect_base + params.reflect_amount * offset, 0.0).rgb;
            refl = refl * params.reflect_tint_linear.rgb;
        }
        var fres = pow(1.0 - saturate(dot(e, nw1)), 5.0);
        fres = fres * saturate((a - 0.05) * 20.0);
        // Refraction fog (step 5): again toward the fog colour.
        var weight = saturate(a - 0.05);
        if params.above_water < 0.5 {
            weight = saturate((depth - params.fog_start) / (params.fog_end - params.fog_start));
        }
        let refracted = mix(refr.rgb, params.fog_linear.rgb, weight);
        if params.reflect > 0.5 && params.refract > 0.5 {
            color = mix(refracted, refl, fres);
        } else if params.reflect > 0.5 {
            color = refl;
        } else {
            color = refracted;
        }
        color = mix(color, params.scene_fog_linear.rgb, fog_f2);
    }

    if params.cheap_pass > 0.5 {
        // Pass 2 (section 7): gamma-space cubemap, raw.
        let r = 2.0 * dot(nw2, e) / max(dot(nw2, nw2), 1e-6) * nw2 - e;
        // Source's cubemap axes: x, -z, y of ours (Z up).
        // (The sky, in the 3D skybox: Bevy's skybox axes, z flipped.)
        let dir = select(vec3<f32>(r.x, -r.z, r.y), vec3<f32>(r.x, r.y, -r.z), params.sky_env > 0.5);
        let env = srgb_encode(textureSample(envmap_texture, envmap_sampler, dir).rgb);
        let spec = env * params.reflect_tint_raw.rgb;
        var fres = pow(1.0 - max(0.0, dot(e, nw2)), 5.0);
        if params.fixed_weight >= 0.0 {
            fres = params.fixed_weight;
        }
        var out_gamma: vec3<f32>;
        if params.force_cheap > 0.5 {
            out_gamma = mix(params.fog_gamma.rgb, spec, fres);
            out_gamma = mix(out_gamma, params.scene_fog_gamma.rgb, fog_f2);
        } else {
            let range = params.cheap_end - params.cheap_start;
            var ramp = select(select(0.0, 1.0, dist >= params.cheap_end), saturate(dist / range - params.cheap_start / range), range > 0.0);
            var alpha = saturate(fres + ramp);
            if params.refract > 0.5 {
                alpha = alpha * saturate((a0 - 0.05) * 20.0);
            }
            let spec_fogged = mix(spec, params.scene_fog_gamma.rgb, fog_f2);
            out_gamma = mix(srgb_encode(color), spec_fogged, alpha);
        }
        color = srgb_decode(out_gamma);
    }
    return vec4<f32>(color, 1.0);
}
