//! CS:S's target ID (`client::target_id`): the player under the crosshair
//! named in the game's words, a teammate with its health, an enemy
//! without; centred with `hud_centerid 1` (the default); none through
//! smoke, none for enemies with `mp_playerid 1`, nobody with 2.

use bevy::prelude::*;
use mashup::{
    client::target_id::{TargetId, TargetIdStatePlugin},
    console::Console,
    core::{Health, LocalPlayer, SightBlocker, Team},
    greybox::{self, GreyboxMapPlugin},
    harness::Sim,
    movement::placeholder,
};

fn sim() -> (Sim, Entity, Entity, Entity) {
    let mut sim = Sim::new((GreyboxMapPlugin, TargetIdStatePlugin));
    let at = greybox::SPAWNS[0];
    let me = sim.spawn_character(at, placeholder::ID);
    let mate = sim.spawn_character(at + Vec3::new(0.0, 0.0, -5.0), placeholder::ID);
    let foe = sim.spawn_character(at + Vec3::new(5.0, 0.0, 0.0), placeholder::ID);
    let w = sim.app.world_mut();
    w.entity_mut(me).insert((LocalPlayer, Team(2), Name::new("Me")));
    w.entity_mut(mate).insert((Team(2), Name::new("Mate")));
    w.entity_mut(foe).insert((Team(1), Name::new("Foe")));
    sim.ticks(30);
    (sim, me, mate, foe)
}

/// Turn `me` to look level at `target`'s middle.
fn look_at(sim: &mut Sim, me: Entity, target: Entity) {
    let d = sim.position(target) - sim.position(me);
    let mut i = sim.intent(me);
    // Yaw 0 looks down -Z.
    i.yaw = (-d.x).atan2(-d.z);
    i.pitch = -0.15;
    sim.ticks(2);
}

fn id(sim: &Sim) -> TargetId {
    sim.app.world().resource::<TargetId>().clone()
}

#[test]
fn a_teammate_is_named_with_its_health_an_enemy_without() {
    let (mut sim, me, mate, foe) = sim();
    sim.app.world_mut().get_mut::<Health>(mate).unwrap().current = 0.42;
    look_at(&mut sim, me, mate);
    let t = id(&sim);
    assert_eq!(t.target, Some(mate));
    assert_eq!(t.text, "Friend: Mate Health: 42%");
    assert_eq!(t.team, Some(Team(2)));
    assert!(t.centered, "hud_centerid 1 is the default");
    look_at(&mut sim, me, foe);
    let t = id(&sim);
    assert_eq!((t.target, t.text.as_str(), t.team), (Some(foe), "Enemy: Foe", Some(Team(1))));
    // hud_centerid 0: at the left.
    sim.app.world_mut().resource_mut::<Console>().submit("hud_centerid 0");
    sim.ticks(2);
    assert!(!id(&sim).centered);
    assert!(sim.app.world().resource::<Console>().cvar("hud_centerid").unwrap().archive);
    // Looking away: nothing.
    let mut i = sim.intent(me);
    i.pitch = 1.2;
    sim.ticks(2);
    assert_eq!(id(&sim).text, "");
}

#[test]
fn smoke_and_mp_playerid_hide_names() {
    let (mut sim, me, mate, foe) = sim();
    look_at(&mut sim, me, foe);
    assert_eq!(id(&sim).target, Some(foe));
    // A smoke cloud between.
    let between = (sim.position(me) + sim.position(foe)) / 2.0 + Vec3::Y * 0.6;
    let cloud = sim
        .app
        .world_mut()
        .spawn(SightBlocker {
            centre: between,
            radius: 1.5,
        })
        .id();
    sim.ticks(2);
    assert_eq!(id(&sim).target, None, "not through smoke");
    sim.app.world_mut().despawn(cloud);
    // mp_playerid 1: teammates only; 2: nobody.
    sim.app.world_mut().resource_mut::<Console>().submit("mp_playerid 1");
    sim.ticks(2);
    assert_eq!(id(&sim).target, None);
    look_at(&mut sim, me, mate);
    assert_eq!(id(&sim).target, Some(mate));
    sim.app.world_mut().resource_mut::<Console>().submit("mp_playerid 2");
    sim.ticks(2);
    assert_eq!(id(&sim).target, None);
}
