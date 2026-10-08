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

/// The scoreboard and spectator layouts, the scoreboard's and bomb's
/// sprites, the C4 screen's look, and GameUI's loading dialog and sounds.
#[test]
fn the_scoreboard_spectator_bars_and_bomb_look_load() {
    use mashup::map::hud::{UiKind, UiSound};
    let Some(map) = dust2() else { return };
    let hud = map.hud.as_ref().expect("CS:S has a HUD");
    let menus = hud.menus.as_ref().expect("CS:S has its VGUI menus");
    let board = &menus.layouts[menus.scoreboard.as_ref().expect("scoreboard.res")];
    let background = board.get("ScoreboardBackground").unwrap();
    assert_eq!(background.kind, UiKind::Image);
    let picture = background.image.as_ref().expect("its picture (../vgui/ resolved)");
    assert!(hud.sprites.contains_key(picture), "{picture}");
    for cell in ["CTPlayerName0", "CTPlayerStatus0", "TPlayerLatency0"] {
        assert!(board.get(cell).is_some(), "{cell}");
    }
    assert_eq!(menus.string("Cstrike_ScoreBoard_CT", ""), "COUNTER-TERRORISTS");
    for sprite in ["scoreboard_bomb", "scoreboard_dead", "scoreboard_defuser", "c4", "ledglow"] {
        assert!(hud.sprites.contains_key(sprite), "{sprite}");
    }
    let bars = &menus.layouts[menus.spectator.as_ref().expect("spectator.res")];
    assert!(bars.get("topbar").is_some() && bars.get("playerlabel").is_some());
    let spec_menu = &menus.layouts[menus.spectator_menu.as_ref().expect("bottomspectator.res")];
    assert!(spec_menu.get("viewcombo").is_some());
    // valve_english's strings under the game's.
    assert_eq!(menus.string("Spec_Map", ""), "Map: %s1");
    let screen = &hud.screens["c4_view_panel"];
    assert_eq!(screen.pixels, Vec2::new(200.0, 100.0));
    assert_eq!(screen.colors.get("C4Panel_Armed"), Some(&[255, 30, 13, 200]));
    assert_eq!(screen.font.as_ref().map(|f| f.family.as_str()), Some("Courier New"));
    let c4 = map.view_models.iter().find(|v| v.key == cs_source::objectives::C4).unwrap();
    let (_, _, size) = mashup::map::view_model::screen_corners(c4).expect("the C4's screen attachments");
    assert!((size.x - 2.7).abs() < 0.01 && (size.y - 1.25).abs() < 0.01, "{size}");
    // GameUI: the loading dialog and the interface sounds.
    let config = LocalConfig::load().unwrap();
    let install = config.game_path("cs_source").unwrap();
    let ui = cs_source::gameui::load(&cs_source::mount::open(&install).unwrap()).expect("GameUI");
    let loading = ui.loading.as_ref().expect("the loading dialog");
    assert_eq!(loading.get("LoadingDialog").map(|c| (c.wide, c.tall)), Some((380.0, 112.0)));
    assert!(loading.get("Progress").is_some() && loading.get("InfoLabel").is_some());
    for s in [UiSound::Rollover, UiSound::Click, UiSound::Release] {
        assert!(ui.sounds.contains_key(&s), "{s:?}");
    }
    assert_eq!(ui.string("LoadingProgress_LoadMap"), Some("Loading world..."));
}

#[test]
fn the_games_buy_and_team_menus_load() {
    use mashup::map::hud::{UiKind, layout_key};
    let Some(map) = dust2() else { return };
    let hud = map.hud.as_ref().expect("CS:S has a HUD");
    let menus = hud.menus.as_ref().expect("CS:S has its VGUI menus");
    for team in [1, 2] {
        let main = &menus.layouts[menus.buy_page(Some(team)).unwrap()];
        let pistols = main.get("pistols").expect("a pistols button");
        assert_eq!(pistols.hotkey, Some('1'));
        assert!(pistols.text.contains("PISTOLS"), "{:?}", pistols.text);
        // Every category page it opens loaded.
        for c in main.controls.iter().filter(|c| c.kind == UiKind::Button) {
            if let Some(page) = c.command.as_deref().filter(|p| p.to_lowercase().ends_with(".res")) {
                assert!(menus.layouts.contains_key(&layout_key(page)), "page {page} missing");
            }
        }
    }
    // An item: its button's description panel, with the weapon's picture.
    let rifles = &menus.layouts["resource/ui/buyrifles_ter.res"];
    let ak = rifles.get("ak47").unwrap();
    assert_eq!(ak.command.as_deref(), Some("buy ak47"));
    let info = &menus.layouts[ak.info.as_ref().expect("ak47 has a description")];
    let image = info.get("classimage").and_then(|c| c.image.as_ref()).expect("a picture");
    assert!(hud.sprites.contains_key(image), "{image} not loaded");
    assert!(info.get("price").is_some_and(|p| p.text.contains('$')));
    // The team menu, with dust2's description.
    let team = &menus.layouts[menus.team.as_ref().unwrap()];
    assert_eq!(team.get("terbutton").unwrap().command.as_deref(), Some("jointeam 2"));
    assert!(menus.map_info.as_ref().is_some_and(|t| !t.is_empty()));
    // The client scheme's text fonts, read as the Windows game reads them.
    let chat = &hud.text_fonts["ChatFont"];
    assert!(hud.text_fonts.contains_key("Default") && hud.text_fonts.contains_key("MenuTitle"));
    assert_eq!(chat[0].family, "Verdana");
    // Bold, one size per screen-height range, and a proportional one
    // (no `yres`) for taller screens.
    assert!(chat[0].bold() && chat[0].yres == Some((480, 599)), "{chat:?}");
    assert!(chat.iter().filter(|s| s.yres.is_some()).count() >= 3, "{chat:?}");
    assert!(chat.iter().any(|s| s.yres.is_none()), "{chat:?}");
    assert_eq!(hud.text_fonts["CenterPrintText"][0].family, "Trebuchet MS");
}

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
    // Every spawn has a facing, and they aren't all the default.
    assert_eq!(map.spawn_yaws.len(), map.spawns.len());
    assert!(map.spawn_yaws.iter().any(|y| y.abs() > 0.1));
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
    let lighting = ambient::MapLighting::read(&bytes);
    let occluders = ambient::Occluders::new(
        &bsp::shadow_hulls(&source, &ambient::raw_leaves(&bytes)),
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

/// Overlays and decals in the 3D skybox go with it (drawn by the sky
/// camera, scaled up), not at their raw position outside the playable map.
#[test]
fn skybox_overlays_stay_in_the_skybox() {
    let Some(map) = dust2() else { return };
    let (mut lo, mut hi) = (Vec3::splat(f32::MAX), Vec3::splat(f32::MIN));
    for m in map
        .meshes
        .iter()
        .filter(|m| !m.skybox && !m.material.starts_with("decal:"))
    {
        for p in &m.positions {
            lo = lo.min(Vec3::from(*p));
            hi = hi.max(Vec3::from(*p));
        }
    }
    let decals: Vec<_> = map.meshes.iter().filter(|m| m.material.starts_with("decal:")).collect();
    for m in decals.iter().filter(|m| !m.skybox) {
        for p in &m.positions {
            let p = Vec3::from(*p);
            assert!(
                p.cmpge(lo - 1.0).all() && p.cmple(hi + 1.0).all(),
                "{} at {p} is outside the playable world",
                m.material
            );
        }
    }
    assert!(
        decals.iter().any(|m| m.skybox),
        "dust2's 3D skybox has overlays; none were assigned to it"
    );
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

/// Physics junk props get their textures: the milk carton's model names its
/// texture directory with a doubled separator (`models\props_junk\\`),
/// and the plastic crate is alpha-tested.
#[test]
fn junk_props_are_textured() {
    let Some(map) = dust2() else { return };
    let warnings: Vec<_> = map
        .warnings
        .iter()
        .filter(|w| w.contains("garbage") || w.contains("plasticcrate"))
        .collect();
    assert!(warnings.is_empty(), "{warnings:#?}");
    for name in ["garbage001a_01", "plasticcrate01a"] {
        let mesh = map
            .models
            .iter()
            .flat_map(|m| &m.meshes)
            .find(|m| m.material.contains(name))
            .unwrap_or_else(|| panic!("{name} not loaded"));
        assert!(mesh.texture.is_some(), "{name} has no texture");
    }
    let crate_mesh = map
        .models
        .iter()
        .flat_map(|m| &m.meshes)
        .find(|m| m.material.contains("plasticcrate01a"))
        .unwrap();
    assert!(
        matches!(crate_mesh.alpha, mashup::map::MapAlpha::Mask(c) if c == 0.5),
        "{:?}",
        crate_mesh.alpha
    );
    // Its mesh survives distance only through the VTF's own mips, which
    // Valve built to keep alpha-test coverage.
    let tex = &map.textures[crate_mesh.texture.unwrap()];
    assert_eq!(tex.mips.len(), 9, "{}: VTF mips not used", tex.name);
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

#[test]
#[ignore]
fn debug_lightmap_texel_percentiles() {
    let Some(map) = dust2() else { return };
    let lm = map.lightmap.as_ref().unwrap();
    let mut v: Vec<u8> = Vec::new();
    let mut push = |rgb: &[[f32; 3]]| {
        for c in rgb.iter().flatten() {
            let t = mashup::map::source_ldr_texel(*c);
            if t > 0 {
                v.push(t);
            }
        }
    };
    push(&lm.rgb);
    if let Some(b) = &lm.bumped {
        for p in b {
            push(p);
        }
    }
    v.sort();
    let n = v.len();
    println!(
        "ours {n} {:?}",
        [0.1, 0.25, 0.5, 0.75, 0.9, 0.99].map(|p| v[(n as f64 * p) as usize])
    );
}

/// dust2's 18 env_sprite lamp glows: 12 world-space glows (mode 9), 6
/// additive (mode 5), sprites/glow (64 texels) at quantised scales.
#[test]
fn sprites_load() {
    let Some(map) = dust2() else { return };
    assert_eq!(map.sprites.len(), 18, "warnings: {:?}", map.warnings);
    assert_eq!(map.sprites.iter().filter(|s| s.glow).count(), 12);
    // Scale 0.8 is drawn at 0.75 (network quantisation).
    let first = map
        .sprites
        .iter()
        .find(|s| s.position.distance(Vec3::new(-1406.8, 320.0, -1152.38) * 0.0254) < 0.01)
        .expect("lamp glow at -1406.8 1152.38 320");
    let t = &map.textures[first.texture];
    assert!(
        (first.size.x - t.width as f32 * 0.75 * 0.0254).abs() < 1e-4,
        "{:?}",
        first.size
    );
}

/// dust2's two func_dustmotes volumes (T house, long doors): 300/s and
/// 50/s, 3-5 s life, 15-unit motes, fading out by 512 units.
#[test]
fn dust_motes_load() {
    let Some(map) = dust2() else { return };
    assert_eq!(map.dust.len(), 2);
    let mut rates: Vec<f32> = map.dust.iter().map(|d| d.rate).collect();
    rates.sort_by(f32::total_cmp);
    assert_eq!(rates, vec![50.0, 300.0]);
    for d in &map.dust {
        assert!(d.texture.is_some(), "particle/sparkles");
        assert_eq!((d.life, d.size), ((3.0, 5.0), (15.0, 15.0)));
        assert!((d.fade_distance - 512.0 * 0.0254).abs() < 1e-4);
        let (lo, hi) = (d.min / 0.0254, d.max / 0.0254);
        println!(
            "dust volume (Source) x {:.0}..{:.0} y {:.0}..{:.0} z {:.0}..{:.0}",
            lo.x, hi.x, -hi.z, -lo.z, lo.y, hi.y
        );
    }
}

/// Sky visibility from BSP leaves (specs/cs_source/shadows_sky.md): open
/// ground sees the 3D sky, a point inside a wall is solid.
#[test]
fn sky_visibility_by_leaf() {
    use mashup::map::LeafSky;
    let Some(map) = dust2() else { return };
    let vis = map.sky_vis.as_ref().expect("sky visibility");
    let at = |x: f32, y: f32, z: f32| vis.at(Vec3::new(x, z, -y) * 0.0254);
    // CT spawn, eye height.
    assert_eq!(at(352.0, 2464.0, -24.0), LeafSky::Sky3d);
    // The refcmp spot where CS:S shows black: inside solid.
    assert_eq!(at(-1406.0, 900.0, 150.0), LeafSky::Solid);
    // Far outside the map.
    assert_eq!(at(0.0, 0.0, 20000.0), LeafSky::Solid);
    let counts = [LeafSky::Solid, LeafSky::None, LeafSky::Sky2d, LeafSky::Sky3d]
        .map(|k| vis.leaves.iter().filter(|l| **l == k).count());
    println!("leaves solid/none/2d/3d: {counts:?}");
    assert!(counts[1] > 0 && counts[3] > 0, "{counts:?}");
}

/// Ambient light samples sit inside their own leaf (leaves read in tree
/// order, not vbsp's cluster-sorted order).
#[test]
fn ambient_samples_lie_in_their_leaf() {
    let Some(_) = dust2() else { return };
    let config = LocalConfig::load().unwrap();
    let mount = cs_source::mount::open(&config.game_path(cs_source::GAME).unwrap()).unwrap();
    let bytes = mount.read("maps/de_dust2.bsp").unwrap();
    let bsp = vbsp::Bsp::read(&bytes).unwrap();
    let leaves = cs_source::ambient::raw_leaves(&bytes);
    let (mut inside, mut total) = (0, 0);
    for (i, leaf) in leaves.iter().enumerate().filter(|(_, l)| l.contents & 1 == 0).take(400) {
        let c = |k: usize| (leaf.mins[k] as f32 + leaf.maxs[k] as f32) / 2.0;
        let p = vbsp::Vector {
            x: c(0),
            y: c(1),
            z: c(2),
        };
        total += 1;
        if cs_source::ambient::leaf_index(&bsp, p) == Some(i) {
            inside += 1;
        }
    }
    // Leaf boxes are bounds of convex cells, so most centres fall inside.
    assert!(
        inside * 10 >= total * 8,
        "{inside}/{total} leaf centres map back to their leaf"
    );
}

/// Props' collision models (`.phy`) line up with their visible models:
/// the pieces' bounds sit inside the mesh bounds (within a few units) and
/// cover most of them.
#[test]
fn prop_collision_models_line_up() {
    let Some(map) = dust2() else { return };
    let mut with = 0;
    let mut bad = Vec::new();
    for (i, m) in map.models.iter().enumerate() {
        let Some(c) = &m.collision else { continue };
        with += 1;
        let pts = c.pieces.iter().flat_map(|p| &p.points);
        let (lo, hi) = pts.fold((Vec3::MAX, Vec3::MIN), |(a, b), p| (a.min(*p), b.max(*p)));
        let (mlo, mhi) = m.bounds;
        // A few collision models are authored slightly larger than the
        // .mdl hull (dust2's dustteeth: up to ~5 units).
        let slack = 6.0 * 0.0254;
        let inside = lo.cmpge(mlo - slack).all() && hi.cmple(mhi + slack).all();
        let covers = (hi - lo).max_element() > 0.5 * (mhi - mlo).max_element();
        if !(inside && covers) {
            bad.push(format!(
                "model {i} ({}): phy {lo}..{hi}, mesh {mlo}..{mhi}",
                m.meshes.first().map_or("", |m| m.material.as_str())
            ));
        }
    }
    println!("{with} of {} models have a .phy", map.models.len());
    assert!(with > 20, "{with} models with collision");
    assert!(bad.is_empty(), "{} misaligned:\n{}", bad.len(), bad.join("\n"));
}

/// dust2's shadow_control: angles (60, 43, 0), colour (159, 168, 181),
/// distance 75; its physics props cast shadows onto the world.
#[test]
fn prop_shadows() {
    let Some(map) = dust2() else { return };
    let s = map.shadows.as_ref().expect("shadow settings");
    let source = Vec3::new(s.direction.x, -s.direction.z, s.direction.y);
    assert!(
        (source - Vec3::new(0.36568, 0.34100, -0.86603)).length() < 1e-4,
        "{source}"
    );
    assert_eq!(s.color, [159, 168, 181]);
    assert!((s.distance - 75.0 * 0.0254).abs() < 1e-5);
    let built = mashup::map::shadows::build(&map, s);
    let casters = map.props.iter().filter(|p| p.casts_shadow && !p.skybox).count();
    assert!(casters >= 70, "{casters} casters");
    // Most casters stand on the world, so their shadows land on it.
    assert!(
        built.meshes.len() * 10 >= casters * 8,
        "{} of {casters} shadows reach the world",
        built.meshes.len()
    );
    assert!(built.atlas.coverage.iter().any(|c| *c > 0.99), "silhouettes drawn");
}

/// Physics props (specs/cs_source/physics_props.md): dust2's are
/// prop_physics_multiplayer bodies with their .phy mass. They settle at map
/// start without falling through the world, and a running player passes
/// through a "solid"-mode one while shoving it away.
#[test]
fn physics_props_settle_and_get_pushed() {
    use mashup::{
        games::cs_source::movement::{self, SourceMovementPlugin, to_engine, to_source},
        map::{PhysicsProp, PushAway},
    };
    let Some(map) = dust2() else { return };
    let bodies: Vec<_> = map.props.iter().filter(|p| p.physics.is_some()).collect();
    assert!(bodies.len() >= 60, "{} physics props", bodies.len());
    let solid = bodies
        .iter()
        .filter(|p| p.physics.as_ref().unwrap().push == PushAway::Solid)
        .count();
    assert!(solid > 0, "some solid-mode props");

    let mut sim = Sim::new((MapPlugin::new(map.clone()), SourceMovementPlugin));
    // Settled within 12 s. Most are still at 4 s; an oil drum stood a
    // degree off upright (828 797 1) rocks on its rim for a while, and
    // how long depends on the solver's order across the whole world (a
    // change to far-away collision triangles changes it).
    sim.seconds(4.0);
    for _ in 0..16 {
        let moving = sim
            .app
            .world_mut()
            .query_filtered::<&avian3d::prelude::LinearVelocity, With<PhysicsProp>>()
            .iter(sim.app.world())
            .any(|v| v.0.length() >= 0.2);
        if !moving {
            break;
        }
        sim.seconds(0.5);
    }
    let mut q = sim
        .app
        .world_mut()
        .query::<(&Name, &Transform, &PhysicsProp, &avian3d::prelude::LinearVelocity)>();
    // (prop index from "Prop {i}", position, body, velocity)
    let props: Vec<(usize, Vec3, PhysicsProp, Vec3)> = q
        .iter(sim.app.world())
        .map(|(n, t, p, v)| (n.as_str()[5..].parse().unwrap(), t.translation, *p, v.0))
        .collect();
    assert_eq!(props.len(), bodies.len());
    for (i, at, _, v) in &props {
        let placed = &map.props[*i];
        let drop = placed.translation.y - at.y;
        assert!(drop < 0.5, "a prop at {} fell {drop} m", placed.translation);
        assert!(
            v.length() < 0.2,
            "a prop at {} still moving at {} m/s",
            placed.translation,
            v.length()
        );
    }

    // Run at a solid-mode prop from 100 units away, from whichever side
    // is open: the player passes through it and shoves it.
    let mut solids: Vec<_> = props
        .iter()
        .filter(|(_, _, p, _)| p.push == PushAway::Solid)
        .copied()
        .collect();
    solids.sort_by_key(|(i, ..)| *i);
    let mut shoved = None;
    let sides = [
        (Vec3::X, 0.0f32),
        (Vec3::NEG_X, 180.0),
        (Vec3::Y, 90.0),
        (Vec3::NEG_Y, 270.0),
    ];
    let tries = solids.iter().take(6).flat_map(|(i, t, ..)| sides.map(|d| (*i, *t, d)));
    for (index, target, (dir, yaw)) in tries {
        let name = format!("Prop {index}");
        let mut sim = Sim::new((MapPlugin::new(map.clone()), SourceMovementPlugin));
        sim.seconds(4.0);
        let start_feet = to_source(target) - dir * 100.0;
        let p = sim.spawn_character(to_engine(start_feet + Vec3::Z * 60.0), movement::ID);
        sim.seconds(0.5);
        // Intent yaw 0 looks down -Z, which is Source yaw 90.
        sim.intent(p).yaw = (yaw - 90.0f32).to_radians();
        sim.intent(p).move_axis = Vec2::Y;
        sim.seconds(1.5);
        let travelled = (to_source(sim.position(p)) - start_feet).dot(dir);
        if travelled < 100.0 {
            continue;
        }
        let mut q = sim.app.world_mut().query::<(&Name, &Transform)>();
        let moved = q
            .iter(sim.app.world())
            .find(|(n, _)| n.as_str() == name)
            .map(|(_, t)| t.translation.distance(target) / 0.0254)
            .unwrap();
        shoved = Some((travelled, moved));
        break;
    }
    let (travelled, moved) = shoved.expect("an open side to run from");
    assert!(
        moved > 10.0,
        "the prop was shoved {moved} units (player ran {travelled})"
    );
}

/// Sounds (specs/cs_source/sounds.md): the map loads the step sounds of
/// every surface and the swim sound, decoded; world meshes know their
/// surface.
#[test]
fn sounds_load() {
    let Some(map) = dust2() else { return };
    let s = &map.sounds;
    let concrete = s.surface("concrete").expect("concrete surface");
    assert_eq!(concrete.game_material, 'C');
    let step = s
        .entry(concrete.step_left.as_deref().unwrap())
        .expect("concrete step entry");
    assert!(!step.waves.is_empty(), "concrete steps have waves");
    assert_eq!(s.surface("dirt").map(|d| d.game_material), Some('D'));
    let swim = s.entry("Player.Swim").expect("swim");
    assert_eq!(swim.waves.len(), 4);
    assert!(matches!(swim.level, mashup::map::SoundLevel::Attenuation(a) if a == 1.0));
    // Weapon entries are precached with the map.
    for name in mashup::games::cs_source::weapons::sounds() {
        let e = s.entry(name).unwrap_or_else(|| panic!("no {name}"));
        assert!(!e.waves.is_empty(), "{name} has no waves");
    }
    // The announcer's round sounds.
    let r = &map.round_sounds;
    for name in [&r.attackers_win, &r.defenders_win, &r.draw].into_iter().flatten().chain(&r.start) {
        let e = s.entry(name).unwrap_or_else(|| panic!("no round sound {name}"));
        assert!(!e.waves.is_empty(), "{name} has no waves");
    }
    assert!(s.clips.len() > 40, "{} clips", s.clips.len());
    for c in &s.clips {
        assert!(c.rate >= 8000 && !c.samples.is_empty());
    }
    let with_surface = map.meshes.iter().filter(|m| m.surface.is_some()).count();
    assert!(
        with_surface * 2 > map.meshes.len(),
        "{with_surface} of {} meshes have a surface",
        map.meshes.len()
    );
}

/// Footsteps (specs/cs_source/sounds.md 3): running across CT spawn steps
/// every 300 ms (20 ticks at 64 tick), right foot first then alternating,
/// at the surface's running volume; a jump plays a full-volume step;
/// walking (+speed) is silent in CS:S.
#[test]
fn footsteps() {
    use mashup::{
        games::cs_source::movement::{self, SourceMovementPlugin},
        map::PlaySound,
    };
    #[derive(Resource, Default)]
    struct Heard(Vec<(u64, PlaySound)>);
    fn listen(mut m: MessageReader<PlaySound>, mut heard: ResMut<Heard>, tick: Res<mashup::core::SimTick>) {
        for s in m.read() {
            heard.0.push((tick.0, s.clone()));
        }
    }
    let Some(map) = dust2() else { return };
    // The T spawn runway (open and flat westward), feet at Source z 140.
    let spawn = mashup::games::cs_source::movement::to_engine(Vec3::new(-1024.0, -784.0, 140.0));
    let mut sim = Sim::new((MapPlugin::new(map.clone()), SourceMovementPlugin));
    sim.app
        .init_resource::<Heard>()
        .add_systems(FixedUpdate, listen.after(mashup::core::SimSet::Movement));
    let p = sim.spawn_character(spawn + Vec3::Y * (36.0 * 0.0254 + 0.2), movement::ID);
    sim.seconds(1.0);
    sim.app.world_mut().resource_mut::<Heard>().0.clear();
    // Face Source yaw 180 (west); intent yaw 0 is Source yaw 90.
    sim.intent(p).yaw = 90f32.to_radians();
    sim.intent(p).move_axis = Vec2::Y;
    sim.seconds(2.0);
    let steps: Vec<(u64, PlaySound)> = sim.app.world_mut().resource_mut::<Heard>().0.drain(..).collect();
    assert!(steps.len() >= 5, "{} steps in 2 s", steps.len());
    // Once up to speed, every 20 ticks.
    let gaps: Vec<u64> = steps.windows(2).map(|w| w[1].0 - w[0].0).collect();
    assert!(gaps[2..].iter().all(|g| *g == 20), "gaps {gaps:?}");
    assert!(
        steps[0].1.entry.to_lowercase().contains("right"),
        "first step {}",
        steps[0].1.entry
    );
    for w in steps.windows(2) {
        assert_ne!(
            w[0].1.entry.contains("Left"),
            w[1].1.entry.contains("Left"),
            "alternating"
        );
    }
    let v = steps.last().unwrap().1.volume.unwrap();
    assert!(v == 0.5 || v == 0.55, "running step volume {v}");

    // Jump: a full-volume step.
    sim.intent(p).jump = true;
    sim.ticks(1);
    sim.intent(p).jump = false;
    let heard: Vec<PlaySound> = sim
        .app
        .world_mut()
        .resource_mut::<Heard>()
        .0
        .drain(..)
        .map(|(_, s)| s)
        .collect();
    assert!(heard.iter().any(|s| s.volume == Some(1.0)), "jump step: {heard:?}");

    // Walking: silent.
    sim.seconds(1.0);
    sim.app.world_mut().resource_mut::<Heard>().0.clear();
    sim.intent(p).walk = true;
    sim.seconds(2.0);
    assert!(sim.app.world().resource::<Heard>().0.is_empty(), "walking is silent");
}

/// Footsteps on props (specs/cs_source/sounds.md 3): solid props carry a
/// surface property (their collision model's, else the model's own), and
/// the map's sounds know its steps. Names the scripts don't define (dust2's
/// rock props say "stone") step as "default".
#[test]
fn solid_props_have_step_surfaces() {
    let Some(map) = dust2() else { return };
    let solid: Vec<_> = map
        .props
        .iter()
        .filter(|p| !p.skybox && p.solid != mashup::map::PropSolid::None)
        .collect();
    assert!(solid.len() > 100, "only {} solid props", solid.len());
    let none = solid
        .iter()
        .filter(|p| map.models[p.model].surfaceprop.is_none())
        .count();
    assert_eq!(none, 0, "{none} solid props have no surface property");
    let steps = |name: &str| map.sounds.surface(name).is_some_and(|s| s.step_left.is_some());
    assert!(steps("default"), "no default steps");
    let known = solid
        .iter()
        .filter(|p| map.models[p.model].surfaceprop.as_deref().is_some_and(steps))
        .count();
    assert!(
        known * 10 >= solid.len() * 9,
        "only {known} of {} props have their own steps",
        solid.len()
    );
    // Props aren't all the world's concrete: crates, metal and wood show up.
    let names: std::collections::HashSet<_> = solid
        .iter()
        .filter_map(|p| map.models[p.model].surfaceprop.as_deref())
        .collect();
    assert!(names.len() >= 3, "surfaces: {names:?}");
}

/// Sounds written during a test.
#[derive(Resource, Default)]
struct Heard(Vec<String>);

fn record_sounds(mut sounds: MessageReader<mashup::map::PlaySound>, mut heard: ResMut<Heard>) {
    heard.0.extend(sounds.read().map(|s| s.entry.to_lowercase()));
}

/// Impact sounds (specs/cs_source/sounds.md 4): a physics prop dropped
/// onto the ground plays its surface's impact, and a shot into a wall plays
/// the wall's bullet impact.
#[test]
fn impact_sounds() {
    use avian3d::prelude::{LinearVelocity, Position};
    use mashup::{
        games::cs_source::{movement::SourceMovementPlugin, weapons::CsWeaponsPlugin},
        map::PhysicsProp,
    };
    let Some(map) = dust2() else { return };
    let mut sim = Sim::new((MapPlugin::new(map.clone()), SourceMovementPlugin, CsWeaponsPlugin));
    sim.app.init_resource::<Heard>().add_systems(Last, record_sounds);
    sim.seconds(3.0);
    sim.app.world_mut().resource_mut::<Heard>().0.clear();

    // Lift the first physics prop 1.5 m and let it fall.
    let mut q = sim
        .app
        .world_mut()
        .query_filtered::<(Entity, &Name), With<PhysicsProp>>();
    let mut props: Vec<(usize, Entity)> = q
        .iter(sim.app.world())
        .map(|(e, n)| (n.as_str()[5..].parse().unwrap(), e))
        .collect();
    props.sort();
    let prop = props[0].1;
    {
        let w = sim.app.world_mut();
        let mut e = w.entity_mut(prop);
        let up = e.get::<Position>().unwrap().0 + Vec3::Y * 1.5;
        e.get_mut::<Position>().unwrap().0 = up;
        e.get_mut::<Transform>().unwrap().translation = up;
        e.get_mut::<LinearVelocity>().unwrap().0 = Vec3::ZERO;
        let _ = avian3d::prelude::WakeBody(prop).apply(w);
    }
    sim.seconds(2.0);
    let heard = std::mem::take(&mut sim.app.world_mut().resource_mut::<Heard>().0);
    assert!(
        heard.iter().any(|s| s.contains(".impact")),
        "no impact sound from the dropped prop: {heard:?}"
    );

    // CT spawn 0 faces a wall corner: fire the AK-47 into it.
    let spawn = map.spawns[0].0 + Vec3::Y * 1.0;
    let p = sim.spawn_character(spawn, mashup::games::cs_source::movement::ID);
    sim.intent(p).yaw = map.spawn_yaws[0];
    sim.seconds(1.2);
    sim.app.world_mut().resource_mut::<Heard>().0.clear();
    sim.intent(p).fire = true;
    sim.ticks(1);
    sim.intent(p).fire = false;
    sim.ticks(2);
    let heard = &sim.app.world().resource::<Heard>().0;
    assert!(heard.iter().any(|s| s == "weapon_ak47.single"), "{heard:?}");
    assert!(heard.iter().any(|s| s.ends_with(".bulletimpact")), "{heard:?}");
}

/// Bots walk dust2's navigation mesh (specs/cs_source/nav.md) toward an
/// enemy they heard but can't see, until they see them.
#[test]
fn bot_walks_the_nav_mesh_to_an_enemy() {
    use mashup::games::cs_source::{
        movement::{self, SourceMovementPlugin},
        weapons::CsWeaponsPlugin,
    };
    let Some(map) = dust2() else { return };
    let nav = map.nav.clone().expect("dust2 has a nav mesh");
    let mut sim = Sim::new((MapPlugin::new(map.clone()), SourceMovementPlugin, CsWeaponsPlugin));
    sim.set_tick_interval(mashup::games::cs_source::TICK_INTERVAL);
    sim.app
        .insert_resource(mashup::slots::Loadout { movement: movement::ID });
    let middle = nav.places.iter().position(|p| p == "Middle").unwrap();
    let area = nav.areas.iter().find(|a| a.place == Some(middle)).unwrap();
    let stand = area.closest_point(area.center) + Vec3::Y * 1.0;
    let player = sim.spawn_character(stand, movement::ID);
    let bot = mashup::bot::add_bot(sim.app.world_mut(), Team(1)).expect("bot");
    sim.app.world_mut().resource_mut::<mashup::bot::BotConfig>().dont_shoot = 1;
    // Characters are never rotated; the spawn's facing is the look yaw.
    let rot = sim.app.world().get::<Transform>(bot).unwrap().rotation;
    assert!(rot.angle_between(Quat::IDENTITY) < 1e-4, "bot spawned rotated");
    let start = sim.position(bot);
    let start_dist = start.distance(stand);
    // The bot doesn't know where the player is until it hears a shot.
    sim.seconds(1.2);
    assert!(
        sim.app.world().get::<mashup::bot::Bot>(bot).unwrap().lead.is_none(),
        "knew too early"
    );
    sim.intent(player).pitch = -0.5;
    sim.intent(player).fire = true;
    sim.ticks(1);
    sim.intent(player).fire = false;
    sim.ticks(1);
    assert!(
        sim.app.world().get::<mashup::bot::Bot>(bot).unwrap().lead.is_some(),
        "didn't hear the shot"
    );
    let mut found = None;
    for s in 0..60 {
        sim.seconds(0.5);
        if sim.app.world().get::<mashup::bot::Bot>(bot).unwrap().target == Some(player) {
            found = Some(s as f32 * 0.5);
            break;
        }
    }
    let end = sim.position(bot);
    assert!(
        found.is_some(),
        "bot never saw the player: {start} -> {end}, {start_dist:.1} m away at start, {:.1} m at the end",
        end.distance(stand)
    );
    // It walked there: T spawn is ~36 m from Middle, out of sight.
    assert!(start.distance(end) > 15.0, "walked only {:.1} m", start.distance(end));
}

/// Soundscapes (specs/cs_source/sounds.md 6): dust2's two scripts, flattened,
/// and the eight trigger boxes that select them.
#[test]
fn soundscapes_load() {
    let Some(map) = dust2() else { return };
    let s = &map.sounds;
    let find = |n: &str| s.soundscapes.iter().find(|x| x.name.eq_ignore_ascii_case(n)).expect(n);
    let out = find("dust2.outdoors");
    assert_eq!(out.loops.len(), 5, "wind and four music loops");
    assert_eq!(out.randoms.len(), 4);
    let positioned: Vec<_> = out.loops.iter().filter_map(|l| l.position).collect();
    assert_eq!(positioned, [2, 1, 3, 4]);
    assert!((out.loops[1].level - 80.0).abs() < 1e-3);
    // "time" "15,40``" in the indoors script reads as 15..40.
    let ind = find("dust2.indoors");
    assert_eq!((ind.loops.len(), ind.randoms.len()), (2, 2));
    assert!((ind.randoms[0].time.start - 15.0).abs() < 1e-3 && (ind.randoms[0].time.range - 25.0).abs() < 1e-3);
    assert!(out.randoms.iter().all(|r| !r.clips.is_empty()));
    assert_eq!(s.soundscape_zones.len(), 8);
    // The outdoors positions (info_targets pos0..pos6) resolve.
    let z = s
        .soundscape_zones
        .iter()
        .find(|z| s.soundscapes[z.scape].name == "dust2.outdoors")
        .unwrap();
    assert_eq!(z.positions.iter().filter(|p| p.is_some()).count(), 7);
    // Every spawn stands in some soundscape zone.
    for (feet, _) in &map.spawns {
        let p = *feet + Vec3::Y * 1.6;
        assert!(
            s.soundscape_zones
                .iter()
                .any(|z| p.cmpge(z.min).all() && p.cmple(z.max).all()),
            "spawn {feet} is in no soundscape zone"
        );
    }
}

/// Reflections (specs/cs_source/shaders.md "$envmap"): the tunnel tile
/// floor's patched materials point at baked cubemaps in the map's pak.
#[test]
fn envmaps_load() {
    use mashup::map::EnvmapMask;
    let Some(map) = dust2() else { return };
    let tiles: Vec<_> = map
        .meshes
        .iter()
        .filter(|m| m.material.to_lowercase().contains("tilefloor02"))
        .collect();
    assert!(!tiles.is_empty());
    let with = tiles.iter().filter(|m| m.envmap.is_some()).count();
    assert!(
        with == tiles.len() && with >= 5,
        "{with} of {} tile meshes reflect",
        tiles.len()
    );
    let e = tiles.iter().find_map(|m| m.envmap).unwrap();
    assert_eq!(e.mask, EnvmapMask::NormalAlpha);
    // Fast path: $envmapsaturation .0001 with no contrast is ignored.
    assert_eq!((e.contrast, e.saturation, e.fresnel), (0.0, 1.0, 1.0));
    let c = &map.cubemaps[e.cubemap.expect("a baked cubemap")];
    assert_eq!(c.size, 64);
    assert!(c.faces.iter().all(|f| f.len() == 64 * 64 * 4));
    // Faces differ (not six copies of one image).
    assert_ne!(c.faces[4], c.faces[5]);
    // The cubemap lump's samples (37 on dust2) load for props.
    assert_eq!(map.cubemap_samples.len(), 37);
    // Cars reflect the nearest cubemap, tinted by the DX9 block (.125 on
    // car002b).
    let car = map
        .models
        .iter()
        .flat_map(|m| &m.meshes)
        .find(|m| m.material.contains("car002b"))
        .expect("car002b");
    let e = car.envmap.expect("car envmap");
    assert_eq!(e.cubemap, None);
    assert_eq!(e.mask, EnvmapMask::BaseAlphaInverted);
    assert!((e.tint[0] - 0.125).abs() < 1e-4, "{:?}", e.tint);
}

/// `map` loads in place (mashup::map::change_map): a second load replaces
/// the first without leaving spawn points, brushes or nav data behind, and
/// characters respawn at the new spawn points.
#[test]
fn maps_change_in_place() {
    use mashup::{core::SpawnPoint, map::MapPart};
    let Some(map) = dust2() else { return };
    let mut sim = Sim::new(MapPlugin::empty());
    let p = sim.spawn_character(Vec3::new(0.0, 100.0, 0.0), mashup::movement::placeholder::ID);
    let count = |sim: &mut Sim| {
        let w = sim.app.world_mut();
        let spawns = w.query_filtered::<(), With<SpawnPoint>>().iter(w).count();
        let parts = w.query_filtered::<(), With<MapPart>>().iter(w).count();
        (spawns, parts)
    };
    for _ in 0..2 {
        mashup::map::change_map(sim.app.world_mut(), map.clone(), mashup::map::MapDebugView::Normal);
        mashup::rules::respawn_everyone(sim.app.world_mut());
        sim.ticks(2);
    }
    let (spawns, parts) = count(&mut sim);
    assert_eq!(spawns, map.spawns.len());
    let w = sim.app.world();
    assert!(w.get_resource::<mashup::map::MapBrushes>().is_some());
    assert!(w.get_resource::<mashup::map::nav::NavMesh>().is_some());
    // Respawned at a spawn point.
    let at = sim.position(p);
    assert!(
        map.spawns.iter().any(|(feet, _)| feet.distance(at) < 2.0),
        "player at {at} after the change ({parts} map parts)"
    );
}

/// Character models (specs/cs_source/weapons.md 5.2, T23): the player
/// models' 19 hitboxes in the reference pose, standing upright.
#[test]
fn character_models_and_hitboxes() {
    use mashup::core::Hitgroup;
    let Some(map) = dust2() else { return };
    assert_eq!(map.characters.len(), 2);
    for c in &map.characters {
        assert!(!c.model.meshes.is_empty());
        assert_eq!(c.hitboxes.len(), 19);
        let count = |g: Hitgroup| c.hitboxes.iter().filter(|h| h.group == g).count();
        assert_eq!(count(Hitgroup::Head), 2, "head and neck");
        assert_eq!(count(Hitgroup::Chest), 1);
        assert_eq!(count(Hitgroup::Stomach), 2, "pelvis and spine1");
        assert_eq!(count(Hitgroup::LeftLeg), 4);
        assert_eq!(count(Hitgroup::RightArm), 3);
        // Upright, feet at 0: heads up near 1.6-1.85 m, legs low.
        let top = |g| {
            c.hitboxes
                .iter()
                .filter(|h| h.group == g)
                .map(|h| h.center.y)
                .fold(f32::MIN, f32::max)
        };
        let low = c.hitboxes.iter().map(|h| h.center.y).fold(f32::MAX, f32::min);
        assert!(
            top(Hitgroup::Head) > 1.5 && top(Hitgroup::Head) < 1.9,
            "head at {}",
            top(Hitgroup::Head)
        );
        assert!(low < 0.2, "lowest box at {low}");
        // Model vertices span the same height.
        let ys: Vec<f32> = c
            .model
            .meshes
            .iter()
            .flat_map(|m| m.positions.iter().map(|p| p[1]))
            .collect();
        let (lo, hi) = (
            ys.iter().cloned().fold(f32::MAX, f32::min),
            ys.iter().cloned().fold(f32::MIN, f32::max),
        );
        assert!(lo > -0.1 && lo < 0.1 && hi > 1.6 && hi < 2.0, "mesh height {lo}..{hi}");
    }
}

/// Shots test the character model's hitboxes: a head shot is a head shot,
/// and a shot that only grazes the collision hull passes through.
#[test]
fn shots_use_model_hitboxes() {
    use mashup::{
        core::{Hitboxes, Hitgroup},
        games::cs_source::{TICK_INTERVAL, weapons::CsWeaponsPlugin},
        weapon::{WeaponEvent, WeaponEventKind},
    };
    #[derive(Resource, Default)]
    struct Hits(Vec<(Entity, Hitgroup)>);
    fn record(mut e: MessageReader<WeaponEvent>, mut hits: ResMut<Hits>) {
        for e in e.read() {
            if let WeaponEventKind::Hit { target, hitgroup, .. } = e.kind {
                hits.0.push((target, hitgroup));
            }
        }
    }
    let Some(map) = dust2() else { return };
    let mut sim = Sim::new((MapPlugin::new(map.clone()), CsWeaponsPlugin));
    sim.set_tick_interval(TICK_INTERVAL);
    sim.app.init_resource::<Hits>().add_systems(Last, record);
    // CT spawns 0 and 2: same height, 4.9 m apart in the open.
    let at = |i: usize| map.spawns[i].0 + Vec3::Y * 0.9;
    let shooter = sim.spawn_character(at(2), mashup::movement::placeholder::ID);
    let target = sim.spawn_character(at(0), mashup::movement::placeholder::ID);
    sim.app.world_mut().entity_mut(target).insert(mashup::core::God);
    sim.seconds(1.5);
    let boxes = sim
        .app
        .world()
        .get::<Hitboxes>(target)
        .expect("hitboxes attached")
        .0
        .clone();
    let feet = sim.position(target) - Vec3::Y * 0.9;
    let shoot = |sim: &mut Sim, at: Vec3| -> Vec<(Entity, Hitgroup)> {
        let eye = sim.position(shooter) + sim.state(shooter).eye_offset;
        let d = at - eye;
        {
            let mut i = sim.intent(shooter);
            i.yaw = (-d.x).atan2(-d.z);
            i.pitch = d.y.atan2(d.xz().length());
        }
        sim.app.world_mut().resource_mut::<Hits>().0.clear();
        sim.intent(shooter).fire = true;
        sim.ticks(1);
        sim.intent(shooter).fire = false;
        sim.seconds(0.6);
        std::mem::take(&mut sim.app.world_mut().resource_mut::<Hits>().0)
    };
    // The target faces -Z (yaw 0): box centres are in its frame as-is.
    let head = boxes
        .iter()
        .filter(|b| b.group == Hitgroup::Head)
        .max_by(|a, b| a.center.y.total_cmp(&b.center.y))
        .unwrap();
    assert_eq!(shoot(&mut sim, feet + head.center), [(target, Hitgroup::Head)]);
    let chest = boxes.iter().find(|b| b.group == Hitgroup::Chest).unwrap();
    assert_eq!(shoot(&mut sim, feet + chest.center), [(target, Hitgroup::Chest)]);
    // Just inside the hull's edge at knee height, outside every box.
    let gap = feet + Vec3::new(0.0, 0.45, 0.36);
    let hits = shoot(&mut sim, gap);
    assert!(hits.iter().all(|(e, _)| *e != target), "grazing shot hit {hits:?}");
}

/// Every gun has its upper body on the player models (by its suffix), the
/// shotguns their shell-by-shell reload gestures and the Elites a shot per
/// hand; a shotgun reload and Elite shots play them.
#[test]
fn every_gun_has_its_body_animations() {
    use mashup::{
        games::cs_source::{
            movement::{self, SourceMovementPlugin},
            player_anim::{PlayerAnimPlugin, suffix},
            weapons::{CsWeaponsPlugin, ELITE, GUNS, M3},
        },
        map::anim::Animator,
        weapon::{Inventory, Magazine, give},
    };
    let Some(map) = dust2() else { return };
    let set = map.characters[0].animations.clone().expect("player animations");
    for g in GUNS {
        let x = suffix(Some(g.id));
        for seq in [format!("Idle_Upper_{x}"), format!("Run_Upper_{x}"), format!("Crouch_Idle_Upper_{x}")] {
            assert!(set.sequence(&seq).is_some(), "{}: no {seq}", g.id);
        }
        let reload = if g.shells.is_some() { "_start" } else { "" };
        assert!(
            set.sequence(&format!("Idle_Reload_{x}{reload}")).is_some(),
            "{}: no reload gesture",
            g.id
        );
    }
    for part in ["_start", "_loop", "_end"] {
        for x in ["M3S90", "XM1014"] {
            assert!(set.sequence(&format!("Idle_Reload_{x}{part}")).is_some(), "{x}{part}");
        }
    }
    for side in ["_L", "_R"] {
        assert!(set.sequence(&format!("Idle_Shoot_ELITES{side}")).is_some(), "ELITES{side}");
    }
    let mut sim = Sim::new((
        MapPlugin::new(map.clone()),
        SourceMovementPlugin,
        CsWeaponsPlugin,
        PlayerAnimPlugin,
    ));
    sim.set_tick_interval(mashup::games::cs_source::TICK_INTERVAL);
    sim.app
        .insert_resource(mashup::slots::Loadout { movement: movement::ID });
    let spawn = movement::to_engine(Vec3::new(-1024.0, -784.0, 140.0));
    let c = sim.spawn_character(spawn + Vec3::Y * (36.0 * 0.0254 + 0.2), movement::ID);
    sim.seconds(0.5);
    let reload_layer = |sim: &Sim| {
        let a = sim.app.world().get::<Animator>(c).unwrap();
        a.layers
            .get(5)
            .copied()
            .flatten()
            .map(|l| set.sequences[l.sequence].name.to_lowercase())
    };
    let fire_layer = |sim: &Sim| {
        let a = sim.app.world().get::<Animator>(c).unwrap();
        a.layers
            .get(4)
            .copied()
            .flatten()
            .map(|l| set.sequences[l.sequence].name.to_lowercase())
    };
    give(sim.app.world_mut(), c, M3).unwrap();
    sim.seconds(1.2);
    let m3 = sim.app.world().get::<Inventory>(c).unwrap().active.unwrap();
    sim.app.world_mut().get_mut::<Magazine>(m3).unwrap().clip = 6;
    sim.intent(c).reload = true;
    sim.ticks(2);
    sim.intent(c).reload = false;
    assert_eq!(reload_layer(&sim).as_deref(), Some("idle_reload_m3s90_start"));
    // The first shell goes in 0.5 s after the start.
    sim.seconds(0.5);
    assert_eq!(reload_layer(&sim).as_deref(), Some("idle_reload_m3s90_loop"));
    // Two shells: the end 1.425 s after the start (spec T25).
    sim.seconds(1.0);
    assert_eq!(reload_layer(&sim).as_deref(), Some("idle_reload_m3s90_end"));
    // The Elites: the first shot from the left hand, the next the right.
    give(sim.app.world_mut(), c, ELITE).unwrap();
    sim.seconds(1.5);
    let mut sides = Vec::new();
    for _ in 0..2 {
        sim.intent(c).fire = true;
        sim.ticks(2);
        sim.intent(c).fire = false;
        sides.push(fire_layer(&sim));
        sim.seconds(0.3);
    }
    assert_eq!(
        sides,
        [Some("idle_shoot_elites_l".to_string()), Some("idle_shoot_elites_r".to_string())]
    );
}

/// Characters' animation follows their movement (specs/cs_source/
/// animation.md §12): idle, run, walk and crouch-walk lower bodies, the
/// held weapon's upper body, and feet that lag the view.
#[test]
fn bodies_animate_with_movement() {
    use mashup::{
        games::cs_source::{
            movement::{self, SourceMovementPlugin},
            player_anim::{PlayerAnim, PlayerAnimPlugin},
            weapons::CsWeaponsPlugin,
        },
        map::anim::Animator,
    };
    let Some(map) = dust2() else { return };
    assert!(map.characters[0].animations.is_some(), "player animations");
    let mut sim = Sim::new((
        MapPlugin::new(map.clone()),
        SourceMovementPlugin,
        CsWeaponsPlugin,
        PlayerAnimPlugin,
    ));
    sim.set_tick_interval(mashup::games::cs_source::TICK_INTERVAL);
    sim.app
        .insert_resource(mashup::slots::Loadout { movement: movement::ID });
    // The T spawn runway, facing west along it.
    let spawn = movement::to_engine(Vec3::new(-1024.0, -784.0, 140.0));
    let c = sim.spawn_character(spawn + Vec3::Y * (36.0 * 0.0254 + 0.2), movement::ID);
    sim.intent(c).yaw = 90f32.to_radians();
    // Characters get the map's character animations.
    sim.seconds(0.5);
    let set = sim.app.world().get::<Animator>(c).expect("an animator").set.clone();
    let head = |sim: &Sim| {
        let boxes = &sim.app.world().get::<mashup::core::Hitboxes>(c).unwrap().0;
        boxes
            .iter()
            .filter(|h| h.group == mashup::core::Hitgroup::Head)
            .map(|h| h.center.y)
            .fold(f32::MIN, f32::max)
    };
    let standing = head(&sim);
    let main = |sim: &Sim| {
        let a = sim.app.world().get::<Animator>(c).unwrap();
        set.sequences[a.main.expect("a main sequence")].name.clone()
    };
    let layer = |sim: &Sim, k: usize| {
        let a = sim.app.world().get::<Animator>(c).unwrap();
        a.layers
            .get(k)
            .copied()
            .flatten()
            .map(|l| (set.sequences[l.sequence].name.clone(), l.weight))
    };
    assert_eq!(main(&sim), "Idle_lower");
    // It holds its weapon's world model.
    assert_eq!(
        sim.app.world().get::<mashup::map::Held>(c).map(|h| h.0.clone()),
        Some(Some(mashup::games::cs_source::weapons::AK47.to_string()))
    );
    let keys: Vec<&str> = map.held.iter().map(|h| h.key.as_str()).collect();
    let all = mashup::games::cs_source::weapons::WORLD_MODELS.len()
        + mashup::games::cs_source::weapons::SILENCED_WORLD_MODELS.len()
        + mashup::games::cs_source::grenades::WORLD_MODELS.len()
        + mashup::games::cs_source::objectives::WORLD_MODELS.len();
    assert_eq!(keys.len(), all, "held models {keys:?}");
    // The drawn AK-47 picks the AK upper body at full weight.
    assert_eq!(layer(&sim, 0), Some(("Idle_Upper_AK".into(), 1.0)));
    // A reload plays the AK's reload gesture over it, then ends.
    let ak = sim.app.world().get::<mashup::weapon::Inventory>(c).unwrap().active.unwrap();
    sim.app.world_mut().write_message(mashup::weapon::WeaponEvent {
        owner: c,
        weapon: ak,
        kind: mashup::weapon::WeaponEventKind::ReloadStarted,
    });
    sim.ticks(2);
    let reload = layer(&sim, 5).map(|(n, _)| n.to_lowercase());
    assert_eq!(reload.as_deref(), Some("idle_reload_ak"));
    sim.seconds(4.0);
    assert_eq!(layer(&sim, 5), None, "the reload gesture ends");

    sim.intent(c).move_axis = Vec2::Y;
    sim.seconds(1.0);
    let speed = sim.velocity(c).length() / 0.0254;
    assert!(speed > 175.0, "running at {speed} u/s");
    assert_eq!(main(&sim), "Run_lower");
    let (name, w) = layer(&sim, 2).expect("moving upper body");
    assert_eq!(name, "Run_Upper_AK");
    assert!(w > 0.8, "run upper weight {w}");
    // Running straight ahead: move_x ~1 (stored ~1), move_y 0 (stored 0.5).
    let a = sim.app.world().get::<Animator>(c).unwrap();
    let (mx, my) = (set.param("move_x").unwrap(), set.param("move_y").unwrap());
    assert!(
        a.params[mx] > 0.9 && (a.params[my] - 0.5).abs() < 0.02,
        "{:?}",
        a.params
    );

    sim.intent(c).walk = true;
    sim.seconds(1.0);
    assert_eq!(main(&sim), "walk_lower");
    sim.intent(c).walk = false;
    sim.intent(c).crouch = true;
    sim.seconds(1.0);
    assert_eq!(main(&sim), "Crouch_walk_lower");
    sim.intent(c).move_axis = Vec2::ZERO;
    sim.seconds(1.0);
    assert_eq!(main(&sim), "Crouch_Idle_Lower");
    // Hitboxes follow the pose: the head drops when crouching.
    let crouched = head(&sim);
    assert!(
        standing > 1.5 && crouched < standing - 0.3,
        "head at {standing} standing, {crouched} crouched"
    );
    sim.intent(c).crouch = false;
    sim.seconds(1.0);

    // Long after the last feet turn, a turn in place brings the feet
    // along; within 3 s of it a 60 degree turn only twists the torso.
    let yaw = sim.intent(c).yaw + 30f32.to_radians();
    sim.intent(c).yaw = yaw;
    sim.seconds(0.5);
    let feet = sim.app.world().get::<PlayerAnim>(c).unwrap().feet_yaw;
    assert!((feet - 120.0).abs() < 0.01, "feet at {feet}");
    sim.intent(c).yaw = yaw + 60f32.to_radians();
    sim.seconds(0.2);
    let state = sim.app.world().get::<PlayerAnim>(c).unwrap();
    assert!(
        (state.feet_yaw - feet).abs() < 0.01,
        "feet turned {} -> {}",
        feet,
        state.feet_yaw
    );
    let a = sim.app.world().get::<Animator>(c).unwrap();
    let body_yaw = set.param("body_yaw").unwrap();
    assert!((a.params[body_yaw] - set.params[body_yaw].encode(60.0)).abs() < 0.01);
    // After 3 s standing still they face the eyes again.
    sim.seconds(3.5);
    let state = sim.app.world().get::<PlayerAnim>(c).unwrap();
    assert!(
        (state.feet_yaw - (feet + 60.0)).abs() < 0.5,
        "feet at {}",
        state.feet_yaw
    );
}

/// The walls below T spawn by top of mid stand on floor displacements that
/// share their bottom plane; they must still be solid (a displacement's own
/// brush collides only as its surface).
#[test]
fn walls_on_displacements_are_solid() {
    use mashup::games::cs_source::movement::{self, SourceMovementPlugin};
    let Some(map) = dust2() else { return };
    let mut sim = Sim::new((MapPlugin::new(map.clone()), SourceMovementPlugin));
    sim.set_tick_interval(mashup::games::cs_source::TICK_INTERVAL);
    sim.app
        .insert_resource(mashup::slots::Loadout { movement: movement::ID });
    // Between the two walls (x -512 .. -384), clear of the junk props.
    let start = movement::to_engine(Vec3::new(-430.0, -120.0, 40.0));
    let p = sim.spawn_character(start + Vec3::Y * (36.0 * 0.0254 + 0.2), movement::ID);
    sim.seconds(0.5);
    let x = |sim: &Sim| sim.position(p).x / 0.0254;
    // Intent yaw 0 is Source yaw 90; east is Source yaw 0.
    sim.intent(p).yaw = (-90f32).to_radians();
    sim.intent(p).move_axis = Vec2::Y;
    sim.seconds(1.5);
    assert!(x(&sim) < -384.0 - 15.0, "walked into the east wall to x {}", x(&sim));
    sim.intent(p).yaw = 90f32.to_radians();
    sim.seconds(1.5);
    assert!(x(&sim) > -512.0 + 15.0, "walked into the west wall to x {}", x(&sim));
}

/// Falling out of the map kills; a character in god mode goes back to a
/// spawn point instead.
#[test]
fn falling_out_of_the_map() {
    use mashup::{
        core::{God, Health},
        games::cs_source::movement::{self, SourceMovementPlugin},
        map::KillHeight,
    };
    let Some(map) = dust2() else { return };
    let mut sim = Sim::new((MapPlugin::new(map.clone()), SourceMovementPlugin));
    sim.set_tick_interval(mashup::games::cs_source::TICK_INTERVAL);
    let kill = sim.app.world().resource::<KillHeight>().0;
    let (lo, _) = map.playable.expect("dust2 has a playable area");
    assert!(kill < lo.y && kill > lo.y - 10.0, "kill height {kill}, floor {}", lo.y);
    // Off the edge of the world: fall from just above the kill height.
    let out = Vec3::new(500.0, kill + 0.5, 500.0);
    let mortal = sim.spawn_character(out, movement::ID);
    let god = sim.spawn_character(out + Vec3::X * 2.0, movement::ID);
    sim.app.world_mut().entity_mut(god).insert(God);
    sim.seconds(1.0);
    let health = sim.app.world().get::<Health>(mortal).unwrap().current;
    assert_eq!(health, 0.0, "survived the fall");
    let at = sim.position(god);
    assert!(
        at.y > lo.y && map.spawns.iter().any(|(feet, _)| feet.distance(at) < 3.0),
        "god mode at {at}"
    );
}

/// Players are boxes to each other (as in Source): one can land and stand
/// on another's head, walk around up there and step off, never stuck.
#[test]
fn standing_on_another_player() {
    use mashup::games::cs_source::movement::{self, SourceMovementPlugin};
    let Some(map) = dust2() else { return };
    let mut sim = Sim::new((MapPlugin::new(map.clone()), SourceMovementPlugin));
    sim.set_tick_interval(mashup::games::cs_source::TICK_INTERVAL);
    sim.app
        .insert_resource(mashup::slots::Loadout { movement: movement::ID });
    let ground = movement::to_engine(Vec3::new(-1024.0, -784.0, 140.0));
    let lift = Vec3::Y * (36.0 * 0.0254 + 0.2);
    let below = sim.spawn_character(ground + lift, movement::ID);
    sim.seconds(0.5);
    let floor = sim.position(below).y;
    // Dropped from above, slightly off centre.
    let above = sim.spawn_character(sim.position(below) + Vec3::new(0.15, 2.5, 0.1), movement::ID);
    sim.seconds(1.5);
    let on_head = sim.position(above).y - floor;
    assert!(sim.state(above).on_ground, "not standing on the head");
    // Standing on a CS:S hull (62 units): origin 62 units higher.
    assert!(
        (on_head / 0.0254 - 62.0).abs() < 2.0,
        "standing {} units up",
        on_head / 0.0254
    );
    // The one below walks and turns about; the rider stays up and free.
    for (k, yaw) in [0.0f32, 120.0, 240.0, 30.0].into_iter().enumerate() {
        sim.intent(below).yaw = yaw.to_radians();
        sim.intent(below).move_axis = Vec2::Y;
        sim.seconds(0.4);
        sim.intent(below).move_axis = Vec2::ZERO;
        sim.seconds(0.2);
        let start = sim.position(above);
        sim.intent(above).yaw = (yaw + 90.0).to_radians();
        sim.intent(above).move_axis = Vec2::Y;
        sim.seconds(0.05);
        sim.intent(above).move_axis = Vec2::ZERO;
        sim.seconds(0.1);
        let moved = (sim.position(above) - start).with_y(0.0).length();
        assert!(moved > 0.01, "stuck while the one below moved (step {k})");
    }
    // Walk about up there, then off the edge.
    for (k, yaw) in [0.0f32, 90.0, 180.0, 270.0].into_iter().enumerate() {
        let start = sim.position(above);
        sim.intent(above).yaw = yaw.to_radians();
        sim.intent(above).move_axis = Vec2::Y;
        sim.seconds(0.05);
        sim.intent(above).move_axis = Vec2::ZERO;
        sim.seconds(0.3);
        let moved = (sim.position(above) - start).with_y(0.0).length();
        assert!(moved > 0.01, "stuck on the head (step {k})");
    }
    sim.intent(above).move_axis = Vec2::Y;
    sim.seconds(1.0);
    let off = sim.position(above);
    assert!((off.y - floor).abs() < 0.05, "didn't get down: {} m up", off.y - floor);
    assert!(off.with_y(0.0).distance(sim.position(below).with_y(0.0)) > 0.5);
}

/// Fuzz: riders dropped on a moving player at random offsets, steering
/// randomly and jumping; holding a move key on the open runway must always
/// move them.
#[test]
fn riding_players_never_sticks() {
    use mashup::games::cs_source::movement::{self, SourceMovementPlugin};
    let Some(map) = dust2() else { return };
    let mut rng = 0x2545f4914f6cdd1du64;
    let mut rand = move || {
        rng ^= rng << 13;
        rng ^= rng >> 7;
        rng ^= rng << 17;
        (rng >> 11) as f32 / (1u64 << 53) as f32
    };
    let mut stuck = Vec::new();
    for trial in 0..12 {
        let mut sim = Sim::new((MapPlugin::new(map.clone()), SourceMovementPlugin));
        sim.set_tick_interval(mashup::games::cs_source::TICK_INTERVAL);
        sim.app
            .insert_resource(mashup::slots::Loadout { movement: movement::ID });
        let ground = movement::to_engine(Vec3::new(-1024.0, -784.0, 140.0));
        let below = sim.spawn_character(ground + Vec3::Y * (36.0 * 0.0254 + 0.2), movement::ID);
        sim.seconds(0.3);
        let offset = Vec3::new(rand() - 0.5, 2.0 + rand(), rand() - 0.5) * Vec3::new(0.9, 1.0, 0.9);
        let above = sim.spawn_character(sim.position(below) + offset, movement::ID);
        for step in 0..30 {
            sim.intent(below).yaw = rand() * 6.28;
            sim.intent(below).move_axis = if rand() < 0.6 { Vec2::Y } else { Vec2::ZERO };
            sim.intent(below).crouch = rand() < 0.2;
            sim.intent(below).jump = rand() < 0.1;
            sim.intent(above).yaw = rand() * 6.28;
            sim.intent(above).jump = rand() < 0.3;
            sim.intent(above).crouch = rand() < 0.2;
            sim.intent(above).move_axis = Vec2::Y;
            let start = sim.position(above);
            sim.seconds(0.3);
            // Away from the other player it may just be against a wall.
            let near = (sim.position(above) - sim.position(below)).with_y(0.0).length() < 1.5;
            if near && (sim.position(above) - start).with_y(0.0).length() < 0.02 {
                let d = (sim.position(above) - sim.position(below)) / 0.0254;
                eprintln!(
                    "stuck trial {trial} step {step}: rider - other {d:?}, rider ground {} crouch {}, other crouch {}, rider yaw {:.0} other yaw {:.0} other moving {}",
                    sim.state(above).on_ground,
                    sim.state(above).crouching,
                    sim.state(below).crouching,
                    sim.intent(above).yaw.to_degrees(),
                    sim.intent(below).yaw.to_degrees(),
                    sim.intent(below).move_axis.length()
                );
                stuck.push((trial, step, offset));
            }
        }
    }
    assert!(
        stuck.is_empty(),
        "{} stuck moments: {:?}",
        stuck.len(),
        &stuck[..stuck.len().min(5)]
    );
}

/// Fuzz: characters walk, run, crouch-walk and jump on random headings
/// over dust2's terrain (displacements). Holding a move key on the ground,
/// away from brushes, props and other characters, the horizontal speed
/// never collapses in one tick (the "stubbed toe").
#[test]
fn walking_on_terrain_never_stubs() {
    use mashup::{
        core::{MapBrushes, Velocity},
        games::cs_source::movement::{self, SourceMovement, SourceMovementPlugin},
    };
    let Some(map) = dust2() else { return };
    let (lo, hi) = map.playable.expect("playable area");
    let mut rng = 0x9e3779b97f4a7c15u64;
    let mut rand = move || {
        rng ^= rng << 13;
        rng ^= rng >> 7;
        rng ^= rng << 17;
        (rng >> 11) as f32 / (1u64 << 53) as f32
    };
    // Walkable terrain triangles (engine space), inside the playable area.
    let tris: Vec<Vec3> = map
        .collision_indices
        .iter()
        .filter_map(|t| {
            let [a, b, c] = t.map(|i| Vec3::from(map.collision_positions[i as usize]));
            let n = (b - a).cross(c - a).normalize_or_zero();
            let centre = (a + b + c) / 3.0;
            (n.y > 0.8 && centre.cmpgt(lo).all() && centre.cmplt(hi).all()).then_some(centre)
        })
        .collect();
    assert!(tris.len() > 1000, "{} terrain triangles", tris.len());
    let mut sim = Sim::new((MapPlugin::new(map.clone()), SourceMovementPlugin));
    sim.set_tick_interval(mashup::games::cs_source::TICK_INTERVAL);
    sim.app
        .insert_resource(mashup::slots::Loadout { movement: movement::ID });
    let brushes = sim.app.world().resource::<MapBrushes>().0.clone();
    const N: usize = 12;
    let players: Vec<Entity> = (0..N)
        .map(|k| sim.spawn_character(tris[k * 97] + Vec3::Y, movement::ID))
        .collect();
    let feet = |sim: &Sim, p: Entity| movement::to_source(sim.position(p)) - Vec3::Z * 36.0;
    let mut stubs = Vec::new();
    let rounds = 8;
    for round in 0..rounds {
        // Place everyone on a random terrain spot, well apart.
        let mut placed: Vec<Vec3> = Vec::new();
        for &p in &players {
            let spot = loop {
                let c = tris[(rand() * tris.len() as f32) as usize % tris.len()];
                if placed.iter().all(|q| q.distance(c) > 4.0) {
                    break c;
                }
            };
            placed.push(spot);
            let w = sim.app.world_mut();
            w.get_mut::<Transform>(p).unwrap().translation = spot + Vec3::Y * (36.0 * 0.0254 + 0.05);
            w.get_mut::<Velocity>(p).unwrap().0 = Vec3::ZERO;
        }
        sim.seconds(0.3);
        let mut prev: Vec<(bool, f32)> = vec![(false, 0.0); N];
        for tick in 0..200 {
            if tick % 20 == 0 {
                for &p in &players {
                    let mut i = sim.intent(p);
                    i.yaw = rand() * std::f32::consts::TAU;
                    i.move_axis = match (rand() * 4.0) as u32 {
                        0 => Vec2::Y,
                        1 => Vec2::new(1.0, 1.0),
                        2 => Vec2::X,
                        _ => Vec2::new(-1.0, 1.0),
                    };
                    let r = rand();
                    i.walk = r < 0.2;
                    i.crouch = (0.2..0.4).contains(&r);
                }
            }
            for &p in &players {
                let jump = rand() < 0.02;
                sim.intent(p).jump = jump;
                sim.intent(p).yaw += (rand() - 0.5) * 0.1;
            }
            let before: Vec<Vec3> = players.iter().map(|&p| feet(&sim, p)).collect();
            let states: Vec<(SourceMovement, Vec3, mashup::core::Intent)> = players
                .iter()
                .map(|&p| {
                    let w = sim.app.world();
                    (
                        w.get::<SourceMovement>(p).unwrap().clone(),
                        sim.velocity(p),
                        w.get::<mashup::core::Intent>(p).unwrap().clone(),
                    )
                })
                .collect();
            sim.ticks(1);
            for (k, &p) in players.iter().enumerate() {
                let me = sim.app.world().get::<SourceMovement>(p).unwrap().clone();
                let speed = sim.velocity(p).xz().length() / 0.0254;
                let (was_ground, was_speed) = prev[k];
                prev[k] = (me.on_ground, speed);
                if !(was_ground && me.on_ground && was_speed > 60.0 && speed < was_speed * 0.5) {
                    continue;
                }
                // Something solid nearby (brushes, other characters) may
                // stop it for real.
                let f = feet(&sim, p);
                let (a, b) = (
                    movement::to_engine(f + Vec3::new(-24.0, -24.0, 1.0)),
                    movement::to_engine(f + Vec3::new(24.0, 24.0, 70.0)),
                );
                let (bl, bh) = (a.min(b), a.max(b));
                let near_brush = brushes
                    .iter()
                    .any(|br| br.max.cmpgt(bl).all() && br.min.cmplt(bh).all());
                let near_player = players
                    .iter()
                    .any(|&q| q != p && (feet(&sim, q) - f).truncate().length() < 64.0);
                if near_brush || near_player {
                    continue;
                }
                let intent = sim.intent(p).clone();
                eprintln!(
                    "stub round {round} tick {tick}: feet {:?} -> {:?}, speed {was_speed:.1} -> {speed:.1}, yaw {:.1}, axis {:?}, walk {} crouch {} ducked {} jump {} normal {:?}",
                    before[k],
                    f,
                    intent.yaw.to_degrees(),
                    intent.move_axis,
                    intent.walk,
                    intent.crouch,
                    me.ducked,
                    intent.jump,
                    me.ground_normal
                );
                eprintln!(
                    "  state before: v {:?} (source {:?}) {:?} intent {:?}",
                    states[k].1,
                    movement::to_source(states[k].1),
                    states[k].0,
                    states[k].2
                );
                stubs.push((round, tick, before[k]));
            }
        }
    }
    assert!(
        stubs.is_empty(),
        "{} stubs: {:?}",
        stubs.len(),
        &stubs[..stubs.len().min(5)]
    );
}

#[test]
fn view_models_load_with_their_sequences() {
    use mashup::games::cs_source::weapons::{AK47, KNIFE};
    let Some(map) = dust2() else { return };
    let ak = map
        .view_models
        .iter()
        .find(|v| v.key == AK47)
        .expect("AK-47 view model");
    assert_eq!(ak.bones.len(), 63, "hands and weapon");
    let tris: usize = ak.model.meshes.iter().map(|m| m.indices.len() / 3).sum();
    assert!(tris > 1000, "{tris} triangles");
    assert!(
        ak.model.meshes.iter().all(|m| m.joints.len() == m.positions.len()),
        "skinned"
    );
    let set = ak.animations.as_ref().expect("sequences");
    let fire: Vec<&str> = set
        .activities("ACT_VM_PRIMARYATTACK")
        .iter()
        .map(|(s, _)| set.sequences[*s].name.as_str())
        .collect();
    assert_eq!(fire, ["ak47_fire1", "ak47_fire2", "ak47_fire3"]);
    // Durations from the spec's view-model table.
    let dur = |act: &str| set.duration(set.activity(act).expect(act));
    assert!((dur("ACT_VM_DRAW") - 1.0).abs() < 1e-4);
    assert!((dur("ACT_VM_RELOAD") - 2.4324).abs() < 1e-4);
    assert!((dur("ACT_VM_PRIMARYATTACK") - 0.75).abs() < 1e-4);
    assert!(set.sequences[set.activity("ACT_VM_IDLE").unwrap()].looping);
    // Left-handed in the file (drawn mirrored into the right hand with
    // cl_righthand 1); the muzzle and ejection-port attachments and the
    // lighting origin as the spec lists them (view_models.md 6, 9).
    assert!(!ak.right_handed && ak.allow_flipping);
    let names: Vec<&str> = ak.attachments.iter().map(|a| a.name.as_str()).collect();
    assert_eq!(names, ["1", "2"]);
    assert!(ak.attachments[0].local.translation.distance(Vec3::new(0.0, 3.5, 19.0)) < 1e-3);
    assert!(
        ak.light_origin.distance(Vec3::new(10.60, 3.87, -6.35)) < 0.01,
        "{}",
        ak.light_origin
    );
    // L1: placed at the eye, the lighting point is 10.6 units ahead, 3.9
    // left and 6.35 below (camera axes: X right, Y up, -Z forward).
    let eye_space = ak.root.transform_point(ak.light_origin);
    let expect = Vec3::new(-3.87, -6.35, -10.60) * 0.0254;
    assert!(eye_space.distance(expect) < 0.001, "{eye_space} vs {expect}");
    // The map's light field lights it: somewhere at a spawn's eye height
    // there is light.
    let field = map.light_field.as_ref().expect("a light field");
    let at = map.spawns[0].0 + Vec3::Y * 1.6;
    let probe = (field.0)(at);
    let total: f32 =
        probe.cube.iter().map(|c| c.length()).sum::<f32>() + probe.lights.iter().map(|l| l.1.length()).sum::<f32>();
    assert!(total > 0.05, "no light at {at}: {probe:?}");
    let fire = set.sequence("ak47_fire1").unwrap();
    let events = &set.sequences[fire].events;
    assert!(
        events
            .iter()
            .any(|e| e.event == 5001 && e.options == "1" && e.cycle == 0.0)
    );
    assert!(
        events
            .iter()
            .any(|e| e.name == "AE_CLIENT_EFFECT_ATTACH" && e.options == "EjectBrass_762Nato 2 150")
    );
    // Draw ends where idle starts (the decoder reads both alike).
    let params = set.default_params();
    let pose = |s: &str, cycle: f32| {
        let mut p = set.defaults.clone();
        set.accumulate(&mut p, set.sequence(s).unwrap(), cycle, 1.0, &params);
        p
    };
    let (end, start) = (pose("ak47_draw", 1.0), pose("ak47_idle", 0.0));
    for (a, b) in end.iter().zip(&start) {
        assert!(a.0.angle_between(b.0) < 0.01 && a.1.distance(b.1) < 0.01);
    }
    let knife = map
        .view_models
        .iter()
        .find(|v| v.key == KNIFE)
        .expect("knife view model");
    let set = knife.animations.as_ref().unwrap();
    for name in ["draw", "idle", "midslash1", "midslash2", "stab", "stab_miss"] {
        assert!(set.sequence(name).is_some(), "{name}");
    }
}

/// CS:S's own HUD look loads from the install: panel layout, the HUD
/// number and icon fonts, icon glyphs and the scheme's colours.
#[test]
fn game_hud_loads() {
    use mashup::map::hud::HudCoord;
    let Some(map) = dust2() else { return };
    let hud = map.hud.as_ref().expect("a HUD");
    let health = &hud.panels["HudHealth"];
    assert_eq!((health.x, health.y), (HudCoord::Start(8.0), HudCoord::Start(446.0)));
    assert_eq!(hud.panels["HudAmmo"].x, HudCoord::End(157.0));
    for f in ["HudNumbers", "Icons", "CSTypeDeath"] {
        assert!(hud.fonts.get(f).is_some_and(|f| f.data.len() > 1000), "font {f}");
    }
    assert_eq!(hud.icons["health_icon"], ("Icons".to_string(), 'b'));
    assert_eq!(hud.icons["d_ak47"], ("CSTypeDeath".to_string(), 'b'));
    assert_eq!(hud.colors["FgColor"], [255, 176, 0, 255]);
}

/// Which side of the eye each view model holds its weapon in the file
/// (Source model space: +y is left), in its idle pose. Explains the
/// handedness table in `weapons::VIEW_MODELS` (spec view_models.md 2).
#[test]
fn view_model_handedness_in_the_files() {
    use mashup::games::cs_source::weapons::{AK47, KNIFE};
    let Some(map) = dust2() else { return };
    let side = |key: &str, bone: &str| {
        let v = map.view_models.iter().find(|v| v.key == key).unwrap();
        let set = v.animations.as_ref().unwrap();
        let mut pose = set.defaults.clone();
        let params = set.default_params();
        set.accumulate(&mut pose, set.activity("ACT_VM_IDLE").unwrap(), 0.5, 1.0, &params);
        let mut global: Vec<(Quat, Vec3)> = Vec::new();
        for (b, (q, p)) in v.bones.iter().zip(&pose) {
            global.push(match b.parent.map(|i| global[i]) {
                Some((pq, pp)) => (pq * *q, pp + pq * *p),
                None => (*q, *p),
            });
        }
        let i = v.bones.iter().position(|b| b.name.eq_ignore_ascii_case(bone)).unwrap();
        eprintln!("{key} {bone}: {:?}", global[i].1);
        for a in &v.attachments {
            let (q, p) = global[a.bone];
            eprintln!(
                "  attachment {} at {} x {}",
                a.name,
                p + q * a.local.translation,
                q * (a.local.rotation * Vec3::X)
            );
        }
        global[i].1
    };
    // The AK sits left of the eye (built left-handed), the knife right
    // (built right-handed).
    assert!(side(AK47, "v_weapon.AK47_Parent").y > 0.0);
    assert!(side(KNIFE, "v_weapon.knife_Parent").y < 0.0);
    side(KNIFE, "v_weapon.Right_Hand");
    side(KNIFE, "v_weapon.Left_Hand");
    side(AK47, "v_weapon.Right_Hand");
    side(AK47, "v_weapon.Left_Hand");
}

#[test]
fn held_ak_has_a_muzzle_ahead_of_the_grip() {
    use mashup::games::cs_source::weapons::AK47;
    let Some(map) = dust2() else { return };
    let ak = map.held.iter().find(|h| h.key == AK47).expect("held AK");
    let m = ak.muzzle.expect("muzzle attachment");
    let fwd = m.rotation * Vec3::X;
    eprintln!("muzzle at {} forward {fwd}", m.translation);
    // The muzzle is well away from the grip, along its own forward axis.
    assert!(m.translation.length() > 15.0, "{}", m.translation);
    assert!(m.translation.normalize().dot(fwd) > 0.8, "{} vs {fwd}", m.translation);
}

/// Impact effects (specs/cs_source/impact_effects.md): surface letters from
/// the CS:S data, the effect materials and their sprite sheets.
#[test]
fn impact_effect_materials_and_letters() {
    use mashup::games::cs_source::impact_effects::{Effect, MATERIALS, effect_for};
    let Some(map) = dust2() else { return };
    let letter = |s: &str| map.sounds.surfaces.get(s).map(|m| m.game_material);
    for (surface, want) in [
        ("brick", 'C'),
        ("sand", 'D'),
        ("metalvent", 'V'),
        ("metalgrate", 'G'),
        ("glass", 'Y'),
        ("armorflesh", 'M'),
        ("flesh", 'F'),
    ] {
        assert_eq!(letter(surface), Some(want), "{surface}");
    }
    assert_eq!(effect_for('V'), Effect::MetalSparks);
    let p = &map.particles;
    for name in MATERIALS {
        let m = p
            .materials
            .iter()
            .find(|m| m.name == *name)
            .unwrap_or_else(|| panic!("{name} missing"));
        assert!(m.texture.is_some(), "{name}: no texture");
    }
    // Sprite sheets: the blood smoke has 16 sequences, the goop at least 13.
    let sheet = |n: &str| &p.materials[p.find(n).unwrap()].sequences;
    assert!(sheet("particle/smoke1/smoke1_nearcull2").len() >= 16);
    let goop = sheet("particle/antlion_goop3/antlion_goop3");
    assert!(goop.len() >= 13, "{} goop sequences", goop.len());
    for frame in goop.iter().flatten() {
        assert!(frame.iter().all(|v| (0.0..=1.0).contains(v)), "{frame:?}");
    }
    // Wood flecks ignore vertex alpha (they pop out, spec Q10); cement fade.
    let mat = |n: &str| &p.materials[p.find(n).unwrap()];
    assert!(!mat("effects/fleck_wood1").vertex_alpha);
    assert!(mat("effects/fleck_cement1").vertex_alpha);
    assert_eq!(
        mat("effects/spark").blend,
        mashup::map::particles::ParticleBlend::Additive
    );
}

/// Cases `walking_on_terrain_never_stubs` found (the "stubbed toe"): running
/// across nearly flat terrain, the sweep met a triangle edge with a sideways
/// normal and lost most of its speed in one tick; on sloped terrain the box
/// ended up inside the terrain and stopped dead.
#[test]
fn running_over_terrain_keeps_speed() {
    use mashup::games::cs_source::movement::{self, SourceMovementPlugin};
    let Some(map) = dust2() else { return };
    // Feet (Source units), intent yaw (radians), move keys.
    let cases = [
        (Vec3::new(1456.79, 1596.71, 1.61), 0.8531, Vec2::new(1.0, 1.0)),
        (Vec3::new(1368.92, 1667.37, 2.81), 2.0684, Vec2::new(1.0, 1.0)),
        (Vec3::new(-1804.60, 115.16, 1.95), 2.5284, Vec2::new(1.0, 1.0)),
        (Vec3::new(-113.39, 3144.78, 286.54), 3.0424, Vec2::new(-1.0, 1.0)),
        (Vec3::new(-2047.10, 3622.04, 183.98), 3.5225, Vec2::Y),
    ];
    for (k, (feet, yaw, keys)) in cases.into_iter().enumerate() {
        let mut sim = Sim::new((MapPlugin::new(map.clone()), SourceMovementPlugin));
        sim.set_tick_interval(cs_source::TICK_INTERVAL);
        // Dropped from above: some of these feet were inside the terrain.
        let p = sim.spawn_character(movement::to_engine(feet + Vec3::Z * 80.0), movement::ID);
        sim.seconds(0.6);
        assert!(sim.state(p).on_ground, "case {k}: not on the ground");
        sim.intent(p).yaw = yaw;
        sim.intent(p).move_axis = keys;
        let mut last = 0.0f32;
        // About half a second: the second case reaches a wall after that.
        for tick in 0..35 {
            sim.ticks(1);
            let speed = sim.velocity(p).xz().length() / 0.0254;
            assert!(
                !(last > 100.0 && speed < last * 0.8),
                "case {k} tick {tick}: speed {last:.1} -> {speed:.1} at {}",
                movement::to_source(sim.position(p))
            );
            last = speed;
        }
        assert!(last > 200.0, "case {k}: running at {last:.1}");
    }
}

/// Every gun's models and death-notice icon load from the install, and
/// each view model's muzzle sits on the side of the eye its
/// `VIEW_MODELS` handedness says (Source model space: +y is left, so a
/// muzzle left of the eye means built left-handed).
#[test]
fn gun_models_icons_and_handedness() {
    use mashup::games::cs_source::weapons::{ELITE, GUNS, SILENCED_WORLD_MODELS, VIEW_MODELS, silenced_key};
    let Some(map) = dust2() else { return };
    let hud = map.hud.as_ref().expect("a HUD");
    for (id, _) in SILENCED_WORLD_MODELS {
        assert!(map.held.iter().any(|h| h.key == silenced_key(id)), "{id} silenced world model");
    }
    for g in GUNS {
        let short = g.id.rsplit("weapon_").next().unwrap();
        assert!(hud.icons.contains_key(&format!("d_{short}")), "no d_{short} icon");
        assert!(map.held.iter().any(|h| h.key == g.id), "{} world model", g.id);
        // Every gun has its view model (the AWP's is MDL version 48).
        let v = map
            .view_models
            .iter()
            .find(|v| v.key == g.id)
            .unwrap_or_else(|| panic!("{} view model: {:?}", g.id, map.warnings));
        let set = v.animations.as_ref().expect("sequences");
        let mut pose = set.defaults.clone();
        let params = set.default_params();
        set.accumulate(&mut pose, set.activity("ACT_VM_IDLE").unwrap(), 0.5, 1.0, &params);
        let mut global: Vec<(Quat, Vec3)> = Vec::new();
        for (b, (q, p)) in v.bones.iter().zip(&pose) {
            global.push(match b.parent.map(|i| global[i]) {
                Some((pq, pp)) => (pq * *q, pp + pq * *p),
                None => (*q, *p),
            });
        }
        let a = v.attachments.iter().find(|a| a.name == "1").expect("muzzle attachment");
        let (q, p) = global[a.bone];
        let muzzle = p + q * a.local.translation;
        let right_handed = VIEW_MODELS.iter().find(|m| m.0 == g.id).unwrap().2;
        eprintln!("{}: muzzle {muzzle}", g.id);
        assert_eq!(muzzle.y < 0.0, right_handed, "{}: muzzle at {muzzle}", g.id);
        // Draw and reload durations from the spec's view-model table.
        let dur = |act: &str| set.duration(set.activity(act).expect(act));
        assert!((dur("ACT_VM_DRAW") - g.draw).abs() < 1e-3, "{} draw", g.id);
        // A shotgun's reload activity is one shell's insert; its start and
        // finish have their own.
        assert!((dur("ACT_VM_RELOAD") - g.reload).abs() < 1e-3, "{} reload", g.id);
        if g.shells.is_some() {
            dur("ACT_SHOTGUN_RELOAD_START");
            dur("ACT_SHOTGUN_RELOAD_FINISH");
        }
    }
    // The Elites: left shots the primary activity, right the secondary,
    // each hand's last round its own.
    let elite = map.view_models.iter().find(|v| v.key == ELITE).unwrap();
    let set = elite.animations.as_ref().unwrap();
    for act in ["ACT_VM_PRIMARYATTACK", "ACT_VM_SECONDARYATTACK", "ACT_VM_DRYFIRE", "ACT_VM_DRYFIRE_LEFT"] {
        assert!(set.activity(act).is_some(), "elite {act}");
    }
}

/// The radar picture lines up with the world: walkable places (nav area
/// centres) fall on the overview's lit floor, not its dark background.
#[test]
fn overview_lines_up_with_the_world() {
    let Some(map) = dust2() else { return };
    let o = map.overview.clone().expect("dust2 has an overview");
    let t = &map.textures[o.texture];
    let green = |p: Vec2| -> f32 {
        if p.x < 0.0 || p.y < 0.0 || p.x >= o.size.x || p.y >= o.size.y {
            return 0.0;
        }
        t.rgba8[((p.y as u32 * t.width + p.x as u32) * 4 + 1) as usize] as f32
    };
    let nav = map.nav.as_ref().unwrap();
    let on_floor = nav.areas.iter().map(|a| green(o.pixel(a.center))).sum::<f32>() / nav.areas.len() as f32;
    let image = t.rgba8.chunks(4).map(|p| p[1] as f32).sum::<f32>() / (t.width * t.height) as f32;
    assert!(on_floor > image * 1.4, "nav areas average {on_floor}, image {image}");
}

/// Buy zones (func_buyzone): each team's spawns lie in its own zone and
/// not the other's; mid isn't a buy zone.
#[test]
fn buy_zones_cover_each_teams_spawns() {
    use mashup::{
        map::MapEntities,
        weapon::economy::{Money, in_buy_zone},
    };
    let Some(map) = dust2() else { return };
    let mut w = World::new();
    w.insert_resource(MapEntities {
        entities: std::sync::Arc::new(map.entities.clone()),
        scale: map.entity_scale,
    });
    let zones = map.entities.iter().filter(|e| e.classname() == "func_buyzone").count();
    assert!(zones >= 2, "{zones} buy zones");
    let check = |w: &mut World, at: Vec3, team: u8| {
        let e = w.spawn((Transform::from_translation(at), Team(team), Money(800))).id();
        in_buy_zone(w, e)
    };
    for (at, team) in map.spawns.iter().filter_map(|(p, t)| Some((*p, (*t)?))) {
        assert!(check(&mut w, at, team.0), "team {} spawn {at} in its zone", team.0);
        assert!(!check(&mut w, at, 3 - team.0), "team {} spawn {at} not the other's", team.0);
    }
    // Mid doors, nowhere near a spawn (Source -480, 420, 0).
    let mid = Vec3::new(-480.0, 0.0, -420.0) * map.entity_scale + Vec3::Y;
    assert!(!check(&mut w, mid, 1) && !check(&mut w, mid, 2));
}

/// Bots' objectives: dust2's two bomb sites, on the floor near the
/// overview's A and B (nav places BombsiteA/BombsiteB).
#[test]
fn bot_objectives_are_the_bomb_sites() {
    use mashup::{bot::objectives, map::MapEntities};
    let Some(map) = dust2() else { return };
    let goals = objectives(&MapEntities {
        entities: std::sync::Arc::new(map.entities.clone()),
        scale: map.entity_scale,
    });
    assert_eq!(goals.len(), 2, "{goals:?}");
    let nav = map.nav.as_ref().expect("nav mesh");
    let mut places: Vec<String> = goals
        .iter()
        .filter_map(|g| nav.area_at(*g + Vec3::Y * 0.5).and_then(|a| nav.places.get(nav.areas[a].place?).cloned()))
        .collect();
    places.sort();
    assert_eq!(places, ["BombsiteA", "BombsiteB"], "{goals:?}");
}

/// The grenades' models, sequences and sounds from the install
/// (specs/cs_source/grenades.md): world and view models load, the view
/// models have the pin, throw and draw sequences with the spec's
/// durations and the handedness `grenades::VIEW_MODELS` says, the HE kill
/// icon exists, and the materials and sound entries the effects use.
#[test]
fn grenade_models_and_sequences() {
    use mashup::games::cs_source::grenades::{
        DRAW_TIME, GRENADES, MATERIALS, PIN_TIME, SOUNDS, THROW_TIME, VIEW_MODELS,
    };
    let Some(map) = dust2() else { return };
    let hud = map.hud.as_ref().expect("a HUD");
    assert!(hud.icons.contains_key("d_hegrenade"));
    for (id, ..) in GRENADES {
        assert!(map.held.iter().any(|h| h.key == *id), "{id} world model: {:?}", map.warnings);
        let v = map
            .view_models
            .iter()
            .find(|v| v.key == *id)
            .unwrap_or_else(|| panic!("{id} view model: {:?}", map.warnings));
        let set = v.animations.as_ref().expect("sequences");
        let dur = |act: &str| set.duration(set.activity(act).expect(act));
        assert!((dur("ACT_VM_PULLPIN") - PIN_TIME).abs() < 1e-3, "{id} pull pin {}", dur("ACT_VM_PULLPIN"));
        assert!((dur("ACT_VM_THROW") - THROW_TIME).abs() < 1e-3, "{id} throw {}", dur("ACT_VM_THROW"));
        assert!((dur("ACT_VM_DRAW") - DRAW_TIME).abs() < 1e-3, "{id} draw {}", dur("ACT_VM_DRAW"));
        // Handedness at idle.
        let mut pose = set.defaults.clone();
        let params = set.default_params();
        set.accumulate(&mut pose, set.activity("ACT_VM_IDLE").unwrap(), 0.5, 1.0, &params);
        let mut global: Vec<(Quat, Vec3)> = Vec::new();
        for (b, (q, p)) in v.bones.iter().zip(&pose) {
            global.push(match b.parent.map(|i| global[i]) {
                Some((pq, pp)) => (pq * *q, pp + pq * *p),
                None => (*q, *p),
            });
        }
        // The grenade (its `*_Parent` bone) sits right of the eye in a
        // right-handed model (as the knife's), left in a left-handed one.
        let grenade = v
            .bones
            .iter()
            .position(|b| b.name.to_lowercase().ends_with("_parent") && !b.name.to_lowercase().contains("hands"))
            .expect("a grenade parent bone");
        let right_handed = VIEW_MODELS.iter().find(|m| m.0 == *id).unwrap().2;
        eprintln!("{id}: grenade at {}", global[grenade].1);
        assert_eq!(global[grenade].1.y < 0.0, right_handed, "{id} handedness");
    }
    for m in MATERIALS {
        assert!(map.particles.find(m).is_some(), "particle material {m}");
    }
    for s in SOUNDS {
        assert!(map.sounds.entry(s).is_some(), "sound {s}");
    }
}

/// Soundscape selection and the room DSP (specs/cs_source/sounds.md 6):
/// a listener moving from dust2's outdoors into an indoors (tunnel)
/// trigger_soundscape switches the soundscape, its loops (live sounds)
/// and the room preset its "dsp" names.
#[test]
fn walking_into_a_tunnel_switches_soundscape_and_room() {
    use mashup::map::{
        LiveSounds,
        live_sound::SOUNDSCAPE_KEYS,
        room::{RoomDsp, preset},
        sound::{SoundListener, SoundscapeZone},
        soundscape::ScapeState,
    };
    let Some(map) = dust2() else { return };
    let s = map.sounds.clone();
    let vis = map.visibility.clone().expect("dust2 has visibility");
    let name = |z: &SoundscapeZone| s.soundscapes[z.scape].name.to_lowercase();
    let inside = |z: &SoundscapeZone, p: Vec3| p.cmpge(z.min).all() && p.cmple(z.max).all();
    let outdoors = |p: Vec3| {
        let zones: Vec<_> = s.soundscape_zones.iter().filter(|z| inside(z, p)).collect();
        !zones.is_empty() && zones.iter().all(|z| name(z) == "dust2.outdoors")
    };
    // A way in: from just outside an indoors box's side, in an outdoors
    // zone, to inside it, through open space.
    let mut route = None;
    'zones: for z in s.soundscape_zones.iter().filter(|z| name(z) == "dust2.indoors") {
        let c = (z.min + z.max) * 0.5;
        let half = (z.max - z.min) * 0.5;
        for dir in [Vec3::X, -Vec3::X, Vec3::Z, -Vec3::Z] {
            let across = Vec3::new(dir.z, 0.0, dir.x);
            for lift in [0.6, 1.0, 1.6, 2.5] {
                for slide in [0.0f32, -0.3, 0.3, -0.6, 0.6] {
                    let edge = Vec3::new(c.x, z.min.y + lift, c.z)
                        + across * slide * half.dot(across.abs())
                        + dir * half.dot(dir.abs());
                    let (from, to) = (edge + dir * 3.0, edge - dir * 3.0);
                    if outdoors(from)
                        && inside(z, to)
                        && vis.cluster_at(from).is_some()
                        && vis.cluster_at(to).is_some()
                        && vis.segment_clear(from, to)
                    {
                        route = Some((from, to, z.scape));
                        break 'zones;
                    }
                }
            }
        }
    }
    let (from, to, indoors) = route.expect("a way into an indoors zone");
    let out = s
        .soundscapes
        .iter()
        .position(|x| x.name.eq_ignore_ascii_case("dust2.outdoors"))
        .unwrap();
    eprintln!(
        "route {from} -> {to}; dsp outdoors {:?}, indoors {:?}",
        s.soundscapes[out].dsp, s.soundscapes[indoors].dsp
    );

    let mut sim = Sim::new((MapPlugin::new(map), cs_source::movement::SourceMovementPlugin));
    sim.set_tick_interval(cs_source::TICK_INTERVAL);
    let p = sim.spawn_character(from, mashup::movement::noclip::ID);
    sim.app.world_mut().entity_mut(p).insert(mashup::core::LocalPlayer);
    sim.app.world_mut().spawn((SoundListener, Transform::default(), ChildOf(p)));
    sim.seconds(0.5);
    let scape = |sim: &Sim| sim.app.world().resource::<ScapeState>().current.map(|c| c.0);
    let room = |sim: &Sim| sim.app.world().resource::<RoomDsp>().preset;
    // Soundscape loops playing (keys under SOUNDSCAPE_KEYS).
    let loops = |sim: &Sim| {
        sim.app
            .world()
            .resource::<LiveSounds>()
            .sounds
            .keys()
            .filter(|k| k.0 >> 62 == SOUNDSCAPE_KEYS >> 62)
            .count()
    };
    assert_eq!(scape(&sim), Some(out));
    let outdoor_loops = loops(&sim);
    assert!(outdoor_loops > 0, "the outdoors loops play");
    let before = room(&sim);
    assert_eq!(before, s.soundscapes[out].dsp.unwrap_or(0));

    // Walk in, 0.1 m a tick.
    let steps = (from.distance(to) / 0.1).ceil() as usize;
    for i in 1..=steps {
        let at = from.lerp(to, i as f32 / steps as f32);
        sim.app.world_mut().get_mut::<Transform>(p).unwrap().translation = at;
        sim.ticks(1);
    }
    sim.seconds(0.2);
    assert_eq!(scape(&sim), Some(indoors), "indoors once inside");
    let after = room(&sim);
    eprintln!("room {} -> {}", preset(before).name, preset(after).name);
    assert_eq!(Some(after), s.soundscapes[indoors].dsp);
    assert_ne!(before, after, "the tunnel sounds like another room");
    // The outdoors loops fade out over 3 s and stop; the indoors ones
    // (some reused) play on.
    let crossing = loops(&sim);
    eprintln!("loops: {outdoor_loops} outdoors, {crossing} while crossing");
    assert!(crossing > 0);
    sim.seconds(3.5);
    let left = loops(&sim);
    eprintln!("loops: {outdoor_loops} outdoors, {left} indoors");
    assert!(left > 0 && left <= s.soundscapes[indoors].loops.len());
}
