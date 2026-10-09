//! Network play, slice 4 (docs/plans/active/multiplayer.md): weapons over
//! `NetSim`'s in-memory link on the greybox with CS:S's weapons.
//!
//! - A client shooting a moving target at 100 and 150 ms hits where it
//!   saw it with lag compensation, and misses the same shots without.
//! - Its predicted ammo, timers, punch, recoil, accuracy, zoom and
//!   weapon switches are the server's bit for bit with no loss.
//! - Another client draws its shots with the same pellets the server
//!   traced; `sv_maxunlag` bounds the rewind; a dead player can't shoot;
//!   a client deals no damage of its own.
//! - Grenades are thrown by the server (the throw is predicted), drawn
//!   from snapshots and go off on every client; dropping asks the
//!   server, and picking up is the server's.
//!
//! `cargo test --features dev --test it net_weapons -- --nocapture`
//! prints the numbers.

use std::{collections::HashMap, time::Duration};

use avian3d::prelude::Position;
use bevy::{
    ecs::message::{MessageCursor, Messages},
    prelude::*,
};
use mashup::{
    core::{Damage, Health, Intent, MovementState, PredictedComponents, SimTick},
    games::cs_source::{
        grenades::HEGRENADE,
        movement::{self as source, SourceMovementPlugin},
        weapons::{AK47, AWP, CsWeaponsPlugin, M3},
    },
    greybox::GreyboxMapPlugin,
    harness::NetSim,
    map::loose::ShownItem,
    net::{
        NetItem,
        memory::LinkConditions,
        predict::{NetGraph, PredictionHistory},
    },
    slots::Loadout,
    weapon::{
        Inventory, Magazine, ViewPunch, Weapon, WeaponEvent, WeaponEventKind, give,
        grenade::{Detonated, Projectile},
        lagcomp::{LagCompSettings, LagCompStats},
    },
};

fn setup(app: &mut App) {
    app.add_plugins((GreyboxMapPlugin, SourceMovementPlugin, CsWeaponsPlugin))
        .insert_resource(Loadout { movement: source::ID });
}

fn link(latency_ms: u64, jitter_ms: u64, loss: f64) -> LinkConditions {
    LinkConditions {
        latency: Duration::from_millis(latency_ms),
        jitter: Duration::from_millis(jitter_ms),
        loss,
    }
}

/// A server and `clients` clients, joined, spawned with their weapons and
/// predicting.
fn joined(conditions: LinkConditions, seed: u64, clients: usize) -> NetSim {
    let mut sim = NetSim::new(conditions, seed, clients, setup);
    sim.until_joined(600);
    sim.ticks(320);
    for i in 0..clients {
        let me = sim.local_player(i).unwrap();
        let inv = sim.clients[i].app.world().get::<Inventory>(me);
        assert!(
            inv.is_some_and(|i| i.active.is_some()),
            "client {i} predicts what it carries"
        );
    }
    sim
}

fn graph(sim: &NetSim, i: usize) -> NetGraph {
    sim.clients[i].app.world().resource::<NetGraph>().clone()
}

fn reset_graph(sim: &mut NetSim, i: usize) {
    *sim.clients[i].app.world_mut().resource_mut::<NetGraph>() = NetGraph::default();
}

fn client_intent(sim: &mut NetSim, i: usize) -> Mut<'_, Intent> {
    let me = sim.local_player(i).unwrap();
    sim.clients[i].app.world_mut().get_mut::<Intent>(me).unwrap()
}

/// The server puts client `i`'s character at `at` (a correction the
/// client takes); its client looks along `yaw`.
fn place(sim: &mut NetSim, i: usize, at: Vec3, yaw: f32) {
    let e = sim.character_of(i).unwrap();
    let world = sim.server.app.world_mut();
    world.get_mut::<Transform>(e).unwrap().translation = at;
    if let Some(mut p) = world.get_mut::<Position>(e) {
        p.0 = at;
    }
    let mut intent = client_intent(sim, i);
    intent.yaw = yaw;
    intent.pitch = 0.0;
    *intent = Intent {
        yaw,
        pitch: 0.0,
        ..default()
    };
}

/// The messages of type `M` one app writes, gathered after every step
/// (a message lasts two updates).
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
}

trait Poll {
    fn poll(&mut self, sim: &NetSim);
}

impl<M: Message + Clone> Poll for Log<M> {
    fn poll(&mut self, sim: &NetSim) {
        let world = world_of(sim, self.app);
        self.all
            .extend(self.cursor.read(world.resource::<Messages<M>>()).cloned());
    }
}

/// One step, then every log gathers.
fn step(sim: &mut NetSim, logs: &mut [&mut dyn Poll]) {
    sim.step();
    for l in logs.iter_mut() {
        l.poll(sim);
    }
}

fn steps(sim: &mut NetSim, n: u32, logs: &mut [&mut dyn Poll]) {
    for _ in 0..n {
        step(sim, logs);
    }
}

/// Yaw and pitch that look from `eye` at `at`.
fn look_at(eye: Vec3, at: Vec3) -> (f32, f32) {
    let d = at - eye;
    ((-d.x).atan2(-d.z).rem_euclid(std::f32::consts::TAU), d.y.atan2(d.xz().length()))
}

/// What one shooting run measured.
#[derive(Debug, Default)]
struct Run {
    shots: usize,
    hits: usize,
    /// How far across the line of fire the server's hit points were from
    /// where the shooter drew the target's centre line when it fired, m
    /// (max).
    off: f32,
    client_damage: usize,
    /// Hits the shooter's client heard of (the server's confirmations).
    client_hits: usize,
}

/// Client 0 stands still and taps single AK-47 shots at client 1's body
/// as client 0 draws it, while client 1 runs sideways across its view:
/// the server's hits on client 1.
fn shooting_run(sim: &mut NetSim, unlag: bool) -> Run {
    sim.server.app.world_mut().resource_mut::<LagCompSettings>().unlag = unlag as u8;
    let shooter = sim.character_of(0).unwrap();
    let target = sim.character_of(1).unwrap();
    // Out of anyone's way: the shooter at the north end of the open
    // floor looking south (-Z), the target 10 m ahead.
    place(sim, 0, Vec3::new(0.0, 1.0, 12.0), 0.0);
    place(sim, 1, Vec3::new(-6.0, 1.0, 2.0), 0.0);
    // The target doesn't die of it (the hits still count).
    sim.server.app.world_mut().entity_mut(target).insert(mashup::core::God);
    sim.ticks(80);
    let id = sim.client_id(1);
    let seen = NetSim::owned_by(&mut sim.clients[0], Some(id)).unwrap();
    let mut events = Log::<WeaponEvent>::new(sim, 0);
    let mut damage = Log::<Damage>::new(sim, 0);
    let mut client_damage = Log::<Damage>::new(sim, 1);
    let mut client_events = Log::<WeaponEvent>::new(sim, 1);
    let mut run = Run::default();
    // Where the shooter saw the target when each shot went (x of its
    // centre line), in order.
    let mut seen_at: Vec<(Vec3, Vec3)> = Vec::new();
    for pass in 0..2 {
        let dir = if pass == 0 { 1.0 } else { -1.0 };
        for k in 0..130u32 {
            client_intent(sim, 1).move_axis = Vec2::new(dir, 0.0);
            // Aim at the target's chest as drawn now.
            let me = sim.local_player(0).unwrap();
            let w = sim.clients[0].app.world();
            let eye = w.get::<Transform>(me).unwrap().translation + w.get::<MovementState>(me).unwrap().eye_offset;
            let body = w.get::<Transform>(seen).unwrap().translation;
            let (yaw, pitch) = look_at(eye, body + Vec3::Y * 0.2);
            let fire = k >= 40 && (k - 40) % 30 == 0;
            if fire {
                seen_at.push((body, eye));
            }
            let mut i = client_intent(sim, 0);
            i.yaw = yaw;
            i.pitch = pitch;
            i.fire = fire;
            step(sim, &mut [&mut events, &mut damage, &mut client_damage, &mut client_events]);
        }
        client_intent(sim, 1).move_axis = Vec2::ZERO;
        steps(sim, 30, &mut [&mut events, &mut damage, &mut client_damage, &mut client_events]);
    }
    steps(sim, 40, &mut [&mut events, &mut damage, &mut client_damage, &mut client_events]);
    for e in events.all {
        if e.owner != shooter {
            continue;
        }
        match e.kind {
            WeaponEventKind::Fired { .. } => run.shots += 1,
            WeaponEventKind::Hit { target: t, .. } if t == target => run.hits += 1,
            _ => {}
        }
    }
    let points: Vec<Vec3> = damage
        .all
        .into_iter()
        .filter(|d| d.attacker == Some(shooter) && d.target == target)
        .map(|d| d.point)
        .collect();
    // Each hit's line of fire, where it crosses the body's centre plane
    // (z), against the nearest place the shooter saw the body's centre.
    for p in &points {
        let off = seen_at
            .iter()
            .map(|(s, eye)| {
                let q = *eye + (*p - *eye) * ((s.z - eye.z) / (p.z - eye.z));
                (q.x - s.x).abs()
            })
            .fold(f32::MAX, f32::min);
        run.off = run.off.max(off);
    }
    run.client_damage = client_damage.all.len();
    run.client_hits = client_events
        .all
        .iter()
        .filter(|e| matches!(e.kind, WeaponEventKind::Hit { .. }))
        .count();
    run
}

#[test]
fn lag_compensation_hits_where_the_shooter_saw_the_target() {
    println!(" link   | lag comp | shots | hits | worst hit off the body as seen (m) | rewound (ms back)");
    for (latency, seed) in [(100, 41), (150, 42)] {
        let mut results = Vec::new();
        for unlag in [true, false] {
            let mut sim = joined(link(latency, 0, 0.0), seed, 2);
            let run = shooting_run(&mut sim, unlag);
            let stats = sim.server.app.world().resource::<LagCompStats>().clone();
            let back = stats
                .last
                .as_ref()
                .map_or(0.0, |r| r.seconds_back(mashup::games::cs_source::TICK_INTERVAL) * 1000.0);
            let back = if unlag { back } else { 0.0 };
            println!(
                " {latency:>3} ms | {:>8} | {:>5} | {:>4} | {:>34.3} | {back:.0}",
                if unlag { "on" } else { "off" },
                run.shots,
                run.hits,
                run.off
            );
            // (f) A client deals no damage of its own; its hits are the
            // ones the server confirmed.
            assert_eq!(run.client_damage, 0, "the shooter's client wrote damage");
            assert_eq!(run.client_hits, run.hits, "hits confirmed to the shooter");
            results.push(run);
        }
        let (on, off) = (&results[0], &results[1]);
        assert_eq!(on.shots, 6, "{latency} ms: shots fired");
        assert_eq!(on.hits, on.shots, "{latency} ms: every compensated shot hits ({on:?})");
        assert!(on.off < 0.15, "{latency} ms: hits are on the body as seen ({on:?})");
        assert_eq!(off.shots, 6);
        assert_eq!(off.hits, 0, "{latency} ms: without compensation the same shots miss ({off:?})");
    }
}

#[test]
fn maxunlag_bounds_the_rewind() {
    let mut sim = joined(link(150, 0, 0.0), 43, 2);
    sim.server.app.world_mut().resource_mut::<LagCompSettings>().maxunlag = 0.05;
    let run = shooting_run(&mut sim, true);
    let stats = sim.server.app.world().resource::<LagCompStats>().clone();
    let last = stats.last.clone().expect("rewound");
    let back = last.seconds_back(mashup::games::cs_source::TICK_INTERVAL);
    println!(
        "sv_maxunlag 0.05: {} rewinds, {} held at the limit, last {:.0} ms back (saw tick {:.1}, traced {:.1}), {} of {} hit",
        stats.rewinds,
        stats.clamped,
        back * 1000.0,
        last.view_tick,
        last.target_tick,
        run.hits,
        run.shots
    );
    assert!(stats.clamped >= 6, "every shot was further back than 50 ms ({stats:?})");
    assert!(back <= 0.05 + 1e-6, "{back}");
    assert!(last.view_tick < last.target_tick);
    assert_eq!(run.hits, 0, "50 ms back isn't where it was seen at 150 ms");
}

/// Step `steps` steps running `script` on client 0, keeping the server's
/// state of its player after each tick; then every predicted tick the
/// server ran must be the server's state, bit for bit.
fn agrees(sim: &mut NetSim, n: u32, logs: &mut [&mut dyn Poll], mut script: impl FnMut(u32, &mut Intent)) -> usize {
    let theirs = sim.character_of(0).unwrap();
    let mut server = HashMap::new();
    let mut compared = 0;
    for k in 0..n {
        script(k, &mut client_intent(sim, 0));
        step(sim, logs);
        let world = sim.server.app.world();
        let tick = world.resource::<SimTick>().0;
        server.insert(tick, world.resource::<PredictedComponents>().encode(world, theirs));
        // Compare what has been predicted and run so far.
        let history = &sim.clients[0].app.world().resource::<PredictionHistory>().0;
        for p in history.iter().filter(|p| !p.state.is_empty()) {
            if let Some(s) = server.remove(&p.tick) {
                assert!(s == p.state, "step {k}: the prediction for tick {} isn't the server's", p.tick);
                compared += 1;
            }
        }
    }
    compared
}

#[test]
fn the_weapon_frame_is_predicted_bit_for_bit() {
    let mut sim = joined(link(100, 0, 0.0), 44, 1);
    let theirs = sim.character_of(0).unwrap();
    // A sniper rifle and a shotgun too (the client hears of them).
    give(sim.server.app.world_mut(), theirs, AWP).unwrap();
    give(sim.server.app.world_mut(), theirs, M3).unwrap();
    sim.ticks(60);
    reset_graph(&mut sim, 0);
    let mut events = Log::<WeaponEvent>::new(&sim, 1);
    // Slot keys: 0 primary (cycles AWP, M3, AK), 1 pistol.
    let compared = agrees(&mut sim, 1100, &mut [&mut events], |k, i| {
        i.move_axis = if (240..300).contains(&k) { Vec2::Y } else { Vec2::ZERO };
        i.yaw = 0.2 + k as f32 * 0.002;
        i.pitch = -0.05;
        i.jump = k == 270;
        i.crouch = (300..330).contains(&k);
        // The M3 (drawn last) pumped twice; the AK-47 sprayed walking and
        // jumping, then reloaded; the AWP zoomed twice and fired (a shot
        // unzooms, it zooms back), again; the pistol tapped, reloaded.
        i.fire = matches!(k, 40..=41 | 110..=111 | 260..=300 | 680 | 790 | 930 | 945 | 960);
        i.select = match k {
            160..=161 | 520..=521 => Some(0),
            850..=851 => Some(1),
            _ => None,
        };
        i.secondary = matches!(k, 620..=621 | 640);
        i.reload = matches!(k, 330..=331 | 1000..=1001);
    });
    let count = |f: fn(&WeaponEventKind) -> bool| events.all.iter().filter(|e| e.shown() && f(&e.kind)).count();
    let shots = count(|k| matches!(k, WeaponEventKind::Fired { .. }));
    let zooms = count(|k| matches!(k, WeaponEventKind::ModeChanged { .. }));
    let reloads = count(|k| matches!(k, WeaponEventKind::ReloadStarted));
    let switches = count(|k| matches!(k, WeaponEventKind::Deployed));
    *client_intent(&mut sim, 0) = Intent::default();
    sim.ticks(60);
    let g = graph(&sim, 0);
    println!(
        "weapon frame at 100 ms: {compared} ticks compared, {} states checked, {} errors; {shots} rounds, \
         {zooms} mode changes, {reloads} reloads, {switches} draws",
        g.checked, g.errors
    );
    assert_eq!(g.errors, 0, "prediction errors ({g:?})");
    assert!(compared > 1000, "only {compared} ticks compared");
    assert!(shots >= 12, "the script fires ({shots})");
    assert!(zooms >= 3 && reloads >= 2 && switches >= 3, "zooms {zooms}, reloads {reloads}, draws {switches}");
    // Ammo, clip and punch as the server has them.
    let me = sim.local_player(0).unwrap();
    let ours = sim.clients[0].app.world();
    let server = sim.server.app.world();
    let clips = |w: &World, e: Entity| {
        let inv = w.get::<Inventory>(e).unwrap();
        inv.weapons
            .iter()
            .map(|x| {
                let m = w.get::<Magazine>(*x);
                (w.get::<Weapon>(*x).unwrap().id, m.map(|m| (m.clip, m.reserve)))
            })
            .collect::<Vec<_>>()
    };
    assert_eq!(clips(ours, me), clips(server, theirs));
    assert_eq!(
        ours.get::<ViewPunch>(me).map(|p| p.0.to_array().map(f32::to_bits)),
        server.get::<ViewPunch>(theirs).map(|p| p.0.to_array().map(f32::to_bits))
    );
    // The AK-47 sprayed and reloaded (rounds came out of the reserve).
    let fired = clips(ours, me).iter().any(|(id, m)| *id == AK47 && m.is_some_and(|m| m.1 < 90));
    assert!(fired, "the AK-47 was fired: {:?}", clips(ours, me));
}

#[test]
fn others_draw_the_same_pellets_the_server_traced() {
    let mut sim = joined(link(80, 10, 0.0), 45, 2);
    let shooter = sim.character_of(0).unwrap();
    give(sim.server.app.world_mut(), shooter, M3).unwrap();
    place(&mut sim, 0, Vec3::new(0.0, 1.0, 12.0), 0.3);
    place(&mut sim, 1, Vec3::new(-8.0, 1.0, 12.0), 0.0);
    sim.ticks(80);
    let id = sim.client_id(0);
    let seen = NetSim::owned_by(&mut sim.clients[1], Some(id)).unwrap();
    let mut server_events = Log::<WeaponEvent>::new(&sim, 0);
    let mut remote_events = Log::<WeaponEvent>::new(&sim, 2);
    // Two shotgun blasts, then the AK-47 sprayed.
    for k in 0..300u32 {
        let mut i = client_intent(&mut sim, 0);
        i.fire = matches!(k, 0..=1 | 70..=71) || (220..260).contains(&k);
        i.select = if (120..=121).contains(&k) { Some(0) } else { None };
        i.pitch = -0.02 * (k % 5) as f32;
        step(&mut sim, &mut [&mut server_events, &mut remote_events]);
    }
    steps(&mut sim, 60, &mut [&mut server_events, &mut remote_events]);
    // Per round: its origin, angles and seed, and its pellets'
    // directions, in order.
    type Round = ((u32, u32, u32, u32), Vec<[f32; 3]>);
    let rounds = |events: Vec<WeaponEvent>, owner: Entity| -> Vec<Round> {
        let mut out: Vec<Round> = Vec::new();
        for e in events.into_iter().filter(|e| e.owner == owner) {
            match e.kind {
                WeaponEventKind::Fired {
                    origin, yaw, pitch, seed, ..
                } => out.push(((origin.y.to_bits(), yaw.to_bits(), pitch.to_bits(), seed), Vec::new())),
                WeaponEventKind::Shot { from, to, .. } => {
                    if let Some(r) = out.last_mut() {
                        r.1.push((to - from).normalize().to_array());
                    }
                }
                _ => {}
            }
        }
        out
    };
    let server = rounds(server_events.all, shooter);
    // The remote client's trace starts no `Fired`: its rounds are the
    // `Shot`s in groups of the weapon's pellets, in the server's order.
    let remote_shots: Vec<[f32; 3]> = remote_events
        .all
        .into_iter()
        .filter(|e| e.owner == seen)
        .filter_map(|e| match e.kind {
            WeaponEventKind::Shot { from, to, .. } => Some((to - from).normalize().to_array()),
            _ => None,
        })
        .collect();
    let server_shots: Vec<[f32; 3]> = server.iter().flat_map(|r| r.1.clone()).collect();
    let pellets = server.iter().filter(|r| r.1.len() == 9).count();
    println!(
        "{} rounds ({} shotgun blasts of 9 pellets), {} traces on the server, {} drawn by the other client",
        server.len(),
        pellets,
        server_shots.len(),
        remote_shots.len()
    );
    assert!(pellets >= 2 && server.len() >= 8, "fired: {}", server.len());
    assert_eq!(remote_shots.len(), server_shots.len(), "every trace drawn");
    let worst = server_shots
        .iter()
        .zip(&remote_shots)
        .map(|(a, b)| Vec3::from_array(*a).angle_between(Vec3::from_array(*b)))
        .fold(0.0f32, f32::max);
    println!("worst direction difference: {worst:e} rad");
    assert!(worst < 1e-4, "the same pellet directions ({worst} rad)");
}

#[test]
fn a_dead_player_cannot_shoot() {
    let mut sim = joined(link(100, 0, 0.0), 46, 2);
    let me = sim.character_of(0).unwrap();
    let other = sim.character_of(1).unwrap();
    place(&mut sim, 0, Vec3::new(0.0, 1.0, 12.0), 0.0);
    place(&mut sim, 1, Vec3::new(0.0, 1.0, 4.0), 0.0);
    sim.ticks(60);
    sim.server.app.world_mut().write_message(Damage {
        target: me,
        attacker: None,
        amount: 2.0,
        point: Vec3::ZERO,
        dir: Vec3::NEG_Y,
        hitgroup: mashup::core::Hitgroup::Generic,
        kind: mashup::core::DamageKind::Generic,
        weapon: None,
        force: Vec3::ZERO,
    });
    sim.ticks(30);
    let mut server_events = Log::<WeaponEvent>::new(&sim, 0);
    let mut client_events = Log::<WeaponEvent>::new(&sim, 1);
    let health = sim.server.app.world().get::<Health>(other).unwrap().current;
    // Holding fire at the other player while dead (respawn is 2 s away).
    for _ in 0..60 {
        client_intent(&mut sim, 0).fire = true;
        step(&mut sim, &mut [&mut server_events, &mut client_events]);
    }
    assert!(sim.server.app.world().get::<mashup::rules::Dead>(me).is_some(), "still dead");
    let fired = |events: &[WeaponEvent]| {
        events
            .iter()
            .filter(|e| matches!(e.kind, WeaponEventKind::Fired { .. } | WeaponEventKind::Shot { .. }))
            .count()
    };
    assert_eq!(fired(&server_events.all), 0, "the server fired");
    assert_eq!(
        fired(&client_events.all),
        0,
        "the client predicted shots"
    );
    assert_eq!(sim.server.app.world().get::<Health>(other).unwrap().current, health);
}

#[test]
fn grenades_are_thrown_by_the_server_and_seen_by_everyone() {
    let mut sim = joined(link(100, 0, 0.0), 47, 2);
    let thrower = sim.character_of(0).unwrap();
    give(sim.server.app.world_mut(), thrower, HEGRENADE).unwrap();
    place(&mut sim, 0, Vec3::new(0.0, 1.0, 12.0), 0.0);
    place(&mut sim, 1, Vec3::new(-8.0, 1.0, 12.0), 0.0);
    sim.ticks(120);
    reset_graph(&mut sim, 0);
    let mut own = Log::<WeaponEvent>::new(&sim, 1);
    let mut seen_detonations = Log::<Detonated>::new(&sim, 2);
    // Pin out, release, watch it fly.
    let mut in_flight_seen = 0;
    for k in 0..300u32 {
        let mut i = client_intent(&mut sim, 0);
        i.fire = (5..40).contains(&k);
        i.pitch = -0.3;
        step(&mut sim, &mut [&mut own, &mut seen_detonations]);
        let w = sim.clients[1].app.world_mut();
        in_flight_seen += w
            .query_filtered::<(), (With<NetItem>, With<ShownItem>)>()
            .iter(w)
            .count();
    }
    let kinds: Vec<String> = own
        .all
        .iter()
        .filter(|e| e.shown())
        .map(|e| format!("{:?}", e.kind))
        .collect();
    let g = graph(&sim, 0);
    let w = sim.clients[0].app.world_mut();
    let local_projectiles = w.query::<&Projectile>().iter(w).count();
    let detonations = seen_detonations.all.len();
    println!(
        "grenade: thrower predicted {kinds:?}; {} prediction errors; other client drew it {in_flight_seen} frames, heard {detonations} detonation(s)",
        g.errors
    );
    assert!(kinds.iter().any(|k| k == "PinPulled") && kinds.iter().any(|k| k == "Thrown"));
    assert_eq!(local_projectiles, 0, "a client throws no projectile itself");
    assert!(in_flight_seen > 30, "the other client drew it in flight");
    assert_eq!(detonations, 1);
    assert_eq!(g.errors, 0, "the throw is predicted ({g:?})");
    // The grenade is gone from what it carries, on both sides.
    let ids = |w: &World, e: Entity| -> Vec<&'static str> {
        w.get::<Inventory>(e)
            .unwrap()
            .weapons
            .iter()
            .map(|x| w.get::<Weapon>(*x).unwrap().id)
            .collect()
    };
    let me = sim.local_player(0).unwrap();
    assert!(!ids(sim.server.app.world(), thrower).contains(&HEGRENADE));
    assert_eq!(ids(sim.clients[0].app.world(), me), ids(sim.server.app.world(), thrower));
}

#[test]
fn dropping_asks_the_server_and_picking_up_is_its() {
    let mut sim = joined(link(60, 0, 0.0), 48, 2);
    let me = sim.character_of(0).unwrap();
    place(&mut sim, 0, Vec3::new(0.0, 1.0, 12.0), 0.0);
    sim.ticks(60);
    let ids = |w: &World, e: Entity| -> Vec<&'static str> {
        w.get::<Inventory>(e)
            .unwrap()
            .weapons
            .iter()
            .map(|x| w.get::<Weapon>(*x).unwrap().id)
            .collect()
    };
    assert!(ids(sim.server.app.world(), me).contains(&AK47));
    mashup::console::execute(sim.clients[0].app.world_mut(), &["drop".into()], 0);
    sim.ticks(20);
    // Away from it, so it isn't picked up again yet.
    place(&mut sim, 0, Vec3::new(8.0, 1.0, 12.0), 0.0);
    sim.ticks(40);
    let local = sim.local_player(0).unwrap();
    assert!(!ids(sim.server.app.world(), me).contains(&AK47), "the server dropped it");
    assert!(!ids(sim.clients[0].app.world(), local).contains(&AK47), "and the client knows");
    let w = sim.clients[1].app.world_mut();
    let lying: Vec<String> = w.query::<&ShownItem>().iter(w).map(|s| s.0.clone()).collect();
    assert!(lying.iter().any(|m| m == AK47), "the other client sees it lying: {lying:?}");
    // Walk back over it after the touch delay: the server gives it back.
    sim.ticks(80);
    let loose = {
        let w = sim.server.app.world_mut();
        w.query::<(&mashup::map::loose::LooseItem, &Transform)>()
            .iter(w)
            .find(|(l, _)| l.0 == AK47)
            .map(|(_, t)| t.translation)
            .unwrap()
    };
    place(&mut sim, 0, loose + Vec3::Y * 0.9, 0.0);
    sim.ticks(60);
    assert!(ids(sim.server.app.world(), me).contains(&AK47), "picked up");
    let local = sim.local_player(0).unwrap();
    assert!(ids(sim.clients[0].app.world(), local).contains(&AK47));
    assert_eq!(ids(sim.clients[0].app.world(), local), ids(sim.server.app.world(), me));
}

#[test]
fn bots_shoot_players_over_the_net() {
    let mut sim = joined(link(80, 0, 0.0), 49, 1);
    let me = sim.character_of(0).unwrap();
    let team = *sim.server.app.world().get::<mashup::core::Team>(me).unwrap();
    let other = mashup::core::Team(if team.0 == 1 { 2 } else { 1 });
    let bot = mashup::bot::add_bot(sim.server.app.world_mut(), other).unwrap();
    {
        let mut c = sim.server.app.world_mut().resource_mut::<mashup::bot::BotConfig>();
        c.stop = 1;
        c.grenades = 0;
    }
    sim.ticks(10);
    // Face to face across the open floor, the bot held still.
    let w = sim.server.app.world_mut();
    for (e, at) in [(bot, Vec3::new(0.0, 1.0, 2.0)), (me, Vec3::new(0.0, 1.0, 12.0))] {
        w.get_mut::<Transform>(e).unwrap().translation = at;
        if let Some(mut p) = w.get_mut::<Position>(e) {
            p.0 = at;
        }
    }
    let id = sim.client_id(0);
    let mut drawn = Log::<WeaponEvent>::new(&sim, 1);
    let mut hurt = false;
    for _ in 0..640 {
        step(&mut sim, &mut [&mut drawn]);
        let local = sim.local_player(0).unwrap();
        hurt |= sim.clients[0].app.world().get::<mashup::core::Health>(local).unwrap().current < 1.0;
        if hurt {
            break;
        }
    }
    let seen = NetSim::owned_by(&mut sim.clients[0], None).expect("the bot is drawn");
    let shots = drawn
        .all
        .iter()
        .filter(|e| e.owner == seen && matches!(e.kind, WeaponEventKind::Shot { .. }))
        .count();
    println!("a bot shot client {id}: its shots drawn {shots} traces, hurt {hurt}");
    assert!(hurt, "the bot's hits reach the client's health");
    assert!(shots > 0, "the client drew the bot's shots");
}

/// Hurt and death sounds over the network (`games::cs_source::pain`):
/// client 0 shoots client 1, in kevlar, while client 2 watches. Every
/// client hears the kevlar hit once, from the victim (the shooter and the
/// victim included: no client predicts being hit), then the second shot's
/// death cry once (from the kill the server sends, not again as a sound).
#[test]
fn everyone_hears_a_hit_and_a_death_once() {
    use mashup::{core::FriendlyFire, map::PlaySound, weapon::Armor};
    let mut sim = joined(link(60, 0, 0.0), 51, 3);
    sim.server.app.insert_resource(FriendlyFire(1));
    let target = sim.character_of(1).unwrap();
    place(&mut sim, 0, Vec3::new(0.0, 1.0, 12.0), 0.0);
    place(&mut sim, 1, Vec3::new(0.0, 1.0, 4.0), 0.0);
    place(&mut sim, 2, Vec3::new(4.0, 1.0, 6.0), 0.0);
    sim.server.app.world_mut().entity_mut(target).insert(Armor {
        amount: 1.0,
        helmet: false,
    });
    sim.ticks(80);
    let id = sim.client_id(1);
    // The victim as each client knows it.
    let victims: Vec<Entity> = (0..3)
        .map(|i| {
            if i == 1 {
                sim.local_player(1).unwrap()
            } else {
                NetSim::owned_by(&mut sim.clients[i], Some(id)).unwrap()
            }
        })
        .collect();
    let mut heard: Vec<Log<PlaySound>> = (1..=3).map(|app| Log::<PlaySound>::new(&sim, app)).collect();
    let shoot = |sim: &mut NetSim, heard: &mut Vec<Log<PlaySound>>| {
        let me = sim.local_player(0).unwrap();
        let w = sim.clients[0].app.world();
        let eye = w.get::<Transform>(me).unwrap().translation + w.get::<MovementState>(me).unwrap().eye_offset;
        let body = w.get::<Transform>(victims[0]).unwrap().translation;
        let (yaw, pitch) = look_at(eye, body + Vec3::Y * 0.2);
        let mut i = client_intent(sim, 0);
        i.yaw = yaw;
        i.pitch = pitch;
        i.fire = true;
        let mut logs: Vec<&mut dyn Poll> = heard.iter_mut().map(|l| l as &mut dyn Poll).collect();
        step(sim, &mut logs);
        client_intent(sim, 0).fire = false;
        steps(sim, 60, &mut logs);
    };
    let count = |heard: &[Log<PlaySound>], entry: &str| -> Vec<usize> {
        heard
            .iter()
            .enumerate()
            .map(|(i, l)| l.all.iter().filter(|s| s.entry == entry && s.source == Some(victims[i])).count())
            .collect()
    };
    shoot(&mut sim, &mut heard);
    let health = sim.server.app.world().get::<Health>(target).unwrap().current;
    assert!(health < 1.0 && health > 0.0, "hit, alive: {health}");
    assert_eq!(count(&heard, "Player.DamageKevlar"), [1, 1, 1], "the kevlar hit, once each");
    assert_eq!(count(&heard, "Player.Death"), [0, 0, 0]);
    // The next one kills.
    sim.server.app.world_mut().get_mut::<Health>(target).unwrap().current = 0.01;
    shoot(&mut sim, &mut heard);
    assert_eq!(sim.server.app.world().get::<Health>(target).unwrap().current, 0.0, "killed");
    assert_eq!(count(&heard, "Player.DamageKevlar"), [2, 2, 2]);
    assert_eq!(count(&heard, "Player.Death"), [1, 1, 1], "the death cry, once each");
}

/// The weapon `id` in `e`'s inventory in `w`.
fn carried(w: &World, e: Entity, id: &str) -> Option<Entity> {
    w.get::<Inventory>(e)?
        .weapons
        .iter()
        .copied()
        .find(|x| w.get::<Weapon>(*x).is_some_and(|x| x.id == id))
}

/// Who walks: client `i`'s own player, or a character on the server.
#[derive(Clone, Copy)]
enum Walker {
    Client(usize),
    Server(Entity),
}

/// Walk `who` toward `to()` (re-aimed each step) until `done`; the steps
/// it took, None within `limit`.
fn walk_until(
    sim: &mut NetSim,
    who: Walker,
    to: impl Fn(&mut NetSim) -> Option<Vec3>,
    limit: u32,
    done: impl Fn(&mut NetSim) -> bool,
) -> Option<u32> {
    let steer = |sim: &mut NetSim, target: Option<Vec3>| {
        let (world, e) = match who {
            Walker::Client(i) => {
                let local = sim.local_player(i).unwrap();
                (sim.clients[i].app.world_mut(), local)
            }
            Walker::Server(e) => (sim.server.app.world_mut(), e),
        };
        let at = world.get::<Transform>(e).unwrap().translation;
        let mut i = world.get_mut::<Intent>(e).unwrap();
        match target {
            Some(target) => {
                let d = target - at;
                i.yaw = (-d.x).atan2(-d.z);
                i.move_axis = if d.xz().length() > 0.05 { Vec2::Y } else { Vec2::ZERO };
            }
            None => i.move_axis = Vec2::ZERO,
        }
    };
    for t in 0..limit {
        if done(sim) {
            steer(sim, None);
            return Some(t);
        }
        let target = to(sim);
        steer(sim, target);
        sim.step();
    }
    steer(sim, None);
    None
}

/// Walked, not placed: a client drops its rifle (`drop`), walks back over
/// where it sees it lying and the server gives it back once the touch
/// delay is over; with another primary carried it walks over it and
/// nothing happens (CS:S's rule). The host (a player on the server
/// itself, as a listen server has) drops and takes its own the same way.
#[test]
fn a_client_and_the_host_walk_back_over_dropped_guns() {
    use mashup::{core::LocalPlayer, games::cs_source::weapons::M4A1};
    let mut sim = joined(link(60, 0, 0.0), 50, 1);
    let me = sim.character_of(0).unwrap();
    place(&mut sim, 0, Vec3::new(0.0, 1.0, 12.0), 0.0);
    sim.ticks(60);
    let seen_lying = |sim: &mut NetSim| -> Option<Vec3> {
        let w = sim.clients[0].app.world_mut();
        w.query::<(&ShownItem, &Transform)>()
            .iter(w)
            .find(|(s, _)| s.0 == AK47)
            .map(|(_, t)| t.translation)
    };
    for slot_full in [false, true] {
        let ak = carried(sim.server.app.world(), me, AK47).expect("carries the AK");
        // Taken back, it isn't drawn (the pistol was): draw it.
        client_intent(&mut sim, 0).select = Some(0);
        sim.ticks(10);
        client_intent(&mut sim, 0).select = None;
        sim.ticks(70);
        let held = sim.server.app.world().get::<Inventory>(me).unwrap().active;
        assert_eq!(
            held,
            Some(ak),
            "holds the AK (slot full {slot_full}), not {:?}",
            held.and_then(|h| sim.server.app.world().get::<Weapon>(h)).map(|w| w.id)
        );
        mashup::console::execute(sim.clients[0].app.world_mut(), &["drop".into()], 0);
        sim.ticks(30);
        assert!(
            carried(sim.server.app.world(), me, AK47).is_none(),
            "the server dropped it"
        );
        let m4 = slot_full.then(|| mashup::weapon::give(sim.server.app.world_mut(), me, M4A1).unwrap());
        sim.ticks(70);
        let took = walk_until(
            &mut sim,
            Walker::Client(0),
            seen_lying,
            if slot_full { 200 } else { 400 },
            |s| carried(s.server.app.world(), me, AK47).is_some(),
        );
        if slot_full {
            assert_eq!(took, None, "taken over the M4 in its slot");
            assert!(seen_lying(&mut sim).is_some(), "still drawn lying");
            let m4 = m4.unwrap();
            assert!(
                sim.server
                    .app
                    .world()
                    .get::<Inventory>(me)
                    .unwrap()
                    .weapons
                    .contains(&m4)
            );
        } else {
            assert!(took.is_some(), "walked over it and not given it back");
            sim.ticks(30);
            let local = sim.local_player(0).unwrap();
            assert!(
                carried(sim.clients[0].app.world(), local, AK47).is_some(),
                "and the client knows"
            );
            sim.ticks(70);
        }
    }
    // The host's own player.
    let host = sim.server.spawn_character(Vec3::new(-6.0, 1.0, 12.0), source::ID);
    sim.server.app.world_mut().entity_mut(host).insert(LocalPlayer);
    sim.ticks(120);
    let ak = carried(sim.server.app.world(), host, AK47).expect("the host carries an AK");
    mashup::console::execute(sim.server.app.world_mut(), &["drop".into()], 0);
    sim.ticks(5);
    assert!(
        carried(sim.server.app.world(), host, AK47).is_none(),
        "the host dropped it"
    );
    let lying = |sim: &mut NetSim| -> Option<Vec3> {
        let w = sim.server.app.world_mut();
        w.query::<(&mashup::weapon::drop::Loose, &Transform)>()
            .iter(w)
            .find(|(l, _)| l.weapon == ak)
            .map(|(_, t)| t.translation)
    };
    sim.ticks(70);
    let took = walk_until(&mut sim, Walker::Server(host), lying, 400, |s| {
        carried(s.server.app.world(), host, AK47) == Some(ak)
    });
    assert!(took.is_some(), "the host walked over its gun and didn't take it");
}

/// A smoke grenade going off on the server is drawn on a client: its
/// cloud starts there with the server's age and its sprites are in the
/// client's particle pool at the cloud's alpha. (The server named the
/// grenade by its spent entity, no longer a `Projectile` by then, so it
/// sent registry index 0, not a grenade, and no client ever drew one.)
#[test]
fn a_smoke_cloud_is_drawn_on_clients() {
    use mashup::{
        games::cs_source::grenades::SMOKEGRENADE,
        map::particles::{MapParticles, ParticleMaterial, ParticleMaterials, Particles},
        weapon::grenade::{SmokeCloud, spawn_projectile},
    };
    let mut sim = joined(link(60, 0, 0.0), 51, 1);
    sim.clients[0].app.insert_resource(ParticleMaterials(MapParticles {
        materials: vec![ParticleMaterial {
            name: "particle/particle_smokegrenade1".into(),
            ..default()
        }],
    }));
    let me = sim.character_of(0).unwrap();
    let w = sim.server.app.world_mut();
    let grenade = give(w, me, SMOKEGRENADE).unwrap();
    spawn_projectile(w, grenade, Vec3::new(0.0, 0.1, 4.0), Vec3::ZERO).unwrap();
    sim.ticks(64 * 4);
    let w = sim.clients[0].app.world_mut();
    let clouds = w.query::<&SmokeCloud>().iter(w).count();
    assert_eq!(clouds, 1, "the client runs the cloud");
    let pool = sim.clients[0].app.world().resource::<Particles>();
    let drawn: Vec<f32> = pool
        .groups
        .iter()
        .flat_map(|g| &g.particles)
        .filter(|p| p.material == 0)
        .map(|p| p.alpha)
        .collect();
    assert!(drawn.len() >= 32, "{} sprites drawn", drawn.len());
    assert!(drawn.iter().filter(|a| **a > 0.99).count() >= 16, "{drawn:?}");
}
