//! Moving brushes (specs/source/doors_buttons.md): pushers that carry,
//! push and are blocked by players; func_door, func_door_rotating,
//! func_button, func_movelinear, func_rotating, func_tracktrain with
//! path_track, func_brush toggling; and the +use search.

use bevy::prelude::*;

use super::classes::Class;
use super::triggers::forward;
use super::value::Value;
use super::world::{Collision, Effect, EntId, LogicWorld, SOLID_SKIN, SWEEP_EPS, Who, place_hull};
use crate::map::MapBrush;
use crate::map::entities::entity_rotation;

/// +use reach (doors_buttons.md constants).
pub const USE_RADIUS: f32 = 80.0;
pub const USE_TRACE_LONG: f32 = 1024.0;
pub const USE_TRACE_BOX: f32 = 72.0;
pub const USE_BOX_HALF: f32 = 16.0;
pub const USE_CONE_DOT: f32 = 0.8;
/// Tangents of the angled use traces (45, 30, 20, 15, 10 down; 10, 15 up).
const USE_TANGENTS: [f32; 7] = [1.0, 0.57735, 0.36397, 0.26795, 0.17633, -0.17633, -0.26795];

/// What a pusher does when its move (or wait) is done.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub enum Done {
    #[default]
    None,
    DoorOpened,
    DoorWaited,
    DoorClosed,
    ButtonIn,
    ButtonOut,
    LinearArrived,
    MomentaryArrived,
    RotStep,
    TrainArrived,
    PropDoorOpened,
    PropDoorClosed,
}

/// A moving brush's motion state (entity space; angles in degrees).
#[derive(Clone, Debug, Default)]
pub struct Pusher {
    pub origin: Vec3,
    pub angles: Vec3,
    pub velocity: Vec3,
    pub avelocity: Vec3,
    /// Local time: advances only on successful steps.
    pub ltime: f64,
    pub move_done: Option<f64>,
    pub goal_origin: Option<Vec3>,
    pub goal_angles: Option<Vec3>,
    pub on_done: Done,
    /// Solid to players.
    pub solid: bool,
    pub visible: bool,
    /// Players are moved through rather than blocking (trains flag 512).
    pub unblockable: bool,
    /// Steps stop exactly at the move-done time (moves and waits); false
    /// for func_rotating's spin steps, which land on whole ticks.
    pub inexact: bool,
    /// When the last move or wait was due (local time).
    pub last_done: f64,
    pub blocker: Option<Who>,
    /// The velocity of the mover carrying it (a parent that moves it
    /// along, `anchors::follow_anchors`), added to its own for riders.
    pub carry: Vec3,
}

impl Pusher {
    pub(super) fn at(origin: Vec3, angles: Vec3) -> Self {
        Self {
            origin,
            angles,
            solid: true,
            visible: true,
            ..default()
        }
    }

    /// Linear move to `goal` at `speed`.
    pub(super) fn move_to(&mut self, goal: Vec3, speed: f32, done: Done) {
        self.on_done = done;
        self.goal_angles = None;
        self.avelocity = Vec3::ZERO;
        let d = goal - self.origin;
        let len = d.length();
        if len < 1e-4 {
            self.velocity = Vec3::ZERO;
            self.goal_origin = Some(goal);
            self.move_done = Some(self.ltime);
            return;
        }
        if speed <= 0.0 {
            // T = distance / 0 never ends: it holds still with its goal
            // kept (func_movelinear's SetSpeed 0 pauses a lift; a later
            // SetSpeed restarts the move).
            self.velocity = Vec3::ZERO;
            self.goal_origin = Some(goal);
            self.move_done = None;
            return;
        }
        let t = len / speed;
        self.velocity = d / t;
        self.goal_origin = Some(goal);
        self.move_done = Some(self.ltime + t as f64);
    }

    /// Angular move to `goal` angles at `speed` degrees per second.
    pub(super) fn rotate_to(&mut self, goal: Vec3, speed: f32, done: Done) {
        self.on_done = done;
        self.goal_origin = None;
        self.velocity = Vec3::ZERO;
        let d = goal - self.angles;
        let t = (d.length() / speed.max(1e-3)).max(0.01);
        self.avelocity = d / t;
        self.goal_angles = Some(goal);
        self.move_done = Some(self.ltime + t as f64);
    }

    /// Stand still until `wait` seconds of local time pass.
    pub(super) fn wait(&mut self, wait: f32, done: Done) {
        self.velocity = Vec3::ZERO;
        self.avelocity = Vec3::ZERO;
        self.goal_origin = None;
        self.goal_angles = None;
        self.on_done = done;
        self.move_done = Some(self.ltime + wait as f64);
    }

    pub(super) fn moving(&self) -> bool {
        self.velocity != Vec3::ZERO || self.avelocity != Vec3::ZERO
    }
}

/// The local bounding box of a set of hulls.
fn hull_bounds(hulls: &[crate::map::MapHull]) -> (Vec3, Vec3) {
    let mut lo = Vec3::MAX;
    let mut hi = Vec3::MIN;
    for p in hulls.iter().flat_map(|h| &h.points) {
        lo = lo.min(*p);
        hi = hi.max(*p);
    }
    if lo.x > hi.x { (Vec3::ZERO, Vec3::ZERO) } else { (lo, hi) }
}

/// Door-style travel along `dir`: the brush size minus 2 minus lip.
fn travel(hulls: &[crate::map::MapHull], dir: Vec3, lip: f32) -> f32 {
    let (lo, hi) = hull_bounds(hulls);
    let size = hi - lo;
    dir.x.abs() * (size.x - 2.0) + dir.y.abs() * (size.y - 2.0) + dir.z.abs() * (size.z - 2.0) - lip
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum DoorState {
    Closed,
    Opening,
    Open,
    Closing,
}

#[derive(Clone, Debug)]
pub struct Door {
    pub push: Pusher,
    pub rotating: bool,
    pub state: DoorState,
    /// Closed and open positions (linear) or angles (rotating).
    pub p1: Vec3,
    pub p2: Vec3,
    /// Rotation axis in angle space (pitch, yaw, roll), signed.
    pub axis: Vec3,
    pub distance: f32,
    pub speed: f32,
    pub wait: f32,
    pub dmg: f32,
    pub forceclosed: bool,
    pub locked: bool,
    pub last_user: Option<Who>,
    pub chain: String,
    pub move_sound: String,
    pub arrive_sound: String,
    pub locked_sound: String,
}

impl Door {
    pub(super) fn spawn(w: &mut LogicWorld, id: EntId) -> Door {
        let e = w.get(id).unwrap();
        let rotating = e.classname.eq_ignore_ascii_case("func_door_rotating");
        let speed = match e.kv_f("speed") {
            s if s == 0.0 => 100.0,
            s => s,
        };
        let flags = e.spawnflags;
        let mut push = Pusher::at(e.origin, e.angles);
        push.solid = flags & (4 | 8) == 0;
        let (p1, p2, axis, distance) = if rotating {
            let mut axis = if flags & 64 != 0 {
                Vec3::Z
            } else if flags & 128 != 0 {
                Vec3::X
            } else {
                Vec3::Y
            };
            if flags & 2 != 0 {
                axis = -axis;
            }
            let distance = e.kv_f("distance");
            (e.angles, e.angles + axis * distance, axis, distance)
        } else {
            let dir = forward(e.kv("movedir").map_or(Vec3::ZERO, crate::map::entities::parse_vector));
            let d = travel(&e.hulls, dir, e.kv_f("lip"));
            (e.origin, e.origin + dir * d, Vec3::ZERO, d)
        };
        let prefix = if rotating { "RotDoorSound" } else { "DoorSound" };
        let sound = |k: &str, d: &str| e.kv(k).filter(|s| !s.is_empty()).unwrap_or(d).to_string();
        let mut door = Door {
            push,
            rotating,
            state: DoorState::Closed,
            p1,
            p2,
            axis,
            distance,
            speed,
            wait: e.kv("wait").map_or(0.0, super::value::atof),
            dmg: e.kv_f("dmg"),
            forceclosed: e.kv_i("forceclosed") != 0,
            locked: flags & 2048 != 0,
            last_user: None,
            chain: e.kv("chainstodoor").unwrap_or("").to_string(),
            move_sound: sound("noise1", &format!("{prefix}.DefaultMove")),
            arrive_sound: sound("noise2", &format!("{prefix}.DefaultArrive")),
            locked_sound: sound("locked_sound", &format!("{prefix}.DefaultLocked")),
        };
        if e.kv_i("spawnpos") == 1 || flags & 1 != 0 {
            door.state = DoorState::Open;
            if rotating {
                door.push.angles = p2;
            } else {
                door.push.origin = p2;
            }
        }
        door
    }

    /// Open angles: away from the user for yaw doors (doors_buttons.md,
    /// "Opening away from the user").
    fn open_angles(&self, w: &LogicWorld, flags: u32, hulls: &[crate::map::MapHull]) -> Vec3 {
        if self.axis.y == 0.0 || flags & 16 != 0 {
            return self.p2;
        }
        let Some(Who::Player(p)) = self.last_user else { return self.p2 };
        let Some(user) = w.player(p).map(|p| p.origin) else {
            return self.p2;
        };
        // The point of the door's box nearest the user.
        let rot = entity_rotation(self.push.angles);
        let mut lo = Vec3::MAX;
        let mut hi = Vec3::MIN;
        for pt in hulls.iter().flat_map(|h| &h.points) {
            let q = rot * *pt + self.push.origin;
            lo = lo.min(q);
            hi = hi.max(q);
        }
        let nearest = user.clamp(lo, hi);
        let a = (self.push.origin - user).truncate();
        let b = (nearest - user).truncate();
        if a.perp_dot(b) > 0.0 {
            self.p1 - self.axis * self.distance
        } else {
            self.p2
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ButtonState {
    Out,
    GoingIn,
    In,
    GoingOut,
}

#[derive(Clone, Debug)]
pub struct Button {
    pub push: Pusher,
    pub state: ButtonState,
    /// Out and in positions, or angles for func_rot_button.
    pub p1: Vec3,
    pub p2: Vec3,
    /// func_rot_button: turns between angles instead of moving.
    pub rotating: bool,
    pub speed: f32,
    pub wait: f32,
    pub locked: bool,
    pub presser: Option<Who>,
    pub last_use_locked: i64,
    pub sound: String,
}

impl Button {
    pub(super) fn spawn(w: &mut LogicWorld, id: EntId) -> Button {
        let e = w.get(id).unwrap();
        let speed = match e.kv_f("speed") {
            s if s == 0.0 => 40.0,
            s => s,
        };
        let lip = match e.kv_f("lip") {
            l if l == 0.0 => 4.0,
            l => l,
        };
        let wait = match e.kv("wait").map_or(0.0, super::value::atof) {
            x if x == 0.0 => 1.0,
            x => x,
        };
        let rotating = e.classname.eq_ignore_ascii_case("func_rot_button");
        let dir = forward(e.kv("movedir").map_or(Vec3::ZERO, crate::map::entities::parse_vector));
        let mut p2 = e.origin + dir * travel(&e.hulls, dir, lip);
        if (p2 - e.origin).length() < 1.0 || e.has_flag(1) {
            p2 = e.origin;
        }
        let mut push = Pusher::at(e.origin, e.angles);
        let (p1, p2) = if rotating {
            // A rotating button turns "distance" degrees about its axis
            // (the rotating door's flags); its flag 1 is "Not solid".
            push.solid = !e.has_flag(1);
            (e.angles, e.angles + rot_axis(e.spawnflags) * e.kv_f("distance"))
        } else {
            (e.origin, p2)
        };
        let sounds = e.kv_i("sounds");
        Button {
            push,
            state: ButtonState::Out,
            p1,
            p2,
            rotating,
            speed,
            wait,
            locked: e.has_flag(2048),
            presser: None,
            last_use_locked: i64::MIN,
            sound: if sounds > 0 { format!("Buttons.snd{sounds}") } else { String::new() },
        }
    }
}

#[derive(Clone, Debug)]
pub struct MoveLinear {
    pub push: Pusher,
    pub p1: Vec3,
    pub p2: Vec3,
    pub speed: f32,
    pub block_damage: f32,
    pub goal: Vec3,
}

impl MoveLinear {
    pub(super) fn spawn(w: &mut LogicWorld, id: EntId) -> MoveLinear {
        let e = w.get(id).unwrap();
        let dir = forward(e.kv("movedir").map_or(Vec3::ZERO, crate::map::entities::parse_vector));
        let mut d = e.kv_f("MoveDistance");
        if d <= 0.0 {
            d = travel(&e.hulls, dir, e.kv_f("lip"));
        }
        let speed = if e.kv_f("speed") <= 0.0 { 100.0 } else { e.kv_f("speed") };
        let p1 = e.origin - dir * d * e.kv_f("StartPosition");
        let mut push = Pusher::at(e.origin, e.angles);
        push.solid = !e.has_flag(8);
        MoveLinear {
            push,
            p1,
            p2: p1 + dir * d,
            speed,
            block_damage: e.kv_f("BlockDamage"),
            goal: e.origin,
        }
    }
}

#[derive(Clone, Debug)]
pub struct Rotating {
    pub push: Pusher,
    pub axis: Vec3,
    pub maxspeed: f32,
    pub friction: f32,
    /// Signed current and target speeds, degrees per second.
    pub speed: f32,
    pub target: f32,
    pub accdcc: bool,
    pub dmg: f32,
    /// Direction: +1 or -1 (Reverse flips it).
    pub dir: f32,
}

impl Rotating {
    pub(super) fn spawn(w: &mut LogicWorld, id: EntId) -> Rotating {
        let e = w.get(id).unwrap();
        let flags = e.spawnflags;
        let axis = if flags & 4 != 0 {
            Vec3::Z
        } else if flags & 8 != 0 {
            Vec3::X
        } else {
            Vec3::Y
        };
        let maxspeed = match e.kv_f("maxspeed").abs() {
            s if s == 0.0 => 100.0,
            s => s,
        };
        let friction = match e.kv_f("fanfriction") {
            f if f == 0.0 => 1.0,
            f => f / 100.0,
        };
        let mut push = Pusher::at(e.origin, e.angles);
        push.solid = flags & 64 == 0;
        Rotating {
            push,
            axis,
            maxspeed,
            friction,
            speed: 0.0,
            target: 0.0,
            accdcc: flags & 16 != 0,
            dmg: e.kv_f("dmg"),
            dir: if flags & 2 != 0 { -1.0 } else { 1.0 },
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct PathTrack {
    pub next: Option<EntId>,
    pub prev: Option<EntId>,
    pub alt: Option<EntId>,
    pub enabled: bool,
    pub alt_enabled: bool,
}

impl PathTrack {
    pub(super) fn spawn(w: &mut LogicWorld, id: EntId) -> PathTrack {
        let e = w.get(id).unwrap();
        PathTrack {
            enabled: !e.has_flag(1),
            ..default()
        }
    }
}

#[derive(Clone, Debug)]
pub struct Train {
    pub push: Pusher,
    pub height: f32,
    /// Signed speed (units/s) and max speed.
    pub speed: f32,
    pub maxspeed: f32,
    pub old_speed: f32,
    pub dmg: f32,
    /// The node the train last passed (behind its look-ahead point).
    pub node: Option<EntId>,
    pub user_control: bool,
    pub started: bool,
}

impl Train {
    pub(super) fn spawn(w: &mut LogicWorld, id: EntId) -> Train {
        let e = w.get(id).unwrap();
        let speed = e.kv_f("speed");
        let mut maxspeed = e.kv_f("startspeed");
        if maxspeed == 0.0 {
            maxspeed = if speed == 0.0 { 100.0 } else { speed.abs() };
        }
        let mut push = Pusher::at(e.origin, e.angles);
        push.solid = !e.has_flag(8);
        push.unblockable = e.has_flag(512);
        Train {
            push,
            height: e.kv_f("height"),
            speed,
            maxspeed,
            old_speed: 0.0,
            dmg: e.kv_f("dmg"),
            node: None,
            user_control: !e.has_flag(2),
            started: false,
        }
    }
}

/// func_brush: shown and solid while enabled.
#[derive(Clone, Debug)]
pub struct Toggle {
    pub push: Pusher,
    pub enabled: bool,
    /// 0 toggles with the brush, 1 never solid, 2 always solid.
    pub solidity: i32,
}

impl Toggle {
    pub(super) fn spawn_brush(w: &mut LogicWorld, id: EntId) -> Toggle {
        let e = w.get(id).unwrap();
        let mut t = Toggle {
            push: Pusher::at(e.origin, e.angles),
            enabled: e.kv_i("StartDisabled") == 0,
            solidity: e.kv_i("Solidity"),
        };
        t.apply();
        t
    }

    /// func_wall_toggle (specs/source/game_entities.md 7): a wall shown
    /// and solid while on; spawnflag 1 starts it off; Toggle (or Use)
    /// flips it.
    pub(super) fn spawn_wall_toggle(w: &mut LogicWorld, id: EntId) -> Toggle {
        let e = w.get(id).unwrap();
        let mut t = Toggle {
            push: Pusher::at(e.origin, e.angles),
            enabled: !e.has_flag(1),
            solidity: 0,
        };
        t.apply();
        t
    }

    fn apply(&mut self) {
        self.push.visible = self.enabled;
        self.push.solid = match self.solidity {
            1 => false,
            2 => true,
            _ => self.enabled,
        };
    }
}

/// A brush entity parented to a mover: keeps its offset from the parent
/// as the parent moves and turns (it does not push players itself).
#[derive(Clone, Debug)]
pub struct Attached {
    pub push: Pusher,
    pub parent: Option<EntId>,
    /// Origin and angles relative to the parent at activation.
    pub offset: Vec3,
    pub base_angles: Vec3,
    pub parent_angles: Vec3,
}

impl Attached {
    pub(super) fn spawn(w: &mut LogicWorld, id: EntId) -> Attached {
        let e = w.get(id).unwrap();
        Attached {
            push: Pusher::at(e.origin, e.angles),
            parent: None,
            offset: Vec3::ZERO,
            base_angles: e.angles,
            parent_angles: Vec3::ZERO,
        }
    }
}

pub(super) fn activate_attached(w: &mut LogicWorld, id: EntId) {
    let name = w.get(id).and_then(|e| e.kv("parentname")).unwrap_or("").to_string();
    let parent = w.find(&name).filter(|p| w.get(*p).and_then(|e| pusher(&e.class)).is_some());
    let Some(p) = parent else {
        // Placed weapons and physics brushes move it (`anchors`).
        let anchored = w.find(&name).and_then(|p| w.get(p)).is_some_and(|p| {
            crate::map::entities::anchor_class(&p.classname) || super::prop_damage::is_physbox(&p.classname)
        });
        if !anchored {
            w.log.push(format!("parented brush: no moving parent '{name}'"));
        }
        w.refresh_solid(id);
        return;
    };
    let pp = pusher(&w.get(p).unwrap().class).unwrap().clone();
    if let Some(a) = w.get_mut(id).and_then(|e| attached_mut(&mut e.class)) {
        a.parent = Some(p);
        a.offset = entity_rotation(pp.angles).inverse() * (a.push.origin - pp.origin);
        a.parent_angles = pp.angles;
    }
    w.refresh_solid(id);
}

/// The parent link of a parented brush (or breakable).
pub fn attached(class: &Class) -> Option<&Attached> {
    match class {
        Class::Attached(a) => Some(a),
        Class::Breakable(b) => Some(&b.attach),
        _ => None,
    }
}

fn attached_mut(class: &mut Class) -> Option<&mut Attached> {
    match class {
        Class::Attached(a) => Some(a),
        Class::Breakable(b) => Some(&mut b.attach),
        _ => None,
    }
}

/// Carry a mover by `delta` (its parent moved, `anchors::
/// follow_anchors`): its place, goal and track move along; `carry` is
/// the parent's velocity. False for entities that aren't movers.
pub(super) fn shift(class: &mut Class, delta: Vec3, carry: Vec3) -> bool {
    let push = match class {
        Class::Door(d) => {
            if !d.rotating {
                d.p1 += delta;
                d.p2 += delta;
            }
            &mut d.push
        }
        Class::Button(b) => {
            if !b.rotating {
                b.p1 += delta;
                b.p2 += delta;
            }
            &mut b.push
        }
        Class::Momentary(m) => &mut m.push,
        Class::MoveLinear(m) => {
            m.p1 += delta;
            m.p2 += delta;
            m.goal += delta;
            &mut m.push
        }
        Class::Rotating(r) => &mut r.push,
        Class::Train(t) => &mut t.push,
        Class::Brush(t) => &mut t.push,
        Class::Conveyor(c) => &mut c.push,
        Class::PropDoor(d) => &mut d.push,
        _ => return false,
    };
    push.origin += delta;
    push.goal_origin = push.goal_origin.map(|g| g + delta);
    push.carry = carry;
    true
}

/// The angle-space axis of a rotating door or button: roll with flag 64,
/// pitch with 128, else yaw; flag 2 reverses it.
fn rot_axis(flags: u32) -> Vec3 {
    let axis = if flags & 64 != 0 {
        Vec3::Z
    } else if flags & 128 != 0 {
        Vec3::X
    } else {
        Vec3::Y
    };
    if flags & 2 != 0 { -axis } else { axis }
}

/// momentary_rot_button (public entity docs; no spec): turned between
/// its start angles (position 0) and "distance" degrees about its axis
/// (position 1) by SetPosition at "speed" deg/s, or at once by
/// SetPositionImmediately; arriving fires Position (the position),
/// OnReachedPosition and OnFullyOpen/OnFullyClosed at 1/0. Players can't
/// turn it (+use) here.
#[derive(Clone, Debug)]
pub struct Momentary {
    pub push: Pusher,
    pub start: Vec3,
    pub axis: Vec3,
    pub distance: f32,
    pub speed: f32,
    pub locked: bool,
}

impl Momentary {
    pub(super) fn spawn(w: &mut LogicWorld, id: EntId) -> Momentary {
        let e = w.get(id).unwrap();
        let axis = rot_axis(e.spawnflags);
        let distance = e.kv_f("distance");
        let mut push = Pusher::at(e.origin, e.angles);
        push.solid = !e.has_flag(1);
        push.angles = e.angles + axis * distance * e.kv_f("startposition").clamp(0.0, 1.0);
        Momentary {
            push,
            start: e.angles,
            axis,
            distance,
            speed: match e.kv_f("speed") {
                s if s <= 0.0 => 100.0,
                s => s,
            },
            locked: e.has_flag(2048),
        }
    }

    /// Where it is between its ends (0..1).
    pub fn position(&self) -> f32 {
        if self.distance == 0.0 {
            return 0.0;
        }
        ((self.push.angles - self.start).dot(self.axis) / self.distance).clamp(0.0, 1.0)
    }

    fn angles_at(&self, f: f32) -> Vec3 {
        self.start + self.axis * self.distance * f.clamp(0.0, 1.0)
    }
}

/// The pusher of a mover entity.
pub fn pusher(class: &Class) -> Option<&Pusher> {
    match class {
        Class::Attached(a) => Some(&a.push),
        Class::Breakable(b) => Some(&b.attach.push),
        Class::Door(d) => Some(&d.push),
        Class::Button(b) => Some(&b.push),
        Class::MoveLinear(m) => Some(&m.push),
        Class::Rotating(r) => Some(&r.push),
        Class::Train(t) => Some(&t.push),
        Class::Brush(t) => Some(&t.push),
        Class::PropDoor(d) => Some(&d.push),
        Class::Conveyor(c) => Some(&c.push),
        Class::Momentary(m) => Some(&m.push),
        _ => None,
    }
}

pub(super) fn pusher_mut(class: &mut Class) -> Option<&mut Pusher> {
    match class {
        Class::Attached(a) => Some(&mut a.push),
        Class::Breakable(b) => Some(&mut b.attach.push),
        Class::Door(d) => Some(&mut d.push),
        Class::Button(b) => Some(&mut b.push),
        Class::MoveLinear(m) => Some(&mut m.push),
        Class::Rotating(r) => Some(&mut r.push),
        Class::Train(t) => Some(&mut t.push),
        Class::Brush(t) => Some(&mut t.push),
        Class::PropDoor(d) => Some(&mut d.push),
        Class::Conveyor(c) => Some(&mut c.push),
        Class::Momentary(m) => Some(&mut m.push),
        _ => None,
    }
}

fn push_of(w: &mut LogicWorld, id: EntId) -> Option<&mut Pusher> {
    w.get_mut(id).and_then(|e| pusher_mut(&mut e.class))
}

fn door(w: &mut LogicWorld, id: EntId) -> Option<&mut Door> {
    match w.get_mut(id).map(|e| &mut e.class) {
        Some(Class::Door(d)) => Some(d),
        _ => None,
    }
}

fn button(w: &mut LogicWorld, id: EntId) -> Option<&mut Button> {
    match w.get_mut(id).map(|e| &mut e.class) {
        Some(Class::Button(b)) => Some(b),
        _ => None,
    }
}

fn rotating(w: &mut LogicWorld, id: EntId) -> Option<&mut Rotating> {
    match w.get_mut(id).map(|e| &mut e.class) {
        Some(Class::Rotating(r)) => Some(r),
        _ => None,
    }
}

fn train(w: &mut LogicWorld, id: EntId) -> Option<&mut Train> {
    match w.get_mut(id).map(|e| &mut e.class) {
        Some(Class::Train(t)) => Some(t),
        _ => None,
    }
}

fn path(w: &LogicWorld, id: EntId) -> Option<&PathTrack> {
    match w.get(id).map(|e| &e.class) {
        Some(Class::PathTrack(p)) => Some(p),
        _ => None,
    }
}

fn sound(w: &mut LogicWorld, id: EntId, entry: &str) {
    if entry.is_empty() {
        return;
    }
    let at = w.get(id).and_then(|e| pusher(&e.class)).map_or(Vec3::ZERO, |p| p.origin);
    w.effects.push(Effect::Sound {
        entry: entry.to_string(),
        at,
        volume: None,
        pitch: None,
    });
}

// ------------------------------------------------------------ doors

fn door_open(w: &mut LogicWorld, id: EntId) {
    let flags = w.get(id).map_or(0, |e| e.spawnflags);
    let hulls = w.get(id).map(|e| e.hulls.clone()).unwrap_or_default();
    let Some(d) = door(w, id).cloned() else { return };
    let silent = flags & 4096 != 0;
    if !silent && !d.push.moving() {
        sound(w, id, &d.move_sound);
    }
    let target = if d.rotating { d.open_angles(w, flags, &hulls) } else { d.p2 };
    let dd = door(w, id).unwrap();
    dd.state = DoorState::Opening;
    if dd.rotating {
        dd.push.rotate_to(target, dd.speed, Done::DoorOpened);
    } else {
        dd.push.move_to(target, dd.speed, Done::DoorOpened);
    }
    w.fire_output(id, "OnOpen", Some(Who::Ent(id)), Value::Void);
}

fn door_close(w: &mut LogicWorld, id: EntId) {
    let flags = w.get(id).map_or(0, |e| e.spawnflags);
    let Some(d) = door(w, id) else { return };
    let moving = d.push.moving();
    let snd = d.move_sound.clone();
    d.state = DoorState::Closing;
    let (p1, speed) = (d.p1, d.speed);
    if d.rotating {
        d.push.rotate_to(p1, speed, Done::DoorClosed);
    } else {
        d.push.move_to(p1, speed, Done::DoorClosed);
    }
    if flags & 4096 == 0 && !moving {
        sound(w, id, &snd);
    }
    w.fire_output(id, "OnClose", Some(Who::Ent(id)), Value::Void);
}

/// A player (or the chain) activates the door.
fn door_activate(w: &mut LogicWorld, id: EntId, user: Option<Who>, chain: bool) {
    let flags = w.get(id).map_or(0, |e| e.spawnflags);
    let Some(d) = door(w, id) else { return };
    if user.is_some() {
        d.last_user = user;
    }
    let state = d.state;
    let chain_name = d.chain.clone();
    if flags & 32 != 0 && state == DoorState::Open {
        door_close(w, id);
    } else if state != DoorState::Open && state != DoorState::Opening {
        door_open(w, id);
    }
    if chain && !chain_name.is_empty() {
        for other in w.resolve(&chain_name, user, Some(Who::Ent(id))) {
            if let Who::Ent(o) = other
                && o != id
                && door(w, o).is_some()
            {
                door_use_or_touch(w, o, user, false);
            }
        }
    }
}

/// Use or touch rules (state and lock), then activate.
fn door_use_or_touch(w: &mut LogicWorld, id: EntId, user: Option<Who>, chain: bool) {
    let flags = w.get(id).map_or(0, |e| e.spawnflags);
    let Some(d) = door(w, id).cloned() else { return };
    let allowed = d.state == DoorState::Closed
        || (flags & 65536 != 0 && d.state == DoorState::Closing)
        || (flags & 32 != 0 && d.state == DoorState::Open);
    if !allowed {
        return;
    }
    if d.locked {
        w.fire_output(id, "OnLockedUse", user, Value::Void);
        sound(w, id, &d.locked_sound);
        return;
    }
    door_activate(w, id, user, chain);
}

fn door_blocked(w: &mut LogicWorld, id: EntId, blocker: Who, started: bool) {
    let Some(d) = door(w, id).cloned() else { return };
    if started {
        let out = if d.state == DoorState::Opening {
            "OnBlockedOpening"
        } else {
            "OnBlockedClosing"
        };
        w.fire_output(id, out, Some(blocker), Value::Void);
    }
    if d.dmg != 0.0
        && let Who::Player(p) = blocker
    {
        w.effects.push(Effect::Damage {
            target: p,
            amount: d.dmg,
            crush: true,
        });
    }
    if d.forceclosed {
        return;
    }
    if d.wait >= 0.0 {
        match d.state {
            DoorState::Closing => door_open(w, id),
            DoorState::Opening => door_close(w, id),
            _ => {}
        }
    }
}

// ---------------------------------------------------------- buttons

fn button_press(w: &mut LogicWorld, id: EntId, presser: Option<Who>, by_use: bool) {
    let toggle = w.get(id).is_some_and(|e| e.has_flag(32));
    let Some(b) = button(w, id).cloned() else { return };
    if matches!(b.state, ButtonState::GoingIn | ButtonState::GoingOut) {
        return;
    }
    if b.locked {
        sound(w, id, "Buttons.snd10");
        if by_use && w.tick.saturating_sub(b.last_use_locked) >= w.think_ticks(0.5) {
            button(w, id).unwrap().last_use_locked = w.tick;
            w.fire_output(id, "OnUseLocked", presser, Value::Void);
        }
        return;
    }
    match b.state {
        ButtonState::Out => {
            button(w, id).unwrap().presser = presser;
            w.fire_output(id, "OnPressed", presser, Value::Void);
            sound(w, id, &b.sound);
            let bb = button(w, id).unwrap();
            bb.state = ButtonState::GoingIn;
            let (p2, speed) = (bb.p2, bb.speed);
            if bb.rotating {
                bb.push.rotate_to(p2, speed, Done::ButtonIn);
            } else {
                bb.push.move_to(p2, speed, Done::ButtonIn);
            }
        }
        ButtonState::In if toggle => {
            button(w, id).unwrap().presser = presser;
            sound(w, id, &b.sound);
            w.fire_output(id, "OnPressed", presser, Value::Void);
            button_out(w, id);
        }
        _ => {}
    }
}

fn button_out(w: &mut LogicWorld, id: EntId) {
    let Some(b) = button(w, id) else { return };
    b.state = ButtonState::GoingOut;
    let (p1, speed) = (b.p1, b.speed);
    if b.rotating {
        b.push.rotate_to(p1, speed, Done::ButtonOut);
    } else {
        b.push.move_to(p1, speed, Done::ButtonOut);
    }
}

// --------------------------------------------------------- rotating

fn rot_set_target(w: &mut LogicWorld, id: EntId, target: f32) {
    let Some(r) = rotating(w, id) else { return };
    r.target = target;
    if !r.accdcc {
        r.speed = target;
        r.push.avelocity = r.axis * target;
        r.push.move_done = None;
    } else if r.push.move_done.is_none() {
        r.push.inexact = true;
        r.push.wait(0.1, Done::RotStep);
        r.push.avelocity = r.axis * r.speed;
    }
}

fn rot_step(w: &mut LogicWorld, id: EntId) {
    let Some(r) = rotating(w, id) else { return };
    let up = 0.2 * r.maxspeed * r.friction;
    let down = 0.1 * r.maxspeed * r.friction;
    let (s, t) = (r.speed, r.target);
    let new = if s == t {
        s
    } else if t != 0.0 && (s == 0.0 || s.signum() == t.signum()) && s.abs() < t.abs() {
        // Spin up (same direction).
        (s.abs() + up).min(t.abs()) * t.signum()
    } else {
        // Spin down (towards 0, or to 0 before reversing).
        let toward = if t != 0.0 && s.signum() == t.signum() { t.abs() } else { 0.0 };
        (s.abs() - down).max(toward) * s.signum()
    };
    let new = if (new - t).abs() < 1e-3 { t } else { new };
    r.speed = new;
    r.push.avelocity = r.axis * new;
    r.push.inexact = true;
    if new != t {
        // Steps every 0.1 s of local time from the last one.
        let av = r.push.avelocity;
        let next = r.push.last_done + 0.1;
        r.push.wait(0.1, Done::RotStep);
        r.push.move_done = Some(next);
        r.push.avelocity = av;
    } else {
        r.push.move_done = None;
    }
}

// ----------------------------------------------------------- trains

/// Next node from `node` going forward (or backward).
fn path_next(w: &LogicWorld, node: EntId, forward: bool) -> Option<EntId> {
    let p = path(w, node)?;
    let reverse = w.get(node).is_some_and(|e| e.has_flag(4));
    let next = if forward {
        if p.alt_enabled && !reverse { p.alt } else { p.next }
    } else if p.alt_enabled && reverse {
        p.alt
    } else {
        p.prev
    };
    next.filter(|n| path(w, *n).is_some_and(|p| p.enabled))
}

fn train_place_at_start(w: &mut LogicWorld, id: EntId) {
    let first = w.get(id).and_then(|e| e.kv("target")).map(String::from).unwrap_or_default();
    let Some(node) = w.find(&first) else {
        w.log.push(format!("func_tracktrain: no path_track '{first}'"));
        return;
    };
    let origin = w.get(node).unwrap().origin;
    let Some(t) = train(w, id) else { return };
    t.push.origin = origin + Vec3::Z * t.height;
    t.node = Some(node);
    t.started = true;
    let speed = t.speed;
    if speed != 0.0 {
        w.think_in(id, 0.1);
    }
    t_set_speed(w, id, 0.0);
    if let Some(t) = train(w, id) {
        t.old_speed = speed;
    }
}

fn t_set_speed(w: &mut LogicWorld, id: EntId, speed: f32) {
    let Some(t) = train(w, id) else { return };
    let was = t.speed;
    t.speed = speed.clamp(-t.maxspeed, t.maxspeed);
    if speed == 0.0 {
        t.push.velocity = Vec3::ZERO;
        t.push.move_done = None;
    }
    if was == 0.0 && speed != 0.0 {
        w.fire_output(id, "OnStart", None, Value::Void);
    }
}

/// Per tick: aim at the look-ahead point; passing nodes fire OnPass.
fn train_update(w: &mut LogicWorld, id: EntId) {
    let Some(t) = train(w, id).cloned() else { return };
    if !t.started || t.speed == 0.0 || t.push.goal_origin.is_some() {
        return;
    }
    let Some(mut node) = t.node else { return };
    let fwd = t.speed > 0.0;
    let q = t.push.origin - Vec3::Z * t.height;
    let mut left = t.speed.abs() * 0.1;
    let mut at = q;
    loop {
        let Some(next) = path_next(w, node, fwd) else {
            // Dead end: head for the last node exactly and stop there.
            let end = w.get(node).unwrap().origin + Vec3::Z * t.height;
            let speed = t.speed.abs();
            let node_now = node;
            let tr = train(w, id).unwrap();
            tr.node = Some(node_now);
            tr.push.move_to(end, speed, Done::TrainArrived);
            return;
        };
        let p = w.get(next).unwrap().origin;
        let seg = p - at;
        if seg.length() > left {
            at += seg.normalize() * left;
            break;
        }
        left -= seg.length();
        at = p;
        node = next;
        // Passing a node.
        train(w, id).unwrap().node = Some(node);
        w.fire_output(node, "OnPass", Some(Who::Ent(id)), Value::Void);
        let nspeed = w.get(node).unwrap().kv_f("speed");
        let tr = train(w, id).unwrap();
        if !tr.user_control && nspeed != 0.0 {
            let s = nspeed.copysign(tr.speed);
            tr.speed = s.clamp(-tr.maxspeed, tr.maxspeed);
        }
        if w.get(node).is_some_and(|e| e.has_flag(8)) {
            train(w, id).unwrap().user_control = false;
        }
    }
    let tr = train(w, id).unwrap();
    let aim = at + Vec3::Z * tr.height - tr.push.origin;
    tr.push.velocity = aim.normalize_or_zero() * tr.speed.abs();
    w.fire_output(id, "OnNextPoint", None, Value::Void);
}

// ------------------------------------------------------- dispatching

pub(super) fn activate(w: &mut LogicWorld, id: EntId) {
    let Some(e) = w.get(id) else { return };
    let flags = e.spawnflags;
    match &e.class {
        Class::Rotating(_) if flags & 1 != 0 => w.think_in(id, 0.2),
        Class::Train(_) => w.think_in(id, 0.0),
        _ => {}
    }
    w.refresh_solid(id);
}

pub(super) fn activate_path(w: &mut LogicWorld, id: EntId) {
    let Some(e) = w.get(id) else { return };
    let target = e.kv("target").unwrap_or("").to_string();
    let alt = e.kv("altpath").unwrap_or("").to_string();
    let next = (!target.is_empty())
        .then(|| w.find(&target))
        .flatten()
        .filter(|n| path(w, *n).is_some());
    let alt = (!alt.is_empty()).then(|| w.find(&alt)).flatten().filter(|n| path(w, *n).is_some());
    if let Some(Class::PathTrack(p)) = w.get_mut(id).map(|e| &mut e.class) {
        p.next = next;
        p.alt = alt;
    }
    if let Some(n) = next
        && let Some(Class::PathTrack(p)) = w.get_mut(n).map(|e| &mut e.class)
        && p.alt != Some(id)
    {
        p.prev = Some(id);
    }
}

pub(super) fn think(w: &mut LogicWorld, id: EntId) {
    let Some(class) = w.get(id).map(|e| e.class.clone()) else { return };
    match class {
        // Button: return after its wait.
        Class::Button(b) if b.state == ButtonState::In => button_out(w, id),
        // Rotating "Start ON".
        Class::Rotating(r) => {
            let target = r.maxspeed * r.dir;
            rot_set_target(w, id, target);
        }
        Class::Train(t) => {
            if !t.started {
                train_place_at_start(w, id);
            } else if t.old_speed != 0.0 && t.speed == 0.0 {
                let s = t.old_speed;
                t_set_speed(w, id, s);
            }
        }
        Class::Breakable(_) => super::breakables::think(w, id),
        Class::PropDoor(_) => super::props::door_think(w, id),
        _ => {}
    }
}

pub(super) fn keyvalue(w: &mut LogicWorld, id: EntId, key: &str) -> bool {
    let v = w.get(id).map(|e| e.kv_f(key)).unwrap_or(0.0);
    match (w.get_mut(id).map(|e| &mut e.class), key) {
        (Some(Class::Door(d)), "speed") => d.speed = v,
        (Some(Class::Door(d)), "wait") => d.wait = v,
        (Some(Class::Button(b)), "wait") => b.wait = v,
        // func_rotating: the speed its next Start (or SetSpeed) spins up
        // to (0 -> 100, as at spawn).
        (Some(Class::Rotating(r)), "maxspeed") => r.maxspeed = if v == 0.0 { 100.0 } else { v.abs() },
        (Some(Class::MoveLinear(m)), "speed") => m.speed = v,
        _ => return false,
    }
    true
}

/// Use (a player's +use or the Use input).
pub(super) fn use_entity(w: &mut LogicWorld, id: EntId, activator: Option<Who>, caller: Option<Who>) {
    use_entity_inner(w, id, activator, caller);
    w.settle(id);
}

fn use_entity_inner(w: &mut LogicWorld, id: EntId, activator: Option<Who>, _caller: Option<Who>) {
    let Some(class) = w.get(id).map(|e| e.class.clone()) else { return };
    let flags = w.get(id).map_or(0, |e| e.spawnflags);
    let by_player = matches!(activator, Some(Who::Player(_)));
    match class {
        Class::Door(d) => {
            if by_player && flags & 256 == 0 {
                sound(w, id, &d.locked_sound);
                return;
            }
            door_use_or_touch(w, id, activator, true);
        }
        Class::Button(_) => {
            if by_player && flags & 1024 == 0 {
                return;
            }
            button_press(w, id, activator, true);
        }
        Class::Rotating(r) => {
            let target = if r.speed != 0.0 { 0.0 } else { r.maxspeed * r.dir };
            rot_set_target(w, id, target);
        }
        Class::MoveLinear(m) => {
            // I/O Use: toggle between the ends.
            let goal = if m.goal == m.p2 { m.p1 } else { m.p2 };
            linear_to(w, id, goal);
        }
        Class::Train(t) => {
            let s = if t.speed != 0.0 { 0.0 } else { t.maxspeed };
            t_set_speed(w, id, s);
        }
        Class::Brush(_) => toggle_brush(w, id, None),
        Class::PropDoor(_) => super::props::door_use(w, id, activator),
        _ => {}
    }
}

fn linear_to(w: &mut LogicWorld, id: EntId, goal: Vec3) {
    let Some(Class::MoveLinear(m)) = w.get_mut(id).map(|e| &mut e.class) else { return };
    m.goal = goal;
    let speed = m.speed;
    m.push.move_to(goal, speed, Done::LinearArrived);
}

fn toggle_brush(w: &mut LogicWorld, id: EntId, on: Option<bool>) {
    if let Some(Class::Brush(t)) = w.get_mut(id).map(|e| &mut e.class) {
        t.enabled = on.unwrap_or(!t.enabled);
        t.apply();
    }
    w.refresh_solid(id);
}

pub(super) fn input(
    w: &mut LogicWorld,
    id: EntId,
    input: &str,
    value: &Value,
    activator: Option<Who>,
    caller: Option<Who>,
) -> bool {
    let handled = input_inner(w, id, input, value, activator, caller);
    w.settle(id);
    handled
}

fn input_inner(
    w: &mut LogicWorld,
    id: EntId,
    input: &str,
    value: &Value,
    activator: Option<Who>,
    _caller: Option<Who>,
) -> bool {
    let Some(class) = w.get(id).map(|e| e.class.clone()) else {
        return true;
    };
    match class {
        Class::Door(d) => match input {
            "open" => {
                if !matches!(d.state, DoorState::Open | DoorState::Opening) && !d.locked {
                    door_open(w, id);
                }
            }
            "close" => {
                if d.state != DoorState::Closed {
                    door_close(w, id);
                }
            }
            "toggle" => {
                if !d.locked {
                    match d.state {
                        DoorState::Open => door_close(w, id),
                        DoorState::Closed => door_open(w, id),
                        _ => {}
                    }
                }
            }
            "lock" => door(w, id).unwrap().locked = true,
            "unlock" => door(w, id).unwrap().locked = false,
            "setspeed" => {
                let Some(v) = w.need_float(value, input) else { return true };
                door(w, id).unwrap().speed = v;
            }
            _ => return false,
        },
        Class::Momentary(m) => match input {
            "setposition" | "setpositionimmediately" => {
                let Some(f) = w.need_float(value, input) else { return true };
                if m.locked {
                    return true;
                }
                let goal = m.angles_at(f);
                let Some(Class::Momentary(mm)) = w.get_mut(id).map(|e| &mut e.class) else { return true };
                if input == "setposition" {
                    let speed = mm.speed;
                    mm.push.rotate_to(goal, speed, Done::MomentaryArrived);
                } else {
                    // At once, no outputs (a map's OnFullyClosed
                    // -> SetPositionImmediately 0 would loop).
                    mm.push.angles = goal;
                    mm.push.avelocity = Vec3::ZERO;
                    mm.push.goal_angles = None;
                    mm.push.move_done = None;
                    w.refresh_solid(id);
                }
            }
            "lock" | "unlock" => {
                if let Some(Class::Momentary(mm)) = w.get_mut(id).map(|e| &mut e.class) {
                    mm.locked = input == "lock";
                }
            }
            // Its "update target" switches: nothing to switch here.
            "enable" | "disable" | "_disableupdatetarget" | "_enableupdatetarget" => {}
            _ => return false,
        },
        Class::Button(b) => match input {
            "press" => button_press(w, id, activator, false),
            "pressin" => {
                if matches!(b.state, ButtonState::Out | ButtonState::GoingOut) {
                    if b.state == ButtonState::GoingOut {
                        button(w, id).unwrap().state = ButtonState::Out;
                    }
                    button_press(w, id, activator, false);
                }
            }
            "pressout" => {
                if matches!(b.state, ButtonState::In | ButtonState::GoingIn) {
                    button_out(w, id);
                }
            }
            "lock" => button(w, id).unwrap().locked = true,
            "unlock" => button(w, id).unwrap().locked = false,
            _ => return false,
        },
        Class::MoveLinear(m) => match input {
            "open" => linear_to(w, id, m.p2),
            "close" => linear_to(w, id, m.p1),
            "setposition" => {
                let Some(f) = w.need_float(value, input) else { return true };
                linear_to(w, id, m.p1 + (m.p2 - m.p1) * f);
            }
            "setspeed" => {
                let Some(v) = w.need_float(value, input) else { return true };
                if let Some(Class::MoveLinear(m)) = w.get_mut(id).map(|e| &mut e.class) {
                    m.speed = v;
                }
                if m.push.origin != m.goal {
                    linear_to(w, id, m.goal);
                }
            }
            _ => return false,
        },
        Class::Rotating(r) => match input {
            "start" => rot_set_target(w, id, r.maxspeed * r.dir),
            "stop" | "stopatstartpos" => rot_set_target(w, id, 0.0),
            "toggle" => {
                let t = if r.speed > 0.0 { 0.0 } else { r.maxspeed * r.dir };
                rot_set_target(w, id, t);
            }
            "reverse" => {
                let rr = rotating(w, id).unwrap();
                rr.dir = -rr.dir;
                let t = -r.target;
                rot_set_target(w, id, t);
            }
            "setspeed" => {
                let Some(f) = w.need_float(value, input) else { return true };
                let dir = if f < 0.0 { -1.0 } else { 1.0 };
                rot_set_target(w, id, f.abs().clamp(0.0, 1.0) * r.maxspeed * dir);
            }
            "startforward" => {
                rotating(w, id).unwrap().dir = 1.0;
                rot_set_target(w, id, r.maxspeed);
            }
            "startbackward" => {
                rotating(w, id).unwrap().dir = -1.0;
                rot_set_target(w, id, -r.maxspeed);
            }
            _ => return false,
        },
        Class::Train(t) => match input {
            "startforward" => t_set_speed(w, id, t.maxspeed),
            "startbackward" => t_set_speed(w, id, -t.maxspeed),
            "stop" => {
                train(w, id).unwrap().old_speed = t.speed;
                t_set_speed(w, id, 0.0);
            }
            "toggle" => {
                let s = if t.speed != 0.0 { 0.0 } else { t.maxspeed };
                t_set_speed(w, id, s);
            }
            "resume" => t_set_speed(w, id, t.old_speed),
            "reverse" => t_set_speed(w, id, -t.speed),
            "setspeed" | "setspeeddir" | "setspeedreal" => {
                let Some(f) = w.need_float(value, input) else { return true };
                let s = match input {
                    "setspeed" => f.clamp(0.0, 1.0) * t.maxspeed * if t.speed < 0.0 { -1.0 } else { 1.0 },
                    "setspeeddir" => f.clamp(-1.0, 1.0) * t.maxspeed,
                    _ => f.clamp(0.0, t.maxspeed) * if t.speed < 0.0 { -1.0 } else { 1.0 },
                };
                t_set_speed(w, id, s);
            }
            _ => return false,
        },
        Class::PathTrack(_) => {
            let set = |w: &mut LogicWorld, f: &dyn Fn(&mut PathTrack)| {
                if let Some(Class::PathTrack(p)) = w.get_mut(id).map(|e| &mut e.class) {
                    f(p);
                }
            };
            match input {
                "enablepath" => set(w, &|p| p.enabled = true),
                "disablepath" => set(w, &|p| p.enabled = false),
                "togglepath" => set(w, &|p| p.enabled = !p.enabled),
                "enablealternatepath" => set(w, &|p| p.alt_enabled = true),
                "disablealternatepath" => set(w, &|p| p.alt_enabled = false),
                "togglealternatepath" => set(w, &|p| p.alt_enabled = !p.alt_enabled),
                "inpass" => w.fire_output(id, "OnPass", activator, Value::Void),
                _ => return false,
            }
        }
        Class::Brush(_) => match input {
            "enable" => toggle_brush(w, id, Some(true)),
            "disable" => toggle_brush(w, id, Some(false)),
            "toggle" => toggle_brush(w, id, None),
            _ => return false,
        },
        Class::PropDoor(_) => return super::props::door_input(w, id, input, value, activator),
        _ => return false,
    }
    true
}

/// Arrival or end of a wait.
fn move_done(w: &mut LogicWorld, id: EntId, done: Done) {
    match done {
        Done::None => {}
        Done::DoorOpened => {
            let toggle = w.get(id).is_some_and(|e| e.has_flag(32));
            let Some(d) = door(w, id) else { return };
            d.state = DoorState::Open;
            let (wait, arrive) = (d.wait, d.arrive_sound.clone());
            if !toggle && wait >= 0.0 {
                d.push.wait(wait, Done::DoorWaited);
            }
            sound(w, id, &arrive);
            w.fire_output(id, "OnFullyOpen", Some(Who::Ent(id)), Value::Void);
        }
        Done::DoorWaited => door_close(w, id),
        Done::DoorClosed => {
            let Some(d) = door(w, id) else { return };
            d.state = DoorState::Closed;
            let (user, arrive) = (d.last_user, d.arrive_sound.clone());
            sound(w, id, &arrive);
            w.fire_output(id, "OnFullyClosed", user, Value::Void);
        }
        Done::ButtonIn => {
            let toggle = w.get(id).is_some_and(|e| e.has_flag(32));
            let Some(b) = button(w, id) else { return };
            b.state = ButtonState::In;
            let (presser, wait) = (b.presser, b.wait);
            w.fire_output(id, "OnIn", presser, Value::Void);
            if wait != -1.0 && !toggle {
                w.think_in(id, wait);
            }
        }
        Done::ButtonOut => {
            let Some(b) = button(w, id) else { return };
            b.state = ButtonState::Out;
            let presser = b.presser;
            w.fire_output(id, "OnOut", presser, Value::Void);
        }
        Done::LinearArrived => {
            let Some(Class::MoveLinear(m)) = w.get(id).map(|e| e.class.clone()) else { return };
            if m.push.origin == m.p2 {
                w.fire_output(id, "OnFullyOpen", None, Value::Void);
            } else if m.push.origin == m.p1 {
                w.fire_output(id, "OnFullyClosed", None, Value::Void);
            }
        }
        Done::RotStep => rot_step(w, id),
        Done::MomentaryArrived => {
            let Some(Class::Momentary(m)) = w.get(id).map(|e| e.class.clone()) else { return };
            let f = m.position();
            w.fire_output(id, "Position", None, Value::Float(f));
            w.fire_output(id, "OnReachedPosition", None, Value::Void);
            if f >= 1.0 - 1e-4 {
                w.fire_output(id, "OnFullyOpen", None, Value::Void);
            } else if f <= 1e-4 {
                w.fire_output(id, "OnFullyClosed", None, Value::Void);
            }
        }
        Done::PropDoorOpened => super::props::door_arrived(w, id, true),
        Done::PropDoorClosed => super::props::door_arrived(w, id, false),
        Done::TrainArrived => {
            let Some(t) = train(w, id) else { return };
            let node = t.node;
            t.speed = 0.0;
            if let Some(n) = node {
                w.fire_output(n, "OnPass", Some(Who::Ent(id)), Value::Void);
            }
        }
    }
}

fn blocked(w: &mut LogicWorld, id: EntId, blocker: Who) {
    let started = {
        let p = push_of(w, id).unwrap();
        let s = p.blocker != Some(blocker);
        p.blocker = Some(blocker);
        s
    };
    let Some(class) = w.get(id).map(|e| e.class.clone()) else { return };
    let crush = |w: &mut LogicWorld, dmg: f32| {
        if dmg != 0.0
            && let Who::Player(p) = blocker
        {
            w.effects.push(Effect::Damage {
                target: p,
                amount: dmg,
                crush: true,
            });
        }
    };
    match class {
        Class::Door(_) => door_blocked(w, id, blocker, started),
        Class::PropDoor(_) => super::props::door_blocked(w, id, blocker, started),
        Class::MoveLinear(m) => crush(w, m.block_damage),
        Class::Rotating(r) => crush(w, r.dmg),
        Class::Train(t) => {
            if let Who::Player(p) = blocker
                && let Some(pl) = w.player_mut(p)
            {
                if pl.ground == Some(id) {
                    if pl.velocity.z == 0.0 {
                        pl.velocity.z = t.speed.abs().min(50.0);
                    }
                    return;
                }
                pl.velocity = (pl.origin - t.push.origin).normalize_or_zero() * t.dmg;
                if !t.push.unblockable {
                    crush(w, t.dmg);
                }
            }
        }
        _ => {}
    }
}

/// One pusher step as `push_players` takes it (entity space; angles in
/// degrees): the mover, where it was, how far it moves and turns.
#[derive(Clone, Copy, Debug)]
pub struct PushStep {
    /// What a player stands on to ride it (`Player::ground`).
    pub id: EntId,
    pub origin: Vec3,
    pub angles: Vec3,
    pub d: Vec3,
    pub da: Vec3,
    /// Model doors: a rotation pushes by the box's leading corner.
    pub physics_solid: bool,
    /// Players are moved through rather than blocking (trains flag 512).
    pub unblockable: bool,
}

/// The players part of a pusher's step (doors_buttons.md, "Pushing,
/// riding and blocking"): riders (standing on it) and players its moved
/// brushes (`moved`) overlap are carried or shoved, swept against the
/// world (`col`) and the other movers' brushes (`others`). Ok: the players
/// it moved, as they were (index, origin, view), for a caller that undoes
/// the step; Err(blocker): a player stopped it and everyone is back.
/// Shared by the logic and a network client predicting its own player on
/// a mover (`logic::carry_player`).
pub fn push_players(
    players: &mut [super::world::Player],
    step: &PushStep,
    moved: &[MapBrush],
    others: &[&MapBrush],
    col: &dyn Collision,
) -> Result<Vec<(usize, Vec3, Vec3)>, Entity> {
    let PushStep {
        id,
        origin,
        angles,
        d,
        da,
        physics_solid,
        unblockable,
    } = *step;
    let (q_old, q_new) = (entity_rotation(angles), entity_rotation(angles + da));
    let (o_old, o_new) = (origin, origin + d);
    let overlaps = |pl: &super::world::Player| {
        let (half, centre) = ((pl.maxs - pl.mins) / 2.0, pl.origin + (pl.mins + pl.maxs) / 2.0);
        moved.iter().any(|b| b.overlaps_box(centre, half, SOLID_SKIN))
    };
    let candidates: Vec<usize> = (0..players.len())
        .filter(|i| {
            let pl = &players[*i];
            pl.alive && (pl.ground == Some(id) || overlaps(pl))
        })
        .collect();
    let saved: Vec<(usize, Vec3, Vec3)> = candidates
        .iter()
        .map(|i| (*i, players[*i].origin, players[*i].view))
        .collect();
    let turn = q_new * q_old.inverse();
    for &i in candidates.iter().rev() {
        let pl = players[i].clone();
        let centre = pl.origin + (pl.mins + pl.maxs) / 2.0;
        let push = if da == Vec3::ZERO {
            d
        } else {
            let mut at = centre;
            if physics_solid {
                let motion = o_new + turn * (centre - o_old) - centre;
                let half = (pl.maxs - pl.mins) / 2.0;
                let side = |m: f32| if m > 0.0 { 1.0 } else if m < 0.0 { -1.0 } else { 0.0 };
                at += half * Vec3::new(side(motion.x), side(motion.y), side(motion.z));
            }
            o_new + turn * (at - o_old) - at
        };
        let to = pl.origin + push;
        let frac = col.sweep(pl.mins, pl.maxs, pl.origin, to).min(super::world::sweep_brushes(
            others.iter().copied(),
            pl.mins,
            pl.maxs,
            pl.origin,
            to,
        ));
        let end = pl.origin + push * frac;
        let mut accepted = frac >= 1.0 && da == Vec3::ZERO;
        if !accepted {
            let stuck = col.solid(pl.mins, pl.maxs, end)
                || super::world::solid_brushes(others.iter().copied().chain(moved.iter()), pl.mins, pl.maxs, end);
            accepted = !stuck;
            if stuck && unblockable {
                players[i].origin = to;
                players[i].moved = true;
                continue;
            }
        }
        if !accepted {
            for (j, o, v) in &saved {
                players[*j].origin = *o;
                players[*j].view = *v;
            }
            return Err(pl.entity);
        }
        let pm = &mut players[i];
        if push.length_squared() > 0.0 {
            pm.origin = end;
            pm.moved = true;
        }
        if da != Vec3::ZERO && pm.ground == Some(id) {
            pm.view.y += da.y;
            pm.moved = true;
        }
    }
    Ok(saved)
}

/// Movers' world brushes (`LogicWorld::solids`), except `skip`'s.
fn other_brushes(solids: &[Option<Vec<MapBrush>>], skip: Option<EntId>) -> impl Iterator<Item = &MapBrush> {
    solids
        .iter()
        .enumerate()
        .filter(move |(i, _)| skip.is_none_or(|s| s.index as usize != *i))
        .filter_map(|(_, b)| b.as_ref())
        .flatten()
}

impl LogicWorld {
    /// Recompute a mover's world brushes after it moved.
    pub(super) fn refresh_solid(&mut self, id: EntId) {
        let Some(e) = self.get(id) else { return };
        let Some(p) = pusher(&e.class) else { return };
        let brushes = if p.solid {
            let rot = entity_rotation(p.angles);
            Some(e.hulls.iter().map(|h| place_hull(h, rot, p.origin)).collect::<Vec<_>>())
        } else {
            None
        };
        let i = id.index as usize;
        if self.solids.len() <= i {
            self.solids.resize(i + 1, None);
        }
        self.solids[i] = brushes;
    }

    /// Movers' brushes where they are now, except `skip`.
    fn mover_brushes(&self, skip: Option<EntId>) -> impl Iterator<Item = &MapBrush> {
        other_brushes(&self.solids, skip)
    }

    /// Mover brush entities in entity order: (id, pose, visible, solid).
    pub fn mover_poses(&self) -> Vec<(EntId, Vec3, Vec3, Vec3, bool, bool)> {
        self.ids()
            .into_iter()
            .filter_map(|id| {
                let e = self.get(id)?;
                let p = pusher(&e.class)?;
                Some((id, p.origin, p.angles, p.velocity + p.carry, p.visible, p.solid))
            })
            .collect()
    }

    /// A mover's world brushes this tick (entity space), if solid.
    pub fn mover_solid(&self, id: EntId) -> Option<&[MapBrush]> {
        self.solids.get(id.index as usize)?.as_deref()
    }

    /// One push step for every mover (doors_buttons.md, "Moving brush
    /// basics" and "Pushing, riding and blocking").
    pub fn step_movers(&mut self, collision: &dyn Collision) {
        let dt = self.dt as f64;
        for id in self.ids() {
            let Some(e) = self.get(id) else { continue };
            if pusher(&e.class).is_none() || e.killed || matches!(e.class, Class::Attached(_)) {
                continue;
            }
            if matches!(e.class, Class::Train(_)) {
                train_update(self, id);
            }
            let p = pusher(&self.get(id).unwrap().class).unwrap().clone();
            if !p.moving() && p.move_done.is_none() {
                self.mover_think(id);
                continue;
            }
            let step = match p.move_done {
                Some(done) if !p.inexact => (done - p.ltime).clamp(0.0, dt),
                _ => dt,
            };
            let d = p.velocity * step as f32;
            let da = p.avelocity * step as f32;
            let result = if (d != Vec3::ZERO || da != Vec3::ZERO) && p.solid {
                self.push(id, &p, d, da, collision)
            } else {
                Ok(())
            };
            match result {
                Ok(()) => {
                    let spinner = matches!(self.get(id).map(|e| &e.class), Some(Class::Rotating(_)));
                    let unblocked = {
                        let pm = push_of(self, id).unwrap();
                        pm.origin += d;
                        pm.angles += da;
                        // Keep endlessly spinning angles small.
                        if spinner {
                            pm.angles = Vec3::new(
                                pm.angles.x.rem_euclid(360.0),
                                pm.angles.y.rem_euclid(360.0),
                                pm.angles.z.rem_euclid(360.0),
                            );
                        }
                        pm.ltime += step;
                        pm.blocker.take()
                    };
                    let door_state = match self.get(id).map(|e| &e.class) {
                        Some(Class::Door(d)) => Some(d.state),
                        Some(Class::PropDoor(d)) => Some(d.state),
                        _ => None,
                    };
                    if let Some(b) = unblocked
                        && let Some(state) = door_state
                    {
                        let out = if state == DoorState::Opening {
                            "OnUnblockedOpening"
                        } else {
                            "OnUnblockedClosing"
                        };
                        self.fire_output(id, out, Some(b), Value::Void);
                    }
                    self.refresh_solid(id);
                }
                Err(who) => blocked(self, id, who),
            }
            self.mover_think(id);
            self.settle(id);
        }
        // Children of movers: carried along, then parented brushes.
        self.follow_anchors();
        self.follow_parents();
    }

    /// Parented brushes take their parent's pose.
    fn follow_parents(&mut self) {
        for id in self.ids() {
            let Some(a) = self.get(id).and_then(|e| attached(&e.class)) else { continue };
            let Some(parent) = a.parent else { continue };
            let Some(pp) = self.get(parent).and_then(|e| pusher(&e.class)).cloned() else {
                continue;
            };
            let (offset, base, parent_base) = (a.offset, a.base_angles, a.parent_angles);
            let origin = pp.origin + entity_rotation(pp.angles) * offset;
            let angles = base + (pp.angles - parent_base);
            let (visible, velocity) = (pp.visible, pp.velocity);
            if let Some(a) = self.get_mut(id).and_then(|e| attached_mut(&mut e.class))
                && (a.push.origin != origin || a.push.angles != angles || a.push.visible != visible)
            {
                a.push.origin = origin;
                a.push.angles = angles;
                a.push.visible = visible;
                a.push.velocity = velocity;
                self.refresh_solid(id);
            }
        }
    }

    /// Finish a move or wait that is done (a zero wait or zero-length move
    /// finishes in the tick it starts).
    pub(super) fn settle(&mut self, id: EntId) {
        for _ in 0..4 {
            let Some(pm) = push_of(self, id) else { return };
            let Some(done) = pm.move_done else { break };
            if pm.ltime < done - 1e-6 {
                break;
            }
            if let Some(g) = pm.goal_origin.take() {
                pm.origin = g;
                pm.velocity = Vec3::ZERO;
            }
            if let Some(g) = pm.goal_angles.take() {
                pm.angles = g;
                pm.avelocity = Vec3::ZERO;
            }
            pm.move_done = None;
            pm.last_done = done;
            let on_done = std::mem::take(&mut pm.on_done);
            self.refresh_solid(id);
            move_done(self, id, on_done);
        }
    }

    /// A mover's think, when due (after its push step), then any arrival
    /// it caused.
    fn mover_think(&mut self, id: EntId) {
        let due = self
            .get(id)
            .is_some_and(|e| !e.killed && e.next_think.is_some_and(|t| t <= self.tick));
        if !due {
            return;
        }
        self.get_mut(id).unwrap().next_think = None;
        think(self, id);
        self.settle(id);
    }

    /// Move a pusher by `d` / rotate by `da`, carrying riders and shoving
    /// players in the way; Err(blocker) puts everything back.
    fn push(&mut self, id: EntId, p: &Pusher, d: Vec3, da: Vec3, col: &dyn Collision) -> Result<(), Who> {
        let hulls = self.get(id).unwrap().hulls.clone();
        let (q_old, q_new) = (entity_rotation(p.angles), entity_rotation(p.angles + da));
        let (o_old, o_new) = (p.origin, p.origin + d);
        let moved: Vec<MapBrush> = hulls.iter().map(|h| place_hull(h, q_new, o_new)).collect();
        let turn = q_new * q_old.inverse();
        // Model doors are physics-solid pushers: a rotation pushes by the
        // corner of the box facing the motion, not its centre.
        let physics_solid = matches!(self.get(id).map(|e| &e.class), Some(Class::PropDoor(_)));
        let saved;
        {
            let others: Vec<&MapBrush> = other_brushes(&self.solids, Some(id)).collect();
            let step = PushStep {
                id,
                origin: p.origin,
                angles: p.angles,
                d,
                da,
                physics_solid,
                unblockable: p.unblockable,
            };
            saved = push_players(&mut self.players, &step, &moved, &others, col).map_err(Who::Player)?;
        }
        // Model doors and loose physics props (doors_buttons.md
        // prop_door_rotating, "Blocked"): the physics pushes a prop in the
        // way; one that would be pushed into the world blocks the door,
        // unless it is forceclosed: then it is pushed through, and a
        // damageable one takes its full health as crush damage.
        if physics_solid {
            let forceclosed = self.get(id).is_some_and(|e| e.kv_i("forceclosed") != 0);
            let mut crushed = Vec::new();
            for (pid, centre, half) in super::props::door_props(self) {
                if !moved.iter().any(|b| b.overlaps_box(centre, half, SOLID_SKIN)) {
                    continue;
                }
                let motion = o_new + turn * (centre - o_old) - centre;
                let side = |m: f32| if m > 0.0 { 1.0 } else if m < 0.0 { -1.0 } else { 0.0 };
                let at = centre + half * Vec3::new(side(motion.x), side(motion.y), side(motion.z));
                let push = o_new + turn * (at - o_old) - at;
                let end = centre + push;
                let stuck = col.solid(-half, half, end)
                    || super::world::solid_brushes(self.mover_brushes(Some(id)), -half, half, end);
                if !stuck {
                    continue;
                }
                if !forceclosed {
                    for (j, o, v) in &saved {
                        self.players[*j].origin = *o;
                        self.players[*j].view = *v;
                    }
                    return Err(Who::Ent(pid));
                }
                crushed.push((pid, centre, push));
            }
            for (pid, centre, push) in crushed {
                let health = self.prop(pid).map_or(0, |p| p.health);
                if health > 0 {
                    let hit = super::prop_damage::Hit {
                        amount: health as f32,
                        kind: crate::core::DamageKind::Crush,
                        attacker: Some(Who::Ent(id)),
                        point: centre,
                        dir: push.normalize_or_zero(),
                        force: 0.0,
                        direct: false,
                    };
                    self.prop_hit(pid, hit);
                }
            }
        }
        Ok(())
    }

    /// +use for players who pressed it this tick.
    pub fn player_uses(&mut self, col: &dyn Collision) {
        self.use_presses.clear();
        for i in 0..self.players.len() {
            let (e, held) = (self.players[i].entity, self.players[i].use_key);
            let was = self.use_held.iter().any(|x| *x == e);
            self.use_held.retain(|x| *x != e);
            if held {
                self.use_held.push(e);
            }
            if !held || was || !self.players[i].alive {
                continue;
            }
            let found = self.find_use(i, col);
            self.use_presses.push((e, found.is_some()));
            if let Some(target) = found {
                let who = Some(Who::Player(e));
                use_entity(self, target, who, who);
            }
        }
    }

    /// The use search (doors_buttons.md, "+use: finding what to use").
    pub fn find_use(&self, i: usize, col: &dyn Collision) -> Option<EntId> {
        let pl = &self.players[i];
        let eye = pl.origin + pl.eye;
        let f = forward(pl.view);
        let (sp, cp) = pl.view.x.to_radians().sin_cos();
        let (sy, cy) = pl.view.y.to_radians().sin_cos();
        let up = Vec3::new(sp * cy, sp * sy, cp);
        let usable: Vec<EntId> = self
            .ids()
            .into_iter()
            .filter(|id| match self.get(*id) {
                Some(e) => match &e.class {
                    Class::Door(_) | Class::Button(_) | Class::Rotating(_) => true,
                    Class::PropDoor(_) => !e.has_flag(super::props::SF_DOOR_IGNORE_USE),
                    _ => false,
                },
                None => false,
            })
            .collect();
        let (feet, head) = (pl.origin.z + pl.mins.z, pl.origin.z + pl.maxs.z);
        let use_distance = |h: Vec3| {
            let dz = if h.z < feet {
                feet - h.z
            } else if h.z > head {
                h.z - head
            } else {
                0.0
            };
            Vec3::new(h.x - eye.x, h.y - eye.y, dz).length()
        };
        // First hit of a box sweep among usable movers, if before the
        // world (and before other movers).
        let hit = |half: f32, to: Vec3| -> Option<(EntId, Vec3)> {
            let (lo, hi) = (Vec3::splat(-half), Vec3::splat(half));
            let world = col.sweep(lo, hi, eye, to);
            let mut best: Option<(f32, Option<EntId>)> = None;
            for (idx, brushes) in self.solids.iter().enumerate() {
                let Some(brushes) = brushes else { continue };
                let f = super::world::sweep_brushes(brushes.iter(), lo, hi, eye, to);
                if f < 1.0 && best.is_none_or(|(b, _)| f < b) {
                    let id = usable.iter().copied().find(|u| u.index as usize == idx);
                    best = Some((f, id));
                }
            }
            let (f, id) = best?;
            (f <= world).then_some(())?;
            let id = id?;
            // Reach is measured to the hit entity itself (its nearest
            // point), not to the centre of the swept box.
            let brushes = self.solids.get(id.index as usize)?.as_ref()?;
            let lo = brushes.iter().fold(Vec3::MAX, |a, b| a.min(b.min));
            let hi = brushes.iter().fold(Vec3::MIN, |a, b| a.max(b.max));
            let _ = f;
            Some((id, eye.clamp(lo, hi)))
        };
        if let Some((id, h)) = hit(0.0, eye + f * USE_TRACE_LONG)
            && use_distance(h) < USE_RADIUS
        {
            return Some(id);
        }
        let mut candidate = None;
        for t in USE_TANGENTS {
            let dir = (f - up * t).normalize();
            if let Some((id, h)) = hit(USE_BOX_HALF, eye + dir * USE_TRACE_BOX)
                && use_distance(h) < USE_RADIUS
            {
                candidate = Some(id);
            }
        }
        if let Some(g) = pl.ground
            && usable.contains(&g)
        {
            candidate = Some(g);
        }
        let nearest = |id: EntId| -> Option<Vec3> {
            let brushes = self.solids.get(id.index as usize)?.as_ref()?;
            let lo = brushes.iter().fold(Vec3::MAX, |a, b| a.min(b.min));
            let hi = brushes.iter().fold(Vec3::MIN, |a, b| a.max(b.max));
            Some(eye.clamp(lo, hi))
        };
        let line_distance = |p: Vec3| (p - eye).cross(f).length();
        let mut best = candidate.and_then(|c| nearest(c).map(|p| (c, line_distance(p))));
        for &id in &usable {
            let Some(p) = nearest(id) else { continue };
            if (p - eye).length() > USE_RADIUS {
                continue;
            }
            let dir = (p - eye).normalize_or_zero();
            if dir.dot(f) < USE_CONE_DOT {
                continue;
            }
            let score = line_distance(p);
            if best.is_some_and(|(_, s)| s <= score) {
                continue;
            }
            // The line to it must be clear of the world.
            let to = eye + (p - eye) * 0.99;
            if col.sweep(Vec3::ZERO, Vec3::ZERO, eye, to) >= 1.0 {
                best = Some((id, score));
            }
        }
        best.map(|(id, _)| id)
    }

    /// Doors that open on touch (flag 1024) and buttons pressed by touch
    /// (flag 256, "Touch Activates"; a locked one does nothing): a player
    /// against them.
    pub(super) fn touch_movers(&mut self) {
        for id in self.ids() {
            let Some(e) = self.get(id) else { continue };
            let door = matches!(e.class, Class::Door(_)) && e.has_flag(1024);
            let button = matches!(&e.class, Class::Button(b) if !b.locked) && e.has_flag(256);
            if !door && !button {
                continue;
            }
            let Some(brushes) = self.solids.get(id.index as usize).cloned().flatten() else {
                continue;
            };
            for i in 0..self.players.len() {
                let pl = &self.players[i];
                if !pl.alive {
                    continue;
                }
                let grow = Vec3::splat(1.0 + SWEEP_EPS);
                let (half, centre) = ((pl.maxs - pl.mins) / 2.0 + grow, pl.origin + (pl.mins + pl.maxs) / 2.0);
                // Standing on it touches it too: a landing can rest up to
                // 2 units above what it stands on (movement.md, ground
                // detection), past the 1-unit reach (bhop blocks: classic
                // touch-open doors under the player). Our reading;
                // doors_buttons.md open question 9.
                let stands_on = pl.on_ground && pl.ground == Some(id);
                if stands_on || brushes.iter().any(|b| b.overlaps_box(centre, half, 0.0)) {
                    let who = Some(Who::Player(pl.entity));
                    if door {
                        door_use_or_touch(self, id, who, true);
                    } else {
                        button_press(self, id, who, false);
                    }
                }
            }
        }
    }
}
