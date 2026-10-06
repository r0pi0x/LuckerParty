//! CS:S maps (BSP v20) to neutral `MapData`. Parsing is the `vbsp` crate;
//! this module owns Source's conventions: Z-up inches to Y-up meters, which
//! surfaces draw and collide, and which entities are spawns.

use std::collections::BTreeMap;

use bevy::prelude::*;
use vbsp::{BrushFlags, Bsp, TextureFlags};

use super::{
    ambient::RawLeaf,
    lightmap::{self, AtlasBuilder},
    material::MaterialLoader,
};
use crate::{
    core::Team,
    map::{MapData, MapMesh},
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
    let path = format!("maps/{name}.bsp");
    let bytes = mount.read(&path).map_err(|e| format!("{path}: {e}"))?;
    let bsp = Bsp::read(&bytes).map_err(|e| format!("{path}: {e}"))?;
    let (mut data, layout) = convert(&bsp, &bytes, name);

    let mut materials = MaterialLoader::new(&bsp, mount);
    for mesh in &mut data.meshes {
        let r = materials.resolve(&mesh.material);
        mesh.texture = r.texture;
        mesh.normal_map = r.normal_map;
        mesh.blend = r.blend;
        mesh.detail = r.detail;
        if mesh.blend.is_none() {
            mesh.blend_weights.clear();
        }
        mesh.alpha = r.alpha;
        mesh.double_sided = r.double_sided;
        mesh.surface = r.surfaceprop;
        mesh.envmap = r.envmap;
    }
    data.sky_vis = Some(sky_vis(&bsp, &bytes));
    let lighting = super::ambient::MapLighting::read(&bytes);
    let occluders = super::ambient::Occluders::new(
        &shadow_hulls(&bsp, &super::ambient::raw_leaves(&bytes)),
        (&data.collision_positions, &data.collision_indices),
    );
    super::props::add_static_props(&bsp, &mut materials, &lighting, &occluders, &mut data);
    super::ropes::add_ropes(&bsp, &mut materials, &lighting, &occluders, &mut data);
    super::sprites::add_sprites(&bsp, &mut materials, &mut data);
    super::dust::add_dust(&bsp, &mut materials, &mut data);
    let surfaces = super::surfaceprops::SurfaceProps::load(&mut materials);
    // Character bodies: a terrorist and a counter-terrorist model (CS:S
    // teams 2 and 3; ours are 1 and 2), the CT one for anyone else.
    for (path, team) in [
        ("models/player/t_phoenix.mdl", Some(crate::core::Team(1))),
        ("models/player/ct_urban.mdl", None),
    ] {
        match super::props::load_character(&mut materials, path, team) {
            Ok(c) => data.characters.push(c),
            Err(e) => data.warnings.push(e),
        }
    }
    data.decals = super::decals::impact_decals(&mut materials);
    data.hud = super::hud::load(&mut materials).map(std::sync::Arc::new);
    // What characters hold: the weapons' world models.
    if let Some(skeleton) = data.characters.first().map(|c| c.bones.clone()) {
        for (weapon, path) in super::weapons::WORLD_MODELS {
            match super::props::load_held(&mut materials, path, weapon, &skeleton) {
                Ok(h) => data.held.push(h),
                Err(e) => data.warnings.push(e),
            }
        }
    }
    // What the local player sees of them: the view models.
    for (weapon, path) in super::weapons::VIEW_MODELS {
        match super::props::load_view_model(&mut materials, path, weapon, super::view_anim::fov()) {
            Ok(v) => data.view_models.push(v),
            Err(e) => data.warnings.push(e),
        }
    }
    let mut sounds = super::sound::load(&mut materials, name, &surfaces);
    super::soundscape::load(&mut materials, &bsp, name, &mut sounds);
    data.sounds = std::sync::Arc::new(sounds);
    if let Some(bytes) = materials.read(&format!("maps/{}.nav", name.to_lowercase())) {
        match super::nav::parse(&bytes) {
            Ok((nav, _)) => data.nav = Some(std::sync::Arc::new(nav)),
            Err(e) => data.warnings.push(e),
        }
    }
    super::decals::add_decals(&bsp, &layout, &mut materials, &mut data);
    super::overlays::add_overlays(&bsp, &bytes, &layout, &mut materials, &mut data);
    super::sky::add_sky(&bsp, &mut materials, &mut data);
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
    Ok(data)
}

/// Triangles of a face in Source space, each vertex with its lightmap
/// coordinate in luxels (see `lightmap::luxel_coords`).
///
/// Flat faces project through the face's lightmap vectors. Displacements
/// don't: their samples follow the displacement grid, with the first
/// lightmap axis along the grid's second axis. This was established by
/// measurement on de_dust2: that mapping makes lighting agree where
/// neighbouring displacements meet (mean mismatch 1%, versus 48% for
/// projection; tests/map_de_dust2.rs checks it).
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
    out
}

/// Where each world face's lighting landed in the atlas, for things that
/// share a face's lighting later (decals).
pub struct LightmapLayout {
    /// Per world face (in `models[0].faces()` order): its block, if lit.
    pub face_slots: Vec<Option<usize>>,
    pub placements: Vec<lightmap::Placement>,
    /// Block for drawn faces without lighting.
    pub white: usize,
}

/// Geometry, collision, lightmaps and spawns, without materials.
/// `bytes` is the whole BSP file (for lumps vbsp doesn't keep as stored).
pub fn convert(bsp: &Bsp, bytes: &[u8], name: &str) -> (MapData, LightmapLayout) {
    let lighting = lightmap::lighting_lump(bytes);
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
    let faces = world
        .faces()
        .map(|f| (f, None))
        .chain(entities.iter().filter(|e| e.drawn).flat_map(|e| {
            bsp.models()
                .nth(e.model)
                .into_iter()
                .flat_map(|m| m.faces().collect::<Vec<_>>())
                .map(|f| (f, Some(e.transform)))
        }));
    for (face, transform) in faces {
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
        let skybox = bounds.as_ref().is_some_and(|b| !b.contains(centre));
        let tex = face.texture();
        let flags = tex.flags;
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
        // Re-wind triangles to face the plane normal.
        let tris: Vec<[(vbsp::Vector, Vec2, f32); 3]> = face_triangles_blend(&face)
            .into_iter()
            .map(|t| {
                let [a, b, c] = t.map(|(v, _, _)| to_engine(place(v)));
                if (b - a).cross(c - a).dot(face_normal) < 0.0 {
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
        if in_world {
            *face_slots.last_mut().unwrap() = slot;
        }
        let material = tex.name().to_lowercase();
        // Meshes are per material and per part (world or 3D skybox).
        let key = if skybox {
            format!("{material}\u{1}skybox")
        } else {
            material.clone()
        };
        let mesh = by_material.entry(key.clone()).or_insert_with(|| MapMesh {
            material: material.clone(),
            skybox,
            color: tex.debug_color(),
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
    data.collision_brushes = collision_brushes(bsp, &leaves);
    data.water = water_volumes(bsp, &leaves);

    let (lightmap, placements, white) = atlas.build();
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
}

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
    let mut out = Vec::new();
    for ent in bsp.entities.iter() {
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
        let drawn = render_mode != 10 && !(class == "func_brush" && start_disabled);
        let solid = !NOT_SOLID.contains(&class)
            // func_brush "Solidity": 0 toggle (with the brush), 1 never, 2 always.
            && !(class == "func_brush" && (num("Solidity") == Some(1.0) || (start_disabled && num("Solidity") != Some(2.0))))
            // func_rotating spawnflag 64: not solid.
            && !(class == "func_rotating" && (num("spawnflags").unwrap_or(0.0) as i32) & 64 != 0);
        out.push(BrushEntity {
            model,
            transform: (rotation, origin),
            drawn,
            solid,
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
    }
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

/// Every player-solid world and brush entity brush as a convex hull in engine space: the
/// corners where three of its planes meet and no other plane cuts them off.
pub fn brush_hulls(bsp: &Bsp, leaves: &[RawLeaf]) -> Vec<Vec<[f32; 3]>> {
    brush_hulls_indexed(bsp, leaves)
        .into_iter()
        .chain(entity_hulls(bsp, leaves))
        .map(|(_, h, _)| h)
        .collect()
}

/// The same brushes as planes (engine space), bevel planes included, for
/// exact swept-box collision.
pub fn collision_brushes(bsp: &Bsp, leaves: &[RawLeaf]) -> Vec<crate::map::MapBrush> {
    brush_hulls_indexed(bsp, leaves)
        .into_iter()
        .chain(entity_hulls(bsp, leaves))
        .map(|(i, points, planes)| map_brush(points, planes, bsp.brushes[i].flags.contains(BrushFlags::LADDER)))
        .collect()
}

/// Player-solid brushes of solid brush entities (doors, windows,
/// breakables) where they spawn, as `brush_hulls_indexed` gives them.
pub fn entity_hulls(bsp: &Bsp, leaves: &[RawLeaf]) -> Vec<(usize, Vec<[f32; 3]>, Vec<(Vec3, f32)>)> {
    brush_entities(bsp)
        .into_iter()
        .filter(|e| e.solid)
        .flat_map(|e| {
            brush_volumes_in(
                bsp,
                PLAYER_SOLID.union(BrushFlags::LADDER),
                model_brushes(bsp, leaves, e.model),
                Some(e.transform),
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
    brush_volumes_in(bsp, mask, world_brushes(bsp, leaves), None)
}

/// `brush_volumes` for a set of brushes, optionally placed by a Source-space
/// rotation and origin (brush entity models).
fn brush_volumes_in(
    bsp: &Bsp,
    mask: BrushFlags,
    brushes: std::collections::BTreeSet<usize>,
    transform: Option<(Quat, Vec3)>,
) -> Vec<(usize, Vec<[f32; 3]>, Vec<(Vec3, f32)>)> {
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
        if !brush.flags.intersects(mask) {
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
        if trigger {
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
            out.push((
                index,
                points
                    .iter()
                    .map(|p| to_engine(vbsp::Vector { x: p.x, y: p.y, z: p.z }).to_array())
                    .collect(),
                engine_planes,
            ));
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
