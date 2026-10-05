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

/// An RGBA8 texture in sRGB, top row first.
#[derive(Clone, Debug)]
pub struct MapTexture {
    /// Source path, e.g. `materials/de_dust/sitebwall01.vtf`.
    pub name: String,
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

/// A placed model.
#[derive(Clone, Debug)]
pub struct MapProp {
    /// Index into `MapData::models`.
    pub model: usize,
    pub translation: Vec3,
    pub rotation: Quat,
    pub solid: PropSolid,
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
    /// Debug view: white surfaces, so only baked lighting shows.
    pub lightmap_only: bool,
}

impl MapPlugin {
    pub fn new(data: MapData) -> Self {
        Self {
            data: Arc::new(data),
            lightmap_only: false,
        }
    }
}

#[derive(Resource)]
struct PendingMap(Arc<MapData>, bool);

/// Marks every entity belonging to the loaded map.
#[derive(Component)]
pub struct MapPart;

impl Plugin for MapPlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(PendingMap(self.data.clone(), self.lightmap_only))
            .insert_resource(GlobalAmbientLight {
                brightness: 600.0,
                // Baked lighting already includes the map's ambient light.
                affects_lightmapped_meshes: false,
                ..default()
            })
            .add_systems(Startup, spawn_map);
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
) {
    let data = &pending.0;
    let lightmap_only = pending.1;
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

    if let (Some(meshes), Some(materials), Some(images)) = (meshes.as_mut(), materials.as_mut(), images.as_mut()) {
        let textures: Vec<Handle<Image>> = data.textures.iter().map(|t| images.add(to_image(t))).collect();
        let lightmap = data.lightmap.as_ref().map(|l| images.add(lightmap_image(l)));
        for m in &data.meshes {
            let lit = lightmap.as_ref().filter(|_| m.lightmap_uvs.len() == m.positions.len());
            let mut part = commands.spawn((
                Name::new(m.material.clone()),
                MapPart,
                Mesh3d(meshes.add(build_mesh(m, lit.is_some()))),
                MeshMaterial3d(materials.add(build_material(m, &textures, lightmap_only))),
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
                            materials.add(build_material(m, &textures, lightmap_only)),
                        )
                    })
                    .collect()
            })
            .collect();
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
        if let Some(parts) = model_parts.get(prop.model) {
            let id = e.id();
            for (mesh, material) in parts {
                commands.spawn((Mesh3d(mesh.clone()), MeshMaterial3d(material.clone()), ChildOf(id)));
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
fn to_image(t: &MapTexture) -> Image {
    let mut data = t.rgba8.clone();
    let (mut w, mut h) = (t.width as usize, t.height as usize);
    let mut level = t.rgba8.clone();
    let mut levels = 1;
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
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::RENDER_WORLD,
    );
    image.data = Some(data);
    image.texture_descriptor.mip_level_count = levels;
    image.sampler = ImageSampler::Descriptor(ImageSamplerDescriptor {
        address_mode_u: ImageAddressMode::Repeat,
        address_mode_v: ImageAddressMode::Repeat,
        mag_filter: ImageFilterMode::Linear,
        min_filter: ImageFilterMode::Linear,
        mipmap_filter: ImageFilterMode::Linear,
        anisotropy_clamp: 8,
        ..default()
    });
    image
}

/// The lighting atlas as a filterable half-float texture, clamped.
fn lightmap_image(l: &MapLightmap) -> Image {
    let mut data = Vec::with_capacity(l.rgb.len() * 8);
    for [r, g, b] in &l.rgb {
        for c in [*r, *g, *b, 1.0] {
            data.extend_from_slice(&half::f16::from_f32(c).to_le_bytes());
        }
    }
    let mut image = Image::new(
        Extent3d {
            width: l.width,
            height: l.height,
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

fn build_material(m: &MapMesh, textures: &[Handle<Image>], lightmap_only: bool) -> StandardMaterial {
    let [r, g, b] = m.color;
    StandardMaterial {
        base_color: if m.texture.is_some() || lightmap_only {
            Color::WHITE
        } else {
            Color::srgb_u8(r, g, b)
        },
        base_color_texture: m.texture.filter(|_| !lightmap_only).map(|i| textures[i].clone()),
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
        lightmap_exposure: LIGHTMAP_EXPOSURE,
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
