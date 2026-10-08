//! Local intent source: keyboard and mouse write the local player's `Intent`.

use bevy::{
    input::mouse::{AccumulatedMouseMotion, AccumulatedMouseScroll},
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
    /// Zoomed in, the turn is also scaled by this times the zoomed FOV over
    /// the normal one (`zoom_sensitivity_ratio`; CS:S's default 1.2 and the
    /// rule as the community documents it, not measured here).
    pub zoom_ratio: f32,
}

impl Default for MouseSettings {
    fn default() -> Self {
        // CS:S defaults.
        Self {
            sensitivity: 3.0,
            m_yaw: 0.022,
            m_pitch: 0.022,
            zoom_ratio: 1.2,
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

    /// The turn multiplier at a zoomed field of view (degrees; the normal
    /// one is 90).
    pub fn zoom_scale(&self, zoomed_fov: Option<f32>) -> f32 {
        zoomed_fov.map_or(1.0, |fov| self.zoom_ratio * fov / 90.0)
    }
}

pub struct LocalInputPlugin;

impl Plugin for LocalInputPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<MouseSettings>()
            .init_resource::<WheelJump>()
            .init_resource::<FreeLook>()
            .register_type::<MouseSettings>()
            .add_systems(Update, ((grab_cursor, write_local_intent).chain(), drop_key))
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
    resource_cvar::<MouseSettings, f32>(
        app,
        "zoom_sensitivity_ratio",
        "Extra mouse scale while zoomed (times the zoomed FOV / 90).",
        |m| &mut m.zoom_ratio,
    );
    let mut console = app.world_mut().resource_mut::<Console>();
    for name in ["sensitivity", "m_yaw", "m_pitch", "zoom_sensitivity_ratio"] {
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

pub(super) fn grab_cursor(
    mut cursor: Single<&mut CursorOptions>,
    mouse: Res<ButtonInput<MouseButton>>,
    keys: Res<ButtonInput<KeyCode>>,
    mut focus: MessageReader<bevy::window::WindowFocused>,
    menu: Option<Res<super::game_menu::GameMenu>>,
    vgui: Option<Res<super::vgui::VguiOpen>>,
    (console, debug_ui, egui): (
        Option<Res<super::console::ConsoleUi>>,
        Option<Res<super::debug_ui::DebugUi>>,
        Option<Res<bevy_inspector_egui::bevy_egui::input::EguiWantsInput>>,
    ),
) {
    // The game menu and the game-look buy and team menus own the mouse
    // while open (Esc opens and closes the game menu), and the click that
    // closes one isn't a click to grab. So do the console (clicks there
    // select and scroll its text), the debug UI, and any egui window
    // under the pointer (the inspector).
    let in_menu = menu.as_ref().is_some_and(|m| m.open || m.is_changed())
        || vgui.as_ref().is_some_and(|v| v.any() || v.is_changed())
        || console.is_some_and(|c| c.open)
        || debug_ui.is_some_and(|d| d.open || d.is_changed())
        || egui.is_some_and(|e| e.wants_any_pointer_input());
    // Losing focus (alt-tab) ends the grab: Windows drops the cursor clip
    // then, and a grab we still believed in would keep turning the view
    // while the real cursor wanders off and clicks other windows. Clicking
    // back in grabs again.
    if focus.read().any(|f| !f.focused) {
        release_cursor(&mut cursor);
    } else if keys.just_pressed(KeyCode::Escape) && menu.is_none() {
        release_cursor(&mut cursor);
    } else if mouse.just_pressed(MouseButton::Left) && !cursor_grabbed(&cursor) && !in_menu {
        capture_cursor(&mut cursor);
    }
}

/// The drop key (`drop`, G) drops the held weapon while playing; the bug
/// report key (`bugreport`, F9) saves a bug report any time.
fn drop_key(
    keys: Res<ButtonInput<KeyCode>>,
    mouse: Res<ButtonInput<MouseButton>>,
    cursor: Single<&CursorOptions>,
    console: Option<ResMut<crate::console::Console>>,
    ui: Option<Res<super::console::ConsoleUi>>,
    chat: Option<Res<super::chat::ChatInput>>,
    menu: Option<Res<super::game_menu::GameMenu>>,
) {
    let Some(mut console) = console else { return };
    // Typing, or the game menu reading keys (rebinding them).
    if ui.is_some_and(|u| u.open) || chat.is_some_and(|c| c.open.is_some()) || menu.is_some_and(|m| m.open) {
        return;
    }
    let pressed = |c: &str| super::binds::just_pressed(&console.binds, &keys, &mouse, c);
    let (drop, report) = (pressed("drop"), pressed("bug") || pressed("bugreport"));
    if drop && cursor_grabbed(&cursor) {
        console.submit("drop");
    }
    // A bug report (screenshot, position, build, console) to send.
    if report {
        console.submit("bugreport");
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
    (keys, mouse, motion, scroll): (
        Res<ButtonInput<KeyCode>>,
        Res<ButtonInput<MouseButton>>,
        Res<AccumulatedMouseMotion>,
        Res<AccumulatedMouseScroll>,
    ),
    console: Option<Res<crate::console::Console>>,
    mut wheel: ResMut<WheelJump>,
    mouse_settings: Res<MouseSettings>,
    held: Option<Res<super::console::HeldActions>>,
    mut free: ResMut<FreeLook>,
    mut freecam: ResMut<super::view::FreeCam>,
    time: Res<Time>,
    (menu, team_menu, radio_menu): (
        Option<Res<super::buy_menu::BuyMenu>>,
        Option<Res<super::team_menu::TeamMenu>>,
        Option<Res<super::radio::RadioMenu>>,
    ),
    zoomed: Query<&crate::weapon::Zoomed, With<LocalPlayer>>,
    spectator: Option<Res<super::spectate::Spectator>>,
) {
    // Every game key is a bind (`binds`): what the keys bound to an action
    // hold.
    let empty = std::collections::BTreeMap::new();
    let binds = console.as_ref().map_or(&empty, |c| &c.binds);
    let bound = |command: &str| super::binds::pressed(binds, &keys, &mouse, command);
    let freelook = bound("+freelook") || held.as_ref().is_some_and(|h| h.freelook);
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

    // Spectating (dead): the keys and mouse drive the spectator camera.
    if spectator.is_some_and(|s| s.active()) {
        *wheel = WheelJump::default();
        let (yaw, pitch) = (intent.yaw, intent.pitch);
        **intent = Intent {
            yaw,
            pitch,
            ..default()
        };
        return;
    }
    let axis = |pos: &str, neg: &str| bound(pos) as i8 as f32 - bound(neg) as i8 as f32;
    // Flying the detached camera: the player stands still.
    if freecam.mode == 1 {
        let input = Vec3::new(
            axis("+moveright", "+moveleft"),
            axis("+forward", "+back"),
            axis("+jump", "+duck"),
        );
        let speed = if bound("+speed") { 12.0 } else { 4.0 };
        freecam.fly(input, mouse_settings.look_delta(motion.delta), speed, time.delta_secs());
        let (yaw, pitch) = (intent.yaw, intent.pitch);
        **intent = Intent {
            yaw,
            pitch,
            ..default()
        };
        return;
    }
    intent.move_axis = Vec2::new(axis("+moveright", "+moveleft"), axis("+forward", "+back"));

    let zoom = mouse_settings.zoom_scale(zoomed.iter().next().map(|z| z.fov));
    let turn = mouse_settings.look_delta(motion.delta) * zoom;
    if freelook {
        free.turn(turn, intent.pitch);
    } else {
        intent.yaw = (intent.yaw + turn.x).rem_euclid(std::f32::consts::TAU);
        intent.pitch = (intent.pitch + turn.y).clamp(-PITCH_LIMIT, PITCH_LIMIT);
    }

    intent.jump = bound("+jump");
    wheel.key_held = intent.jump;
    let notches = super::binds::wheel_notches(binds, &scroll, "+jump");
    wheel.pending = (wheel.pending + notches).min(MAX_WHEEL_JUMPS);
    intent.crouch = bound("+duck");
    intent.sprint = bound("+speed");
    intent.walk = bound("+speed");
    intent.fire = bound("+attack");
    intent.secondary = bound("+attack2");
    intent.reload = bound("+reload");
    intent.last_weapon = bound("lastinv");
    intent.use_key = bound("+use");
    // Number keys pick from the buy, team or radio menu while one is open.
    intent.select = if menu.is_some_and(|m| m.open)
        || team_menu.is_some_and(|m| m.0)
        || radio_menu.is_some_and(|m| m.0.is_some())
    {
        None
    } else {
        (0..5u8).find(|i| bound(&format!("slot{}", i + 1)))
    };
    // Actions held from the console (`+duck` typed, `++attack`) add to the
    // keys.
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
    fn zoomed_turns_scale_by_the_ratio_and_fov() {
        let m = MouseSettings::default();
        assert_eq!(m.zoom_scale(None), 1.0);
        // AWP first zoom (40 degrees): 1.2 * 40 / 90.
        assert!((m.zoom_scale(Some(40.0)) - 1.2 * 40.0 / 90.0).abs() < 1e-6);
    }

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
