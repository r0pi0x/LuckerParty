//! Network play, slice 6 (docs/plans/active/multiplayer.md): bots on the
//! server (`NetSim`, greybox, CS:S's weapons). Bots added on the server
//! are players to every client (names, "BOT" on the scoreboard), their
//! shots, kills and radio calls reach clients, `bot_quota` keeps their
//! number (fill mode counting humans), a client can't add any, their dice
//! don't depend on who is connected, and what they cost the server with
//! clients connected.
//!
//! `cargo test --features dev --test it net_bots -- --nocapture` prints
//! the numbers.

use std::time::{Duration, Instant};

use avian3d::prelude::Position;
use bevy::{
    ecs::message::{MessageCursor, Messages},
    prelude::*,
};
use mashup::{
    bot::{Bot, BotConfig, BotQuota},
    console::{Console, execute, parse},
    core::{Seed, Team},
    games::cs_source::{
        movement::{self as source, SourceMovementPlugin},
        weapons::CsWeaponsPlugin,
    },
    greybox::GreyboxMapPlugin,
    harness::NetSim,
    net::{Killed, NetCharacter, NetScore, RadioCall, memory::LinkConditions, score_flags},
    slots::Loadout,
    weapon::{WeaponEvent, WeaponEventKind},
};

fn setup(app: &mut App) {
    app.add_plugins((GreyboxMapPlugin, SourceMovementPlugin, CsWeaponsPlugin))
        .insert_resource(Loadout { movement: source::ID });
}

fn link() -> LinkConditions {
    LinkConditions {
        latency: Duration::from_millis(40),
        jitter: Duration::from_millis(5),
        loss: 0.0,
    }
}

/// Run a console line in a world now; its output lines.
fn run(world: &mut World, line: &str) -> Vec<String> {
    let before = world.resource::<Console>().output.len();
    for words in parse(line) {
        execute(world, &words, 0);
    }
    world.resource::<Console>().output[before..].iter().map(|l| l.text.clone()).collect()
}

fn bots(world: &mut World) -> Vec<(Entity, String, Team)> {
    let mut v: Vec<(Entity, String, Team)> = world
        .query_filtered::<(Entity, &Name, &Team), With<Bot>>()
        .iter(world)
        .map(|(e, n, t)| (e, n.as_str().to_string(), *t))
        .collect();
    v.sort_by(|a, b| a.1.cmp(&b.1));
    v
}

/// Characters a client draws that the server says are bots: name, team,
/// scoreboard line.
fn bots_seen(world: &mut World) -> Vec<(String, Team, NetScore)> {
    let mut v: Vec<(String, Team, NetScore)> = world
        .query::<(&NetCharacter, &Team, &NetScore)>()
        .iter(world)
        .filter(|(c, _, _)| c.owner.is_none())
        .map(|(c, t, s)| (c.name.clone(), *t, s.clone()))
        .collect();
    v.sort_by(|a, b| a.0.cmp(&b.0));
    v
}

fn place(world: &mut World, e: Entity, at: Vec3) {
    world.get_mut::<Transform>(e).unwrap().translation = at;
    if let Some(mut p) = world.get_mut::<Position>(e) {
        p.0 = at;
    }
}

#[test]
fn bots_on_a_server_are_players_to_every_client() {
    let mut sim = NetSim::new(link(), 61, 2, setup);
    sim.until_joined(400);
    let out = run(sim.server.app.world_mut(), "bot_add 1; bot_add 2");
    assert!(out.iter().any(|l| l.contains("added Bot 1")), "{out:?}");
    assert_eq!(sim.server.app.world().resource::<BotQuota>().quota, 2);
    sim.ticks(40);
    let server = bots(sim.server.app.world_mut());
    assert_eq!(
        server.iter().map(|b| (b.1.as_str(), b.2)).collect::<Vec<_>>(),
        [("Bot 1", Team(1)), ("Bot 2", Team(2))]
    );
    for i in 0..2 {
        let seen = bots_seen(sim.clients[i].app.world_mut());
        println!("client {i} sees bots {:?}", seen.iter().map(|b| (&b.0, b.1)).collect::<Vec<_>>());
        assert_eq!(seen.len(), 2, "client {i} draws both bots");
        for (name, team, score) in &seen {
            assert!(score.flags & score_flags::BOT != 0, "{name}: BOT on the scoreboard");
            assert_eq!(score.ping, 0, "{name}: no ping");
            let on_server = server.iter().find(|b| &b.1 == name).expect("same names");
            assert_eq!(*team, on_server.2);
        }
        // Drawn where the server has them (they stand at their spawns).
        let world = sim.clients[i].app.world_mut();
        let drawn: Vec<(String, Vec3)> = world
            .query::<(&NetCharacter, &Transform)>()
            .iter(world)
            .filter(|(c, _)| c.owner.is_none())
            .map(|(c, t)| (c.name.clone(), t.translation))
            .collect();
        for (name, at) in drawn {
            let e = server.iter().find(|b| b.1 == name).unwrap().0;
            let truth = sim.server.app.world().get::<Transform>(e).unwrap().translation;
            assert!(at.distance(truth) < 0.05, "{name} drawn at {at}, the server has {truth}");
        }
    }
    // A client can't add or kick bots: they're the server's.
    let out = run(sim.clients[0].app.world_mut(), "bot_add 1; bot_kick; bot_quota 5");
    println!("client's bot_add: {out:?}");
    assert!(out.iter().any(|l| l.contains("Can't use server command bot_add")), "{out:?}");
    assert!(out.iter().any(|l| l.contains("Can't use server command bot_kick")), "{out:?}");
    assert!(out.iter().any(|l| l.contains("Can't change replicated ConVar bot_quota")), "{out:?}");
    sim.ticks(10);
    assert_eq!(bots(sim.clients[0].app.world_mut()).len(), 0, "no bot of its own");
    assert_eq!(bots(sim.server.app.world_mut()).len(), 2);
    // Kicking one by name.
    let out = run(sim.server.app.world_mut(), "bot_kick \"Bot 1\"");
    assert!(out.iter().any(|l| l.contains("kicked 1 bots")), "{out:?}");
    sim.ticks(30);
    assert_eq!(sim.server.app.world().resource::<BotQuota>().quota, 1);
    let seen = bots_seen(sim.clients[1].app.world_mut());
    assert_eq!(seen.iter().map(|b| b.0.as_str()).collect::<Vec<_>>(), ["Bot 2"]);
}

/// Messages a world got, gathered after every step.
struct Log<M: Message> {
    cursor: MessageCursor<M>,
    all: Vec<M>,
}

impl<M: Message + Clone> Log<M> {
    fn new(world: &World) -> Self {
        Self {
            cursor: world.resource::<Messages<M>>().get_cursor_current(),
            all: Vec::new(),
        }
    }

    fn poll(&mut self, world: &World) {
        self.all.extend(self.cursor.read(world.resource::<Messages<M>>()).cloned());
    }
}

#[test]
fn bots_fight_and_clients_see_their_shots_kills_and_radio() {
    let mut sim = NetSim::new(link(), 62, 2, setup);
    sim.until_joined(400);
    {
        let w = sim.server.app.world_mut();
        let mut c = w.resource_mut::<BotConfig>();
        c.stop = 1;
        c.grenades = 0;
        c.radio = 1;
    }
    run(sim.server.app.world_mut(), "bot_add 1; bot_add 2");
    sim.ticks(320);
    // The clients out of the way in far corners (and unhurt); the bots face
    // to face 12 m apart, each the other's nearest enemy.
    let server_bots = bots(sim.server.app.world_mut());
    for i in 0..2 {
        let me = sim.character_of(i).unwrap();
        let w = sim.server.app.world_mut();
        w.entity_mut(me).insert(mashup::core::God);
        place(w, me, Vec3::new(35.0 - 70.0 * i as f32, 1.0, 35.0));
    }
    let w = sim.server.app.world_mut();
    place(w, server_bots[0].0, Vec3::new(0.0, 1.0, 0.0));
    place(w, server_bots[1].0, Vec3::new(0.0, 1.0, 12.0));
    let mut killed: Vec<Log<Killed>> = (0..2).map(|i| Log::new(sim.clients[i].app.world())).collect();
    let mut radio: Vec<Log<RadioCall>> = (0..2).map(|i| Log::new(sim.clients[i].app.world())).collect();
    let mut shots: Vec<Log<WeaponEvent>> = (0..2).map(|i| Log::new(sim.clients[i].app.world())).collect();
    let mut kills = 0;
    for _ in 0..1280 {
        sim.step();
        for i in 0..2 {
            killed[i].poll(sim.clients[i].app.world());
            radio[i].poll(sim.clients[i].app.world());
            shots[i].poll(sim.clients[i].app.world());
        }
        kills = killed[0].all.len();
        if kills >= 2 {
            break;
        }
    }
    for i in 0..2 {
        let world = sim.clients[i].app.world_mut();
        let bot_copies: Vec<Entity> = world
            .query::<(Entity, &NetCharacter)>()
            .iter(world)
            .filter(|(_, c)| c.owner.is_none())
            .map(|(e, _)| e)
            .collect();
        let bot_kills = killed[i]
            .all
            .iter()
            .filter(|k| k.attacker.is_some_and(|a| bot_copies.contains(&a)) && bot_copies.contains(&k.victim))
            .count();
        let bot_shots = shots[i]
            .all
            .iter()
            .filter(|e| bot_copies.contains(&e.owner) && matches!(e.kind, WeaponEventKind::Shot { .. }))
            .count();
        let bot_calls: Vec<String> = radio[i]
            .all
            .iter()
            .filter(|r| bot_copies.contains(&r.sender))
            .map(|r| r.command.clone())
            .collect();
        let scores: Vec<(String, u32, u32)> = bots_seen(world).into_iter().map(|(n, _, s)| (n, s.kills, s.deaths)).collect();
        println!(
            "client {i}: bot kills seen {bot_kills}, bot shots drawn {bot_shots}, radio from bots {bot_calls:?}, scoreboard {scores:?}"
        );
        assert!(bot_kills >= 1, "client {i} saw a bot kill a bot");
        assert!(bot_shots > 0, "client {i} drew the bots' shots");
        assert!(
            scores.iter().map(|s| s.1).sum::<u32>() >= 1 && scores.iter().map(|s| s.2).sum::<u32>() >= 1,
            "client {i}'s scoreboard counts the bots' kills and deaths: {scores:?}"
        );
    }
    let calls: usize = (0..2).map(|i| radio[i].all.len()).sum();
    assert!(calls > 0, "the bots' radio reached their teammates");
    assert!(kills >= 1);
}

/// `bot_quota`: normal mode keeps that many; fill counts humans; `bot_kick`
/// puts it to 0, `bot_add` raises it.
#[test]
fn bot_quota_fills_with_humans_counted() {
    let mut sim = NetSim::new(link(), 63, 0, setup);
    run(sim.server.app.world_mut(), "bot_quota 3");
    sim.ticks(20);
    assert_eq!(bots(sim.server.app.world_mut()).len(), 3, "normal: 3 bots");
    let teams: Vec<Team> = bots(sim.server.app.world_mut()).iter().map(|b| b.2).collect();
    assert!(teams.contains(&Team(1)) && teams.contains(&Team(2)), "both teams: {teams:?}");
    run(sim.server.app.world_mut(), "bot_kick");
    sim.ticks(5);
    assert_eq!(bots(sim.server.app.world_mut()).len(), 0);
    assert_eq!(sim.server.app.world().resource::<BotQuota>().quota, 0);
    run(sim.server.app.world_mut(), "bot_add 2; bot_add 2");
    sim.ticks(10);
    assert_eq!(bots(sim.server.app.world_mut()).len(), 2, "bot_add raised the quota: none kicked");

    run(sim.server.app.world_mut(), "bot_kick; bot_quota_mode fill; bot_quota 4");
    sim.ticks(20);
    assert_eq!(bots(sim.server.app.world_mut()).len(), 4, "fill, no humans: 4 bots");
    sim.add_client(|_| {});
    sim.until_joined(400);
    sim.ticks(10);
    assert_eq!(bots(sim.server.app.world_mut()).len(), 3, "fill, one human: 3 bots");
    sim.add_client(|_| {});
    sim.until_joined(400);
    sim.ticks(10);
    let left = bots(sim.server.app.world_mut());
    assert_eq!(left.len(), 2, "fill, two humans: 2 bots");
    // The clients see the same.
    sim.ticks(20);
    for i in 0..2 {
        assert_eq!(bots_seen(sim.clients[i].app.world_mut()).len(), 2);
    }
    mashup::net::disconnect(sim.clients[1].app.world_mut(), "Disconnect by user.");
    sim.ticks(30);
    assert_eq!(bots(sim.server.app.world_mut()).len(), 3, "a human left: a bot back");
    run(sim.server.app.world_mut(), "bot_quota_mode match; bot_quota 2");
    sim.ticks(20);
    assert_eq!(bots(sim.server.app.world_mut()).len(), 2, "match: 2 per human (one left)");
}

/// The bots' dice and command numbers come from their number and team,
/// never from who is connected: the same seeds with and without clients,
/// and two identical games run the same.
#[test]
fn bot_seeds_dont_depend_on_clients() {
    let seeds = |clients: usize| {
        let mut sim = NetSim::new(link(), 64, clients, setup);
        if clients > 0 {
            sim.until_joined(400);
        }
        run(sim.server.app.world_mut(), "bot_add 1; bot_add 2; bot_add 1");
        sim.ticks(5);
        let w = sim.server.app.world_mut();
        let mut v: Vec<(String, u64)> = w
            .query_filtered::<(&Name, &Seed), With<Bot>>()
            .iter(w)
            .map(|(n, s)| (n.as_str().to_string(), s.0))
            .collect();
        v.sort();
        v
    };
    let alone = seeds(0);
    let with_clients = seeds(2);
    println!("bot seeds: {alone:?}");
    assert_eq!(alone, with_clients);

    // Two runs of the same game (same link, same bots): the same paths.
    let run_game = || {
        let mut sim = NetSim::new(link(), 65, 1, setup);
        sim.until_joined(400);
        run(sim.server.app.world_mut(), "bot_add 1; bot_add 2");
        sim.ticks(400);
        let w = sim.server.app.world_mut();
        let mut v: Vec<(String, [u32; 3])> = w
            .query_filtered::<(&Name, &Transform), With<Bot>>()
            .iter(w)
            .map(|(n, t)| (n.as_str().to_string(), t.translation.to_array().map(f32::to_bits)))
            .collect();
        v.sort();
        v
    };
    assert_eq!(run_game(), run_game(), "the same game runs the same");
}

/// What bots cost a server with clients connected: ten bots fighting on
/// the greybox with two clients joined, the server's frame time and what
/// each client receives.
#[test]
fn ten_bots_with_two_clients_cost() {
    let mut sim = NetSim::new(link(), 66, 2, setup);
    sim.until_joined(400);
    sim.ticks(64);
    let (base, base_p95) = server_frames(&mut sim, 320);
    run(sim.server.app.world_mut(), "bot_quota 10");
    sim.ticks(64);
    assert_eq!(bots(sim.server.app.world_mut()).len(), 10);
    let start = sim.link.lock().unwrap().delivered;
    let (mean, p95) = server_frames(&mut sim, 640);
    let packets = sim.link.lock().unwrap().delivered - start;
    let kbs: Vec<f64> = sim
        .clients
        .iter()
        .map(|c| {
            c.app
                .world()
                .get_resource::<bevy_replicon_renet::RenetClient>()
                .map_or(0.0, |r| r.network_info().bytes_received_per_second / 1000.0)
        })
        .collect();
    println!(
        "2 clients, no bots: server frame {base:.2} ms mean, {base_p95:.2} ms p95; 10 bots: {mean:.2} ms mean, {p95:.2} ms p95; \
         {packets} packets in 10 s; clients receive {kbs:.1?} KB/s"
    );
    // Relative and generous (a debug build on a shared, often busy box):
    // ten bots cost a fraction of what the rest of the frame does.
    assert!(mean < base * 3.0 + 5.0, "server frame {mean:.2} ms with bots, {base:.2} ms without");
    for k in kbs {
        assert!(k < 200.0, "{k} KB/s to a client");
    }
}

/// `n` frames everywhere; the server's frame time, mean and 95th
/// percentile, ms.
fn server_frames(sim: &mut NetSim, n: usize) -> (f64, f64) {
    let mut ms = Vec::new();
    for _ in 0..n {
        sim.link.lock().unwrap().advance(Duration::from_secs_f64(1.0 / 64.0));
        let t = Instant::now();
        sim.server.app.update();
        ms.push(t.elapsed().as_secs_f64() * 1000.0);
        for c in &mut sim.clients {
            c.app.update();
        }
    }
    ms.sort_by(f64::total_cmp);
    (ms.iter().sum::<f64>() / n as f64, ms[n * 95 / 100])
}
