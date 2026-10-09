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
