// HUD text and sprites (hud_text.rs): an image tinted with a colour; the
// pipeline adds it to the screen for additive fonts and sprites, else
// blends it. Glyph coverage images are white with the coverage in alpha.

#import bevy_ui::ui_vertex_output::UiVertexOutput

struct GlyphParams {
    color: vec4<f32>,
    // The image's part drawn: uv offset (xy) and size (zw).
    uv: vec4<f32>,
}

@group(1) @binding(0) var<uniform> params: GlyphParams;
@group(1) @binding(1) var coverage: texture_2d<f32>;
@group(1) @binding(2) var coverage_sampler: sampler;

@fragment
fn fragment(in: UiVertexOutput) -> @location(0) vec4<f32> {
    let t = textureSample(coverage, coverage_sampler, params.uv.xy + in.uv * params.uv.zw);
    return vec4<f32>(params.color.rgb * t.rgb, params.color.a * t.a);
}
