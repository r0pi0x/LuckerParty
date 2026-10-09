//! Player and game entities minigame maps use (specs/source/
//! game_entities.md): player_speedmod, game_ui, env_fade, game_score,
//! env_hudhint, env_explosion and func_conveyor; func_wall_toggle is a
//! func_brush (`movers::Toggle::spawn_wall_toggle`). What they do to
//! players goes out as `Effect`s (the bridge writes `core::MapControls`,
//! HUD events, score changes).

use bevy::prelude::*;

use super::classes::Class;
use super::hud::{HudShow, ScreenFade};
use super::movers::Pusher;
use super::value::Value;
use super::world::{Effect, EntId, LogicWorld, Who};
use crate::core::buttons;

/// game_ui spawnflags (game_entities.md 2).
pub const SF_UI_FREEZE: u32 = 32;
pub const SF_UI_HIDE_WEAPON: u32 = 64;
pub const SF_UI_USE_DEACTIVATES: u32 = 128;
pub const SF_UI_JUMP_DEACTIVATES: u32 = 256;

/// A game_ui: the player driving it, the buttons it saw last, the axis
/// values it fired last (x, y, attack, attack2).
#[derive(Clone, Debug)]
pub struct GameUi {
    pub player: Option<Entity>,
    pub last: u32,
    pub force: bool,
    pub axes: [f32; 4],
    /// "FieldOfView": the cosine the player must keep facing it within
    /// (−1 or less: never checked).
    pub fov: f32,
}

/// An env_explosion: its magnitude and radius (spawn values).
#[derive(Clone, Debug)]
pub struct Explosion {
    pub magnitude: i32,
    pub radius: f32,
    /// The fireball sprite's scale (spawn rule; the effect is the game's
    /// blast, so it is only kept).
    pub sprite_scale: i32,
}

/// env_explosion's "Repeatable" and "No Damage" flags.
pub const SF_EXPLOSION_NO_DAMAGE: u32 = 1;
pub const SF_EXPLOSION_REPEATABLE: u32 = 2;
/// Seconds before a non-repeatable env_explosion removes itself.
pub const EXPLOSION_REMOVE_DELAY: f32 = 0.3;

/// A func_conveyor: a still brush whose top carries players along
/// `forward(movedir)` at `speed` units/s (game_entities.md 8).
#[derive(Clone, Debug)]
pub struct Conveyor {
    pub push: Pusher,
    pub dir: Vec3,
    pub speed: f32,
    /// Spawnflag 1: scrolls only.
    pub no_push: bool,
}

/// The class of a newly spawned entity of these, if it is one.
pub(super) fn spawn(w: &mut LogicWorld, id: EntId, class: &str) -> Option<Class> {
    let e = w.get(id)?;
    Some(match class {
        "player_speedmod" => Class::SpeedMod,
        "game_ui" => Class::GameUi(Box::new(GameUi {
            player: None,
            last: 0,
            force: false,
            axes: [0.0; 4],
            fov: e.kv("FieldOfView").map_or(-1.0, super::value::atof),
        })),
        "env_fade" => Class::Fade,
        "game_score" => Class::Score,
        "env_hudhint" => Class::HudHint,
        "env_explosion" => {
            let magnitude = e.kv_i("iMagnitude");
            let over = e.kv_i("iRadiusOverride");
            let radius = if over > 0 { over as f32 } else { (magnitude as f32 * 2.5) as i32 as f32 };
            let (lo, hi) = (
                if e.has_flag(2048) { 1.0 } else { 10.0 },
                if e.has_flag(4096) { 200.0 } else { 50.0 },
            );
            Class::Explosion(Explosion {
                magnitude,
                radius,
                sprite_scale: (((magnitude - 50) as f32 * 0.6).clamp(lo, hi)) as i32,
            })
        }
        "func_conveyor" => {
            let speed = match e.kv_f("speed") {
                s if s == 0.0 => 100.0,
                s => s,
            };
            let mut push = Pusher::at(e.origin, e.angles);
            push.solid = !e.has_flag(2);
            Class::Conveyor(Box::new(Conveyor {
                push,
                dir: super::triggers::forward(e.vector_kv("movedir")),
                speed,
                no_push: e.has_flag(1),
            }))
        }
        _ => return None,
    })
}

/// The buttons player_speedmod's spawnflags take away (4 jump, 8 duck, 16
/// use, 32 sprint (+speed), 64 attack (both), 128 zoom).
pub fn speedmod_buttons(flags: u32) -> u32 {
    let mut b = 0;
    for (flag, bits) in [
        (4, buttons::JUMP),
        (8, buttons::DUCK),
        (16, buttons::USE),
        (32, buttons::SPEED),
        (64, buttons::ATTACK | buttons::ATTACK2),
        (128, buttons::ZOOM),
    ] {
        if flags & flag != 0 {
            b |= bits;
        }
    }
    b
}

/// The player an input acts on: the activator when it is one (CS:S is
/// multiplayer: no "local player" stands in).
fn player_of(w: &LogicWorld, activator: Option<Who>) -> Option<Entity> {
    match activator {
        Some(Who::Player(p)) if w.player(p).is_some() => Some(p),
        _ => None,
    }
}

/// Inputs; false when the class doesn't handle it.
pub(super) fn input(
    w: &mut LogicWorld,
    id: EntId,
    input: &str,
    value: &Value,
    activator: Option<Who>,
    caller: Option<Who>,
) -> bool {
    let Some(e) = w.get(id) else { return true };
    let flags = e.spawnflags;
    let message = e.kv("message").unwrap_or("").to_string();
    let class = e.class.clone();
    match (&class, input) {
        (Class::SpeedMod, "modifyspeed") => {
            let Some(scale) = w.need_float(value, input) else { return true };
            match player_of(w, activator) {
                Some(target) => w.effects.push(Effect::SpeedMod {
                    target,
                    key: id.index,
                    scale,
                    flags,
                }),
                None => w.log.push("player_speedmod: no player activator (nothing in multiplayer)".into()),
            }
        }
        (Class::GameUi(_), "activate") => ui_activate(w, id, value, activator, caller),
        (Class::GameUi(_), "deactivate") => ui_deactivate(w, id),
        (Class::Fade, "fade") => fade(w, id, activator),
        (Class::Score, "applyscore") => apply_score(w, id, activator),
        (Class::HudHint, "showhudhint" | "hidehudhint") => {
            let text = if input == "showhudhint" {
                message
            } else {
                String::new()
            };
            let to = if flags & 1 != 0 {
                None
            } else {
                match player_of(w, activator) {
                    Some(p) => Some(p),
                    None => return true,
                }
            };
            w.effects.push(Effect::Hud {
                to,
                what: HudShow::Hint(text),
            });
        }
        (Class::Explosion(_), "explode") => explode(w, id, activator),
        (Class::Conveyor(_), "toggledirection") => conveyor_reverse(w, id),
        (Class::Conveyor(_), "setspeed") => {
            let Some(s) = w.need_float(value, input) else { return true };
            if let Some(Class::Conveyor(c)) = w.get_mut(id).map(|e| &mut e.class) {
                c.speed = s;
            }
        }
        _ => return false,
    }
    true
}

/// The Use input / +use on these classes; false when not one of them.
pub(super) fn use_entity(w: &mut LogicWorld, id: EntId, activator: Option<Who>) -> bool {
    match w.get(id).map(|e| &e.class) {
        Some(Class::Score) => apply_score(w, id, activator),
        Some(Class::Conveyor(_)) => conveyor_reverse(w, id),
        _ => return false,
    }
    true
}

pub(super) fn think(w: &mut LogicWorld, id: EntId) {
    match w.get(id).map(|e| &e.class) {
        Some(Class::GameUi(_)) => ui_think(w, id),
        Some(Class::Explosion(_)) => w.kill(id),
        _ => {}
    }
}

fn conveyor_reverse(w: &mut LogicWorld, id: EntId) {
    if let Some(Class::Conveyor(c)) = w.get_mut(id).map(|e| &mut e.class) {
        c.speed = -c.speed;
    }
}

fn ui(w: &mut LogicWorld, id: EntId) -> Option<&mut GameUi> {
    match w.get_mut(id).map(|e| &mut e.class) {
        Some(Class::GameUi(u)) => Some(u),
        _ => None,
    }
}

/// game_ui Activate (game_entities.md 2.1).
fn ui_activate(w: &mut LogicWorld, id: EntId, value: &Value, activator: Option<Who>, caller: Option<Who>) {
    let name = value.as_text(|x| w.name_of(x));
    let who = if name.trim().is_empty() {
        activator
    } else {
        w.resolve(name.trim(), activator, caller).into_iter().next()
    };
    let Some(Who::Player(p)) = who.filter(|w2| w.exists(*w2)) else {
        w.log.push("game_ui: Activate without a player (ignored)".into());
        return;
    };
    let Some(u) = ui(w, id) else { return };
    if u.player.is_some_and(|q| q != p) {
        // One driver at a time.
        return;
    }
    u.player = Some(p);
    u.force = true;
    let flags = w.get(id).map_or(0, |e| e.spawnflags);
    w.fire_output(id, "PlayerOn", Some(Who::Player(p)), Value::Void);
    w.effects.push(Effect::GameUi {
        target: p,
        on: true,
        freeze: flags & SF_UI_FREEZE != 0,
        hide_weapon: flags & SF_UI_HIDE_WEAPON != 0,
    });
    w.think_in(id, w.dt);
}

/// game_ui Deactivate (2.3): PlayerOff, then all four axes fire 0.
fn ui_deactivate(w: &mut LogicWorld, id: EntId) {
    let flags = w.get(id).map_or(0, |e| e.spawnflags);
    let Some(u) = ui(w, id) else { return };
    let Some(p) = u.player.take() else {
        w.log.push("game_ui: Deactivate without a player".into());
        if let Some(e) = w.get_mut(id) {
            e.next_think = None;
        }
        return;
    };
    u.axes = [0.0; 4];
    u.force = false;
    if let Some(e) = w.get_mut(id) {
        e.next_think = None;
    }
    w.effects.push(Effect::GameUi {
        target: p,
        on: false,
        freeze: flags & SF_UI_FREEZE != 0,
        hide_weapon: flags & SF_UI_HIDE_WEAPON != 0,
    });
    let me = Some(Who::Player(p));
    w.fire_output(id, "PlayerOff", me, Value::Void);
    for axis in ["XAxis", "YAxis", "AttackAxis", "Attack2Axis"] {
        w.fire_output(id, axis, me, Value::Float(0.0));
    }
}

/// A game_ui's tick while active (2.2).
fn ui_think(w: &mut LogicWorld, id: EntId) {
    let flags = w.get(id).map_or(0, |e| e.spawnflags);
    let origin = w.get(id).map_or(Vec3::ZERO, |e| e.origin);
    let Some(u) = ui(w, id).cloned() else { return };
    let Some(p) = u.player else { return };
    let Some(pl) = w.player(p).cloned() else {
        // Gone (disconnected): stop, no PlayerOff.
        if let Some(u) = ui(w, id) {
            u.player = None;
        }
        return;
    };
    let cur = pl.buttons;
    let last = if u.force { cur } else { u.last };
    if u.fov > -1.0 {
        let to = (origin - pl.centre()).normalize_or_zero();
        if pl.forward().dot(to) < u.fov {
            ui_deactivate(w, id);
            return;
        }
    }
    let pressed = cur & !last;
    if (flags & SF_UI_USE_DEACTIVATES != 0 && pressed & buttons::USE != 0)
        || (flags & SF_UI_JUMP_DEACTIVATES != 0 && pressed & buttons::JUMP != 0)
    {
        ui_deactivate(w, id);
        return;
    }
    let who = Some(Who::Player(p));
    let changed = cur ^ last;
    for (bit, name) in [
        (buttons::MOVERIGHT, "MoveRight"),
        (buttons::MOVELEFT, "MoveLeft"),
        (buttons::FORWARD, "Forward"),
        (buttons::BACK, "Back"),
        (buttons::ATTACK, "Attack"),
        (buttons::ATTACK2, "Attack2"),
    ] {
        if changed & bit != 0 {
            let out = if last & bit != 0 { "Unpressed" } else { "Pressed" };
            w.fire_output(id, &format!("{out}{name}"), who, Value::Void);
        }
    }
    let held = |b: u32| cur & b != 0;
    let x = if held(buttons::MOVERIGHT) {
        1.0
    } else if held(buttons::MOVELEFT) {
        -1.0
    } else {
        0.0
    };
    let y = if held(buttons::FORWARD) {
        1.0
    } else if held(buttons::BACK) {
        -1.0
    } else {
        0.0
    };
    let axes = [x, y, held(buttons::ATTACK) as u8 as f32, held(buttons::ATTACK2) as u8 as f32];
    for (k, name) in ["XAxis", "YAxis", "AttackAxis", "Attack2Axis"].into_iter().enumerate() {
        if u.force || axes[k] != u.axes[k] {
            w.fire_output(id, name, who, Value::Float(axes[k]));
        }
    }
    if let Some(u) = ui(w, id) {
        u.last = cur;
        u.axes = axes;
        u.force = false;
    }
    w.think_in(id, w.dt);
}

/// env_fade's Fade (game_entities.md 3).
fn fade(w: &mut LogicWorld, id: EntId, activator: Option<Who>) {
    let Some(e) = w.get(id) else { return };
    let flags = e.spawnflags;
    let c = e.kv("rendercolor").unwrap_or("0 0 0");
    let rgb: Vec<i32> = c.split_whitespace().map(super::value::atoi).collect();
    let ch = |i: usize| rgb.get(i).copied().unwrap_or(0).clamp(0, 255) as u8;
    let fade = ScreenFade {
        duration: ScreenFade::quantize(e.kv_f("duration")),
        hold: ScreenFade::quantize(e.kv_f("holdtime")),
        color: [ch(0), ch(1), ch(2)],
        alpha: e.kv_i("renderamt").clamp(0, 255) as u8,
        fade_in: flags & 1 != 0,
        modulate: flags & 2 != 0,
        stay_out: flags & 8 != 0,
        purge: flags & 4 == 0,
    };
    let to = if flags & 4 != 0 {
        match player_of(w, activator) {
            Some(p) => Some(p),
            None => {
                w.fire_output(id, "OnBeginFade", activator, Value::Void);
                return;
            }
        }
    } else {
        None
    };
    w.effects.push(Effect::Hud {
        to,
        what: HudShow::Fade(fade),
    });
    w.fire_output(id, "OnBeginFade", activator, Value::Void);
}

/// game_score's ApplyScore (game_entities.md 4); the rules apply it.
fn apply_score(w: &mut LogicWorld, id: EntId, activator: Option<Who>) {
    let Some(target) = player_of(w, activator) else { return };
    let Some(e) = w.get(id) else { return };
    let points = e.kv_i("points");
    if points == 0 {
        return;
    }
    w.effects.push(Effect::Score {
        target,
        points,
        team: e.has_flag(2),
        allow_negative: e.has_flag(1),
    });
}

/// env_explosion's Explode (game_entities.md 6): the game's blast at the
/// origin with the magnitude (0 with "No Damage") and radius; removed
/// 0.3 s later unless repeatable.
fn explode(w: &mut LogicWorld, id: EntId, activator: Option<Who>) {
    let Some(e) = w.get(id) else { return };
    let Class::Explosion(x) = &e.class else { return };
    let damage = if e.has_flag(SF_EXPLOSION_NO_DAMAGE) { 0.0 } else { x.magnitude.max(0) as f32 };
    let (at, radius, repeat) = (e.origin, x.radius, e.has_flag(SF_EXPLOSION_REPEATABLE));
    w.effects.push(Effect::Explosion {
        at,
        damage,
        radius,
        attacker: activator.filter(|a| matches!(a, Who::Player(_))),
        inflictor: id,
    });
    if !repeat {
        w.think_in(id, EXPLOSION_REMOVE_DELAY);
    }
}

impl LogicWorld {
    /// func_conveyor pushes (game_entities.md 8): each player standing on
    /// a pushing conveyor gets its belt velocity as base velocity this
    /// command (added to a push trigger's from the last command).
    pub fn push_conveyors(&mut self) {
        let belts: Vec<(EntId, Vec3)> = self
            .ids()
            .into_iter()
            .filter_map(|id| match &self.get(id)?.class {
                Class::Conveyor(c) if !c.no_push => Some((id, c.dir * c.speed)),
                _ => None,
            })
            .collect();
        if belts.is_empty() {
            return;
        }
        for p in &mut self.players {
            let Some(g) = p.ground.filter(|_| p.on_ground) else { continue };
            let Some((_, v)) = belts.iter().find(|(id, _)| *id == g) else { continue };
            p.base_velocity = if p.base_touched { p.base_velocity + *v } else { *v };
            p.base_touched = true;
        }
    }
}
