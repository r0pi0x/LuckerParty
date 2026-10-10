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

pub use crate::map::entities::{OTHER_OUTPUTS, is_output_key};

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
    /// The buttons of its command this tick (`core::buttons` bits, after
    /// what map entities took away).
    pub buttons: u32,
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
            buttons: 0,
        }
    }

    /// The eye's forward direction (entity space).
    pub fn forward(&self) -> Vec3 {
        super::triggers::forward(self.view)
    }

    /// The centre of its box (entity space).
    pub fn centre(&self) -> Vec3 {
        self.origin + (self.mins + self.maxs) / 2.0
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
    /// A sound entry (or file) at a point (entity space); volume (0..1)
    /// and pitch (percent) override the entry's when given.
    Sound {
        entry: String,
        at: Vec3,
        volume: Option<f32>,
        pitch: Option<f32>,
    },
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
    /// A player's damage filter (`SetDamageFilter`): the damage-type bits
    /// a filter_damage_type tests and its Negated flag (damage passes when
    /// its bits equal `bits`, inverted when negated); None clears it.
    DamageFilter { target: Entity, filter: Option<(u32, bool)> },
    /// Give a player items (game_player_equip: names and counts), after
    /// taking all its weapons when `strip` (also player_weaponstrip).
    Equip { target: Entity, items: Vec<(String, u32)>, strip: bool },
    /// A server command (point_servercommand) that passed
    /// `classes::check_server_command`.
    ServerCommand(super::classes::ServerLine),
    /// An allowed client command for one player (point_clientcommand).
    ClientCommand { player: Entity, command: String },
    /// Gibs thrown by a breaking brush: a gib list name (propdata
    /// "BreakableModels", or a model) and the pieces (entity space).
    Gibs {
        set: String,
        glass: bool,
        pieces: Vec<super::breakables::Gib>,
        /// The sound entry the gibs make when they bounce.
        bounce: Option<&'static str>,
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
        /// The shards' size (units).
        shard: f32,
    },
    /// A collapsing window pane drops a falling piece (the pane piece
    /// model, body `body`): its reference corner and axes (along the
    /// columns, the rows, the window's normal; entity space), the pane's
    /// size (units) and its spin (deg/s about each axis).
    PaneFall {
        at: Vec3,
        axes: [Vec3; 3],
        size: Vec2,
        body: usize,
        spin: Vec3,
    },
    /// An explosion (a breakable's `explodemagnitude`): radius damage
    /// `damage` within `radius` units of `at` (entity space); `inflictor`
    /// (what exploded) takes none of it.
    Explosion {
        at: Vec3,
        damage: f32,
        radius: f32,
        attacker: Option<Who>,
        inflictor: EntId,
    },
    /// A bullet or club hit shattered the pane it hit: the glass-impact
    /// burst at the hit point with the trace normal (entity space;
    /// specs/cs_source/impact_effects.md section 9).
    GlassImpact { at: Vec3, normal: Vec3 },
    /// player_speedmod's ModifySpeed on a player
    /// (specs/source/game_entities.md 1): its movement clock, and what the
    /// entity's spawnflags take away (`key` names the entity).
    SpeedMod { target: Entity, key: u32, scale: f32, flags: u32 },
    /// A game_ui takes (`on`) or gives back a player's controls, with its
    /// "Freeze Player" and "Hide Weapon" flags (game_entities.md 2).
    GameUi {
        target: Entity,
        on: bool,
        freeze: bool,
        hide_weapon: bool,
    },
    /// A point_viewcontrol (viewcontrol_and_templates.md 1): the player
    /// views from `camera` (frozen with `freeze`, invulnerable), or None:
    /// from its own eye again, unfrozen.
    ViewControl {
        target: Entity,
        camera: Option<EntId>,
        freeze: bool,
    },
    /// Something on players' HUDs: one player's (`to`) or everyone's.
    Hud { to: Option<Entity>, what: super::hud::HudShow },
    /// game_score: a player's kills or (with `team`) its team's score.
    Score {
        target: Entity,
        points: i32,
        team: bool,
        allow_negative: bool,
    },
    /// A brush or prop copy a template spawned (`templates`): `id` is the
    /// new entity, `source` the map entity whose node it copies, placed
    /// at `origin`/`angles` (entity space).
    Spawned {
        id: EntId,
        source: usize,
        origin: Vec3,
        angles: Vec3,
    },
    /// env_spark's sparks: where (entity space), the direction (zero:
    /// none) and the magnitude.
    Spark {
        at: Vec3,
        dir: Vec3,
        magnitude: f32,
    },
    /// A point_tesla sparks: the map draws its arcs (its entity, by map
    /// index).
    Tesla {
        entity: usize,
    },
    /// env_viewpunch: add to a player's view punch (pitch, yaw, roll
    /// degrees, Source's: pitch down positive).
    ViewPunch {
        player: Entity,
        angles: Vec3,
    },
    /// env_muzzleflash fired: the map draws a flash at it (by map index).
    MuzzleFlash {
        entity: usize,
    },
    /// Push a physics body (env_entity_maker's PostSpawnSpeed): add
    /// `velocity` (entity space, units/s) to the entity's body.
    BodyVelocity {
        id: EntId,
        velocity: Vec3,
    },
    /// Move a physics body's centre to `origin` (entity space), turned to
    /// `angles` when given, its velocity kept (trigger_teleport,
    /// point_teleport).
    BodyTeleport {
        id: EntId,
        origin: Vec3,
        angles: Option<Vec3>,
    },
    /// A screen shake (env_shake) around `at` (entity space): amplitude
    /// (units; 0 stops the map's shakes), frequency (Hz), duration (s),
    /// radius (units; infinite: everyone).
    Shake {
        at: Vec3,
        amplitude: f32,
        frequency: f32,
        duration: f32,
        radius: f32,
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
    /// Its render colour ("rendercolor", the Color input): a brush
    /// entity's node is tinted by it.
    pub render_color: [u8; 3],
    /// Its render alpha and mode ("renderamt", "rendermode", the Alpha
    /// input): a brush entity's node fades or adds by them.
    pub render_alpha: u8,
    pub render_mode: u8,
    /// The frame its animated textures show (env_texturetoggle).
    pub texture_frame: u32,
}

impl LogicEntity {
    /// How it is drawn (`map::tint::RenderLook`).
    pub fn render_look(&self) -> crate::map::tint::RenderLook {
        crate::map::tint::RenderLook {
            color: self.render_color,
            alpha: self.render_alpha,
            mode: self.render_mode,
        }
    }
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
    /// Classnames given to players (AddOutput classname: kz_ and bhop_
    /// maps mark a player's stage this way for filter_activator_class);
    /// others are "player".
    pub player_classes: Vec<(Entity, String)>,
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
    /// This tick's +use presses: who, and whether they found something
    /// to use (nothing: the game plays its deny sound).
    pub use_presses: Vec<(Entity, bool)>,
    /// Round restarts since the map loaded (0: the map's first round).
    pub round: u32,
    /// Switchable light styles and whether each is lit (`visuals`).
    pub(super) light_styles: Vec<(u8, bool)>,
    /// What env_tonemap_controller inputs set (`visuals::tonemap_input`).
    pub(super) tonemap: crate::map::TonemapInputs,
    /// Global states (env_global): name (lower case), state, counter.
    /// They outlive round restarts.
    pub globals: Vec<(String, GlobalState, i32)>,
    /// The static world for traces made outside a phase's collision (a
    /// fire dropping to the floor inside an input, its line-of-sight
    /// checks in a think). None: open space.
    pub collision: Option<Arc<dyn Collision + Send + Sync>>,
    /// Players with a flame on them (fire.rs).
    pub burning_players: Vec<Entity>,
    /// Entities parented to an anchor entity, parents first (`anchors`).
    pub follows: Vec<(EntId, super::anchors::Follow)>,
    /// Players parented to an entity (`anchors`).
    pub player_follows: Vec<(Entity, super::anchors::Follow)>,
    /// point_template's instance counter (`templates`): for the whole
    /// session, round restarts included.
    pub template_serial: u32,
    /// Players viewing through a point_viewcontrol, and which (`camera`).
    pub views: Vec<(Entity, EntId)>,
    /// Kinds of known, intended gaps already noted (`note`): each is
    /// logged once per map.
    pub notes: std::collections::BTreeSet<String>,
    /// How players are drawn, for those a map changed (Alpha and Color
    /// inputs, AddOutput rendermode/renderamt/rendercolor): invisibility
    /// power-ups, team colours. They keep it across rounds, as the game's
    /// player entities do.
    pub player_looks: Vec<(Entity, crate::map::tint::RenderLook)>,
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
            player_classes: Vec::new(),
            effects: Vec::new(),
            rng: 0x9e37_79b9_7f4a_7c15,
            log: Vec::new(),
            record: false,
            deliveries: Vec::new(),
            fired: Vec::new(),
            solids: Vec::new(),
            use_held: Vec::new(),
            use_presses: Vec::new(),
            round: 0,
            light_styles: Vec::new(),
            tonemap: Default::default(),
            globals: Vec::new(),
            collision: None,
            burning_players: Vec::new(),
            follows: Vec::new(),
            player_follows: Vec::new(),
            notes: Default::default(),
            player_looks: Vec::new(),
            template_serial: 0,
            views: Vec::new(),
        }
    }

    /// Spawn a map's entities (each remembers its index) and activate
    /// them. Returns their ids, in map order.
    pub fn load_map(&mut self, entities: &[crate::map::MapEntity]) -> Vec<EntId> {
        let ids = self.spawn_map(entities);
        self.activate();
        self.link_anchored();
        ids
    }

    fn spawn_map(&mut self, entities: &[crate::map::MapEntity]) -> Vec<EntId> {
        let ids: Vec<EntId> = entities
            .iter()
            .enumerate()
            .map(|(i, e)| {
                let id = self.spawn(&e.keyvalues, e.hulls.clone());
                if let Some(ent) = self.get_mut(id) {
                    ent.map_index = Some(i);
                }
                id
            })
            .collect();
        // Templates take their members out before anything activates.
        self.collect_templates(entities, &ids);
        ids
    }

    /// Drop an entity's slot at once (it never existed: no removal
    /// effects); its id stops resolving.
    pub(super) fn forget(&mut self, id: EntId) {
        if let Some((g, slot)) = self.slots.get_mut(id.index as usize)
            && *g == id.generation
        {
            *slot = None;
            *g += 1;
        }
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
        fresh.template_serial = self.template_serial;
        // The camera's tone-map settings stay (the map's own outputs set
        // them again).
        fresh.tonemap = self.tonemap.clone();
        fresh.collision = self.collision.clone();
        fresh.players = std::mem::take(&mut self.players);
        fresh.player_names = std::mem::take(&mut self.player_names);
        fresh.player_classes = std::mem::take(&mut self.player_classes);
        fresh.player_looks = std::mem::take(&mut self.player_looks);
        fresh.notes = std::mem::take(&mut self.notes);
        fresh.use_held = std::mem::take(&mut self.use_held);
        fresh.effects = std::mem::take(&mut self.effects);
        // Cameras are made again: their viewers see from their eyes
        // (our choice; viewcontrol spec open question 2).
        for (p, _) in std::mem::take(&mut self.views) {
            fresh.effects.push(Effect::ViewControl {
                target: p,
                camera: None,
                freeze: false,
            });
        }
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
        fresh.link_anchored();
        let detail = format!(
            "round {}: {} entities re-created, {} kept",
            fresh.round,
            entities.len() - kept.len(),
            kept.len()
        );
        fresh.note("round restarts re-create the map's entities", detail);
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
        let render_color = get("rendercolor").map_or([255; 3], parse_color);
        let render_alpha = get("renderamt").map_or(255, |v| atoi(v).clamp(0, 255)) as u8;
        let render_mode = get("rendermode").map_or(0, |v| atoi(v).clamp(0, 255)) as u8;
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
            render_color,
            render_alpha,
            render_mode,
            texture_frame: 0,
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

    /// Whether the map's entity `index` is still there (not killed or
    /// broken since the map loaded or the round restarted).
    pub fn map_entity_alive(&self, index: usize) -> bool {
        // Map entities spawn first and in order, and slots are never
        // reused: slot `index` is the map's entity `index`.
        matches!(self.slots.get(index), Some((_, Some(e))) if e.map_index == Some(index))
    }

    /// The live entity the map's entity `index` spawned.
    pub fn by_map_index(&self, index: usize) -> Option<EntId> {
        // Map entities spawn in order, so slot `index` holds it.
        if let Some((g, Some(e))) = self.slots.get(index)
            && e.map_index == Some(index)
        {
            return Some(EntId {
                index: index as u32,
                generation: *g,
            });
        }
        self.ids()
            .into_iter()
            .find(|id| self.get(*id).is_some_and(|e| e.map_index == Some(index)))
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
            Who::Player(p) => self
                .player_classes
                .iter()
                .find(|(e, _)| *e == p)
                .map_or_else(|| "player".into(), |(_, c)| c.clone()),
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
    /// Messages waiting in the event queue.
    pub fn queued(&self) -> usize {
        self.queue.len()
    }

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
                    // The map names something it doesn't have (or has
                    // removed): the game drops the input too.
                    let detail = format!("'{n}' for {}", ev.input);
                    self.note("inputs to names nothing has (dropped, as the game does)", detail);
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
            for p in &self.players {
                if name_matches(target, &self.class_of(Who::Player(p.entity))) {
                    out.push(Who::Player(p.entity));
                }
            }
        }
        out
    }

    // -------------------------------------------------------- inputs

    /// A known, intended gap (a map's own missing names, inputs CS:S
    /// doesn't have, commands for server plugins): logged once per map
    /// and kind, as `note: <kind>: <first case>`. `mapsweep --audit`
    /// counts these apart from complaints.
    pub fn note(&mut self, kind: &str, detail: impl Into<String>) {
        if self.notes.insert(kind.to_string()) {
            self.log.push(format!("note: {kind}: {}", detail.into()));
        }
    }

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
        // Inputs of later engines (CS:GO's VScript and collectibles, model
        // scale): CS:S has none of them, so they do nothing there either.
        if NOT_IN_CSS_INPUTS.contains(&input.as_str()) {
            let class = self.class_of(target);
            self.note(
                "inputs CS:S doesn't have (VScript, later engines)",
                format!("{class}.{input}"),
            );
            return;
        }
        match target {
            Who::Player(p) => self.player_input(p, &input, value, activator, caller),
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

    fn player_input(&mut self, p: Entity, input: &str, value: Value, activator: Option<Who>, caller: Option<Who>) {
        if self.player_parent_input(p, input, &value, activator, caller) {
            return;
        }
        match input {
            "sethealth" => {
                if let Some(h) = self.need_float(&value, input) {
                    self.effects.push(Effect::SetHealth { target: p, health: h });
                }
            }
            "addoutput" => {
                let Some(s) = self.need_str(&value, input) else { return };
                let Some((k, v)) = s.split_once(' ') else { return };
                self.player_keyvalue(p, &k.to_ascii_lowercase(), v.trim());
            }
            "setdamagefilter" => {
                let name = match &value {
                    Value::Void => String::new(),
                    v => self.need_str(v, input).unwrap_or_default(),
                };
                // A name nothing has: no filter at all (the game finds no
                // entity and keeps none).
                let missing = !name.is_empty() && self.find(&name).is_none();
                if missing {
                    self.note(
                        "damage filters naming nothing (none kept, as in the game)",
                        name.clone(),
                    );
                }
                let filter = if name.is_empty() || missing {
                    None
                } else {
                    let found = self.find(&name).and_then(|f| match self.get(f).map(|e| (&e.class, e)) {
                        Some((super::classes::Class::Filter(filter), e))
                            if matches!(filter.kind, super::classes::FilterKind::DamageType) =>
                        {
                            Some((e.kv_i("damagetype") as u32, filter.negated))
                        }
                        _ => None,
                    });
                    if found.is_none() {
                        self.log.push(format!("player: damage filter '{name}' is not a filter_damage_type (ignored)"));
                        return;
                    }
                    found
                };
                self.effects.push(Effect::DamageFilter { target: p, filter });
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
            "alpha" => {
                if let Some(a) = self.need_int(&value, input) {
                    self.player_look(p, |l| l.alpha = a.clamp(0, 255) as u8);
                }
            }
            "color" => {
                if let Some(c) = self.need_str(&value, input) {
                    self.player_look(p, |l| l.color = parse_color(&c));
                }
            }
            // A player has no outputs of its own to fire, and can't break.
            "fireuser1" | "fireuser2" | "fireuser3" | "fireuser4" | "break" => {}
            // The player's fog from an env_fog_controller (public entity
            // docs). One naming nothing changes nothing; a real one isn't
            // switched per player (tech-debt: the map's first controller
            // is everyone's fog).
            "setfogcontroller" => {
                let name = self.need_str(&value, input).unwrap_or_default();
                let found = self
                    .find(name.trim())
                    .and_then(|f| self.get(f))
                    .is_some_and(|e| e.classname.eq_ignore_ascii_case("env_fog_controller"));
                if found {
                    self.note("per-player fog controllers not switched (the map's first is everyone's)", name);
                } else {
                    self.note("SetFogController naming no fog controller (nothing changes, as in the game)", name);
                }
            }
            _ => self.log.push(format!("player: unhandled input {input}")),
        }
    }

    /// A keyvalue set on a player through AddOutput (entity_io.md:
    /// AddOutput sets any field the class reads from a keyvalue). The
    /// ones maps use on `!activator`: its name, gravity, a base velocity
    /// (bhop and surf boosters: the next move adds it to the velocity, as
    /// no trigger keeps it up), health, origin (a teleport).
    fn player_keyvalue(&mut self, p: Entity, key: &str, v: &str) {
        let num = super::value::atof;
        match key {
            "targetname" => {
                self.player_names.retain(|(e, _)| *e != p);
                self.player_names.push((p, v.to_string()));
            }
            "classname" => {
                self.player_classes.retain(|(e, _)| *e != p);
                if !v.eq_ignore_ascii_case("player") {
                    self.player_classes.push((p, v.to_string()));
                }
            }
            "gravity" => {
                if let Some(pl) = self.player_mut(p) {
                    pl.gravity = num(v);
                }
            }
            // The keyvalue is the base velocity itself, replaced. No
            // trigger keeps it up, so the player's next move turns it into
            // velocity, x (1 + dt/2) (triggers.md, trigger_push step 1);
            // it doesn't take the player off the ground itself (only a
            // resulting vz over 250 does: movement.md, per-tick order 7).
            "basevelocity" => {
                let push = crate::map::entities::parse_vector(v);
                if let Some(pl) = self.player_mut(p) {
                    pl.base_velocity = push;
                }
            }
            "health" => self.effects.push(Effect::SetHealth {
                target: p,
                health: num(v),
            }),
            "origin" => {
                let at = crate::map::entities::parse_vector(v);
                if let Some(pl) = self.player_mut(p) {
                    pl.origin = at;
                    pl.on_ground = false;
                    pl.unground = true;
                    pl.teleported = true;
                }
            }
            "rendermode" => self.player_look(p, |l| l.mode = atoi(v).clamp(0, 255) as u8),
            "renderamt" => self.player_look(p, |l| l.alpha = atoi(v).clamp(0, 255) as u8),
            "rendercolor" => self.player_look(p, |l| l.color = parse_color(v)),
            // Render effects (pulses, flicker): not drawn (tech-debt).
            "renderfx" => {}
            _ => self.log.push(format!("player: AddOutput {key} not supported")),
        }
    }

    /// Change how a player is drawn.
    fn player_look(&mut self, p: Entity, f: impl FnOnce(&mut crate::map::tint::RenderLook)) {
        let i = match self.player_looks.iter().position(|(e, _)| *e == p) {
            Some(i) => i,
            None => {
                self.player_looks.push((p, crate::map::tint::RenderLook::default()));
                self.player_looks.len() - 1
            }
        };
        f(&mut self.player_looks[i].1);
    }

    fn base_input(&mut self, id: EntId, input: &str, value: Value, activator: Option<Who>, caller: Option<Who>) {
        if self.parent_input(id, input, &value, activator, caller) {
            return;
        }
        match input {
            "kill" | "killhierarchy" => self.kill(id),
            "use" => super::classes::class_use(self, id, activator, caller),
            "addoutput" => {
                if let Some(s) = self.need_str(&value, input) {
                    self.add_output(id, &s);
                }
            }
            // Every entity's render colour (entity_io.md, base inputs):
            // brush entities with a node show it.
            "color" => {
                if let Some(s) = self.need_str(&value, input)
                    && let Some(e) = self.get_mut(id)
                {
                    e.render_color = parse_color(&s);
                }
            }
            // Render alpha (0-255): brush entities with a node show it.
            "alpha" => {
                if let Some(a) = self.need_int(&value, input)
                    && let Some(e) = self.get_mut(id)
                {
                    e.render_alpha = a.clamp(0, 255) as u8;
                    let a = e.render_alpha.to_string();
                    match e
                        .keyvalues
                        .iter_mut()
                        .find(|(k, _)| k.eq_ignore_ascii_case("renderamt"))
                    {
                        Some(kv) => kv.1 = a,
                        None => e.keyvalues.push(("renderamt".into(), a)),
                    }
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
        let base = match key_lower.as_str() {
            "targetname" => {
                e.targetname = value.trim().to_string();
                true
            }
            "origin" => {
                e.origin = crate::map::entities::parse_vector(&value);
                true
            }
            "angles" => {
                e.angles = crate::map::entities::parse_vector(&value);
                true
            }
            "spawnflags" => {
                e.spawnflags = atoi(&value) as u32;
                true
            }
            "rendercolor" => {
                e.render_color = parse_color(&value);
                true
            }
            "renderamt" => {
                e.render_alpha = atoi(&value).clamp(0, 255) as u8;
                true
            }
            "rendermode" => {
                e.render_mode = atoi(&value).clamp(0, 255) as u8;
                true
            }
            // Render effects (pulses, flicker, hologram): kept, not drawn
            // (tech-debt).
            "renderfx" => true,
            // Filters and classname targets see the new name; the entity
            // keeps behaving as its spawned class.
            "classname" => {
                e.classname = value.trim().to_string();
                true
            }
            _ => false,
        };
        if let Some(kv) = e.keyvalues.iter_mut().find(|(k, _)| k.eq_ignore_ascii_case(key)) {
            kv.1 = value.clone();
        } else {
            e.keyvalues.push((key.to_string(), value.clone()));
        }
        let class = e.classname.clone();
        if !super::classes::class_keyvalue(self, id, &key_lower, &value) && !base {
            self.log.push(format!("{class}: AddOutput {key_lower} has no effect"));
        }
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
        self.move_cameras();
        self.step_movers(collision);
        self.player_uses(collision);
        self.push_conveyors();
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

/// Inputs maps send that CS:S's entities don't have (VScript and
/// collectibles from CS:GO, model scale from later engines): noted once,
/// ignored.
pub const NOT_IN_CSS_INPUTS: &[&str] = &[
    "runscriptcode",
    "runscriptfile",
    "callscriptfunction",
    "addcollectible",
    "clearcollectibles",
    "setmodelscale",
];

/// "r g b" (0-255 each; missing parts 255, as the game's colour keys).
pub fn parse_color(s: &str) -> [u8; 3] {
    let v: Vec<i32> = s.split_whitespace().map(atoi).collect();
    let c = |i: usize| v.get(i).copied().unwrap_or(255).clamp(0, 255) as u8;
    [c(0), c(1), c(2)]
}

fn add_connection(outputs: &mut Vec<(String, Vec<Connection>)>, key: &str, c: Connection) {
    let name = key.to_ascii_lowercase();
    match outputs.iter_mut().find(|(n, _)| *n == name) {
        Some((_, list)) => list.insert(0, c),
        None => outputs.push((name, vec![c])),
    }
}
