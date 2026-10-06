//! Ragdolls for any game (specs/cs_source/ragdolls.md): when a character
//! dies, its body becomes rigid bodies (one per simulated bone) held by
//! joints with per-axis angle limits, seeded from the animated pose and
//! the killing hit, and its drawn skeleton follows them.
//!
//! - Every tick each living character's skeleton pose is kept for a short
//!   while (`SkeletonPose`): hitboxes use the newest, ragdolls take bone
//!   velocities from it.
//! - On `Died` a `Ragdoll` entity is spawned with its `RagdollBody`s and
//!   `RagdollJoint`s (a custom XPBD constraint: ball and socket plus
//!   X-then-Y-then-Z Euler limits, spec Open question 5's proposal); the
//!   owner gets `Ragdolled` and loses its drawn body to the ragdoll.
//! - Ragdolls collide with the world and props only
//!   (`core::RAGDOLL_LAYER`), sleep after 5 s of stillness, and go away
//!   when their owner lives again (respawn or round restart) or is gone.

use std::collections::VecDeque;

use avian3d::{
    dynamics::{
        joints::EntityConstraint,
        solver::{
            joint_graph::JointGraphPlugin,
            solver_body::{SolverBody, SolverBodyInertia},
            xpbd::{
                AngularConstraint, PositionConstraint, XpbdConstraint, XpbdConstraintSolverData, XpbdSolverSystems,
                joints::PointConstraintShared, prepare_xpbd_joint, solve_xpbd_joint,
            },
        },
    },
    prelude::*,
};
use bevy::prelude::*;

use super::{BodyJoint, BodyModel, CharacterBody, CharacterModels, MapCharacterModel, MapPart, anim};
use crate::core::{Died, Health, Intent, RAGDOLL_LAYER};

/// A character model's ragdoll: bodies on its bones, joints between them
/// (specs/cs_source/ragdolls.md 1). Shapes and anchors are in the
/// skeleton's frames and units (like `BoneBox`; `MapCharacterModel::root`
/// converts), masses in kg, angles in radians.
#[derive(Clone, Debug, Default)]
pub struct MapRagdoll {
    pub bodies: Vec<MapRagdollBody>,
    pub joints: Vec<MapRagdollJoint>,
    /// Body pairs allowed to collide with each other (not simulated yet:
    /// docs/tech-debt.md).
    pub collision_pairs: Vec<(usize, usize)>,
}

/// One rigid body, following bone `bone`.
#[derive(Clone, Debug)]
pub struct MapRagdollBody {
    pub bone: usize,
    /// Convex pieces (their corners) in the bone's frame.
    pub pieces: Vec<Vec<Vec3>>,
    /// kg.
    pub mass: f32,
    pub damping: f32,
    pub rotdamping: f32,
    /// Multiplier on the shape's inertia tensor.
    pub inertia: f32,
    pub surfaceprop: String,
}

/// A ball-and-socket joint: `child`'s bone origin is held at its bind-pose
/// place in `parent`'s bone frame; the child's rotation relative to that
/// bind orientation, as X-then-Y-then-Z Euler angles about its own axes,
/// stays inside `limits` (min, max radians per axis).
#[derive(Clone, Debug, PartialEq)]
pub struct MapRagdollJoint {
    pub parent: usize,
    pub child: usize,
    pub limits: [(f32, f32); 3],
}

/// Bone velocities look this far back (spec 2.2: 0.05 s for players).
const BONE_DT: f64 = 0.05;
/// How much pose history characters keep, s.
const HISTORY: f64 = 0.25;
/// The killing hit's push, kg·m/s: CS:S's ammo impulses are unknown (spec
/// Open question 2); this is the SDK template's .50 AE value, 2400
/// kg·in/s, the same one our guns give props.
pub const DEATH_IMPULSE: f32 = 2400.0 * 0.0254;
/// Source clamps every body to 2000 in/s and 3600 °/s.
const MAX_LINEAR_SPEED: f32 = 2000.0 * 0.0254;
const MAX_ANGULAR_SPEED: f32 = 3600.0 * std::f32::consts::PI / 180.0;
/// Body 0 moving less than this on every axis per tick counts as still
/// (spec 6.1: 1 in), m.
const SETTLE_TOLERANCE: f32 = 0.0254;
/// Contact friction of ragdoll bodies (flesh; Source's value comes from
/// the surface properties, not read here).
const FRICTION: f32 = 0.8;

/// Ragdoll settings (the console sets them).
#[derive(Resource, Clone, Debug)]
pub struct RagdollSettings {
    /// 0: the dead don't become ragdolls (`cl_ragdoll_physics_enable`).
    pub enabled: u8,
    /// Stationary this long, a ragdoll is put to sleep
    /// (`ragdoll_sleepaftertime`), s.
    pub sleep_after: f32,
}

impl Default for RagdollSettings {
    fn default() -> Self {
        Self {
            enabled: 1,
            sleep_after: 5.0,
        }
    }
}

/// One tick's skeleton pose: bones in the skeleton's space (parents
/// applied) and where the skeleton's space sits in the world.
#[derive(Clone, Debug)]
pub struct PoseFrame {
    pub time: f64,
    /// Skeleton space to world (the model's `root` at the feet, turned by
    /// the body's yaw).
    pub root: Transform,
    pub bones: Vec<(Quat, Vec3)>,
}

impl PoseFrame {
    /// Bone `b` in the world: rotation and position.
    pub fn world(&self, b: usize) -> (Quat, Vec3) {
        let (q, p) = self.bones[b];
        (self.root.rotation * q, self.root.transform_point(p))
    }
}

/// A character's recent skeleton poses, oldest first (alive only).
#[derive(Component, Clone, Debug, Default)]
pub struct SkeletonPose {
    pub frames: VecDeque<PoseFrame>,
}

/// On a character whose body is a ragdoll now.
#[derive(Component, Clone, Copy, Debug)]
pub struct Ragdolled;

/// A dead character's ragdoll.
#[derive(Component, Clone, Debug)]
pub struct Ragdoll {
    pub owner: Entity,
    /// Index in `CharacterModels`.
    pub model: usize,
    /// Rigid bodies, as `MapRagdoll::bodies`.
    pub bodies: Vec<Entity>,
    pub joints: Vec<Entity>,
    /// The owner's drawn body, once taken over.
    pub visual: Option<Entity>,
    /// Each body's joint parent body, and the child's anchor in the
    /// parent bone's frame (skeleton units).
    pub parents: Vec<Option<(usize, Vec3)>>,
    /// Settling (spec 6.1): body 0's position at the last check, and when
    /// it last moved.
    last_root: Vec3,
    last_moved: f64,
}

/// A rigid body of a ragdoll: which one and which of its bodies.
#[derive(Component, Clone, Copy, Debug)]
pub struct RagdollBody {
    pub ragdoll: Entity,
    pub index: usize,
}

pub(super) fn plugin(app: &mut App) {
    app.init_resource::<RagdollSettings>()
        .add_plugins(JointGraphPlugin::<RagdollJoint>::default())
        .add_systems(
            PhysicsSchedule,
            prepare_xpbd_joint::<RagdollJoint>
                .in_set(SolverSystems::PrepareJoints)
                .ambiguous_with_all(),
        )
        .add_systems(
            SubstepSchedule,
            solve_xpbd_joint::<RagdollJoint>.in_set(XpbdSolverSystems::SolveUserConstraints),
        )
        .add_systems(
            FixedUpdate,
            (remove_ragdolls, spawn_ragdolls, settle_ragdolls)
                .chain()
                .after(crate::core::apply_damage),
        )
        .add_systems(
            Update,
            (adopt_bodies, pose_ragdolls)
                .chain()
                .after(super::DriveAnimation)
                .run_if(resource_exists::<super::CharacterBodies>),
        );
}

/// Bind-pose globals of a skeleton (skeleton space).
fn bind_globals(m: &MapCharacterModel) -> Vec<(Quat, Vec3)> {
    globals(m, &m.bones.iter().map(|b| (b.rotation, b.position)).collect::<Vec<_>>())
}

/// Parent-relative bone transforms to skeleton space.
fn globals(m: &MapCharacterModel, local: &[(Quat, Vec3)]) -> Vec<(Quat, Vec3)> {
    let mut out: Vec<(Quat, Vec3)> = Vec::with_capacity(local.len());
    for (b, (q, p)) in m.bones.iter().zip(local) {
        out.push(match b.parent.and_then(|i| out.get(i)) {
            Some((pq, pp)) => (*pq * *q, *pp + *pq * *p),
            None => (*q, *p),
        });
    }
    out
}

/// Keep each living character's skeleton pose for this tick (its animator's,
/// else the bind pose), placed at its feet and turned by its body's yaw.
#[allow(clippy::type_complexity)]
pub(super) fn record_poses(
    time: Res<Time>,
    models: Option<Res<CharacterModels>>,
    mut characters: Query<(
        Entity,
        &BodyModel,
        &Transform,
        Option<&ColliderAabb>,
        &Intent,
        Option<&anim::Animator>,
        Option<&Health>,
        Option<&mut SkeletonPose>,
    )>,
    mut commands: Commands,
) {
    let Some(models) = models else { return };
    let now = time.elapsed_secs_f64();
    for (e, model, t, aabb, intent, animator, health, pose) in &mut characters {
        if health.is_some_and(|h| h.current <= 0.0) {
            continue;
        }
        let Some(m) = models.0.get(model.0) else { continue };
        let local = match animator.filter(|a| a.main.is_some()) {
            Some(a) => a.pose(now),
            None => m.bones.iter().map(|b| (b.rotation, b.position)).collect(),
        };
        let half = aabb.map_or(0.0, |b| (b.max.y - b.min.y) / 2.0);
        let yaw = animator.and_then(|a| a.yaw).unwrap_or(intent.yaw);
        let feet = Transform::from_translation(t.translation - Vec3::Y * half).with_rotation(Quat::from_rotation_y(yaw));
        let frame = PoseFrame {
            time: now,
            root: feet * m.root,
            bones: globals(m, &local),
        };
        match pose {
            Some(mut pose) => {
                pose.frames.push_back(frame);
                while pose.frames.len() > 2 && pose.frames.front().is_some_and(|f| f.time < now - HISTORY) {
                    pose.frames.pop_front();
                }
            }
            None => {
                commands.entity(e).insert(SkeletonPose {
                    frames: VecDeque::from([frame]),
                });
            }
        }
    }
}

/// Which ragdoll body a hit pushes (spec 1.5, 2.5): the hitbox of the hit's
/// group nearest the hit point, then its bone or the nearest ancestor with
/// a body. None: no hitbox of that group (no push).
fn hit_body(m: &MapCharacterModel, ragdoll: &MapRagdoll, frame: &PoseFrame, d: &crate::core::Damage) -> Option<usize> {
    let bone = m
        .boxes
        .iter()
        .filter(|b| b.group == d.hitgroup && b.bone < frame.bones.len())
        .map(|b| {
            let (q, p) = frame.world(b.bone);
            let centre = p + q * (b.center * frame.root.scale);
            (b.bone, centre.distance_squared(d.point))
        })
        .min_by(|a, b| a.1.total_cmp(&b.1))?
        .0;
    let mut at = Some(bone);
    while let Some(b) = at {
        if let Some(i) = ragdoll.bodies.iter().position(|r| r.bone == b) {
            return Some(i);
        }
        at = m.bones.get(b).and_then(|x| x.parent);
    }
    None
}

/// The rotation vector (axis × angle, radians) taking `from` to `to`.
fn rotation_between(from: Quat, to: Quat) -> Vec3 {
    let mut d = to * from.inverse();
    if d.w < 0.0 {
        d = -d;
    }
    let (axis, angle) = d.to_axis_angle();
    if angle.is_finite() { axis * angle } else { Vec3::ZERO }
}

/// Turn each death into a ragdoll (spec 2.4): bodies at the newest pose,
/// joints with the bind-pose frames, the killing hit's impulse, then bone
/// velocities from the pose `BONE_DT` earlier.
fn spawn_ragdolls(
    mut died: MessageReader<Died>,
    settings: Res<RagdollSettings>,
    models: Option<Res<CharacterModels>>,
    owners: Query<(&BodyModel, &SkeletonPose), Without<Ragdolled>>,
    time: Res<Time>,
    mut commands: Commands,
) {
    let Some(models) = models else {
        died.clear();
        return;
    };
    for d in died.read() {
        if settings.enabled == 0 {
            continue;
        }
        let Ok((model, pose)) = owners.get(d.entity) else { continue };
        let Some(m) = models.0.get(model.0) else { continue };
        let Some(ragdoll) = &m.ragdoll else { continue };
        let Some(b1) = pose.frames.back() else { continue };
        let b0 = pose
            .frames
            .iter()
            .rev()
            .find(|f| f.time <= b1.time - BONE_DT + 1e-6)
            .or(pose.frames.front())
            .unwrap_or(b1);
        let dt = (b1.time - b0.time) as f32;
        let entity = spawn_ragdoll(&mut commands, d.entity, model.0, m, ragdoll, (b0, b1, dt), &d.damage, time.elapsed_secs_f64());
        commands.entity(d.entity).insert(Ragdolled);
        info!("ragdoll {entity} for {}", d.entity);
    }
}

/// Spawn one ragdoll; returns its entity.
#[allow(clippy::too_many_arguments)]
pub fn spawn_ragdoll(
    commands: &mut Commands,
    owner: Entity,
    model: usize,
    m: &MapCharacterModel,
    ragdoll: &MapRagdoll,
    (b0, b1, dt): (&PoseFrame, &PoseFrame, f32),
    hit: &crate::core::Damage,
    now: f64,
) -> Entity {
    let scale = m.root.scale.x;
    let ragdoll_entity = commands.spawn((Name::new("Ragdoll"), MapPart, Transform::default())).id();
    let k = hit_body(m, ragdoll, b1, hit);
    let force = match k {
        Some(_) => hit.dir.normalize_or_zero() * DEATH_IMPULSE,
        None => Vec3::ZERO,
    };
    let total: f32 = ragdoll.bodies.iter().map(|b| b.mass).sum::<f32>().max(1.0);
    let shared_point = k.and_then(|k| ragdoll.bodies.get(k)).map(|b| b1.world(b.bone).1);
    let mut bodies = Vec::with_capacity(ragdoll.bodies.len());
    for (i, body) in ragdoll.bodies.iter().enumerate() {
        let (rot, pos) = b1.world(body.bone);
        let hulls: Vec<_> = body
            .pieces
            .iter()
            .filter_map(|p| Collider::convex_hull(p.iter().map(|v| *v * scale).collect()))
            .map(|c| (Vec3::ZERO, Quat::IDENTITY, c))
            .collect();
        let collider = if hulls.is_empty() {
            Collider::sphere(0.05)
        } else {
            Collider::compound(hulls)
        };
        let unit = collider.mass_properties(1.0);
        let density = if unit.mass > 0.0 { body.mass / unit.mass } else { 1.0 };
        let principal = unit.principal_angular_inertia * density * body.inertia.max(0.01);
        let frame = rot * unit.local_inertial_frame;
        let centre = pos + rot * unit.center_of_mass;
        // Bone velocities (2.6), then the death impulse (2.5): the hit
        // body takes all of it, every other body its mass share at the
        // hit body's origin.
        let (mut v, mut w) = if dt > 0.0 {
            let (r0, p0) = b0.world(body.bone);
            ((pos - p0) / dt, rotation_between(r0, rot) / dt)
        } else {
            (Vec3::ZERO, Vec3::ZERO)
        };
        if Some(i) == k {
            v += force / body.mass;
        } else if let Some(at) = shared_point {
            let share = force * (body.mass / total);
            v += share / body.mass;
            let torque = (at - centre).cross(share);
            let local = frame.inverse() * torque;
            w += frame * (local / principal.max(Vec3::splat(1e-6)));
        }
        bodies.push(
            commands
                .spawn((
                    Name::new(format!("Ragdoll body {i}")),
                    RagdollBody {
                        ragdoll: ragdoll_entity,
                        index: i,
                    },
                    MapPart,
                    Transform::from_translation(pos).with_rotation(rot),
                    RigidBody::Dynamic,
                    collider,
                    Mass(body.mass),
                    AngularInertia::new_with_local_frame(principal, unit.local_inertial_frame),
                    CenterOfMass(unit.center_of_mass),
                    LinearVelocity(v),
                    AngularVelocity(w),
                    LinearDamping(body.damping),
                    AngularDamping(body.rotdamping),
                    (
                        MaxLinearSpeed(MAX_LINEAR_SPEED),
                        MaxAngularSpeed(MAX_ANGULAR_SPEED),
                        Friction::new(FRICTION),
                        Restitution::new(0.0),
                        CollisionLayers::new(RAGDOLL_LAYER, LayerMask::DEFAULT),
                        TransformInterpolation,
                    ),
                ))
                .id(),
        );
    }
    // Joints: the child's bind-pose place and orientation in the parent's
    // bone frame (1.2).
    let bind = bind_globals(m);
    let mut parents = vec![None; ragdoll.bodies.len()];
    let mut joints = Vec::with_capacity(ragdoll.joints.len());
    for j in &ragdoll.joints {
        let (Some(pb), Some(cb)) = (ragdoll.bodies.get(j.parent), ragdoll.bodies.get(j.child)) else {
            continue;
        };
        let (pq, pp) = bind[pb.bone];
        let (cq, cp) = bind[cb.bone];
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
                    JointCollisionDisabled,
                ))
                .id(),
        );
    }
    let root = bodies.first().map_or(Vec3::ZERO, |_| b1.world(ragdoll.bodies[0].bone).1);
    commands.entity(ragdoll_entity).insert(Ragdoll {
        owner,
        model,
        bodies,
        joints,
        visual: None,
        parents,
        last_root: root,
        last_moved: now,
    });
    ragdoll_entity
}

/// Ragdolls go when their owner is alive again or gone; their drawn body
/// goes with them and the owner gets a fresh one.
fn remove_ragdolls(
    ragdolls: Query<(Entity, &Ragdoll)>,
    owners: Query<Option<&Health>>,
    mut commands: Commands,
) {
    for (e, r) in &ragdolls {
        let alive = match owners.get(r.owner) {
            Ok(h) => h.is_none_or(|h| h.current > 0.0),
            Err(_) => true,
        };
        if !alive {
            continue;
        }
        despawn_ragdoll(&mut commands, e, r);
    }
}

fn despawn_ragdoll(commands: &mut Commands, e: Entity, r: &Ragdoll) {
    for x in r.joints.iter().chain(&r.bodies).chain(&r.visual) {
        if let Ok(mut x) = commands.get_entity(*x) {
            x.despawn();
        }
    }
    commands.entity(e).despawn();
    if let Ok(mut owner) = commands.get_entity(r.owner) {
        owner.remove::<Ragdolled>();
    }
}

/// Forced settling (spec 6.1): a ragdoll whose root body moved at most
/// 1 in per axis each tick for `sleep_after` seconds is stopped and put
/// to sleep.
fn settle_ragdolls(
    mut ragdolls: Query<&mut Ragdoll>,
    mut bodies: Query<(&Position, &mut LinearVelocity, &mut AngularVelocity, Has<Sleeping>), With<RagdollBody>>,
    settings: Res<RagdollSettings>,
    time: Res<Time>,
    mut commands: Commands,
) {
    let now = time.elapsed_secs_f64();
    for mut r in &mut ragdolls {
        let Some(root) = r.bodies.first().and_then(|b| bodies.get(*b).ok()) else {
            continue;
        };
        let (at, asleep) = (root.0.0, root.3);
        if (at - r.last_root).abs().max_element() > SETTLE_TOLERANCE {
            r.last_moved = now;
        } else if !asleep && now - r.last_moved >= settings.sleep_after as f64 {
            for b in &r.bodies {
                if let Ok((_, mut v, mut w, _)) = bodies.get_mut(*b) {
                    v.0 = Vec3::ZERO;
                    w.0 = Vec3::ZERO;
                }
                commands.queue(SleepBody(*b));
            }
        }
        r.last_root = at;
    }
}

/// Take the owner's drawn body over: out of the character, visible (also
/// the local player's), without its held weapon, never culled (its mesh
/// bounds are the standing pose's).
#[allow(clippy::type_complexity)]
fn adopt_bodies(
    mut ragdolls: Query<&mut Ragdoll>,
    owners: Query<&Children>,
    mut bodies: Query<(&mut CharacterBody, Option<&Children>)>,
    meshes: Query<(), With<Mesh3d>>,
    mut commands: Commands,
) {
    for mut r in &mut ragdolls {
        if r.visual.is_some() {
            continue;
        }
        let Ok(children) = owners.get(r.owner) else { continue };
        let Some(body) = children.iter().find(|c| bodies.contains(*c)) else {
            continue;
        };
        let (mut b, parts) = bodies.get_mut(body).unwrap();
        if let Some((held, _)) = b.held.take() {
            commands.entity(held).despawn();
        }
        for part in parts.into_iter().flatten() {
            if meshes.contains(*part) {
                commands.entity(*part).insert(bevy::camera::visibility::NoFrustumCulling);
            }
        }
        commands
            .entity(body)
            .remove::<ChildOf>()
            .insert((Transform::IDENTITY, Visibility::Inherited, MapPart));
        r.visual = Some(body);
    }
}

/// Pose a ragdoll's drawn skeleton from its bodies (spec 5): a simulated
/// bone takes its body's rotation, and its position from the parent
/// body's bone and the joint anchor (the root body: its own); the other
/// bones keep their bind pose under their parent.
#[allow(clippy::type_complexity)]
fn pose_ragdolls(
    ragdolls: Query<&Ragdoll>,
    models: Option<Res<CharacterModels>>,
    drawn: Query<&CharacterBody>,
    physics: Query<&Transform, (With<RagdollBody>, Without<BodyJoint>)>,
    mut joints: Query<&mut Transform, With<BodyJoint>>,
) {
    let Some(models) = models else { return };
    for r in &ragdolls {
        let (Some(visual), Some(m)) = (r.visual, models.0.get(r.model)) else {
            continue;
        };
        let (Ok(drawn), Some(ragdoll)) = (drawn.get(visual), m.ragdoll.as_ref()) else {
            continue;
        };
        let to_skeleton = m.root.compute_affine().inverse();
        let inv_root = m.root.rotation.inverse();
        let mut simulated: Vec<Option<usize>> = vec![None; m.bones.len()];
        for (i, b) in ragdoll.bodies.iter().enumerate() {
            if let Some(s) = simulated.get_mut(b.bone) {
                *s = Some(i);
            }
        }
        let mut global: Vec<(Quat, Vec3)> = Vec::with_capacity(m.bones.len());
        for (bone, b) in m.bones.iter().enumerate() {
            let parent = b.parent.and_then(|p| global.get(p).copied());
            let g = match simulated[bone].and_then(|i| Some((i, physics.get(*r.bodies.get(i)?).ok()?))) {
                Some((i, t)) => {
                    let q = inv_root * t.rotation;
                    let p = match r.parents[i] {
                        Some((pi, anchor)) => {
                            let (pq, pp) = global[ragdoll.bodies[pi].bone];
                            pp + pq * anchor
                        }
                        None => to_skeleton.transform_point3(t.translation),
                    };
                    (q, p)
                }
                None => match parent {
                    Some((pq, pp)) => (pq * b.rotation, pp + pq * b.position),
                    None => (b.rotation, b.position),
                },
            };
            global.push(g);
        }
        for (bone, b) in m.bones.iter().enumerate() {
            let (q, p) = global[bone];
            let local = match b.parent.map(|i| global[i]) {
                Some((pq, pp)) => (pq.inverse() * q, pq.inverse() * (p - pp)),
                None => (q, p),
            };
            if let Some(mut t) = drawn.joints.get(bone).and_then(|j| joints.get_mut(*j).ok()) {
                t.rotation = local.0;
                t.translation = local.1;
            }
        }
    }
}

/// A ragdoll joint (avian custom XPBD constraint): the child body's origin
/// stays at `anchor` in the parent body (ball and socket), and the child's
/// rotation relative to `bind` in the parent's frame stays inside `limits`
/// as X-then-Y-then-Z Euler angles about the child's axes.
#[derive(Component, Clone, Debug)]
#[require(RagdollJointSolverData)]
pub struct RagdollJoint {
    pub parent: Entity,
    pub child: Entity,
    /// In the parent body's frame, m.
    pub anchor: Vec3,
    /// The child's bind orientation in the parent's frame.
    pub bind: Quat,
    pub limits: [(f32, f32); 3],
}

#[derive(Component, Clone, Debug, Default)]
pub struct RagdollJointSolverData {
    point: PointConstraintShared,
    rotation1: Quat,
    rotation2: Quat,
}

impl XpbdConstraintSolverData for RagdollJointSolverData {
    fn clear_lagrange_multipliers(&mut self) {
        self.point.clear_lagrange_multipliers();
    }

    fn total_position_lagrange(&self) -> Vec3 {
        self.point.total_position_lagrange()
    }
}

impl bevy::ecs::entity::MapEntities for RagdollJoint {
    fn map_entities<M: bevy::ecs::entity::EntityMapper>(&mut self, mapper: &mut M) {
        self.parent = mapper.get_mapped(self.parent);
        self.child = mapper.get_mapped(self.child);
    }
}

impl EntityConstraint<2> for RagdollJoint {
    fn entities(&self) -> [Entity; 2] {
        [self.parent, self.child]
    }
}

impl RagdollJoint {
    /// Where the child's relative rotation should be, clamped into the
    /// limits; None when it is inside them.
    pub fn clamp(&self, parent: Quat, child: Quat) -> Option<Quat> {
        let frame = parent * self.bind;
        let relative = frame.inverse() * child;
        let (x, y, z) = relative.to_euler(EulerRot::XYZ);
        let [(x0, x1), (y0, y1), (z0, z1)] = self.limits;
        let clamped = (x.clamp(x0, x1), y.clamp(y0, y1), z.clamp(z0, z1));
        if (clamped.0 - x).abs() + (clamped.1 - y).abs() + (clamped.2 - z).abs() < 1e-5 {
            return None;
        }
        Some(frame * Quat::from_euler(EulerRot::XYZ, clamped.0, clamped.1, clamped.2))
    }
}

impl XpbdConstraint<2> for RagdollJoint {
    type SolverData = RagdollJointSolverData;

    fn prepare(&mut self, bodies: [&RigidBodyQueryReadOnlyItem; 2], data: &mut RagdollJointSolverData) {
        data.point.prepare(bodies, self.anchor, Vec3::ZERO);
        data.rotation1 = bodies[0].rotation.0;
        data.rotation2 = bodies[1].rotation.0;
    }

    fn solve(
        &mut self,
        bodies: [&mut SolverBody; 2],
        inertias: [&SolverBodyInertia; 2],
        data: &mut RagdollJointSolverData,
        dt: f32,
    ) {
        let [body1, body2] = bodies;
        let [inertia1, inertia2] = inertias;
        data.point.solve([body1, body2], [inertia1, inertia2], 0.0, dt);
        let parent = body1.delta_rotation.0 * data.rotation1;
        let child = body2.delta_rotation.0 * data.rotation2;
        let Some(target) = self.clamp(parent, child) else { return };
        // Turning body 2 by `d` reaches the target; the solver turns body 1
        // along its argument and body 2 against it.
        let d = rotation_between(child, target);
        self.align_orientation(
            body1,
            body2,
            inertia1.effective_inv_angular_inertia(),
            inertia2.effective_inv_angular_inertia(),
            -d,
            0.0,
            0.0,
            dt,
        );
    }
}

impl PositionConstraint for RagdollJoint {}

impl AngularConstraint for RagdollJoint {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn joint_limits_clamp_per_axis() {
        let j = RagdollJoint {
            parent: Entity::PLACEHOLDER,
            child: Entity::PLACEHOLDER,
            anchor: Vec3::ZERO,
            bind: Quat::IDENTITY,
            limits: [(0.0, 0.0), (0.0, 0.0), (-0.2, 2.0)],
        };
        // A knee bent 1 rad about Z is fine.
        assert!(j.clamp(Quat::IDENTITY, Quat::from_rotation_z(1.0)).is_none());
        // Bent back too far: clamped to -0.2.
        let t = j.clamp(Quat::IDENTITY, Quat::from_rotation_z(-1.0)).unwrap();
        assert!(t.angle_between(Quat::from_rotation_z(-0.2)) < 1e-4);
        // A twist about X is removed.
        let t = j.clamp(Quat::IDENTITY, Quat::from_rotation_x(0.5)).unwrap();
        assert!(t.angle_between(Quat::IDENTITY) < 1e-4);
    }

    #[test]
    fn rotation_vector_between() {
        let a = Quat::from_rotation_y(0.3);
        let b = Quat::from_rotation_y(0.8);
        assert!((rotation_between(a, b) - Vec3::Y * 0.5).length() < 1e-5);
    }
}
