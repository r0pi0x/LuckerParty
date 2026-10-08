//! Bullet and knife impact effects (specs/cs_source/impact_effects.md):
//! the surface's game material letter picks debris flecks and dust
//! (concrete, tile, wood), a dust puff (dirt, sand), metal sparks (metal,
//! vent) or electric sparks (computer); flecks and dust take the hit
//! surface's colour. Players bleed (`blood_impact_red_01`, built from the
//! spec's table of the install's `.pcf`) and stain the walls behind them;
//! bullets into water splash instead. The particle machinery is
//! `map::particles`; the numbers and choices are here.
//!
//! Units: the spec's inches and in/s, converted to meters at the end
//! (`UNIT`). Source's up (z) is our y.

use bevy::prelude::*;

use super::impacts::{surface, surface_of};
use crate::{
    core::{Damage, Intent, MapWater, MapWaterVolume},
    map::{
        PlaySound, PropIndex, PropSurface,
        decal::{DecalGroup, PlaceDecal},
        particles::{
            Bounce, Fade, MapParticles, Motion, Particle, ParticleGroup, ParticleMaterials, ParticleRng, Particles,
            Ramp, Shape, WorldTrace, WorldTracer, probe_planes,
        },
        sound::{SoundBank, SurfaceGrid},
        surface_color::{SurfaceColors, SurfaceLook},
    },
    weapon::{WeaponEvent, WeaponEventKind},
};

const UNIT: f32 = 0.0254;

/// Old-style particles alive at once; further spawns fail silently.
pub const MAX_PARTICLES: usize = 2048;
/// Longest particle time step per frame, s.
pub const MAX_DT: f32 = 0.1;
/// Effect scale for bullets and the knife (template; spec Q2).
const SCALE: f32 = 1.0;
/// Particle budget factor (closed code, spec Q4): always full.
const THROTTLE: f32 = 1.0;
/// Simple sprites fade in between these distances from the camera plane, in.
const NEAR_FADE: (f32, f32) = (16.0, 64.0);
const FLECK_GRAVITY: f32 = 800.0;
const FLECK_LIFE: f32 = 3.0;
const FLECK_KEEP: f32 = 0.3;
/// Fleck groups merge while their boxes together stay under this, in.
const FLECK_MERGE: f32 = 120.0;
/// A fleck landing on a floor (normal z >= 0.5) at most this fast stops, in/s.
const LAND_SPEED: f32 = 48.0;
const DUST_DECAY_TIME: f32 = 0.5;
const DUST_MIN_SPEED: f32 = 32.0;
const SPARK_GRAVITY: f32 = 400.0;
const SPARK_DAMPING: f32 = 8.0;
/// Share of bullet impacts that play the ricochet (spec section 2).
pub const RICOCHET_CHANCE: f32 = 0.3;
pub const RICOCHET: &str = "Bounce.Shrapnel";
/// Blood-on-wall traces behind a hit player, in.
const BLEED_TRACE: f32 = 172.0;
/// Merge key for fleck groups.
const FLECK_KEY: u32 = 1;

/// Every material the effects use, loaded with the map.
pub const MATERIALS: &[&str] = &[
    "effects/fleck_cement1",
    "effects/fleck_cement2",
    "effects/fleck_wood1",
    "effects/fleck_wood2",
    "particle/particle_smokegrenade",
    "effects/blood",
    "effects/spark",
    "effects/yellowflare",
    "effects/yellowflare_noz",
    "particle/particle_noisesphere",
    "particle/smoke1/smoke1_nearcull2",
    "particle/blood_core",
    "particle/antlion_goop3/antlion_goop3",
    "particle/particle_glow_03",
    "effects/splash2",
    "effects/splashwake1",
    "effects/fleck_glass1",
    "effects/fleck_glass2",
];

/// Load the effect materials (runtime, from the install).
pub fn load_materials(loader: &mut super::material::MaterialLoader) -> MapParticles {
    MapParticles {
        materials: MATERIALS
            .iter()
            .chain(super::grenades::MATERIALS)
            .chain(super::fire::MATERIALS)
            .filter_map(|m| loader.particle(m))
            .collect(),
    }
}

/// The effect materials by role (indices into the map's particle
/// materials; None when the install lacks one).
#[derive(Clone, Debug, Default)]
pub struct EffectMaterials {
    pub fleck_cement: [Option<usize>; 2],
    pub fleck_wood: [Option<usize>; 2],
    pub smoke: Option<usize>,
    /// `effects/blood`: the grit/speck sprite.
    pub grit: Option<usize>,
    pub spark: Option<usize>,
    pub flare: Option<usize>,
    pub flare_noz: Option<usize>,
    pub noise: Option<usize>,
    pub blood_smoke: Option<usize>,
    pub blood_core: Option<usize>,
    pub goop: Option<usize>,
    pub glow: Option<usize>,
    pub splash: Option<usize>,
    pub wake: Option<usize>,
    pub fleck_glass: [Option<usize>; 2],
}

impl EffectMaterials {
    pub fn new(p: &MapParticles) -> Self {
        let f = |n: &str| p.find(n);
        Self {
            fleck_cement: [f("effects/fleck_cement1"), f("effects/fleck_cement2")],
            fleck_wood: [f("effects/fleck_wood1"), f("effects/fleck_wood2")],
            smoke: f("particle/particle_smokegrenade"),
            grit: f("effects/blood"),
            spark: f("effects/spark"),
            flare: f("effects/yellowflare"),
            flare_noz: f("effects/yellowflare_noz"),
            noise: f("particle/particle_noisesphere"),
            blood_smoke: f("particle/smoke1/smoke1_nearcull2"),
            blood_core: f("particle/blood_core"),
            goop: f("particle/antlion_goop3/antlion_goop3"),
            glow: f("particle/particle_glow_03"),
            splash: f("effects/splash2"),
            wake: f("effects/splashwake1"),
            fleck_glass: [f("effects/fleck_glass1"), f("effects/fleck_glass2")],
        }
    }

    /// Every role filled with material 0 (tests).
    pub fn all(index: usize) -> Self {
        let s = Some(index);
        Self {
            fleck_cement: [s, s],
            fleck_wood: [s, s],
            smoke: s,
            grit: s,
            spark: s,
            flare: s,
            flare_noz: s,
            noise: s,
            blood_smoke: s,
            blood_core: s,
            goop: s,
            glow: s,
            splash: s,
            wake: s,
            fleck_glass: [s, s],
        }
    }
}

/// The cvars the effects obey (0 or 1).
#[derive(Resource, Clone, Debug)]
pub struct ImpactEffectSettings {
    /// Debris flecks on concrete, tile and wood.
    pub r_drawflecks: i32,
    /// Splashes where bullets enter water.
    pub cl_show_splashes: i32,
    /// Blood from hit players (particles and wall stains).
    pub violence_hblood: i32,
}

impl Default for ImpactEffectSettings {
    fn default() -> Self {
        Self {
            r_drawflecks: 1,
            cl_show_splashes: 1,
            violence_hblood: 1,
        }
    }
}

/// The effect a surface's game material letter makes (spec section 3).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Effect {
    /// Debris flecks and dust; wood flecks or cement ones.
    Debris {
        wood: bool,
    },
    DustPuff,
    MetalSparks,
    ElectricSparks,
    None,
}

pub fn effect_for(letter: char) -> Effect {
    match letter.to_ascii_uppercase() {
        'C' | 'T' => Effect::Debris { wood: false },
        'W' => Effect::Debris { wood: true },
        'D' | 'N' => Effect::DustPuff,
        'M' | 'V' => Effect::MetalSparks,
        'P' => Effect::ElectricSparks,
        _ => Effect::None,
    }
}

/// `c = L^(1/2.2) · B` per channel; an unresolved surface gives 255
/// (out of range: every clamp-ramp of it is 255, bright white).
pub fn surface_color(look: Option<SurfaceLook>) -> Vec3 {
    match look {
        Some(l) => l.light.max(Vec3::ZERO).powf(1.0 / 2.2) * l.base,
        None => Vec3::splat(255.0),
    }
}

/// Clamp-ramp: `min(1, c·r)·255` truncated, as 0-1.
pub fn clamp_ramp(c: f32, r: f32) -> f32 {
    ((c * r).min(1.0) * 255.0).floor().max(0.0) / 255.0
}

fn ramp3(c: Vec3, r: f32) -> Vec3 {
    Vec3::new(clamp_ramp(c.x, r), clamp_ramp(c.y, r), clamp_ramp(c.z, r))
}

/// Flecks per impact: `int(0.5 + θ·s·draw)`, draw = I(4, 16).
pub fn fleck_count(theta: f32, scale: f32, draw: i32) -> usize {
    (0.5 + theta * scale * draw as f32) as usize
}

/// Per-axis spray added to the normal: `max(0.2, 0.6 − 0.2·s)`.
pub fn fleck_spray(scale: f32) -> f32 {
    (0.6 - 0.2 * scale).max(0.2)
}

/// A fleck's velocity: (normal + cube) × speed × (3 − size).
pub fn fleck_velocity(normal: Vec3, cube: Vec3, speed: f32, size: i32) -> Vec3 {
    (normal + cube) * speed * (3 - size) as f32
}

/// Dust puff counts (big puffs, specks, hit puffs) at throttle θ.
pub fn dust_counts(theta: f32) -> (usize, usize, usize) {
    let f = 4.0 * theta;
    ((0.5 + f) as usize, (0.83 + f) as usize, (0.17 + f) as usize)
}

/// The k-th big puff and speck's strength index.
const STRENGTH: [i32; 4] = [3, 1, 2, 0];

/// A big puff's start and end size (half-widths, in): draw = I(3, 4).
pub fn big_puff_size(scale: f32, draw: i32, j: i32) -> (f32, f32) {
    let start = (scale * (draw * (j + 1)) as f32) as i32 as f32;
    (start, (scale * start * 4.0) as i32 as f32)
}

/// Metal sparks per impact: `int(0.5 + θ·draw·2s)`, draw = I(4, 8).
pub fn metal_spark_count(theta: f32, draw: i32, scale: f32) -> usize {
    (0.5 + theta * draw as f32 * 2.0 * scale) as usize
}

/// A metal spark's speed range for spread draw `t` (0..2), in/s.
pub fn metal_spark_speed(t: f32) -> (f32, f32) {
    (128.0 * (2.0 - t), 512.0 * (2.0 - t))
}

/// A sprite at `p` moving at `v` (both already in meters).
fn sprite(p: Vec3, v: Vec3, life: f32, material: usize, color: Vec3) -> Particle {
    Particle {
        velocity: v,
        color,
        ..Particle::new(p, life, material, 0.0)
    }
}

/// Up in Source is our y: lower a velocity's vertical part by `dz` in/s.
fn down(v: Vec3, dz: f32) -> Vec3 {
    v - Vec3::Y * dz * UNIT
}

fn pool_motion() -> Motion {
    Motion {
        near_fade: Some((NEAR_FADE.0 * UNIT, NEAR_FADE.1 * UNIT)),
        ..default()
    }
}

/// Debris flecks + dust (C, T, W; spec section 5) at hit `p` with surface
/// normal `n` (unit, engine space) and surface colour `c`.
pub fn debris(
    rng: &mut ParticleRng,
    m: &EffectMaterials,
    p: Vec3,
    n: Vec3,
    wood: bool,
    c: Vec3,
    flecks: bool,
    world: &impl WorldTrace,
) -> Vec<ParticleGroup> {
    let s = SCALE;
    let mut out = Vec::new();
    let fleck_mats = if wood { m.fleck_wood } else { m.fleck_cement };
    if flecks && fleck_mats.iter().any(Option::is_some) {
        let origin = p + n * UNIT;
        let count = fleck_count(THROTTLE, s, rng.int(4, 16));
        let sigma = fleck_spray(s);
        let speed = (64.0 + 128.0 * s) / 2.0;
        let planes = probe_planes(origin, Some(n), speed * UNIT, FLECK_GRAVITY * UNIT, world);
        let mut g = ParticleGroup::new(Motion {
            gravity: FLECK_GRAVITY * UNIT,
            bounce: Some(Bounce {
                keep: FLECK_KEEP,
                land_normal_y: 0.5,
                land_speed: LAND_SPEED * UNIT,
                planes,
            }),
            ..default()
        });
        let r = Vec3::splat(5.0 * UNIT);
        g.merge = Some((FLECK_KEY, FLECK_MERGE * UNIT, origin - r, origin + r));
        for _ in 0..count {
            let cube = rng.vec3(-sigma, sigma);
            let size = rng.int(1, 2);
            let v = fleck_velocity(n, cube, rng.float(64.0, 128.0 * s), size) * UNIT;
            let pick = rng.int(0, 1) as usize;
            let Some(mat) = fleck_mats[pick].or(fleck_mats[1 - pick]) else {
                continue;
            };
            let roll = rng.float(0.0, 360.0);
            let roll_speed = rng.float(0.0, 360.0);
            let color = ramp3(c, rng.float(0.75, 1.25));
            g.particles.push(Particle {
                fade: Fade::Ramp(Ramp::linear(1.0, 0.0)),
                size: Ramp::constant(size as f32 * UNIT),
                roll,
                roll_speed,
                ..sprite(origin, v, FLECK_LIFE, mat, color)
            });
        }
        out.push(g);
    }
    // The dust sprites go to the global pool: moved on their first frame.
    let mut pool = ParticleGroup::new(pool_motion());
    pool.skip_first = false;
    let at = p + n * 2.0 * UNIT;
    if let Some(smoke) = m.smoke {
        for i in 0..2 {
            let k = (i + 1) as f32;
            let dir = n + rng.vec3(-0.8, 0.8);
            let start = rng.int(2, 4) as f32 * s;
            let v = down(dir * rng.float(2.0, 24.0) * k * UNIT, rng.float(8.0, 32.0) * k);
            let alpha = rng.int(100, 200) as f32 / 255.0;
            let roll = rng.float(0.0, 360.0);
            let roll_speed = rng.float(-1.0, 1.0);
            let color = ramp3(c, rng.float(0.5, 1.25));
            pool.particles.push(Particle {
                alpha,
                fade: Fade::Ramp(Ramp::linear(1.0, 0.0)),
                size: Ramp::linear(start * UNIT, start * 8.0 * s * UNIT),
                roll,
                roll_speed,
                ..sprite(at, v, 1.0, smoke, color)
            });
        }
    }
    if let Some(grit) = m.grit {
        for _ in 0..4 {
            let life = rng.float(0.25, 0.5);
            let dir = n + rng.vec3(-0.8, 0.8);
            let start = rng.int(1, 4) as f32;
            let v = down(dir * rng.float(8.0, 32.0) * UNIT, rng.float(8.0, 64.0));
            let roll_speed = rng.float(-2.0, 2.0);
            let color = ramp3(c, rng.float(0.5, 1.25));
            pool.particles.push(Particle {
                fade: Fade::Ramp(Ramp::linear(1.0, 0.0)),
                size: Ramp::linear(start * UNIT, start * 4.0 * UNIT),
                roll_speed,
                ..sprite(at, v, life, grit, color)
            });
        }
    }
    if let Some(smoke) = m.smoke {
        let life = rng.float(1.0, 1.5);
        let dir = n + rng.vec3(-0.8, 0.8);
        let start = rng.int(4, 8) as f32;
        let mut v = dir * rng.float(2.0, 24.0) * UNIT;
        v.y = rng.float(-2.0, 2.0) * UNIT;
        let alpha = rng.int(100, 200) as f32 / 255.0;
        let roll_speed = rng.float(-2.0, 2.0);
        let color = ramp3(c, rng.float(0.5, 1.25));
        pool.particles.push(Particle {
            alpha,
            fade: Fade::Ramp(Ramp::linear(1.0, 0.0)),
            size: Ramp::linear(start * UNIT, start * 4.0 * UNIT),
            roll_speed,
            ..sprite(at, v, life, smoke, color)
        });
    }
    out.push(pool);
    out
}

/// Dust puff (D, N; spec section 6) at hit `p`.
pub fn dust_puff(rng: &mut ParticleRng, m: &EffectMaterials, p: Vec3, n: Vec3, c: Vec3) -> ParticleGroup {
    let s = SCALE;
    let mut g = ParticleGroup::new(Motion {
        decay: Some((DUST_DECAY_TIME, DUST_MIN_SPEED * UNIT)),
        roll_decay: Some((8.0, 0.5)),
        ..pool_motion()
    });
    let dust = |rng: &mut ParticleRng, mat: usize, v: Vec3, life: f32, size: (f32, f32), roll_speed: f32| {
        let color = ramp3(c, rng.float(0.75, 1.25));
        Particle {
            fade: Fade::Dust,
            size: Ramp::linear(size.0 * UNIT, size.1 * UNIT),
            roll_speed,
            ..sprite(p, v, life, mat, color)
        }
    };
    let (big, specks, hits) = dust_counts(THROTTLE);
    for k in 0..big.max(specks) {
        let j = STRENGTH[k % 4];
        if k < big
            && let Some(smoke) = m.smoke
        {
            let life = rng.float(0.5, 1.0);
            let dir = (rng.vec3(-0.2, 0.2) + n * rng.float(1.0, 6.0)).normalize_or_zero();
            let v = dir * rng.float(250.0, 500.0) * j as f32 * s * UNIT;
            let size = big_puff_size(s, rng.int(3, 4), j);
            let _start_alpha = rng.int(32, 255);
            let roll_speed = rng.float(-8.0, 8.0);
            g.particles.push(dust(rng, smoke, v, life, size, roll_speed));
        }
        if k < specks
            && let Some(grit) = m.grit
        {
            let life = rng.float(0.25, 0.75);
            let dir = (rng.vec3(-0.2, 0.2) + n * rng.float(1.0, 6.0)).normalize_or_zero();
            let v = dir * rng.float(250.0, 500.0) * j as f32 * UNIT;
            let start = (rng.int(2, 4) * (j + 1)) as f32;
            let roll_speed = rng.float(-2.0, 2.0);
            g.particles
                .push(dust(rng, grit, v, life, (start, start * 2.0), roll_speed));
        }
    }
    if let Some(smoke) = m.smoke {
        for _ in 0..hits {
            let life = rng.float(0.5, 1.0);
            let v = (rng.vec3(-1.0, 1.0) + n).normalize_or_zero() * rng.float(0.0, 50.0) * UNIT;
            // A random offset is drawn but unused: every hit puff starts at
            // the hit point (spec quirk).
            let _offset = (rng.float(-8.0, 8.0), rng.float(-8.0, 8.0));
            let start = rng.int(1, 4) as f32;
            let roll_speed = rng.float(-16.0, 16.0);
            g.particles
                .push(dust(rng, smoke, v, life, (start, start * 4.0), roll_speed));
        }
    }
    g
}

/// Metal sparks (M, V; spec section 7) at hit `p`, shot direction `d`.
pub fn metal_sparks(rng: &mut ParticleRng, m: &EffectMaterials, p: Vec3, n: Vec3, d: Vec3) -> Vec<ParticleGroup> {
    let s = SCALE;
    let mut out = Vec::new();
    let aim = d - 2.0 * d.dot(n) * n + rng.vec3(-0.2, 0.2);
    let origin = p + n * UNIT;
    if let Some(spark) = m.spark {
        let mut g = ParticleGroup::new(Motion {
            gravity: SPARK_GRAVITY * UNIT,
            damping: SPARK_DAMPING,
            ..default()
        });
        let count = metal_spark_count(THROTTLE, rng.int(4, 8), s);
        for i in 0..count {
            let t = rng.float(0.0, 2.0);
            let dir = (aim + rng.vec3(-0.5 * t, 0.5 * t)).normalize_or_zero();
            let (lo, hi) = metal_spark_speed(t);
            let v = dir * rng.float(lo, hi) * UNIT;
            let width = rng.float(1.0, 4.0) * UNIT;
            let length = rng.float(0.025, 0.1);
            let life = if s > 1.0 && i % 3 == 0 {
                rng.float(0.15, 0.25)
            } else {
                rng.float(0.05, 0.1)
            };
            g.particles.push(Particle {
                shape: Shape::Trail { width, length },
                ..sprite(origin, v, life, spark, Vec3::ONE)
            });
        }
        out.push(g);
    }
    if let Some(flare) = m.flare {
        let mut glow = ParticleGroup::new(Motion::default());
        glow.skip_first = false;
        let yaw = rng.int(0, 360) as f32;
        let size = rng.int(24, 28) as f32 * UNIT;
        glow.particles.push(Particle {
            fade: Fade::Ramp(Ramp::linear(1.0, 0.0)),
            size: Ramp::linear(size, 0.0),
            shape: Shape::Flat {
                normal: n,
                yaw,
                yaw_speed: 0.0,
            },
            ..sprite(origin, Vec3::ZERO, 0.1, flare, Vec3::ONE)
        });
        out.push(glow);
    }
    out
}

/// Electric sparks (P; spec section 8) at hit `p`.
pub fn electric_sparks(
    rng: &mut ParticleRng,
    m: &EffectMaterials,
    p: Vec3,
    n: Vec3,
    world: &impl WorldTrace,
) -> Vec<ParticleGroup> {
    let origin = p + n * UNIT;
    let mut out = Vec::new();
    if let Some(spark) = m.spark {
        let planes = probe_planes(origin, None, 182.0 * UNIT, 800.0 * UNIT, world);
        let mut big = ParticleGroup::new(Motion {
            gravity: 800.0 * UNIT,
            bounce: Some(Bounce {
                keep: 0.3,
                land_normal_y: 0.5,
                land_speed: LAND_SPEED * UNIT,
                planes,
            }),
            ..default()
        });
        for _ in 0..rng.float(2.0, 4.0) as usize {
            let life = rng.float(1.0, 2.0);
            // Source (x, y, z) = (U(-1,1), U(-1,1), U(0.5,1)); z is our y.
            let dir = Vec3::new(rng.float(-1.0, 1.0), rng.float(0.5, 1.0), rng.float(-1.0, 1.0));
            let width = rng.float(2.0, 5.0) * UNIT;
            let length = rng.float(0.02, 0.05);
            let v = dir * rng.float(64.0, 300.0) * UNIT;
            big.particles.push(Particle {
                shape: Shape::Trail { width, length },
                ..sprite(origin, v, life, spark, Vec3::ONE)
            });
        }
        out.push(big);
        let mut small = ParticleGroup::new(Motion {
            gravity: SPARK_GRAVITY * UNIT,
            ..default()
        });
        for _ in 0..rng.int(16, 32) {
            let dir = rng.vec3(-1.0, 1.0);
            let width = rng.float(2.0, 4.0) * UNIT;
            let length = rng.float(0.02, 0.03);
            let life = rng.float(0.1, 0.2);
            let v = dir * rng.float(128.0, 256.0) * UNIT;
            small.particles.push(Particle {
                shape: Shape::Trail { width, length },
                ..sprite(origin, v, life, spark, Vec3::ONE)
            });
        }
        out.push(small);
    }
    // The glow: gone 0.2 s after it was made.
    let mut glow = ParticleGroup::new(Motion::default());
    glow.kill_after = Some(0.2);
    if let Some(flare) = m.flare_noz.or(m.flare) {
        let size = rng.int(4, 8) as f32 * UNIT;
        glow.particles.push(Particle {
            size: Ramp::linear(size, 0.0),
            ..sprite(origin, Vec3::ZERO, 0.2, flare, Vec3::ONE)
        });
        let grey = rng.int(32, 64) as f32 / 255.0;
        let size = rng.int(32, 64) as f32 * UNIT;
        let roll_speed = rng.float(-1.0, 1.0);
        glow.particles.push(Particle {
            alpha: grey,
            fade: Fade::Ramp(Ramp::linear(1.0, 0.0)),
            size: Ramp::linear(size, 0.0),
            roll_speed,
            ..sprite(origin, Vec3::ZERO, 0.2, flare, Vec3::splat(grey))
        });
    }
    if let Some(noise) = m.noise {
        let at = origin + Vec3::new(rng.float(-4.0, 4.0), 0.0, rng.float(-4.0, 4.0)) * UNIT;
        let v = Vec3::new(rng.float(-16.0, 16.0), 16.0, rng.float(-16.0, 16.0)) * UNIT;
        let alpha = rng.int(16, 32) as f32 / 255.0;
        let size = rng.int(4, 8) as f32 * UNIT;
        let roll_speed = rng.float(-2.0, 2.0);
        glow.particles.push(Particle {
            alpha,
            fade: Fade::Ramp(Ramp::linear(1.0, 0.0)),
            size: Ramp::linear(size, size * 4.0),
            roll_speed,
            ..sprite(at, v, 1.0, noise, Vec3::new(1.0, 1.0, 200.0 / 255.0))
        });
    }
    if !glow.particles.is_empty() {
        out.push(glow);
    }
    out
}

/// A uniformly random unit vector.
fn random_dir(rng: &mut ParticleRng) -> Vec3 {
    loop {
        let v = rng.vec3(-1.0, 1.0);
        let l = v.length_squared();
        if l > 1e-4 && l <= 1.0 {
            return v / l.sqrt();
        }
    }
}

fn color_between(rng: &mut ParticleRng, a: [f32; 3], b: [f32; 3]) -> Vec3 {
    Vec3::from(a).lerp(Vec3::from(b), rng.random()) / 255.0
}

/// One part of the blood system, per the spec's table.
struct BloodPart {
    count: usize,
    material: Option<usize>,
    /// Sheet sequences to pick from (inclusive), and animation rate.
    sequences: (u16, u16),
    fps: f32,
    life: (f32, f32),
    radius: (f32, f32),
    /// Start distance from the hit, in.
    distance: (f32, f32),
    speed: (f32, f32),
    local: (Vec3, Vec3),
    /// Downward, in/s² (negative: up).
    gravity: f32,
    drag: f32,
    colors: ([f32; 3], [f32; 3]),
    alpha: (f32, f32),
}

/// Sizes of the blood system's parts: smoke, mist, goop, droplets,
/// chunks (`blood_impact_red_01`, spec section 10).
pub const BLOOD_COUNTS: [usize; 5] = [7, 2, 3, 40, 15];

/// The blood burst at `p` for a hit travelling along `d`: the forward
/// axis points back at the shooter.
pub fn blood(rng: &mut ParticleRng, m: &EffectMaterials, mats: &MapParticles, p: Vec3, d: Vec3) -> Vec<ParticleGroup> {
    let f = (-d).normalize_or_zero();
    let right = f.cross(Vec3::Y).try_normalize().unwrap_or(Vec3::X);
    let up = right.cross(f);
    let left = -right;
    // Local (forward, left, up) to engine.
    let local = |v: Vec3| f * v.x + left * v.y + up * v.z;
    let parts = [
        BloodPart {
            count: BLOOD_COUNTS[0],
            material: m.blood_smoke,
            sequences: (0, 15),
            fps: 0.0,
            life: (0.7, 1.2),
            radius: (2.0, 3.0),
            distance: (0.0, 0.0),
            speed: (3.0, 15.0),
            local: (Vec3::new(10.0, -5.0, -5.0), Vec3::new(90.0, 5.0, 5.0)),
            gravity: -5.0,
            drag: 0.125,
            colors: ([205.0, 188.0, 173.0], [124.0, 106.0, 86.0]),
            alpha: (25.0, 40.0),
        },
        BloodPart {
            count: BLOOD_COUNTS[1],
            material: m.blood_core,
            sequences: (0, 0),
            fps: 0.0,
            life: (0.25, 0.35),
            radius: (9.0, 19.0),
            distance: (0.0, 0.0),
            speed: (3.0, 15.0),
            local: (Vec3::new(10.0, -5.0, -5.0), Vec3::new(30.0, 5.0, 5.0)),
            gravity: 175.0,
            drag: 0.125,
            colors: ([107.0, 16.0, 16.0], [14.0, 1.0, 1.0]),
            alpha: (80.0, 255.0),
        },
        BloodPart {
            count: BLOOD_COUNTS[2],
            material: m.goop,
            sequences: (0, 5),
            fps: 35.0,
            life: (0.0, 0.0),
            radius: (8.0, 21.0),
            distance: (0.0, 2.0),
            speed: (0.0, 32.0),
            local: (Vec3::new(26.0, 0.0, 24.0), Vec3::new(42.0, 0.0, 50.0)),
            gravity: 400.0,
            drag: 0.025,
            colors: ([93.0, 8.0, 8.0], [48.0, 9.0, 5.0]),
            alpha: (255.0, 255.0),
        },
        BloodPart {
            count: BLOOD_COUNTS[3],
            material: m.glow,
            sequences: (0, 0),
            fps: 0.0,
            life: (0.2, 0.8),
            radius: (0.35, 0.65),
            distance: (4.0, 8.0),
            speed: (75.0, 150.0),
            local: (Vec3::new(0.0, 0.0, 30.0), Vec3::new(0.0, 0.0, 90.0)),
            gravity: 500.0,
            drag: 0.015,
            colors: ([134.0, 1.0, 1.0], [27.0, 3.0, 3.0]),
            alpha: (128.0, 200.0),
        },
        BloodPart {
            count: BLOOD_COUNTS[4],
            material: m.goop,
            sequences: (8, 12),
            fps: 0.0,
            life: (0.1, 0.7),
            radius: (0.3, 2.0),
            distance: (0.0, 0.0),
            speed: (50.0, 150.0),
            local: (Vec3::new(0.0, 0.0, 20.0), Vec3::new(0.0, 0.0, 100.0)),
            gravity: 400.0,
            drag: 0.1,
            colors: ([0.0, 0.0, 0.0], [17.0, 53.0, 81.0]),
            alpha: (255.0, 255.0),
        },
    ];
    let mut out = Vec::new();
    for (i, part) in parts.iter().enumerate() {
        let Some(material) = part.material else { continue };
        let mut g = ParticleGroup::new(Motion {
            gravity: part.gravity * UNIT,
            drag: part.drag,
            ..default()
        });
        // A particle system: its own budget, not the old-style cap.
        g.capped = false;
        for _ in 0..part.count {
            let sequence = rng.int(part.sequences.0 as i32, part.sequences.1 as i32) as u16;
            let at = p + random_dir(rng) * rng.float(part.distance.0, part.distance.1) * UNIT;
            let (lo, hi) = part.local;
            let jitter = Vec3::new(rng.float(lo.x, hi.x), rng.float(lo.y, hi.y), rng.float(lo.z, hi.z));
            let v = (random_dir(rng) * rng.float(part.speed.0, part.speed.1) + local(jitter)) * UNIT;
            let color = color_between(rng, part.colors.0, part.colors.1);
            let alpha = rng.float(part.alpha.0, part.alpha.1) / 255.0;
            let (life, radius, size, fade) = match i {
                // Smoke: grows to 4x; in over the first 12.5 %, out from 25 %.
                0 => {
                    let r = rng.float(part.radius.0, part.radius.1);
                    (
                        rng.float(part.life.0, part.life.1),
                        r,
                        Ramp::linear(r, r * 4.0),
                        Fade::Windows {
                            start_alpha: 0.0,
                            fade_in: (0.0, 0.125),
                            fade_out: (0.25, 1.0),
                        },
                    )
                }
                // Mist: from nothing to 2x (bias 0.75); its fade-in has an
                // empty window, taken as done at once (spec Q7); out from 50 %.
                1 => {
                    let r = rng.float(part.radius.0, part.radius.1);
                    (
                        rng.float(part.life.0, part.life.1),
                        r,
                        Ramp::linear(0.0, r * 2.0).with_bias(0.75),
                        Fade::Windows {
                            start_alpha: 0.0,
                            fade_in: (0.0, 0.0),
                            fade_out: (0.5, 1.0),
                        },
                    )
                }
                // Goop: lives one run of its sheet sequence at 35 fps, fades
                // over a random last 25-50 %.
                2 => {
                    let frames = mats
                        .materials
                        .get(material)
                        .and_then(|m| m.sequences.get(sequence as usize))
                        .map_or(12, |s| s.len().max(1));
                    let r = rng.float(part.radius.0, part.radius.1);
                    let tail = rng.float(0.25, 0.5);
                    (
                        frames as f32 / part.fps,
                        r,
                        Ramp::constant(r),
                        Fade::Windows {
                            start_alpha: 1.0,
                            fade_in: (0.0, 0.0),
                            fade_out: (1.0 - tail, 1.0),
                        },
                    )
                }
                // Droplets: mostly near the largest radius (exponent 0.1),
                // shrinking to 0.75 from half their life.
                3 => {
                    let r = 0.35 + 0.3 * rng.random().powf(0.1);
                    (
                        rng.float(part.life.0, part.life.1),
                        r,
                        Ramp::linear(r, r * 0.75).between(0.5, 1.0),
                        Fade::Windows {
                            start_alpha: 1.0,
                            fade_in: (0.0, 0.0),
                            fade_out: (0.85, 1.0),
                        },
                    )
                }
                _ => {
                    let r = rng.float(part.radius.0, part.radius.1);
                    (
                        rng.float(part.life.0, part.life.1),
                        r,
                        Ramp::constant(r),
                        Fade::Windows {
                            start_alpha: 1.0,
                            fade_in: (0.0, 0.0),
                            fade_out: (0.5, 1.0),
                        },
                    )
                }
            };
            let _ = radius;
            let roll = rng.float(0.0, std::f32::consts::TAU);
            let roll_speed = if i == 2 { rng.float(0.0, 4.0).to_radians() } else { 0.0 };
            g.particles.push(Particle {
                sequence,
                fps: part.fps,
                alpha,
                fade,
                size: Ramp {
                    start: size.start * UNIT,
                    end: size.end * UNIT,
                    ..size
                },
                roll,
                roll_speed,
                ..sprite(at, v, life, material, color)
            });
        }
        out.push(g);
    }
    out
}

/// Blood-on-wall traces behind a hit: (count, direction noise) by damage
/// in hit points.
pub fn bleed_traces(damage: f32) -> (usize, f32) {
    if damage < 10.0 {
        (1, 0.1)
    } else if damage < 25.0 {
        (2, 0.2)
    } else {
        (4, 0.3)
    }
}

/// What a shot from `from` to `to` did with water.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum WaterShot {
    /// The end point is dry.
    Dry,
    /// Entered water (or slime) at this point.
    Entered { at: Vec3, slime: bool },
    /// Wholly under water: nothing at all.
    Under,
}

fn inside(v: &MapWaterVolume, p: Vec3, margin: f32) -> bool {
    v.brush.planes.iter().all(|(n, d)| n.dot(p) - d <= margin)
}

/// Where a shot whose end point is in water entered it (spec section 11).
pub fn water_shot(from: Vec3, to: Vec3, water: &[MapWaterVolume]) -> WaterShot {
    let mut best: Option<(f32, Vec3, bool)> = None;
    for v in water.iter().filter(|v| inside(v, to, 0.01)) {
        if inside(v, from, 0.0) {
            return WaterShot::Under;
        }
        // Clip the segment to the volume: the entry is the latest plane
        // entered.
        let d = to - from;
        let mut t0: f32 = 0.0;
        for (n, dist) in &v.brush.planes {
            let (dn, s) = (n.dot(d), n.dot(from) - dist);
            if dn < 0.0 && s > 0.0 {
                t0 = t0.max(-s / dn);
            }
        }
        if best.is_none_or(|b| t0 < b.0) {
            best = Some((t0, from + d * t0, v.slime));
        }
    }
    match best {
        Some((_, at, slime)) => WaterShot::Entered { at, slime },
        None => WaterShot::Dry,
    }
}

/// Where a knife swing's line (eye to where it stopped) entered water, if
/// it started dry and ended in water (not slime: its splash is a
/// particle system not done yet). Spec section 12.
pub fn knife_splash(line: (Vec3, Vec3), water: Option<&MapWater>) -> Option<Vec3> {
    match water_shot(line.0, line.1, &water?.0) {
        WaterShot::Entered { at, slime: false } => Some(at),
        _ => None,
    }
}

/// Splash scale factor `f = min(s/8, 4)`.
pub fn splash_factor(s: f32) -> f32 {
    (s / 8.0).min(4.0)
}

/// Splash luminosity and colour from the light `l` (0-1 linear) at the
/// splash: λ = clamp(4·luma, 0.25, 1), c = 0.25·l/max(l) + 0.75.
pub fn splash_lighting(l: Vec3) -> (f32, Vec3) {
    let max = l.max_element();
    let tint = if max > 0.0 { l / max } else { Vec3::ZERO };
    let lum = (4.0 * (0.30 * l.x + 0.59 * l.y + 0.11 * l.z)).clamp(0.25, 1.0);
    (lum, tint * 0.25 + Vec3::splat(0.75))
}

/// Gout `i` (0..8) of a splash at factor `f` and luminosity λ: speed,
/// start and end size, start alpha (0-255).
pub fn gout(i: usize, f: f32, lum: f32) -> (f32, f32, f32, f32) {
    let k = i as f32 / 7.0;
    let speed = 50.0 * f * (8 - i) as f32;
    let start = (24.0 * f * (0.5 + 0.5 * k)) as i32 as f32;
    let alpha = ((32.0 + (255.0 - 32.0) * k) * lum) as i32 as f32;
    (speed, start, (2.0 * start).min(255.0), alpha)
}

/// A bullet splash (spec section 11) at water entry `o` with scale `s`
/// and light `l` at the splash.
pub fn water_splash(rng: &mut ParticleRng, m: &EffectMaterials, o: Vec3, s: f32, l: Vec3) -> Vec<ParticleGroup> {
    let up = Vec3::Y;
    let f = splash_factor(s);
    let (lum, c) = splash_lighting(l);
    let mut out = Vec::new();
    let Some(splash) = m.splash else { return out };
    let mut drops = ParticleGroup::new(Motion {
        gravity: 800.0 * UNIT,
        damping: 2.0,
        ..default()
    });
    for _ in 0..16 {
        let at = o + Vec3::new(rng.float(-8.0, 8.0) * f, 0.0, rng.float(-8.0, 8.0) * f) * UNIT;
        let life = rng.float(0.25, 0.5);
        let mut v = (up + rng.vec3(-0.8, 0.8)) * rng.float(150.0 * f, 300.0 * f);
        v.y += rng.float(32.0, 64.0) * f;
        let width = rng.float(1.0, 3.0) * UNIT;
        let length = rng.float(0.025, 0.05);
        let color = ramp3(c, rng.float(0.75, 1.25));
        drops.particles.push(Particle {
            alpha: lum,
            shape: Shape::Trail { width, length },
            ..sprite(at, v * UNIT, life, splash, color)
        });
    }
    out.push(drops);
    let mut gouts = ParticleGroup::new(Motion {
        decay: Some((3.0, 0.0)),
        gravity: 800.0 * UNIT,
        roll_decay: Some((4.0, 0.5)),
        sink: Some(o.y),
        ..pool_motion()
    });
    for i in 0..8 {
        let (speed, start, end, alpha) = gout(i, f, lum);
        let v = (rng.vec3(-0.2, 0.2) + up * rng.float(4.0, 6.0)).normalize_or_zero() * speed * UNIT;
        let color = ramp3(c, rng.float(0.75, 1.25));
        let roll = rng.int(0, 360) as f32;
        let roll_speed = rng.float(-4.0, 4.0);
        gouts.particles.push(Particle {
            alpha: alpha / 255.0,
            size: Ramp::linear(start * UNIT, end * UNIT),
            roll,
            roll_speed,
            ..sprite(o, v, 2.0, splash, color)
        });
    }
    out.push(gouts);
    if let Some(wake) = m.wake {
        let mut ripple = ParticleGroup::new(Motion::default());
        ripple.skip_first = false;
        let yaw = rng.float(0.0, 360.0);
        let yaw_speed = rng.float(-16.0, 16.0);
        ripple.particles.push(Particle {
            fade: Fade::Ramp(Ramp::linear(lum, 0.0).with_bias(0.25)),
            size: Ramp::linear(16.0 * f * UNIT, 128.0 * f * UNIT).with_bias(0.7),
            shape: Shape::Flat {
                normal: up,
                yaw,
                yaw_speed,
            },
            ..sprite(o + up * 0.5 * UNIT, Vec3::ZERO, 1.5, wake, c)
        });
        out.push(ripple);
    }
    out
}

/// Shards of a shattered window pane (spec section 9's glass shards,
/// spread over the pane): flat translucent squares from the pane's area,
/// thrown along its push and normal, falling, bouncing (keep 0.3) and
/// fading over their last 2 s. `light` is the light at the pane (0-1).
/// Sizes and counts per pane are ours (the pane burst isn't specced).
pub fn glass_shards(
    rng: &mut ParticleRng,
    m: &EffectMaterials,
    pane: &crate::map::GlassShatter,
    light: Vec3,
    world: &impl WorldTrace,
) -> Option<ParticleGroup> {
    let mats = m.fleck_glass;
    if mats.iter().all(Option::is_none) {
        return None;
    }
    let n = pane.normal.normalize_or_zero();
    let along = pane.velocity.normalize_or_zero();
    // The pane's in-plane axes.
    let right = n.cross(Vec3::Y).try_normalize().unwrap_or(Vec3::X);
    let up = right.cross(n).normalize_or_zero();
    // One shard per shard-size square of the burst's area (the large
    // bursts of a blast cover a whole window).
    let shard = (pane.shard / UNIT).max(1.0);
    let area = pane.size.x * pane.size.y / (UNIT * UNIT);
    let count = ((area / (shard * shard)) as i32).clamp(2, 48) + rng.int(0, 2);
    let lit = light.clamp(Vec3::ZERO, Vec3::ONE) * 0.7 + Vec3::splat(0.3);
    let color = Vec3::new(200.0, 200.0, 210.0) / 255.0 * lit;
    let planes = probe_planes(
        pane.at,
        Some(if along == Vec3::ZERO { n } else { along }),
        150.0 * UNIT,
        800.0 * UNIT,
        world,
    );
    let mut g = ParticleGroup::new(Motion {
        gravity: 800.0 * UNIT,
        bounce: Some(Bounce {
            keep: 0.3,
            land_normal_y: 0.5,
            land_speed: LAND_SPEED * UNIT,
            planes,
        }),
        ..default()
    });
    let sigma = shard * rng.float(0.5, 1.5);
    for _ in 0..count {
        let Some(mat) = mats[rng.int(0, 1) as usize].or(mats[0]).or(mats[1]) else {
            continue;
        };
        let at = pane.at + right * rng.float(-0.5, 0.5) * pane.size.x + up * rng.float(-0.5, 0.5) * pane.size.y;
        let side = (sigma + rng.float(-sigma / 2.0, sigma / 2.0)).trunc().max(1.0);
        let push = if along == Vec3::ZERO {
            n * rng.float(-1.0, 1.0)
        } else {
            along
        };
        let v = (push * rng.float(20.0, 120.0) + rng.vec3(-30.0, 30.0)) * UNIT;
        let normal = (n + rng.vec3(-0.8, 0.8)).normalize_or(n);
        g.particles.push(Particle {
            fade: Fade::Tail(2.0),
            size: Ramp::constant(side * UNIT),
            shape: Shape::Flat {
                normal,
                yaw: rng.float(0.0, 360.0),
                yaw_speed: rng.float(-800.0, 800.0),
            },
            ..sprite(at, v, rng.float(2.5, 5.0), mat, color)
        });
    }
    Some(g)
}

/// The glass impact (spec section 9): where a bullet or club hit
/// shattered the pane it hit, at hit point `p` with the trace normal `n`;
/// `light` is the light there (0-1). 2-4 flat shards spinning out along
/// the normal, bouncing (keep 0.3) and fading over their last 2 s; 4 white
/// grit sprites and a white smoke cap.
pub fn glass_impact(
    rng: &mut ParticleRng,
    m: &EffectMaterials,
    p: Vec3,
    n: Vec3,
    light: Vec3,
    world: &impl WorldTrace,
) -> Vec<ParticleGroup> {
    let mut out = Vec::new();
    let n = n.normalize_or(Vec3::Y);
    let lit = light.clamp(Vec3::ZERO, Vec3::ONE) * 0.7 + Vec3::splat(0.3);
    let color = (Vec3::new(200.0, 200.0, 210.0) * lit).trunc() / 255.0;
    let mats = m.fleck_glass;
    if mats.iter().any(Option::is_some) {
        let planes = probe_planes(p, Some(n), (1.0 + 300.0) / 2.0 * UNIT, 800.0 * UNIT, world);
        let mut g = ParticleGroup::new(Motion {
            gravity: 800.0 * UNIT,
            bounce: Some(Bounce {
                keep: 0.3,
                land_normal_y: 0.5,
                land_speed: LAND_SPEED * UNIT,
                planes,
            }),
            ..default()
        });
        let count = rng.int(2, 4);
        let sigma = rng.float(2.0, 6.0);
        for _ in 0..count {
            let pick = rng.int(0, 1) as usize;
            let Some(mat) = mats[pick].or(mats[1 - pick]) else { continue };
            let side = (sigma + rng.float(-sigma / 2.0, sigma / 2.0)).trunc().max(1.0);
            let life = rng.float(2.5, 5.0);
            // Each axis: (n + U(-0.8, 0.8)) times its own speed draw.
            let mut v = Vec3::ZERO;
            for i in 0..3 {
                v[i] = (n[i] + rng.float(-0.8, 0.8)) * rng.float(1.0, 300.0);
            }
            let facing = random_dir(rng);
            let yaw = rng.float(0.0, 360.0);
            let spin = rng.float(-800.0, 800.0);
            g.particles.push(Particle {
                fade: Fade::Tail(2.0),
                size: Ramp::constant(side * UNIT),
                shape: Shape::Flat {
                    normal: facing,
                    yaw,
                    yaw_speed: spin,
                },
                ..sprite(p, v * UNIT, life, mat, color)
            });
        }
        out.push(g);
    }
    // Grit and cap: the colour ramp clamps to white (quirk).
    let mut pool = ParticleGroup::new(pool_motion());
    pool.skip_first = false;
    let at = p + n * 2.0 * UNIT;
    if let Some(grit) = m.grit {
        for i in 0..4 {
            let k = (i + 1) as f32;
            let life = rng.float(0.1, 0.25);
            let dir = n + rng.vec3(-0.8, 0.8);
            let start = rng.int(1, 4) as f32;
            let v = down(dir * rng.float(8.0, 16.0) * k * UNIT, rng.float(16.0, 32.0) * k);
            let alpha = rng.int(128, 255) as f32 / 255.0;
            let roll_speed = rng.float(-1.0, 1.0);
            pool.particles.push(Particle {
                alpha,
                fade: Fade::Ramp(Ramp::linear(1.0, 0.0)),
                size: Ramp::linear(start * UNIT, start * 8.0 * UNIT),
                roll_speed,
                ..sprite(at, v, life, grit, Vec3::ONE)
            });
        }
    }
    if let Some(smoke) = m.smoke {
        let life = rng.float(1.0, 1.5);
        let dir = n + rng.vec3(-0.8, 0.8);
        let start = rng.int(4, 8) as f32;
        let mut v = dir * rng.float(2.0, 8.0) * UNIT;
        v.y = rng.float(-2.0, 2.0) * UNIT;
        let alpha = rng.int(32, 64) as f32 / 255.0;
        let roll_speed = rng.float(-2.0, 2.0);
        pool.particles.push(Particle {
            alpha,
            fade: Fade::Ramp(Ramp::linear(1.0, 0.0)),
            size: Ramp::linear(start * UNIT, start * 4.0 * UNIT),
            roll_speed,
            ..sprite(at, v, life, smoke, Vec3::ONE)
        });
    }
    if !pool.particles.is_empty() {
        out.push(pool);
    }
    out
}

/// Shards where window panes shatter, and the glass impact where a hit
/// shattered one.
#[allow(clippy::too_many_arguments)]
fn glass_effects(
    mut panes: MessageReader<crate::map::GlassShatter>,
    mut hits: MessageReader<crate::map::GlassImpact>,
    materials: Option<Res<ParticleMaterials>>,
    colors: Option<Res<SurfaceColors>>,
    mut particles: ResMut<Particles>,
    mut rng: ResMut<EffectRng>,
    world: WorldTracer,
) {
    let Some(materials) = materials else {
        panes.clear();
        hits.clear();
        return;
    };
    let mats = EffectMaterials::new(&materials.0);
    for hit in hits.read() {
        let light = colors.as_deref().map_or(Vec3::ONE, |c| light_below(c, &world, hit.at));
        for g in glass_impact(&mut rng.0, &mats, hit.at, hit.normal, light, &world) {
            particles.add(g);
        }
    }
    for pane in panes.read() {
        if pane.tile {
            continue;
        }
        let light = colors.as_deref().map_or(Vec3::ONE, |c| light_below(c, &world, pane.at));
        if let Some(g) = glass_shards(&mut rng.0, &mats, pane, light, &world) {
            particles.add(g);
        }
    }
}

/// Bullets per tracer: one global count over every bullet (spec
/// tracers.md 1; CS:S's N is open question 1, 4 is the shared default).
pub const TRACER_EVERY: u64 = 4;

/// The tracer's look (spec tracers.md constants), in meters.
pub const TRACER_LOOK: crate::map::tracer::TracerLook = crate::map::tracer::TracerLook {
    material: "effects/spark",
    speed: 5000.0 * super::bsp::METERS_PER_UNIT,
    length: (64.0 * super::bsp::METERS_PER_UNIT, 128.0 * super::bsp::METERS_PER_UNIT),
    half_width: (0.75 * super::bsp::METERS_PER_UNIT, 0.9 * super::bsp::METERS_PER_UNIT),
    min_distance: 256.0 * super::bsp::METERS_PER_UNIT,
};

/// Whether the bullet numbered `count` (from 0) draws a tracer.
pub fn draws_tracer(count: u64) -> bool {
    count.is_multiple_of(TRACER_EVERY)
}

/// One tracer every `TRACER_EVERY` bullets, from the gun to where the
/// bullet stopped.
fn tracers(
    mut events: MessageReader<WeaponEvent>,
    mut out: MessageWriter<crate::map::tracer::Tracer>,
    mut count: Local<u64>,
) {
    for e in events.read() {
        let WeaponEventKind::Shot { from, to, .. } = e.kind else { continue };
        if draws_tracer(*count) {
            out.write(crate::map::tracer::Tracer {
                owner: e.owner,
                from,
                to,
                look: TRACER_LOOK,
            });
        }
        *count += 1;
    }
}

pub struct ImpactEffectsPlugin;

impl Plugin for ImpactEffectsPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Particles>()
            .init_resource::<ImpactEffectSettings>()
            .init_resource::<EffectRng>()
            .add_message::<WeaponEvent>()
            .add_message::<PlaceDecal>()
            .add_message::<PlaySound>()
            .add_message::<crate::map::GlassShatter>()
            .add_message::<crate::map::GlassImpact>()
            .add_message::<crate::map::tracer::Tracer>()
            .add_systems(
                FixedUpdate,
                (impact_effects, blood_effects, tracers).after(crate::core::SimSet::Weapons),
            )
            .add_systems(Update, glass_effects);
        {
            let mut p = app.world_mut().resource_mut::<Particles>();
            p.cap = MAX_PARTICLES;
            p.max_dt = MAX_DT;
        }
        let cvars: [(&str, &str, fn(&mut ImpactEffectSettings) -> &mut i32); 3] = [
            (
                "r_drawflecks",
                "Debris flecks where bullets hit concrete, tile and wood (0/1)",
                |s| &mut s.r_drawflecks,
            ),
            ("cl_show_splashes", "Splashes where bullets enter water (0/1)", |s| {
                &mut s.cl_show_splashes
            }),
            ("violence_hblood", "Blood from hit players (0/1)", |s| {
                &mut s.violence_hblood
            }),
        ];
        for (name, help, field) in cvars {
            crate::console::resource_cvar(app, name, help, field);
        }
        crate::console::ConsoleAppExt::console_command(
            app,
            "mashup_particles",
            "Live particles: groups, total, and those under the 2048 cap",
            |w, _| {
                let p = w
                    .get_resource::<Particles>()
                    .ok_or_else(|| "no particles".to_string())?;
                Ok(Some(format!(
                    "{} groups, {} particles, {} of {} capped",
                    p.groups.len(),
                    p.count(),
                    p.capped_count(),
                    p.cap
                )))
            },
        );
    }
}

#[derive(Resource)]
struct EffectRng(ParticleRng);

impl Default for EffectRng {
    fn default() -> Self {
        Self(ParticleRng::new(0x1_4AC7))
    }
}

/// The surface look at a hit: the prop's, or the world's (None for a world
/// hit with no visible surface: sky, nodraw, clips).
fn look_at(colors: &SurfaceColors, prop: Option<usize>, point: Vec3, normal: Vec3) -> Result<Option<SurfaceLook>, ()> {
    match prop {
        Some(i) => Ok(colors.prop(i, normal)),
        None => colors.world(point, normal).map(Some).ok_or(()),
    }
}

/// The light at a point for splashes: the lightmap of the surface below.
fn light_below(colors: &SurfaceColors, world: &impl WorldTrace, p: Vec3) -> Vec3 {
    let to = p - Vec3::Y * 1000.0 * UNIT;
    world
        .trace(p, to)
        .and_then(|(f, n)| colors.world(p.lerp(to, f), n))
        .map_or(Vec3::ONE, |l| l.light)
}

#[allow(clippy::too_many_arguments)]
fn impact_effects(
    mut events: MessageReader<WeaponEvent>,
    bank: Option<Res<SoundBank>>,
    grid: Option<Res<SurfaceGrid>>,
    surfaces: Query<&PropSurface>,
    props: Query<&PropIndex>,
    characters: Query<(), With<Intent>>,
    colors: Option<Res<SurfaceColors>>,
    water: Option<Res<MapWater>>,
    materials: Option<Res<ParticleMaterials>>,
    settings: Res<ImpactEffectSettings>,
    mut particles: ResMut<Particles>,
    mut rng: ResMut<EffectRng>,
    world: WorldTracer,
    mut play: MessageWriter<PlaySound>,
) {
    // Without materials (headless) there are no particles; sounds still
    // play.
    let mats = materials.map(|m| EffectMaterials::new(&m.0));
    let mats = mats.as_ref();
    let rng = &mut rng.0;
    for e in events.read() {
        let (from, point, normal, hit, bullet) = match &e.kind {
            // Also where a bullet enters the next surface after passing
            // something (penetration).
            WeaponEventKind::Shot {
                from,
                to,
                hit: Some(hit),
                normal: Some(normal),
            }
            | WeaponEventKind::ShotContinued {
                from,
                to,
                hit: Some(hit),
                normal: Some(normal),
            } => (*from, *to, *normal, *hit, true),
            WeaponEventKind::Shot { from, to, .. } | WeaponEventKind::ShotContinued { from, to, .. } => {
                // Misses can still end in water.
                if let Some(w) = &water
                    && let WaterShot::Entered { at, slime: false } = water_shot(*from, *to, &w.0)
                    && settings.cl_show_splashes != 0
                {
                    splash(rng, mats, at, None, colors.as_deref(), &world, &mut particles, &mut play);
                }
                continue;
            }
            WeaponEventKind::Swing { at, line, .. } => {
                // A swing from the dry into water splashes (fixed scale
                // 8) and does nothing else, hit or miss (spec section 12).
                if let Some(at) = knife_splash(*line, water.as_deref()) {
                    if settings.cl_show_splashes != 0 {
                        splash(rng, mats, at, Some(8.0), colors.as_deref(), &world, &mut particles, &mut play);
                    }
                    continue;
                }
                let Some((point, normal, hit)) = at else { continue };
                (*point + *normal * 0.1, *point, *normal, *hit, false)
            }
            _ => continue,
        };
        let d = (point - from).normalize_or_zero();
        if bullet && let Some(w) = &water {
            match water_shot(from, point, &w.0) {
                WaterShot::Dry => {}
                WaterShot::Entered { at, slime } => {
                    // Slime splashes are particle systems not done yet.
                    if !slime && settings.cl_show_splashes != 0 {
                        splash(rng, mats, at, None, colors.as_deref(), &world, &mut particles, &mut play);
                    }
                    continue;
                }
                WaterShot::Under => continue,
            }
        }
        // Players: blood comes from the damage (blood_effects).
        if characters.contains(hit) {
            continue;
        }
        let name = surface_of(Some(hit), point, &surfaces, &characters, grid.as_deref());
        let letter = bank
            .as_ref()
            .and_then(|b| surface(&b.0, &name))
            .map_or('C', |s| s.game_material);
        let effect = effect_for(letter);
        let prop = props.get(hit).ok().map(|p| p.0);
        let look = match colors.as_deref().map(|c| look_at(c, prop, point, normal)) {
            Some(Ok(look)) => look,
            // No visible surface at a world hit: sky or nodraw, nothing.
            Some(Err(())) => continue,
            None => None,
        };
        // Bullets only: 3 in 10 impacts ricochet (spec section 2 step 6).
        if bullet && rng.random() < RICOCHET_CHANCE {
            play.write(PlaySound::at(RICOCHET, point));
        }
        let Some(mats) = mats else { continue };
        let c = surface_color(look);
        let groups = match effect {
            Effect::Debris { wood } => debris(rng, mats, point, normal, wood, c, settings.r_drawflecks != 0, &world),
            Effect::DustPuff => vec![dust_puff(rng, mats, point, normal, c)],
            Effect::MetalSparks => metal_sparks(rng, mats, point, normal, d),
            Effect::ElectricSparks => electric_sparks(rng, mats, point, normal, &world),
            Effect::None => Vec::new(),
        };
        for g in groups {
            particles.add(g);
        }
    }
}

/// A water splash at `at` of scale `size` (None: a bullet's, U(8, 12)).
#[allow(clippy::too_many_arguments)]
fn splash(
    rng: &mut ParticleRng,
    mats: Option<&EffectMaterials>,
    at: Vec3,
    size: Option<f32>,
    colors: Option<&SurfaceColors>,
    world: &impl WorldTrace,
    particles: &mut Particles,
    play: &mut MessageWriter<PlaySound>,
) {
    let s = size.unwrap_or_else(|| rng.float(8.0, 12.0));
    if let Some(mats) = mats {
        let light = colors.map_or(Vec3::ONE, |c| light_below(c, world, at + Vec3::Y * s * UNIT));
        for g in water_splash(rng, mats, at, s, light.min(Vec3::ONE)) {
            particles.add(g);
        }
    }
    play.write(PlaySound::at("Physics.WaterSplash", at));
}

/// Blood where players take weapon damage (not refused by team rules),
/// and stains on the walls behind.
#[allow(clippy::too_many_arguments)]
fn blood_effects(
    mut damage: MessageReader<Damage>,
    characters: Query<(), With<Intent>>,
    teams: Query<&crate::core::Team>,
    friendly_fire: Option<Res<crate::core::FriendlyFire>>,
    materials: Option<Res<ParticleMaterials>>,
    settings: Res<ImpactEffectSettings>,
    bank: Option<Res<SoundBank>>,
    grid: Option<Res<SurfaceGrid>>,
    mut particles: ResMut<Particles>,
    mut rng: ResMut<EffectRng>,
    world: WorldTracer,
    mut decals: MessageWriter<PlaceDecal>,
) {
    let rng = &mut rng.0;
    for hit in damage.read() {
        // Only damage that is taken: not a teammate's with friendly fire
        // off (spec section 10).
        if hit.attacker.is_none()
            || !characters.contains(hit.target)
            || settings.violence_hblood == 0
            || crate::core::refused_by_team(hit, &teams, friendly_fire.as_deref())
        {
            continue;
        }
        if let Some(m) = &materials {
            let mats = EffectMaterials::new(&m.0);
            for g in blood(rng, &mats, &m.0, hit.point, hit.dir) {
                particles.add(g);
            }
        }
        let (count, noise) = bleed_traces(hit.amount / super::weapons::HP);
        for _ in 0..count {
            let dir = (hit.dir + rng.vec3(-noise, noise)).normalize_or_zero();
            let to = hit.point + dir * BLEED_TRACE * UNIT;
            let Some((f, n)) = world.trace(hit.point, to) else {
                continue;
            };
            let at = hit.point.lerp(to, f);
            // Not on grates.
            let grate = bank
                .as_ref()
                .zip(grid.as_ref())
                .and_then(|(b, g)| surface(&b.0, g.nearest(at, 0.3).unwrap_or("default")))
                .is_some_and(|s| s.game_material == 'G');
            if grate || f <= 0.0 {
                continue;
            }
            decals.write(PlaceDecal {
                target: None,
                group: DecalGroup::Named("Blood".into()),
                point: at,
                normal: n,
                dir,
                spin: true,
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_fourth_bullet_draws_a_tracer() {
        let drawn: Vec<u64> = (0..10).filter(|n| draws_tracer(*n)).collect();
        assert_eq!(drawn, [0, 4, 8]);
        // 256 in minimum, 5000 in/s.
        assert!((TRACER_LOOK.min_distance - 6.5024).abs() < 1e-4);
        assert!((TRACER_LOOK.speed - 127.0).abs() < 1e-3);
    }
    use crate::map::particles::{NoWorld, step_particle};

    fn approx(a: f32, b: f32, eps: f32) -> bool {
        (a - b).abs() <= eps
    }

    #[test]
    fn letters_pick_effects() {
        assert_eq!(effect_for('C'), Effect::Debris { wood: false });
        assert_eq!(effect_for('T'), Effect::Debris { wood: false });
        assert_eq!(effect_for('W'), Effect::Debris { wood: true });
        assert_eq!(effect_for('D'), Effect::DustPuff);
        assert_eq!(effect_for('N'), Effect::DustPuff);
        assert_eq!(effect_for('M'), Effect::MetalSparks);
        assert_eq!(effect_for('V'), Effect::MetalSparks);
        assert_eq!(effect_for('P'), Effect::ElectricSparks);
        for none in ['G', 'Y', 'L', 'O', 'F', 'B', 'H', 'S', 'X', 'I', '-'] {
            assert_eq!(effect_for(none), Effect::None, "{none}");
        }
    }

    #[test]
    fn fleck_numbers() {
        assert_eq!(fleck_count(1.0, 1.0, 4), 4);
        assert_eq!(fleck_count(0.5, 1.0, 9), 5);
        assert!(approx(fleck_spray(1.0), 0.4, 1e-6));
        assert!(approx(fleck_spray(2.0), 0.2, 1e-6));
        assert_eq!(fleck_spray(3.0), 0.2);
        let n = Vec3::new(0.0, 0.0, 1.0);
        assert_eq!(fleck_velocity(n, Vec3::ZERO, 64.0, 1), Vec3::new(0.0, 0.0, 128.0));
        assert_eq!(fleck_velocity(n, Vec3::ZERO, 64.0, 2), Vec3::new(0.0, 0.0, 64.0));
    }

    #[test]
    fn fleck_alpha_cement_fades_wood_does_not() {
        // Cement: vertex alpha 1 - age/life. Wood flecks' material has no
        // vertex alpha, so the draw ignores it (ParticleMaterial).
        let mut p = Particle::new(Vec3::ZERO, FLECK_LIFE, 0, 1.0);
        p.fade = Fade::Ramp(Ramp::linear(1.0, 0.0));
        p.age = 1.5;
        assert!(approx(p.current_alpha(), 0.5, 1e-6));
    }

    #[test]
    fn surface_colour_and_clamp_ramp() {
        let c = surface_color(Some(SurfaceLook {
            light: Vec3::splat(0.5),
            base: Vec3::new(0.6, 0.5, 0.4),
        }));
        assert!(c.abs_diff_eq(Vec3::new(0.4378, 0.3649, 0.2919), 1e-4), "{c}");
        assert_eq!(clamp_ramp(0.9, 1.25), 1.0);
        // Unresolved: white whatever the ramp.
        let white = surface_color(None);
        assert_eq!(ramp3(white, 0.5), Vec3::ONE);
    }

    #[test]
    fn dust_numbers() {
        assert_eq!(dust_counts(1.0), (4, 4, 4));
        assert_eq!(dust_counts(0.5), (2, 2, 2));
        assert_eq!(dust_counts(0.25), (1, 1, 1));
        assert_eq!(dust_counts(0.1), (0, 1, 0));
        assert_eq!(big_puff_size(1.0, 3, 3), (12.0, 48.0));
        // Dust alpha ignores the start alpha.
        for (age, a) in [(0.2, 0.8), (0.25, 0.75), (0.26, 0.5476), (0.5, 0.25)] {
            assert!(approx(Fade::Dust.at(age, 1.0), a, 1e-4), "{age}");
        }
    }

    fn dust_motion() -> Motion {
        Motion {
            decay: Some((DUST_DECAY_TIME, DUST_MIN_SPEED)),
            ..default()
        }
    }

    #[test]
    fn dust_decay_and_floor() {
        let mut rng = ParticleRng::new(1);
        let m = dust_motion();
        for (dt, k) in [(0.01, 0.83176), (0.1, 0.15849)] {
            let mut p = Particle::new(Vec3::ZERO, 10.0, 0, 1.0);
            p.velocity = Vec3::X * 1000.0;
            step_particle(&mut p, &m, dt, &NoWorld, &mut rng);
            assert!(approx(p.velocity.x, 1000.0 * k, 0.05), "{dt}: {}", p.velocity.x);
        }
        let mut p = Particle::new(Vec3::ZERO, 10.0, 0, 1.0);
        p.velocity = Vec3::X * 40.0;
        step_particle(&mut p, &m, 0.01, &NoWorld, &mut rng);
        assert!(approx(p.velocity.x, 33.27, 0.01), "{}", p.velocity.x);
        step_particle(&mut p, &m, 0.01, &NoWorld, &mut rng);
        assert!(approx(p.velocity.x, 32.0, 1e-4), "{}", p.velocity.x);
        // A puff with no speed never moves.
        let mut still = Particle::new(Vec3::ZERO, 10.0, 0, 1.0);
        step_particle(&mut still, &m, 0.05, &NoWorld, &mut rng);
        assert_eq!(still.position, Vec3::ZERO);
    }

    #[test]
    fn dust_puff_j0_stays_put() {
        let mut rng = ParticleRng::new(7);
        let g = dust_puff(
            &mut rng,
            &EffectMaterials::all(0),
            Vec3::ZERO,
            Vec3::Y,
            Vec3::splat(0.5),
        );
        assert_eq!(g.particles.len(), 12);
        // The 4th big puff (k = 3) has j = 0: no velocity.
        let big: Vec<_> = g.particles.iter().step_by(2).take(4).collect();
        assert_eq!(big[3].velocity, Vec3::ZERO);
        assert!(big[0].velocity.length() > 0.0);
        // Every sprite starts at the hit point (even the hit puffs).
        assert!(g.particles.iter().all(|p| p.position == Vec3::ZERO));
    }

    #[test]
    fn metal_spark_numbers() {
        assert_eq!(metal_spark_count(1.0, 4, 1.0), 8);
        assert_eq!(metal_spark_count(1.0, 8, 1.0), 16);
        assert_eq!(metal_spark_speed(0.0), (256.0, 1024.0));
        assert_eq!(metal_spark_speed(1.0), (128.0, 512.0));
        assert_eq!(metal_spark_speed(2.0), (0.0, 0.0));
        let mut rng = ParticleRng::new(1);
        let m = Motion {
            damping: SPARK_DAMPING,
            ..default()
        };
        for (dt, k) in [(0.01, 0.92), (0.1, 0.2), (0.125, 0.0)] {
            let mut p = Particle::new(Vec3::ZERO, 10.0, 0, 1.0);
            p.velocity = Vec3::X;
            step_particle(&mut p, &m, dt, &NoWorld, &mut rng);
            assert!(approx(p.velocity.x, k, 1e-5), "{dt}");
        }
        // The glow: 26 across, half after 0.05 s.
        let mut glow = Particle::new(Vec3::ZERO, 0.1, 0, 0.0);
        glow.size = Ramp::linear(26.0, 0.0);
        glow.fade = Fade::Ramp(Ramp::linear(1.0, 0.0));
        glow.age = 0.05;
        assert!(approx(glow.current_size(), 13.0, 1e-4));
        assert!(approx(glow.current_alpha(), 0.5, 1e-5));
    }

    #[test]
    fn effect_counts_and_ranges() {
        let mats = EffectMaterials::all(0);
        let mut rng = ParticleRng::new(99);
        for _ in 0..200 {
            let groups = debris(
                &mut rng,
                &mats,
                Vec3::ZERO,
                Vec3::Y,
                false,
                Vec3::splat(0.5),
                true,
                &NoWorld,
            );
            let flecks = &groups[0].particles;
            assert!((4..=16).contains(&flecks.len()), "{}", flecks.len());
            for f in flecks {
                assert_eq!(f.life, FLECK_LIFE);
                let half = f.size.start / UNIT;
                assert!(half == 1.0 || half == 2.0);
                // Speed: |n + cube| × U(64, 128) × (3 − size), cube per axis ±0.4.
                let speed = f.velocity.length() / UNIT;
                let lo = 0.6 * 64.0 * (3.0 - half);
                let hi = (0.4f32 * 0.4 * 2.0 + 1.4 * 1.4).sqrt() * 128.0 * (3.0 - half);
                assert!(speed >= lo - 1e-3 && speed <= hi + 1e-3, "{speed}");
            }
            assert_eq!(groups[1].particles.len(), 7, "2 trail + 4 grit + 1 cap");
            assert!(!groups[1].skip_first);
            let sparks = metal_sparks(&mut rng, &mats, Vec3::ZERO, Vec3::Y, Vec3::NEG_Y);
            assert!((8..=16).contains(&sparks[0].particles.len()));
            assert!(sparks[0].particles.iter().all(|p| (0.05..=0.1).contains(&p.life)));
            assert_eq!(sparks[1].particles.len(), 1);
            let electric = electric_sparks(&mut rng, &mats, Vec3::ZERO, Vec3::Y, &NoWorld);
            assert!((2..=3).contains(&electric[0].particles.len()));
            assert!((16..=32).contains(&electric[1].particles.len()));
            assert_eq!(electric[2].kill_after, Some(0.2));
        }
        // r_drawflecks 0: no flecks, the 2 + 4 + 1 dust sprites remain.
        let groups = debris(&mut rng, &mats, Vec3::ZERO, Vec3::Y, false, Vec3::ONE, false, &NoWorld);
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].particles.len(), 7);
    }

    #[test]
    fn fleck_bounces() {
        use crate::map::particles::bounce;
        let b = Bounce {
            keep: 0.3,
            land_normal_y: 0.5,
            land_speed: 48.0,
            planes: vec![],
        };
        // Slow onto a floor: lands.
        let mut p = Particle::new(Vec3::ZERO, 3.0, 0, 1.0);
        p.velocity = Vec3::new(10.0, -40.0, 0.0);
        bounce(&mut p, 0.5, Vec3::Y, 0.01, &b, 0.3);
        assert_eq!(p.velocity, Vec3::ZERO);
        // Fast onto a floor: bounces with the keep draw.
        let mut p = Particle::new(Vec3::ZERO, 3.0, 0, 1.0);
        p.velocity = Vec3::new(0.0, -100.0, 0.0);
        bounce(&mut p, 0.5, Vec3::Y, 0.01, &b, 0.3);
        assert!(
            p.velocity.abs_diff_eq(Vec3::new(0.0, 30.0, 0.0), 1e-4),
            "{}",
            p.velocity
        );
        // Into a wall.
        let mut p = Particle::new(Vec3::ZERO, 3.0, 0, 1.0);
        p.velocity = Vec3::new(-100.0, 0.0, 0.0);
        bounce(&mut p, 0.5, Vec3::X, 0.01, &b, 0.25);
        assert!(
            p.velocity.abs_diff_eq(Vec3::new(25.0, 0.0, 0.0), 1e-4),
            "{}",
            p.velocity
        );
    }

    /// Section 9: 2-4 flat glass shards of one shared size draw, spinning,
    /// fading over their last 2 s, thrown along the normal per axis; 4
    /// white grit sprites and one white cap.
    #[test]
    fn glass_impact_numbers() {
        let mats = EffectMaterials::all(0);
        let mut rng = ParticleRng::new(7);
        for _ in 0..50 {
            let n = Vec3::Z;
            let groups = glass_impact(&mut rng, &mats, Vec3::ZERO, n, Vec3::splat(0.5), &NoWorld);
            assert_eq!(groups.len(), 2);
            let shards = &groups[0].particles;
            assert!((2..=4).contains(&shards.len()), "{}", shards.len());
            for s in shards {
                assert!((2.5..=5.0).contains(&s.life));
                assert!(matches!(s.fade, Fade::Tail(t) if t == 2.0));
                assert!(matches!(s.shape, Shape::Flat { yaw_speed, .. } if yaw_speed.abs() <= 800.0));
                let side = s.size.start / UNIT;
                assert!((1.0..=9.0).contains(&side) && side.fract().abs() < 1e-3, "{side}");
                // Per axis (n + U(-0.8, 0.8)) × U(1, 300).
                let v = s.velocity / UNIT;
                assert!(v.z >= 0.2 - 1e-3 && v.z <= 1.8 * 300.0 + 1e-3, "{v}");
                assert!(v.x.abs() <= 0.8 * 300.0 + 1e-3);
                // (200, 200, 210) × (0.7 × 0.5 + 0.3), truncated.
                assert!((s.color * 255.0 - Vec3::new(130.0, 130.0, 136.0)).length() < 1e-3, "{}", s.color);
            }
            let pool = &groups[1].particles;
            assert_eq!(pool.len(), 5, "4 grit + 1 cap");
            assert!(pool.iter().all(|p| p.color == Vec3::ONE));
            assert!(pool[..4].iter().all(|p| (0.1..=0.25).contains(&p.life)));
            assert!((1.0..=1.5).contains(&pool[4].life));
        }
    }

    #[test]
    fn electric_big_spark_count_truncates() {
        assert_eq!(3.7f32 as usize, 3);
    }

    #[test]
    fn splash_numbers() {
        assert_eq!(splash_factor(8.0), 1.0);
        assert_eq!(splash_factor(12.0), 1.5);
        assert_eq!(splash_factor(40.0), 4.0);
        assert_eq!(gout(0, 1.0, 1.0), (400.0, 12.0, 24.0, 32.0));
        assert_eq!(gout(7, 1.0, 1.0), (50.0, 24.0, 48.0, 255.0));
        let (lum, c) = splash_lighting(Vec3::splat(0.1));
        assert!(approx(lum, 0.4, 1e-5) && c.abs_diff_eq(Vec3::ONE, 1e-6));
        assert!(approx(splash_lighting(Vec3::splat(0.02)).0, 0.25, 1e-6));
        let (lum, c) = splash_lighting(Vec3::new(0.5, 0.25, 0.0));
        assert!(approx(lum, 1.0, 1e-6));
        assert!(c.abs_diff_eq(Vec3::new(1.0, 0.875, 0.75), 1e-6), "{c}");
        // Ripple at f = 1, half its life: 16 + 112 × 0.7.
        let ripple = Ramp::linear(16.0, 128.0).with_bias(0.7);
        assert!(approx(ripple.at(0.5), 94.4, 1e-3));
        let alpha = Ramp::linear(1.0, 0.0).with_bias(0.25);
        assert!(approx(alpha.at(0.5), 0.75, 1e-5));
    }

    #[test]
    fn gouts_sink() {
        let m = Motion {
            sink: Some(0.0),
            ..default()
        };
        let mut p = Particle::new(Vec3::new(0.0, -2.5, 0.0), 2.0, 0, 10.0);
        assert!(approx(m.draw_factor(100.0, &p), 0.5, 1e-6));
        p.position.y = -11.0;
        let mut pool = Particles::default();
        let mut g = ParticleGroup::new(m);
        g.skip_first = false;
        g.particles.push(p);
        pool.add(g);
        pool.step(0.0, &NoWorld);
        assert_eq!(pool.count(), 0, "sunk below the surface");
    }

    #[test]
    fn water_entry() {
        let tank = MapWaterVolume {
            brush: crate::core::MapBrush::from_box(Vec3::new(-1.0, -1.0, -1.0), Vec3::new(1.0, 0.0, 1.0)),
            slime: false,
        };
        let w = [tank];
        assert_eq!(
            water_shot(Vec3::new(0.0, 2.0, 0.0), Vec3::new(0.0, 3.0, 0.0), &w),
            WaterShot::Dry
        );
        match water_shot(Vec3::new(0.0, 2.0, 0.0), Vec3::new(0.0, -0.5, 0.0), &w) {
            WaterShot::Entered { at, slime: false } => assert!(at.abs_diff_eq(Vec3::ZERO, 1e-5), "{at}"),
            other => panic!("{other:?}"),
        }
        assert_eq!(
            water_shot(Vec3::new(0.0, -0.2, 0.0), Vec3::new(0.5, -0.5, 0.0), &w),
            WaterShot::Under
        );
    }

    #[test]
    fn blood_is_the_same_for_any_damage() {
        let mats = EffectMaterials::all(0);
        let mut rng = ParticleRng::new(5);
        let groups = blood(&mut rng, &mats, &MapParticles::default(), Vec3::ZERO, Vec3::NEG_Z);
        let counts: Vec<usize> = groups.iter().map(|g| g.particles.len()).collect();
        assert_eq!(counts, BLOOD_COUNTS.to_vec());
        assert_eq!(counts.iter().sum::<usize>(), 67);
        assert!(groups.iter().all(|g| !g.capped));
        assert_eq!(bleed_traces(9.0), (1, 0.1));
        assert_eq!(bleed_traces(10.0), (2, 0.2));
        assert_eq!(bleed_traces(24.0), (2, 0.2));
        assert_eq!(bleed_traces(25.0), (4, 0.3));
    }
}
