//! Spectating while dead, as CS:S does it: a death cam turning to your
//! killer (or looking down at your body) for `DEATH_CAM_SECONDS`, then the
//! camera follows a living player. Jump cycles the mode (first person
//! through the target's eyes with its view model, a chase camera orbited
//! with the mouse, free roaming), attack and attack2 the next and previous
//! target; `mp_forcecamera 1` keeps you on your own team while it has
//! anyone alive. CS:S's spectator bars (the game's `spectator.res`: team
//! scores, map, round clock, the target's name and health, the mode), or
//! without them a plain panel naming the target, its health and weapon;
//! in first person the target's crosshair and scope (`hud.rs`). Back to
//! the eye when you live again (respawn, new round).
//!
//! Client-side only: it reads the simulation and moves the local camera.
//! The state machine (`Spectator::step`) is pure; `SpectateStatePlugin`
//! runs it without a window (tests), `SpectatePlugin` adds the camera,
//! input and HUD.

use avian3d::prelude::*;
use bevy::{input::mouse::AccumulatedMouseMotion, prelude::*, window::CursorOptions};

use super::view::{CameraMode, FreeCam, camera_offset};
use crate::{
    console::{ConsoleAppExt, resource_cvar},
    core::{Died, Health, Intent, LocalPlayer, MovementState, Team},
    map::{
        ViewModelAnchor, ViewModelSource,
        interp::{EyeView, RenderedView},
        ragdoll::Ragdoll,
    },
    objectives::hostages::Hostage,
    rules::Dead,
    weapon::{Inventory, ViewPunch, Weapon, Zoomed},
};

/// Seconds the death cam shows your killer (or your body) before
/// spectating starts. CS:S's feels about this long; not measured.
pub const DEATH_CAM_SECONDS: f64 = 2.0;
/// Seconds the camera stays on a target that died before moving on (a
/// guess at CS:S's, which lingers on the body a moment).
pub const TARGET_DEATH_HOLD: f64 = 2.0;
/// Chase camera distance behind the target's eye, CS:S units (a guess
/// near CS:S's).
const CHASE_DISTANCE: f32 = 96.0;
/// Chase camera's starting pitch below the target's view, degrees.
const CHASE_PITCH: f32 = -15.0;
/// The death cam's rise: back from the body (away from the killer) and up,
/// meters, and how long it takes to get there, seconds.
const DEATH_CAM_BACK: f32 = 1.4;
const DEATH_CAM_UP: f32 = 0.9;
const DEATH_CAM_MOVE: f32 = 0.8;
/// Radius of the sphere swept to keep spectator cameras out of walls.
const CAMERA_RADIUS: f32 = 0.2;
const PITCH_LIMIT: f32 = 89f32.to_radians();

/// How the camera follows the target.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SpecMode {
    /// Through the target's eyes, with its view angles and view model.
    #[default]
    InEye,
    /// Behind the target, orbited with the mouse.
    Chase,
    /// Flying free (WASD where you look, shift faster).
    Roaming,
}

impl SpecMode {
    /// The next mode on jump, as CS:S cycles them.
    pub fn next(self) -> Self {
        match self {
            SpecMode::InEye => SpecMode::Chase,
            SpecMode::Chase => SpecMode::Roaming,
            SpecMode::Roaming => SpecMode::InEye,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            SpecMode::InEye => "First Person",
            SpecMode::Chase => "Chase Camera",
            SpecMode::Roaming => "Free Look",
        }
    }

    /// `spec_mode` numbers, CS:S's observer modes: 4 in-eye, 5 chase, 6
    /// roaming.
    pub fn from_number(n: u32) -> Option<Self> {
        match n {
            4 => Some(SpecMode::InEye),
            5 => Some(SpecMode::Chase),
            6 => Some(SpecMode::Roaming),
            _ => None,
        }
    }

    pub fn number(self) -> u32 {
        match self {
            SpecMode::InEye => 4,
            SpecMode::Chase => 5,
            SpecMode::Roaming => 6,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub enum SpecPhase {
    /// Playing: the camera is at your eye.
    #[default]
    Alive,
    /// Just died: looking at the killer (or your body) since `since`.
    DeathCam { since: f64, killer: Option<Entity> },
    /// Following a target (or roaming).
    Watching,
}

/// A living character that could be watched.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Candidate {
    pub entity: Entity,
    pub team: Option<Team>,
    pub position: Vec3,
}

/// What the local player is.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Me {
    pub dead: bool,
    /// Killed (a `Died`), not just put out of play (team switch): a death
    /// cam first.
    pub killed: bool,
    pub killer: Option<Entity>,
    pub team: Option<Team>,
    pub position: Vec3,
}

/// Spectator requests from keys and console commands.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct SpecInput {
    pub next_mode: bool,
    pub set_mode: Option<SpecMode>,
    /// +1 next target, -1 previous.
    pub target_step: i32,
}

/// The local player's spectator state.
#[derive(Resource, Clone, Debug, Default)]
pub struct Spectator {
    pub phase: SpecPhase,
    /// Kept across deaths, as CS:S keeps your last observer mode.
    pub mode: SpecMode,
    pub target: Option<Entity>,
    /// When the target was last seen no longer watchable (dead).
    target_lost: Option<f64>,
    /// Requests waiting for the next step.
    pub pending: SpecInput,
}

/// `mp_forcecamera`: 0 watch anyone; 1 only your own team while it has
/// anyone alive. Free roaming stays allowed either way (here; CS:S may
/// restrict it).
#[derive(Resource, Clone, Copy, Debug, PartialEq)]
pub struct ForceCamera(pub u8);

impl Default for ForceCamera {
    fn default() -> Self {
        Self(1)
    }
}

/// The candidates you may watch: everyone living, or with `force` 1 only
/// your teammates while any of them lives.
pub fn allowed(living: &[Candidate], team: Option<Team>, force: u8) -> Vec<Entity> {
    let mates: Vec<Entity> = living
        .iter()
        .filter(|c| team.is_some() && c.team == team)
        .map(|c| c.entity)
        .collect();
    if force >= 1 && !mates.is_empty() {
        mates
    } else {
        living.iter().map(|c| c.entity).collect()
    }
}

/// The target `step` places from `current` in `allowed` (wrapping); from
/// none (or one not in the list) the first or, going back, the last.
pub fn cycle(allowed: &[Entity], current: Option<Entity>, step: i32) -> Option<Entity> {
    let n = allowed.len() as i32;
    if n == 0 {
        return None;
    }
    let i = match current.and_then(|c| allowed.iter().position(|e| *e == c)) {
        Some(i) => (i as i32 + step).rem_euclid(n),
        None if step < 0 => n - 1,
        None => 0,
    };
    Some(allowed[i as usize])
}

impl Spectator {
    pub fn active(&self) -> bool {
        self.phase != SpecPhase::Alive
    }

    /// The camera mode in effect: free roaming without a target, the chase
    /// camera on a target that just died.
    pub fn effective_mode(&self) -> SpecMode {
        match self.target {
            None => SpecMode::Roaming,
            Some(_) if self.target_lost.is_some() && self.mode == SpecMode::InEye => SpecMode::Chase,
            Some(_) => self.mode,
        }
    }

    /// Advance the state by a frame at `now` (seconds): `living` are the
    /// characters that could be watched, in cycling order.
    pub fn step(&mut self, now: f64, me: &Me, living: &[Candidate], force: u8, input: SpecInput) {
        if !me.dead {
            self.phase = SpecPhase::Alive;
            self.target = None;
            self.target_lost = None;
            return;
        }
        if self.phase == SpecPhase::Alive {
            self.target = None;
            self.target_lost = None;
            self.phase = if me.killed {
                SpecPhase::DeathCam {
                    since: now,
                    killer: me.killer,
                }
            } else {
                SpecPhase::Watching
            };
        }
        if let SpecPhase::DeathCam { since, .. } = self.phase {
            // The death cam plays out; keys wait for it.
            if now - since < DEATH_CAM_SECONDS {
                return;
            }
            self.phase = SpecPhase::Watching;
        }
        if let Some(m) = input.set_mode {
            self.mode = m;
        } else if input.next_mode {
            self.mode = self.mode.next();
        }
        let allowed = allowed(living, me.team, force);
        match self.target {
            Some(t) if allowed.contains(&t) => self.target_lost = None,
            Some(_) => {
                // Gone (dead, or not allowed now): linger, then move on.
                let lost = *self.target_lost.get_or_insert(now);
                if now - lost >= TARGET_DEATH_HOLD || input.target_step != 0 {
                    self.target = None;
                    self.target_lost = None;
                }
            }
            None => {}
        }
        if input.target_step != 0 {
            self.target = cycle(&allowed, self.target, input.target_step).or(self.target);
            self.target_lost = None;
        } else if self.target.is_none() {
            // The nearest one first.
            self.target = living
                .iter()
                .filter(|c| allowed.contains(&c.entity))
                .min_by(|a, b| {
                    a.position
                        .distance_squared(me.position)
                        .total_cmp(&b.position.distance_squared(me.position))
                })
                .map(|c| c.entity);
        }
    }
}

/// The spectator camera's result for this frame: where the local camera
/// goes (world space) and whose eyes it looks through.
#[derive(Resource, Clone, Copy, Debug, Default, PartialEq)]
pub struct SpecView {
    pub pose: Option<(Vec3, Quat)>,
    /// The target watched in first person (its body hidden, its view model
    /// and zoom shown).
    pub in_eye: Option<Entity>,
}

/// The state machine and its console commands, without a window.
pub struct SpectateStatePlugin;

impl Plugin for SpectateStatePlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Spectator>()
            .init_resource::<ForceCamera>()
            .init_resource::<KilledBy>()
            .add_systems(Update, (remember_killer, track).chain().in_set(SpectateSet));
        resource_cvar::<ForceCamera, u8>(
            app,
            "mp_forcecamera",
            "Spectating while dead: 0 any living player, 1 only your team while it has anyone alive.",
            |f| &mut f.0,
        );
        app.console_command(
            "spec_mode",
            "spec_mode [4|5|6]: next spectator mode, or first person (4), chase (5), free look (6).",
            |w, a| {
                let mut s = w.resource_mut::<Spectator>();
                match a.first() {
                    None => s.pending.next_mode = true,
                    Some(n) => {
                        let m = n
                            .parse()
                            .ok()
                            .and_then(SpecMode::from_number)
                            .ok_or("spec_mode 4|5|6 (first person, chase, free look)")?;
                        s.pending.set_mode = Some(m);
                    }
                }
                Ok(None)
            },
        )
        .console_command("spec_next", "Spectate the next player.", |w, _| {
            w.resource_mut::<Spectator>().pending.target_step = 1;
            Ok(None)
        })
        .console_command("spec_prev", "Spectate the previous player.", |w, _| {
            w.resource_mut::<Spectator>().pending.target_step = -1;
            Ok(None)
        });
    }
}

/// The spectator systems (state, then camera), before `follow_eye`.
#[derive(SystemSet, Clone, Debug, PartialEq, Eq, Hash)]
pub struct SpectateSet;

/// Everything: state, keys, camera, HUD.
pub struct SpectatePlugin;

impl Plugin for SpectatePlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(SpectateStatePlugin)
            .init_resource::<SpecView>()
            .init_resource::<ChaseOrbit>()
            .add_systems(
                Update,
                (
                    keys.before(super::input::grab_cursor).before(track),
                    (place_camera, show_through_eyes).chain().after(track).in_set(SpectateSet),
                    spectator_panel.after(place_camera),
                ),
            );
    }
}

/// Who killed the local player last (from `Died`), until it lives again.
#[derive(Resource, Default)]
struct KilledBy(Option<Option<Entity>>);

fn remember_killer(
    mut died: MessageReader<Died>,
    local: Option<Single<(Entity, Has<Dead>), With<LocalPlayer>>>,
    mut killed: ResMut<KilledBy>,
    mut was_dead: Local<bool>,
) {
    let Some(local) = local else {
        died.clear();
        return;
    };
    let (me, dead) = *local;
    // Forget the killer on living again (`Died` may come a tick before
    // `Dead` is on the player).
    if *was_dead && !dead {
        killed.0 = None;
    }
    *was_dead = dead;
    for d in died.read() {
        if d.entity == me {
            killed.0 = Some(d.attacker.filter(|a| *a != me));
        }
    }
}

type Living<'a> = (
    Entity,
    &'a Transform,
    Option<&'a Team>,
    Option<&'a Name>,
    Option<&'a Health>,
    Has<Dead>,
);

#[allow(clippy::type_complexity)]
fn track(
    time: Res<Time>,
    local: Option<Single<(&Transform, Option<&Team>, Has<Dead>), With<LocalPlayer>>>,
    characters: Query<Living, (With<Intent>, Without<LocalPlayer>, Without<Hostage>)>,
    killed: Res<KilledBy>,
    force: Res<ForceCamera>,
    mut spec: ResMut<Spectator>,
) {
    let Some(local) = local else { return };
    let (at, team, dead) = *local;
    let me = Me {
        dead,
        killed: killed.0.is_some(),
        killer: killed.0.flatten(),
        team: team.copied(),
        position: at.translation,
    };
    let mut living: Vec<(String, Candidate)> = characters
        .iter()
        .filter(|(.., health, dead)| !dead && health.is_none_or(|h| h.current > 0.0))
        .map(|(e, t, team, name, ..)| {
            (
                name.map_or_else(String::new, |n| n.as_str().to_string()),
                Candidate {
                    entity: e,
                    team: team.copied(),
                    position: t.translation,
                },
            )
        })
        .collect();
    // A stable order to cycle in: by name ("Bot 2" before "Bot 10").
    living.sort_by(|(a, ca), (b, cb)| natural(a).cmp(&natural(b)).then(ca.entity.cmp(&cb.entity)));
    let living: Vec<Candidate> = living.into_iter().map(|(_, c)| c).collect();
    let input = std::mem::take(&mut spec.pending);
    spec.step(time.elapsed_secs_f64(), &me, &living, force.0, input);
}

/// A sort key that orders the numbers in names by value.
fn natural(name: &str) -> (String, u64) {
    let digits: String = name.chars().rev().take_while(|c| c.is_ascii_digit()).collect();
    let n = digits.chars().rev().collect::<String>().parse().unwrap_or(0);
    (name[..name.len() - digits.len()].to_string(), n)
}

/// Jump cycles the mode, attack and attack2 the target (while the mouse
/// is captured, so the click that captures it doesn't count).
fn keys(
    keys: Res<ButtonInput<KeyCode>>,
    mouse: Res<ButtonInput<MouseButton>>,
    cursor: Single<&CursorOptions>,
    console: Res<crate::console::Console>,
    mut spec: ResMut<Spectator>,
) {
    if spec.phase != SpecPhase::Watching || !super::input::cursor_grabbed(&cursor) {
        return;
    }
    let pressed = |c: &str| super::binds::just_pressed(&console.binds, &keys, &mouse, c);
    if pressed("+jump") {
        spec.pending.next_mode = true;
    }
    if pressed("+attack") {
        spec.pending.target_step = 1;
    } else if pressed("+attack2") {
        spec.pending.target_step = -1;
    }
}

/// The chase camera's orbit (yaw, pitch, radians) and the roaming camera.
#[derive(Resource, Default)]
struct ChaseOrbit {
    orbit: Vec2,
    /// The target and mode the orbit was set up for.
    set_for: Option<(Entity, SpecMode)>,
    roam: FreeCam,
}

type Watched<'a> = (
    &'a Transform,
    &'a Intent,
    &'a MovementState,
    Option<&'a ViewPunch>,
    Option<&'a RenderedView>,
);

/// Where a character is drawn this frame and its eye (eased between
/// ticks, `map::interp`; the simulation's when not eased).
fn drawn((t, i, s, punch, view): (&Transform, &Intent, &MovementState, Option<&ViewPunch>, Option<&RenderedView>)) -> (Vec3, EyeView) {
    let eye = match view {
        Some(v) => v.now,
        None => EyeView {
            punch: punch.map_or(Vec2::ZERO, |p| p.0),
            ..EyeView::of(i, s)
        },
    };
    (t.translation, eye)
}

/// Where the spectator camera goes this frame.
#[allow(clippy::too_many_arguments, clippy::type_complexity)]
fn place_camera(
    time: Res<Time>,
    spec: Res<Spectator>,
    mut view: ResMut<SpecView>,
    mut chase: ResMut<ChaseOrbit>,
    local: Option<Single<(Entity, &Transform, &Intent, &MovementState, Option<&RenderedView>), With<LocalPlayer>>>,
    watched: Query<Watched>,
    ragdolls: Query<&Ragdoll>,
    bodies: Query<&GlobalTransform>,
    spatial: SpatialQuery,
    characters: Query<Entity, With<Intent>>,
    controls: (
        Option<Res<ButtonInput<KeyCode>>>,
        Option<Res<AccumulatedMouseMotion>>,
        Option<Res<super::input::MouseSettings>>,
        Option<Single<&CursorOptions>>,
        Option<Res<ButtonInput<MouseButton>>>,
        Option<Res<crate::console::Console>>,
    ),
) {
    let (Some(local), true) = (local, spec.active()) else {
        chase.set_for = None;
        chase.roam.at = None;
        view.set_if_neq(SpecView::default());
        return;
    };
    let (me, at, intent, state, own_view) = *local;
    let own = crate::map::interp::eye_view(own_view, intent, state);
    let (keys, motion, settings, cursor, mouse, console) = controls;
    let grabbed = cursor.is_some_and(|c| super::input::cursor_grabbed(&c));
    let turn = match (grabbed, motion, settings) {
        (true, Some(m), Some(s)) => s.look_delta(m.delta),
        _ => Vec2::ZERO,
    };
    let look_at = |from: Vec3, to: Vec3| Transform::from_translation(from).looking_at(to, Vec3::Y).rotation;
    let mode = spec.effective_mode();
    let pose = match spec.phase {
        SpecPhase::Alive => None,
        SpecPhase::DeathCam { since, killer } => {
            let eye = at.translation + own.eye_offset;
            let start = Quat::from_euler(EulerRot::YXZ, intent.yaw, intent.pitch, 0.0);
            let killer = killer.filter(|k| *k != me).and_then(|k| watched.get(k).ok()).map(drawn);
            // Back from the body, away from the killer (or behind where
            // you faced), and up; looking at the killer or the body.
            let (away, focus) = match killer {
                Some((t, e)) => {
                    let focus = t + e.eye_offset;
                    ((eye - focus).with_y(0.0).normalize_or(start * Vec3::Z), focus)
                }
                None => {
                    let body = ragdolls
                        .iter()
                        .find(|r| r.owner == me)
                        .and_then(|r| r.bodies.first())
                        .and_then(|b| bodies.get(*b).ok())
                        .map_or(at.translation, |g| g.translation());
                    ((start * Vec3::Z).with_y(0.0).normalize_or(Vec3::Z), body)
                }
            };
            let s = ((time.elapsed_secs_f64() - since) as f32 / DEATH_CAM_MOVE).clamp(0.0, 1.0);
            let s = s * s * (3.0 - 2.0 * s);
            let to = sweep(&spatial, eye, away * DEATH_CAM_BACK + Vec3::Y * DEATH_CAM_UP, &characters);
            let p = eye.lerp(to, s);
            let end = if focus.distance(p) > 1e-3 { look_at(p, focus) } else { start };
            Some((p, start.slerp(end, s)))
        }
        SpecPhase::Watching => {
            let target = spec.target.and_then(|t| watched.get(t).ok()).map(drawn);
            let set_for = spec.target.map(|t| (t, mode));
            if set_for != chase.set_for {
                // A new target or mode: the orbit starts behind its view.
                if let Some((_, e)) = target {
                    chase.orbit = Vec2::new(e.yaw, CHASE_PITCH.to_radians());
                }
                chase.set_for = set_for;
            }
            if mode != SpecMode::Roaming {
                chase.roam.at = None;
            }
            match (mode, target) {
                (SpecMode::InEye, Some((t, e))) => {
                    let p = e.punch;
                    let look = Quat::from_euler(EulerRot::YXZ, e.yaw + p.y, e.pitch + p.x, 0.0);
                    let fp = CameraMode::default();
                    let offset = camera_offset(&fp, t, e.eye_offset, look, &spatial, &characters);
                    Some((t + offset, look))
                }
                (SpecMode::Chase, Some((t, e))) => {
                    chase.orbit.x = (chase.orbit.x + turn.x).rem_euclid(std::f32::consts::TAU);
                    chase.orbit.y = (chase.orbit.y + turn.y).clamp(-PITCH_LIMIT, PITCH_LIMIT);
                    let look = Quat::from_euler(EulerRot::YXZ, chase.orbit.x, chase.orbit.y, 0.0);
                    let third = CameraMode {
                        third_person: true,
                        ideal_dist: CHASE_DISTANCE,
                        ideal_yaw: 0.0,
                    };
                    let offset = camera_offset(&third, t, e.eye_offset, look, &spatial, &characters);
                    Some((t + offset, look))
                }
                _ => {
                    // Roaming: from wherever the camera was.
                    if chase.roam.at.is_none() {
                        let (p, q) = view
                            .pose
                            .unwrap_or((at.translation + own.eye_offset, Quat::from_euler(EulerRot::YXZ, intent.yaw, intent.pitch, 0.0)));
                        let (yaw, pitch, _) = q.to_euler(EulerRot::YXZ);
                        chase.roam.at = Some((p, yaw, pitch));
                    }
                    // The movement keys (binds) fly it.
                    let held = |c: &str| {
                        grabbed
                            && match (&keys, &mouse, &console) {
                                (Some(k), Some(m), Some(con)) => super::binds::pressed(&con.binds, k, m, c),
                                _ => false,
                            }
                    };
                    let axis = |a: &str, b: &str| held(a) as i8 as f32 - held(b) as i8 as f32;
                    let input = Vec3::new(axis("+moveright", "+moveleft"), axis("+forward", "+back"), 0.0);
                    let speed = if held("+speed") { 12.0 } else { 4.0 };
                    chase.roam.fly(input, turn, speed, time.delta_secs());
                    chase.roam.rotation()
                }
            }
        }
    };
    let in_eye = (spec.phase == SpecPhase::Watching && mode == SpecMode::InEye)
        .then_some(spec.target)
        .flatten();
    view.set_if_neq(SpecView { pose, in_eye });
}

/// `from + offset`, pulled in if the world is in the way.
fn sweep(spatial: &SpatialQuery, from: Vec3, offset: Vec3, characters: &Query<Entity, With<Intent>>) -> Vec3 {
    let Ok(dir) = Dir3::new(offset) else { return from };
    let filter = SpatialQueryFilter::from_excluded_entities(characters).with_mask(crate::core::SOLID_LAYERS);
    let config = ShapeCastConfig {
        max_distance: offset.length(),
        ignore_origin_penetration: true,
        ..default()
    };
    let d = spatial
        .cast_shape(&Collider::sphere(CAMERA_RADIUS), from, Quat::IDENTITY, dir, &config, &filter)
        .map_or(offset.length(), |h| h.distance);
    from + dir * d
}

/// Watching through a target's eyes: its body hidden (the camera is
/// inside it) and its view model drawn at the local camera.
#[allow(clippy::type_complexity)]
fn show_through_eyes(
    view: Res<SpecView>,
    mut hidden: Local<Option<Entity>>,
    children: Query<&Children>,
    mut bodies: Query<&mut Visibility, With<crate::map::CharacterBody>>,
    anchors: Query<(Entity, Option<&ViewModelSource>), With<ViewModelAnchor>>,
    mut commands: Commands,
) {
    let mut set = |owner: Entity, want: Visibility| {
        if let Ok(kids) = children.get(owner) {
            let mut it = bodies.iter_many_mut(kids);
            while let Some(mut v) = it.fetch_next() {
                v.set_if_neq(want);
            }
        }
    };
    if *hidden != view.in_eye
        && let Some(old) = hidden.take()
    {
        set(old, Visibility::Inherited);
    }
    if let Some(t) = view.in_eye {
        // Every frame: a body re-made (team change) comes back visible.
        set(t, Visibility::Hidden);
        *hidden = Some(t);
    }
    for (anchor, source) in &anchors {
        let want = view.in_eye.map(ViewModelSource);
        if source.copied() != want {
            match want {
                Some(s) => commands.entity(anchor).insert(s),
                None => commands.entity(anchor).remove::<ViewModelSource>(),
            };
        }
    }
}

/// The field of view to use while spectating in first person: the
/// target's zoom.
pub(super) fn target_zoom(view: &SpecView, zoomed: &Query<&Zoomed>) -> Option<Option<f32>> {
    view.in_eye.map(|t| zoomed.get(t).ok().map(|z| z.fov))
}

#[derive(Component)]
struct SpectatorPanel;

/// The game-look bars' root (the whole window).
#[derive(Component)]
struct SpectatorBars;

/// What the spectator HUD says: the watched (or killing) player's name
/// and health and their team, the mode's name, the weapon.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SpecHudText {
    pub target: Option<(String, f32, Option<Team>)>,
    pub mode: Option<SpecMode>,
    pub weapon: String,
    pub killed_by: bool,
}

impl SpecHudText {
    /// The plain panel's text: "Spectating: Bot 3 [100]   AK47" and the
    /// mode, or "Killed by ..." during the death cam.
    pub fn plain(&self) -> String {
        match (&self.target, self.mode, self.killed_by) {
            (Some((name, _, _)), _, true) => format!("Killed by {name}"),
            (None, _, true) => String::new(),
            (Some((name, hp, _)), Some(mode), false) => {
                format!("Spectating: {name} [{hp:.0}]   {}\n{}", self.weapon, mode.name())
            }
            (None, Some(mode), false) => mode.name().to_string(),
            (_, None, false) => String::new(),
        }
    }
}

/// What to show for the spectator's state.
#[allow(clippy::type_complexity)]
fn hud_text(
    spec: &Spectator,
    who: &Query<(Option<&Name>, Option<&Health>, Option<&Inventory>, Option<&Team>)>,
    weapons: &Query<&Weapon>,
) -> SpecHudText {
    let about = |e: Entity| {
        let (name, health, _, team) = who.get(e).unwrap_or((None, None, None, None));
        let name = name.map_or_else(|| "Player".to_string(), |n| n.as_str().to_string());
        let hp = health.map_or(0.0, |h| (h.current * 100.0).ceil().max(0.0));
        (name, hp, team.copied())
    };
    match spec.phase {
        SpecPhase::Alive => SpecHudText::default(),
        SpecPhase::DeathCam { killer, .. } => SpecHudText {
            target: killer.filter(|k| who.contains(*k)).map(about),
            killed_by: true,
            ..default()
        },
        SpecPhase::Watching => {
            let mode = spec.effective_mode();
            let target = spec.target.filter(|_| mode != SpecMode::Roaming);
            let weapon = target
                .and_then(|t| who.get(t).ok())
                .and_then(|(_, _, i, _)| i?.active)
                .and_then(|w| weapons.get(w).ok())
                .map(|w| weapon_name(w.id))
                .unwrap_or_default();
            SpecHudText {
                target: target.map(about),
                mode: Some(mode),
                weapon,
                killed_by: false,
            }
        }
    }
}

/// The spectator HUD while dead: with the game's layout (CS:S's
/// `resource/ui/spectator.res`) its top and bottom bars across the
/// window: the team scores, the map's name and the round clock at the top
/// right, the watched player's "name (health)" in their team's colour at
/// the bottom, the camera mode where the spectator menu's view list sits
/// (`bottomspectator.res`'s `viewcombo`). Without it, a plain panel.
#[allow(clippy::type_complexity, clippy::too_many_arguments)]
fn spectator_panel(
    spec: Res<Spectator>,
    windows: Query<&Window>,
    who: Query<(Option<&Name>, Option<&Health>, Option<&Inventory>, Option<&Team>)>,
    weapons: Query<&Weapon>,
    mut panel: Query<(&mut Text, &mut TextFont, &mut Visibility), (With<SpectatorPanel>, Without<SpectatorBars>)>,
    mut bars: Query<(Entity, &mut Visibility), (With<SpectatorBars>, Without<SpectatorPanel>)>,
    (fonts, hud, map): (
        Res<super::fonts::UiFonts>,
        Option<Res<crate::map::hud::ActiveHud>>,
        Option<Res<crate::map::LoadedMapName>>,
    ),
    (rounds, fixed): (Option<Res<crate::rules::rounds::RoundState>>, Res<Time<Fixed>>),
    mut last: Local<Option<BarsShot>>,
    mut commands: Commands,
) {
    let (Ok((mut text, mut font, mut vis)), Ok((root, mut bars_vis))) = (panel.single_mut(), bars.single_mut()) else {
        if panel.is_empty() {
            commands.spawn((
                SpectatorPanel,
                Text::default(),
                TextFont::default(),
                TextColor(Color::WHITE),
                TextShadow::default(),
                TextLayout::justify(Justify::Center),
                Node {
                    position_type: PositionType::Absolute,
                    bottom: Val::Percent(15.0),
                    width: Val::Percent(100.0),
                    justify_content: JustifyContent::Center,
                    ..default()
                },
                Visibility::Hidden,
                GlobalZIndex(44),
            ));
        }
        if bars.is_empty() {
            commands.spawn((
                SpectatorBars,
                Node {
                    position_type: PositionType::Absolute,
                    width: percent(100.0),
                    height: percent(100.0),
                    ..default()
                },
                Visibility::Hidden,
                GlobalZIndex(43),
            ));
        }
        return;
    };
    let about = hud_text(&spec, &who, &weapons);
    let height = windows.iter().next().map_or(480.0, |w| w.height());
    let layouts = hud.as_ref().and_then(|h| h.0.menus.as_ref()).and_then(|m| {
        let bars = m.layouts.get(m.spectator.as_ref()?)?;
        Some((m, bars, m.spectator_menu.as_ref().and_then(|k| m.layouts.get(k))))
    });
    let active = spec.active();
    // The plain panel, without the game's layout.
    let line = if layouts.is_some() { String::new() } else { about.plain() };
    vis.set_if_neq(if line.is_empty() {
        Visibility::Hidden
    } else {
        Visibility::Inherited
    });
    if text.0 != line {
        text.0 = line;
    }
    // The client scheme's Default.
    let want = fonts.client("Default", height, 12.0);
    if *font != want {
        *font = want;
    }
    let (Some((menus, layout, menu_layout)), Some(hud), true) = (layouts, hud.as_deref(), active) else {
        bars_vis.set_if_neq(Visibility::Hidden);
        *last = None;
        return;
    };
    bars_vis.set_if_neq(Visibility::Inherited);
    let size = windows
        .iter()
        .next()
        .map_or(Vec2::new(640.0, 480.0), |w| Vec2::new(w.width(), w.height()));
    let map = map.map_or_else(String::new, |m| m.0.rsplit(':').next().unwrap_or(&m.0).to_string());
    let rounds = rounds.filter(|r| r.phase != crate::rules::rounds::Phase::Off);
    let shot = BarsShot {
        about,
        map,
        clock: rounds
            .as_ref()
            .and_then(|r| r.clock(fixed.elapsed_secs_f64()))
            .map(super::game_hud::clock_text),
        wins: rounds.map(|r| r.wins),
        size,
    };
    if last.as_ref() == Some(&shot) {
        return;
    }
    commands.entity(root).despawn_related::<Children>();
    let painter = super::vgui::Painter::new(hud, &fonts, size.y);
    draw_bars(&mut commands, root, &painter, menus, layout, menu_layout, &shot);
    *last = Some(shot);
}

/// What the bars show; redrawn when it changes.
#[derive(Clone, Debug, PartialEq)]
struct BarsShot {
    about: SpecHudText,
    map: String,
    clock: Option<String>,
    wins: Option<[u32; 2]>,
    size: Vec2,
}

/// The bars' text by control name (lower case), None: as the layout has
/// it.
pub fn bar_text(name: &str, about: &SpecHudText, map: &str, clock: Option<&str>, wins: Option<[u32; 2]>, menus: &crate::map::hud::GameMenus) -> Option<String> {
    Some(match name {
        "playerlabel" => about
            .target
            .as_ref()
            .map(|(name, hp, _)| {
                menus
                    .string("Spec_PlayerItem_Health", "%s1 (%s2)")
                    .replace("%s1", name)
                    .replace("%s2", &format!("{hp:.0}"))
            })
            .unwrap_or_default(),
        "extrainfo" => menus.string("Spec_Map", "Map: %s1").replace("%s1", map),
        "timerlabel" => clock.unwrap_or("").to_string(),
        "ctscorevalue" => wins.map_or(0, |w| w[1]).to_string(),
        "terscorevalue" => wins.map_or(0, |w| w[0]).to_string(),
        _ => return None,
    })
}

#[allow(clippy::too_many_arguments)]
fn draw_bars(
    commands: &mut Commands,
    root: Entity,
    painter: &super::vgui::Painter,
    menus: &crate::map::hud::GameMenus,
    layout: &crate::map::hud::UiLayout,
    menu_layout: Option<&crate::map::hud::UiLayout>,
    shot: &BarsShot,
) {
    let bg = painter.color("BgColor", [0, 0, 0, 196]);
    let rects = layout.rects(Rect::from_corners(Vec2::ZERO, shot.size), painter.scale);
    // The bars span the window (the layout's are 640 wide).
    for name in ["topbar", "bottombarblank"] {
        if let Some(i) = layout.controls.iter().position(|c| c.name.eq_ignore_ascii_case(name)) {
            let r = rects[i];
            commands.spawn((
                Node {
                    position_type: PositionType::Absolute,
                    left: px(0.0),
                    top: px(r.min.y),
                    width: percent(100.0),
                    height: px(r.height()),
                    ..default()
                },
                BackgroundColor(bg),
                ZIndex(-1),
                ChildOf(root),
            ));
        }
    }
    let team_color = |team: Option<Team>| match team.map(|t| t.0) {
        Some(1) => painter.color("T_Red", [255, 64, 64, 255]),
        Some(2) => painter.color("CT_Blue", [153, 204, 255, 255]),
        _ => painter.color("team0", [204, 204, 204, 255]),
    };
    let mut shown = |c: &crate::map::hud::UiControl| {
        let mut s = super::vgui::Shown::of(c);
        let name = c.name.to_lowercase();
        if matches!(name.as_str(), "topbar" | "bottombarblank") {
            s.visible = false;
        }
        s.text = bar_text(
            &name,
            &shot.about,
            &shot.map,
            shot.clock.as_deref(),
            shot.wins,
            menus,
        );
        if name == "playerlabel" {
            s.visible = shot.about.target.is_some();
        }
        if matches!(name.as_str(), "timerclock" | "timerlabel") && shot.clock.is_none() {
            s.visible = false;
        }
        s
    };
    let drawn = painter.spawn(commands, root, shot.size, layout, super::vgui::VguiMenu::Spectator, &mut shown);
    // The name in its team's colour.
    if let (Some((label, _)), Some((_, _, team))) = (drawn.get("playerlabel"), &shot.about.target) {
        let (label, color) = (*label, team_color(*team));
        // The label's text node is its child.
        commands.queue(move |w: &mut World| {
            let kids: Vec<Entity> = w.get::<Children>(label).map(|c| c.iter().collect()).unwrap_or_default();
            for k in kids {
                if let Some(mut c) = w.get_mut::<TextColor>(k) {
                    c.0 = color;
                }
            }
        });
    }
    // The camera mode, where the spectator menu's view list sits.
    if let Some(mode) = shot.about.mode {
        let bar = menu_layout.and_then(|l| {
            let rects = l.rects(Rect::from_corners(Vec2::ZERO, shot.size), painter.scale);
            let frame = l.controls.iter().position(|c| c.name.eq_ignore_ascii_case("specmenu"))?;
            let view = l.controls.iter().position(|c| c.name.eq_ignore_ascii_case("viewcombo"))?;
            Some(Rect::from_corners(
                rects[view].min + Vec2::Y * rects[frame].min.y,
                rects[view].max + Vec2::Y * rects[frame].min.y,
            ))
        });
        if let Some(r) = bar {
            commands
                .spawn((
                    Node {
                        position_type: PositionType::Absolute,
                        left: px(r.min.x),
                        top: px(r.min.y),
                        width: px(r.width()),
                        height: px(r.height()),
                        display: Display::Flex,
                        align_items: AlignItems::Center,
                        ..default()
                    },
                    ChildOf(root),
                ))
                .with_child((
                    Text::new(spec_mode_text(menus, mode)),
                    painter.font(None),
                    TextColor(painter.color("Label.TextColor", [255, 176, 0, 255])),
                ));
        }
    }
}

/// The mode's name as the game words it (`#Spec_Mode3`... in the
/// spectator menu's list: first person, chase camera, free look).
fn spec_mode_text(menus: &crate::map::hud::GameMenus, mode: SpecMode) -> String {
    let token = match mode {
        SpecMode::InEye => "Spec_Mode3",
        SpecMode::Chase => "Spec_Mode4",
        SpecMode::Roaming => "Spec_Mode5",
    };
    menus.string(token, mode.name()).to_string()
}

/// `cs_source:weapon_ak47` -> `AK47`.
fn weapon_name(id: &str) -> String {
    let id = id.rsplit(':').next().unwrap_or(id);
    id.strip_prefix("weapon_").unwrap_or(id).to_uppercase()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn e(i: u32) -> Entity {
        Entity::from_raw_u32(i).unwrap()
    }

    fn cand(i: u32, team: u8, x: f32) -> Candidate {
        Candidate {
            entity: e(i),
            team: Some(Team(team)),
            position: Vec3::X * x,
        }
    }

    fn dead_ct(killed: bool) -> Me {
        Me {
            dead: true,
            killed,
            killer: killed.then(|| e(9)),
            team: Some(Team(2)),
            position: Vec3::ZERO,
        }
    }

    #[test]
    fn force_camera_keeps_you_on_your_team_while_it_lives() {
        let living = [cand(1, 1, 1.0), cand(2, 2, 5.0), cand(3, 2, 9.0)];
        assert_eq!(allowed(&living, Some(Team(2)), 1), vec![e(2), e(3)]);
        assert_eq!(allowed(&living, Some(Team(2)), 0), vec![e(1), e(2), e(3)]);
        // No teammate left: anyone.
        assert_eq!(allowed(&living[..1], Some(Team(2)), 1), vec![e(1)]);
        assert!(allowed(&[], Some(Team(2)), 1).is_empty());
    }

    #[test]
    fn targets_cycle_both_ways_and_wrap() {
        let list = [e(1), e(2), e(3)];
        assert_eq!(cycle(&list, Some(e(1)), 1), Some(e(2)));
        assert_eq!(cycle(&list, Some(e(3)), 1), Some(e(1)));
        assert_eq!(cycle(&list, Some(e(1)), -1), Some(e(3)));
        assert_eq!(cycle(&list, None, 1), Some(e(1)));
        assert_eq!(cycle(&list, Some(e(7)), -1), Some(e(3)));
        assert_eq!(cycle(&[], Some(e(1)), 1), None);
    }

    #[test]
    fn modes_cycle_first_person_chase_free_look() {
        let m = SpecMode::InEye;
        assert_eq!(m.next(), SpecMode::Chase);
        assert_eq!(m.next().next(), SpecMode::Roaming);
        assert_eq!(m.next().next().next(), SpecMode::InEye);
        for m in [SpecMode::InEye, SpecMode::Chase, SpecMode::Roaming] {
            assert_eq!(SpecMode::from_number(m.number()), Some(m));
        }
    }

    #[test]
    fn death_cam_then_the_nearest_teammate() {
        let living = [cand(1, 1, 1.0), cand(2, 2, 9.0), cand(3, 2, 4.0)];
        let mut s = Spectator::default();
        s.step(10.0, &dead_ct(true), &living, 1, SpecInput::default());
        assert_eq!(
            s.phase,
            SpecPhase::DeathCam {
                since: 10.0,
                killer: Some(e(9))
            }
        );
        // Keys wait for the death cam.
        let next = SpecInput {
            next_mode: true,
            target_step: 1,
            ..default()
        };
        s.step(11.0, &dead_ct(true), &living, 1, next);
        assert!(matches!(s.phase, SpecPhase::DeathCam { .. }) && s.target.is_none());
        assert_eq!(s.mode, SpecMode::InEye);
        s.step(10.0 + DEATH_CAM_SECONDS, &dead_ct(true), &living, 1, SpecInput::default());
        assert_eq!(s.phase, SpecPhase::Watching);
        // Not the nearer enemy: the nearest teammate.
        assert_eq!(s.target, Some(e(3)));
        // Attack: the next teammate, wrapping; never the enemy.
        let step = |n| SpecInput {
            target_step: n,
            ..default()
        };
        s.step(13.0, &dead_ct(true), &living, 1, step(1));
        assert_eq!(s.target, Some(e(2)));
        s.step(13.1, &dead_ct(true), &living, 1, step(1));
        assert_eq!(s.target, Some(e(3)));
        s.step(13.2, &dead_ct(true), &living, 1, step(-1));
        assert_eq!(s.target, Some(e(2)));
        // mp_forcecamera 0: the enemy too.
        s.step(13.3, &dead_ct(true), &living, 0, step(-1));
        assert_eq!(s.target, Some(e(1)));
        // Jump: the next mode.
        s.step(
            13.4,
            &dead_ct(true),
            &living,
            1,
            SpecInput {
                next_mode: true,
                ..default()
            },
        );
        assert_eq!(s.mode, SpecMode::Chase);
    }

    #[test]
    fn put_out_of_play_without_dying_skips_the_death_cam() {
        let living = [cand(2, 2, 3.0)];
        let mut s = Spectator::default();
        s.step(1.0, &dead_ct(false), &living, 1, SpecInput::default());
        assert_eq!(s.phase, SpecPhase::Watching);
        assert_eq!(s.target, Some(e(2)));
    }

    #[test]
    fn a_dead_target_lingers_then_the_next_one() {
        let mut living = vec![cand(2, 2, 3.0), cand(3, 2, 5.0)];
        let mut s = Spectator {
            mode: SpecMode::InEye,
            ..default()
        };
        s.step(0.0, &dead_ct(false), &living, 1, SpecInput::default());
        assert_eq!(s.target, Some(e(2)));
        // Bot 2 dies: still on it (chase camera on the body) for a moment.
        living.remove(0);
        s.step(1.0, &dead_ct(false), &living, 1, SpecInput::default());
        assert_eq!(s.target, Some(e(2)));
        assert_eq!(s.effective_mode(), SpecMode::Chase);
        s.step(1.0 + TARGET_DEATH_HOLD, &dead_ct(false), &living, 1, SpecInput::default());
        assert_eq!(s.target, Some(e(3)));
        assert_eq!(s.effective_mode(), SpecMode::InEye);
        // Everyone dead: free look.
        s.step(5.0, &dead_ct(false), &[], 1, SpecInput::default());
        s.step(5.0 + TARGET_DEATH_HOLD, &dead_ct(false), &[], 1, SpecInput::default());
        assert_eq!(s.target, None);
        assert_eq!(s.effective_mode(), SpecMode::Roaming);
    }

    #[test]
    fn living_again_ends_spectating_but_keeps_the_mode() {
        let living = [cand(2, 2, 3.0)];
        let mut s = Spectator::default();
        s.step(0.0, &dead_ct(false), &living, 1, SpecInput {
            set_mode: Some(SpecMode::Chase),
            ..default()
        });
        assert_eq!(s.mode, SpecMode::Chase);
        s.step(1.0, &Me::default(), &living, 1, SpecInput::default());
        assert_eq!(s.phase, SpecPhase::Alive);
        assert!(!s.active() && s.target.is_none());
        assert_eq!(s.mode, SpecMode::Chase);
    }

    #[test]
    fn names_sort_by_their_numbers() {
        let mut names = vec!["Bot 10", "Bot 2", "Bot 1"];
        names.sort_by_key(|n| natural(n));
        assert_eq!(names, ["Bot 1", "Bot 2", "Bot 10"]);
    }

    #[test]
    fn the_bars_fill_in_the_game_strings() {
        let menus = crate::map::hud::GameMenus {
            strings: std::collections::HashMap::from([
                ("spec_map".to_string(), "Map: %s1".to_string()),
                ("spec_playeritem_health".to_string(), "%s1 (%s2)".to_string()),
            ]),
            ..default()
        };
        let about = SpecHudText {
            target: Some(("Bot 3".into(), 76.0, Some(Team(2)))),
            mode: Some(SpecMode::Chase),
            weapon: "AK47".into(),
            killed_by: false,
        };
        let text = |name| bar_text(name, &about, "de_dust2", Some("1:45"), Some([3, 5]), &menus);
        assert_eq!(text("playerlabel").as_deref(), Some("Bot 3 (76)"));
        assert_eq!(text("extrainfo").as_deref(), Some("Map: de_dust2"));
        assert_eq!(text("timerlabel").as_deref(), Some("1:45"));
        assert_eq!(text("ctscorevalue").as_deref(), Some("5"));
        assert_eq!(text("terscorevalue").as_deref(), Some("3"));
        assert_eq!(text("ctscorelabel"), None, "as the layout has it");
        assert_eq!(about.plain(), "Spectating: Bot 3 [76]   AK47\nChase Camera");
        let killed = SpecHudText {
            killed_by: true,
            ..about
        };
        assert_eq!(killed.plain(), "Killed by Bot 3");
    }
}
