//! Players and multiplayer physics props (specs/cs_source/physics_props.md
//! 4.2): players walk through them; each tick a player shoves the props
//! around them (an impulse at the player's centre, so they spin too), and
//! "solid"-mode props push the player back through the move input.

use avian3d::prelude::*;
use bevy::prelude::*;

use super::movement::{SourceMovement, SourceMovementConfig, to_source};
use crate::{
    core::{SimSet, Velocity},
    map::{PhysicsProp, PushAway},
};

/// The query box is the player's box grown by this much, units.
const QUERY_EXPAND: f32 = 3.0;
/// "solid"-mode props aren't shoved by players slower than this (units/s).
const MIN_PLAYER_SPEED: f32 = 75.0;
const PUSH_FORCE: f32 = 30_000.0;
const PUSH_MAX: f32 = 1_000.0;
const PUSHBACK_FORCE: f32 = 200_000.0;
const PUSHBACK_MAX: f32 = 10_000.0;
/// Applied to every client push-back (a quirk the game keeps).
const PUSHBACK_SCALE: f32 = 0.25;
const PUSHBACK_GAP: f32 = 5.0;
const HALF_WIDTH: f32 = 16.0;
/// The movement's transform sits this far above the feet (units).
const ORIGIN_ABOVE_FEET: f32 = 36.0;

pub struct PushAwayPlugin;

impl Plugin for PushAwayPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(FixedUpdate, push_props.after(SimSet::Movement));
    }
}

/// A prop's world box in Source units (its rotated bounds' extent).
pub fn prop_box(transform: &Transform, bounds: (Vec3, Vec3)) -> (Vec3, Vec3) {
    let (lo, hi) = bounds;
    let corners = (0..8).map(|i| {
        let c = Vec3::new(
            if i & 1 == 0 { lo.x } else { hi.x },
            if i & 2 == 0 { lo.y } else { hi.y },
            if i & 4 == 0 { lo.z } else { hi.z },
        );
        to_source(transform.transform_point(c))
    });
    corners.fold((Vec3::MAX, Vec3::MIN), |(a, b), c| (a.min(c), b.max(c)))
}

/// The player's box (Source units) from the movement transform.
pub fn player_box(transform: &Transform, me: &SourceMovement, cfg: &SourceMovementConfig) -> (Vec3, Vec3) {
    let feet = to_source(transform.translation) - Vec3::Z * ORIGIN_ABOVE_FEET;
    let height = if me.ducked { cfg.duck_height } else { cfg.stand_height };
    (
        feet + Vec3::new(-HALF_WIDTH, -HALF_WIDTH, 0.0),
        feet + Vec3::new(HALF_WIDTH, HALF_WIDTH, height),
    )
}

fn overlap(a: (Vec3, Vec3), b: (Vec3, Vec3)) -> bool {
    a.0.cmple(b.1).all() && b.0.cmple(a.1).all()
}

/// Shove props away from players (spec 4.2.2).
fn push_props(
    players: Query<(&Transform, &Velocity, &SourceMovement)>,
    mut props: Query<(&Transform, &PhysicsProp, Forces), Without<SourceMovement>>,
    cfg: Res<SourceMovementConfig>,
) {
    for (pt, pv, me) in &players {
        let mut player = player_box(pt, me, &cfg);
        let centre = (player.0 + player.1) / 2.0;
        player.0 -= Vec3::splat(QUERY_EXPAND);
        player.1 += Vec3::splat(QUERY_EXPAND);
        let speed = to_source(pv.0).truncate().length();
        for (t, p, mut forces) in &mut props {
            if matches!(p.push, PushAway::Collide | PushAway::Ignore) {
                continue;
            }
            if p.push == PushAway::Solid && speed < MIN_PLAYER_SPEED {
                continue;
            }
            let b = prop_box(t, p.bounds);
            if !overlap(player, b) {
                continue;
            }
            // kg.in/s to kg.m/s (to_engine scales units to meters), at the
            // player's centre, so the prop spins too.
            let impulse = super::movement::to_engine(push_impulse(centre, (b.0 + b.1) / 2.0));
            let at = super::movement::to_engine(centre);
            forces.apply_linear_impulse_at_point(impulse, at);
        }
    }
}

/// The impulse a player at `player` (centre, units) gives a prop centred at
/// `prop`: horizontal, away from the player, min(30000 / dist, 1000)
/// kg.units/s (dist at least 1; none when exactly above or below).
pub fn push_impulse(player: Vec3, prop: Vec3) -> Vec3 {
    let mut d = prop - player;
    d.z = 0.0;
    let dist = d.length().max(1.0);
    d.normalize_or_zero() * (PUSH_FORCE / dist).min(PUSH_MAX)
}

/// Push-back from "solid"-mode props near the player (spec 4.2.3),
/// as additions to (forward, side) move input, in key units.
pub fn push_back<'a>(
    player: (Vec3, Vec3),
    forward: Vec3,
    right: Vec3,
    props: impl Iterator<Item = (&'a Transform, &'a PhysicsProp)>,
) -> Vec2 {
    let centre = (player.0 + player.1) / 2.0;
    let mut out = Vec2::ZERO;
    for (t, p) in props {
        if p.push != PushAway::Solid {
            continue;
        }
        let b = prop_box(t, p.bounds);
        // Props within the gap count (the spec's cases: 2 units pushes, 6
        // doesn't).
        let reach = (
            player.0 - Vec3::splat(PUSHBACK_GAP),
            player.1 + Vec3::splat(PUSHBACK_GAP),
        );
        if !overlap(reach, b) {
            continue;
        }
        let w = p.mass.clamp(10.0, 30.0) / 30.0;
        let a = centre.clamp(b.0, b.1);
        let near = a.clamp(player.0, player.1);
        let mut v = near - a;
        let mut dist = v.length();
        let inside = a.cmpge(player.0).all() && a.cmple(player.1).all();
        if dist > PUSHBACK_GAP && !inside {
            continue;
        }
        if v == Vec3::ZERO {
            v = centre - a;
            if v == Vec3::ZERO {
                v = centre - (b.0 + b.1) / 2.0;
            }
            dist = v.length();
        }
        let n = v.normalize_or_zero();
        let dist = dist.max(1.0);
        let f = (PUSHBACK_FORCE * w / dist).min(PUSHBACK_MAX) * PUSHBACK_SCALE;
        out += Vec2::new(f * n.dot(forward), f * n.dot(right));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::games::cs_source::movement::to_engine;

    #[test]
    fn server_push() {
        let player = Vec3::new(0.0, 0.0, 36.0);
        // 10 kg prop centred at (20, 0, 10): J = 1000 along +X (dv 100 in/s).
        assert!((push_impulse(player, Vec3::new(20.0, 0.0, 10.0)) - Vec3::new(1000.0, 0.0, 0.0)).length() < 1e-3);
        // At 40 units: 750.
        assert!((push_impulse(player, Vec3::new(40.0, 0.0, 10.0)).x - 750.0).abs() < 1e-3);
        // Closer than 1: the distance counts as 1, capped at 1000.
        assert!((push_impulse(player, Vec3::new(0.5, 0.0, 10.0)).x - 1000.0).abs() < 1e-3);
        // Straight above: no direction, no push.
        assert_eq!(push_impulse(player, Vec3::new(0.0, 0.0, 80.0)), Vec3::ZERO);
    }

    /// A 20 kg prop whose box face is at x = 18, in front of a player at the
    /// origin looking along +X.
    fn pushback(face: f32, mass: f32, push: PushAway) -> Vec2 {
        let player = (Vec3::new(-16.0, -16.0, 0.0), Vec3::new(16.0, 16.0, 72.0));
        // A prop box from x = face to face + 20, y -10..10, z 0..40, placed
        // so its world box is exactly that (unit model space, identity).
        let lo = to_engine(Vec3::new(face, -10.0, 0.0));
        let hi = to_engine(Vec3::new(face + 20.0, 10.0, 40.0));
        let prop = PhysicsProp {
            push,
            mass,
            bounds: (lo.min(hi), lo.max(hi)),
        };
        let t = Transform::IDENTITY;
        push_back(player, Vec3::X, Vec3::NEG_Y, [(&t, &prop)].into_iter())
    }

    #[test]
    fn client_push_back() {
        // dist 2, w = 20/30: F = min(66667, 10000) x 0.25 = 2500, backwards.
        let f = pushback(18.0, 20.0, PushAway::Solid);
        assert!((f.x + 2500.0).abs() < 1e-2 && f.y.abs() < 1e-3, "{f}");
        // 50 kg: the same (cap).
        assert!((pushback(18.0, 50.0, PushAway::Solid).x + 2500.0).abs() < 1e-2);
        // A 6-unit gap: too far.
        assert_eq!(pushback(22.0, 20.0, PushAway::Solid), Vec2::ZERO);
        // Non-solid props don't push back.
        assert_eq!(pushback(18.0, 20.0, PushAway::NonSolid), Vec2::ZERO);
    }
}
