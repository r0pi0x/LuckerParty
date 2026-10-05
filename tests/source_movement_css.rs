//! CS:S-specific movement measured on the real game with the movement
//! probe (specs/cs_source/movement.md, "CS:S values"): jump stamina, the
//! bunny-hop speed cap, auto-bunnyhopping, and console variables. Flat
//! floor, CS:S tick (0.015 s), CS:S config.

use bevy::prelude::*;
use mashup::{
    core::Velocity,
    games::cs_source::{
        TICK_INTERVAL,
        movement::{self, SourceMovement, SourceMovementConfig, SourceMovementPlugin, to_engine, to_source},
    },
    harness::Sim,
    map::{MapBrush, MapBrushes},
};

fn test_map(mut commands: Commands) {
    let (a, b) = (
        to_engine(Vec3::new(-16384.0, -16384.0, -64.0)),
        to_engine(Vec3::new(16384.0, 16384.0, 0.0)),
    );
    commands.insert_resource(MapBrushes(vec![MapBrush::from_box(a.min(b), a.max(b))]));
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
    fn new(cvars: &[(&str, &str)]) -> Self {
        let mut sim = Sim::new((TestMap, SourceMovementPlugin));
        sim.set_tick_interval(TICK_INTERVAL);
        let mut cfg = SourceMovementConfig::default();
        for (n, v) in cvars {
            cfg.set_cvar(n, v).unwrap();
        }
        sim.app.insert_resource(cfg);
        let p = sim.spawn_character(to_engine(Vec3::new(0.0, 0.0, 36.5)), movement::ID);
        sim.ticks(10);
        // Face Source yaw 0 (+X).
        sim.intent(p).yaw = -std::f32::consts::FRAC_PI_2;
        Self { sim, p }
    }
    fn vel(&self) -> Vec3 {
        to_source(self.sim.velocity(self.p))
    }
    fn speed(&self) -> f32 {
        self.vel().truncate().length()
    }
    fn set_vel(&mut self, v: Vec3) {
        self.sim.app.world_mut().get_mut::<Velocity>(self.p).unwrap().0 = to_engine(v);
    }
    fn on_ground(&self) -> bool {
        self.sim.app.world().get::<SourceMovement>(self.p).unwrap().on_ground
    }
    fn tick(&mut self) {
        self.sim.ticks(1);
    }
    /// Run at full speed, jump (forward still held), and return the speed
    /// on each tick from the landing on, with forward held or not.
    fn run_jump_land(&mut self, hold_after: bool, ticks_after: usize) -> Vec<f32> {
        self.sim.intent(self.p).move_axis = Vec2::Y;
        self.sim.ticks(60);
        self.sim.intent(self.p).jump = true;
        self.tick();
        self.sim.intent(self.p).jump = false;
        while !self.on_ground() {
            self.tick();
        }
        if !hold_after {
            self.sim.intent(self.p).move_axis = Vec2::ZERO;
        }
        (0..ticks_after)
            .map(|_| {
                self.tick();
                self.speed()
            })
            .collect()
    }
}

/// CS:S: after a running jump, landing with forward held dips to ~148 and
/// recovers to 250 within ~40 ticks (CS:S trace: 246.9, 225.4, 208.1, ...,
/// 148.4 minimum, 250 again by 30-40 ticks).
#[test]
fn landing_slowdown_and_recovery() {
    let mut pl = Player::new(&[]);
    let speeds = pl.run_jump_land(true, 60);
    let min = speeds.iter().cloned().fold(f32::MAX, f32::min);
    assert!((140.0..160.0).contains(&min), "lowest speed after landing {min}");
    assert!((speeds[59] - 250.0).abs() < 0.5, "recovered to {}", speeds[59]);
}

/// CS:S: releasing forward on landing stops within ~20 ticks (stamina
/// scales speed on top of friction); plain friction would take ~45.
#[test]
fn quick_stop_after_landing() {
    let mut pl = Player::new(&[]);
    let speeds = pl.run_jump_land(false, 40);
    let stop = speeds.iter().position(|s| *s < 0.5).expect("never stopped");
    assert!((14..=24).contains(&stop), "stopped after {stop} ticks ({speeds:?})");

    let mut plain = Player::new(&[]);
    plain
        .sim
        .app
        .world_mut()
        .resource_mut::<SourceMovementConfig>()
        .jump_stamina = 0.0;
    let speeds = plain.run_jump_land(false, 80);
    let stop_plain = speeds.iter().position(|s| *s < 0.5).unwrap();
    assert!(stop_plain > stop + 10, "without stamina: {stop_plain} ticks");
}

/// CS:S: a jump 50 ticks after the previous one is weaker: stamina
/// 1315.79 - 750 = 565.79 ms scales the jump by 1 - 0.00019 x 565.79.
#[test]
fn repeated_jumps_are_lower() {
    let mut pl = Player::new(&[]);
    let mut takeoff = Vec::new();
    for _ in 0..2 {
        pl.sim.intent(pl.p).jump = true;
        pl.tick();
        takeoff.push(pl.vel().z);
        pl.sim.intent(pl.p).jump = false;
        pl.sim.ticks(49);
    }
    let r = 1.0 - 0.00019 * 565.789_4;
    // Standing jump: (vz after the first gravity half + impulse) x r, then
    // the extra and the second gravity halves (6 each at 0.015 s).
    let expected = ((2.0f32 * 800.0 * 57.0).sqrt() - 6.0) * r - 12.0;
    assert!(
        (takeoff[0] - ((2.0f32 * 800.0 * 57.0).sqrt() - 18.0)).abs() < 0.01,
        "{takeoff:?}"
    );
    assert!(
        (takeoff[1] - expected).abs() < 0.05,
        "second jump {} expected {expected}",
        takeoff[1]
    );
}

/// CS:S with sv_enablebunnyhopping 0: jumping caps speed at 286 (3D);
/// with 1, no cap.
#[test]
fn bunnyhop_cap() {
    for (setting, expected) in [("0", 285.94f32), ("1", 400.0 * 0.94)] {
        let mut pl = Player::new(&[("sv_enablebunnyhopping", setting)]);
        pl.set_vel(Vec3::new(400.0, 0.0, 0.0));
        pl.tick(); // one ground tick: friction 400 -> 376
        pl.sim.intent(pl.p).jump = true;
        pl.tick();
        let s = pl.speed();
        assert!(
            (s - expected).abs() < 0.1,
            "sv_enablebunnyhopping {setting}: {s}, expected {expected}"
        );
    }
}

/// sv_autobunnyhopping 1: holding jump jumps again on landing.
#[test]
fn auto_bunnyhopping() {
    for (setting, min_jumps) in [("0", 1), ("1", 3)] {
        let mut pl = Player::new(&[("sv_autobunnyhopping", setting)]);
        pl.sim.intent(pl.p).jump = true;
        let mut jumps = 0;
        let mut was_ground = true;
        for _ in 0..200 {
            pl.tick();
            if was_ground && !pl.on_ground() {
                jumps += 1;
            }
            was_ground = pl.on_ground();
        }
        if setting == "0" {
            assert_eq!(jumps, 1, "holding jump without autobhop");
        } else {
            assert!(jumps >= min_jumps, "autobhop jumped {jumps} times");
        }
    }
}

/// Console variables, as `--cvar` and `--exec` set them.
#[test]
fn console_variables() {
    let mut cfg = SourceMovementConfig::default();
    cfg.set_cvar("sv_airaccelerate", "150").unwrap();
    assert_eq!(cfg.airaccelerate, 150.0);
    let problems = cfg.exec(
        "// surf server\nsv_airaccelerate \"1000\"\nsv_gravity 800 // default\nsv_enablebunnyhopping 1\nsv_cheats 1\n",
    );
    assert_eq!(cfg.airaccelerate, 1000.0);
    assert!(cfg.enable_bunnyhopping);
    assert_eq!(problems, vec!["unknown console variable sv_cheats".to_string()]);
    assert!(cfg.set_cvar("sv_friction", "fast").is_err());
}
