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
pub mod equip;
pub mod grenade;
pub mod lagcomp;
pub mod random;
pub mod remote;
pub mod sync;

use std::sync::Arc;

use bevy::prelude::*;
use serde::{Deserialize, Serialize};
pub use sync::{CarriedAppExt, NetPart};

pub use deliver::{aim_of, armor_split, falloff, hitgroup_at, pellet_dirs, quantize, spread_dir};

use crate::{
    console::{Command, Console, ConsoleAppExt},
    core::{
        FirstTimePredicted, Health, Hitgroup, Intent, LocalPlayer, MaxSpeed, MovementState, Predict, PredictedAppExt, SimClock,
        SimSet, Team, run_predicted,
    },
    map::{PlaySound, interp::NetDrawn},
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

/// Weapon selection (`Inventory::wanted` applied, deploys): brains that
/// pick weapons run before it.
#[derive(SystemSet, Debug, Clone, PartialEq, Eq, Hash)]
pub struct SelectWeapons;

impl Plugin for WeaponPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<WeaponRegistry>()
            .init_resource::<economy::Prices>()
            .init_resource::<economy::BuyWindow>()
            .init_resource::<drop::DeathDrops>()
            .init_resource::<drop::UsePickup>()
            .add_message::<drop::UsedPickup>()
            .register_type::<economy::Money>()
            .register_type::<economy::DefuseKit>()
            .init_resource::<PassMaterials>()
            .init_resource::<StartingWeapons>()
            .init_resource::<equip::SpawnEquipment>()
            .add_message::<crate::core::Equip>()
            .add_message::<WeaponEvent>()
            .add_message::<crate::map::RagdollShot>()
            .add_message::<PlaySound>()
            .add_systems(
                FixedUpdate,
                (
                    (
                        // What a character carries is the server's to change
                        // (a client hears it with its own player's state).
                        (
                            equip::map_equipment,
                            give_starting_weapons,
                            equip::apply_equips,
                            drop::pick_up,
                            drop::use_pick_up,
                        )
                            .chain()
                            .run_if(crate::core::authoritative),
                        run_predicted(Predict::Select).in_set(SelectWeapons),
                    )
                        .chain()
                        .before(SimSet::Movement),
                    drop::drop_on_death
                        .after(SimSet::Weapons)
                        .run_if(crate::core::authoritative),
                    (run_predicted(Predict::Weapons).in_set(WeaponFrame), ragdoll_shots)
                        .chain()
                        .in_set(SimSet::Weapons),
                ),
            )
            // What prediction re-runs (`core::Predict`): selection, and the
            // weapon frame with its timed sounds and zoom.
            .add_systems(Predict::Select, select_weapons.in_set(SelectWeapons))
            .add_systems(
                Predict::Weapons,
                (weapon_frame.in_set(WeaponFrame), timed_sounds, apply_zoom).chain(),
            )
            // What a character carries and its weapons' state go to the
            // client predicting it (`sync`).
            .predicted::<Inventory>()
            .predicted_codec(sync::name(), sync::encode, sync::decode)
            .predicted_part::<WeaponState>()
            .predicted_part::<Magazine>()
            .predicted_part::<AltModes>()
            .predicted::<Hitscan>()
            .predicted_net::<ViewPunch>()
            .predicted_net::<Zoomed>();
        lagcomp::plugin(app);
        remote::plugin(app);
        grenade::plugin(app);
        app.init_resource::<Console>();
        app.world_mut().resource_mut::<Console>().add_command(Command {
            name: "give".into(),
            help: "Give the local player a weapon by ID (e.g. give cs_source:weapon_ak47).".into(),
            run: Arc::new(|w, args| {
                server_only(w)?;
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
        crate::console::resource_cvar::<drop::UsePickup, u8>(
            app,
            "mashup_usepickup",
            "1: +use on a dropped weapon you look at takes it, dropping the one in its slot (CS:GO's; CS:S has none).",
            |u| &mut u.0,
        );
        app.add_message::<drop::DropRequested>();
        app.console_command("drop", "Drop the weapon you hold (G); walk over one to pick it up.", |w, _| {
            let player = local_player(w)?;
            if client(w) {
                // The server drops it; what we carry follows.
                w.write_message(drop::DropRequested);
                return Ok(None);
            }
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
                server_only(w)?;
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
            "buy <weapon>|vest|vesthelm|defuser, e.g. buy ak47 (costs money when you have some).",
            |w, a| {
                let name = a.first().ok_or("buy <weapon>")?.clone();
                server_only(w)?;
                let player = local_player(w)?;
                economy::buy(w, player, &name).map(Some)
            },
        );
        // CS:S's ammo commands (`,` and `.` run buyammo1/2 by default);
        // `buy primammo`/`buy secammo` fill the reserve instead.
        for (name, slot, _) in economy::AMMO_BUYS.into_iter().filter(|(_, _, fill)| !fill) {
            let gun = if slot == 0 { "primary" } else { "secondary" };
            app.console_command(
                name,
                &format!("Buy a box of ammo for your {gun} weapon (in a buy zone, in the buy time)."),
                move |w, _| {
                    server_only(w)?;
                    let player = local_player(w)?;
                    economy::buy(w, player, name).map(Some)
                },
            );
        }
    }
}

/// Whether this is a network client (what it carries is the server's).
fn client(w: &World) -> bool {
    w.get_resource::<crate::core::NetRole>() == Some(&crate::core::NetRole::Client)
}

/// Refused on a network client: only the server changes what players
/// carry.
fn server_only(w: &World) -> Result<(), String> {
    if client(w) {
        return Err("only the server can do that in a network game".into());
    }
    Ok(())
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
    /// The position of the weapon with this exact ID (the same on every
    /// build with the same games: network messages name weapons by it).
    pub fn index(&self, id: &str) -> Option<usize> {
        self.0.iter().position(|d| d.id == id)
    }

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
    world.get_entity(owner).ok()?;
    let weapon = spawn_weapon(world, id, owner)?;
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

/// A new weapon entity `id` (built from the registry) belonging to
/// `owner`, not yet in its inventory.
pub fn spawn_weapon(world: &mut World, id: &str, owner: Entity) -> Option<Entity> {
    let build = world.resource::<WeaponRegistry>().find(id)?.build;
    let mut e = world.spawn((Name::new(id.to_string()), WeaponState::default()));
    build(&mut e);
    let weapon = e.id();
    if let Some(mut w) = world.get_mut::<Weapon>(weapon) {
        w.owner = Some(owner);
    }
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
    let equipment = world.resource::<equip::SpawnEquipment>().0.clone();
    for (owner, team) in new {
        world.entity_mut(owner).insert(Inventory::default());
        // A map's spawn equipment replaces the starting weapons.
        if let Some(items) = &equipment {
            for (name, count) in items {
                equip::give_item(world, owner, name, *count);
            }
            continue;
        }
        for id in start.for_team(team) {
            give(world, owner, id);
        }
    }
}

// ---------------------------------------------------------------------------
// Components

/// What a character carries, and the player-level weapon timers.
#[derive(Component, Default, Clone, Debug)]
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
    /// Last tick's use key (`drop::use_pick_up`).
    prev_use: bool,
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

/// Magazine add-on: rounds go in one at a time (shotguns; spec 3.4,
/// "reload one at a time", the CS-derived form). A reload first takes
/// `start` (attacks wait for it), then each round `insert`, the next one
/// starting a tick after the last went in, until the clip is full or the
/// reserve empty. Firing with rounds in the clip stops it; switching away
/// too. Reload sounds (`WeaponSounds::reload`) play from each insert.
#[derive(Component, Clone, Debug)]
pub struct ShellReload {
    pub start: f32,
    pub insert: f32,
}

/// Where a one-at-a-time reload is (`ShellReload`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum ShellStage {
    #[default]
    Idle,
    /// The start sequence (or the tick between two inserts) until
    /// `WeaponState::shell_next`.
    Starting,
    /// A round going in, in at `shell_next`.
    Inserting,
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
#[derive(Component, Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
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
#[derive(Component, Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
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
    /// Entries at times after each shot (a bolt worked by the fire
    /// animation).
    pub shot: Vec<(f32, String)>,
    /// Entries at times after a one-at-a-time reload (`ShellReload`) ends.
    pub finish: Vec<(f32, String)>,
    /// (mode, time, entry): played that long after stepping to that
    /// `AltModes` mode with attack2 (e.g. the silencer going on or off).
    pub modes: Vec<(u8, f32, String)>,
}

/// Source's per-weapon timers and flags.
#[derive(Component, Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
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
    /// A one-at-a-time reload (`ShellReload`) and when its stage ends.
    pub shells: ShellStage,
    pub shell_next: f64,
}

impl WeaponState {
    /// A reload of either kind is in progress.
    pub fn reloading(&self) -> bool {
        self.reload_end.is_some() || self.shells != ShellStage::Idle
    }
}

/// What weapons did this tick, for HUDs, effects and tests.
#[derive(Message, Clone, Debug)]
pub struct WeaponEvent {
    pub owner: Entity,
    pub weapon: Entity,
    pub kind: WeaponEventKind,
    /// Written while a network client re-ran a command after a correction
    /// (`core::FirstTimePredicted` false): what the weapon does again
    /// (recoil reads it), not something to show again. Effects, sounds,
    /// HUDs and animation skip these.
    pub replay: bool,
}

impl WeaponEvent {
    /// Something to show (not a replay; `replay`).
    pub fn shown(&self) -> bool {
        !self.replay
    }
}

#[derive(Clone, Debug)]
pub enum WeaponEventKind {
    Deployed,
    /// One round of a hitscan weapon left the barrel (before its traces,
    /// one `Shot` per pellet): from `origin` along `aim_of(yaw, pitch)`
    /// with spread seed `seed` and this spread, in `AltModes` mode `mode`.
    /// Enough to draw the same shot again elsewhere (`pellet_dirs`): a
    /// network server sends it to the other clients.
    Fired {
        origin: Vec3,
        yaw: f32,
        pitch: f32,
        seed: u32,
        spread: SpreadShape,
        mode: u8,
    },
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
        /// The swing's line trace: from the eye to where it stopped (the
        /// range's end when it hit nothing).
        line: (Vec3, Vec3),
    },
    DryFire,
    /// The weapon stepped to this `AltModes` mode (attack2, or a sniper
    /// shot unzooming and re-zooming).
    ModeChanged {
        mode: u8,
    },
    ReloadStarted,
    /// One round started going in (`ShellReload`).
    ShellInserting,
    /// A reload finished (the magazine swapped, or the last round in).
    Reloaded,
    /// A grenade's pin came out (`grenade::Throwable`).
    PinPulled,
    /// A grenade was released (it leaves the hand a moment later).
    Thrown,
    /// Arming a bomb began (the press-button animation, the third-person
    /// gesture): `objectives::bomb`.
    ArmingStarted,
    /// Arming stopped short (let go, moved off): back to idle.
    ArmingStopped,
}

// ---------------------------------------------------------------------------
// Selection and deploy

fn select_weapons(
    mut owners: Query<(Entity, &Intent, &mut Inventory, &Transform), Without<NetDrawn>>,
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
    clock: Res<SimClock>,
    first: Res<FirstTimePredicted>,
) {
    let now = clock.now;
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
            st.shells = ShellStage::Idle;
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
        if let Some(s) = sounds.and_then(|s| s.deploy.clone())
            && first.0
        {
            play.write(owner_sound(s, owner, at.translation, CHAN_ITEM));
        }
        if let Some(s) = sounds {
            st.pending = s.draw.iter().map(|(t, e)| (now + *t as f64, e.clone())).collect();
        }
        events.write(WeaponEvent {
            owner,
            weapon: want,
            kind: WeaponEventKind::Deployed,
            replay: !first.0,
        });
    }
}

/// A sound from the weapon's owner at `at`.
fn owner_sound(entry: String, owner: Entity, at: Vec3, channel: u8) -> PlaySound {
    PlaySound {
        pitch: None,
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
    shells: Option<&'static ShellReload>,
}

impl WeaponPartsItem<'_, '_> {
    fn mode(&self) -> u8 {
        self.modes.as_ref().map_or(0, |m| m.current)
    }

    /// Change mode, telling the HUD and view model.
    fn set_mode(&mut self, mode: u8, owner: Entity, w: &mut deliver::World) {
        let Some(m) = self.modes.as_mut() else { return };
        if m.current == mode {
            return;
        }
        m.current = mode;
        w.event(owner, self.entity, WeaponEventKind::ModeChanged { mode });
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
                let fired = WeaponEventKind::Fired {
                    origin: ctx.eye,
                    yaw: ctx.yaw,
                    pitch: ctx.pitch,
                    seed: ctx.seed,
                    spread: scan.spread,
                    mode: self.mode(),
                };
                ctx.w.event(ctx.owner, self.entity, fired);
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
                ctx.w.sound(owner_sound(s, ctx.owner, ctx.eye, CHAN_WEAPON));
            }
        }
        self.state.burst += shots;
        if shots > 0
            && let Some(s) = self.sounds.filter(|s| !s.shot.is_empty())
        {
            let now = ctx.now;
            self.state.pending = s.shot.iter().map(|(t, e)| (now + *t as f64, e.clone())).collect();
        }
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
        Option<&lagcomp::ViewTick>,
    ), Without<NetDrawn>>,
    mut weapons: Query<WeaponParts>,
    mut world: deliver::World,
    clock: Res<SimClock>,
    (histories, lag, mut lag_stats): (
        Query<(Entity, &lagcomp::HitHistory, &Health)>,
        Res<lagcomp::LagCompSettings>,
        ResMut<lagcomp::LagCompStats>,
    ),
) {
    // The command's time (Source's curtime).
    let now = clock.now;
    // Compare timers against this; record new ones from `now`.
    let due = now + TIME_SLACK;
    let dt = clock.dt();
    for (owner, intent, mut inv, transform, state, health, punch, view) in &mut owners {
        if health.is_some_and(|h| h.current <= 0.0) {
            continue;
        }
        let pressed = intent.fire && !inv.prev_fire;
        let released2 = !intent.secondary && inv.prev_secondary;
        inv.prev_fire = intent.fire;
        inv.prev_secondary = intent.secondary;
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
            w.set_mode(mode, owner, &mut world);
        }
        // Busy (drawing, reloading): only the timers run.
        if due < inv.next_attack {
            continue;
        }
        let eye = transform.translation + state.eye_offset;
        // Shots follow the view plus the weapon's share of the recoil.
        let kick = punch.map_or(Vec2::ZERO, |p| p.0) * w.hitscan.map_or(0.0, |h| h.punch_scale);
        let (yaw, pitch) = (intent.yaw + kick.y, intent.pitch + kick.x);
        // A remote player's shots and swings hit others where it saw them
        // (lag compensation, server only: `ViewTick` is set per command).
        let may_trace = (intent.fire && w.state.next_primary <= due)
            || (intent.secondary && w.melee.is_some() && w.state.next_secondary <= due)
            || (w.state.burst_left > 0 && due >= w.state.next_burst_round);
        let rewound = match view.and_then(|v| v.0) {
            Some(view_tick) if lag.unlag != 0 && may_trace && world.authoritative() => {
                lagcomp::rewind(
                    owner,
                    view_tick,
                    clock.tick,
                    clock.delta.as_secs_f64(),
                    &lag,
                    histories
                        .iter()
                        .filter(|(_, _, h)| h.current > 0.0)
                        .map(|(e, history, _)| (e, history)),
                    &mut lag_stats,
                )
            }
            _ => Vec::new(),
        };
        let mut ctx = deliver::Shot {
            owner,
            weapon: active,
            eye,
            aim: deliver::aim_of(yaw, pitch),
            yaw,
            pitch,
            // Spec 4.2: from the command number.
            seed: random::shot_seed(intent.command),
            now,
            rewound,
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
            ctx.w.event(owner, active, WeaponEventKind::Reloaded);
        }

        // One-at-a-time reloads step on (spec 3.4; T25): the start, then a
        // round per insert, the next insert a tick after.
        if w.state.shells != ShellStage::Idle && due >= w.state.shell_next {
            let insert = w.shells.map_or(0.0, |s| s.insert) as f64;
            let room = w.magazine.as_ref().is_some_and(|m| m.clip < m.size && m.reserve > 0);
            match w.state.shells {
                ShellStage::Starting if room => {
                    w.state.shells = ShellStage::Inserting;
                    w.state.shell_next = now + insert;
                    if let Some(s) = w.sounds {
                        w.state.pending = s.reload.iter().map(|(t, e)| (now + *t as f64, e.clone())).collect();
                    }
                    ctx.w.event(owner, active, WeaponEventKind::ShellInserting);
                }
                ShellStage::Inserting => {
                    if let Some(mag) = w.magazine.as_mut()
                        && mag.reserve > 0
                        && mag.clip < mag.size
                    {
                        mag.clip += 1;
                        mag.reserve -= 1;
                    }
                    w.state.shells = ShellStage::Starting;
                    w.state.shell_next = now;
                    if !w.magazine.as_ref().is_some_and(|m| m.clip < m.size && m.reserve > 0) {
                        w.state.shells = ShellStage::Idle;
                        if let Some(s) = w.sounds {
                            w.state.pending = s.finish.iter().map(|(t, e)| (now + *t as f64, e.clone())).collect();
                        }
                        ctx.w.event(owner, active, WeaponEventKind::Reloaded);
                    }
                }
                _ => {
                    w.state.shells = ShellStage::Idle;
                    ctx.w.event(owner, active, WeaponEventKind::Reloaded);
                }
            }
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
                ctx.w.sound(owner_sound(s, owner, eye, CHAN_ITEM));
            }
            let next = (modes.current + 1) % modes.count.max(1);
            if let Some(s) = w.sounds {
                let timed = s.modes.iter().filter(|(m, ..)| *m == next);
                w.state.pending = timed.map(|(_, t, e)| (now + *t as f64, e.clone())).collect();
            }
            w.set_mode(next, owner, ctx.w);
            blocked = true;
        }

        // 4. Primary attack.
        if intent.fire && !blocked && w.state.next_primary <= due {
            let automatic = w.trigger.is_none_or(|t| t.automatic);
            let empty = w.magazine.as_ref().is_some_and(|m| m.clip == 0);
            if !automatic && w.state.burst > 0 {
                // Semi-automatic: wait for a fresh press.
            } else if w.state.shells != ShellStage::Idle && empty {
                // Loading the first round: the reload goes on.
            } else if w.magazine.as_ref().is_some_and(|m| m.clip == 0) {
                if w.state.fired_on_empty {
                    if w.magazine.as_ref().is_some_and(|m| m.reload_while_held) {
                        try_reload(&mut w, &mut inv, now, owner, ctx.w);
                    }
                } else {
                    // Fire on empty (spec 3.5).
                    w.state.fired_on_empty = true;
                    if now >= w.state.next_empty_sound {
                        w.state.next_empty_sound = now + EMPTY_SOUND_INTERVAL;
                        if let Some(s) = w.sounds.and_then(|s| s.empty.clone()) {
                            ctx.w.sound(owner_sound(s, owner, eye, CHAN_ITEM));
                        }
                    }
                    w.state.next_primary = now + 0.2;
                    ctx.w.event(owner, active, WeaponEventKind::DryFire);
                }
            } else {
                if pressed || released2 {
                    // A fresh press drops any accumulated lag (spec 3.2).
                    w.state.next_primary = now;
                }
                // A shot stops a one-at-a-time reload.
                w.state.shells = ShellStage::Idle;
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
                        w.set_mode(0, owner, ctx.w);
                    }
                }
            }
        }

        // 5. Reload key.
        if intent.reload && w.state.next_primary <= due && !w.state.reloading() {
            try_reload(&mut w, &mut inv, now, owner, ctx.w);
            w.state.fire_duration = 0.0;
        }

        // 6. No buttons: automatic reload of an empty clip.
        if !intent.fire && !intent.secondary && !intent.reload {
            w.state.fired_on_empty = false;
            if w.magazine.as_ref().is_some_and(|m| m.clip == 0)
                && w.state.next_primary <= due
                && w.state.next_secondary <= due
                && !w.state.reloading()
            {
                try_reload(&mut w, &mut inv, now, owner, ctx.w);
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
fn try_reload(w: &mut WeaponPartsItem, inv: &mut Inventory, now: f64, owner: Entity, events: &mut deliver::World) {
    let Some(mag) = w.magazine.as_ref() else { return };
    if (mag.size - mag.clip).min(mag.reserve) == 0 {
        return;
    }
    let reload_time = mag.reload_time;
    // Reloading lowers the scope (UNMEASURED: CS:S's snipers do).
    if w.zoom.is_some() {
        w.state.rezoom = None;
        w.set_mode(0, owner, events);
    }
    w.state.burst_left = 0;
    w.state.fired_on_empty = false;
    if let Some(shells) = w.shells {
        // One at a time: attacks wait for the start only (spec 3.4).
        let end = now + shells.start as f64;
        w.state.shells = ShellStage::Starting;
        w.state.shell_next = end;
        w.state.next_primary = end;
        w.state.next_secondary = end;
        events.event(owner, w.entity, WeaponEventKind::ReloadStarted);
        return;
    }
    let end = now + reload_time as f64;
    inv.next_attack = end;
    w.state.next_primary = end;
    w.state.next_secondary = end;
    w.state.reload_end = Some(end);
    if let Some(s) = w.sounds {
        w.state.pending = s.reload.iter().map(|(t, e)| (now + *t as f64, e.clone())).collect();
    }
    events.event(owner, w.entity, WeaponEventKind::ReloadStarted);
}

/// Characters look through their active weapon's zoom (`Zoomed`) and move
/// at its zoomed speed.
#[allow(clippy::type_complexity)]
fn apply_zoom(
    owners: Query<(Entity, &Inventory, Option<&Zoomed>, Option<&MaxSpeed>), Without<NetDrawn>>,
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
    for e in events.read().filter(|e| e.shown()) {
        if let WeaponEventKind::Shot { from, to, .. } | WeaponEventKind::ShotContinued { from, to, .. } = e.kind {
            shots.write(crate::map::RagdollShot::bullet(from, to));
        }
    }
}

fn timed_sounds(
    mut weapons: Query<(&Weapon, &mut WeaponState)>,
    owners: Query<&Transform>,
    mut play: MessageWriter<PlaySound>,
    clock: Res<SimClock>,
    first: Res<FirstTimePredicted>,
) {
    let now = clock.now;
    for (w, mut st) in &mut weapons {
        let Some(owner) = w.owner else { continue };
        if st.pending.is_empty() {
            continue;
        }
        let at = owners.get(owner).map_or(Vec3::ZERO, |t| t.translation);
        st.pending.retain(|(t, entry)| {
            if *t <= now {
                if first.0 {
                    play.write(owner_sound(entry.clone(), owner, at, CHAN_ITEM));
                }
                false
            } else {
                true
            }
        });
    }
}
