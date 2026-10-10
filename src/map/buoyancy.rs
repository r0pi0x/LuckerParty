//! Bodies in water float (Source's physics floats objects in water
//! brushes by their density against the water's: public physics docs;
//! the numbers below are ours, tech-debt). Each physics step, before the
//! solver: a dynamic body whose box reaches into a water volume gets the
//! water's lift for the part of its box below the surface (water density
//! × its volume × that part × g, upward, at the middle of the wet part,
//! so a tipped boat rights itself), and the water's drag. A body lighter
//! than the water it would displace floats at the depth that balances
//! (wood, boats); a heavier one sinks slowly. Engine space, SI units.

use avian3d::prelude::{
    AngularVelocity, ColliderAabb, ColliderMassProperties, ColliderOf, ComputedAngularInertia, ComputedCenterOfMass,
    ComputedMass, Gravity, LinearVelocity, PhysicsSystems, Position, RigidBody, Rotation, Sleeping,
};
use bevy::prelude::*;

use crate::core::MapWater;

/// Water's density, kg/m³.
pub const WATER_DENSITY: f32 = 1000.0;
/// Speed lost in water per second at full depth (linear, angular).
pub const WATER_DRAG: f32 = 3.0;
pub const WATER_ANGULAR_DRAG: f32 = 2.0;
/// Most lift, in g.
pub const MAX_LIFT_G: f32 = 2.0;

/// The fraction of a box (min, max) below a surface at height `surface`.
pub fn wet_fraction(min: Vec3, max: Vec3, surface: f32) -> f32 {
    let h = max.y - min.y;
    if h <= 1e-6 {
        return if min.y < surface { 1.0 } else { 0.0 };
    }
    ((surface - min.y) / h).clamp(0.0, 1.0)
}

/// Upward acceleration of a body of `mass` kg and `volume` m³ with
/// `wet` of it under water.
pub fn lift(mass: f32, volume: f32, wet: f32, g: f32) -> f32 {
    if mass <= 0.0 {
        return 0.0;
    }
    WATER_DENSITY * volume * wet * g / mass
}

#[allow(clippy::type_complexity)]
fn float_bodies(
    water: Option<Res<MapWater>>,
    gravity: Option<Res<Gravity>>,
    time: Res<Time<Fixed>>,
    colliders: Query<(&ColliderOf, &ColliderAabb, &ColliderMassProperties)>,
    mut bodies: Query<(
        &RigidBody,
        &mut LinearVelocity,
        &mut AngularVelocity,
        &Position,
        &Rotation,
        &ComputedMass,
        Option<&ComputedCenterOfMass>,
        Option<&ComputedAngularInertia>,
        Has<Sleeping>,
    )>,
    mut commands: Commands,
) {
    let Some(water) = water.filter(|w| !w.0.is_empty()) else {
        return;
    };
    let g = gravity.map_or(9.81, |g| g.0.length());
    let h = time.timestep().as_secs_f32();
    // Each body's box and volume, from its colliders.
    let mut boxes: std::collections::HashMap<Entity, (Vec3, Vec3, f32)> = std::collections::HashMap::new();
    for (of, aabb, props) in &colliders {
        let e = boxes
            .entry(of.body)
            .or_insert((Vec3::splat(f32::MAX), Vec3::splat(f32::MIN), 0.0));
        e.0 = e.0.min(aabb.min);
        e.1 = e.1.max(aabb.max);
        e.2 += props.mass;
    }
    for (body, (min, max, volume)) in boxes {
        let Ok((kind, mut v, mut w, pos, rot, mass, com, inertia, sleeping)) = bodies.get_mut(body) else {
            continue;
        };
        if !kind.is_dynamic() || volume <= 0.0 {
            continue;
        }
        // The deepest water volume it reaches into.
        let mut wet = 0.0f32;
        let mut surface = f32::MIN;
        for vol in &water.0 {
            let b = &vol.brush;
            if max.x < b.min.x || min.x > b.max.x || max.z < b.min.z || min.z > b.max.z || min.y > b.max.y {
                continue;
            }
            let f = wet_fraction(min, max, b.max.y);
            if f > wet {
                wet = f;
                surface = b.max.y;
            }
        }
        if wet <= 0.0 {
            continue;
        }
        let m = mass.value();
        // At most twice gravity: a very light body comes up briskly
        // without shooting out (where it floats is unchanged: there the
        // lift is one g).
        let up = lift(m, volume, wet, g).min(MAX_LIFT_G * g);
        v.0.y += up * h;
        // Lift acts at the middle of the wet part: righting turn.
        let centre = pos.0 + rot.0 * com.map_or(Vec3::ZERO, |c| c.0);
        let wet_centre = Vec3::new(centre.x, (min.y + surface.min(max.y)) * 0.5, centre.z);
        let arm = wet_centre - centre;
        let torque = arm.cross(Vec3::Y * up * m);
        if let Some(i) = inertia {
            w.0 += rot.0 * (i.inverse() * (rot.0.inverse() * torque)) * h;
        }
        v.0 *= 1.0 / (1.0 + WATER_DRAG * wet * h);
        w.0 *= 1.0 / (1.0 + WATER_ANGULAR_DRAG * wet * h);
        if sleeping {
            commands.entity(body).remove::<Sleeping>();
        }
    }
}

pub struct BuoyancyPlugin;

impl Plugin for BuoyancyPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(FixedPostUpdate, float_bodies.before(PhysicsSystems::StepSimulation));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn floats_where_lift_meets_weight() {
        // A 0.5 m³ box of 350 kg (density 700): balanced when 70 % is
        // under water.
        let g = 9.81;
        assert!((lift(350.0, 0.5, 0.7, g) - g).abs() < 1e-4);
        assert!(lift(350.0, 0.5, 1.0, g) > g);
        // Steel (7800) can't float.
        assert!(lift(3900.0, 0.5, 1.0, g) < g);
        assert_eq!(wet_fraction(Vec3::ZERO, Vec3::ONE, 0.25), 0.25);
        assert_eq!(wet_fraction(Vec3::ZERO, Vec3::ONE, 2.0), 1.0);
        assert_eq!(wet_fraction(Vec3::ZERO, Vec3::ONE, -1.0), 0.0);
    }
}
