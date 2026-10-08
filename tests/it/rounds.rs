//! Counter-Strike rounds (`mashup_rounds 1`): freeze, elimination and
//! time wins, money rewards, survivors keeping their weapons.

use bevy::prelude::*;
use mashup::{
    bot::Bot,
    console::Console,
    core::{Damage, FreezeTime, Hitgroup, Intent, Team},
    harness::Sim,
    movement::placeholder,
    rules::rounds::{Phase, RoundSettings, RoundState},
    weapon::economy::{BuyWindow, Money},
};

fn kill(sim: &mut Sim, attacker: Entity, target: Entity) {
    sim.app.world_mut().write_message(Damage {
        force: bevy::math::Vec3::ZERO,
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

/// Freeze time holds bots as it holds the player (CS:S freezes
/// everyone): a bot that sees an enemy, and would strafe, stays put
/// until the freeze ends.
#[test]
fn bots_are_held_in_the_freeze_time_too() {
    // On the greybox's floor, so walking moves.
    let mut sim = Sim::new(mashup::greybox::GreyboxMapPlugin);
    let places = [
        Vec3::new(0.0, 1.0, 25.0),
        Vec3::new(2.0, 1.0, 25.0),
        Vec3::new(2.0, 1.0, 15.0),
    ];
    let [t, bot, ct] = places.map(|at| sim.spawn_character(at, placeholder::ID));
    sim.app.world_mut().entity_mut(t).insert(Team(1));
    sim.app.world_mut().entity_mut(bot).insert((Team(1), Bot::default()));
    sim.app.world_mut().entity_mut(ct).insert(Team(2));
    sim.app
        .world_mut()
        .resource_mut::<Console>()
        .submit("mp_freezetime 2; mp_roundtime 0.5; mashup_rounds 1");
    sim.ticks(3);
    assert!(matches!(phase(&sim), Phase::Freeze { .. }), "{:?}", phase(&sim));
    assert!(sim.app.world().resource::<FreezeTime>().0);
    // The round put everyone at the spawns: back where the bot sees the
    // enemy.
    for (e, at) in [t, bot, ct].into_iter().zip(places) {
        sim.app.world_mut().get_mut::<Transform>(e).unwrap().translation = at;
    }
    sim.ticks(10);
    let flat = |sim: &Sim, e: Entity| sim.position(e).with_y(0.0);
    let start = [t, bot, ct].map(|e| flat(&sim, e));
    sim.intent(t).move_axis = Vec2::Y;
    sim.seconds(1.5);
    assert!(sim.app.world().get::<Bot>(bot).unwrap().target == Some(ct), "the bot sees the enemy");
    for (e, at) in [t, bot, ct].into_iter().zip(start) {
        let moved = flat(&sim, e).distance(at);
        assert!(moved < 0.01, "{e} moved {moved} while frozen");
    }
    // Once the round is live the bot strafes (the test would see it move).
    sim.seconds(1.0);
    assert!(matches!(phase(&sim), Phase::Live { .. }), "{:?}", phase(&sim));
    assert!(!sim.app.world().resource::<FreezeTime>().0);
    let moved = flat(&sim, bot).distance(start[1]);
    assert!(moved > 0.1, "the bot moved {moved} after the freeze");
}

/// The dead stop being solid (bots and players walk over the spot, a
/// ragdoll takes the body's place) until they respawn.
#[test]
fn the_dead_are_not_solid_until_the_next_round() {
    let (mut sim, t, ct, _) = setup();
    sim.seconds(1.2);
    kill(&mut sim, t, ct);
    sim.ticks(1);
    assert!(sim.app.world().get::<avian3d::prelude::ColliderDisabled>(ct).is_some());
    assert!(sim.app.world().get::<avian3d::prelude::ColliderDisabled>(t).is_none());
}

/// `mp_restartgame n` (the debug UI's "Restart game") starts the game
/// over n seconds later: round 1, no wins, the start money.
#[test]
fn restartgame_starts_over_after_its_delay() {
    let (mut sim, t, ct, ct2) = setup();
    sim.seconds(1.2);
    kill(&mut sim, t, ct);
    kill(&mut sim, t, ct2);
    assert_eq!(sim.app.world().resource::<RoundState>().wins, [1, 0]);
    assert_ne!(money(&sim, t), 800);
    sim.app.world_mut().resource_mut::<Console>().submit("mp_restartgame 1");
    sim.seconds(0.5);
    assert_eq!(sim.app.world().resource::<RoundState>().wins, [1, 0], "not yet");
    sim.seconds(0.7);
    let state = sim.app.world().resource::<RoundState>().clone();
    assert_eq!((state.number, state.wins, state.restart_at), (1, [0, 0], None));
    assert!(matches!(state.phase, Phase::Freeze { .. }), "{:?}", state.phase);
    assert_eq!(money(&sim, t), 800);
    assert!(sim.app.world().get::<mashup::rules::Dead>(ct).is_none(), "everyone is back");
}

/// Ammo in rounds (CS:S): the spawn pistol carries two more clips, a
/// bought gun only its clip, and ammo is bought by the box (`primammo`,
/// `secammo` fill; `buyammo1` buys one) up to the type's maximum; bots buy
/// theirs.
#[test]
fn rounds_ammo_is_bought_by_the_box() {
    use mashup::{
        core::LocalPlayer,
        games::cs_source::{
            TICK_INTERVAL,
            weapons::{CsWeaponsPlugin, GLOCK, M4A1, USP},
        },
        greybox::GreyboxMapPlugin,
        weapon::{Inventory, Magazine, Weapon, economy::buy},
    };
    let mut sim = Sim::new((GreyboxMapPlugin, CsWeaponsPlugin));
    sim.set_tick_interval(TICK_INTERVAL);
    let t = sim.spawn_character(Vec3::new(0.0, 1.0, 0.0), placeholder::ID);
    let ct = sim.spawn_character(Vec3::new(4.0, 1.0, 0.0), placeholder::ID);
    sim.app.world_mut().entity_mut(t).insert(Team(1));
    // The CT is the player; the terrorist shops as a bot.
    sim.app.world_mut().entity_mut(ct).insert((Team(2), LocalPlayer));
    sim.app
        .world_mut()
        .resource_mut::<Console>()
        .submit("mp_freezetime 5; mashup_rounds 1");
    sim.ticks(3);
    let gun = |sim: &Sim, e: Entity, id: &str| -> (u32, u32) {
        let w = sim.app.world();
        let inv = w.get::<Inventory>(e).unwrap();
        let g = inv
            .weapons
            .iter()
            .find(|g| w.get::<Weapon>(**g).is_some_and(|x| x.id == id))
            .unwrap_or_else(|| panic!("no {id}"));
        let m = w.get::<Magazine>(*g).unwrap();
        (m.clip, m.reserve)
    };
    // The bot: Glock 20 + 40, then 9 mm ($20 a box of 30, at most 120)
    // (3 boxes, the last partly: 60), kevlar for 650 of its 800.
    assert_eq!(gun(&sim, t, GLOCK), (20, 120));
    assert_eq!(money(&sim, t), 90);
    // The player: USP 12 + 24.
    assert_eq!(gun(&sim, ct, USP), (12, 24));
    assert_eq!(money(&sim, ct), 800);
    // The M4A1 (3100 of 16000): its clip, no reserve.
    sim.app.world_mut().entity_mut(ct).insert(Money(16000));
    buy(sim.app.world_mut(), ct, "m4a1").unwrap();
    assert_eq!(gun(&sim, ct, M4A1), (30, 0));
    // 5.56: $60 a box of 30, at most 90.
    buy(sim.app.world_mut(), ct, "buyammo1").unwrap();
    assert_eq!((gun(&sim, ct, M4A1).1, money(&sim, ct)), (30, 12840));
    buy(sim.app.world_mut(), ct, "primammo").unwrap();
    assert_eq!((gun(&sim, ct, M4A1).1, money(&sim, ct)), (90, 12720));
    assert_eq!(
        buy(sim.app.world_mut(), ct, "primammo").unwrap_err(),
        "You cannot carry any more."
    );
    // .45 ACP: $25 a box of 12, at most 100: 24 + 7 boxes (the last
    // partly).
    buy(sim.app.world_mut(), ct, "secammo").unwrap();
    assert_eq!((gun(&sim, ct, USP).1, money(&sim, ct)), (100, 12545));
}

#[test]
fn a_map_change_starts_a_fresh_game_and_brings_the_dead_back() {
    let (mut sim, t, ct, ct2) = setup();
    sim.ticks(80);
    kill(&mut sim, t, ct);
    assert!(sim.app.world().get::<mashup::rules::Dead>(ct).is_some());
    // What loading a map does: a new game, not the old round's clock.
    mashup::rules::new_game(sim.app.world_mut());
    sim.ticks(3);
    assert!(sim.app.world().get::<mashup::rules::Dead>(ct).is_none(), "the dead respawn on the new map");
    assert!(matches!(phase(&sim), Phase::Freeze { .. }), "{:?}", phase(&sim));
    assert_eq!(sim.app.world().resource::<RoundState>().number, 1);
    assert_eq!(money(&sim, ct2), 800);
}
