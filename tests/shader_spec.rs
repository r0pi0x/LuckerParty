//! Test cases from specs/cs_source/shaders.md that our renderer's CPU side
//! reproduces: the LDR lightmap texel encoding.

use mashup::map::source_ldr_texel;

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
