//! Prop shadow test cases from specs/cs_source/shadows_sky.md.

use bevy::prelude::*;
use mashup::map::shadows::{ShadowFrame, cell_size, shadow_color};

const M: f32 = 0.0254;

/// Source units to engine (meters, Y up).
fn engine(v: Vec3) -> Vec3 {
    Vec3::new(v.x, v.z, -v.y) * M
}

fn close(a: f32, b: f32, tol: f32, what: &str) {
    assert!((a - b).abs() <= tol, "{what}: {a} vs {b}");
}

#[test]
fn frame_straight_down() {
    // An axis-aligned 32 x 32 x 48 box, its base at the origin.
    let bounds = (
        engine(Vec3::new(-16.0, -16.0, 0.0)).min(engine(Vec3::new(16.0, 16.0, 48.0))),
        engine(Vec3::new(-16.0, -16.0, 0.0)).max(engine(Vec3::new(16.0, 16.0, 48.0))),
    );
    let down = engine(Vec3::NEG_Z).normalize();
    let f = ShadowFrame::new(bounds, Vec3::ZERO, Quat::IDENTITY, down, 50.0 * M);
    close(f.falloff_start / M, 56.985, 0.01, "falloff start");
    close(f.max_dist / M, 106.985, 0.01, "max dist");
    close(f.size.x / M, 36.0, 1e-3, "size x");
    close(f.size.y / M, 36.0, 1e-3, "size y");
    assert!(
        (f.origin - engine(Vec3::new(0.0, 0.0, 48.0))).length() < 1e-4,
        "origin at the top centre: {}",
        f.origin / M
    );
    let f75 = ShadowFrame::new(bounds, Vec3::ZERO, Quat::IDENTITY, down, 75.0 * M);
    close(f75.max_dist / M, 131.985, 0.01, "max dist at dust2's 75");
}

#[test]
fn texture_sizes() {
    let size = |x: f32, y: f32, z: f32| cell_size((Vec3::ZERO, engine(Vec3::new(x, y, z)).abs()));
    assert_eq!(size(30.0, 30.0, 10.0), 64);
    assert_eq!(size(28.0, 28.0, 46.0), 128);
    assert_eq!(size(32.0, 32.0, 72.0), 256);
    assert_eq!(size(6.0, 6.0, 8.0), 16);
}

#[test]
fn receiver_colour() {
    let c = shadow_color([159, 159, 159]);
    close(c.x, 0.35373, 1e-4, "gamma to linear");
    // Full coverage over linear 0.5; half coverage.
    close(0.5 * c.x, 0.17686, 1e-4, "full");
    close(1.0 + 0.5 * (c.x - 1.0), 0.67686, 1e-4, "half");
}

/// A caster with triangles far outside its shadow box (some community
/// props, found by the map sweep) draws what falls inside and skips the
/// rest instead of writing past the cell.
#[test]
fn silhouette_skips_triangles_outside_the_cell() {
    use mashup::map::{MapMesh, MapModel, shadows::silhouette};
    let bounds = (Vec3::splat(-0.5), Vec3::splat(0.5));
    let down = Vec3::NEG_Y;
    let f = ShadowFrame::new(bounds, Vec3::ZERO, Quat::IDENTITY, down, 50.0 * M);
    let tri = |o: Vec3| [o, o + Vec3::X * 0.2, o + Vec3::Z * 0.2].map(|p| p.to_array());
    let mut positions = Vec::new();
    positions.extend(tri(Vec3::ZERO));
    positions.extend(tri(Vec3::new(40.0, 0.0, 40.0)));
    positions.extend(tri(Vec3::new(-40.0, 0.0, -40.0)));
    let model = MapModel {
        meshes: vec![MapMesh {
            indices: (0..9).collect(),
            positions,
            ..Default::default()
        }],
        ..Default::default()
    };
    let n = 64;
    let cover = silhouette(&model, &[], &f, Vec3::ZERO, Quat::IDENTITY, n);
    assert_eq!(cover.len(), (n * n) as usize);
    assert!(cover.iter().any(|c| *c > 0.0), "the inside triangle draws");
}
