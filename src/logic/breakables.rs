//! Breakable brushes (specs/source/breakables.md): func_breakable (health,
//! material, damage scaling, break sound, gibs, OnBreak, removal 0.1 s
//! later) and func_breakable_surf (glass windows as a grid of panes that
//! shatter where hit or touched and collapse when they lose support).
//!
//! Both are pushers that never move, so they get a mover node: the host
//! hides the node and drops its collider when the brush stops being
//! solid, and draws and collides only the unbroken panes of a window
//! (`LogicWorld::window`). Hits come in through `LogicWorld::damage`.

use bevy::prelude::*;

use super::classes::Class;
use super::movers::Attached;
use super::value::Value;
use super::world::{Effect, EntId, LogicWorld, Who, box_touches, place_hull};
use crate::core::DamageKind;
use crate::map::entities::entity_rotation;

/// func_breakdmg_bullet: bullets do half damage.
pub const DMG_MOD_BULLET: f32 = 0.5;
/// func_breakdmg_club (the knife counts as a club here: spec Q2).
pub const DMG_MOD_CLUB: f32 = 1.5;
/// func_breakdmg_explosive.
pub const DMG_MOD_BLAST: f32 = 1.25;
/// func_break_max_pieces.
pub const MAX_PIECES: usize = 15;
/// Gib count reference: 3 × 12².
const GIB_AREA: f32 = 432.0;
pub const GIB_SPEED_RAND: f32 = 100.0;
pub const GIB_DIRECTED_SPEED: f32 = 200.0;
/// Broken func_breakables are removed this long after breaking.
pub const REMOVE_DELAY: f32 = 0.1;
pub const PANE_SIZE: f32 = 12.0;
pub const MAX_PANES_AXIS: usize = 16;
pub const PANE_FULL_SUPPORT: f32 = 6.75;
pub const PANE_BREAK_SUPPORT: f32 = 0.2;
pub const PANE_BULLET_FORCE: f32 = 500.0;

/// The "material" keyvalue.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Material {
    Glass,
    Wood,
    Metal,
    Flesh,
    CinderBlock,
    CeilingTile,
    Computer,
    UnbreakableGlass,
    Rocks,
    Web,
    None,
}

impl Material {
    pub fn from_index(i: i32) -> Self {
        match i {
            0 => Self::Glass,
            1 => Self::Wood,
            2 => Self::Metal,
            3 => Self::Flesh,
            4 => Self::CinderBlock,
            5 => Self::CeilingTile,
            6 => Self::Computer,
            7 => Self::UnbreakableGlass,
            8 => Self::Rocks,
            9 => Self::Web,
            10 => Self::None,
            _ => Self::Wood,
        }
    }

    /// The sound entry played when it breaks.
    pub fn break_sound(self) -> Option<&'static str> {
        Some(match self {
            Self::Glass => "Breakable.Glass",
            Self::Wood => "Breakable.Crate",
            Self::Metal | Self::Computer => "Breakable.Metal",
            Self::Flesh | Self::Web => "Breakable.Flesh",
            Self::Rocks | Self::CinderBlock => "Breakable.Concrete",
            Self::CeilingTile => "Breakable.Ceiling",
            _ => return None,
        })
    }

    /// The sound entry played when it is damaged but survives (computers:
    /// metal half the time).
    pub fn damage_sound(self, coin: bool) -> Option<&'static str> {
        Some(match self {
            Self::Glass | Self::UnbreakableGlass => "Breakable.MatGlass",
            Self::Wood => "Breakable.MatWood",
            Self::Metal => "Breakable.MatMetal",
            Self::Rocks | Self::CinderBlock => "Breakable.MatConcrete",
            Self::Computer if coin => "Breakable.MatMetal",
            Self::Computer => "Breakable.Computer",
            _ => return None,
        })
    }

    /// The gib list (propdata "BreakableModels" name). Materials without
    /// their own list use wood's.
    pub fn gibs(self) -> &'static str {
        match self {
            Self::Glass | Self::UnbreakableGlass => "GlassChunks",
            Self::Metal => "MetalChunks",
            Self::Rocks | Self::CinderBlock => "ConcreteChunks",
            _ => "WoodChunks",
        }
    }

    pub fn is_glass(self) -> bool {
        matches!(self, Self::Glass | Self::UnbreakableGlass)
    }
}

/// One gib thrown by a break (entity space).
#[derive(Clone, Debug, PartialEq)]
pub struct Gib {
    pub position: Vec3,
    pub velocity: Vec3,
    /// Degrees per second about (x, y, z).
    pub spin: Vec3,
    /// Seconds before it fades out.
    pub life: f32,
}

/// A func_breakable_surf window: a grid of panes over the drawn face.
#[derive(Clone, Debug)]
pub struct Window {
    /// Tile (surfacetype 1) rather than glass.
    pub tile: bool,
    pub fragility: f32,
    /// Lower-left corner, local to the entity (unrotated), and one pane's
    /// steps along the columns and the rows (up).
    pub corner: Vec3,
    pub u: Vec3,
    pub v: Vec3,
    pub normal: Vec3,
    pub cols: usize,
    pub rows: usize,
    /// Per pane (row-major: `r * cols + c`): support, 0 once broken.
    pub support: Vec<f32>,
    pub broken: Vec<bool>,
    /// The window entity broke (first hit): non-solid from then on.
    pub window_broken: bool,
    /// A pane damage sound already played this tick.
    sounded: i64,
}

impl Window {
    /// The grid from the corner keyvalues (entity space), local to
    /// `origin` and `angles`.
    pub fn new(ll: Vec3, lr: Vec3, ul: Vec3, origin: Vec3, angles: Vec3, tile: bool, fragility: f32) -> Self {
        let inv = entity_rotation(angles).inverse();
        let (ll, lr, ul) = (inv * (ll - origin), inv * (lr - origin), inv * (ul - origin));
        let (w, h) = (lr - ll, ul - ll);
        // A window under 12 units in a direction still gets one pane
        // (the original divides by zero).
        let count = |len: f32| ((len / PANE_SIZE).floor() as usize).clamp(1, MAX_PANES_AXIS);
        let (cols, rows) = (count(w.length()), count(h.length()));
        let n = cols * rows;
        Self {
            tile,
            fragility,
            corner: ll,
            u: w / cols as f32,
            v: h / rows as f32,
            normal: w.cross(h).normalize_or_zero(),
            cols,
            rows,
            support: vec![1.0; n],
            broken: vec![false; n],
            window_broken: false,
            sounded: -1,
        }
    }

    pub fn index(&self, c: usize, r: usize) -> usize {
        r * self.cols + c
    }

    pub fn is_broken(&self, c: i32, r: i32) -> bool {
        c < 0
            || r < 0
            || c as usize >= self.cols
            || r as usize >= self.rows
            || self.broken[self.index(c as usize, r as usize)]
    }

    /// Pane coordinates (fractional) of a local point.
    pub fn pane_at(&self, local: Vec3) -> Vec2 {
        let d = local - self.corner;
        Vec2::new(
            d.dot(self.u) / self.u.length_squared(),
            d.dot(self.v) / self.v.length_squared(),
        )
    }

    /// A pane's centre, local.
    pub fn pane_centre(&self, c: usize, r: usize) -> Vec3 {
        self.corner + self.u * (c as f32 + 0.5) + self.v * (r as f32 + 0.5)
    }

    pub fn broken_count(&self) -> usize {
        self.broken.iter().filter(|b| **b).count()
    }

    /// The support pass's new support for every pane (old values in),
    /// spec "Support pass" step 1 and 2; broken panes stay 0.
    pub fn supports(&self) -> Vec<f32> {
        let (cols, rows) = (self.cols as i32, self.rows as i32);
        let at = |c: i32, r: i32, edge: f32| -> f32 {
            if c < 0 || r < 0 || c >= cols || r >= rows {
                edge
            } else {
                self.support[(r * cols + c) as usize] * edge
            }
        };
        let mut out = vec![0.0; self.support.len()];
        for r in 0..rows {
            for c in 0..cols {
                let i = (r * cols + c) as usize;
                if self.broken[i] {
                    continue;
                }
                let s = 0.01
                    + at(c, r + 1, 1.0)
                    + at(c, r - 1, 1.25)
                    + at(c - 1, r, 1.0)
                    + at(c + 1, r, 1.0)
                    + at(c - 1, r - 1, 1.0)
                    + at(c + 1, r - 1, 1.0)
                    + at(c + 1, r + 1, 0.25)
                    + at(c - 1, r + 1, 0.25);
                out[i] = s / PANE_FULL_SUPPORT;
            }
        }
        out
    }
}

/// func_breakable / func_breakable_surf.
#[derive(Clone, Debug)]
pub struct Breakable {
    /// Its pose, and its parent when parented to a mover (de_nuke's door
    /// windows follow their doors).
    pub attach: Attached,
    pub material: Material,
    pub health: i32,
    pub max_health: i32,
    /// Damage can break it (else only the Break input, touch or the
    /// health inputs).
    pub damageable: bool,
    pub broken: bool,
    pub window: Option<Box<Window>>,
}

impl Breakable {
    pub(super) fn spawn(w: &mut LogicWorld, id: EntId) -> Breakable {
        let e = w.get(id).unwrap();
        let mut material = Material::from_index(e.kv_i("material"));
        let surf = e.classname.eq_ignore_ascii_case("func_breakable_surf");
        let window = surf.then(|| {
            let tile = e.kv_i("surfacetype") == 1;
            if !tile {
                material = Material::Glass;
            }
            let fragility = e.kv("fragility").map_or(100.0, |_| e.kv_f("fragility"));
            Box::new(Window::new(
                e.vector_kv("lowerleft"),
                e.vector_kv("lowerright"),
                e.vector_kv("upperleft"),
                e.origin,
                e.angles,
                tile,
                fragility,
            ))
        });
        let mut health = e.kv_i("health");
        let trigger_only = e.has_flag(1) && !surf;
        let damageable = !(health == 0 || trigger_only);
        if health == 0 && material.is_glass() {
            health = 1;
        }
        // A window the compiler flagged (not one quad) removes itself.
        if surf && e.kv_i("error") != 0 {
            w.kill(id);
        }
        Breakable {
            attach: Attached::spawn(w, id),
            material,
            health,
            max_health: health.max(1),
            damageable,
            broken: false,
            window,
        }
    }
}

fn breakable(w: &mut LogicWorld, id: EntId) -> Option<&mut Breakable> {
    match w.get_mut(id).map(|e| &mut e.class) {
        Some(Class::Breakable(b)) => Some(b),
        _ => None,
    }
}

/// Where a breakable is now (it follows a moving parent): origin and
/// rotation.
fn pose(w: &LogicWorld, id: EntId) -> (Vec3, Quat) {
    match w.get(id).map(|e| (&e.class, e)) {
        Some((Class::Breakable(b), _)) => (b.attach.push.origin, entity_rotation(b.attach.push.angles)),
        Some((_, e)) => (e.origin, entity_rotation(e.angles)),
        None => (Vec3::ZERO, Quat::IDENTITY),
    }
}

/// The local bounds of an entity's volumes.
fn local_bounds(w: &LogicWorld, id: EntId) -> (Vec3, Vec3) {
    let Some(e) = w.get(id) else {
        return (Vec3::ZERO, Vec3::ZERO);
    };
    let pts = e.hulls.iter().flat_map(|h| &h.points);
    let (lo, hi) = pts.fold((Vec3::MAX, Vec3::MIN), |(lo, hi), p| (lo.min(*p), hi.max(*p)));
    if lo.x > hi.x {
        (Vec3::ZERO, Vec3::ZERO)
    } else {
        (lo, hi)
    }
}

/// Gib count for a brush of size `s` (spec "Breaking" step 3).
pub fn gib_count(size: Vec3, performance_mode: i32) -> usize {
    let n = ((size.x * size.y + size.y * size.z + size.z * size.x) / GIB_AREA)
        .floor()
        .max(0.0) as usize;
    let n = n.min(MAX_PIECES);
    match performance_mode {
        1 => 0,
        3 if n > 0 => (n / 2).max(1),
        _ => n,
    }
}

/// Scale damage by its kind (spec "Damage" step 3).
pub fn scale_damage(amount: f32, kind: DamageKind) -> f32 {
    match kind {
        DamageKind::Bullet => amount * DMG_MOD_BULLET,
        DamageKind::Melee => amount * DMG_MOD_CLUB,
        DamageKind::Blast => amount * DMG_MOD_BLAST,
        _ => amount,
    }
}

impl LogicWorld {
    /// Damage dealt to a breakable or a prop (`props::prop_damage`):
    /// `amount` in health points, at `point` travelling along `dir`
    /// (entity space). Returns false when `id` is neither.
    pub fn damage(
        &mut self,
        id: EntId,
        amount: f32,
        kind: DamageKind,
        attacker: Option<Who>,
        point: Vec3,
        dir: Vec3,
    ) -> bool {
        let Some(b) = breakable(self, id) else {
            return super::props::prop_damage(self, id, amount, kind, attacker);
        };
        if b.broken && b.window.is_none() {
            return true;
        }
        if b.window.is_some() {
            window_hit(self, id, amount, kind, attacker, point, dir);
            return true;
        }
        let (damageable, material) = (b.damageable, b.material);
        let Some(e) = self.get(id) else { return true };
        let min = e.kv_f("minhealthdmg");
        let filter = e.kv("damagefilter").unwrap_or("").to_string();
        if !damageable || amount < min || material == Material::UnbreakableGlass {
            return true;
        }
        if !filter.is_empty()
            && let Some(f) = self.find(&filter)
            && !super::classes::filter_passes(self, f, attacker)
        {
            return true;
        }
        let a = scale_damage(amount, kind);
        let b = breakable(self, id).unwrap();
        let new = (b.health as f32 - a).trunc() as i32;
        let changed = new != b.health;
        b.health = new;
        let ratio = (new as f32 / b.max_health as f32).clamp(0.0, 1.0);
        if changed {
            self.fire_output(id, "OnHealthChanged", attacker, Value::Float(ratio));
        }
        if new <= 0 {
            self.break_entity(id, attacker, dir);
        } else {
            let coin = self.random() < 0.5;
            if let Some(s) = material.damage_sound(coin) {
                self.breakable_sound(id, s);
            }
        }
        true
    }

    /// The window state of a func_breakable_surf.
    pub fn window(&self, id: EntId) -> Option<&Window> {
        match self.get(id).map(|e| &e.class) {
            Some(Class::Breakable(b)) => b.window.as_deref(),
            _ => None,
        }
    }

    /// Whether shots should still hit a mover brush: solid, or a broken
    /// window with panes left.
    pub fn shootable(&self, id: EntId) -> bool {
        match self.get(id).map(|e| &e.class) {
            Some(Class::Breakable(b)) => match &b.window {
                Some(w) => !w.broken.iter().all(|b| *b),
                None => b.attach.push.solid,
            },
            Some(c) => super::movers::pusher(c).is_some_and(|p| p.solid),
            None => false,
        }
    }

    fn breakable_sound(&mut self, id: EntId, entry: &str) {
        let Some(e) = self.get(id) else { return };
        let (lo, hi) = local_bounds(self, id);
        let _ = e;
        let (origin, rot) = pose(self, id);
        let at = origin + rot * ((lo + hi) / 2.0);
        self.effects.push(Effect::Sound {
            entry: entry.to_string(),
            at,
        });
    }

    /// Break a func_breakable now (spec "Breaking"); `dir` is the
    /// attack's travel direction (zero when unknown).
    pub fn break_entity(&mut self, id: EntId, breaker: Option<Who>, dir: Vec3) {
        let Some(b) = breakable(self, id) else { return };
        if b.broken || b.material == Material::UnbreakableGlass {
            return;
        }
        if b.window.is_some() {
            break_window(self, id, breaker, dir);
            return;
        }
        let material = b.material;
        let health = b.health;
        b.broken = true;
        if let Some(s) = material.break_sound() {
            self.breakable_sound(id, s);
        }
        let e = self.get(id).unwrap();
        let (origin, rot) = pose(self, id);
        let v0 = match e.kv_i("explosion") {
            1 => dir.normalize_or_zero() * GIB_DIRECTED_SPEED,
            2 => super::triggers::forward(e.vector_kv("gibdir")) * GIB_DIRECTED_SPEED,
            _ => Vec3::ZERO,
        };
        let set = e
            .kv("gibmodel")
            .filter(|g| !g.is_empty())
            .map_or_else(|| material.gibs().to_string(), str::to_string);
        let mode = e.kv_i("PerformanceMode");
        let (lo, hi) = local_bounds(self, id);
        let n = gib_count(hi - lo, mode);
        let _ = health;
        let mut pieces = Vec::with_capacity(n);
        for _ in 0..n {
            let local = Vec3::new(
                lo.x + (hi.x - lo.x) * self.random(),
                lo.y + (hi.y - lo.y) * self.random(),
                lo.z + (hi.z - lo.z) * self.random(),
            );
            let spin = if self.random() < 0.78 {
                Vec3::new(
                    self.random() * 511.0 - 256.0,
                    self.random() * 511.0 - 256.0,
                    self.random() * 511.0 - 256.0,
                )
            } else {
                Vec3::ZERO
            };
            let velocity = v0
                + Vec3::new(
                    (self.random() * 2.0 - 1.0) * GIB_SPEED_RAND,
                    (self.random() * 2.0 - 1.0) * GIB_SPEED_RAND,
                    self.random() * GIB_SPEED_RAND,
                );
            let life = 2.5 + self.random();
            pieces.push(Gib {
                position: origin + rot * local,
                velocity,
                spin,
                life,
            });
        }
        if !pieces.is_empty() {
            self.effects.push(Effect::Gibs {
                set,
                glass: material.is_glass(),
                pieces,
            });
        }
        self.unground_riders(id);
        if let Some(e) = self.get_mut(id) {
            e.targetname.clear();
        }
        if let Some(b) = breakable(self, id) {
            b.attach.push.solid = false;
        }
        self.refresh_solid(id);
        self.fire_output(id, "OnBreak", breaker, Value::Void);
        self.think_in(id, REMOVE_DELAY);
    }

    /// Players standing on it lose their ground.
    fn unground_riders(&mut self, id: EntId) {
        for p in self.players.iter_mut().filter(|p| p.ground == Some(id)) {
            p.unground = true;
            p.ground = None;
            p.on_ground = false;
        }
    }

    /// Break-on-touch breakables and broken windows touched by players
    /// (after movement).
    pub fn touch_breakables(&mut self) {
        for id in self.ids() {
            let Some(e) = self.get(id) else { continue };
            let Class::Breakable(b) = &e.class else { continue };
            if b.window.as_ref().is_some_and(|w| w.window_broken) {
                window_touch(self, id);
            } else if b.window.is_none() && !b.broken && e.has_flag(2) {
                break_on_touch(self, id);
            }
        }
    }
}

/// The world brushes of a breakable where it stands (it never moves).
fn placed(w: &LogicWorld, id: EntId) -> Vec<crate::map::MapBrush> {
    let Some(e) = w.get(id) else { return Vec::new() };
    let (origin, rot) = pose(w, id);
    e.hulls.iter().map(|h| place_hull(h, rot, origin)).collect()
}

fn break_on_touch(w: &mut LogicWorld, id: EntId) {
    let brushes = placed(w, id);
    let grow = Vec3::splat(1.0);
    for i in 0..w.players.len() {
        let p = w.players[i].clone();
        if !p.alive {
            continue;
        }
        let (lo, hi) = (p.origin + p.mins - grow, p.origin + p.maxs + grow);
        if !brushes.iter().any(|b| box_touches(b, lo, hi)) {
            continue;
        }
        let damage = p.velocity.length() * 0.01;
        let health = breakable(w, id).map_or(0, |b| b.health);
        if damage >= health as f32 {
            let who = Who::Player(p.entity);
            let (entity, dir) = (p.entity, p.velocity.normalize_or_zero());
            w.break_entity(id, Some(who), dir);
            w.effects.push(Effect::Damage {
                target: entity,
                amount: damage / 4.0,
                crush: false,
            });
            return;
        }
    }
}

// ------------------------------------------------------------ windows

fn window_mut(w: &mut LogicWorld, id: EntId) -> Option<&mut Window> {
    breakable(w, id).and_then(|b| b.window.as_deref_mut())
}

/// A trace hit on a window (spec "Taking a hit").
fn window_hit(
    w: &mut LogicWorld,
    id: EntId,
    amount: f32,
    kind: DamageKind,
    attacker: Option<Who>,
    point: Vec3,
    dir: Vec3,
) {
    if let Some(b) = breakable(w, id) {
        b.health = (b.health as f32 - amount).trunc() as i32;
        let h = b.health;
        w.fire_output(id, "OnHealthChanged", attacker, Value::Int(h));
    }
    let broken = w.window(id).is_some_and(|win| win.window_broken);
    if !broken {
        break_window(w, id, attacker, dir);
    }
    if w.get(id).is_none() {
        return;
    }
    let (origin, rot) = pose(w, id);
    let local = rot.inverse() * (point - origin);
    let local_dir = rot.inverse() * dir.normalize_or_zero();
    let Some(win) = w.window(id) else { return };
    let (cols, rows, glass) = (win.cols as i32, win.rows as i32, !win.tile);
    match kind {
        DamageKind::Bullet | DamageKind::Melee => {
            let at = win.pane_at(local);
            let (c, r) = (at.x.floor() as i32, at.y.floor() as i32);
            let (c, r) = (c.clamp(0, cols - 1), r.clamp(0, rows - 1));
            let force = local_dir * PANE_BULLET_FORCE;
            shatter(w, id, c, r, force);
            if glass {
                let (fx, fy) = (at.x - at.x.floor(), at.y - at.y.floor());
                if fx > 0.8 && c + 1 < cols {
                    shatter(w, id, c + 1, r, force);
                } else if fx < 0.2 && c > 0 {
                    shatter(w, id, c - 1, r, force);
                }
                if fy > 0.8 && r + 1 < rows {
                    shatter(w, id, c, r + 1, force);
                } else if fy < 0.2 && r > 0 {
                    shatter(w, id, c, r - 1, force);
                }
                if w.random() < 0.5 {
                    shatter(w, id, c, r + 1, local_dir * 1000.0);
                    if w.random() < 0.5 {
                        shatter(w, id, c, r + 2, local_dir * 1000.0);
                    }
                }
            }
        }
        DamageKind::Blast if glass => {
            for r in 0..rows {
                for c in 0..cols {
                    shatter(w, id, c, r, local_dir * 3000.0 * amount);
                }
            }
        }
        _ => {}
    }
}

/// Break the window entity (once): sound, OnBreak, non-solid.
fn break_window(w: &mut LogicWorld, id: EntId, breaker: Option<Who>, _dir: Vec3) {
    let Some(b) = breakable(w, id) else { return };
    let Some(win) = b.window.as_deref_mut() else { return };
    if win.window_broken {
        return;
    }
    win.window_broken = true;
    let tile = win.tile;
    b.health = 0;
    b.broken = true;
    b.attach.push.solid = false;
    w.breakable_sound(id, if tile { "Tile.Break" } else { "Glass.Break" });
    w.unground_riders(id);
    w.refresh_solid(id);
    let who = breaker.unwrap_or(Who::Ent(id));
    w.fire_output(id, "OnBreak", Some(who), Value::Void);
}

/// Shatter one pane (if there and unbroken): shards, the damage sound,
/// and a support pass this tick.
fn shatter(w: &mut LogicWorld, id: EntId, c: i32, r: i32, force: Vec3) {
    let tick = w.tick;
    let Some(win) = window_mut(w, id) else { return };
    if win.is_broken(c, r) {
        return;
    }
    let (c, r) = (c as usize, r as usize);
    let i = win.index(c, r);
    win.broken[i] = true;
    win.support[i] = 0.0;
    let (centre, normal, size, tile) = (
        win.pane_centre(c, r),
        win.normal,
        Vec2::new(win.u.length(), win.v.length()),
        win.tile,
    );
    let sound = win.sounded != tick;
    win.sounded = tick;
    if w.get(id).is_none() {
        return;
    }
    let (origin, rot) = pose(w, id);
    w.effects.push(Effect::PaneShatter {
        at: origin + rot * centre,
        normal: rot * normal,
        size,
        velocity: rot * force * 0.01,
        tile,
    });
    if sound {
        w.breakable_sound(id, "Breakable.MatGlass");
    }
    if !tile {
        // The support pass runs with the entity's next think: this tick's
        // when it has not thought yet, else the next.
        let due = w.get(id).and_then(|e| e.next_think);
        if due.is_none_or(|t| t > tick) {
            w.think_in(id, 0.0);
        }
    }
}

/// Panes a player's box overlaps shatter (spec "Touch").
fn window_touch(w: &mut LogicWorld, id: EntId) {
    let brushes = placed(w, id);
    if w.get(id).is_none() {
        return;
    }
    let (origin, rot) = pose(w, id);
    let Some(win) = w.window(id) else { return };
    let (cols, rows, tile) = (win.cols as i32, win.rows as i32, win.tile);
    let mut hits = Vec::new();
    for p in &w.players {
        if !p.alive {
            continue;
        }
        let (lo, hi) = (p.origin + p.mins, p.origin + p.maxs);
        if !brushes.iter().any(|b| box_touches(b, lo, hi)) {
            continue;
        }
        if tile && p.velocity.length() < 500.0 {
            continue;
        }
        // The box's corners in pane units.
        let (mut a, mut b) = (Vec2::MAX, Vec2::MIN);
        for k in 0..8 {
            let corner = Vec3::new(
                if k & 1 == 0 { lo.x } else { hi.x },
                if k & 2 == 0 { lo.y } else { hi.y },
                if k & 4 == 0 { lo.z } else { hi.z },
            );
            let q = win.pane_at(rot.inverse() * (corner - origin));
            a = a.min(q);
            b = b.max(q);
        }
        let c0 = (a.x.floor() as i32).clamp(0, cols);
        let c1 = (b.x.ceil() as i32).clamp(0, cols);
        let r0 = (a.y.floor() as i32).clamp(0, rows);
        let r1 = (b.y.ceil() as i32).clamp(0, rows);
        hits.push((c0, c1, r0, r1, rot.inverse() * p.velocity * 5.0));
    }
    for (c0, c1, r0, r1, force) in hits {
        for r in r0..r1 {
            if w.random() < 0.5 {
                shatter(w, id, c0 - 1, r, force);
            }
            for c in c0..c1 {
                shatter(w, id, c, r, force);
            }
            // Quirk: the pane right of the range is c1 + 1 (c1 skipped).
            if w.random() < 0.5 {
                shatter(w, id, c1 + 1, r, force);
            }
        }
    }
}

/// The support pass (glass): panes that lost support collapse; a collapse
/// runs the pass again next tick.
fn support_pass(w: &mut LogicWorld, id: EntId) {
    let Some(win) = w.window(id) else { return };
    if win.tile {
        return;
    }
    let supports = win.supports();
    let threshold = PANE_BREAK_SUPPORT * win.fragility / 100.0;
    let (cols, rows) = (win.cols, win.rows);
    let win = window_mut(w, id).unwrap();
    let mut fall = Vec::new();
    for r in 0..rows {
        for c in 0..cols {
            let i = r * cols + c;
            if win.broken[i] {
                continue;
            }
            win.support[i] = supports[i];
            if supports[i] < threshold {
                fall.push((c as i32, r as i32));
            }
        }
    }
    for (c, r) in &fall {
        // Half shatter where they are, half drop as a falling piece; both
        // are shards here (no falling pane model yet).
        let drop = w.random() < 0.5;
        shatter(
            w,
            id,
            *c,
            *r,
            if drop { Vec3::new(0.0, 0.0, -50.0) } else { Vec3::ZERO },
        );
    }
    if !fall.is_empty() {
        w.think_in(id, w.dt);
    }
}

/// The Shatter input: (x, y, r) as fractions across the window and a
/// radius in units.
fn shatter_input(w: &mut LogicWorld, id: EntId, value: &Value, activator: Option<Who>) {
    let v = match value {
        Value::Vector(v) => *v,
        other => crate::map::entities::parse_vector(&other.to_str(|_| String::new()).unwrap_or_default()),
    };
    break_window(w, id, activator, Vec3::ZERO);
    let Some(win) = w.window(id) else { return };
    let (cols, rows) = (win.cols, win.rows);
    let (pw, ph) = (win.u.length(), win.v.length());
    let centre = Vec2::new(v.x * cols as f32 * pw, v.y * rows as f32 * ph);
    for r in 0..rows {
        for c in 0..cols {
            let p = Vec2::new((c as f32 + 0.5) * pw, (r as f32 + 0.5) * ph);
            if p.distance(centre) <= v.z {
                shatter(w, id, c as i32, r as i32, Vec3::ZERO);
            }
        }
    }
}

// ---------------------------------------------------------- dispatch

pub(super) fn think(w: &mut LogicWorld, id: EntId) {
    let Some(b) = breakable(w, id) else { return };
    if b.window.is_some() {
        support_pass(w, id);
    } else if b.broken {
        w.kill(id);
    }
}

pub(super) fn input(w: &mut LogicWorld, id: EntId, input: &str, value: &Value, activator: Option<Who>) -> bool {
    match input {
        "break" => w.break_entity(id, activator, Vec3::ZERO),
        "sethealth" | "addhealth" | "removehealth" => {
            let Some(n) = w.need_float(value, input) else {
                return true;
            };
            let Some(b) = breakable(w, id) else { return true };
            if b.broken {
                return true;
            }
            let new = match input {
                "sethealth" => n,
                "addhealth" => b.health as f32 + n,
                _ => b.health as f32 - n,
            }
            .trunc() as i32;
            b.health = new;
            let ratio = (new as f32 / b.max_health as f32).clamp(0.0, 1.0);
            w.fire_output(id, "OnHealthChanged", activator, Value::Float(ratio));
            if new <= 0 {
                w.break_entity(id, activator, Vec3::ZERO);
            }
        }
        "shatter" if w.window(id).is_some() => shatter_input(w, id, value, activator),
        _ => return false,
    }
    true
}

#[cfg(test)]
#[path = "breakable_tests.rs"]
mod tests;
