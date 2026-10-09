// Beams as Source's entity beams draw them (specs/source/visual_entities.md
// 1, 2.3): a strip from start to end, widened sideways to face the camera
// (full width 2 x the width), texture x colour, added to the scene; the
// colour shaded out along the beam over its fade length; a spotlight's
// beam also fades out as the eye comes near its axis.

#import bevy_pbr::{
    forward_io::{Vertex, VertexOutput},
    mesh_view_bindings::view,
    view_transformations::position_world_to_clip,
}

struct BeamParams {
    start: vec4<f32>,
    end: vec4<f32>,
    // rgb x brightness; a: 1 shades out, 2 in, 3 both.
    color: vec4<f32>,
    // x: width at the start, y: at the end, z: fade length (0: the
    // whole beam), w: spotlight width W for the near-axis fade (0: none).
    widths: vec4<f32>,
}

@group(#{MATERIAL_BIND_GROUP}) @binding(0) var<uniform> params: BeamParams;
@group(#{MATERIAL_BIND_GROUP}) @binding(1) var base_texture: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(2) var base_sampler: sampler;

@vertex
fn vertex(vertex: Vertex) -> VertexOutput {
    var out: VertexOutput;
    // uv.x: 0 or 1 across, uv.y: 0 at the start, 1 at the end.
    let f = vertex.uv.y;
    let s = params.start.xyz;
    let e = params.end.xyz;
    let d = e - s;
    let len = max(length(d), 1e-6);
    let along = d / len;
    let p = mix(s, e, f);
    let to_camera = view.world_position - p;
    let side_raw = cross(along, to_camera);
    let side = side_raw / max(length(side_raw), 1e-6);
    let width = mix(params.widths.x, params.widths.y, f);
    let world = p + side * (vertex.uv.x * 2.0 - 1.0) * width;

    // Shading along the beam (1.6).
    var fade = params.widths.z;
    if fade <= 0.0 {
        fade = len;
    }
    let ff = clamp(fade / len, 1e-6, 1.0);
    let mode = u32(params.color.a + 0.5);
    var b = 1.0;
    if mode == 1u {
        b = 1.0 - f / ff;
    } else if mode == 2u {
        b = f / ff;
    } else if mode == 3u {
        b = select(2.0 * (1.0 - f / ff), 2.0 * f / ff, f < 0.5);
    }
    b = clamp(b, 0.0, 1.0);
    // A spotlight's beam near its axis (2.3): 1 from 4W out, 0 within W.
    let w = params.widths.w;
    if w > 0.0 {
        let rel = view.world_position - s;
        let axis_dist = length(rel - along * dot(rel, along));
        b = b * clamp((axis_dist - w) / (3.0 * w), 0.0, 1.0);
    }

    out.world_position = vec4<f32>(world, 1.0);
    out.position = position_world_to_clip(world);
    out.world_normal = normalize(to_camera);
    out.uv = vec2<f32>(vertex.uv.x, f);
    out.color = vec4<f32>(params.color.rgb * b, 1.0);
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
    return vec4<f32>(base.rgb * in.color.rgb, 1.0);
}
