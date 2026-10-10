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
    LightProbe, MapBone, MapCollision, MapConvex, MapData, MapMesh, MapMeshLook, MapModel, MapPhysics, MapProp, MapRig,
    PropSolid, PushAway,
    entities::{DOOR_CLOSE_KEY, DOOR_LOCKED_KEY, DOOR_MOVE_KEY, DOOR_OPEN_KEY, DOOR_UNLOCKED_KEY},
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

/// Offset of `numlocalanim` in the MDL header (public studiohdr_t layout).
const MDL_NUM_LOCAL_ANIM: usize = 180;

/// An MDL header and everything but its animations' data (our own reader,
/// `anim`, reads those). `vmdl` panics on animations stored in external
/// blocks (`.ani` files, common in community models), so a model it can't
/// read is read again with its local animations left out: the meshes,
/// bones, textures and hitboxes don't need them.
pub fn read_mdl(bytes: &[u8]) -> Result<vmdl::mdl::Mdl, String> {
    let read = |b: &[u8]| {
        std::panic::catch_unwind(|| vmdl::mdl::Mdl::read(b))
            .map_err(|_| "unreadable animations".to_string())
            .and_then(|r| r.map_err(|e| e.to_string()))
    };
    // Models with animation blocks go straight to the second read (no
    // panic to catch).
    let blocks = bytes.get(352..356).map_or(0, |b| i32::from_le_bytes(b.try_into().unwrap()));
    if blocks <= 0
        && let Ok(m) = read(bytes)
    {
        return Ok(m);
    }
    let mut copy = bytes.to_vec();
    if let Some(n) = copy.get_mut(MDL_NUM_LOCAL_ANIM..MDL_NUM_LOCAL_ANIM + 4) {
        n.copy_from_slice(&0i32.to_le_bytes());
    }
    read(&copy)
}

/// A model and its key values text (`$keyvalues`: `prop_data`,
/// `door_options`...; None: the model has none).
fn load_model(materials: &mut MaterialLoader, path: &str) -> Result<(vmdl::Model, Option<String>), String> {
    load_model_with_layout(materials, path).map(|(m, kv, _)| (m, kv))
}

/// `load_model`, with the model's body layout (`body_layout`).
#[allow(clippy::type_complexity)]
fn load_model_with_layout(
    materials: &mut MaterialLoader,
    path: &str,
) -> Result<(vmdl::Model, Option<String>, Vec<Vec<usize>>), String> {
    let read = |p: String| materials.read(&p).ok_or_else(|| format!("{p}: not found"));
    let mdl = read_mdl(&read(path.to_string())?).map_err(|e| format!("{path}: {e}"))?;
    let key_values = mdl.key_values.clone();
    let layout = body_layout(&mdl);
    let stem = path.trim_end_matches(".mdl");
    let vtx = vmdl::vtx::Vtx::read(&read(format!("{stem}.dx90.vtx"))?).map_err(|e| format!("{stem}.dx90.vtx: {e}"))?;
    let vvd = vmdl::vvd::Vvd::read(&read(format!("{stem}.vvd"))?).map_err(|e| format!("{stem}.vvd: {e}"))?;
    let model = vmdl::Model::from_parts(mdl, vtx, vvd);
    // Meshes whose strip indices run past their vertices (a `.vtx` that
    // doesn't match its `.vvd`): vmdl would panic converting them.
    let whole = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        model.meshes().map(|m| m.vertices().count() + m.tangents().count()).sum::<usize>()
    }));
    if whole.is_err() {
        return Err(format!("{path}: mesh indices outside its vertices (.vtx and .vvd disagree)"));
    }
    Ok((model, key_values, layout))
}

/// A player model as a character body (specs/cs_source/weapons.md 5): its
/// meshes and first hitbox set in the reference pose, in the character's
/// local space (feet at the origin, facing -Z, meters).
pub fn load_character(
    materials: &mut MaterialLoader,
    surfaces: &super::surfaceprops::SurfaceProps,
    path: &str,
    team: Option<crate::core::Team>,
) -> Result<crate::map::MapCharacterModel, String> {
    use crate::core::{Hitbox, Hitgroup};
    let (model, _) = load_model(materials, path)?;
    let bytes = materials.read(path).ok_or_else(|| format!("{path}: not found"))?;
    let mdl = read_mdl(&bytes).map_err(|e| format!("{path}: {e}"))?;
    // Source models face +X; characters face -Z at yaw 0.
    let face = Quat::from_rotation_y(std::f32::consts::FRAC_PI_2);
    let mut body = convert_model_in(&model, 0, materials, false, &[]);
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
    let bones: Vec<crate::map::MapBone> = mdl
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
    let ragdoll = match materials.read(&format!("{}.phy", path.trim_end_matches(".mdl"))) {
        Some(bytes) => match super::phy::parse_ragdoll(&bytes) {
            Ok(phy) => ragdoll(&phy, &bones, surfaces, path),
            Err(e) => {
                warn!("{path}: ragdoll: {e}");
                None
            }
        },
        None => None,
    };
    Ok(crate::map::MapCharacterModel {
        team,
        name: None,
        ragdoll,
        animations,
        boxes,
        model: body,
        hitboxes,
        bones,
        root: Transform::from_rotation(face * axes).with_scale(Vec3::splat(METERS_PER_UNIT)),
    })
}

/// Most solids a ragdoll may have (specs/cs_source/ragdolls.md, Constants).
const RAGDOLL_MAX_BODIES: usize = 24;

/// A player model's `.phy` as its ragdoll (specs/cs_source/ragdolls.md 1):
/// a body per solid whose bone the skeleton has (the pieces are already in
/// that bone's frame, inches), joints between kept bodies with their
/// limits in radians. None for a model with too many solids or no joints.
pub fn ragdoll(
    phy: &super::phy::PhyRagdoll,
    bones: &[crate::map::MapBone],
    surfaces: &super::surfaceprops::SurfaceProps,
    path: &str,
) -> Option<crate::map::MapRagdoll> {
    use crate::map::{MapRagdoll, MapRagdollBody, MapRagdollJoint};
    if phy.solids.len() > RAGDOLL_MAX_BODIES || phy.solids.is_empty() {
        return None;
    }
    // Solid index → body index (solids naming a missing bone are dropped).
    let mut body_of = vec![None; phy.solids.iter().map(|s| s.index + 1).max().unwrap_or(0)];
    let mut bodies = Vec::new();
    for s in &phy.solids {
        let Some(bone) = bones.iter().position(|b| b.name.eq_ignore_ascii_case(&s.name)) else {
            warn!("{path}: ragdoll solid {} names a missing bone {:?}", s.index, s.name);
            continue;
        };
        body_of[s.index] = Some(bodies.len());
        bodies.push(MapRagdollBody {
            bone,
            pieces: s
                .pieces
                .iter()
                .map(|tris| {
                    let mut points: Vec<Vec3> = Vec::new();
                    for v in tris.iter().flatten() {
                        if !points.iter().any(|p| p.distance_squared(*v) < 1e-8) {
                            points.push(*v);
                        }
                    }
                    points
                })
                .collect(),
            mass: s.mass,
            damping: s.damping,
            rotdamping: s.rotdamping,
            inertia: s.inertia,
            surfaceprop: s.surfaceprop.clone(),
            friction: surfaces.get(&s.surfaceprop).friction,
            elasticity: surfaces.get(&s.surfaceprop).elasticity,
        });
    }
    let body = |i: usize| body_of.get(i).copied().flatten();
    let joints: Vec<MapRagdollJoint> = phy
        .joints
        .iter()
        .filter_map(|j| {
            Some(MapRagdollJoint {
                parent: body(j.parent)?,
                child: body(j.child)?,
                limits: j.limits.map(|(lo, hi)| (lo.to_radians(), hi.to_radians())),
            })
        })
        .collect();
    if joints.is_empty() {
        return None;
    }
    // Spec 1.3: with a rules block only its pairs collide (none after
    // `selfcollisions 0`); without one the default rule applies.
    let collision_pairs = phy.has_collision_rules.then(|| {
        phy.collision_pairs
            .iter()
            .filter_map(|(a, b)| Some((body(*a)?, body(*b)?)))
            .collect()
    });
    Some(MapRagdoll {
        bodies,
        joints,
        collision_pairs,
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
    let mut view = convert_model_in(&model, 0, materials, false, &[]);
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
    Ok(convert_model(&model, 0, materials, &[]))
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
    let mdl = read_mdl(&bytes).map_err(|e| format!("{path}: {e}"))?;
    let mut global: Vec<(Quat, Vec3)> = Vec::with_capacity(mdl.bones.len());
    for b in &mdl.bones {
        let q = Quat::from_xyzw(b.quaternion.x, b.quaternion.y, b.quaternion.z, b.quaternion.w);
        let p = Vec3::new(b.pos.x, b.pos.y, b.pos.z);
        global.push(match global.get(b.parent.max(0) as usize).filter(|_| b.parent >= 0) {
            Some((pq, pp)) => (*pq * q, *pp + *pq * p),
            None => (q, p),
        });
    }
    // Held by a bone it shares with the player skeleton; items that are
    // never held (a planted bomb, a kit on the floor) stay in model space.
    let shared = mdl
        .bones
        .iter()
        .enumerate()
        .find(|(_, b)| skeleton.iter().any(|s| s.name.eq_ignore_ascii_case(&b.name)));
    let (i, bone) = match shared {
        Some(b) => b,
        None => (0, mdl.bones.first().ok_or_else(|| format!("{path}: no bones"))?),
    };
    let to_bone = match global.get(i) {
        Some((q, p)) => Transform::from_rotation(*q).with_translation(*p).to_matrix().inverse(),
        None => Mat4::IDENTITY,
    };
    // Engine space (meters, x z -y) back to the model's units and axes.
    let source = |v: [f32; 3]| Vec3::new(v[0], -v[2], v[1]);
    let mut held = convert_model_in(&model, 0, materials, false, &[]);
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
/// `layout`: mesh counts per choice per body part (`body_layout`).
fn convert_model(model: &vmdl::Model, skin: i32, materials: &mut MaterialLoader, layout: &[Vec<usize>]) -> MapModel {
    convert_model_in(model, skin, materials, true, layout)
}

/// Mesh counts per choice of each body part, in the order vmdl lists the
/// meshes (part by part, choice by choice).
fn body_layout(mdl: &vmdl::mdl::Mdl) -> Vec<Vec<usize>> {
    mdl.body_parts
        .iter()
        .map(|p| p.models.iter().map(|m| m.meshes.len()).collect())
        .collect()
}

/// How a model material looks: resolved through the model's texture
/// directories.
fn mesh_look(materials: &mut MaterialLoader, dirs: &[String], name: &str) -> MapMeshLook {
    let candidates: Vec<String> = dirs.iter().map(|d| format!("{d}{name}")).collect();
    let r = materials.resolve_any(&candidates);
    MapMeshLook {
        material: name.to_lowercase(),
        texture: r.texture,
        alpha: r.alpha,
        double_sided: r.double_sided,
        unlit: r.unlit,
        envmap: r.envmap,
        tint: r.tint,
        detail: r.detail,
        base_transform: r.base_transform,
        selfillum: r.selfillum,
    }
}

/// `convert_model`, optionally without vmdl's root/idle transform (player
/// models' vertices are already in their reference pose's model space).
/// With a body `layout`, meshes of a body part that has several choices
/// say which (`MapMesh::body`); without one every mesh is drawn. A model
/// with several skin families keeps each one's look (`MapModel::skins`).
fn convert_model_in(
    model: &vmdl::Model,
    skin: i32,
    materials: &mut MaterialLoader,
    root: bool,
    layout: &[Vec<usize>],
) -> MapModel {
    let skins: Vec<_> = model.skin_tables().collect();
    let table = skins.get(skin.max(0) as usize).or(skins.first());
    let several = skins.len() > 1;
    let dirs = model.texture_directories().to_vec();
    // Each mesh's (part, choice), where its part has several choices.
    let tags: Vec<Option<(u16, u16)>> = layout
        .iter()
        .enumerate()
        .flat_map(|(p, choices)| {
            let several = choices.len() > 1;
            choices
                .iter()
                .enumerate()
                .flat_map(move |(c, n)| std::iter::repeat_n(several.then_some((p as u16, c as u16)), *n))
        })
        .collect();
    let body_parts: Vec<u16> = layout.iter().map(|c| c.len().max(1) as u16).collect();
    // Meshes merge by material (by material slot when skins differ, so
    // each can take its own skin), per body choice.
    let mut by_material: HashMap<(Option<(u16, u16)>, String), (MapMesh, i32)> = HashMap::new();
    for (k, mesh) in model.meshes().enumerate() {
        let slot = mesh.material_index();
        let Some(name) = table.and_then(|t| t.texture(slot)) else {
            continue;
        };
        let body = tags.get(k).copied().flatten();
        let key = (
            body,
            if several {
                format!("#{slot}")
            } else {
                name.to_lowercase()
            },
        );
        let (entry, _) = by_material.entry(key).or_insert_with(|| {
            let look = mesh_look(materials, &dirs, name);
            let m = MapMesh {
                color: [200, 200, 200],
                body,
                ..default()
            };
            (look.apply(&m), slot)
        });
        let verts: Vec<&vmdl::vvd::Vertex> = mesh.vertices().collect();
        // Each vertex's index in the model's vertex list (baked per-vertex
        // light is stored in that order).
        let ids: Vec<u32> = mesh.vertex_strip_indices().flatten().map(|i| i as u32).collect();
        let place = |t: &vmdl::vvd::Vertex| {
            to_engine(v(if root {
                model.apply_root_transform(t.position)
            } else {
                t.position
            }))
        };
        let tris = verts.as_chunks::<3>().0;
        let tri_ids = ids.as_chunks::<3>().0;
        // Studio models share one winding: the file's triangle order (front
        // faces counter-clockwise once in engine space), whatever their
        // normals say. Normals don't decide it: the knife view model's
        // blade and handle have normals against the winding on about half
        // their triangles (smoothed thin parts), and a majority vote over
        // the mesh turned it inside out; every other CS:S model already
        // agreed with the file. `tests/it/heavy/map_de_dust2.rs`
        // (`view_models_face_outward`) checks the knives.
        for (tri, tri_id) in tris.iter().zip(tri_ids) {
            let p: Vec<Vec3> = tri.iter().map(|t| place(t)).collect();
            let n: Vec<Vec3> = tri.iter().map(|t| to_engine(v(t.normal)).normalize_or_zero()).collect();
            for i in 0..3 {
                entry.indices.push(entry.positions.len() as u32);
                entry.positions.push(p[i].to_array());
                entry.normals.push(n[i].to_array());
                entry.uvs.push(tri[i].texture_coordinates);
                entry.source_vertices.push(tri_id[i]);
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
    let mut meshes: Vec<(MapMesh, i32)> = by_material
        .into_values()
        .filter(|(m, _)| !m.indices.is_empty())
        .collect();
    meshes.sort_by(|a, b| (&a.0.material, a.0.body, a.1).cmp(&(&b.0.material, b.0.body, b.1)));
    // Every skin family's look per mesh.
    let looks: Vec<Vec<MapMeshLook>> = if several {
        skins
            .iter()
            .map(|t| {
                meshes
                    .iter()
                    .map(|(m, slot)| match t.texture(*slot) {
                        // Skin families nobody may use can name missing
                        // materials: not a load warning.
                        Some(name) => {
                            let known = materials.missing.len();
                            let look = mesh_look(materials, &dirs, name);
                            materials.missing.truncate(known);
                            look
                        }
                        None => MapMeshLook {
                            material: m.material.clone(),
                            texture: m.texture,
                            alpha: m.alpha,
                            double_sided: m.double_sided,
                            unlit: m.unlit,
                            envmap: m.envmap,
                            tint: m.tint,
                            detail: m.detail,
                            base_transform: m.base_transform,
                            selfillum: m.selfillum,
                        },
                    })
                    .collect()
            })
            .collect()
    } else {
        Vec::new()
    };
    // Axis conversion is a rotation, so re-take min/max per engine axis.
    let (a, b) = model.bounding_box();
    let (a, b) = (to_engine(v(a)), to_engine(v(b)));
    MapModel {
        meshes: meshes.into_iter().map(|(m, _)| m).collect(),
        bounds: (a.min(b), a.max(b)),
        collision: None,
        surfaceprop: None,
        illum: None,
        skins: looks,
        body_parts,
        rig: None,
        breaks: None,
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
    physicsmode: Option<i32>,
    surfaces: &super::surfaceprops::SurfaceProps,
) -> Option<MapPhysics> {
    const START_ASLEEP: u32 = 0x1;
    const MOTION_DISABLED: u32 = 0x8;
    const FORCE_SERVER_SIDE: u32 = 0x2000;
    let class = prop.class.as_deref()?;
    if !class.starts_with("prop_physics") {
        return None;
    }
    // Pinned until enabled (physics_props.md 3.1 step 8).
    let frozen = prop.spawnflags & MOTION_DISABLED != 0 || prop.enable_threshold;
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
        // The model's prop data physicsmode (through its templates) wins
        // over the map's.
        let physicsmode = physicsmode.unwrap_or(prop.physicsmode);
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
        frozen,
        asleep: prop.spawnflags & START_ASLEEP != 0,
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
    /// Fade distances (`fademindist`, `fademaxdist`), units; None when the
    /// prop never fades (max 0 or less).
    fade: Option<(f32, f32)>,
    /// The entity it's parented to (`parentname`), by index.
    parent: Option<usize>,
    /// The entity that placed it, by index (entity props).
    entity: Option<usize>,
    /// Its body number (`body` keyvalue).
    body: i32,
    /// Plays sequences (a prop_dynamic with a default animation, or one the
    /// map tells to play one): loaded with its skeleton.
    animated: bool,
    /// Has `forcetoenablemotion` or `damagetoenablemotion`: pinned until
    /// they're reached.
    enable_threshold: bool,
    /// Its index in the static prop lump (static props): names its baked
    /// per-vertex light (`vhv::names`).
    static_index: Option<usize>,
}

pub fn add_static_props(
    bsp: &Bsp,
    materials: &mut MaterialLoader,
    lighting: &MapLighting,
    occluders: &Occluders,
    data: &mut MapData,
    hdr: bool,
) {
    let mut placements = Vec::new();
    for (static_index, prop) in bsp.static_props().enumerate() {
        // The stored NO_DRAW flag (0x4) is not read: the public BSP
        // description marks it "computed at run time based on dx level",
        // and community maps' version-10 prop lumps carry it on props the
        // game draws (surf_boreas: its ramps, rocks and trees; no stock
        // map sets it). Every stock and cached map leaves the dx level
        // range open, so nothing else hides a static prop.
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
            fade: (prop.fade_max_distance > 0.0).then_some((prop.fade_min_distance, prop.fade_max_distance)),
            parent: None,
            entity: None,
            body: 0,
            animated: false,
            enable_threshold: false,
            static_index: Some(static_index),
        });
    }
    placements.extend(entity_props(bsp));
    placements.extend(fish_pool_props(bsp));
    place_props(bsp, materials, lighting, occluders, data, placements, hdr);
}

/// The keyvalue a model attachment of a prop entity is given under
/// (`ATTACHMENT_KEY` + its lower-case name): "x y z", model space, units.
pub const ATTACHMENT_KEY: &str = "$attachment ";

/// Model attachments of the prop entities something may be parented to
/// (named by a parentname or a SetParent output), as keyvalues
/// (`ATTACHMENT_KEY`) for the logic's SetParentAttachment: each
/// attachment's place in the model's reference pose.
pub fn add_attachment_keys(materials: &MaterialLoader, data: &mut MapData) {
    let mut parents: std::collections::HashSet<String> = std::collections::HashSet::new();
    for e in &data.entities {
        for (k, v) in &e.keyvalues {
            if k.eq_ignore_ascii_case("parentname") {
                parents.insert(crate::map::entities::parent_name(v).to_ascii_lowercase());
            } else if k.starts_with("On") || k.starts_with("on") {
                let parts: Vec<&str> = v.split([',', '\u{1b}']).collect();
                if parts.len() >= 3 && parts[1].eq_ignore_ascii_case("SetParent") {
                    parents.insert(parts[2].trim().to_ascii_lowercase());
                }
            }
        }
    }
    let read = |p: &str| materials.read(p);
    let mut cache: std::collections::HashMap<String, Vec<(String, Vec3)>> = std::collections::HashMap::new();
    for e in &mut data.entities {
        let named = e.get("targetname").is_some_and(|n| parents.contains(&n.to_ascii_lowercase()));
        let Some(model) = e.get("model").filter(|m| named && m.to_ascii_lowercase().ends_with(".mdl")) else {
            continue;
        };
        let model = model.to_ascii_lowercase().replace('\\', "/");
        let list = cache
            .entry(model.clone())
            .or_insert_with(|| model_attachments(&read, &model).unwrap_or_default())
            .clone();
        for (name, at) in list {
            e.keyvalues
                .push((format!("{ATTACHMENT_KEY}{name}"), format!("{} {} {}", at.x, at.y, at.z)));
        }
    }
}

/// A model's attachments in its reference pose (Source axes, units).
fn model_attachments(read: super::anim::Read, path: &str) -> Result<Vec<(String, Vec3)>, String> {
    let bones = super::anim::bones(read, path)?;
    let mut global: Vec<(Quat, Vec3)> = Vec::with_capacity(bones.len());
    for (_, parent, rotation, position) in &bones {
        let g = match parent.and_then(|p| global.get(p).copied()) {
            Some((pr, pp)) => (pr * *rotation, pp + pr * *position),
            None => (*rotation, *position),
        };
        global.push(g);
    }
    let (attachments, _) = super::anim::attachments(read, path)?;
    Ok(attachments
        .into_iter()
        .map(|(name, bone, local)| {
            let (r, p) = global.get(bone).copied().unwrap_or((Quat::IDENTITY, Vec3::ZERO));
            (name.to_ascii_lowercase(), p + r * local.translation)
        })
        .collect())
}

/// Props placed as entities: physics props (barrels, baskets; static here
/// until there's physics) and dynamic props. Both collide by their physics
/// model (the visible mesh until `.phy` is parsed).
fn entity_props(bsp: &Bsp) -> Vec<PropPlacement> {
    let parse = |v: &str| -> Option<[f32; 3]> {
        let mut it = v.split_whitespace().filter_map(|p| p.parse::<f32>().ok());
        Some([it.next()?, it.next()?, it.next()?])
    };
    // Names some output tells to play a sequence.
    let animated_names = animation_targets(bsp);
    bsp.entities
        .iter()
        .enumerate()
        .filter_map(|(index, e)| {
            let class = e.prop("classname")?;
            let physics = class.starts_with("prop_physics");
            let door = class == DOOR_CLASS;
            let ragdoll = class == "prop_ragdoll";
            if !physics && !door && !ragdoll && !class.starts_with("prop_dynamic") {
                return None;
            }
            let [x, y, z] = parse(e.prop("origin")?)?;
            let [pitch, yaw, roll] = e.prop("angles").and_then(parse).unwrap_or([0.0; 3]);
            // prop_dynamic: solid 0 = not solid, otherwise the physics model.
            let solid = if ragdoll {
                // Its parts are bodies of their own when its model has a
                // ragdoll (`map::ragdoll::PlacedRagdoll`).
                PropSolid::None
            } else if physics || door || e.prop("solid").is_none_or(|s| s.trim() != "0") {
                PropSolid::Mesh
            } else {
                PropSolid::None
            };
            let dynamic = class.starts_with("prop_dynamic");
            let animated = ragdoll
                || dynamic
                    && (e.prop("DefaultAnim").is_some_and(|a| !a.trim().is_empty())
                        || e.prop("targetname")
                            .is_some_and(|n| animated_names.contains(&n.to_ascii_lowercase())));
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
                fade: {
                    let key = |k: &'static str| e.prop(k).and_then(|v| v.trim().parse::<f32>().ok());
                    key("fademaxdist")
                        .filter(|max| *max > 0.0)
                        .map(|max| (key("fademindist").unwrap_or(0.0), max))
                },
                // A model door rides its own mover node (the logic turns
                // it); other props the entity they're parented to.
                parent: if door {
                    Some(index)
                } else {
                    e.prop("parentname").filter(|p| !p.is_empty()).and_then(|p| {
                        bsp.entities
                            .iter()
                            .position(|o| o.prop("targetname").is_some_and(|n| n.eq_ignore_ascii_case(p)))
                    })
                },
                entity: Some(index),
                body: e.prop("body").and_then(|v| v.trim().parse().ok()).unwrap_or(0),
                animated,
                enable_threshold: ["forcetoenablemotion", "damagetoenablemotion"].iter().any(|k| {
                    e.prop(k)
                        .and_then(|v| v.trim().parse::<f32>().ok())
                        .is_some_and(|v| v > 0.0)
                }),
                static_index: None,
            })
        })
        .collect()
}

/// func_fish_pool (public entity docs): "fish_count" fish of its "model"
/// spread around its origin within "max_range" (they swim there:
/// `map::fish`). Placed as dynamic props of the pool's entity.
fn fish_pool_props(bsp: &Bsp) -> Vec<PropPlacement> {
    let parse = |v: &str| -> Option<[f32; 3]> {
        let mut it = v.split_whitespace().filter_map(|p| p.parse::<f32>().ok());
        Some([it.next()?, it.next()?, it.next()?])
    };
    let mut out = Vec::new();
    for (index, e) in bsp.entities.iter().enumerate() {
        if !e.prop("classname").is_some_and(|c| c.eq_ignore_ascii_case("func_fish_pool")) {
            continue;
        }
        let (Some(model), Some([x, y, z])) = (e.prop("model"), e.prop("origin").and_then(parse)) else {
            continue;
        };
        let count = e.prop("fish_count").and_then(|v| v.trim().parse::<usize>().ok()).unwrap_or(10).min(64);
        let range = e.prop("max_range").and_then(|v| v.trim().parse::<f32>().ok()).unwrap_or(150.0);
        for i in 0..count {
            // A fixed spread (golden angle), within the range, a little
            // above and below the origin.
            let a = i as f32 * 2.399_963;
            let r = range * 0.8 * ((i as f32 + 0.5) / count as f32).sqrt();
            let dz = ((i % 5) as f32 - 2.0) * range * 0.05;
            out.push(PropPlacement {
                model: model.to_lowercase(),
                skin: e.prop("skin").and_then(|s| s.trim().parse().ok()).unwrap_or(0),
                origin: vbsp::Vector {
                    x: x + r * a.cos(),
                    y: y + r * a.sin(),
                    z: z + dz,
                },
                angles: vbsp::Angles {
                    pitch: 0.0,
                    yaw: a.to_degrees() + 90.0,
                    roll: 0.0,
                },
                solid: PropSolid::None,
                lighting_origin: None,
                class: Some("func_fish_pool".to_string()),
                spawnflags: 0,
                massscale: 0.0,
                physicsmode: 0,
                fade: None,
                parent: None,
                entity: Some(index),
                body: 0,
                animated: true,
                enable_threshold: false,
                static_index: None,
            });
        }
    }
    out
}

/// A model's first sequence at its start, bone-local (rotation, position)
/// in the skeleton's frame and units: where a map ragdoll without Hammer's
/// pose starts (physics_brushes.md 6.1). None without animations.
fn first_sequence_pose(rig: &crate::map::MapRig) -> Option<Vec<(Quat, Vec3)>> {
    let set = &rig.animations;
    if set.sequences.is_empty() || set.defaults.len() != rig.bones.len() {
        return None;
    }
    let mut pose = set.defaults.clone();
    set.accumulate(&mut pose, 0, 0.0, 1.0, &set.default_params());
    Some(pose)
}

/// A ragdoll model's pose from Hammer's "angleOverride" (pairs of a
/// ragdoll part index, the model's `.phy` solid order, and its world
/// "pitch yaw roll"): each part bone turned to its angles, placed by its
/// nearest posed ancestor's frame and its reference offset from it (the
/// root part at the origin), the other bones following their parents in
/// the reference pose. Bone-local (rotation, position) in the skeleton's
/// frame and units; None without a ragdoll or an override.
pub fn ragdoll_pose(
    materials: &MaterialLoader,
    model: &str,
    bones: &[crate::map::MapBone],
    over: &str,
) -> Option<Vec<(Quat, Vec3)>> {
    let phy = materials.read(&format!("{}.phy", model.trim_end_matches(".mdl")))?;
    let phy = super::phy::parse_ragdoll(&phy).ok()?;
    let parts: Vec<&str> = over.split(',').map(str::trim).collect();
    let mut angles: HashMap<usize, Quat> = HashMap::new();
    for pair in parts.chunks(2) {
        let [index, a] = pair else { continue };
        let Ok(index) = index.parse::<usize>() else { continue };
        let a: Vec<f32> = a.split_whitespace().filter_map(|v| v.parse().ok()).collect();
        let Some(solid) = phy.solids.iter().find(|s| s.index == index) else {
            continue;
        };
        let Some(bone) = bones.iter().position(|b| b.name.eq_ignore_ascii_case(&solid.name)) else {
            continue;
        };
        if a.len() == 3 {
            angles.insert(bone, crate::map::entities::entity_rotation(Vec3::new(a[0], a[1], a[2])));
        }
    }
    if angles.is_empty() {
        return None;
    }
    // Reference pose in the skeleton's space.
    let mut reference: Vec<(Quat, Vec3)> = Vec::with_capacity(bones.len());
    for b in bones {
        let g = match b.parent.and_then(|p| reference.get(p).copied()) {
            Some((r, p)) => (r * b.rotation, p + r * b.position),
            None => (b.rotation, b.position),
        };
        reference.push(g);
    }
    // The posed ancestor of each bone (itself when posed).
    let posed_ancestor = |mut b: usize| -> Option<usize> {
        for _ in 0..bones.len() {
            b = bones[b].parent?;
            if angles.contains_key(&b) {
                return Some(b);
            }
        }
        None
    };
    let mut world: Vec<(Quat, Vec3)> = Vec::with_capacity(bones.len());
    for (i, b) in bones.iter().enumerate() {
        let w = match angles.get(&i) {
            Some(r) => {
                let at = match posed_ancestor(i) {
                    Some(a) => {
                        let (ar, ap) = world[a];
                        let (rr, rp) = reference[a];
                        ap + ar * (rr.inverse() * (reference[i].1 - rp))
                    }
                    None => Vec3::ZERO,
                };
                (*r, at)
            }
            None => match b.parent.and_then(|p| world.get(p).copied()) {
                Some((r, p)) => (r * b.rotation, p + r * b.position),
                None => (b.rotation, b.position),
            },
        };
        world.push(w);
    }
    Some(
        bones
            .iter()
            .enumerate()
            .map(|(i, b)| match b.parent.and_then(|p| world.get(p).copied()) {
                Some((pr, pp)) => (pr.inverse() * world[i].0, pr.inverse() * (world[i].1 - pp)),
                None => world[i],
            })
            .collect(),
    )
}

/// Names (lower case) that outputs send SetAnimation or
/// SetDefaultAnimation to.
fn animation_targets(bsp: &Bsp) -> std::collections::HashSet<String> {
    bsp.entities
        .iter()
        .flat_map(|e| e.properties().map(|(_, v)| v.to_string()).collect::<Vec<_>>())
        .filter_map(|v| {
            let mut it = v.split(['\u{1b}', ',']);
            let target = it.next()?.trim().to_ascii_lowercase();
            let input = it.next()?.trim().to_ascii_lowercase();
            matches!(input.as_str(), "setanimation" | "setdefaultanimation").then_some(target)
        })
        .collect()
}

#[allow(clippy::too_many_arguments)]
fn place_props(
    bsp: &Bsp,
    materials: &mut MaterialLoader,
    lighting: &MapLighting,
    occluders: &Occluders,
    data: &mut MapData,
    placements: Vec<PropPlacement>,
    hdr: bool,
) {
    // Each model's checksum and LOD 0 vertex order (`lod0_vertex_order`),
    // read when a prop of it has baked per-vertex light.
    let mut vertex_keys: HashMap<usize, Option<VertexKey>> = HashMap::new();
    let mut baked = 0usize;
    // Baked light files that didn't fit their prop's model.
    let mut unfit: Vec<String> = Vec::new();
    let mut loaded: HashMap<(String, i32, bool), Option<usize>> = HashMap::new();
    let mut prop_datas: HashMap<usize, HashMap<String, String>> = HashMap::new();
    let mut key_values: HashMap<usize, String> = HashMap::new();
    let mut failed: Vec<String> = Vec::new();
    let bounds = super::bsp::playable_bounds(bsp);
    let surfaces = super::surfaceprops::SurfaceProps::load(materials);
    // propdata.txt's templates (a model's prop_data "base"), read when an
    // entity prop needs one.
    let mut templates: Option<(super::hud::Kv, Vec<(String, Vec<String>)>)> = None;
    // How many pieces each model breaks into (its `MapModel::breaks`).
    let mut broken_into: HashMap<usize, usize> = HashMap::new();
    for prop in placements {
        let key = (prop.model.clone(), prop.skin, prop.animated);
        let model = *loaded
            .entry(key)
            .or_insert_with(|| match load_model_with_layout(materials, &prop.model) {
                Ok((m, kv, layout)) => {
                    let rig = if prop.animated {
                        load_rig(materials, &prop.model)
                    } else {
                        None
                    };
                    let mut model = if rig.is_some() {
                        // Skinned to its skeleton, in the skeleton's frame.
                        convert_model_in(&m, prop.skin, materials, false, &layout)
                    } else {
                        convert_model(&m, prop.skin, materials, &layout)
                    };
                    model.rig = rig;
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
                        .and_then(|b| read_mdl(&b).ok())
                        .map(|mdl| to_engine(v(m.apply_root_transform(mdl.header.illumination_position))));
                    data.models.push(model);
                    let index = data.models.len() - 1;
                    if let Some(pd) = kv.as_deref().and_then(prop_data) {
                        prop_datas.insert(index, pd);
                    }
                    if let Some(kv) = kv {
                        key_values.insert(index, kv);
                    }
                    Some(index)
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
        // The model's prop data, through its propdata.txt templates.
        let pd = if prop_datas.contains_key(&model) && prop.entity.is_some() {
            let t = templates.get_or_insert_with(|| {
                let text = materials
                    .read("scripts/propdata.txt")
                    .map(|b| String::from_utf8_lossy(&b).into_owned())
                    .unwrap_or_default();
                (
                    super::propdata::templates(&text),
                    super::breakables::breakable_models(&text),
                )
            });
            Some(
                key_values
                    .get(&model)
                    .and_then(|kv| super::propdata::resolve(kv, &t.0))
                    .unwrap_or_default(),
            )
        } else {
            None
        };
        let physics = body(
            &prop,
            &data.models[model],
            pd.as_ref().and_then(|p| p.physicsmode),
            &surfaces,
        );
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
        let vertex_light = prop
            .static_index
            .and_then(|n| match baked_vertex_light(materials, n, hdr, &prop.model, vertex_keys.entry(model)) {
                Ok(v) => v,
                Err(e) => {
                    unfit.push(format!("{}: {e}", prop.model));
                    None
                }
            });
        baked += vertex_light.is_some() as usize;
        let skybox = bounds.as_ref().is_some_and(|b| !b.contains(prop.origin));
        let door = prop.class.as_deref() == Some(DOOR_CLASS);
        // What the logic needs to know about the entity from its model.
        if let Some(index) = prop.entity
            && index < data.entities.len()
        {
            let mut extra = Vec::new();
            if !door && let (Some(pd), Some(t)) = (&pd, &templates) {
                let pieces = match broken_into.get(&model) {
                    Some(n) => *n,
                    None => {
                        let n = model_breaks(materials, &prop.model, pd, &t.1, data, model);
                        broken_into.insert(model, n);
                        n
                    }
                };
                let client = prop.class.as_deref() == Some("prop_physics_multiplayer")
                    && physics.as_ref().is_some_and(|p| p.push == crate::map::PushAway::Ignore);
                extra.extend(prop_keys(
                    pd,
                    pieces,
                    client,
                    physics.as_ref(),
                    &data.models[model],
                    &surfaces,
                ));
            }
            if door {
                if let Some((mv, open, close)) = key_values.get(&model).and_then(|kv| door_options(kv, prop.skin)) {
                    extra.push((DOOR_MOVE_KEY.to_string(), mv));
                    extra.push((DOOR_OPEN_KEY.to_string(), open));
                    extra.push((DOOR_CLOSE_KEY.to_string(), close));
                }
                let hardware = data.entities[index].get("hardware").map_or(0, |h| h.trim().parse().unwrap_or(0));
                if let Some((locked, unlocked)) = key_values.get(&model).and_then(|kv| door_hardware(kv, hardware)) {
                    extra.push((DOOR_LOCKED_KEY.to_string(), locked));
                    extra.push((DOOR_UNLOCKED_KEY.to_string(), unlocked));
                }
                let hulls = door_hulls(&data.models[model]);
                let e = &mut data.entities[index];
                e.hulls = hulls;
                e.mover = true;
            }
            data.entities[index].keyvalues.extend(extra);
        }
        // A map-placed ragdoll lies as Hammer posed it ("angleOverride";
        // physics_brushes.md 6.3), the entity's own angles then reset.
        let pose = (prop.class.as_deref() == Some("prop_ragdoll"))
            .then(|| {
                let rig = data.models[model].rig.clone()?;
                let over = data.entities.get(prop.entity?)?.get("angleOverride")?.to_string();
                ragdoll_pose(materials, &prop.model, &rig.bones, &over)
            })
            .flatten();
        let ragdoll = prop.class.as_deref() == Some("prop_ragdoll");
        // Its bodies and joints from the model's `.phy` (6.2): simulated
        // from the pose (`map::ragdoll::PlacedRagdoll`).
        let simulated = ragdoll
            .then(|| {
                let rig = data.models[model].rig.as_ref()?;
                let bytes = materials.read(&format!("{}.phy", prop.model.trim_end_matches(".mdl")))?;
                let phy = super::phy::parse_ragdoll(&bytes)
                    .map_err(|e| warn!("{}: ragdoll: {e}", prop.model))
                    .ok()?;
                self::ragdoll(&phy, &rig.bones, &surfaces, &prop.model).map(std::sync::Arc::new)
            })
            .flatten();
        // Without Hammer's pose a simulated ragdoll starts from its first
        // sequence's pose at the entity's origin and angles (6.1).
        let hammer_posed = pose.is_some();
        let pose = match (pose, &simulated) {
            (None, Some(_)) => data.models[model].rig.as_deref().and_then(first_sequence_pose),
            (pose, _) => pose,
        };
        let (translation, rotation) = match (&pose, ragdoll) {
            _ if hammer_posed => (translation, Quat::IDENTITY),
            _ if simulated.is_some() => (translation, rotation),
            (Some(_), _) => (translation, Quat::IDENTITY),
            // No pose from Hammer and no ragdoll in the model to simulate
            // (Open question 7): it lies on its back on the floor below.
            (None, true) => {
                let down = translation - Vec3::Y * 1024.0 * METERS_PER_UNIT;
                let f = data
                    .collision_brushes
                    .iter()
                    .filter_map(|b| b.sweep_box(Vec3::ZERO, translation, down, 0.0).map(|(f, _)| f))
                    .fold(1.0, f32::min);
                let floor = translation.lerp(down, f);
                let lying = vbsp::Angles {
                    pitch: -90.0,
                    yaw: prop.angles.yaw,
                    roll: 0.0,
                };
                (floor + Vec3::Y * 6.0 * METERS_PER_UNIT, super::props::rotation(lying))
            }
            (None, false) => (translation, rotation),
        };
        data.props.push(MapProp {
            pose: pose.map(std::sync::Arc::new),
            ragdoll: simulated,
            model,
            translation,
            rotation,
            skybox,
            lighting: Some(lighting),
            vertex_light,
            solid,
            // A model door's shadow would stay where it was baked, and an
            // animated prop's where it spawned.
            casts_shadow: prop.class.is_some() && !door && data.models[model].rig.is_none(),
            physics: physics.filter(|_| prop.parent.is_none()),
            parent: prop.parent,
            fade: prop.fade.map(|(a, b)| (a * METERS_PER_UNIT, b * METERS_PER_UNIT)),
            entity: prop.entity,
            skin: prop.skin,
            body: prop.body,
        });
    }
    failed.sort();
    failed.dedup();
    data.warnings.extend(failed);
    if baked > 0 {
        info!("{baked} static props lit by their baked per-vertex light");
    }
    if let Some(first) = unfit.first() {
        data.warnings.push(format!(
            "{} static props' baked per-vertex light doesn't fit their model (lit by the probe instead), e.g. {first}",
            unfit.len()
        ));
    }
}

/// What a model's baked per-vertex light must match: its checksum and the
/// model vertex (`.vvd` index) of each LOD 0 hardware vertex, in `.vtx`
/// order (`lod0_vertex_order`).
struct VertexKey {
    checksum: u32,
    order: Vec<u32>,
}

/// The `.vvd` index of each vertex the model's LOD 0 meshes draw, in the
/// order the `.vtx` lists them (body part, model, mesh, strip group,
/// vertex): the order baked per-vertex light is stored in. (Each LOD's
/// colours count its `.vtx` vertices, not the `.vvd` table, which also
/// holds vertices only lower LODs use: de_nuke's fuel cask draws 1179 of
/// its 1955 at LOD 0, and its `.vhv` holds 1179.)
fn lod0_vertex_order(mdl: &vmdl::mdl::Mdl, vtx: &vmdl::vtx::Vtx) -> Vec<u32> {
    let mut order = Vec::new();
    for (part, vpart) in mdl.body_parts.iter().zip(&vtx.body_parts) {
        for (model, vmodel) in part.models.iter().zip(&vpart.models) {
            let Some(lod) = vmodel.lods.first() else { continue };
            for (mesh, vmesh) in model.meshes.iter().zip(&lod.meshes) {
                let base = (model.vertex_offset + mesh.vertex_offset) as u32;
                for group in &vmesh.strip_groups {
                    order.extend(group.vertices.iter().map(|v| base + v.original_mesh_vertex_id as u32));
                }
            }
        }
    }
    order
}

fn vertex_key(materials: &MaterialLoader, path: &str) -> Option<VertexKey> {
    let mdl_bytes = materials.read(path)?;
    let checksum = u32::from_le_bytes(mdl_bytes.get(8..12)?.try_into().ok()?);
    let mdl = read_mdl(&mdl_bytes).ok()?;
    let vtx = vmdl::vtx::Vtx::read(&materials.read(&format!("{}.dx90.vtx", path.trim_end_matches(".mdl")))?).ok()?;
    Some(VertexKey {
        checksum,
        order: lod0_vertex_order(&mdl, &vtx),
    })
}

/// Static prop `index`'s baked per-vertex light (`vhv`), decoded, by the
/// model's own vertex index, when the map ships it for the prop's model as
/// loaded (`key`: the model's `VertexKey`, read on first need). Ok(None):
/// the map has none for it, and the prop keeps its light probe, as the
/// game does for props compiled without it; Err: a file that doesn't fit
/// the model.
#[allow(clippy::type_complexity)]
fn baked_vertex_light(
    materials: &MaterialLoader,
    index: usize,
    hdr: bool,
    path: &str,
    key: std::collections::hash_map::Entry<usize, Option<VertexKey>>,
) -> Result<Option<std::sync::Arc<Vec<[f32; 3]>>>, String> {
    let Some((name, bytes)) = super::vhv::names(index, hdr)
        .into_iter()
        .find_map(|n| materials.read_packed(&n).map(|b| (n, b)))
    else {
        return Ok(None);
    };
    let Some(key) = key.or_insert_with(|| vertex_key(materials, path)).as_ref() else {
        return Ok(None);
    };
    let v = super::vhv::parse(&bytes).map_err(|e| format!("{name}: {e}"))?;
    if v.checksum != key.checksum || v.colors.len() != key.order.len() {
        return Err(format!(
            "{name} (checksum {:08x}, {} vertices; the model's {:08x}, {})",
            v.checksum,
            v.colors.len(),
            key.checksum,
            key.order.len()
        ));
    }
    let size = key.order.iter().max().map_or(0, |m| *m as usize + 1);
    let mut light = vec![[0.0f32; 3]; size];
    for (&at, c) in key.order.iter().zip(v.colors) {
        light[at as usize] = super::vhv::decode(c);
    }
    Ok(Some(std::sync::Arc::new(light)))
}

/// A model's skeleton and sequences (animated props), when it has any.
fn load_rig(materials: &mut MaterialLoader, path: &str) -> Option<std::sync::Arc<MapRig>> {
    let read = |p: &str| materials.read(p);
    let bones = super::anim::bones(&read, path)
        .inspect_err(|e| warn!("{path}: skeleton: {e}"))
        .ok()?;
    let animations = super::anim::load(&read, path)
        .inspect_err(|e| warn!("{path}: animations: {e}"))
        .ok()?;
    if bones.is_empty() || animations.sequences.is_empty() {
        return None;
    }
    let bones = bones
        .into_iter()
        .map(|(name, parent, rotation, position)| MapBone {
            name,
            parent,
            position,
            rotation,
        })
        .collect();
    // Source axes to ours (x, z, -y): -90 degrees about X; units to meters.
    let root = Transform::from_rotation(Quat::from_rotation_x(-std::f32::consts::FRAC_PI_2))
        .with_scale(Vec3::splat(METERS_PER_UNIT));
    Some(std::sync::Arc::new(MapRig {
        bones,
        root,
        animations: std::sync::Arc::new(animations),
    }))
}

/// Model doors (specs/source/doors_buttons.md, "prop_door_rotating"):
/// drawn as a prop riding a mover node the logic turns.
pub const DOOR_CLASS: &str = "prop_door_rotating";

/// Load what a prop model breaks into (prop_damage.md 7.3) onto
/// `data.models[model].breaks`, its piece models into `data.gibs`: the
/// `.phy` break pieces whose models load, else `breakable_count` chunks of
/// the gib list `breakable_model` names. Returns the piece count (2.2
/// step 1).
fn model_breaks(
    materials: &mut MaterialLoader,
    path: &str,
    pd: &super::propdata::PropData,
    lists: &[(String, Vec<String>)],
    data: &mut MapData,
    model: usize,
) -> usize {
    use crate::map::{MapBreak, MapBreakPiece, breakables::MapGibSet};
    let blocks = materials
        .read(&format!("{}.phy", path.trim_end_matches(".mdl")))
        .map(|b| super::phy::break_blocks(&b))
        .unwrap_or_default();
    let mut models = Vec::new();
    let mut pieces = Vec::new();
    for p in super::propdata::break_pieces(&blocks) {
        match load_shell(materials, &p.model) {
            Ok(m) => {
                pieces.push(MapBreakPiece {
                    model: models.len(),
                    offset: to_engine(v_src(p.offset)),
                    centre: (m.bounds.0 + m.bounds.1) / 2.0,
                    life: if p.fadetime > 0.0 { p.fadetime } else { f32::INFINITY },
                    burst: p.burst * METERS_PER_UNIT,
                    frozen: p.motion_disabled,
                    fade_dist: p.fade_dist.map(|(a, b)| (a * METERS_PER_UNIT, b * METERS_PER_UNIT)),
                });
                models.push(m);
            }
            Err(e) => data.warnings.push(e),
        }
    }
    if !pieces.is_empty() {
        let n = pieces.len();
        data.gibs.push(MapGibSet {
            name: path.to_string(),
            models,
        });
        data.models[model].breaks = Some(MapBreak::Pieces {
            set: path.to_string(),
            pieces,
        });
        return n;
    }
    let (Some(name), count) = (&pd.breakable_model, pd.breakable_count) else {
        return 0;
    };
    if count <= 0 {
        return 0;
    }
    if let Some((list, paths)) = super::propdata::chunk_list(lists, name) {
        if !data.gibs.iter().any(|g| g.name.eq_ignore_ascii_case(list)) {
            let mut models = Vec::new();
            for p in paths {
                match load_shell(materials, p) {
                    Ok(m) => models.push(m),
                    Err(e) => data.warnings.push(e),
                }
            }
            data.gibs.push(MapGibSet {
                name: list.clone(),
                models,
            });
        }
        let centres = data
            .gibs
            .iter()
            .find(|g| g.name.eq_ignore_ascii_case(list))
            .map(|g| g.models.iter().map(|m| (m.bounds.0 + m.bounds.1) / 2.0).collect())
            .unwrap_or_default();
        // The box in Source axes, units.
        let (lo, hi) = data.models[model].bounds;
        let s = (hi - lo) / METERS_PER_UNIT;
        data.models[model].breaks = Some(MapBreak::Chunks {
            set: list.clone(),
            count: count as usize,
            size_limit: super::propdata::chunk_size_limit(Vec3::new(s.x, s.z, s.y)),
            life: CHUNK_LIFE,
            centres,
            skin: pd.breakable_skin,
        });
    }
    count as usize
}

/// Template chunks live this long (seconds, a range; spec 7.3).
const CHUNK_LIFE: (f32, f32) = (5.0, 10.0);

/// The sound entry an exploding prop with the onbreak-explode
/// interaction plays (spec 7.5).
const EXPLODE_FIRE_SOUND: &str = "PropaneTank.Burst";

/// The keyvalues the logic reads a prop's damage rules from
/// (`map::entities::PROP_*_KEY`).
fn prop_keys(
    pd: &super::propdata::PropData,
    pieces: usize,
    client: bool,
    physics: Option<&MapPhysics>,
    model: &MapModel,
    surfaces: &super::surfaceprops::SurfaceProps,
) -> Vec<(String, String)> {
    use crate::map::entities::*;
    let mut out = Vec::new();
    let mut put = |k: &str, v: String| out.push((k.to_string(), v));
    if let Some(h) = pd.health {
        put(PROP_HEALTH_KEY, h.to_string());
    }
    put(PROP_DAMAGE_KEY, format!("{} {} {}", pd.bullets, pd.club, pd.explosive));
    put(PROP_PIECES_KEY, pieces.to_string());
    if !pd.interactions.is_empty() {
        put(PROP_INTERACTIONS_KEY, pd.interactions.join(" "));
    }
    let (d, r) = (pd.explosive_damage.unwrap_or(0.0), pd.explosive_radius.unwrap_or(0.0));
    if d > 0.0 || r > 0.0 {
        put(PROP_EXPLODE_KEY, format!("{d} {r}"));
    }
    if pd.interactions.contains(&"explode_fire") {
        put(PROP_EXPLODE_SOUND_KEY, EXPLODE_FIRE_SOUND.to_string());
    }
    if let Some(t) = &pd.damage_table {
        put(PROP_TABLE_KEY, t.clone());
    }
    if client {
        put(PROP_CLIENT_KEY, "1".to_string());
    }
    if let Some(c) = &model.collision {
        if let Some(s) = super::propdata::break_sound(surfaces, &c.surfaceprop) {
            put(PROP_BREAK_SOUND_KEY, s);
        }
        put(PROP_MASS_KEY, physics.map_or(c.mass, |p| p.mass).to_string());
    }
    out
}

/// A door model's (move, open, close) sound entries from its
/// A door model's handle sounds for its `hardware` type: (locked,
/// unlocked) from the `door_options` block "hardwareN" (stock door
/// models: hardware0 "DoorSound.Null", hardware1/2 "DoorHandles.Locked1",
/// "DoorHandles.Unlocked1"...). The spec leaves which entries the
/// hardware picks open (doors_buttons.md Q8): this is our reading of the
/// model data.
fn door_hardware(text: &str, hardware: i32) -> Option<(String, String)> {
    use super::hud::Kv;
    fn find<'a>(kv: &'a Kv, name: &str) -> Option<&'a Kv> {
        kv.items().iter().find_map(|(k, v)| {
            if k.eq_ignore_ascii_case(name) {
                Some(v)
            } else {
                find(v, name)
            }
        })
    }
    let kv = super::hud::parse(text);
    let block = find(&kv, "door_options")?.get(&format!("hardware{hardware}"))?;
    let s = |k: &str| block.str(k).unwrap_or("").to_string();
    Some((s("locked"), s("unlocked")))
}

/// `door_options` key values: the block for its skin ("skinN"), else
/// "default".
fn door_options(text: &str, skin: i32) -> Option<(String, String, String)> {
    use super::hud::Kv;
    fn find<'a>(kv: &'a Kv, name: &str) -> Option<&'a Kv> {
        kv.items().iter().find_map(|(k, v)| {
            if k.eq_ignore_ascii_case(name) {
                Some(v)
            } else {
                find(v, name)
            }
        })
    }
    let kv = super::hud::parse(text);
    let options = find(&kv, "door_options")?;
    let block = options.get(&format!("skin{skin}")).or_else(|| options.get("default"))?;
    let s = |k: &str| block.str(k).unwrap_or("").to_string();
    Some((s("move"), s("open"), s("close")))
}

/// A model door's solid volumes in entity space (units, Z up, relative to
/// its origin, unrotated): its collision model's pieces, else its bounds.
fn door_hulls(model: &MapModel) -> Vec<crate::map::MapHull> {
    let point = |p: Vec3| Vec3::new(p.x, -p.z, p.y) / METERS_PER_UNIT;
    let normal = |n: Vec3| Vec3::new(n.x, -n.z, n.y);
    if let Some(c) = &model.collision
        && !c.pieces.is_empty()
    {
        return c
            .pieces
            .iter()
            .map(|p| crate::map::MapHull {
                points: p.points.iter().map(|q| point(*q)).collect(),
                planes: p
                    .planes
                    .iter()
                    .map(|(n, d)| (normal(*n), d / METERS_PER_UNIT))
                    .collect(),
            })
            .collect();
    }
    let (a, b) = (point(model.bounds.0), point(model.bounds.1));
    let (lo, hi) = (a.min(b), a.max(b));
    if (hi - lo).min_element() <= 0.0 {
        return Vec::new();
    }
    let points = (0..8)
        .map(|k| {
            Vec3::new(
                if k & 1 == 0 { lo.x } else { hi.x },
                if k & 2 == 0 { lo.y } else { hi.y },
                if k & 4 == 0 { lo.z } else { hi.z },
            )
        })
        .collect();
    let planes = vec![
        (Vec3::X, hi.x),
        (Vec3::NEG_X, -lo.x),
        (Vec3::Y, hi.y),
        (Vec3::NEG_Y, -lo.y),
        (Vec3::Z, hi.z),
        (Vec3::NEG_Z, -lo.z),
    ];
    vec![crate::map::MapHull { planes, points }]
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
