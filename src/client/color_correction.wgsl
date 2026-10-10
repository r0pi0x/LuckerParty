// Source's color correction (see color_correction.rs): the finished frame
// through the map's blended 32x32x32 colour table, which works on the
// 8-bit gamma values the game's framebuffer holds.

#import bevy_core_pipeline::fullscreen_vertex_shader::FullscreenVertexOutput

@group(0) @binding(0) var screen: texture_2d<f32>;
@group(0) @binding(1) var screen_sampler: sampler;
@group(0) @binding(2) var lut: texture_3d<f32>;
@group(0) @binding(3) var lut_sampler: sampler;

fn to_srgb(c: vec3<f32>) -> vec3<f32> {
    let lo = c * 12.92;
    let hi = 1.055 * pow(c, vec3<f32>(1.0 / 2.4)) - 0.055;
    return select(hi, lo, c <= vec3<f32>(0.0031308));
}

fn to_linear(c: vec3<f32>) -> vec3<f32> {
    let lo = c / 12.92;
    let hi = pow((c + 0.055) / 1.055, vec3<f32>(2.4));
    return select(hi, lo, c <= vec3<f32>(0.04045));
}

@fragment
fn fragment(in: FullscreenVertexOutput) -> @location(0) vec4<f32> {
    let c = textureSample(screen, screen_sampler, in.uv);
    let g = clamp(to_srgb(max(c.rgb, vec3<f32>(0.0))), vec3<f32>(0.0), vec3<f32>(1.0));
    // Texel centres: entry i of 32 sits at (i + 0.5) / 32.
    let at = g * (31.0 / 32.0) + vec3<f32>(0.5 / 32.0);
    let corrected = textureSampleLevel(lut, lut_sampler, at, 0.0).rgb;
    return vec4<f32>(to_linear(corrected), c.a);
}
