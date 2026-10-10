//! Controllers map logic attaches to physics bodies (specs/source/
//! physics_brushes.md 4-5): phys_thruster's constant push and turn,
//! phys_keepupright's turn back toward a goal up axis. Each physics step,
//! before the solver, they change the body's velocities; the logic says
//! which are on (`BodyControllers`). Engine space, SI units.

use avian3d::prelude::{
    AngularVelocity, ComputedAngularInertia, ComputedMass, LinearVelocity, PhysicsSystems, Rotation,
    Sleeping,
};
use bevy::prelude::*;

/// One controller on a body.
#[derive(Clone, Debug, PartialEq)]
pub struct BodyController {
    /// Who it is (the logic entity), so its turn-on values are kept.
    pub key: u64,
    pub body: Entity,
    pub kind: ControlKind,
}

#[derive(Clone, Debug, PartialEq)]
pub enum ControlKind {
    /// phys_thruster: `force` (kg·m/s², world, from the thruster's
    /// angles; with "Orient Locally" `force_local`, in the body's frame)
    /// applied at `offset` (the body's frame) from the body's origin,
    /// worked out once per turn-on (`serial`) and then scaled by `scale`.
    Thrust {
        force: Vec3,
        force_local: Vec3,
        offset: Vec3,
        /// "Orient Locally": the push turns with the body.
        local: bool,
        linear: bool,
        angular: bool,
        ignore_mass: bool,
        ignore_pos: bool,
        scale: f32,
        serial: u32,
    },
    /// phys_keepupright: turn the body's up (its entity +Z, engine local
    /// +Y) toward `goal` at most `limit` rad/s per step.
    Upright { goal: Vec3, limit: f32 },
    /// phys_motor: spin about `axis` (world, unit) at `speed` rad/s,
    /// reached over `spinup` seconds.
    Motor { axis: Vec3, speed: f32, spinup: f32 },
}

/// What a joint holds (`BodyJoint`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum JointKind {
    Fixed,
    Ball,
    Hinge {
        axis: Vec3,
    },
    Slide {
        axis: Vec3,
    },
    /// The bodies at most `max` (at least `min`) meters apart, from
    /// `other` on the first to the anchor on the second.
    Length {
        min: f32,
        max: f32,
        other: Vec3,
    },
}

/// A joint between two bodies (`body1` None: the world) at `anchor`
/// (world, engine space), from a map constraint entity; the logic layer
/// writes them (`BodyJoints`).
#[derive(Clone, Debug, PartialEq)]
pub struct BodyJoint {
    pub key: u64,
    pub kind: JointKind,
    pub body1: Option<Entity>,
    pub body2: Entity,
    pub anchor: Vec3,
    pub on: bool,
    /// The two bodies don't collide while it holds.
    pub no_collide: bool,
    /// It breaks when its force (N) or torque (N·m) passes these
    /// (physics_constraints.md 1.5; 0: never): `JointBroke`.
    pub force_limit: f32,
    pub torque_limit: f32,
}

/// A joint passed its break limit this step (`BodyJoint::key`): the logic
/// breaks the constraint (OnBreak, removed).
#[derive(Message, Clone, Copy, Debug, PartialEq, Eq)]
pub struct JointBroke(pub u64);

/// A made joint's key and break limits.
#[derive(Component, Clone, Copy, Debug)]
struct JointLimits {
    key: u64,
    force: f32,
    torque: f32,
}

/// Joints whose force or torque passed their limit this step go off and
/// are reported once (the logic removes them).
fn break_joints(
    joints: Query<(Entity, &JointLimits, &avian3d::prelude::JointForces), Without<avian3d::prelude::JointDisabled>>,
    mut broke: MessageWriter<JointBroke>,
    mut commands: Commands,
) {
    for (e, l, f) in &joints {
        let force = l.force > 0.0 && f.force().length() > l.force;
        let torque = l.torque > 0.0 && f.torque().length() > l.torque;
        if force || torque {
            commands.entity(e).insert(avian3d::prelude::JointDisabled);
            broke.write(JointBroke(l.key));
        }
    }
}

/// The joints that exist now.
#[derive(Resource, Clone, Debug, Default, PartialEq)]
pub struct BodyJoints(pub Vec<BodyJoint>);

/// A spinning motor's change of angular velocity for one step of `h`
/// seconds: the rate about `axis` moves toward `speed`, by at most
/// speed/spinup per second (at once with no spin-up).
pub fn motor_change(omega: Vec3, axis: Vec3, speed: f32, spinup: f32, h: f32) -> Vec3 {
    let now = omega.dot(axis);
    let step = if spinup > 0.0 {
        speed.abs() / spinup * h
    } else {
        f32::INFINITY
    };
    let next = now + (speed - now).clamp(-step, step);
    axis * (next - now)
}

/// The controllers on now (the logic layer writes it).
#[derive(Resource, Clone, Debug, Default, PartialEq)]
pub struct BodyControllers(pub Vec<BodyController>);

/// A thruster's acceleration and angular acceleration from its turn-on
/// (local or world frame).
#[derive(Clone, Copy, Debug)]
struct Started {
    serial: u32,
    linear: Vec3,
    angular: Vec3,
    local: bool,
}

pub struct ControllersPlugin;

impl Plugin for ControllersPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<BodyControllers>()
            .init_resource::<BodyJoints>()
            .add_message::<JointBroke>()
            .add_systems(
                FixedPostUpdate,
                (sync_joints, apply).chain().before(PhysicsSystems::StepSimulation),
            )
            .add_systems(FixedPostUpdate, break_joints.after(PhysicsSystems::Writeback));
    }
}

/// Velocity changes one thruster turn-on works out (spec 4): a = F/m
/// (or the force itself as an acceleration with "Ignore Mass"), α =
/// I⁻¹(r × F) about the body's centre (none with "Ignore Pos"); in the
/// body's frame when local.
pub fn thrust_rates(
    force: Vec3,
    r: Vec3,
    rotation: Quat,
    mass: f32,
    inverse_inertia_world: impl Fn(Vec3) -> Vec3,
    flags: (bool, bool, bool, bool, bool),
) -> (Vec3, Vec3) {
    let (local, linear, angular, ignore_mass, ignore_pos) = flags;
    let force = if ignore_mass { force * mass } else { force };
    let a = if linear && mass > 0.0 { force / mass } else { Vec3::ZERO };
    let r = if ignore_pos { Vec3::ZERO } else { r };
    let alpha = if angular { inverse_inertia_world(r.cross(force)) } else { Vec3::ZERO };
    if local {
        (rotation.inverse() * a, rotation.inverse() * alpha)
    } else {
        (a, alpha)
    }
}

/// phys_keepupright's change of angular velocity for one step of `h`
/// seconds (spec 5): in the body's frame, drive the rate about the
/// correcting axis to θ/h, capped at `limit`, damping the rate about it.
/// `omega` and the result are world rad/s.
pub fn upright_change(rotation: Quat, omega: Vec3, goal: Vec3, limit: f32, h: f32) -> Vec3 {
    let up = Vec3::Y;
    let g = (rotation.inverse() * goal).normalize_or_zero();
    let cross = up.cross(g);
    let len = cross.length();
    if len < 1e-6 {
        // Upright, or upside down (axis undefined): nothing.
        return Vec3::ZERO;
    }
    let k = cross / len;
    let theta = len.atan2(up.dot(g));
    let w = rotation.inverse() * omega;
    let mut want = k * (theta / h) - k * w.dot(k);
    let m = want.length();
    if m > limit && m > 0.0 {
        want *= limit / m;
    }
    rotation * want
}

#[allow(clippy::type_complexity)]
fn apply(
    controllers: Res<BodyControllers>,
    mut started: Local<std::collections::HashMap<u64, Started>>,
    mut bodies: Query<(
        &mut LinearVelocity,
        &mut AngularVelocity,
        &Rotation,
        Option<&ComputedMass>,
        Option<&ComputedAngularInertia>,
        Has<Sleeping>,
    )>,
    time: Res<Time<Fixed>>,
    mut commands: Commands,
) {
    let h = time.timestep().as_secs_f32();
    started.retain(|k, _| controllers.0.iter().any(|c| c.key == *k));
    for c in &controllers.0 {
        let Ok((mut v, mut w, rot, mass, inertia, sleeping)) = bodies.get_mut(c.body) else {
            continue;
        };
        let rotation = rot.0;
        let (dv, dw) = match &c.kind {
            ControlKind::Thrust {
                force,
                force_local,
                offset,
                local,
                linear,
                angular,
                ignore_mass,
                ignore_pos,
                scale,
                serial,
            } => {
                let s = match started.get(&c.key) {
                    Some(s) if s.serial == *serial => *s,
                    _ => {
                        let m = mass.map_or(1.0, |m| m.value());
                        let inv = |t: Vec3| match inertia {
                            Some(i) => rotation * (i.inverse() * (rotation.inverse() * t)),
                            None => Vec3::ZERO,
                        };
                        let f = if *local { rotation * *force_local } else { *force };
                        let (linear, angular) = thrust_rates(
                            f,
                            rotation * *offset,
                            rotation,
                            m,
                            inv,
                            (*local, *linear, *angular, *ignore_mass, *ignore_pos),
                        );
                        let s = Started {
                            serial: *serial,
                            linear,
                            angular,
                            local: *local,
                        };
                        started.insert(c.key, s);
                        // Turning on wakes the body (one asleep since it
                        // spawned too).
                        let body = c.body;
                        commands.queue(move |w: &mut World| super::prop_physics::wake(w, body));
                        s
                    }
                };
                let (a, alpha) = if s.local {
                    (rotation * s.linear, rotation * s.angular)
                } else {
                    (s.linear, s.angular)
                };
                (a * *scale * h, alpha * *scale * h)
            }
            ControlKind::Upright { goal, limit } => (Vec3::ZERO, upright_change(rotation, w.0, *goal, *limit, h)),
            ControlKind::Motor { axis, speed, spinup } => (Vec3::ZERO, motor_change(w.0, *axis, *speed, *spinup, h)),
        };
        if dv == Vec3::ZERO && dw == Vec3::ZERO {
            continue;
        }
        v.0 += dv;
        w.0 += dw;
        if sleeping {
            commands.entity(c.body).remove::<Sleeping>();
        }
    }
}

/// The joint entities made for `BodyJoints`: by key, the joint, the
/// static body standing in for the world, and what it was made from.
#[derive(Default)]
struct MadeJoints(std::collections::HashMap<u64, (Entity, Option<Entity>, BodyJoint)>);

/// Make, switch and remove joint entities to match `BodyJoints`. A joint
/// to the world holds its body to a static body at the anchor.
fn sync_joints(joints: Res<BodyJoints>, mut made: Local<MadeJoints>, mut commands: Commands) {
    use avian3d::prelude::{
        DistanceJoint, FixedJoint, JointCollisionDisabled, JointDisabled, PrismaticJoint, RevoluteJoint, RigidBody,
        SphericalJoint,
    };
    if !joints.is_changed() && joints.0.len() == made.0.len() {
        return;
    }
    let wanted: std::collections::HashSet<u64> = joints.0.iter().map(|j| j.key).collect();
    made.0.retain(|key, (joint, anchor, _)| {
        let keep = wanted.contains(key);
        if !keep {
            commands.entity(*joint).try_despawn();
            if let Some(a) = anchor {
                commands.entity(*a).try_despawn();
            }
        }
        keep
    });
    for j in &joints.0 {
        if let Some((joint, _, was)) = made.0.get_mut(&j.key) {
            if was.on != j.on {
                if j.on {
                    commands.entity(*joint).try_remove::<JointDisabled>();
                } else {
                    commands.entity(*joint).try_insert(JointDisabled);
                }
                was.on = j.on;
            }
            continue;
        }
        let world_body = j.body1.is_none().then(|| {
            commands
                .spawn((
                    Name::new("Joint anchor"),
                    super::MapPart,
                    RigidBody::Static,
                    Transform::from_translation(j.anchor),
                ))
                .id()
        });
        let Some(b1) = j.body1.or(world_body) else { continue };
        let b2 = j.body2;
        let basis = |from: Vec3, to: Vec3| Quat::from_rotation_arc(from, to.try_normalize().unwrap_or(from));
        let mut e = commands.spawn((Name::new("Joint"), super::MapPart));
        match j.kind {
            JointKind::Fixed => {
                e.insert(FixedJoint::new(b1, b2).with_anchor(j.anchor));
            }
            JointKind::Ball => {
                e.insert(SphericalJoint::new(b1, b2).with_anchor(j.anchor));
            }
            JointKind::Hinge { axis } => {
                e.insert(
                    RevoluteJoint::new(b1, b2)
                        .with_anchor(j.anchor)
                        .with_basis(basis(Vec3::Z, axis))
                        .with_hinge_axis(Vec3::Z),
                );
            }
            JointKind::Slide { axis } => {
                let mut p = PrismaticJoint::new(b1, b2)
                    .with_anchor(j.anchor)
                    .with_basis(basis(Vec3::X, axis));
                p.slider_axis = Vec3::X;
                e.insert(p);
            }
            JointKind::Length { min, max, other } => {
                let mut d = DistanceJoint::new(b1, b2).with_limits(min.min(max), max);
                d.anchor1 = avian3d::prelude::JointAnchor::FromGlobal(other);
                d.anchor2 = avian3d::prelude::JointAnchor::FromGlobal(j.anchor);
                e.insert(d);
            }
        }
        if j.no_collide {
            e.insert(JointCollisionDisabled);
        }
        if j.force_limit > 0.0 || j.torque_limit > 0.0 {
            e.insert((
                avian3d::prelude::JointForces::new(),
                JointLimits {
                    key: j.key,
                    force: j.force_limit,
                    torque: j.torque_limit,
                },
            ));
        }
        if !j.on {
            e.insert(JointDisabled);
        }
        let id = e.id();
        made.0.insert(j.key, (id, world_body, j.clone()));
    }
}

#[cfg(test)]
mod motor_tests {
    use super::*;

    #[test]
    fn motor_spins_up_over_its_time() {
        // 100 deg/s over 1 s: a tenth of the way in 0.1 s.
        let speed = 100f32.to_radians();
        let dw = motor_change(Vec3::ZERO, Vec3::Y, speed, 1.0, 0.1);
        assert!((dw.y - speed * 0.1).abs() < 1e-5);
        // At speed: nothing; no spin-up: at once.
        assert_eq!(motor_change(Vec3::Y * speed, Vec3::Y, speed, 1.0, 0.1), Vec3::ZERO);
        assert!((motor_change(Vec3::ZERO, Vec3::Y, speed, 0.0, 0.1).y - speed).abs() < 1e-5);
        // Other axes are left alone.
        assert_eq!(motor_change(Vec3::X, Vec3::Y, 0.0, 1.0, 0.1), Vec3::ZERO);
    }
}
