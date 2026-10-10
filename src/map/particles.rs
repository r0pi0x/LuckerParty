//! Particles for any game: a CPU particle pool of camera-facing sprites,
//! trails (strips along the velocity) and flat quads, in groups that share
//! a motion rule (gravity, decay, damping, drag, bounce planes). Games
//! build groups with their own numbers and add them to `Particles`; the
//! pool steps them once per rendered frame and draws them as one mesh per
//! material, rebuilt each frame, sorted back to front.
//!
//! The rules follow the "old style" client particles of
//! specs/cs_source/impact_effects.md section 4 (time step clamp, near
//! fade, bounce planes found once per group, the particle cap), which are
//! general enough for other games' simple effects. Engine space: meters,
//! Y up.

use std::collections::HashMap;

use bevy::{
    asset::embedded_asset,
    prelude::*,
    reflect::TypePath,
    render::render_resource::AsBindGroup,
    shader::ShaderRef,
};

use super::{MapPart, SkyboxCamera, ViewModelCamera};

/// How a particle material is drawn.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum ParticleBlend {
    /// Blended by alpha over the scene.
    #[default]
    Alpha,
    /// Added to the scene (colour x alpha).
    Additive,
}

/// A material particles can use: a texture, its blend, and which vertex
/// attributes it honours.
#[derive(Clone, Debug, Default)]
pub struct ParticleMaterial {
    pub name: String,
    /// Index into `MapData::textures`.
    pub texture: Option<usize>,
    pub blend: ParticleBlend,
    /// The particle's colour tints the texture (else white).
    pub vertex_color: bool,
    /// The particle's alpha fades it (else always opaque).
    pub vertex_alpha: bool,
    /// Drawn over everything (the game ignores depth). Not supported yet:
    /// drawn depth tested.
    pub no_depth: bool,
    /// Sprite sheet: sequences of frames, each frame a texture rectangle
    /// (min u, min v, max u, max v). Empty: the whole texture.
    pub sequences: Vec<Vec<[f32; 4]>>,
    /// Colour multiplier (sprite cards' `$overbrightfactor`; 0: none
    /// given, 1). Particle systems apply it (`psys`); hand-built effects
    /// use their own.
    pub overbright: f32,
}

/// The particle materials a map's game uses, found by name.
#[derive(Clone, Debug, Default)]
pub struct MapParticles {
    pub materials: Vec<ParticleMaterial>,
}

impl MapParticles {
    pub fn find(&self, name: &str) -> Option<usize> {
        self.materials.iter().position(|m| m.name.eq_ignore_ascii_case(name))
    }
}

/// When the pool steps and draws each frame (`Update`): games that show
/// a group for one frame only (rebuilt every frame, e.g. a smoke cloud)
/// add it between the two, with `kill_after: Some(0.0)`.
#[derive(SystemSet, Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ParticleSet {
    Step,
    Draw,
}

/// The loaded map's particle materials, for games to find theirs.
#[derive(Resource, Clone, Debug, Default)]
pub struct ParticleMaterials(pub MapParticles);

/// `bias(t, b) = t^(ln b / ln 0.5)`: `bias(0.5, b) = b`; 0.5 is linear.
pub fn bias(t: f32, b: f32) -> f32 {
    if (b - 0.5).abs() < 1e-6 || t <= 0.0 {
        return t.max(0.0);
    }
    t.powf(b.ln() / 0.5f32.ln())
}

/// A value going from `start` to `end` over a particle's life (between
/// life fractions `from` and `to`, usually 0 and 1), through `bias`
/// (0.5: linear).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Ramp {
    pub start: f32,
    pub end: f32,
    pub bias: f32,
    pub from: f32,
    pub to: f32,
}

impl Ramp {
    pub fn constant(v: f32) -> Self {
        Self::linear(v, v)
    }

    pub fn linear(start: f32, end: f32) -> Self {
        Self {
            start,
            end,
            bias: 0.5,
            from: 0.0,
            to: 1.0,
        }
    }

    pub fn with_bias(self, bias: f32) -> Self {
        Self { bias, ..self }
    }

    /// Only between life fractions `from` and `to`.
    pub fn between(self, from: f32, to: f32) -> Self {
        Self { from, to, ..self }
    }

    /// The value at life fraction `t`.
    pub fn at(&self, t: f32) -> f32 {
        let x = if self.to > self.from {
            ((t - self.from) / (self.to - self.from)).clamp(0.0, 1.0)
        } else if t >= self.from {
            1.0
        } else {
            0.0
        };
        self.start + (self.end - self.start) * bias(x, self.bias)
    }
}

/// How a particle's alpha changes with age (times its base alpha).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Fade {
    Ramp(Ramp),
    /// `r = 1 - age/life`: `r` while `r >= 0.75`, then `r²` (the old-style
    /// dust puff).
    Dust,
    /// Full until the last `secs` seconds, then fading linearly.
    Tail(f32),
    /// `(1 - age/life)^p` (Source's `Bias(1 - t, b)` is this with
    /// `p = ln b / ln 0.5`).
    Power(f32),
    /// Fade in from `start_alpha` to 1 between life fractions `fade_in`,
    /// out to 0 between `fade_out` (particle-system alpha fade). An empty
    /// fade-in window holds `start_alpha` until it.
    Windows {
        start_alpha: f32,
        fade_in: (f32, f32),
        fade_out: (f32, f32),
    },
}

impl Fade {
    pub fn at(&self, age: f32, life: f32) -> f32 {
        let t = if life > 0.0 { age / life } else { 1.0 };
        match *self {
            Fade::Ramp(r) => r.at(t),
            Fade::Dust => {
                let r = (1.0 - t).max(0.0);
                if r >= 0.75 { r } else { r * r }
            }
            Fade::Tail(secs) => ((life - age) / secs).clamp(0.0, 1.0),
            Fade::Power(p) => (1.0 - t).clamp(0.0, 1.0).powf(p),
            Fade::Windows {
                start_alpha,
                fade_in,
                fade_out,
            } => {
                let window = |(a, b): (f32, f32)| {
                    if b > a {
                        ((t - a) / (b - a)).clamp(0.0, 1.0)
                    } else if t >= a {
                        1.0
                    } else {
                        0.0
                    }
                };
                let fade_in = start_alpha + (1.0 - start_alpha) * window(fade_in);
                let fade_out = 1.0 - window(fade_out);
                fade_in * fade_out
            }
        }
    }
}

/// What a particle looks like.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Shape {
    /// A square facing the camera; `size` is its half-width.
    Sprite,
    /// A strip facing the camera from the particle along `velocity x
    /// length x max(0.01, 1 - age/life)` (`length` in seconds), `width`
    /// wide at most (meters).
    Trail { width: f32, length: f32 },
    /// A square in the plane with `normal`, turned `yaw` degrees about it;
    /// `size` is its full side.
    Flat { normal: Vec3, yaw: f32, yaw_speed: f32 },
    /// A quad with these corners (bottom-left, top-left, top-right,
    /// bottom-right; engine space) showing this texture rectangle (min u,
    /// min v, max u, max v): a segment of a trail.
    Quad { corners: [Vec3; 4], rect: [f32; 4] },
    /// A bullet tracer (specs/cs_source/tracers.md 3.1): a streak `length`
    /// long whose head leaves `start` along `dir` at `speed` (m/s), both
    /// ends kept between `start` and `distance` along; a camera-facing core
    /// of half-width `width` and a dim outline twice as wide.
    Streak {
        start: Vec3,
        dir: Vec3,
        distance: f32,
        length: f32,
        width: f32,
        speed: f32,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub struct Particle {
    pub position: Vec3,
    pub velocity: Vec3,
    pub age: f32,
    pub life: f32,
    /// Index into `MapParticles::materials`.
    pub material: usize,
    /// Sprite-sheet sequence, and its frame rate (0: first frame only).
    pub sequence: u16,
    pub fps: f32,
    /// The sequence starts over after its last frame (else holds it).
    pub frame_loop: bool,
    /// sRGB, 0-1 (above 1: overbright, for additive materials).
    pub color: Vec3,
    /// Blend toward this colour between these life fractions
    /// (particle-system colour fade).
    pub color_fade: Option<(Vec3, f32, f32)>,
    /// Base alpha, 0-1, times `fade`.
    pub alpha: f32,
    pub fade: Fade,
    /// Meters (see `Shape`).
    pub size: Ramp,
    /// Radians and radians per second.
    pub roll: f32,
    pub roll_speed: f32,
    pub shape: Shape,
}

impl Particle {
    /// A white, opaque, unmoving sprite of half-width `size`.
    pub fn new(position: Vec3, life: f32, material: usize, size: f32) -> Self {
        Self {
            position,
            velocity: Vec3::ZERO,
            age: 0.0,
            life,
            material,
            sequence: 0,
            fps: 0.0,
            frame_loop: false,
            color: Vec3::ONE,
            color_fade: None,
            alpha: 1.0,
            fade: Fade::Ramp(Ramp::constant(1.0)),
            size: Ramp::constant(size),
            roll: 0.0,
            roll_speed: 0.0,
            shape: Shape::Sprite,
        }
    }

    pub fn current_alpha(&self) -> f32 {
        (self.alpha * self.fade.at(self.age, self.life)).clamp(0.0, 1.0)
    }

    pub fn current_color(&self) -> Vec3 {
        match self.color_fade {
            Some((to, from_t, to_t)) => {
                let t = if self.life > 0.0 { self.age / self.life } else { 1.0 };
                let x = if to_t > from_t {
                    ((t - from_t) / (to_t - from_t)).clamp(0.0, 1.0)
                } else if t >= from_t {
                    1.0
                } else {
                    0.0
                };
                self.color.lerp(to, x)
            }
            None => self.color,
        }
    }

    pub fn current_size(&self) -> f32 {
        self.size.at(if self.life > 0.0 { self.age / self.life } else { 1.0 })
    }
}

/// Collision against a few planes found when the group was made (the
/// old-style "bounce planes"): particles crossing one re-trace the world
/// and bounce off or land on what they hit.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Bounce {
    /// Speed kept after a bounce: a fresh draw in `keep ± 0.1` each time.
    pub keep: f32,
    /// A hit surface with normal y at least this, approached slower than
    /// `land_speed` (m/s, vertical), stops the particle dead.
    pub land_normal_y: f32,
    pub land_speed: f32,
    /// Plane normal and distance (`n·p = d`).
    pub planes: Vec<(Vec3, f32)>,
}

/// How a group's particles move each frame.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Motion {
    /// Downward acceleration, m/s² (negative: upward).
    pub gravity: f32,
    /// Velocity decays by `10^(-4·dt/time)` each frame, then is raised to
    /// at least `min_speed` along its pre-decay direction (old-style dust).
    pub decay: Option<(f32, f32)>,
    /// Velocity times `max(0, 1 - damping·dt)` each frame, after moving.
    pub damping: f32,
    /// Fraction of velocity removed per frame, before moving
    /// (particle-system drag).
    pub drag: f32,
    pub bounce: Option<Bounce>,
    /// Roll speed `ω += ω·(-rate·dt)`, then kept at least `floor` in size.
    pub roll_decay: Option<(f32, f32)>,
    /// Sprites fade in from distance `.0` to `.1` (meters) in front of the
    /// camera plane.
    pub near_fade: Option<(f32, f32)>,
    /// A liquid surface at this height (meters): sprites fade as they
    /// sink into it (alpha × the part of their half-size still above it)
    /// and go once wholly below.
    pub sink: Option<f32>,
}

impl Motion {
    /// Draw factor for a sprite of half-size `size` at `p` (near fade and
    /// sinking), given its depth in front of the camera.
    pub fn draw_factor(&self, depth: f32, p: &Particle) -> f32 {
        let near = self
            .near_fade
            .map_or(1.0, |(a, b)| ((depth - a) / (b - a)).clamp(0.0, 1.0));
        let sink = self.sink.map_or(1.0, |y| {
            let half = (p.current_size() / 2.0).max(1e-6);
            ((p.position.y - (y - half)) / half).clamp(0.0, 1.0)
        });
        near * sink
    }
}

/// Particles that move and are cut together.
#[derive(Clone, Debug, Default)]
pub struct ParticleGroup {
    pub particles: Vec<Particle>,
    pub motion: Motion,
    /// Counts against (and is cut to) the pool's cap.
    pub capped: bool,
    /// Not moved on the frame it is added (drawn at its spawn state once).
    pub skip_first: bool,
    /// The whole group goes this long after it was added, seconds.
    pub kill_after: Option<f32>,
    /// Joins a live group with the same key if their boxes together stay
    /// under `extent` across: (key, extent in meters, spawn box min, max).
    pub merge: Option<(u32, f32, Vec3, Vec3)>,
    age: f32,
    fresh: bool,
}

impl ParticleGroup {
    pub fn new(motion: Motion) -> Self {
        Self {
            motion,
            capped: true,
            skip_first: true,
            ..default()
        }
    }

    fn bounds(&self) -> Option<(Vec3, Vec3)> {
        self.particles.iter().fold(None, |acc, p| match acc {
            None => Some((p.position, p.position)),
            Some((lo, hi)) => Some((lo.min(p.position), hi.max(p.position))),
        })
    }
}

/// A world trace for bounce planes: the fraction along `from -> to` of the
/// first world surface hit and its normal; fraction 0 when `from` is in
/// solid.
pub trait WorldTrace {
    fn trace(&self, from: Vec3, to: Vec3) -> Option<(f32, Vec3)>;
}

impl<F: Fn(Vec3, Vec3) -> Option<(f32, Vec3)>> WorldTrace for F {
    fn trace(&self, from: Vec3, to: Vec3) -> Option<(f32, Vec3)> {
        self(from, to)
    }
}

/// No world: nothing to bounce off.
pub struct NoWorld;

impl WorldTrace for NoWorld {
    fn trace(&self, _: Vec3, _: Vec3) -> Option<(f32, Vec3)> {
        None
    }
}

/// Plane tolerance for crossing tests (0.01 game units).
const PLANE_EPSILON: f32 = 0.01 * 0.0254;

/// Find bounce planes from `origin`: along `aim` and its right vector and
/// the opposite (or ±X, ±Y, ±Z without an aim), each probe 8 steps of
/// 0.25 s at `speed` m/s, falling `½·g·(0.25·i)²` per step (spec section
/// 4: the drop uses the cumulative time each step, so the path falls
/// faster than a parabola). The first hit of each probe adds its plane.
pub fn probe_planes(
    origin: Vec3,
    aim: Option<Vec3>,
    speed: f32,
    gravity: f32,
    world: &impl WorldTrace,
) -> Vec<(Vec3, f32)> {
    const STEPS: u32 = 8;
    const STEP: f32 = 0.25;
    let dirs: Vec<Vec3> = match aim.and_then(|a| a.try_normalize()) {
        Some(a) => {
            let right = a.cross(Vec3::Y).try_normalize().unwrap_or(Vec3::X);
            vec![a, right, -right]
        }
        None => vec![Vec3::X, Vec3::NEG_X, Vec3::Y, Vec3::NEG_Y, Vec3::Z, Vec3::NEG_Z],
    };
    let mut planes: Vec<(Vec3, f32)> = Vec::new();
    for dir in dirs {
        let mut from = origin;
        for i in 1..=STEPS {
            let t = STEP * i as f32;
            let mut to = from + dir * speed * STEP;
            to.y -= 0.5 * gravity * t * t;
            if let Some((f, n)) = world.trace(from, to) {
                let at = from.lerp(to, f);
                let plane = (n, n.dot(at));
                if !planes
                    .iter()
                    .any(|(pn, pd)| pn.abs_diff_eq(n, 1e-3) && (pd - plane.1).abs() < 1e-3)
                {
                    planes.push(plane);
                }
                break;
            }
            from = to;
        }
    }
    planes
}

/// Random numbers for the particle rules (xorshift).
#[derive(Clone, Debug)]
pub struct ParticleRng(u64);

impl ParticleRng {
    pub fn new(seed: u64) -> Self {
        Self(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1)
    }

    /// Uniform in 0..1.
    pub fn random(&mut self) -> f32 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        (self.0 >> 40) as f32 / (1u64 << 24) as f32
    }

    /// Uniform float in a..b.
    pub fn float(&mut self, a: f32, b: f32) -> f32 {
        a + (b - a) * self.random()
    }

    /// Uniform integer in a..=b.
    pub fn int(&mut self, a: i32, b: i32) -> i32 {
        if b <= a {
            return a;
        }
        (a + (self.random() * (b - a + 1) as f32) as i32).min(b)
    }

    /// Three independent draws in a..b.
    pub fn vec3(&mut self, a: f32, b: f32) -> Vec3 {
        Vec3::new(self.float(a, b), self.float(a, b), self.float(a, b))
    }
}

/// Every live particle group. Games add groups; the map steps and draws
/// them each rendered frame.
#[derive(Resource, Debug)]
pub struct Particles {
    pub groups: Vec<ParticleGroup>,
    /// Most capped particles alive at once; spawns past it are dropped.
    pub cap: usize,
    /// Largest time step, seconds (longer frames advance only this much).
    pub max_dt: f32,
    pub rng: ParticleRng,
}

impl Default for Particles {
    fn default() -> Self {
        Self {
            groups: Vec::new(),
            cap: 2048,
            max_dt: 0.1,
            rng: ParticleRng::new(0x5EED),
        }
    }
}

impl Particles {
    /// Live particles that count against the cap.
    pub fn capped_count(&self) -> usize {
        self.groups.iter().filter(|g| g.capped).map(|g| g.particles.len()).sum()
    }

    pub fn count(&self) -> usize {
        self.groups.iter().map(|g| g.particles.len()).sum()
    }

    /// Add a group, cut to the room left under the cap (the rest are
    /// dropped silently, in order). Returns how many particles were added.
    pub fn add(&mut self, mut group: ParticleGroup) -> usize {
        if group.capped {
            let room = self.cap.saturating_sub(self.capped_count());
            group.particles.truncate(room);
        }
        let n = group.particles.len();
        if n == 0 {
            return 0;
        }
        group.fresh = group.skip_first;
        group.age = 0.0;
        if let Some((key, extent, lo, hi)) = group.merge {
            let target = self.groups.iter_mut().find(|g| {
                g.merge.is_some_and(|m| m.0 == key)
                    && g.bounds()
                        .is_some_and(|(glo, ghi)| glo.min(lo).distance(ghi.max(hi)) < extent)
            });
            if let Some(g) = target {
                // The merged group takes the new impact's bounce planes.
                g.particles.append(&mut group.particles);
                g.motion.bounce = group.motion.bounce;
                return n;
            }
        }
        self.groups.push(group);
        n
    }

    /// Advance everything by the frame time `dt` (clamped to `max_dt`).
    pub fn step(&mut self, dt: f32, world: &impl WorldTrace) {
        let dt = dt.clamp(0.0, self.max_dt);
        let rng = &mut self.rng;
        for g in &mut self.groups {
            g.age += dt;
            if std::mem::take(&mut g.fresh) {
                continue;
            }
            let motion = &g.motion;
            g.particles.retain_mut(|p| {
                step_particle(p, motion, dt, world, rng);
                let sunk = motion.sink.is_some_and(|y| p.position.y + p.current_size() < y);
                p.age < p.life && !sunk
            });
        }
        self.groups
            .retain(|g| !g.particles.is_empty() && g.kill_after.is_none_or(|k| g.age < k));
    }
}

/// One particle's frame: velocity rule, position, age, roll.
pub fn step_particle(p: &mut Particle, m: &Motion, dt: f32, world: &impl WorldTrace, rng: &mut ParticleRng) {
    let moving = !(m.bounce.is_some() && p.velocity == Vec3::ZERO);
    if moving {
        if let Some((time, floor)) = m.decay {
            let before = p.velocity;
            p.velocity *= (1e-4f32.ln() * dt / time).exp();
            if p.velocity.length() < floor {
                p.velocity = before.normalize_or_zero() * floor;
            }
        }
        p.velocity.y -= m.gravity * dt;
        if m.drag > 0.0 {
            p.velocity *= (1.0 - m.drag).max(0.0);
        }
        match &m.bounce {
            Some(b) => bounce_move(p, b, dt, world, rng),
            None => p.position += p.velocity * dt,
        }
        if m.damping > 0.0 {
            p.velocity *= (1.0 - m.damping * dt).max(0.0);
        }
    }
    p.age += dt;
    p.roll += p.roll_speed * dt;
    if let Some((rate, floor)) = m.roll_decay {
        p.roll_speed += p.roll_speed * (-rate * dt);
        if p.roll_speed.abs() < floor {
            p.roll_speed = floor.copysign(p.roll_speed);
        }
    }
    if let Shape::Flat { yaw, yaw_speed, .. } = &mut p.shape {
        *yaw = (*yaw + *yaw_speed * dt).rem_euclid(360.0);
    }
}

/// Move with bounce planes (velocity already has this frame's gravity).
fn bounce_move(p: &mut Particle, b: &Bounce, dt: f32, world: &impl WorldTrace, rng: &mut ParticleRng) {
    let to = p.position + p.velocity * dt;
    let crosses = b.planes.iter().any(|(n, d)| {
        let (a, z) = (n.dot(p.position) - d, n.dot(to) - d);
        a >= -PLANE_EPSILON && z < PLANE_EPSILON
    });
    let hit = if crosses { world.trace(p.position, to) } else { None };
    match hit {
        Some((f, _)) if f <= 0.0 => {
            p.velocity = Vec3::ZERO;
            p.roll_speed = 0.0;
        }
        Some((f, n)) => {
            let draw = rng.float(b.keep - 0.1, b.keep + 0.1);
            bounce(p, f, n, dt, b, draw);
        }
        None => p.position = to,
    }
}

/// A particle hitting a surface with normal `n` at fraction `f` of this
/// frame's move: it lands (stops) on floors when slow enough, else
/// reflects keeping `keep` of its speed.
pub fn bounce(p: &mut Particle, f: f32, n: Vec3, dt: f32, b: &Bounce, keep: f32) {
    p.position += p.velocity * (f - 0.01) * dt;
    if n.y >= b.land_normal_y && p.velocity.y.abs() <= b.land_speed {
        p.velocity = Vec3::ZERO;
        p.roll_speed = 0.0;
    } else {
        p.velocity = (p.velocity - 2.0 * p.velocity.dot(n) * n) * keep;
        p.roll_speed *= -0.25;
    }
}

/// How particles are drawn (particle.wgsl): texture x vertex colour,
/// blended or added, with the scene's fog (`fog::SceneFog`): blended
/// particles fog toward the fog colour, added ones toward black.
#[derive(Asset, TypePath, AsBindGroup, Debug, Clone)]
#[bind_group_data(ParticleDrawKey)]
pub struct ParticleDrawMaterial {
    #[texture(0)]
    #[sampler(1)]
    pub texture: Option<Handle<Image>>,
    #[uniform(2)]
    pub fog: super::fog::FogUniform,
    pub blend: ParticleBlend,
}

#[repr(C)]
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct ParticleDrawKey {
    additive: bool,
}

impl From<&ParticleDrawMaterial> for ParticleDrawKey {
    fn from(m: &ParticleDrawMaterial) -> Self {
        Self {
            additive: m.blend == ParticleBlend::Additive,
        }
    }
}

impl Material for ParticleDrawMaterial {
    fn fragment_shader() -> ShaderRef {
        "embedded://mashup/map/particle.wgsl".into()
    }

    fn alpha_mode(&self) -> AlphaMode {
        match self.blend {
            ParticleBlend::Alpha => AlphaMode::Blend,
            ParticleBlend::Additive => AlphaMode::Add,
        }
    }

    fn specialize(
        _pipeline: &bevy::pbr::MaterialPipeline,
        descriptor: &mut bevy::render::render_resource::RenderPipelineDescriptor,
        _layout: &bevy::mesh::MeshVertexBufferLayoutRef,
        key: bevy::pbr::MaterialPipelineKey<Self>,
    ) -> Result<(), bevy::render::render_resource::SpecializedMeshPipelineError> {
        descriptor.primitive.cull_mode = None;
        if key.bind_group_data.additive
            && let Some(fragment) = descriptor.fragment.as_mut()
        {
            fragment.shader_defs.push("ADDITIVE".into());
        }
        Ok(())
    }

    fn enable_prepass() -> bool {
        false
    }

    fn enable_shadows() -> bool {
        false
    }
}

/// Registers the particle material. Needs rendering.
pub struct ParticleMaterialPlugin;

impl Plugin for ParticleMaterialPlugin {
    fn build(&self, app: &mut App) {
        embedded_asset!(app, "particle.wgsl");
        app.add_plugins((
            super::fog::FogShaderPlugin,
            MaterialPlugin::<ParticleDrawMaterial>::default(),
        ));
    }
}

/// The meshes particles are drawn with: one entity per material.
#[derive(Resource)]
pub(super) struct ParticleAssets {
    data: MapParticles,
    textures: Vec<Handle<Image>>,
    /// Material index -> (mesh entity, mesh, whether it was last written
    /// empty). An empty mesh isn't written again until particles come
    /// back: a modified mesh is uploaded again, and makes every mesh entity
    /// re-check its pipeline that frame.
    meshes: HashMap<usize, (Entity, Handle<Mesh>, bool)>,
}

impl ParticleAssets {
    pub(super) fn new(data: MapParticles, textures: Vec<Handle<Image>>) -> Self {
        Self {
            data,
            textures,
            meshes: HashMap::new(),
        }
    }
}

/// Traces against the world only: the map's static brushes and surfaces,
/// not props, characters or anything that moves (what bounce planes and
/// the game's "world brushes" traces see).
#[derive(bevy::ecs::system::SystemParam)]
pub struct WorldTracer<'w, 's> {
    query: avian3d::prelude::SpatialQuery<'w, 's>,
    statics: Query<'w, 's, &'static avian3d::prelude::RigidBody, Without<super::MapPropCollider>>,
}

impl WorldTrace for WorldTracer<'_, '_> {
    fn trace(&self, from: Vec3, to: Vec3) -> Option<(f32, Vec3)> {
        let d = to - from;
        let len = d.length();
        let dir = Dir3::new(d).ok()?;
        let filter = avian3d::prelude::SpatialQueryFilter::default();
        self.query
            .cast_ray_predicate(from, dir, len, true, &filter, &|e| {
                self.statics.get(e).is_ok_and(|b| b.is_static())
            })
            .map(|h| (h.distance / len, h.normal))
    }
}

/// Step the pool by the frame time against the world.
pub(super) fn step_particles(time: Res<Time>, mut particles: ResMut<Particles>, world: WorldTracer) {
    if particles.groups.is_empty() {
        return;
    }
    particles.step(time.delta_secs(), &world);
}

/// Rebuild the particle meshes for the camera.
#[allow(clippy::type_complexity)]
pub(super) fn draw_particles(
    particles: Res<Particles>,
    assets: Option<ResMut<ParticleAssets>>,
    cameras: Query<
        &GlobalTransform,
        (
            With<Camera3d>,
            Without<super::monitor::ScreenCamera>,
            Without<SkyboxCamera>,
            Without<ViewModelCamera>,
            Without<super::water::WaterReflectionCamera>,
        ),
    >,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<ParticleDrawMaterial>>,
    fog: Option<Res<super::fog::SceneFog>>,
    mut transforms: Query<&mut Transform>,
    mut commands: Commands,
) {
    let Some(mut assets) = assets else { return };
    let Some(eye) = cameras.iter().next() else { return };
    let (from, forward, right, up) = (
        eye.translation(),
        eye.forward().as_vec3(),
        eye.right().as_vec3(),
        eye.up().as_vec3(),
    );
    // Per material: (depth, particle, near fade).
    let mut buckets: HashMap<usize, Vec<(f32, &Particle, f32)>> = HashMap::new();
    for g in &particles.groups {
        for p in &g.particles {
            let depth = (p.position - from).dot(forward);
            let near = g.motion.draw_factor(depth, p);
            buckets.entry(p.material).or_default().push((depth, p, near));
        }
    }
    let mut used: Vec<usize> = assets.meshes.keys().chain(buckets.keys()).copied().collect();
    used.sort_unstable();
    used.dedup();
    for material in used {
        let Some(def) = assets.data.materials.get(material).cloned() else {
            continue;
        };
        let list = buckets.remove(&material).unwrap_or_default();
        if list.is_empty() && !assets.meshes.contains_key(&material) {
            continue;
        }
        let (entity, handle, was_empty) = match assets.meshes.get(&material) {
            Some(m) => m.clone(),
            None => {
                let handle = meshes.add(super::dust::empty_mesh());
                let texture = def.texture.and_then(|t| assets.textures.get(t).cloned());
                let entity = commands
                    .spawn((
                        Name::new(format!("Particles {}", def.name)),
                        MapPart,
                        Mesh3d(handle.clone()),
                        MeshMaterial3d(materials.add(ParticleDrawMaterial {
                            texture,
                            fog: fog.as_ref().map_or_else(Default::default, |f| f.0),
                            blend: def.blend,
                        })),
                        // The mesh moves with its particles.
                        bevy::camera::visibility::NoFrustumCulling,
                        bevy::light::NotShadowCaster,
                        Transform::default(),
                    ))
                    .id();
                assets.meshes.insert(material, (entity, handle.clone(), false));
                (entity, handle, false)
            }
        };
        let mut list = list;
        // Far to near (depth rounded to whole game units, as the game sorts).
        list.sort_by(|a, b| (b.0 / 0.0254).round().total_cmp(&(a.0 / 0.0254).round()));
        // Sorted against other transparent things by its particles' centre.
        let centre = if list.is_empty() {
            Vec3::ZERO
        } else {
            list.iter().map(|(_, p, _)| p.position).sum::<Vec3>() / list.len() as f32
        };
        let mut b = QuadBuilder::default();
        for (_, p, near) in list {
            let alpha = if def.vertex_alpha {
                p.current_alpha() * near
            } else {
                near.min(1.0)
            };
            if def.vertex_alpha && alpha < 0.001 {
                continue;
            }
            let color = if def.vertex_color {
                let c = p.current_color();
                Color::srgb(c.x, c.y, c.z).to_linear()
            } else {
                LinearRgba::WHITE
            };
            let rgba = [color.red, color.green, color.blue, alpha];
            let rect = frame_rect(&def, p);
            let size = p.current_size();
            let at = p.position - centre;
            match p.shape {
                Shape::Sprite => {
                    let (s, c) = p.roll.sin_cos();
                    let (r, u) = ((right * c + up * s) * size, (up * c - right * s) * size);
                    b.quad([at - r - u, at - r + u, at + r + u, at + r - u], rect, rgba);
                }
                Shape::Trail { width, length } => {
                    let life = if p.life > 0.0 { p.age / p.life } else { 1.0 };
                    let along = p.velocity * length * (1.0 - life).max(0.01);
                    let w = width.min(along.length()) / 2.0;
                    let side = along.cross(p.position - from).normalize_or_zero() * w;
                    // Texture V runs along the strip.
                    b.quad([at - side, at - side + along, at + side + along, at + side], rect, rgba);
                }
                Shape::Streak {
                    start,
                    dir,
                    distance,
                    length,
                    width,
                    speed,
                } => {
                    let head = (speed * p.age).clamp(0.0, distance);
                    let tail = (head - length).clamp(0.0, distance);
                    if head <= tail {
                        continue;
                    }
                    let (h, t) = (start + dir * head, start + dir * tail);
                    let side = (h - t).cross(h - from).normalize_or_zero();
                    let (h, t) = (h - centre, t - centre);
                    // Outline: twice as wide at 64/255 of the core (additive).
                    let dim = 64.0 / 255.0;
                    let outline = [rgba[0] * dim, rgba[1] * dim, rgba[2] * dim, rgba[3]];
                    let w2 = side * width * 2.0;
                    b.quad([t - w2, h - w2, h + w2, t + w2], rect, outline);
                    let w = side * width;
                    b.quad([t - w, h - w, h + w, t + w], rect, rgba);
                }
                Shape::Quad { corners, rect } => {
                    b.quad(corners.map(|c| c - centre), rect, rgba);
                }
                Shape::Flat { normal, yaw, .. } => {
                    let n = normal.normalize_or_zero();
                    let t = n.any_orthonormal_vector();
                    let t = Quat::from_axis_angle(n, yaw.to_radians()) * t;
                    let (r, u) = (t * size / 2.0, n.cross(t) * size / 2.0);
                    b.quad([at - r - u, at - r + u, at + r + u, at + r - u], rect, rgba);
                }
            }
        }
        let empty = b.positions.is_empty();
        if empty && was_empty {
            continue;
        }
        if let Some(m) = assets.meshes.get_mut(&material) {
            m.2 = empty;
        }
        if let Ok(mut t) = transforms.get_mut(entity) {
            t.translation = centre;
        }
        if let Some(mut mesh) = meshes.get_mut(&handle) {
            b.write(&mut mesh, -forward);
        }
    }
}

/// The texture rectangle a particle shows now.
fn frame_rect(def: &ParticleMaterial, p: &Particle) -> [f32; 4] {
    let Some(seq) = def.sequences.get(p.sequence as usize).filter(|s| !s.is_empty()) else {
        return [0.0, 0.0, 1.0, 1.0];
    };
    let frame = (p.age * p.fps).max(0.0) as usize;
    let frame = if p.frame_loop {
        frame % seq.len()
    } else {
        frame.min(seq.len() - 1)
    };
    seq[frame]
}

#[derive(Default)]
struct QuadBuilder {
    positions: Vec<[f32; 3]>,
    uvs: Vec<[f32; 2]>,
    colors: Vec<[f32; 4]>,
    indices: Vec<u32>,
}

impl QuadBuilder {
    /// Corners: bottom-left, top-left, top-right, bottom-right.
    fn quad(&mut self, corners: [Vec3; 4], [u0, v0, u1, v1]: [f32; 4], rgba: [f32; 4]) {
        let base = self.positions.len() as u32;
        for (p, uv) in corners.iter().zip([[u0, v1], [u0, v0], [u1, v0], [u1, v1]]) {
            self.positions.push(p.to_array());
            self.uvs.push(uv);
            self.colors.push(rgba);
        }
        self.indices
            .extend([base, base + 1, base + 2, base, base + 2, base + 3]);
    }

    fn write(self, mesh: &mut Mesh, normal: Vec3) {
        super::dust::write_dynamic_mesh(mesh, normal, self.positions, self.uvs, self.colors, self.indices);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn moving_group(n: usize) -> ParticleGroup {
        let mut g = ParticleGroup::new(Motion::default());
        for _ in 0..n {
            let mut p = Particle::new(Vec3::ZERO, 10.0, 0, 1.0);
            p.velocity = Vec3::X;
            g.particles.push(p);
        }
        g
    }

    #[test]
    fn cap_cuts_new_groups() {
        let mut pool = Particles::default();
        assert_eq!(pool.add(moving_group(2000)), 2000);
        assert_eq!(pool.add(moving_group(100)), 48);
        assert_eq!(pool.add(moving_group(10)), 0);
        assert_eq!(pool.capped_count(), 2048);
        // Uncapped groups (particle systems) still go in.
        let mut free = moving_group(10);
        free.capped = false;
        assert_eq!(pool.add(free), 10);
    }

    #[test]
    fn long_frames_advance_at_most_max_dt_and_new_groups_wait_a_frame() {
        let mut pool = Particles::default();
        pool.add(moving_group(1));
        pool.step(0.25, &NoWorld);
        // Created this frame: drawn once where it spawned.
        assert_eq!(pool.groups[0].particles[0].position, Vec3::ZERO);
        pool.step(0.25, &NoWorld);
        let p = &pool.groups[0].particles[0];
        assert!((p.position.x - 0.1).abs() < 1e-6 && (p.age - 0.1).abs() < 1e-6);
        // A pool group moves on its first frame.
        let mut g = moving_group(1);
        g.skip_first = false;
        pool.add(g);
        pool.step(0.05, &NoWorld);
        assert!((pool.groups[1].particles[0].position.x - 0.05).abs() < 1e-6);
    }

    #[test]
    fn nearby_groups_merge_and_take_new_planes() {
        let mut pool = Particles::default();
        let with = |x: f32, plane: f32| {
            let mut g = moving_group(1);
            g.particles[0].position = Vec3::X * x;
            g.motion.bounce = Some(Bounce {
                planes: vec![(Vec3::Y, plane)],
                ..default()
            });
            let r = Vec3::splat(0.127);
            g.merge = Some((1, 3.048, Vec3::X * x - r, Vec3::X * x + r));
            g
        };
        pool.add(with(0.0, 1.0));
        pool.add(with(1.0, 2.0));
        assert_eq!(pool.groups.len(), 1);
        assert_eq!(pool.groups[0].particles.len(), 2);
        assert_eq!(pool.groups[0].motion.bounce.as_ref().unwrap().planes[0].1, 2.0);
        pool.add(with(10.0, 3.0));
        assert_eq!(pool.groups.len(), 2);
    }

    #[test]
    fn bounce_planes_from_probes() {
        // A floor at y = 0: probes from 1 m up fall onto it.
        let floor = |from: Vec3, to: Vec3| -> Option<(f32, Vec3)> {
            (to.y < 0.0 && from.y >= 0.0).then(|| (from.y / (from.y - to.y), Vec3::Y))
        };
        let planes = probe_planes(Vec3::Y, Some(Vec3::X), 2.0, 20.0, &floor);
        assert_eq!(planes, vec![(Vec3::Y, 0.0)]);
        // A particle crossing the plane re-traces and lands.
        let b = Bounce {
            keep: 0.3,
            land_normal_y: 0.5,
            land_speed: 1.2,
            planes,
        };
        let m = Motion {
            bounce: Some(b),
            ..default()
        };
        let mut p = Particle::new(Vec3::Y * 0.05, 3.0, 0, 0.01);
        p.velocity = Vec3::new(0.2, -1.0, 0.0);
        step_particle(&mut p, &m, 0.1, &floor, &mut ParticleRng::new(1));
        assert_eq!(p.velocity, Vec3::ZERO);
        assert!(p.position.y >= 0.0);
    }

    #[test]
    fn ramps_and_bias() {
        assert!((bias(0.5, 0.7) - 0.7).abs() < 1e-6);
        let r = Ramp::linear(1.0, 0.75).between(0.5, 1.0);
        assert_eq!(r.at(0.25), 1.0);
        assert!((r.at(0.75) - 0.875).abs() < 1e-6);
        let w = Fade::Windows {
            start_alpha: 0.0,
            fade_in: (0.0, 0.125),
            fade_out: (0.25, 1.0),
        };
        assert_eq!(w.at(0.0, 1.0), 0.0);
        assert_eq!(w.at(0.2, 1.0), 1.0);
        assert!((w.at(0.625, 1.0) - 0.5).abs() < 1e-6);
    }
}
