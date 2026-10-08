//! Entity classes: the logic entities, filters, game_text and the command
//! entities (specs/source/entity_io.md), and the dispatch to triggers and
//! movers.

use super::hud::HudMessage;
use super::movers::{self, Button, Door, MoveLinear, PathTrack, Rotating, Toggle, Train};
use super::triggers::{self, Trigger};
use super::value::{Value, atoi};
use super::world::{Effect, EntId, GlobalState, LogicWorld, Who, name_matches};

/// Per-class state.
#[derive(Clone, Debug, Default)]
pub enum Class {
    /// Point entities with no behaviour of their own (info_target,
    /// info_teleport_destination...): names, positions, base inputs.
    #[default]
    None,
    Auto,
    Relay(Relay),
    Timer(Timer),
    Counter(Counter),
    Case(Case),
    Compare { value: f32, compare: f32 },
    Branch { value: bool },
    Filter(Filter),
    GameText(HudMessage, bool),
    ServerCommand,
    ClientCommand,
    Trigger(Box<Trigger>),
    Door(Box<Door>),
    Button(Box<Button>),
    MoveLinear(Box<MoveLinear>),
    Rotating(Box<Rotating>),
    Train(Box<Train>),
    PathTrack(PathTrack),
    /// func_brush: shown and solid while enabled.
    Brush(Box<Toggle>),
    /// A brush entity parented to a mover: follows it.
    Attached(Box<movers::Attached>),
    /// func_breakable, func_breakable_surf.
    Breakable(Box<super::breakables::Breakable>),
    /// ambient_generic.
    Ambient(Box<super::ambient::Ambient>),
    /// prop_door_rotating: a model door, turned like a rotating door.
    PropDoor(Box<super::props::PropDoor>),
    /// prop_dynamic, prop_physics*: damage, outputs, visibility, skin,
    /// body group, animation.
    Prop(Box<super::props::Prop>),
    /// env_sprite, func_dustmotes, func_dustcloud: drawn while on.
    Part(super::visuals::Part),
    /// light, light_spot: a switchable light style.
    Light(super::visuals::Light),
    /// env_global: names a global state.
    Global(String),
    /// env_fire.
    Fire(Box<super::fire::Fire>),
    /// env_firesource.
    FireSource(super::fire::FireSource),
    /// env_firesensor.
    FireSensor(super::fire::FireSensor),
    /// An entity flame (made at run time by ignition).
    Flame(Box<super::fire::Flame>),
    /// func_areaportal, func_areaportalwindow.
    AreaPortal(super::visuals::AreaPortal),
    /// func_occluder.
    Occluder(super::visuals::Occluder),
    /// env_tonemap_controller: the HDR camera's exposure and bloom.
    Tonemap,
    /// game_player_equip (what it gives, by Use) and player_weaponstrip
    /// (gives nothing, strips).
    Equip(Equip),
}

/// game_player_equip: its items (keyvalue name, count) and whether it
/// strips the player's weapons first (spawnflag 2); player_weaponstrip:
/// no items, strips. (Spawning players with a non-"Use Only" equip is
/// the weapon layer's: `weapon::equip`.)
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Equip {
    pub items: Vec<(String, u32)>,
    pub strip: bool,
}

/// game_player_equip's "Use Only" spawnflag.
pub const SF_EQUIP_USE_ONLY: u32 = 1;
/// game_player_equip's "Strip all weapons first" spawnflag.
pub const SF_EQUIP_STRIP: u32 = 2;

/// A game_player_equip's items: keyvalues naming weapons or items
/// (`weapon_*`, `item_*`, `ammo_*`) with a count (at least 1).
pub fn equip_items(keyvalues: &[(String, String)]) -> Vec<(String, u32)> {
    keyvalues
        .iter()
        .filter(|(k, _)| {
            let k = k.to_ascii_lowercase();
            k.starts_with("weapon_") || k.starts_with("item_") || k.starts_with("ammo_")
        })
        .map(|(k, v)| (k.to_ascii_lowercase(), atoi(v).max(1) as u32))
        .collect()
}

#[derive(Clone, Debug, Default)]
pub struct Relay {
    pub enabled: bool,
    pub locked: bool,
}

#[derive(Clone, Debug, Default)]
pub struct Timer {
    pub enabled: bool,
    pub refire: f32,
    pub random: bool,
    pub lower: f32,
    pub upper: f32,
    pub high: bool,
}

#[derive(Clone, Debug, Default)]
pub struct Counter {
    pub value: f32,
    pub min: f32,
    pub max: f32,
    pub hit_min: bool,
    pub hit_max: bool,
    pub enabled: bool,
}

#[derive(Clone, Debug, Default)]
pub struct Case {
    pub cases: Vec<String>,
    pub deck: Vec<u8>,
    pub last: Option<u8>,
}

#[derive(Clone, Debug)]
pub enum FilterKind {
    Name(String),
    Class(String),
    Team(i32),
    Mass,
    DamageType,
    Multi { or: bool, names: Vec<String>, resolved: Vec<EntId> },
}

#[derive(Clone, Debug)]
pub struct Filter {
    pub kind: FilterKind,
    pub negated: bool,
}

/// What a map's point_servercommand may do (entity_io.md,
/// "point_servercommand", security; the rule is ours, the spec leaves the
/// list to us): `say <text>`, or set one game-rule setting, `<name>
/// <value>`, where the name starts with one of these prefixes (movement
/// and physics `sv_*`, round rules `mp_*`, `phys_*`, `bot_*`, `ammo_*`;
/// the cvars minigame maps set) and isn't one of `SETTING_DENY`. The
/// bridge then applies it only when our console has a cvar of that name
/// (`mp_restartgame` too, which is a cvar in Source and a command here);
/// unknown names (plugin cvars, settings mashup lacks) are logged and
/// ignored. Everything else (other commands: quit, exec, bind, connect,
/// rcon, changelevel, map, kick, writing configs, plugin commands such
/// as `sm_say`; several commands on one line) is refused.
pub const SETTING_PREFIXES: &[&str] = &["sv_", "mp_", "phys_", "bot_", "ammo_"];

/// Parts of names a map may never set although their prefix fits:
/// passwords, remote control, downloads and uploads, logging, bans,
/// file checks, how the server shows itself to the network, and the
/// gate on point_servercommand itself.
pub const SETTING_DENY: &[&str] = &[
    "password",
    "rcon",
    "download",
    "upload",
    "log",
    "ban",
    "pure",
    "consistency",
    "sv_lan",
    "sv_region",
    "sv_tags",
    "sv_contact",
    "sv_visiblemaxplayers",
    "servercommand",
];

/// Bounds a map's value is clamped to for settings where an extreme value
/// would make the game unplayable or hang it.
pub const SETTING_BOUNDS: &[(&str, f32, f32)] = &[
    ("sv_gravity", -2000.0, 4000.0),
    ("sv_airaccelerate", -100.0, 1000.0),
    ("sv_accelerate", 0.0, 100.0),
    ("sv_friction", 0.0, 100.0),
    ("sv_maxspeed", 0.0, 2000.0),
    ("sv_maxvelocity", 0.0, 100_000.0),
    ("sv_alltalk", 0.0, 1.0),
    ("mp_restartgame", 0.0, 60.0),
    ("mp_roundtime", 0.0, 60.0),
    ("mp_friendlyfire", 0.0, 1.0),
    ("mp_freezetime", 0.0, 60.0),
    ("mp_timelimit", 0.0, 1000.0),
    ("phys_pushscale", 0.0, 100.0),
    ("phys_timescale", 0.0, 10.0),
];

/// Longest value a map may set.
const MAX_VALUE: usize = 64;

/// A point_servercommand line that passed `check_server_command`.
#[derive(Clone, Debug, PartialEq)]
pub enum ServerLine {
    /// `say <text>`: print it.
    Say(String),
    /// Set a game-rule setting (name in lower case; value clamped).
    Set { name: String, value: String },
}

impl ServerLine {
    /// The line as the console would run it.
    pub fn line(&self) -> String {
        match self {
            ServerLine::Say(text) => format!("say {text}"),
            ServerLine::Set { name, value } => format!("{name} {value}"),
        }
    }
}

/// Commands point_clientcommand may send to a player's client.
pub const CLIENT_COMMANDS: &[&str] = &["play", "playgamesound", "r_screenoverlay", "echo"];

/// Check a server command line against the rule (`SETTING_PREFIXES`):
/// what it may do, or why it is refused.
pub fn check_server_command(line: &str) -> Result<ServerLine, &'static str> {
    let line = line.trim();
    if line.contains([';', '\n', '\r']) {
        return Err("more than one command");
    }
    let (name, rest) = line.split_once(char::is_whitespace).unwrap_or((line, ""));
    let name = name.to_ascii_lowercase();
    let rest = rest.trim();
    if name == "say" {
        return Ok(ServerLine::Say(rest.trim_matches('"').to_string()));
    }
    if !SETTING_PREFIXES.iter().any(|p| name.starts_with(p)) {
        return Err("not a game setting");
    }
    if SETTING_DENY.iter().any(|d| name.contains(d)) {
        return Err("a setting maps may not change");
    }
    let value = match rest.strip_prefix('"') {
        Some(quoted) => quoted.strip_suffix('"').unwrap_or(quoted),
        None => rest,
    };
    if value.is_empty() {
        return Err("no value");
    }
    if value.contains('"') || value.len() > MAX_VALUE {
        return Err("bad value");
    }
    let value = match SETTING_BOUNDS.iter().find(|(n, ..)| *n == name) {
        Some(&(_, lo, hi)) => {
            let v: f32 = value.parse().map_err(|_| "bad value")?;
            super::value::fmt_g(v.clamp(lo, hi))
        }
        None => value.to_string(),
    };
    Ok(ServerLine::Set { name, value })
}

/// Check a client command against the allowlist.
pub fn allowed_client_command(line: &str) -> Option<String> {
    let line = line.trim();
    if line.contains(';') || line.contains('\n') {
        return None;
    }
    let name = line.split_whitespace().next()?.to_ascii_lowercase();
    CLIENT_COMMANDS.contains(&name.as_str()).then(|| line.to_string())
}

fn color(s: Option<&str>) -> [u8; 3] {
    let v: Vec<i32> = s.unwrap_or("").split_whitespace().map(atoi).collect();
    let c = |i: usize| v.get(i).copied().unwrap_or(255).clamp(0, 255) as u8;
    [c(0), c(1), c(2)]
}

fn game_text(e: &super::world::LogicEntity) -> HudMessage {
    let f = |k: &str, d: f32| e.kv(k).map_or(d, super::value::atof);
    HudMessage {
        text: e.kv("message").unwrap_or("").to_string(),
        x: f("x", -1.0),
        y: f("y", -1.0),
        channel: e.kv_i("channel").max(0) as usize,
        effect: e.kv_i("effect").clamp(0, 2) as u8,
        color: color(e.kv("color")),
        color2: color(e.kv("color2")),
        fade_in: f("fadein", 1.5),
        fade_out: f("fadeout", 0.5),
        hold: f("holdtime", 1.2),
        fx_time: f("fxtime", 0.25),
    }
}

impl Class {
    /// The class state for a freshly spawned entity.
    pub(super) fn spawn(w: &mut LogicWorld, id: EntId) -> Class {
        let e = w.get(id).unwrap();
        let start_disabled = e.kv_i("StartDisabled") != 0;
        match e.classname.to_ascii_lowercase().as_str() {
            "logic_auto" => Class::Auto,
            "logic_relay" => Class::Relay(Relay {
                enabled: !start_disabled,
                locked: false,
            }),
            "logic_timer" => {
                let random = e.kv_i("UseRandomTime") != 0;
                let mut refire = e.kv_f("RefireTime");
                if !random && refire < 0.01 {
                    refire = 0.01;
                }
                Class::Timer(Timer {
                    enabled: !start_disabled,
                    refire,
                    random,
                    lower: e.kv_f("LowerRandomBound"),
                    upper: e.kv_f("UpperRandomBound"),
                    high: false,
                })
            }
            "math_counter" => {
                let (mut min, mut max) = (e.kv_f("min"), e.kv_f("max"));
                if min > max {
                    std::mem::swap(&mut min, &mut max);
                }
                let mut value = e.kv("startvalue").map_or(0, atoi) as f32;
                if min != 0.0 || max != 0.0 {
                    value = value.clamp(min, max);
                }
                Class::Counter(Counter {
                    value,
                    min,
                    max,
                    hit_min: false,
                    hit_max: false,
                    enabled: !start_disabled,
                })
            }
            "logic_case" => Class::Case(Case {
                cases: (1..=16)
                    .map(|i| e.kv(&format!("Case{i:02}")).unwrap_or("").to_string())
                    .collect(),
                deck: Vec::new(),
                last: None,
            }),
            "logic_compare" => Class::Compare {
                value: e.kv_f("InitialValue"),
                compare: e.kv_f("CompareValue"),
            },
            "logic_branch" => Class::Branch {
                value: e.kv_i("InitialValue") != 0,
            },
            "filter_activator_name" => filter(e, FilterKind::Name(e.kv("filtername").unwrap_or("").into())),
            "filter_activator_class" => filter(e, FilterKind::Class(e.kv("filterclass").unwrap_or("").into())),
            "filter_activator_team" => filter(e, FilterKind::Team(e.kv_i("filterteam"))),
            "filter_activator_mass_greater" => filter(e, FilterKind::Mass),
            "filter_damage_type" => filter(e, FilterKind::DamageType),
            "filter_multi" => filter(
                e,
                FilterKind::Multi {
                    or: e.kv_i("FilterType") == 1,
                    names: (1..=5)
                        .filter_map(|i| e.kv(&format!("Filter{i:02}")).filter(|n| !n.is_empty()))
                        .map(String::from)
                        .collect(),
                    resolved: Vec::new(),
                },
            ),
            "game_text" => Class::GameText(game_text(e), e.has_flag(1)),
            "game_player_equip" => Class::Equip(Equip {
                items: equip_items(&e.keyvalues),
                strip: e.has_flag(SF_EQUIP_STRIP),
            }),
            "player_weaponstrip" => Class::Equip(Equip {
                items: Vec::new(),
                strip: true,
            }),
            "point_servercommand" => Class::ServerCommand,
            "point_clientcommand" => Class::ClientCommand,
            c if c.starts_with("trigger_") => triggers::spawn(w, id).map_or(Class::None, |t| Class::Trigger(Box::new(t))),
            "func_door" | "func_door_rotating" => Class::Door(Box::new(Door::spawn(w, id))),
            "func_button" => Class::Button(Box::new(Button::spawn(w, id))),
            "func_movelinear" => Class::MoveLinear(Box::new(MoveLinear::spawn(w, id))),
            "func_rotating" => Class::Rotating(Box::new(Rotating::spawn(w, id))),
            "func_tracktrain" => Class::Train(Box::new(Train::spawn(w, id))),
            "path_track" => Class::PathTrack(PathTrack::spawn(w, id)),
            "func_brush" => Class::Brush(Box::new(Toggle::spawn_brush(w, id))),
            "ambient_generic" => super::ambient::spawn(w, id).map_or(Class::None, |a| Class::Ambient(Box::new(a))),
            "func_breakable" | "func_breakable_surf" => {
                Class::Breakable(Box::new(super::breakables::Breakable::spawn(w, id)))
            }
            "env_sprite" | "env_glow" => {
                Class::Part(super::visuals::spawn_part(w, id, super::visuals::PartKind::Sprite))
            }
            "func_dustmotes" | "func_dustcloud" => {
                Class::Part(super::visuals::spawn_part(w, id, super::visuals::PartKind::Dust))
            }
            "func_areaportal" | "func_areaportalwindow" => {
                super::visuals::spawn_area_portal(w, id).map_or(Class::None, Class::AreaPortal)
            }
            "func_occluder" => super::visuals::spawn_occluder(w, id).map_or(Class::None, Class::Occluder),
            "light" | "light_spot" => Class::Light(super::visuals::spawn_light(w, id)),
            "env_tonemap_controller" => Class::Tonemap,
            "env_global" => Class::Global(global_spawn(w, id)),
            "env_fire" => Class::Fire(Box::new(super::fire::spawn_fire(w, id))),
            "env_firesource" => Class::FireSource(super::fire::spawn_source(w, id)),
            "env_firesensor" => Class::FireSensor(super::fire::spawn_sensor(w, id)),
            "prop_door_rotating" => Class::PropDoor(Box::new(super::props::PropDoor::spawn(w, id))),
            c if super::props::is_prop_class(c) => Class::Prop(Box::new(super::props::Prop::spawn(w, id))),
            _ if !e.hulls.is_empty() && e.kv("parentname").is_some_and(|p| !p.is_empty()) => {
                Class::Attached(Box::new(movers::Attached::spawn(w, id)))
            }
            _ => Class::None,
        }
    }
}

fn filter(e: &super::world::LogicEntity, kind: FilterKind) -> Class {
    Class::Filter(Filter {
        kind,
        negated: e.kv_i("Negated") != 0,
    })
}

/// Whether a filter entity passes `who` (None fails).
pub fn filter_passes(w: &LogicWorld, filter: EntId, who: Option<Who>) -> bool {
    let Some(Class::Filter(f)) = w.get(filter).map(|e| &e.class) else {
        return true;
    };
    let Some(who) = who.filter(|x| w.exists(*x)) else {
        return false;
    };
    let pass = match &f.kind {
        FilterKind::Name(pattern) => {
            if pattern.eq_ignore_ascii_case("!player") {
                matches!(who, Who::Player(_))
            } else {
                name_matches(pattern, &w.name_of(who))
            }
        }
        FilterKind::Class(pattern) => name_matches(pattern, &w.class_of(who)),
        FilterKind::Team(team) => match who {
            Who::Player(p) => w.player(p).is_some_and(|p| p.team as i32 == *team),
            Who::Ent(_) => false,
        },
        FilterKind::Mass | FilterKind::DamageType => false,
        FilterKind::Multi { or, resolved, .. } => {
            if *or {
                resolved.iter().any(|f| filter_passes(w, *f, Some(who)))
            } else {
                resolved.iter().all(|f| filter_passes(w, *f, Some(who)))
            }
        }
    };
    pass != f.negated
}

/// Activation (after every map entity spawned).
pub(super) fn class_activate(w: &mut LogicWorld, id: EntId) {
    let Some(e) = w.get(id) else { return };
    match &e.class {
        Class::Auto => w.think_in(id, 0.2),
        Class::Relay(_) => {
            if e.has_output("OnSpawn") {
                w.think_in(id, 0.01);
            }
        }
        Class::Timer(t) => {
            if t.enabled {
                timer_reset(w, id);
            }
        }
        Class::Filter(f) => {
            if let FilterKind::Multi { names, .. } = &f.kind {
                let names = names.clone();
                let resolved: Vec<EntId> = names
                    .iter()
                    .filter_map(|n| {
                        let found = w.find(n);
                        let ok = found.is_some_and(|f| matches!(w.get(f).map(|e| &e.class), Some(Class::Filter(_))));
                        if !ok {
                            w.log.push(format!("filter_multi: '{n}' is not a filter"));
                        }
                        found.filter(|_| ok)
                    })
                    .collect();
                if let Some(Class::Filter(Filter {
                    kind: FilterKind::Multi { resolved: r, .. },
                    ..
                })) = w.get_mut(id).map(|e| &mut e.class)
                {
                    *r = resolved;
                }
            }
        }
        Class::Trigger(_) => triggers::activate(w, id),
        Class::Door(_)
        | Class::Button(_)
        | Class::MoveLinear(_)
        | Class::Rotating(_)
        | Class::Train(_)
        | Class::PropDoor(_) => movers::activate(w, id),
        Class::Breakable(_) if e.kv("parentname").is_some_and(|p| !p.is_empty()) => movers::activate_attached(w, id),
        Class::Breakable(_) => movers::activate(w, id),
        Class::PathTrack(_) => movers::activate_path(w, id),
        Class::Attached(_) => movers::activate_attached(w, id),
        Class::Ambient(_) => super::ambient::activate(w, id),
        Class::Prop(_) => super::prop_damage::activate(w, id),
        Class::Fire(_) | Class::FireSource(_) | Class::FireSensor(_) => super::fire::activate(w, id),
        _ => {}
    }
}

pub(super) fn class_think(w: &mut LogicWorld, id: EntId) {
    let Some(e) = w.get(id) else { return };
    match &e.class {
        Class::Auto => {
            // Only while its global state is on (an unset one is off).
            if let Some(g) = e.kv("globalstate").filter(|g| !g.trim().is_empty())
                && w.global(g) != Some(GlobalState::On)
            {
                return;
            }
            let remove = e.has_flag(1);
            w.fire_output(id, "OnNewGame", None, Value::Void);
            w.fire_output(id, "OnMapSpawn", None, Value::Void);
            // A round restart re-creates it (world.rs `round_restart`).
            let output = if w.round > 0 { "OnMultiNewRound" } else { "OnMultiNewMap" };
            w.fire_output(id, output, None, Value::Void);
            if remove {
                w.kill(id);
            }
        }
        Class::Relay(_) => {
            let remove = e.has_flag(1);
            w.fire_output(id, "OnSpawn", Some(Who::Ent(id)), Value::Void);
            if remove {
                w.kill(id);
            }
        }
        Class::Timer(t) => {
            if t.enabled {
                timer_fire(w, id);
                timer_reset(w, id);
            }
        }
        Class::Trigger(_) => triggers::think(w, id),
        Class::Ambient(_) => super::ambient::think(w, id),
        Class::Prop(_) => super::prop_damage::think(w, id),
        Class::Fire(_) | Class::FireSource(_) | Class::FireSensor(_) | Class::Flame(_) => super::fire::think(w, id),
        _ => movers::think(w, id),
    }
}

fn timer_reset(w: &mut LogicWorld, id: EntId) {
    let Some(Class::Timer(t)) = w.get(id).map(|e| e.class.clone()) else { return };
    let refire = if t.random {
        let r = t.lower + w.random() * (t.upper - t.lower);
        if let Some(Class::Timer(t)) = w.get_mut(id).map(|e| &mut e.class) {
            t.refire = r;
        }
        r
    } else {
        t.refire
    };
    w.think_in(id, refire);
}

fn timer_fire(w: &mut LogicWorld, id: EntId) {
    let osc = w.get(id).is_some_and(|e| e.has_flag(1));
    let output = if osc {
        let Some(Class::Timer(t)) = w.get_mut(id).map(|e| &mut e.class) else { return };
        let out = if t.high { "OnTimerHigh" } else { "OnTimerLow" };
        t.high = !t.high;
        out
    } else {
        "OnTimer"
    };
    w.fire_output(id, output, Some(Who::Ent(id)), Value::Void);
}

fn counter_update(w: &mut LogicWorld, id: EntId, new: f32, activator: Option<Who>) {
    let Some(Class::Counter(c)) = w.get_mut(id).map(|e| &mut e.class) else { return };
    let mut new = new;
    let mut outs = Vec::new();
    if c.min != 0.0 || c.max != 0.0 {
        if new >= c.max {
            if !c.hit_max {
                c.hit_max = true;
                outs.push("OnHitMax");
            }
        } else {
            c.hit_max = false;
        }
        if new <= c.min {
            if !c.hit_min {
                c.hit_min = true;
                outs.push("OnHitMin");
            }
        } else {
            c.hit_min = false;
        }
        new = new.clamp(c.min, c.max);
    }
    c.value = new;
    for o in outs {
        w.fire_output(id, o, activator, Value::Void);
    }
    w.fire_output(id, "OutValue", activator, Value::Float(new));
}

fn case_fire(w: &mut LogicWorld, id: EntId, case: u8, activator: Option<Who>) {
    w.fire_output(id, &format!("OnCase{case:02}"), activator, Value::Void);
}

fn case_connected(w: &LogicWorld, id: EntId) -> Vec<u8> {
    let Some(e) = w.get(id) else { return Vec::new() };
    (1..=16u8).filter(|n| e.has_output(&format!("OnCase{n:02}"))).collect()
}

/// Class inputs; false when the class doesn't handle it (base inputs
/// then).
pub(super) fn class_input(
    w: &mut LogicWorld,
    id: EntId,
    input: &str,
    value: &Value,
    activator: Option<Who>,
    caller: Option<Who>,
) -> bool {
    let Some(class) = w.get(id).map(|e| e.class.clone()) else {
        return true;
    };
    let flags = w.get(id).map_or(0, |e| e.spawnflags);
    match class {
        Class::Relay(r) => match input {
            "trigger" => {
                if r.enabled && !r.locked {
                    w.fire_output(id, "OnTrigger", activator, Value::Void);
                    if flags & 1 != 0 {
                        // Gone at the end of the frame; no second trigger
                        // before that.
                        set_relay(w, id, |r| r.enabled = false);
                        w.kill(id);
                    } else if flags & 2 == 0 {
                        let longest = w
                            .get(id)
                            .and_then(|e| e.outputs.iter().find(|(n, _)| n == "ontrigger"))
                            .map_or(0.0, |(_, c)| c.iter().map(|c| c.delay).fold(0.0, f32::max));
                        set_relay(w, id, |r| r.locked = true);
                        w.queue_direct(id, "EnableRefire", Value::Void, longest + 0.001, Some(Who::Ent(id)));
                    }
                }
            }
            "enable" => set_relay(w, id, |r| r.enabled = true),
            "disable" => set_relay(w, id, |r| r.enabled = false),
            "toggle" => set_relay(w, id, |r| r.enabled = !r.enabled),
            "cancelpending" => {
                w.cancel_pending(id);
                set_relay(w, id, |r| r.locked = false);
            }
            "enablerefire" => set_relay(w, id, |r| r.locked = false),
            _ => return false,
        },
        Class::Timer(t) => {
            let set = |w: &mut LogicWorld, f: &dyn Fn(&mut Timer)| {
                if let Some(Class::Timer(t)) = w.get_mut(id).map(|e| &mut e.class) {
                    f(t);
                }
            };
            match input {
                "enable" => {
                    set(w, &|t| t.enabled = true);
                    timer_reset(w, id);
                }
                "disable" => {
                    set(w, &|t| t.enabled = false);
                    w.get_mut(id).unwrap().next_think = None;
                }
                "toggle" => {
                    let on = !t.enabled;
                    set(w, &|t| t.enabled = on);
                    if on {
                        timer_reset(w, id);
                    } else {
                        w.get_mut(id).unwrap().next_think = None;
                    }
                }
                "firetimer" => {
                    if t.enabled {
                        timer_fire(w, id);
                        timer_reset(w, id);
                    }
                }
                "refiretime" => {
                    let Some(v) = w.need_float(value, input) else { return true };
                    let v = v.max(0.01);
                    if v != t.refire {
                        set(w, &|t| t.refire = v);
                        if t.enabled {
                            timer_reset(w, id);
                        }
                    }
                }
                "resettimer" => {
                    if t.enabled {
                        timer_reset(w, id);
                    }
                }
                "addtotimer" => {
                    let Some(v) = w.need_float(value, input) else { return true };
                    if t.enabled {
                        let add = w.think_ticks(v);
                        let e = w.get_mut(id).unwrap();
                        e.next_think = e.next_think.map(|n| n + add);
                    }
                }
                "subtractfromtimer" => {
                    let Some(v) = w.need_float(value, input) else { return true };
                    if t.enabled {
                        let tick = w.tick;
                        let dt = w.dt;
                        let sub = w.think_ticks(v);
                        let e = w.get_mut(id).unwrap();
                        if let Some(next) = e.next_think {
                            e.next_think = Some(if (next - tick) as f32 * dt <= v {
                                tick
                            } else {
                                next - sub
                            });
                        }
                    }
                }
                "userandomtime" => {
                    let Some(v) = w.need_bool(value, input) else { return true };
                    set(w, &|t| t.random = v);
                }
                "lowerrandombound" => {
                    let Some(v) = w.need_float(value, input) else { return true };
                    set(w, &|t| t.lower = v);
                }
                "upperrandombound" => {
                    let Some(v) = w.need_float(value, input) else { return true };
                    set(w, &|t| t.upper = v);
                }
                _ => return false,
            }
        }
        Class::Counter(c) => {
            let arith = ["add", "subtract", "multiply", "setvalue", "divide", "setvaluenofire"];
            if arith.contains(&input) {
                let Some(v) = w.need_float(value, input) else { return true };
                if !c.enabled {
                    return true;
                }
                match input {
                    "add" => counter_update(w, id, c.value + v, activator),
                    "subtract" => counter_update(w, id, c.value - v, activator),
                    "multiply" => counter_update(w, id, c.value * v, activator),
                    "setvalue" => counter_update(w, id, v, activator),
                    "divide" => counter_update(w, id, if v != 0.0 { c.value / v } else { c.value }, activator),
                    _ => {
                        if let Some(Class::Counter(c)) = w.get_mut(id).map(|e| &mut e.class) {
                            c.value = if c.min != 0.0 || c.max != 0.0 {
                                v.clamp(c.min, c.max)
                            } else {
                                v
                            };
                        }
                    }
                }
                return true;
            }
            match input {
                "sethitmax" | "sethitmin" => {
                    let Some(v) = w.need_float(value, input) else { return true };
                    let mut c2 = c.clone();
                    if input == "sethitmax" {
                        c2.max = v;
                        if c2.max < c2.min {
                            c2.min = c2.max;
                        }
                    } else {
                        c2.min = v;
                        if c2.max < c2.min {
                            c2.max = c2.min;
                        }
                    }
                    let value = c2.value;
                    w.get_mut(id).unwrap().class = Class::Counter(c2);
                    counter_update(w, id, value, activator);
                }
                "getvalue" => w.fire_output(id, "OnGetValue", activator, Value::Float(c.value)),
                "enable" | "disable" => {
                    if let Some(Class::Counter(c)) = w.get_mut(id).map(|e| &mut e.class) {
                        c.enabled = input == "enable";
                    }
                }
                _ => return false,
            }
        }
        Class::Case(case) => match input {
            "invalue" => {
                let text = value.as_text(|x| w.name_of(x));
                match case
                    .cases
                    .iter()
                    .position(|c| !c.is_empty() && c.eq_ignore_ascii_case(&text))
                {
                    Some(i) => case_fire(w, id, i as u8 + 1, activator),
                    None => w.fire_output(id, "OnDefault", activator, value.clone()),
                }
            }
            "pickrandom" => {
                let list = case_connected(w, id);
                if !list.is_empty() {
                    let pick = list[w.random_int(list.len())];
                    case_fire(w, id, pick, activator);
                }
            }
            "pickrandomshuffle" => {
                let mut c = case.clone();
                let mut drawable = c.deck.len();
                if c.deck.is_empty() {
                    c.deck = case_connected(w, id);
                    let n = c.deck.len();
                    drawable = n;
                    if n > 1
                        && let Some(last) = c.last
                        && let Some(pos) = c.deck.iter().position(|x| *x == last)
                    {
                        c.deck.remove(pos);
                        c.deck.push(last);
                        drawable = n - 1;
                    }
                }
                if drawable == 0 {
                    return true;
                }
                let r = w.random_int(drawable);
                let pick = c.deck[r];
                let last = *c.deck.last().unwrap();
                c.deck[r] = last;
                c.deck.pop();
                c.last = Some(pick);
                w.get_mut(id).unwrap().class = Class::Case(c);
                case_fire(w, id, pick, activator);
            }
            _ => return false,
        },
        Class::Compare { value: v, compare } => {
            let set = |w: &mut LogicWorld, v: Option<f32>, c: Option<f32>| {
                if let Some(Class::Compare { value, compare }) = w.get_mut(id).map(|e| &mut e.class) {
                    if let Some(v) = v {
                        *value = v;
                    }
                    if let Some(c) = c {
                        *compare = c;
                    }
                }
            };
            let compare_now = |w: &mut LogicWorld, v: f32, c: f32| {
                if v == c {
                    w.fire_output(id, "OnEqualTo", activator, Value::Float(v));
                } else {
                    w.fire_output(id, "OnNotEqualTo", activator, Value::Float(v));
                    let o = if v > c { "OnGreaterThan" } else { "OnLessThan" };
                    w.fire_output(id, o, activator, Value::Float(v));
                }
            };
            match input {
                "setvalue" | "setvaluecompare" | "setcomparevalue" => {
                    let Some(x) = w.need_float(value, input) else { return true };
                    if input == "setcomparevalue" {
                        set(w, None, Some(x));
                    } else {
                        set(w, Some(x), None);
                    }
                    if input == "setvaluecompare" {
                        compare_now(w, x, compare);
                    }
                }
                "compare" => compare_now(w, v, compare),
                _ => return false,
            }
        }
        Class::Branch { value: v } => {
            let set = |w: &mut LogicWorld, b: bool| {
                if let Some(Class::Branch { value }) = w.get_mut(id).map(|e| &mut e.class) {
                    *value = b;
                }
            };
            let test = |w: &mut LogicWorld, b: bool| {
                w.fire_output(id, if b { "OnTrue" } else { "OnFalse" }, activator, Value::Void);
            };
            match input {
                "setvalue" | "setvaluetest" => {
                    let Some(b) = w.need_bool(value, input) else { return true };
                    set(w, b);
                    if input == "setvaluetest" {
                        test(w, b);
                    }
                }
                "toggle" => set(w, !v),
                "toggletest" => {
                    set(w, !v);
                    test(w, !v);
                }
                "test" => test(w, v),
                _ => return false,
            }
        }
        Class::Filter(_) => match input {
            "testactivator" => {
                let pass = filter_passes(w, id, activator);
                w.fire_output(id, if pass { "OnPass" } else { "OnFail" }, activator, Value::Void);
            }
            _ => return false,
        },
        Class::GameText(..) => match input {
            "display" => game_text_display(w, id, activator),
            _ => return false,
        },
        Class::ServerCommand => match input {
            "command" => {
                let Some(line) = w.need_str(value, input) else { return true };
                if line.is_empty() {
                    return true;
                }
                match check_server_command(&line) {
                    Ok(ok) => w.effects.push(Effect::ServerCommand(ok)),
                    Err(why) => w.log.push(format!("point_servercommand: refused '{line}' ({why})")),
                }
            }
            _ => return false,
        },
        Class::ClientCommand => match input {
            "command" => {
                let Some(line) = w.need_str(value, input) else { return true };
                let Some(Who::Player(p)) = activator else { return true };
                match allowed_client_command(&line) {
                    Some(ok) => w.effects.push(Effect::ClientCommand {
                        player: p,
                        command: ok,
                    }),
                    None => w.log.push(format!("point_clientcommand: refused '{line}' (not allowed)")),
                }
            }
            _ => return false,
        },
        Class::Trigger(_) => return triggers::input(w, id, input, value, activator, caller),
        Class::Door(_)
        | Class::Button(_)
        | Class::MoveLinear(_)
        | Class::Rotating(_)
        | Class::Train(_)
        | Class::PathTrack(_)
        | Class::Brush(_)
        | Class::PropDoor(_) => return movers::input(w, id, input, value, activator, caller),
        Class::Prop(_) => return super::props::prop_input(w, id, input, value, activator),
        Class::Attached(_) => return false,
        Class::Breakable(_) => return super::breakables::input(w, id, input, value, activator),
        Class::Ambient(_) => return super::ambient::input(w, id, input, value),
        Class::Part(_) | Class::Light(_) => return super::visuals::input(w, id, input, value),
        Class::Global(name) => return global_input(w, id, &name, input, value, activator),
        Class::Fire(_) | Class::FireSource(_) | Class::FireSensor(_) => return super::fire::input(w, id, input, value),
        Class::AreaPortal(_) => return super::visuals::area_portal_input(w, id, input),
        Class::Occluder(_) => return super::visuals::occluder_input(w, id, input),
        Class::Tonemap => return super::visuals::tonemap_input(w, input, value),
        Class::Equip(equip) => match input {
            // player_weaponstrip: Strip (the activator), StripWeaponsAndSuit.
            "strip" | "stripweaponsandsuit" => equip_player(w, &equip, activator),
            _ => return false,
        },
        Class::Flame(_) | Class::None | Class::Auto => return false,
    }
    true
}

/// Give (or strip) the activator, when it is a player.
fn equip_player(w: &mut LogicWorld, equip: &Equip, activator: Option<Who>) {
    if let Some(Who::Player(p)) = activator
        && w.player(p).is_some()
    {
        w.effects.push(Effect::Equip {
            target: p,
            items: equip.items.clone(),
            strip: equip.strip,
        });
    }
}

fn set_relay(w: &mut LogicWorld, id: EntId, f: impl FnOnce(&mut Relay)) {
    if let Some(Class::Relay(r)) = w.get_mut(id).map(|e| &mut e.class) {
        f(r);
    }
}

fn game_text_display(w: &mut LogicWorld, id: EntId, activator: Option<Who>) {
    let Some(Class::GameText(message, all)) = w.get(id).map(|e| e.class.clone()) else {
        return;
    };
    let to = if all {
        None
    } else {
        match activator {
            Some(Who::Player(p)) if w.player(p).is_some() => Some(p),
            // No player activator: nobody sees it (keep).
            _ => return,
        }
    };
    w.effects.push(Effect::GameText { to, message });
}

/// The Use input / a player's +use.
pub(super) fn class_use(w: &mut LogicWorld, id: EntId, activator: Option<Who>, caller: Option<Who>) {
    match w.get(id).map(|e| &e.class) {
        Some(Class::GameText(..)) => game_text_display(w, id, activator),
        Some(Class::Equip(equip)) => {
            let equip = equip.clone();
            equip_player(w, &equip, activator)
        }
        Some(_) => movers::use_entity(w, id, activator, caller),
        None => {}
    }
}

/// A keyvalue changed at run time (AddOutput).
pub(super) fn class_keyvalue(w: &mut LogicWorld, id: EntId, key: &str, _value: &str) {
    let Some(e) = w.get(id) else { return };
    match &e.class {
        Class::GameText(_, all) => {
            let all = *all;
            let m = game_text(e);
            w.get_mut(id).unwrap().class = Class::GameText(m, all);
        }
        Class::Trigger(_) => triggers::keyvalue(w, id, key),
        _ => movers::keyvalue(w, id, key),
    }
}

/// env_global "Set initial state" spawnflag.
pub const SF_GLOBAL_SET_INITIAL: u32 = 1;

/// An env_global spawning: with "Set initial state", a global not set yet
/// takes `initialstate` (0 off, 1 on, 2 dead) and `counter`. Returns the
/// global's name (lower case).
fn global_spawn(w: &mut LogicWorld, id: EntId) -> String {
    let e = w.get(id).unwrap();
    let name = e.kv("globalstate").unwrap_or("").trim().to_ascii_lowercase();
    let state = match e.kv_i("initialstate") {
        1 => GlobalState::On,
        2 => GlobalState::Dead,
        _ => GlobalState::Off,
    };
    let counter = e.kv_i("counter");
    if !name.is_empty() && e.has_flag(SF_GLOBAL_SET_INITIAL) && w.global(&name).is_none() {
        w.globals.push((name.clone(), state, counter));
    }
    name
}

/// env_global inputs (TurnOn, TurnOff, Toggle, Remove: dead; SetCounter,
/// AddToCounter, GetCounter: fires OutCounter).
fn global_input(w: &mut LogicWorld, id: EntId, name: &str, input: &str, value: &Value, activator: Option<Who>) -> bool {
    const INPUTS: &[&str] = &[
        "turnon",
        "turnoff",
        "toggle",
        "remove",
        "setcounter",
        "addtocounter",
        "getcounter",
    ];
    if name.is_empty() {
        return INPUTS.contains(&input);
    }
    let (state, counter) = w
        .globals
        .iter()
        .find(|g| g.0 == name)
        .map_or((GlobalState::Off, 0), |g| (g.1, g.2));
    let (state, counter) = match input {
        "turnon" => (GlobalState::On, counter),
        "turnoff" => (GlobalState::Off, counter),
        "toggle" if state == GlobalState::On => (GlobalState::Off, counter),
        "toggle" => (GlobalState::On, counter),
        "remove" => (GlobalState::Dead, counter),
        "setcounter" => {
            let Some(n) = w.need_int(value, input) else { return true };
            (state, n)
        }
        "addtocounter" => {
            let Some(n) = w.need_int(value, input) else { return true };
            (state, counter + n)
        }
        "getcounter" => {
            w.fire_output(id, "OutCounter", activator, Value::Int(counter));
            return true;
        }
        _ => return false,
    };
    match w.globals.iter_mut().find(|g| g.0 == name) {
        Some(g) => {
            g.1 = state;
            g.2 = counter;
        }
        None => w.globals.push((name.to_string(), state, counter)),
    }
    true
}

impl LogicWorld {
    /// A global state (env_global), if set.
    pub fn global(&self, name: &str) -> Option<GlobalState> {
        let name = name.trim();
        self.globals
            .iter()
            .find(|g| g.0.eq_ignore_ascii_case(name))
            .map(|g| g.1)
    }

    /// A global's counter (0 when unset).
    pub fn global_counter(&self, name: &str) -> i32 {
        let name = name.trim();
        self.globals
            .iter()
            .find(|g| g.0.eq_ignore_ascii_case(name))
            .map_or(0, |g| g.2)
    }
}
