//! Money and buying for any game: prices by weapon ID and for armour
//! (`Prices`, filled by games), a character's `Money`, and whether buying
//! is open now (`BuyWindow`, set by the rules). Characters without `Money`
//! buy for free (deathmatch, testing).

use std::collections::HashMap;

use bevy::prelude::*;

use super::{Armor, Inventory, Weapon, WeaponRegistry, give};

/// A character's money.
#[derive(Component, Clone, Copy, Debug, PartialEq, Eq, Reflect)]
#[reflect(Component)]
pub struct Money(pub u32);

/// What things cost.
#[derive(Resource, Clone, Debug, Default)]
pub struct Prices {
    /// By full weapon ID.
    pub weapons: HashMap<&'static str, u32>,
    /// Weapons only one team may buy (weapon ID -> `core::Team` number).
    pub team_only: HashMap<&'static str, u8>,
    /// Kevlar; kevlar and helmet; the helmet alone (over full kevlar).
    pub vest: u32,
    pub vest_helmet: u32,
    pub helmet: u32,
}

/// Whether buying is open: Ok, or why not (shown to the player).
#[derive(Resource, Clone, Debug)]
pub struct BuyWindow(pub Result<(), String>);

impl Default for BuyWindow {
    fn default() -> Self {
        Self(Ok(()))
    }
}

/// Buy `name` (a weapon ID, `weapon_ak47`, `ak47`, `vest` or `vesthelm`)
/// for `owner`: checks the window and the money, replaces a weapon in the
/// same slot (dropped, as CS:S does). Returns what happened.
pub fn buy(world: &mut World, owner: Entity, name: &str) -> Result<String, String> {
    world.resource::<BuyWindow>().0.clone()?;
    if world.get::<Money>(owner).is_some() && !in_buy_zone(world, owner) {
        return Err("you are not in a buy zone".into());
    }
    let name = name.to_lowercase();
    let prices = world.resource::<Prices>().clone();
    let money = world.get::<Money>(owner).map(|m| m.0);
    let pay = |world: &mut World, cost: u32| -> Result<(), String> {
        if let Some(have) = money {
            if have < cost {
                return Err(format!("you have insufficient funds (${cost})"));
            }
            world.entity_mut(owner).insert(Money(have - cost));
        }
        Ok(())
    };
    if name == "vest" || name == "vesthelm" {
        let armor = world.get::<Armor>(owner).copied().unwrap_or_default();
        let full = armor.amount >= 1.0;
        let (cost, helmet) = match (name.as_str(), full, armor.helmet) {
            ("vest", true, _) | ("vesthelm", true, true) => return Err("you already have armour".into()),
            ("vest", false, h) => (prices.vest, h),
            (_, true, false) => (prices.helmet, true),
            _ => (prices.vest_helmet, true),
        };
        pay(world, cost)?;
        world.entity_mut(owner).insert(Armor { amount: 1.0, helmet });
        return Ok(format!("bought {name}"));
    }
    let wanted = if name.contains(':') || name.starts_with("weapon_") {
        name
    } else {
        format!("weapon_{name}")
    };
    let id = world
        .resource::<WeaponRegistry>()
        .find(&wanted)
        .map(|d| d.id)
        .ok_or_else(|| format!("no weapon {wanted}"))?;
    let held: Vec<Entity> = world.get::<Inventory>(owner).map(|i| i.weapons.clone()).unwrap_or_default();
    if let Some(team) = prices.team_only.get(id)
        && world.get::<crate::core::Team>(owner).is_some_and(|t| t.0 != *team)
    {
        return Err("your team can't buy that weapon".into());
    }
    if held.iter().any(|w| world.get::<Weapon>(*w).is_some_and(|w| w.id == id)) {
        return Err("you already have that weapon".into());
    }
    let cost = prices.weapons.get(id).copied().unwrap_or(0);
    pay(world, cost)?;
    let new = give(world, owner, id).ok_or("can't carry it")?;
    // One weapon per slot: the old one is dropped.
    let slot = world.get::<Weapon>(new).map(|w| w.slot);
    if let Some(old) = held
        .into_iter()
        .find(|w| world.get::<Weapon>(*w).map(|w| w.slot) == slot && world.get::<super::drop::Undroppable>(*w).is_none())
    {
        super::drop::drop_this(world, owner, old, false);
    }
    Ok(format!("bought {id}"))
}

/// Whether `owner` stands in one of its team's buy zones (Source
/// `func_buyzone`, `TeamNum` 2 terrorists, 3 CTs, else anyone). A map
/// without zones, or no map, lets everyone buy anywhere (CS:S puts
/// zones around the spawns then; not modelled yet).
pub fn in_buy_zone(world: &World, owner: Entity) -> bool {
    use crate::map::entities::{engine_to_entity, entity_rotation};
    let Some(map) = world.get_resource::<crate::map::MapEntities>() else {
        return true;
    };
    let zones: Vec<_> = map
        .entities
        .iter()
        .filter(|e| e.classname().eq_ignore_ascii_case("func_buyzone") && !e.hulls.is_empty())
        .collect();
    if zones.is_empty() {
        return true;
    }
    let Some(at) = world.get::<Transform>(owner).map(|t| t.translation) else {
        return false;
    };
    // Our teams 1 and 2 are Source's 2 and 3.
    let team = world.get::<crate::core::Team>(owner).map(|t| t.0 as i32 + 1);
    let p = engine_to_entity(at, map.scale);
    zones.iter().any(|z| {
        let zone_team = z.get("TeamNum").and_then(|v| v.trim().parse::<i32>().ok()).unwrap_or(0);
        if (zone_team == 2 || zone_team == 3) && team.is_some_and(|t| t != zone_team) {
            return false;
        }
        let local = entity_rotation(z.angles()).inverse() * (p - z.origin());
        z.hulls
            .iter()
            .any(|h| h.planes.iter().all(|(n, d)| n.dot(local) <= *d + 1.0))
    })
}

/// Each registered weapon's slot (built once on a scratch world).
pub fn weapon_slots(registry: &WeaponRegistry) -> Vec<(&'static str, u8)> {
    let mut scratch = World::new();
    registry
        .0
        .iter()
        .filter_map(|d| {
            let mut e = scratch.spawn_empty();
            (d.build)(&mut e);
            e.get::<Weapon>().map(|w| (d.id, w.slot))
        })
        .collect()
}

/// What a computer player buys with its money: the dearest primary its
/// team may buy and afford (if it has none), then armour with what's left.
/// CS:S's own bots weigh preferences and difficulty; this is the simple
/// version.
pub fn autobuy(world: &mut World, owner: Entity) {
    let prices = world.resource::<Prices>().clone();
    let slots = weapon_slots(world.resource::<WeaponRegistry>());
    let team = world.get::<crate::core::Team>(owner).map(|t| t.0);
    let money = world.get::<Money>(owner).map_or(0, |m| m.0);
    let held: Vec<u8> = world
        .get::<Inventory>(owner)
        .map(|i| i.weapons.iter().filter_map(|w| world.get::<Weapon>(*w)).map(|w| w.slot).collect())
        .unwrap_or_default();
    if !held.contains(&0) {
        let best = slots
            .iter()
            .filter(|(_, slot)| *slot == 0)
            .filter_map(|(id, _)| Some((*id, *prices.weapons.get(id)?)))
            .filter(|(id, price)| *price <= money && prices.team_only.get(id).is_none_or(|t| Some(*t) == team))
            .max_by_key(|(_, price)| *price);
        if let Some((id, _)) = best {
            let _ = buy(world, owner, id);
        }
    }
    let _ = buy(world, owner, "vesthelm").or_else(|_| buy(world, owner, "vest"));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn world() -> (World, Entity) {
        let mut w = World::new();
        w.init_resource::<Time>();
        w.init_resource::<WeaponRegistry>();
        w.init_resource::<BuyWindow>();
        let mut prices = Prices {
            vest: 650,
            vest_helmet: 1000,
            helmet: 350,
            ..default()
        };
        prices.weapons.insert("g:weapon_rifle", 2500);
        w.insert_resource(prices);
        w.resource_mut::<WeaponRegistry>().0.push(super::super::WeaponDef {
            id: "g:weapon_rifle",
            build: |e| {
                e.insert(Weapon {
                    id: "g:weapon_rifle",
                    slot: 0,
                    owner: None,
                    draw_time: 0.0,
                    max_speed: None,
                });
            },
        });
        let p = w.spawn((Transform::default(), Inventory::default(), Money(3000))).id();
        (w, p)
    }

    #[test]
    fn buying_costs_money_and_checks_funds_and_the_window() {
        let (mut w, p) = world();
        w.resource_mut::<Prices>().team_only.insert("g:weapon_rifle", 1);
        w.entity_mut(p).insert(crate::core::Team(2));
        assert!(buy(&mut w, p, "rifle").unwrap_err().contains("team"));
        w.entity_mut(p).insert(crate::core::Team(1));
        assert!(buy(&mut w, p, "rifle").is_ok());
        assert_eq!(w.get::<Money>(p), Some(&Money(500)));
        assert!(buy(&mut w, p, "rifle").unwrap_err().contains("already"));
        assert!(buy(&mut w, p, "vesthelm").unwrap_err().contains("insufficient"));
        assert!(buy(&mut w, p, "vest").is_err(), "650 > 500");
        w.entity_mut(p).insert(Money(2000));
        buy(&mut w, p, "vest").unwrap();
        // Helmet over full kevlar: 350.
        buy(&mut w, p, "vesthelm").unwrap();
        assert_eq!(w.get::<Money>(p), Some(&Money(1000)));
        assert_eq!(w.get::<Armor>(p).map(|a| a.helmet), Some(true));
        // A bot with 2700: the rifle (2500), not armour (200 left).
        w.entity_mut(p).insert(Money(2700));
        let rifle = w.get::<Inventory>(p).unwrap().weapons.clone();
        for r in rifle {
            w.get_mut::<Inventory>(p).unwrap().weapons.retain(|x| *x != r);
        }
        autobuy(&mut w, p);
        assert_eq!(w.get::<Money>(p), Some(&Money(200)));
        w.insert_resource(BuyWindow(Err("buy time is over".into())));
        assert_eq!(buy(&mut w, p, "vest").unwrap_err(), "buy time is over");
    }
}
