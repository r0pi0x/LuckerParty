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
}

/// Takes no damage (the `god` command).
#[derive(Component, Reflect, Default, Clone, Copy, Debug)]
#[reflect(Component)]
pub struct God;

/// Something's health reached zero.
#[derive(Message, Clone, Debug)]
pub struct Died {
    pub entity: Entity,
    pub attacker: Option<Entity>,
}

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

/// Subtract damage from health; announce deaths once.
fn apply_damage(
    mut damage: MessageReader<Damage>,
    mut health: Query<&mut Health, Without<God>>,
    mut died: MessageWriter<Died>,
) {
    for d in damage.read() {
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
            });
        }
    }
}

/// Ordering of the fixed-tick simulation. Game plugins put their systems in
/// these sets so that, e.g., weapons always see this tick's movement state.
#[derive(SystemSet, Debug, Clone, PartialEq, Eq, Hash)]
pub enum SimSet {
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
            .add_message::<Damage>()
            .add_message::<Died>()
            .add_systems(FixedUpdate, apply_damage.after(SimSet::Weapons))
            .register_type::<SpawnPoint>()
            .register_type::<LocalPlayer>()
            .register_type::<SimTick>()
            .init_resource::<SimTick>()
            .add_systems(FixedFirst, |mut tick: ResMut<SimTick>| tick.0 += 1)
            .configure_sets(FixedUpdate, (SimSet::Movement, SimSet::Weapons).chain());
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
}

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
