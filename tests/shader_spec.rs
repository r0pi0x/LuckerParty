//! Test cases from specs/cs_source/shaders.md that our renderer's CPU side
//! reproduces: the LDR lightmap texel encoding and the bump page encoding.

use mashup::map::{source_ldr_bump_texels, source_ldr_texel};

#[test]
fn ldr_lightmap_encoding() {
    assert_eq!(source_ldr_texel(1.0), 128, "linear 1.0 is the half-intensity texel");
    assert_eq!(source_ldr_texel(4.0), 239, "values from 3.999 up saturate");
    assert_eq!(source_ldr_texel(100.0), 239);
    assert_eq!(source_ldr_texel(0.0), 0);
    assert_eq!(source_ldr_texel(-1.0), 0);
    // Round trip under a pure 2.2 curve with the 2^2.2 scale returns L.
    for l in [0.25f32, 0.5, 1.0, 2.0] {
        let t = source_ldr_texel(l) as f32 / 255.0;
        let back = t.powf(2.2) * 2f32.powf(2.2);
        assert!((back - l).abs() / l < 0.03, "{l} -> {back}");
    }
}

fn grey(v: f32) -> [f32; 3] {
    [v; 3]
}

#[test]
fn ldr_bump_page_encoding() {
    // The spec's de_dust2 capture rows (RenderDoc, TEMPLEWALL04).
    let pages = source_ldr_bump_texels(grey(1.73), [grey(1.65), grey(0.31), grey(2.93)]);
    assert_eq!(pages.map(|p| p[0]), [185, 51, 255]);
    let pages = source_ldr_bump_texels(grey(1.64), [grey(0.81), grey(0.29), grey(3.37)]);
    assert_eq!(pages.map(|p| p[0]), [140, 84, 255]);
    // No overflow: pages scaled linearly to the flat page's encoded level.
    let pages = source_ldr_bump_texels(grey(1.0), [grey(1.2), grey(0.9), grey(0.9)]);
    assert_eq!(pages.map(|p| p[0]), [153, 115, 115]);
    // Unbumped faces repeat the flat page, which encodes as the flat texel.
    for l in [0.1f32, 0.5, 1.0, 2.5] {
        let t = source_ldr_texel(l);
        assert_eq!(source_ldr_bump_texels(grey(l), [grey(l); 3]), [[t; 3]; 3], "{l}");
    }
    // Black pages stay black.
    assert_eq!(source_ldr_bump_texels(grey(0.0), [grey(0.0); 3]), [[0; 3]; 3]);
}
