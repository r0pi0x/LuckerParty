//! de_dust2 from a real CS:S install, headless. Skipped without one.

use bevy::prelude::*;
use mashup::{
    core::Team,
    games::{self, cs_source},
    harness::Sim,
    map::{MapData, MapPlugin},
    mount::config::LocalConfig,
    movement::placeholder,
};

fn dust2() -> Option<MapData> {
    let installed = LocalConfig::load()
        .ok()?
        .game_path(cs_source::GAME)
        .is_some_and(|p| p.join("cstrike").is_dir());
    if !installed {
        eprintln!("skipping: no CS:S install configured");
        return None;
    }
    Some(games::load_map("cs_source:de_dust2").expect("load de_dust2"))
}

#[test]
fn converts_to_a_plausible_map() {
    let Some(map) = dust2() else { return };
    let tris = map.triangle_count();
    assert!(tris > 10_000, "only {tris} triangles");
    let (lo, hi) = map.bounds();
    let size = hi - lo;
    // The playable area is ~100 m across; the bounds also include the 3D
    // skybox model far to one side (~340 x 63 x 137 m in total).
    assert!(
        size.x > 60.0 && size.x < 400.0 && size.z > 60.0 && size.z < 400.0,
        "size {size}"
    );
    assert!(size.y > 5.0 && size.y < 100.0, "height {}", size.y);
    let t = map.spawns.iter().filter(|(_, team)| *team == Some(Team(1))).count();
    let ct = map.spawns.iter().filter(|(_, team)| *team == Some(Team(2))).count();
    assert!(t >= 10 && ct >= 10, "{t} T spawns, {ct} CT spawns");
    assert!(
        map.meshes.iter().any(|m| m.material.starts_with("de_dust/")),
        "no de_dust materials"
    );
}

#[test]
fn most_surfaces_get_textures() {
    let Some(map) = dust2() else { return };
    let textured: usize = map
        .meshes
        .iter()
        .filter(|m| m.texture.is_some())
        .map(|m| m.indices.len() / 3)
        .sum();
    let share = textured as f32 / map.triangle_count() as f32;
    for w in &map.warnings {
        eprintln!("warning: {w}");
    }
    assert!(share > 0.95, "only {:.1}% of triangles textured", share * 100.0);
    for t in &map.textures {
        assert_eq!(t.rgba8.len(), (t.width * t.height * 4) as usize, "{}", t.name);
    }
}

#[test]
fn player_lands_at_spawn_and_can_walk() {
    let Some(map) = dust2() else { return };
    let (feet, _) = map.spawns[0];
    let mut sim = Sim::new(MapPlugin::new(map));
    let p = sim.spawn_character(feet + Vec3::Y * 1.0, placeholder::ID);
    sim.seconds(1.0);
    assert!(sim.state(p).on_ground, "not grounded at spawn: {}", sim.position(p));
    let start = sim.position(p);
    // Spawn markers float up to ~1 m above dust2's sloped ground; the player
    // drops onto the floor (capsule center 0.9 m above it).
    let below = feet.y + 0.9 - start.y;
    assert!(
        (0.0..1.5).contains(&below),
        "standing at {}, spawn feet at {}",
        start.y,
        feet.y
    );

    // Walk in each direction for a second: we must stay on or above the
    // ground (never fall through the world) and move somewhere.
    let mut moved = 0.0f32;
    for yaw in [0.0f32, 90.0, 180.0, 270.0] {
        sim.intent(p).yaw = yaw.to_radians();
        sim.intent(p).move_axis = Vec2::Y;
        let before = sim.position(p);
        sim.seconds(1.0);
        let after = sim.position(p);
        moved += (after - before).xz().length();
        assert!(after.y > start.y - 3.0, "fell through the map at {after}");
    }
    assert!(moved > 2.0, "barely moved ({moved} m) - stuck in geometry?");
}

#[test]
fn baked_lighting_covers_the_map() {
    let Some(map) = dust2() else { return };
    let l = map.lightmap.as_ref().expect("dust2 has baked lighting");
    assert_eq!(l.rgb.len(), (l.width * l.height) as usize);
    for m in &map.meshes {
        assert_eq!(m.lightmap_uvs.len(), m.positions.len(), "{}: lightmap UVs", m.material);
        assert!(
            m.lightmap_uvs
                .iter()
                .all(|uv| (0.0..=1.0).contains(&uv[0]) && (0.0..=1.0).contains(&uv[1])),
            "{}: lightmap UV outside the atlas",
            m.material
        );
    }
    // Brightness should look like a lit outdoor map: mostly mid values,
    // some overbright sunlight, some shadow.
    let mut lum: Vec<f32> = l
        .rgb
        .iter()
        .map(|c| (c[0] + c[1] + c[2]) / 3.0)
        .filter(|v| *v > 0.0)
        .collect();
    lum.sort_by(|a, b| a.total_cmp(b));
    let q = |p: f32| lum[((lum.len() - 1) as f32 * p) as usize];
    assert!(
        q(0.1) < 0.3 && q(0.5) > 0.2 && q(0.95) > 0.8,
        "luminance p10 {} p50 {} p95 {}",
        q(0.1),
        q(0.5),
        q(0.95)
    );
}
