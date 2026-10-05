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

/// Which window props let a ray through (along the model's facing axis,
/// from 1.5 m out on either side to its centre), with prop collision on or
/// off.
fn open_windows(map: &MapData, props_solid: bool) -> Vec<usize> {
    use avian3d::prelude::*;

    let mut map = map.clone();
    if !props_solid {
        map.props
            .iter_mut()
            .for_each(|p| p.solid = mashup::map::PropSolid::None);
    }
    let windows: Vec<(usize, Vec3, Quat)> = map
        .props
        .iter()
        .enumerate()
        .filter(|(_, p)| map.models[p.model].meshes.iter().any(|m| m.material.contains("window")))
        .map(|(i, p)| (i, p.translation, p.rotation))
        .collect();
    let mut sim = Sim::new(MapPlugin::new(map));
    let world = sim.app.world_mut();
    let mut state: bevy::ecs::system::SystemState<SpatialQuery> = bevy::ecs::system::SystemState::new(world);
    let query = state.get(world).unwrap();
    windows
        .into_iter()
        .filter(|(_, at, rot)| {
            let axis = *rot * Vec3::X;
            let centre = *at + Vec3::Y * 0.3;
            [axis, -axis].iter().any(|dir| {
                query
                    .cast_ray(
                        centre + *dir * 1.5,
                        Dir3::new(-*dir).unwrap(),
                        1.6,
                        true,
                        &SpatialQueryFilter::default(),
                    )
                    .is_none()
            })
        })
        .map(|(i, _, _)| i)
        .collect()
}

#[test]
fn props_load_and_windows_block() {
    let Some(map) = dust2() else { return };
    assert!(map.props.len() > 300, "{} props", map.props.len());
    assert!(
        map.models.iter().all(|m| !m.meshes.is_empty()),
        "a prop model has no meshes"
    );

    // Real openings: windows a ray passes through when props don't collide.
    // Only props the map marks solid should close them (dust2 has a
    // non-solid sill that stays open in the real game too).
    let openings: Vec<usize> = open_windows(&map, false)
        .into_iter()
        .filter(|i| map.props[*i].solid != mashup::map::PropSolid::None)
        .collect();
    eprintln!("{} window props sit in real openings", openings.len());
    assert!(!openings.is_empty(), "expected some windows in wall openings");
    let still_open: Vec<_> = open_windows(&map, true)
        .into_iter()
        .filter(|i| openings.contains(i))
        .collect();
    assert!(
        still_open.is_empty(),
        "{} of {} window openings stay open with props solid: {:?}",
        still_open.len(),
        openings.len(),
        still_open.iter().map(|i| map.props[*i].translation).collect::<Vec<_>>()
    );
}

/// The lighting model used for props (ambient cube + shadow-tested direct
/// lights), evaluated at world surface points, must reproduce the map's own
/// lightmaps there. Guards units, sun direction and shadow geometry.
#[test]
fn prop_lighting_model_predicts_lightmaps() {
    use mashup::games::cs_source::{ambient, bsp};

    let Some(map) = dust2() else { return };
    let install = LocalConfig::load().unwrap().game_path(cs_source::GAME).unwrap();
    let mount = cs_source::mount::open(&install).unwrap();
    let bytes = mount.read("maps/de_dust2.bsp").unwrap();
    let source = vbsp::Bsp::read(&bytes).unwrap();
    let lighting = ambient::MapLighting::read(&source, &bytes);
    let occluders = ambient::Occluders::new(
        &bsp::shadow_hulls(&source),
        (&map.collision_positions, &map.collision_indices),
    );
    let lm = map.lightmap.as_ref().unwrap();
    let at = |uv: [f32; 2]| {
        let x = ((uv[0] * lm.width as f32) as usize).min(lm.width as usize - 1);
        let y = ((uv[1] * lm.height as f32) as usize).min(lm.height as usize - 1);
        Vec3::from(lm.rgb[y * lm.width as usize + x]).element_sum() / 3.0
    };
    let mut ratios = Vec::new();
    for m in &map.meshes {
        for t in m.indices.as_chunks::<3>().0.iter().step_by(7) {
            let c = t.map(|i| Vec3::from(m.positions[i as usize])).iter().sum::<Vec3>() / 3.0;
            let uvs = t.map(|i| m.lightmap_uvs[i as usize]);
            let uv = [
                (uvs[0][0] + uvs[1][0] + uvs[2][0]) / 3.0,
                (uvs[0][1] + uvs[1][1] + uvs[2][1]) / 3.0,
            ];
            let actual = at(uv);
            if actual > 0.02 {
                let n = Vec3::from(m.normals[t[0] as usize]);
                ratios.push(lighting.light_at(&source, &occluders, c, n).element_sum() / 3.0 / actual);
            }
        }
    }
    ratios.sort_by(|a, b| a.total_cmp(b));
    let median = ratios[ratios.len() / 2];
    let within = ratios.iter().filter(|r| (0.5..2.0).contains(*r)).count() as f32 / ratios.len() as f32;
    eprintln!(
        "predicted/actual median {median:.2}, within 2x {:.0}% of {}",
        within * 100.0,
        ratios.len()
    );
    assert!((0.8..1.25).contains(&median), "median predicted/actual {median}");
    assert!(within > 0.7, "only {:.0}% within 2x", within * 100.0);
}

#[test]
fn decals_project_onto_surfaces() {
    use mashup::games::cs_source::decals;

    // Wall decals stay upright and unmirrored for someone facing the wall
    // (Source space: Z up). With u to the right and v down, as images are
    // stored, u x v points into the wall (away from the viewer).
    for n in [
        Vec3::X,
        -Vec3::X,
        Vec3::Y,
        -Vec3::Y,
        Vec3::new(1.0, 1.0, 0.0).normalize(),
    ] {
        let (right, down) = decals::basis(n);
        assert!((down - -Vec3::Z).length() < 1e-5, "wall {n}: down is {down}");
        assert!(right.cross(down).dot(n) < -0.99, "wall {n}: decal mirrored");
    }

    let Some(map) = dust2() else { return };
    let decal_meshes: Vec<_> = map
        .meshes
        .iter()
        .filter(|m| m.material.starts_with("decal:") && !m.material.starts_with("decal:overlay:"))
        .collect();
    assert!(decal_meshes.len() >= 18, "{} decal materials", decal_meshes.len());
    for m in &decal_meshes {
        assert!(m.texture.is_some(), "{}: no texture", m.material);
        assert_eq!(
            m.lightmap_uvs.len(),
            m.positions.len(),
            "{}: decals share the surface's lighting",
            m.material
        );
        assert!(
            m.uvs
                .iter()
                .all(|uv| (-1e-3..=1.001).contains(&uv[0]) && (-1e-3..=1.001).contains(&uv[1])),
            "{}: UVs outside the decal",
            m.material
        );
    }
    // dust2 has 135 infodecals; a few sit on props or brush entities.
    let unplaced: usize = map
        .warnings
        .iter()
        .find_map(|w| w.strip_suffix(" decals found no surface to project onto")?.parse().ok())
        .unwrap_or(0);
    assert!(unplaced <= 10, "{unplaced} decals unplaced");
}

#[test]
fn sky_has_six_faces() {
    let Some(map) = dust2() else { return };
    let sky = map.sky.as_ref().expect("dust2 names a sky (sky_dust)");
    let mut used: Vec<usize> = sky.faces.iter().map(|(t, _)| *t).collect();
    used.sort();
    used.dedup();
    assert_eq!(used.len(), 6, "each cube face needs its own texture");
    for (t, orient) in &sky.faces {
        let tex = &map.textures[*t];
        assert!(tex.name.contains("skybox/sky_dust"), "{}", tex.name);
        assert!(*orient < 8);
    }
}

#[test]
fn three_d_skybox_is_separated() {
    let Some(map) = dust2() else { return };
    let cam = map.sky_camera.as_ref().expect("dust2 has a sky_camera");
    assert_eq!(cam.scale, 4.0);
    assert!(cam.fog.is_some(), "dust2's skybox has fog");
    let sky_props = map.props.iter().filter(|p| p.skybox).count();
    let sky_meshes = map.meshes.iter().filter(|m| m.skybox).count();
    assert!(
        sky_props > 20 && sky_meshes > 5,
        "{sky_props} skybox props, {sky_meshes} skybox meshes"
    );
    // The palm crowns live in the skybox; trunks in the world.
    assert!(map.props.iter().any(|p| {
        p.skybox
            && map.models[p.model]
                .meshes
                .iter()
                .any(|m| m.material.contains("palm_tree_branches"))
    }));
    // Every spawn is in the playable world, nowhere near the skybox.
    for (feet, _) in &map.spawns {
        assert!(feet.distance(cam.origin) > 50.0);
    }
}

#[test]
fn overlays_are_placed() {
    let Some(map) = dust2() else { return };
    let overlays: Vec<_> = map
        .meshes
        .iter()
        .filter(|m| m.material.starts_with("decal:overlay:"))
        .collect();
    // dust2: 55 overlays over 24 textures (posters, graffiti, wires, road).
    assert!(overlays.len() >= 20, "{} overlay materials", overlays.len());
    assert!(overlays.iter().any(|m| m.material.contains("bills")), "posters missing");
    for m in &overlays {
        assert_eq!(m.lightmap_uvs.len(), m.positions.len(), "{}", m.material);
        assert!(!m.indices.is_empty());
    }
    let empty: usize = map
        .warnings
        .iter()
        .find_map(|w| w.strip_suffix(" overlays produced no geometry")?.parse().ok())
        .unwrap_or(0);
    assert!(empty <= 3, "{empty} overlays produced nothing");
}

/// Source movement (specs/cs_source/movement.md) on the real map: every
/// spawn settles onto the floor, and walking reaches knife speed without
/// falling through or getting stuck.
#[test]
fn source_movement_walks_dust2() {
    use mashup::games::cs_source::movement::{self, SourceMovementPlugin};
    let Some(map) = dust2() else { return };
    let spawns = map.spawns.clone();
    let mut sim = Sim::new((MapPlugin::new(map), SourceMovementPlugin));
    // The transform is the standing box's centre, 36 units above the feet.
    let lift = Vec3::Y * (36.0 * 0.0254 + 0.5);
    let players: Vec<_> = spawns
        .iter()
        .map(|(feet, _)| sim.spawn_character(*feet + lift, movement::ID))
        .collect();
    sim.seconds(1.5);
    for ((feet, team), p) in spawns.iter().zip(&players) {
        let standing = sim.position(*p).y - 36.0 * 0.0254;
        assert!(sim.state(*p).on_ground, "{team:?} spawn {feet}: not grounded");
        assert!(
            (-0.3..1.5).contains(&(feet.y - standing)),
            "{team:?} spawn {feet}: feet at {standing}"
        );
    }

    let p = players[0];
    let start = sim.position(p);
    let mut moved = 0.0f32;
    let mut top_speed = 0.0f32;
    for yaw in [0.0f32, 90.0, 180.0, 270.0] {
        sim.intent(p).yaw = yaw.to_radians();
        sim.intent(p).move_axis = Vec2::Y;
        let before = sim.position(p);
        for _ in 0..64 {
            sim.ticks(1);
            top_speed = top_speed.max(sim.velocity(p).xz().length());
        }
        let after = sim.position(p);
        moved += (after - before).xz().length();
        assert!(after.y > start.y - 3.0, "fell through the map at {after}");
    }
    assert!(moved > 4.0, "barely moved: {moved} m");
    let knife = 250.0 * 0.0254;
    // Landing on a downhill slope adds a little speed (the slope clip turns
    // falling speed into horizontal speed), as in the game.
    assert!(
        top_speed > knife - 0.01 && top_speed < knife * 1.03,
        "top speed {top_speed} m/s, expected about {knife}"
    );
}

/// Displacements with two-texture (WorldVertexTransition) materials carry
/// the second texture and per-vertex blend weights from the map.
#[test]
fn displacement_texture_blending() {
    let Some(map) = dust2() else { return };
    let blended: Vec<_> = map.meshes.iter().filter(|m| m.blend.is_some()).collect();
    assert!(!blended.is_empty(), "no two-texture surfaces");
    for m in &blended {
        let b = m.blend.unwrap();
        assert!(b.texture.is_some(), "{}: second texture missing", m.material);
        assert_eq!(m.blend_weights.len(), m.positions.len(), "{}", m.material);
        assert!(m.blend_weights.iter().all(|w| (0.0..=1.0).contains(w)));
    }
    let mixed = blended
        .iter()
        .flat_map(|m| m.blend_weights.iter())
        .filter(|w| **w > 0.05 && **w < 0.95)
        .count();
    assert!(mixed > 100, "only {mixed} partly blended vertices");
}

/// Convex solid props join the exact brush collision used by Source
/// movement (crates and boxes), so sweeps against them get flat faces.
#[test]
fn convex_props_collide_as_brushes() {
    let Some(map) = dust2() else { return };
    let world = map.collision_brushes.len();
    let props = map.props.iter().filter(|p| !p.skybox).count();
    let sim = Sim::new(MapPlugin::new(map));
    let all = sim.app.world().resource::<mashup::map::MapBrushes>().0.len();
    println!("{} of {props} props as brushes", all - world);
    assert!(
        all - world > props / 4,
        "only {} of {props} props became brushes",
        all - world
    );
}

/// Walking into a tall solid prop (crate, box) must not climb it: before
/// convex props were exact brushes, contacts near a face's triangle
/// diagonal gave tilted normals and players could run up vertical faces.
/// (MASHUP_NO_PROP_BRUSHES=1 brings the old collision back to compare.)
#[test]
fn props_cannot_be_climbed_by_walking() {
    use mashup::games::cs_source::movement::{self, SourceMovementPlugin};
    let Some(map) = dust2() else { return };
    let m = 0.0254;
    // World boxes of tall, solid props from the map data.
    let boxes: Vec<(Vec3, Vec3)> = map
        .props
        .iter()
        .filter(|p| !p.skybox && p.solid != mashup::map::PropSolid::None)
        .map(|p| {
            let (lo, hi) = map.models[p.model].bounds;
            let corners = (0..8).map(|i| {
                let c = Vec3::new(
                    if i & 1 == 0 { lo.x } else { hi.x },
                    if i & 2 == 0 { lo.y } else { hi.y },
                    if i & 4 == 0 { lo.z } else { hi.z },
                );
                p.translation + p.rotation * c
            });
            corners.fold((Vec3::splat(f32::MAX), Vec3::splat(f32::MIN)), |(a, b), c| {
                (a.min(c), b.max(c))
            })
        })
        .filter(|(lo, hi)| hi.y - lo.y > 30.0 * m && (hi.x - lo.x) > 16.0 * m)
        .collect();
    let mut sim = Sim::new((MapPlugin::new(map), SourceMovementPlugin));
    let mut tested = 0;
    let mut worst = (0.0f32, Vec3::ZERO);
    for (lo, hi) in boxes.iter().take(30) {
        let centre = (*lo + *hi) / 2.0;
        // Approach the -X face from 40 units out, at points along the face
        // and at angles to it, running into it for a second.
        for along in [-0.3f32, 0.0, 0.3] {
            for angle in [-40.0f32, 0.0, 40.0] {
                let z = centre.z + along * (hi.z - lo.z);
                let start = Vec3::new(lo.x - 40.0 * m, lo.y + 37.0 * m, z);
                let p = sim.spawn_character(start, movement::ID);
                sim.ticks(20);
                let before = sim.position(p);
                // Yaw -90 degrees faces +X; turn by `angle`.
                sim.intent(p).yaw = (-90.0 + angle).to_radians();
                sim.intent(p).move_axis = Vec2::Y;
                let mut top = before.y;
                for _ in 0..64 {
                    sim.ticks(1);
                    top = top.max(sim.position(p).y);
                }
                let rise = (top - before.y) / m;
                let height = (hi.y - (before.y - 36.0 * m)) / m;
                // Count runs that started free and reached the prop.
                let reached = sim.position(p).x > lo.x - 28.0 * m;
                if height > 22.0 && reached && sim.state(p).on_ground {
                    tested += 1;
                    if rise > worst.0 {
                        worst = (rise, centre);
                    }
                }
                let _ = sim.app.world_mut().despawn(p);
            }
        }
    }
    println!("worst rise {:.1} units at {} over {tested} runs", worst.0, worst.1);
    assert!(tested >= 20, "only {tested} runs");
    assert!(worst.0 < 19.0, "climbed {:.1} units up a prop at {}", worst.0, worst.1);
}

/// Prop entities load alongside the static props: dust2's 75
/// prop_physics_multiplayer (baskets, barrels), solid, lit by probes.
#[test]
fn physics_props_load() {
    let Some(map) = dust2() else { return };
    // 321 static props (one is no-draw) plus 75 physics props.
    assert!(map.props.len() >= 320 + 75, "{} props", map.props.len());
    // A grain basket from the entity lump: origin 361.969 2700.69 99.7093.
    let basket = Vec3::new(361.969, 99.7093, -2700.69) * 0.0254;
    let prop = map
        .props
        .iter()
        .find(|p| p.translation.distance(basket) < 0.01)
        .expect("grain basket not placed");
    assert_eq!(prop.solid, mashup::map::PropSolid::Mesh);
    assert!(prop.lighting.is_some());
}

/// `$detail` materials (dust2's crates, sandcrete, tile) carry their
/// detail texture: default mode 0 (mod2x), scale 4, factor 1.
#[test]
fn detail_textures() {
    let Some(map) = dust2() else { return };
    let with: Vec<_> = map.meshes.iter().filter(|m| m.detail.is_some()).collect();
    assert!(with.len() >= 4, "{} meshes with detail", with.len());
    let crate_mesh = with
        .iter()
        .find(|m| m.material.contains("ducrtlrgsd"))
        .expect("crate side has a detail texture");
    let d = crate_mesh.detail.unwrap();
    assert_eq!((d.mode, d.scale, d.factor), (0, [4.0, 4.0], 1.0));
    for m in &with {
        let c = m.positions.iter().fold(Vec3::ZERO, |a, p| a + Vec3::from(*p)) / m.positions.len() as f32;
        println!(
            "{} centre (Source) {:.0} {:.0} {:.0}",
            m.material,
            c.x / 0.0254,
            -c.z / 0.0254,
            c.y / 0.0254
        );
    }
}

/// dust2's env_fog_controller: on, 500 to 4000 units, color 197 196 165.
#[test]
fn world_fog() {
    let Some(map) = dust2() else { return };
    let fog = map.fog.expect("dust2 has world fog");
    assert!((fog.start - 500.0 * 0.0254).abs() < 1e-4 && (fog.end - 4000.0 * 0.0254).abs() < 1e-4);
    assert_eq!(fog.color, [197.0 / 255.0, 196.0 / 255.0, 165.0 / 255.0]);
    assert_eq!(fog.max_density, 1.0);
}
