//! Movement techniques players rely on in Source, on test geometry built in
//! Source units: bunny hopping, surfing (including across a seam between
//! two ramp brushes), and a KZ-style longjump. These check that the
//! techniques work at all; matching CS:S tick for tick needs input replay
//! against the real game.

use bevy::prelude::*;
use mashup::{
    core::Velocity,
    games::cs_source::movement::{
        self, SourceMovement, SourceMovementConfig, SourceMovementPlugin, to_engine, to_source,
    },
    harness::Sim,
    map::{MapBrush, MapBrushes},
};

const M: f32 = 0.0254;

/// A convex brush from Source-space planes (n . p <= d) and bounds.
fn brush(planes: &[(Vec3, f32)], lo: Vec3, hi: Vec3) -> MapBrush {
    let (a, b) = (to_engine(lo), to_engine(hi));
    MapBrush {
        planes: planes
            .iter()
            .map(|(n, d)| (Vec3::new(n.x, n.z, -n.y).normalize(), d * M))
            .collect(),
        min: a.min(b),
        max: a.max(b),
        ladder: false,
        surface: None,
    }
}

fn block(lo: Vec3, hi: Vec3) -> MapBrush {
    let (a, b) = (to_engine(lo), to_engine(hi));
    MapBrush::from_box(a.min(b), a.max(b))
}

/// Surf ramp: a wedge along X whose slope faces -Y, normal z = cos(50°) =
/// 0.643 (too steep to stand on), from x0 to x1, base at z = 0, 512 deep.
const SURF_ANGLE: f32 = 50.0;
const SURF_DEPTH: f32 = 512.0;
fn surf_ramp(x0: f32, x1: f32) -> MapBrush {
    let t = SURF_ANGLE.to_radians();
    let n = Vec3::new(0.0, -t.sin(), t.cos());
    let top = SURF_DEPTH * t.tan();
    brush(
        &[
            (n, 0.0),
            (Vec3::Y, SURF_DEPTH),
            (Vec3::NEG_Z, 0.0),
            (Vec3::X, x1),
            (Vec3::NEG_X, -x0),
        ],
        Vec3::new(x0, 0.0, 0.0),
        Vec3::new(x1, SURF_DEPTH, top),
    )
}

const FLOOR_Y: f32 = -8000.0;
const SURF_Z: f32 = 4000.0;

fn test_map(mut commands: Commands) {
    let mut brushes = vec![block(
        Vec3::new(-8192.0, -12000.0, -64.0),
        Vec3::new(8192.0, -4000.0, 0.0),
    )];
    // Surf ramp up high, made of two brushes meeting at x = 0.
    for (x0, x1) in [(-4096.0, 0.0), (0.0, 4096.0)] {
        let mut b = surf_ramp(x0, x1);
        let lift = to_engine(Vec3::Z * SURF_Z);
        b.min += lift;
        b.max += lift;
        for (n, d) in &mut b.planes {
            *d += n.dot(lift);
        }
        brushes.push(b);
    }
    // Walls rotated off the grid, standing on the floor.
    brushes.push(angled_wall(Vec3::new(0.0, -5000.0, 0.0), 30.0));
    brushes.push(angled_wall(Vec3::new(2000.0, -5000.0, 0.0), 30.0));
    brushes.push(angled_wall(Vec3::new(2000.0, -5000.0, 0.0), -40.0));
    commands.insert_resource(MapBrushes(brushes));
}

/// A wall 32 thick, 512 long and 256 high through `at`, its face normal
/// pointing toward yaw `deg` + 180 (so a player walking toward yaw `deg`
/// from the far side meets it).
fn angled_wall(at: Vec3, deg: f32) -> MapBrush {
    let r = deg.to_radians();
    let n = Vec3::new(-r.cos(), -r.sin(), 0.0); // face toward the player
    let t = Vec3::new(-r.sin(), r.cos(), 0.0);
    let planes = [
        (n, n.dot(at)),
        (-n, -n.dot(at) + 32.0),
        (t, t.dot(at) + 256.0),
        (-t, -t.dot(at) + 256.0),
        (Vec3::Z, 256.0),
        (Vec3::NEG_Z, 0.0),
    ];
    let corners: Vec<Vec3> = [-1.0f32, 1.0]
        .iter()
        .flat_map(|a| [0.0f32, 32.0].map(move |b| (a, b)))
        .flat_map(|(a, b)| [0.0f32, 256.0].map(move |z| at + t * (a * 256.0) - n * b + Vec3::Z * z))
        .collect();
    let lo = corners.iter().fold(Vec3::splat(f32::MAX), |m, c| m.min(*c));
    let hi = corners.iter().fold(Vec3::splat(f32::MIN), |m, c| m.max(*c));
    brush(&planes, lo, hi)
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
    fn at(feet: Vec3) -> Self {
        let mut sim = Sim::new((TestMap, SourceMovementPlugin));
        sim.app.insert_resource(SourceMovementConfig::default());
        let p = sim.spawn_character(to_engine(feet + Vec3::Z * 36.0), movement::ID);
        Self { sim, p }
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
    fn state(&self) -> SourceMovement {
        self.sim.app.world().get::<SourceMovement>(self.p).unwrap().clone()
    }
    /// Face Source yaw `deg`.
    fn look_yaw(&mut self, deg: f32) {
        self.sim.intent(self.p).yaw = deg.to_radians() - std::f32::consts::FRAC_PI_2;
    }
    /// One tick of a perfect strafer: view along the velocity, holding the
    /// strafe key on side `side` (+1 right, -1 left), so the wish direction
    /// is perpendicular to the velocity.
    fn strafe_tick(&mut self, side: f32) {
        let v = self.vel();
        self.look_yaw(v.y.atan2(v.x).to_degrees());
        let intent = &mut *self.sim.intent(self.p);
        intent.move_axis = Vec2::new(side, 0.0);
        self.sim.ticks(1);
    }
}

#[test]
fn bunny_hopping_gains_speed() {
    let mut pl = Player::at(Vec3::new(-6000.0, -9000.0, 0.5));
    // As bhop and surf servers run it: no jump speed cap.
    pl.sim
        .app
        .world_mut()
        .resource_mut::<SourceMovementConfig>()
        .set_cvar("sv_enablebunnyhopping", "1")
        .unwrap();
    pl.sim.ticks(4);
    // Run up to knife speed.
    pl.look_yaw(0.0);
    pl.sim.intent(pl.p).move_axis = Vec2::Y;
    pl.sim.seconds(1.0);
    let start = pl.vel().truncate().length();
    assert!((start - 250.0).abs() < 0.5, "run-up speed {start}");

    // Hop: press jump on every tick spent on the ground (released in the
    // air, as a scroll-wheel jump is), strafe in the air, switching sides
    // each hop.
    let mut hops = 0;
    let mut side = 1.0;
    let mut ground_ticks_after_landing = Vec::new();
    let mut ticks_on_ground = 0;
    for _ in 0..(8 * 64) {
        let grounded = pl.state().on_ground;
        pl.sim.intent(pl.p).jump = grounded;
        if grounded {
            ticks_on_ground += 1;
            pl.sim.intent(pl.p).move_axis = Vec2::ZERO;
            pl.sim.ticks(1);
            if !pl.state().on_ground {
                hops += 1;
                side = -side;
                ground_ticks_after_landing.push(ticks_on_ground);
                ticks_on_ground = 0;
            }
        } else {
            pl.strafe_tick(side);
        }
    }
    let end = pl.vel().truncate().length();
    assert!(hops >= 8, "only {hops} hops");
    // Each hop leaves on the first ground tick, so friction never applies.
    assert!(
        ground_ticks_after_landing.iter().skip(1).all(|&t| t == 1),
        "ground ticks per hop: {ground_ticks_after_landing:?}"
    );
    assert!(end > 450.0, "speed after {hops} hops: {end} (from {start})");
}

#[test]
fn surfing_slides_along_a_ramp_and_across_a_seam() {
    // Drop onto the ramp face partway up, moving along it (+X) at 800.
    let t = SURF_ANGLE.to_radians();
    let y = 300.0;
    let face_z = SURF_Z + y * t.tan();
    let mut pl = Player::at(Vec3::new(-1000.0, y - 40.0, face_z + 40.0));
    pl.set_vel(Vec3::new(800.0, 0.0, 0.0));
    let mut min_speed = f32::MAX;
    let mut crossed_seam = false;
    let mut x_prev = pl.feet().x;
    for _ in 0..(2 * 64) {
        // Hold the strafe key that pushes into the ramp (+Y is into it,
        // which is "left" when looking along +X), as surfers do.
        pl.look_yaw(0.0);
        pl.sim.intent(pl.p).move_axis = Vec2::new(-1.0, 0.0);
        pl.sim.ticks(1);
        let f = pl.feet();
        assert!(!pl.state().on_ground, "a 50 degree ramp must not count as ground");
        if x_prev < 0.0 && f.x >= 0.0 {
            crossed_seam = true;
        }
        x_prev = f.x;
        min_speed = min_speed.min(pl.vel().x);
    }
    let f = pl.feet();
    assert!(crossed_seam, "never reached the seam at x = 0 (x = {})", f.x);
    assert!(
        min_speed > 780.0,
        "lost speed along the ramp (min x speed {min_speed}); seams must not catch"
    );
    // Still on the ramp (near its face), not fallen off or stuck inside.
    let face = SURF_Z + f.y.max(0.0) * t.tan();
    assert!(f.y > 0.0 && f.y < SURF_DEPTH, "left the ramp: {f}");
    assert!(
        (f.z - face).abs() < 60.0,
        "not riding the face: z {} vs face {face}",
        f.z
    );
}

/// Distance of a jump from a 250 run-up (KZ measure: horizontal distance
/// plus the 32-unit box), strafing in the air when `strafe` is set:
/// alternating sides every tick, view along the velocity.
fn longjump(strafe: bool) -> f32 {
    let mut pl = Player::at(Vec3::new(0.0, -6000.0, 0.5));
    pl.sim.ticks(4);
    pl.look_yaw(0.0);
    pl.sim.intent(pl.p).move_axis = Vec2::Y;
    pl.sim.seconds(1.0);
    let take_off = pl.feet();
    pl.sim.intent(pl.p).jump = true;
    pl.sim.ticks(1);
    pl.sim.intent(pl.p).jump = false;
    if !strafe {
        pl.sim.intent(pl.p).move_axis = Vec2::ZERO;
    }
    let mut side = 1.0;
    for _ in 0..200 {
        if pl.state().on_ground {
            break;
        }
        if strafe {
            pl.strafe_tick(side);
            side = -side;
        } else {
            pl.sim.ticks(1);
        }
    }
    (pl.feet() - take_off).truncate().length() + 32.0
}

#[test]
fn longjump_strafes_add_distance() {
    let plain = longjump(false);
    let strafed = longjump(true);
    // No strafing: 250 for the jump's air time.
    assert!((200.0..240.0).contains(&plain), "plain jump {plain}");
    // Strafing helps, though less than a naive estimate: near the top of
    // the jump (rising at up to 140) Source sets surface friction to 0.25
    // (spec, "Ground detection"), which scales air acceleration.
    assert!(strafed > plain + 5.0, "strafed {strafed} vs plain {plain}");
}

/// Walking into a wall that isn't on the grid slides along it, on the
/// ground, without sticking (a float-noise case: the clipped velocity must
/// count as parallel to the wall).
#[test]
fn slides_along_an_angled_wall() {
    // The wall faces yaw 210; walk toward yaw 30 + 45, hitting it at 45.
    let mut pl = Player::at(Vec3::new(-150.0, -5100.0, 0.5));
    pl.sim.ticks(4);
    pl.look_yaw(75.0);
    pl.sim.intent(pl.p).move_axis = Vec2::Y;
    let mut slow_ticks = 0;
    let mut touched = false;
    for _ in 0..(3 * 64) {
        pl.sim.ticks(1);
        let speed = pl.vel().truncate().length();
        let from_wall = (pl.feet() - Vec3::new(0.0, -5000.0, 0.0)).dot(Vec3::new(
            -30f32.to_radians().cos(),
            -30f32.to_radians().sin(),
            0.0,
        ));
        // A 32-unit box touching a wall at 30 degrees: centre within ~22.
        if from_wall < 25.0 {
            touched = true;
            if speed < 100.0 {
                slow_ticks += 1;
            }
        }
        assert!(pl.state().on_ground, "left the ground at {}", pl.feet());
    }
    assert!(touched, "never reached the wall: {}", pl.feet());
    // Sliding at 45 degrees keeps ~cos(45) of the speed along the wall.
    assert!(
        slow_ticks < 3,
        "stuck on the wall for {slow_ticks} ticks at {}",
        pl.feet()
    );
}

/// Into the corner between two angled walls: stops (the crease), but never
/// gets stuck when walking back out.
#[test]
fn angled_corner_does_not_trap() {
    let mut pl = Player::at(Vec3::new(1850.0, -5000.0, 0.5));
    pl.sim.ticks(4);
    pl.look_yaw(0.0);
    pl.sim.intent(pl.p).move_axis = Vec2::Y;
    pl.sim.seconds(1.5);
    let wedged = pl.feet();
    pl.look_yaw(180.0);
    pl.sim.seconds(0.5);
    assert!(
        (pl.feet() - wedged).truncate().length() > 50.0,
        "stuck in the corner at {wedged}"
    );
}

/// Bunny hopping the way a player does it: hold one strafe key and turn
/// the mouse that way at a steady rate (no perfect per-tick aiming),
/// switching sides each hop, jumping on landing. Speed must not drop.
fn human_bhop(turn_deg_per_sec: f32) -> (f32, f32) {
    let mut pl = Player::at(Vec3::new(-6000.0, -9000.0, 0.5));
    pl.sim.ticks(4);
    pl.look_yaw(0.0);
    pl.sim.intent(pl.p).move_axis = Vec2::Y;
    pl.sim.seconds(1.0);
    let start = pl.vel().truncate().length();
    let mut yaw = 0.0f32;
    let mut side = -1.0f32; // A first: turn left
    let dt = 1.0 / 64.0;
    let mut hops = 0;
    let mut was_grounded = true;
    for _ in 0..(6 * 64) {
        let grounded = pl.state().on_ground;
        if grounded && !was_grounded {
            side = -side;
        }
        was_grounded = grounded;
        if grounded {
            hops += 1;
        }
        // Turning left raises yaw; strafe left is move_axis.x = -1.
        yaw += -side * turn_deg_per_sec * dt;
        pl.look_yaw(yaw);
        let intent = &mut *pl.sim.intent(pl.p);
        intent.jump = grounded;
        intent.move_axis = if grounded { Vec2::ZERO } else { Vec2::new(side, 0.0) };
        pl.sim.ticks(1);
    }
    let _ = hops;
    (start, pl.vel().truncate().length())
}

#[test]
fn human_style_strafing_keeps_or_gains_speed() {
    for rate in [90.0f32, 150.0, 220.0] {
        let (start, end) = human_bhop(rate);
        assert!(end > start + 20.0, "turning {rate} deg/s: {start} -> {end}");
    }
}
