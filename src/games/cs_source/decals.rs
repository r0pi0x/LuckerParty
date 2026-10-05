//! Map decals (`infodecal`): applied once at load onto the world faces they
//! touch, like the game does. Each decal is a texture-sized rectangle
//! (texture size x `$decalscale`, in map units) centred on its origin and
//! upright on walls; every nearby face polygon is clipped to it. Decals
//! reuse the face's lightmap, so they sit in the same light as the surface.

use std::collections::BTreeMap;

use bevy::prelude::*;
use vbsp::{Bsp, TextureFlags};

use super::{
    bsp::{LightmapLayout, to_engine},
    lightmap,
    material::MaterialLoader,
};
use crate::map::{MapData, MapMesh};

/// Faces whose plane passes within this distance of the decal's origin
/// (map units) receive it, or half the decal's smaller side if larger:
/// decals reach surfaces within their own size. On dust2 some decal
/// origins sit 4.5-11 units in front of their wall.
const PLANE_DISTANCE: f32 = 4.0;
/// Lift off the surface (map units) so decals draw over it.
const OFFSET: f32 = 0.15;

fn v3(v: vbsp::Vector) -> Vec3 {
    Vec3::new(v.x, v.y, v.z)
}

fn vb(v: Vec3) -> vbsp::Vector {
    vbsp::Vector { x: v.x, y: v.y, z: v.z }
}

/// Decal axes on a surface with normal `n` (Source space): right and down
/// as seen from in front. Walls stay upright; floors and ceilings align to
/// world X.
pub fn basis(n: Vec3) -> (Vec3, Vec3) {
    let up = if n.z.abs() < 0.7 { Vec3::Z } else { Vec3::X };
    let right = up.cross(n).normalize_or_zero();
    let down = -n.cross(right).normalize_or_zero();
    (right, down)
}

/// Clip a polygon (position, uv) to 0 <= u, v <= 1.
fn clip(mut poly: Vec<(Vec3, Vec2)>) -> Vec<(Vec3, Vec2)> {
    for (axis, keep_above) in [(0, true), (0, false), (1, true), (1, false)] {
        let inside = |p: &(Vec3, Vec2)| if keep_above { p.1[axis] >= 0.0 } else { p.1[axis] <= 1.0 };
        let edge = if keep_above { 0.0 } else { 1.0 };
        let mut out = Vec::with_capacity(poly.len() + 4);
        for i in 0..poly.len() {
            let a = poly[i];
            let b = poly[(i + 1) % poly.len()];
            if inside(&a) {
                out.push(a);
            }
            if inside(&a) != inside(&b) {
                let t = (edge - a.1[axis]) / (b.1[axis] - a.1[axis]);
                out.push((a.0.lerp(b.0, t), a.1.lerp(b.1, t)));
            }
        }
        poly = out;
        if poly.is_empty() {
            break;
        }
    }
    poly
}

pub fn add_decals(bsp: &Bsp, layout: &LightmapLayout, materials: &mut MaterialLoader, data: &mut MapData) {
    let Some(atlas) = data.lightmap.clone() else { return };
    let Some(world) = bsp.models().next() else { return };
    let faces: Vec<_> = world.faces().collect();
    let mut meshes: BTreeMap<String, MapMesh> = BTreeMap::new();
    let mut unplaced = 0;

    for ent in bsp.entities.iter().filter(|e| e.prop("classname") == Some("infodecal")) {
        let (Some(name), Some(origin)) = (ent.prop("texture"), ent.prop("origin")) else {
            continue;
        };
        let mut it = origin.split_whitespace().filter_map(|v| v.parse::<f32>().ok());
        let (Some(x), Some(y), Some(z)) = (it.next(), it.next(), it.next()) else {
            continue;
        };
        let origin = Vec3::new(x, y, z);
        let name = name.to_lowercase();

        let r = materials.resolve(&name);
        let Some(tex) = r.texture else { continue };
        let scale = r.decal_scale.unwrap_or(1.0);
        let size = Vec2::new(
            materials.textures[tex].width as f32,
            materials.textures[tex].height as f32,
        ) * scale;
        let max_distance = PLANE_DISTANCE.max(size.min_element() / 2.0);
        let reach = size.length() / 2.0 + max_distance;

        let mut placed = false;
        for (fi, face) in faces.iter().enumerate() {
            let texinfo = face.texture();
            if face.displacement().is_some()
                || texinfo
                    .flags
                    .intersects(TextureFlags::NODRAW | TextureFlags::SKY | TextureFlags::SKY2D | TextureFlags::TRIGGER)
            {
                continue;
            }
            let Some(plane) = bsp.plane(face.plane_num as usize) else {
                continue;
            };
            let n = v3(plane.normal);
            // The decal must be in front of the face (not behind a thin wall).
            let distance = n.dot(origin) - plane.dist;
            if !(-1.0..=max_distance).contains(&distance) {
                continue;
            }
            let poly: Vec<Vec3> = face.vertices().map(|v| v3(v.position)).collect();
            if poly.iter().all(|p| p.distance(origin) > reach * 4.0) {
                continue;
            }
            let (right, down) = basis(n);
            let projected = poly
                .iter()
                .map(|p| {
                    let d = *p - origin;
                    (*p, Vec2::new(d.dot(right) / size.x + 0.5, d.dot(down) / size.y + 0.5))
                })
                .collect();
            let clipped = clip(projected);
            if clipped.len() < 3 {
                continue;
            }
            placed = true;
            let slot = layout.face_slots.get(fi).copied().flatten().unwrap_or(layout.white);
            let mesh = meshes.entry(name.clone()).or_insert_with(|| MapMesh {
                material: format!("decal:{name}"),
                color: [255, 255, 255],
                texture: Some(tex),
                alpha: r.alpha,
                double_sided: false,
                ..default()
            });
            let normal = to_engine(vb(n)).normalize_or_zero();
            let base = mesh.positions.len() as u32;
            for (p, uv) in &clipped {
                mesh.positions.push(to_engine(vb(*p + n * OFFSET)).to_array());
                mesh.normals.push(normal.to_array());
                mesh.uvs.push(uv.to_array());
                let luxel = if layout.face_slots.get(fi).copied().flatten().is_some() {
                    lightmap::luxel_coords(&texinfo, face, vb(*p))
                } else {
                    Vec2::splat(0.5)
                };
                mesh.lightmap_uvs
                    .push(lightmap::atlas_uv(&atlas, layout.placements[slot], luxel));
            }
            // Fan, wound counter-clockwise as seen from the front: the
            // conversion to engine space is a rotation, so check there.
            for i in 1..clipped.len() as u32 - 1 {
                let [a, b, c] = [base, base + i, base + i + 1];
                let pa = Vec3::from(mesh.positions[a as usize]);
                let pb = Vec3::from(mesh.positions[b as usize]);
                let pc = Vec3::from(mesh.positions[c as usize]);
                if (pb - pa).cross(pc - pa).dot(normal) < 0.0 {
                    mesh.indices.extend([a, c, b]);
                } else {
                    mesh.indices.extend([a, b, c]);
                }
            }
        }
        if !placed {
            unplaced += 1;
        }
    }
    if unplaced > 0 {
        data.warnings
            .push(format!("{unplaced} decals found no surface to project onto"));
    }
    data.meshes.extend(meshes.into_values());
}
