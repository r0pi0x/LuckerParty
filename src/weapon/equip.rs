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
        entities::{
            EntityAnchors, FireEntityOutput, MapAnchor, MapEntities, entity_rotation, entity_to_engine,
            rotation_to_engine,
        },
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
    // Placed weapons someone still carries become ordinary weapons: the
    // map's entity starts again where it was placed.
    let carried: Vec<Entity> = world
        .query_filtered::<(Entity, &Weapon), (With<MapWeapon>, Without<Loose>)>()
        .iter(world)
        .filter(|(_, w)| w.owner.is_some())
        .map(|(e, _)| e)
        .collect();
    for e in carried {
        world.entity_mut(e).remove::<MapWeapon>();
    }
    let since = world.resource::<Time>().elapsed_secs_f64() - super::drop::TOUCH_DELAY;
    for (index, e) in entities.iter().enumerate() {
        let class = e.classname().to_ascii_lowercase();
        if !class.starts_with("weapon_") {
            continue;
        }
        let Some(def) = world.resource::<WeaponRegistry>().find(&class) else { continue };
        let (id, build) = (def.id, def.build);
        // The weapon itself remembers its map entity too (its pickup
        // outputs; what is parented to it follows it: `anchor_weapons`).
        let mut w = world.spawn((Name::new(id.to_string()), WeaponState::default(), MapWeapon(index)));
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

/// A placed weapon was picked up: its map entity fires OnPlayerPickup
/// with the player as activator (Source's weapon output, from the public
/// entity documentation; no spec covers it).
pub(super) fn picked_up(world: &mut World, owner: Entity, weapon: Entity) {
    let Some(index) = world.get::<MapWeapon>(weapon).map(|m| m.0) else {
        return;
    };
    if world.contains_resource::<Messages<FireEntityOutput>>() {
        world.write_message(FireEntityOutput {
            map_index: index,
            output: "OnPlayerPickup".into(),
            activator: Some(owner),
        });
    }
}

/// Where each placed weapon with entities parented to it is
/// (`map::entities::EntityAnchors`, and its anchor node): lying loose, the
/// loose item's pose; carried, its owner's origin (the feet, Source's
/// origin) turned to the owner's yaw, as a held weapon follows its owner.
/// What is parented to it rides the anchor node; the logic moves its own
/// children there.
#[allow(clippy::type_complexity)]
pub(super) fn anchor_weapons(
    weapons: Query<(Entity, &Weapon, &MapWeapon), Without<Loose>>,
    loose: Query<(&Loose, &Transform)>,
    owners: Query<(&Transform, &crate::core::Intent, Option<&crate::core::MovementState>)>,
    mut nodes: Query<(&MapAnchor, &mut Transform), (Without<Loose>, Without<crate::core::Intent>)>,
    mut anchors: ResMut<EntityAnchors>,
    map: Option<Res<MapEntities>>,
) {
    if nodes.is_empty() {
        if !anchors.0.is_empty() {
            anchors.0.clear();
        }
        return;
    }
    let mut poses: Vec<(usize, Transform)> = Vec::new();
    for (weapon, w, map_weapon) in &weapons {
        let pose = match w.owner {
            Some(o) => {
                let Ok((t, intent, state)) = owners.get(o) else { continue };
                let feet = t.translation + Vec3::Y * state.map_or(0.0, |s| s.hull_min.y);
                // Entity yaw 0 is engine +X; the intent's yaw 0 looks down -Z.
                let yaw = intent.yaw.to_degrees() + 90.0;
                Transform::from_translation(feet)
                    .with_rotation(rotation_to_engine(entity_rotation(Vec3::new(0.0, yaw, 0.0))))
            }
            None => match loose.iter().find(|(l, _)| l.weapon == weapon) {
                // Where the map placed it, until someone takes it (its
                // loose body settling or tumbling doesn't move what
                // hangs on it).
                Some((l, _)) if l.dropper.is_none() && map.as_ref().is_some_and(|m| m.entities.len() > map_weapon.0) => {
                    let (m, e) = map.as_ref().map(|m| (m, &m.entities[map_weapon.0])).unwrap();
                    Transform::from_translation(entity_to_engine(e.origin(), m.scale))
                        .with_rotation(rotation_to_engine(entity_rotation(e.angles())))
                }
                // Dropped: where it lies, turned only about the vertical.
                Some((_, t)) => {
                    let (yaw, _, _) = t.rotation.to_euler(EulerRot::YXZ);
                    Transform::from_translation(t.translation).with_rotation(Quat::from_rotation_y(yaw))
                }
                None => continue,
            },
        };
        poses.push((map_weapon.0, pose));
    }
    poses.sort_by_key(|(i, _)| *i);
    for (anchor, mut t) in &mut nodes {
        if let Some((_, pose)) = poses.iter().find(|(i, _)| *i == anchor.0)
            && *t != *pose
        {
            *t = *pose;
        }
    }
    if anchors.0 != poses {
        anchors.0 = poses;
    }
}
