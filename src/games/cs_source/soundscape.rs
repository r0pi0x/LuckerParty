//! CS:S soundscapes (specs/cs_source/sounds.md 6): the soundscape scripts,
//! flattened (nested soundscapes, volumes and position overrides
//! resolved), and the map entities that select them: trigger_soundscape
//! boxes with their env_soundscape_triggerable, and env_soundscape points.

use std::collections::HashMap;

use bevy::prelude::*;
use vbsp::Bsp;

use super::{
    material::MaterialLoader,
    movement::to_engine,
    sound::{interval, named_level, wave_dry, wave_file},
    surfaceprops::tokens,
};
use crate::map::sound::{
    Interval, MapSounds, ScapeLoop, ScapePosition, ScapeRandom, Soundscape, SoundscapeEmitter, SoundscapeZone,
};

/// Nested soundscapes stop past this depth.
const MAX_DEPTH: u32 = 8;
const METERS_PER_UNIT: f32 = 0.0254;

/// A parsed script block: keys in order, each a value or a sub-block.
#[derive(Clone, Debug)]
enum Node {
    Value(String),
    Block(Vec<(String, Node)>),
}

fn parse_block(t: &[String], i: &mut usize) -> Vec<(String, Node)> {
    let mut out = Vec::new();
    while *i < t.len() && t[*i] != "}" {
        let key = t[*i].to_lowercase();
        *i += 1;
        if *i >= t.len() {
            break;
        }
        if t[*i] == "{" {
            *i += 1;
            let block = parse_block(t, i);
            *i += 1; // "}"
            out.push((key, Node::Block(block)));
        } else {
            out.push((key, Node::Value(t[*i].clone())));
            *i += 1;
        }
    }
    out
}

fn value<'a>(block: &'a [(String, Node)], key: &str) -> Option<&'a str> {
    block.iter().find_map(|(k, v)| match v {
        Node::Value(s) if k == key => Some(s.as_str()),
        _ => None,
    })
}

/// Level from an "attenuation" (a → 50 + 20/a dB; 0 → no falloff).
fn level_of_attenuation(a: f32) -> f32 {
    if a > 0.0 { 50.0 + 20.0 / a } else { 0.0 }
}

fn level_interval(block: &[(String, Node)], default: f32) -> Interval {
    if let Some(l) = value(block, "soundlevel") {
        return match named_level(l) {
            Some(db) => Interval::fixed(db),
            None => interval(l),
        };
    }
    if let Some(a) = value(block, "attenuation") {
        let a = interval(a);
        return Interval {
            start: level_of_attenuation(a.start),
            range: level_of_attenuation(a.start + a.range) - level_of_attenuation(a.start),
        };
    }
    Interval::fixed(default)
}

fn scaled(i: Interval, by: f32) -> Interval {
    Interval {
        start: i.start * by,
        range: i.range * by,
    }
}

/// Where nested sounds go: an offset for their indices, or overrides.
#[derive(Clone, Copy, Default)]
struct Placement {
    volume: f32,
    offset: usize,
    position_override: Option<usize>,
    ambient_override: Option<usize>,
}

struct Builder<'a, 'm> {
    scripts: &'a HashMap<String, Vec<(String, Node)>>,
    materials: &'a mut MaterialLoader<'m>,
    sounds: &'a mut MapSounds,
    decoded: HashMap<String, Option<usize>>,
}

impl Builder<'_, '_> {
    fn clip(&mut self, wave: &str) -> Option<usize> {
        let file = wave_file(wave).to_lowercase();
        if let Some(c) = self.decoded.get(&file) {
            return *c;
        }
        let clip = self
            .materials
            .read(&file)
            .and_then(|b| super::wav::decode(&b).ok())
            .map(|c| {
                self.sounds.clips.push(c);
                self.sounds.clips.len() - 1
            });
        self.decoded.insert(file, clip);
        clip
    }

    fn position(&self, block: &[(String, Node)], at: Placement) -> ScapePosition {
        if let Some(p) = at.position_override {
            return ScapePosition::Index(p);
        }
        match value(block, "position") {
            Some(p) if p.eq_ignore_ascii_case("random") => ScapePosition::Random,
            Some(p) => ScapePosition::Index(p.trim().parse::<usize>().unwrap_or(0) + at.offset),
            None => at.ambient_override.map_or(ScapePosition::Ambient, ScapePosition::Index),
        }
    }

    fn add(&mut self, name: &str, at: Placement, depth: u32, out: &mut Soundscape) {
        if depth > MAX_DEPTH {
            return;
        }
        let Some(block) = self.scripts.get(&name.to_lowercase()) else {
            return;
        };
        let block = block.clone();
        for (key, node) in &block {
            // "dsp" sets the room preset, at the top level only.
            if let (0, "dsp", Node::Value(v)) = (depth, key.as_str(), node) {
                out.dsp = Some(interval(v).start.max(0.0) as u16);
            }
            let Node::Block(b) = node else { continue };
            match key.as_str() {
                "playlooping" => {
                    let volume = scaled(value(b, "volume").map_or(Interval::fixed(0.0), interval), at.volume);
                    let wave = value(b, "wave");
                    let Some(clip) = wave.and_then(|w| self.clip(w)) else {
                        continue;
                    };
                    if volume.start <= 0.0 && volume.range <= 0.0 {
                        continue;
                    }
                    let position = match self.position(b, at) {
                        ScapePosition::Index(n) => Some(n),
                        _ => None,
                    };
                    out.loops.push(ScapeLoop {
                        clip,
                        volume,
                        pitch: value(b, "pitch").map_or(Interval::fixed(100.0), interval),
                        level: level_interval(b, 75.0).start,
                        position,
                        dry: wave.is_some_and(wave_dry),
                    });
                }
                "playrandom" => {
                    let waves: Vec<String> = b
                        .iter()
                        .filter_map(|(k, v)| match (k.as_str(), v) {
                            ("rndwave", Node::Block(w)) => Some(w),
                            _ => None,
                        })
                        .flat_map(|w| {
                            w.iter().filter_map(|(k, v)| match (k.as_str(), v) {
                                ("wave", Node::Value(s)) => Some(s.clone()),
                                _ => None,
                            })
                        })
                        .collect();
                    let clips: Vec<usize> = waves.iter().filter_map(|w| self.clip(w)).collect();
                    out.randoms.push(ScapeRandom {
                        clips,
                        time: value(b, "time").map_or(Interval::fixed(10.0), interval),
                        volume: scaled(value(b, "volume").map_or(Interval::fixed(0.0), interval), at.volume),
                        pitch: value(b, "pitch").map_or(Interval::fixed(0.0), interval),
                        level: level_interval(b, 0.0),
                        position: self.position(b, at),
                        dry: !waves.is_empty() && waves.iter().all(|w| wave_dry(w)),
                    });
                }
                "playsoundscape" => {
                    let Some(inner) = value(b, "name") else { continue };
                    let num = |k: &str| value(b, k).and_then(|v| v.trim().parse::<usize>().ok());
                    let nested = Placement {
                        volume: at.volume * value(b, "volume").map_or(1.0, |v| interval(v).start),
                        offset: at.offset + num("position").unwrap_or(0),
                        position_override: at.position_override.or(num("positionoverride")),
                        ambient_override: at.ambient_override.or(num("ambientpositionoverride")),
                    };
                    self.add(inner, nested, depth + 1, out);
                }
                _ => {}
            }
        }
    }
}

/// Add the map's soundscapes, their clips and selectors to `sounds`.
pub fn load(materials: &mut MaterialLoader, bsp: &Bsp, map: &str, sounds: &mut MapSounds) {
    let mut files: Vec<String> = Vec::new();
    if let Some(manifest) = materials.read("scripts/soundscapes_manifest.txt") {
        let t = tokens(&String::from_utf8_lossy(&manifest));
        files.extend(
            t.windows(2)
                .filter(|w| w[0].eq_ignore_ascii_case("file"))
                .map(|w| w[1].to_lowercase()),
        );
    }
    let own = format!("scripts/soundscapes_{}.txt", map.to_lowercase());
    if !files.contains(&own) {
        files.push(own);
    }
    // Later definitions win.
    let mut scripts: HashMap<String, Vec<(String, Node)>> = HashMap::new();
    for f in files {
        let Some(bytes) = materials.read(&f) else { continue };
        let t = tokens(&String::from_utf8_lossy(&bytes));
        let mut i = 0;
        while i + 1 < t.len() {
            if t[i + 1] != "{" {
                i += 1;
                continue;
            }
            let name = t[i].to_lowercase();
            i += 2;
            let block = parse_block(&t, &mut i);
            i += 1;
            scripts.insert(name, block);
        }
    }

    // Entities: named positions, then the selectors.
    let entities: Vec<vbsp::RawEntity> = bsp.entities.iter().collect();
    let mut named: HashMap<String, Vec3> = HashMap::new();
    for ent in &entities {
        if let (Some(name), Some(origin)) = (ent.prop("targetname"), ent.prop("origin").and_then(vector)) {
            named.entry(name.to_lowercase()).or_insert(to_engine(origin));
        }
    }
    let positions = |ent: &vbsp::RawEntity<'_>| -> Vec<Option<Vec3>> {
        const KEYS: [&str; 8] = [
            "position0",
            "position1",
            "position2",
            "position3",
            "position4",
            "position5",
            "position6",
            "position7",
        ];
        KEYS.iter()
            .map(|k| ent.prop(k).and_then(|n| named.get(&n.to_lowercase()).copied()))
            .collect()
    };
    let mut builder = Builder {
        scripts: &scripts,
        materials,
        sounds,
        decoded: HashMap::new(),
    };
    let mut index: HashMap<String, usize> = HashMap::new();
    let mut scape = |b: &mut Builder<'_, '_>, name: &str| -> Option<usize> {
        let key = name.to_lowercase();
        if let Some(&i) = index.get(&key) {
            return Some(i);
        }
        b.scripts.get(&key)?;
        let mut s = Soundscape {
            name: name.to_string(),
            ..default()
        };
        b.add(
            name,
            Placement {
                volume: 1.0,
                ..default()
            },
            0,
            &mut s,
        );
        b.sounds.soundscapes.push(s);
        let i = b.sounds.soundscapes.len() - 1;
        index.insert(key, i);
        Some(i)
    };
    let triggerables: HashMap<String, &vbsp::RawEntity<'_>> = entities
        .iter()
        .filter(|e| e.prop("classname") == Some("env_soundscape_triggerable"))
        .filter_map(|e| Some((e.prop("targetname")?.to_lowercase(), e)))
        .collect();
    let mut zones = Vec::new();
    let mut emitters = Vec::new();
    for (index, ent) in entities.iter().enumerate() {
        match ent.prop("classname") {
            Some("trigger_soundscape") => {
                let Some(target) = ent.prop("soundscape").and_then(|t| triggerables.get(&t.to_lowercase())) else {
                    continue;
                };
                let Some(model) = ent
                    .prop("model")
                    .and_then(|m| m.strip_prefix('*'))
                    .and_then(|m| m.parse::<usize>().ok())
                    .and_then(|m| bsp.models.get(m))
                else {
                    continue;
                };
                let Some(s) = target.prop("soundscape").and_then(|n| scape(&mut builder, n)) else {
                    continue;
                };
                // Brush entity models are stored relative to the origin.
                let origin = ent.prop("origin").and_then(vector).unwrap_or(Vec3::ZERO);
                let add = |v: vbsp::Vector| to_engine(Vec3::new(v.x, v.y, v.z) + origin);
                let (a, b) = (add(model.mins), add(model.maxs));
                zones.push(SoundscapeZone {
                    min: a.min(b),
                    max: a.max(b),
                    scape: s,
                    positions: positions(target),
                    entity: Some(index),
                });
            }
            Some("env_soundscape") if ent.prop("StartDisabled") != Some("1") => {
                let (Some(at), Some(s)) = (
                    ent.prop("origin").and_then(vector),
                    ent.prop("soundscape").and_then(|n| scape(&mut builder, n)),
                ) else {
                    continue;
                };
                let radius = ent
                    .prop("radius")
                    .and_then(|r| r.trim().parse::<f32>().ok())
                    .unwrap_or(-1.0);
                emitters.push(SoundscapeEmitter {
                    at: to_engine(at),
                    radius: (radius >= 0.0).then_some(radius * METERS_PER_UNIT),
                    scape: s,
                    positions: positions(ent),
                });
            }
            _ => {}
        }
    }
    builder.sounds.soundscape_zones = zones;
    builder.sounds.soundscape_emitters = emitters;
}

/// A "x y z" keyvalue, Source units.
fn vector(s: &str) -> Option<Vec3> {
    let mut it = s.split_whitespace().map(|p| p.parse::<f32>());
    Some(Vec3::new(it.next()?.ok()?, it.next()?.ok()?, it.next()?.ok()?))
}
