// Runtime decals (bullet holes, slashes): the framebuffer is multiplied by
// the factor this shader returns (blend: src x dst + dst x 0).
//
// Modulate-2x decals (specs/cs_source/overlays_decals.md, "Decal
// blending"): the game multiplies its gamma-space framebuffer by 2 x the
// texel, so 0.5 grey leaves the surface unchanged. Our framebuffer is
// linear, so the same change is the factor raised to 2.2. Texel alpha
// fades the decal out toward its edges.
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

@fragment
fn fragment(in: VertexOutput) -> @location(0) vec4<f32> {
    let texel = textureSample(base_texture, base_sampler, in.uv);
    // The texture is sRGB: back to the game's gamma-space value.
    let gamma = pow(max(texel.rgb, vec3(0.0)), vec3(1.0 / 2.2));
    var factor: vec3<f32>;
    if params.modulate > 0.5 {
        factor = mix(vec3(1.0), 2.0 * gamma, texel.a);
    } else {
        factor = mix(vec3(1.0), gamma, texel.a);
    }
    return vec4(pow(factor, vec3(2.2)), 1.0);
}
