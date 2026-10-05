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
fn collision_comes_from_world_brushes() {
    let Some(map) = dust2() else { return };
    // ~2,100 player-solid world brushes, minus those carrying displacements.
    assert!(
        (1900..2100).contains(&map.collision_hulls.len()),
        "{} hulls",
        map.collision_hulls.len()
    );
    assert!(
        !map.collision_indices.is_empty(),
        "displacement surfaces should collide"
    );
}

/// Every spawn must be usable: a player placed there drops onto the floor
/// just below the marker (a solid covering a spawn would hold it up or trap
/// it; a hole would let it fall).
#[test]
fn every_spawn_lands_on_the_floor() {
    let Some(map) = dust2() else { return };
    let spawns = map.spawns.clone();
    let mut sim = Sim::new(MapPlugin::new(map));
    let players: Vec<_> = spawns
        .iter()
        .map(|(feet, _)| sim.spawn_character(*feet + Vec3::Y * 1.0, placeholder::ID))
        .collect();
    sim.seconds(1.5);
    for ((feet, team), p) in spawns.iter().zip(players) {
        let below = feet.y + 0.9 - sim.position(p).y;
        assert!(sim.state(p).on_ground, "{team:?} spawn {feet}: not grounded");
        // Markers sit up to ~1 m above the floor, or slightly into a slope.
        assert!(
            (-0.3..1.5).contains(&below),
            "{team:?} spawn {feet}: standing {below} m below the marker"
        );
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

/// Lighting must agree where neighbouring faces meet: sample each lit face's
/// own lightmap at vertices it shares with another face on the same plane
/// (flat faces) or anywhere (displacements). Misread or misoriented
/// lightmaps show up as large mismatches.
#[test]
fn lightmaps_agree_at_shared_edges() {
    use std::collections::HashMap;

    use mashup::games::cs_source::{bsp, lightmap};
    use vbsp::Bsp;

    if dust2().is_none() {
        return;
    }
    let install = LocalConfig::load().unwrap().game_path(cs_source::GAME).unwrap();
    let mount = cs_source::mount::open(&install).unwrap();
    let bytes = mount.read("maps/de_dust2.bsp").unwrap();
    let lump = lightmap::lighting_lump(&bytes);
    let map = Bsp::read(&bytes).unwrap();

    fn bilinear(s: &lightmap::FaceSamples, c: Vec2) -> f32 {
        let x = c.x.clamp(0.0, s.width as f32 - 1.0);
        let y = c.y.clamp(0.0, s.height as f32 - 1.0);
        let (x0, y0) = (x.floor() as u32, y.floor() as u32);
        let (x1, y1) = ((x0 + 1).min(s.width - 1), (y0 + 1).min(s.height - 1));
        let (fx, fy) = (x - x0 as f32, y - y0 as f32);
        let v = |x: u32, y: u32| s.rgb[(y * s.width + x) as usize].iter().sum::<f32>();
        (v(x0, y0) * (1.0 - fx) + v(x1, y0) * fx) * (1.0 - fy) + (v(x0, y1) * (1.0 - fx) + v(x1, y1) * fx) * fy
    }

    // (is displacement, quantized position, quantized plane) -> (face, value)
    type Key = (bool, [i32; 3], [i32; 4]);
    let mut shared: HashMap<Key, Vec<(usize, f32)>> = HashMap::new();
    for (fi, face) in map.models().next().unwrap().faces().enumerate() {
        let Some(samples) = lightmap::face_samples(lump, &face) else {
            continue;
        };
        let disp = face.displacement().is_some();
        let n = face.normal();
        let plane = if disp {
            [0; 4]
        } else {
            let d = map.plane(face.plane_num as usize).unwrap().dist;
            [
                (n.x * 100.0) as i32,
                (n.y * 100.0) as i32,
                (n.z * 100.0) as i32,
                d.round() as i32,
            ]
        };
        for t in bsp::face_triangles(&face) {
            for (p, luxel) in t {
                let key = (
                    disp,
                    [(p.x * 4.0) as i32, (p.y * 4.0) as i32, (p.z * 4.0) as i32],
                    plane,
                );
                shared.entry(key).or_default().push((fi, bilinear(&samples, luxel)));
            }
        }
    }
    for disp in [false, true] {
        let (mut err, mut count) = (0.0f32, 0);
        for ((d, _, _), group) in &shared {
            let faces: std::collections::BTreeSet<_> = group.iter().map(|g| g.0).collect();
            if *d != disp || faces.len() < 2 {
                continue;
            }
            let lo = group.iter().map(|g| g.1).fold(f32::MAX, f32::min);
            let hi = group.iter().map(|g| g.1).fold(f32::MIN, f32::max);
            err += (hi - lo) / hi.max(0.05);
            count += 1;
        }
        let mean = err / count.max(1) as f32;
        eprintln!("displacement {disp}: mean seam mismatch {mean:.3} over {count} shared points");
        assert!(count > 100, "too few shared points ({count}) to judge");
        assert!(
            mean < 0.1,
            "lightmaps disagree at shared edges (displacement {disp}): {mean:.3}"
        );
    }
}
