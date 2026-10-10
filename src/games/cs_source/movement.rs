//! Source player movement, from specs/cs_source/movement.md: a swept-box
//! kinematic controller. Friction and acceleration toward a wish direction,
//! air acceleration with the 30-unit cap that makes air-strafing work,
//! split gravity, jumping, ground detection, slide moves with crease
//! handling, stair stepping, sticking to slopes and stairs, and ducking.
//!
//! The maths runs in Source units (inches, Z up) like the spec; the
//! character's `Transform` (engine meters, Y up) is the centre of the
//! standing box, so feet = origin - 36 units. Ladders and water (swimming,
//! water jumps) follow the spec's sections. Not implemented yet: base
//! velocity (conveyors, water currents). Landings and wall slams roll the
//! view (the punch angle's roll, `MovementState::view_roll`); hard
//! landings deal fall damage (specs/cs_source/fall_damage.md) and play
//! its sound.

use avian3d::prelude::*;
use bevy::prelude::*;

use crate::{
    core::{
        BaseVelocity, Damage, EntityGravity, Health, Hitgroup, Intent, MapBrush, MaxSpeed, MovementState, MovingSolid,
        PredictedAppExt, Velocity,
    },
    map::{
        MapBrushCollider, MapBrushTree, MapBrushes, MapTerrain, MapTerrainCollider, MapWater, PhysicsProp, PlaySound, PropSurface,
        PushAway,
        sound::{SoundBank, SurfaceGrid},
    },
    slots::RegisterSlots,
};

pub const ID: &str = "cs_source:movement";

const METERS_PER_UNIT: f32 = 0.0254;

/// Console variables and per-player values the movement reads. `Default`
/// is CS:S as measured on the reference install (RCON, 2026-10-05; see the
/// spec's "CS:S values"); `shared_code` is the SDK's values, which the
/// spec's test cases use.
#[derive(Resource, Clone, Debug)]
pub struct SourceMovementConfig {
    pub accelerate: f32,
    pub airaccelerate: f32,
    pub friction: f32,
    pub stopspeed: f32,
    pub gravity: f32,
    pub maxspeed: f32,
    pub stepsize: f32,
    pub maxvelocity: f32,
    pub bounce: f32,
    /// Jump speed, units/s.
    pub jump_impulse: f32,
    /// Max speed without a weapon's `MaxSpeed` (the knife's 250).
    pub player_maxspeed: f32,
    /// What a full move key sends (cl_forwardspeed etc.), rescaled to the max speed.
    pub key_speed: f32,
    /// Move input scale while ducked on the ground.
    pub duck_speed: f32,
    /// Collision box heights and eye heights above the feet, units.
    pub stand_height: f32,
    pub duck_height: f32,
    pub eye_stand: f32,
    pub eye_duck: f32,
    /// How much of the height difference the feet move when ducking or
    /// unducking in the air (1 = head stays put).
    pub air_duck_shift: f32,
    /// CS:S jump stamina, ms: set by each jump, counting down in real time.
    /// While it lasts, jump speed and ground speed are scaled down by
    /// 1 - coefficient x stamina. 0 turns it off (the shared code has none).
    pub jump_stamina: f32,
    pub stamina_jump_scale: f32,
    pub stamina_ground_scale: f32,
    /// sv_enablebunnyhopping: when false, a jump caps the player's speed
    /// (3D) at `bunnyhop_cap`.
    pub enable_bunnyhopping: bool,
    pub bunnyhop_cap: f32,
    /// sv_autobunnyhopping: holding jump jumps again on landing.
    pub auto_bunnyhopping: bool,
    /// Max speed scale while the walk key (+speed) is held: CS:S walks at
    /// 0.52 x the weapon speed (130 with the knife), on the ground, in the
    /// air and in water; the duck scale applies on top (measured with
    /// movecmp).
    pub walk_speed: f32,
    /// sv_ladder_dampen / sv_ladder_angle (CS:S): sideways ladder input is
    /// scaled by `ladder_dampen` when the push is mostly into the ladder
    /// (the angle test's dot below `ladder_angle`). Dampen 1 turns it off.
    pub ladder_dampen: f32,
    pub ladder_angle: f32,
    /// The player's own max speed property, used by swimming's upward
    /// push (holding jump, looking up). CS:S: 260, the base player speed,
    /// whatever the weapon (measured with movecmp fuzz).
    pub client_maxspeed: f32,
    /// CS:S: ladder speed gets the same duck scale as move input (see
    /// `duck_slows_everywhere`; measured; the shared code leaves climbing
    /// speed alone).
    pub duck_slows_ladder: bool,
    /// CS:S: no footsteps while walking (+speed) or ducked. Players know it
    /// and the shared code's speed bands contradict it; unmeasured (sound
    /// spec open question 3).
    pub silent_walk_duck: bool,
    /// CS:S: the ducked input scale applies while duck is held, or when the
    /// tick started ducked or mid-duck: from the tick duck is pressed until
    /// the tick after it's released, in the air and water too (measured
    /// with movecmp fuzz). The shared code scales only when already ducked
    /// on the ground.
    pub duck_slows_everywhere: bool,
    /// Fall damage (specs/cs_source/fall_damage.md): landing faster than
    /// `fall_safe` (units/s) out of water deals (speed - safe) x
    /// `fall_damage_per_speed` health points, truncated to whole points.
    pub fall_safe: f32,
    pub fall_damage_per_speed: f32,
}

/// Console variables players and server configs know, mapped onto the
/// config: (name, what it sets).
pub const CVARS: &[(&str, &str)] = &[
    ("sv_accelerate", "ground acceleration"),
    ("sv_airaccelerate", "air acceleration (surf servers raise it, e.g. 150)"),
    ("sv_friction", "ground friction"),
    ("sv_stopspeed", "speed below which friction stops at a constant rate"),
    ("sv_gravity", "gravity, units/s^2"),
    ("sv_maxspeed", "upper limit on move speed"),
    ("sv_stepsize", "highest step climbed without jumping"),
    ("sv_maxvelocity", "per-axis velocity limit"),
    ("sv_bounce", "wall bounce (0 in CS:S)"),
    ("sv_enablebunnyhopping", "1 removes the jump speed cap"),
    ("sv_autobunnyhopping", "1: holding jump keeps jumping"),
    ("cl_forwardspeed", "what a held move key sends"),
    (
        "sv_ladder_dampen",
        "sideways ladder input kept when climbing (CS:S 0.2)",
    ),
    (
        "sv_ladder_angle",
        "how squarely into a ladder the dampening starts (CS:S -0.707)",
    ),
];

impl SourceMovementConfig {
    /// Set a console variable by name, as a server config would.
    pub fn set_cvar(&mut self, name: &str, value: &str) -> Result<(), String> {
        let num = || {
            value
                .trim()
                .trim_matches('"')
                .parse::<f32>()
                .map_err(|_| format!("{name}: not a number: {value}"))
        };
        match name.to_ascii_lowercase().as_str() {
            "sv_accelerate" => self.accelerate = num()?,
            "sv_airaccelerate" => self.airaccelerate = num()?,
            "sv_friction" => self.friction = num()?,
            "sv_stopspeed" => self.stopspeed = num()?,
            "sv_gravity" => self.gravity = num()?,
            "sv_maxspeed" => self.maxspeed = num()?,
            "sv_stepsize" => self.stepsize = num()?,
            "sv_maxvelocity" => self.maxvelocity = num()?,
            "sv_bounce" => self.bounce = num()?,
            "sv_enablebunnyhopping" => self.enable_bunnyhopping = num()? != 0.0,
            "sv_autobunnyhopping" => self.auto_bunnyhopping = num()? != 0.0,
            "cl_forwardspeed" | "cl_sidespeed" => self.key_speed = num()?,
            "sv_ladder_dampen" => self.ladder_dampen = num()?,
            "sv_ladder_angle" => self.ladder_angle = num()?,
            other => return Err(format!("unknown console variable {other}")),
        }
        Ok(())
    }

    /// A console variable's value, as `set_cvar` reads it.
    pub fn get_cvar(&self, name: &str) -> Option<String> {
        let b = |v: bool| if v { "1" } else { "0" }.to_string();
        let f = |v: f32| {
            let s = format!("{v}");
            if s.contains('.') {
                s.trim_end_matches('0').trim_end_matches('.').to_string()
            } else {
                s
            }
        };
        Some(match name.to_ascii_lowercase().as_str() {
            "sv_accelerate" => f(self.accelerate),
            "sv_airaccelerate" => f(self.airaccelerate),
            "sv_friction" => f(self.friction),
            "sv_stopspeed" => f(self.stopspeed),
            "sv_gravity" => f(self.gravity),
            "sv_maxspeed" => f(self.maxspeed),
            "sv_stepsize" => f(self.stepsize),
            "sv_maxvelocity" => f(self.maxvelocity),
            "sv_bounce" => f(self.bounce),
            "sv_enablebunnyhopping" => b(self.enable_bunnyhopping),
            "sv_autobunnyhopping" => b(self.auto_bunnyhopping),
            "cl_forwardspeed" | "cl_sidespeed" => f(self.key_speed),
            "sv_ladder_dampen" => f(self.ladder_dampen),
            "sv_ladder_angle" => f(self.ladder_angle),
            _ => return None,
        })
    }

    /// Apply a Source-style config (one `name value` per line, `//`
    /// comments); unknown variables are reported, not fatal.
    pub fn exec(&mut self, text: &str) -> Vec<String> {
        let mut problems = Vec::new();
        for line in text.lines() {
            let line = line.split("//").next().unwrap_or("").trim();
            let mut parts = line.splitn(2, char::is_whitespace);
            let (Some(name), Some(value)) = (parts.next(), parts.next()) else {
                continue;
            };
            if let Err(e) = self.set_cvar(name, value) {
                problems.push(e);
            }
        }
        problems
    }

    /// Feet movement for a duck or unduck in the air.
    fn air_duck_lift(&self) -> f32 {
        (self.stand_height - self.duck_height) * self.air_duck_shift
    }
}

impl SourceMovementConfig {
    /// The SDK 2013 shared movement code's values.
    pub fn shared_code() -> Self {
        Self {
            accelerate: 5.0,
            airaccelerate: 10.0,
            friction: 4.0,
            stopspeed: 100.0,
            gravity: 800.0,
            maxspeed: 320.0,
            stepsize: 18.0,
            maxvelocity: 3500.0,
            bounce: 0.0,
            jump_impulse: 268.328_16,
            player_maxspeed: 250.0,
            key_speed: 450.0,
            duck_speed: 1.0 / 3.0,
            walk_speed: 0.52,
            stand_height: 72.0,
            duck_height: 36.0,
            eye_stand: 64.0,
            eye_duck: 28.0,
            air_duck_shift: 1.0,
            jump_stamina: 0.0,
            stamina_jump_scale: 0.0,
            stamina_ground_scale: 0.0,
            enable_bunnyhopping: true,
            bunnyhop_cap: 0.0,
            auto_bunnyhopping: false,
            // The dampening is compiled only for CS:S; it's off here.
            ladder_dampen: 1.0,
            ladder_angle: -0.707,
            client_maxspeed: 250.0,
            silent_walk_duck: false,
            duck_slows_ladder: false,
            duck_slows_everywhere: false,
            fall_safe: 580.0,
            fall_damage_per_speed: 100.0 / (1024.0 - 580.0),
        }
    }
}

impl Default for SourceMovementConfig {
    /// CS:S: cvars read over RCON; jump speed sqrt(2 * 800 * 57), which
    /// reproduces the measured standing-jump apex (54.75 units at the
    /// server's 66.67 tick).
    fn default() -> Self {
        Self {
            stopspeed: 75.0,
            jump_impulse: (2.0f32 * 800.0 * 57.0).sqrt(),
            key_speed: 400.0,
            // Measured with movecmp: ducked acceleration implies a wish speed
            // of 85 at max speed 250.
            duck_speed: 0.34,
            // Measured with the probe (the player's box and view offset):
            // CS:S stands 62 tall (eye 64) and ducks to 45 (eye 47); in the
            // air the feet move half the difference, 8.5 units.
            stand_height: 62.0,
            duck_height: 45.0,
            eye_stand: 64.0,
            eye_duck: 47.0,
            air_duck_shift: 0.5,
            // Measured with the probe (run/jump/land traces): stamina
            // 1315.79 ms per jump, 15 ms off per tick; jump speed x (1 -
            // 0.00019 T); ground speed x (1 - 0.000199 T) each tick between
            // friction and acceleration. The landing slowdown and the quick
            // stop after a jump.
            jump_stamina: 1315.789_4,
            stamina_jump_scale: 0.000_19,
            stamina_ground_scale: 0.000_199,
            // Measured: with sv_enablebunnyhopping 0, a jump scales the
            // velocity (3D, including this tick's gravity) down to 286 when
            // faster, whatever the weapon (1.1 x 260, the fastest weapon).
            enable_bunnyhopping: false,
            bunnyhop_cap: 286.0,
            ladder_dampen: 0.2,
            client_maxspeed: 260.0,
            silent_walk_duck: true,
            duck_slows_ladder: true,
            duck_slows_everywhere: true,
            // Measured on the probe server: 100 damage at 996, not 1024.
            fall_damage_per_speed: 100.0 / 416.0,
            ..Self::shared_code()
        }
    }
}

const HALF_WIDTH: f32 = 16.0;
/// The transform sits at the standing box's centre.
/// The transform sits this far above the feet (a fixed convention, the
/// shared code's standing-box centre), whatever the hull sizes.
const ORIGIN_ABOVE_FEET: f32 = 36.0;
/// Landing view roll per unit/s of fall speed (degrees), and the punch
/// spring's damping and constant (movement spec).
const LAND_PUNCH_SCALE: f32 = 0.013;
const PUNCH_DAMPING: f32 = 9.0;
const PUNCH_SPRING: f32 = 65.0;

const AIR_WISH_CAP: f32 = 30.0;
const WALKABLE_NORMAL_Z: f32 = 0.7;
const GROUND_PROBE: f32 = 2.0;
const LEAVE_GROUND_VZ: f32 = 140.0;
const UNGROUND_VZ: f32 = 250.0;
const MAX_CLIP_PLANES: usize = 5;
const BUMPS: usize = 4;
const STEP_EPSILON: f32 = 0.031_25;
const STAY_ON_GROUND_UP: f32 = 2.0;
const SNAP_MIN: f32 = 1.0 / 64.0;
const DUCK_TIMER_START: f32 = 1000.0;
const TIME_TO_DUCK: f32 = 0.4;
const TIME_TO_UNDUCK: f32 = 0.2;
const UPWARD_AIR_FRICTION: f32 = 0.25;

// Ladders and water (spec "Ladders", "Water").
const LADDER_REACH: f32 = 2.0;
const LADDER_SPEED: f32 = 200.0;
const LADDER_JUMP_OFF: f32 = 270.0;
const LADDER_BACK_OFF: f32 = 200.0;
const WATER_FEET_PROBE: f32 = 1.0;
const SWIM_SINK: f32 = 60.0;
const SWIM_WISH_SCALE: f32 = 0.8;
const SWIM_JUMP_WATER: f32 = 100.0;
const SWIM_JUMP_SLIME: f32 = 80.0;
const WATER_JUMP_UP: f32 = 256.0;
const WATER_JUMP_PUSH: f32 = 50.0;
const WATER_JUMP_REACH: f32 = 24.0;
const WATER_JUMP_EYE_EXTRA: f32 = 8.0;
const WATER_JUMP_DROP: f32 = 1024.0;
const WATER_JUMP_MIN_VZ: f32 = -180.0;
const WATER_JUMP_TIME: f32 = 2000.0;
/// Source's body channel (footsteps, swim): a new one replaces the last.
const CHAN_BODY: u8 = 4;

/// How far sweeps stop short of what they hit, along the move. Our stand-in
/// for the BSP trace's distance epsilon, so the box never rests touching.
const TRACE_BACKOFF: f32 = 0.031_25;
/// Smallest gap between two stuck nudges, seconds.
const STUCK_NUDGE_GAP: f32 = 0.05;

/// Offsets tried, in order, to free a stuck player (spec, "Stuck recovery").
fn nudge_table() -> Vec<Vec3> {
    let small = [-0.125f32, 0.0, 0.125];
    let mut t = Vec::with_capacity(54);
    t.extend(small.map(|z| Vec3::new(0.0, 0.0, z)));
    t.extend(small.map(|y| Vec3::new(0.0, y, 0.0)));
    t.extend(small.map(|x| Vec3::new(x, 0.0, 0.0)));
    for x in [-0.125f32, 0.125] {
        for y in [-0.125f32, 0.125] {
            for z in [-0.125f32, 0.125] {
                t.push(Vec3::new(x, y, z));
            }
        }
    }
    t.extend([0.0f32, 1.0, 6.0].map(|z| Vec3::new(0.0, 0.0, z)));
    t.extend([-2.0f32, 0.0, 2.0].map(|y| Vec3::new(0.0, y, 0.0)));
    t.extend([-2.0f32, 0.0, 2.0].map(|x| Vec3::new(x, 0.0, 0.0)));
    for z in [0.0f32, 1.0, 6.0] {
        for x in [-2.0f32, 0.0, 2.0] {
            for y in [-2.0f32, 0.0, 2.0] {
                t.push(Vec3::new(x, y, z));
            }
        }
    }
    t.push(Vec3::ZERO);
    t
}

/// Float noise allowance (meters) when deciding whether a sweep runs
/// parallel to a brush plane: about a thousandth of a unit.
const PARALLEL_SLOP: f32 = 2.5e-5;
/// Solid tests use the box shrunk by this much, so resting contact (within
/// the back-off) doesn't count as stuck.
const SOLID_SKIN: f32 = 0.01;

/// Per-character movement state. Units and seconds as in the spec.
#[derive(Component, Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct SourceMovement {
    pub on_ground: bool,
    pub ground_normal: Vec3,
    pub jump_held: bool,
    duck_held: bool,
    /// Small box in use.
    pub ducked: bool,
    /// In a duck or unduck transition.
    pub ducking: bool,
    duck_timer: f32,
    /// Eye height above the feet.
    pub eye: f32,
    pub fall_speed: f32,
    pub surface_friction: f32,
    /// Speed of the last landing (for fall damage and sounds), units/s.
    pub last_landing_speed: f32,
    /// Stuck recovery: ticks run, the next nudge to try (0 = not stuck),
    /// and when the last nudge was tried (seconds of movement time).
    ticks: u64,
    nudge: usize,
    time: f32,
    last_nudge: f32,
    /// Feet at the end of the last tick, to notice teleports.
    last_feet: Option<Vec3>,
    /// The moving solid stood on (`MovingSolid`), if any. Not sent to a
    /// predicting client (entity ids differ).
    #[serde(skip)]
    pub ground_entity: Option<Entity>,
    /// Jump stamina left, ms (CS:S).
    pub stamina: f32,
    /// On a ladder: its surface normal (Source axes, out of the ladder).
    pub ladder: Option<Vec3>,
    /// 0 dry, 1 feet, 2 waist, 3 eyes.
    pub water_level: u8,
    pub in_slime: bool,
    /// Water jump: time left (ms) and the horizontal velocity it holds.
    pub water_jump_time: f32,
    water_jump_vel: Vec3,
    /// Footsteps (sound spec 3): time to the next step (ms), which foot
    /// steps next, the surface last stood on, swim-stroke timer (ms).
    pub step_timer: f32,
    step_left: bool,
    pub surface: Option<String>,
    swim_timer: f32,
    /// The punch angle's roll (degrees) and its velocity: hard landings
    /// and wall slams kick it (movement spec, "Falling and landing").
    pub punch_roll: f32,
    punch_roll_vel: f32,
}

impl Default for SourceMovement {
    fn default() -> Self {
        Self {
            on_ground: false,
            ground_normal: Vec3::Z,
            jump_held: false,
            duck_held: false,
            ducked: false,
            ducking: false,
            duck_timer: 0.0,
            eye: 64.0,
            fall_speed: 0.0,
            surface_friction: 1.0,
            last_landing_speed: 0.0,
            ticks: 0,
            nudge: 0,
            time: 0.0,
            last_nudge: f32::NEG_INFINITY,
            last_feet: None,
            ground_entity: None,
            stamina: 0.0,
            ladder: None,
            water_level: 0,
            in_slime: false,
            water_jump_time: 0.0,
            water_jump_vel: Vec3::ZERO,
            step_timer: 0.0,
            step_left: false,
            surface: None,
            swim_timer: 0.0,
            punch_roll: 0.0,
            punch_roll_vel: 0.0,
        }
    }
}

pub struct SourceMovementPlugin;

impl Plugin for SourceMovementPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<SourceMovementConfig>()
            .add_message::<crate::map::PlaySound>();
        // Every movement cvar in the console, defaulting to CS:S's values.
        let defaults = SourceMovementConfig::default();
        for (name, help) in CVARS {
            let name = *name;
            crate::console::ConsoleAppExt::console_cvar(
                app,
                name,
                help,
                &defaults.get_cvar(name).unwrap_or_default(),
                move |w| w.get_resource::<SourceMovementConfig>().and_then(|c| c.get_cvar(name)),
                move |w, v| {
                    w.get_resource_mut::<SourceMovementConfig>()
                        .ok_or("no Source movement")?
                        .set_cvar(name, v)
                },
            );
        }
        app.register_movement::<SourceMovement>(ID)
            .add_systems(crate::core::Predict::Movement, step)
            .predicted_net::<SourceMovement>()
            .add_plugins((super::pushaway::PushAwayPlugin, super::shadow::ShadowPlugin));
        if !app.is_plugin_added::<super::pain::PainSoundsPlugin>() {
            app.add_plugins(super::pain::PainSoundsPlugin);
        }
    }
}

/// Engine (meters, Y up) to Source (units, Z up), positions and directions.
pub fn to_source(v: Vec3) -> Vec3 {
    Vec3::new(v.x, -v.z, v.y) / METERS_PER_UNIT
}

pub fn to_engine(v: Vec3) -> Vec3 {
    Vec3::new(v.x, v.z, -v.y) * METERS_PER_UNIT
}

/// Result of sweeping a box, as the spec's traces report it.
#[derive(Clone, Copy, Debug)]
struct Trace {
    /// What was hit is a ladder.
    ladder: bool,
    fraction: f32,
    /// Feet position at the end of the sweep.
    end: Vec3,
    /// The moving solid hit, if any.
    owner: Option<Entity>,
    normal: Vec3,
    start_solid: bool,
    /// What was hit, when it carries its own surface property (props).
    /// None for the world: footsteps look it up from the faces below.
    surface: Option<HitSurface>,
}

/// Where a trace hit's surface property lives (`Tracer::surface_name`).
#[derive(Clone, Copy, Debug)]
enum HitSurface {
    /// Index into `MapBrushes`.
    Brush(usize),
    /// A physics collider with a `PropSurface`.
    Collider(Entity),
}

impl Trace {
    fn hit(&self) -> bool {
        self.fraction < 1.0
    }
}

/// Box sweeps against the world, in Source units. Brushes and terrain
/// triangles (displacements, as thin solids with bevel planes) are swept
/// exactly against their planes (as Source traces do), so both stop the
/// same distance short and count resting contact the same way; props go
/// through physics shape casts. (Shape casts against the terrain's one
/// triangle-mesh collider stopped players dead now and then: edge contacts
/// on flat ground gave sideways normals, and a contact ignored as grazing
/// hid any real hit behind it, leaving the box inside the terrain.)
struct Tracer<'a, 'w, 's> {
    query: &'a SpatialQuery<'w, 's>,
    filter: SpatialQueryFilter,
    brushes: Option<&'a MapBrushes>,
    /// The BSP tree over `brushes` (Source maps): decides which of two
    /// faces hit at the same distance a trace reports.
    tree: Option<&'a MapBrushTree>,
    water: Option<&'a MapWater>,
    surfaces: Option<&'a SurfaceGrid>,
    /// Surface property of a physics collider (props), by entity.
    prop_surfaces: &'a dyn Fn(Entity) -> Option<String>,

    sounds: Option<&'a crate::map::MapSounds>,
    /// Standing and ducked box heights.
    heights: (f32, f32),
    /// Other characters' boxes (engine space), swept exactly like brushes:
    /// players are axis-aligned boxes to each other, as in Source.
    others: &'a [MapBrush],
    /// Terrain triangles (displacements), swept exactly like brushes.
    terrain: Option<&'a MapTerrain>,
    /// For each of `others`: the moving solid it belongs to (None for
    /// characters).
    owners: &'a [Option<Entity>],
}

/// Brush sweep result, engine space.
struct BrushHit {
    ladder: bool,
    /// The hit brush, when it has its own surface property.
    surface: Option<usize>,
    /// The moving solid hit, if any.
    owner: Option<Entity>,
    fraction: f32,
    normal: Vec3,
    start_solid: bool,
}

impl Tracer<'_, '_, '_> {
    /// The surface property name a trace hit carries, if any.
    fn surface_name(&self, hit: HitSurface) -> Option<String> {
        match hit {
            HitSurface::Brush(i) => self.brushes?.0.get(i)?.surface.clone(),
            HitSurface::Collider(e) => (self.prop_surfaces)(e),
        }
    }

    /// Sweep an axis-aligned box (centre `from` moved by `delta`, half size
    /// `half`, engine meters) through the brushes: each brush's planes
    /// pushed out by the box's extent along their normals, then the
    /// centre's segment clipped against them. Hits stop a distance epsilon
    /// short.
    ///
    /// The move is a start and a delta, not two end points: how far the
    /// move goes into a plane is `n . delta`, which is exact enough to tell
    /// a move clipped along a plane from one into it. The difference of the
    /// end points' distances isn't, far from the origin: there, rounding a
    /// position to f32 moves it ~1e-5 m, as much as the parallel allowance,
    /// so a velocity just clipped along a surf ramp (surf_sedona's, 200 m
    /// up) read as entering it again and the player stopped dead.
    fn sweep_brushes(&self, half: Vec3, from: Vec3, delta: Vec3) -> BrushHit {
        let eps = TRACE_BACKOFF * METERS_PER_UNIT;
        let to = from + delta;
        let mut out = BrushHit {
            ladder: false,
            surface: None,
            owner: None,
            fraction: 1.0,
            normal: Vec3::ZERO,
            start_solid: false,
        };
        let brushes = self.brushes.map_or(&[][..], |b| &b.0[..]);
        let lo = from.min(to) - half - Vec3::splat(eps);
        let hi = from.max(to) + half + Vec3::splat(eps);
        let terrain = self.terrain_near(lo, hi);
        // Brushes hit at the current fraction: (index, normal, ladder).
        let mut tied: Vec<(usize, Vec3, bool)> = Vec::new();
        'brush: for (i, b) in brushes.iter().chain(self.others).chain(terrain).enumerate() {
            if b.max.cmplt(lo).any() || b.min.cmpgt(hi).any() {
                continue;
            }
            let (mut enter, mut leave) = (-1.0f32, 1.0f32);
            let (mut starts_out, mut gets_out) = (false, false);
            let mut clip = Vec3::ZERO;
            // The last plane the box really crosses (no back-off): the
            // contact's normal.
            let mut last_crossed = -1.0f32;
            for (n, d) in &b.planes {
                let dist = d + n.abs().dot(half);
                let d1 = n.dot(from) - dist;
                let d2 = d1 + n.dot(delta);
                gets_out |= d2 > 0.0;
                starts_out |= d1 > 0.0;
                // Entirely in front of this plane: misses the brush. Moving
                // parallel to it counts as not entering, within float noise
                // (planes and positions are meters at map scale), or a
                // velocity just clipped along an angled surface would catch
                // on it forever.
                if d1 > 0.0 && (d2 >= eps || d2 >= d1 - PARALLEL_SLOP) {
                    continue 'brush;
                }
                if d1 <= 0.0 && d2 <= 0.0 {
                    continue;
                }
                if d1 > d2 {
                    // Where the move stops: the back-off short of the
                    // plane, along its normal.
                    enter = enter.max(((d1 - eps) / (d1 - d2)).max(0.0));
                    // Which plane it stops against: the one it would
                    // really reach last. Ranking the backed-off fractions
                    // instead favours planes met head-on (their back-off
                    // costs less of the move): gliding a back-off above a
                    // surf ramp, a box nears the edge where the next
                    // segment's face rises within the back-off well before
                    // it nears that face, so the edge's bevel (facing back
                    // along the ramp) came last and stopped it dead.
                    let crossed = d1 / (d1 - d2);
                    if crossed > last_crossed {
                        last_crossed = crossed;
                        clip = *n;
                    }
                } else {
                    leave = leave.min(((d1 + eps) / (d1 - d2)).min(1.0));
                }
            }
            if !starts_out {
                out.start_solid = true;
                if !gets_out {
                    out.fraction = 0.0;
                }
                continue;
            }
            if !(enter < leave && enter > -1.0) {
                continue;
            }
            let f = enter.max(0.0);
            if out.fraction < 1.0 && (f - out.fraction).abs() < 1e-6 {
                // The nearest first (the hit when nothing breaks the tie).
                if f < out.fraction {
                    out.fraction = f;
                    tied.insert(0, (i, clip, b.ladder));
                } else {
                    tied.push((i, clip, b.ladder));
                }
            } else if f < out.fraction {
                out.fraction = f;
                tied.clear();
                tied.push((i, clip, b.ladder));
            }
        }
        // A ladder flush with another face (maps flank ladders with player
        // clip): of the brushes hit at the same distance, the one the trace
        // reaches first through the BSP tree wins, as in Source. Otherwise
        // the first in our list.
        let mut winner = tied.first().copied();
        if tied.iter().any(|t| t.2) && tied.iter().any(|t| !t.2)
            && let Some(tree) = self.tree
        {
            let order = tree.sweep_order(half, from, to, METERS_PER_UNIT);
            let rank = |k: usize| order.iter().position(|&o| o as usize == k).unwrap_or(usize::MAX);
            winner = tied.iter().copied().min_by_key(|t| rank(t.0));
        }
        if let Some((i, normal, ladder)) = winner {
            out.normal = normal;
            out.ladder = ladder;
            out.surface = brushes.get(i).and_then(|b| b.surface.is_some().then_some(i));
            out.owner = i
                .checked_sub(brushes.len())
                .and_then(|j| self.owners.get(j).copied().flatten());
        }
        out
    }

    /// Terrain brushes whose bounds overlap `lo`..`hi` (engine space).
    fn terrain_near(&self, lo: Vec3, hi: Vec3) -> impl Iterator<Item = &MapBrush> {
        let mut near = Vec::new();
        if let Some(t) = self.terrain {
            t.near(lo, hi, &mut near);
        }
        near.into_iter()
            .filter_map(|i| self.terrain.map(|t| &t.brushes[i as usize]))
    }

    /// Whether a box (engine centre and half size) overlaps a brush by more
    /// than the solid skin.
    fn in_brush(&self, half: Vec3, centre: Vec3) -> bool {
        let skin = SOLID_SKIN * METERS_PER_UNIT;
        let brushes = self.brushes.map_or(&[][..], |b| &b.0[..]);
        let terrain = self.terrain_near(centre - half, centre + half);
        brushes.iter().chain(self.others).chain(terrain).any(|b| {
            b.max.cmpgt(centre - half).all()
                && b.min.cmplt(centre + half).all()
                && b.planes
                    .iter()
                    .all(|(n, d)| n.dot(centre) - (d + n.abs().dot(half)) < -skin)
        })
    }

    /// Sweep a box (`lo`..`hi` relative to the feet, Source units) from feet
    /// position `from` to `to`.
    fn sweep_box(&self, lo: Vec3, hi: Vec3, from: Vec3, to: Vec3) -> Trace {
        self.sweep_box_by(lo, hi, from, to - from)
    }

    /// `sweep_box` by a move `delta` (Source units) from `from`.
    fn sweep_box_by(&self, lo: Vec3, hi: Vec3, from: Vec3, delta: Vec3) -> Trace {
        let size = hi - lo;
        let centre = |feet: Vec3| to_engine(feet + (lo + hi) / 2.0);
        let start_solid = self.solid_box(lo, hi, from);
        let to = from + delta;
        let len = delta.length();
        let miss = Trace {
            ladder: false,
            owner: None,
            fraction: 1.0,
            end: to,
            normal: Vec3::ZERO,
            start_solid,
            surface: None,
        };
        if len < 1e-6 {
            return Trace { end: from, ..miss };
        }
        // Box half size in engine axes (x, up, z).
        let half = Vec3::new(size.x, size.z, size.y) * METERS_PER_UNIT / 2.0;
        let brush = self.sweep_brushes(half, centre(from), to_engine(delta));
        let mut best = Trace {
            owner: brush.owner,
            ladder: brush.ladder,
            fraction: brush.fraction,
            end: from + delta * brush.fraction,
            normal: to_source(brush.normal).normalize_or_zero(),
            start_solid,
            surface: brush.surface.map(HitSurface::Brush),
        };
        let shape = Collider::cuboid(half.x * 2.0, half.y * 2.0, half.z * 2.0);
        let Ok(dir) = Dir3::new(to_engine(delta / len)) else {
            return best;
        };
        let config = ShapeCastConfig {
            max_distance: len * METERS_PER_UNIT,
            ignore_origin_penetration: true,
            ..default()
        };
        // A surface that doesn't face against the move (we're sliding along
        // it, or already leaving it) doesn't block it; physics casts still
        // report such grazing contacts.
        if let Some(hit) = self
            .query
            .cast_shape(&shape, centre(from), Quat::IDENTITY, dir, &config, &self.filter)
            .filter(|hit| hit.normal1.dot(*dir) < -1e-3)
        {
            // Keep the margin along the surface normal, as Source's traces
            // do (a margin along the move leaves a glancing box almost
            // touching, which the stuck test then flags).
            let facing = (-hit.normal1.dot(*dir)).max(0.05);
            let travelled = (hit.distance / METERS_PER_UNIT - TRACE_BACKOFF / facing).max(0.0);
            let fraction = (travelled / len).clamp(0.0, 0.999_999);
            if fraction < best.fraction {
                best = Trace {
                    owner: None,
                    ladder: false,
                    fraction,
                    end: from + delta * fraction,
                    normal: to_source(hit.normal1).normalize_or_zero(),
                    start_solid,
                    surface: Some(HitSurface::Collider(hit.entity)),
                };
            }
        }
        best
    }

    fn hull(&self, ducked: bool) -> (Vec3, Vec3) {
        let height = if ducked { self.heights.1 } else { self.heights.0 };
        (
            Vec3::new(-HALF_WIDTH, -HALF_WIDTH, 0.0),
            Vec3::new(HALF_WIDTH, HALF_WIDTH, height),
        )
    }

    fn sweep(&self, ducked: bool, from: Vec3, to: Vec3) -> Trace {
        let (lo, hi) = self.hull(ducked);
        self.sweep_box(lo, hi, from, to)
    }

    /// `sweep` by a move `delta` from `from` (`sweep_brushes`: exact for
    /// moves along a plane).
    fn sweep_by(&self, ducked: bool, from: Vec3, delta: Vec3) -> Trace {
        let (lo, hi) = self.hull(ducked);
        self.sweep_box_by(lo, hi, from, delta)
    }

    fn solid_box(&self, lo: Vec3, hi: Vec3, feet: Vec3) -> bool {
        let full = hi - lo;
        if self.in_brush(
            Vec3::new(full.x, full.z, full.y) * METERS_PER_UNIT / 2.0,
            to_engine(feet + (lo + hi) / 2.0),
        ) {
            return true;
        }
        let size = (hi - lo - Vec3::splat(2.0 * SOLID_SKIN)).max(Vec3::splat(0.01));
        let shape = Collider::cuboid(
            size.x * METERS_PER_UNIT,
            size.z * METERS_PER_UNIT,
            size.y * METERS_PER_UNIT,
        );
        !self
            .query
            .shape_intersections(&shape, to_engine(feet + (lo + hi) / 2.0), Quat::IDENTITY, &self.filter)
            .is_empty()
    }

    fn solid(&self, ducked: bool, feet: Vec3) -> bool {
        let (lo, hi) = self.hull(ducked);
        self.solid_box(lo, hi, feet)
    }

    /// Point contents: solid.
    fn point_solid(&self, p: Vec3) -> bool {
        self.solid_box(Vec3::splat(-0.05), Vec3::splat(0.05), p)
    }

    /// Point contents: water (`Some(false)`) or slime (`Some(true)`).
    fn point_water(&self, p: Vec3) -> Option<bool> {
        let at = to_engine(p);
        self.water?.0.iter().find(|w| w.brush.contains(at)).map(|w| w.slime)
    }
}

/// out = in - n (in . n) b, then remove any remaining push into the plane.
fn clip_velocity(v: Vec3, n: Vec3, overbounce: f32) -> Vec3 {
    let mut out = v - n * v.dot(n) * overbounce;
    let into = out.dot(n);
    if into < 0.0 {
        out -= n * into;
    }
    out
}

/// One tick for one character, in Source units.
struct Mover<'a, 'b, 'w, 's> {
    cfg: &'a SourceMovementConfig,
    trace: &'a Tracer<'b, 'w, 's>,
    me: &'a mut SourceMovement,
    feet: Vec3,
    v: Vec3,
    dt: f32,
    /// Water contents per test point (feet, waist, eye) this tick.
    water_cache: [Option<(Vec3, Option<bool>)>; 3],
    /// Move input added by props pushing the player back (forward, side).
    push_input: Vec2,
    /// Sounds made this tick: entry, where (Source units), volume.
    sounds: Vec<(String, Vec3, Option<f32>)>,
    /// Max speed from the held weapon, units/s.
    player_maxspeed: f32,
    /// Fall damage taken this tick, health points.
    fall_damage: f32,
    /// Base velocity (specs/source/triggers.md, trigger_push), units/s;
    /// set by a push during the last tick; taken off the ground by logic.
    base: Vec3,
    base_touched: bool,
    unground: bool,
    /// Gravity multiplier (0 = normal).
    gravity_scale: f32,
    /// Moving solids' velocities (Source units/s), for riders.
    mover_velocity: &'a dyn Fn(Entity) -> Vec3,
    /// Physics colliders the move ran into this tick, the one stood on,
    /// and the wish velocity (units/s): the physics shadow's inputs
    /// (physics_props.md 4.1).
    touched: Vec<Entity>,
    ground_collider: Option<Entity>,
    wish: Vec3,
}

impl Mover<'_, '_, '_, '_> {
    fn clamp_velocity(&mut self) {
        let m = self.cfg.maxvelocity;
        self.v = Vec3::new(
            if self.v.x.is_nan() { 0.0 } else { self.v.x.clamp(-m, m) },
            if self.v.y.is_nan() { 0.0 } else { self.v.y.clamp(-m, m) },
            if self.v.z.is_nan() { 0.0 } else { self.v.z.clamp(-m, m) },
        );
    }

    fn half_gravity(&mut self) {
        let scale = if self.gravity_scale == 0.0 { 1.0 } else { self.gravity_scale };
        self.v.z -= scale * self.cfg.gravity * self.dt * 0.5;
    }

    /// The first gravity half-step also applies the vertical base velocity
    /// as an acceleration (triggers.md, trigger_push, step 2).
    fn start_gravity(&mut self) {
        self.half_gravity();
        self.v.z += self.base.z * self.dt;
        self.base.z = 0.0;
    }

    /// Ground changes: leaving a moving solid adds its velocity to the
    /// base velocity, landing on one takes it off (doors_buttons.md,
    /// "Riding").
    fn set_ground(&mut self, on: bool, owner: Option<Entity>) {
        let (was_on, was) = (self.me.on_ground, self.me.ground_entity);
        if !was_on && on && let Some(e) = owner {
            let mv = (self.mover_velocity)(e);
            self.base.x -= mv.x;
            self.base.y -= mv.y;
            self.base.z = mv.z;
        } else if was_on && !on && let Some(e) = was {
            let mv = (self.mover_velocity)(e);
            self.base.x += mv.x;
            self.base.y += mv.y;
            self.base.z = mv.z;
        }
        self.me.on_ground = on;
        self.me.ground_entity = if on { owner } else { None };
        if !on {
            self.ground_collider = None;
        }
    }

    fn friction(&mut self) {
        let speed = self.v.length();
        if speed < 0.1 {
            return;
        }
        let control = speed.max(self.cfg.stopspeed);
        let drop = control * self.cfg.friction * self.me.surface_friction * self.dt;
        let new_speed = (speed - drop).max(0.0);
        self.v *= new_speed / speed;
    }

    fn accelerate(&mut self, dir: Vec3, wish_speed: f32, accel: f32) {
        let add = wish_speed - self.v.dot(dir);
        if add <= 0.0 {
            return;
        }
        let amount = (accel * self.dt * wish_speed * self.me.surface_friction).min(add);
        self.v += dir * amount;
    }

    fn air_accelerate(&mut self, dir: Vec3, wish_speed: f32, accel: f32) {
        let add = wish_speed.min(AIR_WISH_CAP) - self.v.dot(dir);
        if add <= 0.0 {
            return;
        }
        let amount = (accel * wish_speed * self.dt * self.me.surface_friction).min(add);
        self.v += dir * amount;
    }

    /// Collide and slide for the rest of the tick.
    fn slide(&mut self) {
        let before = self.v.truncate().length();
        self.slide_move();
        // Wall slam: a big loss of horizontal speed in one move.
        let loss = before - self.v.truncate().length();
        if loss > 1160.0 {
            self.impact_step(1.0);
        } else if loss > 580.0 {
            self.impact_step(0.85);
        }
    }

    fn slide_move(&mut self) {
        let primal = self.v;
        let mut original = self.v;
        let mut planes: Vec<Vec3> = Vec::with_capacity(MAX_CLIP_PLANES);
        let mut time_left = self.dt;
        let mut all_fraction = 0.0;
        for _ in 0..BUMPS {
            if self.v.length_squared() == 0.0 {
                break;
            }
            let end = self.feet + self.v * time_left;
            let tr = self.trace.sweep_by(self.me.ducked, self.feet, self.v * time_left);            all_fraction += tr.fraction;
            if tr.start_solid && tr.fraction == 0.0 && self.trace.solid(self.me.ducked, end) {
                self.v = Vec3::ZERO;
                return;
            }
            if tr.fraction > 0.0 {
                // A full sweep is re-tested at its end (it can end inside
                // terrain); partial ones aren't.
                if tr.fraction >= 1.0 && self.trace.solid(self.me.ducked, tr.end) {
                    self.v = Vec3::ZERO;
                    break;
                }
                self.feet = tr.end;
                original = self.v;
                planes.clear();
            }
            if !tr.hit() {
                break;
            }
            if let Some(HitSurface::Collider(e)) = tr.surface
                && !self.touched.contains(&e)
            {
                self.touched.push(e);
            }
            time_left -= time_left * tr.fraction;
            if planes.len() >= MAX_CLIP_PLANES {
                self.v = Vec3::ZERO;
                break;
            }
            planes.push(tr.normal);
            if planes.len() == 1 && !self.me.on_ground && self.me.ladder.is_none() {
                let overbounce = if tr.normal.z > WALKABLE_NORMAL_Z {
                    1.0
                } else {
                    1.0 + self.cfg.bounce * (1.0 - self.me.surface_friction)
                };
                self.v = clip_velocity(original, tr.normal, overbounce);
            } else {
                let found = planes.iter().enumerate().find_map(|(i, n)| {
                    let clipped = clip_velocity(original, *n, 1.0);
                    planes
                        .iter()
                        .enumerate()
                        .all(|(j, m)| j == i || clipped.dot(*m) >= 0.0)
                        .then_some(clipped)
                });
                match found {
                    Some(v) => self.v = v,
                    None if planes.len() == 2 => {
                        let dir = planes[0].cross(planes[1]).normalize_or_zero();
                        self.v = dir * dir.dot(self.v);
                    }
                    None => {
                        self.v = Vec3::ZERO;
                        break;
                    }
                }
                if self.v.dot(primal) <= 0.0 {
                    self.v = Vec3::ZERO;
                    break;
                }
            }
        }
        if all_fraction == 0.0 {
            self.v = Vec3::ZERO;
        }
    }

    /// Try the move flat and stepped up; keep whichever goes farther.
    fn step_move(&mut self) {
        let (p0, v0) = (self.feet, self.v);
        self.slide();
        let (down_pos, down_vel) = (self.feet, self.v);

        self.feet = p0;
        self.v = v0;
        let rise = self.cfg.stepsize + STEP_EPSILON;
        let up = self.trace.sweep(self.me.ducked, p0, p0 + Vec3::Z * rise);
        if !up.start_solid {
            self.feet = up.end;
        }
        self.slide();
        let down = self.trace.sweep(self.me.ducked, self.feet, self.feet - Vec3::Z * rise);
        if !down.hit() || down.normal.z < WALKABLE_NORMAL_Z {
            self.feet = down_pos;
            self.v = down_vel;
            return;
        }
        if !down.start_solid {
            self.feet = down.end;
        }
        let flat = (down_pos - p0).truncate().length_squared();
        let stepped = (self.feet - p0).truncate().length_squared();
        if flat > stepped {
            self.feet = down_pos;
            self.v = down_vel;
        } else {
            self.v.z = down_vel.z;
        }
    }

    fn stay_on_ground(&mut self) {
        let up = self
            .trace
            .sweep(self.me.ducked, self.feet, self.feet + Vec3::Z * STAY_ON_GROUND_UP);
        let start = up.end;
        let end = self.feet - Vec3::Z * self.cfg.stepsize;
        let tr = self.trace.sweep(self.me.ducked, start, end);
        if tr.fraction > 0.0
            && tr.fraction < 1.0
            && !tr.start_solid
            && tr.normal.z >= WALKABLE_NORMAL_Z
            && (self.feet.z - tr.end.z).abs() > SNAP_MIN
        {
            self.feet = tr.end;
        }
    }

    fn wish(&self, f: f32, s: f32, forward: Vec3, right: Vec3, max_speed: f32) -> (Vec3, f32) {
        let mut wish = forward * f + right * s;
        wish.z = 0.0;
        let speed = wish.length();
        if speed < 1e-6 {
            return (Vec3::ZERO, 0.0);
        }
        (wish / speed, speed.min(max_speed))
    }

    fn walk(&mut self, f: f32, s: f32, forward: Vec3, right: Vec3, max_speed: f32) {
        let (dir, speed) = self.wish(f, s, forward, right, max_speed);
        self.wish = dir * speed;
        if self.me.stamina > 0.0 {
            let r = 1.0 - self.cfg.stamina_ground_scale * self.me.stamina;
            self.v.x *= r;
            self.v.y *= r;
        }
        self.v.z = 0.0;
        self.accelerate(dir, speed, self.cfg.accelerate);
        self.v.z = 0.0;
        // Base velocity moves the player but is not kept (trigger_push).
        let base = Vec3::new(self.base.x, self.base.y, 0.0);
        self.v += base;
        if self.v.length() < 1.0 {
            self.v = Vec3::ZERO;
            return;
        }
        let dest = self.feet + Vec3::new(self.v.x, self.v.y, 0.0) * self.dt;
        let tr = self.trace.sweep(self.me.ducked, self.feet, dest);
        if !tr.hit() {
            self.feet = tr.end;
        } else {
            self.step_move();
        }
        self.stay_on_ground();
        self.v -= base;
    }

    fn air(&mut self, f: f32, s: f32, forward: Vec3, right: Vec3, max_speed: f32) {
        let (dir, speed) = self.wish(f, s, forward, right, max_speed);
        self.wish = dir * speed;
        self.air_accelerate(dir, speed, self.cfg.airaccelerate);
        let base = Vec3::new(self.base.x, self.base.y, 0.0);
        self.v += base;
        self.slide();
        self.v -= base;
    }

    /// Ground detection: a 2-unit drop test with the full box, then with
    /// each quarter of it, so edges and ridges still count as ground.
    fn categorize(&mut self) {
        self.me.surface_friction = 1.0;
        // Moving up on a ladder always counts as airborne.
        if self.v.z > LEAVE_GROUND_VZ || (self.me.ladder.is_some() && self.v.z > 0.0) {
            self.set_ground(false, None);
        } else {
            let (lo, hi) = self.trace.hull(self.me.ducked);
            let down = self.feet - Vec3::Z * GROUND_PROBE;
            let full = self.trace.sweep_box(lo, hi, self.feet, down);
            let mut ground = (full.hit() && full.normal.z >= WALKABLE_NORMAL_Z).then_some(full);
            if ground.is_none() {
                let mid = Vec3::ZERO;
                let quarters = [
                    (Vec3::new(lo.x, lo.y, lo.z), Vec3::new(mid.x, mid.y, hi.z)),
                    (Vec3::new(mid.x, lo.y, lo.z), Vec3::new(hi.x, mid.y, hi.z)),
                    (Vec3::new(lo.x, mid.y, lo.z), Vec3::new(mid.x, hi.y, hi.z)),
                    (Vec3::new(mid.x, mid.y, lo.z), Vec3::new(hi.x, hi.y, hi.z)),
                ];
                ground = quarters.iter().find_map(|(a, b)| {
                    let tr = self.trace.sweep_box(*a, *b, self.feet, down);
                    (tr.hit() && tr.normal.z >= WALKABLE_NORMAL_Z).then_some(tr)
                });
            }
            self.set_ground(ground.is_some(), ground.and_then(|g| g.owner));
            self.ground_collider = match ground.and_then(|g| g.surface) {
                Some(HitSurface::Collider(e)) => Some(e),
                _ => None,
            };
            if let Some(tr) = ground {
                self.me.ground_normal = tr.normal;
                self.v.z = 0.0;
                let hit = tr.surface.and_then(|h| self.trace.surface_name(h));
                self.record_surface(hit);
            } else if self.v.z > 0.0 {
                self.me.surface_friction = UPWARD_AIR_FRICTION;
            }
        }
        self.water_check();
    }

    /// The surface under the feet: the ground trace's own (a prop's), else
    /// the world face below (sound spec 3: the ground trace's surface).
    fn record_surface(&mut self, hit: Option<String>) {
        if hit.is_some() {
            self.me.surface = hit;
        } else if let Some(grid) = self.trace.surfaces
            && let Some(s) = grid.below(to_engine(self.feet + Vec3::Z), 6.0 * METERS_PER_UNIT)
        {
            self.me.surface = Some(s.to_string());
        }
    }

    /// Play a footstep on `surface` (unknown names: "default"): its left or
    /// right step, alternating
    /// (the first after spawn is a right step), at `volume`.
    fn play_step(&mut self, surface: &str, volume: f32) {
        // A name the surface scripts don't define (dust2's "stone" props)
        // uses "default", whose steps every surface inherits.
        let Some(s) = self
            .trace
            .sounds
            .and_then(|b| b.surface(surface).or_else(|| b.surface("default")))
        else {
            return;
        };
        let entry = if self.me.step_left { &s.step_left } else { &s.step_right };
        let Some(entry) = entry.clone() else { return };
        self.me.step_left = !self.me.step_left;
        self.sounds.push((entry, self.feet, Some(volume)));
    }

    /// Footsteps (sound spec 3, "When a footstep is checked"), from the
    /// state at the start of the tick.
    fn footsteps(&mut self, intent: &Intent) {
        if self.me.step_timer > 0.0 {
            self.me.step_timer = (self.me.step_timer - 1000.0 * self.dt).max(0.0);
            if self.me.step_timer > 0.0 {
                return;
            }
        }
        if self.cfg.silent_walk_duck && (intent.walk || intent.crouch || self.me.ducked) {
            return;
        }
        let ladder = self.me.ladder.is_some();
        let speed = self.v.length();
        let (walk, run) = if self.me.ducked || ladder {
            (60.0, 80.0)
        } else {
            (90.0, 220.0)
        };
        if speed < walk || !(ladder || (self.me.on_ground && self.v.truncate().length() > 1e-4)) {
            return;
        }
        let walking = speed < run;
        let height = if self.me.ducked {
            self.trace.heights.1
        } else {
            self.trace.heights.0
        };
        let knee = self.trace.point_water(self.feet + Vec3::Z * (0.2 * height)).is_some();
        let (surface, mut volume, mut period) = if ladder {
            ("ladder".to_string(), 0.5, 350.0)
        } else if knee {
            ("wade".to_string(), 0.65, 600.0)
        } else if self.me.water_level == 1 {
            (
                "water".to_string(),
                if walking { 0.2 } else { 0.5 },
                if walking { 400.0 } else { 300.0 },
            )
        } else {
            let Some(surface) = self.me.surface.clone() else { return };
            let material = self
                .trace
                .sounds
                .and_then(|b| b.surface(&surface))
                .map_or('C', |s| s.game_material);
            let volume = match (material, walking) {
                ('D', true) => 0.25,
                ('D', false) => 0.55,
                ('V', true) => 0.4,
                ('V', false) => 0.7,
                (_, true) => 0.2,
                (_, false) => 0.5,
            };
            (surface, volume, if walking { 400.0 } else { 300.0 })
        };
        if self.me.ducked || ladder {
            period += 100.0;
        }
        if self.me.ducked {
            volume *= 0.65;
        }
        self.me.step_timer = period;
        self.play_step(&surface, volume);
    }

    /// A landing or wall-slam step on the current surface.
    fn impact_step(&mut self, volume: f32) {
        if volume <= 0.0 {
            return;
        }
        self.me.step_timer = 400.0;
        // The view rolls by the current fall speed (a wall slam's is
        // usually 0).
        self.me.punch_roll = self.me.fall_speed * LAND_PUNCH_SCALE;
        if let Some(s) = self.me.surface.clone() {
            self.play_step(&s, volume);
        }
    }

    /// Landing (sound spec 3): fall speed tiers.
    fn landing_sound(&mut self, fall: f32) {
        if fall < 350.0 {
            return;
        }
        let volume = if self.me.water_level > 0 {
            0.5
        } else if fall > 580.0 {
            1.0
        } else if fall > 290.0 {
            0.85
        } else if fall < 200.0 {
            0.0
        } else {
            0.5
        };
        self.impact_step(volume);
    }

    /// Water level from three points at the box's centre line: 1 unit
    /// above the feet, mid-box, and the eye.
    fn water_check(&mut self) {
        let height = if self.me.ducked {
            self.trace.heights.1
        } else {
            self.trace.heights.0
        };
        let feet = self.feet;
        let points = [WATER_FEET_PROBE, height / 2.0, self.me.eye].map(|z| feet + Vec3::Z * z);
        // Contents per test point are cached for the tick: a point within
        // 1 unit of the last one tested for that slot reuses its answer.
        let mut test = |slot: usize| -> Option<bool> {
            if let Some((p, answer)) = self.water_cache[slot]
                && p.distance(points[slot]) < 1.0
            {
                return answer;
            }
            let answer = self.trace.point_water(points[slot]);
            self.water_cache[slot] = Some((points[slot], answer));
            answer
        };
        self.me.water_level = 0;
        let Some(slime) = test(0) else {
            return;
        };
        self.me.in_slime = slime;
        self.me.water_level = 1;
        if test(1).is_some() {
            self.me.water_level = 2;
            if test(2).is_some() {
                self.me.water_level = 3;
            }
        }
    }

    /// Full 3D view vectors (Source axes): forward and right.
    fn view(intent: &Intent) -> (Vec3, Vec3) {
        // Intent yaw 0 looks down engine -Z, which is Source yaw 90;
        // intent pitch is up-positive, Source pitch down-positive.
        let yaw = std::f32::consts::FRAC_PI_2 + intent.yaw;
        let pitch = -intent.pitch;
        let forward = Vec3::new(pitch.cos() * yaw.cos(), pitch.cos() * yaw.sin(), -pitch.sin());
        let right = Vec3::new(yaw.sin(), -yaw.cos(), 0.0);
        (forward, right)
    }

    /// Ladder detection (after ducking, before the move): probe 2 units
    /// toward the ladder (along the last normal, or the 3D input direction)
    /// and, on a ladder, set this tick's velocity.
    fn ladder(&mut self, intent: &Intent, f: f32, s: f32, duck_scale: f32) {
        let (forward, right) = Self::view(intent);
        let dir = match self.me.ladder {
            Some(n) => -n,
            None => {
                if f == 0.0 && s == 0.0 {
                    return;
                }
                (forward * f + right * s).normalize_or_zero()
            }
        };
        let tr = self
            .trace
            .sweep(self.me.ducked, self.feet, self.feet + dir * LADDER_REACH);
        if !tr.hit() || !tr.ladder {
            self.me.ladder = None;
            return;
        }
        let n = tr.normal;
        self.me.ladder = Some(n);
        if intent.jump {
            // Off the ladder, pushed away from it; walking from this tick.
            self.me.ladder = None;
            self.v = n * LADDER_JUMP_OFF;
            return;
        }
        let button = |x: f32| {
            if x > 0.0 {
                LADDER_SPEED
            } else if x < 0.0 {
                -LADDER_SPEED
            } else {
                0.0
            }
        };
        let (fwd, side) = (button(intent.move_axis.y), button(intent.move_axis.x));
        if fwd == 0.0 && side == 0.0 {
            self.v = Vec3::ZERO;
            return;
        }
        let u = forward * fwd + right * side;
        let a = u.dot(n);
        let into = n * a;
        let mut lateral = u - into;
        let perp = Vec3::Z.cross(n).normalize_or_zero();
        let up = n.cross(perp);
        // CS:S: mostly pushing into the ladder damps the sideways part.
        let (t, p) = (up.dot(lateral), perp.dot(lateral));
        if (perp * p + into).normalize_or_zero().dot(n) < self.cfg.ladder_angle {
            lateral = up * t + perp * (self.cfg.ladder_dampen * p);
        }
        self.v = lateral - up * a;
        if self.cfg.duck_slows_ladder {
            self.v *= duck_scale;
        }
        let below = self.feet - Vec3::Z * 1.0;
        let on_floor = self.me.on_ground || self.trace.point_solid(below);
        if on_floor && a > 0.0 {
            self.v += n * LADDER_BACK_OFF;
        }
    }

    /// Climbing out of water at a ledge (only at waist depth).
    fn water_jump_check(&mut self, intent: &Intent) {
        if self.me.water_jump_time > 0.0 || self.v.z < WATER_JUMP_MIN_VZ {
            return;
        }
        let yaw = std::f32::consts::FRAC_PI_2 + intent.yaw;
        let flat = Vec3::new(yaw.cos(), yaw.sin(), 0.0);
        let horizontal = self.v.with_z(0.0);
        if horizontal.length_squared() > 0.0 && horizontal.normalize().dot(flat) < 0.0 {
            return;
        }
        let height = if self.me.ducked {
            self.trace.heights.1
        } else {
            self.trace.heights.0
        };
        let from = self.feet + Vec3::Z * (height / 2.0);
        let wall = self.trace.sweep(self.me.ducked, from, from + flat * WATER_JUMP_REACH);
        if !wall.hit() {
            return;
        }
        let high = self.feet + Vec3::Z * (self.me.eye + WATER_JUMP_EYE_EXTRA);
        let over = self.trace.sweep(self.me.ducked, high, high + flat * WATER_JUMP_REACH);
        if over.hit() {
            return;
        }
        let down = self
            .trace
            .sweep(self.me.ducked, over.end, over.end - Vec3::Z * WATER_JUMP_DROP);
        if !down.hit() || down.normal.z < WALKABLE_NORMAL_Z {
            return;
        }
        self.v.z = WATER_JUMP_UP;
        self.me.water_jump_vel = -wall.normal * WATER_JUMP_PUSH;
        self.me.jump_held = true;
        self.me.water_jump_time = WATER_JUMP_TIME;
    }

    /// Swimming (waist deep or more): 3D wish, proportional friction, no
    /// gravity; holding jump rises.
    fn swim(&mut self, intent: &Intent, f: f32, s: f32, max_speed: f32) {
        let (forward, right) = Self::view(intent);
        let mut wish = forward * f + right * s;
        let lift = self.cfg.client_maxspeed;
        if intent.jump {
            wish.z += lift;
        } else if f == 0.0 && s == 0.0 {
            wish.z -= SWIM_SINK;
        } else {
            wish.z += (2.0 * f * forward.z).clamp(0.0, lift);
        }
        let wish_speed = wish.length().min(max_speed) * SWIM_WISH_SCALE;
        let dir = wish.normalize_or_zero();
        let speed = self.v.length();
        let mut new_speed = speed - self.dt * speed * self.cfg.friction * self.me.surface_friction;
        if new_speed < 0.1 {
            new_speed = 0.0;
        }
        self.v = if speed > 0.0 {
            self.v * (new_speed / speed)
        } else {
            Vec3::ZERO
        };
        if wish_speed >= 0.1 {
            let add = wish_speed - new_speed;
            if add > 0.0 {
                let amount = (self.cfg.accelerate * wish_speed * self.dt * self.me.surface_friction).min(add);
                self.v += dir * amount;
            }
        }
        let dest = self.feet + self.v * self.dt;
        let tr = self.trace.sweep(self.me.ducked, self.feet, dest);
        if !tr.hit() {
            // Ride up onto steps while swimming.
            let top = dest + Vec3::Z * (self.cfg.stepsize + 1.0);
            let settle = self.trace.sweep(self.me.ducked, top, dest);
            if settle.start_solid {
                self.slide();
            } else {
                self.feet = settle.end;
            }
        } else if self.me.on_ground {
            self.step_move();
        } else {
            self.slide();
        }
    }

    fn eye_blend(&self, fraction: f32) -> f32 {
        self.cfg.eye_duck * fraction + self.cfg.eye_stand * (1.0 - fraction)
    }

    fn smooth(x: f32) -> f32 {
        let x = x.clamp(0.0, 1.0);
        3.0 * x * x - 2.0 * x * x * x
    }

    fn finish_duck(&mut self) {
        if self.me.ducked {
            return;
        }
        self.me.ducked = true;
        self.me.ducking = false;
        self.me.eye = self.cfg.eye_duck;
        if !self.me.on_ground {
            self.feet.z += self.cfg.air_duck_lift();
        }
        let before = self.feet;
        let mut free = !self.trace.solid(true, self.feet);
        for _ in 0..36 {
            if free {
                break;
            }
            self.feet.z += 1.0;
            free = !self.trace.solid(true, self.feet);
        }
        if !free {
            self.feet = before;
        }
        self.categorize();
    }

    /// Whether the standing box fits: on the ground, where it stands; in the
    /// air, swept from here down by the hull difference (room is needed
    /// above the ducked head and below the feet).
    fn can_stand(&self) -> bool {
        if self.me.on_ground {
            return !self.trace.solid(false, self.feet);
        }
        let down = self.feet - Vec3::Z * self.cfg.air_duck_lift();
        let tr = self.trace.sweep(false, self.feet, down);
        !tr.start_solid && !tr.hit()
    }

    fn finish_unduck(&mut self) {
        if !self.me.on_ground {
            self.feet.z -= self.cfg.air_duck_lift();
        }
        self.me.ducked = false;
        self.me.ducking = false;
        self.me.eye = self.cfg.eye_stand;
        self.categorize();
    }

    /// Ducking transitions; returns the move-input scale, decided from the
    /// state before this tick's changes.
    fn duck(&mut self, held: bool) -> f32 {
        let scale = if self.me.ducked && (self.me.on_ground || self.cfg.duck_slows_everywhere) {
            self.cfg.duck_speed
        } else {
            1.0
        };
        let pressed = held && !self.me.duck_held;
        let released = !held && self.me.duck_held;
        let elapsed = (DUCK_TIMER_START - self.me.duck_timer) / 1000.0;
        if held {
            // A press while still flagged ducked (mid-unduck) doesn't
            // restart anything: the eye stays where it is while held.
            if pressed && !self.me.ducked {
                self.me.duck_timer = DUCK_TIMER_START;
                self.me.ducking = true;
            }
            if self.me.ducking && !self.me.ducked {
                let elapsed = (DUCK_TIMER_START - self.me.duck_timer) / 1000.0;
                if elapsed > TIME_TO_DUCK || !self.me.on_ground {
                    self.finish_duck();
                } else {
                    self.me.eye = self.eye_blend(Self::smooth(elapsed / TIME_TO_DUCK));
                }
            }
        } else if self.me.ducked || self.me.ducking {
            if released {
                self.me.duck_timer = if self.me.ducking && !self.me.ducked {
                    let fraction = (elapsed / TIME_TO_DUCK).clamp(0.0, 1.0);
                    DUCK_TIMER_START - TIME_TO_UNDUCK * 1000.0 + fraction * TIME_TO_UNDUCK * 1000.0
                } else {
                    DUCK_TIMER_START
                };
                self.me.ducking = true;
            }
            if self.can_stand() {
                let elapsed = (DUCK_TIMER_START - self.me.duck_timer) / 1000.0;
                if elapsed > TIME_TO_UNDUCK || !self.me.on_ground {
                    self.finish_unduck();
                } else {
                    self.me.eye = self.eye_blend(Self::smooth(1.0 - elapsed / TIME_TO_UNDUCK));
                    self.me.ducking = true;
                }
            } else if self.me.duck_timer != DUCK_TIMER_START {
                // No room to stand: forced fully ducked until there is.
                self.me.ducked = true;
                self.me.ducking = false;
                self.me.eye = self.cfg.eye_duck;
                self.me.duck_timer = DUCK_TIMER_START;
            }
        } else if (self.me.eye - self.cfg.eye_stand).abs() > 0.1 {
            self.me.eye = self.cfg.eye_stand;
        }
        self.me.duck_held = held;
        scale
    }

    fn jump(&mut self) {
        // Without sv_enablebunnyhopping, jumping caps the speed.
        if !self.cfg.enable_bunnyhopping && self.cfg.bunnyhop_cap > 0.0 {
            let speed = self.v.length();
            if speed > self.cfg.bunnyhop_cap {
                self.v *= self.cfg.bunnyhop_cap / speed;
            }
        }
        self.set_ground(false, None);
        if self.me.ducked || self.me.ducking {
            self.v.z = self.cfg.jump_impulse;
        } else {
            self.v.z += self.cfg.jump_impulse;
        }
        if self.me.stamina > 0.0 {
            self.v.z *= 1.0 - self.cfg.stamina_jump_scale * self.me.stamina;
        }
        self.me.stamina = self.cfg.jump_stamina;
        self.half_gravity();
        self.me.jump_held = true;
        // A full-volume step on the surface jumped from.
        if let Some(s) = self.me.surface.clone() {
            self.play_step(&s, 1.0);
        }
    }

    /// Stuck recovery: about once a second (every tick while recovering),
    /// test the box; if it's stuck, try one nudge from the table, at most
    /// every 0.05 s. Returns false when movement is skipped this tick.
    fn check_stuck(&mut self) -> bool {
        self.me.ticks += 1;
        self.me.time += self.dt;
        let interval = ((1.0 / self.dt) as u64).max(1);
        if self.me.nudge == 0 && !self.me.ticks.is_multiple_of(interval) {
            return true;
        }
        if !self.trace.solid(self.me.ducked, self.feet) {
            self.me.nudge = 0;
            return true;
        }
        if self.me.time - self.me.last_nudge < STUCK_NUDGE_GAP {
            return false;
        }
        self.me.last_nudge = self.me.time;
        let table = nudge_table();
        let at = self.feet + table[self.me.nudge % table.len()];
        self.me.nudge = (self.me.nudge + 1) % table.len();
        if self.trace.solid(self.me.ducked, at) {
            return false;
        }
        self.feet = at;
        self.me.nudge = 0;
        true
    }

    fn tick(&mut self, intent: &Intent) {
        // Move input: full keys send key_speed, rescaled to the max speed.
        // Walking (+speed) lowers the max speed itself (CS:S, measured): the
        // keys rescale to it, and swimming's wish speed is capped by it.
        let walk = if intent.walk { self.cfg.walk_speed } else { 1.0 };
        let max_speed = self.player_maxspeed.min(self.cfg.maxspeed) * walk;
        let (mut f, mut s) = (
            intent.move_axis.y * self.cfg.key_speed + self.push_input.x,
            intent.move_axis.x * self.cfg.key_speed + self.push_input.y,
        );
        let len = (f * f + s * s).sqrt();
        if len > max_speed {
            f *= max_speed / len;
            s *= max_speed / len;
        }
        self.me.duck_timer = (self.me.duck_timer - 1000.0 * self.dt).max(0.0);
        self.me.stamina = (self.me.stamina - 1000.0 * self.dt).max(0.0);
        self.me.swim_timer = (self.me.swim_timer - 1000.0 * self.dt).max(0.0);
        self.decay_punch();

        // Intent yaw 0 looks down engine -Z, which is Source yaw 90.
        let yaw = std::f32::consts::FRAC_PI_2 + intent.yaw;
        let forward = Vec3::new(yaw.cos(), yaw.sin(), 0.0);
        let right = Vec3::new(yaw.sin(), -yaw.cos(), 0.0);

        // Leftover base velocity becomes real velocity when no push
        // touched the player during the last tick (triggers.md, step 1).
        if !self.base_touched {
            self.v += self.base * (1.0 + self.dt / 2.0);
            self.base = Vec3::ZERO;
        }
        self.base_touched = false;
        if !self.check_stuck() {
            return;
        }
        // On a ladder, or moved by game code (teleport, spawn): full ground
        // detection now. Otherwise only rising fast removes the ground.
        // Taken off the ground by logic (push, teleport): stay off.
        let moved = self.me.last_feet.is_none_or(|f| f.distance_squared(self.feet) > 1e-4);
        if self.unground {
            self.set_ground(false, None);
        } else if self.me.ladder.is_some() || moved {
            self.categorize();
        } else if self.v.z > UNGROUND_VZ {
            self.set_ground(false, None);
        }
        if !self.me.on_ground {
            self.me.fall_speed = -self.v.z;
        }
        let water_before = self.me.water_level;
        self.footsteps(intent);
        let was_ducking = self.me.ducked || self.me.ducking;
        let mut scale = self.duck(intent.crouch);
        // CS:S (measured): input is slowed while duck is held, or when the
        // tick started ducked or mid-duck (on the ground or off it).
        if self.cfg.duck_slows_everywhere {
            scale = if intent.crouch || was_ducking {
                self.cfg.duck_speed
            } else {
                1.0
            };
        }
        f *= scale;
        s *= scale;

        self.ladder(intent, f, s, scale);
        if self.me.ladder.is_some() {
            // Ladder move: no gravity, friction or ground snapping.
            self.water_check();
            if !intent.jump {
                self.me.jump_held = false;
            }
            self.slide();
            self.me.last_feet = Some(self.feet);
            return;
        }

        self.water_check();
        if self.me.water_level < 2 {
            self.start_gravity();
            self.clamp_velocity();
        }
        if self.me.water_jump_time > 0.0 {
            self.me.water_jump_time -= 1000.0 * self.dt;
            if self.me.water_jump_time <= 0.0 || self.me.water_level == 0 {
                self.me.water_jump_time = 0.0;
            }
            self.v.x = self.me.water_jump_vel.x;
            self.v.y = self.me.water_jump_vel.y;
            self.slide();
            self.water_check();
            self.water_sound(water_before);
            self.me.last_feet = Some(self.feet);
            return;
        }
        if self.me.water_level >= 2 {
            if self.me.water_level == 2 {
                self.water_jump_check(intent);
            }
            if intent.jump && self.me.swim_timer <= 0.0 {
                // A swim stroke at most once a second while holding jump.
                self.me.swim_timer = 1000.0;
                self.sounds.push(("Player.Swim".into(), self.feet, None));
            }
            if intent.jump {
                // Jump does nothing while water-jumping: a water jump that
                // started this tick keeps its lift.
                if self.me.water_jump_time <= 0.0 {
                    self.v.z = if self.me.in_slime {
                        SWIM_JUMP_SLIME
                    } else {
                        SWIM_JUMP_WATER
                    };
                    self.me.on_ground = false;
                }
            } else {
                self.me.jump_held = false;
            }
            self.swim(intent, f, s, max_speed);
            self.categorize();
            if self.me.on_ground {
                self.v.z = 0.0;
            }
            self.water_sound(water_before);
            self.me.last_feet = Some(self.feet);
            return;
        }

        if intent.jump {
            let fresh = !self.me.jump_held || self.cfg.auto_bunnyhopping;
            if self.me.on_ground && fresh && !(self.me.ducked && self.me.ducking) {
                self.jump();
            } else {
                self.me.jump_held = true;
            }
        } else {
            self.me.jump_held = false;
        }
        if self.me.on_ground {
            self.v.z = 0.0;
            self.friction();
        }
        self.clamp_velocity();
        if self.me.on_ground {
            self.walk(f, s, forward, right, max_speed);
        } else {
            self.air(f, s, forward, right, max_speed);
        }
        self.categorize();
        self.clamp_velocity();
        if self.me.water_level < 2 {
            self.half_gravity();
            self.clamp_velocity();
        }
        if self.me.on_ground {
            self.v.z = 0.0;
            if self.me.fall_speed > 0.0 {
                if self.me.water_level == 0 && self.me.fall_speed > self.cfg.fall_safe {
                    self.fall_damage =
                        ((self.me.fall_speed - self.cfg.fall_safe) * self.cfg.fall_damage_per_speed).floor();
                }
                self.landing_sound(self.me.fall_speed);
                self.me.last_landing_speed = self.me.fall_speed;
                self.me.fall_speed = 0.0;
            }
        }
        self.water_sound(water_before);
        self.me.last_feet = Some(self.feet);
    }

    /// The punch angle's spring (movement spec, "Punch angle decay"), for
    /// its roll: the only part movement sets (recoil is the weapons').
    fn decay_punch(&mut self) {
        let (p, v) = (self.me.punch_roll, self.me.punch_roll_vel);
        if p * p <= 0.001 && v * v <= 0.001 {
            self.me.punch_roll = 0.0;
            self.me.punch_roll_vel = 0.0;
            return;
        }
        let dt = self.dt;
        let p = p + v * dt;
        let v = v * (1.0 - PUNCH_DAMPING * dt).max(0.0);
        let v = v - p * (PUNCH_SPRING * dt).clamp(0.0, 2.0);
        self.me.punch_roll = p.clamp(-89.0, 89.0);
        self.me.punch_roll_vel = v;
    }

    /// Entering or leaving water this tick splashes.
    fn water_sound(&mut self, before: u8) {
        if (before == 0) != (self.me.water_level == 0) {
            self.sounds.push(("Player.Swim".into(), self.feet, None));
        }
    }
}

fn hull_height(cfg: &SourceMovementConfig, ducked: bool) -> f32 {
    if ducked { cfg.duck_height } else { cfg.stand_height }
}

fn step(
    mut q: Query<(
        Entity,
        &Intent,
        &mut SourceMovement,
        &mut Transform,
        &mut Velocity,
        &mut MovementState,
        Option<&MaxSpeed>,
        Option<&mut BaseVelocity>,
        Option<&EntityGravity>,
        Option<&mut super::shadow::PhysicsTouch>,
        Option<&crate::core::MapControls>,
    )>,
    query: SpatialQuery,
    brushes: Option<Res<MapBrushes>>,
    water: Option<Res<MapWater>>,
    brush_colliders: Query<Entity, With<MapBrushCollider>>,
    (terrain, terrain_colliders, movers, tree, shadow_props): (
        Option<Res<MapTerrain>>,
        Query<Entity, With<MapTerrainCollider>>,
        Query<(Entity, &MovingSolid)>,
        Option<Res<MapBrushTree>>,
        super::shadow::ShadowProps,
    ),
    // Props that are there: a broken or killed one (body and collider
    // disabled until a round restart) neither blocks nor pushes back.
    props: Query<
        (Entity, &Transform, &PhysicsProp),
        (Without<SourceMovement>, Without<RigidBodyDisabled>, Without<ColliderDisabled>),
    >,
    surfaces: Option<Res<SurfaceGrid>>,
    prop_surfaces: Query<&PropSurface>,
    bank: Option<Res<SoundBank>>,
    mut play: MessageWriter<PlaySound>,
    mut damage: MessageWriter<Damage>,
    cfg: Res<SourceMovementConfig>,
    (clock, first): (Res<crate::core::SimClock>, Res<crate::core::FirstTimePredicted>),
    other_characters: Query<(Entity, &ColliderAabb, Option<&Health>), (With<Intent>, Without<SourceMovement>)>,
    (health, dead): (Query<&Health>, Query<(), With<ColliderDisabled>>),
) {
    let dt = clock.dt();
    // Every living character's box: Source hulls for Source movers, else
    // the collider's bounds. Those whose collider is off (the dead, also
    // those waiting to respawn with their health, unseen) block nobody.
    let alive = |e: Entity| (health.get(e).is_ok_and(|h| h.current > 0.0) || health.get(e).is_err()) && !dead.contains(e);
    let hull_box = |feet: Vec3, ducked: bool| {
        let height = if ducked { cfg.duck_height } else { cfg.stand_height };
        let (a, b) = (
            to_engine(feet + Vec3::new(-HALF_WIDTH, -HALF_WIDTH, 0.0)),
            to_engine(feet + Vec3::new(HALF_WIDTH, HALF_WIDTH, height)),
        );
        MapBrush::from_box(a.min(b), a.max(b))
    };
    let mut boxes: Vec<(Entity, MapBrush)> = q
        .iter()
        .filter(|(e, ..)| alive(*e))
        .map(|(e, _, me, t, ..)| {
            (
                e,
                hull_box(to_source(t.translation) - Vec3::Z * ORIGIN_ABOVE_FEET, me.ducked),
            )
        })
        .collect();
    boxes.extend(
        other_characters
            .iter()
            .filter(|(e, _, h)| h.is_none_or(|h| h.current > 0.0) && !dead.contains(*e))
            .map(|(e, aabb, _)| (e, MapBrush::from_box(aabb.min, aabb.max))),
    );
    // Characters (dead ones too) never block through physics casts.
    let characters: Vec<Entity> = q
        .iter()
        .map(|(e, ..)| e)
        .chain(other_characters.iter().map(|(e, ..)| e))
        .collect();
    // Moving solids (doors, platforms) are swept like brushes, and know
    // their owner (what the player stands on).
    let mover_brushes: Vec<(Entity, MapBrush)> = movers
        .iter()
        .filter(|(_, m)| m.solid)
        .flat_map(|(e, m)| m.brushes.iter().map(move |b| (e, b.clone())))
        .collect();
    let mover_velocity = |e: Entity| movers.get(e).map_or(Vec3::ZERO, |(_, m)| to_source(m.velocity));
    for (entity, intent, mut me, mut transform, mut vel, mut state, weapon_speed, mut base, gravity, touch, controls) in
        &mut q
    {
        // player_speedmod: the movement runs as if this many ticks passed
        // (specs/source/game_entities.md 1, lagged movement); 0 (or less:
        // our choice) holds it still, gravity included.
        let time_scale = controls.map_or(1.0, |c| c.time_scale);
        // A player parented to a map entity is carried by the logic.
        if time_scale <= 0.0 || controls.is_some_and(|c| c.parented) {
            continue;
        }
        let dt = dt * time_scale;
        // The mover stood on isn't part of the saved form (an entity): a
        // network client restores it into `MovementState::ground` after a
        // correction (`net::movers::restore_ground`). Otherwise the two
        // are the same here (written together at the end of each step).
        if me.ground_entity.is_none() && state.ground.is_some() {
            me.ground_entity = state.ground;
        }
        let mut others: Vec<MapBrush> = boxes
            .iter()
            .filter(|(e, _)| *e != entity)
            .map(|(_, b)| b.clone())
            .collect();
        let mut owners: Vec<Option<Entity>> = vec![None; others.len()];
        for (e, b) in &mover_brushes {
            others.push(b.clone());
            owners.push(Some(*e));
        }
        // With brushes swept exactly, physics queries skip the same brushes.
        // Players pass through multiplayer physics props (their own
        // collision group); props that collide stay in.
        let excluded = std::iter::once(entity)
            .chain(characters.iter().copied())
            .chain(brushes.as_ref().map(|_| brush_colliders.iter()).into_iter().flatten())
            .chain(terrain.as_ref().map(|_| terrain_colliders.iter()).into_iter().flatten())
            .chain(
                props
                    .iter()
                    .filter(|(_, _, p)| p.push != PushAway::Collide)
                    .map(|(e, ..)| e),
            );
        let (forward, right) = Mover::view(intent);
        let push_input = super::pushaway::push_back(
            super::pushaway::player_box(&transform, &me, &cfg),
            forward,
            right,
            props.iter().map(|(_, t, p)| (t, p)),
        );
        let prop_surface = |e: Entity| prop_surfaces.get(e).ok().map(|s| s.0.clone());
        let tracer = Tracer {
            query: &query,
            filter: SpatialQueryFilter::from_excluded_entities(excluded).with_mask(crate::core::SOLID_LAYERS),
            brushes: brushes.as_deref(),
            tree: tree.as_deref(),
            water: water.as_deref(),
            surfaces: surfaces.as_deref(),
            prop_surfaces: &prop_surface,
            sounds: bank.as_ref().map(|b| &*b.0),
            heights: (cfg.stand_height, cfg.duck_height),
            others: &others,
            terrain: terrain.as_deref(),
            owners: &owners,
        };
        let mut mover = Mover {
            cfg: &cfg,
            trace: &tracer,
            feet: to_source(transform.translation) - Vec3::Z * ORIGIN_ABOVE_FEET,
            v: to_source(vel.0),
            me: &mut me,
            dt,
            water_cache: [None; 3],
            push_input,
            sounds: Vec::new(),
            // The held weapon's speed (spec: MaxPlayerSpeed), else the default.
            player_maxspeed: weapon_speed.map_or(cfg.player_maxspeed, |s| s.0 / METERS_PER_UNIT),
            fall_damage: 0.0,
            base: base.as_ref().map_or(Vec3::ZERO, |b| to_source(b.velocity)),
            base_touched: base.as_ref().is_some_and(|b| b.touched),
            unground: base.as_ref().is_some_and(|b| b.unground),
            gravity_scale: gravity.map_or(1.0, |g| g.0),
            mover_velocity: &mover_velocity,
            touched: Vec::new(),
            ground_collider: None,
            wish: Vec3::ZERO,
        };
        let start = mover.feet;
        mover.tick(intent);
        let (feet, v) = (mover.feet, mover.v);
        if let Some(mut touch) = touch {
            *touch = shadow_props.touch(&mover.touched, mover.ground_collider, start, feet, mover.wish, dt);
        }
        if let Some(b) = base.as_mut() {
            let new = BaseVelocity {
                velocity: to_engine(mover.base),
                touched: mover.base_touched,
                unground: false,
            };
            if b.velocity != new.velocity || b.touched != new.touched || b.unground {
                **b = new;
            }
        }
        // Damage and sounds only the first time a command runs.
        if !first.0 {
            mover.sounds.clear();
        }
        if mover.fall_damage > 0.0 && first.0 {
            // Health is normalized: 1.0 = 100 points. No attacker, no
            // armour (measured: armour doesn't absorb it).
            damage.write(Damage {
                force: bevy::math::Vec3::ZERO,
                target: entity,
                attacker: None,
                amount: mover.fall_damage / 100.0,
                point: to_engine(feet),
                dir: Vec3::NEG_Y,
                hitgroup: Hitgroup::Generic,
                kind: crate::core::DamageKind::Fall,
                weapon: None,
            });
            // Taking it plays `Player.FallDamage` (`super::pain`).
        }
        for (entry, at, volume) in mover.sounds.drain(..) {
            play.write(PlaySound {
                pitch: None,
                entry,
                at: Some(to_engine(at)),
                volume,
                source: Some(entity),
                channel: Some(CHAN_BODY),
            });
        }
        transform.translation = to_engine(feet + Vec3::Z * ORIGIN_ABOVE_FEET);
        // Later movers this tick see where this one went.
        if let Some((_, b)) = boxes.iter_mut().find(|(e, _)| *e == entity) {
            *b = hull_box(feet, me.ducked);
        }
        vel.0 = to_engine(v);
        *state = MovementState {
            on_ground: me.on_ground,
            crouching: me.ducked || (me.ducking && intent.crouch),
            sprinting: false,
            eye_offset: Vec3::Y * (me.eye - ORIGIN_ABOVE_FEET) * METERS_PER_UNIT,
            hull_min: {
                let (a, b) = (
                    to_engine(Vec3::new(-HALF_WIDTH, -HALF_WIDTH, -ORIGIN_ABOVE_FEET)),
                    to_engine(Vec3::new(HALF_WIDTH, HALF_WIDTH, hull_height(&cfg, me.ducked) - ORIGIN_ABOVE_FEET)),
                );
                a.min(b)
            },
            hull_max: {
                let (a, b) = (
                    to_engine(Vec3::new(-HALF_WIDTH, -HALF_WIDTH, -ORIGIN_ABOVE_FEET)),
                    to_engine(Vec3::new(HALF_WIDTH, HALF_WIDTH, hull_height(&cfg, me.ducked) - ORIGIN_ABOVE_FEET)),
                );
                a.max(b)
            },
            ground: me.ground_entity,
            on_ladder: me.ladder.is_some(),
            view_roll: me.punch_roll.to_radians(),
        };
    }
}
