//! Beam and glow entities from the map (specs/source/visual_entities.md):
//! point_spotlight (2: its shaft traced to the first wall, a halo at its
//! source), env_laser (3) and persistent env_beam (4.1) between their
//! endpoints as placed, env_lightglow (6), into `map::beams`. Lasers and
//! beams are drawn where the map placed their ends (targets that move
//! aren't followed); strike-generator env_beams (life ≠ 0) and noise
//! aren't drawn.

use bevy::prelude::*;
use vbsp::Bsp;

use super::{bsp::METERS_PER_UNIT, material::MaterialLoader};
use crate::map::MapData;
use crate::map::beams::{MapBeam, MapGlow, SpotHalo};

fn parse3(v: &str) -> Option<Vec3> {
    let mut it = v.split_whitespace().filter_map(|p| p.parse::<f32>().ok());
    Some(Vec3::new(it.next()?, it.next()?, it.next()?))
}

/// Source units (Z up) to engine space.
fn engine(v: Vec3) -> Vec3 {
    Vec3::new(v.x, v.z, -v.y) * METERS_PER_UNIT
}

/// Source's forward vector of (pitch, yaw, roll) angles.
fn forward(angles: Vec3) -> Vec3 {
    let (p, y) = (angles.x.to_radians(), angles.y.to_radians());
    Vec3::new(p.cos() * y.cos(), p.cos() * y.sin(), -p.sin())
}

/// A sprite material's texture (`.vmt` or bare name).
fn sprite(materials: &mut MaterialLoader, path: &str) -> Option<usize> {
    let path = path.to_lowercase().replace('\\', "/");
    let path = path
        .trim_start_matches("materials/")
        .trim_end_matches(".vmt")
        .trim_end_matches(".spr");
    materials.resolve(path).texture
}

/// How far (0..1) a line from `from` to `to` (engine space) gets through
/// the world's brushes (`skip`: brushes line traces pass).
fn trace(data: &MapData, skip: &std::collections::HashSet<usize>, from: Vec3, to: Vec3) -> f32 {
    data.collision_brushes
        .iter()
        .enumerate()
        .filter(|(i, _)| !skip.contains(i))
        .filter_map(|(_, b)| b.sweep_box(Vec3::ZERO, from, to, 0.0).map(|(f, _)| f))
        .fold(1.0, f32::min)
}

pub fn add_beams(bsp: &Bsp, materials: &mut MaterialLoader, data: &mut MapData) {
    let skip: std::collections::HashSet<usize> = data.trace_skip.iter().copied().collect();
    let origin_of = |name: &str| {
        bsp.entities
            .iter()
            .find(|e| e.prop("targetname").is_some_and(|n| n.eq_ignore_ascii_case(name)))
            .and_then(|e| e.prop("origin").and_then(parse3))
    };
    let mut beams = Vec::new();
    let mut glows = Vec::new();
    for (index, e) in bsp.entities.iter().enumerate() {
        let Some(class) = e.prop("classname") else { continue };
        let num = |k: &'static str| e.prop(k).and_then(|v| v.trim().parse::<f32>().ok());
        let origin = e.prop("origin").and_then(parse3).unwrap_or(Vec3::ZERO);
        let angles = e.prop("angles").and_then(parse3).unwrap_or(Vec3::ZERO);
        let rgb = e.prop("rendercolor").and_then(parse3).unwrap_or(Vec3::splat(255.0)) / 255.0;
        let flags = num("spawnflags").unwrap_or(0.0) as u32;
        match class {
            "point_spotlight" => {
                let (Some(texture), Some(halo)) = (sprite(materials, "sprites/glow_test02"), sprite(materials, "sprites/light_glow03"))
                else {
                    continue;
                };
                let length = num("SpotlightLength").filter(|l| *l > 0.0).unwrap_or(500.0);
                let width = num("SpotlightWidth").filter(|w| *w > 0.0).unwrap_or(10.0).min(102.3);
                let d = forward(angles);
                let ignore = num("IgnoreSolid").unwrap_or(0.0) != 0.0;
                let o = engine(origin);
                let reach = |l: f32| {
                    let to = engine(origin + d * l);
                    if ignore { to } else { o.lerp(to, trace(data, &skip, o, to)) }
                };
                let b1 = reach(length);
                let b2 = reach(2.0 * length);
                let c = b2.distance(o) / METERS_PER_UNIT;
                let fade = if c > length { length } else { c };
                let end_width = (width * c / length).clamp(0.0, 102.3).max(1e-3);
                beams.push(MapBeam {
                    entity: Some(index),
                    start: o,
                    end: b1,
                    width: width * METERS_PER_UNIT,
                    end_width: width * METERS_PER_UNIT,
                    // Brightness 64 (2.1 step 3).
                    color: rgb * 64.0 / 255.0,
                    shade: 1,
                    fade: fade * METERS_PER_UNIT,
                    texture,
                    spot: Some(SpotHalo {
                        texture: halo,
                        width: width * METERS_PER_UNIT,
                        color: rgb,
                        scale: 60.0 * METERS_PER_UNIT,
                        proxy: (1.0 + 60.0 * width / end_width).clamp(1.0, 8.0) * METERS_PER_UNIT,
                    }),
                    start_on: flags & 1 != 0,
                });
            }
            "env_laser" | "env_beam" => {
                let Some(texture) = e.prop("texture").and_then(|t| sprite(materials, t)) else {
                    continue;
                };
                let laser = class == "env_laser";
                let (start, end) = if laser {
                    (Some(origin), e.prop("LaserTarget").and_then(origin_of))
                } else {
                    // Strike generators (life ≠ 0, or a ring) send short
                    // beams: not drawn.
                    if num("life").unwrap_or(0.0) != 0.0 || flags & 8 != 0 {
                        continue;
                    }
                    (
                        e.prop("LightningStart").and_then(origin_of),
                        e.prop("LightningEnd").and_then(origin_of),
                    )
                };
                let (Some(start), Some(end)) = (start, end) else { continue };
                let width = num(if laser { "width" } else { "BoltWidth" }).unwrap_or(1.0).clamp(0.0, 102.3);
                let end_width = if !laser && flags & 512 != 0 { 0.0 } else { width };
                let brightness = num("renderamt").unwrap_or(255.0).clamp(0.0, 255.0) / 255.0;
                let shade = if laser {
                    0
                } else if flags & 128 != 0 {
                    2
                } else if flags & 256 != 0 {
                    1
                } else {
                    0
                };
                let named = e.prop("targetname").is_some_and(|n| !n.is_empty());
                beams.push(MapBeam {
                    entity: Some(index),
                    start: engine(start),
                    end: engine(end),
                    width: width * METERS_PER_UNIT,
                    end_width: end_width * METERS_PER_UNIT,
                    color: rgb * brightness,
                    shade,
                    fade: 0.0,
                    texture,
                    spot: None,
                    start_on: !named || flags & 1 != 0,
                });
            }
            "env_lightglow" => {
                let Some(texture) = sprite(materials, "sprites/light_glow02_add_noz") else {
                    continue;
                };
                let h = num("HorizontalGlowSize").unwrap_or(0.0);
                let v = num("VerticalGlowSize").unwrap_or(0.0);
                glows.push(MapGlow {
                    entity: Some(index),
                    position: engine(origin),
                    texture,
                    color: rgb,
                    half: Vec2::new(h, v) * METERS_PER_UNIT,
                    min: num("MinDist").unwrap_or(0.0) * METERS_PER_UNIT,
                    max: num("MaxDist").unwrap_or(0.0).min(65535.0) * METERS_PER_UNIT,
                    outer: num("OuterMaxDist").unwrap_or(0.0).min(65535.0) * METERS_PER_UNIT,
                    front: (flags & 1 != 0).then(|| engine(forward(angles)) / METERS_PER_UNIT),
                    proxy: num("GlowProxySize").unwrap_or(2.0).clamp(0.0, 64.0).max(1.0) * METERS_PER_UNIT,
                });
            }
            _ => {}
        }
    }
    data.beams = beams;
    data.glows = glows;
}
