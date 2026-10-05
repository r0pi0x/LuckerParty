//! Stand-in walking movement so the foundation is playable before any game's
//! movement is specced. Deliberately not modeled on any game; replace with
//! spec-based implementations, don't tune this into one.

use avian3d::prelude::*;
use bevy::prelude::*;

use super::wish_dir;
use crate::{
    character::CAPSULE_HEIGHT,
    core::{Intent, MovementState, SimSet, Velocity},
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
const STAND_EYE: f32 = 1.62 - CAPSULE_HEIGHT / 2.0;
const CROUCH_EYE: f32 = 1.0 - CAPSULE_HEIGHT / 2.0;

#[derive(Component, Default)]
pub struct PlaceholderMovement {
    jump_held: bool,
}

pub struct PlaceholderMovementPlugin;

impl Plugin for PlaceholderMovementPlugin {
    fn build(&self, app: &mut App) {
        app.register_movement::<PlaceholderMovement>(ID)
            .add_systems(FixedUpdate, step.in_set(SimSet::Movement));
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
    time: Res<Time>,
) {
    let dt = time.delta_secs();
    for (entity, intent, mut me, mut transform, mut vel, mut state, collider) in &mut q {
        let filter = SpatialQueryFilter::from_excluded_entities([entity]);
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

        // Vertical: gravity, jumping on a fresh press only.
        let mut vertical = vel.y;
        let jumped = intent.jump && !me.jump_held && state.on_ground;
        me.jump_held = intent.jump;
        if jumped {
            vertical = JUMP_SPEED;
        } else if state.on_ground {
            vertical = vertical.min(0.0);
        }
        vertical -= GRAVITY * dt;

        let out = move_and_slide.move_and_slide(
            collider,
            transform.translation,
            transform.rotation,
            horizontal + Vec3::Y * vertical,
            time.delta(),
            &config,
            &filter,
            |_| MoveAndSlideHitResponse::Accept,
        );
        let mut position = out.position;
        let mut velocity = out.projected_velocity;

        // Ground check, snapping down so we stick to ramps and small drops.
        let ground = move_and_slide
            .cast_move(collider, position, transform.rotation, Vec3::NEG_Y * GROUND_PROBE, config.skin_width, &filter)
            .filter(|hit| hit.normal1.y >= MIN_GROUND_NORMAL_Y);
        let on_ground = !jumped && velocity.y <= 0.0 && ground.is_some();
        if let (true, Some(hit)) = (on_ground, ground) {
            position.y -= hit.distance;
            velocity.y = 0.0;
        }

        transform.translation = position;
        vel.0 = velocity;
        *state = MovementState {
            on_ground,
            crouching,
            sprinting,
            eye_offset: Vec3::Y * if crouching { CROUCH_EYE } else { STAND_EYE },
        };
    }
}
