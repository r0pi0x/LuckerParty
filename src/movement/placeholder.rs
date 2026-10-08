//! Stand-in walking movement so the foundation is playable before any game's
//! movement is specced. Deliberately not modeled on any game; replace with
//! spec-based implementations, don't tune this into one.

use avian3d::prelude::*;
use bevy::prelude::*;

use super::wish_dir;
use crate::{
    character::CAPSULE_HEIGHT,
    core::{Intent, MovementState, Predict, PredictedAppExt, SimClock, Velocity},
    slots::RegisterSlots,
};

pub const ID: &str = "mashup:placeholder_movement";

const WALK_SPEED: f32 = 5.0;
const SPRINT_SPEED: f32 = 7.5;
const CROUCH_SPEED: f32 = 2.5;
/// How quickly horizontal velocity approaches the wished velocity (1/s).
const GROUND_RESPONSE: f32 = 12.0;
const AIR_RESPONSE: f32 = 1.5;
const GRAVITY: f32 = 20.0;
const JUMP_SPEED: f32 = 6.5;
/// Steepest walkable ground: cos(45 degrees).
const MIN_GROUND_NORMAL_Y: f32 = 0.7;
const GROUND_PROBE: f32 = 0.08;
/// Upward speed above which we're airborne even if ground is near (m/s).
const MAX_GROUNDED_RISE: f32 = JUMP_SPEED * 0.5;
const STAND_EYE: f32 = 1.62 - CAPSULE_HEIGHT / 2.0;
const CROUCH_EYE: f32 = 1.0 - CAPSULE_HEIGHT / 2.0;

#[derive(Component, Default, Clone)]
pub struct PlaceholderMovement {
    jump_held: bool,
    /// Normal of the ground we stood on last tick.
    ground_normal: Option<Vec3>,
}

pub struct PlaceholderMovementPlugin;

impl Plugin for PlaceholderMovementPlugin {
    fn build(&self, app: &mut App) {
        app.register_movement::<PlaceholderMovement>(ID)
            .add_systems(Predict::Movement, step)
            .predicted::<PlaceholderMovement>();
    }
}

fn step(
    mut q: Query<(
        Entity,
        &Intent,
        &mut PlaceholderMovement,
        &mut Transform,
        &mut Velocity,
        &mut MovementState,
        &Collider,
    )>,
    move_and_slide: MoveAndSlide,
    clock: Res<SimClock>,
) {
    let dt = clock.dt();
    for (entity, intent, mut me, mut transform, mut vel, mut state, collider) in &mut q {
        // Characters walk through ragdolls and loose items.
        let filter = SpatialQueryFilter::from_excluded_entities([entity]).with_mask(crate::core::CHARACTER_FILTER);
        let config = MoveAndSlideConfig::default();

        // Horizontal: ease toward the wished velocity.
        let crouching = intent.crouch;
        let sprinting = intent.sprint && !crouching && intent.move_axis.y > 0.0 && state.on_ground;
        let speed = if crouching {
            CROUCH_SPEED
        } else if sprinting {
            SPRINT_SPEED
        } else {
            WALK_SPEED
        };
        let wish = wish_dir(intent.move_axis, intent.yaw_rotation()) * speed;
        let response = if state.on_ground { GROUND_RESPONSE } else { AIR_RESPONSE };
        let t = 1.0 - (-response * dt).exp();
        let horizontal = Vec3::new(vel.x, 0.0, vel.z).lerp(wish, t);

        // Jumping needs a fresh press.
        let jumped = intent.jump && !me.jump_held && state.on_ground;
        me.jump_held = intent.jump;

        // On the ground, run along it and ignore gravity; slopes keep the
        // horizontal speed. In the air, fall.
        let desired = match (me.ground_normal, jumped) {
            (Some(n), false) if n.y > 0.0 => {
                // Lift the horizontal velocity onto the ground plane.
                horizontal - Vec3::Y * (horizontal.dot(n) / n.y)
            }
            (_, true) => horizontal + Vec3::Y * JUMP_SPEED,
            (_, false) => horizontal + Vec3::Y * (vel.y - GRAVITY * dt),
        };

        let out = move_and_slide.move_and_slide(
            collider,
            transform.translation,
            transform.rotation,
            desired,
            clock.delta,
            &config,
            &filter,
            |_| MoveAndSlideHitResponse::Accept,
        );
        let mut position = out.position;
        let velocity = out.projected_velocity;

        // Ground check, snapping down so we stick to ramps and small drops.
        let ground = move_and_slide
            .cast_move(
                collider,
                position,
                transform.rotation,
                Vec3::NEG_Y * GROUND_PROBE,
                config.skin_width,
                &filter,
            )
            .filter(|hit| hit.normal1.y >= MIN_GROUND_NORMAL_Y);
        // Walking up a slope moves us upward too, so only rising faster than
        // that counts as leaving the ground.
        let on_ground = !jumped && velocity.y < MAX_GROUNDED_RISE && ground.is_some();
        me.ground_normal = None;
        if let (true, Some(hit)) = (on_ground, ground) {
            position.y -= hit.distance;
            me.ground_normal = Some(hit.normal1);
        }

        transform.translation = position;
        vel.0 = velocity;
        *state = MovementState {
            on_ground,
            crouching,
            sprinting,
            eye_offset: Vec3::Y * if crouching { CROUCH_EYE } else { STAND_EYE },
            ..default()
        };
    }
}
