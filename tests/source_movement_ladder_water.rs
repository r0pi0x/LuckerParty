//! Ladder and water test cases from specs/cs_source/movement.md, on a small
//! map in Source units: a floor at z = 0, a ladder slab whose face (normal
//! -X) is at x = 100, a pool 200 units deep, and a shallow pool with a
//! ledge to climb out onto. 64 tick, the shared code's values.

use bevy::prelude::*;
use mashup::{
    core::Velocity,
    games::cs_source::movement::{
        self, SourceMovement, SourceMovementConfig, SourceMovementPlugin, to_engine, to_source,
    },
    harness::Sim,
    map::{MapBrush, MapBrushes, MapWater, MapWaterVolume},
};

fn brush(lo: Vec3, hi: Vec3, ladder: bool) -> MapBrush {
    let (a, b) = (to_engine(lo), to_engine(hi));
    MapBrush {
        ladder,
        ..MapBrush::from_box(a.min(b), a.max(b))
    }
}

const LADDER_X: f32 = 100.0;
/// Deep pool: x -2000..-1000, water up to z 200, floor at -200.
const DEEP: Vec3 = Vec3::new(-1500.0, 0.0, 0.0);
/// Shallow pool: water up to z 50 over the floor at 0, a ledge at x 1526
/// whose top (60) is below eye height + 8.
const SHALLOW: Vec3 = Vec3::new(1500.0, 0.0, 0.0);

fn test_map(mut commands: Commands) {
    let mut brushes = vec![
        // Floor everywhere but the deep pool.
        brush(
            Vec3::new(-8192.0, -8192.0, -64.0),
            Vec3::new(-2000.0, 8192.0, 0.0),
            false,
        ),
        brush(
            Vec3::new(-1000.0, -8192.0, -64.0),
            Vec3::new(8192.0, 8192.0, 0.0),
            false,
        ),
        brush(
            Vec3::new(-2000.0, -8192.0, -264.0),
            Vec3::new(-1000.0, 8192.0, -200.0),
            false,
        ),
        // Ladder slab.
        brush(
            Vec3::new(LADDER_X, -64.0, 0.0),
            Vec3::new(LADDER_X + 8.0, 64.0, 400.0),
            true,
        ),
    ];
    // Ledge out of the shallow pool: face 10 units in front of the box.
    brushes.push(brush(
        Vec3::new(SHALLOW.x + 26.0, -256.0, 0.0),
        Vec3::new(SHALLOW.x + 600.0, 256.0, 60.0),
        false,
    ));
    let water = |lo: Vec3, hi: Vec3| MapWaterVolume {
        brush: brush(lo, hi, false),
        slime: false,
    };
    commands.insert_resource(MapBrushes(brushes));
    commands.insert_resource(MapWater(vec![
        water(Vec3::new(-2000.0, -8192.0, -200.0), Vec3::new(-1000.0, 8192.0, 200.0)),
        water(
            Vec3::new(SHALLOW.x - 400.0, -256.0, 0.0),
            Vec3::new(SHALLOW.x + 26.0, 256.0, 50.0),
        ),
    ]));
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
    fn at(feet: Vec3, cfg: SourceMovementConfig) -> Self {
        let mut sim = Sim::new((TestMap, SourceMovementPlugin));
        sim.app.insert_resource(cfg);
        let p = sim.spawn_character(to_engine(feet + Vec3::Z * 36.0), movement::ID);
        let mut me = Self { sim, p };
        me.look(0.0, 0.0);
        me
    }

    /// In the air, the box 1 unit from the ladder face, already climbing.
    fn on_ladder(cfg: SourceMovementConfig) -> Self {
        let mut me = Self::at(Vec3::new(LADDER_X - 17.0, 0.0, 100.0), cfg);
        me.sim.app.world_mut().get_mut::<SourceMovement>(me.p).unwrap().ladder = Some(Vec3::NEG_X);
        me
    }

    /// Face Source yaw and pitch (degrees; pitch positive looks down).
    fn look(&mut self, yaw: f32, pitch: f32) {
        let mut i = self.sim.intent(self.p);
        i.yaw = yaw.to_radians() - std::f32::consts::FRAC_PI_2;
        i.pitch = -pitch.to_radians();
    }

    fn keys(&mut self, forward: f32, right: f32) {
        self.sim.intent(self.p).move_axis = Vec2::new(right, forward);
    }

    fn feet(&self) -> Vec3 {
        to_source(self.sim.position(self.p)) - Vec3::Z * 36.0
    }

    fn vel(&self) -> Vec3 {
        to_source(self.sim.velocity(self.p))
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

fn close3(actual: Vec3, expected: Vec3, tol: f32, what: &str) {
    assert!(
        (actual - expected).abs().max_element() <= tol,
        "{what}: got {actual}, expected {expected} (±{tol})"
    );
}

fn shared() -> SourceMovementConfig {
    SourceMovementConfig::shared_code()
}

#[test]
fn climbing_speed_follows_pitch() {
    for (pitch, vz) in [
        (0.0, 200.0),
        (-45.0, 282.843),
        (45.0, 0.0),
        (60.0, -73.205),
        (89.0, -196.48),
        (-89.0, 203.46),
    ] {
        let mut pl = Player::on_ladder(shared());
        pl.look(0.0, pitch);
        pl.keys(1.0, 0.0);
        let z0 = pl.feet().z;
        pl.tick();
        assert!(pl.state().ladder.is_some(), "pitch {pitch}: attached");
        close3(
            pl.vel(),
            Vec3::new(0.0, 0.0, vz),
            0.01,
            &format!("pitch {pitch}: velocity"),
        );
        close(pl.feet().z - z0, vz / 64.0, 0.01, &format!("pitch {pitch}: climbed"));
    }
}

#[test]
fn ladder_back_and_sideways() {
    let mut pl = Player::on_ladder(shared());
    pl.keys(-1.0, 0.0);
    pl.tick();
    close3(pl.vel(), Vec3::new(0.0, 0.0, -200.0), 0.01, "hold back");

    let mut pl = Player::on_ladder(shared());
    pl.keys(0.0, 1.0);
    pl.tick();
    close3(pl.vel(), Vec3::new(0.0, -200.0, 0.0), 0.01, "hold right");

    // Shared code: no dampening.
    let mut pl = Player::on_ladder(shared());
    pl.keys(1.0, 1.0);
    pl.tick();
    close3(
        pl.vel(),
        Vec3::new(0.0, -200.0, 200.0),
        0.01,
        "forward + right, undamped",
    );
}

#[test]
fn css_ladder_dampening() {
    let mut cfg = shared();
    cfg.set_cvar("sv_ladder_dampen", "0.2").unwrap();
    let mut pl = Player::on_ladder(cfg.clone());
    pl.keys(1.0, 1.0);
    pl.tick();
    close3(pl.vel(), Vec3::new(0.0, -40.0, 200.0), 0.01, "forward + right, damped");

    // Looking up 45: the push isn't square enough to damp.
    let mut pl = Player::on_ladder(cfg);
    pl.look(0.0, -45.0);
    pl.keys(1.0, 1.0);
    pl.tick();
    close3(pl.vel(), Vec3::new(0.0, -200.0, 282.843), 0.01, "looking up, undamped");
}

#[test]
fn hanging_and_jumping_off() {
    let mut pl = Player::on_ladder(shared());
    pl.keys(1.0, 0.0);
    pl.tick();
    pl.keys(0.0, 0.0);
    let z = pl.feet().z;
    pl.tick();
    close3(pl.vel(), Vec3::ZERO, 1e-4, "no keys: hangs");
    close(pl.feet().z, z, 1e-4, "no gravity");

    let x0 = pl.feet().x;
    pl.sim.intent(pl.p).jump = true;
    pl.tick();
    assert!(pl.state().ladder.is_none(), "jumped off");
    close(pl.vel().x, -270.0, 0.01, "pushed off along the normal");
    close(pl.vel().z, -12.5, 0.01, "then falls (two gravity halves)");
    close(pl.feet().x - x0, -4.21875, 0.01, "moved away");
    pl.sim.intent(pl.p).jump = false;
    pl.keys(1.0, 0.0);
    pl.tick();
    assert!(pl.state().ladder.is_none(), "4.22 units is beyond the 2-unit reach");
}

#[test]
fn attaching_needs_input_and_reach() {
    let mut pl = Player::at(Vec3::new(LADDER_X - 17.0, 0.0, 100.0), shared());
    pl.tick();
    assert!(pl.state().ladder.is_none(), "no keys: no probe");
    assert!(pl.vel().z < 0.0, "falls");

    let mut pl = Player::at(Vec3::new(LADDER_X - 19.0, 0.0, 100.0), shared());
    pl.keys(1.0, 0.0);
    pl.tick();
    assert!(pl.state().ladder.is_none(), "3 units away: out of reach");

    let mut pl = Player::at(Vec3::new(LADDER_X - 17.0, 0.0, 100.0), shared());
    pl.keys(1.0, 0.0);
    pl.tick();
    assert!(pl.state().ladder.is_some(), "1 unit away, pressing into it: attached");
    close3(pl.vel(), Vec3::new(0.0, 0.0, 200.0), 0.01, "climbing");
}

#[test]
fn water_level_by_depth() {
    // Standing box (72 tall, eye 64) with the deep pool's surface (z 200) H
    // above the feet. One tick of sinking moves the feet by under 0.1.
    for (h, level) in [(0.5, 0), (20.0, 1), (40.0, 2), (63.0, 2), (70.0, 3)] {
        let mut pl = Player::at(DEEP + Vec3::Z * (200.0 - h), shared());
        pl.tick();
        assert_eq!(pl.state().water_level, level, "surface {h} above the feet");
    }
}

#[test]
fn sinking_with_no_input() {
    let mut pl = Player::at(DEEP, shared());
    let expected = [
        (1, -3.75),
        (2, -7.265625),
        (23, -46.40),
        (24, -47.25),
        (25, -48.0),
        (40, -48.0),
    ];
    let mut t = 0;
    for (at, vz) in expected {
        while t < at {
            pl.tick();
            t += 1;
        }
        assert_eq!(pl.state().water_level, 3);
        close(pl.vel().z, vz, 0.01, &format!("sink speed after {at} ticks"));
    }
}

#[test]
fn swimming_forward() {
    let mut pl = Player::at(DEEP, shared());
    pl.keys(1.0, 0.0);
    let expected = [(1, 15.625), (2, 30.273), (24, 196.88), (25, 200.0), (40, 200.0)];
    let mut t = 0;
    for (at, speed) in expected {
        while t < at {
            pl.tick();
            t += 1;
        }
        close(
            pl.vel().truncate().length(),
            speed,
            0.01,
            &format!("swim speed after {at} ticks"),
        );
    }

    // Looking up 30: swims 60 degrees up.
    let mut pl = Player::at(DEEP, shared());
    pl.look(0.0, -30.0);
    pl.keys(1.0, 0.0);
    pl.tick();
    close3(pl.vel(), Vec3::new(7.8125, 0.0, 13.5316), 0.01, "up-swim, tick 1");
    pl.tick();
    close(
        pl.vel().length(),
        19.287,
        0.01,
        "tick 2 (surface friction 0.25 while rising)",
    );

    // Looking down 30: no extra dive.
    let mut pl = Player::at(DEEP, shared());
    pl.look(0.0, 30.0);
    pl.keys(1.0, 0.0);
    pl.tick();
    close3(pl.vel(), Vec3::new(13.5316, 0.0, -7.8125), 0.01, "down-swim, tick 1");
}

#[test]
fn holding_jump_in_water_rises() {
    let mut pl = Player::at(DEEP, shared());
    pl.sim.intent(pl.p).jump = true;
    pl.tick();
    close(pl.vel().z, 109.375, 0.01, "tick 1");
    for _ in 0..5 {
        pl.tick();
        close(pl.vel().z, 102.34375, 0.01, "steady");
    }
}

#[test]
fn water_jump_onto_a_ledge() {
    // Waist deep (the surface 50 above the feet), on the pool floor.
    // Settle facing away: the check needs no input, only the wall ahead.
    let mut pl = Player::at(SHALLOW, shared());
    pl.look(180.0, 0.0);
    pl.tick();
    assert_eq!(pl.state().water_level, 2);
    pl.look(0.0, 0.0);
    pl.keys(1.0, 0.0);
    pl.tick();
    assert!(pl.state().water_jump_time > 0.0, "water jump started");
    close(pl.vel().z, 240.0, 0.01, "256 less a tick of swim friction");
    close(pl.vel().truncate().length(), 0.0, 1e-3, "no horizontal speed yet");
    pl.tick();
    close(pl.vel().x, 50.0, 0.01, "pushed toward the ledge");
    // Out of the water and onto the ledge within the 2 seconds.
    for _ in 0..128 {
        pl.tick();
    }
    let feet = pl.feet();
    assert!(
        feet.x > SHALLOW.x + 26.0 - 16.0 && feet.z >= 59.0,
        "on the ledge: {feet}"
    );
}

#[test]
fn falling_into_deep_water_keeps_its_speed() {
    let mut pl = Player::at(DEEP + Vec3::Z * 100.0, shared());
    pl.sim.app.world_mut().get_mut::<Velocity>(pl.p).unwrap().0 = to_engine(Vec3::new(0.0, 0.0, -600.0));
    pl.tick();
    assert_eq!(pl.state().water_level, 3);
    let v1 = pl.vel().z;
    pl.tick();
    close(pl.vel().z / v1, 0.9375, 0.002, "only proportional water friction");
}
