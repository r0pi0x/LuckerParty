//! Prop entities in the logic: model doors (specs/source/doors_buttons.md,
//! "prop_door_rotating") turned like rotating brush doors, and props that
//! take damage and fire outputs (prop_dynamic, prop_physics*).
//!
//! Prop damage has no spec yet: it follows func_breakable's rules
//! (specs/source/breakables.md "Damage": the shared damage scaling,
//! minhealthdmg, integer health, OnHealthChanged with health / max health,
//! breaking at 0) with the props' own health (the model's prop_data, given
//! by the loader as `map::entities::PROP_HEALTH_KEY`). See
//! docs/tech-debt.md.

use bevy::prelude::*;

use super::breakables::scale_damage;
use super::classes::Class;
use super::movers::{Done, DoorState, Pusher};
use super::triggers::forward;
use super::value::Value;
use super::world::{Effect, EntId, LogicWorld, SOLID_SKIN, Who, place_hull};
use crate::core::DamageKind;
use crate::map::entities::{DOOR_CLOSE_KEY, DOOR_MOVE_KEY, DOOR_OPEN_KEY, PROP_HEALTH_KEY, entity_rotation};

/// prop_door_rotating defaults (doors_buttons.md constants).
pub const PROPDOOR_DEFAULT_DISTANCE: f32 = 90.0;
pub const PROPDOOR_DEFAULT_SPEED: f32 = 100.0;
/// A door tries to close this long after its return delay.
pub const PROPDOOR_RETURN_PAD: f32 = 0.1;

/// prop_door_rotating spawnflags.
pub const SF_DOOR_START_OPEN: u32 = 1;
pub const SF_DOOR_LOCKED: u32 = 2048;
pub const SF_DOOR_SILENT: u32 = 4096;
pub const SF_DOOR_USE_CLOSES: u32 = 8192;
pub const SF_DOOR_IGNORE_USE: u32 = 32768;

/// A model door (prop_door_rotating): turns about its origin (the hinge)
/// around Z.
#[derive(Clone, Debug)]
pub struct PropDoor {
    pub push: Pusher,
    pub state: DoorState,
    /// Closed angles, and the open ones each way (forward: away from
    /// someone behind the door; back: away from someone in front).
    pub closed: Vec3,
    pub open_forward: Vec3,
    pub open_back: Vec3,
    pub speed: f32,
    /// Seconds open before closing again; negative: never.
    pub return_delay: f32,
    /// 0 both ways, 1 forward only, 2 back only.
    pub opendir: i32,
    pub locked: bool,
    /// Doors (by name) that move with this one.
    pub slave: String,
    pub move_sound: String,
    pub open_sound: String,
    pub close_sound: String,
    pub locked_sound: String,
    pub unlocked_sound: String,
}

impl PropDoor {
    pub(super) fn spawn(w: &mut LogicWorld, id: EntId) -> PropDoor {
        let e = w.get(id).unwrap();
        let distance = match e.kv_f("distance") {
            d if d == 0.0 => PROPDOOR_DEFAULT_DISTANCE,
            d => d,
        };
        let speed = match e.kv_f("speed") {
            s if s <= 0.0 => PROPDOOR_DEFAULT_SPEED,
            s => s,
        };
        // Which way the leaf lies from the hinge: with the leaf on the
        // door's left (+Y), a positive turn swings it backward; a model
        // hinged on the left has its leaf on the right, which swaps the
        // two (spec "Open forward/back ... swapped if the hinge is on the
        // left").
        let (mut lo, mut hi) = (f32::MAX, f32::MIN);
        for p in e.hulls.iter().flat_map(|h| &h.points) {
            lo = lo.min(p.y);
            hi = hi.max(p.y);
        }
        let leaf_right = lo < hi && lo + hi < 0.0;
        let turn = |d: f32| Vec3::new(0.0, d, 0.0);
        let (forward_turn, back_turn) = if leaf_right {
            (turn(distance), turn(-distance))
        } else {
            (turn(-distance), turn(distance))
        };
        let sound = |keys: &[&str]| {
            keys.iter()
                .filter_map(|k| e.kv(k).filter(|s| !s.trim().is_empty()))
                .map(str::to_string)
                .next()
                .unwrap_or_default()
        };
        let mut door = PropDoor {
            push: Pusher::at(e.origin, e.angles),
            state: DoorState::Closed,
            closed: e.angles,
            open_forward: e.angles + forward_turn,
            open_back: e.angles + back_turn,
            speed,
            return_delay: e.kv("returndelay").map_or(-1.0, super::value::atof),
            opendir: e.kv_i("opendir"),
            locked: e.has_flag(SF_DOOR_LOCKED),
            slave: e.kv("slavename").unwrap_or("").trim().to_string(),
            move_sound: sound(&["soundmoveoverride", DOOR_MOVE_KEY]),
            open_sound: sound(&["soundopenoverride", DOOR_OPEN_KEY]),
            close_sound: sound(&["soundcloseoverride", DOOR_CLOSE_KEY]),
            locked_sound: sound(&["soundlockedoverride"]),
            unlocked_sound: sound(&["soundunlockedoverride"]),
        };
        // Spawn position: 1 open forward, 2 open back, 3 ajar; the "starts
        // open" flag opens it forward.
        let open_at = match e.kv_i("spawnpos") {
            1 => Some(door.open_forward),
            2 => Some(door.open_back),
            3 => Some(e.vector_kv("ajarangles")),
            _ if e.has_flag(SF_DOOR_START_OPEN) => Some(door.open_forward),
            _ => None,
        };
        if let Some(a) = open_at {
            door.state = DoorState::Open;
            door.push.angles = a;
        }
        door
    }
}

/// A prop entity's damage and visibility state.
#[derive(Clone, Debug)]
pub struct Prop {
    pub health: i32,
    /// 0: takes no health damage (damage still fires its outputs).
    pub max_health: i32,
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
}

impl Prop {
    pub(super) fn spawn(w: &mut LogicWorld, id: EntId) -> Prop {
        let e = w.get(id).unwrap();
        let health = match e.kv_i("health") {
            h if h > 0 => h,
            _ => e.kv_i(PROP_HEALTH_KEY).max(0),
        };
        Prop {
            health,
            max_health: health,
            broken: false,
            visible: true,
            solid: true,
            skin: e.kv_i("skin").max(0),
            body_group: e.kv("SetBodyGroup").map(super::value::atoi),
            sequence: None,
            sequence_serial: 0,
            default_sequence: e.kv("DefaultAnim").unwrap_or("").trim().to_string(),
        }
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
    /// Takes damage from weapons (health, or damage outputs).
    pub damageable: bool,
    pub skin: i32,
    pub body_group: Option<i32>,
    /// The sequence to play and its serial (a new serial restarts it).
    pub sequence: Option<(String, u32)>,
    pub default_sequence: String,
}

/// Whether a class is a prop the logic keeps (not a door).
pub fn is_prop_class(classname: &str) -> bool {
    let c = classname.to_ascii_lowercase();
    c.starts_with("prop_dynamic") || c.starts_with("prop_physics")
}

fn door(w: &mut LogicWorld, id: EntId) -> Option<&mut PropDoor> {
    match w.get_mut(id).map(|e| &mut e.class) {
        Some(Class::PropDoor(d)) => Some(d),
        _ => None,
    }
}

fn prop(w: &mut LogicWorld, id: EntId) -> Option<&mut Prop> {
    match w.get_mut(id).map(|e| &mut e.class) {
        Some(Class::Prop(p)) => Some(p),
        _ => None,
    }
}

fn sound(w: &mut LogicWorld, id: EntId, entry: &str) {
    if entry.is_empty() {
        return;
    }
    let Some(e) = w.get(id) else { return };
    if e.has_flag(SF_DOOR_SILENT) {
        return;
    }
    let at = match &e.class {
        Class::PropDoor(d) => d.push.origin,
        _ => e.origin,
    };
    w.effects.push(Effect::Sound {
        entry: entry.to_string(),
        at,
    });
}

/// Where `who` stands (entity space).
fn origin_of(w: &LogicWorld, who: Who) -> Option<Vec3> {
    match who {
        Who::Player(p) => w.player(p).map(|p| p.origin),
        Who::Ent(e) => w.get(e).map(|e| match super::movers::pusher(&e.class) {
            Some(p) => p.origin,
            None => e.origin,
        }),
    }
}

/// Whether a living player is in the way of the door turning from where
/// it is to `goal` (sampled every 15 degrees).
fn swing_blocked(w: &LogicWorld, id: EntId, goal: Vec3) -> bool {
    let Some(e) = w.get(id) else { return false };
    let Class::PropDoor(d) = &e.class else { return false };
    let from = d.push.angles;
    let steps = ((goal - from).length() / 15.0).ceil().max(1.0) as usize;
    (1..=steps).any(|k| {
        let a = from + (goal - from) * (k as f32 / steps as f32);
        let rot = entity_rotation(a);
        let brushes: Vec<_> = e.hulls.iter().map(|h| place_hull(h, rot, d.push.origin)).collect();
        w.players.iter().filter(|p| p.alive).any(|p| {
            let (half, centre) = ((p.maxs - p.mins) / 2.0, p.origin + (p.mins + p.maxs) / 2.0);
            brushes.iter().any(|b| b.overlaps_box(centre, half, SOLID_SKIN))
        })
    })
}

/// The open angles for an opening by `user` (spec "Direction").
fn open_angles(w: &LogicWorld, id: EntId, user: Option<Who>) -> Vec3 {
    let Some(Class::PropDoor(d)) = w.get(id).map(|e| &e.class) else {
        return Vec3::ZERO;
    };
    match d.opendir {
        1 => return d.open_forward,
        2 => return d.open_back,
        _ => {}
    }
    let Some(at) = user.and_then(|u| origin_of(w, u)) else {
        return d.open_forward;
    };
    let f = forward(Vec3::new(0.0, d.closed.y, 0.0));
    let (first, other) = if f.dot(at) > f.dot(d.push.origin) {
        (d.open_back, d.open_forward)
    } else {
        (d.open_forward, d.open_back)
    };
    // A player opening it into someone: the other way, if that is clear.
    if matches!(user, Some(Who::Player(_))) && swing_blocked(w, id, first) && !swing_blocked(w, id, other) {
        other
    } else {
        first
    }
}

/// The doors that move with this one (`slavename`).
fn slaves(w: &LogicWorld, id: EntId) -> Vec<EntId> {
    let Some(Class::PropDoor(d)) = w.get(id).map(|e| &e.class) else {
        return Vec::new();
    };
    if d.slave.is_empty() {
        return Vec::new();
    }
    w.resolve(&d.slave, None, Some(Who::Ent(id)))
        .into_iter()
        .filter_map(|x| match x {
            Who::Ent(e) if e != id && matches!(w.get(e).map(|e| &e.class), Some(Class::PropDoor(_))) => Some(e),
            _ => None,
        })
        .collect()
}

/// Start turning open (away from `user`) and fire OnOpen.
fn start_open(w: &mut LogicWorld, id: EntId, user: Option<Who>) {
    let target = open_angles(w, id, user);
    let Some(d) = door(w, id) else { return };
    let quiet = d.push.moving();
    let snd = d.move_sound.clone();
    d.state = DoorState::Opening;
    let speed = d.speed;
    d.push.rotate_to(target, speed, Done::PropDoorOpened);
    if let Some(e) = w.get_mut(id) {
        e.next_think = None;
    }
    if !quiet {
        sound(w, id, &snd);
    }
    w.fire_output(id, "OnOpen", Some(Who::Ent(id)), Value::Void);
}

/// Start turning shut and fire OnClose.
fn start_close(w: &mut LogicWorld, id: EntId) {
    let Some(d) = door(w, id) else { return };
    let quiet = d.push.moving();
    let snd = d.move_sound.clone();
    d.state = DoorState::Closing;
    let (closed, speed) = (d.closed, d.speed);
    d.push.rotate_to(closed, speed, Done::PropDoorClosed);
    if let Some(e) = w.get_mut(id) {
        e.next_think = None;
    }
    if !quiet {
        sound(w, id, &snd);
    }
    w.fire_output(id, "OnClose", Some(Who::Ent(id)), Value::Void);
}

/// Open (with its slaves), unless open or opening.
fn open(w: &mut LogicWorld, id: EntId, user: Option<Who>) {
    for d in std::iter::once(id).chain(slaves(w, id)) {
        if door(w, d).is_some_and(|x| !matches!(x.state, DoorState::Open | DoorState::Opening)) {
            start_open(w, d, user);
        }
    }
}

/// Close (with its slaves), unless closed or closing.
fn close(w: &mut LogicWorld, id: EntId) {
    for d in std::iter::once(id).chain(slaves(w, id)) {
        if door(w, d).is_some_and(|x| !matches!(x.state, DoorState::Closed | DoorState::Closing)) {
            start_close(w, d);
        }
    }
}

/// +use or the Use input (spec "Use").
pub(super) fn door_use(w: &mut LogicWorld, id: EntId, user: Option<Who>) {
    let flags = w.get(id).map_or(0, |e| e.spawnflags);
    if flags & SF_DOOR_IGNORE_USE != 0 && matches!(user, Some(Who::Player(_))) {
        return;
    }
    let Some(d) = door(w, id).cloned() else { return };
    let use_closes = flags & SF_DOOR_USE_CLOSES != 0;
    match d.state {
        DoorState::Closed | DoorState::Open if d.state == DoorState::Closed || use_closes => {
            if d.locked {
                sound(w, id, &d.locked_sound);
                w.fire_output(id, "OnLockedUse", user, Value::Void);
            } else if d.state == DoorState::Closed {
                sound(w, id, &d.unlocked_sound);
                open(w, id, user);
            } else {
                close(w, id);
            }
        }
        DoorState::Opening if use_closes => close(w, id),
        DoorState::Closing => open(w, id, user),
        _ => {}
    }
}

/// prop_door_rotating inputs; false when not one of them.
pub(super) fn door_input(w: &mut LogicWorld, id: EntId, input: &str, value: &Value, activator: Option<Who>) -> bool {
    let Some(d) = door(w, id).cloned() else { return false };
    let opening = matches!(d.state, DoorState::Open | DoorState::Opening);
    match input {
        // Open and Close fire their output here and again as the door
        // starts moving (spec quirk: twice).
        "open" | "openawayfrom" => {
            if d.locked || opening {
                return true;
            }
            let user = if input == "openawayfrom" {
                let Some(name) = w.need_str(value, input) else { return true };
                w.resolve(&name, activator, None).into_iter().next()
            } else {
                None
            };
            w.fire_output(id, "OnOpen", Some(Who::Ent(id)), Value::Void);
            open(w, id, user);
        }
        "close" => {
            if matches!(d.state, DoorState::Closed | DoorState::Closing) {
                return true;
            }
            w.fire_output(id, "OnClose", Some(Who::Ent(id)), Value::Void);
            close(w, id);
        }
        "toggle" => {
            if opening {
                close(w, id);
            } else if !d.locked {
                open(w, id, None);
            }
        }
        "lock" => door(w, id).unwrap().locked = true,
        "unlock" => door(w, id).unwrap().locked = false,
        "setspeed" => {
            let Some(v) = w.need_float(value, input) else { return true };
            door(w, id).unwrap().speed = v.max(1e-3);
        }
        _ => return false,
    }
    true
}

/// Fully open or fully closed.
pub(super) fn door_arrived(w: &mut LogicWorld, id: EntId, opened: bool) {
    let Some(d) = door(w, id) else { return };
    if opened {
        d.state = DoorState::Open;
        let (snd, delay) = (d.open_sound.clone(), d.return_delay);
        sound(w, id, &snd);
        w.fire_output(id, "OnFullyOpen", Some(Who::Ent(id)), Value::Void);
        if delay >= 0.0 {
            w.think_in(id, delay + PROPDOOR_RETURN_PAD);
        }
    } else {
        d.state = DoorState::Closed;
        let snd = d.close_sound.clone();
        sound(w, id, &snd);
        w.fire_output(id, "OnFullyClosed", Some(Who::Ent(id)), Value::Void);
    }
}

/// The auto-close think: close if nobody is in the way, else try again
/// after the return delay.
pub(super) fn door_think(w: &mut LogicWorld, id: EntId) {
    let Some(d) = door(w, id).cloned() else { return };
    if d.state != DoorState::Open {
        return;
    }
    if swing_blocked(w, id, d.closed) {
        w.think_in(id, d.return_delay.max(0.0));
    } else {
        close(w, id);
    }
}

/// Blocked by a player: it holds still (the push step is undone) and
/// fires its blocked output when the blocking starts; no damage.
pub(super) fn door_blocked(w: &mut LogicWorld, id: EntId, blocker: Who, started: bool) {
    let Some(d) = door(w, id) else { return };
    if started {
        let out = if d.state == DoorState::Opening {
            "OnBlockedOpening"
        } else {
            "OnBlockedClosing"
        };
        w.fire_output(id, out, Some(blocker), Value::Void);
    }
}

// ------------------------------------------------------------- props

/// Damage to a prop (`amount` in health points). Returns false when `id`
/// is not a prop.
pub(super) fn prop_damage(w: &mut LogicWorld, id: EntId, amount: f32, kind: DamageKind, attacker: Option<Who>) -> bool {
    let Some(p) = prop(w, id).cloned() else { return false };
    if p.broken {
        return true;
    }
    let min = w.get(id).map_or(0.0, |e| e.kv_f("minhealthdmg"));
    if amount < min {
        return true;
    }
    w.fire_output(id, "OnTakeDamage", attacker, Value::Void);
    if p.max_health <= 0 {
        // No health: the hit is only an event. OnHealthChanged still
        // fires (de_nuke's fire extinguishers and cs_office's projector
        // rely on it), with the full ratio.
        w.fire_output(id, "OnHealthChanged", attacker, Value::Float(1.0));
        return true;
    }
    let new = (p.health as f32 - scale_damage(amount, kind)).trunc() as i32;
    set_health(w, id, new, attacker);
    true
}

/// New health: OnHealthChanged if it changed, break at 0.
fn set_health(w: &mut LogicWorld, id: EntId, new: i32, activator: Option<Who>) {
    let Some(p) = prop(w, id) else { return };
    if p.broken {
        return;
    }
    let changed = new != p.health;
    p.health = new;
    let ratio = if p.max_health > 0 {
        (new as f32 / p.max_health as f32).clamp(0.0, 1.0)
    } else {
        0.0
    };
    if changed {
        w.fire_output(id, "OnHealthChanged", activator, Value::Float(ratio));
    }
    if new <= 0 {
        prop_break(w, id, activator);
    }
}

/// Break a prop: OnBreak, then it is removed (no gibs yet).
pub(super) fn prop_break(w: &mut LogicWorld, id: EntId, breaker: Option<Who>) {
    let Some(p) = prop(w, id) else { return };
    if p.broken {
        return;
    }
    p.broken = true;
    p.solid = false;
    if let Some(e) = w.get_mut(id) {
        e.targetname.clear();
    }
    w.fire_output(id, "OnBreak", breaker, Value::Void);
    w.kill(id);
}

/// Prop inputs; false when not one of them.
pub(super) fn prop_input(w: &mut LogicWorld, id: EntId, input: &str, value: &Value, activator: Option<Who>) -> bool {
    let Some(p) = prop(w, id).cloned() else { return false };
    match input {
        "break" => prop_break(w, id, activator),
        "sethealth" | "addhealth" | "removehealth" => {
            let Some(n) = w.need_int(value, input) else { return true };
            let new = match input {
                "sethealth" => n,
                "addhealth" => p.health + n,
                _ => p.health - n,
            };
            set_health(w, id, new, activator);
        }
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

impl LogicWorld {
    /// Map props the logic keeps, in entity order.
    pub fn prop_states(&self) -> Vec<PropState> {
        self.ids()
            .into_iter()
            .filter_map(|id| {
                let e = self.get(id)?;
                let Class::Prop(p) = &e.class else { return None };
                let damageable = p.max_health > 0
                    || ["OnHealthChanged", "OnTakeDamage", "OnBreak"]
                        .iter()
                        .any(|o| e.has_output(o));
                Some(PropState {
                    id,
                    index: e.map_index?,
                    visible: p.visible,
                    solid: p.solid && !p.broken,
                    damageable,
                    skin: p.skin,
                    body_group: p.body_group,
                    sequence: p.sequence.clone().map(|s| (s, p.sequence_serial)),
                    default_sequence: p.default_sequence.clone(),
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

    /// A model door's state.
    pub fn prop_door(&self, id: EntId) -> Option<&PropDoor> {
        match self.get(id).map(|e| &e.class) {
            Some(Class::PropDoor(d)) => Some(d),
            _ => None,
        }
    }
}
