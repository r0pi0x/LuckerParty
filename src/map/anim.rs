//! Skeletal animation of character bodies: a set of sampled animations,
//! sequences that blend them by pose parameters, and the layering that
//! builds a body's pose (specs/cs_source/animation.md §3–§10). Games load
//! an `AnimSet` and drive an `Animator`; this module only does the math
//! and writes the pose onto a body's joints.
//!
//! Poses are in the skeleton's own axes and units (as `MapBone`).

use std::{collections::HashMap, sync::Arc};

use bevy::prelude::*;

/// One bone's local rotation and position.
pub type BonePose = (Quat, Vec3);

/// Everything one skeleton can play. Bone indices are the target
/// skeleton's (`MapCharacterModel::bones`).
#[derive(Debug, Default)]
pub struct AnimSet {
    /// The target skeleton's own default pose.
    pub defaults: Vec<BonePose>,
    /// Base poses of the models the animations came from, mapped onto the
    /// target's bones (`Animation::base`).
    pub bases: Vec<Vec<BonePose>>,
    pub animations: Vec<Animation>,
    pub sequences: Vec<Sequence>,
    pub params: Vec<PoseParam>,
    /// Sequence index by lower-case name (filled by loaders; optional).
    pub names: HashMap<String, usize>,
}

/// Sampled frames of one animation.
#[derive(Debug, Default)]
pub struct Animation {
    pub name: String,
    pub fps: f32,
    pub frames: usize,
    /// Additive: bones start at identity and tracks are offsets.
    pub delta: bool,
    /// Which `AnimSet::bases` entry untracked bones start from (non-delta).
    pub base: usize,
    pub tracks: Vec<Track>,
}

/// One bone's values per frame (one entry: constant); None keeps the
/// starting value.
#[derive(Debug, Default)]
pub struct Track {
    pub bone: usize,
    pub rotation: Option<Vec<Quat>>,
    pub position: Option<Vec<Vec3>>,
}

#[derive(Debug, Default)]
pub struct Sequence {
    pub name: String,
    pub activity: String,
    /// Selection weight among sequences with the same activity (its
    /// absolute value counts).
    pub activity_weight: i32,
    pub looping: bool,
    /// No cross-fade into it.
    pub snap: bool,
    pub delta: bool,
    /// With `delta`: applied in the bone's frame after the base.
    pub post: bool,
    pub fade_in: f32,
    pub fade_out: f32,
    /// Grid width and height, and its animations row-major.
    pub grid: (usize, usize),
    pub anims: Vec<usize>,
    /// Per axis (X, Y): pose parameter, keys (may be empty) and the range
    /// used without keys.
    pub axes: [Axis; 2],
    pub autolayers: Vec<AutoLayer>,
    /// Per target bone; 0 leaves the bone alone.
    pub bone_weights: Vec<f32>,
    /// Animation events, by cycle (games decide what they mean).
    pub events: Vec<AnimEvent>,
}

/// An event a sequence fires when its cycle passes `cycle`.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct AnimEvent {
    pub cycle: f32,
    /// Its number (0 for named events).
    pub event: i32,
    /// Its name, for named events.
    pub name: String,
    pub options: String,
}

impl Sequence {
    /// Events whose cycle the playback passed going from `from` to `to`
    /// (`from` excluded, `to` included); `from` None: the sequence was
    /// (re)started, so events at cycle 0 fire too. A looping sequence that
    /// wrapped fires the end of the old loop and the start of the new.
    pub fn events_between(&self, from: Option<f32>, to: f32) -> impl Iterator<Item = &AnimEvent> {
        self.events.iter().filter(move |e| match from {
            None => e.cycle <= to,
            Some(f) if to >= f => e.cycle > f && e.cycle <= to,
            Some(f) => e.cycle > f || e.cycle <= to,
        })
    }
}

#[derive(Debug, Default, Clone)]
pub struct Axis {
    pub param: Option<usize>,
    pub keys: Vec<f32>,
    pub range: (f32, f32),
}

#[derive(Debug, Clone)]
pub struct AutoLayer {
    pub sequence: usize,
    pub start: f32,
    pub peak: f32,
    pub tail: f32,
    pub end: f32,
}

#[derive(Debug, Clone)]
pub struct PoseParam {
    pub name: String,
    pub start: f32,
    pub end: f32,
    /// Wrap range (0: none).
    pub looping: f32,
}

impl PoseParam {
    /// The stored (normalized, 0..1) value for `value` (§5).
    pub fn encode(&self, mut value: f32) -> f32 {
        if self.looping != 0.0 {
            let l = self.looping;
            let mid = (self.start + self.end) / 2.0;
            value -= l * ((value + l - (mid + l / 2.0)) / l).floor();
        }
        if self.end == self.start {
            return 0.0;
        }
        ((value - self.start) / (self.end - self.start)).clamp(0.0, 1.0)
    }

    pub fn default_value(&self) -> f32 {
        if self.start < 0.0 && 0.0 < self.end {
            self.encode(0.0)
        } else {
            0.5
        }
    }
}

impl AnimSet {
    pub fn sequence(&self, name: &str) -> Option<usize> {
        if self.names.is_empty() {
            return self.sequences.iter().position(|s| s.name.eq_ignore_ascii_case(name));
        }
        self.names.get(&name.to_lowercase()).copied()
    }

    /// The first sequence for an activity (e.g. `ACT_RUN`).
    pub fn activity(&self, activity: &str) -> Option<usize> {
        self.sequences
            .iter()
            .position(|s| s.activity.eq_ignore_ascii_case(activity))
    }

    /// Every sequence for an activity with its selection weight
    /// (|activity weight|, at least 1 when all are 0).
    pub fn activities(&self, activity: &str) -> Vec<(usize, u32)> {
        let mut out: Vec<(usize, u32)> = self
            .sequences
            .iter()
            .enumerate()
            .filter(|(_, s)| s.activity.eq_ignore_ascii_case(activity))
            .map(|(i, s)| (i, s.activity_weight.unsigned_abs()))
            .collect();
        if out.iter().all(|(_, w)| *w == 0) {
            out.iter_mut().for_each(|(_, w)| *w = 1);
        }
        out
    }

    /// One sequence for an activity, chosen by weight with `roll` in
    /// [0, 1) (spec weapons.md 3.8: weighted random when several).
    pub fn pick_activity(&self, activity: &str, roll: f32) -> Option<usize> {
        let options = self.activities(activity);
        let total: u32 = options.iter().map(|(_, w)| w).sum();
        if total == 0 {
            return None;
        }
        let mut at = (roll.clamp(0.0, 0.999_999) * total as f32) as u32;
        for (s, w) in &options {
            if at < *w {
                return Some(*s);
            }
            at -= w;
        }
        options.last().map(|(s, _)| *s)
    }

    /// Seconds sequence `s` takes to play once at its own rate (spec
    /// weapons.md 3.8: (frames - 1) / fps; 0 for a single frame).
    pub fn duration(&self, s: usize) -> f32 {
        let rate = self.cycle_rate(s, &self.default_params());
        if rate > 0.0 { 1.0 / rate } else { 0.0 }
    }

    pub fn param(&self, name: &str) -> Option<usize> {
        self.params.iter().position(|p| p.name.eq_ignore_ascii_case(name))
    }

    /// Stored values of all pose parameters before anything sets them.
    pub fn default_params(&self) -> Vec<f32> {
        self.params.iter().map(PoseParam::default_value).collect()
    }

    /// Animation `a` at `cycle` into `out` (one entry per target bone),
    /// §3.
    pub fn sample(&self, a: usize, cycle: f32, out: &mut [BonePose]) {
        let anim = &self.animations[a];
        if anim.delta {
            out.fill((Quat::IDENTITY, Vec3::ZERO));
        } else {
            out.copy_from_slice(&self.bases[anim.base]);
        }
        let f = cycle * (anim.frames.max(1) - 1) as f32;
        let i = (f as usize).min(anim.frames.saturating_sub(1));
        let t = f - i as f32;
        let blend = t > 0.001;
        for track in &anim.tracks {
            let Some(bone) = out.get_mut(track.bone) else { continue };
            if let Some(r) = &track.rotation {
                bone.0 = if r.len() == 1 {
                    r[0]
                } else if blend && i + 1 < r.len() {
                    nlerp(r[i], r[i + 1], t)
                } else {
                    r[i.min(r.len() - 1)]
                };
            }
            if let Some(p) = &track.position {
                bone.1 = if p.len() == 1 {
                    p[0]
                } else if blend && i + 1 < p.len() {
                    p[i].lerp(p[i + 1], t)
                } else {
                    p[i.min(p.len() - 1)]
                };
            }
        }
    }

    /// Grid cell and fraction on one axis of a sequence (§5).
    pub fn axis(&self, seq: &Sequence, axis: usize, params: &[f32]) -> (usize, f32) {
        let a = &seq.axes[axis];
        let size = if axis == 0 { seq.grid.0 } else { seq.grid.1 };
        let Some(pi) = a.param else { return (0, 0.0) };
        let p = &self.params[pi];
        let n = params.get(pi).copied().unwrap_or_else(|| p.default_value());
        if a.keys.len() >= 2 {
            let v = n * (p.end - p.start) + p.start;
            let mut i = 0;
            let frac = |i: usize| (v - a.keys[i]) / (a.keys[i + 1] - a.keys[i]);
            let mut s = frac(i);
            while s > 1.0 && i + 2 < size.min(a.keys.len()) {
                i += 1;
                s = frac(i);
            }
            (i, s.clamp(0.0, 1.0))
        } else {
            let span = p.end - p.start;
            let (lo, hi) = ((a.range.0 - p.start) / span, (a.range.1 - p.start) / span);
            let s = ((n - lo) / (hi - lo)).clamp(0.0, 1.0);
            if size > 2 {
                let i = ((s * (size - 1) as f32) as usize).min(size - 2);
                (i, s * (size - 1) as f32 - i as f32)
            } else {
                (0, s)
            }
        }
    }

    fn cell(seq: &Sequence, x: usize, y: usize) -> usize {
        let (w, h) = seq.grid;
        seq.anims[y.min(h.max(1) - 1) * w + x.min(w.max(1) - 1)]
    }

    /// The pose of sequence `s` alone at `cycle` (§5). Bones with weight 0
    /// keep the first animation's values.
    pub fn sequence_pose(&self, s: usize, cycle: f32, params: &[f32], out: &mut [BonePose]) {
        let seq = &self.sequences[s];
        let cycle = wrap_cycle(cycle, seq.looping);
        let (i0, s0) = self.axis(seq, 0, params);
        let (i1, s1) = self.axis(seq, 1, params);
        let n = out.len();
        let mut tmp = vec![(Quat::IDENTITY, Vec3::ZERO); n];
        let mut tmp2 = vec![(Quat::IDENTITY, Vec3::ZERO); n];
        let edge = |s: f32| {
            if s < 0.001 {
                Some(0)
            } else if s > 0.999 {
                Some(1)
            } else {
                None
            }
        };
        // Sample cell (x, y) and blend it into `out` by `w` (or set it).
        let put = |out: &mut [BonePose], x: usize, y: usize, w: Option<f32>, tmp: &mut [BonePose]| match w {
            None => self.sample(Self::cell(seq, x, y), cycle, out),
            Some(w) => {
                self.sample(Self::cell(seq, x, y), cycle, tmp);
                blend(out, tmp, w, &seq.bone_weights);
            }
        };
        match (edge(s0), edge(s1)) {
            (Some(e0), Some(e1)) => put(out, i0 + e0, i1 + e1, None, &mut tmp),
            (Some(e0), None) => {
                put(out, i0 + e0, i1, None, &mut tmp);
                put(out, i0 + e0, i1 + 1, Some(s1), &mut tmp);
            }
            (None, Some(e1)) => {
                put(out, i0, i1 + e1, None, &mut tmp);
                put(out, i0 + 1, i1 + e1, Some(s0), &mut tmp);
            }
            (None, None) => {
                let ([c0, c1, c2], [w0, w1, w2]) = triangle(i0, i1, s0, s1);
                put(out, c0.0, c0.1, None, &mut tmp);
                if w1 == 0.0 {
                    put(out, c2.0, c2.1, Some(w2 / (w0 + w2)), &mut tmp);
                } else {
                    put(out, c1.0, c1.1, Some(w1 / (w0 + w1)), &mut tmp);
                    put(out, c2.0, c2.1, Some(w2), &mut tmp2);
                }
            }
        }
    }

    /// Cycles per second of sequence `s` at `params` (§9).
    pub fn cycle_rate(&self, s: usize, params: &[f32]) -> f32 {
        let seq = &self.sequences[s];
        let (i0, s0) = self.axis(seq, 0, params);
        let (i1, s1) = self.axis(seq, 1, params);
        let mut rate = 0.0;
        for (dx, dy, w) in [
            (0, 0, (1.0 - s0) * (1.0 - s1)),
            (1, 0, s0 * (1.0 - s1)),
            (0, 1, (1.0 - s0) * s1),
            (1, 1, s0 * s1),
        ] {
            let a = &self.animations[Self::cell(seq, i0 + dx, i1 + dy)];
            if w > 0.0 && a.frames > 1 {
                rate += w * a.fps / (a.frames - 1) as f32;
            }
        }
        rate
    }

    /// Merge sequence `s` at `cycle` and `weight` into `pose`, then its
    /// autolayers (§6, §7).
    pub fn accumulate(&self, pose: &mut [BonePose], s: usize, cycle: f32, weight: f32, params: &[f32]) {
        self.accumulate_depth(pose, s, cycle, weight, params, 0);
    }

    fn accumulate_depth(&self, pose: &mut [BonePose], s: usize, cycle: f32, weight: f32, params: &[f32], depth: u32) {
        let weight = weight.clamp(0.0, 1.0);
        if weight <= 0.0 || depth > 8 || s >= self.sequences.len() {
            return;
        }
        let seq = &self.sequences[s];
        let mut own = vec![(Quat::IDENTITY, Vec3::ZERO); pose.len()];
        self.sequence_pose(s, cycle, params, &mut own);
        for (b, (cur, new)) in pose.iter_mut().zip(&own).enumerate() {
            let s = (weight * seq.bone_weights.get(b).copied().unwrap_or(0.0)).min(1.0);
            if s <= 0.0 {
                continue;
            }
            if !seq.delta {
                cur.0 = slerp(new.0, cur.0, 1.0 - s);
                cur.1 = cur.1 * (1.0 - s) + new.1 * s;
            } else if seq.post {
                cur.0 = product(cur.0, scale(new.0, s)).normalize();
                cur.1 += new.1 * s;
            } else {
                cur.0 = product(scale(new.0, s), cur.0).normalize();
                cur.1 += new.1 * s;
            }
        }
        for l in &seq.autolayers {
            let ramped = l.start != l.end && !(l.start.is_nan() || l.end.is_nan());
            if !ramped {
                self.accumulate_depth(pose, l.sequence, cycle, weight, params, depth + 1);
                continue;
            }
            if cycle < l.start || cycle >= l.end {
                continue;
            }
            let mut r = 1.0;
            if cycle < l.peak && l.start != l.peak {
                r = (cycle - l.start) / (l.peak - l.start);
            } else if cycle > l.tail && l.end != l.tail {
                r = (l.end - cycle) / (l.end - l.tail);
            }
            let c = (cycle - l.start) / (l.end - l.start);
            self.accumulate_depth(pose, l.sequence, c, weight * r, params, depth + 1);
        }
    }
}

/// Grid corners (offsets added to (i0, i1)) and weights of the triangle
/// blend (§5).
pub fn triangle(i0: usize, i1: usize, s0: f32, s1: f32) -> ([(usize, usize); 3], [f32; 3]) {
    let (c, w0, mut w1): ([(usize, usize); 3], f32, f32) = if (i0 + i1) % 2 == 0 {
        if s0 > s1 {
            ([(0, 0), (1, 0), (1, 1)], 1.0 - s0, s0 - s1)
        } else {
            ([(1, 1), (0, 1), (0, 0)], s0, s1 - s0)
        }
    } else if s0 + s1 > 1.0 {
        ([(1, 0), (1, 1), (0, 1)], 1.0 - s1, s0 + s1 - 1.0)
    } else {
        ([(0, 1), (0, 0), (1, 0)], s1, 1.0 - s0 - s1)
    };
    if w1 < 0.001 {
        w1 = 0.0;
    }
    let cells = c.map(|(x, y)| (i0 + x, i1 + y));
    (cells, [w0, w1, 1.0 - w0 - w1])
}

/// Blend `to` into `from` by `t` for bones with weight > 0: nlerp of
/// rotations, lerp of positions.
fn blend(from: &mut [BonePose], to: &[BonePose], t: f32, weights: &[f32]) {
    for (b, (p, q)) in from.iter_mut().zip(to).enumerate() {
        if weights.get(b).copied().unwrap_or(0.0) > 0.0 {
            p.0 = nlerp(p.0, q.0, t);
            p.1 = p.1.lerp(q.1, t);
        }
    }
}

/// Looping cycles wrap into [0, 1); others clamp to [0, 1].
pub fn wrap_cycle(c: f32, looping: bool) -> f32 {
    if looping { c - c.floor() } else { c.clamp(0.0, 1.0) }
}

/// `q`, or `-q` when that is closer to `p`.
pub fn align(p: Quat, q: Quat) -> Quat {
    let (a, b) = (Vec4::from(p), Vec4::from(q));
    if (a - b).length_squared() > (a + b).length_squared() {
        -q
    } else {
        q
    }
}

pub fn nlerp(p: Quat, q: Quat, t: f32) -> Quat {
    let q = align(p, q);
    Quat::from_vec4(Vec4::from(p) * (1.0 - t) + Vec4::from(q) * t).normalize()
}

/// Slerp without renormalization (§4).
pub fn slerp(p: Quat, q: Quat, t: f32) -> Quat {
    let q = align(p, q);
    let (a, b) = (Vec4::from(p), Vec4::from(q));
    let cos = a.dot(b);
    let (wa, wb) = if 1.0 - cos > 1e-6 {
        let o = cos.clamp(-1.0, 1.0).acos();
        let s = o.sin();
        (((1.0 - t) * o).sin() / s, (t * o).sin() / s)
    } else {
        (1.0 - t, t)
    };
    Quat::from_vec4(a * wa + b * wb)
}

/// The rotation `q` with its angle scaled by `s` (§4).
pub fn scale(q: Quat, s: f32) -> Quat {
    let v = Vec3::new(q.x, q.y, q.z);
    let sigma = v.length().min(1.0);
    let sp = (s * sigma.asin()).sin();
    let v = v * (sp / (sigma + f32::EPSILON));
    let w = (1.0 - sp * sp).max(0.0).sqrt().copysign(q.w);
    Quat::from_xyzw(v.x, v.y, v.z, w)
}

/// `p · q` with `q` first aligned to `p`.
pub fn product(p: Quat, q: Quat) -> Quat {
    p * align(p, q)
}

/// Euler angles (radians, fixed axes X then Y then Z) to a quaternion
/// (§2).
pub fn euler(x: f32, y: f32, z: f32) -> Quat {
    let (sx, cx) = (x / 2.0).sin_cos();
    let (sy, cy) = (y / 2.0).sin_cos();
    let (sz, cz) = (z / 2.0).sin_cos();
    Quat::from_xyzw(
        sx * cy * cz - cx * sy * sz,
        cx * sy * cz + sx * cy * sz,
        cx * cy * sz - sx * sy * cz,
        cx * cy * cz + sx * sy * sz,
    )
}

/// A sequence that is fading out after being replaced (§9).
#[derive(Debug, Clone)]
pub struct Fading {
    pub sequence: usize,
    pub cycle: f32,
    pub playback: f32,
    pub stopped: f64,
    pub fade: f32,
}

impl Fading {
    /// The smoothstep-shaped weight at `now` (0 once gone).
    pub fn weight(&self, now: f64) -> f32 {
        if self.fade <= 0.0 {
            return 0.0;
        }
        let u = (1.0 - (now - self.stopped) as f32 / self.fade).clamp(0.0, 1.0);
        3.0 * u * u - 2.0 * u * u * u
    }
}

/// An overlay layer (§10).
#[derive(Debug, Clone, Copy)]
pub struct Layer {
    pub sequence: usize,
    pub cycle: f32,
    pub weight: f32,
}

/// What a body plays: its main sequence (with the ones fading out), the
/// overlay layers in order, and the pose parameters (stored values).
/// Games drive it; `pose_bodies` turns it into joint transforms.
#[derive(Component, Debug, Default, Clone)]
pub struct Animator {
    pub set: Arc<AnimSet>,
    pub params: Vec<f32>,
    pub main: Option<usize>,
    pub cycle: f32,
    pub playback: f32,
    pub fading: Vec<Fading>,
    pub layers: Vec<Option<Layer>>,
    /// The body's yaw (radians, as `Intent::yaw`); None: the character's
    /// look yaw.
    pub yaw: Option<f32>,
}

impl Animator {
    pub fn new(set: Arc<AnimSet>) -> Self {
        Self {
            params: set.default_params(),
            set,
            playback: 1.0,
            ..default()
        }
    }

    pub fn set_param(&mut self, i: usize, value: f32) {
        if let (Some(p), Some(v)) = (self.set.params.get(i), self.params.get_mut(i)) {
            *v = p.encode(value);
        }
    }

    /// Switch the main sequence at time `now` (§9): non-looping outgoing
    /// sequences restart the cycle, and the outgoing one fades out unless
    /// the new one snaps in.
    pub fn play(&mut self, s: usize, now: f64) {
        if self.main == Some(s) {
            return;
        }
        let set = self.set.clone();
        let new = &set.sequences[s];
        if let Some(old) = self.main {
            let o = &set.sequences[old];
            if new.snap {
                self.fading.clear();
            } else {
                self.fading.insert(
                    0,
                    Fading {
                        sequence: old,
                        cycle: self.cycle,
                        playback: self.playback,
                        stopped: now,
                        fade: o.fade_out.min(new.fade_in),
                    },
                );
            }
            if !o.looping {
                self.cycle = 0.0;
            }
        }
        self.main = Some(s);
        self.playback = 1.0;
    }

    /// Start sequence `s` from cycle 0 even if it is already playing (a
    /// view model replaying its fire sequence, spec weapons.md 3.8); the
    /// outgoing one fades out as in `play`.
    pub fn restart(&mut self, s: usize, now: f64) {
        if let Some(old) = self.main.filter(|&old| old == s) {
            let fade = self.set.sequences[old].fade_out.min(self.set.sequences[s].fade_in);
            if self.set.sequences[s].snap {
                self.fading.clear();
            } else {
                self.fading.insert(
                    0,
                    Fading {
                        sequence: old,
                        cycle: self.cycle,
                        playback: self.playback,
                        stopped: now,
                        fade,
                    },
                );
            }
            self.playback = 1.0;
        } else {
            self.play(s, now);
        }
        self.cycle = 0.0;
    }

    /// Whether the main sequence is non-looping and has reached its end.
    pub fn finished(&self) -> bool {
        self.main
            .is_some_and(|s| !self.set.sequences[s].looping && self.cycle >= 1.0)
    }

    /// Advance the main and fading cycles by `dt`; drop finished fades.
    pub fn advance(&mut self, dt: f32, now: f64) {
        let set = self.set.clone();
        if let Some(s) = self.main {
            let looping = set.sequences[s].looping;
            self.cycle = wrap_cycle(
                self.cycle + dt * set.cycle_rate(s, &self.params) * self.playback,
                looping,
            );
        }
        for f in &mut self.fading {
            let looping = set.sequences[f.sequence].looping;
            f.cycle = wrap_cycle(
                f.cycle + dt * set.cycle_rate(f.sequence, &self.params) * f.playback,
                looping,
            );
        }
        self.fading.retain(|f| f.weight(now) > 0.0);
    }

    /// The whole body's pose (§ "Bone setup").
    pub fn pose(&self, now: f64) -> Vec<BonePose> {
        let set = &*self.set;
        let mut pose = set.defaults.clone();
        if let Some(s) = self.main {
            set.accumulate(&mut pose, s, self.cycle, 1.0, &self.params);
        }
        for f in &self.fading {
            set.accumulate(&mut pose, f.sequence, f.cycle, f.weight(now), &self.params);
        }
        for l in self.layers.iter().flatten() {
            let Some(seq) = set.sequences.get(l.sequence) else {
                continue;
            };
            let cycle = if seq.looping {
                wrap_cycle(l.cycle, true)
            } else {
                l.cycle.clamp(0.0, 0.999)
            };
            set.accumulate(&mut pose, l.sequence, cycle, l.weight.min(1.0), &self.params);
        }
        pose
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn param(start: f32, end: f32, looping: f32) -> PoseParam {
        PoseParam {
            name: String::new(),
            start,
            end,
            looping,
        }
    }

    #[test]
    fn pose_parameters_encode() {
        let body_yaw = param(-90.0, 90.0, 360.0);
        assert!((body_yaw.encode(30.0) - 0.666667).abs() < 1e-5);
        assert_eq!(body_yaw.encode(120.0), 1.0);
        assert!((param(-180.0, 180.0, 360.0).encode(270.0) - 0.25).abs() < 1e-6);
        assert_eq!(param(-90.0, 90.0, 360.0).encode(-100.0), 0.0);
        assert_eq!(body_yaw.default_value(), 0.5);
        assert_eq!(param(-1.0, 1.0, 0.0).default_value(), 0.5);
    }

    #[test]
    fn triangle_blend_weights() {
        let close = |a: [f32; 3], b: [f32; 3]| a.iter().zip(b).all(|(x, y)| (x - y).abs() < 1e-5);
        let (c, w) = triangle(0, 0, 0.5, 0.133975);
        assert_eq!(c, [(0, 0), (1, 0), (1, 1)]);
        assert!(close(w, [0.5, 0.366025, 0.133975]), "{w:?}");
        let (c, w) = triangle(0, 0, 0.2, 0.6);
        assert_eq!(c, [(1, 1), (0, 1), (0, 0)]);
        assert!(close(w, [0.2, 0.4, 0.4]), "{w:?}");
        let (c, w) = triangle(1, 0, 1.0 / 3.0, 0.5);
        assert_eq!(c, [(1, 1), (1, 0), (2, 0)]);
        assert!(close(w, [0.5, 0.166667, 0.333333]), "{w:?}");
        let (c, w) = triangle(1, 0, 0.8, 0.6);
        assert_eq!(c, [(2, 0), (2, 1), (1, 1)]);
        assert!(close(w, [0.4, 0.4, 0.2]), "{w:?}");
    }

    #[test]
    fn euler_matches_the_spec() {
        let q = euler(0.1, 0.2, 0.3);
        let want = Quat::from_xyzw(0.034271, 0.106021, 0.143572, 0.983347);
        assert!(Vec4::from(q).distance(Vec4::from(want)) < 1e-5, "{q}");
    }

    #[test]
    fn fade_weights() {
        let f = Fading {
            sequence: 0,
            cycle: 0.0,
            playback: 1.0,
            stopped: 0.0,
            fade: 0.2,
        };
        assert!((f.weight(0.05) - 0.84375).abs() < 1e-5);
        assert!((f.weight(0.1) - 0.5).abs() < 1e-5);
        assert_eq!(f.weight(0.2), 0.0);
    }
}
