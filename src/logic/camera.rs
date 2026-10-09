//! point_viewcontrol (specs/source/viewcontrol_and_templates.md 1): a
//! camera that takes over one player's view (the input's activator; with
//! no player activator nothing happens in multiplayer), turns toward a
//! target, travels a path_corner chain, can freeze the player, and makes
//! it invulnerable while it views. The bridge writes the view
//! (`core::MapView`) and the freeze (`core::MapControls`).

use bevy::prelude::*;

use super::classes::Class;
use super::value::Value;
use super::world::{Effect, EntId, LogicWorld, Who};

/// Spawnflags.
pub const SF_CAM_START_AT_PLAYER: u32 = 1;
pub const SF_CAM_FOLLOW_PLAYER: u32 = 2;
pub const SF_CAM_FREEZE: u32 = 4;
pub const SF_CAM_INFINITE_HOLD: u32 = 8;
pub const SF_CAM_SNAP: u32 = 16;
pub const SF_CAM_INTERRUPTABLE: u32 = 64;
/// Angular velocity = error × this × dt (deg/s).
pub const TURN_GAIN: f32 = 40.0;
/// Per-tick path velocity blend: 2·dt.
pub const VEL_BLEND: f32 = 2.0;
/// Velocity damping per tick for a camera that doesn't freeze its player.
pub const FREE_DAMP: f32 = 0.8;
/// Below this (units/s, after damping) the velocity is zeroed.
pub const FREE_STOP: f32 = 10.0;
/// acceleration/deceleration 0 become this (units/s²).
pub const DEFAULT_ACCEL: f32 = 500.0;

/// What a point_viewcontrol tracks.
#[derive(Clone, Debug, Default)]
pub struct Camera {
    pub on: bool,
    pub player: Option<Entity>,
    pub buttons: u32,
    pub return_time: f32,
    pub speed: f32,
    pub target_speed: f32,
    pub stop_time: f32,
    pub accel: f32,
    pub decel: f32,
    /// The path corner it heads for, and how far is left.
    pub corner: Option<EntId>,
    pub remaining: f32,
    pub dir: Vec3,
    pub velocity: Vec3,
    pub avelocity: Vec3,
    pub snap: bool,
    /// Looks at the player (flag 2) or this entity.
    pub look_player: bool,
    pub target: Option<EntId>,
    pub thinking: bool,
}

pub(super) fn spawn(e: &super::world::LogicEntity) -> Camera {
    let or_default = |v: f32| if v == 0.0 { DEFAULT_ACCEL } else { v };
    Camera {
        accel: or_default(e.kv_f("acceleration")),
        decel: or_default(e.kv_f("deceleration")),
        ..default()
    }
}

fn cam(w: &mut LogicWorld, id: EntId) -> Option<&mut Camera> {
    match w.get_mut(id).map(|e| &mut e.class) {
        Some(Class::Camera(c)) => Some(c),
        _ => None,
    }
}

/// Wrap degrees to [-180, 180].
fn wrap180(a: f32) -> f32 {
    (a + 180.0).rem_euclid(360.0) - 180.0
}

pub(super) fn input(w: &mut LogicWorld, id: EntId, input: &str, activator: Option<Who>) -> bool {
    match input {
        "enable" => enable(w, id, activator),
        "disable" => disable(w, id),
        _ => return false,
    }
    true
}

/// Enable (1.1).
fn enable(w: &mut LogicWorld, id: EntId, activator: Option<Who>) {
    let Some(c) = cam(w, id) else { return };
    c.on = true;
    let Some(p) = (match activator {
        Some(Who::Player(p)) if w.player(p).is_some() => Some(p),
        _ => None,
    }) else {
        // Multiplayer: no "local player" stands in.
        w.log.push("point_viewcontrol: Enable without a player activator (nothing in multiplayer)".into());
        return;
    };
    // The player's current camera: another one is disabled first; this
    // one again does nothing more.
    match w.views.iter().find(|(pl, _)| *pl == p).map(|(_, c)| *c) {
        Some(other) if other == id => {
            if let Some(c) = cam(w, id) {
                c.player = Some(p);
            }
            return;
        }
        Some(other) => disable(w, other),
        None => {}
    }
    let Some(e) = w.get(id) else { return };
    let flags = e.spawnflags;
    let wait = e.kv("wait").map_or(10.0, super::value::atof);
    let speed = e.kv_f("speed");
    let target_name = e.kv("target").unwrap_or("").to_string();
    let moveto = e.kv("moveto").unwrap_or("").to_string();
    let now = w.now();
    let pl = w.player(p).cloned().unwrap();
    let target = if flags & SF_CAM_FOLLOW_PLAYER != 0 || target_name.is_empty() {
        None
    } else {
        w.resolve(&target_name, activator, Some(Who::Ent(id))).into_iter().find_map(|t| match t {
            Who::Ent(e) => Some(e),
            Who::Player(_) => None,
        })
    };
    let corner = if moveto.is_empty() {
        None
    } else {
        w.resolve(&moveto, activator, Some(Who::Ent(id))).into_iter().find_map(|t| match t {
            Who::Ent(e) => Some(e),
            Who::Player(_) => None,
        })
    };
    let (corner_speed, corner_wait) = corner
        .and_then(|c| w.get(c))
        .map_or((0.0, 0.0), |c| (c.kv_f("speed"), c.kv_f("wait")));
    let look_player = flags & SF_CAM_FOLLOW_PLAYER != 0;
    {
        let c = cam(w, id).unwrap();
        c.player = Some(p);
        c.buttons = pl.buttons;
        c.return_time = now + wait;
        c.speed = speed;
        c.target_speed = speed;
        c.snap = flags & SF_CAM_SNAP != 0;
        c.look_player = look_player;
        c.target = target;
        c.corner = corner;
        c.remaining = 0.0;
        c.stop_time = 0.0;
        if corner.is_some() {
            if corner_speed != 0.0 {
                c.target_speed = corner_speed;
            }
            c.stop_time = now + corner_wait;
        }
        c.velocity = if flags & SF_CAM_START_AT_PLAYER != 0 { pl.velocity } else { Vec3::ZERO };
    }
    if flags & SF_CAM_START_AT_PLAYER != 0
        && let Some(e) = w.get_mut(id)
    {
        e.origin = pl.origin + pl.eye;
        e.angles = Vec3::new(pl.view.x, pl.view.y, 0.0);
    }
    w.views.retain(|(pl, _)| *pl != p);
    w.views.push((p, id));
    w.effects.push(Effect::ViewControl {
        target: p,
        camera: Some(id),
        freeze: flags & SF_CAM_FREEZE != 0,
    });
    let has_target = look_player || target.is_some();
    if let Some(c) = cam(w, id) {
        c.thinking = has_target;
    }
    if has_target {
        w.think_in(id, w.dt);
    }
    camera_move(w, id);
}

/// Disable (1.3): the player gets its own view back (if alive), and
/// OnEndFollow fires even when the camera was off.
fn disable(w: &mut LogicWorld, id: EntId) {
    let now = w.now();
    let Some(c) = cam(w, id) else { return };
    let player = c.player;
    c.on = false;
    c.return_time = now;
    c.avelocity = Vec3::ZERO;
    c.thinking = false;
    if let Some(e) = w.get_mut(id) {
        e.next_think = None;
    }
    if let Some(p) = player
        && w.views.iter().any(|(pl, c)| *pl == p && *c == id)
    {
        let alive = w.player(p).is_some_and(|pl| pl.alive);
        if alive {
            w.views.retain(|(pl, _)| *pl != p);
            w.effects.push(Effect::ViewControl {
                target: p,
                camera: None,
                freeze: false,
            });
        }
    }
    w.fire_output(id, "OnEndFollow", Some(Who::Ent(id)), Value::Void);
}

/// Each tick while on with a look target (1.2), then the camera moves by
/// its velocities.
pub(super) fn think(w: &mut LogicWorld, id: EntId) {
    let now = w.now();
    let dt = w.dt;
    let Some(c) = cam(w, id).cloned() else { return };
    if !c.on || !c.thinking {
        return;
    }
    let Some(p) = c.player.and_then(|p| w.player(p).cloned()) else {
        return;
    };
    let flags = w.get(id).map_or(0, |e| e.spawnflags);
    let goal_point = if c.look_player {
        Some(p.origin)
    } else {
        c.target.and_then(|t| w.get(t)).map(|t| t.origin)
    };
    let Some(goal_point) = goal_point else {
        disable(w, id);
        return;
    };
    if flags & SF_CAM_INFINITE_HOLD == 0 && now > c.return_time {
        disable(w, id);
        return;
    }
    let Some(e) = w.get(id) else { return };
    let (origin, angles) = (e.origin, e.angles);
    let d = goal_point - origin;
    let goal = Vec3::new(
        (-d.z).atan2(d.truncate().length()).to_degrees(),
        d.y.atan2(d.x).to_degrees(),
        0.0,
    );
    let mut angles = angles;
    let mut c = c;
    if c.snap {
        angles = goal;
        c.snap = false;
    } else {
        angles.y = angles.y.rem_euclid(360.0);
        let dx = wrap180(goal.x - angles.x);
        let dy = wrap180(goal.y - angles.y);
        c.avelocity = Vec3::new(dx * TURN_GAIN * dt, dy * TURN_GAIN * dt, c.avelocity.z);
    }
    if flags & SF_CAM_FREEZE == 0 {
        c.velocity *= FREE_DAMP;
        if c.velocity.length() < FREE_STOP {
            c.velocity = Vec3::ZERO;
        }
    }
    if let Some(e) = w.get_mut(id) {
        e.angles = angles;
    }
    if let Some(cc) = cam(w, id) {
        *cc = c;
    }
    camera_move(w, id);
    // Simulated after its think.
    let Some(c) = cam(w, id).cloned() else { return };
    if let Some(e) = w.get_mut(id) {
        e.origin += c.velocity * dt;
        e.angles += c.avelocity * dt;
    }
    if c.on && c.thinking {
        w.think_in(id, dt);
    }
}

/// Move (1.4): the interrupt check, then the path.
fn camera_move(w: &mut LogicWorld, id: EntId) {
    let now = w.now();
    let dt = w.dt;
    let flags = w.get(id).map_or(0, |e| e.spawnflags);
    let Some(c) = cam(w, id).cloned() else { return };
    let buttons = c.player.and_then(|p| w.player(p)).map_or(0, |p| p.buttons);
    if flags & SF_CAM_INTERRUPTABLE != 0 && buttons != c.buttons && buttons != 0 {
        disable(w, id);
        return;
    }
    let mut c = c;
    c.buttons = buttons;
    if let Some(corner) = c.corner {
        c.remaining -= c.speed * dt;
        if c.remaining <= 0.0 {
            w.fire_output(corner, "OnPass", Some(Who::Ent(id)), Value::Void);
            let next = w
                .get(corner)
                .and_then(|e| e.kv("target"))
                .filter(|t| !t.is_empty())
                .map(str::to_string)
                .and_then(|t| w.find(&t));
            match next.and_then(|n| w.get(n).map(|e| (n, e.origin, e.kv_f("speed"), e.kv_f("wait")))) {
                None => {
                    c.velocity = Vec3::ZERO;
                    c.corner = None;
                }
                Some((n, at, speed, wait)) => {
                    if speed != 0.0 {
                        c.target_speed = speed;
                    }
                    let origin = w.get(id).map_or(Vec3::ZERO, |e| e.origin);
                    c.dir = (at - origin).normalize_or_zero();
                    c.remaining = (at - origin).length();
                    c.stop_time = now + wait;
                    c.corner = Some(n);
                }
            }
        }
        if c.corner.is_some() || c.velocity != Vec3::ZERO {
            let approach = |v: f32, goal: f32, step: f32| {
                if v < goal { (v + step).min(goal) } else { (v - step).max(goal) }
            };
            c.speed = if now < c.stop_time {
                approach(c.speed, 0.0, c.decel * dt)
            } else {
                approach(c.speed, c.target_speed, c.accel * dt)
            };
            let blend = VEL_BLEND * dt;
            c.velocity = c.dir * c.speed * blend + c.velocity * (1.0 - blend);
        }
    }
    if let Some(cc) = cam(w, id) {
        *cc = c;
    }
}

impl LogicWorld {
    /// Cameras that are on but don't think (no look target) still move by
    /// the velocity their one Move gave them.
    pub fn move_cameras(&mut self) {
        let dt = self.dt;
        // A viewer who died keeps the camera's view (Disable doesn't
        // restore the dead) until it spawns again, which resets it
        // (`core::apply_map_controls`; spec open question 2).
        let players = &self.players;
        self.views
            .retain(|(p, _)| players.iter().find(|x| x.entity == *p).is_none_or(|x| x.alive));
        for id in self.ids() {
            let Some(Class::Camera(c)) = self.get(id).map(|e| &e.class) else { continue };
            if !c.on || c.thinking || c.velocity == Vec3::ZERO {
                continue;
            }
            let v = c.velocity;
            if let Some(e) = self.get_mut(id) {
                e.origin += v * dt;
            }
        }
    }

    /// Players viewing through a camera: player, camera origin and angles
    /// (entity space).
    pub fn camera_views(&self) -> Vec<(Entity, Vec3, Vec3)> {
        self.views
            .iter()
            .filter_map(|(p, c)| self.get(*c).map(|e| (*p, e.origin, e.angles)))
            .collect()
    }
}
