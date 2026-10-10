//! Brush triggers (specs/source/triggers.md): touch links refreshed each
//! tick at the player's final position, start/touch/end touch with the
//! spawnflag and filter rules, and trigger_multiple/once, trigger_hurt,
//! trigger_push, trigger_teleport, trigger_gravity, trigger_remove.

use bevy::prelude::*;

use super::classes::{Class, filter_passes};
use super::value::Value;
use super::world::{Collision, Effect, EntId, LogicWorld, Who, box_touches, place_hull};
use crate::map::MapBrush;

/// trigger_hurt pass period, seconds (33 ticks at 0.015).
pub const HURT_INTERVAL: f32 = 0.5;
/// Doubling damage resets after this long with no victims.
pub const HURT_FORGIVE: f32 = 3.0;

#[derive(Clone, Debug)]
pub enum Kind {
    /// trigger_multiple / trigger_once: wait in seconds (<= 0: once).
    Multiple { wait: f32 },
    Hurt(Hurt),
    Push { dir: Vec3, speed: f32 },
    Teleport { target: String, landmark: String },
    Gravity(f32),
    Remove,
    /// Base behaviour only (trigger_soundscape and others).
    Other,
}

#[derive(Clone, Debug, Default)]
pub struct Hurt {
    pub damage: f32,
    pub original: f32,
    pub cap: f32,
    pub doubling: bool,
    /// Victims of the last pass.
    pub hurt: Vec<Who>,
    pub forgive_until: f32,
    /// A pass cycle is running.
    pub cycling: bool,
}

#[derive(Clone, Debug)]
pub struct Trigger {
    pub kind: Kind,
    /// The trigger flag (Enable/Disable/Toggle).
    pub enabled: bool,
    pub filter_name: String,
    pub filter: Option<EntId>,
    /// Engine touch links: who, and whether refreshed this frame.
    pub links: Vec<(Who, bool)>,
    /// Entities that passed the filters on start touch.
    pub touching: Vec<Who>,
    /// Volume in the world (entity space).
    pub brushes: Vec<MapBrush>,
    /// trigger_multiple: no OnTrigger before this tick.
    pub wait_until: i64,
    /// trigger_once after firing (deleted shortly).
    pub spent: bool,
}

/// Forward vector of Source angles (pitch down positive).
pub fn forward(angles: Vec3) -> Vec3 {
    let (p, y) = (angles.x.to_radians(), angles.y.to_radians());
    Vec3::new(p.cos() * y.cos(), p.cos() * y.sin(), -p.sin())
}

pub(super) fn spawn(w: &mut LogicWorld, id: EntId) -> Option<Trigger> {
    if w.get(id)?.kv_i("damagetype") & 262_144 != 0 {
        w.log.push("trigger_hurt: radiation timing is not modelled (normal passes)".into());
    }
    let e = w.get(id)?;
    let class = e.classname.to_ascii_lowercase();
    let kind = match class.as_str() {
        "trigger_multiple" => {
            let wait = e.kv_f("wait");
            Kind::Multiple {
                wait: if wait == 0.0 { 0.2 } else { wait },
            }
        }
        "trigger_once" => Kind::Multiple { wait: -1.0 },
        "trigger_hurt" => {
            let damage = e.kv("damage").map_or(10.0, super::value::atof);
            Kind::Hurt(Hurt {
                damage,
                original: damage,
                cap: e.kv_f("damagecap"),
                doubling: e.kv_i("damagemodel") == 1,
                ..default()
            })
        }
        "trigger_push" => {
            let speed = e.kv_f("speed");
            Kind::Push {
                dir: forward(e.kv("pushdir").map_or(Vec3::ZERO, crate::map::entities::parse_vector)),
                speed: if speed == 0.0 { 100.0 } else { speed },
            }
        }
        "trigger_teleport" => Kind::Teleport {
            target: e.kv("target").unwrap_or("").into(),
            landmark: e.kv("landmark").unwrap_or("").into(),
        },
        "trigger_gravity" => Kind::Gravity(e.kv_f("gravity")),
        "trigger_remove" => Kind::Remove,
        _ => Kind::Other,
    };
    let rotation = crate::map::entities::entity_rotation(e.angles);
    let brushes = e.hulls.iter().map(|h| place_hull(h, rotation, e.origin)).collect();
    let e = w.get(id)?;
    Some(Trigger {
        kind,
        enabled: e.kv_i("StartDisabled") == 0,
        filter_name: e.kv("filtername").unwrap_or("").into(),
        filter: None,
        links: Vec::new(),
        touching: Vec::new(),
        brushes,
        wait_until: i64::MIN,
        spent: false,
    })
}

pub(super) fn activate(w: &mut LogicWorld, id: EntId) {
    let name = trig(w, id).map(|t| t.filter_name.clone()).unwrap_or_default();
    if name.is_empty() {
        return;
    }
    let filter = w
        .ids()
        .into_iter()
        .find(|f| {
            w.get(*f).is_some_and(|e| {
                e.targetname.eq_ignore_ascii_case(&name) && matches!(e.class, Class::Filter(_))
            })
        });
    if filter.is_none() {
        w.log.push(format!("trigger: filter '{name}' not found"));
    }
    if let Some(t) = trig_mut(w, id) {
        t.filter = filter;
    }
}

pub(super) fn keyvalue(w: &mut LogicWorld, id: EntId, key: &str) -> bool {
    if let Some(new) = spawn(w, id)
        && let Some(t) = trig_mut(w, id)
    {
        match key {
            "wait" | "damage" | "speed" | "pushdir" | "target" | "landmark" | "gravity" => {
                t.kind = new.kind;
                return true;
            }
            _ => {}
        }
    }
    false
}

fn trig(w: &LogicWorld, id: EntId) -> Option<&Trigger> {
    match w.get(id).map(|e| &e.class) {
        Some(Class::Trigger(t)) => Some(t),
        _ => None,
    }
}

fn trig_mut(w: &mut LogicWorld, id: EntId) -> Option<&mut Trigger> {
    match w.get_mut(id).map(|e| &mut e.class) {
        Some(Class::Trigger(t)) => Some(t),
        _ => None,
    }
}

/// A physics body's world box (entity space) when the host has placed
/// it: a physics prop or physics brush.
pub fn physics_body(w: &LogicWorld, id: EntId) -> Option<(Vec3, Vec3)> {
    match w.get(id).map(|e| &e.class) {
        Some(Class::Prop(p)) if p.physics => p.bounds,
        _ => None,
    }
}

/// Spawnflags and filter (triggers.md, "Which entities can touch").
pub fn passes(w: &LogicWorld, id: EntId, who: Who) -> bool {
    let Some(e) = w.get(id) else { return false };
    let Some(t) = trig(w, id) else { return false };
    let mut flags = e.spawnflags;
    if flags & (32 | 512) != 0 {
        flags |= 1;
    }
    let class_ok = match who {
        Who::Player(p) => {
            let alive = w.player(p).is_some_and(|p| p.alive);
            alive && flags & (1 | 64) != 0 && flags & 32 == 0
        }
        // "Physics Objects" (8): physics props and physics brushes.
        Who::Ent(e) => flags & 64 != 0 || (flags & 8 != 0 && physics_body(w, e).is_some()),
    };
    class_ok && t.filter.is_none_or(|f| filter_passes(w, f, Some(who)))
}

impl LogicWorld {
    /// Re-link every player at its position against the triggers: start
    /// touch then touch for new overlaps, touch for existing ones.
    pub fn touch_triggers(&mut self, _collision: &dyn Collision) {
        let triggers: Vec<EntId> = self
            .ids()
            .into_iter()
            .filter(|id| matches!(self.get(*id).map(|e| &e.class), Some(Class::Trigger(_))))
            .collect();
        for i in 0..self.players.len() {
            if !self.players[i].alive {
                continue;
            }
            let who = Who::Player(self.players[i].entity);
            // The triggers the box overlaps where it stands, then their
            // touches in order: a teleport among them doesn't take the
            // player out of the others' list (mg_wipeout2 scores with a
            // trigger_multiple on its stage teleport); it re-links at the
            // new place, a few times at most (chained teleports), each
            // trigger touched once a tick.
            let mut done: Vec<EntId> = Vec::new();
            for _ in 0..4 {
                let Some(p) = self.players.get(i) else { break };
                let at = p.origin;
                let (lo, hi) = (p.origin + p.mins, p.origin + p.maxs);
                let overlapped: Vec<EntId> = triggers
                    .iter()
                    .copied()
                    .filter(|t| {
                        !done.contains(t)
                            && trig(self, *t)
                                .is_some_and(|tr| tr.enabled && tr.brushes.iter().any(|b| box_touches(b, lo, hi)))
                            && self.get(*t).is_some_and(|e| !e.killed)
                    })
                    .collect();
                done.extend(&overlapped);
                for t in overlapped {
                    let Some(trigger) = trig_mut(self, t) else { continue };
                    match trigger.links.iter_mut().find(|(w, _)| *w == who) {
                        Some(link) => link.1 = true,
                        None => {
                            trigger.links.push((who, true));
                            start_touch(self, t, who);
                        }
                    }
                    touch(self, t, who);
                }
                if self.players.get(i).is_none_or(|p| p.origin == at) {
                    break;
                }
            }
        }
        self.touch_bodies();
        self.touch_movers();
    }

    /// Physics bodies (props, physics brushes; where the host says they
    /// are) against the triggers that let physics objects or everything
    /// touch (spawnflags 8, 64): the same start touch and touch as for
    /// players (triggers.md: the boats of mg_boatrace_scramble cross its
    /// boost and finish triggers).
    fn touch_bodies(&mut self) {
        let bodies: Vec<(EntId, Vec3, Vec3)> = self
            .ids()
            .into_iter()
            .filter_map(|id| {
                let (lo, hi) = physics_body(self, id)?;
                Some((id, lo, hi))
            })
            .collect();
        if bodies.is_empty() {
            return;
        }
        let triggers: Vec<EntId> = self
            .ids()
            .into_iter()
            .filter(|id| {
                self.get(*id)
                    .is_some_and(|e| matches!(e.class, Class::Trigger(_)) && e.spawnflags & (8 | 64) != 0 && !e.killed)
            })
            .collect();
        for t in triggers {
            for &(b, lo, hi) in &bodies {
                let Some(trigger) = trig(self, t) else { break };
                if !trigger.enabled || self.get(b).is_none_or(|e| e.killed) {
                    continue;
                }
                if !trigger.brushes.iter().any(|br| box_touches(br, lo, hi)) {
                    continue;
                }
                let who = Who::Ent(b);
                let trigger = trig_mut(self, t).unwrap();
                match trigger.links.iter_mut().find(|(w, _)| *w == who) {
                    Some(link) => link.1 = true,
                    None => {
                        trigger.links.push((who, true));
                        start_touch(self, t, who);
                    }
                }
                touch(self, t, who);
            }
        }
    }

    /// End touch for links not refreshed this frame.
    pub fn untouch(&mut self) {
        for id in self.ids() {
            let Some(t) = trig_mut(self, id) else { continue };
            let mut ended = Vec::new();
            t.links.retain_mut(|(who, fresh)| {
                let keep = *fresh;
                *fresh = false;
                if !keep {
                    ended.push(*who);
                }
                keep
            });
            for who in ended {
                end_touch(self, id, who);
            }
        }
    }
}

fn start_touch(w: &mut LogicWorld, id: EntId, who: Who) {
    if !passes(w, id, who) {
        return;
    }
    let t = trig_mut(w, id).unwrap();
    t.touching.push(who);
    let first = t.touching.len() == 1;
    w.fire_output(id, "OnStartTouch", Some(who), Value::Void);
    if first {
        w.fire_output(id, "OnStartTouchAll", Some(who), Value::Void);
    }
}

fn end_touch(w: &mut LogicWorld, id: EntId, who: Who) {
    let Some(t) = trig(w, id) else { return };
    let was_in = t.touching.contains(&who);
    // trigger_hurt: leaving without being hurt in the last pass takes a
    // half hit.
    if let Kind::Hurt(h) = &t.kind
        && passes_ignoring_life(w, id, who)
        && !h.hurt.contains(&who)
        && let Who::Player(p) = who
    {
        let per = h.damage * 0.5;
        hurt_player(w, p, per);
    }
    if !was_in {
        return;
    }
    let t = trig_mut(w, id).unwrap();
    if let Some(i) = t.touching.iter().position(|x| *x == who) {
        t.touching.remove(i);
    }
    w.fire_output(id, "OnEndTouch", Some(who), Value::Void);
    // Prune the deleted and the dead before deciding "all gone".
    let alive = |w: &LogicWorld, x: &Who| match x {
        Who::Player(p) => w.player(*p).is_some_and(|p| p.alive),
        Who::Ent(e) => w.get(*e).is_some(),
    };
    let list: Vec<Who> = trig(w, id).unwrap().touching.clone();
    let kept: Vec<Who> = list.into_iter().filter(|x| alive(w, x)).collect();
    let empty = kept.is_empty();
    trig_mut(w, id).unwrap().touching = kept;
    if empty {
        w.fire_output(id, "OnEndTouchAll", Some(who), Value::Void);
    }
}

/// `passes` for a player who may have just died (exit half-hit).
fn passes_ignoring_life(w: &LogicWorld, id: EntId, who: Who) -> bool {
    match who {
        Who::Player(p) if w.player(p).is_some_and(|p| p.alive) => passes(w, id, who),
        _ => false,
    }
}

fn hurt_player(w: &mut LogicWorld, p: Entity, per: f32) {
    if per < 0.0 {
        w.effects.push(Effect::Heal { target: p, amount: -per });
    } else if per > 0.0 {
        w.effects.push(Effect::Damage {
            target: p,
            amount: per,
            crush: false,
        });
    }
}

fn touch(w: &mut LogicWorld, id: EntId, who: Who) {
    if !passes(w, id, who) {
        return;
    }
    let Some(t) = trig(w, id).cloned() else { return };
    match t.kind {
        Kind::Multiple { wait } => {
            if t.spent || t.wait_until > w.tick {
                return;
            }
            w.fire_output(id, "OnTrigger", Some(who), Value::Void);
            if wait > 0.0 {
                let until = w.tick + w.think_ticks(wait);
                trig_mut(w, id).unwrap().wait_until = until;
            } else {
                trig_mut(w, id).unwrap().spent = true;
                // Deleted 0.1 s later (a think).
                w.think_in(id, 0.1);
            }
        }
        Kind::Hurt(h) => {
            if !h.cycling {
                if let Some(Kind::Hurt(h)) = trig_mut(w, id).map(|t| &mut t.kind) {
                    h.cycling = true;
                }
                // This tick's thinks already ran: the first pass is the
                // next think opportunity.
                let tick = w.tick;
                w.get_mut(id).unwrap().next_think = Some(tick);
            }
        }
        Kind::Push { dir, speed } => {
            let Who::Player(p) = who else {
                // Physics objects: a force of speed x direction x 100 x dt
                // on the centre each tick (an impulse, kg units/s); once
                // only: added to the velocity, and the trigger goes.
                if let Who::Ent(b) = who
                    && let Some(Class::Prop(prop)) = w.get(b).map(|e| &e.class)
                {
                    let once = w.get(id).is_some_and(|e| e.has_flag(128));
                    let velocity = if once {
                        dir * speed
                    } else {
                        dir * speed * 100.0 * w.dt / prop.mass.max(1.0)
                    };
                    w.effects.push(Effect::BodyVelocity { id: b, velocity });
                    if once {
                        w.kill(id);
                    }
                }
                return;
            };
            let once = w.get(id).is_some_and(|e| e.has_flag(128));
            let push = dir * speed;
            let Some(pl) = w.player_mut(p) else { return };
            if once {
                pl.velocity += push;
                if push.z > 0.0 {
                    pl.on_ground = false;
                    pl.unground = true;
                }
                w.kill(id);
                return;
            }
            let mut push = push;
            if pl.base_touched {
                push += pl.base_velocity;
            }
            if push.z > 0.0 && pl.on_ground {
                pl.on_ground = false;
                pl.unground = true;
                pl.origin.z += 1.0;
                pl.moved = true;
            }
            pl.base_velocity = push;
            pl.base_touched = true;
        }
        Kind::Teleport { target, landmark } => {
            if let Who::Ent(b) = who {
                teleport_body(w, id, b, &target, &landmark);
                return;
            }
            let Who::Player(p) = who else { return };
            let Some(dest) = w.resolve(&target, Some(who), Some(Who::Ent(id))).into_iter().next() else {
                return;
            };
            // Destination point and angles; a player destination is its
            // feet and view.
            let (point, angles) = match dest {
                Who::Ent(d) => {
                    let e = w.get(d).unwrap();
                    (e.origin, e.angles)
                }
                Who::Player(q) => {
                    let q = w.player(q).unwrap();
                    (q.origin + Vec3::Z * q.mins.z, q.view)
                }
            };
            let mark = (!landmark.is_empty())
                .then(|| w.find(&landmark))
                .flatten()
                .and_then(|l| w.get(l).map(|e| e.origin));
            let Some(pl) = w.player_mut(p) else { return };
            pl.on_ground = false;
            pl.unground = true;
            match mark {
                Some(l) => pl.origin = point + (pl.origin - l),
                None => {
                    pl.origin = point - Vec3::Z * pl.mins.z;
                    pl.view = angles;
                }
            }
            pl.teleported = true;
        }
        Kind::Gravity(g) => {
            if let Who::Player(p) = who
                && let Some(pl) = w.player_mut(p)
            {
                pl.gravity = g;
            }
        }
        Kind::Remove => {
            if let Who::Ent(e) = who {
                w.kill(e);
            }
        }
        Kind::Other => {}
    }
}

/// trigger_teleport on a physics body: to the destination (plus its
/// offset from the landmark), taking the destination's angles without a
/// landmark; velocity kept (triggers.md).
fn teleport_body(w: &mut LogicWorld, trigger: EntId, body: EntId, target: &str, landmark: &str) {
    let who = Who::Ent(body);
    let Some(Who::Ent(dest)) = w.resolve(target, Some(who), Some(Who::Ent(trigger))).into_iter().next() else {
        return;
    };
    let Some((point, angles)) = w.get(dest).map(|e| (e.origin, e.angles)) else {
        return;
    };
    let Some((lo, hi)) = physics_body(w, body) else { return };
    let centre = (lo + hi) / 2.0;
    let mark = (!landmark.is_empty())
        .then(|| w.find(landmark))
        .flatten()
        .and_then(|l| w.get(l).map(|e| e.origin));
    let (origin, angles) = match mark {
        // The body's centre keeps its place relative to the landmark.
        Some(l) => (point + (centre - l), None),
        None => (point, Some(angles)),
    };
    w.effects.push(Effect::BodyTeleport {
        id: body,
        origin,
        angles,
    });
    // Where it is now, until the host says (no second teleport from a
    // stale box this tick).
    let half = (hi - lo) / 2.0;
    w.set_prop_bounds(body, (origin - half, origin + half));
}

/// trigger_hurt pass, trigger_once removal.
pub(super) fn think(w: &mut LogicWorld, id: EntId) {
    let Some(t) = trig(w, id).cloned() else { return };
    match t.kind {
        Kind::Multiple { .. } if t.spent => w.kill(id),
        Kind::Hurt(mut h) => {
            // Doubling damage is forgiven once its deadline passed (also
            // when the cycle had stopped meanwhile).
            if h.doubling && h.forgive_until > 0.0 && w.now() > h.forgive_until {
                h.damage = h.original;
            }
            let per = h.damage * HURT_INTERVAL;
            let victims: Vec<Who> = t
                .links
                .iter()
                .map(|(x, _)| *x)
                .filter(|x| passes(w, id, *x))
                .collect();
            for v in &victims {
                if let Who::Player(p) = v {
                    hurt_player(w, *p, per);
                    w.fire_output(id, "OnHurtPlayer", Some(*v), Value::Void);
                } else {
                    w.fire_output(id, "OnHurt", Some(*v), Value::Void);
                }
            }
            let now = w.now();
            if h.doubling {
                if !victims.is_empty() {
                    h.damage = (h.damage * 2.0).min(h.cap);
                    h.forgive_until = now + HURT_FORGIVE;
                } else if now > h.forgive_until {
                    h.damage = h.original;
                }
            }
            h.cycling = !victims.is_empty();
            h.hurt = victims;
            let cycling = h.cycling;
            trig_mut(w, id).unwrap().kind = Kind::Hurt(h);
            if cycling {
                w.think_in(id, HURT_INTERVAL);
            }
        }
        _ => {}
    }
}

pub(super) fn input(
    w: &mut LogicWorld,
    id: EntId,
    input: &str,
    value: &Value,
    activator: Option<Who>,
    caller: Option<Who>,
) -> bool {
    match input {
        "enable" => trig_mut(w, id).unwrap().enabled = true,
        "disable" => trig_mut(w, id).unwrap().enabled = false,
        "toggle" => {
            let t = trig_mut(w, id).unwrap();
            t.enabled = !t.enabled;
        }
        "disableandendtouch" => {
            let links: Vec<Who> = trig(w, id).unwrap().links.iter().map(|(x, _)| *x).collect();
            trig_mut(w, id).unwrap().links.clear();
            for who in links {
                end_touch(w, id, who);
            }
            trig_mut(w, id).unwrap().enabled = false;
        }
        "touchtest" => {
            let t = trig(w, id).unwrap();
            if t.enabled {
                let any = !t.touching.is_empty();
                w.fire_output(id, if any { "OnTouching" } else { "OnNotTouching" }, activator, Value::Void);
            }
        }
        "starttouch" | "endtouch" => {
            if let Some(c) = caller {
                if input == "starttouch" {
                    start_touch(w, id, c);
                } else {
                    end_touch(w, id, c);
                }
            }
        }
        "setdamage" => {
            let Some(v) = w.need_float(value, input) else { return true };
            if let Some(Kind::Hurt(h)) = trig_mut(w, id).map(|t| &mut t.kind) {
                h.damage = v;
                h.original = v;
            } else {
                return false;
            }
        }
        _ => return false,
    }
    true
}
