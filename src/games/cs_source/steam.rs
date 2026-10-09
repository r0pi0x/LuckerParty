//! Steam jets (env_steam, env_steamjet) from specs/source/visual_entities.md
//! 7: the entity's keys become a `MapSteam` (direction, speeds, sizes,
//! rate, colour sampled from the map's light along the jet). The map
//! layer emits and draws the puffs (`map::steam`); the logic turns them on
//! and off.

use bevy::prelude::*;
use vbsp::Bsp;

use super::{bsp::METERS_PER_UNIT, material::MaterialLoader};
use crate::map::{
    MapData,
    steam::{MapSteam, light_ramp},
};

/// The puff material of normal jets (type 0).
pub const STEAM_MATERIAL: &str = "particle/particle_smokegrenade";

/// Entity keys, ignoring case.
fn key<'a>(e: &vbsp::RawEntity<'a>, name: &str) -> Option<&'a str> {
    e.properties()
        .find(|(k, _)| k.eq_ignore_ascii_case(name))
        .map(|(_, v)| v)
}

fn numbers(s: &str) -> Vec<f32> {
    s.split_whitespace().filter_map(|v| v.parse::<f32>().ok()).collect()
}

/// Source forward, right and up of (pitch, yaw, roll) degrees, in Source
/// axes (x forward, y left, z up).
pub fn angle_vectors(pitch: f32, yaw: f32, roll: f32) -> [Vec3; 3] {
    let (sp, cp) = pitch.to_radians().sin_cos();
    let (sy, cy) = yaw.to_radians().sin_cos();
    let (sr, cr) = roll.to_radians().sin_cos();
    let forward = Vec3::new(cp * cy, cp * sy, -sp);
    let right = Vec3::new(-sr * sp * cy + cr * sy, -sr * sp * sy - cr * cy, -sr * cp);
    let up = Vec3::new(cr * sp * cy + sr * sy, cr * sp * sy - sr * cy, cr * cp);
    [forward, right, up]
}

/// Source axes to engine axes (a direction).
fn dir(v: Vec3) -> Vec3 {
    Vec3::new(v.x, v.z, -v.y).normalize_or_zero()
}

pub fn add_steam(bsp: &Bsp, materials: &mut MaterialLoader, data: &mut MapData) {
    let mut texture = None;
    for (index, e) in bsp.entities.iter().enumerate() {
        let class = e.prop("classname").unwrap_or("");
        let jet = class.eq_ignore_ascii_case("env_steamjet");
        if !(jet || class.eq_ignore_ascii_case("env_steam")) {
            continue;
        }
        // Missing keys: the server sends 0 (no puffs for speed or rate 0),
        // RollSpeed 8.
        let num = |k: &str, d: f32| key(&e, k).and_then(|v| v.trim().parse::<f32>().ok()).unwrap_or(d);
        if num("Type", 0.0) as i32 == 1 {
            // Heat-wave jets refract the scene: not drawn
            // (docs/tech-debt.md).
            continue;
        }
        let Some(origin) = key(&e, "origin").map(numbers).filter(|o| o.len() == 3) else {
            continue;
        };
        let origin = Vec3::new(origin[0], origin[1], origin[2]);
        let angles = key(&e, "angles")
            .map(numbers)
            .filter(|a| a.len() == 3)
            .unwrap_or(vec![0.0; 3]);
        let [forward, right, up] = angle_vectors(angles[0], angles[1], angles[2]);
        // env_steamjet points along its -right.
        let forward = if jet { -right } else { forward };
        let rgb = key(&e, "rendercolor").map(numbers).unwrap_or_default();
        let color = Vec3::new(
            rgb.first().copied().unwrap_or(255.0),
            rgb.get(1).copied().unwrap_or(255.0),
            rgb.get(2).copied().unwrap_or(255.0),
        ) / 255.0;
        let speed = num("Speed", 0.0);
        let length = num("JetLength", 0.0);
        let life = if speed > 0.0 { length / speed } else { 0.0 };
        let at = |p: Vec3| Vec3::new(p.x, p.z, -p.y) * METERS_PER_UNIT;
        let (o, end) = (at(origin), at(origin + forward * speed * life));
        let emissive = num("spawnflags", 0.0) as i32 & 1 != 0;
        let scale = data.look.light_scale;
        let ramp = match &data.light_field {
            Some(field) => light_ramp(
                |p| {
                    let probe = (field.0)(p);
                    let ambient = probe.cube.iter().copied().sum::<Vec3>() / 6.0;
                    (ambient + probe.lights.iter().map(|l| l.1).sum::<Vec3>()) * scale
                },
                o,
                end,
                color,
                emissive,
            ),
            None => light_ramp(|_| Vec3::ONE, o, end, color, emissive),
        };
        if texture.is_none() {
            texture = materials.resolve(STEAM_MATERIAL).texture;
        }
        // Normal jets' sizes are whole units, 0-255.
        let size = |k: &str| num(k, 0.0).trunc().clamp(0.0, 255.0) * METERS_PER_UNIT;
        data.steam.push(MapSteam {
            origin: o,
            forward: dir(forward),
            right: dir(right),
            up: dir(up),
            spread: num("SpreadSpeed", 0.0) * METERS_PER_UNIT,
            speed: speed * METERS_PER_UNIT,
            start_size: size("StartSize"),
            end_size: size("EndSize"),
            rate: num("Rate", 0.0),
            length: length * METERS_PER_UNIT,
            // Degrees, like the spawn roll (spec open question 7).
            roll_speed: num("RollSpeed", 8.0).to_radians(),
            alpha: num("renderamt", 255.0).clamp(0.0, 255.0) / 255.0,
            ramp,
            texture,
            entity: Some(index),
            start_on: num("InitialState", 0.0) as i32 == 1,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn source_angle_vectors() {
        let close = |a: Vec3, b: Vec3| (a - b).length() < 1e-5;
        // Yaw 0: forward +x, right -y, up +z.
        let [f, r, u] = angle_vectors(0.0, 0.0, 0.0);
        assert!(close(f, Vec3::X) && close(r, -Vec3::Y) && close(u, Vec3::Z));
        // Pitch -90 looks up (+z).
        let [f, ..] = angle_vectors(-90.0, 0.0, 0.0);
        assert!(close(f, Vec3::Z));
        // Yaw 90: forward +y, right +x.
        let [f, r, _] = angle_vectors(0.0, 90.0, 0.0);
        assert!(close(f, Vec3::Y) && close(r, Vec3::X));
    }
}
