//! Local intent source: keyboard and mouse write the local player's `Intent`.

use bevy::{
    input::mouse::AccumulatedMouseMotion,
    prelude::*,
    window::{CursorGrabMode, CursorOptions},
};

use crate::core::{Intent, LocalPlayer};

#[derive(Resource, Reflect)]
#[reflect(Resource)]
pub struct MouseSensitivity(pub f32);

impl Default for MouseSensitivity {
    fn default() -> Self {
        // Radians per pixel of mouse motion.
        Self(0.0022)
    }
}

pub struct LocalInputPlugin;

impl Plugin for LocalInputPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<MouseSensitivity>()
            .register_type::<MouseSensitivity>()
            .add_systems(Update, (grab_cursor, write_local_intent).chain());
    }
}

pub fn cursor_grabbed(cursor: &CursorOptions) -> bool {
    cursor.grab_mode != CursorGrabMode::None
}

pub fn release_cursor(cursor: &mut CursorOptions) {
    cursor.visible = true;
    cursor.grab_mode = CursorGrabMode::None;
}

fn grab_cursor(
    mut cursor: Single<&mut CursorOptions>,
    mouse: Res<ButtonInput<MouseButton>>,
    keys: Res<ButtonInput<KeyCode>>,
) {
    if keys.just_pressed(KeyCode::Escape) {
        release_cursor(&mut cursor);
    } else if mouse.just_pressed(MouseButton::Left) && !cursor_grabbed(&cursor) {
        cursor.visible = false;
        cursor.grab_mode = CursorGrabMode::Locked;
    }
}

fn write_local_intent(
    mut intent: Single<&mut Intent, With<LocalPlayer>>,
    cursor: Single<&CursorOptions>,
    keys: Res<ButtonInput<KeyCode>>,
    mouse: Res<ButtonInput<MouseButton>>,
    motion: Res<AccumulatedMouseMotion>,
    sensitivity: Res<MouseSensitivity>,
) {
    if !cursor_grabbed(&cursor) {
        // Not playing (menu, inspector): stop moving, keep looking where we were.
        let (yaw, pitch) = (intent.yaw, intent.pitch);
        **intent = Intent {
            yaw,
            pitch,
            ..default()
        };
        return;
    }

    let axis = |pos: KeyCode, neg: KeyCode| keys.pressed(pos) as i8 as f32 - keys.pressed(neg) as i8 as f32;
    intent.move_axis = Vec2::new(axis(KeyCode::KeyD, KeyCode::KeyA), axis(KeyCode::KeyW, KeyCode::KeyS));

    const PITCH_LIMIT: f32 = 89f32.to_radians();
    intent.yaw = (intent.yaw - motion.delta.x * sensitivity.0).rem_euclid(std::f32::consts::TAU);
    intent.pitch = (intent.pitch - motion.delta.y * sensitivity.0).clamp(-PITCH_LIMIT, PITCH_LIMIT);

    intent.jump = keys.pressed(KeyCode::Space);
    intent.crouch = keys.pressed(KeyCode::ControlLeft) || keys.pressed(KeyCode::KeyC);
    intent.sprint = keys.pressed(KeyCode::ShiftLeft);
    intent.fire = mouse.pressed(MouseButton::Left);
    intent.reload = keys.pressed(KeyCode::KeyR);
}
