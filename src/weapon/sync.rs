//! What a character carries as bytes (docs/plans/active/multiplayer.md,
//! slice 4): a network server sends its state of a client's own player
//! with that player's other predicted state (`core::PredictedComponents`),
//! and the client compares it with its prediction and takes it on a
//! mismatch, as it does for movement.
//!
//! - **The inventory** (`encode`): each weapon carried, in order, as its
//!   registry index (`WeaponRegistry`) and the changing state of each of
//!   its parts that prediction runs (`NetPart`: timers, clip, mode,
//!   accuracy, recoil, grenade count); which one is drawn, the last one
//!   and the one wanted as indices; the player-level timer and button
//!   edges. Entity ids never go over the wire.
//! - **On the client** (`decode`): the weapons are its own entities, built
//!   from the registry like any weapon (`spawn_weapon`); a different list
//!   (one picked up, bought, dropped, a respawn) builds them again, then
//!   each part takes the server's state.
//!
//! What only the server changes isn't here: the use key's edge (pickups
//! by use are the server's).

use bevy::{ecs::component::Mutable, prelude::*};
use serde::{Deserialize, Serialize, de::DeserializeOwned};

use super::{Inventory, Weapon, WeaponRegistry, WeaponState};
use crate::core::PredictedAppExt;

/// A weapon part whose state prediction runs: what of it changes (the rest
/// comes from the weapon's definition, the same on both sides).
pub trait NetPart: Component<Mutability = Mutable> + Clone {
    type Net: Serialize + DeserializeOwned;
    fn net(&self) -> Self::Net;
    fn set_net(&mut self, net: Self::Net);
}

#[derive(Clone, Copy)]
struct PartCodec {
    name: &'static str,
    encode: fn(&World, Entity) -> Option<Vec<u8>>,
    decode: fn(&mut World, Entity, &[u8]),
}

/// The weapon parts that go with the inventory (`predicted_part`), by
/// name.
#[derive(Resource, Default, Clone)]
pub struct NetParts(Vec<PartCodec>);

impl NetParts {
    pub fn names(&self) -> Vec<&'static str> {
        self.0.iter().map(|p| p.name).collect()
    }
}

pub trait CarriedAppExt {
    /// A weapon part prediction saves and restores (`predicted`) and a
    /// server sends with its owner's inventory (`NetPart`).
    fn predicted_part<C: NetPart>(&mut self) -> &mut Self;
}

impl CarriedAppExt for App {
    fn predicted_part<C: NetPart>(&mut self) -> &mut Self {
        self.predicted::<C>();
        let name = std::any::type_name::<C>();
        let mut parts = self.world_mut().get_resource_or_init::<NetParts>();
        if !parts.0.iter().any(|p| p.name == name) {
            parts.0.push(PartCodec {
                name,
                encode: |w, e| w.get::<C>(e).and_then(|c| postcard::to_allocvec(&c.net()).ok()),
                decode: |w, e, bytes| {
                    if let Ok(n) = postcard::from_bytes::<C::Net>(bytes)
                        && let Some(mut c) = w.get_mut::<C>(e)
                    {
                        c.set_net(n);
                    }
                },
            });
            parts.0.sort_by_key(|p| p.name);
        }
        self
    }
}

/// One weapon carried: its registry index and its parts' state (in
/// `NetParts` order; None where it lacks that part).
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
struct NetWeapon {
    def: u16,
    parts: Vec<Option<Vec<u8>>>,
}

/// An inventory over the wire (see the module docs).
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
struct NetCarried {
    weapons: Vec<NetWeapon>,
    active: Option<u8>,
    last: Option<u8>,
    wanted: Option<u8>,
    next_attack: f64,
    prev_fire: bool,
    prev_secondary: bool,
    prev_select: Option<u8>,
    prev_last: bool,
}

/// The name the inventory goes by in the predicted state.
pub fn name() -> &'static str {
    std::any::type_name::<Inventory>()
}

/// What `owner` carries (None: no inventory).
pub fn encode(world: &World, owner: Entity) -> Option<Vec<u8>> {
    let inv = world.get::<Inventory>(owner)?;
    let registry = world.resource::<WeaponRegistry>();
    let parts = world.get_resource::<NetParts>().map(|p| p.0.clone()).unwrap_or_default();
    let mut kept = Vec::with_capacity(inv.weapons.len());
    let mut weapons = Vec::with_capacity(inv.weapons.len());
    for &w in &inv.weapons {
        let Some(def) = world.get::<Weapon>(w).and_then(|x| registry.index(x.id)) else {
            continue;
        };
        weapons.push(NetWeapon {
            def: def as u16,
            parts: parts.iter().map(|p| (p.encode)(world, w)).collect(),
        });
        kept.push(w);
    }
    let index = |e: Option<Entity>| e.and_then(|e| kept.iter().position(|k| *k == e)).map(|i| i as u8);
    postcard::to_allocvec(&NetCarried {
        weapons,
        active: index(inv.active),
        last: index(inv.last),
        wanted: index(inv.wanted),
        next_attack: inv.next_attack,
        prev_fire: inv.prev_fire,
        prev_secondary: inv.prev_secondary,
        prev_select: inv.prev_select,
        prev_last: inv.prev_last,
    })
    .ok()
}

/// Make `owner` carry what `bytes` says (None: nothing, no inventory):
/// its weapons rebuilt from the registry when the list differs, then
/// every part set.
pub fn decode(world: &mut World, owner: Entity, bytes: Option<&[u8]>) {
    let net = bytes.and_then(|b| postcard::from_bytes::<NetCarried>(b).ok());
    let current: Vec<Entity> = world
        .get::<Inventory>(owner)
        .map(|i| i.weapons.clone())
        .unwrap_or_default();
    let Some(net) = net else {
        despawn_weapons(world, &current);
        world.entity_mut(owner).remove::<Inventory>();
        return;
    };
    let ids: Vec<Option<&'static str>> = {
        let registry = world.resource::<WeaponRegistry>();
        net.weapons
            .iter()
            .map(|w| registry.0.get(w.def as usize).map(|d| d.id))
            .collect()
    };
    let same = current.len() == ids.len()
        && current
            .iter()
            .zip(&ids)
            .all(|(e, id)| world.get::<Weapon>(*e).map(|w| w.id) == *id);
    let entities: Vec<Option<Entity>> = if same {
        current.into_iter().map(Some).collect()
    } else {
        despawn_weapons(world, &current);
        ids.iter()
            .map(|id| id.and_then(|id| super::spawn_weapon(world, id, owner)))
            .collect()
    };
    let parts = world.get_resource::<NetParts>().map(|p| p.0.clone()).unwrap_or_default();
    for (e, w) in entities.iter().zip(&net.weapons) {
        let Some(e) = *e else { continue };
        for (codec, part) in parts.iter().zip(&w.parts) {
            if let Some(bytes) = part {
                (codec.decode)(world, e, bytes);
            }
        }
    }
    let at = |i: Option<u8>| i.and_then(|i| entities.get(i as usize).copied().flatten());
    let inv = Inventory {
        weapons: entities.iter().flatten().copied().collect(),
        active: at(net.active),
        last: at(net.last),
        wanted: at(net.wanted),
        next_attack: net.next_attack,
        prev_fire: net.prev_fire,
        prev_secondary: net.prev_secondary,
        prev_select: net.prev_select,
        prev_last: net.prev_last,
        prev_use: world.get::<Inventory>(owner).is_some_and(|i| i.prev_use),
    };
    world.entity_mut(owner).insert(inv);
}

fn despawn_weapons(world: &mut World, weapons: &[Entity]) {
    for w in weapons {
        if let Ok(e) = world.get_entity_mut(*w) {
            e.despawn();
        }
    }
}

// The parts' changing state.

impl NetPart for WeaponState {
    type Net = WeaponState;
    fn net(&self) -> Self::Net {
        self.clone()
    }
    fn set_net(&mut self, net: Self::Net) {
        *self = net;
    }
}

impl NetPart for super::Magazine {
    /// Clip and reserve.
    type Net = (u32, u32);
    fn net(&self) -> Self::Net {
        (self.clip, self.reserve)
    }
    fn set_net(&mut self, (clip, reserve): Self::Net) {
        self.clip = clip;
        self.reserve = reserve;
    }
}

impl NetPart for super::AltModes {
    type Net = u8;
    fn net(&self) -> Self::Net {
        self.current
    }
    fn set_net(&mut self, net: Self::Net) {
        self.current = net;
    }
}

impl NetPart for super::grenade::Throwable {
    /// Count, pin out, throw and redraw times.
    type Net = (u32, bool, Option<f64>, Option<f64>);
    fn net(&self) -> Self::Net {
        (self.count, self.pin, self.throw_at, self.redraw_at)
    }
    fn set_net(&mut self, (count, pin, throw_at, redraw_at): Self::Net) {
        self.count = count;
        self.pin = pin;
        self.throw_at = throw_at;
        self.redraw_at = redraw_at;
    }
}
