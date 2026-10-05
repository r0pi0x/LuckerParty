//! Test cases from specs/cs_source/movement.md, run headless on a small map
//! built in Source units: a floor at z = 0, an 18-unit step and a 19-unit
//! ledge. 64 tick, knife speed 250, sv_accelerate 5, sv_stopspeed 100.

use bevy::prelude::*;
use mashup::{
    core::Velocity,
    games::cs_source::movement::{
        self, SourceMovement, SourceMovementConfig, SourceMovementPlugin, to_engine, to_source,
    },
    harness::Sim,
    map::{MapBrush, MapBrushes},
};

/// A solid box from `lo` to `hi` in Source units, as a map brush.
fn solid(brushes: &mut Vec<MapBrush>, lo: Vec3, hi: Vec3) {
    let (a, b) = (to_engine(lo), to_engine(hi));
    brushes.push(MapBrush::from_box(a.min(b), a.max(b)));
}

const STEP_Y: f32 = 0.0;
const LEDGE_Y: f32 = 1500.0;

fn test_map(mut commands: Commands) {
    let mut brushes = Vec::new();
    solid(
        &mut brushes,
        Vec3::new(-8192.0, -8192.0, -64.0),
        Vec3::new(8192.0, 8192.0, 0.0),
    );
    // 18-unit step (climbable) and 19-unit ledge (not), both from x = 256.
    solid(
        &mut brushes,
        Vec3::new(256.0, STEP_Y - 128.0, 0.0),
        Vec3::new(512.0, STEP_Y + 128.0, 18.0),
    );
    solid(
        &mut brushes,
        Vec3::new(256.0, LEDGE_Y - 128.0, 0.0),
        Vec3::new(512.0, LEDGE_Y + 128.0, 19.0),
    );
    commands.insert_resource(MapBrushes(brushes));
}

struct TestMap;
impl Plugin for TestMap {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, test_map);
    }
}

struct Player {
    sim: Sim,
    p: Entity,
}

impl Player {
    /// A player whose feet start at `feet`, settled for a few ticks.
    fn at(feet: Vec3) -> Self {
        let mut sim = Sim::new((TestMap, SourceMovementPlugin));
        // The spec's test cases use the shared code's values.
        sim.app.insert_resource(SourceMovementConfig::shared_code());
        let p = sim.spawn_character(to_engine(feet + Vec3::Z * 36.0), movement::ID);
        let mut me = Self { sim, p };
        me.look_yaw(0.0);
        me
    }

    fn on_floor() -> Self {
        let mut me = Self::at(Vec3::new(0.0, -3000.0, 0.5));
        me.sim.ticks(4);
        assert!(me.state().on_ground, "should settle on the floor");
        me
    }

    /// Face Source yaw `deg` (0 = +X).
    fn look_yaw(&mut self, deg: f32) {
        self.sim.intent(self.p).yaw = deg.to_radians() - std::f32::consts::FRAC_PI_2;
    }

    fn feet(&self) -> Vec3 {
        to_source(self.sim.position(self.p)) - Vec3::Z * 36.0
    }

    fn vel(&self) -> Vec3 {
        to_source(self.sim.velocity(self.p))
    }

    fn set_vel(&mut self, v: Vec3) {
        self.sim.app.world_mut().get_mut::<Velocity>(self.p).unwrap().0 = to_engine(v);
    }

    fn speed(&self) -> f32 {
        self.vel().truncate().length()
    }

    fn state(&self) -> SourceMovement {
        self.sim.app.world().get::<SourceMovement>(self.p).unwrap().clone()
    }

    fn tick(&mut self) {
        self.sim.ticks(1);
    }
}

fn close(actual: f32, expected: f32, tol: f32, what: &str) {
    assert!(
        (actual - expected).abs() <= tol,
        "{what}: got {actual}, expected {expected} (±{tol})"
    );
}

#[test]
fn ground_acceleration_from_rest() {
    let mut pl = Player::on_floor();
    let x0 = pl.feet().x;
    pl.sim.intent(pl.p).move_axis = Vec2::Y;
    let expected = [
        (1, 19.53125),
        (2, 32.8125),
        (3, 46.09375),
        (7, 99.21875),
        (8, 112.5),
        (9, 125.0),
        (10, 136.71875),
        (25, 245.73),
        (26, 249.91),
        (27, 250.0),
    ];
    let mut tick = 0;
    for (at, speed) in expected {
        while tick < at {
            pl.tick();
            tick += 1;
            if tick == 1 {
                close(pl.feet().x - x0, 0.30518, 0.001, "moved on tick 1");
            }
        }
        close(pl.speed(), speed, 0.01, &format!("speed after {at} ticks"));
    }
    close(pl.feet().x - x0, 67.84, 0.05, "distance after 27 ticks");
    for _ in 0..20 {
        pl.tick();
        close(pl.speed(), 250.0, 0.01, "holds 250");
    }
    assert!(pl.vel().y.abs() < 1e-3, "moves along +x only");
}

#[test]
fn diagonal_input_is_not_faster() {
    let mut pl = Player::on_floor();
    pl.sim.intent(pl.p).move_axis = Vec2::new(1.0, 1.0);
    pl.tick();
    close(pl.speed(), 19.53125, 0.01, "forward+right speed after 1 tick");
    let v = pl.vel();
    close(v.y.atan2(v.x).to_degrees(), -45.0, 0.01, "45 degrees right of view");
}

#[test]
fn ground_friction_stops_the_player() {
    let mut pl = Player::on_floor();
    let x0 = pl.feet().x;
    pl.set_vel(Vec3::new(250.0, 0.0, 0.0));
    let expected = [
        (1, 234.375),
        (2, 219.727),
        (14, 101.28),
        (15, 94.95),
        (16, 88.70),
        (30, 1.20),
        (31, 0.0),
    ];
    let mut tick = 0;
    for (at, speed) in expected {
        while tick < at {
            pl.tick();
            tick += 1;
        }
        close(pl.speed(), speed, 0.01, &format!("speed after {at} ticks"));
    }
    close(pl.feet().x - x0, 46.875, 0.05, "stopping distance");
}

#[test]
fn standing_jump() {
    let mut pl = Player::on_floor();
    let z0 = pl.feet().z;
    pl.sim.intent(pl.p).jump = true;
    pl.tick();
    close(pl.feet().z - z0, 3.9973, 0.001, "height after the jump tick");
    close(pl.vel().z, 249.578, 0.01, "vz at the end of the jump tick");
    let mut apex = (0, 0.0f32);
    let mut landed = None;
    for tick in 2..=60 {
        pl.tick();
        let z = pl.feet().z - z0;
        if z > apex.1 {
            apex = (tick, z);
        }
        if landed.is_none() && pl.state().on_ground {
            landed = Some(tick);
        }
    }
    assert_eq!(apex.0, 21, "apex tick");
    close(apex.1, 42.928, 0.01, "apex height");
    assert_eq!(landed, Some(42), "ground regained on tick 42");
    close(pl.state().last_landing_speed, 250.42, 0.05, "landing speed");
    // Jump is still held: no second jump.
    pl.sim.ticks(30);
    assert!(pl.feet().z - z0 < 0.1, "held jump must not re-jump");
}

#[test]
fn ducked_jump_is_higher() {
    let mut pl = Player::on_floor();
    pl.sim.intent(pl.p).crouch = true;
    pl.sim.ticks(30);
    assert!(pl.state().ducked);
    let z0 = pl.feet().z;
    pl.sim.intent(pl.p).jump = true;
    let mut apex = (0, 0.0f32);
    for tick in 1..=40 {
        pl.tick();
        let z = pl.feet().z - z0;
        if z > apex.1 {
            apex = (tick, z);
        }
    }
    assert_eq!(apex.0, 21, "apex tick");
    close(apex.1, 44.98, 0.01, "ducked apex");
}

#[test]
fn air_strafing_gains_speed() {
    let mut pl = Player::at(Vec3::new(0.0, -6000.0, 3000.0));
    pl.set_vel(Vec3::new(250.0, 0.0, 0.0));
    pl.sim.intent(pl.p).move_axis = Vec2::X; // hold right only
    let mut speeds = Vec::new();
    for _ in 0..64 {
        // Keep the view along the velocity so "right" stays perpendicular.
        let v = pl.vel();
        pl.look_yaw(v.y.atan2(v.x).to_degrees());
        pl.tick();
        speeds.push(pl.speed());
    }
    close(speeds[0], 251.794, 0.01, "after 1 tick");
    close(speeds[9], 267.395, 0.02, "after 10 ticks");
    close(speeds[63], 346.554, 0.05, "after 64 ticks");
}

#[test]
fn air_acceleration_caps_at_30() {
    let mut pl = Player::at(Vec3::new(0.0, -6000.0, 3000.0));
    pl.sim.intent(pl.p).move_axis = Vec2::Y;
    pl.tick();
    close(pl.speed(), 30.0, 0.01, "1 tick");
    pl.sim.ticks(5);
    close(pl.speed(), 30.0, 0.01, "stays at 30");
}

#[test]
fn air_wish_angles() {
    for (angle, expected) in [(85.0f32, 250.85), (80.0, 250.0), (95.0, 249.65)] {
        let mut pl = Player::at(Vec3::new(0.0, -6000.0, 3000.0));
        pl.set_vel(Vec3::new(250.0, 0.0, 0.0));
        // Forward input, view turned `angle` degrees from the velocity.
        pl.look_yaw(angle);
        pl.sim.intent(pl.p).move_axis = Vec2::Y;
        pl.tick();
        close(pl.speed(), expected, 0.01, &format!("wish {angle} degrees from v"));
    }
}

#[test]
fn falls_and_lands() {
    for (h, tick, speed) in [(64.0f32, 26, 312.5f32), (128.0, 36, 437.5), (256.0, 51, 625.0)] {
        let mut pl = Player::at(Vec3::new(0.0, -3000.0, h));
        let mut landed = None;
        for t in 1..=80 {
            pl.tick();
            if pl.state().on_ground {
                landed = Some(t);
                break;
            }
        }
        // The first tick in the harness runs before our counting starts
        // (Sim::new settles one tick), so allow one tick either way.
        let landed = landed.expect("should land");
        assert!(
            (landed as i32 - tick).abs() <= 1,
            "drop {h}: landed on tick {landed}, expected {tick}"
        );
        close(
            pl.state().last_landing_speed,
            speed,
            12.6,
            &format!("drop {h} landing speed"),
        );
    }
}

#[test]
fn duck_and_unduck_timing() {
    let mut pl = Player::on_floor();
    pl.sim.intent(pl.p).crouch = true;
    pl.tick(); // tick 0: the press
    close(pl.state().eye, 64.0, 0.01, "press tick eye");
    pl.sim.ticks(13);
    close(pl.state().eye, 45.58, 0.01, "tick 13 eye");
    pl.sim.ticks(12);
    assert!(!pl.state().ducked, "tick 25 still standing box");
    pl.tick();
    assert!(pl.state().ducked, "tick 26 ducked");
    close(pl.state().eye, 28.0, 0.01, "ducked eye");

    pl.sim.intent(pl.p).crouch = false;
    pl.tick(); // tick 0: the release
    close(pl.state().eye, 28.0, 0.01, "release tick eye");
    pl.sim.ticks(12);
    assert!(pl.state().ducked, "tick 12 still ducked (187.5 ms)");
    pl.tick();
    assert!(!pl.state().ducked, "standing box after 13 ticks");
    close(pl.state().eye, 64.0, 0.01, "standing eye");
}

#[test]
fn ducked_walking_is_a_third() {
    let mut pl = Player::on_floor();
    pl.sim.intent(pl.p).crouch = true;
    pl.sim.ticks(30);
    pl.sim.intent(pl.p).move_axis = Vec2::Y;
    pl.tick();
    close(pl.speed(), 6.5104, 0.001, "ducked speed after 1 tick");
}

#[test]
fn air_duck_lifts_feet() {
    let mut pl = Player::at(Vec3::new(0.0, -3000.0, 500.0));
    let z = pl.feet().z;
    pl.sim.intent(pl.p).crouch = true;
    pl.tick();
    assert!(pl.state().ducked, "ducking in the air is instant");
    // The 36-unit lift, minus one tick of falling from rest (6.25 / 64).
    close(pl.feet().z - z, 36.0 - 6.25 / 64.0, 0.001, "feet tuck up");
    close(pl.state().eye, 28.0, 0.01, "eye");
}

#[test]
fn climbs_an_18_unit_step() {
    let mut pl = Player::at(Vec3::new(0.0, STEP_Y, 0.5));
    pl.sim.ticks(4);
    pl.sim.intent(pl.p).move_axis = Vec2::Y;
    let mut max_z = 0.0f32;
    for _ in 0..(3 * 64) {
        pl.tick();
        max_z = max_z.max(pl.feet().z);
        if pl.feet().x > 300.0 {
            break;
        }
    }
    assert!(pl.feet().x > 300.0, "should get onto the step, x = {}", pl.feet().x);
    close(pl.feet().z, 18.0, 0.1, "standing on the step");
    assert!(pl.state().on_ground);
}

#[test]
fn blocked_by_a_19_unit_ledge() {
    let mut pl = Player::at(Vec3::new(-300.0, LEDGE_Y, 0.5));
    pl.sim.ticks(4);
    pl.sim.intent(pl.p).move_axis = Vec2::Y;
    pl.sim.ticks(3 * 64);
    assert!(
        pl.feet().x < 256.0 - 16.0 + 0.1,
        "passed the ledge: x = {}",
        pl.feet().x
    );
    close(pl.feet().z, 0.0, 0.1, "still on the floor");
}

#[test]
fn walks_down_a_step_without_falling() {
    let mut pl = Player::at(Vec3::new(300.0, STEP_Y, 18.5));
    pl.sim.ticks(4);
    assert!(pl.state().on_ground);
    pl.sim.intent(pl.p).move_axis = Vec2::Y;
    for _ in 0..(2 * 64) {
        pl.tick();
        assert!(pl.state().on_ground, "left the ground at x = {}", pl.feet().x);
    }
    assert!(pl.feet().x > 600.0, "walked off the far edge");
    close(pl.feet().z, 0.0, 0.1, "down on the floor");
}

#[test]
fn css_jump_reaches_its_measured_height() {
    // CS:S's own jump (measured apex 54.75 at 66.67 tick); at 64 tick the
    // same impulse peaks at 54.65.
    let mut pl = Player::on_floor();
    pl.sim.app.insert_resource(SourceMovementConfig::default());
    let z0 = pl.feet().z;
    pl.sim.intent(pl.p).jump = true;
    let mut apex = 0.0f32;
    for _ in 0..60 {
        pl.tick();
        apex = apex.max(pl.feet().z - z0);
    }
    close(apex, 54.654, 0.01, "CS:S jump apex at 64 tick");
}
