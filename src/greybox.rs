//! A Map slot implementation built in code: flat ground, walls, crates, a
//! walkable ramp and a too-steep one. Used to test movement and weapons until
//! real maps load.

use avian3d::prelude::*;
use bevy::prelude::*;

use crate::core::SpawnPoint;

pub struct GreyboxMapPlugin;

impl Plugin for GreyboxMapPlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(GlobalAmbientLight {
            brightness: 400.0,
            ..default()
        })
        .add_systems(Startup, spawn);
    }
}

struct Block {
    name: String,
    size: Vec3,
    transform: Transform,
    color: Color,
}

/// Collision always; meshes and materials only when rendering is present, so
/// the same map runs in headless tests.
fn spawn(
    mut commands: Commands,
    mut meshes: Option<ResMut<Assets<Mesh>>>,
    mut materials: Option<ResMut<Assets<StandardMaterial>>>,
) {
    for b in blocks() {
        let mut e = commands.spawn((
            Name::new(b.name),
            b.transform,
            RigidBody::Static,
            Collider::cuboid(b.size.x, b.size.y, b.size.z),
        ));
        if let (Some(meshes), Some(materials)) = (meshes.as_mut(), materials.as_mut()) {
            e.insert((
                Mesh3d(meshes.add(Cuboid::from_size(b.size))),
                MeshMaterial3d(materials.add(StandardMaterial {
                    base_color: b.color,
                    perceptual_roughness: 0.9,
                    ..default()
                })),
            ));
        }
    }

    commands.spawn((
        Name::new("Sun"),
        DirectionalLight {
            illuminance: 8000.0,
            shadow_maps_enabled: true,
            ..default()
        },
        Transform::default().looking_at(Vec3::new(-0.4, -1.0, -0.6), Vec3::Y),
    ));

    for (i, pos) in SPAWNS.into_iter().enumerate() {
        commands.spawn((
            Name::new(format!("Spawn {i}")),
            SpawnPoint::default(),
            Transform::from_translation(pos),
        ));
    }
}

pub const SPAWNS: [Vec3; 2] = [Vec3::new(0.0, 1.0, 12.0), Vec3::new(0.0, 1.0, -30.0)];

/// Inner face of the north wall (z), and the 20-degree ramp's x position.
pub const NORTH_WALL_Z: f32 = -40.0;
pub const RAMP_X: f32 = 12.0;
pub const STEEP_RAMP_X: f32 = 20.0;
/// Height of the platform at the top of the 20-degree ramp.
pub fn platform_height() -> f32 {
    8.0 * 20f32.to_radians().sin()
}

fn blocks() -> Vec<Block> {
    let mut out = Vec::new();
    let mut block = |name: &str, size: Vec3, transform: Transform, color: Color| {
        out.push(Block {
            name: name.to_string(),
            size,
            transform,
            color,
        });
    };

    let floor = Color::srgb(0.42, 0.42, 0.40);
    let wall = Color::srgb(0.55, 0.53, 0.48);
    let crate_color = Color::srgb(0.60, 0.45, 0.28);
    let ramp = Color::srgb(0.35, 0.45, 0.55);
    let steep = Color::srgb(0.65, 0.30, 0.28);

    block(
        "Floor",
        Vec3::new(80.0, 1.0, 80.0),
        Transform::from_xyz(0.0, -0.5, 0.0),
        floor,
    );
    for (name, size, pos) in [
        (
            "Wall N",
            Vec3::new(80.0, 6.0, 1.0),
            Vec3::new(0.0, 3.0, NORTH_WALL_Z - 0.5),
        ),
        ("Wall S", Vec3::new(80.0, 6.0, 1.0), Vec3::new(0.0, 3.0, 40.5)),
        ("Wall E", Vec3::new(1.0, 6.0, 80.0), Vec3::new(40.5, 3.0, 0.0)),
        ("Wall W", Vec3::new(1.0, 6.0, 80.0), Vec3::new(-40.5, 3.0, 0.0)),
    ] {
        block(name, size, Transform::from_translation(pos), wall);
    }

    // Crates: one low enough to jump onto, stacks to take cover behind.
    for (i, (size, pos)) in [
        (Vec3::splat(0.8), Vec3::new(-4.0, 0.4, 0.0)),
        (Vec3::splat(1.2), Vec3::new(-6.0, 0.6, -3.0)),
        (Vec3::splat(1.2), Vec3::new(-6.0, 1.8, -3.0)),
        (Vec3::new(3.0, 2.0, 1.0), Vec3::new(5.0, 1.0, -8.0)),
        (Vec3::new(1.0, 2.0, 3.0), Vec3::new(-10.0, 1.0, -12.0)),
        (Vec3::new(6.0, 3.0, 1.0), Vec3::new(0.0, 1.5, -20.0)),
    ]
    .into_iter()
    .enumerate()
    {
        block(
            &format!("Crate {i}"),
            size,
            Transform::from_translation(pos),
            crate_color,
        );
    }

    // Walkable ramp (20 degrees), rising toward -Z, up to a platform.
    let angle = 20f32.to_radians();
    let length = 8.0;
    block(
        "Ramp 20deg",
        Vec3::new(3.0, 0.2, length),
        Transform::from_xyz(RAMP_X, length / 2.0 * angle.sin(), 0.0).with_rotation(Quat::from_rotation_x(angle)),
        ramp,
    );
    let top = platform_height();
    block(
        "Platform",
        Vec3::new(6.0, top, 6.0),
        Transform::from_xyz(RAMP_X, top / 2.0, -(length / 2.0 * angle.cos()) - 3.0),
        wall,
    );

    // Too steep to walk up (55 degrees).
    let angle = 55f32.to_radians();
    block(
        "Ramp 55deg",
        Vec3::new(3.0, 0.2, 5.0),
        Transform::from_xyz(STEEP_RAMP_X, 2.5 * angle.sin(), 0.0).with_rotation(Quat::from_rotation_x(angle)),
        steep,
    );

    out
}
