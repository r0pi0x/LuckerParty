//! The test cases of specs/cs_source/view_models.md that are pure maths:
//! bob (B), sway (S), field of view (F) and handedness (H). The scenario
//! cases (E1, events) are in tests/it/view_models.rs, the model data (L1,
//! attachments, events) in tests/it/heavy/map_de_dust2.rs.

use bevy::prelude::*;
use mashup::{
    games::cs_source::{
        view_anim,
        view_motion::{
            Bob, SwayHistory, ViewMotion, angles, bob_offset, bob_wave, placement, rotation, sway_offset, to_camera,
        },
    },
    map::{ViewModelOffset, ViewModelSettings, view_model},
};

fn close(a: f32, b: f32) -> bool {
    (a - b).abs() < 1e-4
}

fn close3(a: Vec3, b: Vec3) -> bool {
    (a - b).length() < 1e-4
}

const P: f32 = 0.8;
const U: f32 = 0.5;

fn vl(t_b: f32, speed: f32, up: f32) -> (f32, f32) {
    (bob_wave(t_b, speed, P, up), bob_wave(t_b, speed, 2.0 * P, up))
}

#[test]
fn b1_bob_time_advances_with_speed() {
    let mut bob = Bob {
        last_speed: 250.0,
        ..Default::default()
    };
    bob.update(0.3125, 0.3125, 250.0, &ViewMotion::default());
    assert!(close(bob.time, 0.244141), "{}", bob.time);
}

#[test]
fn b2_b4_b5_b6_b7_b8_bob_values() {
    let (v, l) = vl(0.078125, 250.0, U);
    assert!(close(v, 0.878832) && close(l, 0.639255), "B2 {v} {l}");
    let (v, l) = vl(0.2, 250.0, U);
    assert!(close(v, 1.25) && close(l, 0.993718), "B4 {v} {l}");
    let (v, l) = vl(0.6, 250.0, U);
    assert!(close(v, -0.5) && close(l, 0.993718), "B5 {v} {l}");
    let (v, l) = vl(0.0, 250.0, U);
    assert!(close(v, 0.375) && close(l, 0.375), "B6 {v} {l}");
    let (v, l) = vl(0.2, 221.0, U);
    assert!(close(v, 1.105) && close(l, 0.878447), "B7 {v} {l}");
    let (v, l) = vl(0.37, 0.0, U);
    assert!(v == 0.0 && l == 0.0, "B8 {v} {l}");
}

#[test]
fn b3_applying_the_bob() {
    let (v, l) = vl(0.078125, 250.0, U);
    let b = bob_offset(v, l);
    assert!(
        close(b.forward, 0.351533) && close(b.right, 0.127851) && close(b.world_up, 0.087883),
        "{b:?}"
    );
    assert!(close3(b.angles, Vec3::new(-0.351533, -0.191777, 0.439416)), "{b:?}");
    // At angles (0, 0, 0) the placement is exactly that (no sway).
    let motion = ViewMotion {
        sway_interp: 0.0,
        ..Default::default()
    };
    let bob = Bob {
        vertical: v,
        lateral: l,
        ..Default::default()
    };
    let (offset, rot) = placement(Vec3::ZERO, &bob, &mut SwayHistory::default(), 0.0, &motion);
    // Source axes: forward, left, up.
    assert!(close3(offset, Vec3::new(0.351533, -0.127851, 0.087883)), "{offset}");
    assert!(close3(angles(rot), b.angles), "{}", angles(rot));
}

#[test]
fn b9_bob_up_shapes_the_cycle() {
    // c = 0.375, theta = 7 pi / 6.
    let v = bob_wave(0.3, 250.0, P, 0.25);
    assert!(close(v, -0.0625), "{v}");
}

#[test]
fn b10_speed_is_slew_limited() {
    let mut bob = Bob {
        last_time: 1.0,
        ..Default::default()
    };
    bob.update(1.01, 0.01, 250.0, &ViewMotion::default());
    assert!(close(bob.last_speed, 3.2), "{}", bob.last_speed);
}

#[test]
fn b11_bob_repeats_every_256_units() {
    let motion = ViewMotion::default();
    let mut bob = Bob {
        last_speed: 250.0,
        ..Default::default()
    };
    let mut t = 0.0;
    let step = |bob: &mut Bob, t: &mut f64, seconds: f64| {
        for _ in 0..(seconds * 1000.0).round() as usize {
            *t += 0.001;
            bob.update(*t, 0.001, 250.0, &motion);
        }
    };
    step(&mut bob, &mut t, 0.1);
    let (v0, l0) = (bob.vertical, bob.lateral);
    // 256 units at 250 units/s.
    step(&mut bob, &mut t, 256.0 / 250.0);
    assert!((bob.vertical - v0).abs() < 1e-2, "{} {v0}", bob.vertical);
    assert!((bob.lateral - l0).abs() > 0.1, "lateral needs 512 units");
    step(&mut bob, &mut t, 256.0 / 250.0);
    assert!((bob.lateral - l0).abs() < 1e-2, "{} {l0}", bob.lateral);
}

#[test]
fn b12_paused_frames_keep_the_bob() {
    let mut bob = Bob {
        last_speed: 250.0,
        vertical: 0.7,
        lateral: 0.3,
        ..Default::default()
    };
    let before = bob;
    bob.update(5.0, 0.0, 250.0, &ViewMotion::default());
    assert_eq!(bob, before);
}

#[test]
fn s1_to_s6_sway_offsets() {
    let s1 = sway_offset(Vec3::new(0.0, 10.0, 0.0), Vec3::ZERO, 1.0);
    assert!(close3(s1, Vec3::new(0.015192, 0.173648, 0.0)), "S1 {s1}");
    let s2 = sway_offset(Vec3::new(10.0, 0.0, 0.0), Vec3::ZERO, 1.0);
    assert!(close3(s2, Vec3::new(0.015192, 0.0, 0.173648)), "S2 {s2}");
    let s3 = sway_offset(Vec3::new(0.0, 36.0, 0.0), Vec3::ZERO, 1.0);
    assert!(close3(s3, Vec3::new(0.190983, 0.587785, 0.0)), "S3 {s3}");
    let s4 = sway_offset(Vec3::new(12.0, 34.0, 5.0), Vec3::new(12.0, 34.0, 5.0), 1.0);
    assert!(close3(s4, Vec3::ZERO), "S4 {s4}");
    let s5 = sway_offset(Vec3::new(0.0, 10.0, 0.0), Vec3::ZERO, 2.0);
    assert!(close3(s5, s1 * 2.0), "S5 {s5}");
    // S6: interp 0 turns the sway off.
    let motion = ViewMotion {
        sway_interp: 0.0,
        ..Default::default()
    };
    let mut history = SwayHistory::default();
    placement(Vec3::ZERO, &Bob::default(), &mut history, 0.0, &motion);
    let (offset, _) = placement(Vec3::new(0.0, 10.0, 0.0), &Bob::default(), &mut history, 0.05, &motion);
    assert!(close3(offset, Vec3::ZERO), "S6 {offset}");
}

#[test]
fn s7_sway_reads_the_history_0_1_s_ago() {
    let mut history = SwayHistory::default();
    history.push(0.90, Vec3::ZERO, 0.1);
    history.push(1.00, Vec3::new(0.0, 20.0, 0.0), 0.1);
    let lagged = history.push(1.05, Vec3::new(0.0, 30.0, 0.0), 0.1);
    assert!(close(lagged.y, 10.0), "{lagged}");
    let s = sway_offset(Vec3::new(0.0, 30.0, 0.0), lagged, 1.0);
    assert!(close3(s, Vec3::new(0.060307, 0.342020, 0.0)), "{s}");
    // Through the whole placement (no bob): the same, in Source axes
    // (forward, left, up).
    let motion = ViewMotion::default();
    let mut history = SwayHistory::default();
    for (t, yaw) in [(0.90, 0.0), (1.00, 20.0)] {
        placement(Vec3::new(0.0, yaw, 0.0), &Bob::default(), &mut history, t, &motion);
    }
    let (offset, _) = placement(Vec3::new(0.0, 30.0, 0.0), &Bob::default(), &mut history, 1.05, &motion);
    assert!(close3(offset, Vec3::new(0.060307, -0.342020, 0.0)), "{offset}");
    // The first 0.1 s: the oldest sample.
    let mut fresh = SwayHistory::default();
    assert!(close3(
        fresh.push(0.0, Vec3::new(0.0, 5.0, 0.0), 0.1),
        Vec3::new(0.0, 5.0, 0.0)
    ));
}

#[test]
fn source_angles_round_trip() {
    let a = Vec3::new(-20.0, 135.0, 10.0);
    assert!(close3(angles(rotation(a)), a), "{}", angles(rotation(a)));
    // Pitch positive looks down.
    assert!((rotation(Vec3::new(30.0, 0.0, 0.0)) * Vec3::X).z < 0.0);
}

#[test]
fn offsets_reach_the_camera_in_its_axes() {
    // Forward 1, left 2, up 3 units -> camera -Z, -X, +Y.
    let o = to_camera(Vec3::new(1.0, 2.0, 3.0), Quat::IDENTITY);
    assert!(
        close3(o.translation, Vec3::new(-2.0, 3.0, -1.0) * 0.0254),
        "{:?}",
        o.translation
    );
    // Yawing left in Source turns left in the camera.
    let q = to_camera(Vec3::ZERO, rotation(Vec3::new(0.0, 10.0, 0.0))).rotation;
    assert!((q * Vec3::NEG_Z).x < 0.0, "{q}");
    // Pitching up (negative Source pitch) raises the camera's forward.
    let q = to_camera(Vec3::ZERO, rotation(Vec3::new(-10.0, 0.0, 0.0))).rotation;
    assert!((q * Vec3::NEG_Z).y > 0.0, "{q}");
}

#[test]
fn f1_to_f4_fields_of_view() {
    use view_model::{aspect_fov, vertical_fov, view_model_fov};
    // The spec's cases use CS:S's viewmodel_fov 54 (ours defaults to 80).
    let s = view_model::ViewModelSettings { fov: 54.0, ..view_anim::settings() };
    let fov = view_model_fov(&s, 90.0);
    assert!(close(fov, 54.0) && close(aspect_fov(fov, 4.0 / 3.0), 54.0), "F1");
    assert!((vertical_fov(fov) - 41.828).abs() < 1e-3, "F1 {}", vertical_fov(fov));
    assert!((aspect_fov(fov, 16.0 / 9.0) - 68.382).abs() < 1e-3, "F2");
    assert!((aspect_fov(90.0, 16.0 / 9.0) - 106.260).abs() < 1e-3, "F2 world");
    assert!((aspect_fov(fov, 16.0 / 10.0) - 62.886).abs() < 1e-3, "F3");
    // F4: zoomed to 40, the view model drops by as much.
    assert!(close(view_model_fov(&s, 40.0), 4.0), "F4");
}

#[test]
fn f5_attachments_reproject_to_the_world_fov() {
    let p = view_model::reproject(Vec3::new(2.0, -3.0, 20.0), 90.0, 54.0);
    assert!(close3(p, Vec3::new(3.92522, -5.88783, 20.0)), "{p}");
}

#[test]
fn h1_to_h3_handedness() {
    use mashup::games::cs_source::weapons::{AK47, KNIFE, VIEW_MODELS};
    let built_right = |key: &str| VIEW_MODELS.iter().find(|v| v.0 == key).unwrap().2;
    let flip = |key: &str, hand: u8| view_model::mirrored(built_right(key), true, hand != 0);
    // H1: cl_righthand 1 (the default) mirrors the left-handed AK but not
    // the knife (built right-handed): both end up in the right hand.
    assert_eq!(view_anim::settings().right_hand, 1);
    assert!(flip(AK47, 1) && !flip(KNIFE, 1));
    // H2: cl_righthand 0 shows the AK as in the file and mirrors the knife.
    assert!(!flip(AK47, 0) && flip(KNIFE, 0));
    assert!(!view_model::mirrored(false, false, true), "AllowFlipping 0");
    // H3: a point right of the eye goes as far left, offsets included;
    // the winding is reversed.
    let offset = ViewModelOffset {
        translation: Vec3::new(0.1, 0.2, -0.3),
        rotation: Quat::from_rotation_y(0.3),
    };
    let mirrored = view_model::placement(&offset, true);
    let plain = view_model::placement(&offset, false);
    for q in [Vec3::ZERO, Vec3::new(0.127, 0.0, 0.0), Vec3::new(0.05, -0.1, -0.2)] {
        let a = mirrored.transform_point(q);
        let expect = plain.transform_point(q) * Vec3::new(-1.0, 1.0, 1.0);
        assert!(close3(a, expect), "{a} {expect}");
    }
    // 5 units right of the eye, no offset: 5 units left.
    let p = view_model::placement(&ViewModelOffset::default(), true).transform_point(Vec3::X * 5.0);
    assert!(close3(p, Vec3::NEG_X * 5.0), "{p}");
    assert!(mirrored.scale.x < 0.0, "winding reversed");
    assert_eq!(ViewModelSettings::default().right_hand, 1);
}

mod shells {
    //! E2–E6, in Source units (the physics is unit-free; CS:S's numbers
    //! scaled back from meters).
    use bevy::prelude::*;
    use mashup::{
        games::cs_source::view_anim::shell_physics,
        map::shells::{MapShellPhysics, ShellHit, ShellState, bounce_volume, shell_alpha, shell_step},
    };

    fn physics() -> MapShellPhysics {
        let p = shell_physics();
        let u = p.unit;
        MapShellPhysics {
            gravity: p.gravity / u,
            unit: 1.0,
            up_jitter: p.up_jitter / u,
            right_jitter: p.right_jitter / u,
            sound_full_speed: p.sound_full_speed / u,
            ..p
        }
    }

    fn flying(velocity: Vec3) -> ShellState {
        ShellState {
            position: Vec3::ZERO,
            velocity,
            rotation: Quat::IDENTITY,
            spin: Vec2::new(1.0, 2.0),
            age: 0.0,
            resting: false,
            bounced: false,
        }
    }

    #[test]
    fn e2_ejection_speeds() {
        let p = physics();
        // The AK's event speed 150: 180..420 units/s forward, up to 10 up
        // or down and 20 left or right.
        let (lo, hi) = (150.0 * p.speed_factor.0, 150.0 * p.speed_factor.1);
        assert!((lo - 180.0).abs() < 1e-3 && (hi - 420.0).abs() < 1e-3, "{lo} {hi}");
        assert!((p.up_jitter - 10.0).abs() < 1e-3 && (p.right_jitter - 20.0).abs() < 1e-3);
    }

    #[test]
    fn e3_shells_fall_with_full_gravity() {
        let mut s = flying(Vec3::new(100.0, 0.0, 0.0));
        assert_eq!(shell_step(&mut s, 0.01, None, &physics()), ShellHit::None);
        assert!((s.velocity.y + 8.0).abs() < 1e-3, "{}", s.velocity);
        assert!((s.position.x - 1.0).abs() < 1e-4);
    }

    #[test]
    fn e4_slow_shells_rest_on_floors() {
        let mut s = flying(Vec3::new(30.0, -20.0, 0.0));
        let hit = Some((Vec3::new(0.3, -0.2, 0.0), Vec3::Y));
        assert_eq!(shell_step(&mut s, 0.01, hit, &physics()), ShellHit::Rested(-20.0));
        assert!(s.resting && s.velocity == Vec3::ZERO && s.spin == Vec2::ZERO);
        // At rest it no longer moves.
        let at = s.position;
        shell_step(&mut s, 0.01, None, &physics());
        assert_eq!(s.position, at);
    }

    #[test]
    fn e5_fast_shells_bounce_at_half_speed() {
        // Source (100, 0, -200) is ours (100, -200, 0).
        let mut s = flying(Vec3::new(100.0, -200.0, 0.0));
        let hit = Some((Vec3::ZERO, Vec3::Y));
        assert_eq!(shell_step(&mut s, 0.01, hit, &physics()), ShellHit::Bounced(-200.0));
        assert!(
            (s.velocity - Vec3::new(50.0, 100.0, 0.0)).length() < 1e-3,
            "{}",
            s.velocity
        );
        assert!(s.bounced && !s.resting);
    }

    #[test]
    fn e6_shells_fade_after_ten_seconds() {
        let p = physics();
        assert_eq!(shell_alpha(5.0, &p), Some(1.0));
        let a = shell_alpha(11.0, &p).unwrap();
        assert!((a - 0.5).abs() < 1e-4 && (255.0 * a) as u8 == 127, "{a}");
        assert_eq!(shell_alpha(12.0, &p), None);
    }

    #[test]
    fn bounce_sounds_scale_with_impact_speed() {
        let p = physics();
        assert!((p.sound_chance - 1.0 / 6.0).abs() < 1e-6);
        assert!((bounce_volume(-225.0, &p) - 0.5).abs() < 1e-4);
        assert_eq!(bounce_volume(-900.0, &p), 1.0);
    }
}
