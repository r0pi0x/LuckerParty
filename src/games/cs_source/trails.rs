//! Sprite trails (env_spritetrail, specs/source/visual_entities.md 5),
//! smoke stacks (env_smokestack, specs/source/particles_and_smoke.md 3)
//! and teslas (point_tesla, public entity docs) from the map's entities
//! into `map::emitters`. Their materials join the
//! map's particle materials (the pool draws them): a trail in an additive
//! render mode gets an additive copy of its sprite.

use bevy::prelude::*;
use vbsp::Bsp;

use super::{bsp::METERS_PER_UNIT, material::MaterialLoader};
use crate::map::{
    MapData,
    emitters::{MapSmokeStack, MapTrail},
    particles::{ParticleBlend, ParticleMaterial},
};

/// env_smokestack's material when "SmokeMaterial" is absent.
pub const SMOKESTACK_MATERIAL: &str = "particle/SmokeStack";

/// Entity keys, ignoring case.
fn key<'a>(e: &vbsp::RawEntity<'a>, name: &str) -> Option<&'a str> {
    e.properties()
        .find(|(k, _)| k.eq_ignore_ascii_case(name))
        .map(|(_, v)| v)
}

fn numbers(s: &str) -> Vec<f32> {
    s.split_whitespace().filter_map(|v| v.parse::<f32>().ok()).collect()
}

fn vec3(s: Option<&str>) -> Option<Vec3> {
    let n = numbers(s?);
    (n.len() >= 3).then(|| Vec3::new(n[0], n[1], n[2]))
}

/// A material path as a key (no `materials/`, no extension, `/`).
fn material_name(path: &str) -> String {
    let p = path.to_lowercase().replace('\\', "/");
    p.trim_start_matches("materials/")
        .trim_end_matches(".vmt")
        .trim_end_matches(".spr")
        .to_string()
}

/// The index of particle material `name` (loaded on first use), as
/// `blend` when given (a render mode overriding the material's own).
pub fn particle_material(
    materials: &mut MaterialLoader,
    data: &mut MapData,
    name: &str,
    blend: Option<ParticleBlend>,
) -> Option<usize> {
    let name = material_name(name);
    let key = match blend {
        Some(ParticleBlend::Additive) => format!("{name}#add"),
        Some(ParticleBlend::Alpha) => format!("{name}#blend"),
        None => name.clone(),
    };
    if let Some(i) = data.particles.find(&key) {
        return Some(i);
    }
    let mut m: ParticleMaterial = materials.particle(&name)?;
    m.name = key;
    if let Some(b) = blend {
        m.blend = b;
        // The entity's colour and alpha always apply.
        m.vertex_color = true;
        m.vertex_alpha = true;
    }
    data.particles.materials.push(m);
    Some(data.particles.materials.len() - 1)
}

/// The integer a value starts with ("12.5" is 12), as the game reads
/// integer keys.
fn leading_int(v: &str) -> i32 {
    let v = v.trim();
    let end = v
        .char_indices()
        .find(|(i, c)| !(c.is_ascii_digit() || (*i == 0 && (*c == '-' || *c == '+'))))
        .map_or(v.len(), |(i, _)| i);
    v[..end].parse().unwrap_or(0)
}

/// Engine space of a Source position.
fn engine(v: Vec3) -> Vec3 {
    Vec3::new(v.x, v.z, -v.y) * METERS_PER_UNIT
}

pub fn add_trails_and_stacks(bsp: &Bsp, materials: &mut MaterialLoader, data: &mut MapData) {
    for (index, e) in bsp.entities.iter().enumerate() {
        let class = e.prop("classname").unwrap_or("");
        let num = |k: &str| key(&e, k).and_then(|v| v.trim().parse::<f32>().ok());
        let origin = vec3(key(&e, "origin")).unwrap_or(Vec3::ZERO);
        let color = vec3(key(&e, "rendercolor")).unwrap_or(Vec3::splat(255.0)) / 255.0;
        let alpha = num("renderamt").unwrap_or(255.0).clamp(0.0, 255.0) / 255.0;
        if class.eq_ignore_ascii_case("env_spritetrail") {
            let Some(sprite) = key(&e, "spritename") else { continue };
            // Additive render modes add; the others blend (Hammer's
            // default is 5, additive).
            let mode = num("rendermode").unwrap_or(5.0) as i32;
            let blend = if matches!(mode, 3 | 5 | 9) {
                ParticleBlend::Additive
            } else {
                ParticleBlend::Alpha
            };
            let Some(material) = particle_material(materials, data, sprite, Some(blend)) else {
                continue;
            };
            let end = num("endwidth").filter(|w| *w >= 0.0);
            data.trails.push(MapTrail {
                entity: Some(index),
                position: engine(origin),
                material,
                life: num("lifetime").unwrap_or(0.0).max(0.0),
                start_width: num("startwidth").unwrap_or(0.0).max(0.0) * METERS_PER_UNIT,
                end_width: end.map(|w| w * METERS_PER_UNIT),
                color,
                alpha,
            });
        } else if class.eq_ignore_ascii_case("env_screenoverlay") {
            for n in 1..=10 {
                let Some(name) = key(&e, &format!("OverlayName{n}")).map(|v| v.trim().to_ascii_lowercase()) else {
                    continue;
                };
                if name.is_empty() || data.screen_overlays.iter().any(|(o, _)| *o == name) {
                    continue;
                }
                if let Some(t) = materials.resolve(&material_name(&name)).texture {
                    data.screen_overlays.push((name, t));
                }
            }
        } else if class.eq_ignore_ascii_case("point_tesla") {
            let sprite = key(&e, "texture").unwrap_or("sprites/physbeam");
            let Some(material) = particle_material(materials, data, sprite, Some(ParticleBlend::Additive)) else {
                continue;
            };
            let range = |a: &str, b: &str| (num(a).unwrap_or(0.0), num(b).unwrap_or(0.0));
            let (w0, w1) = range("thick_min", "thick_max");
            let (b0, b1) = range("beamcount_min", "beamcount_max");
            data.teslas.push(crate::map::emitters::MapTesla {
                entity: Some(index),
                position: engine(origin),
                material,
                color: vec3(key(&e, "m_Color")).unwrap_or(Vec3::splat(255.0)) / 255.0,
                radius: num("m_flRadius").unwrap_or(200.0) * METERS_PER_UNIT,
                width: (w0 * METERS_PER_UNIT, w1 * METERS_PER_UNIT),
                life: range("lifetime_min", "lifetime_max"),
                beams: (b0.max(0.0) as u32, b1.max(0.0) as u32),
            });
        } else if class.eq_ignore_ascii_case("env_smokestack") {
            let name = key(&e, "SmokeMaterial").unwrap_or(SMOKESTACK_MATERIAL);
            let Some(material) = particle_material(materials, data, name, None) else {
                continue;
            };
            // Wind: WindAngle and WindSpeed as integers.
            let int = |k: &str| key(&e, k).map_or(0.0, |v| leading_int(v) as f32);
            let (angle, speed) = (int("WindAngle").to_radians(), int("WindSpeed"));
            let wind = vec3(key(&e, "Wind")).unwrap_or(Vec3::new(angle.cos() * speed, angle.sin() * speed, 0.0));
            let angles = vec3(key(&e, "angles")).unwrap_or(Vec3::ZERO);
            // The colour is rendercolor (no env_particlelight on the
            // maps seen), scaled so its largest channel is at most 1.
            data.smokestacks.push(MapSmokeStack {
                entity: Some(index),
                position: engine(origin),
                rotation: crate::map::entities::rotation_to_engine(crate::map::entities::entity_rotation(angles)),
                material,
                base_spread: num("BaseSpread").unwrap_or(0.0) * METERS_PER_UNIT,
                spread_speed: num("SpreadSpeed").unwrap_or(0.0) * METERS_PER_UNIT,
                speed: num("Speed").unwrap_or(0.0) * METERS_PER_UNIT,
                start_size: num("StartSize").unwrap_or(0.0) * METERS_PER_UNIT,
                end_size: num("EndSize").unwrap_or(0.0) * METERS_PER_UNIT,
                rate: num("Rate").unwrap_or(0.0).max(0.0),
                length: num("JetLength").unwrap_or(0.0) * METERS_PER_UNIT,
                wind: engine(wind),
                twist: num("Twist").unwrap_or(0.0),
                roll: num("Roll").unwrap_or(0.0),
                color,
                alpha,
                start_on: num("InitialState").unwrap_or(0.0) as i32 != 0,
            });
        }
    }
}
