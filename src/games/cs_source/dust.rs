//! Dust motes (`func_dustmotes`) from specs/cs_source/sprites_dust.md: the
//! entity keys and its brush model's bounds become a `MapDust` volume.

use bevy::prelude::*;
use vbsp::Bsp;

use super::{
    bsp::{METERS_PER_UNIT, to_engine},
    material::MaterialLoader,
};
use crate::map::{MapData, MapDust};

/// Entity keys, ignoring case (keys may come back lower-cased).
fn key<'a>(e: &vbsp::RawEntity<'a>, name: &str) -> Option<&'a str> {
    e.properties()
        .find(|(k, _)| k.eq_ignore_ascii_case(name))
        .map(|(_, v)| v)
}

pub fn add_dust(bsp: &Bsp, materials: &mut MaterialLoader, data: &mut MapData) {
    let models: Vec<_> = bsp.models().collect();
    let mut texture = None;
    for e in bsp
        .entities
        .iter()
        .filter(|e| e.prop("classname") == Some("func_dustmotes"))
    {
        if key(&e, "StartDisabled").is_some_and(|v| v.trim() == "1") {
            continue;
        }
        let Some(model) = e
            .prop("model")
            .and_then(|m| m.strip_prefix('*'))
            .and_then(|i| i.parse::<usize>().ok())
            .and_then(|i| models.get(i))
        else {
            continue;
        };
        let num = |k: &str, d: f32| key(&e, k).and_then(|v| v.trim().parse::<f32>().ok()).unwrap_or(d);
        // No Alpha key: the motes are invisible.
        let Some(alpha) = key(&e, "Alpha").and_then(|v| v.trim().parse::<f32>().ok()) else {
            continue;
        };
        let rgb: Vec<f32> = e
            .prop("Color")
            .unwrap_or("255 255 255")
            .split_whitespace()
            .filter_map(|v| v.parse::<f32>().ok())
            .collect();
        if texture.is_none() {
            texture = materials.resolve("particle/sparkles").texture;
        }
        let (a, b) = (to_engine(model.mins), to_engine(model.maxs));
        data.dust.push(MapDust {
            min: a.min(b),
            max: a.max(b),
            rate: num("SpawnRate", 40.0),
            size: (num("SizeMin", 10.0), num("SizeMax", 20.0)),
            speed: num("SpeedMax", 10.0) * METERS_PER_UNIT,
            // Lifetimes are sent in 4 bits: 0-15 s.
            life: (
                num("LifetimeMin", 5.0).clamp(0.0, 15.0),
                num("LifetimeMax", 10.0).clamp(0.0, 15.0),
            ),
            fade_distance: num("DistMax", 1024.0) * METERS_PER_UNIT,
            color: [
                rgb.first().copied().unwrap_or(255.0) / 255.0,
                rgb.get(1).copied().unwrap_or(255.0) / 255.0,
                rgb.get(2).copied().unwrap_or(255.0) / 255.0,
                alpha / 255.0,
            ],
            texture,
        });
    }
}
