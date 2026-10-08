//! Drawing between simulation ticks (`map::interp`): at a frame rate
//! above the tick rate, what moves at the tick is drawn between its last
//! two ticks by the frame's overstep fraction, snaps on teleports, and
//! the camera's eye (position plus eased eye offset) moves every frame
//! instead of standing still and then jumping once a tick.
//!
//! `cargo test --test interpolation -- --nocapture` prints the per-frame
//! camera numbers.

use std::time::Duration;

use avian3d::prelude::*;
use bevy::{prelude::*, time::TimeUpdateStrategy};
use mashup::{
    games::cs_source::{
        TICK_INTERVAL,
        movement::{self, SourceMovementPlugin, to_engine},
    },
    harness::Sim,
    map::{
        MapBrush, MapBrushes,
        interp::{InterpPlugin, Interpolated, Interpolation, RenderedView},
    },
};

const FRAME: f64 = 1.0 / 240.0;

/// Moves along +X at a fixed speed every tick; jumps when told.
#[derive(Component)]
struct Mover {
    speed: f32,
    teleport_at: Option<u32>,
    ticks: u32,
}

fn step(time: Res<Time>, mut q: Query<(&mut Transform, &mut Mover)>) {
    for (mut t, mut m) in &mut q {
        m.ticks += 1;
        if m.teleport_at == Some(m.ticks) {
            t.translation = Vec3::new(100.0, 0.0, 0.0);
        } else {
            t.translation.x += m.speed * time.delta_secs();
        }
    }
}

fn synthetic_app() -> App {
    let mut app = App::new();
    app.add_plugins((MinimalPlugins, TransformPlugin, InterpPlugin))
        .insert_resource(Time::<Fixed>::from_seconds(TICK_INTERVAL))
        .insert_resource(TimeUpdateStrategy::ManualDuration(Duration::from_secs_f64(FRAME)))
        .add_systems(FixedUpdate, step);
    app.finish();
    app.cleanup();
    app
}

#[test]
fn drawn_between_the_last_two_ticks_and_snapped_on_teleport() {
    let mut app = synthetic_app();
    let e = app
        .world_mut()
        .spawn((
            Transform::default(),
            Mover {
                speed: 3.0,
                teleport_at: Some(40),
                ticks: 0,
            },
            Interpolated::default(),
        ))
        .id();
    let mut checked = 0;
    let mut snapped = false;
    for _ in 0..240 {
        app.update();
        let w = app.world();
        let Some((prev, cur)) = w.get::<Interpolated>(e).unwrap().ticks() else {
            continue;
        };
        let drawn = w.get::<Transform>(e).unwrap().translation;
        let f = w.resource::<Time<Fixed>>().overstep_fraction();
        if cur.translation.x == 100.0 && w.get::<Mover>(e).unwrap().ticks == 40 {
            // The teleport's tick: drawn there at once, not smeared.
            assert_eq!(drawn, cur.translation, "teleport drawn at {drawn} (from {})", prev.translation);
            snapped = true;
            continue;
        }
        let (lo, hi) = (prev.translation.x.min(cur.translation.x), prev.translation.x.max(cur.translation.x));
        assert!(drawn.x >= lo - 1e-5 && drawn.x <= hi + 1e-5, "{} outside [{lo}, {hi}]", drawn.x);
        let want = prev.translation.lerp(cur.translation, f);
        assert!((drawn - want).length() < 1e-5, "drawn {drawn}, want {want} at {f}");
        checked += 1;
    }
    assert!(snapped, "no teleport seen");
    assert!(checked > 200, "{checked}");
    assert_eq!(app.world().resource::<Interpolation>().snaps, 1);
}

#[test]
fn a_transform_written_between_ticks_is_taken_as_is() {
    let mut app = synthetic_app();
    let e = app
        .world_mut()
        .spawn((
            Transform::default(),
            Mover {
                speed: 3.0,
                teleport_at: None,
                ticks: 0,
            },
            Interpolated::default(),
        ))
        .id();
    for _ in 0..20 {
        app.update();
    }
    // As `setpos` does (console, outside the fixed tick).
    app.world_mut().get_mut::<Transform>(e).unwrap().translation = Vec3::new(-50.0, 1.0, 2.0);
    let mut seen = Vec::new();
    for _ in 0..8 {
        app.update();
        seen.push(app.world().get::<Transform>(e).unwrap().translation);
    }
    // From there on, never back toward where it was.
    for p in &seen {
        assert!(p.x >= -50.0 && p.x < -49.0 && p.y == 1.0, "{seen:?}");
    }
    assert!(seen.last().unwrap().x > -50.0, "it keeps moving: {seen:?}");
}

fn flat_floor(mut commands: Commands) {
    let (a, b) = (
        to_engine(Vec3::new(-16384.0, -16384.0, -64.0)),
        to_engine(Vec3::new(16384.0, 16384.0, 0.0)),
    );
    commands.insert_resource(MapBrushes(vec![MapBrush::from_box(a.min(b), a.max(b))]));
}

struct FlatFloor;
impl Plugin for FlatFloor {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, flat_floor);
    }
}

/// A CS:S player walking and ducking at 240 frames a second on the
/// CS:S tick; the camera's eye (drawn position + drawn eye offset)
/// per frame, with interpolation on or off.
fn camera_eyes(enabled: u8) -> Vec<Vec3> {
    let mut sim = Sim::new((FlatFloor, SourceMovementPlugin, InterpPlugin));
    sim.set_tick_interval(TICK_INTERVAL);
    sim.app.world_mut().resource_mut::<Interpolation>().enabled = enabled;
    let p = sim.spawn_character(to_engine(Vec3::new(0.0, 0.0, 36.5)), movement::ID);
    sim.ticks(10);
    sim.app
        .insert_resource(TimeUpdateStrategy::ManualDuration(Duration::from_secs_f64(FRAME)));
    {
        let mut i = sim.intent(p);
        i.move_axis = Vec2::new(0.0, 1.0);
        i.crouch = true;
    }
    let mut eyes = Vec::new();
    // Ducking takes 0.4 s; 0.3 s of frames is all in the transition.
    for _ in 0..72 {
        sim.app.update();
        let w = sim.app.world();
        let t = w.get::<Transform>(p).unwrap().translation;
        let v = w.get::<RenderedView>(p).unwrap().now;
        eyes.push(t + v.eye_offset);
    }
    eyes
}

#[test]
fn the_camera_eye_moves_every_frame_while_walking_and_ducking() {
    let on = camera_eyes(1);
    let off = camera_eyes(0);
    let deltas = |eyes: &[Vec3]| -> Vec<Vec3> { eyes.windows(2).map(|w| w[1] - w[0]).collect() };
    let (d_on, d_off) = (deltas(&on), deltas(&off));
    println!("frame  eye y (on)  dy (on, mm)  dz (on, mm) | dy (off, mm)  dz (off, mm)");
    for (k, (a, b)) in d_on.iter().zip(&d_off).enumerate() {
        println!(
            "{:>5}  {:>10.4}  {:>11.3}  {:>11.3} | {:>12.3}  {:>11.3}",
            k + 1,
            on[k + 1].y,
            a.y * 1e3,
            a.z * 1e3,
            b.y * 1e3,
            b.z * 1e3
        );
    }
    // Skip the start (speeding up from rest, the duck starting).
    let steady = |d: &[Vec3]| d[12..].to_vec();
    let (s_on, s_off) = (steady(&d_on), steady(&d_off));
    let still = |d: &[Vec3], f: fn(&Vec3) -> f32| d.iter().filter(|v| f(v).abs() < 1e-6).count();
    // On: every frame moves, eye height and forward position alike.
    assert_eq!(still(&s_on, |v| v.y), 0, "eye height stood still on some frames: {s_on:?}");
    assert_eq!(still(&s_on, |v| v.z), 0, "position stood still on some frames: {s_on:?}");
    // Off: most frames stand still (3.6 frames per tick), then one jumps.
    assert!(still(&s_off, |v| v.z) > s_off.len() / 2, "{s_off:?}");
    // On: no frame moves much more than its neighbours (no jump). The
    // ducking eye speeds up and slows down smoothly; forward speed is
    // the same each tick.
    for w in s_on.windows(2) {
        let (a, b) = (w[0].z.abs(), w[1].z.abs());
        assert!(b < a * 1.5 + 1e-5 && a < b * 1.5 + 1e-5, "forward step jumps: {} then {}", a, b);
        let (a, b) = (w[0].y.abs(), w[1].y.abs());
        assert!(b < a * 2.0 + 2e-5 && a < b * 2.0 + 2e-5, "eye height step jumps: {} then {}", a, b);
    }
    let ratio = |d: &[Vec3]| {
        let m = d.iter().map(|v| v.z.abs()).fold(0.0f32, f32::max);
        let avg = d.iter().map(|v| v.z.abs()).sum::<f32>() / d.len() as f32;
        m / avg
    };
    println!("forward step max/mean: on {:.2}, off {:.2}", ratio(&s_on), ratio(&s_off));
}

#[test]
fn physics_bodies_are_drawn_between_ticks() {
    let mut sim = Sim::new((FlatFloor, InterpPlugin));
    sim.set_tick_interval(TICK_INTERVAL);
    let body = sim
        .app
        .world_mut()
        .spawn((
            Transform::from_xyz(0.0, 20.0, 0.0),
            RigidBody::Dynamic,
            Collider::sphere(0.2),
        ))
        .id();
    sim.ticks(3);
    sim.app
        .insert_resource(TimeUpdateStrategy::ManualDuration(Duration::from_secs_f64(FRAME)));
    let mut between = 0;
    for _ in 0..60 {
        sim.app.update();
        let w = sim.app.world();
        let (prev, cur) = w.get::<Interpolated>(body).expect("bodies get Interpolated").ticks().unwrap();
        let y = w.get::<Transform>(body).unwrap().translation.y;
        assert!(y <= prev.translation.y + 1e-5 && y >= cur.translation.y - 1e-5, "{y} not in {prev:?}..{cur:?}");
        if y < prev.translation.y - 1e-6 && y > cur.translation.y + 1e-6 {
            between += 1;
        }
    }
    assert!(between > 30, "drawn strictly between ticks on {between} of 60 frames");
}
