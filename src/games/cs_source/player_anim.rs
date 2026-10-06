//! What CS:S player bodies play: the player animation state of
//! specs/cs_source/animation.md §12 (the SDK template the CS:S data
//! follows) driving each character's `Animator`.

use bevy::prelude::*;

use crate::{
    core::{Health, Intent, MovementState, Velocity},
    map::{
        DriveAnimation,
        anim::{AnimSet, Animator, Fading, Layer},
    },
    weapon::{Inventory, Weapon, WeaponEvent, WeaponEventKind},
};

const UNIT: f32 = 0.0254;
const MOVING_MIN_SPEED: f32 = 0.5;
const RUN_THRESHOLD: f32 = 175.0;
const MAX_BODY_YAW: f32 = 90.0;
const FEET_YAW_RATE: f32 = 720.0;
const FEET_TURN_FADE: f32 = 45.0;
const FACE_FRONT_TIME: f64 = 3.0;
const JUMP_GROUND_IGNORE: f64 = 0.2;

pub struct PlayerAnimPlugin;

impl Plugin for PlayerAnimPlugin {
    fn build(&self, app: &mut App) {
        app.add_message::<WeaponEvent>()
            .add_systems(Update, (drive.in_set(DriveAnimation), hold_weapons));
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Activity {
    Idle,
    Walk,
    Run,
    CrouchIdle,
    CrouchWalk,
    Hop,
}

impl Activity {
    fn name(self) -> &'static str {
        match self {
            Activity::Idle => "ACT_IDLE",
            Activity::Walk => "ACT_WALK",
            Activity::Run => "ACT_RUN",
            Activity::CrouchIdle => "ACT_CROUCHIDLE",
            Activity::CrouchWalk => "ACT_RUN_CROUCH",
            Activity::Hop => "ACT_HOP",
        }
    }

    /// Full-pose speed (units/s) of the activity's sequence (§12.2).
    fn ground_speed(self) -> f32 {
        match self {
            Activity::Idle | Activity::Walk => 100.0,
            Activity::Run => 250.0,
            Activity::CrouchWalk => 85.0,
            _ => 0.0,
        }
    }
}

/// Upper-body sequences fading into each other (§12.6): (sequence, time
/// it became current).
#[derive(Default, Debug, Clone)]
struct Queue(Vec<(usize, f64)>);

impl Queue {
    /// Make `s` current; the two layers' (sequence, weight).
    fn update(&mut self, set: &AnimSet, s: usize, now: f64) -> [Option<(usize, f32)>; 2] {
        if self.0.last().is_none_or(|(last, _)| *last != s) {
            self.0.push((s, now));
        }
        loop {
            match self.0.as_slice() {
                [] => return [None, None],
                [(a, _)] => return [Some((*a, 1.0)), None],
                [(a, _), (b, since), ..] => {
                    let fade = Fading {
                        sequence: *a,
                        cycle: 0.0,
                        playback: 1.0,
                        stopped: *since,
                        fade: set.sequences[*a].fade_out.min(set.sequences[*b].fade_in),
                    };
                    let u = fade.weight(now);
                    if u <= 0.0 {
                        self.0.remove(0);
                        continue;
                    }
                    return [Some((*a, u)), Some((*b, 1.0 - u))];
                }
            }
        }
    }
}

/// One character's animation state.
#[derive(Component, Debug, Clone)]
pub struct PlayerAnim {
    /// The next update is the first (spawn, respawn).
    reset: bool,
    pub feet_yaw: f32,
    goal_yaw: f32,
    last_turn: f64,
    gait_yaw: f32,
    jumping: Option<f64>,
    restart: bool,
    was_on_ground: bool,
    pub activity: Activity,
    idle: Queue,
    moving: Queue,
    /// The fire layer: sequence and cycle.
    fire: Option<(usize, f32)>,
}

impl Default for PlayerAnim {
    fn default() -> Self {
        Self {
            reset: true,
            feet_yaw: 0.0,
            goal_yaw: 0.0,
            last_turn: 0.0,
            gait_yaw: 0.0,
            jumping: None,
            restart: false,
            was_on_ground: true,
            activity: Activity::Idle,
            idle: Queue::default(),
            moving: Queue::default(),
            fire: None,
        }
    }
}

/// Angle in degrees into (-180, 180].
fn normalize(a: f32) -> f32 {
    let a = a % 360.0;
    if a > 180.0 {
        a - 360.0
    } else if a <= -180.0 {
        a + 360.0
    } else {
        a
    }
}

/// What the body sees this update, in Source terms: degrees, units/s, the
/// game's world axes (yaw 0 = +X, counter-clockwise).
#[derive(Clone, Copy, Debug, Default)]
pub struct Inputs {
    pub eye_yaw: f32,
    /// Positive looking down.
    pub eye_pitch: f32,
    pub velocity: Vec2,
    pub ducked: bool,
    pub on_ground: bool,
    pub jumped: bool,
    pub fired: bool,
}

/// What the state decided: pose parameters by name and the render yaw.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Outputs {
    pub move_x: f32,
    pub move_y: f32,
    pub body_yaw: f32,
    pub body_pitch: f32,
    pub rate: f32,
}

impl PlayerAnim {
    /// Feet yaw, torso twist (§12.4); returns `body_yaw`.
    pub fn turn(&mut self, eye: f32, moving: bool, dt: f32, now: f64) -> f32 {
        if self.reset {
            self.feet_yaw = eye;
            self.goal_yaw = eye;
            self.last_turn = 0.0;
        } else {
            if moving || now - self.last_turn > FACE_FRONT_TIME {
                self.goal_yaw = eye;
            } else {
                let e = normalize(self.goal_yaw - eye);
                if e.abs() > MAX_BODY_YAW {
                    self.goal_yaw += if e > 0.0 { -MAX_BODY_YAW } else { MAX_BODY_YAW };
                }
            }
            self.goal_yaw = normalize(self.goal_yaw);
            if self.feet_yaw != self.goal_yaw {
                let diff = normalize(self.goal_yaw - self.feet_yaw);
                let a = diff.abs();
                let scale = if a <= FEET_TURN_FADE {
                    (a / FEET_TURN_FADE).clamp(0.01, 1.0)
                } else {
                    1.0
                };
                let mut step = FEET_YAW_RATE * dt * scale;
                if a > MAX_BODY_YAW {
                    step = a - MAX_BODY_YAW;
                }
                self.feet_yaw = if a < step {
                    self.goal_yaw
                } else {
                    normalize(self.feet_yaw + step * diff.signum())
                };
                self.last_turn = now;
            }
        }
        self.reset = false;
        normalize(eye - self.feet_yaw)
    }

    /// The main activity (§12.1); also tracks jumping.
    fn pick(&mut self, i: &Inputs, speed: f32, now: f64) -> Activity {
        if i.jumped {
            self.jumping = Some(now);
            self.restart = true;
        }
        if let Some(start) = self.jumping
            && now - start > JUMP_GROUND_IGNORE
            && i.on_ground
        {
            self.jumping = None;
            self.restart = true;
        }
        let moving = speed > MOVING_MIN_SPEED;
        if self.jumping.is_some() {
            Activity::Hop
        } else if i.ducked {
            if moving { Activity::CrouchWalk } else { Activity::CrouchIdle }
        } else if moving {
            if speed > RUN_THRESHOLD { Activity::Run } else { Activity::Walk }
        } else {
            Activity::Idle
        }
    }

    /// One update (§ "Per-frame order") for a body with weapon animation
    /// `suffix`.
    pub fn update(&mut self, a: &mut Animator, i: &Inputs, suffix: &str, dt: f32, now: f64) -> Outputs {
        let set = a.set.clone();
        let speed = i.velocity.length();
        let moving = speed > MOVING_MIN_SPEED;
        // Main sequence.
        let activity = self.pick(i, speed, now);
        if let Some(s) = set.activity(activity.name()) {
            if activity != self.activity || a.main.is_none() {
                a.play(s, now);
            }
            if self.restart {
                a.cycle = 0.0;
            }
        }
        self.restart = false;
        self.activity = activity;
        let ground = activity.ground_speed();
        let rate = if !moving {
            1.0
        } else if ground < 0.001 {
            0.01
        } else {
            (speed / ground).clamp(0.01, 10.0)
        };
        // Pose parameters.
        let body_pitch = normalize(i.eye_pitch).clamp(-90.0, 90.0);
        let body_yaw = self.turn(i.eye_yaw, moving, dt, now);
        if moving {
            self.gait_yaw = i.velocity.y.atan2(i.velocity.x).to_degrees();
        }
        let d = normalize(-(i.eye_yaw - self.gait_yaw)).to_radians();
        let (move_x, move_y) = if moving {
            (d.cos() * rate, -d.sin() * rate)
        } else {
            (0.0, 0.0)
        };
        for (name, v) in [
            ("move_x", move_x),
            ("move_y", move_y),
            ("body_yaw", body_yaw),
            ("body_pitch", body_pitch),
        ] {
            if let Some(p) = set.param(name) {
                a.set_param(p, v);
            }
        }
        a.advance(dt, now);
        let cycle = a.cycle;
        // Upper-body layers 1-4, phase-locked to the legs.
        a.layers.resize(5, None);
        let seq = |name: String| set.sequence(&name);
        let idle_name = if activity == Activity::CrouchIdle {
            format!("Crouch_Idle_Upper_{suffix}")
        } else {
            format!("Idle_Upper_{suffix}")
        };
        let put = |slot: usize, pair: [Option<(usize, f32)>; 2], scale: f32, layers: &mut Vec<Option<Layer>>| {
            for (k, e) in pair.into_iter().enumerate() {
                layers[slot + k] = e.map(|(sequence, w)| Layer {
                    sequence,
                    cycle,
                    weight: (w * scale).clamp(0.0, 1.0),
                });
            }
        };
        let mut layers = std::mem::take(&mut a.layers);
        match seq(idle_name) {
            Some(s) => put(0, self.idle.update(&set, s, now), 1.0, &mut layers),
            None => put(0, [None, None], 1.0, &mut layers),
        }
        let moving_name = match activity {
            Activity::Run => Some(format!("Run_Upper_{suffix}")),
            Activity::Walk => Some(format!("Walk_Upper_{suffix}")),
            Activity::CrouchWalk => Some(format!("Crouch_Walk_Upper_{suffix}")),
            _ => None,
        };
        match moving_name.filter(|_| moving).and_then(seq) {
            Some(s) => put(2, self.moving.update(&set, s, now), rate, &mut layers),
            None => {
                self.moving = Queue::default();
                put(2, [None, None], 1.0, &mut layers);
            }
        }
        let others: f32 = layers[1..4].iter().flatten().map(|l| l.weight).sum();
        if let Some(l) = &mut layers[0] {
            l.weight = (1.0 - others).max(0.0);
        }
        // Layers under a later full-weight one change nothing visible.
        if let Some(full) = (0..4).rev().find(|k| layers[*k].is_some_and(|l| l.weight > 0.99)) {
            layers[..full].fill(None);
        }
        // Fire layer (order 5).
        if i.fired {
            let name = match activity {
                Activity::Run => "Run_Shoot_",
                Activity::Walk => "Walk_Shoot_",
                Activity::CrouchIdle => "Crouch_Idle_Shoot_",
                Activity::CrouchWalk => "Crouch_Walk_Shoot_",
                _ => "Idle_Shoot_",
            };
            self.fire = set.sequence(&format!("{name}{suffix}")).map(|s| (s, 0.0));
        } else if let Some((s, c)) = &mut self.fire {
            *c += set.cycle_rate(*s, &a.params) * dt;
            if *c > 1.0 {
                self.fire = None;
            }
        }
        layers[4] = self.fire.map(|(sequence, cycle)| Layer {
            sequence,
            cycle,
            weight: 1.0,
        });
        a.layers = layers;
        Outputs {
            move_x,
            move_y,
            body_yaw,
            body_pitch,
            rate,
        }
    }
}

/// The weapon's player animation suffix (`PlayerAnimationExtension`).
pub fn suffix(weapon: Option<&str>) -> &'static str {
    let Some(id) = weapon else { return "Pistol" };
    let name = id.rsplit(['_', ':']).next().unwrap_or(id);
    match name {
        "ak47" => "AK",
        "aug" => "AUG",
        "awp" => "AWP",
        "c4" => "C4",
        "elite" => "ELITES",
        "famas" => "FAMAS",
        "g3sg1" => "G3",
        "galil" => "GALIL",
        "m249" => "M249",
        "m3" => "M3S90",
        "m4a1" => "M4",
        "mac10" => "MAC10",
        "mp5navy" => "MP5",
        "p90" => "P90",
        "scout" => "SCOUT",
        "sg550" => "SG550",
        "sg552" => "SG552",
        "tmp" => "TMP",
        "ump45" => "UMP45",
        "xm1014" => "XM1014",
        "knife" => "KNIFE",
        "hegrenade" | "flashbang" | "smokegrenade" => "GREN",
        _ => "PISTOL",
    }
}

/// Characters hold their active weapon's world model.
fn hold_weapons(
    characters: Query<(Entity, Option<&Inventory>, Option<&crate::map::Held>), With<Animator>>,
    weapons: Query<&Weapon>,
    mut commands: Commands,
) {
    for (e, inventory, held) in &characters {
        let id = inventory.and_then(|i| i.active).and_then(|w| weapons.get(w).ok()).map(|w| w.id.to_string());
        if held.is_none_or(|h| h.0 != id) {
            commands.entity(e).insert(crate::map::Held(id));
        }
    }
}

#[allow(clippy::type_complexity)]
fn drive(
    time: Res<Time>,
    mut characters: Query<(
        Entity,
        &mut Animator,
        Option<&mut PlayerAnim>,
        &Intent,
        &Velocity,
        &MovementState,
        Option<&Health>,
        Option<&Inventory>,
    )>,
    weapons: Query<&Weapon>,
    mut events: MessageReader<WeaponEvent>,
    mut commands: Commands,
) {
    let fired: Vec<Entity> = events
        .read()
        .filter(|e| matches!(e.kind, WeaponEventKind::Shot { .. } | WeaponEventKind::Swing { .. }))
        .map(|e| e.owner)
        .collect();
    let (dt, now) = (time.delta_secs(), time.elapsed_secs_f64());
    for (e, mut animator, state, intent, velocity, movement, health, inventory) in &mut characters {
        let Some(mut state) = state else {
            commands.entity(e).insert(PlayerAnim::default());
            continue;
        };
        if health.is_some_and(|h| h.current <= 0.0) {
            state.reset = true;
            continue;
        }
        let jumped = state.was_on_ground && !movement.on_ground && velocity.y > 0.0;
        state.was_on_ground = movement.on_ground;
        // Our axes to the game's: forward at yaw 0 is -Z (+X there), left
        // is -X (+Y there).
        let inputs = Inputs {
            eye_yaw: intent.yaw.to_degrees(),
            eye_pitch: -intent.pitch.to_degrees(),
            velocity: Vec2::new(-velocity.z, -velocity.x) / UNIT,
            ducked: movement.crouching,
            on_ground: movement.on_ground,
            jumped,
            fired: fired.contains(&e),
        };
        let weapon = inventory.and_then(|i| i.active).and_then(|w| weapons.get(w).ok());
        let suffix = suffix(weapon.map(|w| w.id));
        state.update(&mut animator, &inputs, suffix, dt, now);
        animator.yaw = Some(state.feet_yaw.to_radians());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn turned(state: &mut PlayerAnim, eye: f32, moving: bool, now: f64) -> f32 {
        state.turn(eye, moving, 0.01, now)
    }

    #[test]
    fn feet_follow_the_eyes() {
        let mut s = PlayerAnim::default();
        assert_eq!(turned(&mut s, 30.0, false, 0.0), 0.0);
        assert_eq!(s.feet_yaw, 30.0);

        let fresh = || PlayerAnim {
            reset: false,
            last_turn: 1.0,
            ..default()
        };
        let mut s = fresh();
        assert_eq!(turned(&mut s, 30.0, false, 1.5), 30.0);
        assert_eq!(s.feet_yaw, 0.0);

        let mut s = fresh();
        let twist = turned(&mut s, 120.0, false, 1.5);
        assert_eq!(s.goal_yaw, 90.0);
        assert!((s.feet_yaw - 7.2).abs() < 1e-4, "{}", s.feet_yaw);
        assert!((twist - 112.8).abs() < 1e-4);

        let mut s = fresh();
        let twist = turned(&mut s, 100.0, true, 1.5);
        assert!((s.feet_yaw - 10.0).abs() < 1e-4, "{}", s.feet_yaw);
        assert!((twist - 90.0).abs() < 1e-4);
        turned(&mut s, 100.0, true, 1.51);
        assert!((s.feet_yaw - 17.2).abs() < 1e-4, "{}", s.feet_yaw);

        let mut s = fresh();
        turned(&mut s, 10.0, true, 1.5);
        assert!((s.feet_yaw - 1.6).abs() < 1e-4, "{}", s.feet_yaw);

        let mut s = fresh();
        turned(&mut s, 50.0, false, 4.5);
        assert_eq!(s.goal_yaw, 50.0);
        assert!((s.feet_yaw - 7.2).abs() < 1e-4, "{}", s.feet_yaw);
    }

    #[test]
    fn activities_from_speed() {
        let mut s = PlayerAnim::default();
        let at = |s: &mut PlayerAnim, v: Vec2, ducked: bool| {
            let i = Inputs {
                velocity: v,
                ducked,
                on_ground: true,
                ..default()
            };
            s.pick(&i, v.length(), 0.0)
        };
        assert_eq!(at(&mut s, Vec2::new(0.0, 250.0), false), Activity::Run);
        assert_eq!(at(&mut s, Vec2::new(130.0, 0.0), false), Activity::Walk);
        assert_eq!(at(&mut s, Vec2::new(42.5, 0.0), true), Activity::CrouchWalk);
        assert_eq!(at(&mut s, Vec2::ZERO, true), Activity::CrouchIdle);
        assert_eq!(at(&mut s, Vec2::ZERO, false), Activity::Idle);
    }

    #[test]
    fn suffixes() {
        assert_eq!(suffix(Some("cs_source:weapon_ak47")), "AK");
        assert_eq!(suffix(Some("cs_source:weapon_knife")), "KNIFE");
        assert_eq!(suffix(Some("cs_source:weapon_usp")), "PISTOL");
        assert_eq!(suffix(None), "Pistol");
    }
}
