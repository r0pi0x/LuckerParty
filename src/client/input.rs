//! Local intent source: keyboard and mouse write the local player's `Intent`.

use bevy::{
    input::mouse::{AccumulatedMouseMotion, AccumulatedMouseScroll, MouseScrollUnit},
    prelude::*,
    window::{CursorGrabMode, CursorOptions},
};

use crate::core::{Intent, LocalPlayer};

/// Mouse look as CS:S does it: each count of raw mouse motion (no
/// acceleration) turns the view by `sensitivity * m_yaw` degrees
/// sideways and `sensitivity * m_pitch` degrees up or down (negative
/// `m_pitch` inverts).
#[derive(Resource, Reflect, Clone, Copy, Debug, PartialEq)]
#[reflect(Resource)]
pub struct MouseSettings {
    pub sensitivity: f32,
    pub m_yaw: f32,
    pub m_pitch: f32,
}

impl Default for MouseSettings {
    fn default() -> Self {
        // CS:S defaults.
        Self {
            sensitivity: 3.0,
            m_yaw: 0.022,
            m_pitch: 0.022,
        }
    }
}

impl MouseSettings {
    /// Radians to turn (yaw left, pitch up) for a mouse delta in counts
    /// (x right, y down).
    pub fn look_delta(&self, counts: Vec2) -> Vec2 {
        Vec2::new(
            -counts.x * self.sensitivity * self.m_yaw,
            -counts.y * self.sensitivity * self.m_pitch,
        ) * std::f32::consts::PI
            / 180.0
    }
}

pub struct LocalInputPlugin;

impl Plugin for LocalInputPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<MouseSettings>()
            .init_resource::<WheelJump>()
            .init_resource::<FreeLook>()
            .register_type::<MouseSettings>()
            .add_systems(Update, (grab_cursor, write_local_intent).chain())
            .add_systems(FixedPreUpdate, apply_wheel_jump);
        mouse_cvars(app);
    }
}

fn mouse_cvars(app: &mut App) {
    use crate::console::{Console, resource_cvar};
    resource_cvar::<MouseSettings, f32>(
        app,
        "sensitivity",
        "Mouse sensitivity: degrees per count are this times m_yaw / m_pitch.",
        |m| &mut m.sensitivity,
    );
    resource_cvar::<MouseSettings, f32>(
        app,
        "m_yaw",
        "Mouse yaw factor (degrees per count at sensitivity 1).",
        |m| &mut m.m_yaw,
    );
    resource_cvar::<MouseSettings, f32>(
        app,
        "m_pitch",
        "Mouse pitch factor (degrees per count at sensitivity 1; negative inverts).",
        |m| &mut m.m_pitch,
    );
    let mut console = app.world_mut().resource_mut::<Console>();
    for name in ["sensitivity", "m_yaw", "m_pitch"] {
        console.archive(name);
    }
}

pub fn cursor_grabbed(cursor: &CursorOptions) -> bool {
    cursor.grab_mode != CursorGrabMode::None
}

pub fn release_cursor(cursor: &mut CursorOptions) {
    cursor.visible = true;
    cursor.grab_mode = CursorGrabMode::None;
}

/// Lock and hide the cursor so the mouse turns the view.
pub fn capture_cursor(cursor: &mut CursorOptions) {
    cursor.visible = false;
    cursor.grab_mode = CursorGrabMode::Locked;
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
        capture_cursor(&mut cursor);
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

/// Free look: while Left Alt (or `+freelook`) is held, the mouse turns
/// only the camera, by these offsets (radians) from the aim; movement and
/// aim keep their direction. Released, the view snaps back to the aim.
#[derive(Resource, Default, Clone, Copy, Debug, PartialEq)]
pub struct FreeLook {
    pub yaw: f32,
    pub pitch: f32,
}

impl FreeLook {
    /// Turn by a look delta, keeping the camera's total pitch in range.
    fn turn(&mut self, delta: Vec2, aim_pitch: f32) {
        self.yaw = (self.yaw + delta.x + std::f32::consts::PI).rem_euclid(std::f32::consts::TAU) - std::f32::consts::PI;
        self.pitch = (aim_pitch + self.pitch + delta.y).clamp(-PITCH_LIMIT, PITCH_LIMIT) - aim_pitch;
    }
}

const PITCH_LIMIT: f32 = 89f32.to_radians();

#[allow(clippy::too_many_arguments)]
fn write_local_intent(
    mut intent: Single<&mut Intent, With<LocalPlayer>>,
    cursor: Single<&CursorOptions>,
    keys: Res<ButtonInput<KeyCode>>,
    mouse: Res<ButtonInput<MouseButton>>,
    motion: Res<AccumulatedMouseMotion>,
    scroll: Res<AccumulatedMouseScroll>,
    mut wheel: ResMut<WheelJump>,
    mouse_settings: Res<MouseSettings>,
    held: Option<Res<super::console::HeldActions>>,
    mut free: ResMut<FreeLook>,
) {
    let freelook = keys.pressed(KeyCode::AltLeft) || held.as_ref().is_some_and(|h| h.freelook);
    if !freelook {
        free.set_if_neq(FreeLook::default());
    }
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

    let turn = mouse_settings.look_delta(motion.delta);
    if freelook {
        free.turn(turn, intent.pitch);
    } else {
        intent.yaw = (intent.yaw + turn.x).rem_euclid(std::f32::consts::TAU);
        intent.pitch = (intent.pitch + turn.y).clamp(-PITCH_LIMIT, PITCH_LIMIT);
    }

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
    // CS:S binds E to +use.
    intent.use_key = keys.pressed(KeyCode::KeyE);
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
    intent.use_key |= h.use_key;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn free_look_turns_within_the_pitch_limit() {
        let mut f = FreeLook::default();
        f.turn(Vec2::new(0.5, 2.0), 0.5);
        assert_eq!(f.yaw, 0.5);
        // Aim pitch 0.5 plus the offset stays at the limit.
        assert!((0.5 + f.pitch - PITCH_LIMIT).abs() < 1e-6);
        // Yaw wraps to (-pi, pi].
        f.turn(Vec2::new(3.0, 0.0), 0.5);
        assert!(f.yaw < 0.0 && f.yaw > -std::f32::consts::PI);
    }

    #[test]
    fn mouse_look_matches_css() {
        let mut app = App::new();
        app.add_plugins(crate::console::ConsolePlugin)
            .init_resource::<MouseSettings>();
        mouse_cvars(&mut app);
        // CS:S defaults: 3 * 0.022 = 0.066 degrees per count.
        let d = app
            .world()
            .resource::<MouseSettings>()
            .look_delta(Vec2::new(1000.0, -1000.0));
        assert!((d.x.to_degrees() + 66.0).abs() < 1e-3, "{d}");
        assert!((d.y.to_degrees() - 66.0).abs() < 1e-3, "{d}");
        app.world_mut()
            .resource_mut::<crate::console::Console>()
            .submit("sensitivity 1.5; m_pitch -0.022");
        app.update();
        let m = *app.world().resource::<MouseSettings>();
        assert_eq!(m.sensitivity, 1.5);
        let d = m.look_delta(Vec2::new(100.0, 100.0));
        assert!((d.x.to_degrees() + 3.3).abs() < 1e-4, "{d}");
        assert!((d.y.to_degrees() - 3.3).abs() < 1e-4, "inverted: {d}");
        let console = app.world().resource::<crate::console::Console>();
        assert!(
            ["sensitivity", "m_yaw", "m_pitch"]
                .iter()
                .all(|n| console.cvar(n).unwrap().archive)
        );
    }

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
