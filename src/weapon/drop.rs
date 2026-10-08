//! Dropping weapons (Source's `drop`, G in CS:S) and picking them up by
//! walking over them. A dropped weapon keeps its entity (and so its ammo);
//! a separate loose entity (`map::loose::LooseItem`) lies in the world
//! and points back at it.

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

/// Drop one weapon `owner` carries (thrown along the view, or let fall).
pub fn drop_this(world: &mut World, owner: Entity, weapon: Entity, thrown: bool) -> Option<Entity> {
    let now = world.resource::<Time>().elapsed_secs_f64();
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
        .query::<(Entity, &Loose)>()
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
    /// Nothing: that slot holds another weapon.
    Refused,
    /// Take the weapon.
    Equip,
    /// The same weapon type is carried: take only its ammo, into this one.
    Ammo(Entity),
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

/// Living characters touching a loose weapon (spec 3.9): its box grown
/// 36 units sideways and 18 up, touchable 1 s after the drop, seen from
/// the eye (solid geometry and windows block). Taken when nothing is in
/// its slot; when the same type is carried, only its ammo is taken.
pub(super) fn pick_up(
    loose: Query<(Entity, &Loose, &Transform, Option<&crate::map::PhysicsProp>)>,
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
        let (lo, hi) = match state {
            Some(s) if s.hull_max != s.hull_min => (s.hull_min, s.hull_max),
            _ => DEFAULT_HULL,
        };
        let (me_lo, me_hi) = (at.translation + lo, at.translation + hi);
        let eye = at.translation + state.map_or(Vec3::ZERO, |s| s.eye_offset);
        for (item, l, item_at, body) in &loose {
            if taken.contains(&item) || now - l.since < TOUCH_DELAY {
                continue;
            }
            let half = body.map_or(DEFAULT_HALF, |b| (b.bounds.1 - b.bounds.0) / 2.0);
            let (t_lo, t_hi) = touch_box(item_at, half);
            if me_lo.cmpgt(t_hi).any() || me_hi.cmplt(t_lo).any() {
                continue;
            }
            let Ok((weapon, only)) = weapons.get(l.weapon) else {
                continue;
            };
            if only.is_some_and(|t| team != Some(&t.0)) {
                continue;
            }
            let touch = touch_of(weapon, inv, &weapons);
            if touch == Touch::Refused || !in_sight(&spatial, eye, item_at.translation, item, &owners) {
                continue;
            }
            let weapon = l.weapon;
            match touch {
                Touch::Equip => {
                    taken.push(item);
                    commands.entity(item).despawn();
                    commands.queue(move |w: &mut World| take(w, owner, weapon));
                    break;
                }
                Touch::Ammo(mine) => {
                    commands.queue(move |w: &mut World| take_ammo(w, item, weapon, mine));
                }
                Touch::Refused => {}
            }
        }
    }
}

/// Whether `eye` sees `to` through solid geometry (characters and loose
/// items don't block; `item` is the one looked at).
fn in_sight(spatial: &avian3d::prelude::SpatialQuery, eye: Vec3, to: Vec3, item: Entity, owners: &Owners) -> bool {
    let filter = avian3d::prelude::SpatialQueryFilter::default()
        .with_mask(crate::core::SOLID_LAYERS.0 & !crate::core::ITEM_LAYER.0);
    let d = to - eye;
    let Ok(dir) = Dir3::new(d) else { return true };
    spatial
        .cast_ray_predicate(eye, dir, d.length(), true, &filter, &|e| e != item && !owners.contains(e))
        .is_none()
}

/// What touching `weapon` does for a character carrying `inv`.
fn touch_of(weapon: &Weapon, inv: &Inventory, weapons: &Query<(&Weapon, Option<&PickupTeam>)>) -> Touch {
    let mut slot_taken = false;
    for e in &inv.weapons {
        let Ok((mine, _)) = weapons.get(*e) else { continue };
        if mine.id == weapon.id {
            return Touch::Ammo(*e);
        }
        slot_taken |= mine.slot == weapon.slot;
    }
    if slot_taken { Touch::Refused } else { Touch::Equip }
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
}
