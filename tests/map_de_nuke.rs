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
/// forward, and climb. Two of de_nuke's ladders have a ladder model in
/// front whose collision (`.phy`) is only its thin rails; until props
/// collide by their `.phy`, the visible rungs block the ladder.
#[test]
#[ignore = "needs .phy prop collision (docs/plans/active/css-world-physics.md, step 2)"]
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
    assert_eq!(climbed, ladders.len(), "ladders climbed");
}
