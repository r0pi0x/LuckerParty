//! Test cases from specs/cs_source/ropes.md: the rope simulation's settled
//! shapes, spline points and texture step. Source units, Z up.

use bevy::math::Vec3;
use mashup::games::cs_source::ropes::{INITIAL_STEPS, RopeSim, SPEC_GRAVITY, node_count, spline, v_step};

/// Float32 here vs float64 in the spec's worked numbers.
const TOL: f32 = 0.05;

fn close(actual: f32, expected: f32, what: &str) {
    assert!(
        (actual - expected).abs() <= TOL,
        "{what}: got {actual}, expected {expected}"
    );
}

fn horizontal(slack: i32, rope_type: Option<i32>) -> RopeSim {
    RopeSim::new(
        Vec3::ZERO,
        Vec3::new(256.0, 0.0, 0.0),
        slack,
        node_count(rope_type),
        SPEC_GRAVITY,
    )
}

fn run(sim: &mut RopeSim, steps: usize) {
    for _ in 0..steps {
        sim.step();
    }
}

fn check_z(sim: &RopeSim, expected: &[f32], what: &str) {
    assert_eq!(sim.nodes.len(), expected.len(), "{what}: node count");
    for (i, (n, e)) in sim.nodes.iter().zip(expected).enumerate() {
        close(n.z, *e, &format!("{what} node {i} z"));
    }
}

#[test]
fn node_count_follows_type_key() {
    assert_eq!(node_count(Some(0)), 10);
    assert_eq!(node_count(Some(1)), 4);
    assert_eq!(node_count(Some(2)), 2);
    assert_eq!(node_count(Some(7)), 2);
    assert_eq!(node_count(None), 5);
    assert_eq!(INITIAL_STEPS, 251);
}

#[test]
fn ten_node_rope_settles_to_spec_shape() {
    let mut sim = horizontal(25, Some(0));
    close(sim.link_length(), 20.111, "link length");
    sim.settle();
    check_z(
        &sim,
        &[0.0, -1.819, -2.869, -3.548, -3.859, -3.803, -3.379, -2.585, -1.420, 0.0],
        "slack 25",
    );
    let xs = [
        0.0, 35.102, 62.707, 90.321, 117.940, 145.562, 173.182, 200.799, 228.405, 256.0,
    ];
    for (i, (n, e)) in sim.nodes.iter().zip(xs).enumerate() {
        close(n.x, e, &format!("slack 25 node {i} x"));
    }
}

#[test]
fn spline_points_match_spec() {
    let mut sim = horizontal(25, Some(0));
    sim.settle();
    let points = spline(&sim.nodes, 2);
    assert_eq!(points.len(), 28);
    // Between nodes 0 and 1 (first gap: P0 = P1), and nodes 4 and 5.
    for (index, (x, z)) in [
        (1, (9.378, -0.500)),
        (2, (22.657, -1.202)),
        (13, (127.147, -3.881)),
        (14, (136.355, -3.862)),
    ] {
        close(points[index].0.x, x, &format!("point {index} x"));
        close(points[index].0.z, z, &format!("point {index} z"));
    }
    assert_eq!(spline(&sim.nodes, 12).len(), 10 + 9 * 7, "subdiv clamps to 7");
}

#[test]
fn fewer_nodes_sag_less() {
    let mut five = horizontal(25, None);
    five.settle();
    check_z(&five, &[0.0, -0.692, -0.831, -0.561, 0.0], "5 nodes");
    let mut four = horizontal(25, Some(1));
    four.settle();
    check_z(&four, &[0.0, -0.455, -0.384, 0.0], "4 nodes");
    let mut two = horizontal(200, Some(2));
    two.settle();
    check_z(&two, &[0.0, 0.0], "2 nodes stay straight");
}

#[test]
fn slack_zero_still_sags() {
    let mut sim = horizontal(0, Some(0));
    sim.settle();
    check_z(
        &sim,
        &[0.0, -1.445, -2.211, -2.701, -2.918, -2.861, -2.531, -1.925, -1.042, 0.0],
        "slack 0",
    );
}

#[test]
fn initial_hang_shapes() {
    let mut sim = horizontal(100, Some(0));
    run(&mut sim, INITIAL_STEPS);
    check_z(
        &sim,
        &[
            0.0, -12.091, -21.163, -27.357, -30.482, -30.431, -27.204, -20.900, -11.714, 0.0,
        ],
        "slack 100 after 251 steps",
    );

    let mut sloped = RopeSim::new(Vec3::ZERO, Vec3::new(128.0, 0.0, -64.0), 125, 10, SPEC_GRAVITY);
    close(
        sloped.link_length(),
        168.0 / 9.0,
        "sloped link length (span truncated to 143)",
    );
    run(&mut sloped, INITIAL_STEPS);
    let expected = [
        (8.566, -17.964),
        (18.071, -34.599),
        (29.146, -50.134),
        (42.225, -63.930),
        (57.715, -74.860),
        (75.554, -81.189),
        (94.517, -81.222),
        (112.431, -74.904),
    ];
    for (i, (x, z)) in expected.into_iter().enumerate() {
        close(sloped.nodes[i + 1].x, x, &format!("sloped node {} x", i + 1));
        close(sloped.nodes[i + 1].z, z, &format!("sloped node {} z", i + 1));
    }
}

#[test]
fn long_slack_fixed_points() {
    let mut ten = horizontal(200, Some(0));
    run(&mut ten, 20_000);
    close(ten.nodes[4].z, -111.255, "slack 200 node 4");
    close(ten.nodes[5].z, -111.256, "slack 200 node 5");

    let mut five = horizontal(200, None);
    run(&mut five, 20_000);
    for (i, (x, z)) in [(48.714, -74.798), (128.002, -115.394), (207.334, -74.723)]
        .into_iter()
        .enumerate()
    {
        close(five.nodes[i + 1].x, x, "5-node x");
        close(five.nodes[i + 1].z, z, "5-node z");
    }

    let mut vertical = RopeSim::new(Vec3::ZERO, Vec3::new(0.0, 0.0, -256.0), 25, 10, SPEC_GRAVITY);
    run(&mut vertical, 20_000);
    check_z(
        &vertical,
        &[
            0.0, -35.885, -63.738, -91.492, -119.143, -146.693, -174.156, -201.483, -228.741, -256.0,
        ],
        "vertical",
    );
}

#[test]
fn texture_step_undercounts_points() {
    let dv = v_step(256.0, 25, 10, 2, 1.0, 128.0);
    assert!((dv - 0.29770).abs() < 1e-4, "dv {dv}");
    assert!((dv * 27.0 - 8.038).abs() < 1e-3);
}
