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
const STEP16_Y: f32 = 3000.0;
const CEILING_Y: f32 = 4500.0;

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
    // 16-unit step, for walking down.
    solid(
        &mut brushes,
        Vec3::new(256.0, STEP16_Y - 128.0, 0.0),
        Vec3::new(512.0, STEP16_Y + 128.0, 16.0),
    );
    // A low ceiling 86 units up: room to stand (72), not to jump upright.
    solid(
        &mut brushes,
        Vec3::new(-256.0, CEILING_Y - 256.0, 86.0),
        Vec3::new(256.0, CEILING_Y + 256.0, 120.0),
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
    // A 16-unit step: well within the stick-to-ground reach. (An exactly
    // 18-unit drop sits on the edge of the reach, where float rounding
    // decides; to be measured against CS:S with movecmp.)
    let mut pl = Player::at(Vec3::new(300.0, STEP16_Y, 16.5));
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

/// Perfect perpendicular strafing through a whole standing jump from 250:
/// the slow top of the jump (rising at up to 140) strafes at a quarter of
/// the rate, the rest at full rate. Spec: lands at about 302.41.
#[test]
fn strafing_through_a_jump() {
    let mut pl = Player::on_floor();
    pl.set_vel(Vec3::new(250.0, 0.0, 0.0));
    pl.sim.intent(pl.p).jump = true;
    pl.sim.intent(pl.p).move_axis = Vec2::X; // hold right
    let mut ticks = 0;
    loop {
        let v = pl.vel();
        pl.look_yaw(v.y.atan2(v.x).to_degrees());
        pl.tick();
        pl.sim.intent(pl.p).jump = false;
        ticks += 1;
        if pl.state().on_ground || ticks > 80 {
            break;
        }
    }
    close(pl.speed(), 302.41, 0.5, "landing speed after a strafed jump");
}

/// The ducked speed scale is decided before this tick's duck processing:
/// the tick the duck completes still moves at full input (spec).
#[test]
fn duck_slowdown_starts_the_tick_after_the_duck_completes() {
    let mut ducking = Player::on_floor();
    let mut plain = Player::on_floor();
    ducking.sim.intent(ducking.p).crouch = true;
    for pl in [&mut ducking, &mut plain] {
        pl.sim.intent(pl.p).move_axis = Vec2::Y;
    }
    // The press is the spec's tick 0, so the duck completes on our 27th tick.
    for tick in 1..=28 {
        ducking.tick();
        plain.tick();
        let (a, b) = (ducking.speed(), plain.speed());
        if tick <= 27 {
            close(
                a,
                b,
                1e-3,
                &format!("tick {tick}: unscaled until the duck has completed"),
            );
        } else {
            assert!(a < b - 1.0, "tick {tick}: ducked input should be scaled ({a} vs {b})");
        }
    }
    assert!(ducking.state().ducked);
}

/// Releasing duck in the air needs room above the ducked head too: under a
/// low ceiling the player stays ducked (spec, "Can-stand test").
#[test]
fn stays_ducked_in_the_air_under_a_ceiling() {
    let mut pl = Player::at(Vec3::new(0.0, CEILING_Y, 0.5));
    pl.sim.ticks(4);
    pl.sim.intent(pl.p).crouch = true;
    pl.sim.ticks(30);
    assert!(pl.state().ducked);
    // Ducked jump: the head stops at the ceiling.
    pl.sim.intent(pl.p).jump = true;
    pl.tick();
    pl.sim.intent(pl.p).jump = false;
    pl.sim.ticks(3);
    pl.sim.intent(pl.p).crouch = false;
    let mut ducked_in_air = 0;
    for _ in 0..30 {
        pl.tick();
        if pl.state().on_ground {
            break;
        }
        assert!(
            pl.state().ducked,
            "stood up in the air under the ceiling at {}",
            pl.feet()
        );
        ducked_in_air += 1;
    }
    assert!(ducked_in_air > 3, "barely left the ground ({ducked_in_air} ticks)");
}

/// Moved by game code onto the floor: ground detection runs at the start
/// of the next tick, so that tick already has ground friction.
#[test]
fn teleport_onto_the_floor_is_grounded_at_once() {
    let mut pl = Player::at(Vec3::new(0.0, -3000.0, 500.0));
    pl.sim.ticks(2);
    assert!(!pl.state().on_ground);
    let floor = to_engine(Vec3::new(0.0, -3000.0, 0.03125 + 36.0));
    pl.sim.app.world_mut().get_mut::<Transform>(pl.p).unwrap().translation = floor;
    pl.set_vel(Vec3::new(250.0, 0.0, 0.0));
    pl.tick();
    close(
        pl.speed(),
        234.375,
        0.01,
        "friction on the first tick after the teleport",
    );
}

/// Fall damage (specs/cs_source/fall_damage.md), CS:S values at its 0.015 s
/// tick: dropped from 40 units with a downward start speed, as on the probe
/// server. Lands with the probe's fall speed and takes its damage.
#[test]
fn fall_damage_as_measured() {
    use mashup::core::Health;
    // (start speed, measured fall speed, measured damage); None: dead.
    let cases = [
        (400.0f32, 460.0f32, Some(0.0f32)),
        (530.0, 578.0, Some(0.0)),
        (537.0, 585.0, Some(1.0)),
        (600.0, 648.0, Some(16.0)),
        (800.0, 836.0, Some(61.0)),
        (960.0, 984.0, Some(97.0)),
        (972.0, 996.0, None),
        (1100.0, 1124.0, None),
    ];
    for (start, fall, damage) in cases {
        let mut sim = Sim::new((TestMap, SourceMovementPlugin));
        sim.set_tick_interval(mashup::games::cs_source::TICK_INTERVAL);
        let p = sim.spawn_character(to_engine(Vec3::new(0.0, -3000.0, 40.0 + 36.0)), movement::ID);
        sim.app.world_mut().get_mut::<Velocity>(p).unwrap().0 = to_engine(Vec3::Z * -start);
        sim.seconds(0.3);
        let me = sim.app.world().get::<SourceMovement>(p).unwrap().clone();
        assert!(me.on_ground, "start {start}: not landed");
        close(me.last_landing_speed, fall, 0.01, &format!("start {start}: fall speed"));
        let health = sim.app.world().get::<Health>(p).unwrap().current;
        match damage {
            Some(d) => close(health, 1.0 - d / 100.0, 1e-5, &format!("start {start}: health")),
            None => assert_eq!(health, 0.0, "start {start}: survived a fatal fall"),
        }
    }
}

/// No fall damage landing in shallow water (feet wet): the landing check
/// skips damage in water.
#[test]
fn no_fall_damage_into_water() {
    use mashup::{
        core::Health,
        map::{MapWater, MapWaterVolume},
    };
    let mut sim = Sim::new((TestMap, SourceMovementPlugin));
    sim.set_tick_interval(mashup::games::cs_source::TICK_INTERVAL);
    let (a, b) = (to_engine(Vec3::new(-200.0, -3200.0, 0.0)), to_engine(Vec3::new(200.0, -2800.0, 10.0)));
    sim.app.insert_resource(MapWater(vec![MapWaterVolume {
        brush: MapBrush::from_box(a.min(b), a.max(b)),
        slime: false,
    }]));
    let p = sim.spawn_character(to_engine(Vec3::new(0.0, -3000.0, 40.0 + 36.0)), movement::ID);
    sim.app.world_mut().get_mut::<Velocity>(p).unwrap().0 = to_engine(Vec3::Z * -800.0);
    sim.seconds(0.3);
    let me = sim.app.world().get::<SourceMovement>(p).unwrap().clone();
    assert!(me.on_ground && me.water_level == 1, "ground {} water {}", me.on_ground, me.water_level);
    assert!(me.last_landing_speed > 800.0);
    assert_eq!(sim.app.world().get::<Health>(p).unwrap().current, 1.0, "hurt landing in water");
}
