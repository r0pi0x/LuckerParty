//! Network play, slice 2 (docs/plans/active/multiplayer.md): user
//! commands bound to server ticks, the clock sync, and the client's
//! prediction of its own player with reconciliation, over `NetSim`'s
//! in-memory link with seeded latency, jitter and loss on the greybox.
//!
//! - With no loss the prediction is never wrong (walking, jumping,
//!   ducking, a ladder) at 50, 100 and 150 ms.
//! - With loss and jitter, mistakes are corrected and the client ends
//!   where the server has it, bit for bit.
//! - The server's movement for a command stream is single player's for
//!   the same commands, bit for bit.
//! - A client sending more, faster or bigger commands moves no faster.

use std::time::Duration;

use bevy::prelude::*;
use mashup::{
    core::{Intent, PredictedComponents},
    games::cs_source::movement::{self as source, SourceMovement, SourceMovementConfig, SourceMovementPlugin},
    greybox::{self, GreyboxMapPlugin},
    harness::{NetSim, Sim},
    net::{
        NetCmd, UserCmds,
        memory::LinkConditions,
        predict::{CommandClock, NetGraph, PredictionHistory, Smoothing},
        server::CommandBuffer,
    },
    slots::Loadout,
};

fn greybox(app: &mut App) {
    app.add_plugins((GreyboxMapPlugin, SourceMovementPlugin))
        .insert_resource(Loadout { movement: source::ID });
}

fn link(latency_ms: u64, jitter_ms: u64, loss: f64) -> LinkConditions {
    LinkConditions {
        latency: Duration::from_millis(latency_ms),
        jitter: Duration::from_millis(jitter_ms),
        loss,
    }
}

/// A server and `clients` clients, joined, respawned and predicting with
/// their clocks settled.
fn joined(conditions: LinkConditions, seed: u64, clients: usize) -> NetSim {
    joined_at(conditions, seed, clients, mashup::net::server::DEFAULT_UPDATERATE)
}

/// `joined`, the clients asking for `updaterate` updates a second.
fn joined_at(conditions: LinkConditions, seed: u64, clients: usize, updaterate: f32) -> NetSim {
    let mut sim = NetSim::new(conditions, seed, clients, move |app: &mut App| {
        greybox(app);
        app.insert_resource(mashup::net::predict::RateSettings {
            updaterate,
            ..default()
        });
    });
    sim.until_joined(600);
    // Respawn, the first states, the clock sync settling.
    sim.ticks(320);
    for i in 0..clients {
        let world = sim.clients[i].app.world();
        assert!(world.resource::<CommandClock>().tick.is_some(), "client {i} predicts");
    }
    sim
}

fn graph(sim: &NetSim, i: usize) -> NetGraph {
    sim.clients[i].app.world().resource::<NetGraph>().clone()
}

fn reset_graph(sim: &mut NetSim, i: usize) {
    *sim.clients[i].app.world_mut().resource_mut::<NetGraph>() = NetGraph::default();
}

/// The movement script for step `k`: walk, strafe, turn, jump, duck,
/// a duck jump, stop.
fn script(k: u32, i: &mut Intent) {
    i.move_axis = match k {
        0..=39 => Vec2::Y,
        40..=79 => Vec2::new(1.0, 1.0).normalize(),
        80..=119 => Vec2::new(-1.0, 0.0),
        120..=159 => Vec2::Y,
        _ => Vec2::ZERO,
    };
    i.yaw = 0.4 + k as f32 * 0.011;
    i.pitch = -0.1 + (k % 9) as f32 * 0.003;
    i.jump = (20..=22).contains(&k) || (100..=101).contains(&k) || (130..=131).contains(&k);
    i.crouch = (50..=70).contains(&k) || (128..=140).contains(&k);
    i.walk = (85..=95).contains(&k);
}

/// Step `ticks` more, keeping the server's state of client `i`'s player
/// after each tick; then every tick client `i` has predicted that the
/// server has run must be the server's state for it, bit for bit (the
/// client runs ahead, so its present is the server's future).
fn assert_agrees(sim: &mut NetSim, i: usize, what: &str) {
    let theirs = sim.character_of(i).unwrap();
    let mut server = std::collections::HashMap::new();
    for _ in 0..30 {
        sim.step();
        let world = sim.server.app.world();
        let tick = world.resource::<mashup::core::SimTick>().0;
        server.insert(tick, world.resource::<PredictedComponents>().encode(world, theirs));
    }
    let history = &sim.clients[i].app.world().resource::<PredictionHistory>().0;
    let mut compared = 0;
    for p in history.iter().filter(|p| !p.state.is_empty()) {
        if let Some(s) = server.get(&p.tick) {
            assert!(
                *s == p.state,
                "{what}: the prediction for tick {} isn't the server's",
                p.tick
            );
            compared += 1;
        }
    }
    assert!(compared > 0, "{what}: no predicted tick to compare");
}
fn client_intent(sim: &mut NetSim, i: usize) -> Mut<'_, Intent> {
    let me = sim.local_player(i).unwrap();
    sim.clients[i].app.world_mut().get_mut::<Intent>(me).unwrap()
}

/// Run the script on client `i` for `steps` steps, then idle until the
/// last states are back; returns the server character's positions.
fn run_script(sim: &mut NetSim, i: usize, steps: u32) -> Vec<Vec3> {
    let theirs = sim.character_of(i).unwrap();
    let mut path = Vec::new();
    for k in 0..steps {
        script(k, &mut client_intent(sim, i));
        sim.step();
        path.push(sim.server.app.world().get::<Transform>(theirs).unwrap().translation);
    }
    *client_intent(sim, i) = Intent::default();
    sim.ticks(80);
    path
}

#[test]
fn without_loss_the_prediction_is_never_wrong() {
    // At the game's update rate (CS:S's 20 a second), 33 and every tick:
    // the server's states come less often, never differ.
    for (latency, seed, rate) in [(50, 11, 20.0), (100, 12, 33.0), (150, 13, 66.0)] {
        let mut sim = joined_at(link(latency, 0, 0.0), seed, 1, rate);
        reset_graph(&mut sim, 0);
        let theirs = sim.character_of(0).unwrap();
        let start = sim.server.app.world().get::<Transform>(theirs).unwrap().translation;
        let path = run_script(&mut sim, 0, 180);
        let g = graph(&sim, 0);
        println!(
            "{latency} ms, {rate} updates/s: {} states checked, {} errors, {} resyncs, lead {:.2} (target {:.2}), server buffer {}, missed {}",
            g.checked, g.errors, g.resyncs, g.lead, g.target, g.buffered, g.missed
        );
        assert_eq!(g.errors, 0, "{latency} ms: prediction errors ({g:?})");
        assert_eq!(g.resyncs, 0, "{latency} ms: restarts");
        // 260 ticks: a state each update.
        let want = 200.0 * rate.min(64.0) / 64.0;
        assert!(g.checked as f32 >= want, "{latency} ms: only {} states compared", g.checked);
        // It moved: walked, jumped (up), ducked.
        let far = path.iter().map(|p| p.distance(start)).fold(0.0, f32::max);
        assert!(far > 3.0, "{latency} ms: moved {far} m");
        let top = path.iter().map(|p| p.y).fold(f32::MIN, f32::max);
        assert!(top > start.y + 0.3, "{latency} ms: jumped to {top} from {}", start.y);
        assert_agrees(&mut sim, 0, &format!("{latency} ms"));
        // Commands arrived ahead of their ticks, a tick or two buffered.
        let b = sim.server.app.world().get::<CommandBuffer>(theirs).unwrap();
        assert_eq!(b.late, 0, "{latency} ms: late commands");
    }
}

#[test]
fn climbing_a_ladder_is_predicted() {
    let mut sim = joined(link(100, 0, 0.0), 21, 1);
    // The server puts the player in front of the north-wall ladder
    // (a teleport the client takes from the server's state).
    let theirs = sim.character_of(0).unwrap();
    let at = Vec3::new(greybox::LADDER_X, 1.0, greybox::NORTH_WALL_Z + 0.1 + 0.43);
    sim.server
        .app
        .world_mut()
        .get_mut::<Transform>(theirs)
        .unwrap()
        .translation = at;
    {
        let mut i = client_intent(&mut sim, 0);
        i.yaw = 0.0;
        i.pitch = 0.0;
    }
    sim.ticks(60);
    let g = graph(&sim, 0);
    assert!(g.errors >= 1, "the teleport is a correction ({g:?})");
    let me = sim.local_player(0).unwrap();
    let here = sim.clients[0].app.world().get::<Transform>(me).unwrap().translation;
    assert!(here.distance(at) < 0.5, "the client took the server's position: {here}");

    reset_graph(&mut sim, 0);
    let y0 = sim.server.app.world().get::<Transform>(theirs).unwrap().translation.y;
    let mut climbed = false;
    for k in 0..90 {
        let mut i = client_intent(&mut sim, 0);
        i.move_axis = if k < 70 { Vec2::Y } else { Vec2::ZERO };
        i.yaw = 0.0;
        i.pitch = 0.0;
        sim.step();
        climbed |= sim
            .server
            .app
            .world()
            .get::<SourceMovement>(theirs)
            .unwrap()
            .ladder
            .is_some();
    }
    sim.ticks(60);
    let y1 = sim.server.app.world().get::<Transform>(theirs).unwrap().translation.y;
    assert!(climbed && y1 - y0 > 2.0, "climbed {} m", y1 - y0);
    let g = graph(&sim, 0);
    assert_eq!(g.errors, 0, "ladder prediction errors ({g:?})");
    assert_agrees(&mut sim, 0, "ladder");
}

#[test]
fn with_loss_and_jitter_mistakes_are_corrected() {
    let mut corrected = 0;
    for (latency, jitter, loss, seed) in [
        (50, 20, 0.05, 31),
        (100, 40, 0.1, 32),
        (150, 60, 0.2, 33),
        (100, 120, 0.35, 34),
    ] {
        let mut sim = joined(link(latency, jitter, loss), seed, 1);
        reset_graph(&mut sim, 0);
        run_script(&mut sim, 0, 180);
        // Settled: nothing left to correct.
        sim.ticks(64);
        let settled = graph(&sim, 0);
        sim.ticks(64);
        let g = graph(&sim, 0);
        let theirs = sim.character_of(0).unwrap();
        let b = sim.server.app.world().get::<CommandBuffer>(theirs).unwrap();
        println!(
            "{latency}±{jitter} ms, {:.0}% loss: {} states checked, {} errors (worst {:.4} m), {} resyncs, {} replayed, \
             lead {:.2} (target {:.2}), {} clock jumps; server: {} missed, {} late",
            loss * 100.0,
            g.checked,
            g.errors,
            g.worst_error,
            g.resyncs,
            g.replayed,
            g.lead,
            g.target,
            g.clock_jumps,
            b.missed,
            b.late
        );
        assert_eq!(g.errors, settled.errors, "{latency} ms: still correcting while idle");
        assert_agrees(&mut sim, 0, &format!("{latency} ms with loss, converged"));
        assert!(sim.link.lock().unwrap().lost > 0);
        corrected += g.errors;
    }
    assert!(corrected > 0, "the worst link should have needed corrections");
}

#[test]
fn a_correction_is_eased_out_of_the_view() {
    let mut sim = joined(link(60, 0, 0.0), 41, 1);
    reset_graph(&mut sim, 0);
    // The server shoves the player 0.3 m (as a push the client didn't
    // predict would).
    let theirs = sim.character_of(0).unwrap();
    sim.server
        .app
        .world_mut()
        .get_mut::<Transform>(theirs)
        .unwrap()
        .translation
        .x += 0.3;
    sim.ticks(30);
    let g = graph(&sim, 0);
    assert_eq!(g.errors, 1, "{g:?}");
    assert!((g.last_error - 0.3).abs() < 0.01, "off by {} m", g.last_error);
    // The view starts from where it was drawn: the offset is the jump back.
    let smoothing = *sim.clients[0].app.world().resource::<Smoothing>();
    let back = smoothing.offset(smoothing.since(), 0.1);
    assert!((back.x + 0.3).abs() < 0.01, "eased from {back}");
    assert_eq!(smoothing.offset(smoothing.since() + 0.2, 0.1), Vec3::ZERO);
    assert_agrees(&mut sim, 0, "after the shove");
}

/// The server's movement for the commands a client sent is single
/// player's for the same commands, bit for bit (slice 0's determinism:
/// the same code, the same command, the same state).
#[test]
fn the_server_moves_a_player_as_single_player_does() {
    let mut sim = joined(link(80, 0, 0.0), 51, 1);
    let theirs = sim.character_of(0).unwrap();
    let encode = |world: &World, e: Entity| world.resource::<PredictedComponents>().encode(world, e);
    let start = encode(sim.server.app.world(), theirs);
    let mut stream: Vec<(NetCmd, Vec<u8>)> = Vec::new();
    for k in 0..180 {
        script(k, &mut client_intent(&mut sim, 0));
        sim.step();
        let world = sim.server.app.world();
        let tick = world.resource::<mashup::core::SimTick>().0;
        let cmd = NetCmd::of(tick, world.get::<Intent>(theirs).unwrap());
        stream.push((cmd, encode(world, theirs)));
    }
    // Single player: the same state, the same commands.
    let mut sp = Sim::new((GreyboxMapPlugin, SourceMovementPlugin));
    let p = sp.spawn_character(Vec3::new(0.0, 5.0, 0.0), source::ID);
    sp.ticks(1);
    sp.app
        .world_mut()
        .resource_scope(|world, registry: Mut<PredictedComponents>| {
            registry.decode(world, p, &start).unwrap();
        });
    let moved = stream.iter().filter(|(c, _)| c.move_axis != [0.0, 0.0]).count();
    assert!(moved > 100, "the stream moves ({moved} commands)");
    for (k, (cmd, server)) in stream.iter().enumerate() {
        cmd.apply(&mut sp.intent(p));
        sp.ticks(1);
        let ours = encode(sp.app.world(), p);
        assert!(ours == *server, "tick {k} of the stream differs from single player");
    }
}

/// A client can't move faster by sending more commands, commands for
/// far-off ticks, or a longer move axis: the server runs one command per
/// tick, made safe.
#[test]
fn commands_cant_speed_a_player_up() {
    let mut sim = joined(link(50, 0, 0.0), 61, 1);
    let theirs = sim.character_of(0).unwrap();
    let max = SourceMovementConfig::default().player_maxspeed * 0.0254;
    let mut fastest = 0.0f32;
    let start = sim.server.app.world().get::<Transform>(theirs).unwrap().translation;
    let ticks = 64;
    for _ in 0..ticks {
        // Its own commands ask for the same, through the normal path...
        client_intent(&mut sim, 0).move_axis = Vec2::splat(40.0);
        // ...and a flood more: every tick ahead, a huge axis, sprinting.
        let tick = sim.clients[0].app.world().resource::<CommandClock>().tick.unwrap();
        let cmds = (0..80)
            .map(|d| NetCmd {
                tick: tick + d,
                move_axis: [40.0, 40.0],
                buttons: mashup::net::buttons::SPRINT,
                ..default()
            })
            .collect();
        sim.clients[0].app.world_mut().write_message(UserCmds { cmds, epoch: 0, ack: 0 });
        sim.step();
        let v = sim.server.app.world().get::<mashup::core::Velocity>(theirs).unwrap().0;
        fastest = fastest.max(Vec2::new(v.x, v.z).length());
    }
    let end = sim.server.app.world().get::<Transform>(theirs).unwrap().translation;
    let ran = Vec2::new(end.x - start.x, end.z - start.z).length();
    let allowed = max * ticks as f32 / sim.server.tick_hz() as f32;
    println!("fastest {fastest:.3} m/s (max {max:.3}), {ran:.3} m in {ticks} ticks (at most {allowed:.3})");
    assert!(fastest <= max * 1.001, "{fastest} m/s over the {max} m/s cap");
    assert!(ran <= allowed * 1.001, "{ran} m in {ticks} ticks");
    let b = sim.server.app.world().get::<CommandBuffer>(theirs).unwrap();
    assert!(b.early > 0, "commands beyond the buffer's reach were dropped");
    assert!(b.queued.len() as u64 <= mashup::net::server::MAX_AHEAD + 1);
}

/// A clock thrown far off (a long hitch while joining runs a second of
/// ticks at once) comes back in one jump and settles: commands sent
/// before the jump (still on their way, far ahead) never steer it again
/// (`UserCmds::epoch`). Seen live: a client's clock jumping back and
/// forth by ever more, thousands of ticks a minute later.
#[test]
fn a_clock_thrown_off_settles_in_one_jump() {
    let mut sim = joined(link(70, 10, 0.0), 13, 1);
    for (k, off) in [(0, 64i64), (1, 200), (2, -40)] {
        let before = sim.clients[0].app.world().resource::<CommandClock>().jumps;
        {
            let mut clock = sim.clients[0].app.world_mut().resource_mut::<CommandClock>();
            let t = clock.tick.unwrap();
            clock.tick = Some((t as i64 + off) as u64);
        }
        sim.ticks(400);
        let clock = sim.clients[0].app.world().resource::<CommandClock>().clone();
        let jumps = clock.jumps - before;
        println!("case {k}: thrown {off:+} ticks: {jumps} jumps, lead {:.1} (target {:.1})", clock.lead, clock.target());
        assert!(jumps <= 2, "case {k}: {jumps} jumps");
        assert!((clock.lead - clock.target()).abs() < 2.0, "case {k}: settled at lead {}", clock.lead);
    }
    // And it predicts right again.
    reset_graph(&mut sim, 0);
    run_script(&mut sim, 0, 200);
    let g = graph(&sim, 0);
    assert!(g.checked > 50 && g.errors <= 1, "{} errors in {}", g.errors, g.checked);
}
