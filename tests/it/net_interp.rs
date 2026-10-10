//! Network play, slice 3 (docs/plans/active/multiplayer.md): others drawn
//! in the past from snapshots keyed by server tick (`net::interp`,
//! `cl_interp`), and moving brushes replicated and stepped by the client
//! to the tick it predicts (`net::movers`), over `NetSim`'s in-memory link
//! with seeded latency, jitter and loss.
//!
//! - Another player walking a curve, watched at 240 frames a second, is
//!   drawn on the server's path at the render time (between the server's
//!   positions at the two ticks around it), moves every frame and by
//!   about the same distance each frame, at 50 to 150 ms with jitter and
//!   loss; its body gets the interpolated speed, look and crouch.
//! - A teleport is drawn at once, never smeared across the gap.
//! - Riding a lift up and down is predicted: the client only mispredicts
//!   when the lift starts (it learns that a round trip late), and ends
//!   where the server has it, bit for bit.
//!
//! `cargo test --features dev --test it net_interp -- --nocapture` prints
//! the numbers.

use std::{collections::BTreeMap, time::Duration};

use avian3d::prelude::{LinearVelocity, Position, RigidBody};
use bevy::prelude::*;
use mashup::{
    core::{Intent, LocalPlayer, MovementState, PredictedComponents, SimTick, Team, Velocity},
    games::cs_source::movement::{self as source, SourceMovementPlugin, to_engine},
    greybox::GreyboxMapPlugin,
    harness::NetSim,
    logic::{Logic, Value},
    map::{
        MapBrush, MapBrushEntity, MapCollision, MapConvex, MapData, MapEntity, MapHull, MapModel, MapPhysics,
        MapPlugin, MapProp, PropIndex, PropSolid, PushAway,
    },
    net::{
        self,
        interp::InterpClock,
        memory::LinkConditions,
        predict::{NetGraph, PredictionHistory},
    },
    slots::Loadout,
};

/// Frames per second the watchers draw at (between ticks: 64 Hz).
const FPS: f64 = 240.0;

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

fn graph(sim: &NetSim, i: usize) -> NetGraph {
    sim.clients[i].app.world().resource::<NetGraph>().clone()
}

fn pos(world: &World, e: Entity) -> Vec3 {
    world.get::<Transform>(e).unwrap().translation
}

/// Client `viewer`'s copy of the character owned by `owner`.
fn seen_by(sim: &mut NetSim, viewer: usize, owner: u64) -> Entity {
    NetSim::owned_by(&mut sim.clients[viewer], Some(owner)).expect("seen")
}

fn client_intent(sim: &mut NetSim, i: usize) -> Mut<'_, Intent> {
    let me = sim.local_player(i).unwrap();
    sim.clients[i].app.world_mut().get_mut::<Intent>(me).unwrap()
}

/// The server's position at a fractional tick, between its two ticks.
fn on_path(path: &BTreeMap<u64, Vec3>, at: f64) -> Option<Vec3> {
    let a = at.floor() as u64;
    let f = (at - a as f64) as f32;
    let pa = *path.get(&a)?;
    if f == 0.0 {
        return Some(pa);
    }
    Some(pa.lerp(*path.get(&(a + 1))?, f))
}

#[test]
fn others_are_drawn_smoothly_on_the_servers_path() {
    println!(" link             | frames | off path (max, mm) | step mean/max (mm) | zero steps | extrap/held | ahead");
    for (latency, jitter, loss, seed) in [(50, 10, 0.0, 31), (100, 40, 0.1, 32), (150, 60, 0.2, 33)] {
        let mut sim = NetSim::new(link(latency, jitter, loss), seed, 2, greybox);
        sim.until_joined(600);
        sim.ticks(320);
        sim.set_frame(1.0 / FPS);
        let theirs = sim.character_of(0).unwrap();
        let seen = seen_by(&mut sim, 1, 1);
        let mut path = BTreeMap::new();
        let mut drawn: Vec<(f64, Vec3, f32, f32, f32)> = Vec::new();
        let frames = (4.0 * FPS) as u32;
        for k in 0..frames {
            {
                let mut i = client_intent(&mut sim, 0);
                // Walk a curve for 3 s (yaw turning), then stop.
                i.move_axis = if k < (3.0 * FPS) as u32 { Vec2::Y } else { Vec2::ZERO };
                i.yaw = 0.3 + k as f32 * 0.002;
                i.crouch = false;
            }
            sim.step();
            let world = sim.server.app.world();
            path.insert(world.resource::<SimTick>().0, pos(world, theirs));
            let world = sim.clients[1].app.world();
            let at = world.resource::<InterpClock>().render_tick;
            let speed = world.get::<Velocity>(seen).unwrap().0.length();
            let yaw = world.get::<Intent>(seen).unwrap().yaw;
            let server_speed = sim.server.app.world().get::<Velocity>(theirs).unwrap().0.length();
            drawn.push((at, pos(world, seen), speed, yaw, server_speed));
        }
        let g = graph(&sim, 1);
        // Steady walking: after 1 s of frames (sped up, drawn 0.1 s +
        // latency behind) until the stop shows.
        let steady = &drawn[(1.2 * FPS) as usize..(3.0 * FPS) as usize];
        let mut off_max = 0.0f32;
        for (at, p, ..) in steady {
            let want = on_path(&path, *at).expect("the render time is on the server's path");
            let off = p.distance(want);
            off_max = off_max.max(off);
        }
        let steps: Vec<f32> = steady.windows(2).map(|w| w[1].1.distance(w[0].1)).collect();
        let zero = steps.iter().filter(|s| **s < 1e-5).count();
        let mean = steps.iter().sum::<f32>() / steps.len() as f32;
        let max = steps.iter().copied().fold(0.0, f32::max);
        println!(
            " {latency:>3}±{jitter:<3} ms {:>3.0} % | {:>6} | {:>18.3} | {:>8.2} / {:>7.2} | {:>10} | {:>5} / {:<5} | {:.1}",
            loss * 100.0,
            steady.len(),
            off_max * 1e3,
            mean * 1e3,
            max * 1e3,
            zero,
            g.interp_extrapolated,
            g.interp_held,
            g.interp_ahead
        );
        let what = format!("{latency}±{jitter} ms, {:.0} % loss", loss * 100.0);
        // On the server's path, up to the blend's rounding; with loss, a
        // lost snapshot is bridged by a straight line between the two
        // around it (the path wobbles a few mm in height), and an
        // extrapolated frame may stray, not far.
        if loss == 0.0 && g.interp_extrapolated == 0 {
            assert!(off_max < 1e-3, "{what}: {off_max} m off the server's path");
        }
        assert!(off_max < 0.02, "{what}: {off_max} m off the server's path");
        // Moving every frame, about as far each frame (walking 6.35 m/s:
        // 26 mm a frame).
        assert_eq!(zero, 0, "{what}: frames that stood still");
        assert!(mean > 0.02 && mean < 0.03, "{what}: {mean} m a frame");
        assert!(max < mean * 1.5, "{what}: a frame moved {max} m (mean {mean})");
        // The body walks at the interpolated speed, facing the
        // interpolated way (the curve turns 0.48 rad a second).
        for (_, _, speed, yaw, _) in steady {
            assert!((speed - 6.35).abs() < 0.3, "{what}: drawn speed {speed}");
            assert!(*yaw > 0.3 && *yaw < 0.3 + frames as f32 * 0.002, "{what}: yaw {yaw}");
        }
        // Stopped: drawn where the server has it.
        sim.ticks(FPS as u64);
        let end = pos(sim.server.app.world(), theirs);
        let there = pos(sim.clients[1].app.world(), seen);
        assert!(there.distance(end) < 1e-4, "{what}: drawn at {there}, the server has {end}");
        assert!((g.interp_ms - 100.0).abs() < 1e-3, "{what}: cl_interp {}", g.interp_ms);
    }
}

#[test]
fn crouching_and_looking_others_are_drawn_from_snapshots() {
    let mut sim = NetSim::new(link(60, 10, 0.0), 41, 2, greybox);
    sim.until_joined(600);
    sim.ticks(320);
    {
        let mut i = client_intent(&mut sim, 0);
        i.yaw = 1.0;
        i.pitch = -0.25;
        i.crouch = true;
    }
    // Ducking takes 0.4 s; drawn 0.1 s and the latency later.
    sim.ticks(80);
    let seen = seen_by(&mut sim, 1, 1);
    let theirs = sim.character_of(0).unwrap();
    let world = sim.clients[1].app.world();
    let i = world.get::<Intent>(seen).unwrap();
    assert!((i.yaw - 1.0).abs() < 1e-5 && (i.pitch + 0.25).abs() < 1e-5, "{} {}", i.yaw, i.pitch);
    let s = world.get::<MovementState>(seen).unwrap();
    assert!(s.crouching, "drawn crouched");
    let server_eye = sim
        .server
        .app
        .world()
        .get::<MovementState>(theirs)
        .unwrap()
        .eye_offset;
    assert!(s.eye_offset.distance(server_eye) < 1e-5, "{} vs {server_eye}", s.eye_offset);
}

#[test]
fn a_teleport_is_drawn_at_once() {
    let mut sim = NetSim::new(link(80, 20, 0.05), 51, 1, greybox);
    // The listen server's own player, standing; the server moves it 3 m
    // in one tick (further than anything moves in a tick).
    let host = sim.server.spawn_character(Vec3::new(6.0, 1.0, 0.0), source::ID);
    sim.server.app.world_mut().entity_mut(host).insert((LocalPlayer, Team(2)));
    sim.until_joined(600);
    sim.ticks(200);
    sim.set_frame(1.0 / FPS);
    let seen = seen_by(&mut sim, 0, net::HOST_ID);
    let x0 = pos(sim.server.app.world(), host).x;
    let mut drawn = Vec::new();
    for k in 0..(2.0 * FPS) as u32 {
        if k == FPS as u32 {
            let world = sim.server.app.world_mut();
            world.get_mut::<Transform>(host).unwrap().translation.x -= 3.0;
        }
        sim.step();
        drawn.push(pos(sim.clients[0].app.world(), seen).x);
    }
    assert!((drawn[0] - x0).abs() < 1e-4, "{} vs {x0}", drawn[0]);
    assert!((drawn.last().unwrap() - (x0 - 3.0)).abs() < 1e-4, "{drawn:?}");
    // Nothing drawn in between.
    let smeared: Vec<f32> = drawn.iter().copied().filter(|x| *x < x0 - 0.01 && *x > x0 - 2.99).collect();
    assert!(smeared.is_empty(), "drawn in the gap: {smeared:?}");
    assert!(graph(&sim, 0).interp_snaps >= 1);
}

// --- Moving brushes.

const SCALE: f32 = 0.0254;

fn hull(lo: Vec3, hi: Vec3) -> MapHull {
    let b = MapBrush::from_box(lo, hi);
    let points = (0..8)
        .map(|i| {
            Vec3::new(
                if i & 1 == 0 { lo.x } else { hi.x },
                if i & 2 == 0 { lo.y } else { hi.y },
                if i & 4 == 0 { lo.z } else { hi.z },
            )
        })
        .collect();
    MapHull { planes: b.planes, points }
}

/// A floor at z = 0 and a 128x128 lift whose top is at z = 8, rising
/// 100 units at 100 units/s on `Open` and back on `Close`; players spawn
/// on it.
fn lift_map() -> MapData {
    let (a, b) = (to_engine(Vec3::new(-4096.0, -4096.0, -64.0)), to_engine(Vec3::new(4096.0, 4096.0, 0.0)));
    let lift = MapEntity {
        keyvalues: [
            ("classname", "func_door"),
            ("targetname", "lift"),
            ("origin", "0 0 8"),
            ("movedir", "-90 0 0"),
            ("lip", "-94"),
            ("speed", "100"),
            ("wait", "-1"),
            ("spawnflags", "256"),
        ]
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect(),
        hulls: vec![hull(Vec3::new(-64.0, -64.0, -8.0), Vec3::ZERO)],
        mover: true,
        physics: None,
    };
    MapData {
        name: "test:lift".into(),
        collision_brushes: vec![MapBrush::from_box(a.min(b), a.max(b))],
        entities: vec![lift],
        entity_scale: SCALE,
        spawns: vec![(to_engine(Vec3::new(0.0, 0.0, 8.0)), None)],
        ..default()
    }
}

fn lift_world(app: &mut App) {
    app.add_plugins((MapPlugin::new(lift_map()), SourceMovementPlugin))
        .insert_resource(Loadout { movement: source::ID });
}

fn lift_input(sim: &mut NetSim, input: &str) {
    sim.server
        .app
        .world_mut()
        .resource_mut::<Logic>()
        .world
        .queue_input("lift", input, Value::Void, 0.0, None);
}

/// The client's and the server's lift nodes.
fn lift_node(world: &mut World) -> Entity {
    world
        .query::<(Entity, &MapBrushEntity)>()
        .iter(world)
        .find(|(_, n)| n.0 == 0)
        .unwrap()
        .0
}

/// Step until the server's lift stops; returns the ticks with a new
/// prediction error (from the start) and the steps taken.
fn ride(sim: &mut NetSim, max: u32) -> (Vec<u32>, u32) {
    let node = lift_node(sim.server.app.world_mut());
    let mut errors = Vec::new();
    let mut last = graph(sim, 0).errors;
    let mut still = 0;
    let mut prev = pos(sim.server.app.world(), node);
    for k in 0..max {
        sim.step();
        let g = graph(sim, 0);
        if g.errors > last {
            errors.push(k);
            last = g.errors;
        }
        let now = pos(sim.server.app.world(), node);
        still = if now == prev { still + 1 } else { 0 };
        prev = now;
        if still > 64 && k > 64 {
            return (errors, k);
        }
    }
    (errors, max)
}

#[test]
fn riding_a_lift_is_predicted() {
    for (latency, jitter, seed) in [(50, 0, 61), (100, 20, 62), (150, 40, 63)] {
        let mut sim = NetSim::new(link(latency, jitter, 0.0), seed, 1, lift_world);
        sim.until_joined(600);
        sim.ticks(320);
        let theirs = sim.character_of(0).unwrap();
        let feet0 = pos(sim.server.app.world(), theirs).y;
        // The client has the lift where the server does, as a solid.
        let client_node = lift_node(sim.clients[0].app.world_mut());
        let server_node = lift_node(sim.server.app.world_mut());
        assert_eq!(
            pos(sim.clients[0].app.world(), client_node),
            pos(sim.server.app.world(), server_node)
        );
        *sim.clients[0].app.world_mut().resource_mut::<NetGraph>() = NetGraph::default();

        lift_input(&mut sim, "Open");
        let (up_errors, up_ticks) = ride(&mut sim, 400);
        let top = pos(sim.server.app.world(), theirs).y;
        let carried = graph(&sim, 0).carried;
        lift_input(&mut sim, "Close");
        let (down_errors, down_ticks) = ride(&mut sim, 400);
        let bottom = pos(sim.server.app.world(), theirs).y;
        let g = graph(&sim, 0);
        println!(
            "{latency}±{jitter} ms: up {:.2} m in {up_ticks} ticks, errors at {up_errors:?}; down to {:.2} m in \
             {down_ticks}, errors at {down_errors:?}; carried {carried} ticks, {} of {} states wrong, worst {:.3} m",
            top - feet0,
            bottom - feet0,
            g.errors,
            g.checked,
            g.worst_error
        );
        let what = format!("{latency}±{jitter} ms");
        // Rode it up 100 units and back down.
        assert!((top - feet0 - 100.0 * SCALE).abs() < 0.05, "{what}: rose {}", top - feet0);
        assert!((bottom - feet0).abs() < 0.05, "{what}: back at {}", bottom - feet0);
        assert!(carried > 30, "{what}: carried {carried} ticks");
        // Only the starts mispredict (the client hears of them a round
        // trip late): errors only in the first round trip and a bit of
        // each ride, none while riding, none at the stops.
        let window = ((2 * latency + 2 * jitter) as f64 / 1000.0 * 64.0) as u32 + 8;
        for (dir, errs) in [("up", &up_errors), ("down", &down_errors)] {
            assert!(
                errs.iter().all(|k| *k <= window),
                "{what}: {dir}: errors after the start's round trip ({errs:?}, window {window})"
            );
            assert!(errs.len() <= 3, "{what}: {dir}: {errs:?}");
        }
        // And the client ends where the server has it, bit for bit.
        sim.ticks(30);
        let mut server = std::collections::HashMap::new();
        for _ in 0..30 {
            sim.step();
            let world = sim.server.app.world();
            let tick = world.resource::<SimTick>().0;
            server.insert(tick, world.resource::<PredictedComponents>().encode(world, theirs));
        }
        let history = &sim.clients[0].app.world().resource::<PredictionHistory>().0;
        let mut compared = 0;
        for p in history.iter().filter(|p| !p.state.is_empty()) {
            if let Some(s) = server.get(&p.tick) {
                assert!(*s == p.state, "{what}: tick {} isn't the server's", p.tick);
                compared += 1;
            }
        }
        assert!(compared > 0, "{what}: nothing compared");
    }
}

// --- Physics props.

/// A floor and a 20 kg crate (map entity 0, a prop_physics) resting on
/// it; players spawn away from it.
fn crate_map() -> MapData {
    let (a, b) = (to_engine(Vec3::new(-4096.0, -4096.0, -64.0)), to_engine(Vec3::new(4096.0, 4096.0, 0.0)));
    let (a, b) = (a.min(b), a.max(b));
    let mut data = MapData {
        name: "test:crate".into(),
        collision_brushes: vec![MapBrush::from_box(a, b)],
        collision_hulls: vec![
            (0..8)
                .map(|k| [[a.x, b.x][k & 1], [a.y, b.y][(k >> 1) & 1], [a.z, b.z][(k >> 2) & 1]])
                .collect(),
        ],
        entities: vec![MapEntity {
            keyvalues: vec![("classname".into(), "prop_physics".into()), ("origin".into(), "0 0 16".into())],
            hulls: Vec::new(),
            mover: false,
            physics: None,
        }],
        entity_scale: SCALE,
        spawns: vec![(to_engine(Vec3::new(400.0, 0.0, 0.0)), None)],
        ..default()
    };
    let half = 16.0 * SCALE;
    let (lo, hi) = (Vec3::splat(-half), Vec3::splat(half));
    let corners = (0..8)
        .map(|k| Vec3::new([lo.x, hi.x][k & 1], [lo.y, hi.y][(k >> 1) & 1], [lo.z, hi.z][(k >> 2) & 1]))
        .collect();
    data.models.push(MapModel {
        bounds: (lo, hi),
        collision: Some(MapCollision {
            pieces: vec![MapConvex { points: corners, planes: Vec::new() }],
            mass: 20.0,
            ..default()
        }),
        ..default()
    });
    data.props.push(MapProp {
        pose: None,
        ragdoll: None,
        model: 0,
        translation: to_engine(Vec3::new(0.0, 0.0, 16.0)),
        rotation: Quat::IDENTITY,
        solid: PropSolid::Mesh,
        skybox: false,
        lighting: None,
        vertex_light: None,
        casts_shadow: false,
        physics: Some(MapPhysics {
            mass: 20.0,
            friction: 0.8,
            elasticity: 0.0,
            damping: 0.0,
            rotdamping: 0.0,
            push: PushAway::Collide,
            frozen: false,
            asleep: false,
        }),
        fade: None,
        parent: None,
        entity: Some(0),
        skin: 0,
        body: 0,
    });
    data
}

fn crate_world(app: &mut App) {
    app.add_plugins((MapPlugin::new(crate_map()), SourceMovementPlugin))
        .insert_resource(Loadout { movement: source::ID });
}

fn crate_node(world: &mut World) -> Entity {
    world.query::<(Entity, &PropIndex)>().iter(world).next().unwrap().0
}

#[test]
fn physics_props_are_drawn_as_the_server_simulates_them() {
    let mut sim = NetSim::new(link(60, 15, 0.05), 71, 1, crate_world);
    sim.until_joined(600);
    sim.ticks(200);
    let theirs = crate_node(sim.server.app.world_mut());
    let ours = crate_node(sim.clients[0].app.world_mut());
    // At rest where the server has it; not simulated here.
    let rest = pos(sim.server.app.world(), theirs);
    assert!(pos(sim.clients[0].app.world(), ours).distance(rest) < 1e-5);
    assert_eq!(
        sim.clients[0].app.world().get::<RigidBody>(ours),
        Some(&RigidBody::Kinematic),
        "the client doesn't simulate it"
    );
    // The server lifts it 2 m: it falls back.
    {
        let world = sim.server.app.world_mut();
        let up = rest + Vec3::Y * 2.0;
        world.get_mut::<Transform>(theirs).unwrap().translation = up;
        world.get_mut::<Position>(theirs).unwrap().0 = up;
        world.get_mut::<LinearVelocity>(theirs).unwrap().0 = Vec3::ZERO;
    }
    sim.set_frame(1.0 / FPS);
    let mut path = BTreeMap::new();
    let mut drawn = Vec::new();
    for _ in 0..(1.5 * FPS) as u32 {
        sim.step();
        let world = sim.server.app.world();
        path.insert(world.resource::<SimTick>().0, pos(world, theirs));
        let world = sim.clients[0].app.world();
        drawn.push((world.resource::<InterpClock>().render_tick, pos(world, ours)));
    }
    // Drawn on the server's path once the lift is behind the render time
    // (the jump itself is drawn at once).
    let top = drawn.iter().map(|(_, p)| p.y).fold(f32::MIN, f32::max);
    assert!((top - (rest.y + 2.0)).abs() < 0.05, "drawn up at {top}");
    let lifted = *path.iter().find(|(_, p)| p.y > rest.y + 1.0).unwrap().0;
    let mut on = 0;
    for (at, p) in &drawn {
        // (A lost snapshot right at the jump holds the old pose until the
        // next one: the jump is drawn at once, a tick or two late.)
        if *at < lifted as f64 + 3.0 {
            continue;
        }
        if let Some(want) = on_path(&path, *at) {
            assert!(p.distance(want) < 0.03, "drawn {p}, the server's path {want} at {at}");
            on += 1;
        }
    }
    assert!(on > 200, "{on} frames compared");
    // Falling, it moves every frame between the jump and the landing.
    let falling: Vec<f32> = drawn
        .windows(2)
        .map(|w| w[0].1.y - w[1].1.y)
        .filter(|d| *d > 0.0 && *d < 1.0)
        .collect();
    assert!(falling.len() > 60, "{} frames falling", falling.len());
    sim.ticks(FPS as u64);
    let end = pos(sim.server.app.world(), theirs);
    assert!(pos(sim.clients[0].app.world(), ours).distance(end) < 1e-4, "landed where the server has it");
}
