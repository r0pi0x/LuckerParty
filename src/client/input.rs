//! Local intent source: keyboard and mouse write the local player's `Intent`.

use bevy::{
    input::mouse::{AccumulatedMouseMotion, AccumulatedMouseScroll, MouseScrollUnit},
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
            .init_resource::<WheelJump>()
            .register_type::<MouseSensitivity>()
            .add_systems(Update, (grab_cursor, write_local_intent).chain())
            .add_systems(FixedPreUpdate, apply_wheel_jump);
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
    mut focus: MessageReader<bevy::window::WindowFocused>,
) {
    // Losing focus (alt-tab) ends the grab: Windows drops the cursor clip
    // then, and a grab we still believed in would keep turning the view
    // while the real cursor wanders off and clicks other windows. Clicking
    // back in grabs again.
    if focus.read().any(|f| !f.focused) {
        release_cursor(&mut cursor);
    } else if keys.just_pressed(KeyCode::Escape) {
        release_cursor(&mut cursor);
    } else if mouse.just_pressed(MouseButton::Left) && !cursor_grabbed(&cursor) {
        cursor.visible = false;
        cursor.grab_mode = CursorGrabMode::Locked;
    }
}

/// Jumping on the mouse wheel, as CS:S players bind it: each notch (either
/// direction) is one press of jump. Presses are queued and played out one
/// fixed tick held, one tick released, so a jump that needs a fresh press
/// (Source) sees every notch, and spinning the wheel near a landing lands
/// a press on the right tick.
#[derive(Resource, Default)]
struct WheelJump {
    pending: u32,
    /// Jump held on the keyboard.
    key_held: bool,
    pressed_last_tick: bool,
}

/// Notches queued at most, so a long spin doesn't keep jumping after it.
const MAX_WHEEL_JUMPS: u32 = 4;
/// Pixel-unit scrolling (touchpads, smooth wheels): pixels per notch.
const PIXELS_PER_NOTCH: f32 = 40.0;

fn apply_wheel_jump(mut wheel: ResMut<WheelJump>, mut intent: Single<&mut Intent, With<LocalPlayer>>) {
    let pulse = wheel.pending > 0 && !wheel.pressed_last_tick;
    if pulse {
        wheel.pending -= 1;
    }
    wheel.pressed_last_tick = pulse;
    intent.jump = wheel.key_held || pulse;
}

fn write_local_intent(
    mut intent: Single<&mut Intent, With<LocalPlayer>>,
    cursor: Single<&CursorOptions>,
    keys: Res<ButtonInput<KeyCode>>,
    mouse: Res<ButtonInput<MouseButton>>,
    motion: Res<AccumulatedMouseMotion>,
    scroll: Res<AccumulatedMouseScroll>,
    mut wheel: ResMut<WheelJump>,
    sensitivity: Res<MouseSensitivity>,
    held: Option<Res<super::console::HeldActions>>,
) {
    if !cursor_grabbed(&cursor) {
        *wheel = WheelJump::default();
        // Not playing (menu, inspector): stop moving, keep looking where we
        // were. Console-held actions (`+attack` from a bind or script)
        // still apply, as in Source.
        let (yaw, pitch) = (intent.yaw, intent.pitch);
        **intent = Intent {
            yaw,
            pitch,
            ..default()
        };
        if let Some(h) = held {
            apply_held(&mut intent, &mut wheel, &h);
        }
        return;
    }

    let axis = |pos: KeyCode, neg: KeyCode| keys.pressed(pos) as i8 as f32 - keys.pressed(neg) as i8 as f32;
    intent.move_axis = Vec2::new(axis(KeyCode::KeyD, KeyCode::KeyA), axis(KeyCode::KeyW, KeyCode::KeyS));

    const PITCH_LIMIT: f32 = 89f32.to_radians();
    intent.yaw = (intent.yaw - motion.delta.x * sensitivity.0).rem_euclid(std::f32::consts::TAU);
    intent.pitch = (intent.pitch - motion.delta.y * sensitivity.0).clamp(-PITCH_LIMIT, PITCH_LIMIT);

    intent.jump = keys.pressed(KeyCode::Space);
    wheel.key_held = intent.jump;
    let notches = match scroll.unit {
        MouseScrollUnit::Line => scroll.delta.y.abs().round(),
        MouseScrollUnit::Pixel => (scroll.delta.y.abs() / PIXELS_PER_NOTCH).ceil(),
    } as u32;
    wheel.pending = (wheel.pending + notches).min(MAX_WHEEL_JUMPS);
    intent.crouch = keys.pressed(KeyCode::ControlLeft) || keys.pressed(KeyCode::KeyC);
    intent.sprint = keys.pressed(KeyCode::ShiftLeft);
    intent.walk = keys.pressed(KeyCode::ShiftLeft);
    intent.fire = mouse.pressed(MouseButton::Left);
    intent.secondary = mouse.pressed(MouseButton::Right);
    intent.reload = keys.pressed(KeyCode::KeyR);
    intent.last_weapon = keys.pressed(KeyCode::KeyQ);
    const SLOTS: [KeyCode; 5] = [
        KeyCode::Digit1,
        KeyCode::Digit2,
        KeyCode::Digit3,
        KeyCode::Digit4,
        KeyCode::Digit5,
    ];
    intent.select = SLOTS.iter().position(|k| keys.pressed(*k)).map(|i| i as u8);
    // Bound actions (`bind f +duck`) add to the keys.
    if let Some(h) = held {
        apply_held(&mut intent, &mut wheel, &h);
    }
}

fn apply_held(intent: &mut Intent, wheel: &mut WheelJump, h: &super::console::HeldActions) {
    let add = |a: bool, b: bool| a as i8 as f32 - b as i8 as f32;
    intent.move_axis += Vec2::new(add(h.moveright, h.moveleft), add(h.forward, h.back));
    intent.move_axis = intent.move_axis.clamp(Vec2::NEG_ONE, Vec2::ONE);
    intent.jump |= h.jump;
    wheel.key_held |= h.jump;
    intent.crouch |= h.duck;
    intent.walk |= h.speed;
    intent.sprint |= h.speed;
    intent.fire |= h.attack;
    intent.secondary |= h.attack2;
    intent.reload |= h.reload;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn losing_focus_releases_the_cursor() {
        let mut app = App::new();
        app.init_resource::<ButtonInput<MouseButton>>()
            .init_resource::<ButtonInput<KeyCode>>()
            .add_message::<bevy::window::WindowFocused>()
            .add_systems(Update, grab_cursor);
        let window = app
            .world_mut()
            .spawn(CursorOptions {
                visible: false,
                grab_mode: CursorGrabMode::Locked,
                ..default()
            })
            .id();
        app.update();
        assert!(cursor_grabbed(app.world().get::<CursorOptions>(window).unwrap()));
        app.world_mut()
            .write_message(bevy::window::WindowFocused { window, focused: false });
        app.update();
        let cursor = app.world().get::<CursorOptions>(window).unwrap();
        assert!(!cursor_grabbed(cursor) && cursor.visible);
    }
}
