//! Data-driven particle systems (Source's `.pcf` effects, run by
//! info_particle_system: specs/source/particles_and_smoke.md 1-2), for any
//! game whose loader fills `MapParticleSystems`. A system definition is a
//! list of emitters, initializers, operators, forces and renderers; their
//! meanings follow specs/cs_source/impact_effects.md 14 and
//! specs/source/fire.md 7.3 (the library that runs them is not public:
//! interpretations, listed in docs/tech-debt.md). Simulation is in the
//! game's units (Source: inches, Z up); particles reach the pool
//! (`particles`) as one-frame groups in engine space.

use bevy::prelude::*;

use super::EntityPart;
use super::particles::{Particle, ParticleGroup, ParticleRng, Particles, Ramp, Shape};

/// Control points a system can use.
pub const MAX_CONTROL_POINTS: usize = 8;

/// One particle system definition.
#[derive(Clone, Debug, PartialEq)]
pub struct SystemDef {
    pub name: String,
    pub max_particles: usize,
    pub initial_particles: usize,
    /// Index into `MapData::particles::materials`.
    pub material: Option<usize>,
    /// Defaults every particle starts with (radius in units, colour and
    /// alpha 0-1, roll and roll speed in degrees).
    pub radius: f32,
    pub color: Vec3,
    pub alpha: f32,
    pub rotation: f32,
    pub rotation_speed: f32,
    pub sequence: u32,
    /// Largest step, seconds.
    pub max_step: f32,
    /// Not drawn (nor run) farther than this from the eye, units.
    pub max_draw_distance: f32,
    pub emitters: Vec<Emitter>,
    pub initializers: Vec<Init>,
    pub operators: Vec<Op>,
    pub renderer: Render,
    /// Child systems (index into the definitions) and their delay, s.
    pub children: Vec<(usize, f32)>,
}

impl Default for SystemDef {
    fn default() -> Self {
        Self {
            name: String::new(),
            max_particles: 1000,
            initial_particles: 0,
            material: None,
            radius: 5.0,
            color: Vec3::ONE,
            alpha: 1.0,
            rotation: 0.0,
            rotation_speed: 0.0,
            sequence: 0,
            max_step: 0.1,
            max_draw_distance: 100_000.0,
            emitters: Vec::new(),
            initializers: Vec::new(),
            operators: Vec::new(),
            renderer: Render::None,
            children: Vec::new(),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Emitter {
    /// `rate` a second from `start` for `duration` s (0: forever).
    Continuously { start: f32, rate: f32, duration: f32 },
    /// `count` at `start`.
    Instantaneously { start: f32, count: u32 },
}

/// A uniform pick between two values with an exponent on the fraction.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Pick {
    pub min: f32,
    pub max: f32,
    pub exp: f32,
}

impl Pick {
    pub fn new(min: f32, max: f32, exp: f32) -> Self {
        Self { min, max, exp }
    }

    pub fn draw(&self, rng: &mut ParticleRng) -> f32 {
        let f = rng.random();
        let f = if self.exp == 1.0 || self.exp <= 0.0 {
            f
        } else {
            f.powf(self.exp)
        };
        self.min + (self.max - self.min) * f
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Init {
    /// Position at a random direction × distance from the control point
    /// (scaled per axis by `bias`; an axis with `bias_abs` set takes the
    /// absolute value: a half sphere), velocity outward × speed plus a
    /// random vector between the two local speeds in the point's frame.
    Sphere {
        cp: usize,
        distance: Pick,
        bias: Vec3,
        bias_abs: Vec3,
        speed: Pick,
        local_min: Vec3,
        local_max: Vec3,
    },
    /// Position in a box about the control point.
    Box {
        cp: usize,
        min: Vec3,
        max: Vec3,
    },
    /// Position moved by U(min, max) per axis (× radius when
    /// proportional, in the point's frame when local).
    Offset {
        cp: usize,
        min: Vec3,
        max: Vec3,
        local: bool,
        proportional: bool,
    },
    /// Velocity: a random direction × speed plus a random local vector.
    Velocity {
        cp: usize,
        speed: Pick,
        local_min: Vec3,
        local_max: Vec3,
    },
    /// Velocity per axis in [min, max] (from noise: here uniform).
    VelocityNoise {
        cp: usize,
        min: Vec3,
        max: Vec3,
        local: bool,
    },
    Lifetime(Pick),
    /// Life = the chosen sheet sequence's frames / fps.
    LifetimeFromSequence {
        fps: f32,
    },
    Radius(Pick),
    /// A blend between two colours (0-1).
    Color {
        a: Vec3,
        b: Vec3,
    },
    /// Alpha 0-1.
    Alpha(Pick),
    /// Roll = initial + U(min, max), degrees.
    Rotation {
        initial: f32,
        offset: Pick,
    },
    /// Mirror this fraction of particles.
    YawFlip {
        fraction: f32,
    },
    Sequence {
        min: u32,
        max: u32,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub enum Op {
    /// Gravity (units/s²), and drag: the fraction of velocity removed per
    /// step.
    Movement {
        gravity: Vec3,
        drag: f32,
    },
    /// Dies at the end of its life.
    Decay,
    /// Alpha × 0→1 over a random time (fraction of life when
    /// proportional) from birth.
    FadeIn {
        time: Pick,
        proportional: bool,
    },
    /// Alpha × 1→0 over a random time before death.
    FadeOut {
        time: Pick,
        proportional: bool,
    },
    /// Alpha × start→1 over [fade_in], 1→end over [fade_out] (fractions
    /// of life); dies at the end of its life.
    FadeAndDecay {
        start_alpha: f32,
        end_alpha: f32,
        fade_in: (f32, f32),
        fade_out: (f32, f32),
    },
    /// Colour toward `color` between the two fractions of life.
    ColorFade {
        color: Vec3,
        start: f32,
        end: f32,
        ease: bool,
    },
    /// Radius × lerp(start_scale, end_scale, bias(x)) between the two
    /// fractions of life.
    RadiusScale {
        start_scale: f32,
        end_scale: f32,
        start: f32,
        end: f32,
        bias: f32,
        ease: bool,
    },
    /// Roll by the particle's roll speed.
    RotationBasic,
    /// Roll (or yaw) at this rate, degrees/s.
    SpinRoll {
        rate: f32,
    },
    SpinYaw {
        rate: f32,
    },
    /// Speed at most this, units/s.
    MaxVelocity {
        max: f32,
    },
    /// Moves with the control point.
    LockToControlPoint {
        cp: usize,
    },
    /// Acceleration toward the control point: force / distance^falloff.
    Pull {
        cp: usize,
        force: f32,
        falloff: f32,
    },
    /// A random acceleration per axis in [min, max] each step.
    RandomForce {
        min: Vec3,
        max: Vec3,
    },
    /// A tangential acceleration about the axis through the control point.
    Twist {
        cp: usize,
        force: f32,
        axis: Vec3,
        local: bool,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub enum Render {
    None,
    /// Camera-facing (orientation 0) or flat (others) sprites from the
    /// material's sheet: `rate` frames a second when `as_fps`, the
    /// sequence over the particle's life when `fit_life`, else `rate`
    /// times the sequence a second.
    Sprites {
        rate: f32,
        as_fps: bool,
        fit_life: bool,
        orientation: i32,
    },
    /// A streak back along the velocity, between the two lengths (units).
    Trail {
        min_length: f32,
        max_length: f32,
        length_fade_in: f32,
    },
    /// A strip through the particles in birth order.
    Rope,
}

/// The loaded map's particle systems: definitions and the
/// info_particle_systems placed in it.
#[derive(Clone, Debug, Default)]
pub struct MapParticleSystems {
    pub defs: Vec<SystemDef>,
    pub placed: Vec<PlacedSystem>,
    /// Meters per game unit.
    pub scale: f32,
}

impl MapParticleSystems {
    pub fn find(&self, name: &str) -> Option<usize> {
        self.defs.iter().position(|d| d.name.eq_ignore_ascii_case(name))
    }
}

/// An info_particle_system.
#[derive(Clone, Debug, PartialEq)]
pub struct PlacedSystem {
    /// The entity (index into `MapData::entities`).
    pub entity: Option<usize>,
    /// Engine space.
    pub position: Vec3,
    pub rotation: Quat,
    /// Index into `MapParticleSystems::defs`.
    pub system: usize,
    pub start_on: bool,
}

/// A control point: position and orientation in game axes.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ControlPoint {
    pub position: Vec3,
    pub rotation: Quat,
}

impl Default for ControlPoint {
    fn default() -> Self {
        Self {
            position: Vec3::ZERO,
            rotation: Quat::IDENTITY,
        }
    }
}

#[derive(Clone, Debug)]
struct P {
    position: Vec3,
    velocity: Vec3,
    age: f32,
    life: f32,
    radius: f32,
    color: Vec3,
    alpha: f32,
    roll: f32,
    roll_speed: f32,
    sequence: u32,
    flip: bool,
    fade_in: f32,
    fade_out: f32,
    /// Its birth order (ropes).
    serial: u32,
}

/// A running effect: its particles, its children and its clock.
#[derive(Clone, Debug)]
pub struct Instance {
    pub def: usize,
    pub age: f32,
    pub emitting: bool,
    particles: Vec<P>,
    /// Continuous emitters' carried fractions.
    carry: Vec<f32>,
    children: Vec<(f32, Option<Instance>, usize)>,
    serial: u32,
    started: bool,
    /// Where the locked-to control point was last step.
    lock_from: Option<Vec3>,
}

/// The sheet sequences' frame counts of a material (for "lifetime from
/// sequence" and frame animation).
pub trait Sheets {
    fn frames(&self, material: usize, sequence: u32) -> usize;
    /// The material's colour multiplier (sprite cards' overbright).
    fn overbright(&self, _material: usize) -> f32 {
        1.0
    }
}

/// The map's particle materials as `Sheets`.
pub struct MaterialSheets<'a>(pub Option<&'a super::particles::MapParticles>);

impl Sheets for MaterialSheets<'_> {
    fn frames(&self, material: usize, sequence: u32) -> usize {
        self.0
            .and_then(|ms| ms.materials.get(material))
            .and_then(|m| m.sequences.get(sequence as usize))
            .map_or(1, Vec::len)
    }

    fn overbright(&self, material: usize) -> f32 {
        self.0
            .and_then(|ms| ms.materials.get(material))
            .map(|m| m.overbright)
            .filter(|o| *o > 0.0)
            .unwrap_or(1.0)
    }
}

impl<F: Fn(usize, u32) -> usize> Sheets for F {
    fn frames(&self, material: usize, sequence: u32) -> usize {
        self(material, sequence)
    }
}

fn smooth(x: f32) -> f32 {
    x * x * (3.0 - 2.0 * x)
}

fn window(t: f32, a: f32, b: f32) -> f32 {
    if b > a {
        ((t - a) / (b - a)).clamp(0.0, 1.0)
    } else if t >= a {
        1.0
    } else {
        0.0
    }
}

fn pick3(rng: &mut ParticleRng, a: Vec3, b: Vec3) -> Vec3 {
    Vec3::new(rng.float(a.x, b.x), rng.float(a.y, b.y), rng.float(a.z, b.z))
}

fn random_dir(rng: &mut ParticleRng) -> Vec3 {
    loop {
        let v = rng.vec3(-1.0, 1.0);
        let l = v.length_squared();
        if l > 1e-6 && l <= 1.0 {
            return v / l.sqrt();
        }
    }
}

impl Instance {
    pub fn new(defs: &[SystemDef], def: usize) -> Self {
        let d = &defs[def];
        Self {
            def,
            age: 0.0,
            emitting: true,
            particles: Vec::new(),
            carry: vec![0.0; d.emitters.len()],
            children: d.children.iter().map(|&(c, delay)| (delay, None, c)).collect(),
            serial: 0,
            started: false,
            lock_from: None,
        }
    }

    /// Particles alive in it and its children.
    pub fn count(&self) -> usize {
        self.particles.len()
            + self
                .children
                .iter()
                .filter_map(|c| c.1.as_ref())
                .map(Instance::count)
                .sum::<usize>()
    }

    /// Stop emitting (its children too); live particles finish.
    pub fn stop(&mut self) {
        self.emitting = false;
        for c in self.children.iter_mut().filter_map(|c| c.1.as_mut()) {
            c.stop();
        }
    }

    /// Nothing left to show or make.
    pub fn finished(&self, defs: &[SystemDef]) -> bool {
        let d = &defs[self.def];
        let emits = self.emitting
            && d.emitters.iter().any(|e| match *e {
                Emitter::Continuously { start, duration, .. } => duration <= 0.0 || self.age < start + duration,
                Emitter::Instantaneously { start, .. } => self.age <= start,
            });
        let children_left = self.children.iter().any(|(delay, c, _)| match c {
            Some(c) => !c.finished(defs),
            None => self.emitting && self.age < *delay,
        });
        !emits && self.particles.is_empty() && !children_left
    }

    /// Advance `dt` (cut into steps of at most the system's largest step).
    pub fn step(
        &mut self,
        defs: &[SystemDef],
        cps: &[ControlPoint],
        dt: f32,
        rng: &mut ParticleRng,
        sheets: &impl Sheets,
    ) {
        let max = defs[self.def].max_step.max(0.01);
        let mut left = dt;
        while left > 1e-6 {
            let h = left.min(max);
            self.tick(defs, cps, h, rng, sheets);
            left -= h;
        }
    }

    fn tick(&mut self, defs: &[SystemDef], cps: &[ControlPoint], dt: f32, rng: &mut ParticleRng, sheets: &impl Sheets) {
        let d = &defs[self.def];
        let cp = |i: usize| cps.get(i).or(cps.first()).copied().unwrap_or_default();
        let before = self.age;
        self.age += dt;
        // Emission.
        let mut new = 0usize;
        if !self.started {
            self.started = true;
            new += d.initial_particles;
        }
        if self.emitting {
            for (i, e) in d.emitters.iter().enumerate() {
                match *e {
                    Emitter::Continuously { start, rate, duration } => {
                        let end = if duration > 0.0 {
                            start + duration
                        } else {
                            f32::INFINITY
                        };
                        let a = before.max(start);
                        let b = self.age.min(end);
                        if b > a {
                            self.carry[i] += rate * (b - a);
                            let n = self.carry[i].floor();
                            self.carry[i] -= n;
                            new += n as usize;
                        }
                    }
                    Emitter::Instantaneously { start, count } => {
                        let due = if start <= 0.0 {
                            before == 0.0
                        } else {
                            before < start && self.age >= start
                        };
                        if due {
                            new += count as usize;
                        }
                    }
                }
            }
        }
        let room = d.max_particles.saturating_sub(self.particles.len());
        for _ in 0..new.min(room) {
            let p = self.spawn(d, &cp, rng, sheets);
            self.particles.push(p);
        }
        // Forces, movement, life.
        let decays = d
            .operators
            .iter()
            .any(|o| matches!(o, Op::Decay | Op::FadeAndDecay { .. }));
        let lock: Option<usize> = d.operators.iter().find_map(|o| match o {
            Op::LockToControlPoint { cp } => Some(*cp),
            _ => None,
        });
        for p in &mut self.particles {
            p.age += dt;
            let mut accel = Vec3::ZERO;
            let mut gravity = Vec3::ZERO;
            let mut drag = 0.0;
            let mut moves = false;
            for o in &d.operators {
                match *o {
                    Op::Movement { gravity: g, drag: k } => {
                        gravity += g;
                        drag = k;
                        moves = true;
                    }
                    Op::Pull { cp: c, force, falloff } => {
                        let to = cp(c).position - p.position;
                        let dist = to.length().max(1.0);
                        accel += to / dist * force / dist.powf(falloff);
                    }
                    Op::RandomForce { min, max } => accel += pick3(rng, min, max),
                    Op::Twist {
                        cp: c,
                        force,
                        axis,
                        local,
                    } => {
                        let point = cp(c);
                        let axis = if local { point.rotation * axis } else { axis }.normalize_or_zero();
                        let off = p.position - point.position;
                        accel += axis.cross(off).normalize_or_zero() * force;
                    }
                    _ => {}
                }
            }
            if moves {
                p.velocity += (gravity + accel) * dt;
                if drag > 0.0 {
                    p.velocity *= (1.0 - drag).max(0.0);
                }
                for o in &d.operators {
                    if let Op::MaxVelocity { max } = *o
                        && max > 0.0
                    {
                        p.velocity = p.velocity.clamp_length_max(max);
                    }
                }
                p.position += p.velocity * dt;
            }
            for o in &d.operators {
                match *o {
                    Op::RotationBasic => p.roll += p.roll_speed * dt,
                    Op::SpinRoll { rate } => p.roll += rate * dt,
                    _ => {}
                }
            }
        }
        if let Some(c) = lock {
            let now = cp(c).position;
            if let Some(prev) = self.lock_from {
                let delta = now - prev;
                for p in &mut self.particles {
                    p.position += delta;
                }
            }
            self.lock_from = Some(now);
        }
        if decays {
            self.particles.retain(|p| p.age < p.life);
        }
        // Children start after their delay (while the parent emits).
        for (delay, child, def) in &mut self.children {
            if child.is_none() && self.emitting && self.age >= *delay {
                *child = Some(Instance::new(defs, *def));
            }
        }
        for child in self.children.iter_mut().filter_map(|c| c.1.as_mut()) {
            child.tick(defs, cps, dt, rng, sheets);
        }
    }

    fn spawn(
        &mut self,
        d: &SystemDef,
        cp: &impl Fn(usize) -> ControlPoint,
        rng: &mut ParticleRng,
        sheets: &impl Sheets,
    ) -> P {
        self.serial += 1;
        let mut p = P {
            position: cp(0).position,
            velocity: Vec3::ZERO,
            age: 0.0,
            life: 1.0,
            radius: d.radius,
            color: d.color,
            alpha: d.alpha,
            roll: d.rotation,
            roll_speed: d.rotation_speed,
            sequence: d.sequence,
            flip: false,
            fade_in: 0.0,
            fade_out: 0.0,
            serial: self.serial,
        };
        for i in &d.initializers {
            match *i {
                Init::Sphere {
                    cp: c,
                    distance,
                    bias,
                    bias_abs,
                    speed,
                    local_min,
                    local_max,
                } => {
                    let point = cp(c);
                    let mut dir = random_dir(rng) * bias;
                    for k in 0..3 {
                        if bias_abs[k] != 0.0 {
                            dir[k] = dir[k].abs();
                        }
                    }
                    let dir = dir.normalize_or_zero();
                    p.position = point.position + dir * distance.draw(rng);
                    p.velocity = dir * speed.draw(rng) + point.rotation * pick3(rng, local_min, local_max);
                }
                Init::Box { cp: c, min, max } => p.position = cp(c).position + pick3(rng, min, max),
                Init::Offset {
                    cp: c,
                    min,
                    max,
                    local,
                    proportional,
                } => {
                    let mut off = pick3(rng, min, max);
                    if proportional {
                        off *= p.radius;
                    }
                    if local {
                        off = cp(c).rotation * off;
                    }
                    p.position += off;
                }
                Init::Velocity {
                    cp: c,
                    speed,
                    local_min,
                    local_max,
                } => {
                    p.velocity += random_dir(rng) * speed.draw(rng) + cp(c).rotation * pick3(rng, local_min, local_max);
                }
                Init::VelocityNoise { cp: c, min, max, local } => {
                    let v = pick3(rng, min, max);
                    p.velocity += if local { cp(c).rotation * v } else { v };
                }
                Init::Lifetime(pick) => p.life = pick.draw(rng),
                Init::LifetimeFromSequence { fps } => {
                    if let Some(m) = d.material
                        && fps > 0.0
                    {
                        p.life = sheets.frames(m, p.sequence) as f32 / fps;
                    }
                }
                Init::Radius(pick) => p.radius = pick.draw(rng),
                Init::Color { a, b } => p.color = a.lerp(b, rng.random()),
                Init::Alpha(pick) => p.alpha = pick.draw(rng),
                Init::Rotation { initial, offset } => p.roll = initial + offset.draw(rng),
                Init::YawFlip { fraction } => p.flip = rng.random() < fraction,
                Init::Sequence { min, max } => p.sequence = rng.int(min as i32, max as i32).max(0) as u32,
            }
        }
        for o in &d.operators {
            match *o {
                Op::FadeIn { time, .. } => p.fade_in = time.draw(rng),
                Op::FadeOut { time, .. } => p.fade_out = time.draw(rng),
                _ => {}
            }
        }
        p
    }

    /// What it shows now, as one-frame pool particles in engine space
    /// (`to_engine` maps a game-space point), facing `eye` (engine).
    pub fn draw(
        &self,
        defs: &[SystemDef],
        to_engine: &impl Fn(Vec3) -> Vec3,
        scale: f32,
        eye: Vec3,
        sheets: &impl Sheets,
        out: &mut Vec<Particle>,
    ) {
        let d = &defs[self.def];
        if let Some(material) = d.material {
            match &d.renderer {
                Render::Rope => {
                    let mut order: Vec<&P> = self.particles.iter().collect();
                    order.sort_by_key(|p| p.serial);
                    for pair in order.windows(2) {
                        let (a, b) = (pair[0], pair[1]);
                        let (pa, pb) = (to_engine(a.position), to_engine(b.position));
                        let mid = (pa + pb) * 0.5;
                        let side = (pb - pa).cross(mid - eye).normalize_or_zero();
                        let (ra, rb) = (Self::radius(d, a) * scale, Self::radius(d, b) * scale);
                        let mut q = Particle::new(mid, 1.0, material, 0.0);
                        q.color = Self::color(d, a);
                        q.alpha = Self::alpha(d, a);
                        q.shape = Shape::Quad {
                            corners: [pa - side * ra, pb - side * rb, pb + side * rb, pa + side * ra],
                            rect: [0.0, 0.0, 1.0, 1.0],
                        };
                        out.push(q);
                    }
                }
                render => {
                    for p in &self.particles {
                        let radius = Self::radius(d, p);
                        let alpha = Self::alpha(d, p);
                        if radius <= 0.0 || alpha <= 0.001 {
                            continue;
                        }
                        let at = to_engine(p.position);
                        let mut q = Particle::new(at, p.life.max(1e-3), material, radius * scale);
                        q.size = Ramp::constant(radius * scale);
                        q.age = p.age;
                        q.color = Self::color(d, p) * sheets.overbright(material);
                        q.alpha = alpha;
                        q.roll = p.roll.to_radians() * if p.flip { -1.0 } else { 1.0 };
                        q.sequence = p.sequence as u16;
                        match *render {
                            Render::Sprites {
                                rate,
                                as_fps,
                                fit_life,
                                orientation,
                            } => {
                                let frames = sheets.frames(material, p.sequence).max(1) as f32;
                                q.fps = if fit_life {
                                    frames / p.life.max(1e-3)
                                } else if as_fps {
                                    rate
                                } else {
                                    rate * frames
                                };
                                q.frame_loop = !fit_life;
                                if orientation != 0 {
                                    q.shape = Shape::Flat {
                                        normal: Vec3::Y,
                                        yaw: p.roll,
                                        yaw_speed: 0.0,
                                    };
                                    q.size = Ramp::constant(radius * scale * 2.0);
                                }
                            }
                            Render::Trail {
                                min_length,
                                max_length,
                                length_fade_in,
                            } => {
                                let speed = p.velocity.length();
                                let mut length = (speed * 0.1).clamp(min_length, max_length.max(min_length));
                                if length_fade_in > 0.0 {
                                    length *= (p.age / length_fade_in).min(1.0);
                                }
                                let back = to_engine(p.position - p.velocity.normalize_or_zero() * length);
                                let side = (at - back).cross(at - eye).normalize_or_zero() * radius * scale;
                                q.position = (at + back) * 0.5;
                                q.shape = Shape::Quad {
                                    corners: [back - side, at - side, at + side, back + side],
                                    rect: [0.0, 0.0, 1.0, 1.0],
                                };
                            }
                            _ => {}
                        }
                        out.push(q);
                    }
                }
            }
        }
        for child in self.children.iter().filter_map(|c| c.1.as_ref()) {
            child.draw(defs, to_engine, scale, eye, sheets, out);
        }
    }

    fn radius(d: &SystemDef, p: &P) -> f32 {
        let t = if p.life > 0.0 { p.age / p.life } else { 1.0 };
        let mut r = p.radius;
        for o in &d.operators {
            if let Op::RadiusScale {
                start_scale,
                end_scale,
                start,
                end,
                bias,
                ease,
            } = *o
            {
                let mut x = window(t, start, end);
                if ease {
                    x = smooth(x);
                }
                r *= start_scale + (end_scale - start_scale) * super::particles::bias(x, bias);
            }
        }
        r
    }

    fn alpha(d: &SystemDef, p: &P) -> f32 {
        let t = if p.life > 0.0 { p.age / p.life } else { 1.0 };
        let mut a = p.alpha;
        for o in &d.operators {
            match *o {
                Op::FadeIn { proportional, .. } => {
                    let time = if proportional { p.fade_in * p.life } else { p.fade_in };
                    if time > 0.0 {
                        a *= (p.age / time).clamp(0.0, 1.0);
                    }
                }
                Op::FadeOut { proportional, .. } => {
                    let time = if proportional { p.fade_out * p.life } else { p.fade_out };
                    if time > 0.0 {
                        a *= ((p.life - p.age) / time).clamp(0.0, 1.0);
                    }
                }
                Op::FadeAndDecay {
                    start_alpha,
                    end_alpha,
                    fade_in,
                    fade_out,
                } => {
                    let fin = start_alpha + (1.0 - start_alpha) * window(t, fade_in.0, fade_in.1);
                    let fout = 1.0 + (end_alpha - 1.0) * window(t, fade_out.0, fade_out.1);
                    a *= fin * fout;
                }
                _ => {}
            }
        }
        a.clamp(0.0, 1.0)
    }

    fn color(d: &SystemDef, p: &P) -> Vec3 {
        let t = if p.life > 0.0 { p.age / p.life } else { 1.0 };
        let mut c = p.color;
        for o in &d.operators {
            if let Op::ColorFade {
                color,
                start,
                end,
                ease,
            } = *o
            {
                let mut x = window(t, start, end);
                if ease {
                    x = smooth(x);
                }
                c = c.lerp(color, x);
            }
        }
        c
    }
}

/// An info_particle_system's effect: its running instances (a new one at
/// each start; stopped ones fade out), following its entity.
#[derive(Component, Debug)]
pub struct SystemEmitter {
    pub placed: PlacedSystem,
    pub instances: Vec<Instance>,
    was_on: bool,
    rng: ParticleRng,
}

impl SystemEmitter {
    pub fn new(placed: PlacedSystem, seed: u64) -> Self {
        Self {
            placed,
            instances: Vec::new(),
            was_on: false,
            rng: ParticleRng::new(seed),
        }
    }

    /// Turned on: a new instance; turned off: every instance stops
    /// emitting (2.5).
    pub fn switch(&mut self, defs: &[SystemDef], on: bool) {
        if on && !self.was_on {
            self.instances.push(Instance::new(defs, self.placed.system));
        } else if !on && self.was_on {
            for i in &mut self.instances {
                i.stop();
            }
        }
        self.was_on = on;
    }

    pub fn count(&self) -> usize {
        self.instances.iter().map(Instance::count).sum()
    }
}

/// The loaded map's particle system definitions (`MapData::particle_systems`).
#[derive(Resource, Clone, Debug, Default)]
pub struct ParticleSystemDefs(pub std::sync::Arc<MapParticleSystems>);

/// Run and show every placed particle system near enough and not hidden by
/// visibility (hidden ones are frozen, as the game sleeps unseen effects).
#[allow(clippy::type_complexity)]
pub(super) fn update_systems(
    time: Res<Time>,
    defs: Option<Res<ParticleSystemDefs>>,
    materials: Option<Res<super::particles::ParticleMaterials>>,
    eyes: Query<
        &GlobalTransform,
        (
            With<Camera3d>,
            Without<super::SkyboxCamera>,
            Without<super::ViewModelCamera>,
            Without<super::water::WaterReflectionCamera>,
        ),
    >,
    mut emitters: Query<(
        &mut SystemEmitter,
        &GlobalTransform,
        Option<&EntityPart>,
        Option<&Visibility>,
    )>,
    mut pool: ResMut<Particles>,
) {
    let Some(defs) = defs else { return };
    let defs = &defs.0;
    let dt = time.delta_secs().min(0.25);
    let eye = eyes.iter().next().map(|e| e.translation());
    let sheets = MaterialSheets(materials.as_ref().map(|m| &m.0));
    let scale = defs.scale.max(1e-6);
    // Engine space to game space and back (Source: Z up).
    let to_game = |v: Vec3| Vec3::new(v.x, -v.z, v.y) / scale;
    let to_engine = |v: Vec3| Vec3::new(v.x, v.z, -v.y) * scale;
    for (mut e, at, part, visibility) in &mut emitters {
        let exists = part.is_none_or(|p| p.exists);
        let on = exists && part.map_or(e.placed.start_on, |p| p.on);
        e.switch(&defs.defs, on);
        if !exists {
            e.instances.clear();
            continue;
        }
        if e.instances.is_empty() {
            continue;
        }
        let (_, rotation, origin) = at.to_scale_rotation_translation();
        let far = defs.defs[e.placed.system].max_draw_distance * scale;
        let near = eye.is_some_and(|eye| eye.distance(origin) <= far);
        if visibility == Some(&Visibility::Hidden) || !near {
            continue;
        }
        let cp = ControlPoint {
            position: to_game(origin),
            rotation: Quat::from_xyzw(rotation.x, -rotation.z, rotation.y, rotation.w),
        };
        let cps = [cp; MAX_CONTROL_POINTS];
        let e = &mut *e;
        for i in &mut e.instances {
            i.step(&defs.defs, &cps, dt, &mut e.rng, &sheets);
        }
        e.instances.retain(|i| !i.finished(&defs.defs));
        let Some(eye) = eye else { continue };
        let mut out = Vec::new();
        for i in &e.instances {
            i.draw(&defs.defs, &to_engine, scale, eye, &sheets, &mut out);
        }
        if !out.is_empty() {
            let mut g = ParticleGroup::new(default());
            g.particles = out;
            g.capped = false;
            g.skip_first = true;
            g.kill_after = Some(0.0);
            pool.add(g);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn none(_: usize, _: u32) -> usize {
        1
    }

    fn system() -> SystemDef {
        SystemDef {
            name: "test".into(),
            material: Some(0),
            emitters: vec![Emitter::Continuously {
                start: 0.0,
                rate: 100.0,
                duration: 0.5,
            }],
            initializers: vec![Init::Lifetime(Pick::new(0.2, 0.2, 1.0))],
            operators: vec![
                Op::Movement {
                    gravity: Vec3::new(0.0, 0.0, 50.0),
                    drag: 0.0,
                },
                Op::Decay,
            ],
            renderer: Render::Sprites {
                rate: 1.0,
                as_fps: false,
                fit_life: false,
                orientation: 0,
            },
            ..default()
        }
    }

    #[test]
    fn continuous_emission_for_its_duration_then_dies_out() {
        let defs = vec![system()];
        let mut i = Instance::new(&defs, 0);
        let mut rng = ParticleRng::new(1);
        let cps = [ControlPoint::default()];
        for _ in 0..10 {
            i.step(&defs, &cps, 0.01, &mut rng, &none);
        }
        // 100 a second for 0.1 s; none older than 0.2 s yet.
        assert!((9..=11).contains(&i.count()), "{}", i.count());
        for _ in 0..40 {
            i.step(&defs, &cps, 0.01, &mut rng, &none);
        }
        // At 0.5 s: the last 0.2 s worth alive.
        assert!((19..=21).contains(&i.count()), "{}", i.count());
        assert!(!i.finished(&defs));
        for _ in 0..30 {
            i.step(&defs, &cps, 0.01, &mut rng, &none);
        }
        assert_eq!(i.count(), 0);
        assert!(i.finished(&defs));
    }

    #[test]
    fn gravity_moves_and_stop_lets_particles_finish() {
        let mut d = system();
        d.emitters = vec![Emitter::Instantaneously { start: 0.0, count: 5 }];
        d.initializers = vec![Init::Lifetime(Pick::new(1.0, 1.0, 1.0))];
        let defs = vec![d];
        let mut i = Instance::new(&defs, 0);
        let mut rng = ParticleRng::new(2);
        let cps = [ControlPoint::default()];
        i.step(&defs, &cps, 0.1, &mut rng, &none);
        assert_eq!(i.count(), 5);
        i.stop();
        i.step(&defs, &cps, 0.5, &mut rng, &none);
        assert_eq!(i.count(), 5, "stopping keeps live particles");
        // Rising under gravity 50 up for 0.6 s.
        assert!(
            i.particles.iter().all(|p| p.position.z > 5.0),
            "{:?}",
            i.particles[0].position
        );
        i.step(&defs, &cps, 0.5, &mut rng, &none);
        assert_eq!(i.count(), 0);
    }

    #[test]
    fn fades_and_radius_scale() {
        let d = SystemDef {
            operators: vec![
                Op::FadeAndDecay {
                    start_alpha: 0.0,
                    end_alpha: 0.0,
                    fade_in: (0.0, 0.25),
                    fade_out: (0.5, 1.0),
                },
                Op::RadiusScale {
                    start_scale: 2.0,
                    end_scale: 20.0,
                    start: 0.0,
                    end: 1.0,
                    bias: 0.5,
                    ease: false,
                },
                Op::ColorFade {
                    color: Vec3::ZERO,
                    start: 0.0,
                    end: 1.0,
                    ease: false,
                },
            ],
            ..system()
        };
        let p = |age: f32| P {
            position: Vec3::ZERO,
            velocity: Vec3::ZERO,
            age,
            life: 1.0,
            radius: 5.0,
            color: Vec3::ONE,
            alpha: 1.0,
            roll: 0.0,
            roll_speed: 0.0,
            sequence: 0,
            flip: false,
            fade_in: 0.0,
            fade_out: 0.0,
            serial: 0,
        };
        assert_eq!(Instance::alpha(&d, &p(0.0)), 0.0);
        assert!((Instance::alpha(&d, &p(0.125)) - 0.5).abs() < 1e-5);
        assert_eq!(Instance::alpha(&d, &p(0.4)), 1.0);
        assert!((Instance::alpha(&d, &p(0.75)) - 0.5).abs() < 1e-5);
        assert!((Instance::radius(&d, &p(0.5)) - 55.0).abs() < 1e-3);
        assert!((Instance::color(&d, &p(0.25)) - Vec3::splat(0.75)).length() < 1e-5);
    }

    #[test]
    fn children_start_after_their_delay() {
        let mut parent = system();
        parent.children = vec![(1, 0.3)];
        let child = system();
        let defs = vec![parent, child];
        let mut i = Instance::new(&defs, 0);
        let mut rng = ParticleRng::new(3);
        let cps = [ControlPoint::default()];
        i.step(&defs, &cps, 0.2, &mut rng, &none);
        assert!(i.children[0].1.is_none());
        i.step(&defs, &cps, 0.15, &mut rng, &none);
        assert!(i.children[0].1.is_some());
    }

    #[test]
    fn emitter_switches_make_instances() {
        let defs = vec![system()];
        let placed = PlacedSystem {
            entity: None,
            position: Vec3::ZERO,
            rotation: Quat::IDENTITY,
            system: 0,
            start_on: false,
        };
        let mut e = SystemEmitter::new(placed, 1);
        e.switch(&defs, true);
        e.switch(&defs, true);
        assert_eq!(e.instances.len(), 1);
        e.switch(&defs, false);
        assert!(!e.instances[0].emitting);
        // Started again: a new instance beside the fading one.
        e.switch(&defs, true);
        assert_eq!(e.instances.len(), 2);
    }
}
