//! The entity I/O core (specs/source/entity_io.md): named entities,
//! outputs and their connections, the time-ordered event queue, target
//! matching and the inputs every entity has. Plain data, no ECS: the Bevy
//! side (`bridge`) hands it the players each tick and applies its
//! `Effect`s, so tests drive it directly.
//!
//! Positions are in entity space (`crate::map::entities`): the map's own
//! units, Z up. Time is in ticks of `dt` seconds.

use std::collections::VecDeque;
use std::sync::Arc;

use bevy::prelude::*;

use super::classes::Class;
use super::value::{Value, atof, atoi};
use crate::map::{MapBrush, MapHull};

/// A logic entity: index and generation (a killed entity's id never
/// comes back).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct EntId {
    pub index: u32,
    pub generation: u32,
}

/// Something that can activate or receive inputs: a logic entity or a
/// player (an ECS character).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Who {
    Ent(EntId),
    Player(Entity),
}

/// One output connection (`target,input,parameter,delay,timestofire`).
#[derive(Clone, Debug, PartialEq)]
pub struct Connection {
    pub target: String,
    pub input: String,
    /// None: pass the output's own value.
    pub param: Option<String>,
    pub delay: f32,
    /// Deliveries left; -1 unlimited.
    pub times: i32,
    pub id: u32,
}

/// Split a connection value: ESC-separated when it has one, else commas.
/// Returns (target, input, parameter, delay, times).
pub fn parse_connection(value: &str) -> Option<(String, String, Option<String>, f32, i32)> {
    let parts: Vec<&str> = if value.contains('\u{1b}') {
        value.split('\u{1b}').collect()
    } else {
        value.split(',').collect()
    };
    if parts.len() < 2 {
        return None;
    }
    let field = |i: usize| parts.get(i).copied().unwrap_or("");
    let input = if field(1).is_empty() { "Use" } else { field(1) };
    let param = (!field(2).is_empty()).then(|| field(2).to_string());
    let times = match atoi(field(4)) {
        0 | -1 => -1,
        n if n < 0 => -1,
        n => n,
    };
    Some((field(0).to_string(), input.to_string(), param, atof(field(3)), times))
}

/// Outputs whose names don't start with "On": math_counter's OutValue,
/// env_global's OutCounter and func_bomb_target's bomb outputs.
pub const OTHER_OUTPUTS: &[&str] = &["outvalue", "outcounter", "bombexplode", "bombplanted", "bombdefused"];

/// Whether a keyvalue is an output: output names start with "On" (plus
/// `OTHER_OUTPUTS`) and carry a connection.
pub fn is_output_key(key: &str) -> bool {
    let k = key.to_ascii_lowercase();
    (k.starts_with("on") && k.len() > 2) || OTHER_OUTPUTS.contains(&k.as_str())
}

/// Name match (entity_io.md "Match rule"): case-insensitive from the
/// start; a `*` in the query matches the rest.
pub fn name_matches(query: &str, name: &str) -> bool {
    let (q, n) = (query.as_bytes(), name.as_bytes());
    let mut i = 0;
    loop {
        if i < q.len() && q[i] == b'*' {
            return true;
        }
        if i == q.len() || i == n.len() {
            return i == q.len() && i == n.len();
        }
        if !q[i].eq_ignore_ascii_case(&n[i]) {
            return false;
        }
        i += 1;
    }
}

/// A player as the logic sees it this tick; the host fills these before
/// each phase and reads back what changed.
#[derive(Clone, Debug)]
pub struct Player {
    pub entity: Entity,
    /// Origin and box (relative to it), entity space.
    pub origin: Vec3,
    pub mins: Vec3,
    pub maxs: Vec3,
    /// Eye position relative to the origin.
    pub eye: Vec3,
    pub velocity: Vec3,
    pub base_velocity: Vec3,
    /// A push set the base velocity this tick.
    pub base_touched: bool,
    /// Taken off the ground (push, teleport): movement must not snap back.
    pub unground: bool,
    pub on_ground: bool,
    /// The mover it stands on.
    pub ground: Option<EntId>,
    /// View angles (pitch, yaw, roll), degrees, pitch down positive.
    pub view: Vec3,
    pub alive: bool,
    pub team: u8,
    /// Gravity multiplier (trigger_gravity).
    pub gravity: f32,
    /// Use key held this tick.
    pub use_key: bool,
    /// Teleported: the host re-places it and its movement re-checks the
    /// ground.
    pub teleported: bool,
    /// Moved by the logic (carried, pushed): the host re-places it.
    pub moved: bool,
}

impl Player {
    pub fn new(entity: Entity, origin: Vec3) -> Self {
        Self {
            entity,
            origin,
            mins: Vec3::new(-16.0, -16.0, 0.0),
            maxs: Vec3::new(16.0, 16.0, 72.0),
            eye: Vec3::new(0.0, 0.0, 64.0),
            velocity: Vec3::ZERO,
            base_velocity: Vec3::ZERO,
            base_touched: false,
            unground: false,
            on_ground: true,
            ground: None,
            view: Vec3::ZERO,
            alive: true,
            team: 0,
            gravity: 1.0,
            use_key: false,
            teleported: false,
            moved: false,
        }
    }
}

/// What the logic asks the host to do.
#[derive(Clone, Debug, PartialEq)]
pub enum Effect {
    /// Health points (100 = a standard player). `crush`: crush damage.
    Damage {
        target: Entity,
        amount: f32,
        crush: bool,
    },
    /// Burn damage to a player (fire.rs: env_fire, entity flames), health
    /// points.
    Burn { target: Entity, amount: f32 },
    /// A flame started burning `target` (fire.rs): the host loops its
    /// sound on the target (`fire::BURNING_SOUND`, keyed by the flame) and
    /// stops it with `AmbientStop` when the flame ends.
    FlameStart { id: EntId, target: Who },
    /// Heal up to max health.
    Heal { target: Entity, amount: f32 },
    SetHealth { target: Entity, health: f32 },
    /// A sound entry (or file) at a point (entity space).
    Sound { entry: String, at: Vec3 },
    /// Start an entity's long-lived sound (ambient_generic), replacing
    /// the one it plays: at `at` (entity space), or following `source`
    /// when it names one. Volume (0..1), pitch (percent) and level (dB)
    /// override the entry's when given.
    AmbientStart {
        id: EntId,
        entry: String,
        at: Vec3,
        source: Option<EntId>,
        volume: Option<f32>,
        pitch: Option<f32>,
        level: Option<f32>,
    },
    /// New volume/pitch for the entity's playing sound.
    AmbientChange {
        id: EntId,
        volume: Option<f32>,
        pitch: Option<f32>,
    },
    /// Stop the entity's sound.
    AmbientStop { id: EntId },
    /// A HUD message for everyone (`to` None) or one player.
    GameText {
        to: Option<Entity>,
        message: super::hud::HudMessage,
    },
    /// An allowed server command (point_servercommand), already checked.
    ServerCommand(String),
    /// An allowed client command for one player (point_clientcommand).
    ClientCommand { player: Entity, command: String },
    /// Gibs thrown by a breaking brush: a gib list name (propdata
    /// "BreakableModels", or a model) and the pieces (entity space).
    Gibs {
        set: String,
        glass: bool,
        pieces: Vec<super::breakables::Gib>,
    },
    /// A prop broke (spec prop_damage.md 7): the host plays its break
    /// sound where it is, explodes it, throws its pieces.
    PropBreak {
        id: EntId,
        sound: Option<String>,
        explode: Option<super::prop_damage::PropExplosion>,
    },
    /// Something for a physics prop's body (enable/disable motion, wake,
    /// sleep).
    PropMotion {
        id: EntId,
        motion: super::prop_damage::Motion,
    },
    /// A window pane shattered: its centre and normal (entity space), its
    /// size (units) and the shards' push (units/s).
    PaneShatter {
        at: Vec3,
        normal: Vec3,
        size: Vec2,
        velocity: Vec3,
        tile: bool,
    },
}

/// Static collision the logic needs (the world without movers), entity
/// space. Movers are the logic's own.
pub trait Collision {
    /// How far (0..1) a box (`mins`/`maxs` around the origin) gets moving
    /// its origin from `from` to `to`.
    fn sweep(&self, mins: Vec3, maxs: Vec3, from: Vec3, to: Vec3) -> f32;
    /// Whether the box at `at` is inside something solid.
    fn solid(&self, mins: Vec3, maxs: Vec3, at: Vec3) -> bool;
}

/// No static collision (open space).
pub struct NoCollision;

impl Collision for NoCollision {
    fn sweep(&self, _: Vec3, _: Vec3, _: Vec3, _: Vec3) -> f32 {
        1.0
    }
    fn solid(&self, _: Vec3, _: Vec3, _: Vec3) -> bool {
        false
    }
}

/// Static collision from brushes in entity space (tests, simple hosts).
pub struct BrushCollision(pub Vec<MapBrush>);

/// Hits stop this far short, entity units (Source: DIST_EPSILON).
pub const SWEEP_EPS: f32 = 0.03125;
/// A box must be this far inside a solid to count as stuck.
pub const SOLID_SKIN: f32 = 0.01;

impl Collision for BrushCollision {
    fn sweep(&self, mins: Vec3, maxs: Vec3, from: Vec3, to: Vec3) -> f32 {
        sweep_brushes(self.0.iter(), mins, maxs, from, to)
    }
    fn solid(&self, mins: Vec3, maxs: Vec3, at: Vec3) -> bool {
        solid_brushes(self.0.iter(), mins, maxs, at)
    }
}

pub(super) fn sweep_brushes<'a>(
    brushes: impl Iterator<Item = &'a MapBrush>,
    mins: Vec3,
    maxs: Vec3,
    from: Vec3,
    to: Vec3,
) -> f32 {
    let (half, off) = ((maxs - mins) / 2.0, (maxs + mins) / 2.0);
    brushes
        .filter_map(|b| b.sweep_box(half, from + off, to + off, SWEEP_EPS))
        .map(|(f, _)| f)
        .fold(1.0, f32::min)
}

pub(super) fn solid_brushes<'a>(brushes: impl Iterator<Item = &'a MapBrush>, mins: Vec3, maxs: Vec3, at: Vec3) -> bool {
    let (half, off) = ((maxs - mins) / 2.0, (maxs + mins) / 2.0);
    brushes.into_iter().any(|b| b.overlaps_box(at + off, half, SOLID_SKIN))
}

/// A box overlaps a brush with positive volume (triggers; tolerance 0).
pub fn box_touches(brush: &MapBrush, mins: Vec3, maxs: Vec3) -> bool {
    if brush.max.cmple(mins).any() || brush.min.cmpge(maxs).any() {
        return false;
    }
    let (centre, half) = ((mins + maxs) / 2.0, (maxs - mins) / 2.0);
    brush.planes.iter().all(|(n, d)| n.dot(centre) - n.abs().dot(half) < *d)
}

/// A hull as a brush (planes and bounds), moved by `rotation` then
/// `offset`.
pub fn place_hull(hull: &MapHull, rotation: Quat, offset: Vec3) -> MapBrush {
    let base = MapBrush {
        planes: hull.planes.clone(),
        min: Vec3::ZERO,
        max: Vec3::ZERO,
        ladder: false,
        surface: None,
    };
    base.transformed(&hull.points, rotation, offset)
}

/// A logic entity.
#[derive(Clone, Debug)]
pub struct LogicEntity {
    pub classname: String,
    pub targetname: String,
    pub spawnflags: u32,
    /// Keyvalues as spawned (outputs left out).
    pub keyvalues: Vec<(String, String)>,
    /// Output name (lower-case) -> connections, newest first.
    pub outputs: Vec<(String, Vec<Connection>)>,
    pub origin: Vec3,
    pub angles: Vec3,
    /// Brush volumes, local to origin and angles.
    pub hulls: Vec<MapHull>,
    pub class: Class,
    /// Next think tick.
    pub next_think: Option<i64>,
    /// Marked by Kill: removed at the end of the frame.
    pub killed: bool,
    /// Index in the map's entity list, when it came from the map.
    pub map_index: Option<usize>,
}

impl LogicEntity {
    /// A keyvalue (case-insensitive key).
    pub fn kv(&self, key: &str) -> Option<&str> {
        self.keyvalues
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(key))
            .map(|(_, v)| v.as_str())
    }
    pub fn kv_f(&self, key: &str) -> f32 {
        self.kv(key).map_or(0.0, atof)
    }
    pub fn kv_i(&self, key: &str) -> i32 {
        self.kv(key).map_or(0, atoi)
    }
    /// "x y z" keyvalue as a vector (missing parts 0).
    pub fn vector_kv(&self, key: &str) -> Vec3 {
        self.kv(key).map_or(Vec3::ZERO, crate::map::entities::parse_vector)
    }
    pub fn has_flag(&self, flag: u32) -> bool {
        self.spawnflags & flag != 0
    }
    pub fn has_output(&self, name: &str) -> bool {
        self.outputs
            .iter()
            .any(|(n, c)| n.eq_ignore_ascii_case(name) && !c.is_empty())
    }
}

/// Where a queued message goes.
#[derive(Clone, Debug, PartialEq)]
enum Target {
    Name(String),
    Direct(EntId),
}

#[derive(Clone, Debug)]
struct Event {
    due: i64,
    target: Target,
    input: String,
    value: Value,
    activator: Option<Who>,
    caller: Option<Who>,
}

/// A delivered input, recorded for tests (`LogicWorld::record`).
#[derive(Clone, Debug, PartialEq)]
pub struct Delivery {
    pub tick: i64,
    pub target: Who,
    pub input: String,
    pub value: Value,
    pub activator: Option<Who>,
    pub caller: Option<Who>,
}

/// Most deliveries one tick makes; the rest wait for the next tick
/// (the original hangs on zero-delay loops).
pub const MAX_DELIVERIES_PER_TICK: usize = 10_000;

/// The logic world.
pub struct LogicWorld {
    slots: Vec<(u32, Option<LogicEntity>)>,
    queue: VecDeque<Event>,
    /// Current tick; 0 is the activation tick.
    pub tick: i64,
    /// Seconds per tick.
    pub dt: f32,
    next_connection: u32,
    pub players: Vec<Player>,
    /// Names given to players (AddOutput targetname).
    pub player_names: Vec<(Entity, String)>,
    pub effects: Vec<Effect>,
    rng: u64,
    /// Developer messages (bad links, unknown inputs, refused commands).
    pub log: Vec<String>,
    /// Record deliveries and fired outputs (tests).
    pub record: bool,
    pub deliveries: Vec<Delivery>,
    pub fired: Vec<(i64, EntId, String)>,
    /// Movers' world brushes this tick, by entity index (entity space).
    pub(super) solids: Vec<Option<Vec<MapBrush>>>,
    /// Players holding use last tick.
    pub(super) use_held: Vec<Entity>,
    /// Round restarts since the map loaded (0: the map's first round).
    pub round: u32,
    /// Switchable light styles and whether each is lit (`visuals`).
    pub(super) light_styles: Vec<(u8, bool)>,
    /// Global states (env_global): name (lower case), state, counter.
    /// They outlive round restarts.
    pub globals: Vec<(String, GlobalState, i32)>,
    /// The static world for traces made outside a phase's collision (a
    /// fire dropping to the floor inside an input, its line-of-sight
    /// checks in a think). None: open space.
    pub collision: Option<Arc<dyn Collision + Send + Sync>>,
    /// Players with a flame on them (fire.rs).
    pub burning_players: Vec<Entity>,
}

/// An env_global state.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GlobalState {
    Off,
    On,
    Dead,
}

/// Classes a round restart keeps as they are instead of re-creating
/// them (entity_io.md open question 1: the preserve list the spec quotes
/// from the public multiplayer code; CS:S's own list is unknown). Matched
/// case-insensitively; a trailing `*` matches a prefix.
pub const ROUND_KEEP: &[&str] = &[
    "worldspawn",
    "func_brush",
    "func_wall",
    "func_buyzone",
    "info_target",
    "env_soundscape*",
    "trigger_soundscape",
    "keyframe_rope",
    "move_rope",
    "sky_camera",
];

/// Whether a round restart keeps entities of this class.
pub fn kept_on_restart(classname: &str) -> bool {
    let c = classname.to_ascii_lowercase();
    ROUND_KEEP.iter().any(|k| match k.strip_suffix('*') {
        Some(prefix) => c.starts_with(prefix),
        None => c == *k,
    })
}

impl LogicWorld {
    pub fn new(dt: f32) -> Self {
        Self {
            slots: Vec::new(),
            queue: VecDeque::new(),
            tick: 0,
            dt,
            next_connection: 1,
            players: Vec::new(),
            player_names: Vec::new(),
            effects: Vec::new(),
            rng: 0x9e37_79b9_7f4a_7c15,
            log: Vec::new(),
            record: false,
            deliveries: Vec::new(),
            fired: Vec::new(),
            solids: Vec::new(),
            use_held: Vec::new(),
            round: 0,
            light_styles: Vec::new(),
            globals: Vec::new(),
            collision: None,
            burning_players: Vec::new(),
        }
    }

    /// Spawn a map's entities (each remembers its index) and activate
    /// them. Returns their ids, in map order.
    pub fn load_map(&mut self, entities: &[crate::map::MapEntity]) -> Vec<EntId> {
        let ids = self.spawn_map(entities);
        self.activate();
        ids
    }

    fn spawn_map(&mut self, entities: &[crate::map::MapEntity]) -> Vec<EntId> {
        entities
            .iter()
            .enumerate()
            .map(|(i, e)| {
                let id = self.spawn(&e.keyvalues, e.hulls.clone());
                if let Some(ent) = self.get_mut(id) {
                    ent.map_index = Some(i);
                }
                id
            })
            .collect()
    }

    /// A round restart (Counter-Strike): every map entity is re-created
    /// from `entities` as at map load and activated again, except the
    /// classes in `ROUND_KEEP`, which stay as they are (or stay gone).
    /// Queued events are dropped; time, players and the random sequence
    /// go on. This world must have been built by `load_map` from the same
    /// entities. Returns the ids, in map order (removed kept ones too:
    /// they don't resolve).
    pub fn round_restart(&mut self, entities: &[crate::map::MapEntity]) -> Vec<EntId> {
        let mut fresh = LogicWorld::new(self.dt);
        fresh.tick = self.tick;
        fresh.rng = self.rng;
        fresh.record = self.record;
        fresh.round = self.round + 1;
        fresh.globals = std::mem::take(&mut self.globals);
        fresh.collision = self.collision.clone();
        fresh.players = std::mem::take(&mut self.players);
        fresh.player_names = std::mem::take(&mut self.player_names);
        fresh.use_held = std::mem::take(&mut self.use_held);
        fresh.effects = std::mem::take(&mut self.effects);
        fresh.log = std::mem::take(&mut self.log);
        fresh.deliveries = std::mem::take(&mut self.deliveries);
        fresh.fired = std::mem::take(&mut self.fired);
        let mut ids = fresh.spawn_map(entities);
        // Kept entities may hold connections added since the map loaded.
        fresh.next_connection = fresh.next_connection.max(self.next_connection);
        // Kept entities: the old ones in their slots (same index: both
        // worlds spawned the map in order), with their solids.
        let mut kept = Vec::new();
        for (i, e) in entities.iter().enumerate() {
            if !kept_on_restart(e.classname()) {
                continue;
            }
            let Some(old) = self.slots.get(i) else { continue };
            if old.1.as_ref().is_some_and(|o| o.map_index != Some(i)) {
                continue;
            }
            fresh.slots[i] = old.clone();
            ids[i] = EntId {
                index: i as u32,
                generation: old.0,
            };
            let solid = self.solids.get(i).cloned().flatten();
            if fresh.solids.len() <= i {
                fresh.solids.resize(i + 1, None);
            }
            fresh.solids[i] = solid;
            kept.push(i);
        }
        for id in fresh.ids() {
            if !kept.contains(&(id.index as usize)) {
                super::classes::class_activate(&mut fresh, id);
            }
        }
        fresh.log.push(format!(
            "round restart {}: {} entities re-created, {} kept",
            fresh.round,
            entities.len() - kept.len(),
            kept.len()
        ));
        *self = fresh;
        ids
    }

    pub fn seed(&mut self, seed: u64) {
        self.rng = seed | 1;
    }

    /// Uniform in [0, 1).
    pub fn random(&mut self) -> f32 {
        // xorshift64*
        let mut x = self.rng;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.rng = x;
        ((x.wrapping_mul(0x2545_f491_4f6c_dd1d) >> 40) as f32) / (1u64 << 24) as f32
    }

    pub fn random_int(&mut self, n: usize) -> usize {
        ((self.random() * n as f32) as usize).min(n.saturating_sub(1))
    }

    /// Current time, seconds.
    pub fn now(&self) -> f32 {
        self.tick as f32 * self.dt
    }

    /// Ticks a queue delay waits: the first tick at or after the due time
    /// (entity_io.md open question 3: integer ticks, ceil(d/dt - 1e-4)).
    pub fn delay_ticks(&self, delay: f32) -> i64 {
        if delay <= 0.0 {
            0
        } else {
            (delay / self.dt - 1e-4).ceil().max(0.0) as i64
        }
    }

    /// Ticks a think waits: nearest tick (round half up).
    pub fn think_ticks(&self, delay: f32) -> i64 {
        (delay / self.dt + 0.5).floor().max(0.0) as i64
    }

    // ------------------------------------------------------- entities

    /// Spawn an entity from keyvalues (outputs parsed out of them).
    pub fn spawn(&mut self, keyvalues: &[(String, String)], hulls: Vec<MapHull>) -> EntId {
        let mut kv = Vec::new();
        let mut outputs: Vec<(String, Vec<Connection>)> = Vec::new();
        for (k, v) in keyvalues {
            let key = k.split('#').next().unwrap_or("");
            if is_output_key(key)
                && let Some((target, input, param, delay, times)) = parse_connection(v)
            {
                let id = self.next_connection;
                self.next_connection += 1;
                let c = Connection {
                    target,
                    input,
                    param,
                    delay,
                    times,
                    id,
                };
                add_connection(&mut outputs, key, c);
                continue;
            }
            kv.push((key.to_string(), v.clone()));
        }
        let get = |key: &str| kv.iter().find(|(k, _)| k.eq_ignore_ascii_case(key)).map(|(_, v)| v.as_str());
        let classname = get("classname").unwrap_or("").to_string();
        let targetname = get("targetname").unwrap_or("").to_string();
        let spawnflags = get("spawnflags").map_or(0, atoi) as u32;
        let origin = get("origin").map_or(Vec3::ZERO, crate::map::entities::parse_vector);
        let angles = get("angles").map_or(Vec3::ZERO, crate::map::entities::parse_vector);
        let ent = LogicEntity {
            classname,
            targetname,
            spawnflags,
            keyvalues: kv,
            outputs,
            origin,
            angles,
            hulls,
            class: Class::None,
            next_think: None,
            killed: false,
            map_index: None,
        };
        let index = self.slots.len() as u32;
        self.slots.push((0, Some(ent)));
        let id = EntId { index, generation: 0 };
        let class = Class::spawn(self, id);
        if let Some(e) = self.get_mut(id) {
            e.class = class;
        }
        id
    }

    pub fn get(&self, id: EntId) -> Option<&LogicEntity> {
        self.slots
            .get(id.index as usize)
            .filter(|(g, _)| *g == id.generation)
            .and_then(|(_, e)| e.as_ref())
    }

    pub fn get_mut(&mut self, id: EntId) -> Option<&mut LogicEntity> {
        self.slots
            .get_mut(id.index as usize)
            .filter(|(g, _)| *g == id.generation)
            .and_then(|(_, e)| e.as_mut())
    }

    pub fn exists(&self, who: Who) -> bool {
        match who {
            Who::Ent(id) => self.get(id).is_some(),
            Who::Player(e) => self.players.iter().any(|p| p.entity == e),
        }
    }

    /// Every live entity id, in entity-list order.
    pub fn ids(&self) -> Vec<EntId> {
        self.slots
            .iter()
            .enumerate()
            .filter(|(_, (_, e))| e.is_some())
            .map(|(i, (g, _))| EntId {
                index: i as u32,
                generation: *g,
            })
            .collect()
    }

    /// The first entity with this name.
    pub fn find(&self, name: &str) -> Option<EntId> {
        self.ids().into_iter().find(|id| {
            self.get(*id)
                .is_some_and(|e| !e.targetname.is_empty() && name_matches(name, &e.targetname))
        })
    }

    pub fn player(&self, e: Entity) -> Option<&Player> {
        self.players.iter().find(|p| p.entity == e)
    }

    pub fn player_mut(&mut self, e: Entity) -> Option<&mut Player> {
        self.players.iter_mut().find(|p| p.entity == e)
    }

    pub fn name_of(&self, who: Who) -> String {
        match who {
            Who::Ent(id) => self.get(id).map(|e| e.targetname.clone()).unwrap_or_default(),
            Who::Player(e) => self
                .player_names
                .iter()
                .find(|(p, _)| *p == e)
                .map(|(_, n)| n.clone())
                .unwrap_or_default(),
        }
    }

    pub fn class_of(&self, who: Who) -> String {
        match who {
            Who::Ent(id) => self.get(id).map(|e| e.classname.clone()).unwrap_or_default(),
            Who::Player(_) => "player".into(),
        }
    }

    /// Mark for removal at the end of the frame.
    pub fn kill(&mut self, id: EntId) {
        if let Some(e) = self.get_mut(id) {
            e.killed = true;
        }
    }

    /// End of a frame: remove killed entities; next tick.
    pub fn end_frame(&mut self) {
        for (i, (g, slot)) in self.slots.iter_mut().enumerate() {
            if let Some(e) = slot.as_ref().filter(|e| e.killed)
                && let Class::Ambient(a) = &e.class
            {
                let id = EntId {
                    index: i as u32,
                    generation: *g,
                };
                super::ambient::removed(&mut self.effects, id, a);
            }
            if let Some(e) = slot.as_ref().filter(|e| e.killed)
                && let Class::Flame(f) = &e.class
            {
                let id = EntId {
                    index: i as u32,
                    generation: *g,
                };
                if !f.stopping {
                    self.effects.push(Effect::AmbientStop { id });
                    if let Who::Player(p) = f.target {
                        self.burning_players.retain(|x| *x != p);
                    }
                }
            }
            if slot.as_ref().is_some_and(|e| e.killed) {
                *slot = None;
                *g += 1;
            }
        }
        self.tick += 1;
    }

    // ------------------------------------------------------- outputs

    /// Fire an output: queue one message per connection (newest first).
    pub fn fire_output(&mut self, id: EntId, output: &str, activator: Option<Who>, value: Value) {
        self.fire_output_delayed(id, output, activator, value, 0.0);
    }

    /// `fire_output` with an extra delay (dropped by parameter overrides).
    pub fn fire_output_delayed(&mut self, id: EntId, output: &str, activator: Option<Who>, value: Value, extra: f32) {
        if self.record {
            self.fired.push((self.tick, id, output.to_string()));
        }
        let tick = self.tick;
        let dt = self.dt;
        let Some(e) = self.get_mut(id) else { return };
        let Some((_, conns)) = e.outputs.iter_mut().find(|(n, _)| n.eq_ignore_ascii_case(output)) else {
            return;
        };
        let mut events = Vec::new();
        conns.retain_mut(|c| {
            let (value, delay) = match &c.param {
                Some(p) => (Value::Str(p.clone()), c.delay),
                None => (value.clone(), c.delay + extra),
            };
            let ticks = if delay <= 0.0 {
                0
            } else {
                (delay / dt - 1e-4).ceil().max(0.0) as i64
            };
            events.push(Event {
                due: tick + ticks,
                target: Target::Name(c.target.clone()),
                input: c.input.clone(),
                value,
                activator,
                caller: Some(Who::Ent(id)),
            });
            if c.times > 0 {
                c.times -= 1;
                return c.times > 0;
            }
            true
        });
        for ev in events {
            self.push_event(ev);
        }
    }

    fn push_event(&mut self, ev: Event) {
        let at = self.queue.partition_point(|e| e.due <= ev.due);
        self.queue.insert(at, ev);
    }

    /// Queue an input straight to an entity (internal messages such as a
    /// relay's unlock), with a delay in seconds.
    pub fn queue_direct(&mut self, target: EntId, input: &str, value: Value, delay: f32, caller: Option<Who>) {
        let due = self.tick + self.delay_ticks(delay);
        self.push_event(Event {
            due,
            target: Target::Direct(target),
            input: input.to_string(),
            value,
            activator: None,
            caller,
        });
    }

    /// Queue a named input (as an output connection would).
    pub fn queue_input(&mut self, target: &str, input: &str, value: Value, delay: f32, activator: Option<Who>) {
        let due = self.tick + self.delay_ticks(delay);
        self.push_event(Event {
            due,
            target: Target::Name(target.to_string()),
            input: input.to_string(),
            value,
            activator,
            caller: None,
        });
    }

    /// Remove queued messages whose caller is `id` (CancelPending).
    pub fn cancel_pending(&mut self, id: EntId) {
        self.queue.retain(|e| e.caller != Some(Who::Ent(id)));
    }

    /// Deliver every due message (zero-delay chains included).
    pub fn service_queue(&mut self) {
        let mut budget = MAX_DELIVERIES_PER_TICK;
        while self.queue.front().is_some_and(|e| e.due <= self.tick) {
            if budget == 0 {
                self.log.push(format!(
                    "event queue: more than {MAX_DELIVERIES_PER_TICK} deliveries in one tick; the rest wait"
                ));
                for e in self.queue.iter_mut() {
                    if e.due <= self.tick {
                        e.due = self.tick + 1;
                    }
                }
                return;
            }
            budget -= 1;
            let ev = self.queue.pop_front().unwrap();
            let activator = ev.activator.filter(|w| self.exists(*w));
            let caller = ev.caller.filter(|w| self.exists(*w));
            let targets = match &ev.target {
                Target::Direct(id) => self.get(*id).map(|_| Who::Ent(*id)).into_iter().collect(),
                Target::Name(name) => self.resolve(name, activator, caller),
            };
            if targets.is_empty() {
                if let Target::Name(n) = &ev.target {
                    self.log.push(format!("unhandled input: no entity '{n}' for {}", ev.input));
                }
                continue;
            }
            for t in targets {
                self.deliver(t, &ev.input, ev.value.clone(), activator, caller);
            }
        }
    }

    /// Entities a target string reaches (resolved at delivery).
    pub fn resolve(&self, target: &str, activator: Option<Who>, caller: Option<Who>) -> Vec<Who> {
        if target.is_empty() {
            return Vec::new();
        }
        let mut out = Vec::new();
        if let Some(special) = target.strip_prefix('!') {
            let w = match special.to_ascii_lowercase().as_str() {
                "activator" => activator,
                "caller" | "self" => caller,
                "player" | "pvsplayer" | "picker" => self.players.first().map(|p| Who::Player(p.entity)),
                _ => None,
            };
            out.extend(w.filter(|w| self.exists(*w)));
        } else {
            for id in self.ids() {
                let e = self.get(id).unwrap();
                if !e.targetname.is_empty() && name_matches(target, &e.targetname) {
                    out.push(Who::Ent(id));
                }
            }
            for (p, name) in &self.player_names {
                if !name.is_empty() && name_matches(target, name) && self.player(*p).is_some() {
                    out.push(Who::Player(*p));
                }
            }
        }
        if out.is_empty() {
            for id in self.ids() {
                if name_matches(target, &self.get(id).unwrap().classname) {
                    out.push(Who::Ent(id));
                }
            }
            if name_matches(target, "player") {
                out.extend(self.players.iter().map(|p| Who::Player(p.entity)));
            }
        }
        out
    }

    // -------------------------------------------------------- inputs

    /// Deliver an input now.
    pub fn deliver(&mut self, target: Who, input: &str, value: Value, activator: Option<Who>, caller: Option<Who>) {
        if self.record {
            self.deliveries.push(Delivery {
                tick: self.tick,
                target,
                input: input.to_string(),
                value: value.clone(),
                activator,
                caller,
            });
        }
        let input = input.to_ascii_lowercase();
        match target {
            Who::Player(p) => self.player_input(p, &input, value),
            Who::Ent(id) => {
                if self.get(id).is_none() {
                    return;
                }
                if super::classes::class_input(self, id, &input, &value, activator, caller) {
                    return;
                }
                self.base_input(id, &input, value, activator, caller);
            }
        }
    }

    /// A value an input needs as a number: logs and returns None when the
    /// link is bad (no value into a typed input).
    pub fn need_float(&mut self, value: &Value, input: &str) -> Option<f32> {
        let v = value.to_float();
        if v.is_none() {
            self.log.push(format!("bad input/output link: {input} needs a number"));
        }
        v
    }

    pub fn need_int(&mut self, value: &Value, input: &str) -> Option<i32> {
        let v = value.to_int();
        if v.is_none() {
            self.log.push(format!("bad input/output link: {input} needs an integer"));
        }
        v
    }

    pub fn need_bool(&mut self, value: &Value, input: &str) -> Option<bool> {
        let v = value.to_bool();
        if v.is_none() {
            self.log.push(format!("bad input/output link: {input} needs a boolean"));
        }
        v
    }

    pub fn need_str(&mut self, value: &Value, input: &str) -> Option<String> {
        let v = value.to_str(|w| self.name_of(w));
        if v.is_none() {
            self.log.push(format!("bad input/output link: {input} needs a string"));
        }
        v
    }

    fn player_input(&mut self, p: Entity, input: &str, value: Value) {
        match input {
            "sethealth" => {
                if let Some(h) = self.need_float(&value, input) {
                    self.effects.push(Effect::SetHealth { target: p, health: h });
                }
            }
            "addoutput" => {
                let Some(s) = self.need_str(&value, input) else { return };
                if let Some((k, v)) = s.split_once(' ')
                    && k.eq_ignore_ascii_case("targetname")
                {
                    self.player_names.retain(|(e, _)| *e != p);
                    self.player_names.push((p, v.trim().to_string()));
                }
            }
            "kill" | "killhierarchy" => {
                // Players are never removed; a kill input kills them.
                self.effects.push(Effect::Damage {
                    target: p,
                    amount: 10_000.0,
                    crush: false,
                });
            }
            "ignite" | "ignitelifetime" | "ignitenumhitboxfires" | "ignitehitboxfirescale" => {
                super::fire::ignite_input(self, Who::Player(p), input, &value);
            }
            _ => self.log.push(format!("player: unhandled input {input}")),
        }
    }

    fn base_input(&mut self, id: EntId, input: &str, value: Value, activator: Option<Who>, caller: Option<Who>) {
        match input {
            "kill" | "killhierarchy" => self.kill(id),
            "use" => super::classes::class_use(self, id, activator, caller),
            "addoutput" => {
                if let Some(s) = self.need_str(&value, input) {
                    self.add_output(id, &s);
                }
            }
            "fireuser1" | "fireuser2" | "fireuser3" | "fireuser4" => {
                let n = &input[8..];
                self.fire_output(id, &format!("OnUser{n}"), activator, Value::Void);
            }
            _ => {
                let class = self.get(id).map(|e| e.classname.clone()).unwrap_or_default();
                self.log.push(format!("{class}: unhandled input {input}"));
            }
        }
    }

    /// AddOutput: "key value", with ':' as ','.
    pub fn add_output(&mut self, id: EntId, text: &str) {
        let Some((key, rest)) = text.split_once(' ') else {
            self.log.push(format!("AddOutput without a value: '{text}'"));
            return;
        };
        let value = rest.replace(':', ",");
        if is_output_key(key)
            && let Some((target, input, param, delay, times)) = parse_connection(&value)
        {
            let c = Connection {
                target,
                input,
                param,
                delay,
                times,
                id: self.next_connection,
            };
            self.next_connection += 1;
            if let Some(e) = self.get_mut(id) {
                add_connection(&mut e.outputs, key, c);
            }
            return;
        }
        let key_lower = key.to_ascii_lowercase();
        let Some(e) = self.get_mut(id) else { return };
        match key_lower.as_str() {
            "targetname" => e.targetname = value.trim().to_string(),
            "origin" => e.origin = crate::map::entities::parse_vector(&value),
            "angles" => e.angles = crate::map::entities::parse_vector(&value),
            _ => {}
        }
        if let Some(kv) = e.keyvalues.iter_mut().find(|(k, _)| k.eq_ignore_ascii_case(key)) {
            kv.1 = value.clone();
        } else {
            e.keyvalues.push((key.to_string(), value.clone()));
        }
        super::classes::class_keyvalue(self, id, &key_lower, &value);
    }

    // --------------------------------------------------------- thinks

    /// Schedule a think in `delay` seconds (nearest tick).
    pub fn think_in(&mut self, id: EntId, delay: f32) {
        let at = self.tick + self.think_ticks(delay);
        if let Some(e) = self.get_mut(id) {
            e.next_think = Some(at);
        }
    }

    /// Run due thinks, in entity order.
    pub fn run_thinks(&mut self) {
        for id in self.ids() {
            // Movers think after their push step (`step_movers`).
            let due = self.get(id).is_some_and(|e| {
                !e.killed && e.next_think.is_some_and(|t| t <= self.tick) && super::movers::pusher(&e.class).is_none()
            });
            if due {
                self.get_mut(id).unwrap().next_think = None;
                super::classes::class_think(self, id);
            }
        }
    }

    /// One whole frame without players moving (tests): thinks, movers,
    /// use, touches, untouch, the queue, removals.
    pub fn frame(&mut self, collision: &dyn Collision) {
        self.run_thinks();
        self.step_movers(collision);
        self.player_uses(collision);
        self.touch_triggers(collision);
        self.touch_breakables();
        self.pressure_props();
        self.untouch();
        self.service_queue();
        self.end_frame();
    }

    /// Activate everything after spawning (the map's entities): filters,
    /// paths, first thinks.
    pub fn activate(&mut self) {
        for id in self.ids() {
            super::classes::class_activate(self, id);
        }
    }
}

fn add_connection(outputs: &mut Vec<(String, Vec<Connection>)>, key: &str, c: Connection) {
    let name = key.to_ascii_lowercase();
    match outputs.iter_mut().find(|(n, _)| *n == name) {
        Some((_, list)) => list.insert(0, c),
        None => outputs.push((name, vec![c])),
    }
}
