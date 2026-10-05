//! CS:S maps (BSP v20) to neutral `MapData`. Parsing is the `vbsp` crate;
//! this module owns Source's conventions: Z-up inches to Y-up meters, which
//! surfaces draw and collide, and which entities are spawns.

use std::collections::BTreeMap;

use bevy::prelude::*;
use vbsp::{Bsp, TextureFlags};

use crate::{
    core::Team,
    map::{MapData, MapMesh},
    mount::Mount,
};

/// One Hammer unit is one inch.
pub const METERS_PER_UNIT: f32 = 0.0254;

/// Source (x forward, y left, z up) to engine (y up, -z forward), in meters.
/// A rotation, so handedness and winding are preserved.
pub fn to_engine(v: vbsp::Vector) -> Vec3 {
    Vec3::new(v.x, v.z, -v.y) * METERS_PER_UNIT
}

fn to_engine_dir(v: vbsp::Vector) -> Vec3 {
    Vec3::new(v.x, v.z, -v.y).normalize_or_zero()
}

/// Surfaces that are never drawn.
const NOT_DRAWN: TextureFlags = TextureFlags::SKY2D
    .union(TextureFlags::SKY)
    .union(TextureFlags::TRIGGER)
    .union(TextureFlags::HINT)
    .union(TextureFlags::SKIP)
    .union(TextureFlags::NODRAW);
/// Surfaces players never collide with.
const NOT_SOLID: TextureFlags = TextureFlags::SKY2D
    .union(TextureFlags::SKY)
    .union(TextureFlags::TRIGGER)
    .union(TextureFlags::HINT)
    .union(TextureFlags::SKIP);

pub fn load(mount: &Mount, name: &str) -> Result<MapData, String> {
    let path = format!("maps/{name}.bsp");
    let bytes = mount.read(&path).map_err(|e| format!("{path}: {e}"))?;
    let bsp = Bsp::read(&bytes).map_err(|e| format!("{path}: {e}"))?;
    Ok(convert(&bsp, name))
}

pub fn convert(bsp: &Bsp, name: &str) -> MapData {
    let mut by_material: BTreeMap<String, MapMesh> = BTreeMap::new();
    let mut data = MapData {
        name: format!("cs_source:{name}"),
        ..default()
    };

    // The world is model 0; brush entities (doors, breakables) come later.
    let world = bsp.models().next().expect("BSP has no world model");
    for face in world.faces() {
        let tex = face.texture();
        let flags = tex.flags;
        if flags.intersects(NOT_DRAWN) && flags.intersects(NOT_SOLID) {
            continue;
        }
        let face_normal = to_engine_dir(face.normal());
        // Triangles in Source space, re-wound to face the plane normal.
        let tris: Vec<[vbsp::Vector; 3]> = face
            .vertex_positions()
            .collect::<Vec<_>>()
            .as_chunks::<3>()
            .0
            .iter()
            .map(|t| {
                let [a, b, c] = [to_engine(t[0]), to_engine(t[1]), to_engine(t[2])];
                if (b - a).cross(c - a).dot(face_normal) < 0.0 {
                    [t[0], t[2], t[1]]
                } else {
                    [t[0], t[1], t[2]]
                }
            })
            .collect();

        if !flags.intersects(NOT_SOLID) {
            for t in &tris {
                let base = data.collision_positions.len() as u32;
                data.collision_positions
                    .extend(t.iter().map(|v| to_engine(*v).to_array()));
                data.collision_indices.push([base, base + 1, base + 2]);
            }
        }
        if flags.intersects(NOT_DRAWN) {
            continue;
        }

        let material = tex.name().to_lowercase();
        let mesh = by_material.entry(material.clone()).or_insert_with(|| MapMesh {
            material,
            color: tex.debug_color(),
            ..default()
        });
        let displaced = face.displacement().is_some();
        for t in &tris {
            let p: [Vec3; 3] = t.map(to_engine);
            // Brush faces are flat; displacements get per-triangle normals.
            let n = if displaced {
                (p[1] - p[0]).cross(p[2] - p[0]).normalize_or_zero()
            } else {
                face_normal
            };
            for v in t {
                mesh.indices.push(mesh.positions.len() as u32);
                mesh.positions.push(to_engine(*v).to_array());
                mesh.normals.push(n.to_array());
                mesh.uvs.push(tex.uv(*v));
            }
        }
    }
    data.meshes = by_material.into_values().filter(|m| !m.indices.is_empty()).collect();

    for ent in bsp.entities.iter() {
        let team = match ent.prop("classname") {
            Some("info_player_terrorist") => Some(Team(1)),
            Some("info_player_counterterrorist") => Some(Team(2)),
            _ => continue,
        };
        if let Some(origin) = ent.prop("origin").and_then(parse_vector) {
            data.spawns.push((to_engine(origin), team));
        }
    }
    data
}

fn parse_vector(s: &str) -> Option<vbsp::Vector> {
    let mut it = s.split_whitespace().map(|p| p.parse::<f32>());
    let (x, y, z) = (it.next()?.ok()?, it.next()?.ok()?, it.next()?.ok()?);
    Some(vbsp::Vector { x, y, z })
}
