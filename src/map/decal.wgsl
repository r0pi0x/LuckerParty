// Runtime decals (bullet holes, slashes): the framebuffer is multiplied by
// a factor F. The blend is the game's own for these decals, src x dst +
// dst x src, so the shader returns F / 2: F may exceed 1 (brightening)
// while the output stays within an 8-bit target's [0, 1], up to F = 2.
//
// Modulate-2x decals (specs/cs_source/overlays_decals.md, "Decal
// blending"): the game multiplies its gamma-space framebuffer by 2 x the
// texel, so 0.5 grey leaves the surface unchanged and lighter texels
// brighten it. Our framebuffer is linear, so the same change is that
// factor raised to 2.2 (clamped at 2 here; see tech-debt). No alpha.
//
// Other decals ("lit" decals, alpha blended and lightmapped in the game)
// are approximated the same way, as a stain: factor = texel colour where
// the texel is opaque.

#import bevy_pbr::forward_io::VertexOutput

struct DecalParams {
    // 1: modulate 2x; 0: stain.
    modulate: f32,
    _pad0: f32,
    _pad1: f32,
    _pad2: f32,
}

@group(#{MATERIAL_BIND_GROUP}) @binding(0) var<uniform> params: DecalParams;
@group(#{MATERIAL_BIND_GROUP}) @binding(1) var base_texture: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(2) var base_sampler: sampler;

// Share of the decal's width over which its edge fades out (about 3
// texels of a 64-texel decal).
const EDGE_FADE: f32 = 0.05;

@fragment
fn fragment(in: VertexOutput) -> @location(0) vec4<f32> {
    let texel = textureSample(base_texture, base_sampler, in.uv);
    // The texture is sRGB: back to the game's gamma-space value.
    let gamma = pow(max(texel.rgb, vec3(0.0)), vec3(1.0 / 2.2));
    var factor: vec3<f32>;
    if params.modulate > 0.5 {
        factor = 2.0 * gamma;
    } else {
        factor = mix(vec3(1.0), gamma, texel.a);
    }
    // Fade to neutral over the decal's outer edge: decals share an atlas,
    // and filtering (most of all the small mip levels seen from afar)
    // pulls in the neighbouring decal's texels there, as dark lines.
#ifdef VERTEX_UVS_B
    let t = in.uv_b;
    let edge = min(min(t.x, 1.0 - t.x), min(t.y, 1.0 - t.y));
    factor = mix(vec3(1.0), factor, smoothstep(0.0, EDGE_FADE, edge));
#endif
    return vec4(min(pow(factor, vec3(2.2)), vec3(2.0)) * 0.5, 1.0);
}
