//! How fire looks in CS:S (specs/source/fire.md 2.6, 4.4, 7): every lit
//! env_fire shows the `env_fire_large_smoke` particle system (a smoke
//! column, two flame layers and embers) at its floor point, whatever its
//! size; a burning entity shows `burning_character` (smoke and flames on
//! its box) and, in water, bubbles every 0.2 s. No glow and no light. The
//! systems are rebuilt here from the spec's tables of the install's
//! `particles/fire_01.pcf` and `burning_fx.pcf` with `map::particles`; the
//! operators the pool lacks are approximated (noise and random forces as
//! random start velocities, no twist). The logic layer says what burns
//! (`map::fire::MapFires`); its sounds are `General.BurningObject` loops
//! (the logic bridge) and the maps' own ambient_generics.
//!
//! Units: the spec's inches, z up, converted at the end (`v`).

use std::collections::HashMap;

use bevy::prelude::*;

use crate::{
    core::MapWater,
    map::{
        fire::MapFires,
        particles::{
            Fade, MapParticles, Motion, Particle, ParticleGroup, ParticleMaterials, ParticleRng, ParticleSet, Particles,
            Ramp,
        },
    },
};

const UNIT: f32 = 0.0254;
/// Flame sprites are drawn this much brighter (the materials' overbright
/// factor, `fire_env_fire` 2.15 and `fire_burning_character` 2.75).
const OVERBRIGHT_ENV: f32 = 2.15;
const OVERBRIGHT_CHARACTER: f32 = 2.75;
/// Bubbles in water: every flame think (0.2 s), 12 of them in the box
/// from the centre ± 32 in across and 32 below, rising at 8 in/s.
const BUBBLE_INTERVAL: f32 = 0.2;
const BUBBLES: usize = 12;
const BUBBLE_SPEED: f32 = 8.0;
/// A bubble's half-size, in (the temp entity's sprite at scale 1, ours).
const BUBBLE_SIZE: f32 = 1.5;

/// Particle materials fire uses (loaded with the map's).
pub const MATERIALS: &[&str] = &[
    "particle/vistasmokev1/vistasmokev1",
    "particle/vistasmokev1/vistasmokev1_nearcull",
    "particle/fire_burning_character/fire_env_fire",
    "particle/fire_burning_character/fire_burning_character",
    "particle/particle_glow_05_additive",
    "sprites/bubble",
];

/// Sound entries burning entities play (the logic bridge starts and stops
/// them).
pub const SOUNDS: &[&str] = &["General.BurningObject", "General.StopBurning"];

pub struct FirePlugin;

impl Plugin for FirePlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Particles>()
            .init_resource::<FireFx>()
            .add_systems(Update, draw_fires.after(ParticleSet::Step).before(ParticleSet::Draw));
    }
}

/// Emitters of the fires and flames burning now, by their key.
#[derive(Resource)]
struct FireFx {
    rng: ParticleRng,
    emitters: HashMap<u64, Emitter>,
}

impl Default for FireFx {
    fn default() -> Self {
        Self {
            rng: ParticleRng::new(0xF1_8E),
            emitters: HashMap::new(),
        }
    }
}

/// One running system: particles owed per part, and its age.
#[derive(Default)]
struct Emitter {
    owed: [f32; 5],
    age: f32,
    bubbles: f32,
    seen: bool,
}

/// The materials by role.
#[derive(Clone, Copy, Default)]
struct Mats {
    smoke: Option<usize>,
    smoke_near: Option<usize>,
    env_fire: Option<usize>,
    character: Option<usize>,
    glow: Option<usize>,
    bubble: Option<usize>,
}

impl Mats {
    fn new(p: &MapParticles) -> Self {
        let f = |n: &str| p.find(n);
        Self {
            smoke: f(MATERIALS[0]),
            smoke_near: f(MATERIALS[1]).or(f(MATERIALS[0])),
            env_fire: f(MATERIALS[2]).or(f(MATERIALS[3])),
            character: f(MATERIALS[3]).or(f(MATERIALS[2])),
            glow: f(MATERIALS[4]),
            bubble: f(MATERIALS[5]),
        }
    }
}

/// Source (x, y, z) inches to engine meters.
fn v(x: f32, y: f32, z: f32) -> Vec3 {
    Vec3::new(x, z, -y) * UNIT
}

fn rgb(c: [f32; 3]) -> Vec3 {
    Vec3::from(c) / 255.0
}

fn between(rng: &mut ParticleRng, a: [f32; 3], b: [f32; 3]) -> Vec3 {
    rgb(a).lerp(rgb(b), rng.random())
}

/// A random direction in the upper half (z ≥ 0), x/y scaled by `xy`.
fn upper_dir(rng: &mut ParticleRng, xy: f32) -> Vec3 {
    loop {
        let d = rng.vec3(-1.0, 1.0);
        let l = d.length_squared();
        if l > 1e-4 && l <= 1.0 {
            let d = d / l.sqrt();
            return Vec3::new(d.x * xy, d.y * xy, d.z.abs());
        }
    }
}

/// A point on a disc of radius 0..r (Source x/y).
fn disc(rng: &mut ParticleRng, r: f32) -> Vec3 {
    let a = rng.float(0.0, std::f32::consts::TAU);
    let d = rng.float(0.0, r);
    Vec3::new(a.cos() * d, a.sin() * d, 0.0)
}

/// Frames in a material's sheet sequence (`fallback` when unknown).
fn frames(mats: &MapParticles, material: usize, sequence: u16, fallback: usize) -> usize {
    mats.materials
        .get(material)
        .and_then(|m| m.sequences.get(sequence as usize))
        .map_or(fallback, |s| s.len().max(1))
}

/// A Source-space point/velocity pair into a particle.
fn particle(at: Vec3, local: Vec3, vel: Vec3, life: f32, material: usize) -> Particle {
    Particle {
        velocity: v(vel.x, vel.y, vel.z),
        ..Particle::new(at + v(local.x, local.y, local.z), life, material, 0.0)
    }
}

fn group(gravity_up: f32, drag: f32) -> ParticleGroup {
    let mut g = ParticleGroup::new(Motion {
        gravity: -gravity_up * UNIT,
        drag,
        ..default()
    });
    // A particle system: its own budget, not the old-style cap.
    g.capped = false;
    g
}

/// `count` particles of part `part` of `env_fire_large_smoke` (spec 7.1)
/// at floor point `at`.
fn env_fire_part(
    rng: &mut ParticleRng,
    m: &Mats,
    mats: &MapParticles,
    part: usize,
    at: Vec3,
    count: usize,
) -> Option<ParticleGroup> {
    let mut g;
    match part {
        // Smoke (root) and smoke b.
        0 | 1 => {
            let mat = m.smoke?;
            let root = part == 0;
            g = group(if root { 75.0 } else { 45.0 }, 0.01);
            for _ in 0..count {
                let life = rng.float(3.0, 4.0);
                let seq = rng.int(0, 3) as u16;
                let n = frames(mats, mat, seq, 16) as f32;
                let r = rng.float(18.0, 24.0);
                let (shell, lift) = if root {
                    (rng.float(24.0, 48.0), rng.float(48.0, 64.0))
                } else {
                    (24.0, 48.0)
                };
                let local = upper_dir(rng, 1.0) * shell + Vec3::new(rng.float(-4.0, 4.0), rng.float(-4.0, 4.0), lift);
                let up = if root {
                    rng.float(22.0, 24.0) + rng.float(25.0, 42.0)
                } else {
                    rng.float(22.0, 28.0) + rng.float(15.0, 32.0)
                };
                let vel = Vec3::new(rng.float(-12.0, 12.0), rng.float(-12.0, 12.0), up);
                let (color, fade_to, window) = if root {
                    (
                        between(rng, [255.0, 131.0, 9.0], [194.0, 108.0, 32.0]),
                        rgb([59.0, 58.0, 56.0]),
                        (0.1, 0.8),
                    )
                } else {
                    (
                        between(rng, [255.0, 131.0, 9.0], [255.0, 120.0, 0.0]),
                        rgb([52.0, 50.0, 50.0]),
                        (0.2, 0.6),
                    )
                };
                let alpha = if root {
                    rng.float(90.0, 100.0)
                } else {
                    rng.float(120.0, 140.0)
                };
                g.particles.push(Particle {
                    sequence: seq,
                    fps: n / life,
                    color,
                    color_fade: Some((fade_to, window.0, window.1)),
                    alpha: alpha / 255.0,
                    fade: Fade::Windows {
                        start_alpha: 0.0,
                        fade_in: (0.0, 0.15),
                        fade_out: (0.5, 1.0),
                    },
                    size: Ramp::linear(r * UNIT, 5.0 * r * UNIT).with_bias(0.4),
                    roll: rng.float(0.0, std::f32::consts::TAU),
                    roll_speed: rng.float(-0.4, 0.4),
                    ..particle(at, local, vel, life, mat)
                });
            }
        }
        // Flames and flames b.
        2 | 3 => {
            let mat = m.env_fire?;
            let main = part == 2;
            g = group(if main { 100.0 } else { 90.0 }, if main { 0.0055 } else { 0.0 });
            for _ in 0..count {
                let (seq, fps, fallback) = if main {
                    let s = rng.int(3, 5) as u16;
                    (s, 20.0, [24, 35, 28][(s - 3) as usize])
                } else {
                    let s = rng.int(0, 2) as u16;
                    (s, 14.0, [12, 11, 12][s as usize])
                };
                let life = frames(mats, mat, seq, fallback) as f32 / fps;
                let r = if main {
                    rng.float(24.0, 32.0)
                } else {
                    rng.float(32.0, 48.0)
                };
                let local = disc(rng, if main { 20.0 } else { 18.0 }) + Vec3::Z * r * if main { 0.3 } else { 0.45 };
                // Velocity noise and the random force (±16 a second) as
                // a spread of start velocities.
                let vel = if main {
                    Vec3::new(
                        rng.float(-24.0, 24.0),
                        rng.float(-24.0, 24.0),
                        rng.float(0.0, 5.0) + rng.float(12.0, 24.0),
                    )
                } else {
                    Vec3::new(rng.float(-8.0, 8.0), rng.float(-8.0, 8.0), rng.float(48.0, 96.0))
                };
                let (color, color_fade, alpha, fade) = if main {
                    let grey = rng.float(161.0, 222.0);
                    (
                        rgb([grey; 3]),
                        // To black from 70 % of life, an end time of 1.2
                        // (spec Q10): about 60 % of the way at death.
                        Some((Vec3::ZERO, 0.7, 1.2)),
                        rng.float(80.0, 180.0),
                        Fade::Tail(rng.float(0.1, 0.125)),
                    )
                } else {
                    let grey = rng.float(156.0, 194.0);
                    (
                        rgb([grey; 3]),
                        None,
                        rng.float(120.0, 180.0),
                        Fade::Windows {
                            start_alpha: 1.0,
                            fade_in: (0.0, 0.0),
                            fade_out: (0.75, 1.0),
                        },
                    )
                };
                let roll = if main { 8.0 } else { 24.0 };
                g.particles.push(Particle {
                    sequence: seq,
                    fps,
                    color: color * OVERBRIGHT_ENV,
                    color_fade: color_fade.map(|(c, a, b)| (c, a, b)),
                    alpha: alpha / 255.0,
                    fade,
                    size: Ramp::constant(r * UNIT),
                    roll: rng.float(-roll, roll).to_radians(),
                    ..particle(at, local, vel, life, mat)
                });
            }
        }
        // Embers.
        _ => {
            let mat = m.glow?;
            g = group(40.0, 0.01);
            for _ in 0..count {
                let life = rng.float(0.5, 2.0);
                let local = upper_dir(rng, 0.5) * rng.float(32.0, 48.0) + Vec3::Z * rng.float(32.0, 64.0);
                // Random speed, local up, and the random force (±150 a
                // second) as a spread.
                let dir = upper_dir(rng, 1.0);
                let vel = dir * rng.float(0.0, 15.0)
                    + Vec3::Z * rng.float(12.0, 38.0)
                    + Vec3::new(rng.float(-60.0, 60.0), rng.float(-60.0, 60.0), rng.float(-30.0, 30.0));
                g.particles.push(Particle {
                    color: between(rng, [169.0, 16.0, 16.0], [238.0, 204.0, 18.0]),
                    fade: Fade::Windows {
                        start_alpha: 0.0,
                        fade_in: (0.0, 0.25),
                        fade_out: (0.75, 1.0),
                    },
                    size: Ramp::constant(rng.float(1.0, 3.0) * UNIT),
                    ..particle(at, local, vel, life, mat)
                });
            }
        }
    }
    Some(g)
}

/// A random point in a box (engine space), the box scaled by `scale`
/// about its centre.
fn in_box(rng: &mut ParticleRng, min: Vec3, max: Vec3, scale: f32) -> Vec3 {
    let c = (min + max) / 2.0;
    let h = (max - min) / 2.0 * scale;
    c + Vec3::new(rng.float(-h.x, h.x), rng.float(-h.y, h.y), rng.float(-h.z, h.z))
}

/// `count` particles of part `part` of `burning_character` (spec 7.2) on
/// the box `min`..`max`, for a system `age` s old.
#[allow(clippy::too_many_arguments)]
fn character_part(
    rng: &mut ParticleRng,
    m: &Mats,
    mats: &MapParticles,
    part: usize,
    min: Vec3,
    max: Vec3,
    age: f32,
    count: usize,
) -> Option<ParticleGroup> {
    // Flames grow in over the system's first 0.75 s.
    let grow = (age / 0.75).clamp(0.0, 1.0);
    let mut g;
    match part {
        // Smoke (root): builds up from 1 s to 4 s of the system's age.
        0 => {
            let mat = m.smoke_near?;
            let build = ((age - 1.0) / 3.0).clamp(0.0, 1.0);
            if build <= 0.0 {
                return None;
            }
            g = group(47.0, 0.025);
            for _ in 0..count {
                let seq = rng.int(4, 9) as u16;
                let life = 1.5;
                let r = rng.float(18.0, 24.0);
                let at = in_box(rng, min, max, 1.0) + v(0.0, 0.0, 16.0);
                let vel = Vec3::new(rng.float(-15.0, 15.0), rng.float(-15.0, 15.0), rng.float(80.0, 100.0));
                g.particles.push(Particle {
                    sequence: seq,
                    fps: frames(mats, mat, seq, 6) as f32 / life,
                    color: between(rng, [50.0; 3], [39.0; 3]),
                    color_fade: Some((Vec3::ZERO, 0.0, 1.0)),
                    alpha: rng.float(170.0, 190.0) / 255.0 * build,
                    fade: Fade::Windows {
                        start_alpha: 0.0,
                        fade_in: (0.0, 0.25),
                        fade_out: (1.0 - 1.17 / 1.5, 1.0),
                    },
                    size: Ramp::linear(r * UNIT, 4.0 * r * UNIT).with_bias(0.4),
                    roll: rng.float(0.0, std::f32::consts::TAU),
                    ..particle(at, Vec3::ZERO, vel, life, mat)
                });
            }
        }
        // Flames b (1) and d (2).
        _ => {
            let mat = m.character?;
            let b = part == 1;
            g = group(if b { 75.0 } else { 170.0 }, if b { 0.015 } else { 0.025 });
            for _ in 0..count {
                let seq = if b { rng.int(3, 5) } else { rng.int(0, 2) } as u16;
                let fps = if b { 22.0 } else { 20.0 };
                let fallback = if b {
                    [24, 35, 28][(seq - 3) as usize]
                } else {
                    [12, 11, 12][seq as usize]
                };
                let life = frames(mats, mat, seq, fallback) as f32 / fps;
                let r = if b {
                    rng.float(9.0, 30.0)
                } else {
                    rng.float(11.0, 17.0)
                } * grow.max(0.05);
                let at = in_box(rng, min, max, if b { 1.5 } else { 1.0 })
                    + v(0.0, 0.0, r * if b { rng.float(0.35, 0.5) } else { rng.float(0.125, 0.25) });
                let spread = if b { 25.0 } else { 55.0 };
                let vel = Vec3::new(
                    rng.float(-spread, spread),
                    rng.float(-spread, spread),
                    if b {
                        rng.float(10.0, 20.0)
                    } else {
                        rng.float(0.0, 25.0)
                    },
                );
                let (color, fade_from, alpha, fade, size) = if b {
                    (
                        between(rng, [207.0, 174.0, 174.0], [207.0, 205.0, 205.0]),
                        (0.65, 0.95),
                        rng.float(100.0, 200.0),
                        Fade::Tail(life * 0.1),
                        Ramp::linear(0.75 * r * UNIT, 1.25 * r * UNIT).with_bias(0.6),
                    )
                } else {
                    (
                        between(rng, [201.0, 147.0, 147.0], [216.0, 178.0, 141.0]),
                        (0.85, 1.0),
                        rng.float(64.0, 128.0),
                        Fade::Windows {
                            start_alpha: 0.0,
                            fade_in: (0.0, 0.11),
                            fade_out: (0.8, 1.0),
                        },
                        Ramp::linear(0.0, r * UNIT).with_bias(0.75),
                    )
                };
                g.particles.push(Particle {
                    sequence: seq,
                    fps,
                    color: color * OVERBRIGHT_CHARACTER,
                    color_fade: Some((Vec3::ZERO, fade_from.0, fade_from.1)),
                    alpha: alpha / 255.0,
                    fade,
                    size,
                    roll: rng.float(-30.0, 30.0).to_radians(),
                    ..particle(at, Vec3::ZERO, vel, life, mat)
                });
            }
        }
    }
    Some(g)
}

/// Rates per second of `env_fire_large_smoke`'s parts: smoke, smoke b,
/// flames, flames b, embers.
const ENV_RATES: [f32; 5] = [2.0, 4.0, 17.0, 12.0, 8.0];
/// `burning_character`'s: smoke, flames b, flames d (flames e, 150 a
/// second of small sparks, left out).
const CHARACTER_RATES: [f32; 3] = [13.5, 7.0, 30.0];

/// The surface of the water at `p`, if `p` is in water.
fn water_surface(water: Option<&MapWater>, p: Vec3) -> Option<f32> {
    water?.0.iter().find(|w| w.brush.contains(p)).map(|w| w.brush.max.y)
}

#[allow(clippy::too_many_arguments)]
fn draw_fires(
    fires: Option<Res<MapFires>>,
    materials: Option<Res<ParticleMaterials>>,
    water: Option<Res<MapWater>>,
    time: Res<Time>,
    mut particles: ResMut<Particles>,
    mut fx: ResMut<FireFx>,
) {
    let (Some(fires), Some(materials)) = (fires, materials) else {
        fx.emitters.clear();
        return;
    };
    let mats = &materials.0;
    let m = Mats::new(mats);
    let dt = time.delta_secs().clamp(0.0, 0.1);
    let FireFx { rng, emitters } = &mut *fx;
    for e in emitters.values_mut() {
        e.seen = false;
    }
    for f in &fires.fires {
        let e = emitters.entry(f.key).or_default();
        e.seen = true;
        e.age += dt;
        for (part, rate) in ENV_RATES.iter().enumerate() {
            if f.smokeless && part < 2 {
                continue;
            }
            e.owed[part] += rate * dt;
            let n = e.owed[part].floor();
            if n >= 1.0 {
                e.owed[part] -= n;
                if let Some(g) = env_fire_part(rng, &m, mats, part, f.at, n as usize) {
                    particles.add(g);
                }
            }
        }
    }
    for f in &fires.flames {
        let e = emitters.entry(f.key).or_default();
        e.seen = true;
        let age = e.age;
        e.age += dt;
        for (part, rate) in CHARACTER_RATES.iter().enumerate() {
            e.owed[part] += rate * dt;
            let n = e.owed[part].floor();
            if n >= 1.0 {
                e.owed[part] -= n;
                if let Some(g) = character_part(rng, &m, mats, part, f.min, f.max, age, n as usize) {
                    particles.add(g);
                }
            }
        }
        // Bubbles while its feet are in water (spec 4.3 step 3).
        e.bubbles += dt;
        if e.bubbles >= BUBBLE_INTERVAL {
            e.bubbles -= BUBBLE_INTERVAL;
            let centre = (f.min + f.max) / 2.0;
            let feet = Vec3::new(centre.x, f.min.y + 0.01, centre.z);
            if let (Some(surface), Some(mat)) = (water_surface(water.as_deref(), feet), m.bubble) {
                let mut g = group(0.0, 0.0);
                for _ in 0..BUBBLES {
                    let at = centre
                        + v(
                            rng.float(-32.0, 32.0),
                            rng.float(-32.0, 32.0),
                            rng.float(-32.0, 0.0),
                        );
                    let rise = (surface - at.y).max(0.0);
                    let life = rise / (BUBBLE_SPEED * UNIT);
                    if life <= 0.0 {
                        continue;
                    }
                    g.particles.push(Particle {
                        velocity: Vec3::Y * BUBBLE_SPEED * UNIT,
                        ..Particle::new(at, life, mat, BUBBLE_SIZE * UNIT)
                    });
                }
                particles.add(g);
            }
        }
    }
    // A system whose fire went out stops emitting; its particles live on.
    emitters.retain(|_, e| e.seen);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::map::particles::ParticleMaterial;

    fn mats() -> MapParticles {
        MapParticles {
            materials: MATERIALS
                .iter()
                .map(|n| ParticleMaterial {
                    name: n.to_string(),
                    ..default()
                })
                .collect(),
        }
    }

    #[test]
    fn env_fire_parts_start_where_the_spec_puts_them() {
        let mats = mats();
        let m = Mats::new(&mats);
        let mut rng = ParticleRng::new(1);
        let at = Vec3::new(1.0, 2.0, 3.0);
        // Flames: a 20-inch disc, raised 0.3 x radius, rising.
        let g = env_fire_part(&mut rng, &m, &mats, 2, at, 50).unwrap();
        assert_eq!(g.particles.len(), 50);
        for p in &g.particles {
            let d = (p.position - at) / UNIT;
            assert!(Vec2::new(d.x, d.z).length() <= 20.01, "{d}");
            assert!(d.y >= 0.3 * 24.0 - 0.01 && d.y <= 0.3 * 32.0 + 0.01, "{d}");
            assert!(p.velocity.y > 0.0);
            assert!((1.2..=1.75).contains(&p.life), "fallback sheet lives: {}", p.life);
        }
        // Smoke: 48-64 in up plus up to 48 more on the shell, grows x5.
        let g = env_fire_part(&mut rng, &m, &mats, 0, at, 20).unwrap();
        for p in &g.particles {
            let d = (p.position - at) / UNIT;
            assert!(d.y >= 48.0 - 0.01 && d.y <= 112.01, "{d}");
            assert!((p.size.end / p.size.start - 5.0).abs() < 1e-4);
        }
    }

    #[test]
    fn a_fire_emits_at_the_spec_rates_and_stops_when_out() {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .add_plugins(FirePlugin)
            .insert_resource(ParticleMaterials(mats()))
            .insert_resource(MapFires {
                fires: vec![crate::map::fire::FireLook {
                    key: 1,
                    at: Vec3::ZERO,
                    size: 256.0,
                    smokeless: false,
                }],
                flames: Vec::new(),
            });
        app.configure_sets(Update, ParticleSet::Step.before(ParticleSet::Draw));
        app.insert_resource(bevy::time::TimeUpdateStrategy::ManualDuration(
            std::time::Duration::from_millis(50),
        ));
        for _ in 0..41 {
            app.update();
        }
        // 2 s at 43 a second, none dying yet that young but the flames:
        // count everything spawned so far by the emitter's owed sums.
        let n = app.world().resource::<Particles>().count();
        assert!(n > 40, "particles alive: {n}");
        app.world_mut().resource_mut::<MapFires>().fires.clear();
        app.update();
        assert!(app.world().resource::<FireFx>().emitters.is_empty());
    }
}
