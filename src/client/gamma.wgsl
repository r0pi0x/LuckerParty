// mat_monitorgamma (see gamma.rs): the finished frame raised to
// mat_monitorgamma / 2.2. The texture holds linear colour (sRGB decoded
// on sampling, encoded on writing); a power is the same in linear and
// display space, so it applies here as it is.

#import bevy_core_pipeline::fullscreen_vertex_shader::FullscreenVertexOutput

struct MonitorGamma {
    exponent: f32,
    _pad: vec3<f32>,
}

@group(0) @binding(0) var screen: texture_2d<f32>;
@group(0) @binding(1) var screen_sampler: sampler;
@group(0) @binding(2) var<uniform> settings: MonitorGamma;

@fragment
fn fragment(in: FullscreenVertexOutput) -> @location(0) vec4<f32> {
    let c = textureSample(screen, screen_sampler, in.uv);
    let rgb = pow(max(c.rgb, vec3<f32>(0.0)), vec3<f32>(settings.exponent));
    return vec4<f32>(rgb, c.a);
}
