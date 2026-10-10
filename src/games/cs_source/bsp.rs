//! CS:S maps (BSP v20) to neutral `MapData`. Parsing is the `vbsp` crate;
//! this module owns Source's conventions: Z-up inches to Y-up meters, which
//! surfaces draw and collide, and which entities are spawns.

use std::collections::{BTreeMap, HashMap};

use bevy::prelude::*;
use vbsp::{BrushFlags, Bsp, TextureFlags};

use super::{
    ambient::RawLeaf,
    lightmap::{self, AtlasBuilder},
    material::MaterialLoader,
};
use crate::{
    core::Team,
    map::{MapAlpha, MapData, MapMesh},
    mount::Mount,
};

/// One Hammer unit is one inch.
pub const METERS_PER_UNIT: f32 = 0.0254;

/// Source (x forward, y left, z up) to engine (y up, -z forward), in meters.
/// A rotation, so handedness and winding are preserved.
pub fn to_engine(v: vbsp::Vector) -> Vec3 {
    Vec3::new(v.x, v.z, -v.y) * METERS_PER_UNIT
}

fn to_engine_dir(v: vbsp::Vector) -> Vec3 {
    Vec3::new(v.x, v.z, -v.y).normalize_or_zero()
}

/// Surfaces that are never drawn.
const NOT_DRAWN: TextureFlags = TextureFlags::SKY2D
    .union(TextureFlags::SKY)
    .union(TextureFlags::TRIGGER)
    .union(TextureFlags::HINT)
    .union(TextureFlags::SKIP)
    .union(TextureFlags::NODRAW);
/// Surfaces players never collide with.
const NOT_SOLID: TextureFlags = TextureFlags::SKY2D
    .union(TextureFlags::SKY)
    .union(TextureFlags::TRIGGER)
    .union(TextureFlags::HINT)
    .union(TextureFlags::SKIP);

pub fn load(mount: &Mount, name: &str) -> Result<MapData, String> {
    load_level(mount, name, 0)
}

/// Load at a Source `mat_hdr_level`: 0 LDR (the default, what refcmp
/// matches); 1 LDR lighting with bloom; 2 the map's HDR lighting (lumps
/// 53, 51/55, 54) with auto exposure and bloom. Maps without HDR lighting
/// load as LDR at any level, as in the game. Maps compiled with HDR
/// lighting only (no lump 8: surf_sedona) are fullbright below level 2,
/// as CS:S draws them (`lightmap::select_lighting`).
pub fn load_level(mount: &Mount, name: &str, hdr_level: u8) -> Result<MapData, String> {
    crate::map::loading::report(0.02, "LoadingProgress_LoadMap");
    let path = format!("maps/{name}.bsp");
    let bytes = mount.read(&path).map_err(|e| format!("{path}: {e}"))?;
    load_level_bytes(mount, name, bytes, hdr_level)
}

/// `load_level` from the map file's bytes (a copy downloaded from a
/// server, in the content cache), its content from `mount`.
pub fn load_level_bytes(mount: &Mount, name: &str, bytes: Vec<u8>, hdr_level: u8) -> Result<MapData, String> {
    // Stages for a loading screen, worded as the game words them
    // (`map::loading`; the fractions are rough shares of the time).
    use crate::map::loading::report;
    let mut timer = crate::map::loading::LoadTimer::default();
    report(0.02, "LoadingProgress_LoadMap");
    let path = format!("maps/{name}.bsp");
    timer.lap("read");
    // What the network handshake compares (`MapData::file_hash`).
    let file_hash: [u8; 32] = sha2::Digest::finalize(<sha2::Sha256 as sha2::Digest>::new_with_prefix(&bytes)).into();
    timer.lap("hash");
    // Community maps often ship LZMA-compressed lumps; our own lump
    // readers want them plain.
    let bytes = super::lumps::inflate(bytes);
    timer.lap("inflate lumps");
    // Latin-1 text in the entity or texture-name lumps (after inflating).
    let bytes = text_lumps_utf8(bytes);
    // The HDR lighting at mat_hdr_level 2, or on maps that have only
    // that (lump 7 then holds the HDR faces).
    let (bytes, hdr_lighting) = lightmap::select_lighting(bytes, hdr_level >= 2);
    let bsp = Bsp::read(&bytes).map_err(|e| format!("{path}: {e}"))?;
    timer.lap("parse");
    let hdr_level = if lightmap::has_hdr_lighting(&bytes) { hdr_level.min(2) } else { 0 };
    let (mut data, layout) = convert_level(&bsp, &bytes, name, hdr_lighting);
    timer.lap("geometry, lightmaps, collision");
    data.look = source_look_level(hdr_level, &data.entities);
    data.file_hash = Some(file_hash);

    report(0.15, "LoadingProgress_PrecacheWorld");
    let mut materials = MaterialLoader::new(&bsp, mount, super::ambient::lump(&bytes, 40));
    timer.lap("packed files");
    for mesh in &mut data.meshes {
        let r = materials.resolve(&mesh.material);
        mesh.texture = r.texture;
        mesh.normal_map = r.normal_map;
        mesh.blend = r.blend;
        mesh.detail = r.detail;
        mesh.base_transform = r.base_transform;
        mesh.selfillum = r.selfillum;
        // UnlitGeneric brushes (neon signs) draw at their own brightness
        // times $color; lightmapped ones ignore the tint for now.
        mesh.unlit = r.unlit;
        if r.unlit {
            mesh.tint = r.tint;
        }
        if mesh.blend.is_none() {
            mesh.blend_weights.clear();
        }
        mesh.alpha = r.alpha;
        // An entity's translucent render mode replaces the material's.
        if let Some((blend, _)) = mesh.render {
            mesh.alpha = blend;
        }
        mesh.double_sided = r.double_sided;
        mesh.surface = r.surfaceprop;
        mesh.envmap = r.envmap;
    }
    // Brush entities an env_texturetoggle targets: their textures' frames.
    let toggled: std::collections::HashSet<usize> = {
        let targets: std::collections::HashSet<String> = bsp
            .entities
            .iter()
            .filter(|e| {
                e.prop("classname")
                    .is_some_and(|c| c.eq_ignore_ascii_case("env_texturetoggle"))
            })
            .filter_map(|e| e.prop("target").map(|t| t.trim().to_ascii_lowercase()))
            .collect();
        bsp.entities
            .iter()
            .enumerate()
            .filter(|(_, e)| {
                e.prop("targetname")
                    .is_some_and(|n| targets.contains(&n.trim().to_ascii_lowercase()))
            })
            .map(|(i, _)| i)
            .collect()
    };
    for mesh in &mut data.meshes {
        if mesh.entity.is_none_or(|e| !toggled.contains(&e)) {
            continue;
        }
        if let Some(base) = materials.material_value(&mesh.material, "$basetexture") {
            let frames = materials.texture_frames(&base, true);
            if frames.len() > 1 {
                mesh.frames = frames;
            }
        }
    }
    timer.lap("world materials");
    // Broken windows' cracked and jagged-edge looks.
    super::breakables::add_window_looks(&mut materials, &data.entities, &mut data.meshes);
    // Water surfaces (specs/cs_source/water.md), with the map's cheap
    // distances for their WaterLOD proxies.
    let lod_keys = bsp
        .entities
        .iter()
        .find(|e| e.prop("classname") == Some("water_lod_control"))
        .map(|e| {
            let key = |k: &'static str| e.prop(k).map(str::to_string);
            (key("cheapwaterstartdistance"), key("cheapwaterenddistance"))
        });
    let lod = super::water::lod_distances(lod_keys.as_ref().map(|(a, b)| (a.as_deref(), b.as_deref())));
    for mesh in data.meshes.iter_mut() {
        if let Some(w) = super::water::load(&mut materials, &mesh.material, lod) {
            data.water_materials.push(w);
            mesh.water = Some(data.water_materials.len() - 1);
        }
    }
    // The map's downward water faces carry whatever bottom texture the
    // brush had; the engine draws them with the top material's
    // `$bottommaterial` (de_port: dev_waterbeneath2 faces, coast01_beneath
    // drawn). With one bottom material among the map's tops, use it.
    let mut bottoms: Vec<crate::map::water::MapWaterMaterial> = Vec::new();
    for b in data.water_materials.iter().filter_map(|m| m.bottom.as_deref()) {
        if !bottoms.iter().any(|x| x.name == b.name) {
            bottoms.push(b.clone());
        }
    }
    if let [bottom] = bottoms.as_slice() {
        for m in data.water_materials.iter_mut().filter(|m| !m.above_water) {
            *m = bottom.clone();
        }
    }
    data.sky_vis = Some(sky_vis(&bsp, &bytes));
    timer.lap("water");
    data.visibility = visibility(&bsp, &bytes)
        .map(|mut v| {
            // Glass or grates in a portal's opening keep it open.
            v.areas.mark_see_through(&data.meshes);
            v
        })
        .map(std::sync::Arc::new);
    timer.lap("visibility");
    let lighting = if lightmap::fullbright(&bytes, hdr_lighting) {
        super::ambient::MapLighting::fullbright()
    } else {
        super::ambient::MapLighting::read_level(&bytes, hdr_lighting)
    };
    timer.lap("ambient lighting");
    let occluders = super::ambient::Occluders::new(
        &shadow_hulls(&bsp, &super::ambient::raw_leaves(&bytes)),
        (&data.collision_positions, &data.collision_indices),
    );
    timer.lap("occluders");
    super::props::add_static_props(&bsp, &mut materials, &lighting, &occluders, &mut data, hdr_lighting);
    timer.lap("props");
    super::ropes::add_ropes(&bsp, &mut materials, &lighting, &occluders, &mut data);
    // The same query at run time, for view models (spec view_models.md 9).
    if let Some(tree) = data.sky_vis.clone() {
        data.light_field = Some(crate::map::MapLightField(std::sync::Arc::new(move |p| {
            super::props::probe_with(&|q| lighting.ambient_in(tree.leaf(q), q), &lighting, &occluders, p)
        })));
    }
    super::sprites::add_sprites(&bsp, &mut materials, &mut data);
    super::dust::add_dust(&bsp, &mut materials, &mut data);
    super::steam::add_steam(&bsp, &mut materials, &mut data);
    super::beams::add_beams(&bsp, &mut materials, &mut data);
    let surfaces = super::surfaceprops::SurfaceProps::load(&mut materials);
    brush_bodies(&bytes, &surfaces, &mut data);
    timer.lap("ropes, sprites, dust, steam");
    report(0.55, "LoadingProgress_LoadResources");
    // Character bodies: a terrorist and a counter-terrorist model (CS:S
    // teams 2 and 3; ours are 1 and 2), the CT one for anyone else.
    for (path, team) in [
        ("models/player/t_phoenix.mdl", Some(crate::core::Team(1))),
        ("models/player/ct_urban.mdl", None),
    ] {
        match super::props::load_character(&mut materials, &surfaces, path, team) {
            Ok(c) => data.characters.push(c),
            Err(e) => data.warnings.push(e),
        }
    }
    // Hostages' bodies, on maps with hostages (picked by name).
    if data.entities.iter().any(|e| e.classname().eq_ignore_ascii_case("hostage_entity")) {
        for path in super::objectives::HOSTAGE_MODELS {
            match super::props::load_character(&mut materials, &surfaces, path, None) {
                Ok(mut c) => {
                    c.name = Some(path.to_string());
                    data.characters.push(c);
                }
                Err(e) => data.warnings.push(e),
            }
        }
    }
    data.decals = super::decals::impact_decals(&mut materials);
    data.hud = super::hud::load(&mut materials, name).map(std::sync::Arc::new);
    data.round_sounds = super::sound::round_sounds();
    data.radio = super::radio::load(&materials);
    // Bots' names (the install's botprofile.db).
    data.bot_profiles = materials
        .read("botprofile.db")
        .map(|b| crate::map::bot_profiles::BotProfiles::parse(&String::from_utf8_lossy(&b)))
        .filter(|p| !p.0.is_empty());
    data.overview = super::hud::overview(&mut materials, name);
    data.particles = super::impact_effects::load_materials(&mut materials);
    super::trails::add_trails_and_stacks(&bsp, &mut materials, &mut data);
    super::pcf::add_particle_systems(&bsp, &mut materials, &mut data, name);
    super::props::add_attachment_keys(&materials, &mut data);
    data.camera_texture = materials.camera_texture;
    timer.lap("characters, hud, particles");
    // What characters hold: the weapons' world models.
    if let Some(skeleton) = data.characters.first().map(|c| c.bones.clone()) {
        for (weapon, path) in super::weapons::WORLD_MODELS
            .iter()
            .chain(super::grenades::WORLD_MODELS)
            .chain(super::objectives::WORLD_MODELS)
        {
            match super::props::load_held(&mut materials, path, weapon, &skeleton) {
                Ok(h) => data.held.push(h),
                Err(e) => data.warnings.push(e),
            }
        }
        for (weapon, path) in super::weapons::SILENCED_WORLD_MODELS {
            let key = super::weapons::silenced_key(weapon);
            match super::props::load_held(&mut materials, path, &key, &skeleton) {
                Ok(h) => data.held.push(h),
                Err(e) => data.warnings.push(e),
            }
        }
    }
    // What the local player sees of them: the view models.
    for (weapon, path, right_handed) in super::weapons::VIEW_MODELS
        .iter()
        .chain(super::grenades::VIEW_MODELS)
        .chain(super::objectives::VIEW_MODELS)
    {
        match super::props::load_view_model(&mut materials, path, weapon, *right_handed) {
            Ok(v) => data.view_models.push(v),
            Err(e) => data.warnings.push(e),
        }
    }
    data.muzzle_flash = Some(super::view_anim::muzzle_flash(&mut materials));
    for (key, path, bounce) in super::view_anim::SHELLS {
        match super::props::load_shell(&mut materials, path) {
            Ok(model) => data.shells.push(crate::map::shells::MapShell {
                key: key.to_string(),
                model,
                bounce: Some(bounce.to_string()),
            }),
            Err(e) => data.warnings.push(e),
        }
    }
    data.shell_physics = Some(super::view_anim::shell_physics());
    // Props' break pieces are in already (`props::add_static_props`).
    for set in super::breakables::load_gibs(&mut materials, &data.entities, &mut data.warnings) {
        if !data.gibs.iter().any(|g| g.name.eq_ignore_ascii_case(&set.name)) {
            data.gibs.push(set);
        }
    }
    // env_shooter's models, each a gib list of its own (named by path).
    for e in &data.entities {
        if !e.classname().eq_ignore_ascii_case("env_shooter") {
            continue;
        }
        let Some(path) = e
            .get("shootmodel")
            .map(|p| p.trim().to_ascii_lowercase().replace('\\', "/"))
        else {
            continue;
        };
        if path.is_empty() || data.gibs.iter().any(|g| g.name.eq_ignore_ascii_case(&path)) {
            continue;
        }
        match super::props::load_shell(&mut materials, &path) {
            Ok(m) => data.gibs.push(crate::map::breakables::MapGibSet {
                name: path,
                models: vec![m],
            }),
            Err(e) => data.warnings.push(e),
        }
    }
    data.gib_physics = Some(super::breakables::gib_physics());
    timer.lap("held and view models, shells, gibs");
    report(0.8, "LoadingProgress_SignonDataLocal");
    let mut sounds = super::sound::load(&mut materials, name, &surfaces, &data.entities);
    super::soundscape::load(&mut materials, &bsp, name, &mut sounds);
    data.sounds = std::sync::Arc::new(sounds);
    timer.lap("sounds");
    if let Some(bytes) = materials.read(&format!("maps/{}.nav", name.to_lowercase())) {
        match super::nav::parse(&bytes) {
            Ok((nav, _)) => data.nav = Some(std::sync::Arc::new(nav)),
            Err(e) => data.warnings.push(e),
        }
    }
    super::decals::add_decals(&bsp, &layout, &mut materials, &mut data);
    timer.lap("nav, decals");
    super::overlays::add_overlays(&bsp, &bytes, &layout, &mut materials, &mut data);
    timer.lap("overlays");
    super::sky::add_sky(&bsp, &mut materials, &mut data, hdr_level >= 2);
    timer.lap("sky");
    // Cubemap samples (lump 42: origin as three ints, then a size): the
    // baked cubemap at each, for objects that take the nearest.
    for origin in cubemap_samples(&bytes) {
        let path = format!(
            "maps/{}/c{}_{}_{}",
            name.to_lowercase(),
            origin[0],
            origin[1],
            origin[2]
        );
        // A renamed map packs them under its original name.
        let file = format!("c{}_{}_{}.vtf", origin[0], origin[1], origin[2]);
        let path = if materials.read(&format!("materials/{path}.vtf")).is_none()
            && let Some(other) = materials.packed_map_file(&file)
        {
            other
        } else {
            path
        };
        if let Some(c) = materials.cubemap(&path) {
            let at = Vec3::new(origin[0] as f32, origin[1] as f32, origin[2] as f32);
            data.cubemap_samples.push((
                to_engine(vbsp::Vector {
                    x: at.x,
                    y: at.y,
                    z: at.z,
                }),
                c,
            ));
        }
    }
    data.warnings.extend(materials.missing);
    data.textures = materials.textures;
    data.cubemaps = materials.cubemaps;
    timer.lap("cubemaps");
    bevy::log::info!("{name}: loaded in {}", timer.summary());
    data.load_times = timer.stages;
    // Left: putting it in the world (`map::change_map`).
    report(0.95, "LoadingProgress_SignonLocal");
    Ok(data)
}

/// The BSP with its entity and texture-name lumps made valid UTF-8 in
/// place: maps compiled or edited by other tools carry Latin-1 bytes there
/// (`E:\\Ñteam` in a VMEX comment, a `™` in a server message), which the
/// game reads as bytes but `vbsp` refuses. Each byte of an invalid
/// sequence becomes `?`, so no offset moves. Compressed lumps are left
/// alone.
pub fn text_lumps_utf8(mut bytes: Vec<u8>) -> Vec<u8> {
    const LUMPS: [usize; 2] = [0, 43];
    for lump in LUMPS {
        let at = 8 + lump * 16;
        let Some(entry) = bytes.get(at..at + 8) else { continue };
        let ofs = i32::from_le_bytes(entry[0..4].try_into().unwrap()).max(0) as usize;
        let len = i32::from_le_bytes(entry[4..8].try_into().unwrap()).max(0) as usize;
        let Some(data) = bytes.get_mut(ofs..ofs + len) else { continue };
        if data.starts_with(b"LZMA") {
            continue;
        }
        let mut i = 0;
        while i < data.len() {
            match std::str::from_utf8(&data[i..]) {
                Ok(_) => break,
                Err(e) => {
                    let bad = i + e.valid_up_to();
                    let n = e.error_len().unwrap_or(data.len() - bad);
                    data[bad..bad + n].fill(b'?');
                    i = bad + n;
                }
            }
        }
    }
    bytes
}

/// Triangles of a face in Source space, each vertex with its lightmap
/// coordinate in luxels (see `lightmap::luxel_coords`).
///
/// Flat faces project through the face's lightmap vectors. Displacements
/// don't: their samples follow the displacement grid, with the first
/// lightmap axis along the grid's second axis. This was established by
/// measurement on de_dust2: that mapping makes lighting agree where
/// neighbouring displacements meet (mean mismatch 1%, versus 48% for
/// projection; tests/it/heavy/map_de_dust2.rs checks it).
pub fn face_triangles(face: &vbsp::Handle<'_, vbsp::Face>) -> Vec<[(vbsp::Vector, Vec2); 3]> {
    face_triangles_blend(face)
        .into_iter()
        .map(|t| t.map(|(v, l, _)| (v, l)))
        .collect()
}

/// `face_triangles` with each vertex's blend weight for two-texture
/// (WorldVertexTransition) materials: the displacement vertex alpha / 255;
/// 0 on brush faces.
pub fn face_triangles_blend(face: &vbsp::Handle<'_, vbsp::Face>) -> Vec<[(vbsp::Vector, Vec2, f32); 3]> {
    let tex = face.texture();
    let Some(disp) = face.displacement() else {
        return face
            .triangulate()
            .map(|t| t.map(|v| (v, lightmap::luxel_coords(&tex, face, v), 0.0)))
            .collect();
    };
    // Base grid: bilinear over the face's corners, starting at the corner
    // nearest the displacement's start position.
    let mut corners: Vec<vbsp::Vector> = face.vertices().map(|v| v.position).collect();
    if corners.len() != 4 {
        return Vec::new();
    }
    let start = (0..4)
        .min_by(|&a, &b| {
            (corners[a] - disp.start_position)
                .length_squared()
                .total_cmp(&(corners[b] - disp.start_position).length_squared())
        })
        .unwrap();
    corners.rotate_left(start);
    let steps = 2usize.pow(disp.power as u32);
    let n = steps + 1;
    let lerp = |a: vbsp::Vector, b: vbsp::Vector, t: f32| a + (b - a) * t;
    let offsets: Vec<vbsp::Vector> = disp.displacement_vertices().map(|v| v.displacement()).collect();
    let alphas: Vec<f32> = disp
        .displacement_vertices()
        .map(|v| (v.alpha / 255.0).clamp(0.0, 1.0))
        .collect();
    if offsets.len() != n * n {
        return Vec::new();
    }
    let size = Vec2::new(
        face.light_map_texture_size[0] as f32,
        face.light_map_texture_size[1] as f32,
    );
    let grid = |x: usize, y: usize| {
        let (fx, fy) = (x as f32 / steps as f32, y as f32 / steps as f32);
        let base = lerp(lerp(corners[0], corners[1], fx), lerp(corners[3], corners[2], fx), fy);
        (base + offsets[x * n + y], Vec2::new(fy, fx) * size, alphas[x * n + y])
    };
    // Every triangle below runs the grid's way round (x, then y). The
    // whole surface faces the side the base face does, decided once from
    // the undisplaced grid: a sculpted surface folds over (a cave's
    // ceiling curling back from its wall) and those triangles face away
    // from the base normal, which is right: re-winding them one by one
    // turned them inside out (culled: holes in surf_boreas's caves).
    let v3 = |v: vbsp::Vector| Vec3::new(v.x, v.y, v.z);
    let grid_normal = (v3(corners[1]) - v3(corners[0])).cross(v3(corners[3]) - v3(corners[0]));
    let flip = grid_normal.dot(v3(face.normal())) < 0.0;
    let mut out = Vec::with_capacity(steps * steps * 2);
    // Each grid square splits along alternating diagonals (checkerboard).
    // Measured with movecmp: with this split, landing on dust2's CT spawn
    // terrain matches CS:S to 0.002 units; the uniform split didn't.
    for x in 0..steps {
        for y in 0..steps {
            if (x + y) % 2 == 0 {
                out.push([grid(x, y), grid(x + 1, y), grid(x + 1, y + 1)]);
                out.push([grid(x, y), grid(x + 1, y + 1), grid(x, y + 1)]);
            } else {
                out.push([grid(x, y), grid(x + 1, y), grid(x, y + 1)]);
                out.push([grid(x + 1, y), grid(x + 1, y + 1), grid(x, y + 1)]);
            }
        }
    }
    if flip {
        for t in &mut out {
            t.swap(1, 2);
        }
    }
    out
}

/// Where each world face's lighting landed in the atlas, for things that
/// share a face's lighting later (decals).
pub struct LightmapLayout {
    /// Per world face (in `models[0].faces()` order): its block, if lit.
    pub face_slots: Vec<Option<usize>>,
    /// Drawn brush entities' lit faces (by index in the face lump): their
    /// blocks.
    pub entity_face_slots: HashMap<usize, usize>,
    pub placements: Vec<lightmap::Placement>,
    /// Block for drawn faces without lighting.
    pub white: usize,
}

/// Geometry, collision, lightmaps and spawns, without materials.
/// `bytes` is the whole BSP file (for lumps vbsp doesn't keep as stored),
/// as `lightmap::select_lighting` left it; `hdr` is what it chose: the
/// HDR lightmaps (lump 53) or the LDR ones (lump 8).
pub fn convert_level(bsp: &Bsp, bytes: &[u8], name: &str, hdr: bool) -> (MapData, LightmapLayout) {
    let lighting = lightmap::lighting_lump(bytes, hdr);
    let leaves = super::ambient::raw_leaves(bytes);
    // Switchable light styles (32+) whose lights start off (spawnflag 1);
    // the others are lit at map start (cs_office's projector, cs_assault's
    // red lights).
    let dark_styles: std::collections::HashSet<u8> = bsp
        .entities
        .iter()
        .filter(|e| e.prop("classname").is_some_and(|c| c.starts_with("light")))
        .filter(|e| {
            e.prop("spawnflags")
                .and_then(|f| f.trim().parse::<i32>().ok())
                .unwrap_or(0)
                & 1
                != 0
        })
        .filter_map(|e| e.prop("style").and_then(|s| s.trim().parse::<u8>().ok()))
        .filter(|s| *s >= 32)
        .collect();
    let mut by_material: BTreeMap<String, MapMesh> = BTreeMap::new();
    // Per mesh: each vertex's lightmap block slot and luxel coordinate,
    // resolved to atlas UVs once all blocks are packed.
    let mut pending_lm: BTreeMap<String, Vec<(Option<usize>, Vec2)>> = BTreeMap::new();
    let mut atlas = AtlasBuilder::default();
    let mut face_slots: Vec<Option<usize>> = Vec::new();
    let mut data = MapData {
        name: format!("cs_source:{name}"),
        look: source_look(),
        ..default()
    };

    let bounds = playable_bounds(bsp);
    // The world is model 0; drawn brush entities (doors, windows,
    // func_brush) follow, placed by their origin and angles.
    let world = bsp.models().next().expect("BSP has no world model");
    let entities = brush_entities(bsp);
    // Movers (doors, buttons, platforms) keep their faces local to their
    // own node (`MapMesh::entity`), so the logic layer can move them.
    // Brush entity faces' lightmap blocks, by face index (infodecals on
    // brush entities take them).
    let mut entity_face_slots: HashMap<usize, usize> = HashMap::new();
    let faces = world
        .faces()
        .map(|f| (f, None, None, None, None))
        .chain(entities.iter().filter(|e| e.drawn).flat_map(|e| {
            bsp.models()
                .nth(e.model)
                .into_iter()
                .flat_map(|m| {
                    let first = m.first_face.max(0) as usize;
                    m.faces().enumerate().map(move |(i, f)| (f, first + i)).collect::<Vec<_>>()
                })
                .map(|(f, index)| {
                    if e.mover {
                        (f, Some((Quat::IDENTITY, Vec3::ZERO)), Some(e.entity), Some(index), None)
                    } else {
                        (f, Some(e.transform), None, Some(index), e.render)
                    }
                })
        }));
    for (face, transform, mover, face_index, render) in faces {
        let in_world = transform.is_none();
        // Source-space position of a face vertex (brush entity models are
        // stored around their origin).
        let place = |v: vbsp::Vector| match transform {
            Some((r, o)) => {
                let p = r * Vec3::new(v.x, v.y, v.z) + o;
                vbsp::Vector { x: p.x, y: p.y, z: p.z }
            }
            None => v,
        };
        if in_world {
            face_slots.push(None);
        }
        let centre = {
            let pts: Vec<vbsp::Vector> = face.vertices().map(|v| place(v.position)).collect();
            let n = pts.len().max(1) as f32;
            vbsp::Vector {
                x: pts.iter().map(|p| p.x).sum::<f32>() / n,
                y: pts.iter().map(|p| p.y).sum::<f32>() / n,
                z: pts.iter().map(|p| p.z).sum::<f32>() / n,
            }
        };
        let skybox = mover.is_none() && bounds.as_ref().is_some_and(|b| !b.contains(centre));
        let tex = face.texture();
        let flags = tex.flags;
        // The playable world's sky faces, kept for `map::sky_occluder`.
        if in_world && !skybox && flags.intersects(TextureFlags::SKY | TextureFlags::SKY2D) {
            let pts: Vec<Vec3> = face.vertices().map(|v| to_engine(v.position)).collect();
            for i in 1..pts.len().saturating_sub(1) {
                data.sky_surfaces
                    .extend([pts[0], pts[i], pts[i + 1]].map(|p| p.to_array()));
            }
        }
        if flags.intersects(NOT_DRAWN) && flags.intersects(NOT_SOLID) {
            continue;
        }
        let face_normal = match transform {
            Some((r, _)) => {
                let n = r * Vec3::new(face.normal().x, face.normal().y, face.normal().z);
                to_engine_dir(vbsp::Vector { x: n.x, y: n.y, z: n.z })
            }
            None => to_engine_dir(face.normal()),
        };
        let displaced_face = face.displacement().is_some();
        // Re-wind triangles to face the plane normal (displacements come
        // wound per surface, folds included: `face_triangles_blend`).
        let tris: Vec<[(vbsp::Vector, Vec2, f32); 3]> = face_triangles_blend(&face)
            .into_iter()
            .map(|t| {
                let [a, b, c] = t.map(|(v, _, _)| to_engine(place(v)));
                if !displaced_face && (b - a).cross(c - a).dot(face_normal) < 0.0 {
                    [t[0], t[2], t[1]]
                } else {
                    t
                }
            })
            .collect();

        // Brush solids come from the brush lump below; only displacement
        // surfaces (which have no brush volume) collide as triangles.
        if in_world && displaced_face && !flags.intersects(NOT_SOLID) {
            for t in &tris {
                let base = data.collision_positions.len() as u32;
                data.collision_positions
                    .extend(t.iter().map(|(v, _, _)| to_engine(*v).to_array()));
                data.collision_indices.push([base, base + 1, base + 2]);
            }
        }
        if flags.intersects(NOT_DRAWN) {
            continue;
        }

        let (flat, bumped) =
            lightmap::face_samples_lit(lighting, &face, flags.contains(TextureFlags::BUMPLIGHT), &|style| {
                dark_styles.contains(&style)
            });
        let slot = flat.map(|s| atlas.add_bumped(s, bumped));
        // Animated and switchable styles kept apart too, so lights can
        // flicker and switch.
        if let Some(slot) = slot {
            let styles: Vec<_> = lightmap::face_extra_styles(lighting, &face, flags.contains(TextureFlags::BUMPLIGHT))
                .into_iter()
                .map(|s| {
                    let on = s.style < 32 || !dark_styles.contains(&s.style);
                    (s, on)
                })
                    .collect();
            if !styles.is_empty() {
                atlas.set_styles(slot, styles);
            }
        }
        if in_world {
            *face_slots.last_mut().unwrap() = slot;
        } else if let (Some(i), Some(slot)) = (face_index, slot) {
            entity_face_slots.insert(i, slot);
        }
        let material = tex.name().to_lowercase();
        // Meshes are per material and per part (world, 3D skybox, or a
        // mover entity).
        let mut key = match mover {
            Some(i) => format!("{material}\u{2}{i:06}"),
            None if skybox => format!("{material}\u{1}skybox"),
            None => material.clone(),
        };
        // Brush entities in a translucent render mode: meshes of their own.
        if let Some((blend, a)) = render {
            key = format!("{key}\u{3}{blend:?}{a}");
        }
        let mesh = by_material.entry(key.clone()).or_insert_with(|| MapMesh {
            material: material.clone(),
            skybox,
            color: tex.debug_color(),
            entity: mover,
            render,
            ..default()
        });
        let lm = pending_lm.entry(key).or_default();
        let displaced = face.displacement().is_some();
        for t in &tris {
            let p: [Vec3; 3] = t.map(|(v, _, _)| to_engine(place(v)));
            // Brush faces are flat; displacements get per-triangle normals.
            let n = if displaced {
                (p[1] - p[0]).cross(p[2] - p[0]).normalize_or_zero()
            } else {
                face_normal
            };
            for (v, luxel, blend) in t {
                mesh.indices.push(mesh.positions.len() as u32);
                mesh.positions.push(to_engine(place(*v)).to_array());
                mesh.normals.push(n.to_array());
                mesh.uvs.push(tex.uv(*v));
                mesh.blend_weights.push(*blend);
                lm.push((slot, *luxel));
            }
        }
    }

    data.collision_hulls = brush_hulls(bsp, &leaves);
    let (brushes, tree) = collision_brushes(bsp, &leaves);
    data.collision_brushes = brushes;
    data.brush_tree = Some(tree);
    data.trace_skip = trace_skip(bsp, &leaves);
    data.water = water_volumes(bsp, &leaves);
    data.water.extend(analog_water_volumes(bsp, &leaves));
    data.entities = map_entities(bsp, &leaves);
    data.entity_scale = METERS_PER_UNIT;

    let (mut lightmap, placements, white) = atlas.build();
    // Animated styles' patterns: a light's custom `pattern` for its
    // style, else the style's preset.
    for s in lightmap.styles.iter_mut().filter(|s| s.style < 32) {
        let custom = bsp
            .entities
            .iter()
            .filter(|e| e.prop("classname").is_some_and(|c| c.starts_with("light")))
            .filter(|e| e.prop("style").and_then(|v| v.trim().parse::<u8>().ok()) == Some(s.style))
            .find_map(|e| e.prop("pattern").filter(|p| !p.trim().is_empty()));
        s.pattern = lightmap::style_pattern(s.style, custom);
    }
    for (material, mesh) in by_material.iter_mut() {
        mesh.lightmap_uvs = pending_lm[material]
            .iter()
            .map(|&(slot, luxel)| match slot {
                Some(s) => lightmap::atlas_uv(&lightmap, placements[s], luxel),
                None => lightmap::atlas_uv(&lightmap, placements[white], Vec2::splat(0.5)),
            })
            .collect();
    }
    data.meshes = by_material.into_values().filter(|m| !m.indices.is_empty()).collect();
    let layout = LightmapLayout {
        face_slots,
        entity_face_slots,
        placements,
        white,
    };

    for ent in bsp.entities.iter() {
        let team = match ent.prop("classname") {
            Some("info_player_terrorist") => Some(Team(1)),
            Some("info_player_counterterrorist") => Some(Team(2)),
            _ => continue,
        };
        if let Some(origin) = ent.prop("origin").and_then(parse_vector) {
            data.spawns.push((to_engine(origin), team));
            // Source yaw 0 faces +X (engine +X); intent yaw 0 faces -Z,
            // which is Source yaw 90.
            let yaw = ent
                .prop("angles")
                .and_then(parse_vector)
                .map_or(0.0, |a| a.y)
                .to_radians();
            data.spawn_yaws.push(yaw - std::f32::consts::FRAC_PI_2);
        }
    }
    data.lightmap = Some(lightmap);
    data.sky_camera = sky_camera(bsp);
    data.fog = world_fog(bsp);
    data.shadows = Some(shadow_control(bsp));
    data.playable = playable_bounds(bsp).map(|b| b.engine());
    // Physics bodies fall at sv_gravity (800 units/s^2), read at load.
    data.gravity = Some(800.0 * METERS_PER_UNIT);
    (data, layout)
}

/// A brush entity's model and placement.
pub struct BrushEntity {
    /// Index into the BSP's models.
    pub model: usize,
    /// Source-space rotation and origin (units).
    pub transform: (Quat, Vec3),
    /// Drawn (not a trigger or volume, not render mode 10).
    pub drawn: bool,
    /// Players collide with it (doors and breakables as they spawn: closed,
    /// unbroken).
    pub solid: bool,
    /// Index in the entity lump.
    pub entity: usize,
    /// Moves or toggles (`MOVERS`): drawn and solid through its own node,
    /// not baked into the world.
    pub mover: bool,
    /// Baked into the world in a translucent render mode (rendermode,
    /// renderamt): its faces blend or add by this opacity. Movers get
    /// theirs from the logic (`map::tint`).
    pub render: Option<(MapAlpha, f32)>,
}

/// Brush entity classes the logic layer moves or toggles.
pub const MOVERS: &[&str] = &[
    "func_door",
    "func_door_rotating",
    "func_button",
    "func_rot_button",
    "momentary_rot_button",
    "func_movelinear",
    "func_rotating",
    "func_tracktrain",
    "func_tanktrain",
    "func_monitor",
    "func_brush",
    "func_wall_toggle",
    "func_conveyor",
    "func_physbox",
    "func_physbox_multiplayer",
];

/// Brush entities that render or collide. Volumes (triggers, buy zones,
/// bomb sites, area portals, occluders, precipitation, dust) do neither.
pub fn brush_entities(bsp: &Bsp) -> Vec<BrushEntity> {
    const VOLUMES: &[&str] = &[
        "func_buyzone",
        "func_bomb_target",
        "func_hostage_rescue",
        "func_escapezone",
        "func_vip_safetyzone",
        "func_no_defuse",
        "func_areaportal",
        "func_areaportalwindow",
        "func_occluder",
        "func_precipitation",
        "func_dustmotes",
        "func_dustcloud",
        "func_smokevolume",
        "func_clip_vphysics",
        "func_viscluster",
        "func_ladderendpoint",
        "func_vehicleclip",
    ];
    // Never solid to players.
    const NOT_SOLID: &[&str] = &["func_illusionary", "func_lod"];
    // Names of entities that move, for brushes parented to them.
    let moving_names: std::collections::BTreeSet<String> = bsp
        .entities
        .iter()
        .filter(|e| {
            // Placed weapons move too (picked up, dropped: their children
            // follow, `map::entities::anchor_class`).
            e.prop("classname")
                .is_some_and(|c| MOVERS.contains(&c) || crate::map::entities::anchor_class(c))
        })
        .filter_map(|e| e.prop("targetname").map(|n| n.to_ascii_lowercase()))
        .collect();
    let mut out = Vec::new();
    for (index, ent) in bsp.entities.iter().enumerate() {
        let Some(class) = ent.prop("classname") else { continue };
        let Some(model) = ent
            .prop("model")
            .and_then(|m| m.strip_prefix('*'))
            .and_then(|m| m.parse().ok())
        else {
            continue;
        };
        if model == 0 || class.starts_with("trigger_") || VOLUMES.contains(&class) {
            continue;
        }
        let num = |k: &'static str| ent.prop(k).and_then(|v| v.trim().parse::<f32>().ok());
        let origin = ent
            .prop("origin")
            .and_then(parse_vector)
            .map_or(Vec3::ZERO, |o| Vec3::new(o.x, o.y, o.z));
        let angles = ent
            .prop("angles")
            .and_then(parse_vector)
            .map_or(Vec3::ZERO, |a| Vec3::new(a.x, a.y, a.z));
        // Source angles: pitch about Y, yaw about Z, roll about X.
        let rotation = Quat::from_rotation_z(angles.y.to_radians())
            * Quat::from_rotation_y(angles.x.to_radians())
            * Quat::from_rotation_x(angles.z.to_radians());
        let render_mode = num("rendermode").unwrap_or(0.0) as i32;
        let start_disabled = num("StartDisabled").unwrap_or(0.0) != 0.0;
        // Movers (parented or not: a mover child rides its parent,
        // `logic::anchors`), and brushes parented to one (they follow it:
        // de_nuke's door windows). Breakables get a node too, so they can
        // disappear.
        let parent = ent.prop("parentname").filter(|p| !p.is_empty());
        let mover = MOVERS.contains(&class)
            || parent.is_some_and(|p| moving_names.contains(&p.to_ascii_lowercase()))
            || super::breakables::CLASSES.contains(&class);
        // A disabled func_brush that can toggle is drawn through its node
        // (the logic layer hides it).
        let drawn = render_mode != 10 && !(class == "func_brush" && start_disabled && !mover);
        let solid = !NOT_SOLID.contains(&class)
            // func_brush "Solidity": 0 toggle (with the brush), 1 never, 2 always.
            && !(class == "func_brush" && (num("Solidity") == Some(1.0) || (start_disabled && num("Solidity") != Some(2.0))))
            // func_rotating spawnflag 64: not solid.
            && !(class == "func_rotating" && (num("spawnflags").unwrap_or(0.0) as i32) & 64 != 0);
        let look = crate::map::tint::RenderLook {
            color: [255; 3],
            alpha: num("renderamt").unwrap_or(255.0).clamp(0.0, 255.0) as u8,
            mode: render_mode.clamp(0, 255) as u8,
        };
        out.push(BrushEntity {
            model,
            transform: (rotation, origin),
            drawn: drawn && !look.invisible(),
            solid,
            entity: index,
            mover,
            render: look.blend().filter(|_| !mover),
        });
    }
    out
}

/// The playable world's bounds (Source units), from the world's
/// `world_mins`/`world_maxs`. The map compiler computes these without the
/// 3D skybox, so content outside them (with a margin) is skybox content.
pub struct Bounds {
    lo: Vec3,
    hi: Vec3,
}

impl Bounds {
    const MARGIN: f32 = 64.0;

    /// Engine-space box (meters), margin included.
    pub fn engine(&self) -> (Vec3, Vec3) {
        let a = to_engine(vbsp::Vector {
            x: self.lo.x - Self::MARGIN,
            y: self.lo.y - Self::MARGIN,
            z: self.lo.z - Self::MARGIN,
        });
        let b = to_engine(vbsp::Vector {
            x: self.hi.x + Self::MARGIN,
            y: self.hi.y + Self::MARGIN,
            z: self.hi.z + Self::MARGIN,
        });
        (a.min(b), a.max(b))
    }

    pub fn contains(&self, p: vbsp::Vector) -> bool {
        self.contains_point(Vec3::new(p.x, p.y, p.z))
    }

    /// `contains` for a point in Source units held as a `Vec3`.
    pub fn contains_point(&self, p: Vec3) -> bool {
        p.cmpge(self.lo - Self::MARGIN).all() && p.cmple(self.hi + Self::MARGIN).all()
    }
}

pub fn playable_bounds(bsp: &Bsp) -> Option<Bounds> {
    let world = bsp
        .entities
        .iter()
        .find(|e| e.prop("classname") == Some("worldspawn"))?;
    let v = |key: &'static str| world.prop(key).and_then(parse_vector).map(|v| Vec3::new(v.x, v.y, v.z));
    // Only meaningful when there is a 3D skybox.
    bsp.entities
        .iter()
        .find(|e| e.prop("classname") == Some("sky_camera"))?;
    Some(Bounds {
        lo: v("world_mins")?,
        hi: v("world_maxs")?,
    })
}

fn sky_camera(bsp: &Bsp) -> Option<crate::map::MapSkyCamera> {
    let e = bsp
        .entities
        .iter()
        .find(|e| e.prop("classname") == Some("sky_camera"))?;
    let origin = to_engine(e.prop("origin").and_then(parse_vector)?);
    let scale: f32 = e
        .prop("scale")
        .and_then(|s| s.parse().ok())
        .filter(|s: &f32| *s > 0.0)
        .unwrap_or(16.0);
    let fog = parse_fog(&e);
    Some(crate::map::MapSkyCamera { origin, scale, fog })
}

/// The world's BSP tree with each leaf's sky visibility (leaf flags,
/// specs/cs_source/shadows_sky.md).
fn sky_vis(bsp: &Bsp, bytes: &[u8]) -> crate::map::MapSkyVis {
    use super::ambient::{CONTENTS_SOLID, LEAF_SKY, LEAF_SKY2D, raw_leaves};
    use crate::map::LeafSky;
    let head = bsp.models.first().map_or(0, |m| m.head_node.max(0) as usize);
    // Re-root at the world's head node (normally 0) by offsetting indices.
    let nodes = bsp.nodes[head..]
        .iter()
        .map(|n| {
            let child = |c: i32| if c >= 0 { c - head as i32 } else { c };
            (n.plane_index as usize, [child(n.children[0]), child(n.children[1])])
        })
        .collect();
    let planes = bsp
        .planes
        .iter()
        .map(|p| (to_engine_dir(p.normal), p.dist * METERS_PER_UNIT))
        .collect();
    let leaves = raw_leaves(bytes)
        .iter()
        .map(|l| {
            if l.contents & CONTENTS_SOLID != 0 {
                LeafSky::Solid
            } else if l.flags & LEAF_SKY != 0 {
                LeafSky::Sky3d
            } else if l.flags & LEAF_SKY2D != 0 {
                LeafSky::Sky2d
            } else {
                LeafSky::None
            }
        })
        .collect();
    crate::map::MapSkyVis { planes, nodes, leaves }
}

/// The world's visibility (public BSP v20 description): leaf clusters
/// (lump 10) and each cluster's potentially visible set, run-length
/// encoded in the visibility lump (4) after a cluster count and per
/// cluster a PVS and a PAS offset from the lump's start. None when the map
/// was compiled without vis.
pub fn visibility(bsp: &Bsp, bytes: &[u8]) -> Option<crate::map::vis::MapVisibility> {
    let lump = super::ambient::lump(bytes, 4);
    let int = |at: usize| lump.get(at..at + 4).map(|b| i32::from_le_bytes(b.try_into().unwrap()));
    let count = int(0)?.max(0) as usize;
    if count == 0 || lump.len() < 4 + count * 8 {
        return None;
    }
    let visible = (0..count)
        .map(|c| {
            let offset = int(4 + c * 8).unwrap_or(0).max(0) as usize;
            let mut row = crate::map::vis::decompress_row(lump.get(offset..).unwrap_or(&[]), count);
            // A cluster always sees itself.
            row[c / 64] |= 1 << (c % 64);
            row
        })
        .collect();
    let tree = sky_vis(bsp, bytes);
    let leaf_clusters: Vec<i32> = super::ambient::raw_leaves(bytes)
        .iter()
        .map(|l| {
            if l.contents & super::ambient::CONTENTS_SOLID != 0 {
                -1
            } else {
                l.cluster as i32
            }
        })
        .collect();
    let leaf_areas = super::ambient::raw_leaves(bytes).iter().map(|l| l.area).collect();
    let areas = crate::map::vis::MapAreas::new(leaf_areas, &leaf_clusters, count, area_portals(bsp, bytes));
    Some(crate::map::vis::MapVisibility {
        planes: tree.planes,
        nodes: tree.nodes,
        leaf_clusters,
        cluster_count: count,
        visible,
        areas,
        occluders: occluders(bsp, bytes),
    })
}

/// The map's occluders (public BSP v20 description, occlusion lump 9): a
/// count and per occluder its flags, first polygon, polygon count and
/// bounds (lump version 2 adds an area); a count and per polygon its
/// first vertex index, vertex count and plane; a count and the vertex
/// indices, into the vertex lump (3). Each func_occluder names its
/// occluder by `occludernumber`; `StartActive` (default 1) sets whether it
/// starts active. Occluders without an entity are taken as active.
pub fn occluders(bsp: &Bsp, bytes: &[u8]) -> Vec<crate::map::vis::Occluder> {
    const LUMP_OCCLUSION: usize = 9;
    let lump = super::ambient::lump(bytes, LUMP_OCCLUSION);
    let version = bytes
        .get(8 + LUMP_OCCLUSION * 16 + 8..8 + LUMP_OCCLUSION * 16 + 12)
        .map_or(2, |v| i32::from_le_bytes(v.try_into().unwrap()));
    let int = |at: usize| lump.get(at..at + 4).map(|b| i32::from_le_bytes(b.try_into().unwrap()));
    let count = |at: usize| int(at).map_or(0, |n| n.max(0) as usize);
    let size = if version >= 2 { 40 } else { 36 };
    let occluder_count = count(0);
    let polys_at = 4 + occluder_count * size;
    let poly_count = count(polys_at);
    let indices_at = polys_at + 4 + poly_count * 12;
    let index_count = count(indices_at);
    if lump.len() < indices_at + 4 + index_count * 4 {
        return Vec::new();
    }
    let vertices = super::ambient::lump(bytes, 3);
    let vertex = |i: i32| -> Option<Vec3> {
        let b = vertices.get(usize::try_from(i).ok()? * 12..)?.get(..12)?;
        let f = |at: usize| f32::from_le_bytes(b[at..at + 4].try_into().unwrap());
        Some(to_engine(vbsp::Vector {
            x: f(0),
            y: f(4),
            z: f(8),
        }))
    };
    let polygon = |p: usize| -> Option<Vec<Vec3>> {
        let at = polys_at + 4 + p * 12;
        let (first, n) = (count(at), count(at + 4));
        (first..first + n)
            .map(|k| vertex(int(indices_at + 4 + k * 4).filter(|_| k < index_count)?))
            .collect()
    };
    let start_active: std::collections::HashMap<u16, bool> = bsp
        .entities
        .iter()
        .filter(|e| e.prop("classname") == Some("func_occluder"))
        .filter_map(|e| {
            let key = e.prop("occludernumber")?.trim().parse().ok()?;
            let active = e
                .properties()
                .find(|(k, _)| k.eq_ignore_ascii_case("StartActive"))
                .is_none_or(|(_, v)| v.trim().parse::<i32>().map_or(true, |v| v != 0));
            Some((key, active))
        })
        .collect();
    (0..occluder_count)
        .map(|i| {
            let at = 4 + i * size;
            let (first, n) = (count(at + 4), count(at + 8));
            let key = i as u16;
            crate::map::vis::Occluder {
                key,
                polygons: (first..(first + n).min(poly_count)).filter_map(polygon).filter(|p| p.len() >= 3).collect(),
                start_active: start_active.get(&key).copied().unwrap_or(true),
            }
        })
        .collect()
}

/// The areaportals between the map's areas (public BSP v20 description):
/// the areas lump (20: per area, a count and first index into the
/// areaportals lump), the areaportals lump (21: portal key, the area on
/// the other side, first clip vertex and vertex count, plane) and the clip
/// portal vertices (41). Each portal is listed from both its areas; it
/// comes back once. A func_areaportalwindow's `FadeDist` (named by its
/// `portalnumber`, which is the portal key) is kept as the portal's fade.
pub fn area_portals(bsp: &Bsp, bytes: &[u8]) -> Vec<crate::map::vis::AreaPortal> {
    let u16_at = |b: &[u8], at: usize| u16::from_le_bytes([b[at], b[at + 1]]);
    let areas = super::ambient::lump(bytes, 20);
    let portals = super::ambient::lump(bytes, 21);
    let verts: Vec<Vec3> = super::ambient::lump(bytes, 41)
        .chunks_exact(12)
        .map(|b| {
            let f = |at: usize| f32::from_le_bytes(b[at..at + 4].try_into().unwrap());
            to_engine(vbsp::Vector {
                x: f(0),
                y: f(4),
                z: f(8),
            })
        })
        .collect();
    let windows: std::collections::HashMap<u16, crate::map::vis::WindowFade> = bsp
        .entities
        .iter()
        .filter(|e| e.prop("classname") == Some("func_areaportalwindow"))
        .filter_map(|e| {
            let key = e.prop("portalnumber")?.trim().parse().ok()?;
            // Compiled maps lower-case some keys.
            let num = |name: &str| {
                e.properties()
                    .find(|(k, _)| k.eq_ignore_ascii_case(name))
                    .and_then(|(_, v)| v.trim().parse::<f32>().ok())
            };
            let end = num("FadeDist")?;
            // The window's brush: the brush entity its `target` names.
            let brush = e.prop("target").filter(|t| !t.is_empty()).and_then(|t| {
                bsp.entities.iter().position(|b| {
                    b.prop("targetname").is_some_and(|n| n.eq_ignore_ascii_case(t))
                        && b.prop("model").is_some_and(|m| m.starts_with('*'))
                })
            });
            Some((
                key,
                crate::map::vis::WindowFade {
                    start: num("FadeStartDist").unwrap_or(end).min(end) * METERS_PER_UNIT,
                    end: end * METERS_PER_UNIT,
                    limit: num("TranslucencyLimit").unwrap_or(0.0),
                    brush,
                },
            ))
        })
        .collect();
    let mut out: Vec<crate::map::vis::AreaPortal> = Vec::new();
    for (area, a) in areas.chunks_exact(8).enumerate() {
        let count = i32::from_le_bytes(a[0..4].try_into().unwrap()).max(0) as usize;
        let first = i32::from_le_bytes(a[4..8].try_into().unwrap()).max(0) as usize;
        for p in (first..first + count).filter_map(|i| portals.get(i * 12..i * 12 + 12)) {
            let (key, other) = (u16_at(p, 0), u16_at(p, 2));
            let (vfirst, vcount) = (u16_at(p, 4) as usize, u16_at(p, 6) as usize);
            let pair = [(area as u16).min(other), (area as u16).max(other)];
            if out.iter().any(|o| o.key == key && o.areas == pair) {
                continue;
            }
            out.push(crate::map::vis::AreaPortal {
                key,
                areas: pair,
                polygon: verts.get(vfirst..vfirst + vcount).map(<[Vec3]>::to_vec).unwrap_or_default(),
                fade: windows.get(&key).copied(),
                see_through: false,
            });
        }
    }
    out
}

/// Fog keys shared by `sky_camera` and `env_fog_controller`.
fn parse_fog(e: &vbsp::RawEntity) -> Option<crate::map::MapFog> {
    if e.prop("fogenable") != Some("1") {
        return None;
    }
    let c: Vec<f32> = e
        .prop("fogcolor")?
        .split_whitespace()
        .filter_map(|v| v.parse().ok())
        .collect();
    let start: f32 = e.prop("fogstart")?.parse().ok()?;
    let end: f32 = e.prop("fogend")?.parse().ok()?;
    Some(crate::map::MapFog {
        color: [c.first()? / 255.0, c.get(1)? / 255.0, c.get(2)? / 255.0],
        start: start * METERS_PER_UNIT,
        end: end * METERS_PER_UNIT,
        max_density: e.prop("fogmaxdensity").and_then(|v| v.parse().ok()).unwrap_or(1.0),
    })
}

/// The map's dynamic shadow settings (`shadow_control`), or the engine's
/// defaults without one (specs/cs_source/shadows_sky.md B).
fn shadow_control(bsp: &Bsp) -> crate::map::MapShadows {
    let e = bsp
        .entities
        .iter()
        .find(|e| e.prop("classname") == Some("shadow_control"));
    let nums = |key: &'static str| -> Option<Vec<f32>> {
        let v: Vec<f32> = e
            .as_ref()?
            .prop(key)?
            .split_whitespace()
            .filter_map(|v| v.parse().ok())
            .collect();
        (v.len() >= 3).then_some(v)
    };
    let forward = |p: f32, y: f32| {
        let (p, y) = (p.to_radians(), y.to_radians());
        vbsp::Vector {
            x: p.cos() * y.cos(),
            y: p.cos() * y.sin(),
            z: -p.sin(),
        }
    };
    let direction = match &e {
        None => vbsp::Vector {
            x: 0.1,
            y: 0.1,
            z: -1.0,
        },
        Some(_) => match nums("angles") {
            Some(a) if a.iter().any(|v| *v != 0.0) => forward(a[0], a[1]),
            Some(_) => forward(80.0, 30.0),
            None => vbsp::Vector {
                x: 0.2,
                y: 0.2,
                z: -2.0,
            },
        },
    };
    // Without a shadow_control the colour comes from the level's ambient
    // light (open in the spec); the entity default stands in.
    let color = nums("color").map_or([64, 64, 64], |c| [0, 1, 2].map(|i| c[i].clamp(0.0, 255.0) as u8));
    let distance = e
        .as_ref()
        .and_then(|e| e.prop("distance"))
        .and_then(|v| v.trim().parse::<f32>().ok())
        .unwrap_or(50.0);
    crate::map::MapShadows {
        direction: to_engine_dir(direction),
        color,
        distance: distance * METERS_PER_UNIT,
    }
}

/// The world's fog (`env_fog_controller`), if enabled.
pub fn world_fog(bsp: &Bsp) -> Option<crate::map::MapFog> {
    let e = bsp
        .entities
        .iter()
        .find(|e| e.prop("classname") == Some("env_fog_controller"))?;
    parse_fog(&e)
}

/// How CS:S presents maps at its LDR settings (mat_hdr_level 0,
/// mat_trilinear 0, mat_forceaniso 1). Lightmaps use the engine's LDR
/// encoding (specs/cs_source/shaders.md, bump pages included), no
/// tonemapping and no further scale: refcmp luma matches within ~1%.
pub fn source_look() -> crate::map::MapLook {
    crate::map::MapLook {
        light_scale: 1.0,
        trilinear: false,
        anisotropy: 1,
        tonemapping: false,
        source_ldr_lightmaps: true,
        // CS:S runs r_lightmap_bicubic 1 (read over RCON; seen in a
        // RenderDoc capture as 4 taps per lightmap page).
        bicubic_lightmaps: true,
        hdr: None,
    }
}

/// `source_look` at a `mat_hdr_level` (already lowered to 0 for maps
/// without HDR lighting). Level 2 keeps lightmaps linear (shaders.md
/// section 1: in HDR they aren't stored gamma-encoded; their scale is
/// engine-defined and taken as 1 here, an open question) and adds auto
/// exposure within the map's tone-map controller bounds; levels 1 and 2
/// add bloom.
pub fn source_look_level(hdr_level: u8, entities: &[crate::map::MapEntity]) -> crate::map::MapLook {
    let mut look = source_look();
    if hdr_level == 0 {
        return look;
    }
    let t = tonemap_controller(entities);
    look.hdr = Some(crate::map::MapHdr {
        exposure: (hdr_level >= 2).then_some((t.exposure_min, t.exposure_max)),
        bloom_scale: t.bloom_scale,
        rate: 1.0,
    });
    if hdr_level >= 2 {
        look.source_ldr_lightmaps = false;
    }
    look
}

/// What the map's `env_tonemap_controller` is told at map start.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TonemapController {
    pub exposure_min: f32,
    pub exposure_max: f32,
    pub bloom_scale: f32,
}

/// The game's tone-map defaults without a controller: the cvars
/// mat_autoexposure_min 0.5, mat_autoexposure_max 2 and mat_bloomscale 1
/// (their public defaults).
pub const DEFAULT_TONEMAP: TonemapController = TonemapController {
    exposure_min: crate::map::DEFAULT_AUTO_EXPOSURE.0,
    exposure_max: crate::map::DEFAULT_AUTO_EXPOSURE.1,
    bloom_scale: 1.0,
};

/// The tone-map settings an `env_tonemap_controller` gets from map-start
/// outputs (`OnMapSpawn` of `logic_auto`, how every stock map sets them):
/// SetAutoExposureMin, SetAutoExposureMax and SetBloomScale: the look the
/// map starts with. The logic layer then follows every controller input
/// as it fires (`map::TonemapInputs`), these included.
pub fn tonemap_controller(entities: &[crate::map::MapEntity]) -> TonemapController {
    let names: Vec<&str> = entities
        .iter()
        .filter(|e| e.classname().eq_ignore_ascii_case("env_tonemap_controller"))
        .filter_map(|e| e.get("targetname"))
        .collect();
    let mut t = DEFAULT_TONEMAP;
    for e in entities.iter().filter(|e| e.classname().eq_ignore_ascii_case("logic_auto")) {
        for (key, value) in &e.keyvalues {
            if !key.eq_ignore_ascii_case("OnMapSpawn") {
                continue;
            }
            // target, input, parameter, delay, times: ESC-separated when
            // there is one, else commas.
            let sep = if value.contains('\u{1b}') { '\u{1b}' } else { ',' };
            let parts: Vec<&str> = value.split(sep).collect();
            let (Some(target), Some(input), Some(param)) = (parts.first(), parts.get(1), parts.get(2)) else {
                continue;
            };
            if !names.iter().any(|n| n.eq_ignore_ascii_case(target.trim())) {
                continue;
            }
            let Ok(v) = param.trim().parse::<f32>() else {
                continue;
            };
            match input.to_ascii_lowercase().as_str() {
                "setautoexposuremin" => t.exposure_min = v,
                "setautoexposuremax" => t.exposure_max = v,
                "setbloomscale" => t.bloom_scale = v,
                _ => {}
            }
        }
    }
    t
}

/// Contents that stop players: solid world, glass, grates, player clips.
const PLAYER_SOLID: BrushFlags = BrushFlags::SOLID
    .union(BrushFlags::WINDOW)
    .union(BrushFlags::GRATE)
    .union(BrushFlags::PLAYERCLIP);

/// Indices of brushes that belong to the world (model 0), found through its
/// BSP tree. Brush entities (buy zones, bomb sites, doors) have their own
/// trees and are excluded here.
fn world_brushes(bsp: &Bsp, leaves: &[RawLeaf]) -> std::collections::BTreeSet<usize> {
    model_brushes(bsp, leaves, 0)
}

/// Indices of a model's brushes, through its BSP tree.
/// `leaves` are the leaves as stored (`ambient::raw_leaves`): vbsp's own
/// list is sorted by cluster, so its indices don't match the tree's.
fn model_brushes(bsp: &Bsp, leaves: &[RawLeaf], model: usize) -> std::collections::BTreeSet<usize> {
    let mut out = std::collections::BTreeSet::new();
    let Some(world) = bsp.models.get(model) else { return out };
    let mut stack = vec![world.head_node];
    while let Some(child) = stack.pop() {
        if child >= 0 {
            if let Some(node) = bsp.nodes.get(child as usize) {
                stack.extend(node.children);
            }
        } else if let Some(leaf) = leaves.get((-child - 1) as usize) {
            let first = leaf.first_leaf_brush as usize;
            for lb in bsp.leaf_brushes.iter().skip(first).take(leaf.leaf_brush_count as usize) {
                out.insert(lb.brush as usize);
            }
        }
    }
    out
}

/// Every world and brush entity brush that stops shots and physics (player
/// solids minus player-clip-only brushes) as a convex hull in engine space:
/// the corners where three of its planes meet and no other plane cuts them
/// off.
pub fn brush_hulls(bsp: &Bsp, leaves: &[RawLeaf]) -> Vec<Vec<[f32; 3]>> {
    // Player clips stop only players (movement sweeps `collision_brushes`):
    // shots, physics props and ragdolls pass them.
    let blocks = BrushFlags::SOLID
        .union(BrushFlags::WINDOW)
        .union(BrushFlags::GRATE)
        .union(BrushFlags::LADDER);
    brush_hulls_indexed(bsp, leaves)
        .into_iter()
        .chain(entity_hulls(bsp, leaves))
        .filter(|(i, _, _)| bsp.brushes[*i].flags.intersects(blocks))
        .map(|(_, h, _)| h)
        .collect()
}

/// The same brushes as planes (engine space), bevel planes included, for
/// exact swept-box collision.
/// Also the world's BSP tree over them, for the order traces meet
/// coincident faces in (a ladder flush with a player clip).
pub fn collision_brushes(bsp: &Bsp, leaves: &[RawLeaf]) -> (Vec<crate::map::MapBrush>, crate::map::MapBrushTree) {
    let world = brush_hulls_indexed(bsp, leaves);
    let index: std::collections::HashMap<usize, u32> =
        world.iter().enumerate().map(|(k, (i, _, _))| (*i, k as u32)).collect();
    let brushes = world
        .into_iter()
        .chain(entity_hulls(bsp, leaves))
        .map(|(i, points, planes)| map_brush(points, planes, bsp.brushes[i].flags.contains(BrushFlags::LADDER)))
        .collect();
    (brushes, brush_tree(bsp, leaves, &index))
}

/// The world's BSP tree (model 0) in engine space, its leaves listing
/// brushes as `index` numbers them (brushes not in `index` left out).
fn brush_tree(
    bsp: &Bsp,
    leaves: &[RawLeaf],
    index: &std::collections::HashMap<usize, u32>,
) -> crate::map::MapBrushTree {
    let nodes = bsp
        .nodes
        .iter()
        .map(|n| {
            let p = &bsp.planes[n.plane_index as usize];
            crate::map::BrushTreeNode {
                normal: to_engine_dir(p.normal),
                dist: p.dist * METERS_PER_UNIT,
                children: n.children,
            }
        })
        .collect();
    let leaves = leaves
        .iter()
        .map(|l| {
            bsp.leaf_brushes
                .iter()
                .skip(l.first_leaf_brush as usize)
                .take(l.leaf_brush_count as usize)
                .filter_map(|lb| index.get(&(lb.brush as usize)).copied())
                .collect()
        })
        .collect();
    crate::map::MapBrushTree {
        nodes,
        leaves,
        root: bsp.models.first().map_or(0, |m| m.head_node),
    }
}

/// Indices into `collision_brushes` of the brushes line traces pass
/// (fire's drop and line of sight: solid and window contents stop them;
/// player clips, grates and ladders don't).
pub fn trace_skip(bsp: &Bsp, leaves: &[RawLeaf]) -> Vec<usize> {
    let stops = BrushFlags::SOLID.union(BrushFlags::WINDOW);
    brush_hulls_indexed(bsp, leaves)
        .into_iter()
        .chain(entity_hulls(bsp, leaves))
        .enumerate()
        .filter(|(_, (i, _, _))| !bsp.brushes[*i].flags.intersects(stops))
        .map(|(n, _)| n)
        .collect()
}

/// Player-solid brushes of solid brush entities (doors, windows,
/// breakables) where they spawn, as `brush_hulls_indexed` gives them.
pub fn entity_hulls(bsp: &Bsp, leaves: &[RawLeaf]) -> Vec<(usize, Vec<[f32; 3]>, Vec<(Vec3, f32)>)> {
    brush_entities(bsp)
        .into_iter()
        .filter(|e| e.solid && !e.mover)
        .flat_map(|e| {
            brush_volumes_in(
                bsp,
                PLAYER_SOLID.union(BrushFlags::LADDER),
                model_brushes(bsp, leaves, e.model),
                Some(e.transform),
                false,
            )
        })
        .collect()
}

/// Hulls that block light: player-solid brushes minus sky brushes (rays
/// that reach the sky are lit).
pub fn shadow_hulls(bsp: &Bsp, leaves: &[RawLeaf]) -> Vec<Vec<[f32; 3]>> {
    brush_hulls_indexed(bsp, leaves)
        .into_iter()
        .filter(|(i, _, _)| {
            let b = &bsp.brushes[*i];
            !bsp.brush_sides[b.brush_side as usize..(b.brush_side + b.num_brush_sides) as usize]
                .iter()
                .any(|side| {
                    side.texture_info >= 0
                        && bsp
                            .texture_info(side.texture_info as usize)
                            .is_some_and(|t| t.flags.intersects(TextureFlags::SKY | TextureFlags::SKY2D))
                })
                && b.flags.intersects(BrushFlags::SOLID)
        })
        .map(|(_, h, _)| h)
        .collect()
}

/// `brush_hulls` with each hull's brush index (for diagnostics) and its
/// planes in engine space (outward normal, distance in meters).
pub fn brush_hulls_indexed(bsp: &Bsp, leaves: &[RawLeaf]) -> Vec<(usize, Vec<[f32; 3]>, Vec<(Vec3, f32)>)> {
    // Ladders block players too (you climb what you run into).
    brush_volumes(bsp, leaves, PLAYER_SOLID.union(BrushFlags::LADDER))
}

/// Water and slime brushes as volumes.
pub fn water_volumes(bsp: &Bsp, leaves: &[RawLeaf]) -> Vec<crate::map::MapWaterVolume> {
    brush_volumes(bsp, leaves, BrushFlags::WATER.union(BrushFlags::SLIME))
        .into_iter()
        .map(|(i, points, planes)| crate::map::MapWaterVolume {
            brush: map_brush(points, planes, false),
            slime: !bsp.brushes[i].flags.contains(BrushFlags::WATER),
        })
        .collect()
}

/// func_water_analog's water (public entity docs: a brush of water that
/// can move like func_movelinear), where it spawns: a swimmable volume
/// like the world's water. Its motion isn't followed (tech-debt).
pub fn analog_water_volumes(bsp: &Bsp, leaves: &[RawLeaf]) -> Vec<crate::map::MapWaterVolume> {
    let parse = |v: Option<&str>| {
        let n: Vec<f32> = v
            .unwrap_or("")
            .split_whitespace()
            .filter_map(|x| x.parse().ok())
            .collect();
        if n.len() >= 3 {
            Vec3::new(n[0], n[1], n[2])
        } else {
            Vec3::ZERO
        }
    };
    bsp.entities
        .iter()
        .filter(|e| {
            e.prop("classname")
                .is_some_and(|c| c.eq_ignore_ascii_case("func_water_analog"))
        })
        .filter_map(|e| {
            let model: usize = e.prop("model")?.strip_prefix('*')?.parse().ok()?;
            let (origin, angles) = (parse(e.prop("origin")), parse(e.prop("angles")));
            let rotation = Quat::from_rotation_z(angles.y.to_radians())
                * Quat::from_rotation_y(angles.x.to_radians())
                * Quat::from_rotation_x(angles.z.to_radians());
            Some(brush_volumes_in(
                bsp,
                BrushFlags::WATER.union(BrushFlags::SLIME),
                model_brushes(bsp, leaves, model),
                Some((rotation, origin)),
                false,
            ))
        })
        .flatten()
        .map(|(i, points, planes)| crate::map::MapWaterVolume {
            brush: map_brush(points, planes, false),
            slime: !bsp.brushes[i].flags.contains(BrushFlags::WATER),
        })
        .collect()
}

fn map_brush(points: Vec<[f32; 3]>, planes: Vec<(Vec3, f32)>, ladder: bool) -> crate::map::MapBrush {
    let (mut min, mut max) = (Vec3::splat(f32::MAX), Vec3::splat(f32::MIN));
    for p in &points {
        min = min.min(Vec3::from(*p));
        max = max.max(Vec3::from(*p));
    }
    crate::map::MapBrush {
        planes,
        min,
        max,
        ladder,
        surface: None,
    }
}

/// World brushes with any of `mask`'s contents, as hulls and planes.
fn brush_volumes(bsp: &Bsp, leaves: &[RawLeaf], mask: BrushFlags) -> Vec<(usize, Vec<[f32; 3]>, Vec<(Vec3, f32)>)> {
    brush_volumes_in(bsp, mask, world_brushes(bsp, leaves), None, false)
}

/// `brush_volumes` for a set of brushes, optionally placed by a Source-space
/// rotation and origin (brush entity models). `triggers`: keep brushes with
/// trigger textures (a trigger entity's own volume).
fn brush_volumes_in(
    bsp: &Bsp,
    mask: BrushFlags,
    brushes: std::collections::BTreeSet<usize>,
    transform: Option<(Quat, Vec3)>,
    triggers: bool,
) -> Vec<(usize, Vec<[f32; 3]>, Vec<(Vec3, f32)>)> {
    brush_volumes_source(bsp, mask, brushes, transform, triggers)
        .into_iter()
        .map(|(index, points, planes)| {
            // Source plane n.p = d becomes n'.p' = d * meters-per-unit,
            // with n' the same rotation of n as positions get.
            let engine_planes = planes
                .iter()
                .map(|(n, d)| {
                    (
                        to_engine_dir(vbsp::Vector { x: n.x, y: n.y, z: n.z }),
                        d * METERS_PER_UNIT,
                    )
                })
                .collect();
            (
                index,
                points
                    .iter()
                    .map(|p| to_engine(vbsp::Vector { x: p.x, y: p.y, z: p.z }).to_array())
                    .collect(),
                engine_planes,
            )
        })
        .collect()
}

/// The map's entities for the logic layer (`MapData::entities`): every
/// entity's keyvalues in lump order, with the brush volumes of brush
/// entities in entity space (Source units), local to the entity: trigger
/// volumes for triggers, player-solid brushes for movers.
pub fn map_entities(bsp: &Bsp, leaves: &[RawLeaf]) -> Vec<crate::map::MapEntity> {
    let movers: std::collections::BTreeSet<usize> =
        brush_entities(bsp).into_iter().filter(|e| e.mover).map(|e| e.entity).collect();
    bsp.entities
        .iter()
        .enumerate()
        .map(|(index, ent)| {
            let keyvalues: Vec<(String, String)> =
                ent.properties().map(|(k, v)| (k.to_string(), v.to_string())).collect();
            let class = ent.prop("classname").unwrap_or("");
            let model = ent
                .prop("model")
                .and_then(|m| m.strip_prefix('*'))
                .and_then(|m| m.parse::<usize>().ok())
                .filter(|m| *m > 0);
            // Buy zones, bomb sites and rescue zones are trigger-like
            // volumes too.
            let trigger = class.starts_with("trigger_")
                || matches!(class, "func_buyzone" | "func_bomb_target" | "func_hostage_rescue");
            let mover = movers.contains(&index);
            let hulls = match model {
                Some(model) if trigger || mover => brush_volumes_source(
                    bsp,
                    if trigger {
                        BrushFlags::all()
                    } else {
                        PLAYER_SOLID.union(BrushFlags::LADDER)
                    },
                    model_brushes(bsp, leaves, model),
                    Some((Quat::IDENTITY, Vec3::ZERO)),
                    trigger,
                )
                .into_iter()
                .map(|(_, points, planes)| crate::map::MapHull { planes, points })
                .collect(),
                _ => Vec::new(),
            };
            crate::map::MapEntity {
                keyvalues,
                hulls,
                mover,
                physics: None,
            }
        })
        .collect()
}

/// The physics collision lump (public BSP v20 description, lump 29): per
/// brush model, `int model, data size, key data size, solid count`, the
/// collision data, then a text block of "solid" sections carrying the
/// compiler's "mass" and "surfaceprop"; a model of -1 ends the list.
/// Returns each model's total mass (kg) and first surface property.
pub fn brush_model_physics(bytes: &[u8]) -> std::collections::HashMap<usize, (f32, String)> {
    let lump = super::ambient::lump(bytes, 29);
    let mut out = std::collections::HashMap::new();
    let int = |at: usize| lump.get(at..at + 4).map(|b| i32::from_le_bytes(b.try_into().unwrap()));
    let mut at = 0;
    while let (Some(model), Some(data), Some(keys)) = (int(at), int(at + 4), int(at + 8)) {
        if model < 0 || data < 0 || keys < 0 {
            break;
        }
        let text_at = at + 16 + data as usize;
        let Some(text) = lump.get(text_at..text_at + keys as usize) else { break };
        let text = String::from_utf8_lossy(text);
        let t = super::surfaceprops::tokens(&text);
        let (mut mass, mut surface) = (0.0f32, None);
        for w in t.windows(2) {
            if w[0].eq_ignore_ascii_case("mass") {
                mass += w[1].parse::<f32>().unwrap_or(0.0);
            } else if w[0].eq_ignore_ascii_case("surfaceprop") && surface.is_none() {
                surface = Some(w[1].clone());
            }
        }
        out.insert(model as usize, (mass, surface.unwrap_or_else(|| "default".into())));
        at = text_at + keys as usize;
    }
    out
}

/// func_physbox(_multiplayer) bodies (specs/source/physics_brushes.md 1-3):
/// the compiler's mass from the BSP (else the volume of its hulls times
/// the default surface's density), times "massScale", or an
/// "overridescript" mass; friction and elasticity from its surface
/// property; pinned by "Motion Disabled" or the enable thresholds; asleep
/// with "Start Asleep"; the multiplayer kind walked through and shoved by
/// players. Its mass goes to the logic too (`PROP_MASS_KEY`).
fn brush_bodies(bytes: &[u8], surfaces: &super::surfaceprops::SurfaceProps, data: &mut MapData) {
    const START_ASLEEP: u32 = 4096;
    const MOTION_DISABLED: u32 = 32768;
    let stored = brush_model_physics(bytes);
    for e in data.entities.iter_mut() {
        let class = e.classname().to_ascii_lowercase();
        if class != "func_physbox" && class != "func_physbox_multiplayer" {
            continue;
        }
        let num = |k: &str| e.get(k).and_then(|v| v.trim().parse::<f32>().ok()).unwrap_or(0.0);
        let model = e
            .get("model")
            .and_then(|m| m.strip_prefix('*'))
            .and_then(|m| m.parse::<usize>().ok());
        let (mut mass, surface) = match model.and_then(|m| stored.get(&m)) {
            Some((m, s)) => (*m, s.clone()),
            None => {
                let density = surfaces
                    .text("default", "density")
                    .and_then(|d| d.parse::<f32>().ok())
                    .unwrap_or(2000.0);
                let volume: f32 = e
                    .hulls
                    .iter()
                    .map(|h| {
                        let (lo, hi) = h
                            .points
                            .iter()
                            .fold((Vec3::MAX, Vec3::MIN), |(a, b), p| (a.min(*p), b.max(*p)));
                        let s = (hi - lo).max(Vec3::ZERO);
                        s.x * s.y * s.z
                    })
                    .sum();
                ((volume * density * 1.638_706_4e-5).min(50_000.0), "default".to_string())
            }
        };
        let scale = num("massScale");
        if scale > 0.0 {
            mass *= scale;
        }
        // "overridescript": comma-separated key,value pairs.
        if let Some(script) = e.get("overridescript") {
            let parts: Vec<&str> = script.split(',').map(str::trim).collect();
            for kv in parts.chunks(2) {
                if let [k, v] = kv
                    && k.eq_ignore_ascii_case("mass")
                {
                    mass = v.parse().unwrap_or(mass);
                }
            }
        }
        let flags = e.get("spawnflags").and_then(|v| v.trim().parse::<u32>().ok()).unwrap_or(0);
        let s = surfaces.get(&surface);
        let mass = mass.clamp(0.1, 50_000.0);
        e.physics = Some(crate::map::MapPhysics {
            mass,
            friction: s.friction,
            elasticity: s.elasticity,
            // physics_brushes.md constants: physboxes damp by 0.1 both
            // ways (the thin-rod rule isn't applied: our bodies are
            // boxes of hulls).
            damping: 0.1,
            rotdamping: 0.1,
            push: if class == "func_physbox_multiplayer" {
                crate::map::PushAway::NonSolid
            } else {
                crate::map::PushAway::Collide
            },
            frozen: flags & MOTION_DISABLED != 0 || num("damagetoenablemotion") > 0.0 || num("forcetoenablemotion") > 0.0,
            asleep: flags & START_ASLEEP != 0,
        });
        e.keyvalues.push((crate::map::entities::PROP_MASS_KEY.into(), mass.to_string()));
    }
}

/// `brush_volumes_in` in Source space (units, Z up): corner points and
/// planes.
fn brush_volumes_source(
    bsp: &Bsp,
    mask: BrushFlags,
    brushes: std::collections::BTreeSet<usize>,
    transform: Option<(Quat, Vec3)>,
    triggers: bool,
) -> Vec<(usize, Vec<Vec3>, Vec<(Vec3, f32)>)> {
    const EPS: f32 = 0.01;
    // Plane index (either side) -> corners of displacement base faces on it.
    let mut disp_bases: std::collections::HashMap<u16, Vec<Vec<Vec3>>> = Default::default();
    let Some(world) = bsp.models().next() else {
        return Vec::new();
    };
    for face in world.faces() {
        if face.displacement().is_none() {
            continue;
        }
        let corners: Vec<Vec3> = face
            .vertices()
            .map(|v| Vec3::new(v.position.x, v.position.y, v.position.z))
            .collect();
        disp_bases.entry(face.plane_num & !1).or_default().push(corners);
    }
    let mut out = Vec::new();
    for index in brushes {
        let brush = &bsp.brushes[index];
        if !triggers && !brush.flags.intersects(mask) {
            continue;
        }
        let first = brush.brush_side as usize;
        let sides = &bsp.brush_sides[first..first + brush.num_brush_sides as usize];
        // Trigger volumes (buy zones, soundscapes) can carry solid contents
        // and appear in the world tree, but players never collide with them.
        let trigger = sides.iter().any(|side| {
            side.texture_info >= 0
                && bsp.texture_info(side.texture_info as usize).is_some_and(|t| {
                    t.flags.intersects(TextureFlags::TRIGGER) || t.name().eq_ignore_ascii_case("tools/toolstrigger")
                })
        });
        if trigger && !triggers {
            continue;
        }
        let planes: Vec<(Vec3, f32)> = sides
            .iter()
            .filter_map(|side| bsp.plane(side.plane as usize))
            .map(|p| {
                let n = Vec3::new(p.normal.x, p.normal.y, p.normal.z);
                match transform {
                    Some((r, o)) => {
                        let n = r * n;
                        (n, p.dist + n.dot(o))
                    }
                    None => (n, p.dist),
                }
            })
            .collect();
        // A brush carrying a displacement collides as the displacement
        // surface (added as triangles), not as its original volume. Compiled
        // maps don't record which brush that was (the brush side field is
        // always 0), so match a side's plane and the displacement's base
        // face, all of whose corners lie on that side. (Only its centre is
        // not enough: dust2's walls stand on floor displacements that share
        // their bottom plane and reach under them.)
        let carries_displacement = transform.is_none()
            && sides.iter().any(|side| {
                disp_bases.get(&(side.plane & !1)).is_some_and(|faces| {
                    faces
                        .iter()
                        .any(|corners| corners.iter().all(|c| planes.iter().all(|(n, d)| n.dot(*c) <= d + 1.0)))
                })
            });
        if carries_displacement {
            continue;
        }
        let mut points: Vec<Vec3> = Vec::new();
        for i in 0..planes.len() {
            for j in i + 1..planes.len() {
                for k in j + 1..planes.len() {
                    let (n1, d1) = planes[i];
                    let (n2, d2) = planes[j];
                    let (n3, d3) = planes[k];
                    let denom = n1.dot(n2.cross(n3));
                    if denom.abs() < 1e-6 {
                        continue;
                    }
                    let p = (n2.cross(n3) * d1 + n3.cross(n1) * d2 + n1.cross(n2) * d3) / denom;
                    let inside = planes.iter().all(|(n, d)| n.dot(p) <= d + EPS);
                    if inside && !points.iter().any(|q| q.distance_squared(p) < EPS) {
                        points.push(p);
                    }
                }
            }
        }
        if points.len() >= 4 {
            out.push((index, points, planes));
        }
    }
    out
}

fn parse_vector(s: &str) -> Option<vbsp::Vector> {
    let mut it = s.split_whitespace().map(|p| p.parse::<f32>());
    let (x, y, z) = (it.next()?.ok()?, it.next()?.ok()?, it.next()?.ok()?);
    Some(vbsp::Vector { x, y, z })
}

/// The cubemap lump's sample origins (lump 42, 16 bytes each: three i32
/// coordinates and a size).
fn cubemap_samples(bytes: &[u8]) -> Vec<[i32; 3]> {
    let lump = |i: usize| -> Option<(usize, usize)> {
        let at = 8 + i * 16;
        let off = i32::from_le_bytes(bytes.get(at..at + 4)?.try_into().ok()?);
        let len = i32::from_le_bytes(bytes.get(at + 4..at + 8)?.try_into().ok()?);
        Some((off.max(0) as usize, len.max(0) as usize))
    };
    let Some((off, len)) = lump(42) else { return Vec::new() };
    let Some(data) = bytes.get(off..off + len) else {
        return Vec::new();
    };
    data.chunks_exact(16)
        .map(|c| {
            let i = |k: usize| i32::from_le_bytes(c[k * 4..k * 4 + 4].try_into().unwrap());
            [i(0), i(1), i(2)]
        })
        .collect()
}

#[cfg(test)]
mod tonemap_tests {
    use super::*;
    use crate::map::MapEntity;

    /// Latin-1 bytes in the entity lump (a compile-tool comment, a server
    /// message) become `?`, same length, so vbsp reads the map.
    #[test]
    fn text_lumps_become_utf8_in_place() {
        let text = b"{ \"classname\" \"worldspawn\" \"comment\" \"E:\xd1team \x99\" }\0";
        let mut bytes = vec![0u8; 8 + 64 * 16];
        let ofs = bytes.len() as i32;
        bytes[8..12].copy_from_slice(&ofs.to_le_bytes());
        bytes[12..16].copy_from_slice(&(text.len() as i32).to_le_bytes());
        bytes.extend_from_slice(text);
        let out = text_lumps_utf8(bytes.clone());
        assert_eq!(out.len(), bytes.len());
        let lump = &out[ofs as usize..];
        assert!(std::str::from_utf8(lump).is_ok());
        assert!(String::from_utf8_lossy(lump).contains("E:?team ?"));
    }

    fn entity(kv: &[(&str, &str)]) -> MapEntity {
        MapEntity {
            keyvalues: kv.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect(),
            ..default()
        }
    }

    #[test]
    fn tonemap_controller_follows_map_start_outputs() {
        // de_nuke's set-up (commas), plus an ESC-separated output and one
        // aimed at something else.
        let entities = [
            entity(&[("classname", "env_tonemap_controller"), ("targetname", "tonemap_global")]),
            entity(&[
                ("classname", "logic_auto"),
                ("OnMapSpawn", "tonemap_global,SetAutoExposureMin,0,0,-1"),
                ("OnMapSpawn", "tonemap_global,SetAutoExposureMax,2.5,0,-1"),
                ("OnMapSpawn", "tonemap_global\u{1b}SetBloomScale\u{1b}.6\u{1b}0\u{1b}-1"),
                ("OnMapSpawn", "other,SetAutoExposureMax,9,0,-1"),
            ]),
        ];
        let t = tonemap_controller(&entities);
        assert_eq!((t.exposure_min, t.exposure_max, t.bloom_scale), (0.0, 2.5, 0.6));
        // No controller: the game's defaults.
        assert_eq!(tonemap_controller(&entities[1..]), DEFAULT_TONEMAP);
    }

    #[test]
    fn hdr_levels_change_the_look_only_above_zero() {
        let entities = [
            entity(&[("classname", "env_tonemap_controller"), ("targetname", "tonemap")]),
            entity(&[("classname", "logic_auto"), ("OnMapSpawn", "tonemap,SetAutoExposureMax,1,0.1,-1")]),
        ];
        let ldr = source_look_level(0, &entities);
        assert!(ldr.hdr.is_none() && ldr.source_ldr_lightmaps);
        let bloom = source_look_level(1, &entities);
        assert!(bloom.source_ldr_lightmaps, "level 1 keeps LDR lighting");
        assert_eq!(bloom.hdr.as_ref().map(|h| h.exposure), Some(None));
        let hdr = source_look_level(2, &entities);
        assert!(!hdr.source_ldr_lightmaps);
        assert_eq!(
            hdr.hdr,
            Some(crate::map::MapHdr {
                exposure: Some((0.5, 1.0)),
                bloom_scale: 1.0,
                rate: 1.0,
            })
        );
    }
}
