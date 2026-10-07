//! What CS:S hostages play. Their models take every animation from HL2's
//! citizen models (`humans/male_*`, included; specs/cs_source/objectives.md
//! constants), so they move like HL2 citizens: `ACT_IDLE`, `ACT_WALK` or
//! `ACT_RUN` by speed, the move sequences steered by `move_yaw`, played
//! back at the speed the hostage really moves. The body turns toward where
//! it goes. Starting and stopping to follow plays a nod gesture over that.
//! CS:S's own hostage animation code is not public: the thresholds, turn
//! rate and gestures are ours.

use bevy::prelude::*;

use crate::{
    core::{Health, Intent, Velocity},
    map::anim::{Animator, Layer},
    objectives::hostages::Hostage,
};

const UNIT: f32 = 0.0254;
/// Slower than this (units/s) stands idle.
const IDLE_BELOW: f32 = 10.0;
/// Faster than this runs; between, walks (the follow rules walk at 100
/// and run at 250).
const RUN_ABOVE: f32 = 150.0;
/// How fast the body turns toward where it moves, degrees/s.
const TURN_RATE: f32 = 360.0;
/// Playback rate bounds when matching the authored ground speed.
const RATE_MIN: f32 = 0.3;
const RATE_MAX: f32 = 2.5;
/// Gestures when it starts and stops following.
pub const FOLLOW_GESTURE: &str = "hg_nod_right";
pub const STOP_GESTURE: &str = "hg_nod_left";

/// One hostage's animation state.
#[derive(Component, Debug, Clone, Default)]
pub struct HostageAnim {
    /// Body yaw (radians, as `Intent::yaw`); None until the first update.
    pub yaw: Option<f32>,
    /// The activity playing (`ACT_IDLE`, `ACT_WALK`, `ACT_RUN`).
    pub activity: &'static str,
    /// Whom it followed at the last update.
    leader: Option<Entity>,
    /// The gesture layer: sequence and cycle.
    gesture: Option<(usize, f32)>,
}

/// The activity for a horizontal speed (units/s).
pub fn activity(speed: f32) -> &'static str {
    if speed < IDLE_BELOW {
        "ACT_IDLE"
    } else if speed < RUN_ABOVE {
        "ACT_WALK"
    } else {
        "ACT_RUN"
    }
}

/// Angle in radians into (-π, π].
fn wrap(a: f32) -> f32 {
    let t = std::f32::consts::TAU;
    let a = a.rem_euclid(t);
    if a > std::f32::consts::PI { a - t } else { a }
}

#[allow(clippy::type_complexity)]
pub(super) fn drive(
    time: Res<Time>,
    mut hostages: Query<(
        Entity,
        &Hostage,
        &mut Animator,
        Option<&mut HostageAnim>,
        &Intent,
        &Velocity,
        Option<&Health>,
    )>,
    mut commands: Commands,
) {
    let (dt, now) = (time.delta_secs(), time.elapsed_secs_f64());
    for (e, hostage, mut a, state, intent, velocity, health) in &mut hostages {
        let Some(mut state) = state else {
            commands.entity(e).insert(HostageAnim::default());
            continue;
        };
        if health.is_some_and(|h| h.current <= 0.0) {
            continue;
        }
        let set = a.set.clone();
        let v = Vec2::new(velocity.0.x, velocity.0.z) / UNIT;
        let speed = v.length();
        // Main sequence by speed.
        let act = activity(speed);
        if let Some(s) = set.pick_activity(act, 0.0)
            && (act != state.activity || a.main.is_none())
        {
            a.play(s, now);
        }
        state.activity = act;
        // The body turns toward where it moves.
        let mut yaw = state.yaw.unwrap_or(intent.yaw);
        let heading = (-v.x).atan2(-v.y);
        if speed >= IDLE_BELOW {
            let diff = wrap(heading - yaw);
            let step = TURN_RATE.to_radians() * dt;
            yaw = wrap(yaw + diff.clamp(-step, step));
        }
        state.yaw = Some(yaw);
        a.yaw = Some(yaw);
        let move_yaw = if speed >= IDLE_BELOW {
            wrap(heading - yaw).to_degrees()
        } else {
            0.0
        };
        if let Some(p) = set.param("move_yaw") {
            a.set_param(p, move_yaw);
        }
        // Playback at the real speed over the authored one.
        a.playback = match a.main {
            Some(s) if speed >= IDLE_BELOW => {
                let ground = set.ground_speed(s, &a.params);
                if ground > 1.0 {
                    (speed / ground).clamp(RATE_MIN, RATE_MAX)
                } else {
                    1.0
                }
            }
            _ => 1.0,
        };
        a.advance(dt, now);
        // A gesture when it starts or stops following.
        if hostage.leader != state.leader {
            let name = if hostage.leader.is_some() {
                FOLLOW_GESTURE
            } else {
                STOP_GESTURE
            };
            state.gesture = set.sequence(name).map(|s| (s, 0.0));
            state.leader = hostage.leader;
        } else if let Some((s, c)) = &mut state.gesture {
            *c += set.cycle_rate(*s, &a.params) * dt;
            if *c >= 1.0 {
                state.gesture = None;
            }
        }
        a.layers.resize(1, None);
        a.layers[0] = state.gesture.map(|(sequence, cycle)| Layer {
            sequence,
            cycle,
            weight: 1.0,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn activities_by_speed() {
        assert_eq!(activity(0.0), "ACT_IDLE");
        assert_eq!(activity(100.0), "ACT_WALK");
        assert_eq!(activity(250.0), "ACT_RUN");
        assert!((wrap(3.5) - (3.5 - std::f32::consts::TAU)).abs() < 1e-6);
    }
}
