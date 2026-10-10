//! A map logic audit (docs/plans/active/community-maps.md, "Map logic
//! audit"; `mapsweep --audit`): what a map's entities ask of the logic
//! that it doesn't do.
//!
//! Static: every output connection's target is resolved (map names,
//! template members, `!activator`-style names) and its input is delivered
//! once per target class to a probe world, which logs inputs no class
//! handles. Dynamic: the map runs with one scripted player who visits
//! every trigger (standing in it a few ticks) and presses every usable
//! brush within reach (+use from four sides), then a round restart; what
//! the logic logs, movers told to move that never did, and panics are
//! collected.

use std::collections::{BTreeMap, BTreeSet};
use std::panic::{AssertUnwindSafe, catch_unwind};

use bevy::prelude::*;

use super::classes::Class;
use super::movers::pusher;
use super::value::Value;
use super::world::{EntId, LogicWorld, NoCollision, Player, Who, place_hull};
use crate::map::MapEntity;
use crate::map::entities::entity_rotation;

/// Inputs that tell a mover to move (for the "never moved" check).
const MOVE_INPUTS: &[&str] = &[
    "open",
    "close",
    "toggle",
    "start",
    "startforward",
    "startbackward",
    "setposition",
    "press",
    "pressin",
    "setspeed",
    "setspeedreal",
    "resume",
    "reverse",
];

/// Brush classes with no node of their own whose inputs don't need one
/// (volumes the game reads, and classes with their own handling).
const BAKED_OK: &[&str] = &[
    "func_buyzone",
    "func_bomb_target",
    "func_hostage_rescue",
    "func_areaportal",
    "func_areaportalwindow",
    "func_occluder",
    "func_precipitation",
    "func_dustmotes",
    "func_dustcloud",
    "func_smokevolume",
];

/// One map's audit.
#[derive(Clone, Debug, Default)]
pub struct MapAudit {
    /// Output connections in the map (template members' included).
    pub connections: usize,
    /// Targets no entity, template member or special name matches:
    /// name -> connections.
    pub missing_targets: BTreeMap<String, usize>,
    /// (target class, input) -> connections whose target class doesn't
    /// handle the input. The class is `player` for inputs to players
    /// (`!activator`, `!player`...); AddOutput of a keyvalue reads
    /// `addoutput <key>`.
    pub unsupported: BTreeMap<(String, String), usize>,
    /// (target class, input) -> connections that work.
    pub supported: BTreeMap<(String, String), usize>,
    /// Special target names (`!activator`...) -> connections.
    pub special: BTreeMap<String, usize>,
    /// (class, input) -> connections to brush entities the map loader
    /// baked into the world (no node of their own): what the input does
    /// to their look or collision doesn't show.
    pub static_brushes: BTreeMap<(String, String), usize>,
    /// Connections whose target has a `*` wildcard.
    pub wildcards: usize,
    /// (class, output) -> connections, for outputs the map connects.
    pub outputs: BTreeMap<(String, String), usize>,
    /// (class, keyvalue key) -> entities, for every key the map sets.
    pub keys: BTreeMap<(String, String), usize>,
    /// (class, spawnflag bit) -> entities.
    pub flags: BTreeMap<(String, u32), usize>,
    /// What the logic logged while the map ran.
    pub log: Vec<String>,
    /// Panics (probe or run).
    pub panics: Vec<String>,
    /// Movers told to move that didn't: "class name: input".
    pub stuck: Vec<String>,
    /// Movers that moved much further in one tick than their speed takes
    /// them (snapped to a goal): "class name: how far".
    pub jumps: Vec<String>,
    /// Usable brushes +use didn't find from any side: "class name".
    pub missed: Vec<String>,
    /// Entities parented to a mover that stay where they spawned:
    /// class -> entities.
    pub left_behind: BTreeMap<String, usize>,
    /// Triggers visited, usable brushes pressed / not found by +use.
    pub triggers_visited: usize,
    pub buttons_pressed: usize,
    pub buttons_missed: usize,
    /// Outputs that fired while the map ran: (class, output).
    pub fired: BTreeSet<(String, String)>,
    /// Ticks run.
    pub ticks: i64,
}

/// What a probe of one input on one class found.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Probe {
    Supported,
    Unsupported,
}

/// Whether a log line says an input or keyvalue did nothing.
fn unsupported_line(l: &str) -> bool {
    l.contains("unhandled input") || l.contains("not supported") || l.contains("has no effect")
}

/// Audit a map's entities: the static checks, then `secs` seconds (at
/// least long enough for every visit) of the scripted run.
pub fn audit_map(entities: &[MapEntity], secs: f32) -> MapAudit {
    let mut a = MapAudit::default();
    for e in entities {
        let class = e.classname().to_ascii_lowercase();
        let mut seen = BTreeSet::new();
        for (k, _) in &e.keyvalues {
            let k = k.split('#').next().unwrap_or("").to_ascii_lowercase();
            if super::world::is_output_key(&k) || !seen.insert(k.clone()) {
                continue;
            }
            *a.keys.entry((class.clone(), k)).or_default() += 1;
        }
        let flags = e.get("spawnflags").map_or(0, super::value::atoi) as u32;
        for bit in 0..32 {
            if flags & (1 << bit) != 0 {
                *a.flags.entry((class.clone(), 1 << bit)).or_default() += 1;
            }
        }
    }
    if let Err(p) = catch_unwind(AssertUnwindSafe(|| static_pass(entities, &mut a))) {
        a.panics.push(format!("probe: {}", panic_text(&p)));
    }
    if let Err(p) = catch_unwind(AssertUnwindSafe(|| run(entities, secs, &mut a))) {
        a.panics.push(format!("run: {}", panic_text(&p)));
    }
    a
}

fn panic_text(p: &Box<dyn std::any::Any + Send>) -> String {
    p.downcast_ref::<String>()
        .cloned()
        .or_else(|| p.downcast_ref::<&str>().map(|s| s.to_string()))
        .unwrap_or_else(|| "(no message)".into())
}

/// Every connection, with the entity that has it: (source id, output,
/// target, input, parameter).
type Link = (EntId, String, String, String, Option<String>);

fn static_pass(entities: &[MapEntity], a: &mut MapAudit) {
    let mut w = LogicWorld::new(1.0 / 66.0);
    w.load_map(entities);
    let player = Entity::from_raw_u32(1).unwrap();
    w.players.push(Player::new(player, Vec3::ZERO));
    // Template members: spawned into the probe world as they would be
    // (names unfixed), so their connections and inputs are checked too.
    for id in w.ids() {
        if let Some(Class::Template(t)) = w.get(id).map(|e| e.class.clone()) {
            for m in &t.members {
                let kv: Vec<(String, String)> = m
                    .keyvalues
                    .iter()
                    .map(|(k, v)| (k.clone(), v.replace(super::templates::FIXUP_MARKER, "")))
                    .collect();
                w.spawn(&kv, m.hulls.clone());
            }
        }
    }
    let mut links: Vec<Link> = Vec::new();
    for id in w.ids() {
        let e = w.get(id).unwrap();
        for (output, conns) in &e.outputs {
            for c in conns {
                links.push((id, output.clone(), c.target.clone(), c.input.clone(), c.param.clone()));
            }
        }
    }
    let mut cache: BTreeMap<(String, String), Probe> = BTreeMap::new();
    for (source, output, target, input, param) in links {
        a.connections += 1;
        let source_class = w
            .get(source)
            .map(|e| e.classname.to_ascii_lowercase())
            .unwrap_or_default();
        *a.outputs.entry((source_class, output)).or_default() += 1;
        if target.contains('*') {
            a.wildcards += 1;
        }
        let targets: Vec<Who> = if let Some(special) = target.strip_prefix('!') {
            *a.special
                .entry(format!("!{}", special.to_ascii_lowercase()))
                .or_default() += 1;
            match special.to_ascii_lowercase().as_str() {
                // Activators are players nearly always (the scripted run
                // sees the rest).
                "activator" | "player" | "pvsplayer" | "picker" => vec![Who::Player(player)],
                "self" | "caller" => vec![Who::Ent(source)],
                _ => Vec::new(),
            }
        } else {
            w.resolve(&target, Some(Who::Player(player)), Some(Who::Ent(source)))
        };
        if targets.is_empty() {
            *a.missing_targets.entry(target.to_ascii_lowercase()).or_default() += 1;
            continue;
        }
        let mut classes = BTreeSet::new();
        for t in targets {
            let class = w.class_of(t).to_ascii_lowercase();
            if !classes.insert(class.clone()) {
                continue;
            }
            let lower = input.to_ascii_lowercase();
            // AddOutput: by the key it sets.
            let label = match (lower.as_str(), &param) {
                ("addoutput", Some(p)) => {
                    let key = p.split_whitespace().next().unwrap_or("").to_ascii_lowercase();
                    if super::world::is_output_key(&key) {
                        "addoutput <output>".to_string()
                    } else {
                        format!("addoutput {key}")
                    }
                }
                _ => lower.clone(),
            };
            if let Who::Ent(tid) = t
                && let Some(index) = w.get(tid).and_then(|e| e.map_index)
                && let Some(me) = entities.get(index)
                && me.get("model").is_some_and(|m| m.starts_with('*'))
                && !me.mover
                && me.hulls.is_empty()
                && !BAKED_OK.contains(&class.as_str())
            {
                *a.static_brushes.entry((class.clone(), label.clone())).or_default() += 1;
            }
            let key = (class.clone(), label.clone());
            let result = match cache.get(&key) {
                Some(r) => *r,
                None => {
                    let r = probe(&mut w, t, &input, param.as_deref(), source);
                    cache.insert(key.clone(), r);
                    r
                }
            };
            let map = if result == Probe::Supported {
                &mut a.supported
            } else {
                &mut a.unsupported
            };
            *map.entry(key).or_default() += 1;
        }
    }
}

/// Deliver one input to `target` in the probe world and read what it
/// logged.
fn probe(w: &mut LogicWorld, target: Who, input: &str, param: Option<&str>, source: EntId) -> Probe {
    let before = w.log.len();
    let value = param.map_or(Value::Void, |p| Value::Str(p.to_string()));
    let player = w.players.first().map(|p| Who::Player(p.entity));
    let ok = catch_unwind(AssertUnwindSafe(|| {
        w.deliver(target, input, value, player, Some(Who::Ent(source)));
    }));
    if ok.is_err() {
        return Probe::Unsupported;
    }
    let bad = w.log[before..].iter().any(|l| unsupported_line(l));
    w.log.truncate(before);
    w.effects.clear();
    if bad { Probe::Unsupported } else { Probe::Supported }
}

/// A place the scripted player visits.
enum Visit {
    /// Stand in a trigger: the player's origin.
    Trigger(Vec3),
    /// Press use at a usable brush.
    Use(EntId),
}

/// The centre of an entity's volumes (entity space), or its origin.
fn centre(w: &LogicWorld, id: EntId) -> Vec3 {
    let Some(e) = w.get(id) else { return Vec3::ZERO };
    let (origin, angles) = match pusher(&e.class) {
        Some(p) => (p.origin, p.angles),
        None => (e.origin, e.angles),
    };
    let rot = entity_rotation(angles);
    let mut min = Vec3::splat(f32::MAX);
    let mut max = Vec3::splat(f32::MIN);
    for h in &e.hulls {
        let b = place_hull(h, rot, origin);
        min = min.min(b.min);
        max = max.max(b.max);
    }
    if min.x > max.x { origin } else { (min + max) / 2.0 }
}

fn run(entities: &[MapEntity], secs: f32, a: &mut MapAudit) {
    let mut w = LogicWorld::new(1.0 / 66.0);
    w.record = true;
    w.load_map(entities);
    // Children of movers that don't follow them.
    for id in w.ids() {
        let e = w.get(id).unwrap();
        let Some(parent) = e
            .kv("parentname")
            .map(crate::map::entities::parent_name)
            .filter(|p| !p.is_empty())
        else {
            continue;
        };
        let moving = w
            .find(parent)
            .and_then(|p| w.get(p))
            .is_some_and(|p| pusher(&p.class).is_some() || crate::map::entities::anchor_class(&p.classname));
        let follows = super::movers::attached(&e.class).is_some_and(|a| a.parent.is_some())
            || matches!(e.class, Class::Prop(_) | Class::PropDoor(_))
            || w.follows_anchor(id);
        if moving && !follows {
            *a.left_behind.entry(e.classname.to_ascii_lowercase()).or_default() += 1;
        }
    }
    let spawn = entities
        .iter()
        .find(|e| {
            let c = e.classname().to_ascii_lowercase();
            c == "info_player_terrorist" || c == "info_player_counterterrorist" || c == "info_player_start"
        })
        .map_or(Vec3::ZERO, |e| e.origin());
    let me = Entity::from_raw_u32(1).unwrap();
    let mut p = Player::new(me, spawn);
    p.team = 2;
    w.players.push(p);
    let mut visits = Vec::new();
    for id in w.ids() {
        let e = w.get(id).unwrap();
        match &e.class {
            Class::Trigger(_) => visits.push(Visit::Trigger(centre(&w, id) - Vec3::new(0.0, 0.0, 36.0))),
            Class::Button(_) | Class::Door(_) | Class::Rotating(_) | Class::PropDoor(_) => visits.push(Visit::Use(id)),
            _ => {}
        }
    }
    let mut tr = Tracker {
        quiet_until: w.tick + 33,
        ..Tracker::default()
    };
    let total = ((secs / w.dt) as i64).max(visits.len() as i64 * 12 + 66 * 3);
    let mut next = 0usize;
    let mut step = 0;
    let set_player = |w: &mut LogicWorld, at: Vec3, view: Vec3, use_key: bool| {
        if let Some(p) = w.player_mut(me) {
            p.origin = at;
            p.view = view;
            p.use_key = use_key;
            p.alive = true;
            p.velocity = Vec3::ZERO;
            p.on_ground = true;
        }
    };
    // Warm up: logic_auto and timers.
    for t in 0..total {
        // The visit schedule: 12 ticks a visit (2 settle, then up to 4
        // use attempts of 2 ticks, or 4 ticks in a trigger and out).
        if t >= 66 && next < visits.len() {
            match &visits[next] {
                Visit::Trigger(at) => {
                    if step < 4 {
                        set_player(&mut w, *at, Vec3::ZERO, false);
                    } else {
                        set_player(&mut w, spawn, Vec3::ZERO, false);
                    }
                    if step == 0 {
                        a.triggers_visited += 1;
                    }
                    step += 1;
                    if step >= 6 {
                        step = 0;
                        next += 1;
                    }
                }
                Visit::Use(id) => {
                    let id = *id;
                    let c = centre(&w, id);
                    let attempt = step / 2;
                    let dirs = [Vec3::X, -Vec3::X, Vec3::Y, -Vec3::Y];
                    let dir = dirs[attempt.min(3)];
                    let eye = c + dir * 40.0;
                    let look = c - eye;
                    let yaw = look.y.atan2(look.x).to_degrees();
                    let pitch = -(look.z / look.length().max(1e-3)).asin().to_degrees();
                    let press = step % 2 == 1;
                    set_player(
                        &mut w,
                        eye - Vec3::new(0.0, 0.0, 64.0),
                        Vec3::new(pitch, yaw, 0.0),
                        press,
                    );
                    step += 1;
                    w.frame(&NoCollision);
                    let found = w.use_presses.iter().any(|(_, f)| *f);
                    if press && found {
                        a.buttons_pressed += 1;
                        step = 0;
                        next += 1;
                    } else if step >= 8 {
                        a.buttons_missed += 1;
                        if let Some(e) = w.get(id) {
                            a.missed.push(format!("{} '{}'", e.classname, e.targetname));
                        }
                        step = 0;
                        next += 1;
                    }
                    tr.track(&mut w, a);
                    continue;
                }
            }
        } else if let Some(p) = w.player_mut(me) {
            p.use_key = false;
        }
        w.frame(&NoCollision);
        tr.track(&mut w, a);
    }
    // A round restart, then two seconds more.
    w.round_restart(entities);
    // Everything is back where it spawned: not a jump.
    tr.last.clear();
    tr.quiet_until = w.tick + 33;
    for _ in 0..132 {
        w.frame(&NoCollision);
        tr.track(&mut w, a);
    }
    a.ticks = w.tick;
    for ((index, generation), (input, _, _, moved)) in tr.told {
        if !moved {
            let id = EntId { index, generation };
            let (class, name) = w
                .get(id)
                .map(|e| (e.classname.clone(), e.targetname.clone()))
                .unwrap_or_default();
            if !class.is_empty() {
                a.stuck.push(format!("{class} '{name}': {input}"));
            }
        }
    }
    a.log = std::mem::take(&mut w.log);
}

/// What the scripted run watches tick by tick.
#[derive(Default)]
struct Tracker {
    /// Movers told to move: id -> (input, where it was, angles, moved
    /// since).
    told: BTreeMap<(u32, u32), (String, Vec3, Vec3, bool)>,
    /// Every mover's place last tick.
    last: BTreeMap<(u32, u32), (Vec3, f32)>,
    /// No jumps reported before this tick (trains placing themselves on
    /// their path after a (re)start).
    quiet_until: i64,
}

impl Tracker {
    /// After a tick: record fired outputs, movers told to move, whether
    /// they moved, and movers that jumped (moved much further in a tick
    /// than their speed takes them).
    fn track(&mut self, w: &mut LogicWorld, a: &mut MapAudit) {
        let fired: Vec<_> = w.fired.drain(..).collect();
        for (_, id, output) in fired {
            let class = w.get(id).map(|e| e.classname.to_ascii_lowercase()).unwrap_or_default();
            a.fired.insert((class, output.to_ascii_lowercase()));
        }
        let new: Vec<_> = w.deliveries.drain(..).collect();
        let mut placed = BTreeSet::new();
        for d in new {
            let Who::Ent(id) = d.target else { continue };
            let input = d.input.to_ascii_lowercase();
            if input == "addoutput" || input.starts_with("teleport") {
                placed.insert((id.index, id.generation));
            }
            if !MOVE_INPUTS.contains(&input.as_str()) {
                continue;
            }
            let Some(p) = w.get(id).and_then(|e| pusher(&e.class)) else {
                continue;
            };
            // Brushes that toggle in place and buttons don't count; nor a
            // mover already where it was sent (Close on a closed door).
            if matches!(w.get(id).map(|e| &e.class), Some(Class::Brush(_) | Class::Button(_))) {
                continue;
            }
            let pending = p.goal_origin.is_some_and(|g| g.distance(p.origin) > 0.01)
                || p.goal_angles.is_some_and(|g| (g - p.angles).abs().max_element() > 0.01)
                || p.velocity != Vec3::ZERO
                || p.avelocity != Vec3::ZERO
                || matches!(w.get(id).map(|e| &e.class), Some(Class::Rotating(_)));
            if pending {
                self.told
                    .entry((id.index, id.generation))
                    .or_insert((d.input.clone(), p.origin, p.angles, false));
            }
        }
        for ((index, generation), (_, o, ang, moved)) in self.told.iter_mut() {
            if *moved {
                continue;
            }
            let id = EntId {
                index: *index,
                generation: *generation,
            };
            if let Some(p) = w.get(id).and_then(|e| pusher(&e.class))
                && (p.origin.distance(*o) > 0.01
                    || (p.angles - *ang).abs().max_element() > 0.01
                    || p.velocity != Vec3::ZERO
                    || p.avelocity != Vec3::ZERO)
            {
                *moved = true;
            }
        }
        for id in w.ids() {
            let Some(e) = w.get(id) else { continue };
            let Some(p) = pusher(&e.class) else { continue };
            let key = (id.index, id.generation);
            if let Some((last, speed)) = self.last.insert(key, (p.origin, p.velocity.length()))
                && !placed.contains(&key)
                && w.tick >= self.quiet_until
                && !w.follows_anchor(id)
            {
                let moved = p.origin.distance(last);
                if moved > 16.0 + p.velocity.length().max(speed) * w.dt * 2.0 && a.jumps.len() < 50 {
                    a.jumps.push(format!(
                        "{} '{}' moved {moved:.0} units in a tick",
                        e.classname, e.targetname
                    ));
                }
            }
        }
        w.effects.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ent(pairs: &[(&str, &str)]) -> MapEntity {
        MapEntity {
            keyvalues: pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect(),
            ..Default::default()
        }
    }

    #[test]
    fn audit_finds_missing_targets_and_unhandled_inputs() {
        let ents = [
            ent(&[("classname", "info_player_terrorist"), ("origin", "0 0 0")]),
            ent(&[("classname", "info_target"), ("targetname", "t")]),
            ent(&[("classname", "math_counter"), ("targetname", "c")]),
            ent(&[
                ("classname", "logic_auto"),
                ("OnMapSpawn", "nobody,Trigger,,0,-1"),
                ("OnMapSpawn", "t,Frobnicate,,0,-1"),
                ("OnMapSpawn", "c,Add,1,0,-1"),
                ("OnMapSpawn", "!activator,SetHealth,50,0,-1"),
            ]),
        ];
        let a = audit_map(&ents, 1.0);
        assert_eq!(a.connections, 4);
        assert_eq!(a.missing_targets.get("nobody"), Some(&1));
        assert_eq!(
            a.unsupported.get(&("info_target".into(), "frobnicate".into())),
            Some(&1)
        );
        assert_eq!(a.supported.get(&("math_counter".into(), "add".into())), Some(&1));
        assert_eq!(a.supported.get(&("player".into(), "sethealth".into())), Some(&1));
        assert!(a.fired.contains(&("logic_auto".into(), "onmapspawn".into())));
        assert!(a.panics.is_empty());
        assert!(a.log.iter().any(|l| l.contains("frobnicate")), "{:?}", a.log);
    }
}
