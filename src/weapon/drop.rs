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
/// Seconds before the dropper can pick it up again.
const REPICK_DELAY: f64 = 1.5;
/// How close a character's centre must come to pick an item up (meters,
/// horizontally and vertically): about touching it.
const PICKUP_REACH: Vec2 = Vec2::new(0.6, 1.1);
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

/// Living characters walking over a loose weapon take it when they have
/// nothing in its slot.
pub(super) fn pick_up(
    loose: Query<(Entity, &Loose, &Transform)>,
    owners: Query<(Entity, &Transform, &Inventory, &Health, Option<&crate::core::Team>)>,
    weapons: Query<(&Weapon, Option<&PickupTeam>)>,
    time: Res<Time>,
    mut commands: Commands,
) {
    let now = time.elapsed_secs_f64();
    let mut taken = Vec::new();
    for (owner, at, inv, health, team) in &owners {
        if health.current <= 0.0 {
            continue;
        }
        for (item, l, item_at) in &loose {
            if taken.contains(&item) || (l.dropper == Some(owner) && now - l.since < REPICK_DELAY) {
                continue;
            }
            let d = item_at.translation - at.translation;
            if d.with_y(0.0).length() > PICKUP_REACH.x || d.y.abs() > PICKUP_REACH.y {
                continue;
            }
            let Ok((slot, only)) = weapons.get(l.weapon).map(|(w, t)| (w.slot, t.copied())) else {
                continue;
            };
            if only.is_some_and(|t| team != Some(&t.0)) {
                continue;
            }
            if inv.weapons.iter().any(|w| weapons.get(*w).is_ok_and(|(w, _)| w.slot == slot)) {
                continue;
            }
            taken.push(item);
            let weapon = l.weapon;
            commands.entity(item).despawn();
            commands.queue(move |w: &mut World| take(w, owner, weapon));
            break;
        }
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
