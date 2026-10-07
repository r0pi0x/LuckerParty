// Source-style tone mapping for mat_hdr_level 2 (see hdr.rs): the scene's
// linear light times one scale, chosen by auto exposure and kept within
// the map's tone-map controller bounds, then clamped as an 8-bit
// framebuffer would. No curve: Source's HDR keeps the LDR look in range.

#import bevy_core_pipeline::fullscreen_vertex_shader::FullscreenVertexOutput
#import bevy_render::view::View

struct SourceTonemap {
    min_scale: f32,
    max_scale: f32,
    // 1: scale by auto exposure; 0: scale 1 (bloom only).
    auto_exposure: f32,
    _pad: f32,
}

@group(0) @binding(0) var screen: texture_2d<f32>;
@group(0) @binding(1) var screen_sampler: sampler;
@group(0) @binding(2) var<uniform> settings: SourceTonemap;
// Auto exposure writes its result into the view's colour grading exposure
// (in stops) before this pass runs.
@group(0) @binding(3) var<uniform> view: View;

@fragment
fn fragment(in: FullscreenVertexOutput) -> @location(0) vec4<f32> {
    let c = textureSample(screen, screen_sampler, in.uv);
    var scale = 1.0;
    if settings.auto_exposure > 0.5 {
        scale = clamp(exp2(view.color_grading.exposure), settings.min_scale, settings.max_scale);
    }
    return vec4<f32>(clamp(c.rgb * scale, vec3<f32>(0.0), vec3<f32>(1.0)), 1.0);
}
