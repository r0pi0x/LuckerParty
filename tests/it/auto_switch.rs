//! `cl_autowepswitch` (CS:S's Multiplayer > Advanced "Automatically switch
//! to picked up weapons (if more powerful)"): walking over a weapon into
//! an empty slot draws it when its script weight is above the one in
//! hand's, only with the setting on, not while attack is held; the
//! local player's setting reaches its `UserInfo` from the console. The
//! network version is in tests/it/net_weapons.rs.

use avian3d::prelude::Position;
use bevy::prelude::*;
use mashup::{
    console::Console,
    core::{LocalPlayer, UserInfo},
    games::cs_source::{
        TICK_INTERVAL,
        weapons::{AK47, AWP, CsWeaponsPlugin, DEAGLE, KNIFE, USP},
    },
    greybox::{self, GreyboxMapPlugin},
    harness::Sim,
    movement::placeholder,
    weapon::{Inventory, Weapon, drop::drop_this, give},
};

fn sim() -> Sim {
    let mut sim = Sim::new((GreyboxMapPlugin, CsWeaponsPlugin));
    sim.set_tick_interval(TICK_INTERVAL);
    sim
}

fn ids(sim: &Sim, p: Entity) -> Vec<&'static str> {
    let w = sim.app.world();
    w.get::<Inventory>(p)
        .unwrap()
        .weapons
        .iter()
        .map(|e| w.get::<Weapon>(*e).unwrap().id)
        .collect()
}

fn active_id(sim: &Sim, p: Entity) -> Option<&'static str> {
    let w = sim.app.world();
    let a = w.get::<Inventory>(p)?.active?;
    Some(w.get::<Weapon>(a)?.id)
}

fn place(sim: &mut Sim, p: Entity, at: Vec3) {
    let w = sim.app.world_mut();
    w.get_mut::<Transform>(p).unwrap().translation = at;
    if let Some(mut pos) = w.get_mut::<Position>(p) {
        pos.0 = at;
    }
}

/// Drop `p`'s `id` where it stands (not thrown).
fn drop_id(sim: &mut Sim, p: Entity, id: &str) {
    let w = sim.app.world_mut();
    let weapon = w
        .get::<Inventory>(p)
        .unwrap()
        .weapons
        .iter()
        .copied()
        .find(|e| w.get::<Weapon>(*e).unwrap().id == id)
        .unwrap();
    drop_this(w, p, weapon, false).unwrap();
}

/// A player holding its pistol (its rifle dropped far away), with
/// `cl_autowepswitch` `on`, and `id` lying 6 m away, touchable.
fn setup(on: bool, id: &str) -> (Sim, Entity, Vec3) {
    let mut sim = sim();
    let p = sim.spawn_character(greybox::SPAWNS[0], placeholder::ID);
    let other = sim.spawn_character(greybox::SPAWNS[1], placeholder::ID);
    sim.ticks(2);
    sim.app
        .world_mut()
        .entity_mut(p)
        .insert(UserInfo([("cl_autowepswitch".to_string(), if on { "1" } else { "0" }.to_string())].into()));
    // The rifle goes, and the player steps away from it.
    drop_id(&mut sim, p, AK47);
    place(&mut sim, p, greybox::SPAWNS[0] + Vec3::new(5.0, 0.0, 0.0));
    // `id` lies where the other player stood.
    let at = sim.position(other);
    if !ids(&sim, other).contains(&id) {
        give(sim.app.world_mut(), other, id).unwrap();
        sim.ticks(1);
    }
    drop_id(&mut sim, other, id);
    sim.app.world_mut().despawn(other);
    // The pistol drawn, the touch delay over.
    sim.seconds(1.2);
    assert!(!ids(&sim, p).contains(&AK47));
    assert_ne!(active_id(&sim, p), Some(AK47));
    sim.intent(p).select = Some(1);
    sim.ticks(1);
    sim.intent(p).select = None;
    sim.seconds(1.0);
    (sim, p, at)
}

#[test]
fn a_heavier_weapon_walked_over_is_drawn_when_enabled() {
    let (mut sim, p, at) = setup(true, AWP);
    assert_eq!(active_id(&sim, p), Some(USP));
    place(&mut sim, p, at + Vec3::Y * 0.9);
    sim.ticks(10);
    assert!(ids(&sim, p).contains(&AWP), "picked up");
    assert_eq!(active_id(&sim, p), Some(AWP), "the AWP (30) outweighs the USP (5)");
}

#[test]
fn no_switch_when_disabled() {
    let (mut sim, p, at) = setup(false, AWP);
    place(&mut sim, p, at + Vec3::Y * 0.9);
    sim.ticks(10);
    assert!(ids(&sim, p).contains(&AWP), "still picked up");
    assert_eq!(active_id(&sim, p), Some(USP));
}

#[test]
fn no_switch_to_a_lighter_weapon() {
    // The knife (0) in hand would switch to anything; the AK (25) in hand
    // keeps over a Deagle (7) in the empty pistol slot.
    let (mut sim, p, _) = setup(true, AWP);
    let usp_at = sim.position(p);
    drop_id(&mut sim, p, USP);
    give(sim.app.world_mut(), p, AK47).unwrap();
    sim.ticks(1);
    // Away from the lying USP while the Deagle is set down elsewhere.
    place(&mut sim, p, usp_at + Vec3::new(0.0, 0.0, -8.0));
    let other = sim.spawn_character(usp_at + Vec3::new(-10.0, 0.0, -8.0), placeholder::ID);
    sim.ticks(2);
    give(sim.app.world_mut(), other, DEAGLE).unwrap();
    sim.ticks(1);
    let deagle_at = sim.position(other);
    drop_id(&mut sim, other, DEAGLE);
    sim.app.world_mut().despawn(other);
    sim.seconds(1.5);
    assert_eq!(active_id(&sim, p), Some(AK47));
    place(&mut sim, p, deagle_at + Vec3::Y * 0.9);
    sim.ticks(10);
    assert!(ids(&sim, p).contains(&DEAGLE), "picked up");
    assert_eq!(active_id(&sim, p), Some(AK47));
}

#[test]
fn no_switch_while_attack_is_held() {
    let (mut sim, p, at) = setup(true, AWP);
    // The knife in hand, attack held (a slash).
    sim.intent(p).select = Some(2);
    sim.ticks(1);
    sim.intent(p).select = None;
    sim.seconds(1.1);
    assert_eq!(active_id(&sim, p), Some(KNIFE));
    sim.intent(p).fire = true;
    place(&mut sim, p, at + Vec3::Y * 0.9);
    sim.ticks(10);
    assert!(ids(&sim, p).contains(&AWP));
    assert_eq!(active_id(&sim, p), Some(KNIFE));
}

#[test]
fn a_full_slot_takes_nothing() {
    // Only into an empty slot: holding the AK, an AWP lying there stays.
    let mut sim = sim();
    let p = sim.spawn_character(greybox::SPAWNS[0], placeholder::ID);
    let other = sim.spawn_character(greybox::SPAWNS[1], placeholder::ID);
    sim.ticks(2);
    sim.app
        .world_mut()
        .entity_mut(p)
        .insert(UserInfo([("cl_autowepswitch".to_string(), "1".to_string())].into()));
    give(sim.app.world_mut(), other, AWP).unwrap();
    sim.ticks(1);
    let at = sim.position(other);
    drop_id(&mut sim, other, AWP);
    sim.app.world_mut().despawn(other);
    sim.seconds(1.5);
    place(&mut sim, p, at + Vec3::Y * 0.9);
    sim.ticks(10);
    assert!(!ids(&sim, p).contains(&AWP));
    assert_eq!(active_id(&sim, p), Some(AK47));
}

/// The local player's userinfo follows its console: `cl_autowepswitch`
/// (archived, default 1) is in its `UserInfo`.
#[test]
fn the_local_players_userinfo_follows_the_cvar() {
    let mut sim = sim();
    let p = sim.spawn_character(greybox::SPAWNS[0], placeholder::ID);
    sim.app.world_mut().entity_mut(p).insert(LocalPlayer);
    sim.ticks(2);
    let flag = |sim: &Sim| sim.app.world().get::<UserInfo>(p).and_then(|u| u.flag("cl_autowepswitch"));
    assert_eq!(flag(&sim), Some(true));
    let console = sim.app.world().resource::<Console>();
    let cvar = console.cvar("cl_autowepswitch").unwrap();
    assert!(cvar.archive && cvar.userinfo);
    assert_eq!(cvar.default, "1");
    sim.app.world_mut().resource_mut::<Console>().submit("cl_autowepswitch 0");
    sim.ticks(2);
    assert_eq!(flag(&sim), Some(false));
}
