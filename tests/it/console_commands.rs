//! The client's console commands on a headless `Sim`: `getpos` and
//! `setpos` follow CS:S's conventions.

use bevy::prelude::*;
use mashup::{
    client::console::{HeldActions, client_commands},
    console::Console,
    core::LocalPlayer,
    games::cs_source::movement::{self, SourceMovementPlugin},
    greybox::GreyboxMapPlugin,
    harness::Sim,
};

/// CS:S's conventions (reference captures: `setpos x y z` puts the
/// camera at z + 64 standing, in noclip too; `getpos` printed the view's
/// origin, standing on a floor its height + 64): `getpos` prints the
/// eye, the movement's own view offset above the feet (64 standing, 47
/// ducked), and `setpos` takes the feet.
#[test]
fn getpos_prints_the_eye_and_setpos_takes_the_feet() {
    let mut sim = Sim::new((GreyboxMapPlugin, SourceMovementPlugin));
    sim.app.init_resource::<HeldActions>();
    client_commands(&mut sim.app);
    let p = sim.spawn_character(Vec3::new(0.0, 1.0, 0.0), movement::ID);
    sim.app.world_mut().entity_mut(p).insert(LocalPlayer);
    fn run(sim: &mut Sim, line: &str) -> String {
        sim.app.world_mut().resource_mut::<Console>().submit(line);
        sim.ticks(1);
        sim.app.world().resource::<Console>().output.last().map(|l| l.text.clone()).unwrap_or_default()
    }
    let getpos = |sim: &mut Sim| -> Vec3 {
        let line = run(sim, "getpos");
        let n: Vec<f32> = line
            .trim_start_matches("setpos ")
            .split(';')
            .next()
            .unwrap()
            .split_whitespace()
            .map(|w| w.parse().unwrap())
            .collect();
        Vec3::new(n[0], n[1], n[2])
    };
    let feet = |sim: &Sim| (sim.position(p).y + sim.state(p).hull_min.y) / 0.0254;
    sim.seconds(1.0);
    assert!(sim.state(p).on_ground);
    let floor = feet(&sim);
    let at = getpos(&mut sim);
    assert!((at.z - (floor + 64.0)).abs() < 0.02, "standing: {at}, floor {floor}");
    sim.intent(p).crouch = true;
    sim.seconds(1.0);
    let at = getpos(&mut sim);
    assert!((at.z - (feet(&sim) + 47.0)).abs() < 0.02, "ducked: {at} feet {}", feet(&sim));
    sim.intent(p).crouch = false;
    sim.seconds(1.0);
    // setpos on the floor: back where it was, the eye 64 up.
    run(&mut sim, &format!("setpos 10 20 {floor}"));
    let at = getpos(&mut sim);
    assert!(at.distance(Vec3::new(10.0, 20.0, floor + 64.0)) < 0.02, "{at}");
    assert!((feet(&sim) - floor).abs() < 0.02);
    // In noclip too (the dust2 view: the camera at 564).
    run(&mut sim, "noclip");
    run(&mut sim, "setpos -295 1078 500");
    let at = getpos(&mut sim);
    assert!(at.distance(Vec3::new(-295.0, 1078.0, 564.0)) < 0.02, "noclip: {at}");
}

/// `thirdperson` with `cam_idealdist 150`, as CS:S's capture on
/// mg_item_battle_v4b (view about 15.4 degrees down): the camera sits
/// 64.5 units above the eye, 150 units back along the view from a pivot
/// 24.7 above the eye, and looks along the view. A ceiling over the eye
/// stops the pivot short of it.
#[test]
fn third_person_camera_sits_as_css_does() {
    use avian3d::prelude::SpatialQuery;
    use bevy::ecs::system::RunSystemOnce;
    use mashup::client::view::{CameraMode, THIRD_PERSON_PIVOT_UP, camera_offset};
    const U: f32 = 0.0254;
    let mut sim = Sim::new((GreyboxMapPlugin, SourceMovementPlugin));
    // High over the greybox: nothing in the way.
    let origin = Vec3::new(0.0, 40.0, 12.0);
    let eye = Vec3::Y * 28.0 * U;
    sim.ticks(1);
    let mode = CameraMode {
        third_person: true,
        ..default()
    };
    let at = |sim: &mut Sim, pitch_down: f32, mode: CameraMode| -> Vec3 {
        let look = Quat::from_euler(EulerRot::YXZ, 0.0, -pitch_down.to_radians(), 0.0);
        sim.app
            .world_mut()
            .run_system_once(move |spatial: SpatialQuery| camera_offset(&mode, origin, eye, look, &spatial, []))
            .unwrap()
    };
    let o = at(&mut sim, 15.4, mode);
    let above = (o.y - eye.y) / U;
    assert!((above - 64.5).abs() < 0.3, "{above} units above the eye");
    let back = (o.z - eye.z) / U;
    assert!((back - 150.0 * 15.4f32.to_radians().cos()).abs() < 0.1, "{back} back");
    // Level: the pivot's height.
    let o = at(&mut sim, 0.0, mode);
    assert!(((o.y - eye.y) / U - THIRD_PERSON_PIVOT_UP).abs() < 0.01);
    // The spectators' chase camera has no raise.
    let o = at(&mut sim, 0.0, CameraMode { pivot_up: 0.0, ..mode });
    assert!((o.y - eye.y).abs() < 1e-4);
    // A ceiling 16 units over the eye: the pivot stops under it, less
    // the camera's radius.
    sim.app.world_mut().spawn((
        avian3d::prelude::RigidBody::Static,
        avian3d::prelude::Collider::cuboid(4.0, 0.2, 4.0),
        Transform::from_translation(origin + eye + Vec3::Y * (16.0 * U + 0.1)),
    ));
    sim.ticks(2);
    let o = at(&mut sim, 0.0, mode);
    let above = (o.y - eye.y) / U;
    assert!(above > 0.0 && above < 16.0 - 0.2 / U + 0.1, "{above} under the ceiling");
}
