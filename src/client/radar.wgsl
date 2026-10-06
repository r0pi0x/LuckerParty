// The radar's map: the overview picture sampled around the player, turned
// so the way they face is up. Outside the picture: transparent.

#import bevy_ui::ui_vertex_output::UiVertexOutput

struct RadarParams {
    // Overview uv of the radar's centre.
    centre: vec2<f32>,
    // Overview uv per unit of the radar's own uv (0..1 across).
    span: vec2<f32>,
    // Clockwise turn of the map, radians; then 1 if the overview's axes are
    // swapped, and the map's opacity.
    angle: f32,
    alpha: f32,
    _pad: vec2<f32>,
}

@group(1) @binding(0) var<uniform> params: RadarParams;
@group(1) @binding(1) var overview: texture_2d<f32>;
@group(1) @binding(2) var overview_sampler: sampler;

@fragment
fn fragment(in: UiVertexOutput) -> @location(0) vec4<f32> {
    // Screen offset from the centre, turned back by the map's turn.
    let d = in.uv - vec2<f32>(0.5);
    let c = cos(params.angle);
    let s = sin(params.angle);
    let r = vec2<f32>(c * d.x + s * d.y, -s * d.x + c * d.y);
    let uv = params.centre + r * params.span;
    if any(uv < vec2<f32>(0.0)) || any(uv > vec2<f32>(1.0)) {
        return vec4<f32>(0.0);
    }
    let t = textureSample(overview, overview_sampler, uv);
    return vec4<f32>(t.rgb, t.a * params.alpha);
}
