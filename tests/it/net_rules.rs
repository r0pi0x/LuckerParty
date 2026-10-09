//! Network play, slice 5 (docs/plans/active/multiplayer.md): the game's
//! rules over `NetSim`'s in-memory link, on the greybox with CS:S's
//! weapons and rounds.
//!
//! - A full round with two clients: the freeze, buying, the round going
//!   live, a win by elimination, the money it pays and both clients'
//!   scoreboards.
//! - Buying is the server's to refuse: outside a buy zone, after the buy
//!   time, without the money, the other team's weapons.
//! - The bomb planted by one client and defused by another, the clocks on
//!   both clients matching the server's.
//! - Chat and team chat reach exactly who reads them; radio calls only
//!   teammates.
//! - A server cvar change (`sv_airaccelerate`) reaches the clients, which
//!   can't change it back, and prediction stays exact with it.
//! - A player joining mid-round spectates others until the next round.
//!
//! `cargo test --features dev --test it net_rules -- --nocapture` prints
//! the numbers.

use std::{sync::Arc, time::Duration};

use avian3d::prelude::Position;
use bevy::{
    ecs::message::{MessageCursor, Messages},
    prelude::*,
};
use mashup::{
    client::{chat::ChatLine, spectate::{SpecPhase, SpectateStatePlugin, Spectator}},
    console::Console,
    core::{Damage, Hitgroup, Intent, LocalPlayer, Team},
    games::cs_source::{
        TICK_INTERVAL,
        movement::{self as source, SourceMovementPlugin},
        weapons::{CsWeaponsPlugin, DEAGLE},
    },
    greybox::GreyboxMapPlugin,
    harness::NetSim,
    map::{
        MapEntities,
        entities::{MapEntity, MapHull, engine_to_entity},
    },
    net::{
        ChatMessage, NetScore, Notice, RadioCall, RadioRequest,
        memory::LinkConditions,
        predict::NetGraph,
    },
    objectives::{
        MapObjectives, ObjectiveEvent, Zone,
        bomb::{Arming, BombOutcome, BombState, C4, PlantedBomb},
    },
    rules::{
        Dead, Score,
        rounds::{Phase, RoundEndReason, RoundEnded, RoundSettings, RoundState},
    },
    slots::Loadout,
    weapon::{
        Inventory, Weapon,
        economy::{Money, NOT_IN_BUY_ZONE},
    },
};

/// The bomb target: a box around the greybox's first spawn.
const ZONE: (Vec3, Vec3) = (Vec3::new(-3.0, -1.0, 8.0), Vec3::new(3.0, 3.0, 16.0));

fn setup(app: &mut App) {
    app.add_plugins((GreyboxMapPlugin, SourceMovementPlugin, CsWeaponsPlugin))
        .add_plugins(mashup::client::chat::GameMessagesPlugin)
        .add_plugins(SpectateStatePlugin)
        .insert_resource(Loadout { movement: source::ID })
        .insert_resource(MapObjectives {
            bomb_targets: vec![Zone::engine_box(ZONE.0, ZONE.1)],
            ..default()
        })
        .insert_resource(Time::<Fixed>::from_seconds(TICK_INTERVAL));
}

fn link(latency_ms: u64, jitter_ms: u64, loss: f64) -> LinkConditions {
    LinkConditions {
        latency: Duration::from_millis(latency_ms),
        jitter: Duration::from_millis(jitter_ms),
        loss,
    }
}

fn server_console(sim: &mut NetSim, line: &str) {
    sim.server.app.world_mut().resource_mut::<Console>().submit(line);
}

fn client_console(sim: &mut NetSim, i: usize, line: &str) {
    sim.clients[i].app.world_mut().resource_mut::<Console>().submit(line);
}

/// A server with rounds on (`settings` on top) and `clients` joined,
/// named "Player 1", "Player 2", ...
fn rounds(conditions: LinkConditions, seed: u64, clients: usize, settings: &str) -> NetSim {
    let mut sim = NetSim::new(conditions, seed, 0, setup);
    for i in 0..clients {
        sim.add_client(|w| w.resource_mut::<mashup::net::NetSettings>().name = format!("Player {}", i + 1));
    }
    server_console(
        &mut sim,
        &format!("mp_freezetime 2; mp_roundtime 1; mp_c4timer 20; mashup_rounds 1; {settings}"),
    );
    sim.until_joined(600);
    sim.ticks(60);
    sim
}

/// `mp_restartgame 1`, stepped until the new game has started (its
/// first round's freeze).
fn restart(sim: &mut NetSim) {
    server_console(sim, "mp_restartgame 1");
    sim.ticks(2);
    let ok = sim.until(400, |s| {
        let r = s.server.app.world().resource::<RoundState>();
        r.restart_at.is_none() && matches!(r.phase, Phase::Freeze { .. })
    });
    assert!(ok, "the game restarted");
}

fn phase_of(world: &World) -> Phase {
    world.resource::<RoundState>().phase
}

fn server_phase(sim: &NetSim) -> Phase {
    phase_of(sim.server.app.world())
}

fn client_phase(sim: &NetSim, i: usize) -> Phase {
    phase_of(sim.clients[i].app.world())
}

/// Step until the server is in a phase like `want` (by its variant).
fn until_phase(sim: &mut NetSim, max: u64, want: fn(&Phase) -> bool) {
    assert!(
        sim.until(max, |s| want(&server_phase(s))),
        "server phase {:?} after {max} ticks",
        server_phase(sim)
    );
}

fn money_of(sim: &mut NetSim, i: usize) -> Option<u32> {
    let me = sim.local_player(i)?;
    sim.clients[i].app.world().get::<Money>(me).map(|m| m.0)
}

fn server_money(sim: &mut NetSim, i: usize) -> u32 {
    let e = sim.character_of(i).unwrap();
    sim.server.app.world().get::<Money>(e).unwrap().0
}

fn team_of(sim: &mut NetSim, i: usize) -> u8 {
    let e = sim.character_of(i).unwrap();
    sim.server.app.world().get::<Team>(e).unwrap().0
}

/// The server puts client `i`'s character at `at`.
fn place(sim: &mut NetSim, i: usize, at: Vec3) {
    let e = sim.character_of(i).unwrap();
    let world = sim.server.app.world_mut();
    world.get_mut::<Transform>(e).unwrap().translation = at;
    if let Some(mut p) = world.get_mut::<Position>(e) {
        p.0 = at;
    }
}

/// The server kills client `victim`'s character, by `attacker`'s.
fn kill(sim: &mut NetSim, attacker: usize, victim: usize) {
    let a = sim.character_of(attacker).unwrap();
    let v = sim.character_of(victim).unwrap();
    sim.server.app.world_mut().write_message(Damage {
        force: Vec3::ZERO,
        target: v,
        attacker: Some(a),
        amount: 10.0,
        point: Vec3::ZERO,
        dir: Vec3::X,
        hitgroup: Hitgroup::Chest,
        kind: default(),
        weapon: None,
    });
}

fn client_intent(sim: &mut NetSim, i: usize) -> Mut<'_, Intent> {
    let me = sim.local_player(i).unwrap();
    sim.clients[i].app.world_mut().get_mut::<Intent>(me).unwrap()
}

/// The messages of type `M` one app gets, gathered after every step.
struct Log<M: Message> {
    /// 0: the server; i + 1: client i.
    app: usize,
    cursor: MessageCursor<M>,
    all: Vec<M>,
}

fn world_of(sim: &NetSim, app: usize) -> &World {
    if app == 0 {
        sim.server.app.world()
    } else {
        sim.clients[app - 1].app.world()
    }
}

impl<M: Message + Clone> Log<M> {
    fn new(sim: &NetSim, app: usize) -> Self {
        Self {
            app,
            cursor: world_of(sim, app).resource::<Messages<M>>().get_cursor_current(),
            all: Vec::new(),
        }
    }

    fn poll(&mut self, sim: &NetSim) {
        let world = world_of(sim, self.app);
        self.all.extend(self.cursor.read(world.resource::<Messages<M>>()).cloned());
    }
}

/// Weapon ids client `i` carries (its predicted copy).
fn carried(sim: &mut NetSim, i: usize) -> Vec<&'static str> {
    let me = sim.local_player(i).unwrap();
    let w = sim.clients[i].app.world();
    w.get::<Inventory>(me)
        .map(|inv| inv.weapons.iter().filter_map(|e| w.get::<Weapon>(*e).map(|x| x.id)).collect())
        .unwrap_or_default()
}

/// Each client's scoreboard values for every character: (name, kills,
/// deaths, ping) as its world has them.
fn board(sim: &mut NetSim, i: usize) -> Vec<(String, u32, u32, u16)> {
    let w = sim.clients[i].app.world_mut();
    let mut rows: Vec<_> = w
        .query::<(&mashup::net::NetCharacter, Option<&Score>, Option<&NetScore>)>()
        .iter(w)
        .map(|(c, s, n)| {
            let s = s.copied().unwrap_or_default();
            (c.name.clone(), s.kills, s.deaths, n.map_or(0, |n| n.ping))
        })
        .collect();
    rows.sort();
    rows
}

#[test]
fn a_full_round_with_two_clients() {
    let mut sim = rounds(link(50, 5, 0.0), 51, 2, "");
    // Teams by who joined first: the first a counter-terrorist (a tie),
    // the second a terrorist.
    assert_eq!((team_of(&mut sim, 0), team_of(&mut sim, 1)), (2, 1));
    // The next round (everyone at a spawn with the start money), frozen.
    restart(&mut sim);
    let mut ended = [Log::<RoundEnded>::new(&sim, 1), Log::<RoundEnded>::new(&sim, 2)];
    sim.ticks(30);
    for i in 0..2 {
        assert!(matches!(client_phase(&sim, i), Phase::Freeze { .. }), "client {i}: {:?}", client_phase(&sim, i));
        assert_eq!(money_of(&mut sim, i), Some(800), "client {i}'s start money");
    }
    // The round clock: each client's matches the server's, less the time
    // its updates take to arrive.
    let server_left = sim.server.app.world().resource::<RoundState>().clock(
        sim.server.app.world().resource::<Time<Fixed>>().elapsed_secs_f64(),
    );
    for i in 0..2 {
        let w = sim.clients[i].app.world();
        let left = w.resource::<RoundState>().clock(w.resource::<Time<Fixed>>().elapsed_secs_f64());
        let (s, c) = (server_left.unwrap(), left.unwrap());
        println!("freeze left: server {s:.3} s, client {i} {c:.3} s");
        assert!((c - s).abs() < 0.12, "client {i}'s freeze clock {c} vs the server's {s}");
    }
    // Buying in the freeze: the counter-terrorist buys a Desert Eagle.
    client_console(&mut sim, 0, "buy deagle");
    sim.ticks(30);
    assert_eq!(server_money(&mut sim, 0), 150);
    assert_eq!(money_of(&mut sim, 0), Some(150), "the client's money follows");
    assert!(carried(&mut sim, 0).contains(&DEAGLE), "{:?}", carried(&mut sim, 0));
    // Live: the counter-terrorist kills the terrorist.
    until_phase(&mut sim, 400, |p| matches!(p, Phase::Live { .. }));
    sim.ticks(20);
    for i in 0..2 {
        assert!(matches!(client_phase(&sim, i), Phase::Live { .. }), "client {i}: {:?} (server {:?})", client_phase(&sim, i), server_phase(&sim));
    }
    kill(&mut sim, 0, 1);
    for _ in 0..40 {
        sim.step();
        for l in &mut ended {
            l.poll(&sim);
        }
    }
    match server_phase(&sim) {
        Phase::Over { winner, .. } => assert_eq!(winner, Some(Team(2))),
        p => panic!("{p:?}"),
    }
    let s = sim.server.app.world().resource::<RoundSettings>().clone();
    for (i, l) in ended.iter().enumerate() {
        assert_eq!(
            l.all,
            vec![RoundEnded {
                winner: Some(Team(2)),
                reason: RoundEndReason::Eliminated
            }],
            "client {i} heard the round end"
        );
        match client_phase(&sim, i) {
            Phase::Over { winner, .. } => assert_eq!(winner, Some(Team(2))),
            p => panic!("client {i}: {p:?}"),
        }
        assert_eq!(sim.clients[i].app.world().resource::<RoundState>().wins, [0, 1]);
    }
    // Money: the kill and the win for one, the loss bonus for the other.
    assert_eq!(money_of(&mut sim, 0), Some(150 + s.kill_reward + s.win_bonus));
    assert_eq!(money_of(&mut sim, 1), Some(800 + s.loss_bonus));
    // The other client sees nobody's money but its own.
    let id = sim.client_id(0);
    let other = NetSim::owned_by(&mut sim.clients[1], Some(id)).unwrap();
    assert!(sim.clients[1].app.world().get::<Money>(other).is_none());
    // Scores on both scoreboards, with ping (measured every 64 ticks).
    sim.ticks(70);
    let boards = [board(&mut sim, 0), board(&mut sim, 1)];
    println!("scoreboards: {boards:?}");
    assert_eq!(boards[0].iter().map(|r| (r.1, r.2)).collect::<Vec<_>>(), boards[1].iter().map(|r| (r.1, r.2)).collect::<Vec<_>>());
    let ct = &boards[0].iter().find(|r| r.0 == "Player 1").unwrap();
    let t = &boards[0].iter().find(|r| r.0 == "Player 2").unwrap();
    assert_eq!((ct.1, ct.2), (1, 0));
    assert_eq!((t.1, t.2), (0, 1));
    for r in &boards[1] {
        assert!((90..=140).contains(&r.3), "{} pings {} ms at 50 ms each way", r.0, r.3);
    }
    // The dead terrorist is dead on both clients until the next round.
    let id = sim.client_id(1);
    let t_on_0 = NetSim::owned_by(&mut sim.clients[0], Some(id)).unwrap();
    assert!(sim.clients[0].app.world().get::<Dead>(t_on_0).is_some());
    until_phase(&mut sim, 600, |p| matches!(p, Phase::Freeze { .. }));
    sim.ticks(30);
    for i in 0..2 {
        assert!(matches!(client_phase(&sim, i), Phase::Freeze { .. }));
        assert_eq!(sim.clients[i].app.world().resource::<RoundState>().number, sim.server.app.world().resource::<RoundState>().number);
        let me = sim.local_player(i).unwrap();
        assert!(sim.clients[i].app.world().get::<Dead>(me).is_none(), "client {i} plays the new round");
    }
    // The survivor kept its Desert Eagle.
    assert!(carried(&mut sim, 0).contains(&DEAGLE));
}

/// A buy zone (func_buyzone) for both teams around `lo`..`hi` (engine
/// space), on the server.
fn buy_zone(lo: Vec3, hi: Vec3) -> MapEntities {
    let scale = mashup::objectives::UNIT;
    let (a, b) = (engine_to_entity(lo, scale), engine_to_entity(hi, scale));
    let (lo, hi) = (a.min(b), a.max(b));
    let hull = MapHull {
        planes: vec![
            (Vec3::X, hi.x),
            (Vec3::NEG_X, -lo.x),
            (Vec3::Y, hi.y),
            (Vec3::NEG_Y, -lo.y),
            (Vec3::Z, hi.z),
            (Vec3::NEG_Z, -lo.z),
        ],
        points: vec![lo, hi],
    };
    MapEntities {
        entities: Arc::new(vec![MapEntity {
            keyvalues: vec![("classname".into(), "func_buyzone".into())],
            hulls: vec![hull],
            mover: false,
        }]),
        scale,
    }
}

#[test]
fn buying_is_refused_outside_the_zone_the_time_and_the_money() {
    let mut sim = rounds(link(50, 0, 0.0), 52, 1, "mp_freezetime 1; mp_buytime 0.05");
    let zones = buy_zone(Vec3::new(-6.0, -1.0, -6.0), Vec3::new(6.0, 4.0, 6.0));
    sim.server.app.world_mut().insert_resource(zones.clone());
    restart(&mut sim);
    let mut notices = Log::<Notice>::new(&sim, 1);
    let wait = |sim: &mut NetSim, n: u32, notices: &mut Log<Notice>| {
        for _ in 0..n {
            sim.step();
            notices.poll(sim);
        }
    };
    // Outside the zone.
    place(&mut sim, 0, Vec3::new(0.0, 1.0, 12.0));
    wait(&mut sim, 10, &mut notices);
    client_console(&mut sim, 0, "buy deagle");
    wait(&mut sim, 20, &mut notices);
    assert_eq!(server_money(&mut sim, 0), 800);
    assert_eq!(notices.all.pop().map(|n| n.text).as_deref(), Some(NOT_IN_BUY_ZONE));
    // Inside: too dear, then the other team's rifle.
    place(&mut sim, 0, Vec3::new(0.0, 1.0, 0.0));
    wait(&mut sim, 10, &mut notices);
    client_console(&mut sim, 0, "buy awp");
    wait(&mut sim, 20, &mut notices);
    assert_eq!(notices.all.pop().map(|n| n.text).as_deref(), Some("You have insufficient funds."));
    let rifle = if team_of(&mut sim, 0) == 2 { "buy ak47" } else { "buy m4a1" };
    client_console(&mut sim, 0, rifle);
    wait(&mut sim, 20, &mut notices);
    assert_eq!(notices.all.pop().map(|n| n.text).as_deref(), Some("Your team can't buy that weapon."));
    // What it can afford, in the zone and the time: bought.
    client_console(&mut sim, 0, "buy deagle");
    wait(&mut sim, 20, &mut notices);
    assert!(notices.all.is_empty(), "{:?}", notices.all);
    assert_eq!(server_money(&mut sim, 0), 150);
    assert_eq!(money_of(&mut sim, 0), Some(150));
    // After the buy time (the freeze, then 3 s): refused, in the game's
    // words, and the client's buy window says so too.
    until_phase(&mut sim, 400, |p| matches!(p, Phase::Live { .. }));
    wait(&mut sim, 300, &mut notices);
    client_console(&mut sim, 0, "buy vest");
    wait(&mut sim, 20, &mut notices);
    let why = notices.all.pop().map(|n| n.text).unwrap_or_default();
    assert!(why.contains("You can't buy anything now."), "{why}");
    assert!(sim.clients[0].app.world().resource::<mashup::weapon::economy::BuyWindow>().0.is_err());
    assert_eq!(server_money(&mut sim, 0), 150);
}

/// The client's and the server's seconds left on the planted bomb.
fn bomb_left(sim: &mut NetSim) -> (f64, Vec<f64>) {
    let left = |w: &mut World| -> Option<f64> {
        let now = w.resource::<Time<Fixed>>().elapsed_secs_f64();
        w.query::<&PlantedBomb>().iter(w).next().map(|b| b.explode_at - now)
    };
    let s = left(sim.server.app.world_mut()).unwrap();
    let c = (0..sim.clients.len()).filter_map(|i| left(sim.clients[i].app.world_mut())).collect();
    (s, c)
}

#[test]
fn the_bomb_is_planted_and_defused_over_the_network() {
    let mut sim = rounds(link(50, 5, 0.0), 53, 2, "mp_freezetime 1");
    // Client 1 is the terrorist: at the next round it gets the bomb.
    assert_eq!(team_of(&mut sim, 1), 1);
    restart(&mut sim);
    until_phase(&mut sim, 400, |p| matches!(p, Phase::Live { .. }));
    sim.ticks(30);
    assert!(carried(&mut sim, 1).iter().any(|id| id.ends_with("weapon_c4")), "{:?}", carried(&mut sim, 1));
    // Into the target, the bomb drawn (its key, 5: slot 4), fire held.
    place(&mut sim, 1, Vec3::new(0.0, 1.0, 12.0));
    place(&mut sim, 0, Vec3::new(0.0, 1.0, -20.0));
    sim.ticks(10);
    client_intent(&mut sim, 1).select = Some(4);
    sim.ticks(2);
    client_intent(&mut sim, 1).select = None;
    sim.ticks(80);
    let mut news = Log::<ObjectiveEvent>::new(&sim, 1);
    let mut news0 = Log::<ObjectiveEvent>::new(&sim, 2);
    client_intent(&mut sim, 1).fire = true;
    let mut armed_seen = false;
    for _ in 0..260 {
        sim.step();
        news.poll(&sim);
        news0.poll(&sim);
        let me = sim.local_player(1).unwrap();
        armed_seen |= sim.clients[1].app.world().get::<Arming>(me).is_some();
        if sim.server.app.world().resource::<BombState>().planted.is_some() {
            break;
        }
    }
    client_intent(&mut sim, 1).fire = false;
    assert!(sim.server.app.world().resource::<BombState>().planted.is_some(), "planted");
    assert!(armed_seen, "the planter's client showed its arming progress");
    for _ in 0..30 {
        sim.step();
        news.poll(&sim);
        news0.poll(&sim);
    }
    for i in 0..2 {
        assert!(sim.clients[i].app.world().resource::<BombState>().planted.is_some(), "client {i} sees the planted bomb");
    }
    let (s, c) = bomb_left(&mut sim);
    println!("bomb: server {s:.3} s left, clients {c:?}");
    assert_eq!(c.len(), 2);
    for x in &c {
        assert!((x - s).abs() < 0.12, "client clock {x} vs server {s}");
    }
    // The counter-terrorist walks up and defuses it (no kit: 10 s).
    let (bomb_at, _) = {
        let w = sim.server.app.world_mut();
        let t = w.query_filtered::<&Transform, With<PlantedBomb>>().iter(w).next().unwrap().translation;
        (t, ())
    };
    place(&mut sim, 0, bomb_at + Vec3::new(0.0, 1.0, 0.9));
    place(&mut sim, 1, Vec3::new(0.0, 1.0, -25.0));
    for _ in 0..40 {
        sim.step();
        news.poll(&sim);
        news0.poll(&sim);
    }
    let me = sim.character_of(0).unwrap();
    let eye = {
        let w = sim.server.app.world();
        w.get::<Transform>(me).unwrap().translation + w.get::<mashup::core::MovementState>(me).unwrap().eye_offset
    };
    let d = bomb_at + Vec3::Y * 0.05 - eye;
    {
        let mut i = client_intent(&mut sim, 0);
        i.yaw = (-d.x).atan2(-d.z).rem_euclid(std::f32::consts::TAU);
        i.pitch = d.y.atan2(d.xz().length());
        i.use_key = true;
    }
    let mut defusing_seen = 0;
    let mut ends: Vec<(f64, f64)> = Vec::new();
    for _ in 0..800 {
        sim.step();
        news.poll(&sim);
        news0.poll(&sim);
        let local0 = sim.local_player(0).unwrap();
        let w = sim.clients[0].app.world_mut();
        let now = w.resource::<Time<Fixed>>().elapsed_secs_f64();
        if let Some(d) = w.query::<&PlantedBomb>().iter(w).next().and_then(|b| b.defuse) {
            if d.who == local0 {
                defusing_seen += 1;
                ends.push((d.ends - now, 0.0));
            }
        }
        let sw = sim.server.app.world_mut();
        let snow = sw.resource::<Time<Fixed>>().elapsed_secs_f64();
        if let (Some(last), Some(d)) = (ends.last_mut(), sw.query::<&PlantedBomb>().iter(sw).next().and_then(|b| b.defuse)) {
            last.1 = d.ends - snow;
        }
        if sim.server.app.world().resource::<BombState>().outcome.is_some() {
            break;
        }
    }
    client_intent(&mut sim, 0).use_key = false;
    assert_eq!(sim.server.app.world().resource::<BombState>().outcome, Some(BombOutcome::Defused));
    assert!(defusing_seen > 100, "the defuser's client showed its progress ({defusing_seen} frames)");
    let worst = ends.iter().filter(|(_, s)| *s > 0.0).map(|(c, s)| (c - s).abs()).fold(0.0, f64::max);
    println!("defuse: {} frames compared, client vs server time left worst {worst:.3} s", ends.len());
    assert!(worst < 0.12, "defuse clocks differ by {worst} s");
    for _ in 0..30 {
        sim.step();
        news.poll(&sim);
        news0.poll(&sim);
    }
    for i in 0..2 {
        let w = sim.clients[i].app.world();
        assert_eq!(w.resource::<BombState>().outcome, Some(BombOutcome::Defused), "client {i}");
        match phase_of(w) {
            Phase::Over { winner, .. } => assert_eq!(winner, Some(Team(2))),
            p => panic!("client {i}: {p:?}"),
        }
    }
    // Both clients heard the plant, beeps and the defuse.
    for (i, l) in [&news, &news0].into_iter().enumerate() {
        let planted = l.all.iter().filter(|e| matches!(e, ObjectiveEvent::Planted { .. })).count();
        let beeps = l.all.iter().filter(|e| matches!(e, ObjectiveEvent::Beep { .. })).count();
        let defused = l.all.iter().filter(|e| matches!(e, ObjectiveEvent::Defused { .. })).count();
        println!("client {i}: planted {planted}, beeps {beeps}, defused {defused}");
        assert_eq!((planted, defused), (1, 1), "client {i}");
        assert!(beeps > 5, "client {i}");
    }
    let s = sim.server.app.world().resource::<RoundSettings>().clone();
    // The planter's 300, then the loss bonus and the plant bonus.
    assert_eq!(money_of(&mut sim, 1), Some(800 + s.planter_reward + s.loss_bonus + s.plant_loss_bonus));
    assert_eq!(money_of(&mut sim, 0), Some(800 + s.win_bomb));
}

/// Chat lines each client got (sender name, text).
fn heard(l: &Log<ChatMessage>) -> Vec<(String, String)> {
    l.all.iter().map(|m| (m.name.clone(), m.text.clone())).collect()
}

#[test]
fn chat_and_radio_reach_who_reads_them() {
    let mut sim = rounds(link(40, 0, 0.0), 54, 3, "");
    // Counter-terrorists 0 and 2, terrorist 1.
    assert_eq!((team_of(&mut sim, 0), team_of(&mut sim, 1), team_of(&mut sim, 2)), (2, 1, 2));
    // Everyone in play (they joined after the first round began).
    restart(&mut sim);
    until_phase(&mut sim, 400, |p| matches!(p, Phase::Live { .. }));
    let mut logs: Vec<Log<ChatMessage>> = (1..=3).map(|a| Log::new(&sim, a)).collect();
    let mut lines: Vec<Log<ChatLine>> = (1..=3).map(|a| Log::new(&sim, a)).collect();
    let mut calls: Vec<Log<RadioCall>> = (1..=3).map(|a| Log::new(&sim, a)).collect();
    let run = |sim: &mut NetSim, n: u32, logs: &mut Vec<Log<ChatMessage>>, lines: &mut Vec<Log<ChatLine>>, calls: &mut Vec<Log<RadioCall>>| {
        for _ in 0..n {
            sim.step();
            for l in logs.iter_mut() {
                l.poll(sim);
            }
            for l in lines.iter_mut() {
                l.poll(sim);
            }
            for l in calls.iter_mut() {
                l.poll(sim);
            }
        }
    };
    client_console(&mut sim, 0, "say hello everyone");
    run(&mut sim, 20, &mut logs, &mut lines, &mut calls);
    client_console(&mut sim, 0, "say_team cts only");
    run(&mut sim, 20, &mut logs, &mut lines, &mut calls);
    let line = |n: &str, t: &str| (n.to_string(), t.to_string());
    assert_eq!(heard(&logs[0]), vec![line("Player 1", "hello everyone"), line("Player 1", "cts only")]);
    assert_eq!(heard(&logs[1]), vec![line("Player 1", "hello everyone")], "the terrorist reads no CT team chat");
    assert_eq!(heard(&logs[2]), vec![line("Player 1", "hello everyone"), line("Player 1", "cts only")]);
    // In the game's format: the team line marked so.
    let text = |l: &ChatLine| l.0.iter().map(|(_, s)| s.as_str()).collect::<String>();
    let shown: Vec<String> = lines[2].all.iter().map(text).collect();
    println!("client 2's chat: {shown:?}");
    assert!(shown.iter().any(|s| s.contains("Player 1") && s.contains("hello everyone")));
    assert!(shown.iter().any(|s| s.contains("(Team)") || s.contains("(Counter-Terrorist)")), "{shown:?}");
    // The dead talk only to the dead.
    for l in &mut logs {
        l.all.clear();
    }
    kill(&mut sim, 0, 1);
    run(&mut sim, 20, &mut logs, &mut lines, &mut calls);
    client_console(&mut sim, 1, "say i am dead");
    run(&mut sim, 20, &mut logs, &mut lines, &mut calls);
    assert!(heard(&logs[0]).is_empty() && heard(&logs[2]).is_empty(), "the living don't read the dead");
    assert_eq!(heard(&logs[1]), vec![line("Player 2", "i am dead")]);
    assert!(!logs[1].all[0].alive);
    // ... but the dead read the living.
    client_console(&mut sim, 2, "say still here");
    run(&mut sim, 20, &mut logs, &mut lines, &mut calls);
    assert_eq!(heard(&logs[1]).last(), Some(&line("Player 3", "still here")));
    // Radio: living teammates hear it (the caller too).
    sim.clients[2].app.world_mut().write_message(RadioRequest {
        command: "coverme".into(),
    });
    run(&mut sim, 20, &mut logs, &mut lines, &mut calls);
    let got: Vec<usize> = calls.iter().map(|c| c.all.len()).collect();
    assert_eq!(got, vec![1, 0, 1], "radio calls per client");
    let id = sim.client_id(2);
    let sender = NetSim::owned_by(&mut sim.clients[0], Some(id)).unwrap();
    assert_eq!(calls[0].all[0].sender, sender, "mapped to the caller's character");
}

#[test]
fn server_cvars_reach_clients_and_prediction_uses_them() {
    let mut sim = NetSim::new(link(50, 5, 0.0), 55, 1, setup);
    sim.until_joined(600);
    sim.ticks(200);
    let get = |w: &mut World, name: &str| -> String {
        let c = w.resource::<Console>().cvar(name).cloned().unwrap();
        (c.get)(w).unwrap()
    };
    let before = get(sim.clients[0].app.world_mut(), "sv_airaccelerate");
    server_console(&mut sim, "sv_airaccelerate 100");
    sim.ticks(30);
    assert_eq!(get(sim.clients[0].app.world_mut(), "sv_airaccelerate"), "100");
    // The client can't set it while connected; a slider or a map load
    // that changes it gets the server's back.
    client_console(&mut sim, 0, "sv_airaccelerate 3");
    sim.ticks(2);
    assert_eq!(get(sim.clients[0].app.world_mut(), "sv_airaccelerate"), "100");
    let set = sim.clients[0].app.world().resource::<Console>().cvar("sv_airaccelerate").cloned().unwrap();
    (set.set)(sim.clients[0].app.world_mut(), "3").unwrap();
    sim.ticks(2);
    assert_eq!(get(sim.clients[0].app.world_mut(), "sv_airaccelerate"), "100");
    // Air strafing (where air acceleration matters): no prediction errors.
    *sim.clients[0].app.world_mut().resource_mut::<NetGraph>() = NetGraph::default();
    for k in 0..400u32 {
        let mut i = client_intent(&mut sim, 0);
        let side = if (k / 40) % 2 == 0 { 1.0 } else { -1.0 };
        i.move_axis = Vec2::new(side, 1.0).normalize();
        i.yaw = (i.yaw + 0.02 * side).rem_euclid(std::f32::consts::TAU);
        i.jump = k % 50 < 3;
        sim.step();
    }
    client_intent(&mut sim, 0).move_axis = Vec2::ZERO;
    sim.ticks(40);
    let g = sim.clients[0].app.world().resource::<NetGraph>().clone();
    println!("air strafing at sv_airaccelerate 100: {} errors in {} compared", g.errors, g.checked);
    assert!(g.checked > 300);
    assert_eq!(g.errors, 0);
    // Cheat commands need the server's sv_cheats 1 (replicated).
    let last = |sim: &NetSim| {
        sim.clients[0].app.world().resource::<Console>().output.last().map(|l| l.text.clone()).unwrap_or_default()
    };
    client_console(&mut sim, 0, "give weapon_awp");
    sim.ticks(2);
    assert!(last(&sim).starts_with("Can't use cheat command give"), "{}", last(&sim));
    server_console(&mut sim, "sv_cheats 1");
    sim.ticks(30);
    client_console(&mut sim, 0, "give weapon_awp");
    sim.ticks(2);
    // Allowed now; what a client carries is still the server's.
    assert!(last(&sim).contains("only the server"), "{}", last(&sim));
    // Leaving puts the client's own value back.
    mashup::net::disconnect(sim.clients[0].app.world_mut(), "test");
    assert_eq!(get(sim.clients[0].app.world_mut(), "sv_airaccelerate"), before);
}

#[test]
fn a_late_joiner_spectates_until_the_next_round() {
    let mut sim = rounds(link(50, 0, 0.0), 56, 2, "mp_freezetime 1; mp_roundtime 0.5");
    restart(&mut sim);
    until_phase(&mut sim, 400, |p| matches!(p, Phase::Live { .. }));
    sim.ticks(30);
    let joiner = sim.add_client(|w| w.resource_mut::<mashup::net::NetSettings>().name = "Late".into());
    sim.until_joined(600);
    sim.ticks(120);
    assert!(matches!(server_phase(&sim), Phase::Live { .. }), "still the same round");
    let me = sim.local_player(joiner).unwrap();
    let w = sim.clients[joiner].app.world();
    assert!(w.get::<Dead>(me).is_some(), "joined mid-round: out until the next one");
    let spec = w.resource::<Spectator>().clone();
    assert_eq!(spec.phase, SpecPhase::Watching, "{spec:?}");
    let target = spec.target.expect("watching someone");
    // Someone else, drawn where the server has them (in the past).
    let name = w.get::<mashup::net::NetCharacter>(target).unwrap().name.clone();
    assert_ne!(target, me);
    let owner = w.get::<mashup::net::NetCharacter>(target).unwrap().owner;
    let on_server = NetSim::owned_by(&mut sim.server, owner).unwrap();
    let there = sim.server.app.world().get::<Transform>(on_server).unwrap().translation;
    let drawn = sim.clients[joiner].app.world().get::<Transform>(target).unwrap().translation;
    println!("the joiner watches {name}: drawn {drawn:?}, on the server {there:?}");
    assert!(drawn.distance(there) < 0.5);
    // The round ends (time runs out), the next one brings it in.
    until_phase(&mut sim, 3000, |p| matches!(p, Phase::Freeze { .. }));
    sim.ticks(30);
    let w = sim.clients[joiner].app.world();
    assert!(w.get::<Dead>(me).is_none(), "plays the next round");
    assert_eq!(w.resource::<Spectator>().phase, SpecPhase::Alive);
    assert!(w.get::<LocalPlayer>(me).is_some());
    let _ = C4;
}

/// How many sounds by this entry an app played (`PlaySound`).
fn played(l: &Log<mashup::map::PlaySound>, entry: &str) -> usize {
    l.all.iter().filter(|s| s.entry == entry).count()
}

/// Arming and defusing the bomb are predicted on the client like its
/// other weapon actions: the progress, the hold, the key presses' sounds
/// and the bomb leaving its hands show without a round trip, the server's
/// state agrees with them (no prediction errors), and the planter's and
/// defuser's own sounds aren't sent back to them.
#[test]
fn arming_and_defusing_are_predicted() {
    let mut sim = rounds(link(100, 0, 0.0), 57, 2, "mp_freezetime 1");
    assert_eq!(team_of(&mut sim, 1), 1);
    restart(&mut sim);
    until_phase(&mut sim, 400, |p| matches!(p, Phase::Live { .. }));
    sim.ticks(30);
    place(&mut sim, 1, Vec3::new(0.0, 1.0, 12.0));
    place(&mut sim, 0, Vec3::new(0.0, 1.0, -20.0));
    sim.ticks(40);
    client_intent(&mut sim, 1).select = Some(4);
    sim.ticks(2);
    client_intent(&mut sim, 1).select = None;
    sim.ticks(80);
    assert!(carried(&mut sim, 1).iter().any(|id| id.ends_with("weapon_c4")));
    let mut sounds = Log::<mashup::map::PlaySound>::new(&sim, 2);
    let mut other = Log::<mashup::map::PlaySound>::new(&sim, 1);
    *sim.clients[1].app.world_mut().resource_mut::<NetGraph>() = NetGraph::default();
    client_intent(&mut sim, 1).fire = true;
    // The frame fire is pressed, the client arms: no round trip.
    sim.step();
    let me = sim.local_player(1).unwrap();
    let c = sim.character_of(1).unwrap();
    assert!(sim.clients[1].app.world().get::<Arming>(me).is_some(), "arming shown at once");
    assert!(sim.server.app.world().get::<Arming>(c).is_none(), "the server hasn't heard yet");
    // Held still while arming, as the server holds it.
    client_intent(&mut sim, 1).move_axis = Vec2::Y;
    let mut gone_first = false;
    for _ in 0..260 {
        sim.step();
        sounds.poll(&sim);
        other.poll(&sim);
        let lost = !carried(&mut sim, 1).iter().any(|id| id.ends_with("weapon_c4"));
        if lost && sim.server.app.world().resource::<BombState>().planted.is_none() {
            gone_first = true;
        }
        if sim.server.app.world().resource::<BombState>().planted.is_some() {
            break;
        }
    }
    client_intent(&mut sim, 1).fire = false;
    client_intent(&mut sim, 1).move_axis = Vec2::ZERO;
    for _ in 0..40 {
        sim.step();
        sounds.poll(&sim);
        other.poll(&sim);
    }
    assert!(sim.server.app.world().resource::<BombState>().planted.is_some(), "planted");
    assert!(gone_first, "the bomb left the planter's hands before the server's word came");
    let g = sim.clients[1].app.world().resource::<NetGraph>().clone();
    let clicks = played(&sounds, "c4.click");
    println!(
        "planting at 100 ms: {} prediction errors in {} states; the planter heard {clicks} key presses, the other client {}",
        g.errors,
        g.checked,
        played(&other, "c4.click")
    );
    assert_eq!(g.errors, 0, "arming, holding still and the plant predicted exactly");
    let rules = sim.server.app.world().resource::<mashup::objectives::bomb::BombRules>().clone();
    assert_eq!(clicks, rules.clicks.len(), "each key press once (predicted, not sent back)");
    assert!(!carried(&mut sim, 1).iter().any(|id| id.ends_with("weapon_c4")));

    // The defuse: client 0 at the bomb, +use.
    let bomb_at = {
        let w = sim.server.app.world_mut();
        w.query_filtered::<&Transform, With<PlantedBomb>>().iter(w).next().unwrap().translation
    };
    place(&mut sim, 0, bomb_at + Vec3::new(0.0, 1.0, 0.9));
    place(&mut sim, 1, Vec3::new(0.0, 1.0, -25.0));
    sim.ticks(40);
    let me0 = sim.character_of(0).unwrap();
    let eye = {
        let w = sim.server.app.world();
        w.get::<Transform>(me0).unwrap().translation + w.get::<mashup::core::MovementState>(me0).unwrap().eye_offset
    };
    let d = bomb_at + Vec3::Y * 0.05 - eye;
    {
        let mut i = client_intent(&mut sim, 0);
        i.yaw = (-d.x).atan2(-d.z).rem_euclid(std::f32::consts::TAU);
        i.pitch = d.y.atan2(d.xz().length());
    }
    sim.ticks(20);
    let mut defuser = Log::<mashup::map::PlaySound>::new(&sim, 1);
    *sim.clients[0].app.world_mut().resource_mut::<NetGraph>() = NetGraph::default();
    client_intent(&mut sim, 0).use_key = true;
    sim.step();
    let local0 = sim.local_player(0).unwrap();
    let w = sim.clients[0].app.world();
    assert!(w.get::<mashup::objectives::bomb::Defusing>(local0).is_some(), "defusing shown at once");
    assert!(sim.server.app.world().get::<mashup::objectives::bomb::Defusing>(me0).is_none());
    for _ in 0..800 {
        sim.step();
        defuser.poll(&sim);
        if sim.server.app.world().resource::<BombState>().outcome.is_some() {
            break;
        }
    }
    client_intent(&mut sim, 0).use_key = false;
    for _ in 0..30 {
        sim.step();
        defuser.poll(&sim);
    }
    assert_eq!(sim.server.app.world().resource::<BombState>().outcome, Some(BombOutcome::Defused));
    let g = sim.clients[0].app.world().resource::<NetGraph>().clone();
    println!(
        "defusing at 100 ms: {} prediction errors in {} states; the defuser heard the start {} times",
        g.errors,
        g.checked,
        played(&defuser, "c4.disarmstart")
    );
    assert_eq!(g.errors, 0, "the defuse predicted exactly");
    assert_eq!(played(&defuser, "c4.disarmstart"), 1, "its own start sound once");
    assert!(sim.clients[0].app.world().get::<mashup::objectives::bomb::Defusing>(local0).is_none());
}

/// The `name` cvar over the network: a client's new name goes to the
/// server (setinfo), which renames its character for everyone and tells
/// everyone "* old changed name to new".
#[test]
fn a_client_changes_its_name() {
    let mut sim = rounds(link(40, 0, 0.0), 58, 2, "");
    let mut renamed: Vec<Log<mashup::net::NameChanged>> = (1..=2).map(|a| Log::new(&sim, a)).collect();
    let mut lines: Vec<Log<ChatLine>> = (1..=2).map(|a| Log::new(&sim, a)).collect();
    client_console(&mut sim, 0, "name \"Alice Liddell\"");
    for _ in 0..30 {
        sim.step();
        for l in renamed.iter_mut() {
            l.poll(&sim);
        }
        for l in lines.iter_mut() {
            l.poll(&sim);
        }
    }
    let c = sim.character_of(0).unwrap();
    assert_eq!(sim.server.app.world().get::<mashup::net::NetCharacter>(c).unwrap().name, "Alice Liddell");
    let text = |l: &ChatLine| l.0.iter().map(|(_, s)| s.as_str()).collect::<String>();
    for i in 0..2 {
        assert_eq!(
            renamed[i].all,
            vec![mashup::net::NameChanged {
                old: "Player 1".into(),
                new: "Alice Liddell".into()
            }],
            "client {i}"
        );
        let shown: Vec<String> = lines[i].all.iter().map(text).collect();
        println!("client {i}'s chat: {shown:?}");
        assert!(shown.iter().any(|s| s == "* Player 1 changed name to Alice Liddell"), "{shown:?}");
        // The scoreboard and kill feed read the new name.
        let id = sim.client_id(0);
        let e = NetSim::owned_by(&mut sim.clients[i], Some(id)).unwrap();
        let w = sim.clients[i].app.world();
        assert_eq!(w.get::<mashup::net::NetCharacter>(e).unwrap().name, "Alice Liddell");
        assert_eq!(w.get::<Name>(e).unwrap().as_str(), "Alice Liddell");
    }
    // Its chat lines carry it.
    let mut logs: Vec<Log<ChatMessage>> = (1..=2).map(|a| Log::new(&sim, a)).collect();
    client_console(&mut sim, 0, "say hi");
    for _ in 0..20 {
        sim.step();
        for l in logs.iter_mut() {
            l.poll(&sim);
        }
    }
    assert_eq!(heard(&logs[1]), vec![("Alice Liddell".to_string(), "hi".to_string())]);
    // The same name again: nothing to tell.
    client_console(&mut sim, 0, "name \"Alice Liddell\"");
    for _ in 0..20 {
        sim.step();
        renamed[1].poll(&sim);
    }
    assert_eq!(renamed[1].all.len(), 1);
}

/// Flood protection: a burst of chat lines or radio calls from one
/// player gets through only so far; normal talk always does.
#[test]
fn chat_and_radio_floods_are_cut_short() {
    let mut sim = rounds(link(40, 0, 0.0), 59, 3, "");
    restart(&mut sim);
    until_phase(&mut sim, 400, |p| matches!(p, Phase::Live { .. }));
    let mut logs: Vec<Log<ChatMessage>> = (1..=3).map(|a| Log::new(&sim, a)).collect();
    let mut calls: Vec<Log<RadioCall>> = (1..=3).map(|a| Log::new(&sim, a)).collect();
    let mut notices = Log::<Notice>::new(&sim, 1);
    let run = |sim: &mut NetSim, n: u32, logs: &mut Vec<Log<ChatMessage>>, calls: &mut Vec<Log<RadioCall>>, notices: &mut Log<Notice>| {
        for _ in 0..n {
            sim.step();
            for l in logs.iter_mut() {
                l.poll(sim);
            }
            for l in calls.iter_mut() {
                l.poll(sim);
            }
            notices.poll(sim);
        }
    };
    // Normal talk: a line every two seconds, all of them.
    for k in 0..4 {
        client_console(&mut sim, 0, &format!("say line {k}"));
        run(&mut sim, 128, &mut logs, &mut calls, &mut notices);
    }
    assert_eq!(heard(&logs[1]).len(), 4, "normal chat all read");
    // A flood: twelve lines in one frame.
    for l in logs.iter_mut() {
        l.all.clear();
    }
    for k in 0..12 {
        client_console(&mut sim, 0, &format!("say spam {k}"));
    }
    run(&mut sim, 30, &mut logs, &mut calls, &mut notices);
    let got = heard(&logs[1]).len();
    println!("a flood of 12 lines: {got} read; the sender was told {} times", notices.all.len());
    assert_eq!(got as f64, mashup::net::chat::SAY_BURST, "only the burst gets through");
    assert!(!notices.all.is_empty(), "the sender hears why");
    // A moment later it talks again.
    run(&mut sim, 128, &mut logs, &mut calls, &mut notices);
    client_console(&mut sim, 0, "say back again");
    run(&mut sim, 20, &mut logs, &mut calls, &mut notices);
    assert_eq!(heard(&logs[1]).last().map(|l| l.1.as_str()), Some("back again"));
    // Radio: ten calls at once from client 2 (a counter-terrorist like
    // client 0): its teammate hears the burst.
    for _ in 0..10 {
        sim.clients[2].app.world_mut().write_message(RadioRequest {
            command: "coverme".into(),
        });
    }
    run(&mut sim, 30, &mut logs, &mut calls, &mut notices);
    let heard_calls = calls[0].all.len();
    println!("a flood of 10 radio calls: {heard_calls} heard");
    assert_eq!(heard_calls as f64, mashup::net::chat::RADIO_BURST);
}

/// Client `i`'s character on the server, by the id it has now.
fn current_character(sim: &mut NetSim, i: usize) -> Entity {
    let id = sim.clients[i].app.world().resource::<mashup::net::client::LocalClientId>().0;
    NetSim::owned_by(&mut sim.server, Some(id)).expect("a character")
}

/// How far a character walks forward in `ticks` (the host on the server,
/// or client `i`'s own player as the server has it).
fn walks(sim: &mut NetSim, who: Option<usize>, ticks: u64) -> f32 {
    let (e_server, set) = match who {
        None => {
            let w = sim.server.app.world_mut();
            let host = w.query_filtered::<Entity, With<LocalPlayer>>().iter(w).next().unwrap();
            (host, None)
        }
        Some(i) => (current_character(sim, i), Some(i)),
    };
    let before = sim.server.app.world().get::<Transform>(e_server).unwrap().translation;
    let axis = |sim: &mut NetSim, v: Vec2| match set {
        None => sim.server.app.world_mut().get_mut::<Intent>(e_server).unwrap().move_axis = v,
        Some(i) => client_intent(sim, i).move_axis = v,
    };
    axis(sim, Vec2::Y);
    sim.ticks(ticks);
    axis(sim, Vec2::ZERO);
    sim.ticks(20);
    let after = sim.server.app.world().get::<Transform>(e_server).unwrap().translation;
    after.distance(before)
}

/// The slice 5 report: after a client was killed and reconnected (and a
/// long session of rounds), neither the host nor the client could walk.
/// Killed, dropped, back with a new connection, map reloads, restarts:
/// both walk every time.
#[test]
fn everyone_walks_after_deaths_reconnects_and_map_changes() {
    let mut sim = rounds(link(50, 5, 0.0), 60, 1, "mp_freezetime 1");
    // A listen server's host.
    let host = sim.server.spawn_character(Vec3::new(6.0, 1.0, 0.0), source::ID);
    sim.server.app.world_mut().entity_mut(host).insert((LocalPlayer, Team(1)));
    let mut next_id = 100;
    for cycle in 0..4 {
        restart(&mut sim);
        until_phase(&mut sim, 400, |p| matches!(p, Phase::Live { .. }));
        sim.ticks(10);
        let (h, c) = (walks(&mut sim, None, 64), walks(&mut sim, Some(0), 64));
        println!("cycle {cycle}: the host walked {h:.2} m, the client {c:.2} m");
        assert!(h > 1.0, "cycle {cycle}: the host walks ({h} m)");
        assert!(c > 1.0, "cycle {cycle}: the client walks ({c} m)");
        // The client is killed (by the host), then leaves and comes back
        // on a new connection; every other time the map loads again.
        let victim = current_character(&mut sim, 0);
        sim.server.app.world_mut().write_message(Damage {
            force: Vec3::ZERO,
            target: victim,
            attacker: Some(host),
            amount: 10.0,
            point: Vec3::ZERO,
            dir: Vec3::X,
            hitgroup: Hitgroup::Chest,
            kind: default(),
            weapon: None,
        });
        sim.ticks(30);
        mashup::net::disconnect(sim.clients[0].app.world_mut(), "Disconnect by user.");
        sim.ticks(30);
        if cycle % 2 == 1 {
            mashup::harness::load_level(sim.server.app.world_mut(), "greybox", None).unwrap();
            sim.ticks(10);
        }
        let link = sim.link.clone();
        mashup::net::memory::join(sim.clients[0].app.world_mut(), link, next_id).unwrap();
        next_id += 1;
        sim.until_joined(600);
        sim.ticks(30);
    }
}

/// What froze the host and a client once (slice 5 notes): two players put
/// in one spot are each inside the other's box and neither can walk
/// (Source's stuck test; noclip, which ignores players, still moved).
/// Spawning now skips spawn points a living player stands on, and the
/// dead (also those waiting to respawn, unseen) block nobody.
#[test]
fn players_never_spawn_inside_each_other() {
    let mut sim = rounds(link(50, 5, 0.0), 61, 1, "mp_freezetime 1");
    let host = sim.server.spawn_character(Vec3::new(6.0, 1.0, 0.0), source::ID);
    sim.server.app.world_mut().entity_mut(host).insert((LocalPlayer, Team(2)));
    restart(&mut sim);
    until_phase(&mut sim, 400, |p| matches!(p, Phase::Live { .. }));
    sim.ticks(10);
    // The report, reproduced: the client put where the host stands.
    let at = sim.server.app.world().get::<Transform>(host).unwrap().translation;
    place(&mut sim, 0, at);
    sim.ticks(10);
    let (h, c) = (walks(&mut sim, None, 64), walks(&mut sim, Some(0), 64));
    println!("one inside the other: the host walked {h:.2} m, the client {c:.2} m");
    assert!(h < 0.1 && c < 0.1, "stuck in each other (the reported freeze)");
    // Every counter-terrorist spawn but one taken by someone alive: a
    // spawn puts the client on the free one, never on another player.
    let spawns: Vec<Vec3> = {
        let w = sim.server.app.world_mut();
        w.query::<(&Transform, &mashup::core::SpawnPoint)>()
            .iter(w)
            .filter(|(_, s)| s.team.is_none_or(|t| t == Team(2)))
            .map(|(t, _)| t.translation)
            .collect()
    };
    assert!(spawns.len() >= 2, "{} spawns", spawns.len());
    let c = current_character(&mut sim, 0);
    sim.server.app.world_mut().entity_mut(c).insert(Team(2));
    for p in &spawns[1..] {
        let e = sim.server.spawn_character(*p, source::ID);
        sim.server.app.world_mut().entity_mut(e).insert(Team(2));
    }
    place(&mut sim, 0, spawns[0] + Vec3::new(0.0, 0.0, 6.0));
    {
        let w = sim.server.app.world_mut();
        let to = spawns[0] + Vec3::new(3.0, 0.0, 0.0);
        w.get_mut::<Transform>(host).unwrap().translation = to;
        if let Some(mut p) = w.get_mut::<Position>(host) {
            p.0 = to;
        }
    }
    for _ in 0..spawns.len() {
        mashup::rules::put_at_spawn(sim.server.app.world_mut(), c, false);
        let there = sim.server.app.world().get::<Transform>(c).unwrap().translation;
        assert!(there.distance(spawns[0]) < 0.01, "put on the free spawn, not {there}");
    }
    // A dead player waiting to respawn (health left, unseen) blocks nobody.
    let ghost_at = sim.server.app.world().get::<Transform>(c).unwrap().translation;
    let host_at = sim.server.app.world().get::<Transform>(host).unwrap().translation;
    let ghost = sim.server.spawn_character(host_at, source::ID);
    // As the rules leave one (a team change, a new game): dead, its
    // collider off.
    sim.server.app.world_mut().entity_mut(ghost).insert((
        Team(2),
        mashup::rules::Dead { since: f64::MIN },
        avian3d::prelude::ColliderDisabled,
    ));
    let _ = ghost_at;
    sim.ticks(70);
    let h = walks(&mut sim, None, 64);
    println!("the host with an unseen dead player on it walked {h:.2} m");
    assert!(h > 1.0, "a dead player doesn't hold the host");
}
