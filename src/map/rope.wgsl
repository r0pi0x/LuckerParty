// Ropes as Source draws them (specs/cs_source/ropes.md): a strip through
// the rope's points, widened sideways so it always faces the camera, shaded
// texture x (normal map blue)^2 x per-point light.
//
// Widths use the game's own screen-size estimate: on-screen pixels =
// width * (screen width / 2) / depth. The back strip is at least 0.3 px
// wide, with alpha ramping 0.2 -> 0.5 between 0.3 and 1.75 px; the rope in
// front is narrowed by depth * 1.4 / screen width.

#import bevy_pbr::{
    forward_io::{Vertex, VertexOutput},
    mesh_functions,
    mesh_view_bindings::view,
    view_transformations::{position_world_to_clip, position_world_to_view},
}
#import mashup::fog::source_fog

struct FogUniform {
    color: vec4<f32>,
    range: vec4<f32>,
    water_color: vec4<f32>,
    water_range: vec4<f32>,
}

struct RopeParams {
    width: f32,
    back: f32,
    light_scale: f32,
    has_normal_map: f32,
    fog: FogUniform,
}

@group(#{MATERIAL_BIND_GROUP}) @binding(0) var<uniform> params: RopeParams;
@group(#{MATERIAL_BIND_GROUP}) @binding(1) var base_texture: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(2) var base_sampler: sampler;
@group(#{MATERIAL_BIND_GROUP}) @binding(3) var normal_texture: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(4) var normal_sampler: sampler;

@vertex
fn vertex(vertex: Vertex) -> VertexOutput {
    var out: VertexOutput;
    let world_from_local = mesh_functions::get_world_from_local(vertex.instance_index);
    let center = (world_from_local * vec4<f32>(vertex.position, 1.0)).xyz;
    let along = normalize((world_from_local * vec4<f32>(vertex.normal, 0.0)).xyz);
    let to_camera = view.world_position - center;
    let depth = max(-position_world_to_view(center).z, 1e-4);
    let screen_width = view.viewport.z;

    var width = params.width;
    var alpha = 1.0;
    if params.back > 0.5 {
        let pixels = width * (screen_width * 0.5) / depth;
        width = max(width, 0.3 * depth / (screen_width * 0.5));
        alpha = mix(0.2, 0.5, saturate((pixels - 0.3) / (1.75 - 0.3)));
    } else {
        width = max(width - depth * 1.4 / screen_width, 0.0);
    }

    let side_raw = cross(along, to_camera);
    let side = side_raw / max(length(side_raw), 1e-6);
    let world = center + side * (vertex.uv.x * 2.0 - 1.0) * width * 0.5;

    out.world_position = vec4<f32>(world, 1.0);
    out.position = position_world_to_clip(world);
    out.world_normal = normalize(to_camera);
    out.uv = vertex.uv;
    out.color = vec4<f32>(vertex.color.rgb, alpha);
#ifdef VERTEX_OUTPUT_INSTANCE_INDEX
    out.instance_index = vertex.instance_index;
#endif
#ifdef VISIBILITY_RANGE_DITHER
    out.visibility_range_dither = 0;
#endif
    return out;
}

@fragment
fn fragment(in: VertexOutput) -> @location(0) vec4<f32> {
    let base = textureSample(base_texture, base_sampler, in.uv);
    var b = 1.0;
    if params.has_normal_map > 0.5 {
        b = textureSample(normal_texture, normal_sampler, in.uv).z;
    }
    var rgb = base.rgb * b * b * in.color.rgb * params.light_scale;
    // Range fog, and the water's below its surface (Source fogs ropes as
    // any other surface).
    let f = params.fog;
    let fog = source_fog(f.color, f.range, f.water_color, f.water_range, in.world_position.xyz);
    rgb = mix(rgb, fog.rgb, fog.a);
    return vec4<f32>(rgb, base.a * in.color.a);
}
