//! Lag compensation (specs/cs_source/weapons.md 4.6; the Valve Developer
//! Wiki's "Lag compensation"): a network server traces a remote player's
//! shots against the other characters where that player saw them.
//!
//! - **History.** After every tick the server keeps each living
//!   character's hit volume for `sv_maxunlag` (1 s): origin, look yaw,
//!   its box (`ColliderAabb`, relative to the origin) and its hitboxes
//!   (`HitHistory`). The pose of tick T is the one clients get for T, so
//!   it is what they draw between ticks.
//! - **View time.** Each command carries the server tick (fractional) its
//!   client was drawing others at when it made it (`ViewTick`, set by the
//!   network layer; Source's command tick less the client's
//!   interpolation). Bots and a listen server's host have none: they see
//!   the present.
//! - **Rewind.** When that player fires, every other living character is
//!   put where it was at the view time, between the two kept ticks around
//!   it (`rewind`), at most `sv_maxunlag` back, never across a teleport
//!   (64 units between two ticks): its hitboxes, or its box when it has
//!   none, replace its collider for that shot's traces
//!   (`deliver::Shot::rewound`). Nothing is moved in the world, so nothing
//!   needs restoring. `sv_unlag 0` turns it off; `sv_showlagcompensation
//!   1` logs every rewind.

use std::{collections::VecDeque, sync::Arc};

use avian3d::prelude::ColliderAabb;
use bevy::prelude::*;

use crate::{
    console::resource_cvar,
    core::{Health, Hitbox, Hitboxes, Intent, NetRole, SimClock},
};

/// Source's teleport distance for lag compensation: 64 units, m.
pub const TELEPORT: f32 = 64.0 * 0.0254;

/// `sv_unlag`, `sv_maxunlag`, `sv_showlagcompensation` (Source's names
/// and defaults).
#[derive(Resource, Clone, Debug)]
pub struct LagCompSettings {
    /// 0: shots are traced against the present.
    pub unlag: u8,
    /// How far back a rewind may go, s.
    pub maxunlag: f32,
    /// 1: log every rewind.
    pub show: u8,
}

impl Default for LagCompSettings {
    fn default() -> Self {
        Self {
            unlag: 1,
            maxunlag: 1.0,
            show: 0,
        }
    }
}

/// What rewinds did (the server's `status`, tests).
#[derive(Resource, Clone, Debug, Default)]
pub struct LagCompStats {
    /// Shots traced in the past, how many were held at `sv_maxunlag`,
    /// characters not moved back across a teleport.
    pub rewinds: u64,
    pub clamped: u64,
    pub teleports: u64,
    /// The last rewind.
    pub last: Option<RewindInfo>,
}

/// One rewind: who fired on which tick, the view time asked for and the
/// one used (server ticks, fractional), and how many characters moved.
#[derive(Clone, Debug, PartialEq)]
pub struct RewindInfo {
    pub shooter: Entity,
    pub tick: u64,
    pub view_tick: f64,
    pub target_tick: f64,
    pub moved: usize,
}

impl RewindInfo {
    /// How far back the shot was traced, s.
    pub fn seconds_back(&self, step: f64) -> f64 {
        (self.tick as f64 - self.target_tick) * step
    }
}

/// On a character whose shots are lag compensated: the server tick its
/// client drew others at for this tick's command (the network layer sets
/// it per command; None: the present).
#[derive(Component, Clone, Copy, Debug, Default, PartialEq)]
pub struct ViewTick(pub Option<f64>);

/// One tick's hit volume of a character (`HitHistory`).
#[derive(Clone, Debug)]
pub struct HitFrame {
    pub tick: u64,
    pub origin: Vec3,
    /// The look yaw its hitboxes turn by.
    pub yaw: f32,
    /// Its box, relative to `origin`.
    pub min: Vec3,
    pub max: Vec3,
    /// Its hitboxes (character space), if it has any.
    pub hitboxes: Option<Arc<Vec<Hitbox>>>,
}

/// A living character's recent hit volumes, oldest first, one per tick
/// for `sv_maxunlag` (a network server only).
#[derive(Component, Clone, Debug, Default)]
pub struct HitHistory {
    pub frames: VecDeque<HitFrame>,
}

/// A character as a shot sees it after a rewind.
#[derive(Clone, Debug)]
pub struct Rewound {
    pub entity: Entity,
    pub origin: Vec3,
    pub yaw: f32,
    pub min: Vec3,
    pub max: Vec3,
    pub hitboxes: Option<Arc<Vec<Hitbox>>>,
}

impl Rewound {
    /// Its box in the world.
    pub fn aabb(&self) -> ColliderAabb {
        ColliderAabb {
            min: self.origin + self.min,
            max: self.origin + self.max,
        }
    }

    /// Where its hitboxes stand: the feet (the box's bottom under the
    /// origin) and the turn into character space.
    pub fn frame(&self) -> (Vec3, Quat) {
        (
            self.origin.with_y(self.origin.y + self.min.y),
            Quat::from_rotation_y(self.yaw).inverse(),
        )
    }
}

/// A character's box when it has no collider yet (the placeholder
/// capsule's, centred on the origin).
const DEFAULT_BOX: (Vec3, Vec3) = (Vec3::new(-0.4, -0.9, -0.4), Vec3::new(0.4, 0.9, 0.4));

pub(super) fn plugin(app: &mut App) {
    app.init_resource::<LagCompSettings>()
        .init_resource::<LagCompStats>()
        .add_systems(
            FixedLast,
            record
                .after(crate::map::SimPose)
                .run_if(resource_equals(NetRole::Server)),
        );
    resource_cvar::<LagCompSettings, u8>(
        app,
        "sv_unlag",
        "1: a remote player's shots hit others where that player saw them (lag compensation); 0: where they are now.",
        |s| &mut s.unlag,
    );
    resource_cvar::<LagCompSettings, f32>(
        app,
        "sv_maxunlag",
        "Seconds lag compensation may move others back, at most.",
        |s| &mut s.maxunlag,
    );
    resource_cvar::<LagCompSettings, u8>(
        app,
        "sv_showlagcompensation",
        "1: log every lag-compensated shot (view time, how far back, characters moved).",
        |s| &mut s.show,
    );
}

/// After each tick (a network server): every living character's hit
/// volume, kept `sv_maxunlag` long.
#[allow(clippy::type_complexity)]
fn record(
    clock: Res<SimClock>,
    settings: Res<LagCompSettings>,
    mut q: Query<(
        Entity,
        &Transform,
        &Intent,
        &Health,
        Option<&ColliderAabb>,
        Option<&Hitboxes>,
        Option<&mut HitHistory>,
    )>,
    mut commands: Commands,
) {
    let keep = ((settings.maxunlag.max(0.0) as f64 / clock.delta.as_secs_f64().max(1e-4)).ceil() as usize + 2).min(4096);
    for (e, t, intent, health, aabb, boxes, history) in &mut q {
        let Some(mut history) = history else {
            commands.entity(e).insert(HitHistory::default());
            continue;
        };
        if health.current <= 0.0 {
            // The dead aren't hit; a respawn starts over.
            history.frames.clear();
            continue;
        }
        let origin = t.translation;
        let (min, max) = aabb.map_or(DEFAULT_BOX, |b| (b.min - origin, b.max - origin));
        let hitboxes = boxes.map(|b| match history.frames.back().and_then(|f| f.hitboxes.clone()) {
            Some(last) if *last == b.0 => last,
            _ => Arc::new(b.0.clone()),
        });
        history.frames.push_back(HitFrame {
            tick: clock.tick,
            origin,
            yaw: intent.yaw,
            min,
            max,
            hitboxes,
        });
        while history.frames.len() > keep {
            history.frames.pop_front();
        }
    }
}

/// Where the characters in `histories` (all but `shooter`, alive now)
/// were at server tick `view` (fractional) for a shot on tick `tick`
/// (ticks `step` s long): the view time is held within `sv_maxunlag`
/// and the history kept, and a character isn't moved back across a
/// teleport (as far back as the history goes, the newest kept tick: the
/// last one clients heard).
pub fn rewind<'a>(
    shooter: Entity,
    view: f64,
    tick: u64,
    step: f64,
    settings: &LagCompSettings,
    histories: impl Iterator<Item = (Entity, &'a HitHistory)>,
    stats: &mut LagCompStats,
) -> Vec<Rewound> {
    let oldest = tick as f64 - settings.maxunlag.max(0.0) as f64 / step.max(1e-6);
    let mut target = view.min(tick as f64);
    if !target.is_finite() || target < oldest {
        target = if target.is_finite() { oldest } else { tick as f64 };
        stats.clamped += 1;
    }
    let mut out = Vec::new();
    for (e, h) in histories {
        if e == shooter {
            continue;
        }
        let Some(newest) = h.frames.back() else { continue };
        if target >= newest.tick as f64 {
            // As new as the history goes: the last tick clients heard.
            out.push(lerp(e, newest, newest, 0.0));
            continue;
        }
        // Back from the newest to the target, stopping at a teleport.
        let mut i = h.frames.len() - 1;
        let mut teleported = false;
        while i > 0 && h.frames[i].tick as f64 > target {
            if h.frames[i].origin.distance(h.frames[i - 1].origin) > TELEPORT {
                teleported = true;
                break;
            }
            i -= 1;
        }
        if teleported {
            stats.teleports += 1;
        }
        let a = &h.frames[i];
        let r = match h.frames.get(i + 1).filter(|_| !teleported && (a.tick as f64) <= target) {
            Some(b) => {
                let f = ((target - a.tick as f64) / (b.tick - a.tick).max(1) as f64).clamp(0.0, 1.0) as f32;
                lerp(e, a, b, f)
            }
            None => lerp(e, a, a, 0.0),
        };
        out.push(r);
    }
    stats.rewinds += 1;
    if settings.show > 0 {
        info!(
            "lag compensation: {shooter} on tick {tick} saw tick {view:.2}, traced at {target:.2} ({:.0} ms back), {} moved",
            (tick as f64 - target) * step * 1000.0,
            out.len()
        );
    }
    stats.last = Some(RewindInfo {
        shooter,
        tick,
        view_tick: view,
        target_tick: target,
        moved: out.len(),
    });
    out
}

/// Between two kept ticks: origin, box and hitboxes along a line, yaw the
/// short way round.
fn lerp(entity: Entity, a: &HitFrame, b: &HitFrame, f: f32) -> Rewound {
    let turn = (b.yaw - a.yaw + std::f32::consts::PI).rem_euclid(std::f32::consts::TAU) - std::f32::consts::PI;
    let hitboxes = match (&a.hitboxes, &b.hitboxes) {
        (Some(x), Some(y)) if f > 0.0 && x.len() == y.len() && !Arc::ptr_eq(x, y) => Some(Arc::new(
            x.iter()
                .zip(y.iter())
                .map(|(p, q)| Hitbox {
                    center: p.center.lerp(q.center, f),
                    half: p.half.lerp(q.half, f),
                    rotation: p.rotation.slerp(q.rotation, f),
                    group: p.group,
                })
                .collect(),
        )),
        (x, _) => x.clone(),
    };
    Rewound {
        entity,
        origin: a.origin.lerp(b.origin, f),
        yaw: a.yaw + turn * f,
        min: a.min.lerp(b.min, f),
        max: a.max.lerp(b.max, f),
        hitboxes,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(tick: u64, x: f32) -> HitFrame {
        HitFrame {
            tick,
            origin: Vec3::new(x, 0.0, 0.0),
            yaw: 0.0,
            min: Vec3::splat(-0.4),
            max: Vec3::splat(0.4),
            hitboxes: None,
        }
    }

    #[test]
    fn rewinds_between_ticks_within_maxunlag_and_not_across_a_teleport() {
        let e = Entity::from_raw_u32(7).unwrap();
        let shooter = Entity::from_raw_u32(8).unwrap();
        let h = HitHistory {
            frames: (90..100).map(|t| frame(t, t as f32 * 0.1)).collect(),
        };
        let settings = LagCompSettings::default();
        let mut stats = LagCompStats::default();
        let r = rewind(shooter, 95.5, 100, 0.015, &settings, [(e, &h)].into_iter(), &mut stats);
        assert_eq!(r.len(), 1);
        assert!((r[0].origin.x - 9.55).abs() < 1e-5, "{}", r[0].origin.x);
        // Held at sv_maxunlag (3 ticks of 0.015 s at most).
        let tight = LagCompSettings {
            maxunlag: 0.045,
            ..default()
        };
        let r = rewind(shooter, 91.0, 100, 0.015, &tight, [(e, &h)].into_iter(), &mut stats);
        assert_eq!(stats.clamped, 1);
        assert!((r[0].origin.x - 9.7).abs() < 1e-4, "{}", r[0].origin.x);
        // A teleport between ticks 97 and 98: no further back than 98.
        let mut h2 = h.clone();
        for f in h2.frames.iter_mut().filter(|f| f.tick >= 98) {
            f.origin.x += 5.0;
        }
        let r = rewind(shooter, 93.0, 100, 0.015, &settings, [(e, &h2)].into_iter(), &mut stats);
        assert_eq!(stats.teleports, 1);
        assert!((r[0].origin.x - 14.8).abs() < 1e-4, "{}", r[0].origin.x);
        // Newer than the history: the newest kept tick.
        let r = rewind(shooter, 99.5, 100, 0.015, &settings, [(e, &h)].into_iter(), &mut stats);
        assert!((r[0].origin.x - 9.9).abs() < 1e-5, "{}", r[0].origin.x);
    }
}
