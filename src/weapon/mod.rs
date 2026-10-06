//! Weapons as composable parts (README, "Weapons and items"): a trigger, a
//! cost, a delivery and an effect. A weapon is an entity carrying those
//! parts and its timers; a character's `Inventory` lists its weapons. Game
//! plugins build weapons from the parts and register them by ID. The frame
//! below runs them the way Source's shared weapon base does
//! (specs/cs_source/weapons.md, sections 3 and 4), which is general enough
//! for other games' weapons: per-game feel goes in extra parts.
//!
//! Units: meters, seconds, damage normalized like `Health`.

mod deliver;

use std::sync::Arc;

use bevy::prelude::*;

pub use deliver::{falloff, hitgroup_at, spread_dir};

use crate::{
    console::{Command, Console, ConsoleAppExt},
    core::{Health, Hitgroup, Intent, LocalPlayer, MaxSpeed, MovementState, SimSet},
    map::PlaySound,
};

/// Source's weapon sound channel: a new shot cuts off the last one's tail.
pub const CHAN_WEAPON: u8 = 1;
/// Reload part sounds and other item noises.
pub const CHAN_ITEM: u8 = 3;

pub struct WeaponPlugin;

/// The weapon frame within `SimSet::Weapons`: games order their weapon
/// feel (inaccuracy, recoil) before or after it.
#[derive(SystemSet, Debug, Clone, PartialEq, Eq, Hash)]
pub struct WeaponFrame;

impl Plugin for WeaponPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<WeaponRegistry>()
            .init_resource::<StartingWeapons>()
            .add_message::<WeaponEvent>()
            .add_message::<PlaySound>()
            .add_systems(
                FixedUpdate,
                (
                    (give_starting_weapons, select_weapons).chain().before(SimSet::Movement),
                    (weapon_frame.in_set(WeaponFrame), timed_sounds)
                        .chain()
                        .in_set(SimSet::Weapons),
                ),
            );
        app.init_resource::<Console>();
        app.world_mut().resource_mut::<Console>().add_command(Command {
            name: "give".into(),
            help: "Give the local player a weapon by ID (e.g. give cs_source:weapon_ak47).".into(),
            run: Arc::new(|w, args| {
                let id = args.first().ok_or("usage: give <weapon id>")?;
                let player = w
                    .query_filtered::<Entity, With<LocalPlayer>>()
                    .iter(w)
                    .next()
                    .ok_or("no local player")?;
                let def = w
                    .resource::<WeaponRegistry>()
                    .find(id)
                    .ok_or_else(|| format!("unknown weapon {id}"))?;
                let id = def.id;
                give(w, player, id);
                Ok(None)
            }),
            complete: Some(Arc::new(|w, _| {
                w.resource::<WeaponRegistry>()
                    .0
                    .iter()
                    .map(|d| d.id.to_string())
                    .collect()
            })),
        });
        for slot in 0..5u8 {
            app.console_command(
                &format!("slot{}", slot + 1),
                &format!("Select the weapon in slot {} (again: the next one there).", slot + 1),
                move |w, _| {
                    select_local(w, |inv, weapons| {
                        let in_slot: Vec<Entity> = inv
                            .weapons
                            .iter()
                            .copied()
                            .filter(|e| weapons.iter().any(|(we, s)| we == e && *s == slot))
                            .collect();
                        match inv.active.and_then(|a| in_slot.iter().position(|w| *w == a)) {
                            Some(i) => Some(in_slot[(i + 1) % in_slot.len()]),
                            None => in_slot.first().copied(),
                        }
                    })
                },
            );
        }
        app.console_command("lastinv", "Switch to the previously held weapon.", |w, _| {
            select_local(w, |inv, _| inv.last)
        })
        .console_command(
            "impulse",
            "impulse 101: every weapon, full clips and reserves.",
            |w, a| {
                if a.first().map(String::as_str) != Some("101") {
                    return Err("only impulse 101 (all weapons and ammo) exists".into());
                }
                let player = local_player(w)?;
                let held: Vec<&'static str> = held_ids(w, player);
                let all: Vec<&'static str> = w.resource::<WeaponRegistry>().0.iter().map(|d| d.id).collect();
                for id in all.into_iter().filter(|id| !held.contains(id)) {
                    give(w, player, id);
                }
                let weapons = w
                    .get::<Inventory>(player)
                    .map(|i| i.weapons.clone())
                    .unwrap_or_default();
                for e in weapons {
                    if let Some(mut m) = w.get_mut::<Magazine>(e) {
                        m.clip = m.size;
                        m.reserve = m.reserve_max;
                    }
                }
                Ok(None)
            },
        )
        .console_command("buy", "buy <weapon>, e.g. buy ak47 (no money yet).", |w, a| {
            let name = a.first().ok_or("buy <weapon>")?.to_lowercase();
            let name = if name.starts_with("weapon_") {
                name
            } else {
                format!("weapon_{name}")
            };
            let player = local_player(w)?;
            let id = w
                .resource::<WeaponRegistry>()
                .find(&name)
                .map(|d| d.id)
                .ok_or_else(|| format!("no weapon {name}"))?;
            give(w, player, id);
            Ok(None)
        });
    }
}

fn local_player(w: &mut World) -> Result<Entity, String> {
    w.query_filtered::<Entity, With<LocalPlayer>>()
        .iter(w)
        .next()
        .ok_or_else(|| "no local player".to_string())
}

/// IDs of the weapons `owner` carries.
fn held_ids(w: &World, owner: Entity) -> Vec<&'static str> {
    let Some(inv) = w.get::<Inventory>(owner) else {
        return Vec::new();
    };
    inv.weapons
        .iter()
        .filter_map(|e| w.get::<Weapon>(*e).map(|x| x.id))
        .collect()
}

/// Queue a weapon switch for the local player.
fn select_local(
    w: &mut World,
    pick: impl Fn(&Inventory, &[(Entity, u8)]) -> Option<Entity>,
) -> Result<Option<String>, String> {
    let player = w
        .query_filtered::<Entity, With<LocalPlayer>>()
        .iter(w)
        .next()
        .ok_or("no local player")?;
    let weapons: Vec<(Entity, u8)> = w
        .query::<(Entity, &Weapon)>()
        .iter(w)
        .map(|(e, x)| (e, x.slot))
        .collect();
    let mut inv = w.get_mut::<Inventory>(player).ok_or("no inventory")?;
    if let Some(next) = pick(&inv, &weapons) {
        inv.wanted = Some(next);
    }
    Ok(None)
}

// ---------------------------------------------------------------------------
// Registry

/// One weapon a game offers: `build` inserts its parts on a fresh entity.
pub struct WeaponDef {
    /// Namespaced, e.g. `cs_source:weapon_ak47`.
    pub id: &'static str,
    pub build: fn(&mut EntityWorldMut),
}

#[derive(Resource, Default)]
pub struct WeaponRegistry(pub Vec<WeaponDef>);

impl WeaponRegistry {
    /// By full ID, or by the part after the namespace (`weapon_ak47`).
    pub fn find(&self, id: &str) -> Option<&WeaponDef> {
        let id = id.to_lowercase();
        self.0
            .iter()
            .find(|d| d.id == id)
            .or_else(|| self.0.iter().find(|d| d.id.rsplit(':').next() == Some(id.as_str())))
    }
}

pub trait RegisterWeapons {
    fn register_weapon(&mut self, id: &'static str, build: fn(&mut EntityWorldMut)) -> &mut Self;
}

impl RegisterWeapons for App {
    fn register_weapon(&mut self, id: &'static str, build: fn(&mut EntityWorldMut)) -> &mut Self {
        self.init_resource::<WeaponRegistry>()
            .world_mut()
            .resource_mut::<WeaponRegistry>()
            .0
            .push(WeaponDef { id, build });
        self
    }
}

/// Weapons every new character gets, first one drawn last (Source draws
/// the best weapon).
#[derive(Resource, Default, Clone)]
pub struct StartingWeapons(pub Vec<&'static str>);

/// Give `owner` the weapon `id`; draws it if they hold nothing. Returns the
/// weapon entity.
pub fn give(world: &mut World, owner: Entity, id: &str) -> Option<Entity> {
    let build = world.resource::<WeaponRegistry>().find(id)?.build;
    let mut e = world.spawn((Name::new(id.to_string()), WeaponState::default()));
    build(&mut e);
    let weapon = e.id();
    if let Some(mut w) = world.get_mut::<Weapon>(weapon) {
        w.owner = Some(owner);
    }
    let Ok(mut o) = world.get_entity_mut(owner) else {
        world.despawn(weapon);
        return None;
    };
    if o.get::<Inventory>().is_none() {
        o.insert(Inventory::default());
    }
    let mut inv = o.get_mut::<Inventory>().unwrap();
    inv.weapons.push(weapon);
    inv.wanted = Some(weapon);
    Some(weapon)
}

fn give_starting_weapons(world: &mut World) {
    let new: Vec<Entity> = world
        .query_filtered::<Entity, (With<Intent>, With<Health>, Without<Inventory>)>()
        .iter(world)
        .collect();
    if new.is_empty() {
        return;
    }
    let ids = world.resource::<StartingWeapons>().0.clone();
    for owner in new {
        world.entity_mut(owner).insert(Inventory::default());
        for id in &ids {
            give(world, owner, id);
        }
    }
}

// ---------------------------------------------------------------------------
// Components

/// What a character carries, and the player-level weapon timers.
#[derive(Component, Default, Debug)]
pub struct Inventory {
    pub weapons: Vec<Entity>,
    pub active: Option<Entity>,
    /// The previously active weapon (Source `lastinv`).
    pub last: Option<Entity>,
    /// Switch to this weapon at the start of the next tick.
    pub wanted: Option<Entity>,
    /// No weapon logic before this time (Source's player "next attack").
    pub next_attack: f64,
    /// Last tick's buttons, for press and release edges.
    prev_fire: bool,
    prev_secondary: bool,
    prev_select: Option<u8>,
    prev_last: bool,
    /// Counts ticks with an attack; seeds the shot spread.
    command: u32,
}

/// A weapon's identity and handling.
#[derive(Component, Clone, Debug)]
pub struct Weapon {
    pub id: &'static str,
    /// Slot key (0 = first) used by weapon selection.
    pub slot: u8,
    pub owner: Option<Entity>,
    /// Seconds from drawing to the first allowed attack.
    pub draw_time: f32,
    /// Max movement speed while held, m/s (None: the movement's own).
    pub max_speed: Option<f32>,
}

/// Trigger: how the primary attack is activated.
#[derive(Component, Clone, Debug)]
pub struct Trigger {
    /// Holding fires again; otherwise every shot needs a fresh press.
    pub automatic: bool,
    /// Seconds between shots.
    pub cycle: f32,
    pub timing: FireTiming,
}

/// How refire times are kept (spec 3.2).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FireTiming {
    /// One shot, then next = now + cycle (fire rate quantised to ticks).
    Set,
    /// next += cycle per shot; several shots per tick if behind.
    Accumulate,
    /// CS:S (measured, spec M4): a fresh press sets next = now + cycle;
    /// while held, next = previous next + cycle, at most one shot a tick
    /// (the AK-47's 0.1 s fires 7, 7, 7, 6 ticks apart).
    CarryOver,
}

/// Cost: a magazine. Rounds move from the reserve when a reload ends.
#[derive(Component, Clone, Debug)]
pub struct Magazine {
    pub clip: u32,
    pub size: u32,
    pub reserve: u32,
    /// Most rounds the reserve holds (CS:S: `ammo_<type>_max`).
    pub reserve_max: u32,
    /// Seconds from starting a reload to the swap.
    pub reload_time: f32,
    /// Holding attack on an empty clip reloads as soon as allowed (the
    /// SDK); otherwise it dry-fires once and the reload waits for the
    /// buttons to be released (CS:S, measured M10).
    pub reload_while_held: bool,
}

/// Delivery: instant traces from the eye.
#[derive(Component, Clone, Debug)]
pub struct Hitscan {
    pub range: f32,
    pub pellets: u32,
    pub spread: SpreadShape,
    /// Shots go along the view plus this times the owner's `ViewPunch`.
    pub punch_scale: f32,
}

/// How shots scatter around the aim, as offsets along its right and up
/// vectors (tangent units).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum SpreadShape {
    /// SDK template: each axis U(-.5,.5)+U(-.5,.5), times the scale.
    Template(f32),
    /// CS:S (measured M2): a uniform radius up to `inaccuracy` at a random
    /// angle, plus one up to `spread` at another. Game code updates
    /// `inaccuracy` as it changes.
    Disc { inaccuracy: f32, spread: f32 },
}

/// Recoil on a character's view (pitch up, yaw left; radians): added to
/// the camera, and to shots by `Hitscan::punch_scale`. Games that model
/// recoil add and decay it.
#[derive(Component, Clone, Copy, Debug, Default, PartialEq)]
pub struct ViewPunch(pub Vec2);

/// Delivery: melee swings, one per attack button.
#[derive(Component, Clone, Debug)]
pub struct Melee {
    pub primary: Swing,
    pub secondary: Option<Swing>,
}

#[derive(Clone, Debug)]
pub struct Swing {
    pub range: f32,
    /// Half size of the box swept when the line misses (spec 6.1); none: no
    /// fallback.
    pub hull: Option<f32>,
    /// How far the box is swept (the SDK: range minus the box's corner
    /// distance; CS:S's knife: the full range).
    pub hull_reach: f32,
    /// The box only hits targets within this cosine of the view.
    pub facing_cos: f32,
    pub damage: f32,
    /// (window, damage): this damage instead when the previous swing of
    /// this kind was less than `window` seconds ago (CS:S slash: 15 within
    /// 0.9 s of the last slash).
    pub follow_up: Option<(f32, f32)>,
    /// Damage when hitting from behind, if different: the target faces
    /// within acos(`BACKSTAB_COS`) of the attacker-to-target bearing.
    pub backstab: Option<f32>,
    /// Hits scale by hitgroup (the SDK); CS:S's knife reports generic.
    pub hitgroups: bool,
    /// Seconds until either attack after a hit.
    pub hit_refire: f32,
    /// After a miss: until this swing again, and until the other one.
    pub miss_refire: f32,
    pub miss_refire_other: f32,
    /// Impulse per unit of damage, kg·m/s.
    pub force: f32,
    pub sound_hit: Option<String>,
    pub sound_hit_world: Option<String>,
    pub sound_miss: Option<String>,
}

/// Effect: damage on hit, for hitscan deliveries.
#[derive(Component, Clone, Debug)]
pub struct DamageEffect {
    pub amount: f32,
    /// Damage × falloff^(distance / falloff_step).
    pub falloff: f32,
    pub falloff_step: f32,
    pub hitgroups: HitgroupScale,
    /// Impulse given to physics objects per shot, kg·m/s.
    pub impulse: f32,
    /// Damage dealt is truncated to whole multiples of this (CS:S: one hit
    /// point, 0.01); 0 keeps fractions.
    pub quantum: f32,
}

/// Damage multiplier per hitgroup.
#[derive(Clone, Copy, Debug)]
pub struct HitgroupScale {
    pub head: f32,
    pub chest: f32,
    pub stomach: f32,
    pub arm: f32,
    pub leg: f32,
}

impl Default for HitgroupScale {
    /// The SDK's generic multipliers (sk_player_*).
    fn default() -> Self {
        Self {
            head: 2.0,
            chest: 1.0,
            stomach: 1.0,
            arm: 1.0,
            leg: 1.0,
        }
    }
}

impl HitgroupScale {
    pub fn get(&self, g: Hitgroup) -> f32 {
        match g {
            Hitgroup::Generic => 1.0,
            Hitgroup::Head => self.head,
            Hitgroup::Chest => self.chest,
            Hitgroup::Stomach => self.stomach,
            Hitgroup::LeftArm | Hitgroup::RightArm => self.arm,
            Hitgroup::LeftLeg | Hitgroup::RightLeg => self.leg,
        }
    }
}

/// Sound entries a weapon plays.
#[derive(Component, Clone, Debug, Default)]
pub struct WeaponSounds {
    pub fire: Option<String>,
    pub empty: Option<String>,
    pub deploy: Option<String>,
    /// Entries at times after a reload starts (view-model animation events).
    pub reload: Vec<(f32, String)>,
}

/// Source's per-weapon timers and flags.
#[derive(Component, Clone, Debug, Default)]
pub struct WeaponState {
    pub next_primary: f64,
    pub next_secondary: f64,
    /// When the reload in progress swaps the magazine.
    pub reload_end: Option<f64>,
    /// Seconds attack has been held.
    pub fire_duration: f32,
    pub fired_on_empty: bool,
    pub next_empty_sound: f64,
    /// Shots since the trigger was last released.
    pub burst: u32,
    /// When each swing (primary, secondary) was last made.
    pub last_swing: [Option<f64>; 2],
    /// Sounds waiting for their time (reload parts).
    pending: Vec<(f64, String)>,
}

/// What weapons did this tick, for HUDs, effects and tests.
#[derive(Message, Clone, Debug)]
pub struct WeaponEvent {
    pub owner: Entity,
    pub weapon: Entity,
    pub kind: WeaponEventKind,
}

#[derive(Clone, Debug)]
pub enum WeaponEventKind {
    Deployed,
    /// One trace: from the eye to where it stopped.
    Shot {
        from: Vec3,
        to: Vec3,
        hit: Option<Entity>,
        /// Surface normal where it hit.
        normal: Option<Vec3>,
    },
    /// Damage dealt to something with health (summed per firing call).
    Hit {
        target: Entity,
        amount: f32,
        hitgroup: Hitgroup,
    },
    /// A melee swing (hit or not).
    Swing {
        hit: bool,
        secondary: bool,
    },
    DryFire,
    ReloadStarted,
    Reloaded,
}

// ---------------------------------------------------------------------------
// Selection and deploy

fn select_weapons(
    mut owners: Query<(Entity, &Intent, &mut Inventory, &Transform)>,
    mut weapons: Query<(&Weapon, &mut WeaponState, Option<&WeaponSounds>)>,
    mut commands: Commands,
    mut events: MessageWriter<WeaponEvent>,
    mut play: MessageWriter<PlaySound>,
    time: Res<Time>,
) {
    let now = time.elapsed_secs_f64();
    for (owner, intent, mut inv, at) in &mut owners {
        inv.weapons.retain(|w| weapons.contains(*w));
        // Slot keys on press: the first weapon in that slot, or the next one
        // after the active weapon when several share it.
        if intent.select != inv.prev_select
            && let Some(slot) = intent.select
        {
            let in_slot: Vec<Entity> = inv
                .weapons
                .iter()
                .copied()
                .filter(|w| weapons.get(*w).is_ok_and(|(w, ..)| w.slot == slot))
                .collect();
            let next = match inv.active.and_then(|a| in_slot.iter().position(|w| *w == a)) {
                Some(i) => in_slot[(i + 1) % in_slot.len()],
                None => match in_slot.first() {
                    Some(w) => *w,
                    None => {
                        inv.prev_select = intent.select;
                        continue;
                    }
                },
            };
            inv.wanted = Some(next);
        }
        inv.prev_select = intent.select;
        if intent.last_weapon && !inv.prev_last {
            inv.wanted = inv.last.filter(|l| inv.weapons.contains(l));
        }
        inv.prev_last = intent.last_weapon;

        let Some(want) = inv.wanted.take() else { continue };
        // Selecting what you hold changes nothing (spec 3.6).
        if inv.active == Some(want) {
            continue;
        }
        // Holster: cancels a reload; CS:S models have no holster animation.
        if let Some(old) = inv.active
            && let Ok((_, mut st, _)) = weapons.get_mut(old)
        {
            st.reload_end = None;
            st.pending.clear();
            st.fire_duration = 0.0;
        }
        let Ok((w, mut st, sounds)) = weapons.get_mut(want) else {
            continue;
        };
        // Deploy (spec 3.3).
        let ready = now + w.draw_time as f64;
        inv.next_attack = ready;
        st.next_primary = ready;
        st.next_secondary = ready;
        st.fired_on_empty = false;
        inv.last = inv.active;
        inv.active = Some(want);
        match w.max_speed {
            Some(s) => commands.entity(owner).insert(MaxSpeed(s)),
            None => commands.entity(owner).remove::<MaxSpeed>(),
        };
        if let Some(s) = sounds.and_then(|s| s.deploy.clone()) {
            play.write(owner_sound(s, owner, at.translation, CHAN_ITEM));
        }
        events.write(WeaponEvent {
            owner,
            weapon: want,
            kind: WeaponEventKind::Deployed,
        });
    }
}

/// A sound from the weapon's owner at `at`.
fn owner_sound(entry: String, owner: Entity, at: Vec3, channel: u8) -> PlaySound {
    PlaySound {
        entry,
        at: Some(at),
        volume: None,
        source: Some(owner),
        channel: Some(channel),
    }
}

// ---------------------------------------------------------------------------
// The weapon frame (spec 3.1)

#[derive(bevy::ecs::query::QueryData)]
#[query_data(mutable)]
struct WeaponParts {
    entity: Entity,
    weapon: &'static Weapon,
    state: &'static mut WeaponState,
    trigger: Option<&'static Trigger>,
    magazine: Option<&'static mut Magazine>,
    hitscan: Option<&'static Hitscan>,
    melee: Option<&'static Melee>,
    effect: Option<&'static DamageEffect>,
    sounds: Option<&'static WeaponSounds>,
}

/// Seconds between dry-fire clicks (spec: empty_sound_interval).
const EMPTY_SOUND_INTERVAL: f64 = 0.5;

#[allow(clippy::too_many_arguments)]
fn weapon_frame(
    mut owners: Query<(
        Entity,
        &Intent,
        &mut Inventory,
        &Transform,
        &MovementState,
        Option<&Health>,
        Option<&ViewPunch>,
    )>,
    mut weapons: Query<WeaponParts>,
    mut world: deliver::World,
    time: Res<Time>,
) {
    let now = time.elapsed_secs_f64();
    let dt = time.delta_secs();
    for (owner, intent, mut inv, transform, state, health, punch) in &mut owners {
        if health.is_some_and(|h| h.current <= 0.0) {
            continue;
        }
        let pressed = intent.fire && !inv.prev_fire;
        let released2 = !intent.secondary && inv.prev_secondary;
        inv.prev_fire = intent.fire;
        inv.prev_secondary = intent.secondary;
        inv.command = inv.command.wrapping_add(1);
        let Some(active) = inv.active else { continue };
        let Ok(mut w) = weapons.get_mut(active) else { continue };
        if !intent.fire {
            w.state.burst = 0;
        }
        // Busy (drawing, reloading): only the timers run.
        if now < inv.next_attack {
            continue;
        }
        let eye = transform.translation + state.eye_offset;
        // Shots follow the view plus the weapon's share of the recoil.
        let kick = punch.map_or(Vec2::ZERO, |p| p.0) * w.hitscan.map_or(0.0, |h| h.punch_scale);
        let aim = Quat::from_euler(EulerRot::YXZ, intent.yaw + kick.y, intent.pitch + kick.x, 0.0);
        let mut ctx = deliver::Shot {
            owner,
            weapon: active,
            eye,
            aim,
            seed: owner.to_bits() as u32 ^ inv.command.wrapping_mul(0x9E37_79B9),
            w: &mut world,
        };

        // 1. Fire duration.
        w.state.fire_duration = if intent.fire { w.state.fire_duration + dt } else { 0.0 };

        // 2. Reload completion.
        if let Some(end) = w.state.reload_end
            && now >= end
        {
            if let Some(mag) = w.magazine.as_mut() {
                let n = (mag.size - mag.clip).min(mag.reserve);
                mag.clip += n;
                mag.reserve -= n;
            }
            w.state.reload_end = None;
            w.state.next_primary = now;
            w.state.next_secondary = now;
            ctx.w.events.write(WeaponEvent {
                owner,
                weapon: active,
                kind: WeaponEventKind::Reloaded,
            });
        }

        // 3. Secondary attack has priority (melee only for now).
        let mut blocked = false;
        if intent.secondary
            && let Some(swing) = w.melee.and_then(|m| m.secondary.clone())
            && w.state.next_secondary <= now
        {
            let swing = follow_up(&swing, w.state.last_swing[1], now);
            let hit = ctx.swing(&swing, true);
            let (own, other) = swing_refire(&swing, hit, now, &mut w.state.last_swing[1]);
            w.state.next_secondary = own;
            w.state.next_primary = other;
            blocked = true;
        }

        // 4. Primary attack.
        if intent.fire && !blocked && w.state.next_primary <= now {
            let automatic = w.trigger.is_none_or(|t| t.automatic);
            if !automatic && w.state.burst > 0 {
                // Semi-automatic: wait for a fresh press.
            } else if w.magazine.as_ref().is_some_and(|m| m.clip == 0) {
                if w.state.fired_on_empty {
                    if w.magazine.as_ref().is_some_and(|m| m.reload_while_held) {
                        try_reload(&mut w, &mut inv, now, owner, &mut ctx.w.events);
                    }
                } else {
                    // Fire on empty (spec 3.5).
                    w.state.fired_on_empty = true;
                    if now >= w.state.next_empty_sound {
                        w.state.next_empty_sound = now + EMPTY_SOUND_INTERVAL;
                        if let Some(s) = w.sounds.and_then(|s| s.empty.clone()) {
                            ctx.w.play.write(owner_sound(s, owner, eye, CHAN_ITEM));
                        }
                    }
                    w.state.next_primary = now + 0.2;
                    ctx.w.events.write(WeaponEvent {
                        owner,
                        weapon: active,
                        kind: WeaponEventKind::DryFire,
                    });
                }
            } else {
                if pressed || released2 {
                    // A fresh press drops any accumulated lag (spec 3.2).
                    w.state.next_primary = now;
                }
                if let Some(swing) = w.melee.map(|m| m.primary.clone()) {
                    let swing = follow_up(&swing, w.state.last_swing[0], now);
                    let hit = ctx.swing(&swing, false);
                    let (own, other) = swing_refire(&swing, hit, now, &mut w.state.last_swing[0]);
                    w.state.next_primary = own;
                    w.state.next_secondary = other;
                    w.state.burst += 1;
                } else if let Some(trigger) = w.trigger.cloned() {
                    let mut shots = 0;
                    match trigger.timing {
                        FireTiming::Set => {
                            shots = 1;
                            w.state.next_primary = now + trigger.cycle as f64;
                            w.state.next_secondary = w.state.next_primary;
                        }
                        FireTiming::Accumulate => {
                            while w.state.next_primary <= now {
                                shots += 1;
                                w.state.next_primary += trigger.cycle as f64;
                            }
                        }
                        FireTiming::CarryOver => {
                            // A fresh press reset the timer to now above.
                            shots = 1;
                            w.state.next_primary += trigger.cycle as f64;
                            w.state.next_secondary = w.state.next_primary;
                        }
                    }
                    if let Some(mag) = w.magazine.as_mut() {
                        shots = shots.min(mag.clip);
                        mag.clip -= shots;
                    }
                    for i in 0..shots {
                        if let (Some(scan), Some(effect)) = (w.hitscan, w.effect) {
                            ctx.seed = ctx.seed.wrapping_add(i);
                            ctx.fire(scan, effect);
                        }
                        if let Some(s) = w.sounds.and_then(|s| s.fire.clone()) {
                            ctx.w.play.write(owner_sound(s, owner, eye, CHAN_WEAPON));
                        }
                    }
                    w.state.burst += shots;
                }
            }
        }

        // 5. Reload key.
        if intent.reload && w.state.next_primary <= now && w.state.reload_end.is_none() {
            try_reload(&mut w, &mut inv, now, owner, &mut ctx.w.events);
            w.state.fire_duration = 0.0;
        }

        // 6. No buttons: automatic reload of an empty clip.
        if !intent.fire && !intent.secondary && !intent.reload {
            w.state.fired_on_empty = false;
            if w.magazine.as_ref().is_some_and(|m| m.clip == 0)
                && w.state.next_primary <= now
                && w.state.next_secondary <= now
                && w.state.reload_end.is_none()
            {
                try_reload(&mut w, &mut inv, now, owner, &mut ctx.w.events);
            }
        }
    }
}

/// The swing with its follow-up damage when the previous one was recent.
fn follow_up(swing: &Swing, last_swing: Option<f64>, now: f64) -> Swing {
    match (swing.follow_up, last_swing) {
        (Some((window, damage)), Some(t)) if now - t < window as f64 => Swing {
            damage,
            ..swing.clone()
        },
        _ => swing.clone(),
    }
}

/// Next times for (this swing, the other) after a swing, recording it.
fn swing_refire(swing: &Swing, hit: bool, now: f64, last_swing: &mut Option<f64>) -> (f64, f64) {
    *last_swing = Some(now);
    if hit {
        let t = now + swing.hit_refire as f64;
        (t, t)
    } else {
        (now + swing.miss_refire as f64, now + swing.miss_refire_other as f64)
    }
}

/// Start a reload (spec 3.4): refused when nothing would move.
fn try_reload(
    w: &mut WeaponPartsItem,
    inv: &mut Inventory,
    now: f64,
    owner: Entity,
    events: &mut MessageWriter<WeaponEvent>,
) {
    let Some(mag) = w.magazine.as_ref() else { return };
    if (mag.size - mag.clip).min(mag.reserve) == 0 {
        return;
    }
    let end = now + mag.reload_time as f64;
    inv.next_attack = end;
    w.state.next_primary = end;
    w.state.next_secondary = end;
    w.state.reload_end = Some(end);
    w.state.fired_on_empty = false;
    if let Some(s) = w.sounds {
        w.state.pending = s.reload.iter().map(|(t, e)| (now + *t as f64, e.clone())).collect();
    }
    events.write(WeaponEvent {
        owner,
        weapon: w.entity,
        kind: WeaponEventKind::ReloadStarted,
    });
}

/// Play sounds whose time has come (reload parts), from the owner.
fn timed_sounds(
    mut weapons: Query<(&Weapon, &mut WeaponState)>,
    owners: Query<&Transform>,
    mut play: MessageWriter<PlaySound>,
    time: Res<Time>,
) {
    let now = time.elapsed_secs_f64();
    for (w, mut st) in &mut weapons {
        let Some(owner) = w.owner else { continue };
        if st.pending.is_empty() {
            continue;
        }
        let at = owners.get(owner).map_or(Vec3::ZERO, |t| t.translation);
        st.pending.retain(|(t, entry)| {
            if *t <= now {
                play.write(owner_sound(entry.clone(), owner, at, CHAN_ITEM));
                false
            } else {
                true
            }
        });
    }
}
