//! Thrown grenades for any game (specs/cs_source/grenades.md gives the
//! Counter-Strike rules this follows): a `Throwable` weapon part (a count
//! of grenades, pull the pin on attack, throw on release), the flying
//! `Projectile` (a small box under its own gravity that mirrors its
//! velocity on every contact and keeps part of its speed, stopping dead
//! when slow; a fuse checked every few ticks), and what it does when it
//! goes off (`GrenadeEffect`): a blast (radius damage through anything but
//! the world, a push on loose bodies, a scorch mark), a flash (blinds
//! everyone who sees it: `core::Blinded`) or a smoke cloud (`SmokeCloud`,
//! with a `core::SightBlocker`). Games fill in the numbers and draw the
//! effects from `Detonated`.
//!
//! Units: meters, seconds, damage normalized like `Health`. Pitch-based
//! throw rules use degrees with the Source convention (positive looks
//! down).

use avian3d::prelude::*;
use bevy::prelude::*;

use super::{Armor, CHAN_WEAPON, Inventory, Weapon, WeaponEvent, WeaponEventKind, WeaponState, armor_split};
use crate::{
    core::{
        Blinded, Damage, DamageKind, Damageable, Died, Explosion, Health, Hitgroup, Intent, MapWater, MovementState, Radio,
        RoundRestarts, SOLID_LAYERS, SightBlocker, Velocity,
    },
    map::{
        MapPropCollider, PlaySound,
        decal::{DecalGroup, PlaceDecal},
        loose::ShownItem,
        particles::ParticleRng,
    },
};

/// Source's voice channel (grenade bounces).
const CHAN_VOICE: u8 = 2;
/// Timers due on this tick fire on it (see `TIME_SLACK` in the frame).
const TIME_SLACK: f64 = 1e-5;
/// Gap kept between the box and what it touched, m (so a resting or
/// sliding box doesn't start its next sweep inside the surface).
const SKIN: f32 = 1e-3;

pub(super) fn plugin(app: &mut App) {
    app.add_message::<Detonated>()
        .add_message::<PlaceDecal>()
        .add_message::<Radio>()
        .init_resource::<GrenadeRng>()
        .init_resource::<GrenadeRadio>()
        .add_systems(
            FixedUpdate,
            (
                (throw_frame, fly, explosions, smoke_clouds)
                    .chain()
                    .after(super::WeaponFrame)
                    .in_set(crate::core::SimSet::Weapons),
                drop_primed
                    .after(crate::core::apply_damage)
                    .before(super::drop::drop_on_death),
                clear_on_restart
                    .after(crate::core::SimSet::Rules)
                    .before(crate::core::SimSet::Movement),
            ),
        );
}

#[derive(Resource)]
struct GrenadeRng(ParticleRng);

/// `sv_ignoregrenaderadio`: 1 stops the "Fire in the hole!" radio call
/// on a throw (spec 2).
#[derive(Resource, Default)]
pub struct GrenadeRadio(pub u8);

impl Default for GrenadeRng {
    fn default() -> Self {
        Self(ParticleRng::new(0x6E_4ADE))
    }
}

// ---------------------------------------------------------------------------
// Parts

/// How the throw direction and speed follow the view (spec 3): the pitch
/// is bent `p' = lift + p x scale` (degrees, positive down), the speed is
/// `min(max, per_deg x (90 - p'))`, the projectile starts `forward` ahead
/// of the eye and takes the thrower's velocity on top.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ThrowRule {
    pub pitch_lift: f32,
    pub pitch_scale: f32,
    /// m/s per degree.
    pub speed_per_deg: f32,
    pub speed_max: f32,
    pub forward: f32,
}

impl ThrowRule {
    /// The bent pitch for a view pitch `p` (degrees, positive down).
    pub fn pitch(&self, p: f32) -> f32 {
        self.pitch_lift + p * self.pitch_scale
    }

    /// Speed for a bent pitch, m/s.
    pub fn speed(&self, bent: f32) -> f32 {
        (self.speed_per_deg * (90.0 - bent)).min(self.speed_max)
    }

    /// (start, velocity) of a throw from `eye` looking at `yaw` (radians,
    /// ours) and `pitch` (radians, ours: positive up), by a thrower moving
    /// at `carried`.
    pub fn throw(&self, eye: Vec3, yaw: f32, pitch: f32, carried: Vec3) -> (Vec3, Vec3) {
        let bent = self.pitch(-pitch.to_degrees());
        let dir = Quat::from_euler(EulerRot::YXZ, yaw, -bent.to_radians(), 0.0) * Vec3::NEG_Z;
        (eye + dir * self.forward, dir * self.speed(bent) + carried)
    }
}

/// How a projectile flies and bounces (spec 4).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Flight {
    /// Half the side of its collision box, m.
    pub half: f32,
    /// Downward acceleration, m/s².
    pub gravity: f32,
    /// Speed kept per contact; times `player_elasticity` against
    /// characters; the product is capped at `elasticity_cap`.
    pub elasticity: f32,
    pub player_elasticity: f32,
    pub elasticity_cap: f32,
    /// A contact leaving it slower than this stops it, m/s.
    pub stop_speed: f32,
    /// Contacts with a normal's up part above this are floors.
    pub floor_normal: f32,
    /// Damage dealt to breakables it hits; its velocity after breaking
    /// through (factor).
    pub breakable_damage: f32,
    pub breakable_slowdown: f32,
    /// Velocity factor per check while in water; vertical velocity factor
    /// on entering water.
    pub water_slowdown: f32,
    pub water_entry: f32,
    /// Each velocity component is clamped to this, m/s.
    pub max_velocity: f32,
    /// Seconds between fuse checks (rounded to whole ticks).
    pub check_interval: f32,
    /// Initial spin: roll rate, and the pitch rate's range (radians/s).
    pub spin_roll: f32,
    pub spin_pitch: f32,
}

/// Mirror `v` about the surface with normal `n` and keep part of its speed
/// (spec 4, contact resolution). `player`: the surface is a character.
/// Returns the new velocity and whether it comes to rest (a floor contact
/// slower than the stop speed). A wall contact slower than the stop speed
/// leaves it with no velocity, to fall from there.
pub fn bounce(v: Vec3, n: Vec3, player: bool, f: &Flight, snap: f32) -> (Vec3, bool) {
    let e = (f.elasticity * if player { f.player_elasticity } else { 1.0 }).clamp(0.0, f.elasticity_cap);
    let mut r = v - 2.0 * v.dot(n) * n;
    for c in [&mut r.x, &mut r.y, &mut r.z] {
        if c.abs() < snap {
            *c = 0.0;
        }
    }
    r *= e;
    let slow = r.length() < f.stop_speed;
    if n.y > f.floor_normal {
        if slow { (Vec3::ZERO, true) } else { (r, false) }
    } else if slow {
        (Vec3::ZERO, false)
    } else {
        (r, false)
    }
}

/// The parabola step of one tick (spec 4 step 4): the move and the new
/// velocity.
pub fn gravity_step(v: Vec3, gravity: f32, dt: f32) -> (Vec3, Vec3) {
    let vy = v.y - gravity * dt;
    (
        Vec3::new(v.x * dt, (v.y + vy) / 2.0 * dt, v.z * dt),
        Vec3::new(v.x, vy, v.z),
    )
}

/// Radius damage (spec 5.3): `damage` at the centre, falling linearly to
/// zero at `radius`.
pub fn blast_damage(damage: f32, radius: f32, distance: f32) -> f32 {
    (damage - distance * damage / radius).max(0.0)
}

/// A blast (the HE grenade, spec 5).
#[derive(Clone, Debug)]
pub struct Blast {
    pub damage: f32,
    pub radius: f32,
    /// As `DamageEffect::armor_ratio`; blasts hit no hitgroup, so armour
    /// without a helmet covers them.
    pub armor_ratio: Option<f32>,
    pub quantum: f32,
    /// The ground under the blast: probed from this far above to this far
    /// below the grenade, m; the blast point is pulled out of the surface
    /// by `pull_out`.
    pub probe_up: f32,
    pub probe_down: f32,
    pub pull_out: f32,
    /// Damage is traced from this far above the blast point.
    pub src_lift: f32,
    /// Characters are aimed at between these fractions of their eye
    /// height above the feet.
    pub body_target: (f32, f32),
    /// Push on loose bodies, kg·m/s, times a random factor in `jitter`.
    pub force: f32,
    pub force_jitter: (f32, f32),
    /// A pushed body gets at most this speed from it, m/s.
    pub max_push_speed: f32,
    /// Decal group placed on the probed ground.
    pub scorch: Option<String>,
    pub sound: Option<String>,
}

/// How strongly a flash blinds someone (a game's model): from the distance
/// to the grenade (m) and the facing `k` (the cosine between the view and
/// the direction to it): (peak whiteness 0-1, hold seconds, fade seconds),
/// or None when it doesn't blind.
pub type FlashModel = fn(distance: f32, facing: f32) -> Option<(f32, f32, f32)>;

#[derive(Clone, Debug)]
pub struct Flash {
    pub model: FlashModel,
    pub sound: Option<String>,
}

/// A smoke cloud (spec 7): grows to `max_radius` over `expand_time`, holds,
/// fades between `fade_start` and `fade_end` (seconds after it starts).
#[derive(Clone, Debug)]
pub struct Smoke {
    pub max_radius: f32,
    pub expand_time: f32,
    pub fade_start: f32,
    pub fade_end: f32,
    /// It pops only once the grenade moves slower than this (m/s).
    pub pop_speed: f32,
    /// The sight blocker's radius as a fraction of the cloud's.
    pub sight_radius: f32,
    pub sound: Option<String>,
}

#[derive(Clone, Debug)]
pub enum GrenadeEffect {
    Blast(Blast),
    Flash(Flash),
    Smoke(Smoke),
}

impl GrenadeEffect {
    pub fn kind(&self) -> GrenadeKind {
        match self {
            GrenadeEffect::Blast(_) => GrenadeKind::Blast,
            GrenadeEffect::Flash(_) => GrenadeKind::Flash,
            GrenadeEffect::Smoke(_) => GrenadeKind::Smoke,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GrenadeKind {
    Blast,
    Flash,
    Smoke,
}

/// Weapon part: grenades thrown one at a time (spec 2). Primary pulls the
/// pin; releasing it throws `throw_delay` later; the weapon is drawn again
/// after the throw, or goes when the last one is thrown.
#[derive(Component, Clone, Debug)]
pub struct Throwable {
    /// Grenades held, and the most that can be carried.
    pub count: u32,
    pub max: u32,
    /// Seconds: the pin-pull animation (no throw before... it may be cut
    /// short by a release), release to the projectile, the throw
    /// animation, the redraw.
    pub pin_time: f32,
    pub throw_delay: f32,
    pub throw_time: f32,
    pub redraw_time: f32,
    /// Fuse, seconds from the projectile's start.
    pub fuse: f32,
    pub throw: ThrowRule,
    pub flight: Flight,
    pub effect: GrenadeEffect,
    /// (seconds after the pin pull, entry): the view model's sounds.
    pub pin_sounds: Vec<(f32, String)>,
    pub bounce_sound: Option<String>,
    /// The pin is out, waiting for the release.
    pub pin: bool,
    /// When the released grenade leaves the hand.
    pub throw_at: Option<f64>,
    /// When the throw animation is over (redraw, or the weapon goes).
    pub redraw_at: Option<f64>,
}

impl Throwable {
    /// In the hand with the pin out, or released and not yet thrown.
    pub fn primed(&self) -> bool {
        self.pin || self.throw_at.is_some()
    }
}

/// A grenade in flight (or lying, waiting to go off).
#[derive(Component, Clone, Debug)]
pub struct Projectile {
    pub velocity: Vec3,
    pub thrower: Option<Entity>,
    /// The weapon ID it came from (kill notices, drawing).
    pub weapon: &'static str,
    pub flight: Flight,
    pub fuse: f32,
    pub effect: GrenadeEffect,
    pub bounce_sound: Option<String>,
    /// Ticks simulated so far (the spawn tick is 0).
    pub ticks: u32,
    /// At rest on a floor.
    pub resting: bool,
    /// Radians/s about its own axes.
    pub spin: Vec3,
    /// The breakable it broke into last (a second contact bounces).
    pub breakable: Option<Entity>,
    in_water: bool,
}

/// A smoke cloud: where, when it started, and its rule. Carries a
/// `core::SightBlocker` that grows with it.
#[derive(Component, Clone, Debug)]
pub struct SmokeCloud {
    pub centre: Vec3,
    pub started: f64,
    pub smoke: Smoke,
    /// The spent grenade lying in it (removed with the cloud).
    pub grenade: Option<Entity>,
}

impl SmokeCloud {
    /// Radius at `t` seconds (spec 7.2: `E(t)`).
    pub fn radius(&self, t: f32) -> f32 {
        smoke_radius(self.smoke.max_radius, self.smoke.expand_time, t)
    }

    /// 1 while holding, falling to 0 by the end (spec 7.2).
    pub fn fade(&self, t: f32) -> f32 {
        smoke_fade(self.smoke.fade_start, self.smoke.fade_end, t)
    }

    /// The cloud's overall alpha: fade × growth.
    pub fn alpha(&self, t: f32) -> f32 {
        self.fade(t) * self.radius(t) / self.smoke.max_radius.max(1e-6)
    }
}

/// `E(t) = max · sin(π/2 · min(t / expand, 1))`.
pub fn smoke_radius(max: f32, expand: f32, t: f32) -> f32 {
    max * (std::f32::consts::FRAC_PI_2 * (t / expand.max(1e-6)).clamp(0.0, 1.0)).sin()
}

/// Cosine fade between `start` and `end`.
pub fn smoke_fade(start: f32, end: f32, t: f32) -> f32 {
    if t < start {
        1.0
    } else if t >= end {
        0.0
    } else {
        0.5 + 0.5 * (std::f32::consts::PI * (t - start) / (end - start)).cos()
    }
}

/// The grey a camera inside a cloud sees (spec 7.4): what one cloud of
/// radius `e` and alpha `alpha` adds at distance `d` from its centre.
pub fn smoke_fog(d: f32, e: f32, alpha: f32) -> f32 {
    if e <= 0.0 {
        0.0
    } else if d < 0.3 * e {
        alpha
    } else if d < e {
        (1.0 - (d - 0.3 * e) / (0.7 * e)) * alpha
    } else {
        0.0
    }
}

/// The grey overlay's alpha at `camera` from every cloud (clamped to 1).
pub fn smoke_fog_at<'a>(camera: Vec3, now: f64, clouds: impl Iterator<Item = &'a SmokeCloud>) -> f32 {
    clouds
        .map(|c| {
            let t = (now - c.started) as f32;
            smoke_fog(camera.distance(c.centre), c.radius(t), c.alpha(t))
        })
        .sum::<f32>()
        .clamp(0.0, 1.0)
}

/// A grenade went off: games draw it.
#[derive(Message, Clone, Debug)]
pub struct Detonated {
    pub kind: GrenadeKind,
    pub weapon: &'static str,
    pub thrower: Option<Entity>,
    /// The blast point.
    pub at: Vec3,
    /// What the ground probe hit under it: point, normal, body.
    pub ground: Option<(Vec3, Vec3, Entity)>,
    /// The projectile (still alive for smoke: it lies in the cloud).
    pub projectile: Entity,
}

// ---------------------------------------------------------------------------
// Throwing

/// Start a projectile from `weapon`'s rules.
fn launch(
    commands: &mut Commands,
    rng: &mut ParticleRng,
    weapon: &Weapon,
    t: &Throwable,
    thrower: Option<Entity>,
    start: Vec3,
    velocity: Vec3,
) -> Entity {
    let spin = Vec3::new(
        rng.float(-t.flight.spin_pitch, t.flight.spin_pitch),
        0.0,
        t.flight.spin_roll,
    );
    commands
        .spawn((
            Name::new(format!("Grenade {}", weapon.id)),
            Transform::from_translation(start),
            Projectile {
                velocity,
                thrower,
                weapon: weapon.id,
                flight: t.flight,
                fuse: t.fuse,
                effect: t.effect.clone(),
                bounce_sound: t.bounce_sound.clone(),
                ticks: 0,
                resting: false,
                spin,
                breakable: None,
                in_water: false,
            },
            ShownItem(weapon.id.to_string()),
        ))
        .id()
}

/// Start a projectile of `weapon`'s kind (a `Throwable`) at `start` with
/// `velocity`, thrown by its owner, without using up a grenade (tests,
/// tools).
pub fn spawn_projectile(world: &mut World, weapon: Entity, start: Vec3, velocity: Vec3) -> Option<Entity> {
    let w = world.get::<Weapon>(weapon)?.clone();
    let t = world.get::<Throwable>(weapon)?.clone();
    let mut rng = ParticleRng::new(weapon.to_bits());
    let mut queue = bevy::ecs::world::CommandQueue::default();
    let e = {
        let mut commands = Commands::new(&mut queue, world);
        launch(&mut commands, &mut rng, &w, &t, w.owner, start, velocity)
    };
    queue.apply(world);
    Some(e)
}

#[allow(clippy::type_complexity)]
fn throw_frame(
    mut owners: Query<(
        Entity,
        &Intent,
        &mut Inventory,
        &Transform,
        &MovementState,
        Option<&Velocity>,
        Option<&Health>,
    )>,
    mut weapons: Query<(Entity, &Weapon, &mut WeaponState, &mut Throwable)>,
    mut events: MessageWriter<WeaponEvent>,
    mut rng: ResMut<GrenadeRng>,
    (mut radio, radio_off): (MessageWriter<Radio>, Res<GrenadeRadio>),
    mut commands: Commands,
    time: Res<Time>,
) {
    let now = time.elapsed_secs_f64();
    let due = now + TIME_SLACK;
    for (entity, weapon, mut st, mut t) in &mut weapons {
        let Some(owner) = weapon.owner else { continue };
        let Ok((_, intent, mut inv, transform, state, vel, health)) = owners.get_mut(owner) else {
            continue;
        };
        let active = inv.active == Some(entity);
        let alive = health.is_none_or(|h| h.current > 0.0);
        if !active && t.pin {
            // Switched away with the pin out: the throw is cancelled and
            // the grenade kept (spec 2, Q3).
            t.pin = false;
            st.pending.clear();
        }
        // 2. Released: throw (a release is checked first, so a tap throws
        // at once).
        if active && alive && t.pin && !intent.fire {
            t.pin = false;
            t.throw_at = Some(now + t.throw_delay as f64);
            t.count = t.count.saturating_sub(1);
            t.redraw_at = Some(now + t.throw_time as f64);
            st.next_primary = now + t.throw_time as f64;
            st.next_secondary = st.next_primary;
            // The throw animation cuts the pin's.
            st.pending.clear();
            events.write(WeaponEvent {
                owner,
                weapon: entity,
                kind: WeaponEventKind::Thrown,
            });
            // The thrower's team hears "Fire in the hole!" (spec 2).
            if radio_off.0 == 0 {
                radio.write(Radio {
                    sender: owner,
                    command: "fireinhole".into(),
                });
            }
        }
        // 3. The grenade leaves the hand (strictly after its time).
        if let Some(at) = t.throw_at
            && now > at + 1e-9
        {
            t.throw_at = None;
            let eye = transform.translation + state.eye_offset;
            let carried = vel.map_or(Vec3::ZERO, |v| v.0);
            let (start, velocity) = t.throw.throw(eye, intent.yaw, intent.pitch, carried);
            launch(&mut commands, &mut rng.0, weapon, &t, Some(owner), start, velocity);
        }
        // 4. The throw animation is over: draw the next one, or the weapon
        // goes with its last grenade.
        if let Some(at) = t.redraw_at
            && t.throw_at.is_none()
            && (due >= at || !active)
        {
            t.redraw_at = None;
            if t.count == 0 {
                commands.queue(move |w: &mut World| exhaust(w, owner, entity));
                continue;
            }
            if active {
                let ready = now + t.redraw_time as f64;
                inv.next_attack = ready;
                st.next_primary = ready;
                st.next_secondary = ready;
                events.write(WeaponEvent {
                    owner,
                    weapon: entity,
                    kind: WeaponEventKind::Deployed,
                });
            }
        }
        // 1. Pressed: pull the pin.
        if active
            && alive
            && intent.fire
            && !t.primed()
            && t.redraw_at.is_none()
            && t.count > 0
            && due >= inv.next_attack
            && due >= st.next_primary
        {
            t.pin = true;
            st.next_primary = now + t.pin_time as f64;
            st.pending = t.pin_sounds.iter().map(|(s, e)| (now + *s as f64, e.clone())).collect();
            events.write(WeaponEvent {
                owner,
                weapon: entity,
                kind: WeaponEventKind::PinPulled,
            });
        }
    }
}

/// The last grenade is gone: the weapon leaves the inventory, and its
/// owner draws the best of the rest (the lowest slot).
fn exhaust(world: &mut World, owner: Entity, weapon: Entity) {
    let next = world.get::<Inventory>(owner).and_then(|inv| {
        inv.weapons
            .iter()
            .copied()
            .filter(|w| *w != weapon)
            .min_by_key(|e| world.get::<Weapon>(*e).map_or(u8::MAX, |w| w.slot))
    });
    if let Some(mut inv) = world.get_mut::<Inventory>(owner) {
        inv.weapons.retain(|w| *w != weapon);
        if inv.last == Some(weapon) {
            inv.last = None;
        }
        if inv.active == Some(weapon) {
            inv.active = None;
            if inv.wanted.is_none_or(|w| w == weapon) {
                inv.wanted = next;
            }
        }
    }
    if let Ok(e) = world.get_entity_mut(weapon) {
        e.despawn();
    }
}

/// Dying with the pin out drops the grenade live where the eye was, with
/// only the body's velocity (spec 9, hypothesis Q17); one released but not
/// yet thrown still goes (its throw is pending).
fn drop_primed(
    mut died: MessageReader<Died>,
    owners: Query<(&Inventory, &Transform, &MovementState, &Intent, Option<&Velocity>)>,
    mut weapons: Query<(&Weapon, &mut Throwable)>,
    mut rng: ResMut<GrenadeRng>,
    mut commands: Commands,
) {
    for d in died.read() {
        let Ok((inv, transform, state, intent, vel)) = owners.get(d.entity) else {
            continue;
        };
        let Some(active) = inv.active else { continue };
        let Ok((weapon, mut t)) = weapons.get_mut(active) else {
            continue;
        };
        if !t.pin {
            continue;
        }
        t.pin = false;
        t.count = t.count.saturating_sub(1);
        let eye = transform.translation + state.eye_offset;
        let start = eye + intent.look_rotation() * Vec3::NEG_Z * t.throw.forward;
        let velocity = vel.map_or(Vec3::ZERO, |v| v.0);
        launch(&mut commands, &mut rng.0, weapon, &t, Some(d.entity), start, velocity);
        if t.count == 0 {
            let owner = d.entity;
            commands.queue(move |w: &mut World| exhaust(w, owner, active));
        }
    }
}

// ---------------------------------------------------------------------------
// Flight

#[derive(bevy::ecs::query::QueryData)]
struct Body {
    entity: Entity,
    transform: &'static Transform,
    aabb: Option<&'static ColliderAabb>,
    health: Option<&'static Health>,
    damageable: Option<&'static Damageable>,
    intent: Option<&'static Intent>,
    state: Option<&'static MovementState>,
}

/// The world access grenades need.
#[derive(bevy::ecs::system::SystemParam)]
struct GrenadeWorld<'w, 's> {
    spatial: SpatialQuery<'w, 's>,
    bodies: Query<'w, 's, Body, Without<Projectile>>,
    colliders: Query<'w, 's, &'static ColliderOf>,
    rigid: Query<'w, 's, (&'static RigidBody, Option<&'static MapPropCollider>)>,
    dynamic: Query<
        'w,
        's,
        (
            Entity,
            &'static RigidBody,
            &'static Position,
            Option<&'static ComputedMass>,
            Forces,
        ),
    >,
    armor: Query<'w, 's, &'static mut Armor>,
    blinded: Query<'w, 's, &'static Blinded>,
    water: Option<Res<'w, MapWater>>,
    damage: MessageWriter<'w, Damage>,
    play: MessageWriter<'w, PlaySound>,
    decals: MessageWriter<'w, PlaceDecal>,
    detonated: MessageWriter<'w, Detonated>,
}

impl GrenadeWorld<'_, '_> {
    fn body(&self, collider: Entity) -> Entity {
        self.colliders.get(collider).map_or(collider, |c| c.body)
    }

    fn is_character(&self, e: Entity) -> bool {
        self.bodies.get(e).is_ok_and(|b| b.intent.is_some())
    }

    /// Characters that are dead (their bodies are ragdolls or hidden).
    fn dead(&self) -> Vec<Entity> {
        self.bodies
            .iter()
            .filter(|b| b.intent.is_some() && b.health.is_some_and(|h| h.current <= 0.0))
            .map(|b| b.entity)
            .collect()
    }

    /// A world-only line trace (static bodies that aren't props: the map's
    /// brushes and surfaces), also stopping at `target`: the fraction and
    /// the body hit.
    fn world_trace(&self, from: Vec3, to: Vec3, target: Option<Entity>) -> Option<(f32, Vec3, Entity)> {
        let d = to - from;
        let len = d.length();
        let dir = Dir3::new(d).ok()?;
        let filter = SpatialQueryFilter::default().with_mask(SOLID_LAYERS);
        self.spatial
            .cast_ray_predicate(from, dir, len, true, &filter, &|e| {
                let body = self.body(e);
                Some(body) == target
                    || self
                        .rigid
                        .get(e)
                        .or_else(|_| self.rigid.get(body))
                        .is_ok_and(|(rb, prop)| rb.is_static() && prop.is_none())
            })
            .map(|h| (h.distance / len, h.normal, self.body(h.entity)))
    }

    fn in_water(&self, p: Vec3) -> bool {
        self.water.as_ref().is_some_and(|w| {
            w.0.iter()
                .any(|v| v.brush.planes.iter().all(|(n, d)| n.dot(p) - d <= 0.0))
        })
    }
}

#[allow(clippy::too_many_arguments)]
fn fly(
    mut projectiles: Query<(Entity, &mut Projectile, &mut Transform)>,
    mut world: GrenadeWorld,
    mut rng: ResMut<GrenadeRng>,
    mut commands: Commands,
    time: Res<Time>,
) {
    let dt = time.delta_secs();
    if dt <= 0.0 {
        return;
    }
    let now = time.elapsed_secs_f64();
    let dead = world.dead();
    for (entity, mut p, mut transform) in &mut projectiles {
        let f = p.flight;
        // 1. The periodic check: fuse, water.
        let check = ((f.check_interval / dt).round() as u32).max(1);
        if p.ticks.is_multiple_of(check) {
            let elapsed = p.ticks as f32 * dt;
            let pop = match &p.effect {
                GrenadeEffect::Smoke(s) => p.velocity.length() < s.pop_speed,
                _ => true,
            };
            if elapsed > p.fuse + 1e-4 && pop {
                detonate(
                    entity,
                    &p,
                    transform.translation,
                    &mut world,
                    &mut rng.0,
                    &mut commands,
                    now,
                );
                continue;
            }
            if world.in_water(transform.translation) {
                p.velocity *= f.water_slowdown;
            }
        }
        p.ticks += 1;
        // 2. At rest on the ground: nothing moves.
        if p.resting && p.velocity == Vec3::ZERO {
            continue;
        }
        p.resting = false;
        // 3-5. Clamp, gravity, spin.
        p.velocity = p
            .velocity
            .clamp(Vec3::splat(-f.max_velocity), Vec3::splat(f.max_velocity));
        let (mut step, v) = gravity_step(p.velocity, f.gravity, dt);
        p.velocity = v;
        transform.rotation *= Quat::from_scaled_axis(p.spin * dt);
        // 6-7. Sweep and contacts (a floor contact keeps the rest of the
        // move; walls lose it).
        let shape = Collider::cuboid(f.half * 2.0, f.half * 2.0, f.half * 2.0);
        let excluded: Vec<Entity> = p.thrower.into_iter().chain(dead.iter().copied()).collect();
        let filter = SpatialQueryFilter::from_excluded_entities(excluded).with_mask(SOLID_LAYERS);
        for _ in 0..2 {
            let len = step.length();
            let Ok(dir) = Dir3::new(step) else { break };
            let config = ShapeCastConfig {
                max_distance: len,
                ignore_origin_penetration: true,
                ..ShapeCastConfig::DEFAULT
            };
            let Some(hit) =
                world
                    .spatial
                    .cast_shape(&shape, transform.translation, Quat::IDENTITY, dir, &config, &filter)
            else {
                transform.translation += step;
                break;
            };
            let fraction = hit.distance / len;
            transform.translation += *dir * (hit.distance - SKIN).max(0.0);
            let body = world.body(hit.entity);
            let n = hit.normal1.normalize_or_zero();
            if let Some(s) = &p.bounce_sound {
                world.play.write(PlaySound {
                    entry: s.clone(),
                    at: Some(transform.translation),
                    volume: None,
                    source: Some(entity),
                    channel: Some(CHAN_VOICE),
                });
            }
            // Glass and other breakables: damage it and fly on slower
            // (when it breaks this tick; a second contact with the same
            // one bounces).
            let breakable = world.bodies.get(body).is_ok_and(|b| b.damageable.is_some());
            if breakable && p.breakable != Some(body) {
                p.breakable = Some(body);
                world.damage.write(Damage {
                    force: bevy::math::Vec3::ZERO,
                    target: body,
                    attacker: p.thrower,
                    amount: f.breakable_damage,
                    point: hit.point1,
                    dir: *dir,
                    hitgroup: Hitgroup::Generic,
                    kind: DamageKind::Melee,
                    weapon: Some(p.weapon),
                });
                p.velocity *= f.breakable_slowdown;
                break;
            }
            let player = world.is_character(body);
            let (v, rest) = bounce(p.velocity, n, player, &f, 0.1 * 0.0254);
            p.velocity = v;
            if rest {
                p.resting = true;
                p.spin = Vec3::ZERO;
                let yaw = rng.0.float(0.0, std::f32::consts::TAU);
                transform.rotation = Quat::from_rotation_arc(Vec3::Y, n) * Quat::from_rotation_y(yaw);
                break;
            }
            if n.y <= f.floor_normal || v == Vec3::ZERO {
                break;
            }
            step = v * (1.0 - fraction) * dt;
        }
        // 8. Into water.
        let wet = world.in_water(transform.translation);
        if wet && !p.in_water {
            p.velocity.y *= f.water_entry;
        }
        p.in_water = wet;
    }
}

/// The grenade goes off.
fn detonate(
    entity: Entity,
    p: &Projectile,
    at: Vec3,
    world: &mut GrenadeWorld,
    rng: &mut ParticleRng,
    commands: &mut Commands,
    now: f64,
) {
    let mut message = Detonated {
        kind: p.effect.kind(),
        weapon: p.weapon,
        thrower: p.thrower,
        at,
        ground: None,
        projectile: entity,
    };
    match &p.effect {
        GrenadeEffect::Blast(b) => {
            let (origin, probe) = explode_at(at, b, p.thrower, p.weapon, Some(entity), world, rng);
            message.at = origin;
            message.ground = probe;
            if let Some(s) = &b.sound {
                world.play.write(PlaySound {
                    entry: s.clone(),
                    at: Some(origin),
                    volume: None,
                    source: Some(entity),
                    channel: Some(CHAN_WEAPON),
                });
            }
            commands.entity(entity).despawn();
        }
        GrenadeEffect::Flash(fl) => {
            flash(at, fl, world, commands, now);
            if let Some(s) = &fl.sound {
                world.play.write(PlaySound {
                    entry: s.clone(),
                    at: Some(at),
                    volume: None,
                    source: None,
                    channel: None,
                });
            }
            commands.entity(entity).despawn();
        }
        GrenadeEffect::Smoke(s) => {
            if let Some(sound) = &s.sound {
                world.play.write(PlaySound::at(sound.clone(), at));
            }
            commands.spawn((
                Name::new("Smoke cloud"),
                SmokeCloud {
                    centre: at,
                    started: now,
                    smoke: s.clone(),
                    grenade: Some(entity),
                },
                SightBlocker {
                    centre: at,
                    radius: 0.0,
                },
            ));
            // The spent grenade stays where it lies.
            commands.entity(entity).remove::<Projectile>();
        }
    }
    world.detonated.write(message);
}

/// A blast at `at`: the ground probe under it (spec 5.2), radius damage
/// and pushes from just above it, the scorch mark. Returns the blast
/// point and what the probe hit.
fn explode_at(
    at: Vec3,
    b: &Blast,
    thrower: Option<Entity>,
    weapon: &'static str,
    inflictor: Option<Entity>,
    world: &mut GrenadeWorld,
    rng: &mut ParticleRng,
) -> (Vec3, Option<(Vec3, Vec3, Entity)>) {
    let (from, to) = (at + Vec3::Y * b.probe_up, at - Vec3::Y * b.probe_down);
    let filter = SpatialQueryFilter::default().with_mask(SOLID_LAYERS);
    let probe = Dir3::new(to - from).ok().and_then(|dir| {
        world
            .spatial
            .cast_ray_predicate(from, dir, from.distance(to), true, &filter, &|e| {
                let body = world.body(e);
                !world.is_character(body) && Some(body) != inflictor
            })
            .map(|h| (from + *dir * h.distance, h.normal, world.body(h.entity)))
    });
    let origin = probe.map_or(at, |(point, n, _)| point + n * b.pull_out);
    blast(origin, b, thrower, weapon, inflictor, world, rng);
    if let (Some(group), Some((point, n, body))) = (&b.scorch, probe) {
        world.decals.write(PlaceDecal {
            target: Some(body),
            group: DecalGroup::Named(group.clone()),
            point,
            normal: n,
            dir: -n,
            spin: true,
        });
    }
    (origin, probe)
}

/// The blast rules explosions from map logic (`core::Explosion`) follow:
/// a game's grenade blast, with the explosion's damage and radius (the
/// push scaled with the damage). Without one, explosions do nothing.
#[derive(Resource, Clone, Debug)]
pub struct ExplosionRule(pub Blast);

/// Apply `core::Explosion`s: a blast like a grenade's, then `Detonated`
/// so games draw it.
fn explosions(
    mut events: MessageReader<Explosion>,
    rule: Option<Res<ExplosionRule>>,
    mut world: GrenadeWorld,
    mut rng: ResMut<GrenadeRng>,
) {
    let Some(rule) = rule else {
        events.clear();
        return;
    };
    for e in events.read() {
        let mut b = rule.0.clone();
        let scale = if b.damage > 0.0 { e.damage / b.damage } else { 1.0 };
        b.force *= scale;
        b.damage = e.damage;
        b.radius = e.radius;
        let (origin, probe) = explode_at(
            e.origin,
            &b,
            e.attacker,
            e.weapon.unwrap_or(EXPLOSION_WEAPON),
            e.inflictor,
            &mut world,
            &mut rng.0,
        );
        if let Some(s) = &e.sound {
            world.play.write(PlaySound {
                entry: s.clone(),
                at: Some(origin),
                volume: None,
                source: e.inflictor,
                channel: None,
            });
        }
        world.detonated.write(Detonated {
            kind: GrenadeKind::Blast,
            weapon: e.weapon.unwrap_or(EXPLOSION_WEAPON),
            thrower: e.attacker,
            at: origin,
            ground: probe,
            projectile: Entity::PLACEHOLDER,
        });
    }
}

/// The weapon name explosions from map logic count as (kill notices).
pub const EXPLOSION_WEAPON: &str = "env_explosion";

/// Radius damage from `origin` (spec 5.3) and the push on loose bodies;
/// `skip` (what exploded) takes none.
fn blast(
    origin: Vec3,
    b: &Blast,
    thrower: Option<Entity>,
    weapon: &'static str,
    skip: Option<Entity>,
    world: &mut GrenadeWorld,
    rng: &mut ParticleRng,
) {
    let src = origin + Vec3::Y * b.src_lift;
    let mut hits: Vec<(Entity, Vec3, f32)> = Vec::new();
    for t in world.bodies.iter() {
        if Some(t.entity) == skip {
            continue;
        }
        let alive = t.health.is_some_and(|h| h.current > 0.0);
        if !alive && t.damageable.is_none() {
            continue;
        }
        // Bounds touching the sphere.
        let (lo, hi) = t
            .aabb
            .map_or((t.transform.translation, t.transform.translation), |a| (a.min, a.max));
        if src.clamp(lo, hi).distance(src) > b.radius {
            continue;
        }
        let target = match (t.intent, t.state) {
            (Some(_), Some(state)) => {
                let eye = t.transform.translation + state.eye_offset;
                let feet = if state.hull_min != Vec3::ZERO {
                    t.transform.translation.y + state.hull_min.y
                } else {
                    lo.y
                };
                let h = rng.float(b.body_target.0, b.body_target.1);
                eye.with_y(feet + (eye.y - feet) * h)
            }
            _ => (lo + hi) / 2.0,
        };
        hits.push((t.entity, target, 0.0));
    }
    for (target, point, amount) in &mut hits {
        // Characters aren't in the trace (it sees world brushes only);
        // breakable brushes are, and a hit on the target itself counts.
        let own = (!world.is_character(*target)).then_some(*target);
        let end = match world.world_trace(src, *point, own) {
            None => *point,
            Some((fraction, _, body)) if body == *target => src.lerp(*point, fraction),
            Some(_) => continue,
        };
        *point = end;
        *amount = blast_damage(b.damage, b.radius, end.distance(src));
    }
    for (target, end, raw) in hits {
        if raw <= 0.0 {
            continue;
        }
        let mut amount = super::quantize(raw, b.quantum);
        if let Some(ratio) = b.armor_ratio
            && let Ok(mut armor) = world.armor.get_mut(target)
            && armor.covers(Hitgroup::Generic)
        {
            let (to_health, to_armor) = armor_split(raw, ratio, b.quantum);
            amount = to_health;
            armor.amount = (armor.amount - to_armor).max(0.0);
        }
        let dir = (end - src).normalize_or_zero();
        world.damage.write(Damage {
            force: dir * b.force,
            target,
            attacker: thrower,
            amount,
            point: end,
            dir,
            hitgroup: Hitgroup::Generic,
            kind: DamageKind::Blast,
            weapon: Some(weapon),
        });
    }
    // Loose bodies in reach and in the open get the push (physics_props.md
    // 5.3: the undecayed force), capped to a sane speed.
    let pushes: Vec<(Entity, Vec3, f32)> = world
        .dynamic
        .iter()
        .filter(|(e, rb, ..)| rb.is_dynamic() && Some(*e) != skip)
        .filter(|(_, _, pos, ..)| pos.0.distance(src) < b.radius)
        .map(|(e, _, pos, mass, _)| (e, pos.0, mass.map_or(1.0, |m| m.value())))
        .collect();
    for (e, centre, mass) in pushes {
        if world
            .world_trace(src, centre, Some(e))
            .is_some_and(|(_, _, body)| body != e)
        {
            continue;
        }
        let j = (b.force * rng.float(b.force_jitter.0, b.force_jitter.1)).min(mass * b.max_push_speed);
        let dir = (centre - src).normalize_or(Vec3::Y);
        if let Ok((.., mut forces)) = world.dynamic.get_mut(e) {
            forces.apply_linear_impulse(dir * j);
        }
    }
}

/// Blind everyone who sees the flash (spec 6.2): teammates and the
/// thrower too.
fn flash(at: Vec3, fl: &Flash, world: &mut GrenadeWorld, commands: &mut Commands, now: f64) {
    let mut blinded = Vec::new();
    for b in world.bodies.iter() {
        let (Some(intent), Some(state)) = (b.intent, b.state) else {
            continue;
        };
        if b.health.is_none_or(|h| h.current <= 0.0) {
            continue;
        }
        let eye = b.transform.translation + state.eye_offset;
        if world.world_trace(at, eye, None).is_some() {
            continue;
        }
        let to = at - eye;
        let facing = (intent.look_rotation() * Vec3::NEG_Z).dot(to.normalize_or_zero());
        if let Some((alpha, hold, fade)) = (fl.model)(to.length(), facing) {
            blinded.push((b.entity, alpha, hold, fade));
        }
    }
    for (e, alpha, hold, fade) in blinded {
        let new = Blinded {
            alpha,
            fade_start: now + hold as f64,
            end: now + (hold + fade) as f64,
        };
        let next = match world.blinded.get(e) {
            Ok(old) => old.raised(new, now),
            Err(_) => new,
        };
        commands.entity(e).insert(next);
    }
}

/// Clouds grow (their sight blocker with them) and go when faded, with
/// the grenade lying in them.
fn smoke_clouds(mut clouds: Query<(Entity, &SmokeCloud, &mut SightBlocker)>, mut commands: Commands, time: Res<Time>) {
    let now = time.elapsed_secs_f64();
    for (e, cloud, mut blocker) in &mut clouds {
        let t = (now - cloud.started) as f32;
        if t >= cloud.smoke.fade_end {
            commands.entity(e).despawn();
            if let Some(g) = cloud.grenade {
                commands.entity(g).try_despawn();
            }
            continue;
        }
        // A thinning cloud stops hiding things halfway through its fade.
        let radius = if cloud.fade(t) > 0.5 {
            cloud.radius(t) * cloud.smoke.sight_radius
        } else {
            0.0
        };
        if blocker.radius != radius {
            blocker.radius = radius;
        }
    }
}

/// A new round clears grenades in flight, spent ones, clouds and
/// blindness.
#[allow(clippy::type_complexity)]
fn clear_on_restart(
    restarts: Option<Res<RoundRestarts>>,
    mut seen: Local<u32>,
    gone: Query<Entity, Or<(With<Projectile>, With<SmokeCloud>)>>,
    clouds: Query<&SmokeCloud>,
    blinded: Query<Entity, With<Blinded>>,
    mut commands: Commands,
) {
    let Some(restarts) = restarts else { return };
    if restarts.0 == *seen {
        return;
    }
    *seen = restarts.0;
    for c in &clouds {
        if let Some(g) = c.grenade {
            commands.entity(g).try_despawn();
        }
    }
    for e in &gone {
        commands.entity(e).try_despawn();
    }
    for e in &blinded {
        commands.entity(e).remove::<Blinded>();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const U: f32 = 0.0254;

    fn css_throw() -> ThrowRule {
        ThrowRule {
            pitch_lift: -10.0,
            pitch_scale: 100.0 / 90.0,
            speed_per_deg: 6.0 * U,
            speed_max: 750.0 * U,
            forward: 16.0 * U,
        }
    }

    fn css_flight() -> Flight {
        Flight {
            half: 2.0 * U,
            gravity: 320.0 * U,
            elasticity: 0.45,
            player_elasticity: 0.3,
            elasticity_cap: 0.9,
            stop_speed: 30.0 * U,
            floor_normal: 0.7,
            breakable_damage: 0.1,
            breakable_slowdown: 0.4,
            water_slowdown: 0.5,
            water_entry: 0.5,
            max_velocity: 3500.0 * U,
            check_interval: 0.2,
            spin_roll: 600f32.to_radians(),
            spin_pitch: 1200f32.to_radians(),
        }
    }

    /// Source (x forward, y left, z up) inches from ours at yaw 0.
    fn src(v: Vec3) -> Vec3 {
        Vec3::new(-v.z, -v.x, v.y) / U
    }

    fn close(a: Vec3, b: Vec3, eps: f32) -> bool {
        (a - b).length() < eps
    }

    #[test]
    fn throws_g1_to_g7() {
        let r = css_throw();
        let eye = Vec3::new(0.0, 64.0 * U, 0.0);
        // (view pitch, Source degrees positive down; spawn offset; velocity)
        let cases = [
            (0.0, Some(Vec3::new(15.757, 0.0, 2.778)), Vec3::new(590.88, 0.0, 104.19)),
            (-22.5, None, Vec3::new(614.36, 0.0, 430.18)),
            (-45.0, None, Vec3::new(375.0, 0.0, 649.52)),
            (
                -89.0,
                Some(Vec3::new(-5.18, 0.0, 15.138)),
                Vec3::new(-242.8, 0.0, 709.61),
            ),
            (45.0, None, Vec3::new(229.81, 0.0, -192.84)),
            (89.0, Some(Vec3::new(0.31, 0.0, -15.997)), Vec3::new(0.13, 0.0, -6.67)),
        ];
        for (p, offset, vel) in cases {
            let (start, v) = r.throw(eye, 0.0, (-p as f32).to_radians(), Vec3::ZERO);
            assert!(close(src(v), vel, 0.1), "pitch {p}: {}", src(v));
            if let Some(o) = offset {
                assert!(close(src(start - eye), o, 0.01), "pitch {p}: {}", src(start - eye));
            }
        }
        assert!((r.pitch(-22.5) + 35.0).abs() < 1e-4 && (r.speed(-35.0) / U - 750.0).abs() < 1e-3);
        // G7: running forward at 250 adds it all.
        let run = Vec3::new(0.0, 0.0, -250.0 * U);
        let (_, v) = r.throw(eye, 0.0, 0.0, run);
        assert!(close(src(v), Vec3::new(840.88, 0.0, 104.19), 0.1));
    }

    #[test]
    fn flight_g8_g9() {
        // G9: 20 ticks from rest with (600, 0, 300).
        let mut v = Vec3::new(0.0, 300.0, -600.0) * U;
        let mut p = Vec3::ZERO;
        for _ in 0..20 {
            let (step, nv) = gravity_step(v, 320.0 * U, 0.015);
            p += step;
            v = nv;
        }
        assert!(close(p / U, Vec3::new(0.0, 75.6, -180.0), 1e-2), "{}", p / U);
        assert!((v.y / U - 204.0).abs() < 1e-3);
        // G8: apex height 104.19² / 640.
        assert!((104.19f32.powi(2) / (2.0 * 320.0) - 16.96).abs() < 0.01);
    }

    #[test]
    fn bounces_g10_to_g14() {
        let f = css_flight();
        let snap = 0.1 * U;
        let up = Vec3::Y;
        // G10: floor, keeps moving.
        let (v, rest) = bounce(Vec3::new(590.88, -200.0, 0.0) * U, up, false, &f, snap);
        assert!(!rest && close(v / U, Vec3::new(265.896, 90.0, 0.0), 1e-2), "{}", v / U);
        // G11 wall, G12 player.
        let wall = Vec3::NEG_X;
        let (v, _) = bounce(Vec3::X * 500.0 * U, wall, false, &f, snap);
        assert!(close(v / U, Vec3::X * -225.0, 1e-3));
        let (v, _) = bounce(Vec3::X * 500.0 * U, wall, true, &f, snap);
        assert!(close(v / U, Vec3::X * -67.5, 1e-3));
        // G13: slow on a floor: rests.
        let (v, rest) = bounce(Vec3::new(40.0, -50.0, 0.0) * U, up, false, &f, snap);
        assert!(rest && v == Vec3::ZERO);
        // G14: slow on a wall: stops, no rest (falls next tick).
        let (v, rest) = bounce(Vec3::X * -50.0 * U, Vec3::X, false, &f, snap);
        assert!(!rest && v == Vec3::ZERO);
    }

    #[test]
    fn blast_falls_off_linearly() {
        assert_eq!(blast_damage(100.0, 350.0, 0.0), 100.0);
        assert!((blast_damage(100.0, 350.0, 175.0) - 50.0).abs() < 1e-4);
        assert_eq!(blast_damage(100.0, 350.0, 350.0), 0.0);
        assert_eq!(blast_damage(100.0, 350.0, 400.0), 0.0);
        // G18's range: d in [204.61, 209.51].
        assert!((blast_damage(100.0, 350.0, 204.61) - 41.54).abs() < 0.01);
        assert!((blast_damage(100.0, 350.0, 209.51) - 40.14).abs() < 0.01);
    }

    #[test]
    fn smoke_g25_to_g29() {
        for (t, e) in [(0.0, 0.0), (0.25, 91.84), (0.5, 169.71), (1.0, 240.0), (5.0, 240.0)] {
            assert!((smoke_radius(240.0, 1.0, t) - e).abs() < 0.01, "{t}");
        }
        for (t, f) in [(17.0, 1.0), (18.0, 0.9045), (19.5, 0.5), (21.0, 0.0955), (22.0, 0.0)] {
            assert!((smoke_fade(17.0, 22.0, t) - f).abs() < 1e-3, "{t}");
        }
        for (d, a) in [(50.0, 1.0), (120.0, 0.714), (200.0, 0.238), (260.0, 0.0)] {
            assert!((smoke_fog(d, 240.0, 1.0) - a).abs() < 1e-3, "{d}");
        }
        let cloud = |at: Vec3| SmokeCloud {
            centre: at,
            started: 0.0,
            smoke: Smoke {
                max_radius: 240.0,
                expand_time: 1.0,
                fade_start: 17.0,
                fade_end: 22.0,
                pop_speed: 0.1,
                sight_radius: 1.0,
                sound: None,
            },
            grenade: None,
        };
        let both = [cloud(Vec3::ZERO), cloud(Vec3::X)];
        assert_eq!(smoke_fog_at(Vec3::ZERO, 5.0, both.iter()), 1.0);
    }

    #[test]
    fn blindness_holds_fades_and_only_rises() {
        let b = Blinded {
            alpha: 1.0,
            fade_start: 2.0,
            end: 4.0,
        };
        assert_eq!(b.alpha_at(1.0), 1.0);
        assert!((b.alpha_at(3.0) - 0.5).abs() < 1e-6);
        assert_eq!(b.alpha_at(4.0), 0.0);
        let weak = Blinded {
            alpha: 0.3,
            fade_start: 1.5,
            end: 2.5,
        };
        assert_eq!(b.raised(weak, 1.0), b);
        assert_eq!(weak.raised(b, 1.0), b);
    }

    #[test]
    fn sight_blocker_chords() {
        let s = SightBlocker {
            centre: Vec3::ZERO,
            radius: 2.0,
        };
        assert!((s.chord(Vec3::new(-5.0, 0.0, 0.0), Vec3::new(5.0, 0.0, 0.0)) - 4.0).abs() < 1e-5);
        assert!(s.blocks(Vec3::new(-5.0, 0.0, 0.0), Vec3::new(5.0, 0.0, 0.0)));
        // Grazing: a short chord doesn't block.
        assert!(!s.blocks(Vec3::new(-5.0, 1.9, 0.0), Vec3::new(5.0, 1.9, 0.0)));
        // Stopping inside.
        assert!((s.chord(Vec3::new(-5.0, 0.0, 0.0), Vec3::ZERO) - 2.0).abs() < 1e-5);
    }
}
