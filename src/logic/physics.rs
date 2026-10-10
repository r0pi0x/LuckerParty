//! Physics helpers map logic switches (specs/source/physics_brushes.md
//! 4-5): phys_thruster and phys_keepupright; phys_motor (public entity
//! docs) and the two-object constraints (specs/source/
//! physics_constraints.md: bodies, world, on/off, Break; the joints are
//! the physics engine's, `map::controllers::BodyJoints`). The logic keeps whether
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

/// phys_motor spawnflags (public entity docs).
pub const SF_MOTOR_START_ON: u32 = 1;
pub const SF_MOTOR_NO_COLLIDE: u32 = 2;
pub const SF_MOTOR_HINGE: u32 = 4;

/// A phys_motor: spins its body about an axis at a speed, reached over
/// the spin-up time; with "Hinge Object" the body is also hinged to the
/// world on that axis.
#[derive(Clone, Debug, Default)]
pub struct Motor {
    pub on: bool,
    pub body: Option<EntId>,
    /// Degrees per second.
    pub speed: f32,
    /// Unit axis, entity space (from the origin toward the "axis" point).
    pub axis: Vec3,
}

/// Constraint spawnflags (physics_constraints.md 1).
pub const SF_JOINT_NO_COLLIDE: u32 = 1;
pub const SF_JOINT_START_INACTIVE: u32 = 4;
pub const SF_JOINT_CONNECT_ON_TURN_ON: u32 = 16;

/// What a constraint holds.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum JointKind {
    /// phys_constraint: welded.
    Fixed,
    /// phys_ballsocket, phys_ragdollconstraint (its angle limits aren't
    /// kept): a point both turn about.
    Ball,
    /// phys_hinge (and a motor's "Hinge Object"): turns about an axis.
    Hinge { axis: Vec3 },
    /// phys_slideconstraint: slides along an axis.
    Slide { axis: Vec3 },
    /// phys_lengthconstraint: at most `max` (at least `min`) apart.
    Length { min: f32, max: f32, other: Vec3 },
}

/// A two-object constraint (physics_constraints.md): its bodies (None:
/// the world), resolved at activation; on or off; gone once broken.
#[derive(Clone, Debug)]
pub struct Joint {
    pub kind: JointKind,
    pub on: bool,
    pub connected: bool,
    pub body1: Option<EntId>,
    pub body2: Option<EntId>,
    /// Where the bodies are held together (entity space).
    pub anchor: Vec3,
    pub no_collide: bool,
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
    /// phys_motor: spin about `axis` (entity space) at `speed` deg/s,
    /// reached over `spinup` s.
    Motor { axis: Vec3, speed: f32, spinup: f32 },
}

/// A constraint as the logic exports it: bodies (body1 None: the world).
#[derive(Clone, Debug, PartialEq)]
pub struct JointExport {
    pub id: EntId,
    pub kind: JointKind,
    pub body1: Option<EntId>,
    pub body2: EntId,
    pub anchor: Vec3,
    pub on: bool,
    pub no_collide: bool,
}

/// A point keyvalue's direction from the entity's origin (unit; zero
/// when it is missing or at the origin).
fn axis_from(e: &super::world::LogicEntity, key: &str) -> Vec3 {
    match e.kv(key) {
        Some(v) => (crate::map::entities::parse_vector(v) - e.origin).normalize_or_zero(),
        None => Vec3::ZERO,
    }
}

pub(super) fn spawn(e: &super::world::LogicEntity, class: &str) -> Option<Class> {
    Some(match class {
        "phys_thruster" => Class::Thruster(Box::default()),
        "phys_keepupright" => Class::Upright(Box::new(Upright {
            limit: e.kv("angularlimit").map_or(UPRIGHT_DEFAULT_LIMIT, super::value::atof),
            goal: entity_rotation(e.angles) * Vec3::Z,
            ..default()
        })),
        "phys_motor" => Class::Motor(Box::new(Motor {
            on: false,
            body: None,
            speed: e.kv_f("speed"),
            axis: axis_from(e, "axis"),
        })),
        "phys_constraint"
        | "phys_ballsocket"
        | "phys_hinge"
        | "phys_slideconstraint"
        | "phys_lengthconstraint"
        | "phys_ragdollconstraint" => {
            let kind = match class {
                "phys_constraint" => JointKind::Fixed,
                "phys_hinge" => JointKind::Hinge {
                    axis: axis_from(e, "hingeaxis"),
                },
                "phys_slideconstraint" => JointKind::Slide {
                    axis: axis_from(e, "slideaxis"),
                },
                "phys_lengthconstraint" => JointKind::Length {
                    min: e.kv_f("minlength"),
                    max: e.kv_f("addlength")
                        + e.kv("attachpoint")
                            .map_or(0.0, |p| crate::map::entities::parse_vector(p).distance(e.origin)),
                    other: e.kv("attachpoint").map_or(e.origin, crate::map::entities::parse_vector),
                },
                _ => JointKind::Ball,
            };
            Class::Joint(Box::new(Joint {
                kind,
                on: false,
                connected: false,
                body1: None,
                body2: None,
                anchor: e.origin,
                no_collide: e.has_flag(SF_JOINT_NO_COLLIDE),
            }))
        }
        _ => return None,
    })
}

/// The body named `attach1`: a prop or physics brush.
fn body_named(w: &LogicWorld, id: EntId) -> Option<EntId> {
    body_by_key(w, id, "attach1")
}

/// The first entity named by `key` that has a physics body: a prop,
/// physics brush or mover (physics_constraints.md 1.2).
fn body_by_key(w: &LogicWorld, id: EntId, key: &str) -> Option<EntId> {
    let name = w.get(id)?.kv(key)?.trim().to_string();
    if name.is_empty() {
        return None;
    }
    w.ids().into_iter().find(|b| {
        w.get(*b).is_some_and(|e| {
            !e.targetname.is_empty()
                && super::world::name_matches(&name, &e.targetname)
                && (matches!(e.class, Class::Prop(_)) || super::movers::pusher(&e.class).is_some())
        })
    })
}

/// Connect a constraint to its bodies (at activation, or its first
/// TurnOn with "Do not connect until turned on").
fn connect(w: &mut LogicWorld, id: EntId) {
    let (b1, b2) = (body_by_key(w, id, "attach1"), body_by_key(w, id, "attach2"));
    // One body: held to the world (1.3); none: nothing to hold.
    let (b1, b2) = match (b1, b2) {
        (Some(a), Some(b)) => (Some(a), Some(b)),
        (Some(a), None) | (None, Some(a)) => (None, Some(a)),
        (None, None) => (None, None),
    };
    if b2.is_none() {
        w.log.push("phys constraint: no body to hold (removed)".into());
        w.kill(id);
        return;
    }
    if let Some(Class::Joint(j)) = w.get_mut(id).map(|e| &mut e.class) {
        j.body1 = b1;
        j.body2 = b2;
        j.connected = true;
    }
}

/// A constraint breaks: OnBreak, and it is gone.
fn break_joint(w: &mut LogicWorld, id: EntId) {
    w.fire_output(id, "OnBreak", Some(Who::Ent(id)), Value::Void);
    w.kill(id);
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
        Class::Motor(_) => {
            if let Some(Class::Motor(m)) = w.get_mut(id).map(|e| &mut e.class) {
                m.body = body;
                m.on = flags & SF_MOTOR_START_ON != 0;
            }
        }
        Class::Joint(_) => {
            let on = flags & SF_JOINT_START_INACTIVE == 0;
            if let Some(Class::Joint(j)) = w.get_mut(id).map(|e| &mut e.class) {
                j.on = on;
            }
            if flags & SF_JOINT_CONNECT_ON_TURN_ON == 0 {
                connect(w, id);
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
        (Class::Motor(_), "turnon" | "turnoff") => {
            if let Some(Class::Motor(m)) = w.get_mut(id).map(|e| &mut e.class) {
                m.on = input == "turnon";
            }
        }
        (Class::Motor(_), "setspeed") => {
            let Some(v) = w.need_float(value, input) else {
                return true;
            };
            if let Some(Class::Motor(m)) = w.get_mut(id).map(|e| &mut e.class) {
                m.speed = v;
            }
        }
        (Class::Joint(j), "turnon") => {
            if !j.connected {
                connect(w, id);
            }
            if let Some(Class::Joint(j)) = w.get_mut(id).map(|e| &mut e.class) {
                j.on = true;
            }
        }
        (Class::Joint(_), "turnoff") => {
            if let Some(Class::Joint(j)) = w.get_mut(id).map(|e| &mut e.class) {
                j.on = false;
            }
        }
        (Class::Joint(_), "break" | "constraintbroken") => break_joint(w, id),
        // Motors and friction of hinges and sliders: kept as placed
        // (tech-debt).
        (Class::Joint(_), "setangularvelocity" | "sethingefriction" | "setvelocity") => {}
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
    /// The constraints holding bodies (connected, not broken), with
    /// phys_motors' "Hinge Object" hinges.
    pub fn joints(&self) -> Vec<JointExport> {
        self.ids()
            .into_iter()
            .filter_map(|id| {
                let e = self.get(id)?;
                match &e.class {
                    Class::Joint(j) if j.connected => Some(JointExport {
                        id,
                        kind: j.kind,
                        body1: j.body1.filter(|b| self.get(*b).is_some()),
                        body2: j.body2.filter(|b| self.get(*b).is_some())?,
                        anchor: j.anchor,
                        on: j.on,
                        no_collide: j.no_collide,
                    }),
                    Class::Motor(m) if e.has_flag(SF_MOTOR_HINGE) => Some(JointExport {
                        id,
                        kind: JointKind::Hinge { axis: m.axis },
                        body1: None,
                        body2: m.body.filter(|b| self.get(*b).is_some())?,
                        anchor: e.origin,
                        on: true,
                        no_collide: e.has_flag(SF_MOTOR_NO_COLLIDE),
                    }),
                    _ => None,
                }
            })
            .collect()
    }

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
                    Class::Motor(m) if m.on => Some(Control {
                        id,
                        body: m.body.filter(|b| self.get(*b).is_some())?,
                        kind: ControlKind::Motor {
                            axis: m.axis,
                            speed: m.speed,
                            spinup: e.kv_f("spinup"),
                        },
                    }),
                    _ => None,
                }
            })
            .collect()
    }
}
