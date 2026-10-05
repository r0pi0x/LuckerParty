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
use crate::map::{LightProbe, MapData, MapMesh, MapModel, MapProp, PropSolid};

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
    }
}

/// Add the BSP's static props to `data`, loading each (model, skin) once.
pub fn add_static_props(
    bsp: &Bsp,
    materials: &mut MaterialLoader,
    lighting: &MapLighting,
    occluders: &Occluders,
    data: &mut MapData,
) {
    let mut loaded: HashMap<(String, i32), Option<usize>> = HashMap::new();
    let mut failed: Vec<String> = Vec::new();
    for prop in bsp.static_props() {
        if prop.flags.contains(vbsp::StaticPropLumpFlags::NO_DRAW) {
            continue;
        }
        let path = prop.model().to_lowercase();
        let key = (path.clone(), prop.skin);
        let model = *loaded.entry(key).or_insert_with(|| match load_model(materials, &path) {
            Ok(m) => {
                data.models.push(convert_model(&m, prop.skin, materials));
                Some(data.models.len() - 1)
            }
            Err(e) => {
                failed.push(e);
                None
            }
        });
        let Some(model) = model else { continue };
        let translation = to_engine(prop.origin);
        let rotation = rotation(prop.angles);
        // Like the game for maps without baked prop lighting: one lighting
        // point per prop (its bounds' centre, or the mapper's lighting
        // origin), so a prop is lit or shadowed as a whole.
        let origin = if prop.flags.contains(vbsp::StaticPropLumpFlags::USE_LIGHTING_ORIGIN) {
            to_engine(prop.lighting_origin)
        } else {
            let (lo, hi) = data.models[model].bounds;
            translation + rotation * ((lo + hi) / 2.0)
        };
        let lighting = probe(bsp, lighting, occluders, origin);
        data.props.push(MapProp {
            model,
            translation,
            rotation,
            lighting: Some(lighting),
            // Box solids collide as the model's bounds, like the game;
            // physics solids should use the .phy model, which isn't parsed
            // yet, so they fall back to the visible mesh.
            solid: match prop.solid {
                vbsp::SolidType::None => PropSolid::None,
                vbsp::SolidType::Bbox | vbsp::SolidType::Obb | vbsp::SolidType::ObbYaw => PropSolid::Box,
                _ => PropSolid::Mesh,
            },
        });
    }
    failed.sort();
    failed.dedup();
    data.warnings.extend(failed);
}

/// The light probe at `origin`: ambient cube plus each light visible from
/// there. If the point is buried in solid (no ambient samples), try nearby
/// points: props such as window frames sit inside walls.
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
