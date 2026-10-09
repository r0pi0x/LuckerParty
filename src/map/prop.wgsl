// Props lit by a light probe: texture x the light baked into vertex
// colors (or the probe evaluated per pixel, for moving models), plus the
// scene's point lights, then Source's range fog (f^2), as for world
// surfaces.

#import bevy_pbr::{
    forward_io::VertexOutput,
    mesh_view_bindings::{view, globals},
    mesh_view_bindings as view_bindings,
    clustered_forward as clustering,
    view_transformations::position_world_to_view,
}
#import mashup::fog::source_fog
#import mashup::dither::dithered_out

struct PropParams {
    base_color: vec4<f32>,
    alpha_cutoff: f32,
    base_uv_u: vec4<f32>,
    base_uv_v: vec4<f32>,
    fog_color: vec4<f32>,
    fog_range: vec4<f32>,
    translucent: f32,
    envmap: f32,
    envmap_mask: f32,
    envmap_contrast: f32,
    envmap_saturation: f32,
    envmap_tint: vec4<f32>,
    dynamic: f32,
    probe: f32,
    probe_cube: array<vec4<f32>, 6>,
    probe_light_dir: array<vec4<f32>, 4>,
    probe_light_color: array<vec4<f32>, 4>,
    water_fog_color: vec4<f32>,
    water_fog_range: vec4<f32>,
    detail: f32,
    detail_factor: f32,
    detail_scale: vec2<f32>,
    detail_tint: vec4<f32>,
    selfillum: f32,
    selfillum_tint: vec4<f32>,
}

@group(#{MATERIAL_BIND_GROUP}) @binding(0) var<uniform> params: PropParams;
@group(#{MATERIAL_BIND_GROUP}) @binding(1) var base_texture: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(2) var base_sampler: sampler;
@group(#{MATERIAL_BIND_GROUP}) @binding(3) var envmap_texture: texture_cube<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(4) var envmap_sampler: sampler;
@group(#{MATERIAL_BIND_GROUP}) @binding(5) var envmap_mask_texture: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(6) var detail_texture: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(7) var selfillum_mask_texture: texture_2d<f32>;

// A texture transform (map::MapUvTransform): rows . (u, v, 1), plus the
// TextureScroll translation, wrapped to [0, 1) as the proxy does.
fn transform_uv(uv: vec2<f32>, ru: vec4<f32>, rv: vec4<f32>) -> vec2<f32> {
    let scroll = fract(vec2<f32>(ru.w, rv.w) * globals.time);
    return vec2<f32>(dot(ru.xyz, vec3<f32>(uv, 1.0)), dot(rv.xyz, vec3<f32>(uv, 1.0))) + scroll;
}

// The detail combine before lighting (shaders.md 2 and 4; mode = the
// Source $detailblendmode; map::material_fx::detail_combine mirrors it).
fn detail_combine(mode: f32, a: vec4<f32>, d: vec4<f32>, f: f32) -> vec4<f32> {
    if mode < 0.5 {
        return vec4<f32>(a.rgb * mix(vec3<f32>(1.0), 2.0 * d.rgb, f), a.a);
    } else if mode < 1.5 {
        return vec4<f32>(a.rgb + f * d.rgb, a.a);
    } else if mode < 2.5 {
        return vec4<f32>(mix(a.rgb, d.rgb, f * d.a), a.a);
    } else if mode < 3.5 {
        return mix(a, d, f);
    } else if mode < 4.5 {
        return vec4<f32>(mix(a.rgb, d.rgb, f * (1.0 - a.a)), d.a);
    } else if mode < 6.5 {
        return a;
    } else if mode < 7.5 {
        let c = mix(d.r, d.a, a.a);
        return vec4<f32>(a.rgb * mix(1.0, 2.0 * c, f), a.a);
    } else if mode < 8.5 {
        return mix(a, a * d, f);
    }
    return vec4<f32>(a.rgb, mix(a.a, a.a * d.a, f));
}

const LUMA = vec3<f32>(0.299, 0.587, 0.114);

// The probe on a surface with normal n (shaders.md 4): the ambient cube
// weighted by the squared normal, plus each light by its cosine.
fn probe_light(n: vec3<f32>) -> vec3<f32> {
    let n2 = n * n;
    var light = n2.x * select(params.probe_cube[1].rgb, params.probe_cube[0].rgb, n.x >= 0.0)
        + n2.y * select(params.probe_cube[3].rgb, params.probe_cube[2].rgb, n.y >= 0.0)
        + n2.z * select(params.probe_cube[5].rgb, params.probe_cube[4].rgb, n.z >= 0.0);
    for (var i = 0u; i < 4u; i = i + 1u) {
        light = light + params.probe_light_color[i].rgb * max(dot(n, params.probe_light_dir[i].xyz), 0.0);
    }
    return light;
}

// The scene's point lights (muzzle flashes; map::DynamicLight), in
// lightmap units: colour x (1 - d^2/r^2) x cosine. Same as world.wgsl.
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

@fragment
fn fragment(in: VertexOutput) -> @location(0) vec4<f32> {
#ifdef VISIBILITY_RANGE_DITHER
    // Inside the prop's fade band (map::vis::fade_band).
    if dithered_out(in.position, in.visibility_range_dither) {
        discard;
    }
#endif
    // $basetexturetransform: the base's coordinates, and the detail's
    // before $detailscale.
    let uv = transform_uv(in.uv, params.base_uv_u, params.base_uv_v);
    var color = textureSample(base_texture, base_sampler, uv);
    let base_alpha = color.a;
    // $detail (shaders.md 4): times $detailtint, combined before the
    // material colour; modes 5 and 6 add after lighting (below).
    var d = vec4<f32>(0.0);
    let mode = params.detail - 1.0;
    if params.detail > 0.5 {
        d = textureSample(detail_texture, base_sampler, uv * params.detail_scale);
        d = vec4<f32>(d.rgb * params.detail_tint.rgb, d.a);
        color = detail_combine(mode, color, d, params.detail_factor);
    }
    color = color * params.base_color;
    // Self-illuminated: the base alpha is the mask, not opacity.
    if params.selfillum > 0.5 {
        color.a = params.base_color.a;
    }
    if params.alpha_cutoff > 0.0 && color.a < params.alpha_cutoff {
        discard;
    }
    let n = normalize(in.world_normal);
    var light = vec3<f32>(1.0);
#ifdef VERTEX_COLORS
    light = in.color.rgb;
#endif
    if params.probe > 0.5 {
        light = probe_light(n);
    }
    if params.dynamic > 0.5 {
        light = light + dynamic_light(in.world_position.xyz, n, in.position.xy);
    }
    var rgb = color.rgb * light;
    // Detail modes 5 and 6: added after lighting.
    if params.detail > 0.5 && mode > 4.5 && mode < 6.5 {
        let f = params.detail_factor;
        if mode < 5.5 {
            rgb = rgb + f * d.rgb;
        } else {
            let m = select(4.0 * f, 1.0 / f, f >= 0.5);
            let a = select(-0.5 * m, 1.0 - m, f >= 0.5);
            rgb = rgb + saturate(m * d.rgb + a);
        }
    }
    // $selfillum: toward tint x albedo, unlit, by the base alpha or the
    // $selfillummask texture.
    if params.selfillum > 0.5 {
        var mask = vec3<f32>(base_alpha);
        if params.selfillum > 1.5 {
            mask = textureSample(selfillum_mask_texture, base_sampler, uv).rgb;
        }
        rgb = mix(rgb, params.selfillum_tint.rgb * color.rgb, mask);
    }
    // $envmap (specs/cs_source/shaders.md 4): the cubemap in the reflected
    // view direction x mask x tint, contrast and saturation; no fresnel.
    if params.envmap > 0.5 {
        let v = view.world_position - in.world_position.xyz;
        let r = 2.0 * dot(n, v) * n - dot(n, n) * v;
        var spec = textureSample(envmap_texture, envmap_sampler, vec3<f32>(r.x, -r.z, r.y)).rgb;
        if params.envmap_mask > 1.5 && params.envmap_mask < 2.5 {
            spec = spec * (1.0 - color.a);
        } else if params.envmap_mask > 2.5 {
            spec = spec * textureSample(envmap_mask_texture, base_sampler, uv).rgb;
        }
        spec = spec * params.envmap_tint.rgb;
        spec = mix(spec, spec * spec, params.envmap_contrast);
        spec = mix(vec3<f32>(dot(spec, LUMA)), spec, params.envmap_saturation);
        rgb = rgb + spec;
    }
    // Under (or, at the surface, just below) water, what is below the
    // surface takes the water's fog.
    let fog = source_fog(params.fog_color, params.fog_range, params.water_fog_color, params.water_fog_range, in.world_position.xyz);
    rgb = mix(rgb, fog.rgb, fog.a);
    // Additive (translucent 2): alpha 0 under premultiplied blending adds.
    if params.translucent > 1.5 {
        return vec4<f32>(rgb, 0.0);
    }
    return vec4<f32>(rgb, select(1.0, color.a, params.translucent > 0.5));
}
