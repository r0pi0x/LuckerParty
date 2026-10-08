//! Players and `prop_physics` props (specs/cs_source/physics_props.md 4.1):
//! each Source player has a physics shadow, an invisible 85 kg box body
//! (`map::prop_physics::PhysicsShadow`) driven to the player every tick.
//! The movement treats such props as solid; the shadow is what pushes
//! them, so a light prop is shoved at the speed the mass ratio allows
//! and a heavy one (over 350 kg) is a wall the shadow never touches.
//! Props resting on or against the player rest on the shadow (the
//! player's own kinematic collider touches only the heavy ones, so it
//! can't launch light ones). After the physics step the
//! player is moved to where the shadow got to while pushing or standing
//! on a prop (reconciliation). Multiplayer props are pushaway.rs's.

use avian3d::prelude::*;
use bevy::{ecs::system::SystemParam, prelude::*};

use super::movement::{SourceMovement, SourceMovementConfig, to_engine, to_source};
use crate::{
    core::{Health, Intent, PropSurface, SHADOW_LAYER, Velocity},
    map::{
        PhysicsProp, PushAway,
        prop_physics::{PhysicsShadow, SHADOW_MASS, SHADOW_PUSH_MASS, Shadowed, StartAsleep},
    },
};

const METERS_PER_UNIT: f32 = 0.0254;
const HALF_WIDTH: f32 = 16.0;
/// The movement's transform sits this far above the feet (units).
const ORIGIN_ABOVE_FEET: f32 = 36.0;
/// Reconciliation thresholds (spec 4.1 step 7): position (units) and
/// velocity (units/s) errors, and the stricter pair on a rideable body.
const MAX_DIST_ERROR: f32 = 2.0;
const MAX_VEL_ERROR: f32 = 10.0;
const RIDE_DIST_ERROR: f32 = 1.0;
const RIDE_VEL_ERROR: f32 = 5.0;
/// A body heavier than this (kg) is ridden: twice the shadow's mass.
pub const RIDEABLE_MASS: f32 = 2.0 * SHADOW_MASS;
/// A rise in one tick over this is a step up (units; spec 4.1 step 4).
const STEP_UP: f32 = 0.1;
/// Farther than this from its target (units), the shadow is put there
/// instead of driven (a teleport, a respawn, a big step).
const TELEPORT: f32 = 24.0;
/// Reconciliation stops this far short of what blocks the player (units).
const BACKOFF: f32 = 0.031_25;

pub struct ShadowPlugin;

impl Plugin for ShadowPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(FixedUpdate, attach.before(crate::core::SimSet::Movement))
            .add_systems(
                FixedPostUpdate,
                (
                    drive.before(PhysicsSystems::First),
                    reconcile.after(PhysicsSystems::Writeback),
                ),
            );
    }
}

/// What this tick's movement did with physics props: the shadow's input.
#[derive(Component, Clone, Copy, Debug, Default, PartialEq)]
pub struct PhysicsTouch {
    /// Ran into a movable `prop_physics` it can push (other than the one
    /// stood on).
    pub touched: bool,
    /// The movable `prop_physics` stood on, and its mass (kg).
    pub standing_on: Option<(Entity, f32)>,
    /// Feet before and after the move, wish velocity (Source units).
    pub prev_feet: Vec3,
    pub feet: Vec3,
    pub wish: Vec3,
    /// The move's time step (s).
    pub dt: f32,
}

/// Which physics props the movement's hits are.
#[derive(SystemParam)]
pub struct ShadowProps<'w, 's> {
    of: Query<'w, 's, &'static ColliderOf>,
    props: Query<'w, 's, (&'static RigidBody, &'static PhysicsProp, Has<StartAsleep>)>,
}

impl ShadowProps<'_, '_> {
    /// The movable `prop_physics` body collider `c` belongs to, and its
    /// mass: simulated, or asleep since spawning (touching wakes it).
    fn movable(&self, c: Entity) -> Option<(Entity, f32)> {
        let body = self.of.get(c).map_or(c, |o| o.body);
        let (rb, p, asleep) = self.props.get(body).ok()?;
        (p.push == PushAway::Collide && (rb.is_dynamic() || asleep)).then_some((body, p.mass))
    }

    /// The shadow's input from one move: colliders run into, the one
    /// stood on, feet before and after, wish velocity (units/s).
    pub fn touch(
        &self,
        touched: &[Entity],
        ground: Option<Entity>,
        prev_feet: Vec3,
        feet: Vec3,
        wish: Vec3,
        dt: f32,
    ) -> PhysicsTouch {
        let standing_on = ground.and_then(|c| self.movable(c));
        let touched = touched
            .iter()
            .filter_map(|c| self.movable(*c))
            .any(|(b, m)| standing_on.is_none_or(|s| s.0 != b) && m <= SHADOW_PUSH_MASS);
        PhysicsTouch {
            touched,
            standing_on,
            prev_feet,
            feet,
            wish,
            dt,
        }
    }
}

/// Where the shadow is sent (feet, units; spec 4.1 step 4): half a step
/// ahead of where the move stopped while pushing a prop on the ground,
/// else the player's feet.
pub fn target(t: &PhysicsTouch, on_ground: bool) -> Vec3 {
    let stepped = t.feet.z - t.prev_feet.z > STEP_UP;
    if t.touched && on_ground && !stepped && t.standing_on.is_none() {
        let dt = if t.dt <= 0.0 || t.dt > 0.1 { 0.1 } else { t.dt };
        0.5 * t.feet + 0.5 * (t.prev_feet + dt * t.wish)
    } else {
        t.feet
    }
}

/// The player after the physics step (spec 4.1 step 7), Source units:
/// given its feet and velocity, the shadow's feet and velocity, and the
/// move's touch, the velocity to take and where to go (None: stay).
/// `airborne` halves the vertical error when not touching.
pub fn reconciled(
    feet: Vec3,
    v: Vec3,
    shadow_feet: Vec3,
    shadow_v: Vec3,
    touch: &PhysicsTouch,
    airborne: bool,
) -> (Vec3, Option<Vec3>) {
    let standing = touch.standing_on.is_some();
    if !touch.touched && !standing {
        return (v, None);
    }
    let rideable = touch.standing_on.is_some_and(|(_, m)| m > RIDEABLE_MASS);
    let (max_dist, max_vel) = if rideable {
        (RIDE_DIST_ERROR, RIDE_VEL_ERROR)
    } else {
        (MAX_DIST_ERROR, MAX_VEL_ERROR)
    };
    let mut d = shadow_feet - feet;
    if !touch.touched && airborne {
        d.z *= 0.5;
    }
    let vel_error = (shadow_v - v).length() > max_vel;
    if !(d.length() > max_dist || vel_error || (standing && !touch.touched)) {
        return (v, None);
    }
    let mut out = v;
    if vel_error && !rideable {
        // The shadow's velocity less its part along the player's own
        // (that part clamped to the player's speed) adds to the player's.
        let speed = v.length();
        let dir = v.normalize_or_zero();
        let along = shadow_v.dot(dir).clamp(-speed, speed);
        out += shadow_v - dir * along;
    }
    (out, Some(shadow_feet))
}

fn hull_height(cfg: &SourceMovementConfig, ducked: bool) -> f32 {
    if ducked { cfg.duck_height } else { cfg.stand_height }
}

/// The shadow's box for a player (engine meters).
fn shadow_collider(height: f32) -> Collider {
    let w = 2.0 * HALF_WIDTH * METERS_PER_UNIT;
    Collider::cuboid(w, height * METERS_PER_UNIT, w)
}

/// The shadow's state the game keeps: the box height it has.
#[derive(Component, Clone, Copy, Debug)]
struct ShadowBox(f32);

/// Give every Source player a shadow; remove the shadows of players gone
/// or no longer moving the Source way.
fn attach(
    mut commands: Commands,
    new: Query<(Entity, &Transform), (With<SourceMovement>, Without<Shadowed>)>,
    gone: Query<(Entity, &Shadowed), Without<SourceMovement>>,
    shadows: Query<(Entity, &PhysicsShadow)>,
    owners: Query<(), With<Shadowed>>,
    cfg: Res<SourceMovementConfig>,
) {
    for (shadow, s) in &shadows {
        if !owners.contains(s.owner) {
            commands.entity(shadow).try_despawn();
        }
    }
    for (owner, s) in &gone {
        commands.entity(s.0).try_despawn();
        commands.entity(owner).remove::<(Shadowed, PhysicsTouch)>();
    }
    for (owner, t) in &new {
        let height = cfg.stand_height;
        let centre = t.translation + Vec3::Y * (height / 2.0 - ORIGIN_ABOVE_FEET) * METERS_PER_UNIT;
        let shadow = commands
            .spawn((
                Name::new("Physics shadow"),
                PhysicsShadow { owner, active: false },
                ShadowBox(height),
                Transform::from_translation(centre),
                RigidBody::Dynamic,
                shadow_collider(height),
                Mass(SHADOW_MASS),
                // Surfaceprop "player" (friction 0.5, elasticity 0.001);
                // no rotation, no gravity: its controller holds it up.
                (
                    Friction::new(0.5),
                    Restitution::new(0.001),
                    LockedAxes::ROTATION_LOCKED,
                    GravityScale(0.0),
                    SleepingDisabled,
                    PropSurface("player".into()),
                ),
                CollisionLayers::new(SHADOW_LAYER, LayerMask::DEFAULT),
                ActiveCollisionHooks::FILTER_PAIRS,
            ))
            .id();
        commands
            .entity(owner)
            .insert((Shadowed(shadow), PhysicsTouch::default()));
    }
}

/// Send each shadow to its target for this physics step: the velocity
/// that gets it there in one step (put there when far off, or when the
/// box changes with ducking). A dead player's shadow touches nothing.
#[allow(clippy::type_complexity)]
fn drive(
    owners: Query<(&Transform, &SourceMovement, &PhysicsTouch, &Shadowed, Option<&Health>)>,
    mut shadows: Query<
        (
            &mut PhysicsShadow,
            &mut ShadowBox,
            &mut Position,
            &mut Transform,
            &mut LinearVelocity,
        ),
        Without<Shadowed>,
    >,
    mut commands: Commands,
    cfg: Res<SourceMovementConfig>,
    time: Res<Time>,
) {
    let dt = time.delta_secs();
    for (t, me, touch, shadowed, health) in &owners {
        let Ok((mut s, mut size, mut pos, mut st, mut v)) = shadows.get_mut(shadowed.0) else {
            continue;
        };
        let alive = health.is_none_or(|h| h.current > 0.0);
        if s.active != alive {
            s.active = alive;
        }
        let feet = to_source(t.translation) - Vec3::Z * ORIGIN_ABOVE_FEET;
        // Moved since the movement (a teleport): no push this step.
        let target_feet = if touch.feet.distance_squared(feet) < 1.0 {
            target(touch, me.on_ground)
        } else {
            feet
        };
        let height = hull_height(&cfg, me.ducked);
        let centre = to_engine(target_feet + Vec3::Z * height / 2.0);
        let resized = size.0 != height;
        if resized {
            size.0 = height;
            commands.entity(shadowed.0).insert(shadow_collider(height));
        }
        if !alive || dt <= 0.0 {
            v.0 = Vec3::ZERO;
            continue;
        }
        if resized || pos.0.distance(centre) > TELEPORT * METERS_PER_UNIT {
            pos.0 = centre;
            st.translation = centre;
            v.0 = Vec3::ZERO;
        } else {
            v.0 = (centre - pos.0) / dt;
        }
    }
}

/// After the physics step: players pushing or standing on a prop follow
/// their shadow (spec 4.1 step 7), swept so they never end inside
/// anything.
#[allow(clippy::type_complexity)]
fn reconcile(
    mut owners: Query<(
        Entity,
        &mut Transform,
        &mut Velocity,
        &SourceMovement,
        &PhysicsTouch,
        &Shadowed,
    )>,
    shadows: Query<(&PhysicsShadow, &ShadowBox, &Position, &LinearVelocity), Without<Shadowed>>,
    characters: Query<Entity, With<Intent>>,
    props: Query<(Entity, &PhysicsProp)>,
    spatial: SpatialQuery,
    cfg: Res<SourceMovementConfig>,
) {
    for (owner, mut t, mut vel, me, touch, shadowed) in &mut owners {
        let Ok((s, size, pos, sv)) = shadows.get(shadowed.0) else {
            continue;
        };
        if !s.active || !(touch.touched || touch.standing_on.is_some()) {
            continue;
        }
        let height = hull_height(&cfg, me.ducked);
        if size.0 != height {
            continue;
        }
        let feet = to_source(t.translation) - Vec3::Z * ORIGIN_ABOVE_FEET;
        let shadow_feet = to_source(pos.0) - Vec3::Z * height / 2.0;
        let (v, to) = reconciled(
            feet,
            to_source(vel.0),
            shadow_feet,
            to_source(sv.0),
            touch,
            !me.on_ground,
        );
        let v = to_engine(v);
        if vel.0 != v {
            vel.0 = v;
        }
        let Some(to) = to else { continue };
        // Sweep the player's box there through what blocks players
        // (multiplayer props and characters don't).
        let from = t.translation;
        let delta = to_engine(to - feet);
        let Ok((dir, len)) = Dir3::new_and_length(delta) else {
            continue;
        };
        let excluded = characters.iter().chain(std::iter::once(owner)).chain(
            props
                .iter()
                .filter(|(_, p)| p.push != PushAway::Collide)
                .map(|(e, _)| e),
        );
        let filter = SpatialQueryFilter::from_excluded_entities(excluded).with_mask(crate::core::CHARACTER_FILTER);
        let w = 2.0 * HALF_WIDTH * METERS_PER_UNIT;
        let shape = Collider::cuboid(w, height * METERS_PER_UNIT, w);
        let centre = from + Vec3::Y * (height / 2.0 - ORIGIN_ABOVE_FEET) * METERS_PER_UNIT;
        let config = ShapeCastConfig {
            max_distance: len,
            ignore_origin_penetration: true,
            ..default()
        };
        let travel = match spatial.cast_shape(&shape, centre, Quat::IDENTITY, dir, &config, &filter) {
            Some(hit) => (hit.distance - BACKOFF * METERS_PER_UNIT).max(0.0),
            None => len,
        };
        if travel > 0.0 {
            t.translation = from + dir * travel;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn touch(touched: bool, standing_on: Option<f32>) -> PhysicsTouch {
        PhysicsTouch {
            touched,
            standing_on: standing_on.map(|m| (Entity::PLACEHOLDER, m)),
            prev_feet: Vec3::ZERO,
            feet: Vec3::new(1.0, 0.0, 0.0),
            wish: Vec3::new(250.0, 0.0, 0.0),
            dt: 0.015,
        }
    }

    /// Spec 4.1 step 4: pushing on the ground aims half a step past
    /// where the move stopped; otherwise the shadow follows the feet.
    #[test]
    fn target_presses_into_the_prop_while_pushing() {
        let t = touch(true, None);
        let at = target(&t, true);
        // 0.5 * 1 + 0.5 * (0 + 0.015 * 250) = 2.375
        assert!((at.x - 2.375).abs() < 1e-5, "{at}");
        assert_eq!(target(&t, false), t.feet, "airborne");
        assert_eq!(target(&touch(false, None), true), t.feet, "not touching");
        assert_eq!(target(&touch(true, Some(20.0)), true), t.feet, "standing on physics");
        let mut stepped = t;
        stepped.feet.z = 0.5;
        assert_eq!(target(&stepped, true), stepped.feet, "stepped up");
        let mut slow = t;
        slow.dt = 0.0;
        assert!((target(&slow, true).x - (0.5 + 0.5 * 25.0)).abs() < 1e-4, "dt 0 → 0.1");
    }

    /// Spec 4.1 step 7.
    #[test]
    fn reconciliation_follows_the_shadow_only_past_its_thresholds() {
        let feet = Vec3::ZERO;
        // Not touching, not standing: nothing.
        let (v, to) = reconciled(feet, Vec3::ZERO, Vec3::X * 5.0, Vec3::ZERO, &touch(false, None), false);
        assert_eq!((v, to), (Vec3::ZERO, None));
        // Touching, within 2 units and 10 u/s: nothing.
        let (v, to) = reconciled(
            feet,
            Vec3::ZERO,
            Vec3::X * 1.5,
            Vec3::X * 9.0,
            &touch(true, None),
            false,
        );
        assert_eq!((v, to), (Vec3::ZERO, None));
        // Pushing a crate: the shadow moved on at 100 u/s; the stopped
        // player takes its velocity and its place.
        let (v, to) = reconciled(
            feet,
            Vec3::ZERO,
            Vec3::X * 1.5,
            Vec3::X * 100.0,
            &touch(true, None),
            false,
        );
        assert_eq!(v, Vec3::X * 100.0);
        assert_eq!(to, Some(Vec3::X * 1.5));
        // The part along the player's own velocity is not added (clamped
        // to the player's speed).
        let (v, _) = reconciled(
            feet,
            Vec3::X * 50.0,
            Vec3::X * 3.0,
            Vec3::new(100.0, 20.0, 0.0),
            &touch(true, None),
            false,
        );
        assert!((v - Vec3::new(100.0, 20.0, 0.0)).length() < 1e-4, "{v}");
        // Standing on a light prop without touching: always follow it.
        let (v, to) = reconciled(
            feet,
            Vec3::ZERO,
            Vec3::X * 0.1,
            Vec3::ZERO,
            &touch(false, Some(30.0)),
            false,
        );
        assert_eq!((v, to), (Vec3::ZERO, Some(Vec3::X * 0.1)));
        // Riding a heavy body: no velocity correction, stricter thresholds.
        let (v, to) = reconciled(
            feet,
            Vec3::ZERO,
            Vec3::X * 1.5,
            Vec3::X * 100.0,
            &touch(false, Some(200.0)),
            false,
        );
        assert_eq!((v, to), (Vec3::ZERO, Some(Vec3::X * 1.5)));
        assert!(RIDEABLE_MASS == 170.0);
    }
}
