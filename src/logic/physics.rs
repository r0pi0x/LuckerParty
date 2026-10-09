//! Physics helpers map logic switches (specs/source/physics_brushes.md
//! 4-5): phys_thruster and phys_keepupright. The logic keeps whether
//! each is on and its numbers; the bridge hands them to the physics as
//! `map::controllers::BodyControllers` (the body's own motion is the
//! physics engine's). func_physbox is a prop here (`prop_damage`).

use bevy::prelude::*;

use super::classes::Class;
use super::value::Value;
use super::world::{EntId, LogicWorld, Who};
use crate::map::entities::entity_rotation;

/// phys_thruster spawnflags.
pub const SF_THRUST_START_ON: u32 = 1;
pub const SF_THRUST_FORCE: u32 = 2;
pub const SF_THRUST_TORQUE: u32 = 4;
pub const SF_THRUST_LOCAL: u32 = 8;
pub const SF_THRUST_IGNORE_MASS: u32 = 16;
pub const SF_THRUST_IGNORE_POS: u32 = 32;
/// phys_keepupright "Start inactive".
pub const SF_UPRIGHT_START_INACTIVE: u32 = 1;
/// Default "angularlimit", deg/s.
pub const UPRIGHT_DEFAULT_LIMIT: f32 = 15.0;

/// A phys_thruster: its body (resolved once at activation), and its
/// place and push direction in the body's frame from then.
#[derive(Clone, Debug, Default)]
pub struct Thruster {
    pub on: bool,
    pub body: Option<EntId>,
    pub scale: f32,
    /// Counts turn-ons: the physics works the push out again on each.
    pub serial: u32,
    pub offset: Vec3,
    pub dir_local: Vec3,
}

/// A phys_keepupright.
#[derive(Clone, Debug, Default)]
pub struct Upright {
    pub on: bool,
    pub body: Option<EntId>,
    pub limit: f32,
    /// The goal up axis (entity space), fixed at spawn.
    pub goal: Vec3,
}

/// A controller as the logic exports it (entity space): what pushes which
/// body.
#[derive(Clone, Debug, PartialEq)]
pub struct Control {
    pub id: EntId,
    pub body: EntId,
    pub kind: ControlKind,
}

#[derive(Clone, Debug, PartialEq)]
pub enum ControlKind {
    /// Force in kg·units/s² (world, entity space), and in the body's
    /// frame; the place in the body's frame (units).
    Thrust {
        force: Vec3,
        force_local: Vec3,
        offset: Vec3,
        flags: u32,
        scale: f32,
        serial: u32,
    },
    /// Goal up axis (entity space) and limit (deg/s).
    Upright { goal: Vec3, limit: f32 },
}

pub(super) fn spawn(e: &super::world::LogicEntity, class: &str) -> Option<Class> {
    Some(match class {
        "phys_thruster" => Class::Thruster(Box::default()),
        "phys_keepupright" => Class::Upright(Box::new(Upright {
            limit: e.kv("angularlimit").map_or(UPRIGHT_DEFAULT_LIMIT, super::value::atof),
            goal: entity_rotation(e.angles) * Vec3::Z,
            ..default()
        })),
        _ => return None,
    })
}

/// The body named `attach1`: a prop or physics brush.
fn body_named(w: &LogicWorld, id: EntId) -> Option<EntId> {
    let name = w.get(id)?.kv("attach1")?.trim().to_string();
    if name.is_empty() {
        return None;
    }
    w.find(&name)
        .filter(|b| matches!(w.get(*b).map(|e| &e.class), Some(Class::Prop(_))))
}

pub(super) fn activate(w: &mut LogicWorld, id: EntId) {
    let body = body_named(w, id);
    let Some(e) = w.get(id) else { return };
    let (origin, angles, flags) = (e.origin, e.angles, e.spawnflags);
    match e.class.clone() {
        Class::Thruster(_) => {
            let (bo, ba) = body.and_then(|b| w.get(b)).map_or((origin, Vec3::ZERO), |b| (b.origin, b.angles));
            let inv = entity_rotation(ba).inverse();
            if let Some(Class::Thruster(t)) = w.get_mut(id).map(|e| &mut e.class) {
                t.body = body;
                t.offset = if flags & SF_THRUST_IGNORE_POS != 0 { Vec3::ZERO } else { inv * (origin - bo) };
                t.dir_local = inv * super::triggers::forward(angles);
            }
            if body.is_none() {
                w.log.push("phys_thruster: no body to push (inputs do nothing)".into());
            }
            if flags & SF_THRUST_START_ON != 0 {
                thrust_on(w, id, 1.0);
            }
        }
        Class::Upright(_) => {
            let Some(b) = body else {
                // Nothing to hold up: it removes itself.
                w.kill(id);
                return;
            };
            if let Some(Class::Upright(u)) = w.get_mut(id).map(|e| &mut e.class) {
                u.body = Some(b);
                u.on = flags & SF_UPRIGHT_START_INACTIVE == 0;
            }
        }
        _ => {}
    }
}

/// Turn a thruster on (ignored when it is on).
fn thrust_on(w: &mut LogicWorld, id: EntId, scale: f32) {
    let forcetime = w.get(id).map_or(0.0, |e| e.kv_f("forcetime"));
    let Some(Class::Thruster(t)) = w.get_mut(id).map(|e| &mut e.class) else { return };
    if t.on || t.body.is_none() {
        return;
    }
    t.on = true;
    t.scale = scale;
    t.serial += 1;
    if forcetime > 0.0 {
        w.think_in(id, forcetime);
    }
}

pub(super) fn think(w: &mut LogicWorld, id: EntId) {
    // forcetime ran out.
    if let Some(Class::Thruster(t)) = w.get_mut(id).map(|e| &mut e.class) {
        t.on = false;
    }
}

pub(super) fn input(w: &mut LogicWorld, id: EntId, input: &str, value: &Value, _activator: Option<Who>) -> bool {
    let Some(class) = w.get(id).map(|e| e.class.clone()) else { return true };
    match (class, input) {
        (Class::Thruster(_), "activate") => thrust_on(w, id, 1.0),
        (Class::Thruster(_), "deactivate") => {
            if let Some(Class::Thruster(t)) = w.get_mut(id).map(|e| &mut e.class) {
                t.on = false;
            }
            if let Some(e) = w.get_mut(id) {
                e.next_think = None;
            }
        }
        (Class::Thruster(t), "scale") => {
            let Some(s) = w.need_float(value, input) else { return true };
            if !t.on {
                thrust_on(w, id, s);
            }
            if let Some(Class::Thruster(t)) = w.get_mut(id).map(|e| &mut e.class) {
                t.scale = s;
            }
        }
        (Class::Upright(_), "turnon" | "turnoff") => {
            if let Some(Class::Upright(u)) = w.get_mut(id).map(|e| &mut e.class) {
                u.on = input == "turnon";
            }
        }
        (Class::Upright(_), "setangularlimit") => {
            let Some(l) = w.need_float(value, input) else { return true };
            if let Some(Class::Upright(u)) = w.get_mut(id).map(|e| &mut e.class) {
                u.limit = l;
            }
        }
        _ => return false,
    }
    true
}

impl LogicWorld {
    /// The thrusters and keep-uprights that are on, on bodies that are
    /// there.
    pub fn controls(&self) -> Vec<Control> {
        self.ids()
            .into_iter()
            .filter_map(|id| {
                let e = self.get(id)?;
                match &e.class {
                    Class::Thruster(t) if t.on => {
                        let body = t.body.filter(|b| self.get(*b).is_some())?;
                        let force = e.kv_f("force");
                        Some(Control {
                            id,
                            body,
                            kind: ControlKind::Thrust {
                                force: super::triggers::forward(e.angles) * force,
                                force_local: t.dir_local * force,
                                offset: t.offset,
                                flags: e.spawnflags,
                                scale: t.scale,
                                serial: t.serial,
                            },
                        })
                    }
                    Class::Upright(u) if u.on => Some(Control {
                        id,
                        body: u.body.filter(|b| self.get(*b).is_some())?,
                        kind: ControlKind::Upright {
                            goal: u.goal,
                            limit: u.limit,
                        },
                    }),
                    _ => None,
                }
            })
            .collect()
    }
}
