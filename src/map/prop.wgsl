// Props lit by a light probe: texture x the light baked into vertex
// colors (or the probe evaluated per pixel, for moving models), plus the
// scene's point lights, then Source's range fog (f^2), as for world
// surfaces.

#import bevy_pbr::{
    forward_io::VertexOutput,
    mesh_view_bindings::view,
    mesh_view_bindings as view_bindings,
    clustered_forward as clustering,
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
    dynamic: f32,
    probe: f32,
    probe_cube: array<vec4<f32>, 6>,
    probe_light_dir: array<vec4<f32>, 4>,
    probe_light_color: array<vec4<f32>, 4>,
}

@group(#{MATERIAL_BIND_GROUP}) @binding(0) var<uniform> params: PropParams;
@group(#{MATERIAL_BIND_GROUP}) @binding(1) var base_texture: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(2) var base_sampler: sampler;
@group(#{MATERIAL_BIND_GROUP}) @binding(3) var envmap_texture: texture_cube<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(4) var envmap_sampler: sampler;
@group(#{MATERIAL_BIND_GROUP}) @binding(5) var envmap_mask_texture: texture_2d<f32>;

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
    var color = textureSample(base_texture, base_sampler, in.uv) * params.base_color;
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
    color = vec4<f32>(color.rgb * light, color.a);
    var rgb = color.rgb;
    // $envmap (specs/cs_source/shaders.md 4): the cubemap in the reflected
    // view direction x mask x tint, contrast and saturation; no fresnel.
    if params.envmap > 0.5 {
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
    // Additive (translucent 2): alpha 0 under premultiplied blending adds.
    if params.translucent > 1.5 {
        return vec4<f32>(rgb, 0.0);
    }
    return vec4<f32>(rgb, select(1.0, color.a, params.translucent > 0.5));
}
