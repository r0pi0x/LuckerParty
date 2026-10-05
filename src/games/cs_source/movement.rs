//! Source player movement, from specs/cs_source/movement.md: a swept-box
//! kinematic controller. Friction and acceleration toward a wish direction,
//! air acceleration with the 30-unit cap that makes air-strafing work,
//! split gravity, jumping, ground detection, slide moves with crease
//! handling, stair stepping, sticking to slopes and stairs, and ducking.
//!
//! The maths runs in Source units (inches, Z up) like the spec; the
//! character's `Transform` (engine meters, Y up) is the centre of the
//! standing box, so feet = origin - 36 units. Not implemented yet: water,
//! ladders, base velocity (conveyors), view punch, fall damage (the landing
//! speed is published for it).

use avian3d::prelude::*;
use bevy::prelude::*;

use crate::{
    core::{Intent, MovementState, SimSet, Velocity},
    map::{MapBrushCollider, MapBrushes},
    slots::RegisterSlots,
};

pub const ID: &str = "cs_source:movement";

const METERS_PER_UNIT: f32 = 0.0254;

/// Console variables and per-player values the movement reads. `Default`
/// is CS:S as measured on the reference install (RCON, 2026-10-05; see the
/// spec's "CS:S values"); `shared_code` is the SDK's values, which the
/// spec's test cases use.
#[derive(Resource, Clone, Debug)]
pub struct SourceMovementConfig {
    pub accelerate: f32,
    pub airaccelerate: f32,
    pub friction: f32,
    pub stopspeed: f32,
    pub gravity: f32,
    pub maxspeed: f32,
    pub stepsize: f32,
    pub maxvelocity: f32,
    pub bounce: f32,
    /// Jump speed, units/s.
    pub jump_impulse: f32,
    /// The held weapon's speed (knife: 250) until a weapon slot sets it.
    pub player_maxspeed: f32,
    /// What a full move key sends (cl_forwardspeed etc.), rescaled to the max speed.
    pub key_speed: f32,
}

impl SourceMovementConfig {
    /// The SDK 2013 shared movement code's values.
    pub fn shared_code() -> Self {
        Self {
            accelerate: 5.0,
            airaccelerate: 10.0,
            friction: 4.0,
            stopspeed: 100.0,
            gravity: 800.0,
            maxspeed: 320.0,
            stepsize: 18.0,
            maxvelocity: 3500.0,
            bounce: 0.0,
            jump_impulse: 268.328_16,
            player_maxspeed: 250.0,
            key_speed: 450.0,
        }
    }
}

impl Default for SourceMovementConfig {
    /// CS:S: cvars read over RCON; jump speed sqrt(2 * 800 * 57), which
    /// reproduces the measured standing-jump apex (54.75 units at the
    /// server's 66.67 tick).
    fn default() -> Self {
        Self {
            stopspeed: 75.0,
            jump_impulse: (2.0f32 * 800.0 * 57.0).sqrt(),
            key_speed: 400.0,
            ..Self::shared_code()
        }
    }
}

const HALF_WIDTH: f32 = 16.0;
const STAND_HEIGHT: f32 = 72.0;
const DUCK_HEIGHT: f32 = 36.0;
const EYE_STAND: f32 = 64.0;
const EYE_DUCK: f32 = 28.0;
/// The transform sits at the standing box's centre.
const ORIGIN_ABOVE_FEET: f32 = STAND_HEIGHT / 2.0;

const AIR_WISH_CAP: f32 = 30.0;
const WALKABLE_NORMAL_Z: f32 = 0.7;
const GROUND_PROBE: f32 = 2.0;
const LEAVE_GROUND_VZ: f32 = 140.0;
const UNGROUND_VZ: f32 = 250.0;
const MAX_CLIP_PLANES: usize = 5;
const BUMPS: usize = 4;
const STEP_EPSILON: f32 = 0.031_25;
const STAY_ON_GROUND_UP: f32 = 2.0;
const SNAP_MIN: f32 = 1.0 / 64.0;
const DUCK_TIMER_START: f32 = 1000.0;
const TIME_TO_DUCK: f32 = 0.4;
const TIME_TO_UNDUCK: f32 = 0.2;
const DUCK_SPEED_FRAC: f32 = 1.0 / 3.0;
const UPWARD_AIR_FRICTION: f32 = 0.25;

/// How far sweeps stop short of what they hit, along the move. Our stand-in
/// for the BSP trace's distance epsilon, so the box never rests touching.
const TRACE_BACKOFF: f32 = 0.031_25;
/// Float noise allowance (meters) when deciding whether a sweep runs
/// parallel to a brush plane: about a thousandth of a unit.
const PARALLEL_SLOP: f32 = 2.5e-5;
/// Solid tests use the box shrunk by this much, so resting contact (within
/// the back-off) doesn't count as stuck.
const SOLID_SKIN: f32 = 0.01;

/// Per-character movement state. Units and seconds as in the spec.
#[derive(Component, Clone, Debug)]
pub struct SourceMovement {
    pub on_ground: bool,
    pub ground_normal: Vec3,
    pub jump_held: bool,
    duck_held: bool,
    /// Small box in use.
    pub ducked: bool,
    /// In a duck or unduck transition.
    pub ducking: bool,
    duck_timer: f32,
    /// Eye height above the feet.
    pub eye: f32,
    pub fall_speed: f32,
    pub surface_friction: f32,
    /// Speed of the last landing (for fall damage and sounds), units/s.
    pub last_landing_speed: f32,
}

impl Default for SourceMovement {
    fn default() -> Self {
        Self {
            on_ground: false,
            ground_normal: Vec3::Z,
            jump_held: false,
            duck_held: false,
            ducked: false,
            ducking: false,
            duck_timer: 0.0,
            eye: EYE_STAND,
            fall_speed: 0.0,
            surface_friction: 1.0,
            last_landing_speed: 0.0,
        }
    }
}

pub struct SourceMovementPlugin;

impl Plugin for SourceMovementPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<SourceMovementConfig>()
            .register_movement::<SourceMovement>(ID)
            .add_systems(FixedUpdate, step.in_set(SimSet::Movement));
    }
}

/// Engine (meters, Y up) to Source (units, Z up), positions and directions.
pub fn to_source(v: Vec3) -> Vec3 {
    Vec3::new(v.x, -v.z, v.y) / METERS_PER_UNIT
}

pub fn to_engine(v: Vec3) -> Vec3 {
    Vec3::new(v.x, v.z, -v.y) * METERS_PER_UNIT
}

fn hull_height(ducked: bool) -> f32 {
    if ducked { DUCK_HEIGHT } else { STAND_HEIGHT }
}

/// Result of sweeping a box, as the spec's traces report it.
#[derive(Clone, Copy, Debug)]
struct Trace {
    fraction: f32,
    /// Feet position at the end of the sweep.
    end: Vec3,
    normal: Vec3,
    start_solid: bool,
}

impl Trace {
    fn hit(&self) -> bool {
        self.fraction < 1.0
    }
}

/// Box sweeps against the world, in Source units. Brushes are swept
/// exactly against their planes (as Source traces do); everything else
/// (props, displacement triangles) through physics shape casts.
struct Tracer<'a, 'w, 's> {
    query: &'a SpatialQuery<'w, 's>,
    filter: SpatialQueryFilter,
    brushes: Option<&'a MapBrushes>,
}

/// Brush sweep result, engine space.
struct BrushHit {
    fraction: f32,
    normal: Vec3,
    start_solid: bool,
}

impl Tracer<'_, '_, '_> {
    /// Sweep an axis-aligned box (centre `from` to `to`, half size `half`,
    /// engine meters) through the brushes: each brush's planes pushed out
    /// by the box's extent along their normals, then the centre's segment
    /// clipped against them. Hits stop a distance epsilon short.
    fn sweep_brushes(&self, half: Vec3, from: Vec3, to: Vec3) -> BrushHit {
        let eps = TRACE_BACKOFF * METERS_PER_UNIT;
        let mut out = BrushHit {
            fraction: 1.0,
            normal: Vec3::ZERO,
            start_solid: false,
        };
        let Some(brushes) = self.brushes else {
            return out;
        };
        let lo = from.min(to) - half - Vec3::splat(eps);
        let hi = from.max(to) + half + Vec3::splat(eps);
        'brush: for b in &brushes.0 {
            if b.max.cmplt(lo).any() || b.min.cmpgt(hi).any() {
                continue;
            }
            let (mut enter, mut leave) = (-1.0f32, 1.0f32);
            let (mut starts_out, mut gets_out) = (false, false);
            let mut clip = Vec3::ZERO;
            for (n, d) in &b.planes {
                let dist = d + n.abs().dot(half);
                let d1 = n.dot(from) - dist;
                let d2 = n.dot(to) - dist;
                gets_out |= d2 > 0.0;
                starts_out |= d1 > 0.0;
                // Entirely in front of this plane: misses the brush. Moving
                // parallel to it counts as not entering, within float noise
                // (planes and positions are meters at map scale), or a
                // velocity just clipped along an angled surface would catch
                // on it forever.
                if d1 > 0.0 && (d2 >= eps || d2 >= d1 - PARALLEL_SLOP) {
                    continue 'brush;
                }
                if d1 <= 0.0 && d2 <= 0.0 {
                    continue;
                }
                if d1 > d2 {
                    let f = ((d1 - eps) / (d1 - d2)).max(0.0);
                    if f > enter {
                        enter = f;
                        clip = *n;
                    }
                } else {
                    leave = leave.min(((d1 + eps) / (d1 - d2)).min(1.0));
                }
            }
            if !starts_out {
                out.start_solid = true;
                if !gets_out {
                    out.fraction = 0.0;
                }
                continue;
            }
            if enter < leave && enter > -1.0 && enter < out.fraction {
                out.fraction = enter.max(0.0);
                out.normal = clip;
            }
        }
        out
    }

    /// Whether a box (engine centre and half size) overlaps a brush by more
    /// than the solid skin.
    fn in_brush(&self, half: Vec3, centre: Vec3) -> bool {
        let skin = SOLID_SKIN * METERS_PER_UNIT;
        self.brushes.is_some_and(|brushes| {
            brushes.0.iter().any(|b| {
                b.max.cmpgt(centre - half).all()
                    && b.min.cmplt(centre + half).all()
                    && b.planes
                        .iter()
                        .all(|(n, d)| n.dot(centre) - (d + n.abs().dot(half)) < -skin)
            })
        })
    }

    /// Sweep a box (`lo`..`hi` relative to the feet, Source units) from feet
    /// position `from` to `to`.
    fn sweep_box(&self, lo: Vec3, hi: Vec3, from: Vec3, to: Vec3) -> Trace {
        let size = hi - lo;
        let centre = |feet: Vec3| to_engine(feet + (lo + hi) / 2.0);
        let start_solid = self.solid_box(lo, hi, from);
        let delta = to - from;
        let len = delta.length();
        let miss = Trace {
            fraction: 1.0,
            end: to,
            normal: Vec3::ZERO,
            start_solid,
        };
        if len < 1e-6 {
            return Trace { end: from, ..miss };
        }
        // Box half size in engine axes (x, up, z).
        let half = Vec3::new(size.x, size.z, size.y) * METERS_PER_UNIT / 2.0;
        let brush = self.sweep_brushes(half, centre(from), centre(to));
        let mut best = Trace {
            fraction: brush.fraction,
            end: from + delta * brush.fraction,
            normal: to_source(brush.normal).normalize_or_zero(),
            start_solid,
        };
        let shape = Collider::cuboid(half.x * 2.0, half.y * 2.0, half.z * 2.0);
        let Ok(dir) = Dir3::new(to_engine(delta / len)) else {
            return best;
        };
        let config = ShapeCastConfig {
            max_distance: len * METERS_PER_UNIT,
            ignore_origin_penetration: true,
            ..default()
        };
        // A surface that doesn't face against the move (we're sliding along
        // it, or already leaving it) doesn't block it; physics casts still
        // report such grazing contacts.
        if let Some(hit) = self
            .query
            .cast_shape(&shape, centre(from), Quat::IDENTITY, dir, &config, &self.filter)
            .filter(|hit| hit.normal1.dot(*dir) < -1e-3)
        {
            let travelled = (hit.distance / METERS_PER_UNIT - TRACE_BACKOFF).max(0.0);
            let fraction = (travelled / len).clamp(0.0, 0.999_999);
            if fraction < best.fraction {
                best = Trace {
                    fraction,
                    end: from + delta * fraction,
                    normal: to_source(hit.normal1).normalize_or_zero(),
                    start_solid,
                };
            }
        }
        best
    }

    fn hull(ducked: bool) -> (Vec3, Vec3) {
        (
            Vec3::new(-HALF_WIDTH, -HALF_WIDTH, 0.0),
            Vec3::new(HALF_WIDTH, HALF_WIDTH, hull_height(ducked)),
        )
    }

    fn sweep(&self, ducked: bool, from: Vec3, to: Vec3) -> Trace {
        let (lo, hi) = Self::hull(ducked);
        self.sweep_box(lo, hi, from, to)
    }

    fn solid_box(&self, lo: Vec3, hi: Vec3, feet: Vec3) -> bool {
        let full = hi - lo;
        if self.in_brush(
            Vec3::new(full.x, full.z, full.y) * METERS_PER_UNIT / 2.0,
            to_engine(feet + (lo + hi) / 2.0),
        ) {
            return true;
        }
        let size = (hi - lo - Vec3::splat(2.0 * SOLID_SKIN)).max(Vec3::splat(0.01));
        let shape = Collider::cuboid(
            size.x * METERS_PER_UNIT,
            size.z * METERS_PER_UNIT,
            size.y * METERS_PER_UNIT,
        );
        !self
            .query
            .shape_intersections(&shape, to_engine(feet + (lo + hi) / 2.0), Quat::IDENTITY, &self.filter)
            .is_empty()
    }

    fn solid(&self, ducked: bool, feet: Vec3) -> bool {
        let (lo, hi) = Self::hull(ducked);
        self.solid_box(lo, hi, feet)
    }
}

/// out = in - n (in . n) b, then remove any remaining push into the plane.
fn clip_velocity(v: Vec3, n: Vec3, overbounce: f32) -> Vec3 {
    let mut out = v - n * v.dot(n) * overbounce;
    let into = out.dot(n);
    if into < 0.0 {
        out -= n * into;
    }
    out
}

/// One tick for one character, in Source units.
struct Mover<'a, 'b, 'w, 's> {
    cfg: &'a SourceMovementConfig,
    trace: &'a Tracer<'b, 'w, 's>,
    me: &'a mut SourceMovement,
    feet: Vec3,
    v: Vec3,
    dt: f32,
}

impl Mover<'_, '_, '_, '_> {
    fn clamp_velocity(&mut self) {
        let m = self.cfg.maxvelocity;
        self.v = Vec3::new(
            if self.v.x.is_nan() { 0.0 } else { self.v.x.clamp(-m, m) },
            if self.v.y.is_nan() { 0.0 } else { self.v.y.clamp(-m, m) },
            if self.v.z.is_nan() { 0.0 } else { self.v.z.clamp(-m, m) },
        );
    }

    fn half_gravity(&mut self) {
        self.v.z -= self.cfg.gravity * self.dt * 0.5;
    }

    fn friction(&mut self) {
        let speed = self.v.length();
        if speed < 0.1 {
            return;
        }
        let control = speed.max(self.cfg.stopspeed);
        let drop = control * self.cfg.friction * self.me.surface_friction * self.dt;
        let new_speed = (speed - drop).max(0.0);
        self.v *= new_speed / speed;
    }

    fn accelerate(&mut self, dir: Vec3, wish_speed: f32, accel: f32) {
        let add = wish_speed - self.v.dot(dir);
        if add <= 0.0 {
            return;
        }
        let amount = (accel * self.dt * wish_speed * self.me.surface_friction).min(add);
        self.v += dir * amount;
    }

    fn air_accelerate(&mut self, dir: Vec3, wish_speed: f32, accel: f32) {
        let add = wish_speed.min(AIR_WISH_CAP) - self.v.dot(dir);
        if add <= 0.0 {
            return;
        }
        let amount = (accel * wish_speed * self.dt * self.me.surface_friction).min(add);
        self.v += dir * amount;
    }

    /// Collide and slide for the rest of the tick.
    fn slide(&mut self) {
        let primal = self.v;
        let mut original = self.v;
        let mut planes: Vec<Vec3> = Vec::with_capacity(MAX_CLIP_PLANES);
        let mut time_left = self.dt;
        let mut all_fraction = 0.0;
        for _ in 0..BUMPS {
            if self.v.length_squared() == 0.0 {
                break;
            }
            let end = self.feet + self.v * time_left;
            let tr = self.trace.sweep(self.me.ducked, self.feet, end);
            all_fraction += tr.fraction;
            if tr.start_solid && tr.fraction == 0.0 && self.trace.solid(self.me.ducked, end) {
                self.v = Vec3::ZERO;
                return;
            }
            if tr.fraction > 0.0 {
                if tr.fraction < 1.0 && self.trace.solid(self.me.ducked, tr.end) {
                    self.v = Vec3::ZERO;
                    break;
                }
                self.feet = tr.end;
                original = self.v;
                planes.clear();
            }
            if !tr.hit() {
                break;
            }
            time_left -= time_left * tr.fraction;
            if planes.len() >= MAX_CLIP_PLANES {
                self.v = Vec3::ZERO;
                break;
            }
            planes.push(tr.normal);
            if planes.len() == 1 && !self.me.on_ground {
                let overbounce = if tr.normal.z > WALKABLE_NORMAL_Z {
                    1.0
                } else {
                    1.0 + self.cfg.bounce * (1.0 - self.me.surface_friction)
                };
                self.v = clip_velocity(original, tr.normal, overbounce);
            } else {
                let found = planes.iter().enumerate().find_map(|(i, n)| {
                    let clipped = clip_velocity(original, *n, 1.0);
                    planes
                        .iter()
                        .enumerate()
                        .all(|(j, m)| j == i || clipped.dot(*m) >= 0.0)
                        .then_some(clipped)
                });
                match found {
                    Some(v) => self.v = v,
                    None if planes.len() == 2 => {
                        let dir = planes[0].cross(planes[1]).normalize_or_zero();
                        self.v = dir * dir.dot(self.v);
                    }
                    None => {
                        self.v = Vec3::ZERO;
                        break;
                    }
                }
                if self.v.dot(primal) <= 0.0 {
                    self.v = Vec3::ZERO;
                    break;
                }
            }
        }
        if all_fraction == 0.0 {
            self.v = Vec3::ZERO;
        }
    }

    /// Try the move flat and stepped up; keep whichever goes farther.
    fn step_move(&mut self) {
        let (p0, v0) = (self.feet, self.v);
        self.slide();
        let (down_pos, down_vel) = (self.feet, self.v);

        self.feet = p0;
        self.v = v0;
        let rise = self.cfg.stepsize + STEP_EPSILON;
        let up = self.trace.sweep(self.me.ducked, p0, p0 + Vec3::Z * rise);
        if !up.start_solid {
            self.feet = up.end;
        }
        self.slide();
        let down = self.trace.sweep(self.me.ducked, self.feet, self.feet - Vec3::Z * rise);
        if !down.hit() || down.normal.z < WALKABLE_NORMAL_Z {
            self.feet = down_pos;
            self.v = down_vel;
            return;
        }
        if !down.start_solid {
            self.feet = down.end;
        }
        let flat = (down_pos - p0).truncate().length_squared();
        let stepped = (self.feet - p0).truncate().length_squared();
        if flat > stepped {
            self.feet = down_pos;
            self.v = down_vel;
        } else {
            self.v.z = down_vel.z;
        }
    }

    fn stay_on_ground(&mut self) {
        let up = self
            .trace
            .sweep(self.me.ducked, self.feet, self.feet + Vec3::Z * STAY_ON_GROUND_UP);
        let start = up.end;
        let end = self.feet - Vec3::Z * self.cfg.stepsize;
        let tr = self.trace.sweep(self.me.ducked, start, end);
        if tr.fraction > 0.0
            && tr.fraction < 1.0
            && !tr.start_solid
            && tr.normal.z >= WALKABLE_NORMAL_Z
            && (self.feet.z - tr.end.z).abs() > SNAP_MIN
        {
            self.feet = tr.end;
        }
    }

    fn wish(&self, f: f32, s: f32, forward: Vec3, right: Vec3, max_speed: f32) -> (Vec3, f32) {
        let mut wish = forward * f + right * s;
        wish.z = 0.0;
        let speed = wish.length();
        if speed < 1e-6 {
            return (Vec3::ZERO, 0.0);
        }
        (wish / speed, speed.min(max_speed))
    }

    fn walk(&mut self, f: f32, s: f32, forward: Vec3, right: Vec3, max_speed: f32) {
        let (dir, speed) = self.wish(f, s, forward, right, max_speed);
        self.v.z = 0.0;
        self.accelerate(dir, speed, self.cfg.accelerate);
        self.v.z = 0.0;
        if self.v.length() < 1.0 {
            self.v = Vec3::ZERO;
            return;
        }
        let dest = self.feet + Vec3::new(self.v.x, self.v.y, 0.0) * self.dt;
        let tr = self.trace.sweep(self.me.ducked, self.feet, dest);
        if !tr.hit() {
            self.feet = tr.end;
        } else {
            self.step_move();
        }
        self.stay_on_ground();
    }

    fn air(&mut self, f: f32, s: f32, forward: Vec3, right: Vec3, max_speed: f32) {
        let (dir, speed) = self.wish(f, s, forward, right, max_speed);
        self.air_accelerate(dir, speed, self.cfg.airaccelerate);
        self.slide();
    }

    /// Ground detection: a 2-unit drop test with the full box, then with
    /// each quarter of it, so edges and ridges still count as ground.
    fn categorize(&mut self) {
        self.me.surface_friction = 1.0;
        if self.v.z > LEAVE_GROUND_VZ {
            self.me.on_ground = false;
        } else {
            let (lo, hi) = Tracer::hull(self.me.ducked);
            let down = self.feet - Vec3::Z * GROUND_PROBE;
            let full = self.trace.sweep_box(lo, hi, self.feet, down);
            let mut ground = (full.hit() && full.normal.z >= WALKABLE_NORMAL_Z).then_some(full.normal);
            if ground.is_none() {
                let mid = Vec3::ZERO;
                let quarters = [
                    (Vec3::new(lo.x, lo.y, lo.z), Vec3::new(mid.x, mid.y, hi.z)),
                    (Vec3::new(mid.x, lo.y, lo.z), Vec3::new(hi.x, mid.y, hi.z)),
                    (Vec3::new(lo.x, mid.y, lo.z), Vec3::new(mid.x, hi.y, hi.z)),
                    (Vec3::new(mid.x, mid.y, lo.z), Vec3::new(hi.x, hi.y, hi.z)),
                ];
                ground = quarters.iter().find_map(|(a, b)| {
                    let tr = self.trace.sweep_box(*a, *b, self.feet, down);
                    (tr.hit() && tr.normal.z >= WALKABLE_NORMAL_Z).then_some(tr.normal)
                });
            }
            self.me.on_ground = ground.is_some();
            if let Some(n) = ground {
                self.me.ground_normal = n;
                self.v.z = 0.0;
            } else if self.v.z > 0.0 {
                self.me.surface_friction = UPWARD_AIR_FRICTION;
            }
        }
    }

    fn eye_blend(fraction: f32) -> f32 {
        EYE_DUCK * fraction + EYE_STAND * (1.0 - fraction)
    }

    fn smooth(x: f32) -> f32 {
        let x = x.clamp(0.0, 1.0);
        3.0 * x * x - 2.0 * x * x * x
    }

    fn finish_duck(&mut self) {
        self.me.ducked = true;
        self.me.ducking = false;
        self.me.eye = EYE_DUCK;
        if !self.me.on_ground {
            self.feet.z += STAND_HEIGHT - DUCK_HEIGHT;
        }
        for _ in 0..36 {
            if !self.trace.solid(true, self.feet) {
                break;
            }
            self.feet.z += 1.0;
        }
        self.categorize();
    }

    fn can_unduck(&self) -> bool {
        let at = if self.me.on_ground {
            self.feet
        } else {
            self.feet - Vec3::Z * (STAND_HEIGHT - DUCK_HEIGHT)
        };
        !self.trace.solid(false, at)
    }

    fn finish_unduck(&mut self) {
        if !self.me.on_ground {
            self.feet.z -= STAND_HEIGHT - DUCK_HEIGHT;
        }
        self.me.ducked = false;
        self.me.ducking = false;
        self.me.eye = EYE_STAND;
        self.categorize();
    }

    /// Ducking transitions; returns the move-input scale.
    fn duck(&mut self, held: bool) -> f32 {
        let pressed = held && !self.me.duck_held;
        let released = !held && self.me.duck_held;
        let elapsed = (DUCK_TIMER_START - self.me.duck_timer) / 1000.0;
        if held {
            if pressed && !self.me.ducked {
                self.me.duck_timer = DUCK_TIMER_START;
                self.me.ducking = true;
            }
            if self.me.ducking && !self.me.ducked {
                let elapsed = (DUCK_TIMER_START - self.me.duck_timer) / 1000.0;
                if elapsed > TIME_TO_DUCK || !self.me.on_ground {
                    self.finish_duck();
                } else {
                    self.me.eye = Self::eye_blend(Self::smooth(elapsed / TIME_TO_DUCK));
                }
            } else if self.me.ducked {
                // Held through a partial unduck: back to fully ducked.
                self.me.ducking = false;
                self.me.eye = EYE_DUCK;
            }
        } else if self.me.ducked || self.me.ducking {
            if released {
                self.me.duck_timer = if self.me.ducking && !self.me.ducked {
                    let fraction = (elapsed / TIME_TO_DUCK).clamp(0.0, 1.0);
                    DUCK_TIMER_START - TIME_TO_UNDUCK * 1000.0 + fraction * TIME_TO_UNDUCK * 1000.0
                } else {
                    DUCK_TIMER_START
                };
                self.me.ducking = true;
            }
            if self.can_unduck() {
                let elapsed = (DUCK_TIMER_START - self.me.duck_timer) / 1000.0;
                if elapsed > TIME_TO_UNDUCK || !self.me.on_ground {
                    self.finish_unduck();
                } else {
                    self.me.eye = Self::eye_blend(Self::smooth(1.0 - elapsed / TIME_TO_UNDUCK));
                }
            } else {
                self.me.eye = EYE_DUCK;
                self.me.duck_timer = DUCK_TIMER_START;
            }
        } else if (self.me.eye - EYE_STAND).abs() > 0.1 {
            self.me.eye = EYE_STAND;
        }
        self.me.duck_held = held;
        if self.me.ducked && self.me.on_ground {
            DUCK_SPEED_FRAC
        } else {
            1.0
        }
    }

    fn jump(&mut self) {
        self.me.on_ground = false;
        if self.me.ducked || self.me.ducking {
            self.v.z = self.cfg.jump_impulse;
        } else {
            self.v.z += self.cfg.jump_impulse;
        }
        self.half_gravity();
        self.me.jump_held = true;
    }

    /// Recover from being inside something: nudge up to the step height.
    fn unstick(&mut self) -> bool {
        if !self.trace.solid(self.me.ducked, self.feet) {
            return true;
        }
        for k in 1..=self.cfg.stepsize as i32 {
            let at = self.feet + Vec3::Z * k as f32;
            if !self.trace.solid(self.me.ducked, at) {
                self.feet = at;
                return true;
            }
        }
        false
    }

    fn tick(&mut self, intent: &Intent) {
        // Move input: full keys send key_speed, rescaled to the max speed.
        let max_speed = self.cfg.player_maxspeed.min(self.cfg.maxspeed);
        let (mut f, mut s) = (
            intent.move_axis.y * self.cfg.key_speed,
            intent.move_axis.x * self.cfg.key_speed,
        );
        let len = (f * f + s * s).sqrt();
        if len > max_speed {
            f *= max_speed / len;
            s *= max_speed / len;
        }
        self.me.duck_timer = (self.me.duck_timer - 1000.0 * self.dt).max(0.0);

        // Intent yaw 0 looks down engine -Z, which is Source yaw 90.
        let yaw = std::f32::consts::FRAC_PI_2 + intent.yaw;
        let forward = Vec3::new(yaw.cos(), yaw.sin(), 0.0);
        let right = Vec3::new(yaw.sin(), -yaw.cos(), 0.0);

        if !self.unstick() {
            return;
        }
        if self.v.z > UNGROUND_VZ {
            self.me.on_ground = false;
        }
        if !self.me.on_ground {
            self.me.fall_speed = -self.v.z;
        }
        let scale = self.duck(intent.crouch);
        f *= scale;
        s *= scale;

        self.half_gravity();
        self.clamp_velocity();

        if intent.jump {
            if self.me.on_ground && !self.me.jump_held && !(self.me.ducked && self.me.ducking) {
                self.jump();
            } else {
                self.me.jump_held = true;
            }
        } else {
            self.me.jump_held = false;
        }
        if self.me.on_ground {
            self.v.z = 0.0;
            self.friction();
        }
        self.clamp_velocity();
        if self.me.on_ground {
            self.walk(f, s, forward, right, max_speed);
        } else {
            self.air(f, s, forward, right, max_speed);
        }
        self.categorize();
        self.clamp_velocity();
        self.half_gravity();
        self.clamp_velocity();
        if self.me.on_ground {
            self.v.z = 0.0;
            if self.me.fall_speed > 0.0 {
                self.me.last_landing_speed = self.me.fall_speed;
                self.me.fall_speed = 0.0;
            }
        }
    }
}

fn step(
    mut q: Query<(
        Entity,
        &Intent,
        &mut SourceMovement,
        &mut Transform,
        &mut Velocity,
        &mut MovementState,
    )>,
    query: SpatialQuery,
    brushes: Option<Res<MapBrushes>>,
    brush_colliders: Query<Entity, With<MapBrushCollider>>,
    cfg: Res<SourceMovementConfig>,
    time: Res<Time>,
) {
    let dt = time.delta_secs();
    for (entity, intent, mut me, mut transform, mut vel, mut state) in &mut q {
        // With brushes swept exactly, physics queries skip the same brushes.
        let excluded =
            std::iter::once(entity).chain(brushes.as_ref().map(|_| brush_colliders.iter()).into_iter().flatten());
        let tracer = Tracer {
            query: &query,
            filter: SpatialQueryFilter::from_excluded_entities(excluded),
            brushes: brushes.as_deref(),
        };
        let mut mover = Mover {
            cfg: &cfg,
            trace: &tracer,
            feet: to_source(transform.translation) - Vec3::Z * ORIGIN_ABOVE_FEET,
            v: to_source(vel.0),
            me: &mut me,
            dt,
        };
        mover.tick(intent);
        let (feet, v) = (mover.feet, mover.v);
        transform.translation = to_engine(feet + Vec3::Z * ORIGIN_ABOVE_FEET);
        vel.0 = to_engine(v);
        *state = MovementState {
            on_ground: me.on_ground,
            crouching: me.ducked || (me.ducking && intent.crouch),
            sprinting: false,
            eye_offset: Vec3::Y * (me.eye - ORIGIN_ABOVE_FEET) * METERS_PER_UNIT,
        };
    }
}
