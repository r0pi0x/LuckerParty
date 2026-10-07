// The flash overlay (specs/cs_source/grenades.md 6.3): added on top of the
// frame, white at the flash's alpha plus the frame captured at the flash
// (the frozen after-image) at its own weight. The pipeline blends it
// additively.

#import bevy_ui::ui_vertex_output::UiVertexOutput

struct FlashParams {
    // White amount, after-image amount, unused.
    white: f32,
    image: f32,
    _pad: vec2<f32>,
}

@group(1) @binding(0) var<uniform> params: FlashParams;
@group(1) @binding(1) var frozen: texture_2d<f32>;
@group(1) @binding(2) var frozen_sampler: sampler;

@fragment
fn fragment(in: UiVertexOutput) -> @location(0) vec4<f32> {
    let t = textureSample(frozen, frozen_sampler, in.uv).rgb;
    return vec4<f32>(vec3<f32>(params.white) + t * params.image, 0.0);
}
