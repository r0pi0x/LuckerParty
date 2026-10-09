//! Map decals (`infodecal`): applied once at load onto the world faces they
//! touch, like the game does. Each decal is a texture-sized rectangle
//! (texture size x `$decalscale`, in map units) centred on its origin and
//! upright on walls; every nearby face polygon (or displacement triangle)
//! is clipped to it. Decals reuse the face's lightmap, so they sit in the
//! same light as the surface.

use std::collections::{BTreeMap, HashMap};

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

/// Clip a polygon (position, uv, lightmap luxel) to 0 <= u, v <= 1.
fn clip(mut poly: Vec<(Vec3, Vec2, Vec2)>) -> Vec<(Vec3, Vec2, Vec2)> {
    for (axis, keep_above) in [(0, true), (0, false), (1, true), (1, false)] {
        let inside = |p: &(Vec3, Vec2, Vec2)| if keep_above { p.1[axis] >= 0.0 } else { p.1[axis] <= 1.0 };
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
                out.push((a.0.lerp(b.0, t), a.1.lerp(b.1, t), a.2.lerp(b.2, t)));
            }
        }
        poly = out;
        if poly.is_empty() {
            break;
        }
    }
    poly
}

/// The probe an infodecal finds its target with: the diagonal segment
/// origin -/+ (5, 5, 5) units (specs/cs_source/overlays_decals.md, "Finding
/// the target entity").
const PROBE: f32 = 5.0;

/// Where along `a`..`b` (0..1) the segment first enters the box, if it does.
fn segment_box(a: Vec3, b: Vec3, min: Vec3, max: Vec3) -> Option<f32> {
    let d = b - a;
    let (mut t0, mut t1) = (0.0f32, 1.0f32);
    for i in 0..3 {
        if d[i].abs() < 1e-6 {
            if a[i] < min[i] || a[i] > max[i] {
                return None;
            }
            continue;
        }
        let (mut near, mut far) = ((min[i] - a[i]) / d[i], (max[i] - a[i]) / d[i]);
        if near > far {
            std::mem::swap(&mut near, &mut far);
        }
        t0 = t0.max(near);
        t1 = t1.min(far);
        if t0 > t1 {
            return None;
        }
    }
    Some(t0)
}

/// The brush entity an infodecal at `origin` belongs to: the first solid,
/// drawn one (bounds `min`..`max` in its own space, placed by `transform`)
/// its probe segment crosses. Index into `boxes`; None: the world.
pub fn decal_target(origin: Vec3, boxes: &[((Quat, Vec3), Vec3, Vec3)]) -> Option<usize> {
    boxes
        .iter()
        .enumerate()
        .filter_map(|(i, ((r, o), min, max))| {
            let local = |p: Vec3| r.inverse() * (p - *o);
            segment_box(local(origin - Vec3::splat(PROBE)), local(origin + Vec3::splat(PROBE)), *min, *max)
                .map(|t| (t, i))
        })
        .min_by(|a, b| a.0.total_cmp(&b.0))
        .map(|(_, i)| i)
}

/// The faces one decal may land on.
struct Surface<'a> {
    /// Faces with their lightmap block.
    faces: Vec<(vbsp::Handle<'a, vbsp::Face>, Option<usize>)>,
    /// Source-space placement of the faces (static brush entities), or
    /// None (the world, and movers, which keep their own space).
    place: Option<(Quat, Vec3)>,
    /// A mover's entity index: the decal goes with it.
    entity: Option<usize>,
}

/// Infodecal counts of one map: placed on the world, placed on brush
/// entities, and not placed (no face within reach).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DecalCounts {
    pub world: usize,
    pub entities: usize,
    pub unplaced: usize,
}

pub fn add_decals(bsp: &Bsp, layout: &LightmapLayout, materials: &mut MaterialLoader, data: &mut MapData) -> DecalCounts {
    let mut counts = DecalCounts::default();
    let Some(atlas) = data.lightmap.clone() else { return counts };
    let Some(world) = bsp.models().next() else { return counts };
    let world = Surface {
        faces: world
            .faces()
            .enumerate()
            .map(|(i, f)| (f, layout.face_slots.get(i).copied().flatten()))
            .collect(),
        place: None,
        entity: None,
    };
    // Solid, drawn brush entities: what the probe can hit.
    let entities: Vec<_> = super::bsp::brush_entities(bsp)
        .into_iter()
        .filter(|e| e.drawn && e.solid)
        .filter_map(|e| Some((bsp.models().nth(e.model)?, e)))
        .collect();
    let boxes: Vec<_> = entities
        .iter()
        .map(|(m, e)| (e.transform, v3(m.mins), v3(m.maxs)))
        .collect();
    // Per (texture, in the 3D skybox, mover).
    let mut meshes: BTreeMap<(String, bool, Option<usize>), MapMesh> = BTreeMap::new();
    let bounds = super::bsp::playable_bounds(bsp);

    for ent in bsp.entities.iter().filter(|e| e.prop("classname") == Some("infodecal")) {
        let (Some(name), Some(origin)) = (ent.prop("texture"), ent.prop("origin")) else {
            continue;
        };
        let mut it = origin.split_whitespace().filter_map(|v| v.parse::<f32>().ok());
        let (Some(x), Some(y), Some(z)) = (it.next(), it.next(), it.next()) else {
            continue;
        };
        let origin = Vec3::new(x, y, z);
        let skybox = bounds.as_ref().is_some_and(|b| !b.contains_point(origin));
        let name = name.to_lowercase();

        let r = materials.resolve(&name);
        let Some(tex) = r.texture else { continue };
        let scale = r.decal_scale.unwrap_or(1.0);
        let size = Vec2::new(
            materials.textures[tex].width as f32,
            materials.textures[tex].height as f32,
        ) * scale;
        let decal = Decal {
            name: &name,
            tex,
            alpha: r.alpha,
            size,
            skybox,
        };
        // The brush entity the probe hits, if any (else the world; and the
        // world too if none of the entity's faces takes the decal).
        let on_entity = decal_target(origin, &boxes).is_some_and(|i| {
            let (model, e) = &entities[i];
            let first = model.first_face.max(0) as usize;
            let surface = Surface {
                faces: model
                    .faces()
                    .enumerate()
                    .map(|(j, f)| (f, layout.entity_face_slots.get(&(first + j)).copied()))
                    .collect(),
                place: (!e.mover).then_some(e.transform),
                entity: e.mover.then_some(e.entity),
            };
            // Movers keep their faces in their own unrotated space.
            let at = if e.mover {
                e.transform.0.inverse() * (origin - e.transform.1)
            } else {
                origin
            };
            project(&decal, at, &surface, layout, &atlas, &mut meshes)
        });
        if on_entity {
            counts.entities += 1;
        } else if project(&decal, origin, &world, layout, &atlas, &mut meshes) {
            counts.world += 1;
        } else {
            counts.unplaced += 1;
            debug!("infodecal {name} at {origin}: no surface within reach");
        }
    }
    if counts.unplaced > 0 {
        data.warnings
            .push(format!("{} decals found no surface to project onto", counts.unplaced));
    }
    info!(
        "infodecals: {} on the world, {} on brush entities, {} unplaced",
        counts.world, counts.entities, counts.unplaced
    );
    data.meshes.extend(meshes.into_values());
    counts
}

/// A map decal's look and size (units).
struct Decal<'a> {
    name: &'a str,
    tex: usize,
    alpha: crate::map::MapAlpha,
    size: Vec2,
    skybox: bool,
}

/// Clip the decal centred at `origin` (in the surface's own space) onto
/// every face of `surface` within reach; false when none takes it.
fn project(
    decal: &Decal,
    origin: Vec3,
    surface: &Surface,
    layout: &LightmapLayout,
    atlas: &crate::map::MapLightmap,
    meshes: &mut BTreeMap<(String, bool, Option<usize>), MapMesh>,
) -> bool {
    let size = decal.size;
    let max_distance = PLANE_DISTANCE.max(size.min_element() / 2.0);
    let reach = size.length() / 2.0 + max_distance;
    let (rotation, offset) = surface.place.unwrap_or((Quat::IDENTITY, Vec3::ZERO));
    let place = |v: vbsp::Vector| rotation * v3(v) + offset;
    let mut placed = false;
    for (face, slot) in &surface.faces {
        let texinfo = face.texture();
        if texinfo
            .flags
            .intersects(TextureFlags::NODRAW | TextureFlags::SKY | TextureFlags::SKY2D | TextureFlags::TRIGGER)
        {
            continue;
        }
        let plane_normal = rotation * v3(face.normal());
        // Polygons with their plane normal: the face itself, or a
        // displacement's triangles (terrain; luxels follow its grid).
        let polys: Vec<(Vec3, Vec<(Vec3, Vec2)>)> = if face.displacement().is_some() {
            super::bsp::face_triangles(face)
                .into_iter()
                .filter_map(|t| {
                    let p = t.map(|(v, l)| (place(v), l));
                    let n = (p[1].0 - p[0].0).cross(p[2].0 - p[0].0).normalize_or_zero();
                    let n = if n.dot(plane_normal) < 0.0 { -n } else { n };
                    (n != Vec3::ZERO).then(|| (n, p.to_vec()))
                })
                .collect()
        } else {
            let poly = face
                .vertices()
                .map(|v| (place(v.position), lightmap::luxel_coords(&texinfo, face, v.position)))
                .collect();
            vec![(plane_normal, poly)]
        };
        for (n, poly) in polys {
            // The decal must be in front of the face (not behind a thin wall).
            let distance = n.dot(origin - poly[0].0);
            if !(-1.0..=max_distance).contains(&distance) {
                continue;
            }
            // A quick skip of faces out of reach: by their box (a large
            // floor's corners can all be far from a decal in its middle).
            let (lo, hi) = poly
                .iter()
                .fold((Vec3::MAX, Vec3::MIN), |(lo, hi), (p, _)| (lo.min(*p), hi.max(*p)));
            if origin.clamp(lo, hi).distance(origin) > reach {
                continue;
            }
            let (right, down) = basis(n);
            let projected = poly
                .iter()
                .map(|&(p, luxel)| {
                    let d = p - origin;
                    (
                        p,
                        Vec2::new(d.dot(right) / size.x + 0.5, d.dot(down) / size.y + 0.5),
                        luxel,
                    )
                })
                .collect();
            let clipped = clip(projected);
            if clipped.len() < 3 {
                continue;
            }
            placed = true;
            let lit = slot.is_some();
            let slot = slot.unwrap_or(layout.white);
            let mesh = meshes
                .entry((decal.name.to_string(), decal.skybox, surface.entity))
                .or_insert_with(|| MapMesh {
                    material: format!("decal:{}", decal.name),
                    skybox: decal.skybox && surface.entity.is_none(),
                    color: [255, 255, 255],
                    texture: Some(decal.tex),
                    alpha: decal.alpha,
                    double_sided: false,
                    entity: surface.entity,
                    ..default()
                });
            let normal = to_engine(vb(n)).normalize_or_zero();
            let base = mesh.positions.len() as u32;
            for (p, uv, luxel) in &clipped {
                mesh.positions.push(to_engine(vb(*p + n * OFFSET)).to_array());
                mesh.normals.push(normal.to_array());
                mesh.uvs.push(uv.to_array());
                let luxel = if lit { *luxel } else { Vec2::splat(0.5) };
                mesh.lightmap_uvs
                    .push(lightmap::atlas_uv(atlas, layout.placements[slot], luxel));
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
    }
    placed
}

/// Runtime decals from `scripts/decals_subrect.txt`: named groups of
/// weighted decal materials ("Impact.Concrete": shot1..5), and the group
/// each surface game material letter takes when shot ("TranslationData";
/// an empty name means no decal).
pub fn impact_decals(materials: &mut MaterialLoader) -> crate::map::decal::MapDecals {
    let mut out = crate::map::decal::MapDecals::default();
    let Some(text) = materials
        .read("scripts/decals_subrect.txt")
        .map(|b| String::from_utf8_lossy(&b).into_owned())
    else {
        return out;
    };
    let t = super::surfaceprops::tokens(&text);
    let mut by_path: HashMap<String, Option<usize>> = HashMap::new();
    let mut i = 0;
    while i + 1 < t.len() {
        if t[i + 1] != "{" {
            i += 1;
            continue;
        }
        let name = t[i].to_lowercase();
        i += 2;
        let mut entries = Vec::new();
        while i + 1 < t.len() && t[i] != "}" {
            entries.push((t[i].clone(), t[i + 1].clone()));
            i += 2;
        }
        i += 1;
        if name == "translationdata" {
            for (letter, group) in entries {
                if let Some(c) = letter.chars().next().filter(|_| !group.is_empty()) {
                    out.by_material.insert(c.to_ascii_uppercase(), group.to_lowercase());
                }
            }
            continue;
        }
        let mut variants = Vec::new();
        for (path, weight) in entries {
            let key = path.to_lowercase();
            let index = *by_path.entry(key).or_insert_with(|| {
                let d = materials.decal(&path)?;
                out.decals.push(d);
                Some(out.decals.len() - 1)
            });
            if let Some(index) = index {
                variants.push((weight.parse().unwrap_or(1.0), index));
            }
        }
        if !variants.is_empty() {
            out.groups.insert(name, variants);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The probe (origin -/+ 5 units diagonally) picks the brush entity it
    /// crosses first, in the entity's own space; else the world.
    #[test]
    fn probe_picks_the_brush_entity() {
        let door = (
            (Quat::IDENTITY, Vec3::new(100.0, 0.0, 0.0)),
            Vec3::new(-2.0, -32.0, 0.0),
            Vec3::new(2.0, 32.0, 96.0),
        );
        // A decal 4 units in front of the door's face (x = 102 in the world).
        assert_eq!(decal_target(Vec3::new(106.0, 0.0, 48.0), &[door]), Some(0));
        // 8 units away: the probe misses it.
        assert_eq!(decal_target(Vec3::new(110.0, 0.0, 48.0), &[door]), None);
        // Turned 90 degrees about Z: its faces now face y (world y -2..2).
        let turned = (
            (Quat::from_rotation_z(90f32.to_radians()), Vec3::new(100.0, 0.0, 0.0)),
            door.1,
            door.2,
        );
        assert_eq!(decal_target(Vec3::new(100.0, 6.0, 48.0), &[turned]), Some(0));
        assert_eq!(decal_target(Vec3::new(100.0, 8.0, 48.0), &[turned]), None);
        assert_eq!(decal_target(Vec3::new(140.0, 0.0, 48.0), &[turned]), None);
        // Two in reach: the one the segment (from origin - 5) enters first.
        let near = ((Quat::IDENTITY, Vec3::ZERO), Vec3::splat(-10.0), Vec3::new(-3.0, 10.0, 10.0));
        let far = ((Quat::IDENTITY, Vec3::ZERO), Vec3::new(3.0, -10.0, -10.0), Vec3::splat(10.0));
        assert_eq!(decal_target(Vec3::ZERO, &[far, near]), Some(1));
    }
}
