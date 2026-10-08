//! Spectating while dead in a rounds game: the death cam on the killer,
//! then living teammates (`mp_forcecamera 1`), moving on when the target
//! dies, anyone once the team is gone, back to playing at the next round.

use bevy::prelude::*;
use mashup::{
    client::spectate::{DEATH_CAM_SECONDS, SpecMode, SpecPhase, SpectateStatePlugin, Spectator, TARGET_DEATH_HOLD},
    console::Console,
    core::{Damage, Hitgroup, LocalPlayer, Team},
    harness::Sim,
    movement::placeholder,
    rules::rounds::{Phase, RoundState},
};

fn kill(sim: &mut Sim, attacker: Entity, target: Entity) {
    sim.app.world_mut().write_message(Damage {
        force: Vec3::ZERO,
        target,
        attacker: Some(attacker),
        amount: 10.0,
        point: Vec3::ZERO,
        dir: Vec3::X,
        hitgroup: Hitgroup::Chest,
        kind: Default::default(),
        weapon: None,
    });
    sim.ticks(2);
}

fn spec(sim: &Sim) -> Spectator {
    sim.app.world().resource::<Spectator>().clone()
}

fn console(sim: &mut Sim, line: &str) {
    sim.app.world_mut().resource_mut::<Console>().submit(line);
    sim.ticks(1);
}

#[test]
fn dead_players_watch_their_team_then_play_again() {
    let mut sim = Sim::new(SpectateStatePlugin);
    let add = |sim: &mut Sim, x: f32, team: u8, name: &str| {
        let e = sim.spawn_character(Vec3::new(x, 1.0, 0.0), placeholder::ID);
        sim.app.world_mut().entity_mut(e).insert((Team(team), Name::new(name.to_string())));
        e
    };
    let me = add(&mut sim, 0.0, 2, "Player");
    sim.app.world_mut().entity_mut(me).insert(LocalPlayer);
    let near = add(&mut sim, 3.0, 2, "Bot 1");
    let far = add(&mut sim, 9.0, 2, "Bot 2");
    let enemy = add(&mut sim, -2.0, 1, "Bot 3");
    console(&mut sim, "mp_freezetime 0; mp_roundtime 5; mashup_rounds 1");
    sim.seconds(0.5);
    assert!(matches!(sim.app.world().resource::<RoundState>().phase, Phase::Live { .. }));
    assert_eq!(spec(&sim).phase, SpecPhase::Alive);

    // Killed: the death cam on the killer; keys wait.
    kill(&mut sim, enemy, me);
    assert!(
        matches!(spec(&sim).phase, SpecPhase::DeathCam { killer: Some(k), .. } if k == enemy),
        "{:?}",
        spec(&sim).phase
    );
    console(&mut sim, "spec_next");
    assert!(spec(&sim).target.is_none());
    sim.seconds(DEATH_CAM_SECONDS);
    // Then the nearest teammate (not the nearer enemy), first person.
    let s = spec(&sim);
    assert_eq!(s.phase, SpecPhase::Watching);
    assert_eq!(s.target, Some(near));
    assert_eq!(s.effective_mode(), SpecMode::InEye);

    // Attack: the next teammate, around and back; never the enemy.
    console(&mut sim, "spec_next");
    assert_eq!(spec(&sim).target, Some(far));
    console(&mut sim, "spec_next");
    assert_eq!(spec(&sim).target, Some(near));
    // Jump: chase camera, free look, first person; or by number.
    console(&mut sim, "spec_mode");
    assert_eq!(spec(&sim).mode, SpecMode::Chase);
    console(&mut sim, "spec_mode 6");
    assert_eq!(spec(&sim).effective_mode(), SpecMode::Roaming);
    console(&mut sim, "spec_mode 4");

    // The watched teammate dies: on its body a moment, then the other.
    kill(&mut sim, enemy, near);
    assert_eq!(spec(&sim).target, Some(near));
    assert_eq!(spec(&sim).effective_mode(), SpecMode::Chase);
    sim.seconds(TARGET_DEATH_HOLD + 0.1);
    assert_eq!(spec(&sim).target, Some(far));

    // mp_forcecamera 0: the enemy may be watched too.
    console(&mut sim, "mp_forcecamera 0; spec_next");
    assert_eq!(spec(&sim).target, Some(enemy));
    console(&mut sim, "mp_forcecamera 1");
    sim.seconds(TARGET_DEATH_HOLD + 0.1);
    assert_eq!(spec(&sim).target, Some(far), "back on the team");

    // The last teammate dies: the round is lost and the enemy is all
    // that's left to watch.
    kill(&mut sim, enemy, far);
    sim.seconds(TARGET_DEATH_HOLD + 0.1);
    assert_eq!(spec(&sim).target, Some(enemy));

    // The next round: playing again, in first person.
    sim.seconds(10.0);
    assert!(!matches!(sim.app.world().resource::<RoundState>().phase, Phase::Over { .. }));
    let s = spec(&sim);
    assert_eq!(s.phase, SpecPhase::Alive);
    assert!(!s.active() && s.target.is_none());
}
