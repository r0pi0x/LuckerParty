//! The Map slot's neutral data: what any game's map importer produces and
//! the engine spawns. Units are meters, Y up (README: mounts own each game's
//! units and axes; the engine sees only this).

use std::sync::Arc;

use avian3d::prelude::*;
use bevy::{asset::RenderAssetUsages, mesh::Indices, prelude::*, render::render_resource::PrimitiveTopology};

use crate::core::{SpawnPoint, Team};

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
    /// Flat debug color until real materials load.
    pub color: [u8; 3],
}

#[derive(Clone, Debug, Default)]
pub struct MapData {
    pub name: String,
    pub meshes: Vec<MapMesh>,
    /// Collision triangles (may include surfaces that aren't drawn).
    pub collision_positions: Vec<[f32; 3]>,
    pub collision_indices: Vec<[u32; 3]>,
    /// Feet positions.
    pub spawns: Vec<(Vec3, Option<Team>)>,
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
}

#[derive(Resource)]
struct PendingMap(Arc<MapData>);

/// Marks every entity belonging to the loaded map.
#[derive(Component)]
pub struct MapPart;

impl Plugin for MapPlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(PendingMap(self.data.clone()))
            .insert_resource(GlobalAmbientLight {
                brightness: 600.0,
                ..default()
            })
            .add_systems(Startup, spawn_map);
    }
}

/// Height of the capsule center above the feet, so spawns start standing.
const SPAWN_LIFT: f32 = 1.0;

fn spawn_map(
    mut commands: Commands,
    pending: Res<PendingMap>,
    mut meshes: Option<ResMut<Assets<Mesh>>>,
    mut materials: Option<ResMut<Assets<StandardMaterial>>>,
) {
    let data = &pending.0;
    let root = commands
        .spawn((
            Name::new(format!("Map {}", data.name)),
            MapPart,
            Transform::default(),
            Visibility::default(),
        ))
        .id();

    commands.entity(root).with_child((
        Name::new("Map collision"),
        MapPart,
        RigidBody::Static,
        Collider::trimesh(
            data.collision_positions.iter().map(|p| Vec3::from(*p)).collect(),
            data.collision_indices.clone(),
        ),
        Transform::default(),
    ));

    if let (Some(meshes), Some(materials)) = (meshes.as_mut(), materials.as_mut()) {
        for m in &data.meshes {
            let mut mesh = Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::default());
            mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, m.positions.clone());
            mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, m.normals.clone());
            mesh.insert_attribute(Mesh::ATTRIBUTE_UV_0, m.uvs.clone());
            mesh.insert_indices(Indices::U32(m.indices.clone()));
            let [r, g, b] = m.color;
            commands.entity(root).with_child((
                Name::new(m.material.clone()),
                MapPart,
                Mesh3d(meshes.add(mesh)),
                MeshMaterial3d(materials.add(StandardMaterial {
                    base_color: Color::srgb_u8(r, g, b),
                    perceptual_roughness: 0.95,
                    ..default()
                })),
                Transform::default(),
            ));
        }
    }

    commands.spawn((
        Name::new("Sun"),
        MapPart,
        DirectionalLight {
            illuminance: 9000.0,
            shadow_maps_enabled: true,
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
