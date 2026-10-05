//! CS:S maps (BSP v20) to neutral `MapData`. Parsing is the `vbsp` crate;
//! this module owns Source's conventions: Z-up inches to Y-up meters, which
//! surfaces draw and collide, and which entities are spawns.

use std::collections::BTreeMap;

use bevy::prelude::*;
use vbsp::{Bsp, TextureFlags};

use super::{
    lightmap::{self, AtlasBuilder},
    material::MaterialLoader,
};
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
    let mut data = convert(&bsp, lightmap::lighting_lump(&bytes), name);

    let mut materials = MaterialLoader::new(&bsp, mount);
    for mesh in &mut data.meshes {
        let r = materials.resolve(&mesh.material);
        mesh.texture = r.texture;
        mesh.alpha = r.alpha;
        mesh.double_sided = r.double_sided;
    }
    data.warnings.extend(materials.missing);
    data.textures = materials.textures;
    Ok(data)
}

/// Triangles of a face in Source space, each vertex with its lightmap
/// coordinate in luxels (see `lightmap::luxel_coords`).
///
/// Flat faces project through the face's lightmap vectors. Displacements
/// don't: their samples follow the displacement grid, with the first
/// lightmap axis along the grid's second axis. This was established by
/// measurement on de_dust2: that mapping makes lighting agree where
/// neighbouring displacements meet (mean mismatch 1%, versus 48% for
/// projection; tests/map_de_dust2.rs checks it).
pub fn face_triangles(face: &vbsp::Handle<'_, vbsp::Face>) -> Vec<[(vbsp::Vector, Vec2); 3]> {
    let tex = face.texture();
    let Some(disp) = face.displacement() else {
        return face
            .triangulate()
            .map(|t| t.map(|v| (v, lightmap::luxel_coords(&tex, face, v))))
            .collect();
    };
    // Base grid: bilinear over the face's corners, starting at the corner
    // nearest the displacement's start position.
    let mut corners: Vec<vbsp::Vector> = face.vertices().map(|v| v.position).collect();
    if corners.len() != 4 {
        return Vec::new();
    }
    let start = (0..4)
        .min_by(|&a, &b| {
            (corners[a] - disp.start_position)
                .length_squared()
                .total_cmp(&(corners[b] - disp.start_position).length_squared())
        })
        .unwrap();
    corners.rotate_left(start);
    let steps = 2usize.pow(disp.power as u32);
    let n = steps + 1;
    let lerp = |a: vbsp::Vector, b: vbsp::Vector, t: f32| a + (b - a) * t;
    let offsets: Vec<vbsp::Vector> = disp.displacement_vertices().map(|v| v.displacement()).collect();
    if offsets.len() != n * n {
        return Vec::new();
    }
    let size = Vec2::new(
        face.light_map_texture_size[0] as f32,
        face.light_map_texture_size[1] as f32,
    );
    let grid = |x: usize, y: usize| {
        let (fx, fy) = (x as f32 / steps as f32, y as f32 / steps as f32);
        let base = lerp(lerp(corners[0], corners[1], fx), lerp(corners[3], corners[2], fx), fy);
        (base + offsets[x * n + y], Vec2::new(fy, fx) * size)
    };
    let mut out = Vec::with_capacity(steps * steps * 2);
    for x in 0..steps {
        for y in 0..steps {
            out.push([grid(x, y), grid(x + 1, y), grid(x, y + 1)]);
            out.push([grid(x + 1, y), grid(x + 1, y + 1), grid(x, y + 1)]);
        }
    }
    out
}

/// Geometry, collision, lightmaps and spawns, without materials.
/// `lighting` is the BSP's lighting lump (see `lightmap::lighting_lump`).
pub fn convert(bsp: &Bsp, lighting: &[u8], name: &str) -> MapData {
    let mut by_material: BTreeMap<String, MapMesh> = BTreeMap::new();
    // Per mesh: each vertex's lightmap block slot and luxel coordinate,
    // resolved to atlas UVs once all blocks are packed.
    let mut pending_lm: BTreeMap<String, Vec<(Option<usize>, Vec2)>> = BTreeMap::new();
    let mut atlas = AtlasBuilder::default();
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
        // Re-wind triangles to face the plane normal.
        let tris: Vec<[(vbsp::Vector, Vec2); 3]> = face_triangles(&face)
            .into_iter()
            .map(|t| {
                let [a, b, c] = t.map(|(v, _)| to_engine(v));
                if (b - a).cross(c - a).dot(face_normal) < 0.0 {
                    [t[0], t[2], t[1]]
                } else {
                    t
                }
            })
            .collect();

        if !flags.intersects(NOT_SOLID) {
            for t in &tris {
                let base = data.collision_positions.len() as u32;
                data.collision_positions
                    .extend(t.iter().map(|(v, _)| to_engine(*v).to_array()));
                data.collision_indices.push([base, base + 1, base + 2]);
            }
        }
        if flags.intersects(NOT_DRAWN) {
            continue;
        }

        let slot = lightmap::face_samples(lighting, &face).map(|s| atlas.add(s));
        let material = tex.name().to_lowercase();
        let mesh = by_material.entry(material.clone()).or_insert_with(|| MapMesh {
            material: material.clone(),
            color: tex.debug_color(),
            ..default()
        });
        let lm = pending_lm.entry(material).or_default();
        let displaced = face.displacement().is_some();
        for t in &tris {
            let p: [Vec3; 3] = t.map(|(v, _)| to_engine(v));
            // Brush faces are flat; displacements get per-triangle normals.
            let n = if displaced {
                (p[1] - p[0]).cross(p[2] - p[0]).normalize_or_zero()
            } else {
                face_normal
            };
            for (v, luxel) in t {
                mesh.indices.push(mesh.positions.len() as u32);
                mesh.positions.push(to_engine(*v).to_array());
                mesh.normals.push(n.to_array());
                mesh.uvs.push(tex.uv(*v));
                lm.push((slot, *luxel));
            }
        }
    }

    let (lightmap, placements, white) = atlas.build();
    for (material, mesh) in by_material.iter_mut() {
        mesh.lightmap_uvs = pending_lm[material]
            .iter()
            .map(|&(slot, luxel)| match slot {
                Some(s) => lightmap::atlas_uv(&lightmap, placements[s], luxel),
                None => lightmap::atlas_uv(&lightmap, placements[white], Vec2::splat(0.5)),
            })
            .collect();
    }
    data.lightmap = Some(lightmap);
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
