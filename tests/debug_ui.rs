//! The debug UI's controls are console lines: opening it, switching
//! tabs and running what its Player, Bots, Logic and Rounds tabs run
//! leave the simulation ticking and do what they say (headless; the
//! drawing needs a window).

use bevy::prelude::*;
use mashup::{
    client::debug_ui::{DebugUi, DebugUiStatePlugin, Tab},
    console::Console,
    core::{Health, LocalPlayer, SimTick, Team},
    harness::Sim,
    movement::placeholder,
    rules::rounds::RoundState,
    weapon::{Armor, economy::Money},
};

fn run(sim: &mut Sim, line: &str) {
    sim.app.world_mut().resource_mut::<Console>().submit(line);
    sim.ticks(2);
}

#[test]
fn debug_ui_lines_leave_the_sim_running() {
    let mut sim = Sim::new(DebugUiStatePlugin);
    let p = sim.spawn_character(Vec3::new(0.0, 1.0, 0.0), placeholder::ID);
    let bot_side = sim.spawn_character(Vec3::new(4.0, 1.0, 0.0), placeholder::ID);
    sim.app.world_mut().entity_mut(p).insert((LocalPlayer, Team(2)));
    sim.app.world_mut().entity_mut(bot_side).insert(Team(1));
    let tick = |sim: &Sim| sim.app.world().resource::<SimTick>().0;

    // Open, switch through every tab, close: the ticks go on.
    let start = tick(&sim);
    run(&mut sim, "debugui");
    for t in Tab::ALL {
        run(&mut sim, &format!("debugui {}", t.name()));
        assert_eq!(sim.app.world().resource::<DebugUi>().tab, t);
    }
    assert!(sim.app.world().resource::<DebugUi>().open);
    run(&mut sim, "debugui close");
    assert!(!sim.app.world().resource::<DebugUi>().open);
    assert!(tick(&sim) >= start + 2 * (Tab::ALL.len() as u64 + 2));

    // The Player tab's setters.
    run(&mut sim, "mashup_sethealth 42; mashup_setarmor 60 1; mashup_setmoney 5000");
    let w = sim.app.world();
    assert!((w.get::<Health>(p).unwrap().current - 0.42).abs() < 1e-4);
    let a = w.get::<Armor>(p).unwrap();
    assert!((a.amount - 0.6).abs() < 1e-4 && a.helmet);
    assert_eq!(w.get::<Money>(p).unwrap().0, 5000);

    // Rounds tab: rounds on, then a restart a second later.
    run(&mut sim, "mp_freezetime 0; mashup_rounds 1; mashup_logic_record 1");
    run(&mut sim, "mashup_setmoney 9000; mp_restartgame 1");
    assert_eq!(sim.app.world().get::<Money>(p).unwrap().0, 9000);
    sim.seconds(1.2);
    assert_eq!(sim.app.world().get::<Money>(p).unwrap().0, 800, "the restart gave the start money");
    assert_eq!(sim.app.world().resource::<RoundState>().number, 1);
    let before = tick(&sim);
    sim.ticks(10);
    assert_eq!(tick(&sim), before + 10);
}
