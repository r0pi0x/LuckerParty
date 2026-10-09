//! The logic of beams, glows and sparks (specs/source/visual_entities.md):
//! point_spotlight's LightOn/LightOff and outputs (2.5), env_laser's and
//! persistent env_beam's on/off, damage every 0.1 s to the first player
//! on the line and env_beam's touch output (3, 4.1), env_lightglow's
//! inputs, env_spark's timing and OnSpark (8). The map draws them
//! (`map::beams`); what is on reaches it as `map::EntityPart`.

use bevy::prelude::*;

use super::classes::Class;
use super::value::Value;
use super::visuals::PartKind;
use super::world::{Effect, EntId, LogicEntity, LogicWorld, Who};

/// env_spark "Start ON".
pub const SF_SPARK_START_ON: u32 = 64;
/// Continuous beam damage needs this much time since the last hit.
pub const DAMAGE_PERIOD: f32 = 0.1;

/// A laser's or beam's damage and touch state.
#[derive(Clone, Debug, Default)]
pub struct Zap {
    /// Damage per second.
    pub damage: f32,
    /// Tick of the last damage (or of turning on).
    pub last: i64,
    /// env_beam TouchType (0: none).
    pub touch: i32,
    /// Stopped thinking after OnTouchedByEntity (until TurnOn).
    pub stopped: bool,
    pub laser: bool,
    /// Where the laser aimed last (entity space).
    pub end: Option<Vec3>,
}

fn class(e: &LogicEntity) -> String {
    e.classname.to_ascii_lowercase()
}

/// Whether a beam entity starts on.
pub fn starts_on(e: &LogicEntity) -> bool {
    match class(e).as_str() {
        "point_spotlight" => e.has_flag(1),
        _ => e.targetname.is_empty() || e.has_flag(1),
    }
}

/// A laser's or persistent beam's damage state, when it can hurt or be
/// touched.
pub fn zap(e: &LogicEntity) -> Option<Box<Zap>> {
    let c = class(e);
    let laser = c == "env_laser";
    if !laser && (c != "env_beam" || e.kv_f("life") != 0.0) {
        return None;
    }
    let damage = e.kv_f("damage");
    let touch = if laser { 0 } else { e.kv_i("TouchType") };
    (damage > 0.0 || touch != 0).then(|| {
        Box::new(Zap {
            damage,
            touch,
            laser,
            ..default()
        })
    })
}

fn part(w: &mut LogicWorld, id: EntId) -> Option<&mut super::visuals::Part> {
    match w.get_mut(id).map(|e| &mut e.class) {
        Some(Class::Part(p)) => Some(p),
        _ => None,
    }
}

/// Turn on or off; outputs and thinking as the class does.
fn set_on(w: &mut LogicWorld, id: EntId, on: bool) {
    let tick = w.tick;
    let Some(p) = part(w, id) else { return };
    if p.on == on {
        return;
    }
    p.on = on;
    if let Some(z) = p.zap.as_mut()
        && on
    {
        z.last = tick;
        z.stopped = false;
    }
    let kind = p.kind;
    let thinks = p.zap.is_some();
    let spotlight = w.get(id).is_some_and(|e| class(e) == "point_spotlight");
    if spotlight {
        let out = if on { "OnLightOn" } else { "OnLightOff" };
        w.fire_output(id, out, Some(Who::Ent(id)), Value::Void);
    }
    match (kind, on) {
        (PartKind::Beam, true) if thinks => w.think_in(id, w.dt),
        (PartKind::Spark, true) => w.think_in(id, w.dt),
        (_, false) => {
            if let Some(e) = w.get_mut(id) {
                e.next_think = None;
            }
        }
        _ => {}
    }
}

pub(super) fn input(w: &mut LogicWorld, id: EntId, input: &str) -> bool {
    let Some(p) = w.get(id).and_then(|e| match &e.class {
        Class::Part(p) => Some((p.kind, p.on)),
        _ => None,
    }) else {
        return false;
    };
    let spotlight = w.get(id).is_some_and(|e| class(e) == "point_spotlight");
    match (p.0, input) {
        (PartKind::Beam, "lighton") if spotlight => set_on(w, id, true),
        (PartKind::Beam, "lightoff") if spotlight => set_on(w, id, false),
        (PartKind::Beam, "turnon") if !spotlight => set_on(w, id, true),
        (PartKind::Beam, "turnoff") if !spotlight => set_on(w, id, false),
        (PartKind::Beam, "toggle") if !spotlight => set_on(w, id, !p.1),
        // The beam's look (width, noise, colour, scroll): kept as placed.
        (
            PartKind::Beam,
            "width" | "noise" | "colorredvalue" | "colorgreenvalue" | "colorbluevalue" | "scrollspeed" | "strikeonce",
        ) => {}
        (PartKind::Glow, "color") => {}
        (PartKind::Spark, "startspark") => set_on(w, id, true),
        (PartKind::Spark, "stopspark") => set_on(w, id, false),
        (PartKind::Spark, "togglespark") => set_on(w, id, !p.1),
        (PartKind::Spark, "sparkonce") => {
            spark(w, id);
            set_on(w, id, false);
        }
        _ => return false,
    }
    true
}

/// At activation: a lit spotlight fires OnLightOn; lasers and beams that
/// are on start thinking; a spark that starts on sparks after 0.1 +
/// U(0, 1.5) s.
pub(super) fn activate(w: &mut LogicWorld, id: EntId) {
    let Some((kind, on, zap)) = w.get(id).and_then(|e| match &e.class {
        Class::Part(p) => Some((p.kind, p.on, p.zap.is_some())),
        _ => None,
    }) else {
        return;
    };
    let spotlight = w.get(id).is_some_and(|e| class(e) == "point_spotlight");
    match kind {
        PartKind::Beam if spotlight && on => w.fire_output(id, "OnLightOn", Some(Who::Ent(id)), Value::Void),
        PartKind::Beam if zap && on => w.think_in(id, w.dt),
        PartKind::Spark if on => {
            let delay = 0.1 + w.random() * 1.5;
            w.think_in(id, delay);
        }
        _ => {}
    }
}

/// One spark: OnSpark (the effect itself isn't drawn yet).
fn spark(w: &mut LogicWorld, id: EntId) {
    w.fire_output(id, "OnSpark", Some(Who::Ent(id)), Value::Void);
}

pub(super) fn think(w: &mut LogicWorld, id: EntId) {
    let Some((kind, on)) = w.get(id).and_then(|e| match &e.class {
        Class::Part(p) => Some((p.kind, p.on)),
        _ => None,
    }) else {
        return;
    };
    if !on {
        return;
    }
    match kind {
        PartKind::Spark => {
            spark(w, id);
            let max = w.get(id).map_or(0.0, |e| e.kv_f("MaxDelay").max(0.0));
            let delay = 0.1 + w.random() * max;
            w.think_in(id, delay);
        }
        PartKind::Beam => {
            zap_tick(w, id);
            if part(w, id).and_then(|p| p.zap.as_ref()).is_some_and(|z| !z.stopped) {
                w.think_in(id, w.dt);
            }
        }
        _ => {}
    }
}

/// Where a laser or beam runs this tick (entity space).
fn line(w: &mut LogicWorld, id: EntId) -> Option<(Vec3, Vec3)> {
    let e = w.get(id)?;
    let laser = class(e) == "env_laser";
    let origin_of = |w: &LogicWorld, name: &str| w.find(name).and_then(|t| w.get(t)).map(|t| t.origin);
    if laser {
        let start = e.origin;
        let name = e.kv("LaserTarget").unwrap_or("").to_string();
        // A random one of the targets, every tick (3).
        let targets: Vec<Vec3> = w
            .resolve(&name, None, None)
            .into_iter()
            .filter_map(|t| match t {
                Who::Ent(t) => w.get(t).map(|t| t.origin),
                Who::Player(p) => w.player(p).map(|p| p.origin),
            })
            .collect();
        let end = if targets.is_empty() {
            part(w, id).and_then(|p| p.zap.as_ref()).and_then(|z| z.end)?
        } else {
            targets[w.random_int(targets.len())]
        };
        if let Some(z) = part(w, id).and_then(|p| p.zap.as_mut()) {
            z.end = Some(end);
        }
        Some((start, end))
    } else {
        let (a, b) = (
            e.kv("LightningStart").unwrap_or("").to_string(),
            e.kv("LightningEnd").unwrap_or("").to_string(),
        );
        Some((origin_of(w, &a)?, origin_of(w, &b)?))
    }
}

/// Where a segment enters a box, as a fraction (None: it misses).
fn segment_box(a: Vec3, b: Vec3, lo: Vec3, hi: Vec3) -> Option<f32> {
    let d = b - a;
    let (mut t0, mut t1) = (0.0f32, 1.0f32);
    for i in 0..3 {
        if d[i].abs() < 1e-9 {
            if a[i] < lo[i] || a[i] > hi[i] {
                return None;
            }
            continue;
        }
        let (u, v) = ((lo[i] - a[i]) / d[i], (hi[i] - a[i]) / d[i]);
        t0 = t0.max(u.min(v));
        t1 = t1.min(u.max(v));
        if t0 > t1 {
            return None;
        }
    }
    Some(t0)
}

/// The first player on the line before the world stops it.
fn first_player(w: &LogicWorld, a: Vec3, b: Vec3) -> Option<Entity> {
    let wall = w
        .collision
        .as_ref()
        .map_or(1.0, |c| c.sweep(Vec3::ZERO, Vec3::ZERO, a, b));
    w.players
        .iter()
        .filter(|p| p.alive)
        .filter_map(|p| segment_box(a, b, p.origin + p.mins, p.origin + p.maxs).map(|t| (t, p.entity)))
        .filter(|(t, _)| *t <= wall)
        .min_by(|x, y| x.0.total_cmp(&y.0))
        .map(|(_, e)| e)
}

/// A laser's or beam's tick (3 step 4, 4.1): damage × elapsed to the
/// first player on the line every 0.1 s; a touch-type beam fires
/// OnTouchedByEntity when a player crosses it, and stops thinking.
fn zap_tick(w: &mut LogicWorld, id: EntId) {
    let Some(z) = part(w, id).and_then(|p| p.zap.as_deref()).cloned() else { return };
    let Some((a, b)) = line(w, id) else { return };
    if (b - a).length() < 0.1 {
        return;
    }
    let tick = w.tick;
    let elapsed = (tick - z.last) as f32 * w.dt;
    if z.damage > 0.0 && elapsed >= DAMAGE_PERIOD - 1e-4 {
        if let Some(p) = first_player(w, a, b) {
            w.effects.push(Effect::Damage {
                target: p,
                amount: z.damage * elapsed,
                crush: false,
            });
        }
        if let Some(z) = part(w, id).and_then(|p| p.zap.as_mut()) {
            z.last = tick;
        }
    }
    // TouchType 1/3/4: players (no NPCs here).
    if matches!(z.touch, 1 | 3 | 4)
        && let Some(p) = first_player(w, a, b)
    {
        let filter = w.get(id).and_then(|e| e.kv("filtername")).unwrap_or("").to_string();
        let passes = filter.is_empty()
            || w.find(&filter)
                .is_none_or(|f| super::classes::filter_passes(w, f, Some(Who::Player(p))));
        if passes {
            w.fire_output(id, "OnTouchedByEntity", Some(Who::Player(p)), Value::Void);
            if let Some(z) = part(w, id).and_then(|p| p.zap.as_mut()) {
                z.stopped = true;
            }
        }
    }
}
