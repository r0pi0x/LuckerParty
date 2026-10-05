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

mod dust;
pub mod prop_material;
pub mod rope_material;
pub mod sprite_material;
pub mod world_material;

use prop_material::{PropMaterial, PropParams};
use rope_material::{RopeMaterial, RopeParams};
use sprite_material::{SpriteMaterial, SpriteParams};
use world_material::{WorldMaterial, WorldParams};

/// Signs for normal maps' red and green channels in the world material.
/// Source's normal maps are DirectX-style (x along texture u, y along v,
/// image-down) and used unflipped (specs/cs_source/shaders.md). Overridable
/// with MASHUP_NORMAL_X_SIGN / MASHUP_NORMAL_G_SIGN for experiments.
const NORMAL_G_SIGN: f32 = 1.0;
const NORMAL_X_SIGN: f32 = 1.0;

/// Source LDR lightmaps (specs/cs_source/shaders.md): each linear value L is
/// rounded to 1/1024, clamped to 4095/1024, stored as the 8-bit sRGB texel
/// 0.5 L^(1/2.2), and multiplied back by 2^2.2 after the sampler's decode.
const SOURCE_LIGHTMAP_SCALE: f32 = 4.594_793;

pub fn source_ldr_texel(l: f32) -> u8 {
    let i = (l * 1024.0).round().clamp(0.0, 4095.0);
    (255.0 * 0.5 * (i / 1024.0).powf(1.0 / 2.2)).round() as u8
}

/// Source LDR bump pages (specs/cs_source/shaders.md, "Bump page encoding
/// at upload"): the three directional values of a luxel (linear) are
/// scaled so their mean is the flat value's encoded level, overflow above 1
/// is shared out to the other pages, and the result is stored linearly
/// (the sampler's sRGB decode x 2^2.2 still applies when reading).
pub fn source_ldr_bump_texels(flat: [f32; 3], pages: [[f32; 3]; 3]) -> [[u8; 3]; 3] {
    let mut q = [[0.0f32; 3]; 3];
    for c in 0..3 {
        let i = (flat[c] * 1024.0).round().clamp(0.0, 4095.0);
        let goal = 0.5 * (i / 1024.0).powf(1.0 / 2.2);
        let mean = (pages[0][c] + pages[1][c] + pages[2][c]) / 3.0;
        let s = if mean > 0.0 { goal / mean } else { 0.0 };
        for k in 0..3 {
            q[k][c] = pages[k][c] * s;
        }
    }
    let max = |v: [f32; 3]| v[0].max(v[1]).max(v[2]);
    // Order once by largest channel, descending; ties go to the last
    // matching permutation in this list.
    const ORDERS: [[usize; 3]; 6] = [[0, 1, 2], [0, 2, 1], [1, 0, 2], [1, 2, 0], [2, 0, 1], [2, 1, 0]];
    let m = q.map(max);
    let order = ORDERS
        .iter()
        .rev()
        .find(|o| m[o[0]] >= m[o[1]] && m[o[1]] >= m[o[2]])
        .copied()
        .unwrap_or([0, 1, 2]);
    for &k in &order {
        let top = max(q[k]);
        if top > 1.0 {
            let excess = q[k].map(|v| v * (top - 1.0) / top);
            for c in 0..3 {
                q[k][c] -= excess[c];
                for other in (0..3).filter(|&o| o != k) {
                    q[other][c] += excess[c] / 2.0;
                }
            }
        }
    }
    q.map(|v| {
        let top = max(v);
        let v = if top > 1.0 { v.map(|x| x / top) } else { v };
        v.map(|x| (255.0 * x.max(0.0)).round().clamp(0.0, 255.0) as u8)
    })
}

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
    /// A second texture blended in per vertex (Source's WorldVertexTransition
    /// on displacements).
    pub blend: Option<MapBlend>,
    /// Per-vertex blend weight: 0 = first texture, 1 = second. Empty when
    /// there's no blend.
    pub blend_weights: Vec<f32>,
    /// A detail texture tiled over the base texture.
    pub detail: Option<MapDetail>,
    /// Drawn at the texture's own brightness, ignoring lighting (Source's
    /// UnlitGeneric).
    pub unlit: bool,
}

/// Source `$detail`: a texture tiled `scale` times per base texture repeat
/// and combined with the base color (specs/cs_source/shaders.md).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MapDetail {
    /// Index into `MapData::textures` (linear for mode 0, sRGB for 1).
    pub texture: usize,
    pub scale: [f32; 2],
    pub factor: f32,
    /// 0: multiply by 2 x detail ("mod2x"); 1: add.
    pub mode: u8,
}

/// The second layer of a two-texture surface. Indices into
/// `MapData::textures`.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct MapBlend {
    pub texture: Option<usize>,
    pub normal_map: Option<usize>,
    /// Shapes the blend: green moves the transition point, red sets its
    /// softness (linear texture).
    pub mask: Option<usize>,
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
    /// The same volumes as planes, for exact swept-box movement collision.
    pub collision_brushes: Vec<MapBrush>,
    /// Feet positions.
    pub spawns: Vec<(Vec3, Option<Team>)>,
    /// Things the importer couldn't load (missing materials etc.).
    pub warnings: Vec<String>,
    /// How the source game presents the map, so it can be reproduced.
    pub look: MapLook,
    pub sky: Option<MapSky>,
    pub sky_camera: Option<MapSkyCamera>,
    /// The world's fog (not the 3D skybox's, which `sky_camera` has).
    pub fog: Option<MapFog>,
    pub sprites: Vec<MapSprite>,
    pub dust: Vec<MapDust>,
    pub ropes: Vec<MapRope>,
}

/// A solid convex volume as planes: a point p is inside when n.p <= d for
/// every plane (n, d). Engine space, meters; `min`/`max` bound it. Games
/// whose movement sweeps boxes against brushes (Source) use these instead
/// of the physics engine's shape casts, which lose precision on large
/// volumes.
#[derive(Clone, Debug)]
pub struct MapBrush {
    pub planes: Vec<(Vec3, f32)>,
    pub min: Vec3,
    pub max: Vec3,
}

impl MapBrush {
    /// An axis-aligned box.
    pub fn from_box(min: Vec3, max: Vec3) -> Self {
        let planes = vec![
            (Vec3::X, max.x),
            (Vec3::NEG_X, -min.x),
            (Vec3::Y, max.y),
            (Vec3::NEG_Y, -min.y),
            (Vec3::Z, max.z),
            (Vec3::NEG_Z, -min.z),
        ];
        Self { planes, min, max }
    }
}

/// The loaded map's brushes (`MapData::collision_brushes`).
#[derive(Resource, Clone, Debug, Default)]
pub struct MapBrushes(pub Vec<MapBrush>);

/// Marks the physics collider built from the same brushes, so movement that
/// sweeps `MapBrushes` itself can leave it out of physics queries.
#[derive(Component, Debug)]
pub struct MapBrushCollider;

/// A dust mote volume (`func_dustmotes`): slow specks spawned inside a box,
/// fading in and out over their life and with distance
/// (specs/cs_source/sprites_dust.md). Engine space, meters, seconds.
#[derive(Clone, Debug)]
pub struct MapDust {
    pub min: Vec3,
    pub max: Vec3,
    /// Motes per second.
    pub rate: f32,
    /// Screen-constant size: half-width = size / 10000 x view depth.
    pub size: (f32, f32),
    /// Initial speed per axis, up to this (m/s).
    pub speed: f32,
    pub life: (f32, f32),
    /// Not drawn beyond this view depth; fade toward it (meters).
    pub fade_distance: f32,
    /// sRGB color and alpha (0-1).
    pub color: [f32; 4],
    /// Index into `MapData::textures`.
    pub texture: Option<usize>,
}

/// A camera-facing sprite (lamp glows): a quad parallel to the view plane,
/// drawn additively (specs/cs_source/sprites_dust.md).
#[derive(Clone, Debug)]
pub struct MapSprite {
    /// Engine space, meters.
    pub position: Vec3,
    /// Index into `MapData::textures`.
    pub texture: usize,
    /// Full width and height, meters.
    pub size: Vec2,
    /// Color x texture is added to the image: RGB scale and alpha weight.
    pub color: Vec4,
    /// A glow: drawn over everything, faded by how much of its occlusion
    /// proxy can be seen. Otherwise depth tested against the world.
    pub glow: bool,
    /// Glow occlusion proxy: half-diagonal of a square pulled this far
    /// toward the viewer, meters.
    pub proxy: f32,
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

/// Distance fog, distances in meters as seen in the world. Source's range
/// fog: f = clamp(min(max_density, (depth - start) / (end - start))), and
/// the color moves toward the fog color by f^2 (specs/cs_source/shaders.md).
#[derive(Clone, Debug)]
pub struct MapFog {
    /// sRGB color.
    pub color: [f32; 3],
    pub start: f32,
    pub end: f32,
    pub max_density: f32,
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
    /// Store lightmaps the way Source's LDR path does (8-bit, gamma, 2x
    /// overbright), with its banding, clamp and gamma-space filtering.
    pub source_ldr_lightmaps: bool,
    /// Sample lightmaps with a bicubic B-spline filter (4 bilinear taps)
    /// instead of bilinear.
    pub bicubic_lightmaps: bool,
}

impl Default for MapLook {
    fn default() -> Self {
        Self {
            light_scale: 1.0,
            trilinear: true,
            anisotropy: 8,
            tonemapping: true,
            source_ldr_lightmaps: false,
            bicubic_lightmaps: false,
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
            .add_systems(Update, (attach_sky, glow_visibility, dust::update_dust))
            .add_systems(
                PostUpdate,
                // After propagation, so it sees this frame's (interpolated)
                // eye rather than the last physics tick's; before frusta.
                follow_sky_camera
                    .after(bevy::transform::TransformSystems::Propagate)
                    .before(bevy::camera::visibility::VisibilitySystems::UpdateFrusta),
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
    mut sprite_materials: Option<ResMut<Assets<SpriteMaterial>>>,
    mut prop_materials: Option<ResMut<Assets<PropMaterial>>>,
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
            MapBrushCollider,
            RigidBody::Static,
            Collider::compound(hulls),
            Transform::default(),
        ));
    }

    // Prop models: one collider per model (shared by its placements), and
    // render handles when rendering exists.
    let model_colliders: Vec<Option<Collider>> = data.models.iter().map(model_collider).collect();
    let mut model_parts: Vec<Vec<(Handle<Mesh>, Handle<StandardMaterial>)>> = Vec::new();
    // Per model mesh: [in the world (fogged), in the 3D skybox].
    let mut lit_model_materials: Vec<Vec<[Handle<PropMaterial>; 2]>> = Vec::new();

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
        // The world material's copies, in the game's own encoding if asked.
        let source_ldr = data.look.source_ldr_lightmaps;
        let world_layer = |rgb: &[[f32; 3]], w: u32, h: u32| {
            if source_ldr {
                source_lightmap_image(rgb, w, h)
            } else {
                lightmap_image(rgb, w, h)
            }
        };
        let world_lightmap = data
            .lightmap
            .as_ref()
            .map(|l| images.add(world_layer(&l.rgb, l.width, l.height)));
        let bumped_lightmaps: Option<[Handle<Image>; 3]> = data.lightmap.as_ref().and_then(|l| {
            let b = l.bumped.as_ref()?;
            if source_ldr {
                // The game encodes the three pages together, against the flat one.
                let mut texels: [Vec<u8>; 3] = std::array::from_fn(|_| Vec::with_capacity(l.rgb.len() * 4));
                for (i, flat) in l.rgb.iter().enumerate() {
                    let pages = source_ldr_bump_texels(*flat, [b[0][i], b[1][i], b[2][i]]);
                    for (t, [r, g, b]) in texels.iter_mut().zip(pages) {
                        t.extend([r, g, b, 255]);
                    }
                }
                return Some(texels.map(|t| images.add(srgb8_image(t, l.width, l.height))));
            }
            Some(std::array::from_fn(|i| {
                images.add(world_layer(&b[i], l.width, l.height))
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
            if let (Some(_), Some(lm), Some(world_materials)) = (lit, world_lightmap.as_ref(), world_materials.as_mut())
            {
                let [r, g, b] = m.color;
                // Calibration hook: MASHUP_NORMAL_G_SIGN overrides the sign;
                // 0 turns radiosity normal mapping off.
                let g_sign = std::env::var("MASHUP_NORMAL_G_SIGN")
                    .ok()
                    .and_then(|v| v.parse::<f32>().ok())
                    .unwrap_or(NORMAL_G_SIGN);
                let bumped = m.normal_map.is_some() && bumped_lightmaps.is_some() && g_sign != 0.0;
                // Two-texture blend: the weights ride in the vertex color's alpha.
                let blend = m.blend.unwrap_or_default();
                let blended = blend.texture.is_some() && m.blend_weights.len() == m.positions.len();
                let material = WorldMaterial {
                    params: WorldParams {
                        base_color: if m.texture.is_some() {
                            Vec4::ONE
                        } else {
                            Vec4::new(r as f32 / 255.0, g as f32 / 255.0, b as f32 / 255.0, 1.0)
                        },
                        light_scale: data.look.light_scale,
                        lightmap_scale: if source_ldr { SOURCE_LIGHTMAP_SCALE } else { 1.0 },
                        bicubic: if data.look.bicubic_lightmaps { 1.0 } else { 0.0 },
                        blend: if blended { 1.0 } else { 0.0 },
                        blend_masked: if blend.mask.is_some() { 1.0 } else { 0.0 },
                        blend_normal: if bumped && blend.normal_map.is_some() { 1.0 } else { 0.0 },
                        detail: m.detail.map_or(0.0, |d| d.mode as f32 + 1.0),
                        detail_factor: m.detail.map_or(0.0, |d| d.factor),
                        detail_scale: m.detail.map_or(Vec2::ONE, |d| Vec2::from_array(d.scale)),
                        fog_color: fog_color(data.fog.as_ref().filter(|_| view == MapDebugView::Normal && !m.skybox)),
                        fog_range: fog_range(data.fog.as_ref()),
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
                    base2: blend.texture.filter(|_| blended).map(|i| textures[i].clone()),
                    normal2: blend
                        .normal_map
                        .filter(|_| blended && bumped)
                        .map(|i| textures[i].clone()),
                    blend_mask: blend.mask.filter(|_| blended).map(|i| textures[i].clone()),
                    detail: m.detail.map(|d| textures[d.texture].clone()),
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
                    Mesh3d(meshes.add({
                        let mut mesh = build_mesh(m, true);
                        if blended {
                            let colors: Vec<[f32; 4]> = m.blend_weights.iter().map(|w| [1.0, 1.0, 1.0, *w]).collect();
                            mesh.insert_attribute(Mesh::ATTRIBUTE_COLOR, colors);
                        }
                        mesh
                    })),
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
        if let Some(prop_materials) = prop_materials.as_mut() {
            let lighting_only = matches!(view, MapDebugView::Lighting { .. });
            lit_model_materials = data
                .models
                .iter()
                .map(|model| {
                    model
                        .meshes
                        .iter()
                        .map(|m| {
                            let [r, g, b] = m.color;
                            [false, true].map(|skybox| {
                                prop_materials.add(PropMaterial {
                                    params: PropParams {
                                        base_color: if m.texture.is_some() || lighting_only {
                                            Vec4::ONE
                                        } else {
                                            Color::srgb_u8(r, g, b).to_linear().to_vec4()
                                        },
                                        alpha_cutoff: if let MapAlpha::Mask(c) = m.alpha { c } else { 0.0 },
                                        fog_color: fog_color(
                                            data.fog.as_ref().filter(|_| view == MapDebugView::Normal && !skybox),
                                        ),
                                        fog_range: fog_range(data.fog.as_ref()),
                                    },
                                    base: m.texture.filter(|_| !lighting_only).map(|i| textures[i].clone()),
                                    alpha_mode: match m.alpha {
                                        MapAlpha::Opaque | MapAlpha::Mask(_) => AlphaMode::Opaque,
                                        MapAlpha::Blend => AlphaMode::Blend,
                                    },
                                    double_sided: m.double_sided,
                                })
                            })
                        })
                        .collect()
                })
                .collect();
        }

        if view == MapDebugView::Normal {
            for (i, dust) in data.dust.iter().enumerate() {
                let mesh = meshes.add(dust::empty_mesh());
                commands.spawn((
                    Name::new(format!("Dust {i}")),
                    MapPart,
                    Mesh3d(mesh.clone()),
                    MeshMaterial3d(materials.add(StandardMaterial {
                        base_color_texture: dust.texture.map(|t| textures[t].clone()),
                        unlit: true,
                        alpha_mode: AlphaMode::Blend,
                        double_sided: true,
                        cull_mode: None,
                        ..default()
                    })),
                    dust::DustEmitter::new(dust.clone(), mesh, i as u64 + 1),
                    // The mesh moves with the motes; don't cull it by stale bounds.
                    bevy::camera::visibility::NoFrustumCulling,
                    bevy::light::NotShadowCaster,
                    Transform::default(),
                    ChildOf(root),
                ));
            }
        }
        if let Some(sprite_materials) = sprite_materials.as_mut()
            && view == MapDebugView::Normal
        {
            for (i, sprite) in data.sprites.iter().enumerate() {
                let t = &data.textures[sprite.texture];
                let mut e = commands.spawn((
                    Name::new(format!("Sprite {i}")),
                    MapPart,
                    Mesh3d(meshes.add(sprite_material::sprite_mesh(UVec2::new(t.width, t.height)))),
                    MeshMaterial3d(sprite_materials.add(SpriteMaterial {
                        params: SpriteParams {
                            color: sprite.color,
                            size: sprite.size,
                        },
                        texture: Some(textures[sprite.texture].clone()),
                        glow: sprite.glow,
                    })),
                    bevy::light::NotShadowCaster,
                    // The quad is spread in the vertex shader, so the mesh's
                    // own bounds are a point: culling would drop the sprite
                    // as soon as its centre left the view.
                    bevy::camera::visibility::NoFrustumCulling,
                    Transform::from_translation(sprite.position),
                    ChildOf(root),
                ));
                if sprite.glow {
                    e.insert(GlowSprite {
                        color: sprite.color,
                        proxy: sprite.proxy,
                    });
                }
            }
        }
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

    // Exact brushes for movement that sweeps them (Source): the world's,
    // plus props that are (close to) convex, in world space.
    let mut brushes = data.collision_brushes.clone();
    // Comparison hook: MASHUP_NO_PROP_BRUSHES=1 sweeps props as meshes.
    let no_prop_brushes = std::env::var("MASHUP_NO_PROP_BRUSHES").is_ok_and(|v| v == "1");
    let model_hulls: Vec<Option<Vec<(Vec3, f32)>>> = data
        .models
        .iter()
        .map(|m| convex_planes(m, CONVEX_SURFACE_SHARE))
        .collect();
    info!(
        "props: {} of {} models collide as exact convex brushes",
        model_hulls.iter().filter(|h| h.is_some()).count(),
        model_hulls.len()
    );

    for (i, prop) in data.props.iter().enumerate() {
        let prop_brush = match prop.solid {
            PropSolid::Mesh => model_hulls[prop.model].as_ref(),
            _ => None,
        }
        .cloned()
        .or_else(|| {
            let (lo, hi) = data.models[prop.model].bounds;
            (prop.solid == PropSolid::Box && (hi - lo).min_element() > 0.0).then(|| MapBrush::from_box(lo, hi).planes)
        })
        .filter(|_| !prop.skybox && !no_prop_brushes)
        .map(|planes| place_brush(&planes, prop.translation, prop.rotation));
        if let Some(b) = &prop_brush {
            brushes.push(b.clone());
        }
        let mut e = commands.spawn((
            Name::new(format!("Prop {i}")),
            MapPart,
            Transform::from_translation(prop.translation).with_rotation(prop.rotation),
            Visibility::default(),
            ChildOf(root),
        ));
        match (prop.solid, &model_colliders[prop.model]) {
            (PropSolid::Mesh, Some(collider)) => {
                e.insert((RigidBody::Static, collider.clone(), MapPropCollider));
                if prop_brush.is_some() {
                    e.insert(MapBrushCollider);
                }
            }
            (PropSolid::Box, _) => {
                let (lo, hi) = data.models[prop.model].bounds;
                let size = hi - lo;
                if size.min_element() > 0.0 {
                    let child = (
                        Collider::cuboid(size.x, size.y, size.z),
                        Transform::from_translation((lo + hi) / 2.0),
                    );
                    e.insert(RigidBody::Static).with_children(|c| {
                        let mut ec = c.spawn((child, MapPropCollider));
                        if prop_brush.is_some() {
                            ec.insert(MapBrushCollider);
                        }
                    });
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
                    let material = &material[prop.skybox as usize];
                    let layer = layer_of(prop.skybox);
                    let colors: Vec<[f32; 4]> = m
                        .normals
                        .iter()
                        .map(|n| {
                            if m.unlit {
                                return [1.0, 1.0, 1.0, 1.0];
                            }
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

    if !brushes.is_empty() {
        commands.insert_resource(MapBrushes(brushes));
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
/// A lighting atlas layer in Source's LDR encoding (see
/// `SOURCE_LIGHTMAP_SCALE`): 8-bit sRGB, decoded by the sampler.
fn source_lightmap_image(rgb: &[[f32; 3]], width: u32, height: u32) -> Image {
    let mut data = Vec::with_capacity(rgb.len() * 4);
    for [r, g, b] in rgb {
        data.extend([source_ldr_texel(*r), source_ldr_texel(*g), source_ldr_texel(*b), 255]);
    }
    srgb8_image(data, width, height)
}

/// RGBA8 sRGB texels as a linearly filtered lightmap layer.
fn srgb8_image(data: Vec<u8>, width: u32, height: u32) -> Image {
    let mut image = Image::new(
        Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        data,
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::RENDER_WORLD,
    );
    image.sampler = ImageSampler::linear();
    image
}

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

/// A prop's physics collider (sprite glows see through static props, as
/// the game's line test ignores them).
#[derive(Component)]
struct MapPropCollider;

/// A glow sprite: drawn over everything, faded by how much of its
/// occlusion proxy is visible.
#[derive(Component)]
struct GlowSprite {
    color: Vec4,
    proxy: f32,
}

/// Glow visibility (specs/cs_source/sprites_dust.md, 5a): the game measures
/// the visible fraction of a view-facing square (half-diagonal = proxy
/// size) pulled the proxy size toward the eye, with occlusion queries. We
/// approximate it with lines to the square's centre and corners, and fade
/// the glow by the fraction that get through. Static props don't occlude.
#[allow(clippy::type_complexity)]
fn glow_visibility(
    query: SpatialQuery,
    cameras: Query<&GlobalTransform, (With<Camera3d>, Without<SkyboxCamera>)>,
    ignored: Query<Entity, Or<(With<crate::core::Intent>, With<MapPropCollider>)>>,
    mut glows: Query<(
        &GlobalTransform,
        &GlowSprite,
        &MeshMaterial3d<SpriteMaterial>,
        &mut Visibility,
    )>,
    materials: Option<ResMut<Assets<SpriteMaterial>>>,
) {
    let (Some(eye), Some(mut materials)) = (cameras.iter().next(), materials) else {
        return;
    };
    let filter = SpatialQueryFilter::from_excluded_entities(ignored.iter());
    let from = eye.translation();
    let (right, up) = (eye.right().as_vec3(), eye.up().as_vec3());
    for (glow, sprite, material, mut visibility) in &mut glows {
        let p = glow.translation();
        let centre = p + (from - p).normalize_or_zero() * sprite.proxy;
        let r = sprite.proxy / std::f32::consts::SQRT_2;
        let points = [
            centre,
            centre + (right + up) * r,
            centre + (right - up) * r,
            centre - (right + up) * r,
            centre - (right - up) * r,
        ];
        let seen = points
            .iter()
            .filter(|to| {
                Dir3::new(**to - from).map_or(true, |dir| {
                    query.cast_ray(from, dir, from.distance(**to), true, &filter).is_none()
                })
            })
            .count() as f32
            / points.len() as f32;
        visibility.set_if_neq(if seen > 0.0 {
            Visibility::Inherited
        } else {
            Visibility::Hidden
        });
        if let Some(mut m) = materials.get_mut(&material.0) {
            let color = (sprite.color.truncate() * seen).extend(sprite.color.w);
            if m.params.color != color {
                m.params.color = color;
            }
        }
    }
}

/// Fog color for shaders: linear RGB, w = 1 when fog is on.
fn fog_color(fog: Option<&MapFog>) -> Vec4 {
    fog.map_or(Vec4::ZERO, |f| {
        let c = Color::srgb(f.color[0], f.color[1], f.color[2]).to_linear();
        Vec4::new(c.red, c.green, c.blue, 1.0)
    })
}

/// Fog start, end (meters) and max density for shaders.
fn fog_range(fog: Option<&MapFog>) -> Vec4 {
    fog.map_or(Vec4::new(0.0, 1.0, 0.0, 0.0), |f| {
        Vec4::new(f.start, f.end.max(f.start + 0.01), f.max_density, 0.0)
    })
}

/// Share of a prop's surface that must lie on its convex hull for the hull
/// to stand in for it in exact (brush) collision. Crates and boxes with
/// shallow panel insets pass; arches and other concave props don't.
const CONVEX_SURFACE_SHARE: f32 = 0.75;
const MAX_HULL_PLANES: usize = 48;
/// How close (meters) a triangle must be to a hull face to count as on it.
const ON_HULL: f32 = 0.01;

/// A model's convex hull as planes (model space), if at least `share` of
/// its surface area lies on the hull.
fn convex_planes(model: &MapModel, share: f32) -> Option<Vec<(Vec3, f32)>> {
    let points: Vec<Vec3> = model
        .meshes
        .iter()
        .flat_map(|m| m.positions.iter().map(|p| Vec3::from(*p)))
        .collect();
    if points.len() < 4 {
        return None;
    }
    let (verts, tris) = avian3d::parry::transformation::convex_hull(&points);
    let mut planes: Vec<(Vec3, f32)> = Vec::new();
    for t in &tris {
        let [a, b, c] = t.map(|i| verts[i as usize]);
        let n = (b - a).cross(c - a);
        if n.length_squared() < 1e-12 {
            continue;
        }
        let n = n.normalize();
        let d = n.dot(a);
        if !planes.iter().any(|(m, e)| m.dot(n) > 0.9999 && (e - d).abs() < 1e-4) {
            planes.push((n, d));
        }
    }
    // Rounded props (many faces) stay meshes: little to gain, slow to place.
    if !(4..=MAX_HULL_PLANES).contains(&planes.len()) {
        return None;
    }
    let (mut on, mut total) = (0.0f32, 0.0f32);
    for m in &model.meshes {
        for t in m.indices.as_chunks::<3>().0 {
            let [a, b, c] = t.map(|i| Vec3::from(m.positions[i as usize]));
            let area = (b - a).cross(c - a).length() / 2.0;
            total += area;
            if planes
                .iter()
                .any(|(n, d)| [a, b, c].iter().all(|p| (n.dot(*p) - d).abs() < ON_HULL))
            {
                on += area;
            }
        }
    }
    (total > 0.0 && on >= share * total).then_some(planes)
}

/// Model-space planes placed in the world, with the bounding box's planes
/// added as bevels (so box sweeps stop at corners like Source's brushes).
fn place_brush(planes: &[(Vec3, f32)], translation: Vec3, rotation: Quat) -> MapBrush {
    let mut world: Vec<(Vec3, f32)> = planes
        .iter()
        .map(|(n, d)| {
            let n2 = rotation * *n;
            (n2, d + n2.dot(translation))
        })
        .collect();
    // Corners: intersections of plane triples that lie inside all planes.
    let mut corners = Vec::new();
    for i in 0..world.len() {
        for j in i + 1..world.len() {
            for k in j + 1..world.len() {
                let ((n1, d1), (n2, d2), (n3, d3)) = (world[i], world[j], world[k]);
                let denom = n1.dot(n2.cross(n3));
                if denom.abs() < 1e-6 {
                    continue;
                }
                let p = (n2.cross(n3) * d1 + n3.cross(n1) * d2 + n1.cross(n2) * d3) / denom;
                if world.iter().all(|(n, d)| n.dot(p) <= d + 1e-3) {
                    corners.push(p);
                }
            }
        }
    }
    let min = corners.iter().fold(Vec3::splat(f32::MAX), |a, c| a.min(*c));
    let max = corners.iter().fold(Vec3::splat(f32::MIN), |a, c| a.max(*c));
    for (n, d) in MapBrush::from_box(min, max).planes {
        if !world.iter().any(|(m, e)| m.dot(n) > 0.9999 && (e - d).abs() < 1e-4) {
            world.push((n, d));
        }
    }
    MapBrush {
        planes: world,
        min,
        max,
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
    mut sky: Query<(Entity, &mut Transform, &mut GlobalTransform, &mut Projection), With<SkyboxCamera>>,
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
        Ok((entity, mut tf, mut global, mut proj)) => {
            tf.translation = translation;
            tf.rotation = rotation;
            // Propagation has run: set the global transform too (no parent).
            *global = GlobalTransform::from(*tf);
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
