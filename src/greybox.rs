//! A Map slot implementation built in code: flat ground, walls, crates, a
//! walkable ramp and a too-steep one, a tall ladder on the north wall, and a
//! water tank (ladder up the outside, deep enough to swim, low enough to
//! water-jump out). Used to test movement and weapons.
//!
//! Ladders and water only do something with Source movement
//! (`--movement cs_source:movement`, or V in game).

use avian3d::prelude::*;
use bevy::prelude::*;

use crate::{
    core::SpawnPoint,
    core::{MapBrush, MapBrushCollider, MapBrushes, MapWater, MapWaterVolume},
};

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

/// Everything the greybox map spawned, so it can be removed.
#[derive(Component)]
pub struct GreyboxPart;

/// Remove the greybox map (before loading another map).
pub fn unload(world: &mut World) {
    let parts: Vec<Entity> = world
        .query_filtered::<Entity, With<GreyboxPart>>()
        .iter(world)
        .collect();
    for e in parts {
        if let Ok(e) = world.get_entity_mut(e) {
            e.despawn();
        }
    }
    world.remove_resource::<MapBrushes>();
    world.remove_resource::<MapWater>();
}

/// Spawn the greybox map again (after another map was unloaded).
pub fn respawn(world: &mut World) {
    world.insert_resource(GlobalAmbientLight {
        brightness: 400.0,
        ..default()
    });
    if let Err(e) = world.run_system_cached(spawn) {
        error!("spawning the greybox: {e}");
    }
}

struct Block {
    name: String,
    size: Vec3,
    transform: Transform,
    color: Color,
    kind: Kind,
}

#[derive(Clone, Copy, PartialEq)]
enum Kind {
    Solid,
    /// Climbable (an axis-aligned box).
    Ladder,
    /// A water volume: no collider, drawn translucent.
    Water,
}

/// Collision always; meshes and materials only when rendering is present, so
/// the same map runs in headless tests.
fn spawn(
    mut commands: Commands,
    mut meshes: Option<ResMut<Assets<Mesh>>>,
    mut materials: Option<ResMut<Assets<StandardMaterial>>>,
) {
    let (mut ladders, mut water) = (Vec::new(), Vec::new());
    for b in blocks() {
        let (lo, hi) = (
            b.transform.translation - b.size / 2.0,
            b.transform.translation + b.size / 2.0,
        );
        let mut e = commands.spawn((Name::new(b.name), GreyboxPart, b.transform));
        match b.kind {
            Kind::Solid => {
                e.insert((RigidBody::Static, Collider::cuboid(b.size.x, b.size.y, b.size.z)));
            }
            Kind::Ladder => {
                // Source movement sweeps the brush itself (and skips this
                // collider); other movement collides with the collider.
                e.insert((
                    RigidBody::Static,
                    Collider::cuboid(b.size.x, b.size.y, b.size.z),
                    MapBrushCollider,
                ));
                ladders.push(MapBrush {
                    ladder: true,
                    ..MapBrush::from_box(lo, hi)
                });
            }
            Kind::Water => water.push(MapWaterVolume {
                brush: MapBrush::from_box(lo, hi),
                slime: false,
            }),
        }
        if let (Some(meshes), Some(materials)) = (meshes.as_mut(), materials.as_mut()) {
            e.insert((
                Mesh3d(meshes.add(Cuboid::from_size(b.size))),
                MeshMaterial3d(materials.add(StandardMaterial {
                    base_color: b.color,
                    perceptual_roughness: 0.9,
                    alpha_mode: if b.kind == Kind::Water {
                        AlphaMode::Blend
                    } else {
                        AlphaMode::Opaque
                    },
                    double_sided: b.kind == Kind::Water,
                    cull_mode: if b.kind == Kind::Water {
                        None
                    } else {
                        Some(bevy::render::render_resource::Face::Back)
                    },
                    ..default()
                })),
            ));
        }
    }
    commands.insert_resource(MapBrushes(ladders));
    commands.insert_resource(MapWater(water));

    commands.spawn((
        Name::new("Sun"),
        GreyboxPart,
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
            GreyboxPart,
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
    let mut add = |name: &str, size: Vec3, transform: Transform, color: Color, kind: Kind| {
        out.push(Block {
            name: name.to_string(),
            size,
            transform,
            color,
            kind,
        });
    };
    let mut block = |name: &str, size: Vec3, transform: Transform, color: Color| {
        add(name, size, transform, color, Kind::Solid);
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

    // A ladder up the north wall to its top (6 m).
    let ladder = Color::srgb(0.20, 0.18, 0.15);
    add(
        "Ladder (north wall)",
        Vec3::new(0.8, 6.0, 0.1),
        Transform::from_xyz(LADDER_X, 3.0, NORTH_WALL_Z + 0.05),
        ladder,
        Kind::Ladder,
    );

    // Water tank: 2.8 m of water (over head height) inside 3 m walls. A
    // ladder up the east side; swim to a wall at the surface and move
    // toward it to water-jump out.
    let (lo, hi) = (TANK_MIN, TANK_MAX);
    let centre = (lo + hi) / 2.0;
    let size = hi - lo;
    let (t, h) = (0.4, 3.0);
    for (name, s, p) in [
        (
            "Tank wall W",
            Vec3::new(t, h, size.y + 2.0 * t),
            Vec3::new(lo.x - t / 2.0, h / 2.0, centre.y),
        ),
        (
            "Tank wall E",
            Vec3::new(t, h, size.y + 2.0 * t),
            Vec3::new(hi.x + t / 2.0, h / 2.0, centre.y),
        ),
        (
            "Tank wall N",
            Vec3::new(size.x, h, t),
            Vec3::new(centre.x, h / 2.0, lo.y - t / 2.0),
        ),
        (
            "Tank wall S",
            Vec3::new(size.x, h, t),
            Vec3::new(centre.x, h / 2.0, hi.y + t / 2.0),
        ),
    ] {
        add(name, s, Transform::from_translation(p), wall, Kind::Solid);
    }
    add(
        "Water",
        Vec3::new(size.x, TANK_WATER_DEPTH, size.y),
        Transform::from_xyz(centre.x, TANK_WATER_DEPTH / 2.0, centre.y),
        Color::srgba(0.15, 0.35, 0.55, 0.45),
        Kind::Water,
    );
    add(
        "Ladder (tank)",
        Vec3::new(0.1, h, 0.8),
        Transform::from_xyz(hi.x + t + 0.05, h / 2.0, centre.y),
        ladder,
        Kind::Ladder,
    );

    out
}

/// The north-wall ladder's x, and the water tank's inside (x, z) and water
/// depth, meters.
pub const LADDER_X: f32 = -15.0;
pub const TANK_MIN: Vec2 = Vec2::new(-34.0, 24.0);
pub const TANK_MAX: Vec2 = Vec2::new(-26.0, 34.0);
pub const TANK_WATER_DEPTH: f32 = 2.8;
