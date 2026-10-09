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
        app.init_resource::<BodyControllers>().add_systems(
            FixedPostUpdate,
            apply.before(PhysicsSystems::StepSimulation),
        );
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
