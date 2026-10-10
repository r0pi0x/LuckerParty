// Detail sprites (map::detail): fixed quads or quads facing the view
// (turning about the vertical only, or fully), their tops swaying in the
// map's wind, faded by their distance from the view (dithered out), the
// sprite sheet x the sprite's baked light, alpha tested, then fogged.
// Vertex layout: see detail.rs.

#import bevy_pbr::{
    forward_io::{Vertex, VertexOutput},
    mesh_functions,
    mesh_view_bindings::{view, globals},
    view_transformations::position_world_to_clip,
}
#import mashup::fog::source_fog

struct FogUniform {
    color: vec4<f32>,
    range: vec4<f32>,
    water_color: vec4<f32>,
    water_range: vec4<f32>,
}

struct DetailParams {
    fade: vec4<f32>,
    wind: vec4<f32>,
    fog: FogUniform,
}

@group(#{MATERIAL_BIND_GROUP}) @binding(0) var<uniform> params: DetailParams;
@group(#{MATERIAL_BIND_GROUP}) @binding(1) var sheet: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(2) var sheet_sampler: sampler;

@vertex
fn vertex(vertex: Vertex) -> VertexOutput {
    var out: VertexOutput;
    let world_from_local = mesh_functions::get_world_from_local(vertex.instance_index);
    let foot = (world_from_local * vec4<f32>(vertex.position, 1.0)).xyz;
    let kind = vertex.tangent.w;
    var world = foot + vertex.tangent.xyz;
    if kind > 0.5 {
        var right = normalize(view.world_from_view[0].xyz);
        var up = normalize(view.world_from_view[1].xyz);
        if kind > 1.5 {
            // Turning about the vertical only: square to the view's
            // direction seen from above.
            let to_eye = view.world_position - foot;
            let flat = vec2<f32>(to_eye.x, to_eye.z);
            let d = select(vec2<f32>(0.0, 1.0), normalize(flat), dot(flat, flat) > 1e-8);
            right = vec3<f32>(d.y, 0.0, -d.x);
            up = vec3<f32>(0.0, 1.0, 0.0);
        }
        world = foot + right * vertex.uv_b.x + up * vertex.uv_b.y;
    }
    // Sway: tops move along the wind and back, each sprite in its own
    // phase.
    let phase = dot(foot, vec3<f32>(1.7, 0.0, 2.3));
    let swing = sin(globals.time * params.wind.w + phase) * params.wind.z * vertex.color.a;
    world = world + vec3<f32>(params.wind.x, 0.0, params.wind.y) * swing;
    // Fade by the foot's distance from the view.
    let dist = distance(view.world_position, foot);
    let start = params.fade.x;
    let end = params.fade.y;
    let shown = select(select(0.0, 1.0, dist < end), clamp((end - dist) / (end - start), 0.0, 1.0), end > start);
    out.world_position = vec4<f32>(world, 1.0);
    out.position = position_world_to_clip(world);
    if shown <= 0.0 {
        // Gone: outside the clip volume.
        out.position = vec4<f32>(2.0, 2.0, 2.0, 1.0);
    }
    out.world_normal = vec3<f32>(0.0, 1.0, 0.0);
    out.uv = vertex.uv;
    out.uv_b = vertex.uv_b;
    out.world_tangent = vertex.tangent;
    out.color = vec4<f32>(vertex.color.rgb, shown);
#ifdef VERTEX_OUTPUT_INSTANCE_INDEX
    out.instance_index = vertex.instance_index;
#endif
#ifdef VISIBILITY_RANGE_DITHER
    out.visibility_range_dither = 0;
#endif
    return out;
}

// 4x4 ordered dither threshold (0..1) for a pixel.
fn dither(p: vec2<f32>) -> f32 {
    let x = u32(p.x) % 4u;
    let y = u32(p.y) % 4u;
    let m = array<f32, 16>(0.0, 8.0, 2.0, 10.0, 12.0, 4.0, 14.0, 6.0, 3.0, 11.0, 1.0, 9.0, 15.0, 7.0, 13.0, 5.0);
    return (m[y * 4u + x] + 0.5) / 16.0;
}

@fragment
fn fragment(in: VertexOutput) -> @location(0) vec4<f32> {
    let t = textureSample(sheet, sheet_sampler, in.uv);
    if t.a < 0.5 || in.color.a < dither(in.position.xy) {
        discard;
    }
    let rgb = t.rgb * in.color.rgb;
    let f = source_fog(params.fog.color, params.fog.range, params.fog.water_color, params.fog.water_range, in.world_position.xyz);
    return vec4<f32>(mix(rgb, f.rgb, f.a), 1.0);
}
