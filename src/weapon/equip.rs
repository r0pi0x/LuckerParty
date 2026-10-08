//! Equipment a map hands out (Source entities, as described in the public
//! entity documentation; CS:S behaviour beyond it is unmeasured):
//!
//! - `game_player_equip` without "Use Only": what everyone spawns with,
//!   instead of the starting weapons (`SpawnEquipment`, read here from the
//!   map's entities; the rules give it at each spawn).
//! - `game_player_equip` used, `player_weaponstrip`: `core::Equip` from
//!   the logic layer gives the activator its items, after taking its
//!   weapons when the entity strips.
//! - `weapon_*` entities placed in the map: loose weapons lying where they
//!   stand, put back at every round restart (`MapWeapon`).

use avian3d::prelude::{AngularVelocity, LinearVelocity};
use bevy::prelude::*;

use super::{Armor, Inventory, Weapon, WeaponRegistry, WeaponState, drop::Loose, give, grenade::Throwable};
use crate::{
    core::{Equip, RoundRestarts},
    map::{
        entities::{MapEntities, entity_rotation, entity_to_engine, rotation_to_engine},
        loose::LooseItem,
    },
};

/// What players spawn with on this map instead of the starting weapons:
/// the items of its `game_player_equip`s without "Use Only" (None: the
/// map has none).
#[derive(Resource, Default, Clone, Debug, PartialEq)]
pub struct SpawnEquipment(pub Option<Vec<(String, u32)>>);

/// A loose weapon the map placed (`weapon_*` entity, by index): replaced
/// at round restarts while it lies there, and not counted against the
/// dropped-weapon limit.
#[derive(Component, Clone, Copy, Debug)]
pub struct MapWeapon(pub usize);

/// game_player_equip's "Use Only" spawnflag.
const USE_ONLY: u32 = 1;

/// A map entity's items as game_player_equip lists them: keyvalues naming
/// `weapon_*`/`item_*` with a count.
fn items(keyvalues: &[(String, String)]) -> Vec<(String, u32)> {
    keyvalues
        .iter()
        .filter(|(k, _)| {
            let k = k.to_ascii_lowercase();
            k.starts_with("weapon_") || k.starts_with("item_")
        })
        .map(|(k, v)| (k.to_ascii_lowercase(), v.trim().parse::<i64>().unwrap_or(1).clamp(1, 16) as u32))
        .collect()
}

/// The spawn equipment of a map's entities.
pub fn spawn_equipment(entities: &[crate::map::MapEntity]) -> SpawnEquipment {
    let mut all: Option<Vec<(String, u32)>> = None;
    for e in entities.iter().filter(|e| e.classname().eq_ignore_ascii_case("game_player_equip")) {
        let flags = e.get("spawnflags").and_then(|f| f.trim().parse::<u32>().ok()).unwrap_or(0);
        if flags & USE_ONLY != 0 {
            continue;
        }
        all.get_or_insert_default().extend(items(&e.keyvalues));
    }
    SpawnEquipment(all)
}

/// Take every weapon `owner` carries.
pub fn strip(world: &mut World, owner: Entity) {
    let held = world.get::<Inventory>(owner).map(|i| i.weapons.clone()).unwrap_or_default();
    for w in held {
        world.despawn(w);
    }
    if let Ok(mut e) = world.get_entity_mut(owner) {
        e.insert(Inventory::default());
    }
}

/// Give `owner` one item by its entity name (`weapon_ak47`, `item_kevlar`,
/// `item_assaultsuit`), `count` times for grenades: a weapon it already
/// carries gives nothing (a grenade one more, up to its limit), one in an
/// occupied slot replaces what is there. Returns whether anything was
/// given.
pub fn give_item(world: &mut World, owner: Entity, name: &str, count: u32) -> bool {
    match name {
        "item_kevlar" | "item_assaultsuit" => {
            let helmet = name == "item_assaultsuit" || world.get::<Armor>(owner).is_some_and(|a| a.helmet);
            world.entity_mut(owner).insert(Armor { amount: 1.0, helmet });
            return true;
        }
        n if !n.starts_with("weapon_") => return false,
        _ => {}
    }
    let Some(id) = world.resource::<WeaponRegistry>().find(name).map(|d| d.id) else {
        return false;
    };
    let held = world.get::<Inventory>(owner).map(|i| i.weapons.clone()).unwrap_or_default();
    let mut given = false;
    for _ in 0..count.max(1) {
        if let Some(have) = held.iter().copied().find(|w| world.get::<Weapon>(*w).is_some_and(|w| w.id == id)) {
            if let Some(mut t) = world.get_mut::<Throwable>(have)
                && t.count < t.max
            {
                t.count += 1;
                given = true;
                continue;
            }
            break;
        }
        let Some(new) = give(world, owner, id) else { break };
        given = true;
        if world.get::<Throwable>(new).is_some() {
            continue;
        }
        // One weapon per slot.
        let slot = world.get::<Weapon>(new).map(|w| w.slot);
        if let Some(old) = held.iter().copied().find(|w| world.get::<Weapon>(*w).map(|w| w.slot) == slot) {
            if let Some(mut inv) = world.get_mut::<Inventory>(owner) {
                inv.weapons.retain(|w| *w != old);
            }
            world.despawn(old);
        }
        break;
    }
    given
}

/// `core::Equip`: strip, then give.
pub(super) fn apply_equips(mut asks: MessageReader<Equip>, mut commands: Commands) {
    for ask in asks.read().cloned() {
        commands.queue(move |world: &mut World| {
            if world.get_entity(ask.target).is_err() {
                return;
            }
            if ask.strip {
                strip(world, ask.target);
            }
            for (name, count) in &ask.items {
                give_item(world, ask.target, name, *count);
            }
        });
    }
}

/// On a new map: its spawn equipment, and its placed weapons; at a round
/// restart the placed weapons still lying there are put back as new.
pub(super) fn map_equipment(world: &mut World, mut seen: Local<(Option<usize>, u32)>) {
    let Some(map) = world.get_resource::<MapEntities>() else { return };
    let (entities, scale) = (map.entities.clone(), map.scale);
    let key = std::sync::Arc::as_ptr(&entities) as usize;
    let restarts = world.get_resource::<RoundRestarts>().map_or(0, |r| r.0);
    let new_map = seen.0 != Some(key);
    if !new_map && seen.1 == restarts {
        return;
    }
    *seen = (Some(key), restarts);
    if new_map {
        world.insert_resource(spawn_equipment(&entities));
    }
    place_weapons(world, &entities, scale);
}

/// Put the map's `weapon_*` entities down as loose weapons (any placed
/// before and still lying there go first).
pub fn place_weapons(world: &mut World, entities: &[crate::map::MapEntity], scale: f32) {
    let old: Vec<(Entity, Entity)> = world
        .query::<(Entity, &Loose, &MapWeapon)>()
        .iter(world)
        .map(|(e, l, _)| (e, l.weapon))
        .collect();
    for (e, w) in old {
        world.despawn(e);
        if world.get::<Weapon>(w).is_some_and(|w| w.owner.is_none()) {
            world.despawn(w);
        }
    }
    let since = world.resource::<Time>().elapsed_secs_f64() - super::drop::TOUCH_DELAY;
    for (index, e) in entities.iter().enumerate() {
        let class = e.classname().to_ascii_lowercase();
        if !class.starts_with("weapon_") {
            continue;
        }
        let Some(def) = world.resource::<WeaponRegistry>().find(&class) else { continue };
        let (id, build) = (def.id, def.build);
        let mut w = world.spawn((Name::new(id.to_string()), WeaponState::default()));
        build(&mut w);
        let weapon = w.id();
        // A little up from the origin (usually on the floor) so the body
        // starts clear of it.
        let at = entity_to_engine(e.origin(), scale) + Vec3::Y * 4.0 * scale;
        let turn = rotation_to_engine(entity_rotation(e.angles()));
        world.spawn((
            Name::new(format!("Map {id}")),
            LooseItem(id.to_string()),
            Loose {
                weapon,
                dropper: None,
                since,
            },
            MapWeapon(index),
            Transform::from_translation(at).with_rotation(turn),
            LinearVelocity(Vec3::ZERO),
            AngularVelocity(Vec3::ZERO),
        ));
    }
}
