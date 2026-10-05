//! The Map slot's neutral data: what any game's map importer produces and
//! the engine spawns. Units are meters, Y up (README: mounts own each game's
//! units and axes; the engine sees only this).

use std::sync::Arc;

use avian3d::prelude::*;
use bevy::{
    asset::RenderAssetUsages,
    image::{ImageAddressMode, ImageFilterMode, ImageSampler, ImageSamplerDescriptor},
    mesh::Indices,
    prelude::*,
    render::render_resource::{Extent3d, PrimitiveTopology, TextureDimension, TextureFormat},
};

use crate::core::{SpawnPoint, Team};

pub mod rope_material;
pub mod world_material;

use rope_material::{RopeMaterial, RopeParams};
use world_material::{WorldMaterial, WorldParams};

/// Signs for normal maps' red and green channels in the world material.
/// Chosen with refcmp against CS:S on de_dust2: x -1, g -1 gave the lowest
/// pixel difference of the four combinations (0.531 vs 0.538 without bump
/// mapping) and grooves lit on top as in the game. The margin between
/// combinations is small; revisit with more views.
const NORMAL_G_SIGN: f32 = -1.0;
const NORMAL_X_SIGN: f32 = -1.0;

/// An RGBA8 texture in sRGB, top row first.
#[derive(Clone, Debug)]
pub struct MapTexture {
    /// Source path, e.g. `materials/de_dust/sitebwall01.vtf`.
    pub name: String,
    /// Color data (sRGB) or data such as normal maps (linear).
    pub srgb: bool,
    /// Smaller mip levels shipped with the texture (RGBA8, halving each
    /// time). Empty: generate them.
    pub mips: Vec<Vec<u8>>,
    pub width: u32,
    pub height: u32,
    pub rgba8: Vec<u8>,
}

/// How a surface's alpha is used.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub enum MapAlpha {
    #[default]
    Opaque,
    /// Cut out below this alpha.
    Mask(f32),
    Blend,
}

/// Triangles sharing one material.
#[derive(Clone, Debug, Default)]
pub struct MapMesh {
    /// The source game's material name, e.g. `de_dust/sitebwall01`.
    pub material: String,
    /// Part of the 3D skybox (drawn by the sky camera, see `MapSkyCamera`).
    pub skybox: bool,
    pub positions: Vec<[f32; 3]>,
    pub normals: Vec<[f32; 3]>,
    /// Texture coordinates in texture repeats (1.0 = one texture width).
    pub uvs: Vec<[f32; 2]>,
    /// Counter-clockwise triangles when seen from the front.
    pub indices: Vec<u32>,
    /// Flat color, used when there's no texture.
    pub color: [u8; 3],
    /// Index into `MapData::textures`.
    pub texture: Option<usize>,
    pub alpha: MapAlpha,
    pub double_sided: bool,
    /// Tangent-space normal map (index into `MapData::textures`, linear).
    pub normal_map: Option<usize>,
    /// Per-vertex coordinates into `MapData::lightmap` (0..1); empty when
    /// the map has no baked lighting.
    pub lightmap_uvs: Vec<[f32; 2]>,
}

/// Baked lighting atlas: linear RGB, where 1.0 shows a texture at its own
/// brightness (above 1.0 is overbright).
#[derive(Clone, Debug, Default)]
pub struct MapLightmap {
    pub width: u32,
    pub height: u32,
    /// Row-major, top row first.
    pub rgb: Vec<[f32; 3]>,
    /// Directional lightmaps for normal-mapped surfaces (same layout), one
    /// per basis direction of radiosity normal mapping. Surfaces without
    /// them repeat `rgb`.
    pub bumped: Option<[Vec<[f32; 3]>; 3]>,
}

/// A reusable model (e.g. a window frame), in its own space: meters, Y up.
#[derive(Clone, Debug, Default)]
pub struct MapModel {
    pub meshes: Vec<MapMesh>,
    /// Collision box in model space (min, max), for box-solid props.
    pub bounds: (Vec3, Vec3),
}

/// How players collide with a prop.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum PropSolid {
    None,
    /// The model's bounds as a box, rotated with the prop.
    Box,
    /// The model's triangles.
    Mesh,
}

/// Lighting for something without a lightmap, sampled at one point: light
/// arriving from six axis directions (+X, -X, +Y, -Y, +Z, -Z) plus
/// directional lights (direction toward the light, color), all in lightmap
/// units. The same model Source uses for props and characters.
#[derive(Clone, Debug, Default)]
pub struct LightProbe {
    pub cube: [Vec3; 6],
    pub lights: Vec<(Vec3, Vec3)>,
}

impl LightProbe {
    /// Light on a surface with normal `n`.
    pub fn eval(&self, n: Vec3) -> Vec3 {
        let c = &self.cube;
        let pick = |v: f32, pos: Vec3, neg: Vec3| if v >= 0.0 { pos } else { neg };
        let ambient =
            pick(n.x, c[0], c[1]) * n.x * n.x + pick(n.y, c[2], c[3]) * n.y * n.y + pick(n.z, c[4], c[5]) * n.z * n.z;
        self.lights
            .iter()
            .fold(ambient, |acc, (dir, color)| acc + *color * n.dot(*dir).max(0.0))
    }
}

/// A placed model.
#[derive(Clone, Debug)]
pub struct MapProp {
    /// Index into `MapData::models`.
    pub model: usize,
    pub translation: Vec3,
    pub rotation: Quat,
    pub solid: PropSolid,
    /// Part of the 3D skybox.
    pub skybox: bool,
    /// Baked lighting; lit by the scene's lights when absent.
    pub lighting: Option<LightProbe>,
}

#[derive(Clone, Debug, Default)]
pub struct MapData {
    pub name: String,
    pub meshes: Vec<MapMesh>,
    pub textures: Vec<MapTexture>,
    pub lightmap: Option<MapLightmap>,
    pub models: Vec<MapModel>,
    pub props: Vec<MapProp>,
    /// Collision triangles (surfaces with no solid volume, e.g. terrain).
    pub collision_positions: Vec<[f32; 3]>,
    pub collision_indices: Vec<[u32; 3]>,
    /// Solid convex volumes, each as its corner points.
    pub collision_hulls: Vec<Vec<[f32; 3]>>,
    /// Feet positions.
    pub spawns: Vec<(Vec3, Option<Team>)>,
    /// Things the importer couldn't load (missing materials etc.).
    pub warnings: Vec<String>,
    /// How the source game presents the map, so it can be reproduced.
    pub look: MapLook,
    pub sky: Option<MapSky>,
    pub sky_camera: Option<MapSkyCamera>,
    pub ropes: Vec<MapRope>,
}

/// A rope or cable: a line of points drawn as a strip that always faces the
/// camera (the rope shader widens it per view). Source's Cable look:
/// texture x normal map's blue squared x per-point light.
#[derive(Clone, Debug, Default)]
pub struct MapRope {
    /// Engine space, meters.
    pub points: Vec<Vec3>,
    /// Texture V per point; U runs across the strip.
    pub v: Vec<f32>,
    /// Light per point, lightmap units (0-1).
    pub light: Vec<Vec3>,
    /// Full strip width, meters.
    pub width: f32,
    pub texture: Option<usize>,
    pub normal_map: Option<usize>,
    /// Texture and normal map of a translucent strip drawn behind the rope
    /// that keeps thin, distant ropes visible (fake anti-aliasing).
    pub back: Option<(Option<usize>, Option<usize>)>,
}

/// A 3D skybox: a miniature scene (meshes and props marked `skybox`) drawn
/// behind the world from `origin + eye / scale`, so it appears `scale` times
/// larger and moves with the player.
#[derive(Clone, Debug)]
pub struct MapSkyCamera {
    /// Engine space, meters.
    pub origin: Vec3,
    pub scale: f32,
    pub fog: Option<MapFog>,
}

/// Linear distance fog, distances in meters as seen in the world.
#[derive(Clone, Debug)]
pub struct MapFog {
    /// sRGB color.
    pub color: [f32; 3],
    pub start: f32,
    pub end: f32,
}

/// A sky cubemap: per engine face (+X, -X, +Y, -Y, +Z, -Z), an index into
/// `MapData::textures` and an orientation (0-3 clockwise quarter turns, 4-7
/// the same after mirroring horizontally).
#[derive(Clone, Debug)]
pub struct MapSky {
    pub faces: [(usize, u8); 6],
}

/// Presentation choices that differ between games, measured against the
/// original (see `refcmp`).
#[derive(Clone, Debug)]
pub struct MapLook {
    /// Multiplier on all baked light (lightmaps and prop probes).
    pub light_scale: f32,
    /// Blend between mip levels (trilinear) or snap to the nearest one.
    pub trilinear: bool,
    /// Anisotropic filtering level; 1 = off.
    pub anisotropy: u16,
    /// Apply a filmic tonemapper (off: plain clamped output, like LDR games).
    pub tonemapping: bool,
}

impl Default for MapLook {
    fn default() -> Self {
        Self {
            light_scale: 1.0,
            trilinear: true,
            anisotropy: 8,
            tonemapping: true,
        }
    }
}

impl MapData {
    pub fn triangle_count(&self) -> usize {
        self.meshes.iter().map(|m| m.indices.len() / 3).sum()
    }

    pub fn bounds(&self) -> (Vec3, Vec3) {
        let mut lo = Vec3::splat(f32::MAX);
        let mut hi = Vec3::splat(f32::MIN);
        for p in self.meshes.iter().flat_map(|m| &m.positions) {
            lo = lo.min(Vec3::from(*p));
            hi = hi.max(Vec3::from(*p));
        }
        (lo, hi)
    }
}

/// Spawns a loaded map: collision always, meshes only when rendering is
/// present (same pattern as the greybox, so maps load in headless tests).
pub struct MapPlugin {
    pub data: Arc<MapData>,
    pub view: MapDebugView,
}

/// Debug renders, used to compare against reference screenshots.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub enum MapDebugView {
    #[default]
    Normal,
    /// White surfaces: only baked lighting (lightmaps, prop probes),
    /// multiplied by `scale` (e.g. 0.25 so overbright light fits in 0..1).
    Lighting { scale: f32 },
    /// Textures only, unlit.
    Albedo,
}

impl MapPlugin {
    pub fn new(data: MapData) -> Self {
        Self {
            data: Arc::new(data),
            view: MapDebugView::Normal,
        }
    }
}

#[derive(Resource)]
struct PendingMap(Arc<MapData>, MapDebugView);

/// Render layer for 3D skybox content.
pub const SKYBOX_LAYER: usize = 1;

/// Draws the 3D skybox; follows the main camera.
#[derive(Component)]
pub struct SkyboxCamera;

#[derive(Resource, Clone)]
struct SkyCameraInfo(MapSkyCamera);

/// The map's sky cubemap, for cameras to show.
#[derive(Resource, Clone)]
pub struct MapSkybox(pub Handle<Image>);

/// The loaded map's presentation settings, for cameras to follow.
#[derive(Resource, Clone, Debug)]
pub struct ActiveMapLook(pub MapLook);

/// Marks every entity belonging to the loaded map.
#[derive(Component)]
pub struct MapPart;

impl Plugin for MapPlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(PendingMap(self.data.clone(), self.view))
            .insert_resource(ActiveMapLook(self.data.look.clone()))
            .insert_resource(GlobalAmbientLight {
                brightness: 600.0,
                // Baked lighting already includes the map's ambient light.
                affects_lightmapped_meshes: false,
                ..default()
            })
            .add_systems(Startup, spawn_map)
            .add_systems(Update, attach_sky)
            .add_systems(
                PostUpdate,
                follow_sky_camera.before(bevy::transform::TransformSystems::Propagate),
            );
    }
}

/// Bevy adds `lightmap * lightmap_exposure` as light, then applies the
/// camera's exposure. This cancels the default camera exposure (EV100 9.7)
/// so a lightmap value of 1.0 shows a texture at its own brightness.
const LIGHTMAP_EXPOSURE: f32 = 1.2 * 831.746_4; // 1.2 * 2^9.7

/// Height of the capsule center above the feet, so spawns start standing.
const SPAWN_LIFT: f32 = 1.0;

fn spawn_map(
    mut commands: Commands,
    pending: Res<PendingMap>,
    mut meshes: Option<ResMut<Assets<Mesh>>>,
    mut materials: Option<ResMut<Assets<StandardMaterial>>>,
    mut images: Option<ResMut<Assets<Image>>>,
    mut world_materials: Option<ResMut<Assets<WorldMaterial>>>,
    mut rope_materials: Option<ResMut<Assets<RopeMaterial>>>,
) {
    let data = &pending.0;
    let view = pending.1;
    info!(
        "spawning map: world material {}, lightmap {}",
        if world_materials.is_some() {
            "available"
        } else {
            "missing"
        },
        data.lightmap.is_some()
    );
    let root = commands
        .spawn((
            Name::new(format!("Map {}", data.name)),
            MapPart,
            Transform::default(),
            Visibility::default(),
        ))
        .id();

    if !data.collision_indices.is_empty() {
        commands.entity(root).with_child((
            Name::new("Map collision (surfaces)"),
            MapPart,
            RigidBody::Static,
            Collider::trimesh(
                data.collision_positions.iter().map(|p| Vec3::from(*p)).collect(),
                data.collision_indices.clone(),
            ),
            Transform::default(),
        ));
    }
    if !data.collision_hulls.is_empty() {
        let hulls: Vec<_> = data
            .collision_hulls
            .iter()
            .filter_map(|h| Collider::convex_hull(h.iter().map(|p| Vec3::from(*p)).collect()))
            .map(|c| (Vec3::ZERO, Quat::IDENTITY, c))
            .collect();
        commands.entity(root).with_child((
            Name::new("Map collision (solids)"),
            MapPart,
            RigidBody::Static,
            Collider::compound(hulls),
            Transform::default(),
        ));
    }

    // Prop models: one collider per model (shared by its placements), and
    // render handles when rendering exists.
    let model_colliders: Vec<Option<Collider>> = data.models.iter().map(model_collider).collect();
    let mut model_parts: Vec<Vec<(Handle<Mesh>, Handle<StandardMaterial>)>> = Vec::new();
    let mut lit_model_materials: Vec<Vec<Handle<StandardMaterial>>> = Vec::new();

    if let (Some(meshes), Some(materials), Some(images)) = (meshes.as_mut(), materials.as_mut(), images.as_mut()) {
        let textures: Vec<Handle<Image>> = data
            .textures
            .iter()
            .map(|t| images.add(to_image(t, &data.look)))
            .collect();
        let lightmap = data
            .lightmap
            .as_ref()
            .map(|l| images.add(lightmap_image(&l.rgb, l.width, l.height)));
        let bumped_lightmaps: Option<[Handle<Image>; 3]> = data.lightmap.as_ref().and_then(|l| {
            let b = l.bumped.as_ref()?;
            Some(std::array::from_fn(|i| {
                images.add(lightmap_image(&b[i], l.width, l.height))
            }))
        });
        if let Some(sky) = &data.sky
            && view == MapDebugView::Normal
        {
            commands.insert_resource(MapSkybox(images.add(sky_image(sky, &data.textures))));
        }
        if let Some(cam) = &data.sky_camera
            && view == MapDebugView::Normal
        {
            commands.insert_resource(SkyCameraInfo(cam.clone()));
        }
        for m in &data.meshes {
            let lit = lightmap.as_ref().filter(|_| m.lightmap_uvs.len() == m.positions.len());
            // Lightmapped world surfaces: Source-style texture x baked light.
            if let (Some(lm), Some(world_materials), false) =
                (lit, world_materials.as_mut(), m.material.starts_with("decal:"))
            {
                let [r, g, b] = m.color;
                // Calibration hook: MASHUP_NORMAL_G_SIGN overrides the sign;
                // 0 turns radiosity normal mapping off.
                let g_sign = std::env::var("MASHUP_NORMAL_G_SIGN")
                    .ok()
                    .and_then(|v| v.parse::<f32>().ok())
                    .unwrap_or(NORMAL_G_SIGN);
                let bumped = m.normal_map.is_some() && bumped_lightmaps.is_some() && g_sign != 0.0;
                let material = WorldMaterial {
                    params: WorldParams {
                        base_color: if m.texture.is_some() {
                            Vec4::ONE
                        } else {
                            Vec4::new(r as f32 / 255.0, g as f32 / 255.0, b as f32 / 255.0, 1.0)
                        },
                        light_scale: data.look.light_scale,
                        bumped: if bumped { 1.0 } else { 0.0 },
                        normal_g_sign: g_sign,
                        normal_x_sign: std::env::var("MASHUP_NORMAL_X_SIGN")
                            .ok()
                            .and_then(|v| v.parse::<f32>().ok())
                            .unwrap_or(NORMAL_X_SIGN),
                        alpha_cutoff: if let MapAlpha::Mask(c) = m.alpha { c } else { 0.0 },
                        debug_view: match view {
                            MapDebugView::Normal => 0.0,
                            MapDebugView::Lighting { .. } => 1.0,
                            MapDebugView::Albedo => 2.0,
                        },
                    },
                    base: m.texture.map(|i| textures[i].clone()),
                    normal: m.normal_map.filter(|_| bumped).map(|i| textures[i].clone()),
                    lightmap: Some(lm.clone()),
                    lightmap_b0: bumped_lightmaps.as_ref().map(|b| b[0].clone()),
                    lightmap_b1: bumped_lightmaps.as_ref().map(|b| b[1].clone()),
                    lightmap_b2: bumped_lightmaps.as_ref().map(|b| b[2].clone()),
                    alpha_mode: match m.alpha {
                        MapAlpha::Opaque | MapAlpha::Mask(_) => AlphaMode::Opaque,
                        MapAlpha::Blend => AlphaMode::Blend,
                    },
                    double_sided: m.double_sided,
                };
                commands.spawn((
                    Name::new(m.material.clone()),
                    MapPart,
                    layer_of(m.skybox),
                    Mesh3d(meshes.add(build_mesh(m, true))),
                    MeshMaterial3d(world_materials.add(material)),
                    Transform::default(),
                    ChildOf(root),
                ));
                continue;
            }
            let mut part = commands.spawn((
                Name::new(m.material.clone()),
                MapPart,
                layer_of(m.skybox),
                Mesh3d(meshes.add(build_mesh(m, lit.is_some()))),
                MeshMaterial3d(materials.add(build_material(m, &textures, view, data.look.light_scale))),
                Transform::default(),
                ChildOf(root),
            ));
            if let Some(image) = lit {
                part.insert(bevy::pbr::Lightmap {
                    image: image.clone(),
                    uv_rect: Rect::new(0.0, 0.0, 1.0, 1.0),
                    bicubic_sampling: false,
                });
            }
        }
        model_parts = data
            .models
            .iter()
            .map(|model| {
                model
                    .meshes
                    .iter()
                    .map(|m| {
                        (
                            meshes.add(build_mesh(m, false)),
                            materials.add(build_material(m, &textures, view, data.look.light_scale)),
                        )
                    })
                    .collect()
            })
            .collect();
        lit_model_materials = data
            .models
            .iter()
            .map(|model| {
                model
                    .meshes
                    .iter()
                    .map(|m| {
                        materials.add(StandardMaterial {
                            unlit: true,
                            ..build_material(m, &textures, view, data.look.light_scale)
                        })
                    })
                    .collect()
            })
            .collect();

        if let Some(rope_materials) = rope_materials.as_mut()
            && view == MapDebugView::Normal
        {
            for (i, rope) in data.ropes.iter().enumerate() {
                let mesh = meshes.add(rope_material::rope_mesh(rope));
                let strips =
                    rope.back
                        .map(|(t, n)| (t, n, true))
                        .into_iter()
                        .chain([(rope.texture, rope.normal_map, false)]);
                for (texture, normal, back) in strips {
                    commands.spawn((
                        Name::new(format!("Rope {i}{}", if back { " (back)" } else { "" })),
                        MapPart,
                        Mesh3d(mesh.clone()),
                        MeshMaterial3d(rope_materials.add(RopeMaterial {
                            params: RopeParams {
                                width: rope.width,
                                back: if back { 1.0 } else { 0.0 },
                                light_scale: data.look.light_scale,
                                has_normal_map: if normal.is_some() { 1.0 } else { 0.0 },
                            },
                            base: texture.map(|t| textures[t].clone()),
                            normal: normal.map(|t| textures[t].clone()),
                            blend: back,
                        })),
                        bevy::light::NotShadowCaster,
                        Transform::default(),
                        ChildOf(root),
                    ));
                }
            }
        }
    }

    for (i, prop) in data.props.iter().enumerate() {
        let mut e = commands.spawn((
            Name::new(format!("Prop {i}")),
            MapPart,
            Transform::from_translation(prop.translation).with_rotation(prop.rotation),
            Visibility::default(),
            ChildOf(root),
        ));
        match (prop.solid, &model_colliders[prop.model]) {
            (PropSolid::Mesh, Some(collider)) => {
                e.insert((RigidBody::Static, collider.clone()));
            }
            (PropSolid::Box, _) => {
                let (lo, hi) = data.models[prop.model].bounds;
                let size = hi - lo;
                if size.min_element() > 0.0 {
                    e.insert(RigidBody::Static).with_child((
                        Collider::cuboid(size.x, size.y, size.z),
                        Transform::from_translation((lo + hi) / 2.0),
                    ));
                }
            }
            _ => {}
        }
        let id = e.id();
        let probe_scale = data.look.light_scale
            * if let MapDebugView::Lighting { scale } = view {
                scale
            } else {
                1.0
            };
        let probe = prop.lighting.as_ref().filter(|_| view != MapDebugView::Albedo);
        match (probe, meshes.as_mut(), lit_model_materials.get(prop.model)) {
            // Baked: own mesh copy with per-vertex light, unlit material
            // (texture x light, like the lightmapped world).
            (Some(probe), Some(meshes), Some(mats)) => {
                for (m, material) in data.models[prop.model].meshes.iter().zip(mats) {
                    let layer = layer_of(prop.skybox);
                    let colors: Vec<[f32; 4]> = m
                        .normals
                        .iter()
                        .map(|n| {
                            let l = probe.eval(prop.rotation * Vec3::from(*n)) * probe_scale;
                            [l.x, l.y, l.z, 1.0]
                        })
                        .collect();
                    let mut mesh = build_mesh(m, false);
                    mesh.insert_attribute(Mesh::ATTRIBUTE_COLOR, colors);
                    commands.spawn((
                        Mesh3d(meshes.add(mesh)),
                        MeshMaterial3d(material.clone()),
                        layer,
                        ChildOf(id),
                    ));
                }
            }
            _ => {
                if let Some(parts) = model_parts.get(prop.model) {
                    for (mesh, material) in parts {
                        commands.spawn((
                            Mesh3d(mesh.clone()),
                            MeshMaterial3d(material.clone()),
                            layer_of(prop.skybox),
                            ChildOf(id),
                        ));
                    }
                }
            }
        }
    }

    commands.spawn((
        Name::new("Sun"),
        MapPart,
        DirectionalLight {
            illuminance: 9000.0,
            shadow_maps_enabled: true,
            // The sun is baked into the lightmap; it still lights characters.
            affects_lightmapped_mesh_diffuse: false,
            ..default()
        },
        Transform::default().looking_at(Vec3::new(-0.4, -1.0, -0.3), Vec3::Y),
    ));

    for (i, (feet, team)) in data.spawns.iter().enumerate() {
        commands.spawn((
            Name::new(format!("Spawn {i}")),
            MapPart,
            SpawnPoint { team: *team },
            Transform::from_translation(*feet + Vec3::Y * SPAWN_LIFT),
        ));
    }
}

/// A repeating, mipmapped GPU image. Mipmaps are box-filtered here; source
/// files' own mip levels aren't used yet.
fn to_image(t: &MapTexture, look: &MapLook) -> Image {
    let mut data = t.rgba8.clone();
    let (mut w, mut h) = (t.width as usize, t.height as usize);
    let mut level = t.rgba8.clone();
    let mut levels = 1;
    // Prefer the texture's own mip levels (the original tools' filtering).
    let complete = (t.width.max(t.height) as f32).log2() as usize;
    if !t.mips.is_empty() && t.mips.len() == complete {
        for m in &t.mips {
            data.extend_from_slice(m);
        }
        levels += t.mips.len() as u32;
        w = 1;
        h = 1;
    }
    while w > 1 || h > 1 {
        let (nw, nh) = ((w / 2).max(1), (h / 2).max(1));
        let mut next = vec![0u8; nw * nh * 4];
        for y in 0..nh {
            for x in 0..nw {
                for c in 0..4 {
                    let mut sum = 0u32;
                    for (dx, dy) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
                        let sx = (x * 2 + dx).min(w - 1);
                        let sy = (y * 2 + dy).min(h - 1);
                        sum += level[(sy * w + sx) * 4 + c] as u32;
                    }
                    next[(y * nw + x) * 4 + c] = (sum / 4) as u8;
                }
            }
        }
        data.extend_from_slice(&next);
        level = next;
        (w, h) = (nw, nh);
        levels += 1;
    }
    let mut image = Image::new_uninit(
        Extent3d {
            width: t.width,
            height: t.height,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        if t.srgb {
            TextureFormat::Rgba8UnormSrgb
        } else {
            TextureFormat::Rgba8Unorm
        },
        RenderAssetUsages::RENDER_WORLD,
    );
    image.data = Some(data);
    image.texture_descriptor.mip_level_count = levels;
    image.sampler = ImageSampler::Descriptor(ImageSamplerDescriptor {
        address_mode_u: ImageAddressMode::Repeat,
        address_mode_v: ImageAddressMode::Repeat,
        mag_filter: ImageFilterMode::Linear,
        min_filter: ImageFilterMode::Linear,
        mipmap_filter: if look.trilinear {
            ImageFilterMode::Linear
        } else {
            ImageFilterMode::Nearest
        },
        anisotropy_clamp: look.anisotropy.max(1),
        ..default()
    });
    image
}

/// A lighting atlas layer as a filterable half-float texture, clamped.
fn lightmap_image(rgb: &[[f32; 3]], width: u32, height: u32) -> Image {
    let mut data = Vec::with_capacity(rgb.len() * 8);
    for [r, g, b] in rgb {
        for c in [*r, *g, *b, 1.0] {
            data.extend_from_slice(&half::f16::from_f32(c).to_le_bytes());
        }
    }
    let mut image = Image::new(
        Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        data,
        TextureFormat::Rgba16Float,
        RenderAssetUsages::RENDER_WORLD,
    );
    image.sampler = ImageSampler::linear();
    image
}

fn build_mesh(m: &MapMesh, with_lightmap: bool) -> Mesh {
    let mut mesh = Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::default());
    mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, m.positions.clone());
    mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, m.normals.clone());
    mesh.insert_attribute(Mesh::ATTRIBUTE_UV_0, m.uvs.clone());
    if with_lightmap {
        mesh.insert_attribute(Mesh::ATTRIBUTE_UV_1, m.lightmap_uvs.clone());
    }
    mesh.insert_indices(Indices::U32(m.indices.clone()));
    mesh
}

fn build_material(m: &MapMesh, textures: &[Handle<Image>], view: MapDebugView, light_scale: f32) -> StandardMaterial {
    let [r, g, b] = m.color;
    let lighting_only = matches!(view, MapDebugView::Lighting { .. });
    let scale = if let MapDebugView::Lighting { scale } = view {
        scale
    } else {
        1.0
    };
    StandardMaterial {
        base_color: if m.texture.is_some() || lighting_only {
            Color::WHITE
        } else {
            Color::srgb_u8(r, g, b)
        },
        base_color_texture: m.texture.filter(|_| !lighting_only).map(|i| textures[i].clone()),
        unlit: view == MapDebugView::Albedo,
        perceptual_roughness: 0.95,
        reflectance: 0.2,
        alpha_mode: match m.alpha {
            MapAlpha::Opaque => AlphaMode::Opaque,
            MapAlpha::Mask(cutoff) => AlphaMode::Mask(cutoff),
            MapAlpha::Blend => AlphaMode::Blend,
        },
        double_sided: m.double_sided,
        cull_mode: if m.double_sided {
            None
        } else {
            Some(bevy::render::render_resource::Face::Back)
        },
        lightmap_exposure: LIGHTMAP_EXPOSURE * scale * light_scale,
        ..default()
    }
}

/// All of a model's triangles as one collider, in model space.
fn model_collider(model: &MapModel) -> Option<Collider> {
    let mut positions = Vec::new();
    let mut indices = Vec::new();
    for m in &model.meshes {
        let base = positions.len() as u32;
        positions.extend(m.positions.iter().map(|p| Vec3::from(*p)));
        indices.extend(
            m.indices
                .as_chunks::<3>()
                .0
                .iter()
                .map(|t| [base + t[0], base + t[1], base + t[2]]),
        );
    }
    (!indices.is_empty()).then(|| Collider::trimesh(positions, indices))
}

/// Give every 3D camera the map's sky.
#[allow(clippy::type_complexity)]
fn attach_sky(
    mut commands: Commands,
    sky: Option<Res<MapSkybox>>,
    sky_camera: Option<Res<SkyCameraInfo>>,
    cameras: Query<(Entity, Has<SkyboxCamera>), (With<Camera3d>, Without<bevy::light::Skybox>)>,
) {
    let Some(sky) = sky else { return };
    for (cam, is_sky_camera) in &cameras {
        // With a 3D skybox, the 2D sky is drawn behind it by the sky camera.
        if sky_camera.is_some() && !is_sky_camera {
            continue;
        }
        commands.entity(cam).insert(bevy::light::Skybox {
            image: Some(sky.0.clone()),
            // Show the texture at its own brightness (cancel camera exposure).
            brightness: LIGHTMAP_EXPOSURE,
            ..default()
        });
    }
}

/// Six faces into one cube texture. Faces narrower than tall or shorter
/// than wide (Source sides are often half height) are stretched to square.
fn sky_image(sky: &MapSky, textures: &[MapTexture]) -> Image {
    let size = sky
        .faces
        .iter()
        .map(|(t, _)| textures[*t].width.max(textures[*t].height))
        .max()
        .unwrap_or(1);
    let mut data = Vec::with_capacity((size * size * 4 * 6) as usize);
    // Calibration hook for `refcmp skyconv`: each pixel encodes its own
    // face (blue) and position (red = u, green = v).
    let debug = std::env::var_os("MASHUP_SKY_DEBUG").is_some();
    for (face, (tex, turns)) in sky.faces.into_iter().enumerate() {
        if debug {
            for y in 0..size {
                for x in 0..size {
                    data.extend_from_slice(&[
                        (x * 255 / size) as u8,
                        (y * 255 / size) as u8,
                        face as u8 * 40 + 20,
                        255,
                    ]);
                }
            }
            continue;
        }
        let t = &textures[tex];
        for y in 0..size {
            for x in 0..size {
                // Rotate clockwise by `turns` quarter turns: sample the source
                // pixel that lands at (x, y).
                let (mut u, mut v) = (x, y);
                for _ in 0..turns % 4 {
                    (u, v) = (v, size - 1 - u);
                }
                if turns >= 4 {
                    u = size - 1 - u;
                }
                let sx = (u as u64 * t.width as u64 / size as u64) as u32;
                let sy = (v as u64 * t.height as u64 / size as u64) as u32;
                let i = ((sy * t.width + sx) * 4) as usize;
                data.extend_from_slice(&t.rgba8[i..i + 4]);
            }
        }
    }
    let mut image = Image::new(
        Extent3d {
            width: size,
            height: size,
            depth_or_array_layers: 6,
        },
        TextureDimension::D2,
        data,
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::RENDER_WORLD,
    );
    image.texture_view_descriptor = Some(bevy::render::render_resource::TextureViewDescriptor {
        dimension: Some(bevy::render::render_resource::TextureViewDimension::Cube),
        ..default()
    });
    image.sampler = ImageSampler::linear();
    image
}

fn layer_of(skybox: bool) -> bevy::camera::visibility::RenderLayers {
    bevy::camera::visibility::RenderLayers::layer(if skybox { SKYBOX_LAYER } else { 0 })
}

/// Keep a sky camera behind each frame's main camera: at
/// `origin + eye / scale`, same rotation, projection and render target. The
/// main camera then draws the world over it without clearing.
#[allow(clippy::type_complexity)]
fn follow_sky_camera(
    mut commands: Commands,
    info: Option<Res<SkyCameraInfo>>,
    main: Query<
        (
            &GlobalTransform,
            &Projection,
            Option<&bevy::camera::RenderTarget>,
            Option<&bevy::core_pipeline::tonemapping::Tonemapping>,
            Entity,
            &Camera,
        ),
        (With<Camera3d>, Without<SkyboxCamera>),
    >,
    mut sky: Query<(Entity, &mut Transform, &mut Projection), With<SkyboxCamera>>,
) {
    let Some(info) = info else { return };
    let Some((main_tf, projection, target, tonemapping, main_entity, main_camera)) =
        main.iter().find(|(.., c)| c.is_active)
    else {
        return;
    };
    let eye = main_tf.translation();
    let translation = info.0.origin + eye / info.0.scale;
    let rotation = main_tf.rotation();
    if !matches!(main_camera.clear_color, ClearColorConfig::None) {
        commands.entity(main_entity).insert(Camera {
            clear_color: ClearColorConfig::None,
            ..main_camera.clone()
        });
    }
    match sky.single_mut() {
        Ok((entity, mut tf, mut proj)) => {
            tf.translation = translation;
            tf.rotation = rotation;
            if let (Projection::Perspective(p), Projection::Perspective(main_p)) = (&mut *proj, projection) {
                p.fov = main_p.fov;
                p.aspect_ratio = main_p.aspect_ratio;
            }
            let mut e = commands.entity(entity);
            if let Some(t) = target {
                e.insert(t.clone());
            }
            if let Some(t) = tonemapping {
                e.insert(*t);
            }
        }
        Err(_) => {
            let mut e = commands.spawn((
                Name::new("Sky camera"),
                SkyboxCamera,
                MapPart,
                Camera3d::default(),
                Camera {
                    order: main_camera.order - 1,
                    ..default()
                },
                Projection::Perspective(PerspectiveProjection {
                    near: 0.01,
                    ..default()
                }),
                Transform::from_translation(translation).with_rotation(rotation),
                bevy::camera::visibility::RenderLayers::layer(SKYBOX_LAYER),
            ));
            if let Some(fog) = &info.0.fog {
                let [r, g, b] = fog.color;
                e.insert(bevy::pbr::DistanceFog {
                    color: Color::srgb(r, g, b),
                    falloff: bevy::pbr::FogFalloff::Linear {
                        start: fog.start / info.0.scale,
                        end: fog.end / info.0.scale,
                    },
                    ..default()
                });
            }
        }
    }
}
