//! Fire (specs/source/fire.md): `env_fire` (the heat model, growth,
//! burn damage in a box with a line-of-sight check, fuel, extinguishing,
//! outputs), the heat helpers `env_firesource` and `env_firesensor`, and
//! entity flames (a flame on one prop or player: it burns its target and
//! what is around it and heats nearby fires). Burning props' rules are in
//! `prop_damage`; how fires look and sound is the host's
//! (`LogicWorld::fire_looks`, `flame_looks`, `Effect::FlameStart`).
//!
//! Units as the spec: entity space (inches, Z up), seconds. Fire thinks
//! advance their own clock by the nominal 0.1 s per think whatever the
//! tick rounds it to.

use bevy::prelude::*;

use super::classes::Class;
use super::prop_damage::Hit;
use super::value::Value;
use super::world::{Effect, EntId, LogicWorld, Who, solid_brushes, sweep_brushes};
use crate::core::DamageKind;

/// env_fire spawnflags.
pub const SF_FIRE_INFINITE: u32 = 1;
pub const SF_FIRE_SMOKELESS: u32 = 2;
pub const SF_FIRE_START_ON: u32 = 4;
pub const SF_FIRE_START_FULL: u32 = 8;
pub const SF_FIRE_DONT_DROP: u32 = 16;
pub const SF_FIRE_NO_GLOW: u32 = 32;
pub const SF_FIRE_DIE_PERMANENT: u32 = 128;
pub const SF_FIRE_VISIBLE_FROM_ABOVE: u32 = 256;

/// Heat that counts as "strength 1" (damage and effect scale).
pub const STRENGTH_REF: f32 = 64.0;
/// Full heat of a size-256 fire (H = S/4).
pub const HEAT_PER_256: f32 = 64.0;
/// env_fire update interval, nominal seconds.
pub const FIRE_THINK: f32 = 0.1;
/// StartFire looks this far down for a floor.
pub const DROP_DISTANCE: f32 = 1024.0;
/// Smallest half-width of the damage box.
pub const MIN_DAMAGE_RADIUS: f32 = 16.0;
/// Taken off the heat when a fire goes out.
pub const GO_OUT_PENALTY: f32 = 20.0;
/// Fade time when fuel runs out.
pub const FUEL_FADE: f32 = 1.0;
/// An unlit fire's bounds.
pub const OUT_MINS: Vec3 = Vec3::new(-8.0, -8.0, 0.0);
pub const OUT_MAXS: Vec3 = Vec3::new(8.0, 8.0, 8.0);
/// The fire cvars at their defaults (`fire_absorbrate`, `fire_heatscale`,
/// `fire_incomingheatscale`, `fire_dmgscale`, `fire_dmgbase`,
/// `fire_growthrate`, `fire_dmginterval`). `fire_maxabsorb`,
/// `fire_extscale` and `fire_extabsorb` belong to extinguishing in a
/// radius, which nothing in CS:S calls.
pub const FIRE_ABSORB_RATE: f32 = 3.0;
pub const FIRE_HEAT_SCALE: f32 = 1.0;
pub const FIRE_INCOMING_HEAT_SCALE: f32 = 0.1;
pub const FIRE_DMG_SCALE: f32 = 0.1;
pub const FIRE_DMG_BASE: f32 = 1.0;
pub const FIRE_GROWTH_RATE: f32 = 1.0;
pub const FIRE_DMG_INTERVAL: f32 = 1.0;
/// absorb = `ignitionpoint` × this at spawn.
pub const ABSORB_FROM_IGNITION: f32 = 0.05;
/// Most neighbouring fires an update heats.
pub const MAX_NEIGHBOURS: usize = 16;

/// Entity flames.
pub const FLAME_THINK: f32 = 0.2;
pub const FLAME_FIRST_THINK: f32 = 0.1;
/// Burn + direct damage to the burning entity per think.
pub const FLAME_DIRECT: f32 = 1.0;
/// Burn radius damage to others per think (radius = flame size / 2).
pub const FLAME_RADIUS_DAMAGE: f32 = 4.0;
/// Heat per think to fires within flame size / 2.
pub const FLAME_HEAT: f32 = 2.0;
pub const FLAME_MIN_SIZE: f32 = 16.0;
/// Lifetime set at creation before the caller's.
pub const FLAME_DEFAULT_LIFE: f32 = 2.0;
/// The Ignite input's lifetime.
pub const IGNITE_LIFE: f32 = 30.0;
/// After "stop burning" until the flame is removed.
pub const FLAME_REMOVE_DELAY: f32 = 0.5;
/// A prop set on fire by damage burns U(10, 15) s.
pub const PROP_BURN_TIME: (f32, f32) = (10.0, 15.0);

pub const FIRESOURCE_THINK: f32 = 0.25;
pub const SENSOR_MIN_THINK: f32 = 0.1;

/// The sound entry a flame loops on a non-NPC target.
pub const BURNING_SOUND: &str = "General.BurningObject";

/// A placed fire (env_fire).
#[derive(Clone, Debug)]
pub struct Fire {
    /// `firesize`: height of a full fire.
    pub size: f32,
    /// `fireattack`: nominal seconds to grow to full heat.
    pub attack: f32,
    pub heat: f32,
    /// Full heat, S/4.
    pub max_heat: f32,
    /// Heat still to be absorbed before it lights.
    pub absorb: f32,
    /// Seconds of fuel left; 0: infinite.
    pub fuel: f32,
    pub damage_scale: f32,
    /// `firetype` 1 (plasma; no stock map uses it).
    pub plasma: bool,
    pub enabled: bool,
    /// The effect exists (burning, or fading out).
    pub lit: bool,
    /// Fading out: no growth or damage until it goes out.
    pub fading: bool,
    /// Deleted when it goes out (flag 128, or it had fuel).
    pub delete_when_out: bool,
    /// When the next damage tick is due, seconds of world time.
    pub damage_due: f64,
    /// How many times it has lit (a new number restarts its look).
    pub serial: u32,
    /// The damage of the last damage tick (tests).
    pub last_damage: i32,
}

/// A heat source (env_firesource).
#[derive(Clone, Debug)]
pub struct FireSource {
    pub on: bool,
    pub radius: f32,
    pub damage: f32,
}

/// A heat sensor (env_firesensor).
#[derive(Clone, Debug)]
pub struct FireSensor {
    pub on: bool,
    pub radius: f32,
    pub level: f32,
    pub time: f32,
    pub accumulated: f32,
    pub at_level: bool,
}

/// A flame burning one entity.
#[derive(Clone, Debug)]
pub struct Flame {
    pub target: Who,
    /// max(16, (box x + box y) / 2) of the target.
    pub size: f32,
    /// When it stops burning, seconds of world time.
    pub life_end: f64,
    /// Stopped: removed at its next think.
    pub stopping: bool,
}

/// How a lit fire looks, for the host.
#[derive(Clone, Debug, PartialEq)]
pub struct FireLook {
    pub id: EntId,
    pub serial: u32,
    /// The floor point (entity space).
    pub at: Vec3,
    pub size: f32,
    pub smokeless: bool,
    pub plasma: bool,
}

/// A burning entity, for the host.
#[derive(Clone, Debug, PartialEq)]
pub struct FlameLook {
    pub id: EntId,
    pub target: Who,
    pub size: f32,
}

// ----------------------------------------------------------- env_fire

pub(super) fn spawn_fire(w: &LogicWorld, id: EntId) -> Fire {
    let e = w.get(id).unwrap();
    let size = e.kv_f("firesize");
    let max_heat = HEAT_PER_256 * size / 256.0;
    Fire {
        size,
        attack: e.kv_f("fireattack"),
        heat: if e.has_flag(SF_FIRE_START_FULL) { max_heat } else { 0.0 },
        max_heat,
        absorb: ABSORB_FROM_IGNITION * e.kv_f("ignitionpoint"),
        fuel: 0.0,
        damage_scale: e.kv_f("damagescale"),
        plasma: e.kv_i("firetype") == 1,
        enabled: e.kv_i("StartDisabled") == 0,
        lit: false,
        fading: false,
        delete_when_out: false,
        damage_due: 0.0,
        serial: 0,
        last_damage: 0,
    }
}

pub(super) fn spawn_source(w: &LogicWorld, id: EntId) -> FireSource {
    let e = w.get(id).unwrap();
    FireSource {
        on: e.has_flag(1),
        radius: e.kv_f("fireradius"),
        damage: e.kv_f("firedamage"),
    }
}

pub(super) fn spawn_sensor(w: &LogicWorld, id: EntId) -> FireSensor {
    let e = w.get(id).unwrap();
    FireSensor {
        on: e.has_flag(1),
        radius: e.kv_f("fireradius"),
        level: e.kv_f("heatlevel"),
        time: e.kv_f("heattime"),
        accumulated: 0.0,
        at_level: false,
    }
}

fn fire(w: &mut LogicWorld, id: EntId) -> Option<&mut Fire> {
    match w.get_mut(id).map(|e| &mut e.class) {
        Some(Class::Fire(f)) => Some(f),
        _ => None,
    }
}

/// After every entity spawned: "start on" lights it at full heat.
pub(super) fn activate(w: &mut LogicWorld, id: EntId) {
    match w.get(id).map(|e| (&e.class, e.spawnflags)) {
        Some((Class::Fire(_), flags)) if flags & SF_FIRE_START_ON != 0 => {
            let f = fire(w, id).unwrap();
            f.heat = f.max_heat;
            start_fire(w, id, false);
        }
        Some((Class::FireSource(s), _)) if s.on => w.think_in(id, 0.0),
        Some((Class::FireSensor(s), _)) if s.on => {
            let t = sensor_interval(s);
            w.think_in(id, t);
        }
        _ => {}
    }
}

/// A straight line through the static world and the movers (all but
/// `ignore`'s): the fraction it gets and whether it started in solid.
pub fn trace_line(w: &LogicWorld, from: Vec3, to: Vec3, ignore: Option<EntId>) -> (f32, bool) {
    let mut frac = 1.0f32;
    let mut start_solid = false;
    if let Some(c) = &w.collision {
        frac = c.sweep(Vec3::ZERO, Vec3::ZERO, from, to);
        start_solid = c.solid(Vec3::ZERO, Vec3::ZERO, from);
    }
    let movers = w
        .solids
        .iter()
        .enumerate()
        .filter(|(i, _)| ignore.is_none_or(|x| x.index as usize != *i))
        .filter_map(|(_, s)| s.as_ref())
        .flatten();
    frac = frac.min(sweep_brushes(movers.clone(), Vec3::ZERO, Vec3::ZERO, from, to));
    start_solid |= solid_brushes(movers, Vec3::ZERO, Vec3::ZERO, from);
    (frac, start_solid)
}

/// StartFire (spec 2.2): drop to the floor, refuel, light, fire
/// OnIgnited and run the first update now.
pub(super) fn start_fire(w: &mut LogicWorld, id: EntId, check_enabled: bool) {
    let Some(f) = fire(w, id) else { return };
    if (check_enabled && !f.enabled) || f.lit {
        return;
    }
    let e = w.get(id).unwrap();
    let (flags, origin, health) = (e.spawnflags, e.origin, e.kv_f("health"));
    if flags & SF_FIRE_DONT_DROP == 0 {
        let to = origin - Vec3::Z * DROP_DISTANCE;
        let (frac, _) = trace_line(w, origin, to, None);
        w.get_mut(id).unwrap().origin = origin.lerp(to, frac);
    }
    let f = fire(w, id).unwrap();
    f.fuel = if flags & SF_FIRE_INFINITE != 0 { 0.0 } else { health };
    if f.fuel != 0.0 {
        f.delete_when_out = true;
    }
    f.max_heat = HEAT_PER_256 * f.size / 256.0;
    if flags & SF_FIRE_START_FULL != 0 {
        f.heat = f.max_heat;
    }
    f.lit = true;
    f.fading = false;
    f.serial += 1;
    f.damage_due = 0.0;
    w.fire_output(id, "OnIgnited", Some(Who::Ent(id)), Value::Void);
    update(w, id);
}

/// Add heat (spec 2.4): from others (neighbours, helpers, flames) or the
/// fire's own growth.
pub fn add_heat(w: &mut LogicWorld, id: EntId, heat: f32, from_others: bool) {
    let Some(f) = fire(w, id) else { return };
    if !f.enabled {
        return;
    }
    let mut heat = heat;
    if from_others && f.heat > 0.0 {
        heat *= FIRE_INCOMING_HEAT_SCALE;
    }
    if f.absorb > 0.0 {
        let cost = heat * FIRE_ABSORB_RATE;
        if cost > f.absorb {
            heat -= f.absorb / FIRE_ABSORB_RATE;
            f.absorb = 0.0;
        } else {
            f.absorb -= cost;
            heat = 0.0;
        }
    }
    let old = f.heat;
    f.heat += heat;
    if old <= 0.0 && f.heat > 0.0 && !f.lit {
        start_fire(w, id, false);
    }
    if let Some(f) = fire(w, id) {
        f.heat = f.heat.min(f.max_heat);
    }
}

/// The damage box (spec 2.3 step 5) for heat `h`: the full box and the
/// object box, around the floor point; None when not burning.
pub fn damage_boxes(origin: Vec3, size: f32, h: f32, max_heat: f32) -> Option<((Vec3, Vec3), (Vec3, Vec3))> {
    if h <= 0.0 || max_heat <= 0.0 {
        return None;
    }
    let c = h / max_heat;
    let r = (c * size / 2.0).max(MIN_DAMAGE_RADIUS);
    let top = c * size;
    Some((
        (origin + Vec3::new(-r, -r, 0.0), origin + Vec3::new(r, r, top)),
        (
            origin + Vec3::new(-r / 2.0, -r / 2.0, 0.0),
            origin + Vec3::new(r / 2.0, r / 2.0, top / 2.0),
        ),
    ))
}

fn touches((a0, a1): (Vec3, Vec3), (b0, b1): (Vec3, Vec3)) -> bool {
    a0.cmple(b1).all() && b0.cmple(a1).all()
}

/// A fire's bounds: lit (−S/4, −S/4, 0)–(S/4, S/4, S), else the out box.
fn fire_bounds(origin: Vec3, f: &Fire) -> (Vec3, Vec3) {
    if f.lit {
        let q = f.size / 4.0;
        (origin + Vec3::new(-q, -q, 0.0), origin + Vec3::new(q, q, f.size))
    } else {
        (origin + OUT_MINS, origin + OUT_MAXS)
    }
}

/// What a damage tick can hit.
enum Victim {
    Player(Entity),
    Ent(EntId),
}

/// A breakable's or prop's world bounds (entity space).
fn entity_bounds(w: &LogicWorld, id: EntId) -> Option<(Vec3, Vec3)> {
    let e = w.get(id)?;
    match &e.class {
        Class::Prop(p) if !p.broken => Some(p.bounds.unwrap_or((e.origin - Vec3::splat(8.0), e.origin + Vec3::splat(8.0)))),
        Class::Breakable(b) if !b.broken => {
            let solid = w.mover_solid(id).filter(|s| !s.is_empty());
            let (mut lo, mut hi) = (Vec3::MAX, Vec3::MIN);
            match solid {
                Some(s) => {
                    for b in s {
                        lo = lo.min(b.min);
                        hi = hi.max(b.max);
                    }
                }
                None => {
                    for p in e.hulls.iter().flat_map(|h| &h.points) {
                        lo = lo.min(*p + e.origin);
                        hi = hi.max(*p + e.origin);
                    }
                }
            }
            (lo.cmple(hi).all()).then_some((lo, hi))
        }
        _ => None,
    }
}

/// One env_fire update (spec 2.3).
fn update(w: &mut LogicWorld, id: EntId) {
    let now = w.now64();
    let Some(f) = fire(w, id) else { return };
    if f.fuel != 0.0 {
        f.fuel -= FIRE_THINK;
        if f.fuel <= 0.0 {
            fade_out(w, id, FUEL_FADE);
            return;
        }
    }
    let h_old = f.heat;
    let rate = if f.attack > 0.0 { f.max_heat / f.attack } else { f.max_heat };
    add_heat(w, id, rate * FIRE_THINK * FIRE_GROWTH_RATE, false);
    let Some(f) = fire(w, id).cloned() else { return };
    let h_new = f.heat;
    let q = (h_old / STRENGTH_REF) * h_new;
    let origin = w.get(id).unwrap().origin;
    let mut damage = 0;
    if f.damage_due <= now {
        let due = now + FIRE_DMG_INTERVAL as f64;
        damage = ((FIRE_DMG_BASE + q * FIRE_DMG_SCALE * f.damage_scale) * FIRE_DMG_INTERVAL).trunc() as i32;
        let f = fire(w, id).unwrap();
        f.damage_due = due;
        f.last_damage = damage;
    }
    let mut neighbours = Vec::new();
    if let Some((full, object)) = damage_boxes(origin, f.size, h_new, f.max_heat) {
        for other in w.ids() {
            if other == id || neighbours.len() >= MAX_NEIGHBOURS {
                continue;
            }
            let Some(e) = w.get(other) else { continue };
            if let Class::Fire(o) = &e.class
                && touches(full, fire_bounds(e.origin, o))
            {
                neighbours.push(other);
            }
        }
        if damage != 0 {
            let centre = origin + Vec3::Z * f.size / 2.0;
            let mut victims = Vec::new();
            for p in &w.players {
                if p.alive && touches(full, (p.origin + p.mins, p.origin + p.maxs)) {
                    victims.push((Victim::Player(p.entity), p.origin + (p.mins + p.maxs) / 2.0));
                }
            }
            for other in w.ids() {
                if other == id {
                    continue;
                }
                if let Some(b) = entity_bounds(w, other)
                    && touches(object, b)
                {
                    victims.push((Victim::Ent(other), (b.0 + b.1) / 2.0));
                }
            }
            for (victim, at) in victims {
                let ignore = match victim {
                    Victim::Ent(e) => Some(e),
                    Victim::Player(_) => None,
                };
                let (frac, start_solid) = trace_line(w, centre, at, ignore);
                if start_solid || frac < 1.0 {
                    continue;
                }
                hurt(w, victim, damage as f32, Who::Ent(id), at, (at - centre).normalize_or_zero(), false);
            }
        }
    }
    let spread = q * FIRE_HEAT_SCALE * FIRE_THINK;
    if !neighbours.is_empty() {
        let share = spread / neighbours.len() as f32;
        for n in neighbours {
            add_heat(w, n, share, true);
        }
    }
    if fire(w, id).is_some_and(|f| f.lit && !f.fading) {
        w.think_in(id, FIRE_THINK);
    }
}

/// Burn damage to a player or an entity.
fn hurt(w: &mut LogicWorld, victim: Victim, amount: f32, by: Who, at: Vec3, dir: Vec3, direct: bool) {
    match victim {
        Victim::Player(p) => w.effects.push(Effect::Burn { target: p, amount }),
        Victim::Ent(e) => {
            let hit = Hit {
                amount,
                kind: DamageKind::Burn,
                attacker: Some(by),
                point: at,
                dir,
                force: 0.0,
                direct,
            };
            if !w.prop_hit(e, hit) {
                w.damage(e, amount, DamageKind::Burn, Some(by), at, dir);
            }
        }
    }
}

/// Fade out over `t` (spec 2.5): no growth or damage, then out.
fn fade_out(w: &mut LogicWorld, id: EntId, t: f32) {
    let Some(f) = fire(w, id) else { return };
    f.fading = true;
    w.think_in(id, t.max(0.0));
}

/// Going out (spec 2.5).
fn go_out(w: &mut LogicWorld, id: EntId) {
    let permanent = w.get(id).is_some_and(|e| e.has_flag(SF_FIRE_DIE_PERMANENT));
    let Some(f) = fire(w, id) else { return };
    f.lit = false;
    f.fading = false;
    f.heat = (f.heat - GO_OUT_PENALTY).min(0.0);
    let delete = f.delete_when_out || permanent;
    w.get_mut(id).unwrap().next_think = None;
    w.fire_output(id, "OnExtinguished", Some(Who::Ent(id)), Value::Void);
    if delete {
        w.kill(id);
    }
}

pub(super) fn think(w: &mut LogicWorld, id: EntId) {
    match w.get(id).map(|e| e.class.clone()) {
        Some(Class::Fire(f)) => {
            if f.fading {
                go_out(w, id);
            } else if f.lit {
                update(w, id);
            }
        }
        Some(Class::FireSource(s)) => {
            if s.on {
                let origin = w.get(id).unwrap().origin;
                heat_fires_near(w, origin, s.radius, s.damage * FIRESOURCE_THINK, 128);
                w.think_in(id, FIRESOURCE_THINK);
            }
        }
        Some(Class::FireSensor(s)) => {
            if s.on {
                sensor_think(w, id, s);
            }
        }
        Some(Class::Flame(f)) => flame_think(w, id, *f),
        _ => {}
    }
}

/// Heat from a helper to every env_fire (lit or not) whose origin is
/// within `radius` of `at` (at most `max`).
fn heat_fires_near(w: &mut LogicWorld, at: Vec3, radius: f32, heat: f32, max: usize) {
    let near: Vec<EntId> = w
        .ids()
        .into_iter()
        .filter(|f| {
            w.get(*f)
                .is_some_and(|e| matches!(e.class, Class::Fire(_)) && e.origin.distance(at) <= radius)
        })
        .take(max)
        .collect();
    for f in near {
        add_heat(w, f, heat, true);
    }
}

fn sensor_interval(s: &FireSensor) -> f32 {
    (s.time / 4.0).max(SENSOR_MIN_THINK)
}

fn sensor_think(w: &mut LogicWorld, id: EntId, s: FireSensor) {
    let origin = w.get(id).unwrap().origin;
    let tau = sensor_interval(&s);
    let sum: f32 = w
        .ids()
        .into_iter()
        .filter_map(|f| {
            let e = w.get(f)?;
            match &e.class {
                Class::Fire(x) if x.lit && x.heat > 0.0 && e.origin.distance(origin) <= s.radius => Some(x.heat),
                _ => None,
            }
        })
        .sum();
    let mut out = None;
    if let Some(Class::FireSensor(x)) = w.get_mut(id).map(|e| &mut e.class) {
        if sum >= x.level {
            x.accumulated += tau;
            if x.accumulated >= x.time && !x.at_level {
                x.at_level = true;
                out = Some("OnHeatLevelStart");
            }
        } else {
            x.accumulated = 0.0;
            if x.at_level {
                x.at_level = false;
                out = Some("OnHeatLevelEnd");
            }
        }
    }
    if let Some(o) = out {
        w.fire_output(id, o, Some(Who::Ent(id)), Value::Void);
    }
    w.think_in(id, tau);
}

/// env_fire, env_firesource and env_firesensor inputs.
pub(super) fn input(w: &mut LogicWorld, id: EntId, input: &str, value: &Value) -> bool {
    match w.get(id).map(|e| e.class.clone()) {
        Some(Class::Fire(f)) => match input {
            "startfire" => start_fire(w, id, true),
            "extinguish" | "extinguishtemporary" => {
                let t = value.to_float().unwrap_or(0.0);
                if input == "extinguish"
                    && let Some(e) = w.get_mut(id)
                {
                    e.spawnflags &= !SF_FIRE_INFINITE;
                }
                fade_out(w, id, t);
            }
            "enable" => fire(w, id).unwrap().enabled = true,
            "disable" => {
                fire(w, id).unwrap().enabled = false;
                if f.lit {
                    go_out(w, id);
                }
            }
            _ => return false,
        },
        Some(Class::FireSource(_)) => match input {
            "enable" => {
                if let Some(Class::FireSource(s)) = w.get_mut(id).map(|e| &mut e.class) {
                    s.on = true;
                }
                w.think_in(id, 0.0);
            }
            "disable" => {
                if let Some(Class::FireSource(s)) = w.get_mut(id).map(|e| &mut e.class) {
                    s.on = false;
                }
                w.get_mut(id).unwrap().next_think = None;
            }
            _ => return false,
        },
        Some(Class::FireSensor(s)) => match input {
            "enable" => {
                if let Some(Class::FireSensor(s)) = w.get_mut(id).map(|e| &mut e.class) {
                    s.on = true;
                    s.accumulated = 0.0;
                    s.at_level = false;
                }
                w.think_in(id, sensor_interval(&s));
            }
            "disable" => {
                if let Some(Class::FireSensor(s)) = w.get_mut(id).map(|e| &mut e.class) {
                    s.on = false;
                    s.accumulated = 0.0;
                    s.at_level = false;
                }
                w.get_mut(id).unwrap().next_think = None;
                if s.at_level {
                    w.fire_output(id, "OnHeatLevelEnd", Some(Who::Ent(id)), Value::Void);
                }
            }
            _ => return false,
        },
        _ => return false,
    }
    true
}

// ------------------------------------------------------- entity flames

/// Where a flame's target is (entity space) and its flame size; None
/// when it is gone.
fn target_place(w: &LogicWorld, target: Who) -> Option<(Vec3, f32)> {
    let size = |lo: Vec3, hi: Vec3| ((hi.x - lo.x + hi.y - lo.y) / 2.0).max(FLAME_MIN_SIZE);
    match target {
        Who::Player(p) => {
            let p = w.player(p)?;
            Some((p.origin, size(p.mins, p.maxs)))
        }
        Who::Ent(id) => {
            let e = w.get(id)?;
            if e.killed {
                return None;
            }
            match &e.class {
                Class::Prop(p) if p.broken => None,
                Class::Prop(p) => Some(match p.bounds {
                    Some((lo, hi)) => ((lo + hi) / 2.0, size(lo, hi)),
                    None => (e.origin, FLAME_MIN_SIZE),
                }),
                _ => Some((e.origin, FLAME_MIN_SIZE)),
            }
        }
    }
}

/// A new flame on `target` burning for `lifetime` s (spec 4.1). The
/// caller marks the target burning.
pub fn create_flame(w: &mut LogicWorld, target: Who, lifetime: f32) -> Option<EntId> {
    let (origin, size) = target_place(w, target)?;
    let id = w.spawn(&[("classname".to_string(), "entityflame".to_string())], Vec::new());
    let now = w.now64();
    let e = w.get_mut(id)?;
    e.origin = origin;
    e.class = Class::Flame(Box::new(Flame {
        target,
        size,
        life_end: now + FLAME_DEFAULT_LIFE as f64,
        stopping: false,
    }));
    w.think_in(id, FLAME_FIRST_THINK);
    w.effects.push(Effect::FlameStart { id, target });
    set_flame_life(w, id, lifetime);
    Some(id)
}

fn set_flame_life(w: &mut LogicWorld, id: EntId, lifetime: f32) {
    let now = w.now64();
    if let Some(Class::Flame(f)) = w.get_mut(id).map(|e| &mut e.class) {
        f.life_end = now + lifetime as f64;
    }
}

/// The flames burning `target`.
pub fn flames_on(w: &LogicWorld, target: Who) -> Vec<EntId> {
    w.ids()
        .into_iter()
        .filter(|id| {
            w.get(*id)
                .is_some_and(|e| !e.killed && matches!(&e.class, Class::Flame(f) if f.target == target))
        })
        .collect()
}

/// Remove the flames on `target` now (it is gone).
pub fn remove_flames_on(w: &mut LogicWorld, target: Who) {
    for f in flames_on(w, target) {
        w.kill(f);
    }
}

/// Whether `target` is marked burning.
pub fn is_burning(w: &LogicWorld, target: Who) -> bool {
    match target {
        Who::Player(p) => w.burning_players.contains(&p),
        Who::Ent(id) => w.prop(id).is_some_and(|p| p.burning),
    }
}

/// Ignite `target` for `lifetime` s unless it is burning (spec 4.2):
/// props only if flammable (they fire OnIgnite); players always (the
/// Ignite input skips the NPC-only rule, spec Q3). Returns whether it
/// caught fire.
pub fn ignite(w: &mut LogicWorld, target: Who, lifetime: f32) -> bool {
    if is_burning(w, target) {
        return false;
    }
    match target {
        Who::Player(p) => {
            if w.player(p).is_none_or(|p| !p.alive) {
                return false;
            }
            w.burning_players.push(p);
            create_flame(w, target, lifetime);
        }
        Who::Ent(id) => {
            let flammable = w.prop(id).is_some_and(|p| !p.broken && p.interactions.iter().any(|i| i == "flammable"));
            if !flammable {
                return false;
            }
            if let Some(Class::Prop(p)) = w.get_mut(id).map(|e| &mut e.class) {
                p.burning = true;
            }
            create_flame(w, target, lifetime);
            w.fire_output(id, "OnIgnite", Some(target), Value::Void);
        }
    }
    true
}

/// The Ignite input family on a model entity or a player.
pub(super) fn ignite_input(w: &mut LogicWorld, target: Who, input: &str, value: &Value) -> bool {
    match input {
        "ignite" | "ignitenumhitboxfires" | "ignitehitboxfirescale" => {
            ignite(w, target, IGNITE_LIFE);
        }
        "ignitelifetime" => {
            let t = value.to_float().unwrap_or(0.0);
            ignite(w, target, IGNITE_LIFE);
            for f in flames_on(w, target) {
                set_flame_life(w, f, t);
            }
        }
        _ => return false,
    }
    true
}

/// A prop set on fire by damage: U(10, 15) s.
pub(super) fn ignite_from_damage(w: &mut LogicWorld, id: EntId) {
    let t = PROP_BURN_TIME.0 + (PROP_BURN_TIME.1 - PROP_BURN_TIME.0) * w.random();
    ignite(w, Who::Ent(id), t);
}

fn stop_burning(w: &mut LogicWorld, target: Who) {
    if let Who::Player(p) = target {
        w.burning_players.retain(|x| *x != p);
    }
}

/// One flame think (spec 4.3).
fn flame_think(w: &mut LogicWorld, id: EntId, f: Flame) {
    if f.stopping {
        w.kill(id);
        return;
    }
    let Some((origin, _)) = target_place(w, f.target) else {
        stop_burning(w, f.target);
        w.kill(id);
        return;
    };
    // A dead player: its flame goes (CS:S's ragdoll copy is the client's).
    if let Who::Player(p) = f.target
        && w.player(p).is_some_and(|p| !p.alive)
    {
        stop_burning(w, f.target);
        w.kill(id);
        return;
    }
    if let Some(e) = w.get_mut(id) {
        e.origin = origin;
    }
    if f.life_end < w.now64() {
        // General.StopBurning cuts the loop; removed 0.5 s later.
        w.effects.push(Effect::AmbientStop { id });
        if let Some(Class::Flame(x)) = w.get_mut(id).map(|e| &mut e.class) {
            x.stopping = true;
        }
        stop_burning(w, f.target);
        w.think_in(id, FLAME_REMOVE_DELAY);
        return;
    }
    radius_burn(w, origin, FLAME_RADIUS_DAMAGE, f.size / 2.0, f.target, Who::Ent(id));
    let victim = match f.target {
        Who::Player(p) => Victim::Player(p),
        Who::Ent(e) => Victim::Ent(e),
    };
    hurt(w, victim, FLAME_DIRECT, Who::Ent(id), origin, Vec3::NEG_Z, true);
    heat_fires_near(w, origin, f.size / 2.0, FLAME_HEAT, usize::MAX);
    if w.get(id).is_some_and(|e| !e.killed) {
        w.think_in(id, FLAME_THINK);
    }
}

/// Burn radius damage from `at` (spec 4.3 step 5): falls off linearly to
/// 0 at `radius`; a player's target point is at 85 % of its eye height
/// (CS:S's radius damage aims at 70-100 %, grenades.md 5.3), so a small
/// flame never reaches one; props by their centre. Blocked by the world.
fn radius_burn(w: &mut LogicWorld, at: Vec3, damage: f32, radius: f32, ignore: Who, by: Who) {
    if radius <= 0.0 {
        return;
    }
    let mut hits = Vec::new();
    for p in &w.players {
        if !p.alive || ignore == Who::Player(p.entity) {
            continue;
        }
        let point = p.origin + p.eye * 0.85;
        let d = point.distance(at);
        if d < radius {
            hits.push((Victim::Player(p.entity), point, damage * (1.0 - d / radius)));
        }
    }
    for id in w.ids() {
        if ignore == Who::Ent(id) || !matches!(w.get(id).map(|e| &e.class), Some(Class::Prop(_))) {
            continue;
        }
        let Some((lo, hi)) = entity_bounds(w, id) else { continue };
        let point = (lo + hi) / 2.0;
        let d = point.distance(at);
        if d < radius {
            hits.push((Victim::Ent(id), point, damage * (1.0 - d / radius)));
        }
    }
    for (victim, point, amount) in hits {
        let ignore = match victim {
            Victim::Ent(e) => Some(e),
            Victim::Player(_) => None,
        };
        let (frac, start_solid) = trace_line(w, at, point, ignore);
        if start_solid || frac < 1.0 || amount <= 0.0 {
            continue;
        }
        hurt(w, victim, amount, by, point, (point - at).normalize_or_zero(), false);
    }
}

impl LogicWorld {
    /// Current time in seconds, double precision.
    pub fn now64(&self) -> f64 {
        self.tick as f64 * self.dt as f64
    }

    /// An env_fire's state.
    pub fn fire(&self, id: EntId) -> Option<&Fire> {
        match self.get(id).map(|e| &e.class) {
            Some(Class::Fire(f)) => Some(f),
            _ => None,
        }
    }

    /// A flame's state.
    pub fn flame(&self, id: EntId) -> Option<&Flame> {
        match self.get(id).map(|e| &e.class) {
            Some(Class::Flame(f)) => Some(f),
            _ => None,
        }
    }

    /// Lit fires (their effect exists), for the host to draw.
    pub fn fire_looks(&self) -> Vec<FireLook> {
        self.ids()
            .into_iter()
            .filter_map(|id| {
                let e = self.get(id)?;
                let Class::Fire(f) = &e.class else { return None };
                (f.lit && !e.killed).then(|| FireLook {
                    id,
                    serial: f.serial,
                    at: e.origin,
                    size: f.size,
                    smokeless: e.has_flag(SF_FIRE_SMOKELESS),
                    plasma: f.plasma,
                })
            })
            .collect()
    }

    /// Burning flames (not stopped), for the host to draw.
    pub fn flame_looks(&self) -> Vec<FlameLook> {
        self.ids()
            .into_iter()
            .filter_map(|id| {
                let e = self.get(id)?;
                let Class::Flame(f) = &e.class else { return None };
                (!f.stopping && !e.killed).then(|| FlameLook {
                    id,
                    target: f.target,
                    size: f.size,
                })
            })
            .collect()
    }

    /// Ignite an entity or player (the Ignite input), for hosts and tests.
    pub fn ignite(&mut self, target: Who, lifetime: f32) -> bool {
        ignite(self, target, lifetime)
    }
}
