//! Scenario tests for how bots buy (`weapon::economy::autobuy_rolling`
//! with a `BotBuying` from `bot::buy`) on the greybox map: a full buy
//! (preferred rifle, kevlar and helmet, a kit for counter-terrorists), an
//! eco round when broke, a pistol round's kevlar, and profile
//! preferences (`botprofile.db`'s `WeaponPreference`).

use bevy::prelude::*;
use mashup::{
    bot::buy::{GRENADES_DEFEND, resolve},
    core::Team,
    games::cs_source::{
        TICK_INTERVAL,
        weapons::{AK47, AWP, CsWeaponsPlugin, FAMAS, M4A1},
    },
    greybox::GreyboxMapPlugin,
    harness::Sim,
    movement::placeholder,
    weapon::{
        Armor, Inventory, Weapon, WeaponRegistry,
        economy::{BotBuying, BuyKind, DefuseKit, Money, autobuy_rolling},
    },
};

fn sim() -> Sim {
    let mut sim = Sim::new((GreyboxMapPlugin, CsWeaponsPlugin));
    sim.set_tick_interval(TICK_INTERVAL);
    sim
}

/// A character on `team` with `money` and no primary, wanting `prefs`
/// (and a kit if a counter-terrorist).
fn buyer(sim: &mut Sim, team: u8, money: u32, prefs: Vec<&'static str>) -> Entity {
    let p = sim.spawn_character(Vec3::new(team as f32 * 2.0, 0.9, 12.0), placeholder::ID);
    sim.app.world_mut().entity_mut(p).insert((
        Team(team),
        Money(money),
        BotBuying {
            prefs,
            grenades: [0.0; 3],
            kit: team == 2,
        },
    ));
    sim.ticks(1);
    // Whatever primary it started with goes (a round lost with it dead).
    let w = sim.app.world_mut();
    let held = w.get::<Inventory>(p).map(|i| i.weapons.clone()).unwrap_or_default();
    for x in held {
        if w.get::<Weapon>(x).is_some_and(|x| x.slot == 0) {
            w.get_mut::<Inventory>(p).unwrap().weapons.retain(|y| *y != x);
            if w.get::<Inventory>(p).unwrap().active == Some(x) {
                w.get_mut::<Inventory>(p).unwrap().active = None;
            }
            w.despawn(x);
        }
    }
    p
}

fn primary(sim: &Sim, p: Entity) -> Option<&'static str> {
    let w = sim.app.world();
    w.get::<Inventory>(p)?
        .weapons
        .iter()
        .filter_map(|x| w.get::<Weapon>(*x))
        .find(|x| x.slot == 0)
        .map(|x| x.id)
}

#[test]
fn bots_buy_rifles_armour_and_kits_or_save() {
    let mut sim = sim();
    // Rich counter-terrorist: the rifle, kevlar and helmet, a kit.
    let ct = buyer(&mut sim, 2, 5000, vec![M4A1]);
    assert_eq!(autobuy_rolling(sim.app.world_mut(), ct, &mut || 0.0), BuyKind::Full);
    assert_eq!(primary(&sim, ct), Some(M4A1));
    assert_eq!(sim.app.world().get::<Armor>(ct).map(|a| a.helmet), Some(true));
    assert!(sim.app.world().get::<DefuseKit>(ct).is_some(), "a kit");
    let left = sim.app.world().get::<Money>(ct).unwrap().0;
    assert!(left < 5000 - 3100 - 1000 - 200 + 1, "{left}");

    // The preferred one too dear: the next it prefers.
    let ct2 = buyer(&mut sim, 2, 3500, vec![AWP, FAMAS]);
    autobuy_rolling(sim.app.world_mut(), ct2, &mut || 0.0);
    assert_eq!(primary(&sim, ct2), Some(FAMAS));

    // Broke (a lost round): saves, no primary, no armour, no kit.
    let poor = buyer(&mut sim, 2, 1400, vec![]);
    assert_eq!(autobuy_rolling(sim.app.world_mut(), poor, &mut || 0.0), BuyKind::Eco);
    assert_eq!(primary(&sim, poor), None);
    assert!(sim.app.world().get::<Armor>(poor).is_none_or(|a| a.amount == 0.0));
    assert!(sim.app.world().get::<DefuseKit>(poor).is_none());
    assert_eq!(sim.app.world().get::<Money>(poor), Some(&Money(1400)));
    // Saving, but with kit money: the kit only.
    let saver = buyer(&mut sim, 2, 1600, vec![]);
    assert_eq!(autobuy_rolling(sim.app.world_mut(), saver, &mut || 0.0), BuyKind::Eco);
    assert!(sim.app.world().get::<DefuseKit>(saver).is_some());
    assert_eq!(sim.app.world().get::<Money>(saver), Some(&Money(1400)));

    // Pistol round money: kevlar, no helmet.
    let t = buyer(&mut sim, 1, 800, vec![AK47]);
    assert_eq!(autobuy_rolling(sim.app.world_mut(), t, &mut || 0.0), BuyKind::Pistol);
    assert_eq!(sim.app.world().get::<Armor>(t).map(|a| a.helmet), Some(false));
    assert!(
        sim.app.world().get::<Money>(t).unwrap().0 <= 150,
        "kevlar, then pistol ammo"
    );
    assert!(sim.app.world().get::<DefuseKit>(t).is_none(), "terrorists buy no kit");

    // A terrorist's preferences skip what its team can't buy.
    let t2 = buyer(&mut sim, 1, 4000, vec![M4A1, AK47]);
    autobuy_rolling(sim.app.world_mut(), t2, &mut || 0.0);
    assert_eq!(primary(&sim, t2), Some(AK47));
}

/// Profile names as `botprofile.db` writes them resolve to weapons
/// (aliases, any case, "none" skipped).
#[test]
fn profile_preferences_resolve_to_weapons() {
    let sim = sim();
    let registry = sim.app.world().resource::<WeaponRegistry>();
    let names: Vec<String> = ["M4A1", "none", "mp5", "AWP", "nosuchgun"]
        .iter()
        .map(|s| s.to_string())
        .collect();
    let ids = resolve(&names, registry);
    assert_eq!(ids, [M4A1, mashup::games::cs_source::weapons::MP5NAVY, AWP]);
    assert!(GRENADES_DEFEND.iter().all(|c| (0.0..=1.0).contains(c)));
}
