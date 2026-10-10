//! Classes community maps use that no spec covers yet (docs/plans/
//! active/community-maps.md, "Map logic audit"), from the public entity
//! documentation; our choices where it is silent are in docs/tech-debt.md:
//! logic_measure_movement, point_teleport, logic_multicompare, env_shake.

use bevy::prelude::*;

use super::anchors::angles_of;
use super::classes::Class;
use super::value::Value;
use super::world::{Effect, EntId, LogicWorld, Who};
use crate::map::entities::entity_rotation;

/// One of this module's classes.
#[derive(Clone, Debug)]
pub enum Extra {
    Measure(Measure),
    Teleport,
    MultiCompare(MultiCompare),
    Shake(Shake),
}

/// logic_measure_movement: each tick, the target's pose relative to its
/// reference is the measured entity's pose relative to its own reference,
/// its position scaled ("TargetScale"). "MeasureType" 1 measures a
/// player's eye (position and view angles).
#[derive(Clone, Debug)]
pub struct Measure {
    pub enabled: bool,
    pub measure: String,
    pub measure_ref: String,
    pub target: String,
    pub target_ref: String,
    pub scale: f32,
    pub eye: bool,
}

/// logic_multicompare: the values given since the last compare.
#[derive(Clone, Debug, Default)]
pub struct MultiCompare {
    pub values: Vec<i32>,
}

/// env_shake's numbers (units, Hz, seconds).
#[derive(Clone, Debug)]
pub struct Shake {
    pub amplitude: f32,
    pub frequency: f32,
    pub duration: f32,
    pub radius: f32,
}

/// env_shake's "GlobalShake" flag.
pub const SF_SHAKE_GLOBAL: u32 = 1;

pub(super) fn spawn(w: &LogicWorld, id: EntId, class: &str) -> Option<Class> {
    let e = w.get(id)?;
    let s = |k: &str| e.kv(k).unwrap_or("").trim().to_string();
    let extra = match class {
        "logic_measure_movement" => Extra::Measure(Measure {
            // Measures from activation (the Disable input stops it).
            enabled: true,
            measure: s("MeasureTarget"),
            measure_ref: s("MeasureReference"),
            target: s("Target"),
            target_ref: s("TargetReference"),
            scale: e.kv("TargetScale").map_or(1.0, super::value::atof),
            eye: e.kv_i("MeasureType") == 1,
        }),
        "point_teleport" => Extra::Teleport,
        "logic_multicompare" => Extra::MultiCompare(MultiCompare::default()),
        "env_shake" => Extra::Shake(Shake {
            amplitude: e.kv_f("amplitude"),
            frequency: e.kv("frequency").map_or(40.0, super::value::atof),
            duration: e.kv("duration").map_or(1.0, super::value::atof),
            radius: e.kv_f("radius"),
        }),
        _ => return None,
    };
    Some(Class::Extra(Box::new(extra)))
}

fn extra(w: &LogicWorld, id: EntId) -> Option<&Extra> {
    match w.get(id).map(|e| &e.class) {
        Some(Class::Extra(x)) => Some(x),
        _ => None,
    }
}

fn extra_mut(w: &mut LogicWorld, id: EntId) -> Option<&mut Extra> {
    match w.get_mut(id).map(|e| &mut e.class) {
        Some(Class::Extra(x)) => Some(x),
        _ => None,
    }
}

pub(super) fn activate(w: &mut LogicWorld, id: EntId) {
    if matches!(extra(w, id), Some(Extra::Measure(_))) {
        w.think_in(id, w.dt);
    }
}

pub(super) fn think(w: &mut LogicWorld, id: EntId) {
    let Some(Extra::Measure(m)) = extra(w, id).cloned() else { return };
    if !m.enabled {
        return;
    }
    measure(w, id, &m);
    w.think_in(id, w.dt);
}

/// The first entity or player a name reaches.
fn named(w: &LogicWorld, name: &str, me: EntId) -> Option<Who> {
    if name.is_empty() {
        return None;
    }
    w.resolve(name, None, Some(Who::Ent(me))).into_iter().next()
}

/// Where something is: a player's feet (or eye) and view, a mover's
/// pusher, else an entity's origin and angles.
fn pose(w: &LogicWorld, who: Who, eye: bool) -> Option<(Vec3, Quat)> {
    match who {
        Who::Player(p) => {
            let p = w.player(p)?;
            let (at, angles) = if eye {
                (p.origin + p.eye, p.view)
            } else {
                (p.origin, Vec3::new(0.0, p.view.y, 0.0))
            };
            Some((at, entity_rotation(angles)))
        }
        Who::Ent(id) => {
            let (o, a) = w.parent_pose(Who::Ent(id));
            w.get(id)?;
            Some((o, entity_rotation(a)))
        }
    }
}

fn measure(w: &mut LogicWorld, id: EntId, m: &Measure) {
    let (Some(src), Some(src_ref), Some(target), Some(target_ref)) = (
        named(w, &m.measure, id),
        named(w, &m.measure_ref, id),
        named(w, &m.target, id),
        named(w, &m.target_ref, id),
    ) else {
        return;
    };
    let (Some((so, sr)), Some((ro, rr)), Some((to, tr))) =
        (pose(w, src, m.eye), pose(w, src_ref, false), pose(w, target_ref, false))
    else {
        return;
    };
    let local = rr.inverse() * (so - ro) * m.scale;
    let local_rot = rr.inverse() * sr;
    place(w, target, to + tr * local, angles_of(tr * local_rot));
}

/// Put an entity (or player) at a pose: a player's feet and view, a
/// physics body (its centre), a mover (its track along), else its origin.
pub(super) fn place(w: &mut LogicWorld, who: Who, origin: Vec3, angles: Vec3) {
    match who {
        Who::Player(p) => {
            if let Some(pl) = w.player_mut(p) {
                pl.origin = origin;
                pl.view = angles;
                pl.on_ground = false;
                pl.unground = true;
                pl.teleported = true;
            }
        }
        Who::Ent(id) => {
            if super::triggers::physics_body(w, id).is_some() {
                w.effects.push(Effect::BodyTeleport {
                    id,
                    origin,
                    angles: Some(angles),
                });
                return;
            }
            let (o, _) = w.parent_pose(Who::Ent(id));
            let Some(e) = w.get_mut(id) else { return };
            let delta = origin - o;
            if super::movers::shift(&mut e.class, delta, Vec3::ZERO) {
                if let Some(p) = super::movers::pusher_mut(&mut e.class) {
                    p.angles = angles;
                }
                w.refresh_solid(id);
                return;
            }
            e.origin = origin;
            e.angles = angles;
            let hulls = e.hulls.clone();
            if let Class::Trigger(t) = &mut e.class {
                let rot = entity_rotation(angles);
                t.brushes = hulls.iter().map(|h| super::world::place_hull(h, rot, origin)).collect();
            }
        }
    }
}

pub(super) fn input(w: &mut LogicWorld, id: EntId, input: &str, value: &Value, activator: Option<Who>) -> bool {
    let Some(x) = extra(w, id).cloned() else { return false };
    match (x, input) {
        (Extra::Measure(_), "enable" | "disable") => {
            let on = input == "enable";
            if let Some(Extra::Measure(m)) = extra_mut(w, id) {
                m.enabled = on;
            }
            if on {
                w.think_in(id, w.dt);
            }
        }
        (Extra::Measure(_), "setmeasuretarget" | "setmeasurereference" | "settarget" | "settargetreference") => {
            let Some(name) = w.need_str(value, input) else { return true };
            if let Some(Extra::Measure(m)) = extra_mut(w, id) {
                *match input {
                    "setmeasuretarget" => &mut m.measure,
                    "setmeasurereference" => &mut m.measure_ref,
                    "settarget" => &mut m.target,
                    _ => &mut m.target_ref,
                } = name.trim().to_string();
            }
        }
        (Extra::Measure(_), "settargetscale") => {
            let Some(v) = w.need_float(value, input) else { return true };
            if let Some(Extra::Measure(m)) = extra_mut(w, id) {
                m.scale = v;
            }
        }
        (Extra::Teleport, "teleport") => {
            let Some(e) = w.get(id) else { return true };
            let (origin, angles) = (e.origin, e.angles);
            let target = e.kv("target").unwrap_or("").trim().to_string();
            for who in w.resolve(&target, activator, Some(Who::Ent(id))) {
                place(w, who, origin, angles);
            }
        }
        (Extra::MultiCompare(_), "updatevalue") => {
            let Some(v) = w.need_int(value, input) else { return true };
            if let Some(Extra::MultiCompare(m)) = extra_mut(w, id) {
                m.values.push(v);
            }
        }
        (Extra::MultiCompare(m), "comparevalues") => {
            let (to_value, value) = w
                .get(id)
                .map_or((false, 0), |e| (e.kv_i("ShouldComparetoValue") != 0, e.kv_i("IntegerValue")));
            // Every value given since the last compare equal (to each
            // other, or to IntegerValue when asked); none given: equal.
            let first = if to_value { Some(value) } else { m.values.first().copied() };
            let equal = first.is_none_or(|f| m.values.iter().all(|v| *v == f));
            if let Some(Extra::MultiCompare(m)) = extra_mut(w, id) {
                m.values.clear();
            }
            w.fire_output(id, if equal { "OnEqual" } else { "OnNotEqual" }, activator, Value::Void);
        }
        (Extra::Shake(s), "startshake" | "stopshake") => {
            let Some(e) = w.get(id) else { return true };
            let global = e.has_flag(SF_SHAKE_GLOBAL);
            let at = e.origin;
            let amplitude = if input == "startshake" { s.amplitude } else { 0.0 };
            w.effects.push(Effect::Shake {
                at,
                amplitude,
                frequency: s.frequency,
                duration: if input == "startshake" { s.duration } else { 0.0 },
                radius: if global { f32::INFINITY } else { s.radius },
            });
        }
        (Extra::Shake(_), "amplitude" | "frequency") => {
            let Some(v) = w.need_float(value, input) else { return true };
            if let Some(Extra::Shake(s)) = extra_mut(w, id) {
                if input == "amplitude" {
                    s.amplitude = v;
                } else {
                    s.frequency = v;
                }
            }
        }
        _ => return false,
    }
    true
}
