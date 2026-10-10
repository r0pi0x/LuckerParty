//! Props that take damage and break (specs/source/prop_damage.md):
//! prop_dynamic and prop_physics*. Health, damage multipliers, pieces and
//! the rest come from the model's prop data, which the game's loader
//! hands over as keyvalues (`map::entities::PROP_*_KEY`); the map's
//! `health` counts only on the `_override` classes. A prop with health
//! and something to break into loses health and breaks; any other takes
//! "events only" damage (outputs, never breaks). Breaking fires OnBreak,
//! removes the prop and what is parented to it, and asks the host for the
//! break sound, the pieces and an explosion (`Effect::PropBreak`).
//! Client-only props (multiplayer physics mode 3) follow their own simple
//! rule (section 3). Physics impacts reach props as crush damage the host
//! computes with `impact_damage`.

use bevy::prelude::*;

use super::classes::Class;
use super::value::Value;
use super::world::{Effect, EntId, LogicWorld, Who};
use crate::core::DamageKind;
use crate::map::entities::{
    PROP_BREAK_SOUND_KEY, PROP_CLIENT_KEY, PROP_DAMAGE_KEY, PROP_EXPLODE_KEY, PROP_EXPLODE_SOUND_KEY, PROP_HEALTH_KEY,
    PROP_INTERACTIONS_KEY, PROP_MASS_KEY, PROP_PIECES_KEY, PROP_TABLE_KEY,
};

/// Spawnflags.
pub const SF_PHYSPROP_START_ASLEEP: u32 = 1;
pub const SF_PHYSPROP_DONT_TAKE_PHYSICS_DAMAGE: u32 = 2;
pub const SF_PHYSPROP_MOTION_DISABLED: u32 = 8;
pub const SF_PHYSPROP_BREAK_ON_PRESSURE: u32 = 32;

/// `physdamagescale` 0 means this.
pub const DEFAULT_IMPACT_SCALE: f32 = 0.1;
/// A client prop takes this from every bullet, and from other impact
/// effects (spec 3.2).
pub const CLIENT_BULLET_DAMAGE: i32 = 30;
pub const CLIENT_OTHER_DAMAGE: i32 = 50;
/// Blasts lift client objects' direction by this (units).
pub const CLIENT_BLAST_LIFT: f32 = 32.0;
/// An explosion with radius 0 reaches this times its magnitude.
pub const EXPLODE_RADIUS_FACTOR: f32 = 2.5;
/// Buckshot doubles the bullet multiplier (unused: no damage kind marks
/// buckshot yet).
pub const BUCKSHOT_FACTOR: f32 = 2.0;

/// A prop entity's damage and visibility state.
#[derive(Clone, Debug)]
pub struct Prop {
    pub health: i32,
    /// At least 1.
    pub max_health: i32,
    /// Takes damage only as events: outputs, never health.
    pub events_only: bool,
    pub broken: bool,
    /// Drawn (Enable/Disable, TurnOn/TurnOff).
    pub visible: bool,
    /// Solid to players and shots (EnableCollision/DisableCollision).
    pub solid: bool,
    /// The model's skin family (`skin`, the Skin input).
    pub skin: i32,
    /// The body number set by SetBodyGroup (the combined body index;
    /// `SetBodyGroup` keyvalue at spawn); None: the model's own.
    pub body_group: Option<i32>,
    /// The sequence asked for last (SetAnimation) and how many times one
    /// was asked for (each ask restarts it).
    pub sequence: Option<String>,
    pub sequence_serial: u32,
    /// Played when a sequence ends and at spawn (`DefaultAnim`,
    /// SetDefaultAnimation).
    pub default_sequence: String,
    /// Damage multipliers: bullets, club, blast.
    pub dmg: [f32; 3],
    /// A client-only prop (spec 3): no name, no outputs, its own damage.
    pub client: bool,
    /// Explosion on breaking: magnitude and radius (units), and the sound.
    pub explode: (f32, f32),
    pub explode_sound: String,
    pub break_sound: String,
    pub interactions: Vec<String>,
    /// Physics impact energy scale (`physdamagescale`).
    pub impact_scale: f32,
    pub table: ImpactTable,
    /// kg.
    pub mass: f32,
    /// A physics prop (it can be woken, frozen and unfrozen).
    pub physics: bool,
    /// Held in place (motion disabled) until enabled.
    pub frozen: bool,
    pub force_to_enable: f32,
    pub damage_to_enable: f32,
    /// Started asleep and hasn't fired OnAwakened yet.
    pub asleep: bool,
    /// The tick it spawned (hits then are events only).
    pub spawn_tick: i64,
    /// The player who hurt it last (an explosion's attacker).
    pub last_attacker: Option<Who>,
    /// Break on pressure: who stood on it last, and whether the break is
    /// scheduled.
    pub pressure_breaker: Option<Who>,
    pub pressure_pending: bool,
    /// Set on fire (fire.rs); never cleared, as props are never told
    /// their flame ended (specs/source/fire.md, Quirks).
    pub burning: bool,
    /// Where its body is now: world box (entity space), from the host;
    /// None: not known (its origin stands for it).
    pub bounds: Option<(Vec3, Vec3)>,
}

/// Whether a class is a prop the logic keeps (not a door): model props,
/// and physics brushes (func_physbox, which take damage, move and break
/// as physics props do: specs/source/physics_brushes.md 2).
pub fn is_prop_class(classname: &str) -> bool {
    let c = classname.to_ascii_lowercase();
    c.starts_with("prop_dynamic") || c.starts_with("prop_physics") || c == "prop_ragdoll" || is_physbox(&c)
}

/// func_physbox and func_physbox_multiplayer.
pub fn is_physbox(classname: &str) -> bool {
    classname.eq_ignore_ascii_case("func_physbox") || classname.eq_ignore_ascii_case("func_physbox_multiplayer")
}

/// func_physbox spawnflags (physics_brushes.md 2).
pub const SF_PHYSBOX_ONLY_BREAK_ON_TRIGGER: u32 = 1;
pub const SF_PHYSBOX_START_ASLEEP: u32 = 4096;
pub const SF_PHYSBOX_MOTION_DISABLED: u32 = 32768;

impl Prop {
    pub(super) fn spawn(w: &mut LogicWorld, id: EntId) -> Prop {
        let tick = w.tick;
        let e = w.get(id).unwrap();
        let class = e.classname.to_ascii_lowercase();
        let model_health = e.kv(PROP_HEALTH_KEY).map(super::value::atoi);
        let map_health = e.kv_i("health");
        let client = e.kv(PROP_CLIENT_KEY).is_some_and(|v| v.trim() == "1");
        let override_class = class.ends_with("_override");
        let pieces = e.kv_i(PROP_PIECES_KEY);
        let interactions: Vec<String> = e
            .kv(PROP_INTERACTIONS_KEY)
            .unwrap_or("")
            .split_whitespace()
            .map(str::to_string)
            .collect();
        let explode_kv = |k: &str| {
            e.kv(PROP_EXPLODE_KEY)
                .map(|v| super::value::atof(v.split_whitespace().nth(if k == "d" { 0 } else { 1 }).unwrap_or("0")))
        };
        // The map's ExplodeDamage/ExplodeRadius are defaults prop data
        // overrides (spec 7.5); the override classes keep the map's.
        let explode = if override_class {
            (e.kv_f("ExplodeDamage"), e.kv_f("ExplodeRadius"))
        } else {
            (
                explode_kv("d").unwrap_or(e.kv_f("ExplodeDamage")),
                explode_kv("r").unwrap_or(e.kv_f("ExplodeRadius")),
            )
        };
        let health = if client {
            // Client props: prop data's health wins, the map's otherwise
            // (spec 3.1).
            model_health.unwrap_or(map_health)
        } else if override_class {
            map_health
        } else {
            model_health.unwrap_or(0)
        };
        let breaks_into = pieces > 0
            || interactions.iter().any(|i| {
                matches!(
                    i.as_str(),
                    "flammable" | "explosive_resist" | "ignite_halfhealth" | "explode_fire" | "firstimpact_break"
                )
            });
        let physbox = is_physbox(&class);
        // A physbox breaks as a func_breakable: by its own health, unless
        // "Only Break on Trigger" (physics_brushes.md 2.1 step 2).
        let health = if physbox { map_health } else { health };
        let events_only = if physbox {
            health <= 0 || e.has_flag(SF_PHYSBOX_ONLY_BREAK_ON_TRIGGER)
        } else if client {
            health == 0
        } else {
            health <= 0 || !breaks_into
        };
        let health = if events_only { 0 } else { health };
        let mut dmg = [1.0f32; 3];
        if let Some(v) = e.kv(PROP_DAMAGE_KEY) {
            for (i, x) in v.split_whitespace().take(3).enumerate() {
                dmg[i] = super::value::atof(x);
            }
        }
        let scale = e.kv_f("physdamagescale");
        let physics = class.starts_with("prop_physics") || physbox;
        let (motion_flag, asleep_flag) = if physbox {
            (SF_PHYSBOX_MOTION_DISABLED, SF_PHYSBOX_START_ASLEEP)
        } else {
            (SF_PHYSPROP_MOTION_DISABLED, SF_PHYSPROP_START_ASLEEP)
        };
        let force_to_enable = e.kv_f("forcetoenablemotion");
        let damage_to_enable = e.kv_f("damagetoenablemotion");
        Prop {
            health,
            max_health: health.max(1),
            events_only,
            broken: false,
            visible: true,
            solid: true,
            skin: e.kv_i("skin").max(0),
            body_group: e.kv("SetBodyGroup").map(super::value::atoi),
            sequence: None,
            sequence_serial: 0,
            default_sequence: e.kv("DefaultAnim").unwrap_or("").trim().to_string(),
            dmg,
            client,
            explode,
            explode_sound: e.kv(PROP_EXPLODE_SOUND_KEY).unwrap_or("").to_string(),
            break_sound: e.kv(PROP_BREAK_SOUND_KEY).unwrap_or("").to_string(),
            interactions,
            impact_scale: if scale > 0.0 { scale } else { DEFAULT_IMPACT_SCALE },
            table: ImpactTable::named(e.kv(PROP_TABLE_KEY).unwrap_or("")),
            mass: e.kv_f(PROP_MASS_KEY).max(0.0),
            physics,
            frozen: physics
                && (e.has_flag(motion_flag) || force_to_enable > 0.0 || damage_to_enable > 0.0),
            force_to_enable,
            damage_to_enable,
            asleep: physics && e.has_flag(asleep_flag),
            spawn_tick: tick,
            last_attacker: None,
            pressure_breaker: None,
            pressure_pending: false,
            burning: false,
            bounds: None,
        }
    }
}

/// Client props have no name: map logic can't reach them.
pub(super) fn activate(w: &mut LogicWorld, id: EntId) {
    if prop(w, id).is_some_and(|p| p.client)
        && let Some(e) = w.get_mut(id)
    {
        e.targetname.clear();
    }
}

/// What the map draws of a prop the logic keeps (`LogicWorld::prop_states`).
#[derive(Clone, Debug, PartialEq)]
pub struct PropState {
    pub id: EntId,
    /// Index in the map's entity list.
    pub index: usize,
    pub visible: bool,
    pub solid: bool,
    /// Takes damage from weapons (every prop does, if only as events).
    pub damageable: bool,
    pub skin: i32,
    pub body_group: Option<i32>,
    /// The sequence to play and its serial (a new serial restarts it).
    pub sequence: Option<(String, u32)>,
    pub default_sequence: String,
    /// Started asleep (and hasn't woken yet).
    pub asleep: bool,
}

/// What exploding props do: magnitude (health points), radius (units),
/// whose damage it is and the sound.
#[derive(Clone, Debug, PartialEq)]
pub struct PropExplosion {
    pub damage: f32,
    pub radius: f32,
    pub attacker: Option<Who>,
    pub sound: Option<String>,
}

/// What the host does to a physics prop's body.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Motion {
    Enable,
    Disable,
    Wake,
    Sleep,
}

fn prop(w: &mut LogicWorld, id: EntId) -> Option<&mut Prop> {
    match w.get_mut(id).map(|e| &mut e.class) {
        Some(Class::Prop(p)) => Some(p),
        _ => None,
    }
}

// ------------------------------------------------------- impact damage

/// A physics impact damage table (spec 5): linear entries (squared speed
/// threshold in in²/s², damage), the small-object rule (mass below which
/// damage is capped, the cap) and the large-object rule (mass threshold,
/// energy scale, falling emphasis).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ImpactTable {
    pub linear: &'static [(f32, f32)],
    pub small_mass: f32,
    pub small_cap: f32,
    pub large_mass: f32,
    pub large_scale: f32,
    pub falling: f32,
    /// The filters players use (spec 6): minimum speed², minimum mass,
    /// small-object minimum speed².
    pub min_speed_sqr: f32,
    pub min_mass: f32,
    pub small_min_speed_sqr: f32,
    /// The receiver's own speed drop below this counts as none.
    pub my_min_velocity: f32,
}

const fn sq(x: f32) -> f32 {
    x * x
}

pub const DEFAULT_TABLE: ImpactTable = ImpactTable {
    linear: &[
        (sq(150.0), 5.0),
        (sq(250.0), 10.0),
        (sq(350.0), 50.0),
        (sq(500.0), 100.0),
        (sq(1000.0), 500.0),
    ],
    small_mass: 5.0,
    small_cap: 5.0,
    large_mass: 500.0,
    large_scale: 4.0,
    falling: 5.0,
    min_speed_sqr: sq(24.0),
    min_mass: 2.0,
    small_min_speed_sqr: sq(36.0),
    my_min_velocity: 0.0,
};

/// Its last entry is out of order (spec quirk, kept).
pub const GLASS_TABLE: ImpactTable = ImpactTable {
    linear: &[
        (sq(25.0), 10.0),
        (sq(50.0), 20.0),
        (sq(100.0), 50.0),
        (sq(200.0), 75.0),
        (sq(500.0), 100.0),
        (sq(250.0), 500.0),
    ],
    small_mass: 1.0,
    small_cap: 10.0,
    large_mass: 50.0,
    large_scale: 4.0,
    falling: 0.0,
    min_speed_sqr: sq(8.0),
    min_mass: 2.0,
    small_min_speed_sqr: sq(8.0),
    my_min_velocity: 0.0,
};

pub const PLAYER_TABLE: ImpactTable = ImpactTable {
    linear: &[
        (sq(150.0), 5.0),
        (sq(250.0), 10.0),
        (sq(450.0), 20.0),
        (sq(550.0), 50.0),
        (sq(700.0), 100.0),
        (sq(1000.0), 500.0),
    ],
    small_mass: 5.0,
    small_cap: 5.0,
    large_mass: 0.0,
    large_scale: 1.0,
    falling: 2.0,
    min_speed_sqr: sq(24.0),
    min_mass: 2.0,
    small_min_speed_sqr: sq(36.0),
    my_min_velocity: 320.0,
};

impl ImpactTable {
    /// A prop's `damage_table` (unknown names: the default).
    pub fn named(name: &str) -> ImpactTable {
        match name.trim().to_ascii_lowercase().as_str() {
            "glass" => GLASS_TABLE,
            "player" => PLAYER_TABLE,
            _ => DEFAULT_TABLE,
        }
    }

    /// The damage for an energy: the entry before the first whose
    /// threshold exceeds it.
    pub fn lookup(&self, energy: f32) -> f32 {
        let mut out = 0.0;
        for (threshold, damage) in self.linear {
            if *threshold > energy {
                break;
            }
            out = *damage;
        }
        out
    }
}

/// One side of a collision for impact damage (units/s, kg).
#[derive(Clone, Copy, Debug, Default)]
pub struct Impactor {
    pub mass: f32,
    /// Velocity before and after the physics step (entity space, Z up).
    pub before: Vec3,
    pub after: Vec3,
    /// A static body (the world).
    pub fixed: bool,
}

/// The crush damage the receiver `me` (energy scale `scale`) takes from
/// a collision with `other` (spec 5 steps 3-7; with `filters`, the
/// player rules of spec 6).
pub fn impact_damage(table: &ImpactTable, scale: f32, me: Impactor, other: Impactor, filters: bool) -> f32 {
    if scale <= 0.0 || me.mass <= 0.0 {
        return 0.0;
    }
    let drop = |i: &Impactor| {
        if i.fixed {
            0.0
        } else {
            i.before.length() - i.after.length()
        }
    };
    let mut ds = drop(&me);
    if ds > 0.0 && ds < table.my_min_velocity {
        ds = 0.0;
    }
    let d_o = drop(&other);
    if filters {
        let speed_sqr = (other.before - me.before).length_squared();
        if other.mass < table.min_mass
            || (other.mass < table.small_mass && speed_sqr < table.small_min_speed_sqr)
            || speed_sqr < table.min_speed_sqr
        {
            return 0.0;
        }
    }
    let large = if !other.fixed && other.mass >= table.large_mass {
        // Falling onto it: its z speed dropped while it moved down.
        let dz = other.before.z - other.after.z;
        let k = if d_o > 0.0 && dz < 0.0 && other.before.z < 0.0 {
            (dz / d_o).abs()
        } else {
            0.0
        };
        table.large_scale * (1.0 + k * (table.falling - 1.0))
    } else {
        1.0
    };
    let energy = ds * ds * me.mass
        + if other.fixed {
            0.0
        } else {
            d_o * d_o * other.mass * large
        };
    let mut damage = table.lookup(energy * scale / me.mass);
    if !other.fixed && other.mass < table.small_mass && table.small_cap > 0.0 {
        damage = damage.clamp(0.0, table.small_cap);
    }
    damage
}

// ------------------------------------------------------------ damage

/// One hit on a prop (entity space).
#[derive(Clone, Copy, Debug)]
pub struct Hit {
    /// Health points.
    pub amount: f32,
    pub kind: DamageKind,
    pub attacker: Option<Who>,
    pub point: Vec3,
    pub dir: Vec3,
    /// The push's size, kg·units/s.
    pub force: f32,
    /// Direct damage (an entity flame burning its own target).
    pub direct: bool,
}

/// The multiplier for a kind of damage (spec 4 step 6; the knife counts
/// as a club, breakables.md Q2).
fn multiplier(p: &Prop, kind: DamageKind) -> f32 {
    match kind {
        DamageKind::Bullet => p.dmg[0],
        DamageKind::Melee => p.dmg[1],
        DamageKind::Blast => p.dmg[2],
        _ => 1.0,
    }
}

/// Damage to a prop. Returns false when `id` is not a prop.
pub(super) fn prop_damage(w: &mut LogicWorld, id: EntId, hit: Hit) -> bool {
    let Some(p) = prop(w, id).cloned() else { return false };
    if p.broken {
        return true;
    }
    if p.client {
        client_damage(w, id, &p, hit);
        return true;
    }
    let Some(e) = w.get(id) else { return true };
    // The damage filter tests the attacker; minhealthdmg the raw amount.
    let filter = e.kv("damagefilter").unwrap_or("").trim().to_string();
    if !filter.is_empty()
        && let Some(f) = w.find(&filter)
        && !super::classes::filter_passes(w, f, hit.attacker)
    {
        return true;
    }
    if hit.amount < e.kv_f("minhealthdmg") {
        return true;
    }
    let mut scaled = hit.amount * multiplier(&p, hit.kind);
    // Fire rules (spec 4 step 8; specs/source/fire.md 4.2). The
    // explosive-resist rule is left out (no stock prop has it).
    let has = |i: &str| p.interactions.iter().any(|x| x == i);
    if p.burning && hit.kind == DamageKind::Burn && !hit.direct {
        return true;
    }
    let deadly = scaled >= p.health as f32;
    let mut ignite = false;
    if !deadly && matches!(hit.kind, DamageKind::Blast | DamageKind::Burn) {
        ignite = true;
    }
    if !deadly && hit.kind == DamageKind::Bullet && has("ignite_halfhealth") {
        if !p.burning && p.health as f32 - scaled <= (p.max_health / 2) as f32 {
            let p = prop(w, id).unwrap();
            p.health = p.max_health;
            ignite = true;
        } else if p.burning {
            scaled = p.health as f32;
        }
    }
    if ignite {
        super::fire::ignite_from_damage(w, id);
    }
    let spawn_tick = w.tick == p.spawn_tick;
    let p = prop(w, id).unwrap();
    if matches!(hit.attacker, Some(Who::Player(_))) {
        p.last_attacker = hit.attacker;
    }
    let mut killed = false;
    if !p.events_only && !spawn_tick {
        p.health = (p.health as f32 - scaled).trunc() as i32;
        killed = p.health <= 0;
    }
    let ratio = if p.events_only {
        0.0
    } else {
        (p.health as f32 / p.max_health as f32).clamp(0.0, 1.0)
    };
    let health = p.health;
    if killed {
        // Breaker = the inflictor (the shooter, here); the outputs below
        // queue after OnBreak.
        prop_break(w, id, hit.attacker, hit.attacker);
    }
    w.fire_output(id, "OnHealthChanged", hit.attacker, Value::Float(ratio));
    w.fire_output(id, "OnTakeDamage", hit.attacker, Value::Void);
    if killed {
        return true;
    }
    // A push wakes a prop asleep since it spawned (OnAwakened comes back
    // from the host).
    if prop(w, id).is_some_and(|p| p.asleep && !p.frozen) && hit.force > 0.0 {
        w.effects.push(Effect::PropMotion {
            id,
            motion: Motion::Wake,
        });
    }
    // Motion enabling (spec 4 step 13).
    let p = prop(w, id).unwrap();
    if p.force_to_enable > 0.0 && hit.force >= p.force_to_enable {
        p.force_to_enable = 0.0;
        enable_motion(w, id, true);
    }
    let p = prop(w, id).unwrap();
    if p.damage_to_enable > 0.0 && (health as f32) < p.damage_to_enable {
        p.damage_to_enable = 0.0;
        enable_motion(w, id, true);
    }
    true
}

/// A client prop's damage (spec 3.2): 30 per bullet, a blast's lifted
/// push size, 50 for anything else but impacts.
fn client_damage(w: &mut LogicWorld, id: EntId, p: &Prop, hit: Hit) {
    if p.events_only {
        return;
    }
    let damage = match hit.kind {
        DamageKind::Bullet => CLIENT_BULLET_DAMAGE,
        DamageKind::Blast => {
            if hit.amount <= 1.0 {
                return;
            }
            (hit.dir.normalize_or_zero() * hit.amount + Vec3::Z * CLIENT_BLAST_LIFT)
                .length()
                .trunc() as i32
        }
        DamageKind::Crush | DamageKind::Fall | DamageKind::Generic | DamageKind::Burn => return,
        DamageKind::Melee => CLIENT_OTHER_DAMAGE,
    };
    let p = prop(w, id).unwrap();
    p.health -= damage;
    if p.health <= 0 {
        prop_break(w, id, hit.attacker, hit.attacker);
    }
}

/// Unfreeze a frozen physics prop (host: `Effect::PropMotion`), firing
/// OnMotionEnabled when `output`.
fn enable_motion(w: &mut LogicWorld, id: EntId, output: bool) {
    let Some(p) = prop(w, id) else { return };
    if !p.physics {
        return;
    }
    p.frozen = false;
    w.effects.push(Effect::PropMotion {
        id,
        motion: Motion::Enable,
    });
    if output {
        w.fire_output(id, "OnMotionEnabled", Some(Who::Ent(id)), Value::Void);
    }
}

/// New health from an input: OnHealthChanged if it changed, break at 0.
fn set_health(w: &mut LogicWorld, id: EntId, new: i32, activator: Option<Who>) {
    let Some(p) = prop(w, id) else { return };
    if p.broken {
        return;
    }
    let changed = new != p.health;
    p.health = new;
    let ratio = (new as f32 / p.max_health as f32).clamp(0.0, 1.0);
    if changed {
        w.fire_output(id, "OnHealthChanged", activator, Value::Float(ratio));
    }
    if new <= 0 {
        // The prop itself is the explosion's attacker (spec 7 step 5).
        prop_break(w, id, activator, Some(Who::Ent(id)));
    }
}

/// The entities parented to `name` (by `parentname`, an attachment after
/// a comma ignored), and theirs.
fn children(w: &LogicWorld, id: EntId) -> Vec<EntId> {
    let mut out = Vec::new();
    let mut names = vec![w.get(id).map(|e| e.targetname.clone()).unwrap_or_default()];
    while let Some(name) = names.pop() {
        if name.is_empty() {
            continue;
        }
        for c in w.ids() {
            let Some(e) = w.get(c) else { continue };
            let parent = e.kv("parentname").unwrap_or("").split(',').next().unwrap_or("").trim();
            if c != id && !out.contains(&c) && !parent.is_empty() && parent.eq_ignore_ascii_case(&name) {
                out.push(c);
                names.push(e.targetname.clone());
            }
        }
    }
    out
}

/// Break a prop now (spec 7): OnBreak (activator = `breaker`), then the
/// host plays its break sound, explodes it (as `attacker`'s unless a
/// player hurt it last) and throws its pieces; it and everything parented
/// to it are removed (its name at once).
pub(super) fn prop_break(w: &mut LogicWorld, id: EntId, breaker: Option<Who>, attacker: Option<Who>) {
    let Some(p) = prop(w, id) else { return };
    if p.broken {
        return;
    }
    p.broken = true;
    p.solid = false;
    let p = p.clone();
    let doomed = children(w, id);
    if !p.client {
        w.fire_output(id, "OnBreak", breaker, Value::Void);
    }
    let (d, mut r) = (p.explode.0.trunc(), p.explode.1.trunc());
    if r <= 0.0 {
        r = EXPLODE_RADIUS_FACTOR * d;
    }
    let explode_fire = p.interactions.iter().any(|i| i == "explode_fire");
    let explode = if (d > 0.0 || r > 0.0) && !p.client {
        Some(PropExplosion {
            damage: d,
            radius: r,
            attacker: p.last_attacker.or(attacker),
            sound: Some(p.explode_sound.clone()).filter(|s| !s.is_empty()),
        })
    } else if explode_fire && !p.client {
        // A 1-damage explosion of the same radius. Its ignition of the
        // combat characters in the radius uses the NPC-only rule, which
        // players refuse (specs/source/fire.md 4.2): nothing to do here.
        Some(PropExplosion {
            damage: 1.0,
            radius: r,
            attacker: p.last_attacker.or(attacker),
            sound: Some(p.explode_sound.clone()).filter(|s| !s.is_empty()),
        })
    } else {
        None
    };
    w.effects.push(Effect::PropBreak {
        id,
        sound: Some(p.break_sound.clone()).filter(|s| !s.is_empty() && !p.client),
        explode,
    });
    if let Some(e) = w.get_mut(id) {
        e.targetname.clear();
    }
    w.kill(id);
    // Its flame goes with it (General.StopBurning).
    super::fire::remove_flames_on(w, Who::Ent(id));
    for c in doomed {
        w.kill(c);
        if let Some(e) = w.get_mut(c) {
            e.targetname.clear();
        }
    }
}

/// Prop inputs; false when not one of them.
pub(super) fn prop_input(w: &mut LogicWorld, id: EntId, input: &str, value: &Value, activator: Option<Who>) -> bool {
    let Some(p) = prop(w, id).cloned() else { return false };
    match input {
        "break" => prop_break(w, id, activator, Some(Who::Ent(id))),
        "sethealth" | "addhealth" | "removehealth" => {
            let Some(n) = w.need_int(value, input) else { return true };
            let new = match input {
                "sethealth" => n,
                "addhealth" => p.health + n,
                _ => p.health - n,
            };
            set_health(w, id, new, activator);
        }
        "physdamagescale" => {
            let Some(v) = w.need_float(value, input) else {
                return true;
            };
            prop(w, id).unwrap().impact_scale = v;
        }
        "wake" if p.physics => w.effects.push(Effect::PropMotion {
            id,
            motion: Motion::Wake,
        }),
        "sleep" if p.physics => w.effects.push(Effect::PropMotion {
            id,
            motion: Motion::Sleep,
        }),
        "enablemotion" if p.physics => enable_motion(w, id, true),
        "disablemotion" if p.physics => {
            prop(w, id).unwrap().frozen = true;
            w.effects.push(Effect::PropMotion {
                id,
                motion: Motion::Disable,
            });
        }
        // A map ragdoll (physics_brushes.md 6): its bodies aren't
        // simulated, so motion inputs do nothing; FadeAndRemove removes it
        // (at once: its fade isn't drawn).
        "enablemotion" | "disablemotion" | "startragdollboogie" => {}
        // Whether damage pushes the body: our pushes don't read it
        // (tech-debt).
        "enabledamageforces" | "disabledamageforces" => {}
        "fadeandremove" => w.kill(id),
        "enable" | "turnon" => prop(w, id).unwrap().visible = true,
        "disable" | "turnoff" => prop(w, id).unwrap().visible = false,
        "enablecollision" => prop(w, id).unwrap().solid = true,
        "disablecollision" => prop(w, id).unwrap().solid = false,
        "skin" => {
            let Some(n) = w.need_int(value, input) else { return true };
            prop(w, id).unwrap().skin = n.max(0);
        }
        "setbodygroup" => {
            let Some(n) = w.need_int(value, input) else { return true };
            prop(w, id).unwrap().body_group = Some(n.max(0));
        }
        "setanimation" => {
            let Some(name) = w.need_str(value, input) else {
                return true;
            };
            let p = prop(w, id).unwrap();
            p.sequence = Some(name.trim().to_string());
            p.sequence_serial += 1;
        }
        "ignite" | "ignitelifetime" | "ignitenumhitboxfires" | "ignitehitboxfirescale" => {
            return super::fire::ignite_input(w, Who::Ent(id), input, value);
        }
        "setdefaultanimation" => {
            let Some(name) = w.need_str(value, input) else {
                return true;
            };
            prop(w, id).unwrap().default_sequence = name.trim().to_string();
        }
        _ => return false,
    }
    true
}

/// A pressure break comes due.
pub(super) fn think(w: &mut LogicWorld, id: EntId) {
    let Some(p) = prop(w, id) else { return };
    if p.pressure_pending {
        p.pressure_pending = false;
        let breaker = p.pressure_breaker;
        prop_break(w, id, breaker, Some(Who::Ent(id)));
    }
}

impl LogicWorld {
    /// Damage dealt to a prop with the push that came with it; false
    /// when `id` is not a prop (see `damage` for breakables too).
    pub fn prop_hit(&mut self, id: EntId, hit: Hit) -> bool {
        prop_damage(self, id, hit)
    }

    /// A prop that started asleep woke up: OnAwakened, once.
    pub fn prop_awakened(&mut self, id: EntId) {
        let Some(p) = prop(self, id) else { return };
        if !p.asleep || p.broken {
            return;
        }
        p.asleep = false;
        self.fire_output(id, "OnAwakened", Some(Who::Ent(id)), Value::Void);
    }

    /// Break-on-pressure props someone stands on: break after their
    /// `PressureDelay` (after movement; spec 7.1).
    pub fn pressure_props(&mut self) {
        for id in self.ids() {
            let Some(e) = self.get(id) else { continue };
            let Class::Prop(p) = &e.class else { continue };
            if p.broken || !e.has_flag(SF_PHYSPROP_BREAK_ON_PRESSURE) {
                continue;
            }
            let delay = e.kv_f("PressureDelay");
            let Some(on) = self
                .players
                .iter()
                .find(|pl| pl.alive && pl.ground == Some(id))
                .map(|pl| pl.entity)
            else {
                continue;
            };
            let p = prop(self, id).unwrap();
            p.pressure_breaker = Some(Who::Player(on));
            if !p.pressure_pending {
                p.pressure_pending = true;
                self.think_in(id, delay.max(0.0));
            }
        }
    }

    /// What impact damage a prop takes (None: none): its table and energy
    /// scale, and its mass (kg).
    pub fn prop_impact(&self, id: EntId) -> Option<(ImpactTable, f32, f32)> {
        let e = self.get(id)?;
        let Class::Prop(p) = &e.class else { return None };
        // Client props take none; "don't take physics damage" counts only
        // without health (spec 2.1 step 3).
        if p.broken || p.client || (p.events_only && e.has_flag(SF_PHYSPROP_DONT_TAKE_PHYSICS_DAMAGE)) {
            return None;
        }
        Some((p.table, p.impact_scale, p.mass))
    }

    /// Map props the logic keeps, in entity order.
    pub fn prop_states(&self) -> Vec<PropState> {
        self.ids()
            .into_iter()
            .filter_map(|id| {
                let e = self.get(id)?;
                let Class::Prop(p) = &e.class else { return None };
                Some(PropState {
                    id,
                    index: e.map_index?,
                    visible: p.visible,
                    solid: p.solid && !p.broken,
                    damageable: !p.broken,
                    skin: p.skin,
                    body_group: p.body_group,
                    sequence: p.sequence.clone().map(|s| (s, p.sequence_serial)),
                    default_sequence: p.default_sequence.clone(),
                    asleep: p.asleep,
                })
            })
            .collect()
    }

    /// A prop's health and max health.
    pub fn prop_health(&self, id: EntId) -> Option<(i32, i32)> {
        match self.get(id).map(|e| &e.class) {
            Some(Class::Prop(p)) => Some((p.health, p.max_health)),
            _ => None,
        }
    }

    /// Where a prop's body is now (world box, entity space).
    pub fn set_prop_bounds(&mut self, id: EntId, bounds: (Vec3, Vec3)) {
        if let Some(p) = prop(self, id) {
            p.bounds = Some(bounds);
        }
    }

    /// A prop's state.
    pub fn prop(&self, id: EntId) -> Option<&Prop> {
        match self.get(id).map(|e| &e.class) {
            Some(Class::Prop(p)) => Some(p),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn body(mass: f32, before: Vec3, after: Vec3) -> Impactor {
        Impactor {
            mass,
            before,
            after,
            fixed: false,
        }
    }

    const WORLD: Impactor = Impactor {
        mass: 0.0,
        before: Vec3::ZERO,
        after: Vec3::ZERO,
        fixed: true,
    };

    #[test]
    fn glass_table_lookup() {
        let got: Vec<f32> = [600.0, 625.0, 40000.0, 249999.0, 250000.0, 300000.0]
            .iter()
            .map(|e| GLASS_TABLE.lookup(*e))
            .collect();
        assert_eq!(got, vec![0.0, 10.0, 75.0, 75.0, 500.0, 500.0]);
    }

    #[test]
    fn crate_falling_onto_the_floor() {
        // 30 kg, scale 0.1, 600 -> 0: E' = 36000 -> 5.
        let me = body(30.0, Vec3::new(0.0, 0.0, -600.0), Vec3::ZERO);
        assert_eq!(impact_damage(&DEFAULT_TABLE, 0.1, me, WORLD, false), 5.0);
        // Thresholds: 5 from 474.34 in/s, 10 from 790.57.
        let at = |v: f32| impact_damage(&DEFAULT_TABLE, 0.1, body(30.0, Vec3::Z * -v, Vec3::ZERO), WORLD, false);
        assert_eq!((at(474.0), at(475.0), at(790.0), at(791.0)), (0.0, 5.0, 5.0, 10.0));
        // de_port's crate (scale 1) landing at 400: 160000 -> 50.
        assert_eq!(
            impact_damage(
                &DEFAULT_TABLE,
                1.0,
                body(30.0, Vec3::Z * -400.0, Vec3::ZERO),
                WORLD,
                false
            ),
            50.0
        );
    }

    #[test]
    fn crate_into_a_window_and_heavy_falling_objects() {
        // 30 kg crate at 300 stops on a 10 kg window: 27000 -> glass 50.
        let window = body(10.0, Vec3::ZERO, Vec3::ZERO);
        let crate_ = body(30.0, Vec3::X * 300.0, Vec3::ZERO);
        assert_eq!(impact_damage(&GLASS_TABLE, 0.1, window, crate_, false), 50.0);
        // 600 kg falling at 200 onto a 30 kg crate: 1.6e6 -> 500.
        let me = body(30.0, Vec3::ZERO, Vec3::ZERO);
        let heavy = body(600.0, Vec3::Z * -200.0, Vec3::ZERO);
        assert_eq!(impact_damage(&DEFAULT_TABLE, 0.1, me, heavy, false), 500.0);
    }

    #[test]
    fn objects_hitting_players() {
        let player = body(85.0, Vec3::ZERO, Vec3::ZERO);
        let hit = |m: f32, v: Vec3| impact_damage(&PLAYER_TABLE, 1.0, player, body(m, v, Vec3::ZERO), true);
        assert_eq!(hit(30.0, Vec3::X * 500.0), 10.0);
        // Falling straight down: x2.
        assert_eq!(hit(30.0, Vec3::Z * -500.0), 10.0);
        assert_eq!(hit(30.0, Vec3::Z * -550.0), 20.0);
        // A 4 kg can at 800: capped at 5; 1.5 kg: nothing.
        assert_eq!(hit(4.0, Vec3::X * 800.0), 5.0);
        assert_eq!(hit(1.5, Vec3::X * 800.0), 0.0);
    }
}
