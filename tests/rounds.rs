//! Counter-Strike rounds (`mashup_rounds 1`): freeze, elimination and
//! time wins, money rewards, survivors keeping their weapons.

use bevy::prelude::*;
use mashup::{
    console::Console,
    core::{Damage, Hitgroup, Intent, Team},
    harness::Sim,
    movement::placeholder,
    rules::rounds::{Phase, RoundSettings, RoundState},
    weapon::economy::{BuyWindow, Money},
};

fn kill(sim: &mut Sim, attacker: Entity, target: Entity) {
    sim.app.world_mut().write_message(Damage {
        target,
        attacker: Some(attacker),
        amount: 10.0,
        point: Vec3::ZERO,
        dir: Vec3::X,
        hitgroup: Hitgroup::Chest,
        kind: Default::default(),
    });
    sim.ticks(2);
}

fn money(sim: &Sim, e: Entity) -> u32 {
    sim.app.world().get::<Money>(e).unwrap().0
}

fn phase(sim: &Sim) -> Phase {
    sim.app.world().resource::<RoundState>().phase
}

fn setup() -> (Sim, Entity, Entity, Entity) {
    let mut sim = Sim::new(());
    let t = sim.spawn_character(Vec3::new(0.0, 1.0, 0.0), placeholder::ID);
    let ct = sim.spawn_character(Vec3::new(4.0, 1.0, 0.0), placeholder::ID);
    let ct2 = sim.spawn_character(Vec3::new(8.0, 1.0, 0.0), placeholder::ID);
    sim.app.world_mut().entity_mut(t).insert(Team(1));
    sim.app.world_mut().entity_mut(ct).insert(Team(2));
    sim.app.world_mut().entity_mut(ct2).insert(Team(2));
    sim.app
        .world_mut()
        .resource_mut::<Console>()
        .submit("mp_freezetime 1; mp_roundtime 0.5; mashup_rounds 1");
    sim.ticks(3);
    (sim, t, ct, ct2)
}

#[test]
fn rounds_freeze_then_play_and_eliminations_pay() {
    let (mut sim, t, ct, ct2) = setup();
    assert!(matches!(phase(&sim), Phase::Freeze { .. }), "{:?}", phase(&sim));
    assert_eq!(money(&sim, t), 800);
    // No free rifle in rounds: a pistol and the knife (here none: Sim has
    // no weapons registered), bought up from there.
    assert!(sim.app.world().resource::<BuyWindow>().0.is_ok());
    // Frozen: walking does nothing.
    let start = sim.position(t);
    sim.intent(t).move_axis = Vec2::Y;
    sim.ticks(10);
    let moved = (sim.position(t) - start).with_y(0.0).length();
    assert!(moved < 0.01, "moved {moved} while frozen");
    sim.seconds(1.2);
    assert!(matches!(phase(&sim), Phase::Live { .. }), "{:?}", phase(&sim));

    // The terrorist kills one CT (+300), then the other: terrorists win.
    kill(&mut sim, t, ct);
    assert_eq!(money(&sim, t), 1100);
    assert!(matches!(phase(&sim), Phase::Live { .. }));
    kill(&mut sim, t, ct2);
    let winner = match phase(&sim) {
        Phase::Over { winner, .. } => winner,
        p => panic!("round not over: {p:?}"),
    };
    assert_eq!(winner, Some(Team(1)));
    assert_eq!(money(&sim, t), 1400 + 3250);
    assert_eq!(money(&sim, ct), 800 + 1400, "first loss bonus");
    assert_eq!(sim.app.world().resource::<RoundState>().wins, [1, 0]);

    // The next round brings everyone back, frozen.
    sim.seconds(5.2);
    assert!(matches!(phase(&sim), Phase::Freeze { .. }), "{:?}", phase(&sim));
    assert_eq!(sim.app.world().resource::<RoundState>().number, 2);
    assert_eq!(
        sim.app.world().resource::<mashup::core::RoundRestarts>().0,
        2,
        "each round start restarts the map's entities"
    );
    assert!(sim.app.world().get::<mashup::rules::Dead>(ct).is_none());
}

#[test]
fn defenders_win_when_time_runs_out_and_buying_closes() {
    let (mut sim, t, ct, _) = setup();
    let s = sim.app.world().resource::<RoundSettings>().clone();
    sim.seconds(1.2);
    // Buy period: 90 s by default, longer than this 30 s round; shorten it.
    sim.app.world_mut().resource_mut::<Console>().submit("mp_buytime 0.1");
    sim.seconds(6.5);
    assert!(sim.app.world().resource::<BuyWindow>().0.is_err(), "buy period over");
    sim.seconds(25.0);
    match phase(&sim) {
        Phase::Over { winner, .. } => assert_eq!(winner, Some(Team(2))),
        p => panic!("{p:?}"),
    }
    assert_eq!(money(&sim, ct), s.start_money + s.win_bonus);
    assert_eq!(money(&sim, t), s.start_money + s.loss_bonus);
    let _ = sim.app.world().get::<Intent>(t);
}
