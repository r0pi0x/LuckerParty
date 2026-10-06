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
use crate::map::{
    LightProbe, MapCollision, MapConvex, MapData, MapMesh, MapModel, MapPhysics, MapProp, PropSolid, PushAway,
};

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

/// A model and its `prop_data` key values (None: the model has none).
fn load_model(
    materials: &mut MaterialLoader,
    path: &str,
) -> Result<(vmdl::Model, Option<HashMap<String, String>>), String> {
    let read = |p: String| materials.read(&p).ok_or_else(|| format!("{p}: not found"));
    let mdl = vmdl::mdl::Mdl::read(&read(path.to_string())?).map_err(|e| format!("{path}: {e}"))?;
    let prop_data = mdl.key_values.as_deref().and_then(prop_data);
    let stem = path.trim_end_matches(".mdl");
    let vtx = vmdl::vtx::Vtx::read(&read(format!("{stem}.dx90.vtx"))?).map_err(|e| format!("{stem}.dx90.vtx: {e}"))?;
    let vvd = vmdl::vvd::Vvd::read(&read(format!("{stem}.vvd"))?).map_err(|e| format!("{stem}.vvd: {e}"))?;
    Ok((vmdl::Model::from_parts(mdl, vtx, vvd), prop_data))
}

/// A player model as a character body (specs/cs_source/weapons.md 5): its
/// meshes and first hitbox set in the reference pose, in the character's
/// local space (feet at the origin, facing -Z, meters).
pub fn load_character(
    materials: &mut MaterialLoader,
    path: &str,
    team: Option<crate::core::Team>,
) -> Result<crate::map::MapCharacterModel, String> {
    use crate::core::{Hitbox, Hitgroup};
    let (model, _) = load_model(materials, path)?;
    let bytes = materials.read(path).ok_or_else(|| format!("{path}: not found"))?;
    let mdl = vmdl::mdl::Mdl::read(&bytes).map_err(|e| format!("{path}: {e}"))?;
    // Source models face +X; characters face -Z at yaw 0.
    let face = Quat::from_rotation_y(std::f32::consts::FRAC_PI_2);
    let mut body = convert_model_in(&model, 0, materials, false);
    for mesh in &mut body.meshes {
        for p in &mut mesh.positions {
            *p = (face * Vec3::from(*p)).to_array();
        }
        for n in &mut mesh.normals {
            *n = (face * Vec3::from(*n)).to_array();
        }
    }
    // Bones' reference pose in model space (Source units, Z up).
    let mut pose: Vec<(Quat, Vec3)> = Vec::with_capacity(mdl.bones.len());
    for b in &mdl.bones {
        let q = Quat::from_xyzw(b.quaternion.x, b.quaternion.y, b.quaternion.z, b.quaternion.w);
        let p = Vec3::new(b.pos.x, b.pos.y, b.pos.z);
        pose.push(match pose.get(b.parent.max(0) as usize).filter(|_| b.parent >= 0) {
            Some((pq, pp)) => (*pq * q, *pp + *pq * p),
            None => (q, p),
        });
    }
    // Source axes to ours (x, z, -y): -90 degrees about X.
    let axes = Quat::from_rotation_x(-std::f32::consts::FRAC_PI_2);
    let set = mdl.hit_boxes.first().ok_or_else(|| format!("{path}: no hitbox set"))?;
    let hitboxes: Vec<Hitbox> = set
        .boxes
        .iter()
        .filter_map(|b| {
            let (q, p) = *pose.get(b.bone.max(0) as usize)?;
            let (lo, hi) = (
                Vec3::new(b.min.x, b.min.y, b.min.z),
                Vec3::new(b.max.x, b.max.y, b.max.z),
            );
            let centre = p + q * ((lo + hi) / 2.0);
            Some(Hitbox {
                center: face * axes * centre * METERS_PER_UNIT,
                half: (hi - lo) / 2.0 * METERS_PER_UNIT,
                rotation: face * axes * q,
                group: match b.group {
                    1 => Hitgroup::Head,
                    2 => Hitgroup::Chest,
                    3 => Hitgroup::Stomach,
                    4 => Hitgroup::LeftArm,
                    5 => Hitgroup::RightArm,
                    6 => Hitgroup::LeftLeg,
                    7 => Hitgroup::RightLeg,
                    _ => Hitgroup::Generic,
                },
            })
        })
        .collect();
    let boxes = set
        .boxes
        .iter()
        .zip(&hitboxes)
        .filter(|(b, _)| b.bone >= 0 && (b.bone as usize) < mdl.bones.len())
        .map(|(b, h)| {
            let (lo, hi) = (
                Vec3::new(b.min.x, b.min.y, b.min.z),
                Vec3::new(b.max.x, b.max.y, b.max.z),
            );
            crate::map::BoneBox {
                bone: b.bone as usize,
                center: (lo + hi) / 2.0,
                half: (hi - lo) / 2.0,
                group: h.group,
            }
        })
        .collect();
    let bones = mdl
        .bones
        .iter()
        .map(|b| crate::map::MapBone {
            name: b.name.clone(),
            parent: (b.parent >= 0).then_some(b.parent as usize),
            position: Vec3::new(b.pos.x, b.pos.y, b.pos.z),
            rotation: Quat::from_xyzw(b.quaternion.x, b.quaternion.y, b.quaternion.z, b.quaternion.w),
        })
        .collect();
    let animations = match super::anim::load(&|p| materials.read(p), path) {
        Ok(set) => Some(std::sync::Arc::new(set)),
        Err(e) => {
            warn!("{path}: no animations: {e}");
            None
        }
    };
    Ok(crate::map::MapCharacterModel {
        team,
        animations,
        boxes,
        model: body,
        hitboxes,
        bones,
        root: Transform::from_rotation(face * axes).with_scale(Vec3::splat(METERS_PER_UNIT)),
    })
}

/// A weapon's first-person view model (spec weapons.md 3.3: the script's
/// `viewmodel`): its meshes skinned to its own skeleton (hands and
/// weapon), in the eye's space (Source view models have their origin at
/// the eye, facing +X), and its sequences.
pub fn load_view_model(
    materials: &mut MaterialLoader,
    path: &str,
    key: &str,
    right_handed: bool,
) -> Result<crate::map::MapViewModel, String> {
    let (model, _) = load_model(materials, path)?;
    // Source models face +X; the eye looks along -Z.
    let face = Quat::from_rotation_y(std::f32::consts::FRAC_PI_2);
    let mut view = convert_model_in(&model, 0, materials, false);
    for mesh in &mut view.meshes {
        for p in &mut mesh.positions {
            *p = (face * Vec3::from(*p)).to_array();
        }
        for n in &mut mesh.normals {
            *n = (face * Vec3::from(*n)).to_array();
        }
    }
    let read = |p: &str| materials.read(p);
    let bones = super::anim::bones(&read, path)
        .map_err(|e| format!("{path}: {e}"))?
        .into_iter()
        .map(|(name, parent, rotation, position)| crate::map::MapBone {
            name,
            parent,
            position,
            rotation,
        })
        .collect();
    let animations = super::anim::load(&read, path)
        .map(std::sync::Arc::new)
        .map_err(|e| format!("{path}: {e}"))?;
    let (attachments, light_origin) = super::anim::attachments(&read, path)?;
    // Source axes to ours (x, z, -y): -90 degrees about X.
    let axes = Quat::from_rotation_x(-std::f32::consts::FRAC_PI_2);
    Ok(crate::map::MapViewModel {
        key: key.to_string(),
        model: view,
        bones,
        root: Transform::from_rotation(face * axes).with_scale(Vec3::splat(METERS_PER_UNIT)),
        animations: Some(animations),
        right_handed,
        allow_flipping: true,
        attachments: attachments
            .into_iter()
            .map(|(name, bone, local)| crate::map::MapAttachment { name, bone, local })
            .collect(),
        light_origin,
    })
}

/// A shell model (spec view_models.md 8), in its own space (engine axes,
/// meters).
pub fn load_shell(materials: &mut MaterialLoader, path: &str) -> Result<crate::map::MapModel, String> {
    let (model, _) = load_model(materials, path)?;
    Ok(convert_model(&model, 0, materials))
}

/// A weapon's world model held by characters: its meshes in the frame of
/// its first bone that player skeletons also have (bone merge: that bone
/// follows the hand), in the skeleton's axes and units.
pub fn load_held(
    materials: &mut MaterialLoader,
    path: &str,
    key: &str,
    skeleton: &[crate::map::MapBone],
) -> Result<crate::map::MapHeldModel, String> {
    let (model, _) = load_model(materials, path)?;
    let bytes = materials.read(path).ok_or_else(|| format!("{path}: not found"))?;
    let mdl = vmdl::mdl::Mdl::read(&bytes).map_err(|e| format!("{path}: {e}"))?;
    let mut global: Vec<(Quat, Vec3)> = Vec::with_capacity(mdl.bones.len());
    for b in &mdl.bones {
        let q = Quat::from_xyzw(b.quaternion.x, b.quaternion.y, b.quaternion.z, b.quaternion.w);
        let p = Vec3::new(b.pos.x, b.pos.y, b.pos.z);
        global.push(match global.get(b.parent.max(0) as usize).filter(|_| b.parent >= 0) {
            Some((pq, pp)) => (*pq * q, *pp + *pq * p),
            None => (q, p),
        });
    }
    let (i, bone) = mdl
        .bones
        .iter()
        .enumerate()
        .find(|(_, b)| skeleton.iter().any(|s| s.name.eq_ignore_ascii_case(&b.name)))
        .ok_or_else(|| format!("{path}: no bone in common with the player skeleton"))?;
    let (q, p) = global[i];
    let to_bone = Transform::from_rotation(q).with_translation(p).to_matrix().inverse();
    // Engine space (meters, x z -y) back to the model's units and axes.
    let source = |v: [f32; 3]| Vec3::new(v[0], -v[2], v[1]);
    let mut held = convert_model_in(&model, 0, materials, false);
    for mesh in &mut held.meshes {
        for v in &mut mesh.positions {
            *v = to_bone.transform_point3(source(*v) / METERS_PER_UNIT).to_array();
        }
        for n in &mut mesh.normals {
            *n = to_bone.transform_vector3(source(*n)).normalize_or_zero().to_array();
        }
    }
    // The muzzle attachment (spec view_models.md 6: `muzzle_flash`), in the
    // same frame.
    let muzzle = super::anim::attachments(&|p| materials.read(p), path)
        .ok()
        .and_then(|(list, _)| {
            let (_, b, local) = list
                .iter()
                .find(|(n, _, _)| n.eq_ignore_ascii_case("muzzle_flash"))
                .or_else(|| list.first())?
                .clone();
            let (bq, bp) = *global.get(b)?;
            let m = to_bone * Mat4::from_rotation_translation(bq, bp) * local.to_matrix();
            Some(Transform::from_matrix(m))
        });
    Ok(crate::map::MapHeldModel {
        key: key.to_string(),
        model: held,
        bone: bone.name.clone(),
        muzzle,
    })
}

/// The `prop_data` block of a model's key values (spec 2.2), lower-case
/// keys.
fn prop_data(text: &str) -> Option<HashMap<String, String>> {
    let t = super::surfaceprops::tokens(text);
    let start = t
        .windows(2)
        .position(|w| w[0].eq_ignore_ascii_case("prop_data") && w[1] == "{")?;
    let mut out = HashMap::new();
    let mut i = start + 2;
    while i + 1 < t.len() && t[i] != "}" {
        if t[i + 1] == "{" {
            // A nested block: skip it.
            while i < t.len() && t[i] != "}" {
                i += 1;
            }
            i += 1;
            continue;
        }
        out.insert(t[i].to_lowercase(), t[i + 1].clone());
        i += 2;
    }
    Some(out)
}

fn v(p: vmdl::Vector) -> vbsp::Vector {
    vbsp::Vector { x: p.x, y: p.y, z: p.z }
}

/// Convert one model with one skin into engine-space meshes (meters).
fn convert_model(model: &vmdl::Model, skin: i32, materials: &mut MaterialLoader) -> MapModel {
    convert_model_in(model, skin, materials, true)
}

/// `convert_model`, optionally without vmdl's root/idle transform (player
/// models' vertices are already in their reference pose's model space).
fn convert_model_in(model: &vmdl::Model, skin: i32, materials: &mut MaterialLoader, root: bool) -> MapModel {
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
                envmap: r.envmap,
                tint: r.tint,
                ..default()
            }
        });
        let verts: Vec<&vmdl::vvd::Vertex> = mesh.vertices().collect();
        for tri in verts.as_chunks::<3>().0 {
            let p: Vec<Vec3> = tri
                .iter()
                .map(|t| {
                    to_engine(v(if root {
                        model.apply_root_transform(t.position)
                    } else {
                        t.position
                    }))
                })
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
                if !root {
                    // Skinning weights, renormalized to sum to 1.
                    let (mut joints, mut weights) = ([0u16; 4], [0f32; 4]);
                    for (k, w) in tri[i].bone_weights.weights().take(3).enumerate() {
                        joints[k] = w.bone_id as u16;
                        weights[k] = w.weight;
                    }
                    let sum: f32 = weights.iter().sum();
                    if sum > 0.0 {
                        weights.iter_mut().for_each(|w| *w /= sum);
                    } else {
                        weights[0] = 1.0;
                    }
                    entry.joints.push(joints);
                    entry.joint_weights.push(weights);
                }
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
        surfaceprop: None,
        illum: None,
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

/// A physics prop's body (spec 3.1, 4.2): mass from its `.phy` (times
/// `massscale`), friction and elasticity from its surface property, and how
/// players interact with it. None for props that don't move.
fn body(
    prop: &PropPlacement,
    model: &MapModel,
    prop_data: Option<&HashMap<String, String>>,
    surfaces: &super::surfaceprops::SurfaceProps,
) -> Option<MapPhysics> {
    const MOTION_DISABLED: u32 = 0x8;
    const FORCE_SERVER_SIDE: u32 = 0x2000;
    let class = prop.class.as_deref()?;
    if !class.starts_with("prop_physics") || prop.spawnflags & MOTION_DISABLED != 0 {
        return None;
    }
    let c = model.collision.as_ref()?;
    let mass = (c.mass * if prop.massscale > 0.0 { prop.massscale } else { 1.0 }).clamp(0.1, 50_000.0);
    let surface = surfaces.get(&c.surfaceprop);
    let push = if class == "prop_physics_multiplayer" {
        let size = (model.bounds.1 - model.bounds.0) / METERS_PER_UNIT;
        let auto = if size.x * size.y * size.z < 15.0f32.powi(3) {
            PushAway::Ignore
        } else if mass < 8.0 {
            PushAway::NonSolid
        } else {
            PushAway::Solid
        };
        // The model's prop_data physicsmode wins over the map's.
        let physicsmode = prop_data
            .and_then(|d| d.get("physicsmode"))
            .and_then(|v| v.trim().parse().ok())
            .unwrap_or(prop.physicsmode);
        let mode = match physicsmode {
            1 => PushAway::Solid,
            2 => PushAway::NonSolid,
            3 => PushAway::Ignore,
            _ => auto,
        };
        if prop.spawnflags & FORCE_SERVER_SIDE != 0 {
            PushAway::NonSolid
        } else {
            mode
        }
    } else {
        PushAway::Collide
    };
    Some(MapPhysics {
        mass,
        friction: surface.friction,
        elasticity: surface.elasticity,
        damping: c.damping,
        rotdamping: c.rotdamping,
        push,
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
    /// Entity keys the physics uses: spawnflags, massscale, physicsmode.
    spawnflags: u32,
    massscale: f32,
    physicsmode: i32,
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
            spawnflags: 0,
            massscale: 0.0,
            physicsmode: 0,
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
                spawnflags: e.prop("spawnflags").and_then(|v| v.trim().parse().ok()).unwrap_or(0),
                massscale: e.prop("massscale").and_then(|v| v.trim().parse().ok()).unwrap_or(0.0),
                physicsmode: e.prop("physicsmode").and_then(|v| v.trim().parse().ok()).unwrap_or(0),
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
    let mut prop_datas: HashMap<usize, HashMap<String, String>> = HashMap::new();
    let mut failed: Vec<String> = Vec::new();
    let bounds = super::bsp::playable_bounds(bsp);
    let surfaces = super::surfaceprops::SurfaceProps::load(materials);
    for prop in placements {
        let key = (prop.model.clone(), prop.skin);
        let model = *loaded
            .entry(key)
            .or_insert_with(|| match load_model(materials, &prop.model) {
                Ok((m, pd)) => {
                    let mut model = convert_model(&m, prop.skin, materials);
                    model.collision = load_collision(materials, &prop.model);
                    // What traces against the prop report: its collision
                    // model's surface, else the model's own $surfaceprop.
                    model.surfaceprop = model
                        .collision
                        .as_ref()
                        .map(|c| c.surfaceprop.as_str())
                        .filter(|s| !s.is_empty())
                        .or(Some(m.surface_prop()).filter(|s| !s.is_empty()))
                        .map(str::to_lowercase);
                    // The model's lighting position (its header's
                    // illumination position, in the same space as its
                    // vertices).
                    model.illum = materials
                        .read(&prop.model)
                        .and_then(|b| vmdl::mdl::Mdl::read(&b).ok())
                        .map(|mdl| to_engine(v(m.apply_root_transform(mdl.header.illumination_position))));
                    data.models.push(model);
                    if let Some(pd) = pd {
                        prop_datas.insert(data.models.len() - 1, pd);
                    }
                    Some(data.models.len() - 1)
                }
                Err(e) => {
                    failed.push(e);
                    None
                }
            });
        let Some(model) = model else { continue };
        // A plain prop_physics whose model has no prop_data is deleted at
        // spawn (override and multiplayer variants are exempt).
        if prop.class.as_deref() == Some("prop_physics") && !prop_datas.contains_key(&model) {
            continue;
        }
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
        let physics = body(&prop, &data.models[model], prop_datas.get(&model), &surfaces);
        let translation = to_engine(prop.origin);
        let rotation = rotation(prop.angles);
        // Like the game for maps without baked prop lighting: one lighting
        // point per prop (the mapper's lighting origin, else the model's
        // illumination position, else its bounds' centre), so a prop is lit
        // or shadowed as a whole. (The collision bounds' centre can sit
        // under the ground: de_nuke's dumpsters lost the sun.)
        let origin = match (prop.lighting_origin, data.models[model].illum) {
            (Some(o), _) => to_engine(o),
            (None, Some(illum)) => translation + rotation * illum,
            (None, None) => {
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
            casts_shadow: prop.class.is_some(),
            physics,
        });
    }
    failed.sort();
    failed.dedup();
    data.warnings.extend(failed);
}

pub fn probe(bsp: &Bsp, lighting: &MapLighting, occluders: &Occluders, origin: Vec3) -> LightProbe {
    probe_with(&|p| lighting.ambient_at(bsp, p), lighting, occluders, origin)
}

/// The light at `origin` (the game's light-at-a-point query, as props
/// bake it): the ambient cube from `ambient`, nudged out of solid, plus
/// the world lights that reach it.
pub fn probe_with(
    ambient: &dyn Fn(Vec3) -> ambient::AmbientCube,
    lighting: &MapLighting,
    occluders: &Occluders,
    origin: Vec3,
) -> LightProbe {
    let offsets = [Vec3::ZERO, Vec3::Y, Vec3::X, -Vec3::X, Vec3::Z, -Vec3::Z, -Vec3::Y]
        .into_iter()
        .flat_map(|d| [0.0, 0.25, 0.5, 1.0].map(|k| d * k));
    let (point, cube) = offsets
        .map(|o| origin + o)
        .map(|p| (p, ambient(p)))
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

#[cfg(test)]
mod prop_data_tests {
    use super::prop_data;

    #[test]
    fn reads_the_prop_data_block() {
        let text = "\"prop_data\" { \"base\" \"Wooden.Medium\" \"physicsmode\" \"1\" \"breakable_model\" { \"x\" \"y\" } \"health\" \"20\" } \"physgun_interactions\" { \"onfirstimpact\" \"break\" }";
        let d = prop_data(text).unwrap();
        assert_eq!(d.get("base").map(String::as_str), Some("Wooden.Medium"));
        assert_eq!(d.get("physicsmode").map(String::as_str), Some("1"));
        assert_eq!(d.get("health").map(String::as_str), Some("20"));
        assert!(prop_data("\"physgun_interactions\" { }").is_none());
    }
}
