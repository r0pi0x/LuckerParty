//! Free flight through walls. A debug tool, and the second Movement
//! implementation that keeps the slot honest from day one.

use bevy::prelude::*;

use crate::{
    core::{Intent, MovementState, Predict, SimClock, Velocity},
    slots::RegisterSlots,
};

pub const ID: &str = "mashup:noclip";

const SPEED: f32 = 10.0;
const FAST_MULTIPLIER: f32 = 3.0;

#[derive(Component, Default)]
pub struct Noclip;

pub struct NoclipPlugin;

impl Plugin for NoclipPlugin {
    fn build(&self, app: &mut App) {
        app.register_movement::<Noclip>(ID)
            .add_systems(Predict::Movement, step);
    }
}

fn step(mut q: Query<(&Intent, &mut Transform, &mut Velocity, &mut MovementState), With<Noclip>>, clock: Res<SimClock>) {
    for (intent, mut transform, mut vel, mut state) in &mut q {
        let up = intent.jump as i8 as f32 - intent.crouch as i8 as f32;
        let local = Vec3::new(intent.move_axis.x, 0.0, -intent.move_axis.y).clamp_length_max(1.0);
        let speed = if intent.sprint { SPEED * FAST_MULTIPLIER } else { SPEED };
        vel.0 = (intent.look_rotation() * local + Vec3::Y * up) * speed;
        transform.translation += vel.0 * clock.dt();
        *state = MovementState {
            eye_offset: state.eye_offset,
            ..default()
        };
    }
}
