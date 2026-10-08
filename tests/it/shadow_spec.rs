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
