//! Ropes and cables (`move_rope` / `keyframe_rope`): each rope entity with a
//! `nextkey` hangs a cable to that entity. Source simulates the rope; maps
//! are static here, so we draw the settled shape: a parabola whose sag makes
//! its length the straight distance plus `slack` (arc length of a shallow
//! parabola ~ d + 8 h^2 / 3d). Cables become thin tubes, lit by a light probe
//! at their middle like props.

use std::collections::HashMap;

use bevy::prelude::*;
use vbsp::Bsp;

use super::{
    ambient::{MapLighting, Occluders},
    bsp::{METERS_PER_UNIT, to_engine},
    material::MaterialLoader,
    props::probe,
};
use crate::map::{MapData, MapMesh, MapModel, MapProp, PropSolid};

const SEGMENTS: usize = 16;
const SIDES: usize = 6;
const ROPE_WIDTH_SCALE: f32 = 2.0;
/// Fraction of the full hanging sag. Calibrated with refcmp on de_dust2's
/// sky views (0.35 best matches the A-site cables overhead); Source's rope
/// simulation evidently doesn't hang ropes to their full slack.
const ROPE_SAG_SCALE: f32 = 0.35;

fn parse(v: &str) -> Option<Vec3> {
    let mut it = v.split_whitespace().filter_map(|p| p.parse::<f32>().ok());
    Some(Vec3::new(it.next()?, it.next()?, it.next()?))
}

pub fn add_ropes(
    bsp: &Bsp,
    materials: &mut MaterialLoader,
    lighting: &MapLighting,
    occluders: &Occluders,
    data: &mut MapData,
) {
    let ropes: Vec<_> = bsp
        .entities
        .iter()
        .filter(|e| matches!(e.prop("classname"), Some("move_rope") | Some("keyframe_rope")))
        .collect();
    let by_name: HashMap<&str, Vec3> = ropes
        .iter()
        .filter_map(|e| Some((e.prop("targetname")?, parse(e.prop("origin")?)?)))
        .collect();
    let mut material_cache: HashMap<String, MapMesh> = HashMap::new();

    for e in &ropes {
        let (Some(start), Some(end)) = (
            e.prop("origin").and_then(parse),
            e.prop("nextkey").and_then(|n| by_name.get(n).copied()),
        ) else {
            continue;
        };
        let slack: f32 = e
            .prop("slack")
            .and_then(|s| s.parse::<f32>().ok())
            .unwrap_or(25.0)
            .max(0.0);
        let width: f32 = e
            .prop("width")
            .and_then(|s| s.parse::<f32>().ok())
            .unwrap_or(2.0)
            .max(0.1);
        let material = e.prop("ropematerial").unwrap_or("cable/cable").to_lowercase();
        let template = material_cache.entry(material.clone()).or_insert_with(|| {
            let r = materials.resolve(&material);
            MapMesh {
                material: format!("rope:{material}"),
                color: [30, 30, 30],
                texture: r.texture,
                // Source's Cable shader draws ropes solid.
                alpha: crate::map::MapAlpha::Opaque,
                double_sided: true,
                ..default()
            }
        });

        // Sag (Source units) for the extra length, applied downward.
        let d = start.distance(end).max(1.0);
        let sag_scale = std::env::var("MASHUP_ROPE_SAG")
            .ok()
            .and_then(|v| v.parse::<f32>().ok())
            .unwrap_or(ROPE_SAG_SCALE);
        let sag = (3.0 * d * slack / 8.0).sqrt() * sag_scale;
        let point = |t: f32| start.lerp(end, t) - Vec3::Z * 4.0 * sag * t * (1.0 - t);
        // Measured with refcmp on de_dust2: width-1 cables look about 2 units
        // thick in CS:S. Whether Source scales width or enforces a minimum
        // screen width is not known yet.
        let radius = width * ROPE_WIDTH_SCALE / 2.0;

        let mut mesh = MapMesh {
            positions: Vec::new(),
            normals: Vec::new(),
            uvs: Vec::new(),
            indices: Vec::new(),
            ..template.clone()
        };
        for i in 0..=SEGMENTS {
            let t = i as f32 / SEGMENTS as f32;
            let p = point(t);
            let tangent = (point((t + 0.01).min(1.0)) - point((t - 0.01).max(0.0))).normalize_or(Vec3::X);
            let side = tangent.any_orthonormal_vector();
            let up = tangent.cross(side);
            for k in 0..=SIDES {
                let a = k as f32 / SIDES as f32 * std::f32::consts::TAU;
                let n = side * a.cos() + up * a.sin();
                let pos = p + n * radius;
                mesh.positions.push(
                    to_engine(vbsp::Vector {
                        x: pos.x,
                        y: pos.y,
                        z: pos.z,
                    })
                    .to_array(),
                );
                mesh.normals.push(
                    to_engine(vbsp::Vector { x: n.x, y: n.y, z: n.z })
                        .normalize_or_zero()
                        .to_array(),
                );
                mesh.uvs.push([k as f32 / SIDES as f32, t * d / 64.0]);
            }
        }
        let ring = (SIDES + 1) as u32;
        for i in 0..SEGMENTS as u32 {
            for k in 0..SIDES as u32 {
                let (a, b, c, dd) = (
                    i * ring + k,
                    i * ring + k + 1,
                    (i + 1) * ring + k,
                    (i + 1) * ring + k + 1,
                );
                mesh.indices.extend([a, c, b, b, c, dd]);
            }
        }
        let middle = point(0.5);
        let probe_at = to_engine(vbsp::Vector {
            x: middle.x,
            y: middle.y,
            z: middle.z,
        });
        data.models.push(MapModel {
            meshes: vec![mesh],
            bounds: (Vec3::splat(-0.1), Vec3::splat(0.1)),
        });
        data.props.push(MapProp {
            model: data.models.len() - 1,
            translation: Vec3::ZERO,
            rotation: Quat::IDENTITY,
            solid: PropSolid::None,
            skybox: false,
            lighting: Some(probe(
                bsp,
                lighting,
                occluders,
                probe_at + Vec3::Y * 4.0 * METERS_PER_UNIT,
            )),
        });
    }
}
