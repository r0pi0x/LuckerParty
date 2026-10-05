//! Static props (the BSP's `sprp` game lump): Source models (MDL + VVD +
//! dx90 VTX, parsed by `vmdl`) placed around the map. Each model is
//! converted once; props reference it with a transform.

use std::collections::HashMap;

use bevy::prelude::*;
use vbsp::Bsp;

use super::{
    ambient::{self, MapLighting, Occluders},
    bsp::{METERS_PER_UNIT, to_engine},
    material::MaterialLoader,
};
use crate::map::{LightProbe, MapCollision, MapConvex, MapData, MapMesh, MapModel, MapProp, PropSolid};

/// Source rotation (pitch about Y, yaw about Z, roll about X; degrees) in
/// engine axes.
pub fn rotation(angles: vbsp::Angles) -> Quat {
    let r = Quat::from_rotation_z(angles.yaw.to_radians())
        * Quat::from_rotation_y(angles.pitch.to_radians())
        * Quat::from_rotation_x(angles.roll.to_radians());
    // Engine = C * source, with C mapping (x, y, z) to (x, z, -y): a -90
    // degree turn about X. Conjugate the rotation into engine axes.
    let c = Quat::from_rotation_x(-std::f32::consts::FRAC_PI_2);
    c * r * c.inverse()
}

fn load_model(materials: &mut MaterialLoader, path: &str) -> Result<vmdl::Model, String> {
    let read = |p: String| materials.read(&p).ok_or_else(|| format!("{p}: not found"));
    let mdl = vmdl::mdl::Mdl::read(&read(path.to_string())?).map_err(|e| format!("{path}: {e}"))?;
    let stem = path.trim_end_matches(".mdl");
    let vtx = vmdl::vtx::Vtx::read(&read(format!("{stem}.dx90.vtx"))?).map_err(|e| format!("{stem}.dx90.vtx: {e}"))?;
    let vvd = vmdl::vvd::Vvd::read(&read(format!("{stem}.vvd"))?).map_err(|e| format!("{stem}.vvd: {e}"))?;
    Ok(vmdl::Model::from_parts(mdl, vtx, vvd))
}

fn v(p: vmdl::Vector) -> vbsp::Vector {
    vbsp::Vector { x: p.x, y: p.y, z: p.z }
}

/// Convert one model with one skin into engine-space meshes (meters).
fn convert_model(model: &vmdl::Model, skin: i32, materials: &mut MaterialLoader) -> MapModel {
    let skins: Vec<_> = model.skin_tables().collect();
    let table = skins.get(skin.max(0) as usize).or(skins.first());
    let dirs = model.texture_directories().to_vec();
    let mut by_material: HashMap<String, MapMesh> = HashMap::new();
    for mesh in model.meshes() {
        let Some(name) = table.and_then(|t| t.texture(mesh.material_index())) else {
            continue;
        };
        let entry = by_material.entry(name.to_lowercase()).or_insert_with(|| {
            let candidates: Vec<String> = dirs.iter().map(|d| format!("{d}{name}")).collect();
            let r = materials.resolve_any(&candidates);
            MapMesh {
                material: name.to_lowercase(),
                color: [200, 200, 200],
                texture: r.texture,
                alpha: r.alpha,
                double_sided: r.double_sided,
                unlit: r.unlit,
                ..default()
            }
        });
        let verts: Vec<&vmdl::vvd::Vertex> = mesh.vertices().collect();
        for tri in verts.as_chunks::<3>().0 {
            let p: Vec<Vec3> = tri
                .iter()
                .map(|t| to_engine(v(model.apply_root_transform(t.position))))
                .collect();
            let n: Vec<Vec3> = tri.iter().map(|t| to_engine(v(t.normal)).normalize_or_zero()).collect();
            // Wind counter-clockwise against the vertex normals.
            let face = (p[1] - p[0]).cross(p[2] - p[0]);
            let order = if face.dot(n[0] + n[1] + n[2]) < 0.0 {
                [0, 2, 1]
            } else {
                [0, 1, 2]
            };
            for i in order {
                entry.indices.push(entry.positions.len() as u32);
                entry.positions.push(p[i].to_array());
                entry.normals.push(n[i].to_array());
                entry.uvs.push(tri[i].texture_coordinates);
            }
        }
    }
    let mut meshes: Vec<MapMesh> = by_material.into_values().filter(|m| !m.indices.is_empty()).collect();
    meshes.sort_by(|a, b| a.material.cmp(&b.material));
    // Axis conversion is a rotation, so re-take min/max per engine axis.
    let (a, b) = model.bounding_box();
    let (a, b) = (to_engine(v(a)), to_engine(v(b)));
    MapModel {
        meshes,
        bounds: (a.min(b), a.max(b)),
        collision: None,
    }
}

/// The model's `.phy` as convex pieces in engine model space, if it has one.
fn load_collision(materials: &mut MaterialLoader, path: &str) -> Option<MapCollision> {
    let bytes = materials.read(&format!("{}.phy", path.trim_end_matches(".mdl")))?;
    let phy = super::phy::parse(&bytes)
        .inspect_err(|e| warn!("{path}.phy: {e}"))
        .ok()?;
    let pieces = phy
        .pieces
        .iter()
        .map(|tris| {
            let mut points: Vec<Vec3> = Vec::new();
            let mut planes: Vec<(Vec3, f32)> = Vec::new();
            for tri in tris {
                let [a, b, c] = tri.map(|v| to_engine(v_src(v)));
                for p in [a, b, c] {
                    if !points.iter().any(|q| q.distance_squared(p) < 1e-10) {
                        points.push(p);
                    }
                }
                let n = (b - a).cross(c - a).normalize_or_zero();
                if n == Vec3::ZERO {
                    continue;
                }
                let d = n.dot(a);
                // Coplanar triangles share one plane.
                if !planes.iter().any(|(m, e)| m.dot(n) > 0.9999 && (e - d).abs() < 1e-4) {
                    planes.push((n, d));
                }
            }
            MapConvex { points, planes }
        })
        .filter(|p| p.points.len() >= 4)
        .collect();
    Some(MapCollision {
        pieces,
        mass: phy.mass,
        damping: phy.damping,
        rotdamping: phy.rotdamping,
        inertia: phy.inertia,
        surfaceprop: phy.surfaceprop,
    })
}

fn v_src(v: Vec3) -> vbsp::Vector {
    vbsp::Vector { x: v.x, y: v.y, z: v.z }
}

/// Add the BSP's static props to `data`, loading each (model, skin) once.
/// One prop to place: what the static prop lump and prop entities share.
struct PropPlacement {
    model: String,
    skin: i32,
    origin: vbsp::Vector,
    angles: vbsp::Angles,
    solid: PropSolid,
    lighting_origin: Option<vbsp::Vector>,
    /// Entity classname (None for static props).
    class: Option<String>,
}

pub fn add_static_props(
    bsp: &Bsp,
    materials: &mut MaterialLoader,
    lighting: &MapLighting,
    occluders: &Occluders,
    data: &mut MapData,
) {
    let mut placements = Vec::new();
    for prop in bsp.static_props() {
        if prop.flags.contains(vbsp::StaticPropLumpFlags::NO_DRAW) {
            continue;
        }
        placements.push(PropPlacement {
            model: prop.model().to_lowercase(),
            skin: prop.skin,
            origin: prop.origin,
            angles: prop.angles,
            // Box solids collide as the model's bounds, like the game;
            // physics solids should use the .phy model, which isn't parsed
            // yet, so they fall back to the visible mesh.
            solid: match prop.solid {
                vbsp::SolidType::None => PropSolid::None,
                vbsp::SolidType::Bbox | vbsp::SolidType::Obb | vbsp::SolidType::ObbYaw => PropSolid::Box,
                _ => PropSolid::Mesh,
            },
            lighting_origin: prop
                .flags
                .contains(vbsp::StaticPropLumpFlags::USE_LIGHTING_ORIGIN)
                .then_some(prop.lighting_origin),
            class: None,
        });
    }
    placements.extend(entity_props(bsp));
    place_props(bsp, materials, lighting, occluders, data, placements);
}

/// Props placed as entities: physics props (barrels, baskets; static here
/// until there's physics) and dynamic props. Both collide by their physics
/// model (the visible mesh until `.phy` is parsed).
fn entity_props(bsp: &Bsp) -> Vec<PropPlacement> {
    let parse = |v: &str| -> Option<[f32; 3]> {
        let mut it = v.split_whitespace().filter_map(|p| p.parse::<f32>().ok());
        Some([it.next()?, it.next()?, it.next()?])
    };
    bsp.entities
        .iter()
        .filter_map(|e| {
            let class = e.prop("classname")?;
            let physics = class.starts_with("prop_physics");
            if !physics && !class.starts_with("prop_dynamic") {
                return None;
            }
            let [x, y, z] = parse(e.prop("origin")?)?;
            let [pitch, yaw, roll] = e.prop("angles").and_then(parse).unwrap_or([0.0; 3]);
            // prop_dynamic: solid 0 = not solid, otherwise the physics model.
            let solid = if physics || e.prop("solid").is_none_or(|s| s.trim() != "0") {
                PropSolid::Mesh
            } else {
                PropSolid::None
            };
            Some(PropPlacement {
                model: e.prop("model")?.to_lowercase(),
                skin: e.prop("skin").and_then(|s| s.trim().parse().ok()).unwrap_or(0),
                origin: vbsp::Vector { x, y, z },
                angles: vbsp::Angles { pitch, yaw, roll },
                solid,
                lighting_origin: None,
                class: Some(class.to_string()),
            })
        })
        .collect()
}

fn place_props(
    bsp: &Bsp,
    materials: &mut MaterialLoader,
    lighting: &MapLighting,
    occluders: &Occluders,
    data: &mut MapData,
    placements: Vec<PropPlacement>,
) {
    let mut loaded: HashMap<(String, i32), Option<usize>> = HashMap::new();
    let mut failed: Vec<String> = Vec::new();
    let bounds = super::bsp::playable_bounds(bsp);
    for prop in placements {
        let key = (prop.model.clone(), prop.skin);
        let model = *loaded
            .entry(key)
            .or_insert_with(|| match load_model(materials, &prop.model) {
                Ok(m) => {
                    let mut model = convert_model(&m, prop.skin, materials);
                    model.collision = load_collision(materials, &prop.model);
                    data.models.push(model);
                    Some(data.models.len() - 1)
                }
                Err(e) => {
                    failed.push(e);
                    None
                }
            });
        let Some(model) = model else { continue };
        // Physics props need a collision model (spec section 2.3): without
        // one, prop_physics stays visible but not solid, and the
        // multiplayer variant is removed at spawn.
        let mut solid = prop.solid;
        if data.models[model].collision.is_none() {
            match prop.class.as_deref() {
                Some("prop_physics_multiplayer") => continue,
                Some(c) if c.starts_with("prop_physics") => solid = PropSolid::None,
                _ => {}
            }
        }
        let translation = to_engine(prop.origin);
        let rotation = rotation(prop.angles);
        // Like the game for maps without baked prop lighting: one lighting
        // point per prop (its bounds' centre, or the mapper's lighting
        // origin), so a prop is lit or shadowed as a whole.
        let origin = match prop.lighting_origin {
            Some(o) => to_engine(o),
            None => {
                let (lo, hi) = data.models[model].bounds;
                translation + rotation * ((lo + hi) / 2.0)
            }
        };
        let lighting = probe(bsp, lighting, occluders, origin);
        let skybox = bounds.as_ref().is_some_and(|b| !b.contains(prop.origin));
        data.props.push(MapProp {
            model,
            translation,
            rotation,
            skybox,
            lighting: Some(lighting),
            solid,
        });
    }
    failed.sort();
    failed.dedup();
    data.warnings.extend(failed);
}

pub fn probe(bsp: &Bsp, lighting: &MapLighting, occluders: &Occluders, origin: Vec3) -> LightProbe {
    let offsets = [Vec3::ZERO, Vec3::Y, Vec3::X, -Vec3::X, Vec3::Z, -Vec3::Z, -Vec3::Y]
        .into_iter()
        .flat_map(|d| [0.0, 0.25, 0.5, 1.0].map(|k| d * k));
    let (point, cube) = offsets
        .map(|o| origin + o)
        .map(|p| (p, lighting.ambient_at(bsp, p)))
        .find(|(_, c)| c.0.iter().any(|v| v.max_element() > 0.0))
        .unwrap_or((origin, Default::default()));
    let lights = lighting
        .lights
        .iter()
        .filter_map(|l| {
            // Evaluate facing the light; the per-vertex normal applies the
            // cosine at render time.
            let to = match l {
                ambient::WorldLight::Sky { direction, .. } => -*direction,
                ambient::WorldLight::Point { position, .. } | ambient::WorldLight::Spot { position, .. } => {
                    (*position - point).normalize_or_zero()
                }
            };
            let (value, to, dist) = ambient::direct(l, point, to)?;
            let start = point + to * (2.0 * METERS_PER_UNIT);
            (value.max_element() > 0.002 && !occluders.blocked(start, to, dist)).then_some((to, value))
        })
        .collect();
    LightProbe { cube: cube.0, lights }
}
