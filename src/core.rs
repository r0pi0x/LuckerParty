//! Core vocabulary shared by every game plugin. Keep this small: a concept
//! moves in here only after two or three games need it (README, "rule of
//! three").

use bevy::prelude::*;

/// What a character wants to do this tick. Written by exactly one intent
/// source (local input, a bot brain, later the network) and read by the
/// Movement and Weapon slots. Buttons are level-based ("held"), not edges:
/// implementations that need edges (e.g. no auto-jump) track the previous
/// tick themselves.
#[derive(Component, Reflect, Default, Clone, Debug)]
#[reflect(Component)]
pub struct Intent {
    /// x = right, y = forward, each in [-1, 1]. Relative to `yaw`.
    pub move_axis: Vec2,
    /// Absolute look angles in radians. Yaw 0 looks down -Z; pitch up is +.
    pub yaw: f32,
    pub pitch: f32,
    pub jump: bool,
    pub crouch: bool,
    pub sprint: bool,
    /// The walk key (Source's +speed). Shift sets it along with `sprint`;
    /// each movement reads the one it has.
    pub walk: bool,
    pub fire: bool,
    /// Secondary attack (Source attack2: stab, zoom, burst toggle).
    pub secondary: bool,
    pub reload: bool,
    /// A weapon slot key held this tick (0 = first slot); the weapon slot
    /// switches when it changes.
    pub select: Option<u8>,
    /// Switch to the previously held weapon (Source `lastinv`).
    pub last_weapon: bool,
    /// The use key (Source `+use`, E): doors, buttons. Held level; the
    /// logic layer acts on presses.
    pub use_key: bool,
}

impl Intent {
    pub fn look_rotation(&self) -> Quat {
        Quat::from_euler(EulerRot::YXZ, self.yaw, self.pitch, 0.0)
    }

    pub fn yaw_rotation(&self) -> Quat {
        Quat::from_rotation_y(self.yaw)
    }
}

/// Character velocity in meters per second, owned by the active Movement
/// implementation.
#[derive(Component, Reflect, Default, Clone, Copy, Debug, Deref, DerefMut)]
#[reflect(Component)]
pub struct Velocity(pub Vec3);

/// Published by the active Movement implementation every tick. Other slots
/// (weapons, HUD, animation) read this instead of depending on a specific
/// movement implementation.
#[derive(Component, Reflect, Default, Clone, Debug)]
#[reflect(Component)]
pub struct MovementState {
    pub on_ground: bool,
    pub crouching: bool,
    pub sprinting: bool,
    /// Eye position relative to the character's origin, in meters.
    pub eye_offset: Vec3,
    /// The collision box relative to the character's origin (engine
    /// space, meters), when the movement has one (Source hulls); zero
    /// when it doesn't (the logic layer then uses the collider's bounds).
    pub hull_min: Vec3,
    pub hull_max: Vec3,
    /// The moving solid (`MovingSolid`) the character stands on, if any.
    pub ground: Option<Entity>,
}

/// A velocity that moves a character without being part of its own
/// velocity (Source base velocity: push triggers, leaving a moving
/// platform), engine space, m/s. Movement implementations that model it
/// read and consume it (specs/source/triggers.md, trigger_push).
#[derive(Component, Reflect, Default, Clone, Copy, Debug)]
#[reflect(Component)]
pub struct BaseVelocity {
    pub velocity: Vec3,
    /// Set this tick by a push (Source: FL_BASEVELOCITY); the movement
    /// clears it at the start of its next tick.
    pub touched: bool,
    /// Taken off the ground by game logic (push, teleport) this tick: the
    /// movement must not snap it back down before it moves.
    pub unground: bool,
}

/// Per-character gravity multiplier (Source entity gravity, set by
/// trigger_gravity); 0 means normal.
#[derive(Component, Reflect, Clone, Copy, Debug)]
#[reflect(Component)]
pub struct EntityGravity(pub f32);

impl Default for EntityGravity {
    fn default() -> Self {
        Self(1.0)
    }
}

/// A solid that moves (doors, platforms): its brushes where they are this
/// tick (engine space) and its velocity (m/s), for movement that sweeps
/// brushes exactly. Kept up to date by whoever moves it (the logic layer).
#[derive(Component, Clone, Debug, Default)]
pub struct MovingSolid {
    pub brushes: Vec<MapBrush>,
    pub velocity: Vec3,
    /// Solid to characters right now (doors with "Passable", disabled
    /// func_brush: false).
    pub solid: bool,
}

#[derive(Component, Reflect, Clone, Copy, Debug)]
#[reflect(Component)]
pub struct Health {
    /// 1.0 is a standard player's full health (README normalization rules).
    pub current: f32,
    pub max: f32,
}

impl Default for Health {
    fn default() -> Self {
        Self { current: 1.0, max: 1.0 }
    }
}

#[derive(Component, Reflect, Default, Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[reflect(Component)]
pub struct Team(pub u8);

/// Where on a body a hit landed (Source hitgroups).
#[derive(Reflect, Default, Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Hitgroup {
    #[default]
    Generic,
    Head,
    Chest,
    Stomach,
    LeftArm,
    RightArm,
    LeftLeg,
    RightLeg,
}

/// Damage to apply: normalized like `Health` (1.0 = a standard player's
/// full health).
#[derive(Message, Clone, Debug)]
pub struct Damage {
    pub target: Entity,
    pub attacker: Option<Entity>,
    pub amount: f32,
    pub point: Vec3,
    /// Direction the damage travelled (unit).
    pub dir: Vec3,
    pub hitgroup: Hitgroup,
    /// How it was dealt (breakables scale damage by it).
    pub kind: DamageKind,
}

/// How damage was dealt (Source damage types, as far as anything here
/// tells them apart).
#[derive(Reflect, Default, Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum DamageKind {
    #[default]
    Generic,
    /// Hitscan shots.
    Bullet,
    /// Melee swings (the knife).
    Melee,
    Blast,
    /// Crushed by a mover.
    Crush,
    Fall,
}

/// Takes `Damage` without having `Health`: something else (the logic
/// layer's breakables) reads the messages aimed at it. Weapons hit it as
/// a plain object (no hitgroups, no flesh sounds).
#[derive(Component, Reflect, Default, Clone, Copy, Debug)]
#[reflect(Component)]
pub struct Damageable;

/// An oriented box on a character's body that shots test (Source
/// hitboxes), in the character's local space: feet at the origin, Y up,
/// facing -Z (yaw 0), meters.
#[derive(Clone, Copy, Debug, PartialEq, Reflect)]
pub struct Hitbox {
    pub center: Vec3,
    pub half: Vec3,
    pub rotation: Quat,
    pub group: Hitgroup,
}

impl Hitbox {
    /// Where a ray (local space) enters the box, as a distance along `dir`
    /// (unit), if it does.
    pub fn ray_entry(&self, origin: Vec3, dir: Vec3) -> Option<f32> {
        self.ray_span(origin, dir).map(|(enter, _)| enter)
    }

    /// Where a ray (local space) enters and leaves the box, as distances
    /// along `dir` (unit; the entry clamped at 0), if it crosses it.
    pub fn ray_span(&self, origin: Vec3, dir: Vec3) -> Option<(f32, f32)> {
        let inv = self.rotation.inverse();
        let o = inv * (origin - self.center);
        let d = inv * dir;
        let (mut enter, mut leave) = (f32::MIN, f32::MAX);
        for i in 0..3 {
            if d[i].abs() < 1e-9 {
                if o[i].abs() > self.half[i] {
                    return None;
                }
                continue;
            }
            let (a, b) = ((-self.half[i] - o[i]) / d[i], (self.half[i] - o[i]) / d[i]);
            enter = enter.max(a.min(b));
            leave = leave.min(a.max(b));
        }
        (enter <= leave && leave >= 0.0).then_some((enter.max(0.0), leave))
    }
}

/// A character's hitboxes (see `Hitbox`).
#[derive(Component, Clone, Debug, Default, Reflect)]
#[reflect(Component)]
pub struct Hitboxes(pub Vec<Hitbox>);

/// Takes no damage (the `god` command).
#[derive(Component, Reflect, Default, Clone, Copy, Debug)]
#[reflect(Component)]
pub struct God;

/// Something's health reached zero.
#[derive(Message, Clone, Debug)]
pub struct Died {
    pub entity: Entity,
    pub attacker: Option<Entity>,
    /// The hit that killed it (direction and point push its ragdoll).
    pub damage: Damage,
}

/// Physics layer of ragdoll bodies. They collide only with the default
/// layer (the world, props), not with characters or each other, and
/// gameplay queries (movement, bullets, sight) leave them out with
/// `SOLID_LAYERS` (specs/cs_source/ragdolls.md 4: ragdolls are debris).
pub const RAGDOLL_LAYER: avian3d::prelude::LayerMask = avian3d::prelude::LayerMask(1 << 1);
/// Every layer but ragdolls: the mask for gameplay spatial queries.
pub const SOLID_LAYERS: avian3d::prelude::LayerMask = avian3d::prelude::LayerMask(!(1 << 1));

/// A speed cap the character's equipment imposes (e.g. the held weapon),
/// meters per second. Movement implementations that model it read it.
#[derive(Component, Reflect, Clone, Copy, Debug)]
#[reflect(Component)]
pub struct MaxSpeed(pub f32);

/// A place a character can spawn, provided by the Map slot.
#[derive(Component, Reflect, Default, Clone, Copy, Debug)]
#[reflect(Component)]
pub struct SpawnPoint {
    pub team: Option<Team>,
}

/// The character controlled by this machine's keyboard and mouse.
#[derive(Component, Reflect, Default)]
#[reflect(Component)]
pub struct LocalPlayer;

/// Number of fixed ticks simulated so far. Tests and tools use it to step
/// the simulation by exact ticks.
#[derive(Resource, Reflect, Default, Clone, Copy, Debug)]
#[reflect(Resource)]
pub struct SimTick(pub u64);

/// Round restarts so far (the rules count one up when a new round
/// starts). Each count puts the map's entities back as they spawned:
/// the logic layer re-creates its world, the map layer makes broken
/// brushes whole and clears gibs (Counter-Strike's round restart).
#[derive(Resource, Default, Clone, Copy, Debug, PartialEq, Eq)]
pub struct RoundRestarts(pub u32);

/// Whether characters on the same team hurt each other (CS:S
/// `mp_friendlyfire`, default 0). Team 0 (no team) is never friendly.
#[derive(Resource, Default, Clone, Copy, Debug, PartialEq, Eq)]
pub struct FriendlyFire(pub u8);

/// Subtract damage from health; announce deaths once (public so other
/// systems can order themselves after it).
pub fn apply_damage(
    mut damage: MessageReader<Damage>,
    mut health: Query<&mut Health, Without<God>>,
    teams: Query<&Team>,
    friendly_fire: Option<Res<FriendlyFire>>,
    mut died: MessageWriter<Died>,
) {
    let friendly_fire = friendly_fire.is_some_and(|f| f.0 != 0);
    for d in damage.read() {
        // Teammates don't hurt each other unless friendly fire is on (your
        // own damage, e.g. falling, always counts).
        let teammate = d
            .attacker
            .filter(|a| *a != d.target)
            .is_some_and(|a| matches!((teams.get(a), teams.get(d.target)), (Ok(x), Ok(y)) if x == y && x.0 != 0));
        if teammate && !friendly_fire {
            continue;
        }
        let Ok(mut h) = health.get_mut(d.target) else { continue };
        if h.current <= 0.0 {
            continue;
        }
        h.current -= d.amount;
        if h.current <= 0.0 {
            h.current = 0.0;
            died.write(Died {
                entity: d.target,
                attacker: d.attacker,
                damage: d.clone(),
            });
        }
    }
}

/// Ordering of the fixed-tick simulation. Game plugins put their systems in
/// these sets so that, e.g., weapons always see this tick's movement state.
#[derive(SystemSet, Debug, Clone, PartialEq, Eq, Hash)]
pub enum SimSet {
    /// Match rules (rounds, respawns), before everything that acts on
    /// them this tick.
    Rules,
    Movement,
    Weapons,
}

pub struct CorePlugin;

impl Plugin for CorePlugin {
    fn build(&self, app: &mut App) {
        app.register_type::<Intent>()
            .register_type::<Velocity>()
            .register_type::<MovementState>()
            .register_type::<Health>()
            .register_type::<Team>()
            .register_type::<MaxSpeed>()
            .register_type::<BaseVelocity>()
            .register_type::<EntityGravity>()
            .register_type::<Hitboxes>()
            .register_type::<Damageable>()
            .add_message::<Damage>()
            .add_message::<Died>()
            .init_resource::<FriendlyFire>()
            .add_systems(FixedUpdate, apply_damage.after(SimSet::Weapons))
            .register_type::<SpawnPoint>()
            .register_type::<LocalPlayer>()
            .register_type::<SimTick>()
            .init_resource::<SimTick>()
            .add_systems(FixedFirst, |mut tick: ResMut<SimTick>| tick.0 += 1)
            .init_resource::<RoundRestarts>()
            .configure_sets(FixedUpdate, (SimSet::Rules, SimSet::Movement, SimSet::Weapons).chain());
    }
}

// Collision world shared by maps and movement: exact brushes for swept-box
// movement, and water volumes.

/// A solid convex volume as planes: a point p is inside when n.p <= d for
/// every plane (n, d). Engine space, meters; `min`/`max` bound it. Games
/// whose movement sweeps boxes against brushes (Source) use these instead
/// of the physics engine's shape casts, which lose precision on large
/// volumes.
#[derive(Clone, Debug)]
pub struct MapBrush {
    pub planes: Vec<(Vec3, f32)>,
    pub min: Vec3,
    pub max: Vec3,
    /// Climbable (Source: ladder contents).
    pub ladder: bool,
    /// Surface property name (footsteps), lower-case, for brushes that
    /// carry their own (props). None: look it up from the world's faces.
    pub surface: Option<String>,
}

impl MapBrush {
    /// An axis-aligned box.
    pub fn from_box(min: Vec3, max: Vec3) -> Self {
        let planes = vec![
            (Vec3::X, max.x),
            (Vec3::NEG_X, -min.x),
            (Vec3::Y, max.y),
            (Vec3::NEG_Y, -min.y),
            (Vec3::Z, max.z),
            (Vec3::NEG_Z, -min.z),
        ];
        Self {
            planes,
            min,
            max,
            ladder: false,
            surface: None,
        }
    }

    /// Whether a point (engine space) is inside.
    pub fn contains(&self, p: Vec3) -> bool {
        p.cmpge(self.min).all() && p.cmple(self.max).all() && self.planes.iter().all(|(n, d)| n.dot(p) <= *d)
    }

    /// Whether an axis-aligned box (centre, half size) overlaps the brush
    /// by more than `skin` (any space, as long as the brush uses it too).
    pub fn overlaps_box(&self, centre: Vec3, half: Vec3, skin: f32) -> bool {
        self.max.cmpgt(centre - half).all()
            && self.min.cmplt(centre + half).all()
            && self
                .planes
                .iter()
                .all(|(n, d)| n.dot(centre) - (d + n.abs().dot(half)) < -skin)
    }

    /// Sweep an axis-aligned box (half size `half`) with its centre from
    /// `from` to `to`; hits stop `eps` short along the plane. Returns the
    /// fraction travelled and the hit plane's normal when the box hits
    /// (fraction 0 with a zero normal when it starts inside and stays in).
    pub fn sweep_box(&self, half: Vec3, from: Vec3, to: Vec3, eps: f32) -> Option<(f32, Vec3)> {
        let lo = from.min(to) - half - Vec3::splat(eps);
        let hi = from.max(to) + half + Vec3::splat(eps);
        if self.max.cmplt(lo).any() || self.min.cmpgt(hi).any() {
            return None;
        }
        let (mut enter, mut leave) = (-1.0f32, 1.0f32);
        let (mut starts_out, mut gets_out) = (false, false);
        let mut clip = Vec3::ZERO;
        for (n, d) in &self.planes {
            let dist = d + n.abs().dot(half);
            let d1 = n.dot(from) - dist;
            let d2 = n.dot(to) - dist;
            gets_out |= d2 > 0.0;
            starts_out |= d1 > 0.0;
            if d1 > 0.0 && (d2 >= eps || d2 >= d1) {
                return None;
            }
            if d1 <= 0.0 && d2 <= 0.0 {
                continue;
            }
            if d1 > d2 {
                let f = ((d1 - eps) / (d1 - d2)).max(0.0);
                if f > enter {
                    enter = f;
                    clip = *n;
                }
            } else {
                leave = leave.min(((d1 + eps) / (d1 - d2)).min(1.0));
            }
        }
        if !starts_out {
            return (!gets_out).then_some((0.0, Vec3::ZERO));
        }
        (enter < leave && enter > -1.0).then_some((enter.max(0.0), clip))
    }

    /// The brush moved: rotated by `rotation` about the origin, then
    /// moved by `offset` (planes and bounds; `points` are its corners
    /// before the move, for the new bounds).
    pub fn transformed(&self, points: &[Vec3], rotation: Quat, offset: Vec3) -> Self {
        let planes = self
            .planes
            .iter()
            .map(|(n, d)| {
                let n = rotation * *n;
                (n, d + n.dot(offset))
            })
            .collect();
        let (mut min, mut max) = (Vec3::MAX, Vec3::MIN);
        for p in points {
            let q = rotation * *p + offset;
            min = min.min(q);
            max = max.max(q);
        }
        if points.is_empty() {
            (min, max) = (self.min + offset, self.max + offset);
        }
        Self {
            planes,
            min,
            max,
            ladder: self.ladder,
            surface: self.surface.clone(),
        }
    }
}

impl MapBrush {
    /// A triangle as a thin solid: the triangle's face (normal `(b - a) x
    /// (c - a)`), a back face `thickness` behind it, its three sides, and
    /// the bevel planes a box sweep needs to be exact (the box's axes and
    /// each edge crossed with each axis), all as supporting planes.
    pub fn from_triangle(a: Vec3, b: Vec3, c: Vec3, thickness: f32) -> Option<Self> {
        let n = (b - a).cross(c - a).try_normalize()?;
        let back = n * -thickness;
        let corners = [a, b, c, a + back, b + back, c + back];
        let edges = [b - a, c - b, a - c, n];
        let mut axes: Vec<Vec3> = vec![Vec3::X, Vec3::Y, Vec3::Z, n];
        for e in &edges[..3] {
            axes.push(n.cross(*e));
        }
        for e in edges {
            for axis in [Vec3::X, Vec3::Y, Vec3::Z] {
                axes.push(e.cross(axis));
            }
        }
        let mut planes: Vec<(Vec3, f32)> = Vec::with_capacity(axes.len() * 2);
        for axis in axes {
            let Some(m) = axis.try_normalize() else { continue };
            for m in [m, -m] {
                if planes.iter().any(|(q, _)| q.dot(m) > 1.0 - 1e-6) {
                    continue;
                }
                let d = corners.iter().map(|p| m.dot(*p)).fold(f32::MIN, f32::max);
                planes.push((m, d));
            }
        }
        // The face first: sweeps that tie report the earliest plane.
        let face = planes.iter().position(|(m, _)| m.dot(n) > 1.0 - 1e-6)?;
        planes.swap(0, face);
        let min = corners.iter().fold(Vec3::MAX, |acc, p| acc.min(*p));
        let max = corners.iter().fold(Vec3::MIN, |acc, p| acc.max(*p));
        Some(Self {
            planes,
            min,
            max,
            ladder: false,
            surface: None,
        })
    }
}

/// The loaded map's terrain triangles (Source displacements) as thin
/// solids (`MapBrush::from_triangle`), with a grid to find the ones near a
/// box quickly. Movement that sweeps brushes exactly sweeps these the same
/// way instead of casting against the terrain's physics collider.
#[derive(Resource, Clone, Debug, Default)]
pub struct MapTerrain {
    pub brushes: Vec<MapBrush>,
    /// Grid over engine x and z: origin, cell size and counts.
    origin: Vec2,
    cell: f32,
    size: (usize, usize),
    cells: Vec<Vec<u32>>,
}

impl MapTerrain {
    /// Grid cell size, meters.
    const CELL: f32 = 2.0;

    pub fn from_triangles(triangles: impl IntoIterator<Item = [Vec3; 3]>, thickness: f32) -> Self {
        let brushes: Vec<MapBrush> = triangles
            .into_iter()
            .filter_map(|[a, b, c]| MapBrush::from_triangle(a, b, c, thickness))
            .collect();
        if brushes.is_empty() {
            return Self::default();
        }
        let lo = brushes.iter().fold(Vec2::MAX, |acc, b| acc.min(b.min.xz()));
        let hi = brushes.iter().fold(Vec2::MIN, |acc, b| acc.max(b.max.xz()));
        let size = (
            ((hi.x - lo.x) / Self::CELL).floor() as usize + 1,
            ((hi.y - lo.y) / Self::CELL).floor() as usize + 1,
        );
        let mut terrain = Self {
            brushes,
            origin: lo,
            cell: Self::CELL,
            size,
            cells: vec![Vec::new(); size.0 * size.1],
        };
        for i in 0..terrain.brushes.len() {
            let (x0, z0, x1, z1) = terrain.cell_range(terrain.brushes[i].min, terrain.brushes[i].max);
            for z in z0..=z1 {
                for x in x0..=x1 {
                    terrain.cells[z * size.0 + x].push(i as u32);
                }
            }
        }
        terrain
    }

    fn cell_range(&self, lo: Vec3, hi: Vec3) -> (usize, usize, usize, usize) {
        let to = |v: f32, o: f32, n: usize| (((v - o) / self.cell).floor().max(0.0) as usize).min(n - 1);
        (
            to(lo.x, self.origin.x, self.size.0),
            to(lo.z, self.origin.y, self.size.1),
            to(hi.x, self.origin.x, self.size.0),
            to(hi.z, self.origin.y, self.size.1),
        )
    }

    /// Indices of brushes whose bounds may overlap `lo`..`hi` (sorted,
    /// no repeats), into `out`.
    pub fn near(&self, lo: Vec3, hi: Vec3, out: &mut Vec<u32>) {
        out.clear();
        if self.cells.is_empty()
            || hi.x < self.origin.x
            || hi.z < self.origin.y
            || lo.x > self.origin.x + self.cell * self.size.0 as f32
            || lo.z > self.origin.y + self.cell * self.size.1 as f32
        {
            return;
        }
        let (x0, z0, x1, z1) = self.cell_range(lo, hi);
        for z in z0..=z1 {
            for x in x0..=x1 {
                out.extend(self.cells[z * self.size.0 + x].iter().copied().filter(|&i| {
                    let b = &self.brushes[i as usize];
                    b.max.cmpge(lo).all() && b.min.cmple(hi).all()
                }));
            }
        }
        out.sort_unstable();
        out.dedup();
    }
}

/// Marks the physics collider built from the terrain triangles
/// (`MapTerrain`), so movement that sweeps them exactly can leave it out
/// of physics queries.
#[derive(Component, Debug)]
pub struct MapTerrainCollider;

/// A water (or slime) volume, for swimming.
#[derive(Clone, Debug)]
pub struct MapWaterVolume {
    pub brush: MapBrush,
    pub slime: bool,
}

/// The loaded map's water volumes (`MapData::water`).
#[derive(Resource, Clone, Debug, Default)]
pub struct MapWater(pub Vec<MapWaterVolume>);

/// The loaded map's brushes (`MapData::collision_brushes`).
#[derive(Resource, Clone, Debug, Default)]
pub struct MapBrushes(pub Vec<MapBrush>);

/// Marks the physics collider built from the same brushes, so movement that
/// sweeps `MapBrushes` itself can leave it out of physics queries.
#[derive(Component, Debug)]
pub struct MapBrushCollider;

/// A prop collider's surface property name (lower-case), reported by traces
/// that hit it (footsteps).
#[derive(Component, Clone, Debug)]
pub struct PropSurface(pub String);
