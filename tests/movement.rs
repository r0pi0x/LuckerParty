//! Scenario tests for Movement implementations on the greybox map. These run
//! headless: each drives `Intent` and checks the published state. Spec-based
//! movement gets the same kind of tests, taken from the spec's test cases.

use bevy::prelude::*;
use mashup::{
    character::CAPSULE_HEIGHT,
    greybox::{self, GreyboxMapPlugin},
    harness::Sim,
    movement::{noclip, placeholder},
};

const STANDING_Y: f32 = CAPSULE_HEIGHT / 2.0;

fn sim_with_player(at: Vec3, movement: &'static str) -> (Sim, Entity) {
    let mut sim = Sim::new(GreyboxMapPlugin);
    let player = sim.spawn_character(at, movement);
    sim.seconds(0.5); // fall from the spawn height and settle
    (sim, player)
}

#[test]
fn settles_on_the_floor() {
    let (sim, p) = sim_with_player(greybox::SPAWNS[0], placeholder::ID);
    assert!(sim.state(p).on_ground, "should be grounded after settling");
    let y = sim.position(p).y;
    assert!(
        (y - STANDING_Y).abs() < 0.05,
        "standing height {y}, expected ~{STANDING_Y}"
    );
}

#[test]
fn walking_forward_reaches_walk_speed() {
    let (mut sim, p) = sim_with_player(greybox::SPAWNS[0], placeholder::ID);
    sim.intent(p).move_axis = Vec2::Y;
    sim.seconds(1.0);
    let v = sim.velocity(p);
    let speed = v.xz().length();
    assert!((speed - 5.0).abs() < 0.1, "speed {speed}, expected ~5.0");
    assert!(v.z < 0.0, "yaw 0 should move toward -Z, velocity {v}");
    assert!(sim.state(p).on_ground);
}

#[test]
fn sprinting_is_faster_than_walking() {
    let (mut sim, p) = sim_with_player(greybox::SPAWNS[0], placeholder::ID);
    sim.intent(p).move_axis = Vec2::Y;
    sim.intent(p).sprint = true;
    sim.seconds(1.0);
    assert!(sim.state(p).sprinting);
    let speed = sim.velocity(p).xz().length();
    assert!(speed > 7.0, "sprint speed {speed}");
}

#[test]
fn jump_peaks_about_a_meter_and_lands() {
    let (mut sim, p) = sim_with_player(greybox::SPAWNS[0], placeholder::ID);
    sim.intent(p).jump = true;
    let mut peak = 0.0f32;
    for _ in 0..(sim.tick_hz() as u64) {
        sim.ticks(1);
        peak = peak.max(sim.position(p).y - STANDING_Y);
    }
    assert!((0.9..1.2).contains(&peak), "jump peak {peak} m");
    assert!(sim.state(p).on_ground, "should have landed within a second");

    // Holding jump must not jump again: jumping needs a fresh press.
    sim.seconds(0.5);
    assert!(sim.position(p).y - STANDING_Y < 0.05, "re-jumped while jump was held");
}

#[test]
fn walls_stop_the_player() {
    let (mut sim, p) = sim_with_player(Vec3::new(0.0, 1.0, -30.0), placeholder::ID);
    sim.intent(p).move_axis = Vec2::Y;
    sim.seconds(4.0);
    let z = sim.position(p).z;
    assert!(z > greybox::NORTH_WALL_Z, "went through the north wall: z = {z}");
    assert!(z < greybox::NORTH_WALL_Z + 0.5, "stopped short of the wall: z = {z}");
}

#[test]
fn walks_up_a_20_degree_ramp() {
    let (mut sim, p) = sim_with_player(Vec3::new(greybox::RAMP_X, 1.0, 6.0), placeholder::ID);
    sim.intent(p).move_axis = Vec2::Y;
    sim.seconds(1.0);
    // Mid-ramp: still grounded at full horizontal walking speed.
    assert!(sim.state(p).on_ground, "lost the ground on the ramp");
    let speed = sim.velocity(p).xz().length();
    assert!(
        (speed - 5.0).abs() < 0.1,
        "horizontal speed on the ramp {speed}, expected ~5.0"
    );
    sim.seconds(2.0);
    let y = sim.position(p).y;
    let expected = greybox::platform_height() + STANDING_Y;
    assert!(
        (y - expected).abs() < 0.15,
        "height {y}, expected ~{expected} on the platform"
    );
}

#[test]
fn cannot_walk_up_a_55_degree_ramp() {
    let (mut sim, p) = sim_with_player(Vec3::new(greybox::STEEP_RAMP_X, 1.0, 6.0), placeholder::ID);
    sim.intent(p).move_axis = Vec2::Y;
    sim.seconds(4.0);
    let y = sim.position(p).y;
    assert!(y < STANDING_Y + 1.0, "climbed the steep ramp to y = {y}");
}

#[test]
fn swapping_to_noclip_passes_through_walls() {
    let (mut sim, p) = sim_with_player(Vec3::new(0.0, 1.0, -30.0), placeholder::ID);
    sim.set_movement(p, noclip::ID);
    sim.intent(p).move_axis = Vec2::Y;
    sim.seconds(2.0);
    let z = sim.position(p).z;
    assert!(z < greybox::NORTH_WALL_Z - 1.0, "noclip should pass the wall: z = {z}");
}

#[test]
fn swapping_back_keeps_working() {
    let (mut sim, p) = sim_with_player(greybox::SPAWNS[0], placeholder::ID);
    sim.set_movement(p, noclip::ID);
    sim.set_movement(p, placeholder::ID);
    sim.seconds(1.0);
    assert!(sim.state(p).on_ground, "placeholder movement should resume and settle");
}
