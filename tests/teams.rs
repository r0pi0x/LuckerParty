//! Teams: friendly fire (CS:S `mp_friendlyfire`, off by default) and
//! `jointeam`, which respawns the local player at its new team's spawn.

use bevy::prelude::*;
use mashup::{
    console::Console,
    core::{Damage, Health, Hitgroup, LocalPlayer, SpawnPoint, Team},
    harness::Sim,
    movement::placeholder,
};

fn hit(sim: &mut Sim, attacker: Entity, target: Entity, amount: f32) {
    sim.app.world_mut().write_message(Damage {
        target,
        attacker: Some(attacker),
        amount,
        point: Vec3::ZERO,
        dir: Vec3::X,
        hitgroup: Hitgroup::Chest,
        kind: Default::default(),
    });
    sim.ticks(1);
}

fn health(sim: &Sim, e: Entity) -> f32 {
    sim.app.world().get::<Health>(e).unwrap().current
}

#[test]
fn teammates_do_not_hurt_each_other_unless_friendly_fire() {
    let mut sim = Sim::new(());
    let a = sim.spawn_character(Vec3::new(0.0, 1.0, 0.0), placeholder::ID);
    let b = sim.spawn_character(Vec3::new(3.0, 1.0, 0.0), placeholder::ID);
    let c = sim.spawn_character(Vec3::new(6.0, 1.0, 0.0), placeholder::ID);
    sim.app.world_mut().entity_mut(a).insert(Team(1));
    sim.app.world_mut().entity_mut(b).insert(Team(1));
    sim.app.world_mut().entity_mut(c).insert(Team(2));
    let full = health(&sim, b);
    hit(&mut sim, a, b, 0.3);
    assert_eq!(health(&sim, b), full, "teammate took damage");
    hit(&mut sim, a, c, 0.3);
    assert!(health(&sim, c) < full, "enemy took none");
    // Your own damage (falling) always counts.
    hit(&mut sim, b, b, 0.1);
    assert!(health(&sim, b) < full);
    sim.app
        .world_mut()
        .resource_mut::<Console>()
        .submit("mp_friendlyfire 1");
    sim.ticks(1);
    let before = health(&sim, a);
    hit(&mut sim, b, a, 0.2);
    assert!(health(&sim, a) < before, "friendly fire on, no damage");
}

#[test]
fn no_team_is_nobodys_teammate() {
    let mut sim = Sim::new(());
    let a = sim.spawn_character(Vec3::new(0.0, 1.0, 0.0), placeholder::ID);
    let b = sim.spawn_character(Vec3::new(3.0, 1.0, 0.0), placeholder::ID);
    // Both Team(0) from the harness.
    let full = health(&sim, b);
    hit(&mut sim, a, b, 0.3);
    assert!(health(&sim, b) < full);
}

#[test]
fn jointeam_respawns_at_the_new_teams_spawn() {
    let mut sim = Sim::new(());
    let t_spawn = Vec3::new(-20.0, 1.0, 0.0);
    let ct_spawn = Vec3::new(20.0, 1.0, 0.0);
    for (at, team) in [(t_spawn, 1), (ct_spawn, 2)] {
        sim.app
            .world_mut()
            .spawn((Transform::from_translation(at), SpawnPoint { team: Some(Team(team)) }));
    }
    let p = sim.spawn_character(Vec3::new(0.0, 1.0, 0.0), placeholder::ID);
    sim.app.world_mut().entity_mut(p).insert((LocalPlayer, Team(2)));
    sim.app.world_mut().resource_mut::<Console>().submit("jointeam 2");
    sim.ticks(3);
    assert_eq!(sim.app.world().get::<Team>(p), Some(&Team(1)));
    assert!(sim.position(p).distance(t_spawn) < 0.5, "at {}", sim.position(p));
    sim.app.world_mut().resource_mut::<Console>().submit("jointeam 3");
    sim.ticks(3);
    assert_eq!(sim.app.world().get::<Team>(p), Some(&Team(2)));
    assert!(sim.position(p).distance(ct_spawn) < 0.5, "at {}", sim.position(p));
}
