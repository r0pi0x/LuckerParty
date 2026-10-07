// A full-screen refraction of the finished frame: the under-water overlay
// (Source's `$underwateroverlay`, a Refract material). The screen is read
// at an offset from a scrolling, animated normal map and tinted.
//
// Drawn by the view-model camera in its transmissive phase: the
// transmission texture then holds the world and the view models.

#import bevy_pbr::{
    forward_io::{Vertex, VertexOutput},
    mesh_view_bindings as vb,
    mesh_view_bindings::{view, globals},
}

struct ScreenWarpParams {
    tint: vec4<f32>,
    // Normal-map repeats across the screen (xy), scroll per second (zw).
    scale_scroll: vec4<f32>,
    refract_amount: f32,
    frame_rate: f32,
    frame_count: f32,
    _pad: f32,
}

@group(#{MATERIAL_BIND_GROUP}) @binding(0) var<uniform> params: ScreenWarpParams;
@group(#{MATERIAL_BIND_GROUP}) @binding(1) var normal_texture: texture_2d_array<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(2) var normal_sampler: sampler;

// The mesh is one triangle already in clip space.
@vertex
fn vertex(vertex: Vertex) -> VertexOutput {
    var out: VertexOutput;
    out.position = vec4<f32>(vertex.position.xy, 0.5, 1.0);
    out.world_position = vec4<f32>(0.0, 0.0, 0.0, 1.0);
    out.world_normal = vec3<f32>(0.0, 0.0, 1.0);
#ifdef VERTEX_UVS_A
    out.uv = vertex.uv;
#endif
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
    let t = globals.time;
    let count = max(params.frame_count, 1.0);
    let layer = i32(floor(max(t * params.frame_rate, 0.0)) % count);
    let screen = (in.position.xy - view.viewport.xy) / view.viewport.zw;
    let uv = screen * params.scale_scroll.xy + fract(params.scale_scroll.zw * t);
    let n = textureSample(normal_texture, normal_sampler, uv, layer).xyz * 2.0 - 1.0;
    let at = clamp(screen + params.refract_amount * n.xy, vec2<f32>(0.0), vec2<f32>(1.0));
    let c = textureSampleLevel(vb::view_transmission_texture, vb::view_transmission_sampler, at, 0.0).rgb;
    return vec4<f32>(c * params.tint.rgb, 1.0);
}
