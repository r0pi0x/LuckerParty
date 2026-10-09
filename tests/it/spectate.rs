//! Spectating while dead in a rounds game: the death cam on the killer,
//! then living teammates (`mp_forcecamera 1`), moving on when the target
//! dies, anyone once the team is gone, back to playing at the next round.

use bevy::prelude::*;
use mashup::{
    client::spectate::{
        DEATH_CAM_SECONDS, FREEZE_HOLD, FREEZE_TRAVEL, SpecMode, SpecPhase, SpecView, SpectateCameraPlugin,
        SpectateStatePlugin, Spectator, TARGET_DEATH_HOLD,
    },
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
    // The freeze cam on the killer (keys still wait).
    assert!(
        matches!(spec(&sim).phase, SpecPhase::FreezeCam { killer, .. } if killer == enemy),
        "{:?}",
        spec(&sim).phase
    );
    sim.seconds((FREEZE_TRAVEL + FREEZE_HOLD) as f64);
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

/// A rounds game with the local player (CT) between a teammate 3 m away
/// and an enemy 8 m away, the camera's systems running.
fn freeze_sim() -> (Sim, Entity, Entity, Entity) {
    let mut sim = Sim::new((mashup::greybox::GreyboxMapPlugin, SpectateCameraPlugin));
    let add = |sim: &mut Sim, x: f32, team: u8, name: &str| {
        let e = sim.spawn_character(Vec3::new(x, 1.0, 12.0), placeholder::ID);
        sim.app.world_mut().entity_mut(e).insert((Team(team), Name::new(name.to_string())));
        e
    };
    let me = add(&mut sim, 0.0, 2, "Player");
    sim.app.world_mut().entity_mut(me).insert(LocalPlayer);
    let mate = add(&mut sim, 3.0, 2, "Bot 1");
    let enemy = add(&mut sim, -8.0, 1, "Bot 2");
    console(&mut sim, "mp_freezetime 0; mp_roundtime 5; mashup_rounds 1");
    sim.seconds(0.5);
    // The round put everyone at the spawns: back in a row.
    for (e, x) in [(me, 0.0), (mate, 3.0), (enemy, -8.0)] {
        let at = Vec3::new(x, 1.0, 12.0);
        let w = sim.app.world_mut();
        w.get_mut::<Transform>(e).unwrap().translation = at;
        if let Some(mut p) = w.get_mut::<avian3d::prelude::Position>(e) {
            p.0 = at;
        }
    }
    sim.seconds(0.2);
    (sim, me, mate, enemy)
}

/// CS:S's freeze cam: after the death cam the camera zooms toward the
/// killer (stopping 128 units from its eye, looking at it), holds, then
/// spectating starts; `cl_disablefreezecam 1` (archived) skips it.
#[test]
fn freeze_cam_turns_to_the_killer_then_spectating() {
    let (mut sim, me, mate, enemy) = freeze_sim();
    kill(&mut sim, enemy, me);
    sim.seconds(DEATH_CAM_SECONDS);
    assert!(matches!(spec(&sim).phase, SpecPhase::FreezeCam { killer, .. } if killer == enemy));
    sim.seconds(FREEZE_TRAVEL as f64 + 0.1);
    let (at, look) = sim.app.world().resource::<SpecView>().pose.expect("a camera pose");
    let foe = sim.position(enemy);
    let to = (foe - at).with_y(0.0);
    let facing = (look * Vec3::NEG_Z).with_y(0.0).normalize();
    assert!(facing.dot(to.normalize()) > 0.99, "looks at the killer: {facing} vs {to}");
    let gap = to.length();
    assert!((gap - 128.0 * 0.0254).abs() < 0.4, "framed 128 units away: {gap} m");
    // Held: the camera stays put.
    sim.seconds(1.0);
    assert_eq!(sim.app.world().resource::<SpecView>().pose.unwrap().0, at);
    sim.seconds(FREEZE_HOLD as f64);
    let s = spec(&sim);
    assert_eq!(s.phase, SpecPhase::Watching);
    assert_eq!(s.target, Some(mate));
}

#[test]
fn disabling_the_freeze_cam_goes_straight_to_spectating() {
    let (mut sim, me, mate, enemy) = freeze_sim();
    console(&mut sim, "cl_disablefreezecam 1");
    assert!(
        sim.app
            .world()
            .resource::<mashup::console::Console>()
            .cvar("cl_disablefreezecam")
            .unwrap()
            .archive
    );
    kill(&mut sim, enemy, me);
    sim.seconds(DEATH_CAM_SECONDS);
    let s = spec(&sim);
    assert_eq!(s.phase, SpecPhase::Watching);
    assert_eq!(s.target, Some(mate));
}

#[test]
fn no_freeze_cam_on_a_dead_killer() {
    let (mut sim, me, mate, enemy) = freeze_sim();
    kill(&mut sim, enemy, me);
    // The killer dies during the death cam.
    kill(&mut sim, mate, enemy);
    sim.seconds(DEATH_CAM_SECONDS);
    assert_eq!(spec(&sim).phase, SpecPhase::Watching);
}

/// The freeze cam on a network client: the server's death message names
/// the killer (mapped to the client's entity for it), so the client's
/// camera turns to it, freezes, then spectates.
#[test]
fn a_network_clients_freeze_cam_finds_its_killer() {
    use mashup::{greybox::GreyboxMapPlugin, harness::NetSim, net::memory::LinkConditions};
    let mut sim = NetSim::new(LinkConditions::default(), 61, 2, |app| {
        app.add_plugins((GreyboxMapPlugin, SpectateStatePlugin));
    });
    sim.until_joined(600);
    sim.ticks(120);
    let victim = sim.character_of(0).unwrap();
    let killer = sim.character_of(1).unwrap();
    // No respawn while it plays out.
    sim.server
        .app
        .world_mut()
        .resource_mut::<mashup::console::Console>()
        .submit("mp_respawn_delay 30");
    sim.ticks(2);
    sim.server.app.world_mut().write_message(Damage {
        force: Vec3::ZERO,
        target: victim,
        attacker: Some(killer),
        amount: 10.0,
        point: Vec3::ZERO,
        dir: Vec3::X,
        hitgroup: Hitgroup::Chest,
        kind: Default::default(),
        weapon: None,
    });
    // The client's view of its killer: client 1's character there.
    let id1 = sim.client_id(1);
    let theirs = NetSim::owned_by(&mut sim.clients[0], Some(id1)).unwrap();
    let ok = sim.until(120, |s| {
        matches!(s.clients[0].app.world().resource::<Spectator>().phase, SpecPhase::DeathCam { .. })
    });
    let phase = |s: &NetSim| s.clients[0].app.world().resource::<Spectator>().phase;
    assert!(ok, "the client's death cam: {:?}", phase(&sim));
    assert!(matches!(phase(&sim), SpecPhase::DeathCam { killer: Some(k), .. } if k == theirs));
    let ok = sim.until(400, |s| matches!(phase(s), SpecPhase::FreezeCam { .. }));
    assert!(ok, "{:?}", phase(&sim));
    assert!(matches!(phase(&sim), SpecPhase::FreezeCam { killer, .. } if killer == theirs));
    let ok = sim.until(600, |s| phase(s) == SpecPhase::Watching);
    assert!(ok, "{:?}", phase(&sim));
}
