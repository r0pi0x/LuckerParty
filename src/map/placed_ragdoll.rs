//! Map-placed ragdolls (prop_ragdoll, specs/source/physics_brushes.md 6):
//! the model's `.phy` ragdoll simulated from its placed pose, as dead
//! characters' ragdolls are (`ragdoll`), and its drawn skeleton posed from
//! the bodies.
//!
//! - The loader gives the prop its ragdoll and start pose (Hammer's
//!   "angleOverride", else its first sequence at the entity's angles);
//!   the map spawn puts `PlacedRagdoll` on its node.
//! - `make` spawns the bodies and joints (a `Ragdoll` whose owner is the
//!   node): debris (flag 4, every community map's) on the ragdoll layer,
//!   so players walk through and shots push the part they hit
//!   (`ragdoll::RagdollShot`); others solid like a physics prop. Flag
//!   16384 pins every part (static) until EnableMotion, flag 65536 holds
//!   them still until a shot or the Wake input wakes them (6.4, 6.7).
//! - `pose` moves the node with body 0 and poses the drawn bones from the
//!   bodies (as `ragdoll::pose_ragdolls` does a character's).
//! - Killed by the logic: the bodies go out of the simulation; a round
//!   restart makes it again where it was placed.
//! - In a network game the server sends the parts' poses (`NetRagdoll`,
//!   spec 6.10: up to 24 parts) and a client draws its copy from them
//!   (`net::ragdolls`).

use std::sync::Arc;

use avian3d::prelude::*;
use bevy::prelude::*;

use super::ragdoll::{BodyShape, Ragdoll, RagdollJoint, spawn_body};
use super::{MapBone, MapPart, MapRagdoll};
use crate::core::{NOT_SHADOW, RAGDOLL_LAYER};

/// prop_ragdoll spawnflags (6).
pub const SF_DEBRIS: u32 = 4;
pub const SF_MOTION_DISABLED: u32 = 16384;
pub const SF_START_ASLEEP: u32 = 65536;

/// A map-placed ragdoll, on its prop node.
#[derive(Component, Clone, Debug)]
pub struct PlacedRagdoll {
    pub ragdoll: Arc<MapRagdoll>,
    /// The model's skeleton (`MapRig::bones`) and its space in the node's.
    pub bones: Arc<Vec<MapBone>>,
    pub root: Transform,
    /// The start pose, bone-local (`MapProp::pose`; None: the reference).
    pub pose: Option<Arc<Vec<(Quat, Vec3)>>>,
    /// The drawn skeleton's joints, by bone.
    pub joints: Vec<Entity>,
    pub flags: u32,
    /// The node's own transform as placed (a round restart puts it back).
    pub home: Transform,
    /// The simulation (`Ragdoll`), once made.
    pub sim: Option<Entity>,
    /// The node in body 0's frame (the node follows body 0).
    pub follow: Transform,
    /// Every part pinned (motion disabled: 6.7).
    pub pinned: bool,
    /// Held still since it spawned (flag 65536) until woken.
    pub asleep: bool,
    /// The logic removed it (until a round restart), or turned its
    /// collision off (`logic::bridge` sets these).
    pub gone: bool,
    pub solid: bool,
}

impl PlacedRagdoll {
    pub fn new(
        ragdoll: Arc<MapRagdoll>,
        rig: &super::MapRig,
        pose: Option<Arc<Vec<(Quat, Vec3)>>>,
        joints: Vec<Entity>,
        flags: u32,
        home: Transform,
    ) -> Self {
        Self {
            ragdoll,
            bones: Arc::new(rig.bones.clone()),
            root: rig.root,
            pose,
            joints,
            flags,
            home,
            sim: None,
            follow: Transform::IDENTITY,
            pinned: flags & SF_MOTION_DISABLED != 0,
            asleep: flags & SF_START_ASLEEP != 0,
            gone: false,
            solid: true,
        }
    }

    /// Bone `i`'s start pose (parent-relative).
    fn local(&self, i: usize) -> (Quat, Vec3) {
        self.pose
            .as_ref()
            .and_then(|p| p.get(i).copied())
            .unwrap_or((self.bones[i].rotation, self.bones[i].position))
    }

    /// Bones in the skeleton's space from parent-relative transforms.
    fn globals(&self, local: impl Fn(usize) -> (Quat, Vec3)) -> Vec<(Quat, Vec3)> {
        let mut out: Vec<(Quat, Vec3)> = Vec::with_capacity(self.bones.len());
        for (i, b) in self.bones.iter().enumerate() {
            let (q, p) = local(i);
            out.push(match b.parent.and_then(|p| out.get(p)) {
                Some((pq, pp)) => (*pq * q, *pp + *pq * p),
                None => (q, p),
            });
        }
        out
    }

    /// Whether its bodies are held still (pinned, or asleep since spawn).
    pub fn held(&self) -> bool {
        self.pinned || self.asleep
    }
}

pub(super) fn plugin(app: &mut App) {
    app.add_systems(
        FixedUpdate,
        (restart, make, sync).chain().after(crate::core::apply_damage),
    )
    .add_systems(Update, pose.before(bevy::transform::TransformSystems::Propagate));
}

/// Where a node sits in the world: its parent's global transform and its
/// own (a node made this frame has no propagated global transform yet).
fn node_world(t: &Transform, parent: Option<&GlobalTransform>) -> Transform {
    match parent {
        Some(g) => Transform::from_matrix(g.to_matrix() * t.to_matrix()),
        None => *t,
    }
}

/// Make each placed ragdoll's bodies and joints from its start pose.
#[allow(clippy::type_complexity)]
fn make(
    mut nodes: Query<(
        Entity,
        &mut PlacedRagdoll,
        &Transform,
        Option<&ChildOf>,
        Option<&Children>,
    )>,
    globals: Query<&GlobalTransform>,
    meshes: Query<(), With<Mesh3d>>,
    time: Res<Time>,
    mut commands: Commands,
) {
    for (node, mut placed, t, parent, children) in &mut nodes {
        if placed.sim.is_some() || placed.gone {
            continue;
        }
        let world = node_world(t, parent.and_then(|p| globals.get(p.parent()).ok()));
        let base = world * placed.root;
        let scale = placed.root.scale.x;
        let start = placed.globals(|i| placed.local(i));
        let bind = placed.globals(|i| (placed.bones[i].rotation, placed.bones[i].position));
        let ragdoll = placed.ragdoll.clone();
        let sim = commands
            .spawn((Name::new("Placed ragdoll"), MapPart, Transform::default()))
            .id();
        let masks = ragdoll.collision_masks();
        let layers = if placed.flags & SF_DEBRIS != 0 {
            CollisionLayers::new(RAGDOLL_LAYER, LayerMask::DEFAULT | RAGDOLL_LAYER)
        } else {
            CollisionLayers::new(LayerMask::DEFAULT, NOT_SHADOW)
        };
        let mut bodies = Vec::with_capacity(ragdoll.bodies.len());
        let mut root_world = Transform::IDENTITY;
        for (i, body) in ragdoll.bodies.iter().enumerate() {
            let (q, p) = start.get(body.bone).copied().unwrap_or_default();
            let at = Transform::from_translation(base.transform_point(p)).with_rotation(base.rotation * q);
            if i == 0 {
                root_world = at;
            }
            let e = spawn_body(
                &mut commands,
                sim,
                i,
                body,
                BodyShape::new(body, scale),
                masks[i],
                at,
                (Vec3::ZERO, Vec3::ZERO),
                layers,
            );
            if placed.held() {
                commands.entity(e).insert(RigidBody::Static);
            }
            bodies.push(e);
        }
        let mut parents = vec![None; ragdoll.bodies.len()];
        let mut joints = Vec::with_capacity(ragdoll.joints.len());
        for j in &ragdoll.joints {
            let (Some(pb), Some(cb)) = (ragdoll.bodies.get(j.parent), ragdoll.bodies.get(j.child)) else {
                continue;
            };
            let (Some(&(pq, pp)), Some(&(cq, cp))) = (bind.get(pb.bone), bind.get(cb.bone)) else {
                continue;
            };
            let anchor = pq.inverse() * (cp - pp);
            parents[j.child] = Some((j.parent, anchor));
            joints.push(
                commands
                    .spawn((
                        Name::new(format!("Ragdoll joint {}-{}", j.parent, j.child)),
                        MapPart,
                        RagdollJoint {
                            parent: bodies[j.parent],
                            child: bodies[j.child],
                            anchor: anchor * scale,
                            bind: pq.inverse() * cq,
                            limits: j.limits,
                        },
                    ))
                    .id(),
            );
        }
        commands.entity(sim).insert(Ragdoll::placed(
            node,
            bodies,
            joints,
            parents,
            scale,
            root_world.translation,
            time.elapsed_secs_f64(),
        ));
        // Its meshes' bounds are the reference pose's.
        for c in children.into_iter().flatten() {
            if meshes.contains(*c) {
                commands.entity(*c).insert(bevy::camera::visibility::NoFrustumCulling);
            }
        }
        placed.follow = Transform::from_matrix(root_world.to_matrix().inverse() * world.to_matrix());
        placed.sim = Some(sim);
        debug!("placed ragdoll {node}: {} bodies", ragdoll.bodies.len());
    }
}

/// A round restart: each placed ragdoll is made again where it was placed
/// (6.4: the entity respawns with the map's).
fn restart(
    restarts: Option<Res<crate::core::RoundRestarts>>,
    mut nodes: Query<(&mut PlacedRagdoll, &mut Transform)>,
    sims: Query<&Ragdoll>,
    mut joints: Query<&mut Transform, Without<PlacedRagdoll>>,
    mut commands: Commands,
) {
    if !restarts.is_some_and(|r| r.is_changed() && !r.is_added()) {
        return;
    }
    for (mut placed, mut t) in &mut nodes {
        if let Some(sim) = placed.sim.take()
            && let Ok(r) = sims.get(sim)
        {
            for x in r.joints.iter().chain(&r.bodies) {
                if let Ok(mut x) = commands.get_entity(*x) {
                    x.despawn();
                }
            }
            commands.entity(sim).despawn();
        }
        *t = placed.home;
        for (i, j) in placed.joints.iter().enumerate() {
            if let Ok(mut jt) = joints.get_mut(*j) {
                let (q, p) = placed.local(i);
                jt.rotation = q;
                jt.translation = p;
            }
        }
        placed.pinned = placed.flags & SF_MOTION_DISABLED != 0;
        placed.asleep = placed.flags & SF_START_ASLEEP != 0;
    }
}

/// Bodies follow their ragdoll's state: out of the simulation once the
/// logic removed it, not solid while its collision is off, held still
/// while pinned or asleep; a simulation whose node is gone goes too.
#[allow(clippy::type_complexity)]
fn sync(
    sims: Query<(Entity, &Ragdoll)>,
    nodes: Query<&PlacedRagdoll>,
    bodies: Query<(
        &RigidBody,
        Has<RigidBodyDisabled>,
        Has<ColliderDisabled>,
        Has<super::interp::NetDrawn>,
    )>,
    mut commands: Commands,
) {
    for (e, r) in &sims {
        if !r.placed {
            continue;
        }
        let Ok(placed) = nodes.get(r.owner) else {
            for x in r.joints.iter().chain(&r.bodies) {
                if let Ok(mut x) = commands.get_entity(*x) {
                    x.despawn();
                }
            }
            commands.entity(e).despawn();
            continue;
        };
        for b in &r.bodies {
            let Ok((rb, disabled, no_collision, drawn)) = bodies.get(*b) else {
                continue;
            };
            if disabled != placed.gone {
                if placed.gone {
                    commands.entity(*b).insert(RigidBodyDisabled);
                } else {
                    commands.entity(*b).remove::<RigidBodyDisabled>();
                }
            }
            let solid = placed.solid && !placed.gone;
            if no_collision == solid {
                if solid {
                    commands.entity(*b).remove::<ColliderDisabled>();
                } else {
                    commands.entity(*b).insert(ColliderDisabled);
                }
            }
            // A network client's copy only follows the server's.
            if drawn {
                continue;
            }
            let want = if placed.held() {
                RigidBody::Static
            } else {
                RigidBody::Dynamic
            };
            if *rb != want {
                commands.entity(*b).insert(want);
            }
        }
    }
}

/// Pose each placed ragdoll's drawn skeleton from its bodies: the node
/// follows body 0, a simulated bone takes its body's rotation and its
/// place from its parent body's bone and the joint anchor (body 0: its
/// own), the other bones keep their start pose under their parent.
#[allow(clippy::type_complexity)]
fn pose(
    mut nodes: Query<(&PlacedRagdoll, &mut Transform, Option<&ChildOf>)>,
    sims: Query<&Ragdoll>,
    globals: Query<&GlobalTransform>,
    parts: Query<&Transform, (With<super::RagdollBody>, Without<PlacedRagdoll>)>,
    mut joints: Query<&mut Transform, (Without<super::RagdollBody>, Without<PlacedRagdoll>)>,
) {
    for (placed, mut t, parent) in &mut nodes {
        let Some(r) = placed.sim.and_then(|s| sims.get(s).ok()) else {
            continue;
        };
        let Some(root) = r.bodies.first().and_then(|b| parts.get(*b).ok()) else {
            continue;
        };
        let world = *root * placed.follow;
        let local = match parent.and_then(|p| globals.get(p.parent()).ok()) {
            Some(g) => Transform::from_matrix(g.to_matrix().inverse() * world.to_matrix()),
            None => world,
        };
        if *t != local {
            *t = local;
        }
        let base = world * placed.root;
        let to_skeleton = base.to_matrix().inverse();
        let inv = base.rotation.inverse();
        let mut simulated: Vec<Option<usize>> = vec![None; placed.bones.len()];
        for (i, b) in placed.ragdoll.bodies.iter().enumerate() {
            if let Some(s) = simulated.get_mut(b.bone) {
                *s = Some(i);
            }
        }
        let mut global: Vec<(Quat, Vec3)> = Vec::with_capacity(placed.bones.len());
        for (bone, b) in placed.bones.iter().enumerate() {
            let parent = b.parent.and_then(|p| global.get(p).copied());
            let body = simulated[bone].and_then(|i| Some((i, parts.get(*r.bodies.get(i)?).ok()?)));
            let g = match body {
                Some((i, bt)) => {
                    let q = inv * bt.rotation;
                    let p = match r.parents.get(i).copied().flatten() {
                        Some((pi, anchor)) => {
                            let (pq, pp) = global[placed.ragdoll.bodies[pi].bone];
                            pp + pq * anchor
                        }
                        None => to_skeleton.transform_point3(bt.translation),
                    };
                    (q, p)
                }
                None => {
                    let (q, p) = placed.local(bone);
                    match parent {
                        Some((pq, pp)) => (pq * q, pp + pq * p),
                        None => (q, p),
                    }
                }
            };
            global.push(g);
        }
        for (bone, b) in placed.bones.iter().enumerate() {
            let (q, p) = global[bone];
            let local = match b.parent.map(|i| global[i]) {
                Some((pq, pp)) => (pq.inverse() * q, pq.inverse() * (p - pp)),
                None => (q, p),
            };
            if let Some(mut jt) = placed.joints.get(bone).and_then(|j| joints.get_mut(*j).ok())
                && (jt.rotation != local.0 || jt.translation != local.1)
            {
                jt.rotation = local.0;
                jt.translation = local.1;
            }
        }
    }
}

/// Wake a placed ragdoll held asleep since it spawned (a shot, the Wake
/// input); false when it isn't one.
pub fn wake(world: &mut World, node: Entity) -> bool {
    match world.get_mut::<PlacedRagdoll>(node) {
        Some(mut p) => {
            p.asleep = false;
            true
        }
        None => false,
    }
}

/// EnableMotion / DisableMotion (6.7); false when it isn't one.
pub fn set_motion(world: &mut World, node: Entity, enabled: bool) -> bool {
    match world.get_mut::<PlacedRagdoll>(node) {
        Some(mut p) => {
            p.pinned = !enabled;
            if enabled {
                p.asleep = false;
            }
            true
        }
        None => false,
    }
}
