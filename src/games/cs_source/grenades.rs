//! CS:S's grenades (specs/cs_source/grenades.md): the HE grenade,
//! flashbang and smoke grenade as `weapon::grenade::Throwable` weapons with
//! the spec's numbers, the HE explosion's particles (spec 5.4, built on
//! `map::particles`), the smoke cloud's 64 sprites (7.3), and the flash
//! model (6.2, a fit pending the probe's measurement), and what blasts
//! and flashes do to hearing (5.5, 6.4: the DSP presets as
//! `core::HearingEffect`s).
//!
//! Units: the spec's inches, converted with `UNIT`. Source's up (z) is our
//! y.

use bevy::prelude::*;

use super::weapons::HP;
use crate::{
    core::HearingEffect,
    map::{
        LightField,
        particles::{
            Bounce, Fade, MapParticles, Motion, Particle, ParticleGroup, ParticleMaterials, ParticleRng, ParticleSet,
            Particles, Ramp, Shape, WorldTrace, WorldTracer, probe_planes,
        },
    },
    weapon::{
        RegisterWeapons, Weapon, WeaponSounds,
        grenade::{
            Blast, BlastHearing, Detonated, ExplosionRule, Flash, Flight, GrenadeEffect, GrenadeKind, Shake, Smoke,
            SmokeCloud, ThrowRule, Throwable,
        },
    },
};

const UNIT: f32 = 0.0254;

pub const HEGRENADE: &str = "cs_source:weapon_hegrenade";
pub const FLASHBANG: &str = "cs_source:weapon_flashbang";
pub const SMOKEGRENADE: &str = "cs_source:weapon_smokegrenade";

/// (weapon, price, carry limit): `WeaponPrice` and `ammo_*_max` (spec 1).
pub const GRENADES: &[(&str, u32, u32)] = &[(HEGRENADE, 300, 1), (FLASHBANG, 200, 2), (SMOKEGRENADE, 300, 1)];

/// World models (held, and drawn in flight: the `_thrown` models have no
/// bone in common with the player skeleton, so the held ones stand in).
pub const WORLD_MODELS: &[(&str, &str)] = &[
    (HEGRENADE, "models/weapons/w_eq_fraggrenade.mdl"),
    (FLASHBANG, "models/weapons/w_eq_flashbang.mdl"),
    (SMOKEGRENADE, "models/weapons/w_eq_smokegrenade.mdl"),
];

/// View models and whether each is built right-handed (all three hold the
/// grenade left of the eye, like the AK; checked by
/// `tests/it/heavy/map_de_dust2.rs::grenade_models_and_sequences`).
pub const VIEW_MODELS: &[(&str, &str, bool)] = &[
    (HEGRENADE, "models/weapons/v_eq_fraggrenade.mdl", false),
    (FLASHBANG, "models/weapons/v_eq_flashbang.mdl", false),
    (SMOKEGRENADE, "models/weapons/v_eq_smokegrenade.mdl", false),
];

/// Sound entries the grenades use.
pub const SOUNDS: &[&str] = &[
    "Default.PullPin_Grenade",
    "HEGrenade.Bounce",
    "Flashbang.Bounce",
    "SmokeGrenade.Bounce",
    "BaseGrenade.Explode",
    "BaseExplosionEffect.Sound",
    "Flashbang.Explode",
    "BaseSmokeEffect.Sound",
];

/// Particle materials the effects use (loaded with the map's).
pub const MATERIALS: &[&str] = &[
    "effects/fire_embers1",
    "effects/fire_embers2",
    "effects/fire_cloud2",
    "particle/particle_smokegrenade1",
    "sprites/light_glow02_add_noz",
];

/// The view models' durations (spec weapons.md, view-model table):
/// `pullpin` 40 frames at 41 fps (0.9512 s), `throw` 24 at 30 (0.7667 s),
/// `deploy`.
pub const PIN_TIME: f32 = 39.0 / 41.0;
pub const THROW_TIME: f32 = 23.0 / 30.0;
pub const DRAW_TIME: f32 = 0.6667;
/// The pin sound: the `pullpin` event at cycle 0.6923.
pub const PIN_SOUND_AT: f32 = 0.6923 * PIN_TIME;

pub struct GrenadesPlugin;

impl Plugin for GrenadesPlugin {
    fn build(&self, app: &mut App) {
        app.register_weapon(HEGRENADE, hegrenade)
            .register_weapon(FLASHBANG, flashbang)
            .register_weapon(SMOKEGRENADE, smokegrenade)
            .add_message::<Detonated>()
            // Exploding props blast like the HE grenade (prop_damage.md
            // 7.5: the game's radius damage; spec Q3).
            .insert_resource(ExplosionRule(he_blast()))
            .init_resource::<Particles>()
            .init_resource::<FxRng>()
            .add_systems(FixedUpdate, explosions.after(crate::core::SimSet::Weapons))
            .add_systems(Update, draw_clouds.after(ParticleSet::Step).before(ParticleSet::Draw));
    }
}

#[derive(Resource)]
struct FxRng(ParticleRng);

impl Default for FxRng {
    fn default() -> Self {
        Self(ParticleRng::new(0x0E_FFEC7))
    }
}

/// The throw (spec 3).
fn throw_rule() -> ThrowRule {
    ThrowRule {
        pitch_lift: -10.0,
        pitch_scale: 100.0 / 90.0,
        speed_per_deg: 6.0 * UNIT,
        speed_max: 750.0 * UNIT,
        forward: 16.0 * UNIT,
    }
}

/// The projectile (spec 4, "all CS grenades"): 0.4 x sv_gravity 800.
pub fn flight() -> Flight {
    Flight {
        half: 2.0 * UNIT,
        gravity: 0.4 * 800.0 * UNIT,
        elasticity: 0.45,
        player_elasticity: 0.3,
        elasticity_cap: 0.9,
        stop_speed: 30.0 * UNIT,
        floor_normal: 0.7,
        breakable_damage: 10.0 * HP,
        breakable_slowdown: 0.4,
        water_slowdown: 0.5,
        water_entry: 0.5,
        max_velocity: 3500.0 * UNIT,
        check_interval: 0.2,
        spin_roll: 600f32.to_radians(),
        spin_pitch: 1200f32.to_radians(),
    }
}

/// The HE blast (spec 5): 100 at the centre to 0 at 350 units, armour
/// by the script's ratio (H1), the push capped at 75 kg x 400 units/s
/// times the template's 1.5.
pub fn he_blast() -> Blast {
    Blast {
        damage: 100.0 * HP,
        radius: 350.0 * UNIT,
        // UNMEASURED (Q4): hypothesis H1, the HE script's WeaponArmorRatio.
        armor_ratio: Some(1.475),
        quantum: HP,
        probe_up: 8.0 * UNIT,
        probe_down: 24.0 * UNIT,
        pull_out: 0.6 * UNIT,
        src_lift: UNIT,
        body_target: (0.7, 1.0),
        force: (100.0f32 * 300.0).min(30000.0) * 1.5 * UNIT,
        force_jitter: (0.85, 1.15),
        // Source physics' speed limit, 2000 units/s.
        max_push_speed: 2000.0 * UNIT,
        scorch: Some("scorch".into()),
        sound: Some("BaseGrenade.Explode".into()),
        hearing: Some(BlastHearing {
            shock_damage: SHOCK_DAMAGE * HP,
            shock: SHOCK_MUFFLE,
            ring_distance: EAR_RING_DISTANCE * UNIT,
            ring: EXPLOSION_RING,
        }),
        shake: Some(he_shake()),
    }
}

/// The HE grenade's view shake: a GUESS (spec 5.2: the template's base
/// grenade has none; whether CS:S shakes is Q9). Modest: 2 units at the
/// blast, none past 700 units, 30 changes a second, over 1 s.
pub fn he_shake() -> Shake {
    Shake {
        amplitude: 2.0 * UNIT,
        frequency: 30.0,
        duration: 1.0,
        radius: 700.0 * UNIT,
    }
}

// The hearing effects (spec 5.5, 6.4): the install's DSP presets (values
// from scripts/dsp_presets.txt). Each row's duration holds the preset; its
// fade time -1 means a 1 s exponential fade. We use the top of each
// preset's mix range (sounds far from the listener get the most) and
// stand in for its parts: a low-pass filter as is, a diffusor (all-pass
// smearing) as a gentle low-pass (`DIFFUSOR_CUTOFF`), a 3 kHz sine LFO as
// a ringing tone at the LFO's gain.

/// Blast damage from which the shock effect plays, hp (spec 5.5).
pub const SHOCK_DAMAGE: f32 = 30.0;
/// Closer than this (units) the ring effect plays (spec 5.5).
pub const EAR_RING_DISTANCE: f32 = 240.0;
/// Our stand-in cutoff for a diffusor, Hz (ours).
pub const DIFFUSOR_CUTOFF: f32 = 2500.0;
/// The presets' ringing LFO, Hz.
pub const RING_HZ: f32 = 3000.0;

/// Presets 32-34, "explosion ring" (identical rows): diffusor, then a
/// 1 kHz low-pass at gain 0.25; mix 0.2-0.7; 1.6 s, exponential fade.
pub const EXPLOSION_RING: HearingEffect = HearingEffect {
    hold: 1.6,
    fade: 1.0,
    exponential: true,
    mix: 0.7,
    cutoff: 1000.0,
    wet_gain: 0.25,
    ring_hz: RING_HZ,
    ring_gain: 0.0,
};

/// Presets 35-37, "shock muffle" (identical rows): diffusor, then a 3 kHz
/// sine LFO at gain 0.25; mix 0.2-0.7; 1.6 s, exponential fade.
pub const SHOCK_MUFFLE: HearingEffect = HearingEffect {
    hold: 1.6,
    fade: 1.0,
    exponential: true,
    mix: 0.7,
    cutoff: DIFFUSOR_CUTOFF,
    wet_gain: 1.0,
    ring_hz: RING_HZ,
    ring_gain: 0.25,
};

/// Presets 134-136, "flashbang muffle" long / medium / short (spec 6.4):
/// diffusor plus a 3 kHz sine LFO at gain 0.05; mix up to 0.7 / 0.4 /
/// 0.2, held 1.6 / 0.2 / 0.1 s, exponential fade.
pub const FLASH_MUFFLE: [HearingEffect; 3] = [
    flash_muffle(0.7, 1.6),
    flash_muffle(0.4, 0.2),
    flash_muffle(0.2, 0.1),
];

const fn flash_muffle(mix: f32, hold: f32) -> HearingEffect {
    HearingEffect {
        hold,
        fade: 1.0,
        exponential: true,
        mix,
        cutoff: DIFFUSOR_CUTOFF,
        wet_gain: 1.0,
        ring_hz: RING_HZ,
        ring_gain: 0.05,
    }
}

/// Which flashbang muffle a blindness gets (spec 6.4 hypothesis, Q14:
/// by its strength): the long one from 60 % of the longest blindness
/// (`FLASH_MAX_TIME`), the medium one from 30 %, else the short one.
pub fn flash_hearing(_alpha: f32, seconds: f32) -> Option<HearingEffect> {
    let s = seconds / FLASH_MAX_TIME;
    Some(if s >= 0.6 {
        FLASH_MUFFLE[0]
    } else if s >= 0.3 {
        FLASH_MUFFLE[1]
    } else {
        FLASH_MUFFLE[2]
    })
}

/// How far a flash reaches, units (FIT, Q12: not measured).
pub const FLASH_RADIUS: f32 = 1500.0;
/// The longest blindness, seconds, and how much of it is the fade (FIT).
pub const FLASH_MAX_TIME: f32 = 5.0;
pub const FLASH_MAX_FADE: f32 = 3.0;

/// The flash model (spec 6.2; a FIT pending the probe's measurement grid,
/// Q12/Q13): strength falls with the square of the distance to 0 at
/// `FLASH_RADIUS` and with facing (full within 60° of the view, a quarter
/// facing away: the tutor's "lessens"); a strong flash is full white and
/// lasts up to `FLASH_MAX_TIME`, held, then fading over at most
/// `FLASH_MAX_FADE`.
pub fn flash_model(distance: f32, facing: f32) -> Option<(f32, f32, f32)> {
    let r = distance / UNIT;
    if r >= FLASH_RADIUS {
        return None;
    }
    let near = 1.0 - (r / FLASH_RADIUS).powi(2);
    let look = 0.25 + 0.75 * ((facing + 0.5) / 1.0).clamp(0.0, 1.0);
    let s = near * look;
    if s < 0.05 {
        return None;
    }
    let time = FLASH_MAX_TIME * s;
    let fade = time.min(FLASH_MAX_FADE);
    Some(((2.0 * s).min(1.0), time - fade, fade))
}

/// The smoke cloud (spec 7): 240 units across in 1 s, held to 17 s, gone
/// by 22 s. Bots' sight is blocked through three quarters of its radius
/// (Q16 hypothesis: the sprites reach E/2 plus their 80-unit size).
pub fn smoke() -> Smoke {
    Smoke {
        max_radius: 240.0 * UNIT,
        expand_time: 1.0,
        fade_start: 17.0,
        fade_end: 22.0,
        // Q15 hypothesis: it pops once (nearly) still.
        pop_speed: 0.1 * UNIT,
        sight_radius: 0.75,
        sound: Some("BaseSmokeEffect.Sound".into()),
    }
}

fn grenade(e: &mut EntityWorldMut, id: &'static str, max_speed: f32, effect: GrenadeEffect, bounce: &str) {
    let max = GRENADES.iter().find(|g| g.0 == id).map_or(1, |g| g.2);
    e.insert((
        Weapon {
            id,
            slot: 3,
            owner: None,
            draw_time: DRAW_TIME,
            max_speed: Some(max_speed * UNIT),
        },
        Throwable {
            count: 1,
            max,
            pin_time: PIN_TIME,
            // Q1: the template's release + 0.1 s (not the throw event).
            throw_delay: 0.1,
            throw_time: THROW_TIME,
            redraw_time: DRAW_TIME,
            // Q8: the template's 1.5 s, checked every 0.2 s.
            fuse: 1.5,
            throw: throw_rule(),
            flight: flight(),
            effect,
            pin_sounds: vec![(PIN_SOUND_AT, "Default.PullPin_Grenade".into())],
            bounce_sound: Some(bounce.into()),
            pin: false,
            throw_at: None,
            redraw_at: None,
        },
        WeaponSounds::default(),
    ));
}

fn hegrenade(e: &mut EntityWorldMut) {
    grenade(
        e,
        HEGRENADE,
        250.0,
        GrenadeEffect::Blast(he_blast()),
        "HEGrenade.Bounce",
    );
}

fn flashbang(e: &mut EntityWorldMut) {
    let effect = GrenadeEffect::Flash(Flash {
        model: flash_model,
        sound: Some("Flashbang.Explode".into()),
        hearing: Some(flash_hearing),
    });
    grenade(e, FLASHBANG, 250.0, effect, "Flashbang.Bounce");
}

fn smokegrenade(e: &mut EntityWorldMut) {
    grenade(
        e,
        SMOKEGRENADE,
        245.0,
        GrenadeEffect::Smoke(smoke()),
        "SmokeGrenade.Bounce",
    );
}

// ---------------------------------------------------------------------------
// The HE explosion (spec 5.4)

/// Particle materials by role.
#[derive(Clone, Debug, Default)]
pub struct FxMaterials {
    pub noise: Option<usize>,
    pub embers: [Option<usize>; 2],
    pub fire: Option<usize>,
    pub cement: [Option<usize>; 2],
    pub glow: Option<usize>,
    pub cloud: Option<usize>,
}

impl FxMaterials {
    pub fn new(p: &MapParticles) -> Self {
        let f = |n: &str| p.find(n);
        Self {
            noise: f("particle/particle_noisesphere"),
            embers: [f("effects/fire_embers1"), f("effects/fire_embers2")],
            fire: f("effects/fire_cloud2"),
            cement: [f("effects/fleck_cement1"), f("effects/fleck_cement2")],
            glow: f("sprites/light_glow02_add_noz").or_else(|| f("particle/particle_glow_03")),
            cloud: f("particle/particle_smokegrenade1").or_else(|| f("particle/particle_smokegrenade")),
        }
    }
}

/// The blast's direction (spec 5.4): probes 100 units along each axis;
/// each hit pushes away from its surface by how close it is.
pub fn blast_direction(at: Vec3, world: &impl WorldTrace) -> Vec3 {
    let mut sum = Vec3::ZERO;
    for dir in [Vec3::X, Vec3::NEG_X, Vec3::Y, Vec3::NEG_Y, Vec3::Z, Vec3::NEG_Z] {
        if let Some((f, _)) = world.trace(at, at + dir * 100.0 * UNIT) {
            sum -= dir * (1.0 - f);
        }
    }
    sum.try_normalize().unwrap_or(Vec3::Y)
}

/// Light at a point as a colour (0-1), from the map's light field.
fn light_at(field: Option<&LightField>, p: Vec3) -> Vec3 {
    let Some(field) = field else { return Vec3::splat(0.6) };
    let probe = (field.0.0)(p);
    let dirs = [Vec3::X, Vec3::NEG_X, Vec3::Y, Vec3::NEG_Y, Vec3::Z, Vec3::NEG_Z];
    (dirs.iter().map(|d| probe.eval(*d)).sum::<Vec3>() / 6.0).min(Vec3::ONE)
}

fn rand_unit_cube(rng: &mut ParticleRng, s: f32) -> Vec3 {
    rng.vec3(-s, s)
}

/// The HE explosion's particles at `b` blowing along `a` (spec 5.4: core
/// smoke, embers, fireballs, sparks, flecks), lit by `light`.
pub fn explosion(
    rng: &mut ParticleRng,
    m: &FxMaterials,
    b: Vec3,
    a: Vec3,
    light: Vec3,
    world: &impl WorldTrace,
) -> Vec<ParticleGroup> {
    let mut out = Vec::new();
    let sigma = 1.0 - 0.15 * 2.0;
    // Smoke, embers and fireballs share the core's motion: speed decaying
    // to 1/10000 in 0.5 s but not below 32 units/s, roll slowing.
    let mut core = ParticleGroup::new(Motion {
        decay: Some((0.5, 32.0 * UNIT)),
        roll_decay: Some((8.0, 0.5)),
        ..default()
    });
    // Smoke takes the world's light colour at its luminance.
    let lum = (light.dot(Vec3::new(0.299, 0.587, 0.114)) * 255.0).clamp(1.0, 255.0);
    let tint = light / light.max_element().max(1e-3);
    let smoke_color = |rng: &mut ParticleRng| tint * rng.int((lum / 2.0) as i32, lum as i32) as f32 / 255.0;
    let fade = Fade::Power(2.0);
    if let Some(noise) = m.noise {
        for _ in 0..4 {
            let dir = (rand_unit_cube(rng, sigma) + a * rng.float(1.0, 6.0)).normalize_or(Vec3::Y);
            let speed = rng.float(1.0, 750.0) * 2.0 * sigma * dir.dot(a).abs();
            let color = smoke_color(rng);
            core.particles.push(Particle {
                velocity: dir * speed * UNIT,
                color,
                fade,
                size: Ramp::linear(72.0 * UNIT, 144.0 * UNIT),
                roll: (rng.int(0, 360) as f32).to_radians(),
                roll_speed: rng.float(-2.0, 2.0),
                ..Particle::new(b, rng.float(2.0, 3.0), noise, 0.0)
            });
        }
        for _ in 0..8 {
            let at = b + rand_unit_cube(rng, 16.0 * UNIT);
            let dir = (rand_unit_cube(rng, sigma) + a * rng.float(1.0, 6.0)).normalize_or(Vec3::Y);
            let speed = rng.float(1.0, 2000.0) * 2.0 * sigma * dir.dot(a).abs();
            let size = rng.int(32, 64) as f32;
            let color = smoke_color(rng);
            core.particles.push(Particle {
                velocity: dir * speed * UNIT,
                color,
                alpha: rng.float(128.0, 255.0) / 255.0,
                fade,
                size: Ramp::linear(size * UNIT, size * 2.0 * UNIT),
                roll_speed: rng.float(-8.0, 8.0),
                ..Particle::new(at, rng.float(0.5, 1.0), noise, 0.0)
            });
        }
        for i in 0..32 {
            let yaw = (11.25 * (i + 1) as f32).to_radians();
            let h = Vec3::new(yaw.cos(), 0.0, -yaw.sin());
            let at = b + rand_unit_cube(rng, 4.0 * UNIT) + h * rng.float(8.0, 16.0) * UNIT;
            let speed = rng.float(500.0, 2000.0) * 2.0 * sigma;
            let size = rng.int(16, 32) as f32;
            let color = smoke_color(rng);
            core.particles.push(Particle {
                velocity: h * speed * UNIT,
                color,
                alpha: rng.float(16.0, 32.0) / 255.0,
                fade,
                size: Ramp::linear(size * UNIT, size * 4.0 * UNIT),
                roll_speed: rng.float(-8.0, 8.0),
                ..Particle::new(at, rng.float(0.5, 1.5), noise, 0.0)
            });
        }
    }
    for _ in 0..16 {
        let pick = rng.int(0, 1) as usize;
        let Some(mat) = m.embers[pick].or(m.embers[1 - pick]) else {
            break;
        };
        let at = b + rand_unit_cube(rng, 32.0 * UNIT);
        let dir = (rand_unit_cube(rng, 1.4) + a).normalize_or(Vec3::Y);
        let k = sigma * dir.dot(a).abs();
        let speed = rng.float(1.0, 400.0) * 8.0 * k * k;
        let grey = rng.int(192, 255) as f32 / 255.0;
        let size = (rng.int(8, 16) as f32 * k).clamp(4.0, 32.0);
        core.particles.push(Particle {
            velocity: dir * speed * UNIT,
            color: Vec3::splat(grey),
            fade,
            roll_speed: rng.float(-8.0, 8.0),
            ..Particle::new(at, rng.float(2.0, 3.0), mat, size * UNIT)
        });
    }
    if let Some(fire) = m.fire {
        for _ in 0..32 {
            let at = b + rand_unit_cube(rng, 48.0 * UNIT);
            let dir = (rand_unit_cube(rng, 0.525) + a).normalize_or(Vec3::Y);
            let k = sigma * dir.dot(a).abs();
            let speed = rng.float(400.0, 800.0) * 8.0 * k * k;
            let grey = rng.int(128, 255) as f32 / 255.0;
            let size = (rng.int(32, 85) as f32 * k).clamp(32.0, 85.0);
            core.particles.push(Particle {
                velocity: dir * speed * UNIT,
                color: Vec3::splat(grey),
                fade,
                size: Ramp::linear(size * UNIT, size * 1.5 * UNIT),
                roll_speed: rng.float(-16.0, 16.0),
                ..Particle::new(at, rng.float(0.2, 0.4), fire, 0.0)
            });
        }
        // Debris: spark streaks.
        let mut sparks = ParticleGroup::new(Motion {
            gravity: 200.0 * UNIT,
            damping: 8.0,
            ..default()
        });
        for _ in 0..rng.int(8, 16) {
            let dir = (rand_unit_cube(rng, 1.0) + a).normalize_or(Vec3::Y);
            sparks.particles.push(Particle {
                velocity: dir * rng.float(1500.0, 2500.0) * UNIT,
                fade: Fade::Ramp(Ramp::linear(1.0, 0.0)),
                shape: Shape::Trail {
                    width: rng.float(2.0, 16.0) * UNIT,
                    length: rng.float(0.05, 0.1),
                },
                ..Particle::new(b, rng.float(0.1, 0.15), fire, 0.0)
            });
        }
        out.push(sparks);
    }
    // The flash: one glow, fading out in 0.1 s while growing 16 units/s
    // (its start size isn't in the spec: ours).
    if let Some(glow) = m.glow {
        core.particles.push(Particle {
            color: Vec3::new(1.0, 0.9, 0.7),
            fade: Fade::Ramp(Ramp::linear(1.0, 0.0)),
            size: Ramp::linear(96.0 * UNIT, (96.0 + 1.6) * UNIT),
            ..Particle::new(b, 0.1, glow, 0.0)
        });
    }
    out.push(core);
    // Cement flecks bouncing off the world.
    if m.cement.iter().any(Option::is_some) {
        let origin = b + a * 16.0 * UNIT;
        let planes = probe_planes(origin, None, 256.0 * UNIT, 800.0 * UNIT, world);
        let mut g = ParticleGroup::new(Motion {
            gravity: 800.0 * UNIT,
            bounce: Some(Bounce {
                keep: 0.3,
                land_normal_y: 0.5,
                land_speed: 48.0 * UNIT,
                planes,
            }),
            ..default()
        });
        for _ in 0..rng.int(16, 32) {
            let pick = rng.int(0, 1) as usize;
            let Some(mat) = m.cement[pick].or(m.cement[1 - pick]) else {
                break;
            };
            let at = origin + rand_unit_cube(rng, 8.0 * UNIT);
            let dir = (rand_unit_cube(rng, 1.0) + a).normalize_or(Vec3::Y);
            let size = rng.int(1, 3);
            let k = 0.8 * dir.dot(a).abs();
            let speed = rng.float(64.0, 256.0) * (4 - size) as f32 * 8.0 * k * k;
            let grey = (0.25 * rng.float(0.5, 1.5)).min(1.0);
            g.particles.push(Particle {
                velocity: dir * speed * UNIT,
                color: Vec3::splat(grey),
                fade: Fade::Tail(0.5),
                roll_speed: rng.float(0.0, 6.0),
                ..Particle::new(at, 3.0, mat, size as f32 * UNIT)
            });
        }
        out.push(g);
    }
    out
}

fn explosions(
    mut detonated: MessageReader<Detonated>,
    materials: Option<Res<ParticleMaterials>>,
    light: Option<Res<LightField>>,
    mut particles: ResMut<Particles>,
    mut rng: ResMut<FxRng>,
    world: WorldTracer,
) {
    let Some(materials) = materials else {
        detonated.clear();
        return;
    };
    let mats = FxMaterials::new(&materials.0);
    for d in detonated.read() {
        if d.kind != GrenadeKind::Blast {
            continue;
        }
        let a = blast_direction(d.at, &world);
        let l = light_at(light.as_deref(), d.at + a * 32.0 * UNIT);
        for g in explosion(&mut rng.0, &mats, d.at, a, l, &world) {
            particles.add(g);
        }
    }
}

// ---------------------------------------------------------------------------
// The smoke cloud (spec 7.3)

/// Grid offsets per axis, units.
const GRID: [f32; 4] = [-120.0, -40.0, 40.0, 120.0];
const SPRITE_SIZE: f32 = 80.0;

/// One sprite of a cloud: its grid slot, grey, roll, and a swap in
/// progress with another sprite (index, start, duration).
#[derive(Clone, Debug)]
struct CloudSprite {
    slot: usize,
    grey: f32,
    roll: f32,
    roll_speed: f32,
    swap: Option<(usize, f64, f32)>,
}

/// A cloud's sprites and each grid slot's light, made when it is first
/// drawn.
#[derive(Component, Clone, Debug)]
pub struct CloudSprites {
    sprites: Vec<CloudSprite>,
    light: Vec<Vec3>,
    last: f64,
}

fn slot_offset(slot: usize) -> Vec3 {
    Vec3::new(GRID[slot % 4], GRID[(slot / 4) % 4], GRID[slot / 16]) * UNIT
}

/// A sprite's alpha at distance `l` from the centre of a cloud of radius
/// `e` (spec 7.3), before the cloud's alpha: none beyond the edge, full in
/// the inner 70 %.
pub fn sprite_alpha(l: f32, e: f32) -> f32 {
    if e <= 0.0 || l > e {
        return 0.0;
    }
    let a = 1.0 - l / e;
    if a > 0.3 { 1.0 } else { a / 0.3 }
}

#[allow(clippy::type_complexity)]
fn draw_clouds(
    mut clouds: Query<(Entity, &SmokeCloud, Option<&mut CloudSprites>)>,
    materials: Option<Res<ParticleMaterials>>,
    light: Option<Res<LightField>>,
    mut particles: ResMut<Particles>,
    mut rng: ResMut<FxRng>,
    time: Res<Time>,
    mut commands: Commands,
) {
    let Some(materials) = materials else { return };
    let Some(mat) = FxMaterials::new(&materials.0).cloud else {
        return;
    };
    let now = time.elapsed_secs_f64();
    let rng = &mut rng.0;
    for (e, cloud, sprites) in &mut clouds {
        let Some(mut s) = sprites else {
            let light = (0..64)
                .map(|i| light_at(light.as_deref(), cloud.centre + slot_offset(i)))
                .collect();
            let sprites = (0..64)
                .map(|slot| CloudSprite {
                    slot,
                    grey: rng.int(0, 255) as f32 / 255.1,
                    roll: rng.float(-6.0, 6.0),
                    roll_speed: rng.float(-0.1, 0.1),
                    swap: None,
                })
                .collect();
            commands.entity(e).insert(CloudSprites {
                sprites,
                light,
                last: now,
            });
            continue;
        };
        let dt = (now - s.last).max(0.0) as f32;
        s.last = now;
        churn(&mut s, rng, now);
        let t = (now - cloud.started) as f32;
        let radius = cloud.radius(t);
        let alpha = cloud.alpha(t);
        let mut g = ParticleGroup::new(Motion {
            near_fade: Some((0.0, 10.0 * UNIT)),
            ..default()
        });
        g.capped = false;
        g.kill_after = Some(0.0);
        let slots: Vec<usize> = s.sprites.iter().map(|x| x.slot).collect();
        for i in 0..s.sprites.len() {
            let sp = s.sprites[i].clone();
            let (mut q, mut l) = (slot_offset(sp.slot), s.light[sp.slot]);
            if let Some((other, start, dur)) = sp.swap {
                let w = 0.5 + 0.5 * (std::f32::consts::PI * ((now - start) as f32 / dur).clamp(0.0, 1.0)).cos();
                q = q * w + slot_offset(slots[other]) * (1.0 - w);
                l = l * w + s.light[slots[other]] * (1.0 - w);
            }
            let dist = q.length();
            let a = sprite_alpha(dist, radius) * alpha;
            if a <= 0.0 {
                continue;
            }
            let pos = if dist <= radius / 2.0 {
                cloud.centre + q
            } else {
                cloud.centre + q * (radius / 2.0) / dist
            };
            let c = (0.5 + 0.1 * sp.grey) * l;
            s.sprites[i].roll += sp.roll_speed * dt;
            g.particles.push(Particle {
                color: (c + Vec3::splat(0.5)) / 2.0,
                alpha: a,
                roll: s.sprites[i].roll,
                ..Particle::new(pos, 1.0, mat, SPRITE_SIZE * UNIT)
            });
        }
        particles.add(g);
    }
}

/// Neighbouring sprites trade places over 10-20 s (spec 7.3, churn).
fn churn(s: &mut CloudSprites, rng: &mut ParticleRng, now: f64) {
    // Finished swaps exchange their slots.
    for i in 0..s.sprites.len() {
        if let Some((j, start, dur)) = s.sprites[i].swap
            && now - start >= dur as f64
            && i < j
        {
            let (a, b) = (s.sprites[i].slot, s.sprites[j].slot);
            s.sprites[i].slot = b;
            s.sprites[j].slot = a;
            s.sprites[i].swap = None;
            s.sprites[j].swap = None;
        }
    }
    let by_slot: Vec<usize> = {
        let mut v = vec![0; 64];
        for (i, sp) in s.sprites.iter().enumerate() {
            v[sp.slot] = i;
        }
        v
    };
    for i in 0..s.sprites.len() {
        if s.sprites[i].swap.is_some() {
            continue;
        }
        let slot = s.sprites[i].slot;
        let (x, y, z) = ((slot % 4) as i32, ((slot / 4) % 4) as i32, (slot / 16) as i32);
        let start = [rng.int(0, 2), rng.int(0, 2), rng.int(0, 2)];
        'search: for dx in 0..3 {
            for dy in 0..3 {
                for dz in 0..3 {
                    let o = [
                        (dx + start[0]) % 3 - 1,
                        (dy + start[1]) % 3 - 1,
                        (dz + start[2]) % 3 - 1,
                    ];
                    if o == [0, 0, 0] {
                        continue;
                    }
                    let (nx, ny, nz) = (x + o[0], y + o[1], z + o[2]);
                    if !(0..4).contains(&nx) || !(0..4).contains(&ny) || !(0..4).contains(&nz) {
                        continue;
                    }
                    let j = by_slot[(nx + ny * 4 + nz * 16) as usize];
                    if s.sprites[j].swap.is_some() {
                        continue;
                    }
                    let dur = rng.float(10.0, 20.0);
                    s.sprites[i].swap = Some((j, now, dur));
                    s.sprites[j].swap = Some((i, now, dur));
                    break 'search;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sprite_alphas_g27() {
        let e = 240.0;
        let corner = Vec3::splat(120.0).length();
        let edge = Vec3::new(120.0, 120.0, 40.0).length();
        let face = Vec3::new(120.0, 40.0, 40.0).length();
        let inner = Vec3::splat(40.0).length();
        assert!((sprite_alpha(corner, e) - 0.447).abs() < 1e-3);
        assert!((sprite_alpha(edge, e) - 0.912).abs() < 1e-3);
        assert_eq!(sprite_alpha(face, e), 1.0);
        assert_eq!(sprite_alpha(inner, e), 1.0);
    }

    #[test]
    fn flash_fit_is_stronger_near_and_facing() {
        let full = flash_model(100.0 * UNIT, 1.0).unwrap();
        assert_eq!(full.0, 1.0);
        let behind = flash_model(100.0 * UNIT, -1.0).unwrap();
        assert!(behind.1 + behind.2 < full.1 + full.2);
        let far = flash_model(1200.0 * UNIT, 1.0).unwrap();
        assert!(far.1 + far.2 < full.1 + full.2);
        assert!(flash_model(1600.0 * UNIT, 1.0).is_none());
    }

    /// A flash at a given distance and angle: how long the white and its
    /// after-image last (the blindness) and which ringing follows.
    #[test]
    fn flash_at_distance_and_angle_gives_after_image_and_ring_times() {
        let hearing = |d: f32, k: f32| {
            let (alpha, hold, fade) = flash_model(d * UNIT, k).unwrap();
            (hold + fade, flash_hearing(alpha, hold + fade).unwrap())
        };
        // Close, looking at it: 5 s of white and after-image, the long
        // muffle (1.6 s held, then a 1 s fade).
        let (t, h) = hearing(100.0, 1.0);
        assert!((t - 4.98).abs() < 0.01, "{t}");
        assert_eq!(h, FLASH_MUFFLE[0]);
        assert!((h.length() - 2.6).abs() < 1e-5);
        // Close, facing away: a quarter of the strength, the short one.
        let (t, h) = hearing(100.0, -1.0);
        assert!((t - 1.245).abs() < 0.01, "{t}");
        assert_eq!(h, FLASH_MUFFLE[2]);
        // Halfway out, looking at it: three quarters, the long one;
        // further, under 60 %: the medium one.
        let (t, h) = hearing(750.0, 1.0);
        assert!((t - 3.75).abs() < 0.01, "{t}");
        assert_eq!(h, FLASH_MUFFLE[0]);
        let (t, h) = hearing(1000.0, 1.0);
        assert!((t - 2.78).abs() < 0.01, "{t}");
        assert_eq!(h, FLASH_MUFFLE[1]);
    }

    #[test]
    fn he_hearing_by_damage_then_distance() {
        let h = he_blast().hearing.unwrap();
        assert_eq!(h.effect(30.0 * HP, 300.0 * UNIT), Some(SHOCK_MUFFLE));
        assert_eq!(h.effect(29.0 * HP, 200.0 * UNIT), Some(EXPLOSION_RING));
        assert_eq!(h.effect(20.0 * HP, 250.0 * UNIT), None);
        // The shock rings; the explosion ring only muffles (a low-pass).
        assert!(SHOCK_MUFFLE.ring_gain > 0.0 && EXPLOSION_RING.ring_gain == 0.0);
    }

    #[test]
    fn churn_swaps_neighbours_and_keeps_every_slot() {
        let mut s = CloudSprites {
            sprites: (0..64)
                .map(|slot| CloudSprite {
                    slot,
                    grey: 0.0,
                    roll: 0.0,
                    roll_speed: 0.0,
                    swap: None,
                })
                .collect(),
            light: vec![Vec3::ONE; 64],
            last: 0.0,
        };
        let mut rng = ParticleRng::new(3);
        churn(&mut s, &mut rng, 0.0);
        assert!(s.sprites.iter().filter(|x| x.swap.is_some()).count() > 32);
        churn(&mut s, &mut rng, 30.0);
        let mut slots: Vec<usize> = s.sprites.iter().map(|x| x.slot).collect();
        slots.sort();
        assert_eq!(slots, (0..64).collect::<Vec<_>>());
    }
}
