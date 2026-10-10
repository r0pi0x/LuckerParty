//! Particle effects (`.pcf`) for info_particle_system
//! (specs/source/particles_and_smoke.md 1-2): the manifests (the install's
//! `particles/particles_manifest.txt`, then the map's own from its pak or
//! the install: map files replace systems of the same name), the files
//! they list (Valve's DMX, binary encoding 2: the public "DMX/Binary
//! format" description), and the systems the map's info_particle_systems
//! name with their children, into `map::psys`. Files are read at load
//! time from the user's install and the map; nothing is copied.

use std::collections::HashMap;

use bevy::prelude::*;
use vbsp::Bsp;

use super::{bsp::METERS_PER_UNIT, material::MaterialLoader, trails::particle_material};
use crate::map::{
    MapData,
    psys::{Emitter, Init, MapParticleSystems, Op, Pick, PlacedSystem, Render, SystemDef},
};

/// Entries of a map manifest past this are ignored.
pub const MAP_MANIFEST_MAX: usize = 64;

/// A DMX attribute value.
#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    Element(Option<usize>),
    Int(i32),
    Float(f32),
    Bool(bool),
    Str(String),
    Binary(Vec<u8>),
    Time(f32),
    Color([u8; 4]),
    Vec2([f32; 2]),
    Vec3([f32; 3]),
    Vec4([f32; 4]),
    Matrix,
    Array(Vec<Value>),
}

/// A DMX element: its type, name and attributes.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Element {
    pub kind: String,
    pub name: String,
    pub attributes: Vec<(String, Value)>,
}

impl Element {
    /// An attribute by name: exactly, else ignoring case and spaces
    /// against underscores (files of different tool versions spell
    /// `fade out time min` and `fade_out_time_min`).
    pub fn get(&self, key: &str) -> Option<&Value> {
        let norm = |s: &str| s.to_ascii_lowercase().replace(' ', "_");
        self.attributes
            .iter()
            .find(|(k, _)| k == key)
            .or_else(|| {
                let want = norm(key);
                self.attributes.iter().find(|(k, _)| norm(k) == want)
            })
            .map(|(_, v)| v)
    }

    pub fn float(&self, key: &str, default: f32) -> f32 {
        match self.get(key) {
            Some(Value::Float(f)) | Some(Value::Time(f)) => *f,
            Some(Value::Int(i)) => *i as f32,
            Some(Value::Bool(b)) => f32::from(u8::from(*b)),
            _ => default,
        }
    }

    pub fn int(&self, key: &str, default: i32) -> i32 {
        match self.get(key) {
            Some(Value::Int(i)) => *i,
            Some(Value::Float(f)) => *f as i32,
            Some(Value::Bool(b)) => i32::from(*b),
            _ => default,
        }
    }

    pub fn bool(&self, key: &str) -> bool {
        match self.get(key) {
            Some(Value::Bool(b)) => *b,
            Some(Value::Int(i)) => *i != 0,
            _ => false,
        }
    }

    pub fn vec3(&self, key: &str, default: Vec3) -> Vec3 {
        match self.get(key) {
            Some(Value::Vec3(v)) => Vec3::from_array(*v),
            _ => default,
        }
    }

    /// A colour as 0-1 rgb and alpha.
    pub fn color(&self, key: &str) -> Option<(Vec3, f32)> {
        match self.get(key) {
            Some(Value::Color(c)) => Some((
                Vec3::new(c[0] as f32, c[1] as f32, c[2] as f32) / 255.0,
                c[3] as f32 / 255.0,
            )),
            _ => None,
        }
    }

    pub fn str(&self, key: &str) -> Option<&str> {
        match self.get(key) {
            Some(Value::Str(s)) => Some(s),
            _ => None,
        }
    }

    pub fn elements(&self, key: &str) -> Vec<usize> {
        match self.get(key) {
            Some(Value::Array(v)) => v
                .iter()
                .filter_map(|e| match e {
                    Value::Element(Some(i)) => Some(*i),
                    _ => None,
                })
                .collect(),
            _ => Vec::new(),
        }
    }

    pub fn element(&self, key: &str) -> Option<usize> {
        match self.get(key) {
            Some(Value::Element(i)) => *i,
            _ => None,
        }
    }
}

struct Reader<'a> {
    b: &'a [u8],
    at: usize,
}

impl Reader<'_> {
    fn take(&mut self, n: usize) -> Result<&[u8], String> {
        let s = self.b.get(self.at..self.at + n).ok_or("truncated")?;
        self.at += n;
        Ok(s)
    }
    fn u8(&mut self) -> Result<u8, String> {
        Ok(self.take(1)?[0])
    }
    fn i16(&mut self) -> Result<i16, String> {
        Ok(i16::from_le_bytes(self.take(2)?.try_into().unwrap()))
    }
    fn i32(&mut self) -> Result<i32, String> {
        Ok(i32::from_le_bytes(self.take(4)?.try_into().unwrap()))
    }
    fn f32(&mut self) -> Result<f32, String> {
        Ok(f32::from_le_bytes(self.take(4)?.try_into().unwrap()))
    }
    fn cstr(&mut self) -> Result<String, String> {
        let end = self.b[self.at..]
            .iter()
            .position(|c| *c == 0)
            .ok_or("unterminated string")?;
        let s = String::from_utf8_lossy(&self.b[self.at..self.at + end]).into_owned();
        self.at += end + 1;
        Ok(s)
    }
    fn floats<const N: usize>(&mut self) -> Result<[f32; N], String> {
        let mut out = [0.0; N];
        for v in &mut out {
            *v = self.f32()?;
        }
        Ok(out)
    }
}

/// Read a binary DMX file (encoding 2): its elements in file order.
pub fn read_dmx(bytes: &[u8]) -> Result<Vec<Element>, String> {
    let mut r = Reader { b: bytes, at: 0 };
    let header = r.cstr()?;
    if !header.contains("binary 2") {
        return Err(format!("unsupported DMX encoding: {}", header.trim()));
    }
    let n = r.i16()?.max(0) as usize;
    let mut strings = Vec::with_capacity(n);
    for _ in 0..n {
        strings.push(r.cstr()?);
    }
    let string = |i: i16| {
        strings
            .get(i as usize)
            .cloned()
            .ok_or_else(|| format!("bad string index {i}"))
    };
    let count = r.i32()?.max(0) as usize;
    if count > 1 << 20 {
        return Err("too many elements".into());
    }
    let mut elements = Vec::with_capacity(count);
    for _ in 0..count {
        let kind = string(r.i16()?)?;
        let name = r.cstr()?;
        r.take(16)?;
        elements.push(Element {
            kind,
            name,
            attributes: Vec::new(),
        });
    }
    fn value(r: &mut Reader, t: u8) -> Result<Value, String> {
        Ok(match t {
            1 => {
                let i = r.i32()?;
                Value::Element((i >= 0).then_some(i as usize))
            }
            2 => Value::Int(r.i32()?),
            3 => Value::Float(r.f32()?),
            4 => Value::Bool(r.u8()? != 0),
            5 => Value::Str(r.cstr()?),
            6 => {
                let n = r.i32()?.max(0) as usize;
                Value::Binary(r.take(n)?.to_vec())
            }
            7 => Value::Time(r.i32()? as f32 / 10_000.0),
            8 => {
                let c = r.take(4)?;
                Value::Color([c[0], c[1], c[2], c[3]])
            }
            9 => Value::Vec2(r.floats::<2>()?),
            10 | 12 => Value::Vec3(r.floats::<3>()?),
            11 | 13 => Value::Vec4(r.floats::<4>()?),
            14 => {
                r.floats::<16>()?;
                Value::Matrix
            }
            15..=28 => {
                let n = r.i32()?.max(0) as usize;
                if n > 1 << 20 {
                    return Err("array too long".into());
                }
                let mut v = Vec::with_capacity(n);
                for _ in 0..n {
                    v.push(value(r, t - 14)?);
                }
                Value::Array(v)
            }
            _ => return Err(format!("unknown attribute type {t}")),
        })
    }
    for e in &mut elements {
        let n = r.i32()?.max(0) as usize;
        for _ in 0..n {
            let key = string(r.i16()?)?;
            let t = r.u8()?;
            let v = value(&mut r, t)?;
            e.attributes.push((key, v));
        }
    }
    Ok(elements)
}

/// The `file` entries of a particle manifest, without the precache `!`.
pub fn manifest_files(text: &str) -> Vec<String> {
    let t = super::surfaceprops::tokens(text);
    t.windows(2)
        .filter(|w| w[0].eq_ignore_ascii_case("file"))
        .map(|w| w[1].trim_start_matches('!').replace('\\', "/"))
        .collect()
}

/// One file's systems: name (lower case) → (its elements, the system's
/// element).
type Systems = HashMap<String, (std::sync::Arc<Vec<Element>>, usize)>;

fn add_file(systems: &mut Systems, bytes: &[u8], warnings: &mut Vec<String>, path: &str) {
    match read_dmx(bytes) {
        Ok(elements) => {
            let elements = std::sync::Arc::new(elements);
            for (i, e) in elements.iter().enumerate() {
                if e.kind == "DmeParticleSystemDefinition" {
                    systems.insert(e.name.to_lowercase(), (elements.clone(), i));
                }
            }
        }
        Err(e) => warnings.push(format!("{path}: {e}")),
    }
}

fn pick(e: &Element, min: &str, max: &str, exp: &str) -> Pick {
    Pick::new(e.float(min, 0.0), e.float(max, 0.0), e.float(exp, 1.0))
}

/// One operator element into the definition; false when its function
/// isn't one we run.
fn add_operator(def: &mut SystemDef, list: &str, op: &Element) -> bool {
    let function = op.str("functionName").unwrap_or(&op.name).to_lowercase();
    let cp = |k: &str| op.int(k, 0).clamp(0, crate::map::psys::MAX_CONTROL_POINTS as i32 - 1) as usize;
    match (list, function.as_str()) {
        ("emitters", "emit_continuously") => def.emitters.push(Emitter::Continuously {
            start: op.float("emission_start_time", 0.0),
            rate: op.float("emission_rate", 100.0),
            duration: op.float("emission_duration", 0.0),
        }),
        ("emitters", "emit_instantaneously") => def.emitters.push(Emitter::Instantaneously {
            start: op.float("emission_start_time", 0.0),
            count: op.int("num_to_emit", 100).max(0) as u32,
        }),
        ("initializers", "position within sphere random") => def.initializers.push(Init::Sphere {
            cp: cp("control_point_number"),
            distance: pick(op, "distance_min", "distance_max", "_"),
            bias: op.vec3("distance_bias", Vec3::ONE),
            bias_abs: op.vec3("distance_bias_absolute_value", Vec3::ZERO),
            speed: pick(op, "speed_min", "speed_max", "speed_random_exponent"),
            local_min: op.vec3("speed_in_local_coordinate_system_min", Vec3::ZERO),
            local_max: op.vec3("speed_in_local_coordinate_system_max", Vec3::ZERO),
        }),
        ("initializers", "position within box random") => def.initializers.push(Init::Box {
            cp: cp("control point number"),
            min: op.vec3("min", Vec3::ZERO),
            max: op.vec3("max", Vec3::ZERO),
        }),
        ("initializers", "position modify offset random") => def.initializers.push(Init::Offset {
            cp: cp("control_point_number"),
            min: op.vec3("offset min", Vec3::ZERO),
            max: op.vec3("offset max", Vec3::ZERO),
            local: op.bool("offset in local space 0/1"),
            proportional: op.bool("offset proportional to radius 0/1"),
        }),
        ("initializers", "velocity random") => def.initializers.push(Init::Velocity {
            cp: cp("control_point_number"),
            speed: pick(op, "random_speed_min", "random_speed_max", "_"),
            local_min: op.vec3("speed_in_local_coordinate_system_min", Vec3::ZERO),
            local_max: op.vec3("speed_in_local_coordinate_system_max", Vec3::ZERO),
        }),
        ("initializers", "velocity noise") => def.initializers.push(Init::VelocityNoise {
            cp: cp("Control Point Number"),
            min: op.vec3("output minimum", Vec3::ZERO),
            max: op.vec3("output maximum", Vec3::ZERO),
            local: op.bool("Apply Velocity in Local Space (0/1)"),
        }),
        ("initializers", "lifetime random") => def.initializers.push(Init::Lifetime(pick(
            op,
            "lifetime_min",
            "lifetime_max",
            "lifetime_random_exponent",
        ))),
        ("initializers", "lifetime from sequence") => def.initializers.push(Init::LifetimeFromSequence {
            fps: op.float("Frames Per Second", 30.0),
        }),
        ("initializers", "radius random") => def.initializers.push(Init::Radius(pick(
            op,
            "radius_min",
            "radius_max",
            "radius_random_exponent",
        ))),
        ("initializers", "color random") => {
            let a = op.color("color1").map_or(Vec3::ONE, |c| c.0);
            let b = op.color("color2").map_or(Vec3::ONE, |c| c.0);
            def.initializers.push(Init::Color { a, b });
        }
        ("initializers", "alpha random") => {
            let p = pick(op, "alpha_min", "alpha_max", "alpha_random_exponent");
            def.initializers
                .push(Init::Alpha(Pick::new(p.min / 255.0, p.max / 255.0, p.exp)));
        }
        ("initializers", "rotation random") => def.initializers.push(Init::Rotation {
            initial: op.float("rotation_initial", 0.0),
            offset: pick(
                op,
                "rotation_offset_min",
                "rotation_offset_max",
                "rotation_random_exponent",
            ),
        }),
        ("initializers", "rotation yaw flip random") => def.initializers.push(Init::YawFlip {
            fraction: op.float("Flip Percentage", 0.5),
        }),
        ("initializers", "sequence random") => def.initializers.push(Init::Sequence {
            min: op.int("sequence_min", 0).max(0) as u32,
            max: op.int("sequence_max", 0).max(0) as u32,
        }),
        ("operators", "movement basic") => def.operators.push(Op::Movement {
            gravity: op.vec3("gravity", Vec3::ZERO),
            drag: op.float("drag", 0.0),
        }),
        ("operators", "lifespan decay") => def.operators.push(Op::Decay),
        ("operators", "alpha fade in random") => def.operators.push(Op::FadeIn {
            time: pick(op, "fade_in_time_min", "fade_in_time_max", "fade_in_time_exp"),
            proportional: op.bool("proportional 0/1"),
        }),
        ("operators", "alpha fade out random") => def.operators.push(Op::FadeOut {
            time: pick(op, "fade_out_time_min", "fade_out_time_max", "fade_out_time_exp"),
            proportional: op.bool("proportional 0/1"),
        }),
        ("operators", "alpha fade and decay") => def.operators.push(Op::FadeAndDecay {
            start_alpha: op.float("start_alpha", 1.0),
            end_alpha: op.float("end_alpha", 0.0),
            fade_in: (op.float("start_fade_in_time", 0.0), op.float("end_fade_in_time", 0.5)),
            fade_out: (op.float("start_fade_out_time", 0.5), op.float("end_fade_out_time", 1.0)),
        }),
        ("operators", "color fade") => def.operators.push(Op::ColorFade {
            color: op.color("color_fade").map_or(Vec3::ONE, |c| c.0),
            start: op.float("fade_start_time", 0.0),
            end: op.float("fade_end_time", 1.0),
            ease: op.bool("ease_in_and_out"),
        }),
        ("operators", "radius scale") => def.operators.push(Op::RadiusScale {
            start_scale: op.float("radius_start_scale", 1.0),
            end_scale: op.float("radius_end_scale", 1.0),
            start: op.float("start_time", 0.0),
            end: op.float("end_time", 1.0),
            bias: op.float("scale_bias", 0.5),
            ease: op.bool("ease_in_and_out"),
        }),
        ("operators", "rotation basic") => def.operators.push(Op::RotationBasic),
        ("operators", "rotation spin roll") => def.operators.push(Op::SpinRoll {
            rate: op.float("spin_rate_degrees", 0.0),
        }),
        ("operators", "rotation spin yaw") => def.operators.push(Op::SpinYaw {
            rate: op.float("yaw_rate_degrees", 0.0),
        }),
        ("operators", "movement max velocity") => def.operators.push(Op::MaxVelocity {
            max: op.float("Maximum Velocity", 0.0),
        }),
        ("operators", "movement lock to control point") => def.operators.push(Op::LockToControlPoint {
            cp: cp("control_point_number"),
        }),
        // A wobble on one field: not run (its look is small).
        ("operators", "oscillate scalar") => {}
        ("forces", "pull towards control point") => def.operators.push(Op::Pull {
            cp: cp("control point number"),
            force: op.float("amount of force", 0.0),
            falloff: op.float("falloff power", 2.0),
        }),
        ("forces", "random force") => def.operators.push(Op::RandomForce {
            min: op.vec3("min force", Vec3::ZERO),
            max: op.vec3("max force", Vec3::ZERO),
        }),
        ("forces", "twist around axis") => def.operators.push(Op::Twist {
            cp: cp("control point"),
            force: op.float("amount of force", 0.0),
            axis: op.vec3("twist axis", Vec3::Z),
            local: op.bool("object local space axis 0/1"),
        }),
        // World collision of particles: not run.
        ("constraints", "collision via traces") => {}
        ("renderers", "render_animated_sprites") => {
            def.renderer = Render::Sprites {
                rate: op.float("animation rate", 1.0),
                as_fps: op.bool("use animation rate as FPS"),
                fit_life: op.bool("animation_fit_lifetime"),
                orientation: op.int("orientation_type", 0),
            }
        }
        ("renderers", "render_sprite_trail") => {
            def.renderer = Render::Trail {
                min_length: op.float("min length", 0.0),
                max_length: op.float("max length", 2000.0),
                length_fade_in: op.float("length fade in time", 0.0),
            }
        }
        ("renderers", "render_rope") => def.renderer = Render::Rope,
        _ => return false,
    }
    true
}

/// Build the definitions of `wanted` systems and their children; their
/// indices by lower-case name.
fn build(
    systems: &Systems,
    wanted: &[String],
    materials: &mut MaterialLoader,
    data: &mut MapData,
    unknown: &mut std::collections::BTreeSet<String>,
) -> (Vec<SystemDef>, HashMap<String, usize>) {
    let mut defs: Vec<SystemDef> = Vec::new();
    let mut index: HashMap<String, usize> = HashMap::new();
    let mut todo: Vec<String> = wanted.iter().map(|w| w.to_lowercase()).collect();
    // Children by name, filled after all are numbered.
    let mut child_names: Vec<Vec<(String, f32)>> = Vec::new();
    while let Some(name) = todo.pop() {
        if index.contains_key(&name) {
            continue;
        }
        let Some((elements, at)) = systems.get(&name) else {
            continue;
        };
        let e = &elements[*at];
        let (color, alpha) = e.color("color").unwrap_or((Vec3::ONE, 1.0));
        let mut def = SystemDef {
            name: e.name.clone(),
            max_particles: e.int("max_particles", 1000).clamp(0, 5000) as usize,
            initial_particles: e.int("initial_particles", 0).clamp(0, 5000) as usize,
            material: e
                .str("material")
                .filter(|m| !m.is_empty())
                .and_then(|m| particle_material(materials, data, m, None)),
            radius: e.float("radius", 5.0),
            color,
            alpha,
            rotation: e.float("rotation", 0.0),
            rotation_speed: e.float("rotation_speed", 0.0),
            sequence: e.int("sequence_number", 0).max(0) as u32,
            max_step: e.float("maximum time step", 0.1).max(0.01),
            max_draw_distance: e.float("maximum draw distance", 100_000.0),
            ..default()
        };
        for list in [
            "emitters",
            "initializers",
            "operators",
            "forces",
            "constraints",
            "renderers",
        ] {
            for op in e.elements(list) {
                let Some(op) = elements.get(op) else { continue };
                if !add_operator(&mut def, list, op) {
                    unknown.insert(format!("{list}: {}", op.str("functionName").unwrap_or(&op.name)));
                }
            }
        }
        let mut kids = Vec::new();
        for c in e.elements("children") {
            let Some(child) = elements.get(c) else { continue };
            let Some(target) = child.element("child").and_then(|t| elements.get(t)) else {
                continue;
            };
            let n = target.name.to_lowercase();
            kids.push((n.clone(), child.float("delay", 0.0)));
            todo.push(n);
        }
        index.insert(name, defs.len());
        defs.push(def);
        child_names.push(kids);
    }
    for (def, kids) in defs.iter_mut().zip(child_names) {
        def.children = kids
            .into_iter()
            .filter_map(|(n, delay)| Some((*index.get(&n)?, delay)))
            .collect();
    }
    (defs, index)
}

/// Load the map's particle systems (the ones its info_particle_systems
/// name) into `data.particle_systems`.
pub fn add_particle_systems(bsp: &Bsp, materials: &mut MaterialLoader, data: &mut MapData, map: &str) {
    let placed: Vec<(usize, String, Vec3, Vec3, bool)> = bsp
        .entities
        .iter()
        .enumerate()
        .filter(|(_, e)| {
            e.prop("classname")
                .is_some_and(|c| c.eq_ignore_ascii_case("info_particle_system"))
        })
        .filter_map(|(i, e)| {
            let key = |k: &str| e.properties().find(|(n, _)| n.eq_ignore_ascii_case(k)).map(|(_, v)| v);
            let name = key("effect_name").map(str::trim).filter(|n| !n.is_empty())?;
            let v3 = |k: &str| {
                let n: Vec<f32> = key(k)
                    .unwrap_or("")
                    .split_whitespace()
                    .filter_map(|x| x.parse().ok())
                    .collect();
                if n.len() >= 3 {
                    Vec3::new(n[0], n[1], n[2])
                } else {
                    Vec3::ZERO
                }
            };
            let on = key("start_active").is_some_and(|v| v.trim().starts_with('1'));
            Some((i, name.to_string(), v3("origin"), v3("angles"), on))
        })
        .collect();
    if placed.is_empty() {
        return;
    }
    let mut systems: Systems = HashMap::new();
    let mut warnings = Vec::new();
    for f in materials
        .read_text("particles/particles_manifest.txt")
        .map(|t| manifest_files(&t))
        .unwrap_or_default()
    {
        if let Some(b) = materials.read(&f) {
            add_file(&mut systems, &b, &mut warnings, &f);
        }
    }
    let map = map.rsplit('/').next().unwrap_or(map).to_lowercase();
    let map_manifest = materials
        .read_packed("particles.txt")
        .or_else(|| materials.read(&format!("maps/{map}_particles.txt")));
    if let Some(text) = map_manifest {
        for f in manifest_files(&String::from_utf8_lossy(&text))
            .into_iter()
            .take(MAP_MANIFEST_MAX)
        {
            if !f.to_lowercase().starts_with("particles/") {
                warnings.push(format!("particle manifest: {f} is not under particles/ (refused)"));
                continue;
            }
            match materials.read(&f) {
                Some(b) => add_file(&mut systems, &b, &mut warnings, &f),
                None => warnings.push(format!("particle file not found: {f}")),
            }
        }
    }
    let names: Vec<String> = placed.iter().map(|p| p.1.clone()).collect();
    let mut unknown = std::collections::BTreeSet::new();
    let (defs, index) = build(&systems, &names, materials, data, &mut unknown);
    let mut missing = std::collections::BTreeSet::new();
    let mut out = MapParticleSystems {
        defs,
        placed: Vec::new(),
        scale: METERS_PER_UNIT,
    };
    for (entity, name, origin, angles, on) in placed {
        let Some(&system) = index.get(&name.to_lowercase()) else {
            missing.insert(name);
            continue;
        };
        out.placed.push(PlacedSystem {
            entity: Some(entity),
            position: Vec3::new(origin.x, origin.z, -origin.y) * METERS_PER_UNIT,
            rotation: crate::map::entities::rotation_to_engine(crate::map::entities::entity_rotation(angles)),
            system,
            start_on: on,
        });
    }
    if !missing.is_empty() {
        warnings.push(format!(
            "particle systems not found (not drawn): {}",
            missing.into_iter().collect::<Vec<_>>().join(", ")
        ));
    }
    if !unknown.is_empty() {
        warnings.push(format!(
            "particle operators not run: {}",
            unknown.into_iter().collect::<Vec<_>>().join(", ")
        ));
    }
    data.warnings.extend(warnings);
    data.particle_systems = std::sync::Arc::new(out);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A tiny binary-2 DMX: a root with a system holding one emitter.
    fn sample() -> Vec<u8> {
        let mut b = b"<!-- dmx encoding binary 2 format pcf 1 -->\n\0".to_vec();
        let strings = [
            "DmElement",
            "DmeParticleSystemDefinition",
            "DmeParticleOperator",
            "particleSystemDefinitions",
            "emitters",
            "functionName",
            "emission_rate",
            "material",
        ];
        b.extend((strings.len() as i16).to_le_bytes());
        for s in strings {
            b.extend(s.as_bytes());
            b.push(0);
        }
        b.extend(3i32.to_le_bytes());
        for (kind, name) in [(0i16, "root"), (1, "fx"), (2, "emit")] {
            b.extend(kind.to_le_bytes());
            b.extend(name.as_bytes());
            b.push(0);
            b.extend([0u8; 16]);
        }
        // root: particleSystemDefinitions = [1]
        b.extend(1i32.to_le_bytes());
        b.extend(3i16.to_le_bytes());
        b.push(15);
        b.extend(1i32.to_le_bytes());
        b.extend(1i32.to_le_bytes());
        // fx: emitters = [2], material = "x"
        b.extend(2i32.to_le_bytes());
        b.extend(4i16.to_le_bytes());
        b.push(15);
        b.extend(1i32.to_le_bytes());
        b.extend(2i32.to_le_bytes());
        b.extend(7i16.to_le_bytes());
        b.push(5);
        b.extend(b"x\0");
        // emit: functionName, emission_rate 25
        b.extend(2i32.to_le_bytes());
        b.extend(5i16.to_le_bytes());
        b.push(5);
        b.extend(b"emit_continuously\0");
        b.extend(6i16.to_le_bytes());
        b.push(3);
        b.extend(25f32.to_le_bytes());
        b
    }

    #[test]
    fn reads_binary_dmx() {
        let e = read_dmx(&sample()).unwrap();
        assert_eq!(e.len(), 3);
        assert_eq!(e[1].kind, "DmeParticleSystemDefinition");
        assert_eq!(e[0].elements("particleSystemDefinitions"), vec![1]);
        assert_eq!(e[1].str("material"), Some("x"));
        let mut def = SystemDef::default();
        assert!(add_operator(&mut def, "emitters", &e[2]));
        assert_eq!(
            def.emitters,
            vec![Emitter::Continuously {
                start: 0.0,
                rate: 25.0,
                duration: 0.0
            }]
        );
        assert!(read_dmx(b"<!-- dmx encoding keyvalues2 1 format pcf 1 -->\n\0").is_err());
    }

    #[test]
    fn manifest_entries() {
        let text = "particles_manifest\n{\n\t\"file\"\t\"!particles/a.pcf\"\n\tfile \"particles/b.pcf\"\n}";
        assert_eq!(manifest_files(text), vec!["particles/a.pcf", "particles/b.pcf"]);
    }
}
