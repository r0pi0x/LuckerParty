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

// Detail combine, self-illum and texture transforms
// (map::material_fx mirrors world.wgsl and prop.wgsl).

use bevy::math::{Vec2, Vec3, Vec4};
use mashup::map::{
    MapUvTransform,
    material_fx::{detail_combine, detail_post_lighting, self_illum},
};

fn close(a: f32, b: f32) -> bool {
    (a - b).abs() < 1e-4
}

#[test]
fn detail_blend_modes() {
    let grey = |v: f32, a: f32| Vec4::new(v, v, v, a);
    // Spec test cases: mod2x and mode 2.
    assert!(close(detail_combine(0, grey(0.5, 1.0), grey(0.5, 1.0), 1.0).x, 0.5));
    assert!(close(detail_combine(0, grey(0.5, 1.0), grey(1.0, 1.0), 1.0).x, 1.0));
    assert!(close(detail_combine(0, grey(0.5, 1.0), grey(1.0, 1.0), 0.5).x, 0.75));
    assert!(close(detail_combine(2, grey(0.2, 1.0), grey(0.8, 0.5), 1.0).x, 0.5));
    // The table in shaders.md 2, "$detail".
    let (a, d) = (grey(0.4, 0.25), grey(0.8, 0.5));
    assert!(close(detail_combine(1, a, d, 0.5).x, 0.8), "additive");
    let m3 = detail_combine(3, a, d, 0.5);
    assert!(close(m3.x, 0.6) && close(m3.w, 0.375), "mode 3 lerps alpha too");
    let m4 = detail_combine(4, a, d, 1.0);
    assert!(
        close(m4.x, 0.4 + 0.4 * 0.75) && close(m4.w, 0.5),
        "mode 4 by 1 - base alpha, alpha = detail's"
    );
    // Mode 7: c = lerp(d.r, d.a, A.a) = 0.8 + (0.5 - 0.8) x 0.25 = 0.725.
    assert!(close(detail_combine(7, a, d, 1.0).x, 0.4 * 1.45));
    let m8 = detail_combine(8, a, d, 0.5);
    assert!(close(m8.x, 0.4 * 0.9) && close(m8.w, 0.25 * 0.75));
    let m9 = detail_combine(9, a, d, 1.0);
    assert!(close(m9.x, 0.4) && close(m9.w, 0.125), "mode 9 only touches alpha");
    // Models' post-lighting modes.
    assert!(close(detail_post_lighting(5, Vec3::splat(0.5), 0.5).x, 0.25));
    // Mode 6, f >= 0.5: m = 1/f, a = 1 - m; f < 0.5: m = 4f, a = -m/2.
    assert!(close(detail_post_lighting(6, Vec3::splat(0.75), 0.5).x, 0.5));
    assert!(close(detail_post_lighting(6, Vec3::splat(0.75), 0.25).x, 0.25));
    assert!(close(detail_post_lighting(6, Vec3::splat(0.1), 0.25).x, 0.0), "clamped");
    assert_eq!(detail_post_lighting(0, Vec3::ONE, 1.0), Vec3::ZERO);
}

#[test]
fn self_illumination() {
    // Spec: A = 0.5, lighting 0.2, base alpha 1, tint 1: lighting ignored.
    let a = Vec3::splat(0.5);
    assert!(close(self_illum(a * 0.2, a, Vec3::ONE, 1.0).x, 0.5));
    assert!(close(self_illum(a * 0.2, a, Vec3::ONE, 0.0).x, 0.1));
    assert!(close(self_illum(a * 0.2, a, Vec3::splat(2.0), 0.5).x, 0.55));
}

#[test]
fn texture_transforms() {
    let at = |t: &MapUvTransform, u: f32, v: f32| t.apply(Vec2::new(u, v), 0.0);
    let id = MapUvTransform::parse("center .5 .5 scale 1 1 rotate 0 translate 0 0").unwrap();
    assert!(at(&id, 0.3, 0.7).distance(Vec2::new(0.3, 0.7)) < 1e-6);
    // Scale: the texture fits that many times; the centre is only the
    // point of rotation.
    let s = MapUvTransform::parse("center .5 .5 scale 2 2 rotate 0 translate 0 0").unwrap();
    assert!(at(&s, 0.5, 0.5).distance(Vec2::splat(1.0)) < 1e-6);
    assert!(at(&s, 1.0, 1.0).distance(Vec2::splat(2.0)) < 1e-6);
    // Rotation about the centre.
    let rc = MapUvTransform::parse("center .5 .5 scale 1 1 rotate 180 translate 0 0").unwrap();
    assert!(at(&rc, 0.5, 0.5).distance(Vec2::splat(0.5)) < 1e-5);
    assert!(at(&rc, 1.0, 1.0).distance(Vec2::splat(0.0)) < 1e-5);
    // A sky face's half height (community skies: "center 0 0 scale 1 2").
    let sky = MapUvTransform::parse("center 0 0 scale 1 2 rotate 0 translate 0 0").unwrap();
    assert!(at(&sky, 0.5, 0.5).distance(Vec2::new(0.5, 1.0)) < 1e-6);
    // Rotation, counter-clockwise in (u, v).
    let r = MapUvTransform::parse("center 0 0 scale 1 1 rotate 90 translate 0 0").unwrap();
    assert!(at(&r, 1.0, 0.0).distance(Vec2::new(0.0, 1.0)) < 1e-5);
    // Translation.
    let t = MapUvTransform::parse("center .5 .5 scale 1 1 rotate 0 translate .25 .5").unwrap();
    assert!(at(&t, 0.0, 0.0).distance(Vec2::new(0.25, 0.5)) < 1e-6);
    // Unreadable values give nothing (the material's identity).
    assert!(MapUvTransform::parse("11").is_none());
    // shaders_two_texture_blend.md test case: $detailscale [2 3] over a
    // base transform translating (0.5, 0.25), uv (1, 1): (3.0, 3.75).
    let base = MapUvTransform::parse("center 0 0 scale 1 1 rotate 0 translate .5 .25").unwrap();
    assert!((at(&base, 1.0, 1.0) * Vec2::new(2.0, 3.0)).distance(Vec2::new(3.0, 3.75)) < 1e-5);
    // TextureScroll (water.md test case): rate 0.01 at 45 degrees, t = 100 s.
    let scroll = MapUvTransform::scrolling(0.01, 45.0, 1.0);
    let p = scroll.apply(Vec2::ZERO, 100.0);
    assert!(p.distance(Vec2::splat(0.70711)) < 1e-4, "{p}");
    // Wrapped to [0, 1): negative rates wrap up.
    let back = MapUvTransform::scrolling(0.3, 180.0, 1.0).apply(Vec2::ZERO, 1.0);
    assert!(close(back.x, 0.7), "{back}");
}
