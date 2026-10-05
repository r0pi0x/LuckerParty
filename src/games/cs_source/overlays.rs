//! Overlays (`info_overlay`, BSP lump 45): textures such as posters,
//! graffiti, wires and road markings laid over the faces they list. Format
//! per the public BSP description: an overlay is a quad (four points in its
//! own plane, around an origin) with texture coordinates at the corners; its
//! plane's U axis is packed into the corner points' unused third components.
//! Each listed face is clipped to the quad, and the pieces reuse the face's
//! lightmap, like decals.

use std::collections::BTreeMap;

use bevy::prelude::*;
use vbsp::Bsp;

use super::{
    bsp::{LightmapLayout, face_triangles, to_engine},
    lightmap,
    material::MaterialLoader,
};
use crate::map::{MapAlpha, MapData, MapMesh};

const LUMP_OVERLAYS: usize = 45;
const OVERLAY_SIZE: usize = 352;
/// Lift off the surface (map units) so overlays draw over it.
const OFFSET: f32 = 0.1;

fn f32_at(b: &[u8], at: usize) -> f32 {
    f32::from_le_bytes(b[at..at + 4].try_into().unwrap())
}

fn v3(b: &[u8], at: usize) -> Vec3 {
    Vec3::new(f32_at(b, at), f32_at(b, at + 4), f32_at(b, at + 8))
}

fn vb(v: Vec3) -> vbsp::Vector {
    vbsp::Vector { x: v.x, y: v.y, z: v.z }
}

/// Clip a convex polygon of (position, uv) against the half-plane
/// `dot(n, p2d) >= d` in the overlay's 2D plane coordinates.
fn clip(poly: Vec<(Vec3, Vec2, Vec2)>, n: Vec2, d: f32) -> Vec<(Vec3, Vec2, Vec2)> {
    let inside = |p: &(Vec3, Vec2, Vec2)| n.dot(p.1) >= d;
    let mut out = Vec::with_capacity(poly.len() + 2);
    for i in 0..poly.len() {
        let (a, b) = (poly[i], poly[(i + 1) % poly.len()]);
        if inside(&a) {
            out.push(a);
        }
        if inside(&a) != inside(&b) {
            let t = (d - n.dot(a.1)) / (n.dot(b.1) - n.dot(a.1));
            out.push((a.0.lerp(b.0, t), a.1.lerp(b.1, t), a.2.lerp(b.2, t)));
        }
    }
    out
}

pub fn add_overlays(
    bsp: &Bsp,
    bsp_bytes: &[u8],
    layout: &LightmapLayout,
    materials: &mut MaterialLoader,
    data: &mut MapData,
) {
    let lump = {
        let at = 8 + LUMP_OVERLAYS * 16;
        let ofs = i32::from_le_bytes(bsp_bytes[at..at + 4].try_into().unwrap()).max(0) as usize;
        let len = i32::from_le_bytes(bsp_bytes[at + 4..at + 8].try_into().unwrap()).max(0) as usize;
        bsp_bytes.get(ofs..ofs + len).unwrap_or(&[])
    };
    let Some(atlas) = data.lightmap.clone() else { return };
    let mut meshes: BTreeMap<String, MapMesh> = BTreeMap::new();
    let mut empty = 0;

    for o in lump.as_chunks::<OVERLAY_SIZE>().0 {
        let texinfo = i16::from_le_bytes([o[4], o[5]]);
        let face_count = (u16::from_le_bytes([o[6], o[7]]) & 0x3fff) as usize;
        let faces: Vec<usize> = (0..face_count.min(64))
            .map(|i| i32::from_le_bytes(o[8 + i * 4..12 + i * 4].try_into().unwrap()) as usize)
            .collect();
        let (u, v) = (
            Vec2::new(f32_at(o, 264), f32_at(o, 268)),
            Vec2::new(f32_at(o, 272), f32_at(o, 276)),
        );
        let corners: Vec<Vec3> = (0..4).map(|k| v3(o, 280 + k * 12)).collect();
        let origin = v3(o, 328);
        let normal = v3(o, 340).normalize_or_zero();
        let basis_u = Vec3::new(corners[0].z, corners[1].z, corners[2].z).normalize_or_zero();
        let basis_v = normal.cross(basis_u).normalize_or_zero();
        // Corner texture coordinates: (U0,V0), (U0,V1), (U1,V1), (U1,V0).
        let quad: [(Vec2, Vec2); 4] = [
            (corners[0].truncate(), Vec2::new(u.x, v.x)),
            (corners[1].truncate(), Vec2::new(u.x, v.y)),
            (corners[2].truncate(), Vec2::new(u.y, v.y)),
            (corners[3].truncate(), Vec2::new(u.y, v.x)),
        ];
        let Some(tex_name) = bsp
            .texture_info(texinfo.max(0) as usize)
            .map(|t| t.name().to_lowercase())
        else {
            continue;
        };
        let r = materials.resolve(&tex_name);
        if r.texture.is_none() {
            continue;
        }

        let mut placed = false;
        for &fi in &faces {
            let Some(face) = bsp.face(fi) else { continue };
            let slot = layout.face_slots.get(fi).copied().flatten();
            let face_normal = face.normal();
            let fnorm = Vec3::new(face_normal.x, face_normal.y, face_normal.z);
            // The face as drawn: for displacements, the raised terrain
            // triangles (not the flat base face, which lies under them),
            // each vertex with its lightmap coordinates.
            for tri3 in face_triangles(&face) {
                let [pa, pb, pc] = tri3.map(|(v, _)| Vec3::new(v.x, v.y, v.z));
                let mut tnorm = (pb - pa).cross(pc - pa).normalize_or_zero();
                if tnorm.dot(fnorm) < 0.0 {
                    tnorm = -tnorm;
                }
                // Triangle in overlay plane coordinates, carrying luxels.
                let poly: Vec<(Vec3, Vec2, Vec2)> = tri3
                    .iter()
                    .map(|(v, luxel)| {
                        let p = Vec3::new(v.x, v.y, v.z);
                        let d = p - origin;
                        (p, Vec2::new(d.dot(basis_u), d.dot(basis_v)), *luxel)
                    })
                    .collect();
                // Two triangles of the quad; texture coordinates by barycentrics.
                for tri in [[0usize, 1, 2], [0, 2, 3]] {
                    let [a, b, c] = tri.map(|i| quad[i]);
                    let area = (b.0 - a.0).perp_dot(c.0 - a.0);
                    if area.abs() < 1e-6 {
                        continue;
                    }
                    let s = area.signum();
                    let mut piece = poly.clone();
                    for (p, q) in [(a.0, b.0), (b.0, c.0), (c.0, a.0)] {
                        let edge = q - p;
                        let n = Vec2::new(-edge.y, edge.x) * s;
                        piece = clip(piece, n, n.dot(p));
                        if piece.is_empty() {
                            break;
                        }
                    }
                    if piece.len() < 3 {
                        continue;
                    }
                    placed = true;
                    let mesh = meshes.entry(tex_name.clone()).or_insert_with(|| MapMesh {
                        material: format!("decal:overlay:{tex_name}"),
                        color: [255, 255, 255],
                        texture: r.texture,
                        alpha: if r.alpha == MapAlpha::Opaque {
                            MapAlpha::Blend
                        } else {
                            r.alpha
                        },
                        ..default()
                    });
                    let engine_normal = to_engine(vb(tnorm)).normalize_or_zero();
                    let base = mesh.positions.len() as u32;
                    for (p, p2, luxel) in &piece {
                        // Barycentric texture coordinates within the triangle.
                        let w_b = (p2 - a.0).perp_dot(c.0 - a.0) / (b.0 - a.0).perp_dot(c.0 - a.0);
                        let w_c = (b.0 - a.0).perp_dot(*p2 - a.0) / (b.0 - a.0).perp_dot(c.0 - a.0);
                        let uv = a.1 * (1.0 - w_b - w_c) + b.1 * w_b + c.1 * w_c;
                        mesh.positions.push(to_engine(vb(*p + tnorm * OFFSET)).to_array());
                        mesh.normals.push(engine_normal.to_array());
                        mesh.uvs.push(uv.to_array());
                        let luxel = if slot.is_some() { *luxel } else { Vec2::splat(0.5) };
                        mesh.lightmap_uvs.push(lightmap::atlas_uv(
                            &atlas,
                            layout.placements[slot.unwrap_or(layout.white)],
                            luxel,
                        ));
                    }
                    for i in 1..piece.len() as u32 - 1 {
                        let [ia, ib, ic] = [base, base + i, base + i + 1];
                        let (qa, qb, qc) = (
                            Vec3::from(mesh.positions[ia as usize]),
                            Vec3::from(mesh.positions[ib as usize]),
                            Vec3::from(mesh.positions[ic as usize]),
                        );
                        if (qb - qa).cross(qc - qa).dot(engine_normal) < 0.0 {
                            mesh.indices.extend([ia, ic, ib]);
                        } else {
                            mesh.indices.extend([ia, ib, ic]);
                        }
                    }
                }
            }
        }
        if !placed {
            empty += 1;
        }
    }
    if empty > 0 {
        data.warnings.push(format!("{empty} overlays produced no geometry"));
    }
    data.meshes.extend(meshes.into_values());
}
