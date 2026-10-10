//! Core vocabulary shared by every game plugin. Keep this small: a concept
//! moves in here only after two or three games need it (README, "rule of
//! three").

use bevy::prelude::*;
use serde::{Deserialize, Serialize, de::DeserializeOwned};

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
    /// This tick's command number (Source's usercmd `command_number`):
    /// shared randoms (shot spread, recoil) are seeded from it. Stamped
    /// every tick after the rules (`SimSet::Commands`, `number_commands`);
    /// intent sources needn't set it.
    pub command: u32,
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
#[derive(Component, Reflect, Default, Clone, Copy, Debug, Deref, DerefMut, Serialize, Deserialize)]
#[reflect(Component)]
pub struct Velocity(pub Vec3);

/// Published by the active Movement implementation every tick. Other slots
/// (weapons, HUD, animation) read this instead of depending on a specific
/// movement implementation.
#[derive(Component, Reflect, Default, Clone, Debug, Serialize, Deserialize)]
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
    /// Not sent to a predicting client (entity ids differ).
    #[serde(skip)]
    pub ground: Option<Entity>,
    /// Climbing a ladder (movements that have them).
    pub on_ladder: bool,
    /// The eye's roll from a view punch (radians; positive tilts the view
    /// clockwise, Source's roll): hard landings kick it.
    pub view_roll: f32,
}

/// A velocity that moves a character without being part of its own
/// velocity (Source base velocity: push triggers, leaving a moving
/// platform), engine space, m/s. Movement implementations that model it
/// read and consume it (specs/source/triggers.md, trigger_push).
#[derive(Component, Reflect, Default, Clone, Copy, Debug, Serialize, Deserialize)]
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

// Serde: replicated to clients (`net`).
#[derive(Component, Reflect, Clone, Copy, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
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

#[derive(Component, Reflect, Default, Clone, Copy, Debug, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[reflect(Component)]
pub struct Team(pub u8);

/// A character's own dice seed, for rolls that should repeat run to run
/// (bots' route noise and choices, buying, bullet spread). Entity ids
/// won't do: every spawn shifts them, resources included (Bevy 0.19
/// resources are entities). Bots get one from their number and team
/// (`bot::add_bot`); without one, systems fall back to the entity id.
#[derive(Component, Reflect, Default, Clone, Copy, Debug, PartialEq, Eq)]
#[reflect(Component)]
pub struct Seed(pub u64);

impl Seed {
    /// The seed of `entity`, or one made from its id without a `Seed`.
    pub fn of(seed: Option<&Seed>, entity: Entity) -> u64 {
        seed.map_or(entity.to_bits(), |s| s.0)
    }
}

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
    /// The weapon that dealt it when that isn't what the attacker holds
    /// now (a thrown grenade), for kill notices; None: the held one.
    pub weapon: Option<&'static str>,
    /// The push that came with it (kg·m/s, engine space), already given
    /// to a moving body; zero for none. Props read it (a frozen prop
    /// that a strong enough push unfreezes).
    pub force: Vec3,
}

/// An explosion to apply (radius damage and pushes, as a grenade's blast
/// with this magnitude and radius): from map logic (exploding props).
/// `damage` is normalized like `Damage::amount`, `radius` in meters.
#[derive(Message, Clone, Debug)]
pub struct Explosion {
    pub origin: Vec3,
    pub damage: f32,
    pub radius: f32,
    /// Who the damage counts for (kill notices, friendly fire).
    pub attacker: Option<Entity>,
    /// What exploded (it takes none of the damage).
    pub inflictor: Option<Entity>,
    /// A sound entry to play at the origin, if any.
    pub sound: Option<String>,
    /// The weapon kills count as (kill notices); None: a map explosion
    /// (`weapon::grenade::EXPLOSION_WEAPON`).
    pub weapon: Option<&'static str>,
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
    /// Fire (Source's burn type): env_fire and entity flames.
    Burn,
}

/// Give a character items a map names (`weapon_ak47`, `item_kevlar`, with
/// counts), after taking all its weapons when `strip`: the logic layer's
/// game_player_equip and player_weaponstrip; the weapon layer gives them.
#[derive(Message, Clone, Debug, PartialEq)]
pub struct Equip {
    pub target: Entity,
    pub items: Vec<(String, u32)>,
    pub strip: bool,
}

/// Damage kinds a character takes none of (a map's damage filter on it:
/// bhop and surf maps turn fall damage off this way).
#[derive(Component, Clone, Debug, Default, PartialEq, Eq)]
pub struct DamageFilter {
    pub blocked: Vec<DamageKind>,
}

/// A character's command buttons as Source numbers them (the `IN_*` bits
/// map entities read and take away: game_ui, player_speedmod).
pub mod buttons {
    pub const ATTACK: u32 = 1;
    pub const JUMP: u32 = 2;
    pub const DUCK: u32 = 4;
    pub const FORWARD: u32 = 8;
    pub const BACK: u32 = 16;
    pub const USE: u32 = 32;
    pub const MOVELEFT: u32 = 512;
    pub const MOVERIGHT: u32 = 1024;
    pub const ATTACK2: u32 = 2048;
    pub const RELOAD: u32 = 8192;
    pub const SPEED: u32 = 131072;
    pub const ZOOM: u32 = 524288;

    /// The buttons an intent holds.
    pub fn of(i: &super::Intent) -> u32 {
        let mut b = 0;
        for (on, bit) in [
            (i.fire, ATTACK),
            (i.jump, JUMP),
            (i.crouch, DUCK),
            (i.move_axis.y > 0.0, FORWARD),
            (i.move_axis.y < 0.0, BACK),
            (i.use_key, USE),
            (i.move_axis.x < 0.0, MOVELEFT),
            (i.move_axis.x > 0.0, MOVERIGHT),
            (i.secondary, ATTACK2),
            (i.reload, RELOAD),
            (i.walk || i.sprint, SPEED),
        ] {
            if on {
                b |= bit;
            }
        }
        b
    }
}

/// What map entities do to a character's controls (specs/source/
/// game_entities.md 1-2, viewcontrol_and_templates.md 1): player_speedmod's
/// movement clock and taken buttons, game_ui's "at controls", a camera's
/// freeze. Applied to its `Intent` each tick before movement
/// (`apply_map_controls`, on the server and on the client predicting it:
/// a predicted part). Absent: nothing changed.
#[derive(Component, Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MapControls {
    /// Its movement runs this many ticks per tick (player_speedmod's
    /// lagged movement; 0 holds it still). Reset to 1 when it spawns.
    pub time_scale: f32,
    /// Buttons taken away (`buttons` bits), per entity that took them
    /// (a key the logic gives each).
    pub disabled: Vec<(u32, u32)>,
    /// Its movement inputs are zeroed (game_ui "Freeze Player").
    pub at_controls: bool,
    /// Frozen (point_viewcontrol "Freeze Player"): no movement, buttons
    /// or impulse; the view held at these angles (yaw, pitch, radians).
    pub frozen: Option<(f32, f32)>,
    /// Its weapons put away (bits: 1 player_speedmod, 2 game_ui, 4 a
    /// camera): no firing, no view model.
    pub weapon_hidden: u8,
    /// The HUD hidden (player_speedmod "Suppress HUD").
    pub hud_hidden: bool,
    /// Takes no damage (viewing through a point_viewcontrol).
    pub invulnerable: bool,
    /// The buttons of this tick's command after the above (what game_ui
    /// reads): `buttons` bits.
    pub buttons: u32,
    /// Alive last tick (a respawn resets the movement clock).
    pub alive: bool,
}

impl Default for MapControls {
    fn default() -> Self {
        Self {
            time_scale: 1.0,
            disabled: Vec::new(),
            at_controls: false,
            frozen: None,
            weapon_hidden: 0,
            hud_hidden: false,
            invulnerable: false,
            buttons: 0,
            alive: true,
        }
    }
}

impl MapControls {
    /// All buttons taken away.
    pub fn disabled_buttons(&self) -> u32 {
        self.disabled.iter().fold(0, |a, (_, b)| a | b)
    }
}

/// Apply each character's `MapControls` to its intent: taken buttons
/// released, movement zeroed at controls, everything but the held view
/// zeroed when frozen, no firing with weapons put away; the buttons left
/// recorded for the logic. A respawn resets the movement clock (the
/// taken buttons and the hidden HUD stay, as in the base code; spec open
/// question 3); a camera's view and freeze go too (our choice,
/// viewcontrol_and_templates.md open question 2).
pub fn apply_map_controls(
    mut q: Query<(Entity, &mut Intent, &mut MapControls, Option<&Health>, Has<MapView>)>,
    mut commands: Commands,
) {
    for (e, mut intent, mut c, health, viewing) in &mut q {
        let alive = health.is_none_or(|h| h.current > 0.0);
        if alive && !c.alive {
            c.time_scale = 1.0;
            c.frozen = None;
            c.invulnerable = false;
            c.weapon_hidden &= !4;
            if viewing {
                commands.entity(e).remove::<MapView>();
            }
        }
        if c.alive != alive {
            c.alive = alive;
        }
        let off = c.disabled_buttons();
        let i = &mut *intent;
        if off & buttons::JUMP != 0 {
            i.jump = false;
        }
        if off & buttons::DUCK != 0 {
            i.crouch = false;
        }
        if off & buttons::USE != 0 {
            i.use_key = false;
        }
        if off & buttons::SPEED != 0 {
            i.walk = false;
            i.sprint = false;
        }
        if off & buttons::ATTACK != 0 {
            i.fire = false;
        }
        if off & buttons::ATTACK2 != 0 {
            i.secondary = false;
        }
        if let Some((yaw, pitch)) = c.frozen {
            let (command, select) = (i.command, i.select);
            *i = Intent {
                yaw,
                pitch,
                command,
                select,
                ..default()
            };
        }
        let pressed = buttons::of(i);
        if c.at_controls {
            i.move_axis = Vec2::ZERO;
        }
        if c.weapon_hidden != 0 {
            i.fire = false;
            i.secondary = false;
            i.reload = false;
        }
        if c.buttons != pressed {
            c.buttons = pressed;
        }
    }
}

/// Where a character views the world from instead of its eye (a
/// point_viewcontrol camera; viewcontrol_and_templates.md 1.5): engine
/// space. The server's logic moves it; the client predicting the
/// character hears it with its own state.
#[derive(Component, Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct MapView {
    pub origin: Vec3,
    pub rotation: Quat,
}

/// A map entity changed a character's score (game_score): kills, or with
/// `team` its team's score; `allow_negative` as the entity's flag 1
/// (specs/source/game_entities.md 4). The rules apply it.
#[derive(Message, Clone, Debug, PartialEq)]
pub struct ScoreChange {
    pub target: Entity,
    pub points: i32,
    pub team: bool,
    pub allow_negative: bool,
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

/// A character says a radio command to its team (Counter-Strike's radio):
/// `command` is the game's console name for it (`coverme`, `enemyspot`,
/// ...; `fireinhole` for a thrown grenade). The client resolves it through
/// the map's `map::radio::RadioCommands` and plays it if it hears it.
#[derive(Message, Clone, Debug, PartialEq)]
pub struct Radio {
    pub sender: Entity,
    pub command: String,
}

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
/// Every layer but ragdolls, physics shadows (and the loose items' copy
/// of the terrain): the mask for gameplay spatial queries.
pub const SOLID_LAYERS: avian3d::prelude::LayerMask =
    avian3d::prelude::LayerMask(!(1 << 1) & !(1 << 3) & !SHADOW_LAYER.0);
/// Physics layer of players' physics shadows (specs/cs_source/physics_props.md
/// 4.1, `map::prop_physics::PhysicsShadow`): invisible bodies that only
/// touch the physics props players push. No spatial query should see
/// them: use `SOLID_LAYERS`, or `NOT_SHADOW` for queries of everything.
pub const SHADOW_LAYER: avian3d::prelude::LayerMask = avian3d::prelude::LayerMask(1 << 4);
/// Every layer but physics shadows.
pub const NOT_SHADOW: avian3d::prelude::LayerMask = avian3d::prelude::LayerMask(!SHADOW_LAYER.0);
/// Physics layer of loose items (dropped weapons): they collide with the
/// world, props and each other but never with characters (Source's
/// weapon collision group), so a weapon thrown from inside its dropper
/// isn't shoved out of them. Shots and other gameplay queries still see
/// them.
pub const ITEM_LAYER: avian3d::prelude::LayerMask = avian3d::prelude::LayerMask(1 << 2);
/// What a character's body collides with: solid things but loose items.
pub const CHARACTER_FILTER: avian3d::prelude::LayerMask = avian3d::prelude::LayerMask(SOLID_LAYERS.0 & !ITEM_LAYER.0);
/// The map's displacement surfaces as loose items collide with them (a
/// copy with its internal edges fixed, `map::spawn_map`); nothing else
/// touches or queries it.
pub const ITEM_GROUND_LAYER: avian3d::prelude::LayerMask = avian3d::prelude::LayerMask(1 << 3);

/// A speed cap the character's equipment imposes (e.g. the held weapon),
/// meters per second. Movement implementations that model it read it.
#[derive(Component, Reflect, Clone, Copy, Debug, Serialize, Deserialize)]
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

/// A character a person plays from another machine (a network server's
/// client): not this machine's `LocalPlayer`, and not a computer player.
#[derive(Component, Reflect, Default, Clone, Copy, Debug)]
#[reflect(Component)]
pub struct RemotePlayer;

/// A remote player whose client is still loading the map (joining, or
/// after a map change): not in the game yet. It is dead, not solid and
/// at no health, so nothing hits it; the rules don't spawn it or count it
/// on its team until its client has the map (`rules::enter_game`), as
/// CS:S keeps connecting players out of the game.
#[derive(Component, Reflect, Default, Clone, Copy, Debug)]
#[reflect(Component)]
pub struct Connecting;

/// Number of fixed ticks simulated so far. Tests and tools use it to step
/// the simulation by exact ticks.
#[derive(Resource, Reflect, Default, Clone, Copy, Debug)]
#[reflect(Resource)]
pub struct SimTick(pub u64);

/// The clock of the simulation step running now: Source's `curtime`
/// (tickbase × tick interval) and the tick's length. Code that prediction
/// runs again (`Predict`: weapon selection, movement, the weapon frame)
/// reads time from here, never from `Time`, so a replayed command sees the
/// time it first ran at. Every fixed tick starts by copying `Time<Fixed>`
/// into it (the same values `Time` gives in `FixedUpdate`); a prediction
/// replay sets it per command.
#[derive(Resource, Default, Clone, Copy, Debug, PartialEq)]
pub struct SimClock {
    /// The tick (`SimTick`) this time belongs to.
    pub tick: u64,
    /// Seconds since the simulation started.
    pub now: f64,
    /// Length of this tick.
    pub delta: std::time::Duration,
}

impl SimClock {
    /// Length of this tick, s.
    pub fn dt(&self) -> f32 {
        self.delta.as_secs_f32()
    }
}

/// What this process is in a game (docs/plans/active/multiplayer.md):
/// standalone (single player: everything runs, as a listen server with no
/// remote clients), the server, or a client of a remote server.
/// Server-only systems (bots, rules, damage, logic, objectives) run under
/// `authoritative`.
#[derive(Resource, Default, Clone, Copy, Debug, PartialEq, Eq)]
pub enum NetRole {
    #[default]
    Standalone,
    Server,
    Client,
}

/// A new map's tick length. The fixed clock keeps running (single player
/// and a network server alike), so its time stays with `Time<Virtual>`'s
/// (the clock frame systems and console commands read) and never goes
/// back while clients follow a server to the new map. Starting it over
/// put it behind by however long the game had run (the main menu, the
/// last map): times stamped in a frame or a command (a drop's touch
/// delay, a smoke cloud's age as drawn) were that far ahead of the
/// tick's. A client takes the server's clock anyway.
pub fn set_tick_length(world: &mut World, tick: std::time::Duration) {
    let client = world.get_resource::<NetRole>() == Some(&NetRole::Client);
    match world.get_resource_mut::<Time<Fixed>>() {
        Some(mut fixed) if !client => fixed.set_timestep(tick),
        _ => world.insert_resource(Time::<Fixed>::from_duration(tick)),
    }
}

/// Run condition: this process owns the game's outcome (standalone or
/// server; not a client, which only predicts its own player and draws
/// what the server sends).
pub fn authoritative(role: Option<Res<NetRole>>) -> bool {
    role.is_none_or(|r| *r != NetRole::Client)
}

/// Whether the commands running now run for the first time (Source's
/// `IsFirstTimePredicted`). A client re-running commands after a server
/// correction sets it false: side effects (sounds, damage, pushes) happen
/// only the first time. True outside a replay.
#[derive(Resource, Clone, Copy, Debug, PartialEq, Eq)]
pub struct FirstTimePredicted(pub bool);

impl Default for FirstTimePredicted {
    fn default() -> Self {
        Self(true)
    }
}

/// The parts of a tick prediction runs for a client's own player, as
/// schedules: weapon selection, movement and the weapon frame. A fixed tick
/// runs each from its place in `FixedUpdate` (`weapon::SelectWeapons`,
/// `SimSet::Movement`, `weapon::WeaponFrame`); a replay runs them in order
/// per command (`predict`). Their systems read time from `SimClock`,
/// write side effects only under `FirstTimePredicted`, and act on every
/// entity with the components they need (on a client, only its own player
/// will have them; others are drawn from snapshots).
#[derive(bevy::ecs::schedule::ScheduleLabel, Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Predict {
    Select,
    Movement,
    Weapons,
}

impl Predict {
    pub const ALL: [Predict; 3] = [Predict::Select, Predict::Movement, Predict::Weapons];
}

/// An exclusive system that runs the `stage` schedule, placed in
/// `FixedUpdate` where that stage belongs.
pub fn run_predicted(stage: Predict) -> impl FnMut(&mut World) {
    move |world: &mut World| {
        let _ = world.try_run_schedule(stage);
    }
}

/// Run one command's predicted tick (every `Predict` stage, in order) with
/// the intents, clock and flag already set.
pub fn predict(world: &mut World) {
    for stage in Predict::ALL {
        let _ = world.try_run_schedule(stage);
    }
}

/// The components prediction re-simulates (`PredictedAppExt::predicted`),
/// saved and restored for the entities a client predicts (its player and
/// the weapons it carries): restore the server's state, then replay the
/// commands since.
#[derive(Resource, Default)]
pub struct PredictedComponents(Vec<PredictedComponent>);

type SavedComponent = Option<Box<dyn std::any::Any + Send + Sync>>;

struct PredictedComponent {
    name: &'static str,
    save: fn(&World, Entity) -> SavedComponent,
    restore: fn(&mut World, Entity, &SavedComponent),
    /// How the server sends it to the client that predicts it
    /// (`PredictedAppExt::predicted_net`), if it does.
    net: Option<NetCodec>,
}

/// A predicted component's network form: the server's state of a
/// client's own player is sent as these bytes, compared byte for byte
/// with the client's prediction, and decoded over it on a mismatch.
#[derive(Clone, Copy)]
struct NetCodec {
    encode: fn(&World, Entity) -> Option<Vec<u8>>,
    decode: fn(&mut World, Entity, Option<&[u8]>),
}

/// The length `PredictedComponents::encode` writes for a component the
/// entity doesn't have.
const ABSENT: u16 = u16::MAX;

/// Some entities' predicted components as they were
/// (`PredictedComponents::save`).
#[derive(Default)]
pub struct PredictedSnapshot(Vec<(Entity, Vec<SavedComponent>)>);

impl PredictedComponents {
    /// The registered components' type names, in order.
    pub fn names(&self) -> Vec<&'static str> {
        self.0.iter().map(|c| c.name).collect()
    }

    pub fn save(&self, world: &World, entities: &[Entity]) -> PredictedSnapshot {
        PredictedSnapshot(
            entities
                .iter()
                .map(|e| (*e, self.0.iter().map(|c| (c.save)(world, *e)).collect()))
                .collect(),
        )
    }

    /// The networked predicted components of `entity` as one blob
    /// (`predicted_net`), in name order: per component a little-endian
    /// `u16` length (`u16::MAX`: absent) and its postcard bytes. Equal
    /// blobs are equal states, bit for bit.
    pub fn encode(&self, world: &World, entity: Entity) -> Vec<u8> {
        let mut out = Vec::new();
        for (_, net) in self.net_order() {
            match (net.encode)(world, entity) {
                Some(bytes) if bytes.len() < ABSENT as usize => {
                    out.extend_from_slice(&(bytes.len() as u16).to_le_bytes());
                    out.extend_from_slice(&bytes);
                }
                _ => out.extend_from_slice(&ABSENT.to_le_bytes()),
            }
        }
        out
    }

    /// Write a blob from `encode` over `entity`'s components (inserting
    /// or removing them as the blob says). A blob of another layout fails
    /// and changes nothing.
    pub fn decode(&self, world: &mut World, entity: Entity, blob: &[u8]) -> Result<(), String> {
        let parts = self.split(blob)?;
        for ((_, net), part) in self.net_order().into_iter().zip(parts) {
            (net.decode)(world, entity, part);
        }
        Ok(())
    }

    /// The names of the networked components two blobs disagree on
    /// (`cl_showerror`).
    pub fn differing(&self, a: &[u8], b: &[u8]) -> Vec<&'static str> {
        let (Ok(pa), Ok(pb)) = (self.split(a), self.split(b)) else {
            return vec!["(layout)"];
        };
        self.net_order()
            .into_iter()
            .zip(pa.into_iter().zip(pb))
            .filter(|(_, (x, y))| x != y)
            .map(|((name, _), _)| name)
            .collect()
    }

    /// A blob's parts by component name (None: absent), in its order:
    /// for tools that look inside a prediction error.
    pub fn parts<'a>(&self, blob: &'a [u8]) -> Result<Vec<(&'static str, Option<&'a [u8]>)>, String> {
        Ok(self.net_order().into_iter().map(|(n, _)| n).zip(self.split(blob)?).collect())
    }

    /// The networked components by name.
    fn net_order(&self) -> Vec<(&'static str, NetCodec)> {
        let mut v: Vec<(&'static str, NetCodec)> = self.0.iter().filter_map(|c| Some((c.name, c.net?))).collect();
        v.sort_by_key(|(name, _)| *name);
        v
    }

    fn split<'a>(&self, mut blob: &'a [u8]) -> Result<Vec<Option<&'a [u8]>>, String> {
        let short = || "predicted state blob too short".to_string();
        let mut parts = Vec::new();
        for _ in 0..self.net_order().len() {
            let (len, rest) = blob.split_first_chunk::<2>().ok_or_else(short)?;
            blob = rest;
            let len = u16::from_le_bytes(*len);
            if len == ABSENT {
                parts.push(None);
                continue;
            }
            let (part, rest) = blob.split_at_checked(len as usize).ok_or_else(short)?;
            parts.push(Some(part));
            blob = rest;
        }
        if !blob.is_empty() {
            return Err("predicted state blob too long".into());
        }
        Ok(parts)
    }

    /// Put each saved entity's predicted components back as they were
    /// (removing the ones it didn't have). Entities gone since are skipped.
    pub fn restore(&self, world: &mut World, snapshot: &PredictedSnapshot) {
        for (e, saved) in &snapshot.0 {
            if world.get_entity(*e).is_err() {
                continue;
            }
            for (c, value) in self.0.iter().zip(saved) {
                (c.restore)(world, *e, value);
            }
        }
    }
}

pub trait PredictedAppExt {
    /// Register a component prediction saves and restores.
    fn predicted<C: Component<Mutability = bevy::ecs::component::Mutable> + Clone>(&mut self) -> &mut Self;

    /// The same, and the server sends it to the client predicting it,
    /// which compares it with its own (`PredictedComponents::encode`).
    /// Entity fields stay out of its serde form (`#[serde(skip)]`): ids
    /// differ between server and client.
    fn predicted_net<C: Component<Mutability = bevy::ecs::component::Mutable> + Clone + Serialize + DeserializeOwned>(
        &mut self,
    ) -> &mut Self;

    /// A networked part of the predicted state that isn't one component
    /// (e.g. what a character carries: its inventory and the weapon
    /// entities in it), under `name` (a component already registered with
    /// `predicted` by that name keeps its save and restore): `encode` it
    /// for an entity (None: absent), `decode` it over the entity (None:
    /// absent).
    fn predicted_codec(
        &mut self,
        name: &'static str,
        encode: fn(&World, Entity) -> Option<Vec<u8>>,
        decode: fn(&mut World, Entity, Option<&[u8]>),
    ) -> &mut Self;
}

impl PredictedAppExt for App {
    fn predicted<C: Component<Mutability = bevy::ecs::component::Mutable> + Clone>(&mut self) -> &mut Self {
        register_predicted::<C>(self, None);
        self
    }

    fn predicted_net<C: Component<Mutability = bevy::ecs::component::Mutable> + Clone + Serialize + DeserializeOwned>(
        &mut self,
    ) -> &mut Self {
        register_predicted::<C>(
            self,
            Some(NetCodec {
                encode: |w, e| w.get::<C>(e).and_then(|c| postcard::to_allocvec(c).ok()),
                decode: |w, e, bytes| match bytes.and_then(|b| postcard::from_bytes::<C>(b).ok()) {
                    Some(c) => {
                        w.entity_mut(e).insert(c);
                    }
                    None => {
                        w.entity_mut(e).remove::<C>();
                    }
                },
            }),
        );
        self
    }

    fn predicted_codec(
        &mut self,
        name: &'static str,
        encode: fn(&World, Entity) -> Option<Vec<u8>>,
        decode: fn(&mut World, Entity, Option<&[u8]>),
    ) -> &mut Self {
        let net = Some(NetCodec { encode, decode });
        let mut registry = self.world_mut().get_resource_or_init::<PredictedComponents>();
        if let Some(c) = registry.0.iter_mut().find(|c| c.name == name) {
            c.net = net;
        } else {
            registry.0.push(PredictedComponent {
                name,
                net,
                save: |_, _| None,
                restore: |_, _, _| {},
            });
        }
        self
    }
}

fn register_predicted<C: Component<Mutability = bevy::ecs::component::Mutable> + Clone>(
    app: &mut App,
    net: Option<NetCodec>,
) {
    let name = std::any::type_name::<C>();
    let mut registry = app.world_mut().get_resource_or_init::<PredictedComponents>();
    let save: fn(&World, Entity) -> SavedComponent = |w, e| {
        w.get::<C>(e)
            .map(|c| Box::new(c.clone()) as Box<dyn std::any::Any + Send + Sync>)
    };
    let restore: fn(&mut World, Entity, &SavedComponent) = |w, e, v| match v.as_ref().and_then(|v| v.downcast_ref::<C>()) {
        Some(c) => {
            w.entity_mut(e).insert(c.clone());
        }
        None => {
            w.entity_mut(e).remove::<C>();
        }
    };
    if let Some(c) = registry.0.iter_mut().find(|c| c.name == name) {
        c.net = c.net.or(net);
        c.save = save;
        c.restore = restore;
    } else {
        registry.0.push(PredictedComponent {
            name,
            net,
            save,
            restore,
        });
    }
}

/// Round restarts so far (the rules count one up when a new round
/// starts). Each count puts the map's entities back as they spawned:
/// the logic layer re-creates its world, the map layer makes broken
/// brushes whole and clears gibs (Counter-Strike's round restart).
#[derive(Resource, Default, Clone, Copy, Debug, PartialEq, Eq)]
pub struct RoundRestarts(pub u32);

/// The rules hold every character still (a round's freeze time): they
/// may turn, look and buy, not move or shoot. The rules clear the
/// intents (`SimSet::Rules`); bots read it so standing still doesn't
/// count as being stuck.
#[derive(Resource, Default, Clone, Copy, Debug, PartialEq, Eq)]
pub struct FreezeTime(pub bool);

/// What a blast or a flash does to a character's hearing for a while
/// (Source: a player DSP preset): what they hear is mixed `mix` toward a
/// muffled copy (low-passed at `cutoff` Hz, times `wet_gain`), and a
/// ringing tone (`ring_hz`, amplitude `ring_gain` times the mix) plays.
/// Full for `hold` seconds, then fading out over `fade` (linearly, or
/// exponentially down to 1 % by its end).
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct HearingEffect {
    pub hold: f32,
    pub fade: f32,
    pub exponential: bool,
    pub mix: f32,
    pub cutoff: f32,
    pub wet_gain: f32,
    pub ring_hz: f32,
    pub ring_gain: f32,
}

impl HearingEffect {
    /// Strength (0-1) `t` seconds after it started.
    pub fn level(&self, t: f32) -> f32 {
        if t < 0.0 || t >= self.hold + self.fade {
            0.0
        } else if t <= self.hold {
            1.0
        } else {
            let x = (t - self.hold) / self.fade;
            if self.exponential {
                // e^-4.6 = 1 %: then cut.
                (-4.6 * x).exp()
            } else {
                1.0 - x
            }
        }
    }

    /// Seconds until it is over.
    pub fn length(&self) -> f32 {
        self.hold + self.fade
    }
}

/// A character's hearing is hit (`HearingEffect`): clients apply it to
/// what the local player hears.
#[derive(Message, Clone, Copy, Debug)]
pub struct Deafened {
    pub target: Entity,
    pub effect: HearingEffect,
}

/// A character's blindness (a flashbang): the screen goes white at
/// `alpha` (0-1), holds until `fade_start` (seconds, the simulation
/// clock) and fades out linearly by `end`. Clients draw it; bots don't
/// see while it is strong.
#[derive(Component, Clone, Copy, Debug, Default, PartialEq)]
pub struct Blinded {
    pub alpha: f32,
    pub fade_start: f64,
    pub end: f64,
}

impl Blinded {
    /// Whiteness at `now`.
    pub fn alpha_at(&self, now: f64) -> f32 {
        if now >= self.end {
            0.0
        } else if now <= self.fade_start || self.end <= self.fade_start {
            self.alpha
        } else {
            self.alpha * ((self.end - now) / (self.end - self.fade_start)) as f32
        }
    }

    /// The stronger of this blindness and `other` (a new flash only
    /// raises what is left of the current one).
    pub fn raised(self, other: Blinded, now: f64) -> Blinded {
        if self.alpha_at(now) >= other.alpha && self.end >= other.end {
            self
        } else if other.alpha >= self.alpha_at(now) && other.end >= self.end {
            other
        } else {
            // One is whiter, the other lasts longer: keep both maxima.
            Blinded {
                alpha: self.alpha_at(now).max(other.alpha),
                fade_start: self.fade_start.max(other.fade_start),
                end: self.end.max(other.end),
            }
        }
    }
}

/// Something sight doesn't pass (a smoke cloud): a sphere. A sight line
/// is blocked when more than `radius` of it lies inside (Counter-Strike
/// bots don't see through smoke; the exact rule isn't known, see
/// specs/cs_source/grenades.md Q16).
#[derive(Component, Clone, Copy, Debug, Default, PartialEq)]
pub struct SightBlocker {
    pub centre: Vec3,
    pub radius: f32,
}

impl SightBlocker {
    /// Length of the segment `from`-`to` inside the sphere.
    pub fn chord(&self, from: Vec3, to: Vec3) -> f32 {
        let d = to - from;
        let len = d.length();
        if len < 1e-6 || self.radius <= 0.0 {
            return 0.0;
        }
        let dir = d / len;
        let t = (self.centre - from).dot(dir);
        let closest = from + dir * t;
        let h2 = self.radius * self.radius - closest.distance_squared(self.centre);
        if h2 <= 0.0 {
            return 0.0;
        }
        let h = h2.sqrt();
        ((t + h).min(len) - (t - h).max(0.0)).max(0.0)
    }

    pub fn blocks(&self, from: Vec3, to: Vec3) -> bool {
        self.chord(from, to) > self.radius
    }
}

/// Whether characters on the same team hurt each other (CS:S
/// `mp_friendlyfire`, default 0). Team 0 (no team) is never friendly.
#[derive(Resource, Default, Clone, Copy, Debug, PartialEq, Eq)]
pub struct FriendlyFire(pub u8);

/// Whether team rules refuse this damage: teammates don't hurt each other
/// unless friendly fire is on (your own damage, e.g. falling, always
/// counts).
pub fn refused_by_team(d: &Damage, teams: &Query<&Team>, friendly_fire: Option<&FriendlyFire>) -> bool {
    let teammate = d
        .attacker
        .filter(|a| *a != d.target)
        .is_some_and(|a| matches!((teams.get(a), teams.get(d.target)), (Ok(x), Ok(y)) if x == y && x.0 != 0));
    teammate && !friendly_fire.is_some_and(|f| f.0 != 0)
}

/// What decides whether damage is refused before it reaches health:
/// team rules, a map's invulnerability and damage filters (not `God`,
/// which the health query leaves out).
#[derive(bevy::ecs::system::SystemParam)]
pub struct DamageRules<'w, 's> {
    teams: Query<'w, 's, &'static Team>,
    filters: Query<'w, 's, &'static DamageFilter>,
    controls: Query<'w, 's, &'static MapControls>,
    friendly_fire: Option<Res<'w, FriendlyFire>>,
}

impl DamageRules<'_, '_> {
    pub fn refuse(&self, d: &Damage) -> bool {
        refused_by_team(d, &self.teams, self.friendly_fire.as_deref())
            || self.controls.get(d.target).is_ok_and(|c| c.invulnerable)
            || self.filters.get(d.target).is_ok_and(|f| f.blocked.contains(&d.kind))
    }
}

/// Subtract damage from health; announce deaths once (public so other
/// systems can order themselves after it).
pub fn apply_damage(
    mut damage: MessageReader<Damage>,
    mut health: Query<&mut Health, Without<God>>,
    rules: DamageRules,
    mut died: MessageWriter<Died>,
) {
    for d in damage.read() {
        if rules.refuse(d) {
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
    /// Intents are final for the tick: each gets its command number.
    Commands,
    Movement,
    Weapons,
}

/// Stamp each intent with this tick's command number: the tick
/// (`SimClock::tick`; a network client's is the server tick its command
/// is for), offset by
/// the character's `Seed` (Source clients number their commands from
/// their own start, so two players firing on the same tick don't share a
/// spread pattern). Never from entity ids.
pub fn number_commands(clock: Res<SimClock>, mut intents: Query<(&mut Intent, Option<&Seed>)>) {
    for (mut intent, seed) in &mut intents {
        let n = (clock.tick as u32).wrapping_add(seed.map_or(0, |s| s.0 as u32));
        if intent.command != n {
            intent.command = n;
        }
    }
}

/// Count the tick and set the clock (`SimClock`) from `Time<Fixed>` (a
/// network client then sets it to the server tick it predicts,
/// `net::predict`).
pub fn start_tick(mut tick: ResMut<SimTick>, mut clock: ResMut<SimClock>, time: Res<Time<Fixed>>) {
    tick.0 += 1;
    *clock = SimClock {
        tick: tick.0,
        now: time.elapsed_secs_f64(),
        delta: time.delta(),
    };
}

pub struct CorePlugin;

impl Plugin for CorePlugin {
    fn build(&self, app: &mut App) {
        app.register_type::<Intent>()
            .register_type::<Velocity>()
            .register_type::<MovementState>()
            .register_type::<Health>()
            .register_type::<Team>()
            .register_type::<Seed>()
            .register_type::<MaxSpeed>()
            .register_type::<BaseVelocity>()
            .register_type::<EntityGravity>()
            .register_type::<Hitboxes>()
            .register_type::<Damageable>()
            .add_message::<Damage>()
            .add_message::<Equip>()
            .add_message::<Explosion>()
            .add_message::<Deafened>()
            .add_message::<Died>()
            .add_message::<Radio>()
            .init_resource::<FriendlyFire>()
            .add_systems(FixedUpdate, apply_damage.after(SimSet::Weapons).run_if(authoritative))
            .register_type::<SpawnPoint>()
            .register_type::<LocalPlayer>()
            .register_type::<Connecting>()
            .register_type::<SimTick>()
            .init_resource::<SimTick>()
            .init_resource::<SimClock>()
            .init_resource::<NetRole>()
            .init_resource::<FirstTimePredicted>()
            .init_resource::<PredictedComponents>()
            .add_systems(FixedFirst, start_tick)
            .init_resource::<RoundRestarts>()
            .init_resource::<FreezeTime>()
            .configure_sets(
                FixedUpdate,
                (SimSet::Rules, SimSet::Commands, SimSet::Movement, SimSet::Weapons).chain(),
            )
            .add_systems(FixedUpdate, number_commands.in_set(SimSet::Commands))
            .add_message::<ScoreChange>()
            .add_systems(
                FixedUpdate,
                apply_map_controls.after(SimSet::Rules).before(SimSet::Commands),
            )
            .predicted_net::<MapControls>()
            .add_systems(FixedUpdate, run_predicted(Predict::Movement).in_set(SimSet::Movement))
            .predicted_net::<Transform>()
            .predicted_net::<Velocity>()
            .predicted_net::<MovementState>()
            .predicted_net::<BaseVelocity>()
            .predicted_net::<MaxSpeed>();
        for stage in Predict::ALL {
            app.init_schedule(stage);
        }
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
    /// A convex solid from its face planes (`n . p <= d`, normals
    /// normalized), with the bevel planes a box sweep needs to be exact:
    /// the bounding box's planes and, through each edge, the planes along
    /// the edge and each axis that touch the solid only there (as vbsp
    /// adds to brushes). Without them a box swept by pushing the planes
    /// out stops short of a non-axial edge, in the air beside it: on a
    /// surf ramp built of pieces, a ghost wall at each seam.
    pub fn from_planes(mut planes: Vec<(Vec3, f32)>) -> Self {
        let n = planes.len();
        // Corners: where three planes meet inside all the others.
        let mut corners: Vec<Vec3> = Vec::new();
        for i in 0..n {
            for j in i + 1..n {
                for k in j + 1..n {
                    let ((n1, d1), (n2, d2), (n3, d3)) = (planes[i], planes[j], planes[k]);
                    let denom = n1.dot(n2.cross(n3));
                    if denom.abs() < 1e-6 {
                        continue;
                    }
                    let p = (n2.cross(n3) * d1 + n3.cross(n1) * d2 + n1.cross(n2) * d3) / denom;
                    if planes.iter().all(|(m, d)| m.dot(p) <= d + 1e-3) {
                        corners.push(p);
                    }
                }
            }
        }
        let on = |p: Vec3, (m, d): (Vec3, f32)| (m.dot(p) - d).abs() < 1e-3;
        let min = corners.iter().fold(Vec3::MAX, |a, c| a.min(*c));
        let max = corners.iter().fold(Vec3::MIN, |a, c| a.max(*c));
        let add = |planes: &mut Vec<(Vec3, f32)>, m: Vec3, d: f32| {
            if !planes.iter().any(|(q, e)| q.dot(m) > 0.9999 && (e - d).abs() < 1e-4) {
                planes.push((m, d));
            }
        };
        for (m, d) in Self::from_box(min, max).planes {
            add(&mut planes, m, d);
        }
        // Edges: two face planes that share at least two corners.
        for i in 0..n {
            for j in i + 1..n {
                let ends: Vec<Vec3> = corners
                    .iter()
                    .copied()
                    .filter(|p| on(*p, planes[i]) && on(*p, planes[j]))
                    .collect();
                let (Some(a), Some(b)) = (ends.first(), ends.iter().find(|p| p.distance(ends[0]) > 1e-4)) else {
                    continue;
                };
                let e = *b - *a;
                for axis in [Vec3::X, Vec3::Y, Vec3::Z] {
                    let Some(m) = e.cross(axis).try_normalize() else { continue };
                    for m in [m, -m] {
                        let d = m.dot(*a);
                        // A bevel touches the solid along the edge only.
                        if corners.iter().all(|c| m.dot(*c) <= d + 1e-3) {
                            add(&mut planes, m, d);
                        }
                    }
                }
            }
        }
        Self {
            planes,
            min,
            max,
            ladder: false,
            surface: None,
        }
    }

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

/// The map's BSP tree over the leading `MapBrushes` (Source maps; the
/// world's brushes). A Source trace tests brushes leaf by leaf down this
/// tree, so of two faces hit at the same distance (a ladder flush with a
/// player clip) the one reached first wins. Engine space.
#[derive(Resource, Clone, Debug, Default)]
pub struct MapBrushTree {
    pub nodes: Vec<BrushTreeNode>,
    /// Each leaf's brushes (indices into `MapBrushes`), in stored order.
    pub leaves: Vec<Vec<u32>>,
    /// Root: a node index (>= 0) or leaf -1-index.
    pub root: i32,
}

#[derive(Clone, Debug)]
pub struct BrushTreeNode {
    /// Splitting plane n.p = dist; children[0] is in front of it.
    pub normal: Vec3,
    pub dist: f32,
    pub children: [i32; 2],
}

impl MapBrushTree {
    /// The order a box (half size `half`) swept from centre `from` to `to`
    /// meets the brushes: leaves near the box's path, front to back along
    /// the move (in front of a split plane first when moving along it),
    /// each brush at its first leaf. `slop` widens the box when deciding
    /// which side of a plane it touches.
    pub fn sweep_order(&self, half: Vec3, from: Vec3, to: Vec3, slop: f32) -> Vec<u32> {
        let mut out = Vec::new();
        let mut seen = std::collections::HashSet::new();
        let mut stack = vec![self.root];
        while let Some(child) = stack.pop() {
            if child < 0 {
                for &b in self.leaves.get((-child - 1) as usize).map_or(&[][..], |l| &l[..]) {
                    if seen.insert(b) {
                        out.push(b);
                    }
                }
                continue;
            }
            let Some(node) = self.nodes.get(child as usize) else { continue };
            let offset = node.normal.abs().dot(half) + slop;
            let (t1, t2) = (node.normal.dot(from) - node.dist, node.normal.dot(to) - node.dist);
            if t1 >= offset && t2 >= offset {
                stack.push(node.children[0]);
            } else if t1 < -offset && t2 < -offset {
                stack.push(node.children[1]);
            } else {
                // The side the move starts on first: the back when moving
                // toward the front; the front when moving along the plane.
                let first = usize::from(t1 < t2);
                stack.push(node.children[first ^ 1]);
                stack.push(node.children[first]);
            }
        }
        out
    }
}

/// Marks the physics collider built from the same brushes, so movement that
/// sweeps `MapBrushes` itself can leave it out of physics queries.
#[derive(Component, Debug)]
pub struct MapBrushCollider;

/// A prop collider's surface property name (lower-case), reported by traces
/// that hit it (footsteps).
#[derive(Component, Clone, Debug)]
pub struct PropSurface(pub String);
