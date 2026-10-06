//! Where CS:S's view model sits relative to the eye
//! (specs/cs_source/view_models.md 1, 3, 4): the walking bob (the
//! cvar-driven model, tied to distance travelled) and the sway that lags
//! the view's rotation by `cl_wpn_sway_interp`, applied in that order on
//! top of the eye angles (which already include the view punch). Writes the
//! local player's `map::ViewModelOffset`; `map::view_model` mirrors it with
//! the model.
//!
//! Angles here are Source's: degrees (pitch, yaw, roll), pitch positive
//! looking down, yaw positive turning left; vectors in Source axes (x
//! forward, y left, z up) and units.

use std::collections::VecDeque;

use bevy::prelude::*;

use crate::{
    core::{Intent, LocalPlayer, Velocity},
    map::ViewModelOffset,
    weapon::ViewPunch,
};

const UNIT: f32 = 0.0254;

/// The bob's speed clamp and slew limit (units/s, units/s^2).
pub const BOB_SPEED_MAX: f32 = 320.0;
/// Bob amplitude per unit/s of speed.
pub const BOB_AMP: f32 = 0.005;
/// Clamp on each bob value.
pub const BOB_CLAMP: (f32, f32) = (-7.0, 4.0);
/// Offsets and angles per unit of vertical (V) or lateral (L) bob.
pub const BOB_FORWARD: f32 = 0.4;
pub const BOB_UP: f32 = 0.1;
pub const BOB_ROLL: f32 = 0.5;
pub const BOB_PITCH: f32 = -0.4;
pub const BOB_YAW: f32 = -0.3;
pub const BOB_RIGHT: f32 = 0.2;
/// Sway history kept beyond the lag (s).
const SWAY_KEEP: f64 = 0.05;

/// The motion cvars (CS:S defaults): `cl_bobcycle`, `cl_bobup`,
/// `cl_wpn_sway_interp`, `cl_wpn_sway_scale`.
#[derive(Resource, Clone, Copy, Debug, PartialEq)]
pub struct ViewMotion {
    pub bobcycle: f32,
    pub bobup: f32,
    pub sway_interp: f32,
    pub sway_scale: f32,
}

impl Default for ViewMotion {
    fn default() -> Self {
        Self {
            bobcycle: 0.8,
            bobup: 0.5,
            sway_interp: 0.1,
            sway_scale: 1.0,
        }
    }
}

/// One bob wave at bob time `t_b` (s) for `speed` (units/s): period
/// `period`, rising for the fraction `up` of each cycle (spec 3.5–3.7).
pub fn bob_wave(t_b: f32, speed: f32, period: f32, up: f32) -> f32 {
    let period = if period <= 0.0 { 0.01 } else { period };
    let up = if up <= 0.0 { 0.01 } else { up };
    let c = (t_b / period).fract();
    let theta = if c < up {
        std::f32::consts::PI * c / up
    } else {
        std::f32::consts::PI + std::f32::consts::PI * (c - up) / (1.0 - up)
    };
    (BOB_AMP * speed * (0.3 + 0.7 * theta.sin())).clamp(BOB_CLAMP.0, BOB_CLAMP.1)
}

/// The bob's state between frames (spec 3, "State").
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Bob {
    /// Bob time T_b.
    pub time: f32,
    pub last_time: f64,
    pub last_speed: f32,
    /// Vertical and lateral bob (V, L).
    pub vertical: f32,
    pub lateral: f32,
}

impl Bob {
    /// One frame at time `now` (`frametime` 0: paused, nothing changes)
    /// with the player's horizontal `speed` (units/s).
    pub fn update(&mut self, now: f64, frametime: f32, speed: f32, motion: &ViewMotion) {
        if frametime == 0.0 {
            return;
        }
        let dt = (now - self.last_time) as f32;
        let slew = (dt * BOB_SPEED_MAX).max(0.0);
        let s = speed
            .clamp(self.last_speed - slew, self.last_speed + slew)
            .clamp(-BOB_SPEED_MAX, BOB_SPEED_MAX);
        self.last_speed = s;
        self.time += dt * s / BOB_SPEED_MAX;
        self.last_time = now;
        self.vertical = bob_wave(self.time, s, motion.bobcycle, motion.bobup);
        self.lateral = bob_wave(self.time, s, 2.0 * motion.bobcycle, motion.bobup);
    }
}

/// What a bob adds (spec 3, "Applying it"): offsets along the view's
/// forward and right and along world up (units), and angles (degrees).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct BobOffset {
    pub forward: f32,
    pub right: f32,
    pub world_up: f32,
    /// (pitch, yaw, roll).
    pub angles: Vec3,
}

pub fn bob_offset(vertical: f32, lateral: f32) -> BobOffset {
    BobOffset {
        forward: BOB_FORWARD * vertical,
        right: BOB_RIGHT * lateral,
        world_up: BOB_UP * vertical,
        angles: Vec3::new(BOB_PITCH * vertical, BOB_YAW * lateral, BOB_ROLL * vertical),
    }
}

/// Source angles (pitch, yaw, roll; degrees) as a rotation in Source axes.
pub fn rotation(angles: Vec3) -> Quat {
    let a = angles * std::f32::consts::PI / 180.0;
    Quat::from_euler(EulerRot::ZYX, a.y, a.x, a.z)
}

/// The angles of a rotation in Source axes.
pub fn angles(q: Quat) -> Vec3 {
    let (yaw, pitch, roll) = q.to_euler(EulerRot::ZYX);
    Vec3::new(pitch, yaw, roll) * 180.0 / std::f32::consts::PI
}

/// The sway offset (spec 4.4–4.7) in the frame of the current angles:
/// (forward, right, up), units.
pub fn sway_offset(now: Vec3, lagged: Vec3, scale: f32) -> Vec3 {
    let d = (lagged - now) * std::f32::consts::PI / 180.0;
    // Forward of the angles -D.
    let g = Vec3::new((-d.x).cos() * (-d.y).cos(), (-d.x).cos() * (-d.y).sin(), -(-d.x).sin());
    let k = scale * (Vec3::X - g);
    Vec3::new(k.x, -k.y, k.z)
}

/// Recent view-model angles, for the lagged value (spec 4.2–4.3).
#[derive(Clone, Debug, Default)]
pub struct SwayHistory {
    samples: VecDeque<(f64, Quat)>,
}

impl SwayHistory {
    /// Add the angles at `now` and return those `interp` seconds ago.
    pub fn push(&mut self, now: f64, angles: Vec3, interp: f32) -> Vec3 {
        self.samples.push_back((now, rotation(angles)));
        let target = now - interp as f64;
        // Drop what is older than the lag plus a margin, keeping one
        // sample at or before the lagged time to interpolate from.
        while self.samples.len() > 1 && self.samples[1].0 < target - SWAY_KEEP {
            self.samples.pop_front();
        }
        self.at(target)
    }

    /// The (slerped) angles at time `t`; the oldest sample before the
    /// history starts.
    pub fn at(&self, t: f64) -> Vec3 {
        let Some(&(t0, q0)) = self.samples.front() else {
            return Vec3::ZERO;
        };
        if t <= t0 || self.samples.len() == 1 {
            return angles(q0);
        }
        for w in self.samples.iter().zip(self.samples.iter().skip(1)) {
            let ((ta, qa), (tb, qb)) = (*w.0, *w.1);
            if t >= ta && t <= tb {
                let f = if tb > ta { ((t - ta) / (tb - ta)) as f32 } else { 1.0 };
                return angles(qa.slerp(qb, f));
            }
        }
        angles(self.samples.back().unwrap().1)
    }
}

/// The view model's placement relative to the eye, in Source eye axes
/// (units; rotation): bob then sway, for eye angles `eye`.
pub fn placement(eye: Vec3, bob: &Bob, history: &mut SwayHistory, now: f64, motion: &ViewMotion) -> (Vec3, Quat) {
    let b = bob_offset(bob.vertical, bob.lateral);
    let pitch = eye.x.to_radians();
    // World up seen from the eye (roll 0): forward -sin p, up cos p.
    let mut offset = Vec3::new(b.forward - b.world_up * pitch.sin(), -b.right, b.world_up * pitch.cos());
    let bobbed = eye + b.angles;
    let relative = rotation(eye).inverse() * rotation(bobbed);
    if motion.sway_interp > 0.0 {
        let lagged = history.push(now, bobbed, motion.sway_interp);
        let s = sway_offset(bobbed, lagged, motion.sway_scale);
        // (forward, right, up) in the bobbed frame, to Source axes.
        offset += relative * Vec3::new(s.x, -s.y, s.z);
    }
    (offset, relative)
}

/// Source eye axes and units to the camera's (X right, Y up, -Z forward;
/// meters).
pub fn to_camera(offset: Vec3, rotation: Quat) -> ViewModelOffset {
    ViewModelOffset {
        translation: Vec3::new(-offset.y, offset.z, -offset.x) * UNIT,
        rotation: Quat::from_xyzw(-rotation.y, rotation.z, -rotation.x, rotation.w),
    }
}

/// The local player's bob and sway state.
#[derive(Component, Default)]
pub struct ViewMotionState {
    bob: Bob,
    history: SwayHistory,
}

/// Each frame: the local player's view-model offset.
pub fn update(
    time: Res<Time>,
    motion: Res<ViewMotion>,
    mut players: Query<
        (
            Entity,
            &Intent,
            &Velocity,
            Option<&ViewPunch>,
            Option<&mut ViewMotionState>,
            Option<&mut ViewModelOffset>,
        ),
        With<LocalPlayer>,
    >,
    mut commands: Commands,
) {
    let now = time.elapsed_secs_f64();
    for (e, intent, velocity, punch, state, offset) in &mut players {
        let (Some(mut state), Some(mut offset)) = (state, offset) else {
            commands.entity(e).insert((
                ViewMotionState {
                    bob: Bob {
                        last_time: now,
                        ..default()
                    },
                    ..default()
                },
                ViewModelOffset::default(),
            ));
            continue;
        };
        let speed = Vec2::new(velocity.0.x, velocity.0.z).length() / UNIT;
        let state = &mut *state;
        state.bob.update(now, time.delta_secs(), speed, &motion);
        let p = punch.map_or(Vec2::ZERO, |p| p.0);
        let eye = Vec3::new(-(intent.pitch + p.x), intent.yaw + p.y, 0.0) * 180.0 / std::f32::consts::PI;
        let (o, q) = placement(eye, &state.bob, &mut state.history, now, &motion);
        let next = to_camera(o, q);
        offset.set_if_neq(next);
    }
}
