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
pub mod drop;
pub mod economy;
pub mod grenade;

use std::sync::Arc;

use bevy::prelude::*;

pub use deliver::{armor_split, falloff, hitgroup_at, quantize, spread_dir};

use crate::{
    console::{Command, Console, ConsoleAppExt},
    core::{Health, Hitgroup, Intent, LocalPlayer, MaxSpeed, MovementState, SimSet, Team},
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
            .init_resource::<economy::Prices>()
            .init_resource::<economy::BuyWindow>()
            .register_type::<economy::Money>()
            .init_resource::<PassMaterials>()
            .init_resource::<StartingWeapons>()
            .add_message::<WeaponEvent>()
            .add_message::<crate::map::RagdollShot>()
            .add_message::<PlaySound>()
            .add_systems(
                FixedUpdate,
                (
                    (give_starting_weapons, drop::pick_up, select_weapons)
                        .chain()
                        .before(SimSet::Movement),
                    drop::drop_on_death.after(SimSet::Weapons),
                    (weapon_frame.in_set(WeaponFrame), timed_sounds, apply_zoom, ragdoll_shots)
                        .chain()
                        .in_set(SimSet::Weapons),
                ),
            );
        grenade::plugin(app);
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
        app.console_command("drop", "Drop the weapon you hold (G); walk over one to pick it up.", |w, _| {
            let player = local_player(w)?;
            drop::drop_weapon(w, player, true).ok_or("nothing to drop")?;
            Ok(None)
        });
        app.console_command("lastinv", "Switch to the previously held weapon.", |w, _| {
            select_local(w, |inv, _| inv.last)
        })
        .console_command(
            "impulse",
            "impulse 101: every weapon, full clips and reserves, kevlar and helmet.",
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
                w.entity_mut(player).insert(Armor {
                    amount: 1.0,
                    helmet: true,
                });
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
        .console_command(
            "buy",
            "buy <weapon>|vest|vesthelm, e.g. buy ak47 (costs money when you have some).",
            |w, a| {
                let name = a.first().ok_or("buy <weapon>")?.clone();
                let player = local_player(w)?;
                economy::buy(w, player, &name).map(Some)
            },
        );
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

/// Weapons every new character gets, the last one given drawn (Source
/// draws the best weapon): its team's list (`team`: the entry for its
/// team, else the one with no team), then `all`.
#[derive(Resource, Default, Clone)]
pub struct StartingWeapons {
    pub team: Vec<(Option<u8>, Vec<&'static str>)>,
    pub all: Vec<&'static str>,
}

impl StartingWeapons {
    /// What a character of `team` starts with, in giving order.
    pub fn for_team(&self, team: Option<u8>) -> Vec<&'static str> {
        let own = self
            .team
            .iter()
            .find(|(t, _)| t.is_some() && *t == team)
            .or_else(|| self.team.iter().find(|(t, _)| t.is_none()));
        own.map(|(_, ids)| ids.clone())
            .unwrap_or_default()
            .into_iter()
            .chain(self.all.iter().copied())
            .collect()
    }

    pub fn is_empty(&self) -> bool {
        self.all.is_empty() && self.team.iter().all(|(_, ids)| ids.is_empty())
    }
}

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
    let new: Vec<(Entity, Option<u8>)> = world
        .query_filtered::<(Entity, Option<&Team>), (With<Intent>, With<Health>, Without<Inventory>)>()
        .iter(world)
        .map(|(e, t)| (e, t.map(|t| t.0)))
        .collect();
    if new.is_empty() {
        return;
    }
    let start = world.resource::<StartingWeapons>().clone();
    for (owner, team) in new {
        world.entity_mut(owner).insert(Inventory::default());
        for id in start.for_team(team) {
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

/// Delivery add-on: bullets that pass through things (Source's bullet
/// penetration). Everything passed, walls and characters alike, counts as
/// one object and uses part of a shared budget: a wall uses its thickness
/// along the path divided by its material's scale (`PassMaterials`), a
/// character a fixed amount. The bullet leaves an object only while the
/// budget stays at or above zero and fewer than `objects` were passed;
/// either way it still hits the next thing. Damage falls off again at
/// every hit (by the distance from the eye) and each object passed scales
/// what's carried on by its material's damage factor.
#[derive(Component, Clone, Debug)]
pub struct Penetration {
    /// Budget, meters of a scale-1 material.
    pub power: f32,
    /// Objects it can pass.
    pub objects: u32,
    /// Objects this far from the eye or farther are not passed, meters.
    pub max_distance: f32,
}

/// How bullets pass each kind of thing (see `Penetration`). Games set it.
#[derive(Resource, Clone, Debug)]
pub struct PassMaterials {
    /// By material class (`MapSurface::game_material`).
    pub by_class: Vec<(char, PassMaterial)>,
    /// Classes not listed, and surfaces whose class isn't known.
    pub default: PassMaterial,
    /// Characters (anything with `Intent` or `Hitboxes`).
    pub character: CharacterPass,
}

impl Default for PassMaterials {
    fn default() -> Self {
        Self {
            by_class: Vec::new(),
            default: PassMaterial {
                scale: 1.0,
                damage: 0.5,
            },
            character: CharacterPass {
                cost: f32::INFINITY,
                damage: 0.5,
                range_scale: 1.0,
            },
        }
    }
}

impl PassMaterials {
    pub fn get(&self, class: Option<char>) -> PassMaterial {
        class
            .and_then(|c| self.by_class.iter().find(|(k, _)| *k == c))
            .map_or(self.default, |(_, m)| *m)
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PassMaterial {
    /// Meters of this material a bullet passes per meter of budget.
    pub scale: f32,
    /// Damage carried on after passing, as a factor.
    pub damage: f32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CharacterPass {
    /// Budget one character uses, meters.
    pub cost: f32,
    /// Damage carried on after passing, as a factor (of the hit's damage
    /// before its hitgroup multiplier).
    pub damage: f32,
    /// After a character, the range left beyond its exit is scaled by this.
    pub range_scale: f32,
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

/// Secondary attack: step through the weapon's modes (0 = normal; scope
/// levels, silencer on, burst). Other parts read `current`: `Zoom`,
/// `Burst`, `WeaponSounds::fire_alt`, games' accuracy. Holding the button
/// steps again every `toggle_time`.
#[derive(Component, Clone, Debug)]
pub struct AltModes {
    /// Modes including the normal one.
    pub count: u8,
    pub current: u8,
    /// Seconds until the secondary attack again (and the primary too when
    /// `blocks_primary`, e.g. while a silencer is screwed on).
    pub toggle_time: f32,
    pub blocks_primary: bool,
    /// Played on every step.
    pub sound: Option<String>,
}

impl AltModes {
    pub fn new(count: u8, toggle_time: f32, blocks_primary: bool) -> Self {
        Self {
            count,
            current: 0,
            toggle_time,
            blocks_primary,
            sound: None,
        }
    }
}

/// Modes 1.. of `AltModes` look through a scope: the owner's view narrows
/// to `fov[mode - 1]` (horizontal degrees at 4:3, like Source's `fov`) and
/// it moves at most `max_speed`. Switching away and reloading unzoom.
#[derive(Component, Clone, Debug)]
pub struct Zoom {
    pub fov: Vec<f32>,
    pub max_speed: Option<f32>,
    /// A shot unzooms; the zoom comes back when the weapon can fire again
    /// (CS:S's AWP and scout).
    pub unzoom_after_shot: bool,
    /// Drawn as a sniper scope (overlay, no view model).
    pub scope: bool,
}

/// While the weapon is in mode `mode`, one trigger pull fires `count`
/// rounds `interval` seconds apart, and the next pull may come `refire`
/// seconds after the first round.
#[derive(Component, Clone, Debug)]
pub struct Burst {
    pub mode: u8,
    pub count: u32,
    pub interval: f32,
    pub refire: f32,
}

/// On a character looking through a zoomed weapon: its field of view
/// (horizontal degrees at 4:3) and whether that is a sniper scope.
#[derive(Component, Clone, Copy, Debug, PartialEq)]
pub struct Zoomed {
    pub fov: f32,
    pub scope: bool,
}

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
    /// As `DamageEffect::armor_ratio`; damage truncation as its `quantum`.
    pub armor_ratio: Option<f32>,
    pub quantum: f32,
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
    /// How much of a hit armour lets through to health (`Armor`); None:
    /// armour doesn't apply.
    pub armor_ratio: Option<f32>,
}

/// Body armour (CS:S kevlar and helmet), normalized like `Health`
/// (1.0 = 100). Hits it covers (CS:S, measured M7: chest, stomach, arms,
/// generic, and the head only with a helmet) take `damage x ratio x 0.5`
/// from health and half the rest from armour, both truncated.
#[derive(Component, Clone, Copy, Debug, Default, PartialEq)]
pub struct Armor {
    pub amount: f32,
    pub helmet: bool,
}

impl Armor {
    pub fn covers(&self, group: Hitgroup) -> bool {
        self.amount > 0.0
            && match group {
                Hitgroup::Head => self.helmet,
                Hitgroup::LeftLeg | Hitgroup::RightLeg => false,
                _ => true,
            }
    }
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
    /// Instead of `fire` in any alternate mode (`AltModes`), e.g. silenced.
    pub fire_alt: Option<String>,
    pub empty: Option<String>,
    pub deploy: Option<String>,
    /// Entries at times after a reload starts (view-model animation events).
    pub reload: Vec<(f32, String)>,
    /// Entries at times after the weapon is drawn.
    pub draw: Vec<(f32, String)>,
    /// (mode, time, entry): played that long after stepping to that
    /// `AltModes` mode with attack2 (e.g. the silencer going on or off).
    pub modes: Vec<(u8, f32, String)>,
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
    /// Rounds of the current burst still to fire, and when the next goes.
    pub burst_left: u32,
    pub next_burst_round: f64,
    /// The zoom mode to return to once the weapon can fire again.
    pub rezoom: Option<u8>,
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
    /// The same bullet going on after passing something (`Penetration`):
    /// from where it left that to where it stopped next.
    ShotContinued {
        from: Vec3,
        to: Vec3,
        hit: Option<Entity>,
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
        /// Where it hit: point, surface normal and what was hit.
        at: Option<(Vec3, Vec3, Entity)>,
    },
    DryFire,
    /// The weapon stepped to this `AltModes` mode (attack2, or a sniper
    /// shot unzooming and re-zooming).
    ModeChanged {
        mode: u8,
    },
    ReloadStarted,
    Reloaded,
    /// A grenade's pin came out (`grenade::Throwable`).
    PinPulled,
    /// A grenade was released (it leaves the hand a moment later).
    Thrown,
}

// ---------------------------------------------------------------------------
// Selection and deploy

fn select_weapons(
    mut owners: Query<(Entity, &Intent, &mut Inventory, &Transform)>,
    mut weapons: Query<(
        &Weapon,
        &mut WeaponState,
        Option<&WeaponSounds>,
        Option<&mut AltModes>,
        Option<&Zoom>,
    )>,
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
        // Holster: cancels a reload, a burst and the zoom; CS:S models have
        // no holster animation.
        if let Some(old) = inv.active
            && let Ok((_, mut st, _, modes, zoom)) = weapons.get_mut(old)
        {
            st.reload_end = None;
            st.pending.clear();
            st.fire_duration = 0.0;
            st.burst_left = 0;
            st.rezoom = None;
            if let (Some(mut modes), Some(_)) = (modes, zoom) {
                modes.current = 0;
            }
        }
        let Ok((w, mut st, sounds, ..)) = weapons.get_mut(want) else {
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
        if let Some(s) = sounds {
            st.pending = s.draw.iter().map(|(t, e)| (now + *t as f64, e.clone())).collect();
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
    penetration: Option<&'static Penetration>,
    sounds: Option<&'static WeaponSounds>,
    modes: Option<&'static mut AltModes>,
    zoom: Option<&'static Zoom>,
    burst: Option<&'static Burst>,
}

impl WeaponPartsItem<'_, '_> {
    fn mode(&self) -> u8 {
        self.modes.as_ref().map_or(0, |m| m.current)
    }

    /// Change mode, telling the HUD and view model.
    fn set_mode(&mut self, mode: u8, owner: Entity, events: &mut MessageWriter<WeaponEvent>) {
        let Some(m) = self.modes.as_mut() else { return };
        if m.current == mode {
            return;
        }
        m.current = mode;
        events.write(WeaponEvent {
            owner,
            weapon: self.entity,
            kind: WeaponEventKind::ModeChanged { mode },
        });
    }

    /// Fire up to `rounds` from the clip: traces, sounds; returns how many.
    fn fire_rounds(&mut self, ctx: &mut deliver::Shot, rounds: u32) -> u32 {
        let mut shots = rounds;
        if let Some(mag) = self.magazine.as_mut() {
            shots = shots.min(mag.clip);
            mag.clip -= shots;
        }
        let alt = self.mode() > 0;
        for i in 0..shots {
            if let (Some(scan), Some(effect)) = (self.hitscan, self.effect) {
                ctx.seed = ctx.seed.wrapping_add(i);
                ctx.fire(scan, effect, self.penetration);
            }
            let sound = self.sounds.and_then(|s| {
                if alt && s.fire_alt.is_some() {
                    s.fire_alt.clone()
                } else {
                    s.fire.clone()
                }
            });
            if let Some(s) = sound {
                ctx.w.play.write(owner_sound(s, ctx.owner, ctx.eye, CHAN_WEAPON));
            }
        }
        self.state.burst += shots;
        shots
    }
}

/// Seconds between dry-fire clicks (spec: empty_sound_interval).
const EMPTY_SOUND_INTERVAL: f64 = 0.5;

/// Timers that come due exactly on a tick fire on that tick: CS:S does
/// (measured M4/M8: the M4A1's 0.975 s draw takes 65 ticks, its 0.09 s
/// cycle 6, the pistols' 0.15 s 10), while our f32 durations widened to
/// f64 land a hair past the tick. Times are compared with this slack.
const TIME_SLACK: f64 = 1e-5;

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
    // Compare timers against this; record new ones from `now`.
    let due = now + TIME_SLACK;
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
        // A sniper shot's zoom comes back when the weapon can fire again
        // (measured M15).
        if let Some(mode) = w.state.rezoom
            && due >= w.state.next_primary
        {
            w.state.rezoom = None;
            w.set_mode(mode, owner, &mut world.events);
        }
        // Busy (drawing, reloading): only the timers run.
        if due < inv.next_attack {
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
            && due >= end
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

        // The rest of a burst fires on its own, button or not (M16).
        if w.state.burst_left > 0 && due >= w.state.next_burst_round {
            w.state.burst_left -= 1;
            w.state.next_burst_round += w.burst.map_or(0.0, |b| b.interval) as f64;
            if w.fire_rounds(&mut ctx, 1) == 0 {
                w.state.burst_left = 0;
            }
        }

        // 3. Secondary attack has priority: a melee swing or the next mode.
        let mut blocked = false;
        if intent.secondary
            && let Some(swing) = w.melee.and_then(|m| m.secondary.clone())
            && w.state.next_secondary <= due
        {
            let swing = follow_up(&swing, w.state.last_swing[1], now);
            let hit = ctx.swing(&swing, true);
            let (own, other) = swing_refire(&swing, hit, now, &mut w.state.last_swing[1]);
            w.state.next_secondary = own;
            w.state.next_primary = other;
            blocked = true;
        } else if intent.secondary
            && let Some(modes) = w.modes.as_ref().map(|m| (**m).clone())
            && w.state.next_secondary <= due
        {
            // Each step sets only the secondary timer (measured M15), but a
            // silencer blocks both (M16).
            w.state.rezoom = None;
            w.state.next_secondary = now + modes.toggle_time as f64;
            if modes.blocks_primary {
                w.state.next_primary = w.state.next_secondary;
            }
            if let Some(s) = modes.sound {
                ctx.w.play.write(owner_sound(s, owner, eye, CHAN_ITEM));
            }
            let next = (modes.current + 1) % modes.count.max(1);
            if let Some(s) = w.sounds {
                let timed = s.modes.iter().filter(|(m, ..)| *m == next);
                w.state.pending = timed.map(|(_, t, e)| (now + *t as f64, e.clone())).collect();
            }
            w.set_mode(next, owner, &mut ctx.w.events);
            blocked = true;
        }

        // 4. Primary attack.
        if intent.fire && !blocked && w.state.next_primary <= due {
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
                } else if let Some(burst) = w.burst.filter(|b| b.mode == w.mode() && b.count > 0).cloned() {
                    // A burst: the first round now, the rest on their own.
                    if w.fire_rounds(&mut ctx, 1) > 0 {
                        w.state.burst_left = burst.count - 1;
                        w.state.next_burst_round = now + burst.interval as f64;
                    }
                    w.state.next_primary = now + burst.refire as f64;
                    w.state.next_secondary = w.state.next_primary;
                } else if let Some(trigger) = w.trigger.cloned() {
                    let mut shots = 0;
                    match trigger.timing {
                        FireTiming::Set => {
                            shots = 1;
                            w.state.next_primary = now + trigger.cycle as f64;
                            w.state.next_secondary = w.state.next_primary;
                        }
                        FireTiming::Accumulate => {
                            while w.state.next_primary <= due {
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
                    let fired = w.fire_rounds(&mut ctx, shots);
                    // A sniper shot unzooms until it can fire again (M15).
                    let mode = w.mode();
                    if fired > 0 && mode > 0 && w.zoom.is_some_and(|z| z.unzoom_after_shot) {
                        w.state.rezoom = Some(mode);
                        w.set_mode(0, owner, &mut ctx.w.events);
                    }
                }
            }
        }

        // 5. Reload key.
        if intent.reload && w.state.next_primary <= due && w.state.reload_end.is_none() {
            try_reload(&mut w, &mut inv, now, owner, &mut ctx.w.events);
            w.state.fire_duration = 0.0;
        }

        // 6. No buttons: automatic reload of an empty clip.
        if !intent.fire && !intent.secondary && !intent.reload {
            w.state.fired_on_empty = false;
            if w.magazine.as_ref().is_some_and(|m| m.clip == 0)
                && w.state.next_primary <= due
                && w.state.next_secondary <= due
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
    // Reloading lowers the scope (UNMEASURED: CS:S's snipers do).
    if w.zoom.is_some() {
        w.state.rezoom = None;
        w.set_mode(0, owner, events);
    }
    w.state.burst_left = 0;
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

/// Characters look through their active weapon's zoom (`Zoomed`) and move
/// at its zoomed speed.
#[allow(clippy::type_complexity)]
fn apply_zoom(
    owners: Query<(Entity, &Inventory, Option<&Zoomed>, Option<&MaxSpeed>)>,
    weapons: Query<(&Weapon, Option<&AltModes>, Option<&Zoom>)>,
    mut commands: Commands,
) {
    for (owner, inv, zoomed, speed) in &owners {
        let Some((weapon, modes, zoom)) = inv.active.and_then(|a| weapons.get(a).ok()) else {
            if zoomed.is_some() {
                commands.entity(owner).remove::<Zoomed>();
            }
            continue;
        };
        let mode = modes.map_or(0, |m| m.current) as usize;
        let now = zoom.filter(|_| mode > 0).and_then(|z| {
            Some((
                Zoomed {
                    fov: *z.fov.get(mode - 1)?,
                    scope: z.scope,
                },
                z.max_speed.or(weapon.max_speed),
            ))
        });
        let want_speed = now.map_or(weapon.max_speed, |(_, s)| s);
        if zoomed != now.as_ref().map(|(z, _)| z) {
            match now {
                Some((z, _)) => commands.entity(owner).insert(z),
                None => commands.entity(owner).remove::<Zoomed>(),
            };
        }
        if speed.map(|s| s.0) != want_speed {
            match want_speed {
                Some(s) => commands.entity(owner).insert(MaxSpeed(s)),
                None => commands.entity(owner).remove::<MaxSpeed>(),
            };
        }
    }
}

/// Play sounds whose time has come (reload parts), from the owner.
/// Each bullet path (a shot and its continuations through walls) is also
/// traced against ragdolls, which it passes through but pushes
/// (specs/cs_source/ragdolls.md 6.2).
fn ragdoll_shots(mut events: MessageReader<WeaponEvent>, mut shots: MessageWriter<crate::map::RagdollShot>) {
    for e in events.read() {
        if let WeaponEventKind::Shot { from, to, .. } | WeaponEventKind::ShotContinued { from, to, .. } = e.kind {
            shots.write(crate::map::RagdollShot { from, to, blast: false });
        }
    }
}

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
