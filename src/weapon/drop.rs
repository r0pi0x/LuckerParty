//! Dropping weapons (Source's `drop`, G in CS:S) and picking them up by
//! walking over them. A dropped weapon keeps its entity (and so its ammo);
//! a separate loose entity (`map::loose::LooseItem`) lies in the world
//! and points back at it. With `mashup_usepickup 1`, +use on a loose
//! weapon in view swaps it for the one in its slot (CS:GO's rule; CS:S
//! has only the walk-over pickup).

use avian3d::prelude::{AngularVelocity, LinearVelocity};
use bevy::prelude::*;

use super::{Inventory, Weapon, Zoomed};
use crate::{
    core::{Died, Health, Intent, Velocity},
    map::loose::LooseItem,
};

/// A weapon that can't be dropped (CS:S's knife).
#[derive(Component, Clone, Copy, Debug)]
pub struct Undroppable;

/// Only characters of this team may pick the weapon up (CS:S's C4: the
/// terrorists).
#[derive(Component, Clone, Copy, Debug, PartialEq, Eq)]
pub struct PickupTeam(pub crate::core::Team);

/// A weapon lying in the world: the weapon entity, who dropped it and when.
#[derive(Component, Clone, Copy, Debug)]
pub struct Loose {
    pub weapon: Entity,
    pub dropper: Option<Entity>,
    pub since: f64,
}

/// Throw speed forward and up, m/s (a guess, not measured).
const THROW_FORWARD: f32 = 4.0;
const THROW_UP: f32 = 1.0;
/// Seconds after a drop before anyone can pick the weapon up
/// (specs/cs_source/weapons.md 3.9).
pub const TOUCH_DELAY: f64 = 1.0;
/// The touch volume: the weapon's box grown this much on each horizontal
/// side and on top, none below (36 and 18 units; spec 3.9).
const GROW_SIDES: f32 = 36.0 * 0.0254;
const GROW_TOP: f32 = 18.0 * 0.0254;
/// A weapon's half size when it has no body yet (headless): meters.
const DEFAULT_HALF: Vec3 = Vec3::splat(0.1);
/// A character's box when its movement publishes none (the placeholder
/// capsule's: 0.8 m wide, 1.8 m tall, origin at its centre).
const DEFAULT_HULL: (Vec3, Vec3) = (Vec3::new(-0.4, -0.9, -0.4), Vec3::new(0.4, 0.9, 0.4));
/// Loose weapons kept at most; the oldest go first.
const MAX_LOOSE: usize = 32;
/// +use pickup reach from the eye to the weapon's centre (100 units: past
/// the touch box's 36, as CS:GO's use reach; a guess, not measured).
pub const USE_REACH: f32 = 100.0 * 0.0254;
/// How far the view line may pass from a weapon's centre beyond its own
/// half size and still take it (12 units; a guess).
const USE_SLACK: f32 = 12.0 * 0.0254;

/// `mashup_usepickup`: 1 lets +use take the loose weapon looked at
/// (default 0: CS:S's walk-over pickup only).
#[derive(Resource, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct UsePickup(pub u8);

/// The local player asked to drop what it holds on a network client: the
/// network layer asks the server (which drops it).
#[derive(Message, Clone, Copy, Debug, Default)]
pub struct DropRequested;

/// A character took a loose weapon with +use (so the use key found
/// something: no deny sound).
#[derive(Message, Clone, Copy, Debug)]
pub struct UsedPickup {
    pub who: Entity,
    pub weapon: Entity,
}

/// The weapon `owner` would drop: the active one if droppable, else (for
/// the dead) the droppable one in the lowest slot.
fn droppable(world: &World, owner: Entity, active_only: bool) -> Option<Entity> {
    let inv = world.get::<Inventory>(owner)?;
    let ok = |e: &Entity| world.get::<Weapon>(*e).is_some() && world.get::<Undroppable>(*e).is_none();
    if let Some(a) = inv.active.filter(ok) {
        return Some(a);
    }
    if active_only {
        return None;
    }
    inv.weapons
        .iter()
        .copied()
        .filter(ok)
        .min_by_key(|e| world.get::<Weapon>(*e).map_or(u8::MAX, |w| w.slot))
}

/// Drop `owner`'s active weapon (or for the dead, its best one), thrown
/// along its view. Returns the loose entity.
pub fn drop_weapon(world: &mut World, owner: Entity, thrown: bool) -> Option<Entity> {
    let weapon = droppable(world, owner, thrown)?;
    drop_this(world, owner, weapon, thrown)
}

/// Now on the tick's clock (`Time<Fixed>`, which `pick_up` reads as
/// `Time`), wherever it's asked from.
pub(super) fn tick_time(world: &World) -> f64 {
    world
        .get_resource::<Time<Fixed>>()
        .map_or_else(|| world.resource::<Time>().elapsed_secs_f64(), Time::elapsed_secs_f64)
}

/// Drop one weapon `owner` carries (thrown along the view, or let fall).
pub fn drop_this(world: &mut World, owner: Entity, weapon: Entity, thrown: bool) -> Option<Entity> {
    // The tick's clock, which `pick_up` reads: `drop` runs from the
    // console (a frame, where `Time` is the virtual clock).
    let now = tick_time(world);
    let at = world.get::<Transform>(owner)?.translation;
    let look = world.get::<Intent>(owner).map_or(Quat::IDENTITY, Intent::look_rotation);
    let carried = world.get::<Velocity>(owner).map_or(Vec3::ZERO, |v| v.0);
    let id = world.get::<Weapon>(weapon)?.id;
    let dead = world.get::<Health>(owner).is_some_and(|h| h.current <= 0.0);
    // Draw the best of what's left (the lowest slot), as CS:S does.
    let next = world.get::<Inventory>(owner).and_then(|inv| {
        inv.weapons
            .iter()
            .copied()
            .filter(|w| *w != weapon)
            .min_by_key(|e| world.get::<Weapon>(*e).map_or(u8::MAX, |w| w.slot))
    });
    {
        let mut inv = world.get_mut::<Inventory>(owner)?;
        inv.weapons.retain(|w| *w != weapon);
        if inv.last == Some(weapon) {
            inv.last = None;
        }
        if inv.active == Some(weapon) {
            inv.active = None;
            // Unless a switch is already pending (a bought replacement), or
            // the owner is dead (no draw, and no draw sound, for a body).
            if inv.wanted.is_none_or(|w| w == weapon) && !dead {
                inv.wanted = next;
            }
        }
    }
    world.entity_mut(owner).remove::<Zoomed>();
    if let Some(mut w) = world.get_mut::<Weapon>(weapon) {
        w.owner = None;
    }
    let forward = look * Vec3::NEG_Z;
    let (start, velocity) = if thrown {
        (
            at + Vec3::Y * 0.4 + forward.with_y(0.0).normalize_or_zero() * 0.3,
            carried + forward * THROW_FORWARD + Vec3::Y * THROW_UP,
        )
    } else {
        (at, carried)
    };
    let yaw = Quat::from_rotation_y(look.to_euler(EulerRot::YXZ).0);
    let loose = world
        .spawn((
            Name::new(format!("Loose {id}")),
            LooseItem(id.to_string()),
            Loose {
                weapon,
                dropper: Some(owner),
                since: now,
            },
            Transform::from_translation(start).with_rotation(yaw),
            LinearVelocity(velocity),
            AngularVelocity(yaw * Vec3::new(0.0, 2.0, 4.0)),
        ))
        .id();
    // Too many lying around: the oldest go.
    let mut all: Vec<(Entity, f64, Entity)> = world
        .query_filtered::<(Entity, &Loose), Without<super::equip::MapWeapon>>()
        .iter(world)
        .map(|(e, l)| (e, l.since, l.weapon))
        .collect();
    if all.len() > MAX_LOOSE {
        all.sort_by(|a, b| a.1.total_cmp(&b.1));
        let over = all.len() - MAX_LOOSE;
        for (e, _, w) in all.into_iter().take(over) {
            world.despawn(e);
            world.despawn(w);
        }
    }
    Some(loose)
}

/// Whether the dead drop their best weapon: in CS:S rounds they do; in
/// deathmatch (respawning with fresh weapons) they don't. The rules set it.
#[derive(Resource, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DeathDrops(pub bool);

/// The dead drop their best weapon where they fell (when `DeathDrops`).
pub(super) fn drop_on_death(mut died: MessageReader<Died>, drops: Option<Res<DeathDrops>>, mut commands: Commands) {
    if !drops.is_some_and(|d| d.0) {
        died.clear();
        return;
    }
    for d in died.read() {
        let owner = d.entity;
        commands.queue(move |w: &mut World| {
            drop_weapon(w, owner, false);
        });
    }
}

/// The world box of a loose weapon's touch volume (spec 3.9): its box,
/// turned with it, grown 36 units on each horizontal side and 18 on top.
pub fn touch_box(at: &Transform, half: Vec3) -> (Vec3, Vec3) {
    let m = Mat3::from_quat(at.rotation);
    let ext = m.x_axis.abs() * half.x + m.y_axis.abs() * half.y + m.z_axis.abs() * half.z;
    let grow_lo = Vec3::new(GROW_SIDES, 0.0, GROW_SIDES);
    let grow_hi = Vec3::new(GROW_SIDES, GROW_TOP, GROW_SIDES);
    (at.translation - ext - grow_lo, at.translation + ext + grow_hi)
}

/// What touching a loose weapon does for a character (spec 3.9).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Touch {
    /// Nothing: that slot holds this other weapon.
    Refused(Entity),
    /// Take the weapon.
    Equip,
    /// The same weapon type is carried: take only its ammo, into this one.
    Ammo(Entity),
}

/// Why a character does or doesn't take a loose weapon this tick (the
/// walk-over rules in order; `mashup_debug_pickup` prints it).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Verdict {
    /// Taken.
    Take,
    /// The same type is carried (this one): only its ammo is taken.
    Ammo(Entity),
    /// Dropped this long ago (s), under `TOUCH_DELAY`.
    TooSoon(f64),
    /// This far (m) outside the touch box.
    OutOfReach(f32),
    /// Only another team takes it (the C4).
    WrongTeam,
    /// That slot holds this weapon (CS:S takes a gun only into an empty
    /// slot: drop that one first).
    SlotFull(Entity),
    /// Solid geometry (this entity) lies between the eye and its centre.
    Blocked(Entity),
}

type Owners<'w, 's> = Query<
    'w,
    's,
    (
        Entity,
        &'static Transform,
        &'static Inventory,
        &'static Health,
        Option<&'static crate::core::Team>,
        Option<&'static crate::core::MovementState>,
    ),
>;

type LooseItems<'w, 's> =
    Query<'w, 's, (Entity, &'static Loose, &'static Transform, Option<&'static crate::map::PhysicsProp>)>;

/// A character as the walk-over pickup sees it: its box, eye, team and
/// what it carries.
struct Toucher<'a> {
    lo: Vec3,
    hi: Vec3,
    eye: Vec3,
    team: Option<&'a crate::core::Team>,
    inv: &'a Inventory,
}

impl<'a> Toucher<'a> {
    fn new(
        at: &Transform,
        inv: &'a Inventory,
        team: Option<&'a crate::core::Team>,
        state: Option<&crate::core::MovementState>,
    ) -> Self {
        let (lo, hi) = match state {
            Some(s) if s.hull_max != s.hull_min => (s.hull_min, s.hull_max),
            _ => DEFAULT_HULL,
        };
        Self {
            lo: at.translation + lo,
            hi: at.translation + hi,
            eye: at.translation + state.map_or(Vec3::ZERO, |s| s.eye_offset),
            team,
            inv,
        }
    }

    /// How far (m) its box is from a loose weapon's touch box (0: they
    /// overlap).
    fn gap(&self, item_at: &Transform, body: Option<&crate::map::PhysicsProp>) -> f32 {
        let (t_lo, t_hi) = touch_box(item_at, half_of(body));
        (t_lo - self.hi).max(self.lo - t_hi).max(Vec3::ZERO).length()
    }
}

/// A loose weapon's half size (its body's box).
fn half_of(body: Option<&crate::map::PhysicsProp>) -> Vec3 {
    body.map_or(DEFAULT_HALF, |b| (b.bounds.1 - b.bounds.0) / 2.0)
}

/// The walk-over rules for one character and one loose weapon (spec 3.9),
/// cheapest first: the touch delay, the touch box, the team, the slot,
/// then line of sight. None: the loose entity's weapon is gone.
#[allow(clippy::too_many_arguments)]
fn verdict(
    me: &Toucher,
    l: &Loose,
    item: Entity,
    item_at: &Transform,
    body: Option<&crate::map::PhysicsProp>,
    weapons: &Query<(&Weapon, Option<&PickupTeam>)>,
    spatial: &avian3d::prelude::SpatialQuery,
    now: f64,
    character: impl Fn(Entity) -> bool,
) -> Option<Verdict> {
    if now - l.since < TOUCH_DELAY {
        return Some(Verdict::TooSoon(now - l.since));
    }
    let gap = me.gap(item_at, body);
    if gap > 0.0 {
        return Some(Verdict::OutOfReach(gap));
    }
    let (weapon, only) = weapons.get(l.weapon).ok()?;
    if only.is_some_and(|t| me.team != Some(&t.0)) {
        return Some(Verdict::WrongTeam);
    }
    let touch = touch_of(weapon, me.inv, weapons);
    if let Touch::Refused(mine) = touch {
        return Some(Verdict::SlotFull(mine));
    }
    if let Some(by) = blocker(spatial, me.eye, item_at.translation, item, character) {
        return Some(Verdict::Blocked(by));
    }
    Some(match touch {
        Touch::Ammo(mine) => Verdict::Ammo(mine),
        _ => Verdict::Take,
    })
}

/// Living characters touching a loose weapon (spec 3.9): its box grown
/// 36 units sideways and 18 up, touchable 1 s after the drop, seen from
/// the eye (solid geometry and windows block). Taken when nothing is in
/// its slot; when the same type is carried, only its ammo is taken.
pub(super) fn pick_up(
    loose: LooseItems,
    owners: Owners,
    weapons: Query<(&Weapon, Option<&PickupTeam>)>,
    spatial: avian3d::prelude::SpatialQuery,
    time: Res<Time>,
    mut commands: Commands,
) {
    let now = time.elapsed_secs_f64();
    let mut taken = Vec::new();
    for (owner, at, inv, health, team, state) in &owners {
        if health.current <= 0.0 {
            continue;
        }
        let me = Toucher::new(at, inv, team, state);
        for (item, l, item_at, body) in &loose {
            if taken.contains(&item) {
                continue;
            }
            let v = verdict(&me, l, item, item_at, body, &weapons, &spatial, now, |c| owners.contains(c));
            let weapon = l.weapon;
            match v {
                Some(Verdict::Take) => {
                    taken.push(item);
                    commands.entity(item).despawn();
                    commands.queue(move |w: &mut World| take(w, owner, weapon));
                    break;
                }
                Some(Verdict::Ammo(mine)) => {
                    commands.queue(move |w: &mut World| take_ammo(w, item, weapon, mine));
                }
                _ => {}
            }
        }
    }
}

/// `mashup_debug_pickup`: 1 prints, for the local player, why the nearest
/// loose weapon (within `DEBUG_RANGE` of the touch box) is or isn't
/// taken, each time that changes; 2 for every character (a server's
/// remote players and bots too). Pickups are the server's: on a network
/// client, set it on the server.
#[derive(Resource, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DebugPickup(pub u8);

/// How far (m, from a character's box to a touch box) the readout looks
/// for a loose weapon.
const DEBUG_RANGE: f32 = 3.0;

/// What `debug_pickups` last said per character: the loose entity it was
/// about, that weapon, and the reason's kind (a distance changing every
/// tick doesn't print again).
#[derive(Resource, Default)]
pub(super) struct DebugPickupSeen(std::collections::HashMap<Entity, (Option<Entity>, Option<Entity>, &'static str)>);

/// The `mashup_debug_pickup` readout: one line per character each time
/// its nearest loose weapon or the reason changes, and when it takes one.
#[allow(clippy::too_many_arguments)]
pub(super) fn debug_pickups(
    setting: Res<DebugPickup>,
    mut seen: ResMut<DebugPickupSeen>,
    loose: LooseItems,
    owners: Owners,
    local: Query<(), With<crate::core::LocalPlayer>>,
    weapons: Query<(&Weapon, Option<&PickupTeam>)>,
    names: Query<&Name>,
    spatial: avian3d::prelude::SpatialQuery,
    time: Res<Time>,
    mut console: Option<ResMut<crate::console::Console>>,
) {
    if setting.0 == 0 {
        if !seen.0.is_empty() {
            seen.0.clear();
        }
        return;
    }
    let now = time.elapsed_secs_f64();
    let id_of = |w: Entity| weapons.get(w).map_or("?", |(w, _)| w.id);
    for (owner, at, inv, health, team, state) in &owners {
        if setting.0 < 2 && !local.contains(owner) {
            continue;
        }
        let me = Toucher::new(at, inv, team, state);
        let nearest = loose
            .iter()
            .map(|(e, l, t, b)| (me.gap(t, b), e, l, t, b))
            .filter(|(gap, ..)| *gap <= DEBUG_RANGE)
            .min_by(|a, b| a.0.total_cmp(&b.0));
        let (item, weapon) = nearest.map_or((None, None), |(_, e, l, ..)| (Some(e), Some(l.weapon)));
        let before = seen.0.get(&owner).copied();
        // The weapon it was about is now its own: taken (the loose entity
        // went with the pickup's commands).
        let took = before.and_then(|b| b.1).filter(|w| inv.weapons.contains(w) && item != before.and_then(|b| b.0));
        let (kind, text) = if let Some(w) = took {
            ("taken", format!("{}: taken", id_of(w)))
        } else if health.current <= 0.0 {
            ("dead", "dead, takes nothing".to_string())
        } else if let Some((_, item, l, t, b)) = nearest {
            let what = id_of(l.weapon);
            match verdict(&me, l, item, t, b, &weapons, &spatial, now, |c| owners.contains(c)) {
                None => ("gone", format!("{what}: its weapon is gone")),
                Some(Verdict::Take) => ("take", format!("{what}: taking it")),
                Some(Verdict::Ammo(_)) => ("ammo", format!("{what}: same weapon carried, takes only its ammo")),
                Some(Verdict::TooSoon(s)) => (
                    "soon",
                    format!("{what}: dropped {s:.1} s ago, touchable from {TOUCH_DELAY} s"),
                ),
                Some(Verdict::OutOfReach(m)) => ("reach", format!("{what}: out of reach, {m:.2} m from its touch box")),
                Some(Verdict::WrongTeam) => ("team", format!("{what}: only the other team takes it")),
                Some(Verdict::SlotFull(mine)) => (
                    "slot",
                    format!(
                        "{what}: slot full ({} carried; a gun is taken only into an empty slot, drop that first)",
                        id_of(mine)
                    ),
                ),
                Some(Verdict::Blocked(by)) => {
                    let name = names.get(by).map_or_else(|_| format!("{by}"), |n| n.to_string());
                    ("sight", format!("{what}: not in sight from the eye, blocked by {name}"))
                }
            }
        } else {
            ("none", format!("no loose weapon within {DEBUG_RANGE} m"))
        };
        let key = (item, weapon, kind);
        if before == Some(key) || (kind == "none" && before.is_some_and(|b| b.2 == "taken")) {
            seen.0.insert(owner, key);
            continue;
        }
        seen.0.insert(owner, key);
        let who = names.get(owner).map_or_else(|_| format!("{owner}"), |n| n.to_string());
        let line = format!("pickup {who}: {text}");
        info!("{line}");
        if let Some(c) = console.as_mut() {
            c.info(line);
        }
    }
}

/// The loose weapon a +use press from `eye` along `aim` takes: within
/// `USE_REACH` of the eye, ahead, the view line passing within its half
/// size plus `USE_SLACK` of its centre; the one nearest the line (by
/// angle) wins.
pub fn use_target(eye: Vec3, aim: Vec3, items: impl Iterator<Item = (Entity, Vec3, f32)>) -> Option<Entity> {
    let mut best: Option<(f32, Entity)> = None;
    for (item, centre, half) in items {
        let to = centre - eye;
        let along = to.dot(aim);
        if along <= 0.0 || to.length() > USE_REACH {
            continue;
        }
        let off = (to - aim * along).length();
        if off > half + USE_SLACK {
            continue;
        }
        let score = off / along;
        if best.is_none_or(|(b, _)| score < b) {
            best = Some((score, item));
        }
    }
    best.map(|(_, e)| e)
}

/// +use on a loose weapon (`mashup_usepickup 1`; CS:GO's rule, CS:S has
/// none): on the press, the weapon looked at (`use_target`), touchable
/// (1 s after its drop), seen from the eye and allowed to the team, is
/// taken; whatever the character carries in its slot (the held one if it
/// is there, else the first) is dropped, thrown as with `drop`. The taken
/// weapon is drawn when the dropped one was held.
#[allow(clippy::type_complexity, clippy::too_many_arguments)]
pub(super) fn use_pick_up(
    setting: Res<UsePickup>,
    loose: Query<(Entity, &Loose, &Transform, Option<&crate::map::PhysicsProp>)>,
    mut users: Query<(
        Entity,
        &Transform,
        &Health,
        Option<&crate::core::Team>,
        Option<&crate::core::MovementState>,
        &Intent,
        &mut Inventory,
    )>,
    characters: Query<(), With<Inventory>>,
    weapons: Query<(&Weapon, Option<&PickupTeam>)>,
    spatial: avian3d::prelude::SpatialQuery,
    time: Res<Time>,
    mut used: MessageWriter<UsedPickup>,
    mut commands: Commands,
) {
    let now = time.elapsed_secs_f64();
    let mut taken = Vec::new();
    for (owner, at, health, team, state, intent, mut inv) in &mut users {
        let pressed = intent.use_key && !inv.prev_use;
        if inv.prev_use != intent.use_key {
            inv.prev_use = intent.use_key;
        }
        if !pressed || setting.0 == 0 || health.current <= 0.0 {
            continue;
        }
        let eye = at.translation + state.map_or(Vec3::ZERO, |s| s.eye_offset);
        let aim = intent.look_rotation() * Vec3::NEG_Z;
        let items = loose
            .iter()
            .filter(|(e, l, ..)| !taken.contains(e) && now - l.since >= TOUCH_DELAY)
            .filter(|(_, l, ..)| {
                weapons
                    .get(l.weapon)
                    .is_ok_and(|(w, only)| w.owner.is_none() && only.is_none_or(|t| team == Some(&t.0)))
            })
            .filter(|(e, _, t, _)| in_sight(&spatial, eye, t.translation, *e, |c| characters.contains(c)))
            .map(|(e, _, t, body)| {
                let half = body.map_or(DEFAULT_HALF, |b| (b.bounds.1 - b.bounds.0) / 2.0);
                (e, t.translation, half.max_element())
            });
        let Some(item) = use_target(eye, aim, items) else { continue };
        let Ok((_, l, ..)) = loose.get(item) else { continue };
        let weapon = l.weapon;
        let Ok((wanted, _)) = weapons.get(weapon) else { continue };
        // What it replaces: the held weapon if in that slot, else the
        // first there (none: an empty slot, it is only taken).
        let in_slot = |e: &Entity| weapons.get(*e).is_ok_and(|(w, _)| w.slot == wanted.slot);
        let mine = inv
            .active
            .filter(in_slot)
            .or_else(|| inv.weapons.iter().copied().find(in_slot));
        taken.push(item);
        used.write(UsedPickup { who: owner, weapon });
        commands.queue(move |w: &mut World| swap(w, owner, item, weapon, mine));
    }
}

/// Take `weapon` (lying as `item`), dropping `mine` first.
fn swap(world: &mut World, owner: Entity, item: Entity, weapon: Entity, mine: Option<Entity>) {
    if world.get_entity(item).is_err() || world.get::<Weapon>(weapon).is_none_or(|w| w.owner.is_some()) {
        return;
    }
    if mine.is_some_and(|m| world.get::<Undroppable>(m).is_some()) {
        return;
    }
    let held = world.get::<Inventory>(owner).and_then(|i| i.active);
    world.despawn(item);
    if let Some(m) = mine {
        drop_this(world, owner, m, true);
    }
    take(world, owner, weapon);
    if let Some(mut inv) = world.get_mut::<Inventory>(owner)
        && (held.is_none() || held == mine)
    {
        inv.wanted = Some(weapon);
    }
}

/// Whether `eye` sees `to` through solid geometry (characters and loose
/// items don't block; `item` is the one looked at).
fn in_sight(
    spatial: &avian3d::prelude::SpatialQuery,
    eye: Vec3,
    to: Vec3,
    item: Entity,
    character: impl Fn(Entity) -> bool,
) -> bool {
    blocker(spatial, eye, to, item, character).is_none()
}

/// What solid geometry hides `to` from `eye`, if anything (characters
/// and loose items don't block; `item` is the one looked at).
fn blocker(
    spatial: &avian3d::prelude::SpatialQuery,
    eye: Vec3,
    to: Vec3,
    item: Entity,
    character: impl Fn(Entity) -> bool,
) -> Option<Entity> {
    let filter = avian3d::prelude::SpatialQueryFilter::default()
        .with_mask(crate::core::SOLID_LAYERS.0 & !crate::core::ITEM_LAYER.0);
    let d = to - eye;
    let dir = Dir3::new(d).ok()?;
    spatial
        .cast_ray_predicate(eye, dir, d.length(), true, &filter, &|e| e != item && !character(e))
        .map(|hit| hit.entity)
}

/// What touching `weapon` does for a character carrying `inv`.
fn touch_of(weapon: &Weapon, inv: &Inventory, weapons: &Query<(&Weapon, Option<&PickupTeam>)>) -> Touch {
    let mut slot_taken = None;
    for e in &inv.weapons {
        let Ok((mine, _)) = weapons.get(*e) else { continue };
        if mine.id == weapon.id {
            return Touch::Ammo(*e);
        }
        if mine.slot == weapon.slot {
            slot_taken = slot_taken.or(Some(*e));
        }
    }
    slot_taken.map_or(Touch::Equip, Touch::Refused)
}

/// The same weapon type touched: its rounds go into `mine`'s reserve, up
/// to the most it carries (spec 3.9: the clip; ours also carries the
/// dropper's reserve with the gun, taken after the clip). A gun left with
/// nothing is removed; one nothing was taken from stays.
fn take_ammo(world: &mut World, item: Entity, weapon: Entity, mine: Entity) {
    let Some(room) = world
        .get::<super::Magazine>(mine)
        .map(|m| m.reserve_max.saturating_sub(m.reserve))
    else {
        return;
    };
    let Some(mut theirs) = world.get_mut::<super::Magazine>(weapon) else {
        return;
    };
    let from_clip = theirs.clip.min(room);
    theirs.clip -= from_clip;
    let from_reserve = theirs.reserve.min(room - from_clip);
    theirs.reserve -= from_reserve;
    let empty = theirs.clip == 0 && theirs.reserve == 0;
    let got = from_clip + from_reserve;
    if got == 0 {
        return;
    }
    if let Some(mut m) = world.get_mut::<super::Magazine>(mine) {
        m.reserve += got;
    }
    if empty {
        world.despawn(item);
        world.despawn(weapon);
    }
}

fn take(world: &mut World, owner: Entity, weapon: Entity) {
    if world.get::<Weapon>(weapon).is_none() {
        return;
    }
    let Some(mut inv) = world.get_mut::<Inventory>(owner) else { return };
    inv.weapons.push(weapon);
    if inv.active.is_none() && inv.wanted.is_none() {
        inv.wanted = Some(weapon);
    }
    if let Some(mut w) = world.get_mut::<Weapon>(weapon) {
        w.owner = Some(owner);
    }
    super::equip::picked_up(world, owner, weapon);
}
