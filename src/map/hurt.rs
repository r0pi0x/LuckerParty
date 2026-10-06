//! Hurt volumes (Source `trigger_hurt`): characters touching one lose
//! health at its rate, in half-second bites (specs/cs_source/fall_damage.md,
//! "trigger_hurt", measured on de_port).

use bevy::prelude::*;

use crate::core::{Damage, Health, Hitgroup, Intent, MapBrush};

/// One hurt volume: convex brushes (engine space) and its damage rate.
#[derive(Clone, Debug)]
pub struct MapHurtVolume {
    pub brushes: Vec<MapBrush>,
    /// Health points per second (100 = a standard player's full health).
    pub damage_per_second: f32,
}

/// The loaded map's hurt volumes, with when each next bites (sim seconds).
#[derive(Resource, Clone, Debug, Default)]
pub struct MapHurt {
    pub volumes: Vec<MapHurtVolume>,
    next: Vec<f64>,
}

impl MapHurt {
    pub fn new(volumes: Vec<MapHurtVolume>) -> Self {
        let next = vec![f64::NEG_INFINITY; volumes.len()];
        Self { volumes, next }
    }
}

/// Seconds between bites; each takes half the per-second damage.
const INTERVAL: f64 = 0.5;

/// Whether an axis-aligned box (engine min/max) overlaps a convex brush.
fn overlaps(brush: &MapBrush, min: Vec3, max: Vec3) -> bool {
    if brush.max.cmplt(min).any() || brush.min.cmpgt(max).any() {
        return false;
    }
    let (centre, half) = ((min + max) / 2.0, (max - min) / 2.0);
    brush.planes.iter().all(|(n, d)| n.dot(centre) - n.abs().dot(half) < *d)
}

/// Every volume with a living character touching it bites everyone in it
/// once per interval; the first touch after a quiet spell bites at once.
/// Measured: 25 damage every 33 ticks at the 0.015 s tick (0.495 s) for a
/// rate of 50, so a bite is due within half a tick of the interval.
pub(super) fn hurt_characters(
    hurt: Option<ResMut<MapHurt>>,
    time: Res<Time>,
    characters: Query<(Entity, &Transform, &avian3d::prelude::ColliderAabb, &Health), With<Intent>>,
    mut damage: MessageWriter<Damage>,
) {
    let Some(mut hurt) = hurt else { return };
    let now = time.elapsed_secs_f64();
    let slack = time.delta_secs_f64() / 2.0;
    let MapHurt { volumes, next } = &mut *hurt;
    for (volume, next) in volumes.iter().zip(next.iter_mut()) {
        if now + slack < *next {
            continue;
        }
        let mut bit = false;
        for (e, at, aabb, health) in &characters {
            if health.current <= 0.0 || !volume.brushes.iter().any(|b| overlaps(b, aabb.min, aabb.max)) {
                continue;
            }
            bit = true;
            damage.write(Damage {
                target: e,
                attacker: None,
                amount: volume.damage_per_second * INTERVAL as f32 / 100.0,
                point: at.translation,
                dir: Vec3::NEG_Y,
                hitgroup: Hitgroup::Generic,
            });
        }
        if bit {
            *next = now + INTERVAL;
        }
    }
}
