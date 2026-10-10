//! Bots and grenades, for any game whose grenades are `weapon::grenade`
//! `Throwable`s: planning a throw (`plan_throw`: the view pitch whose arc,
//! by the grenade's own throw rule, gravity and bounces, ends nearest a
//! target, checked against the world with segment traces), carrying it out
//! (`Toss`: draw the grenade, pull the pin, hold, release on aim, draw the
//! previous weapon again) and keeping out of flashes (`Avert`: look away
//! from one about to pop, its own or one it saw thrown).
//!
//! When bots throw is ours (CS:S's bot code isn't public): at a remembered
//! enemy out of sight (HE or flash), or at the objective it walks to
//! (smoke or flash), now and then, when an arc lands close enough.

use bevy::prelude::*;

use super::{Bot, BotConfig, wrap};
use crate::{
    core::Intent,
    weapon::{
        Inventory, Weapon,
        grenade::{Flight, GrenadeKind, ThrowRule, Throwable, bounce},
    },
};

/// A planned throw: the view angles to throw at, the arc, and where the
/// grenade should be when it goes off (or comes to rest, for smoke).
#[derive(Clone, Debug, PartialEq)]
pub struct GrenadePlan {
    pub kind: GrenadeKind,
    pub target: Vec3,
    /// View angles (radians; pitch positive up, as `Intent`).
    pub yaw: f32,
    pub pitch: f32,
    pub start: Vec3,
    pub velocity: Vec3,
    /// The arc, from the start through each bounce to `pop`.
    pub points: Vec<Vec3>,
    /// Where it goes off (blast, flash) or rests (smoke).
    pub pop: Vec3,
    /// Seconds from the throw to `pop`.
    pub time: f32,
    /// `pop`'s distance from the target, m.
    pub error: f32,
}

/// Gap kept off a touched surface, m.
const SKIN: f32 = 1e-3;
/// Seconds per trace segment along a planned arc.
const SEGMENT: f32 = 0.06;
/// Longest arc simulated, s.
const MAX_TIME: f32 = 4.0;
/// View pitches tried (degrees, positive up), coarse then fine.
const PITCH_RANGE: f32 = 80.0;
const COARSE_STEP: f32 = 2.5;
const FINE_STEP: f32 = 0.25;
/// Per second of flight added to an arc's score (prefer quick throws).
const TIME_WEIGHT: f32 = 0.15;

/// A sweep of the grenade's box against the world from one centre point
/// to another: the centre where it first touches, and the surface normal.
pub type Trace<'a> = dyn FnMut(Vec3, Vec3) -> Option<(Vec3, Vec3)> + 'a;

/// Follow a grenade from `start` with `velocity`: the parabola under
/// `flight.gravity` in `SEGMENT` steps, each traced; a hit bounces it by
/// `weapon::grenade::bounce`. Ends at `until` seconds (the fuse), or for
/// `rest` (smoke) once it has stopped (at most `MAX_TIME`). Returns the
/// points, the end and the time.
pub fn follow(
    start: Vec3,
    velocity: Vec3,
    flight: &Flight,
    until: f32,
    rest: bool,
    trace: &mut Trace,
) -> (Vec<Vec3>, Vec3, f32) {
    let g = Vec3::NEG_Y * flight.gravity;
    let (mut p, mut v, mut t) = (start, velocity, 0.0f32);
    let mut points = vec![p];
    let mut resting = false;
    let end = if rest { MAX_TIME } else { until.min(MAX_TIME) };
    for _ in 0..400 {
        if t >= end - 1e-4 || (resting && (rest || t >= until)) {
            break;
        }
        if resting {
            // Lying still: nothing moves until the fuse.
            t = end;
            break;
        }
        let dt = SEGMENT.min(end - t);
        let to = p + v * dt + 0.5 * g * dt * dt;
        match trace(p, to) {
            Some((hit, n)) => {
                let full = (to - p).length().max(1e-6);
                let tau = dt * ((hit - p).length() / full).clamp(0.0, 1.0);
                let (nv, stop) = bounce(v + g * tau, n, false, flight, 0.1 * 0.0254);
                p = hit + n * SKIN;
                v = nv;
                t += tau.max(1e-3);
                resting = stop;
                points.push(p);
            }
            None => {
                p = to;
                v += g * dt;
                t += dt;
                points.push(p);
            }
        }
    }
    (points, p, t.min(end))
}

/// When a grenade with `fuse` really goes off: at the first fuse check
/// (every `check` seconds, whole ticks of `dt`) strictly after it, as
/// `weapon::grenade` flies it (1.5 s → 1.56 s at CS:S's tick).
pub fn fuse_time(fuse: f32, check: f32, dt: f32) -> f32 {
    if dt <= 0.0 {
        return fuse;
    }
    let period = (check / dt).round().max(1.0) * dt;
    ((fuse / period + 1e-4).floor() + 1.0) * period
}

/// The throw from `eye` that ends nearest `target`: tries view pitches
/// (coarse, then fine around the best) toward the target's bearing, each
/// followed by `follow` with the grenade's rule (`fuse`; `rest` for
/// smoke). A start point inside a wall rules a pitch out. Scores prefer
/// small errors, then short flights. None if every pitch is ruled out.
#[allow(clippy::too_many_arguments)]
pub fn plan_throw(
    kind: GrenadeKind,
    rule: &ThrowRule,
    flight: &Flight,
    fuse: f32,
    rest: bool,
    eye: Vec3,
    target: Vec3,
    trace: &mut Trace,
) -> Option<GrenadePlan> {
    let to = target - eye;
    let yaw = (-to.x).atan2(-to.z);
    let mut try_pitch = |deg: f32| -> Option<GrenadePlan> {
        let pitch = deg.to_radians();
        let (start, velocity) = rule.throw(eye, yaw, pitch, Vec3::ZERO);
        if trace(eye, start).is_some() {
            return None;
        }
        let (points, pop, time) = follow(start, velocity, flight, fuse, rest, trace);
        Some(GrenadePlan {
            kind,
            target,
            yaw,
            pitch,
            start,
            velocity,
            points,
            pop,
            time,
            error: pop.distance(target),
        })
    };
    let score = |p: &GrenadePlan| p.error + TIME_WEIGHT * p.time;
    let better = |best: Option<GrenadePlan>, p: Option<GrenadePlan>| match (best, p) {
        (Some(b), Some(p)) => Some(if score(&p) < score(&b) { p } else { b }),
        (b, p) => b.or(p),
    };
    let mut best = None;
    let mut deg = -PITCH_RANGE;
    while deg <= PITCH_RANGE + 1e-3 {
        best = better(best, try_pitch(deg));
        deg += COARSE_STEP;
    }
    let centre = best.as_ref()?.pitch.to_degrees();
    let mut deg = centre - COARSE_STEP;
    while deg <= centre + COARSE_STEP + 1e-3 {
        if deg.abs() <= 89.0 {
            best = better(best, try_pitch(deg));
        }
        deg += FINE_STEP;
    }
    best
}

/// A throw being carried out.
#[derive(Clone, Debug)]
pub(super) struct Toss {
    pub weapon: Entity,
    /// Held before; drawn again after.
    pub previous: Option<Entity>,
    pub plan: GrenadePlan,
    pub started: f64,
    /// When the pin came out.
    pub pulled: Option<f64>,
    pub released: bool,
}

/// Looking away from a flash until it pops.
#[derive(Clone, Copy, Debug)]
pub(super) struct Avert {
    pub from: Vec3,
    pub until: f64,
    /// The flash in flight, followed while it exists.
    pub grenade: Option<Entity>,
}

/// Seconds the pin is held at least before the release.
const PIN_HOLD: f64 = 0.4;
/// Release once the aim is within this, radians.
const RELEASE_AIM: f32 = 0.6 * std::f32::consts::PI / 180.0;
/// A throw not done by then is given up, s.
const TOSS_TIMEOUT: f64 = 5.0;
/// Seconds between throws of a bot.
pub(super) const TOSS_COOLDOWN: (f64, f64) = (6.0, 12.0);
/// Seconds between looks for a chance to throw.
pub(super) const TOSS_CHECK: f64 = 0.5;
/// Chance per check of throwing at a remembered enemy, and at the
/// objective the bot walks to.
pub(super) const TOSS_CHANCE_LEAD: f32 = 0.2;
pub(super) const TOSS_CHANCE_OBJECTIVE: f32 = 0.06;
/// Chance per check while the attackers gather before their site.
pub(super) const TOSS_CHANCE_STAGING: f32 = 0.5;
/// Throwing distances (horizontal), m.
pub(super) const TOSS_RANGE: (f32, f32) = (4.0, 30.0);
/// Seconds after an enemy was last seen or heard that a grenade still
/// goes where they were (mashup's; CS:S bots throw at a fresh sighting).
pub(super) const TOSS_LEAD_MEMORY: f64 = 5.0;
/// Where to aim above a target on the floor, m: a blast at the body's
/// centre, a flash at head height (both may go off in the air), a smoke
/// on the floor.
pub(super) fn aim_lift(kind: GrenadeKind) -> f32 {
    match kind {
        GrenadeKind::Blast => 1.0,
        GrenadeKind::Flash => 1.5,
        GrenadeKind::Smoke => 0.0,
    }
}
/// A plan is used if it ends this close to the target, m.
pub(super) fn tolerance(kind: GrenadeKind) -> f32 {
    match kind {
        GrenadeKind::Blast => 2.0,
        GrenadeKind::Flash => 2.5,
        GrenadeKind::Smoke => 3.0,
    }
}
/// Seconds a finished plan stays drawn by `mashup_drawbots`.
pub(super) const PLAN_SHOWN: f64 = 3.0;
/// Chance of looking away from a flash seen in flight.
pub(super) const FLASH_REACT: f32 = 0.6;
/// Flashes farther than this aren't watched, m.
pub(super) const FLASH_WATCH: f32 = 25.0;

/// A teammate as a thrower sees it: eye, view direction, velocity, and
/// whether it is a person (who may turn toward a flash any moment).
#[derive(Clone, Copy, Debug)]
pub struct MateView {
    pub eye: Vec3,
    pub look: Vec3,
    pub velocity: Vec3,
    pub person: bool,
}

/// A flash that would blind a teammate longer than this is not thrown
/// (ours: a teammate facing away at arm's length gets 1.25 s by
/// `games::cs_source::grenades::flash_model`, which is allowed), s.
pub const TEAM_FLASH_TIME: f32 = 1.5;
/// Teammates are checked where they stand and where they will be this
/// long ahead at their speed (at most), s.
pub const TEAM_FLASH_AHEAD: f32 = 1.5;
/// A re-aimed flash goes this much farther along the throw, m.
pub(super) const FLASH_DEEPER: f32 = 3.0;

/// Whether the flash `plan` would blind any of `mates` (by the game's
/// `model`) for longer than `TEAM_FLASH_TIME`: from where each stands and
/// where it walks to by the pop, if `sight` (a clear line) reaches its eye.
/// People are taken as looking at least sideways at it (facing 0) since
/// they turn any moment; bots as they look now.
pub fn flashes_mate(
    plan: &GrenadePlan,
    model: crate::weapon::grenade::FlashModel,
    mates: &[MateView],
    sight: &dyn Fn(Vec3, Vec3) -> bool,
) -> bool {
    let ahead = (plan.time + PIN_HOLD as f32).min(TEAM_FLASH_AHEAD);
    mates.iter().any(|m| {
        [m.eye, m.eye + m.velocity.with_y(0.0) * ahead].into_iter().any(|eye| {
            let to = plan.pop - eye;
            let mut facing = m.look.dot(to.normalize_or_zero());
            if m.person {
                facing = facing.max(0.0);
            }
            model(to.length(), facing).is_some_and(|(_, hold, fade)| hold + fade > TEAM_FLASH_TIME) && sight(plan.pop, eye)
        })
    })
}

/// Weapons, and the grenade part of those that are grenades.
pub(super) type Arms<'w, 's> = Query<'w, 's, (&'static Weapon, Option<&'static Throwable>)>;

/// The grenades a bot holds: (weapon, its rule).
pub(super) fn held<'a>(inv: &Inventory, arms: &'a Arms) -> Vec<(Entity, &'a Throwable)> {
    inv.weapons
        .iter()
        .filter_map(|w| arms.get(*w).ok().and_then(|(_, t)| Some((*w, t?))))
        .filter(|(_, t)| t.count > 0)
        .collect()
}

/// Step a throw: draw, pull, aim, release, then draw the previous weapon.
/// Returns false once it's over (done or given up).
pub(super) fn step_toss(
    bot: &mut Bot,
    intent: &mut Intent,
    inv: &mut Inventory,
    arms: &Arms,
    cfg: &BotConfig,
    now: f64,
    dt: f32,
) -> bool {
    let Some(toss) = bot.toss.as_mut() else { return false };
    intent.fire = false;
    intent.move_axis = Vec2::ZERO;
    intent.jump = false;
    // Turn to the planned angles.
    let step = cfg.turn_rate.to_radians() * dt;
    intent.yaw = wrap(intent.yaw + wrap(toss.plan.yaw - intent.yaw).clamp(-step, step));
    intent.pitch += (toss.plan.pitch - intent.pitch).clamp(-step, step);
    let aimed = wrap(toss.plan.yaw - intent.yaw)
        .abs()
        .max((toss.plan.pitch - intent.pitch).abs())
        < RELEASE_AIM;
    let grenade = arms.get(toss.weapon).ok().and_then(|(_, t)| t);
    let over = |toss: &Toss, inv: &mut Inventory| {
        // Back to what was held (if it's still there and isn't this).
        let back = toss
            .previous
            .filter(|p| *p != toss.weapon && inv.weapons.contains(p))
            .or_else(|| best_weapon(inv, arms));
        if inv.active != back {
            inv.wanted = back;
        }
    };
    let Some(grenade) = grenade.filter(|_| inv.weapons.contains(&toss.weapon)) else {
        // Gone (spent, dropped): done if released.
        let toss = bot.toss.take().unwrap();
        over(&toss, inv);
        return false;
    };
    if now - toss.started > TOSS_TIMEOUT && !toss.released {
        let toss = bot.toss.take().unwrap();
        over(&toss, inv);
        return false;
    }
    if !toss.released {
        if inv.active != Some(toss.weapon) {
            inv.wanted = Some(toss.weapon);
            return true;
        }
        if grenade.pin {
            let pulled = *toss.pulled.get_or_insert(now);
            if now - pulled >= PIN_HOLD && aimed {
                toss.released = true;
            } else {
                intent.fire = true;
            }
        } else {
            intent.fire = true;
        }
        return true;
    }
    // Released: keep the aim until it leaves the hand.
    if grenade.throw_at.is_some() {
        return true;
    }
    let toss = bot.toss.take().unwrap();
    if toss.plan.kind == GrenadeKind::Flash {
        bot.avert = Some(Avert {
            from: toss.plan.pop,
            until: now + toss.plan.time as f64 + 0.3,
            grenade: None,
        });
    }
    over(&toss, inv);
    false
}

/// The weapon to hold when not throwing: the lowest slot that isn't a
/// grenade.
pub(super) fn best_weapon(inv: &Inventory, arms: &Arms) -> Option<Entity> {
    inv.weapons
        .iter()
        .filter_map(|w| arms.get(*w).ok().map(|a| (*w, a)))
        .filter(|(_, (_, t))| t.is_none())
        .min_by_key(|(_, (w, _))| w.slot)
        .map(|(e, _)| e)
}

#[cfg(test)]
mod tests_support {
    use super::*;

    pub const UNIT: f32 = 0.0254;

    /// CS:S's numbers (specs/cs_source/grenades.md 3-4), in meters.
    pub fn rule() -> ThrowRule {
        ThrowRule {
            pitch_lift: -10.0,
            pitch_scale: 100.0 / 90.0,
            speed_per_deg: 6.0 * UNIT,
            speed_max: 750.0 * UNIT,
            forward: 16.0 * UNIT,
        }
    }

    pub fn flight() -> Flight {
        Flight {
            half: 2.0 * UNIT,
            gravity: 320.0 * UNIT,
            elasticity: 0.45,
            player_elasticity: 0.3,
            elasticity_cap: 0.9,
            stop_speed: 30.0 * UNIT,
            floor_normal: 0.7,
            breakable_damage: 0.1,
            breakable_slowdown: 0.4,
            water_slowdown: 0.5,
            water_entry: 0.5,
            max_velocity: 3500.0 * UNIT,
            check_interval: 0.2,
            spin_roll: 0.0,
            spin_pitch: 0.0,
        }
    }

    /// Axis-aligned boxes (min, max) and the floor y = 0, swept by the
    /// grenade's box (so grown by its half size).
    pub fn world(boxes: Vec<(Vec3, Vec3)>) -> impl FnMut(Vec3, Vec3) -> Option<(Vec3, Vec3)> {
        let h = flight().half;
        move |a: Vec3, b: Vec3| {
            let d = b - a;
            let mut best: Option<(f32, Vec3)> = None;
            if a.y >= h && b.y < h {
                best = Some(((a.y - h) / (a.y - b.y), Vec3::Y));
            }
            for (lo, hi) in boxes
                .iter()
                .map(|(lo, hi)| (*lo - Vec3::splat(h), *hi + Vec3::splat(h)))
            {
                // Slab test, keeping the entry face.
                let (mut t0, mut t1, mut n) = (0.0f32, 1.0f32, Vec3::ZERO);
                let mut miss = false;
                for k in 0..3 {
                    if d[k].abs() < 1e-9 {
                        if a[k] < lo[k] || a[k] > hi[k] {
                            miss = true;
                        }
                        continue;
                    }
                    let (mut e, mut x) = ((lo[k] - a[k]) / d[k], (hi[k] - a[k]) / d[k]);
                    let mut face = Vec3::ZERO;
                    face[k] = -d[k].signum();
                    if e > x {
                        std::mem::swap(&mut e, &mut x);
                    }
                    if e > t0 {
                        t0 = e;
                        n = face;
                    }
                    t1 = t1.min(x);
                }
                if !miss && t0 <= t1 && n != Vec3::ZERO && best.is_none_or(|(t, _)| t0 < t) {
                    best = Some((t0, n));
                }
            }
            best.map(|(t, n)| (a + d * t, n))
        }
    }

    /// The grenade tick by tick (0.015 s, the game's own step and bounce),
    /// traced in the same world: where it is at `until`.
    pub fn replay(plan: &GrenadePlan, until: f32, trace: &mut Trace) -> (Vec3, Vec<Vec3>) {
        let f = flight();
        let (mut p, mut v) = (plan.start, plan.velocity);
        let mut path = vec![p];
        let dt = 0.015;
        let mut t = 0.0;
        let mut resting = false;
        while t < until {
            t += dt;
            if resting {
                continue;
            }
            let (step, nv) = crate::weapon::grenade::gravity_step(v, f.gravity, dt);
            match trace(p, p + step) {
                Some((hit, n)) => {
                    let (r, stop) = bounce(nv, n, false, &f, 0.1 * UNIT);
                    p = hit + n * 1e-3;
                    v = r;
                    resting = stop;
                }
                None => {
                    p += step;
                    v = nv;
                }
            }
            path.push(p);
        }
        (p, path)
    }
}

#[cfg(test)]
mod tests {
    use super::tests_support::*;
    use super::*;

    #[test]
    fn lands_near_the_target_on_flat_ground() {
        let eye = Vec3::new(0.0, 1.6, 0.0);
        for target in [
            Vec3::new(0.0, 0.0, -6.0),
            Vec3::new(9.0, 0.0, -9.0),
            Vec3::new(-15.0, 0.0, 4.0),
        ] {
            let mut trace = world(vec![]);
            let plan = plan_throw(
                GrenadeKind::Blast,
                &rule(),
                &flight(),
                1.56,
                false,
                eye,
                target,
                &mut trace,
            )
            .expect("a plan");
            assert!(plan.error < 1.0, "{target}: off by {}", plan.error);
            let (end, _) = replay(&plan, 1.56, &mut trace);
            assert!(end.distance(target) < 1.5, "{target}: replay ends at {end}");
        }
    }

    #[test]
    fn lobs_over_a_wall() {
        // A 3 m wall across the way 20 m out; the target (a body's centre)
        // 4 m behind it. CS throws are fast (a level view throws at 600
        // units/s), so this is an air burst over the wall; a wall that
        // close to a near target can't be cleared at all.
        let wall = (Vec3::new(-3.0, 0.0, -20.5), Vec3::new(3.0, 3.0, -19.5));
        let eye = Vec3::new(0.0, 1.6, 0.0);
        let target = Vec3::new(0.0, 1.0, -24.0);
        let mut trace = world(vec![wall]);
        let plan = plan_throw(
            GrenadeKind::Blast,
            &rule(),
            &flight(),
            1.56,
            false,
            eye,
            target,
            &mut trace,
        )
        .expect("a plan");
        assert!(plan.error < 2.0, "off by {}: {plan:?}", plan.error);
        assert!(plan.pitch > 0.0, "thrown upward over it");
        let (end, path) = replay(&plan, 1.56, &mut trace);
        assert!(end.distance(target) < 2.5, "replay ends at {end}");
        assert!(
            path.iter().all(|p| p.z > -19.5 || p.z < -20.5 || p.y > 3.0),
            "never through the wall"
        );
        // A low wall right in front rules low throws out but still finds
        // one.
        let mut trace = world(vec![wall, (Vec3::new(-3.0, 0.0, -1.0), Vec3::new(3.0, 1.2, -0.8))]);
        let plan = plan_throw(
            GrenadeKind::Flash,
            &rule(),
            &flight(),
            1.56,
            false,
            eye,
            target,
            &mut trace,
        )
        .expect("a plan");
        assert!(plan.error < 2.5, "{}", plan.error);
        // Too close behind too high a wall: the best is still far off, and
        // the caller's tolerance turns it down.
        let wall = (Vec3::new(-3.0, 0.0, -8.5), Vec3::new(3.0, 3.0, -7.5));
        let mut trace = world(vec![wall]);
        let near = Vec3::new(0.0, 1.0, -12.0);
        let plan = plan_throw(
            GrenadeKind::Blast,
            &rule(),
            &flight(),
            1.56,
            false,
            eye,
            near,
            &mut trace,
        )
        .expect("a plan");
        assert!(plan.error > 2.0, "{}", plan.error);
    }

    /// CS:S's flash model's shape (`games::cs_source::grenades::
    /// flash_model`): 1500 units, full within 60 degrees, a quarter facing
    /// away, up to 5 s.
    fn model(distance: f32, facing: f32) -> Option<(f32, f32, f32)> {
        let r = distance / UNIT;
        if r >= 1500.0 {
            return None;
        }
        let s = (1.0 - (r / 1500.0).powi(2)) * (0.25 + 0.75 * (facing + 0.5).clamp(0.0, 1.0));
        let time = 5.0 * s;
        (s >= 0.05).then(|| ((2.0 * s).min(1.0), time - time.min(3.0), time.min(3.0)))
    }

    #[test]
    fn flashes_spare_teammates_who_would_be_blinded() {
        let plan = GrenadePlan {
            kind: GrenadeKind::Flash,
            target: Vec3::new(0.0, 1.5, -15.0),
            yaw: 0.0,
            pitch: 0.0,
            start: Vec3::ZERO,
            velocity: Vec3::ZERO,
            points: vec![],
            pop: Vec3::new(0.0, 1.5, -15.0),
            time: 1.5,
            error: 0.0,
        };
        let clear = |_: Vec3, _: Vec3| true;
        let wall = |_: Vec3, _: Vec3| false;
        let mate = |eye: Vec3, look: Vec3, person: bool| MateView {
            eye,
            look,
            velocity: Vec3::ZERO,
            person,
        };
        let at = Vec3::new(2.0, 1.6, -3.0);
        // A bot teammate looking at where it pops: blinded.
        assert!(flashes_mate(&plan, model, &[mate(at, Vec3::NEG_Z, false)], &clear));
        // Behind a wall from it: not.
        assert!(!flashes_mate(&plan, model, &[mate(at, Vec3::NEG_Z, false)], &wall));
        // A bot looking away: a short white at most, allowed.
        assert!(!flashes_mate(&plan, model, &[mate(at, Vec3::Z, false)], &clear));
        // A person looking away may turn: spared all the same.
        assert!(flashes_mate(&plan, model, &[mate(at, Vec3::Z, true)], &clear));
        // Far beyond its reach: fine.
        let far = Vec3::new(0.0, 1.6, 30.0);
        assert!(!flashes_mate(&plan, model, &[mate(far, Vec3::NEG_Z, true)], &clear));
        // Running into it from out of reach: counted where it will be.
        let runner = MateView {
            velocity: Vec3::new(0.0, 0.0, -6.0),
            ..mate(Vec3::new(0.0, 1.6, 25.0), Vec3::NEG_Z, false)
        };
        assert!(flashes_mate(&plan, model, &[runner], &clear));
    }

    #[test]
    fn smoke_plans_where_it_comes_to_rest() {
        let eye = Vec3::new(0.0, 1.6, 0.0);
        let target = Vec3::new(4.0, 0.0, -10.0);
        let mut trace = world(vec![]);
        let plan = plan_throw(
            GrenadeKind::Smoke,
            &rule(),
            &flight(),
            1.56,
            true,
            eye,
            target,
            &mut trace,
        )
        .expect("a plan");
        assert!(plan.error < 1.5, "{}", plan.error);
        let (end, _) = replay(&plan, MAX_TIME, &mut trace);
        assert!(end.distance(plan.pop) < 1.0, "{end} vs {}", plan.pop);
    }
}
