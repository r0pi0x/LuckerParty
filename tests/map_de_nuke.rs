//! de_nuke from a real CS:S install, headless: its ladders are climbable.
//! Skipped without an install.

use bevy::prelude::*;
use mashup::{
    games::{
        self, cs_source,
        cs_source::movement::{self, SourceMovement, SourceMovementPlugin},
    },
    harness::Sim,
    map::{MapData, MapPlugin},
    mount::config::LocalConfig,
};

fn nuke() -> Option<MapData> {
    let installed = LocalConfig::load()
        .ok()?
        .game_path(cs_source::GAME)
        .is_some_and(|p| p.join("cstrike").is_dir());
    if !installed {
        eprintln!("skipping: no CS:S install configured");
        return None;
    }
    Some(games::load_map("cs_source:de_nuke").expect("load de_nuke"))
}

/// For every ladder brush: stand in front of one of its faces, hold
/// forward, and climb. de_nuke's ladder models collide only by their thin
/// rails (`.phy`): props must collide by their `.phy`, or the visible rungs
/// block the ladders.
#[test]
fn ladders_can_be_climbed() {
    const M: f32 = 0.0254;
    let Some(map) = nuke() else { return };
    let ladders: Vec<_> = map.collision_brushes.iter().filter(|b| b.ladder).cloned().collect();
    assert!(!ladders.is_empty(), "de_nuke has ladders");
    let mut climbed = 0;
    for b in &ladders {
        let centre = (b.min + b.max) / 2.0;
        // Engine axes: X, Y up, Z. Try the four horizontal faces.
        let tries = [Vec3::X, Vec3::NEG_X, Vec3::Z, Vec3::NEG_Z].map(|d| {
            let reach = (b.max - b.min).dot(d.abs()) / 2.0 + 17.0 * M;
            // Feet well above the ladder's bottom (the floor in front may be
            // raised); the player settles first. The transform is 36 up.
            let at = Vec3::new(centre.x, b.min.y + 64.0 * M + 36.0 * M, centre.z) + d * reach;
            (at, -d)
        });
        let ok = tries.iter().any(|(at, facing)| {
            let mut sim = Sim::new((MapPlugin::new(map.clone()), SourceMovementPlugin));
            let p = sim.spawn_character(*at, movement::ID);
            // Intent yaw 0 looks down -Z.
            sim.intent(p).yaw = (-facing.x).atan2(-facing.z);
            sim.ticks(32);
            sim.intent(p).move_axis = Vec2::Y;
            let y0 = sim.position(p).y;
            sim.ticks(48);
            let on = sim.app.world().get::<SourceMovement>(p).unwrap().ladder.is_some();
            let dy = (sim.position(p).y - y0) / M;
            on && dy > 40.0
        });
        climbed += ok as usize;
    }
    // Checked against CS:S with movecmp: the outside ladder climbs (tick
    // for tick as in the game). The ladder at (856..864, -1448..-1422) has
    // a player-clip face coinciding with its face, which wins the trace in
    // CS:S too, so it doesn't attach from straight in front. The third
    // (230..232, -816..-784) is player-width between walls 0.01 units proud
    // of its face; not attached from in front either.
    assert!(climbed >= 1, "{climbed} of {} ladders climbed", ladders.len());
}

/// Feet (Source units) to the character's transform (engine meters).
fn feet_to_transform(x: f32, y: f32, z: f32) -> Vec3 {
    const M: f32 = 0.0254;
    Vec3::new(x, z + 36.0, -y) * M
}

/// Walk at a ladder from `feet` (Source units) facing `yaw_source` degrees
/// (Source yaw: 0 = +x), holding forward: the most it rose (units) and
/// whether it got onto the ladder.
fn climb(map: &MapData, feet: Vec3, yaw_source: f32, ticks: u64) -> (f32, bool) {
    const M: f32 = 0.0254;
    let mut sim = Sim::new((MapPlugin::new(map.clone()), SourceMovementPlugin));
    sim.set_tick_interval(cs_source::TICK_INTERVAL);
    let p = sim.spawn_character(feet_to_transform(feet.x, feet.y, feet.z), movement::ID);
    // Intent yaw 0 looks down engine -Z, which is Source yaw 90.
    sim.intent(p).yaw = (yaw_source - 90.0).to_radians();
    sim.ticks(16);
    let y0 = sim.position(p).y;
    sim.intent(p).move_axis = Vec2::Y;
    let (mut rise, mut on) = (0.0f32, false);
    for _ in 0..ticks {
        sim.ticks(1);
        on |= sim.app.world().get::<SourceMovement>(p).unwrap().ladder.is_some();
        rise = rise.max((sim.position(p).y - y0) / M);
    }
    (rise, on)
}

/// The vent ladders (from the ducts up toward A, (416..424, -1448..-1422)
/// and (856..864, ...)) sit flush with player-clip brushes on both sides,
/// and the 32-unit box is wider than the 26-unit ladder, so every probe
/// hits a clip face at the same distance as the ladder face. As in CS:S
/// the brush the trace reaches first through the BSP tree wins: the
/// ladder beats the south clip, the north clip beats the ladder. So the
/// ladders climb from the south half of the duct, not from dead centre
/// (CS:S didn't attach there either, measured with movecmp on the east
/// one). The outside ladder climbs as before.
#[test]
fn vent_and_outside_ladders_climb() {
    let Some(map) = nuke() else { return };
    // West vent ladder: walk west along the duct's south half.
    let (rise, on) = climb(&map, Vec3::new(470.0, -1446.0, -598.0), 180.0, 120);
    assert!(on && rise > 150.0, "west vent ladder: on {on}, rose {rise}");
    // East vent ladder: walk east.
    let (rise, on) = climb(&map, Vec3::new(780.0, -1446.0, -598.0), 0.0, 120);
    assert!(on && rise > 150.0, "east vent ladder: on {on}, rose {rise}");
    // Dead centre: the north clip wins the tie.
    let (_, on) = climb(&map, Vec3::new(780.0, -1435.0, -598.0), 0.0, 60);
    assert!(!on, "east vent ladder from dead centre attaches");
    // The outside ladder (1038..1039, -429..-407), from its +x side.
    let (rise, on) = climb(&map, Vec3::new(1060.0, -418.0, -414.0), 180.0, 120);
    assert!(on && rise > 150.0, "outside ladder: on {on}, rose {rise}");
}
