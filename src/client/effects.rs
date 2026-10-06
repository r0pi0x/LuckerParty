//! Shot feedback until the games' own effects load: tracer streaks and
//! bullet marks on static surfaces. Placeholders, not CS:S's look (its
//! tracer sprites, impact decals by surface and particle puffs).

use std::collections::VecDeque;

use avian3d::prelude::RigidBody;
use bevy::prelude::*;

use crate::{
    core::LocalPlayer,
    weapon::{WeaponEvent, WeaponEventKind},
};

pub struct ShotEffectsPlugin;

impl Plugin for ShotEffectsPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Marks>()
            .add_systems(Startup, setup)
            .add_systems(Update, (spawn_effects, move_tracers));
    }
}

/// Tracer speed and length, m/s and m (unmeasured; looks plausible).
const TRACER_SPEED: f32 = 250.0;
const TRACER_LENGTH: f32 = 2.5;
const TRACER_WIDTH: f32 = 0.012;
/// The local player's tracers start this far below and right of the eye
/// (roughly where a gun would be), and only every Nth shot shows one.
const LOCAL_OFFSET: Vec3 = Vec3::new(0.12, -0.12, 0.0);
const LOCAL_TRACER_EVERY: u32 = 3;
const MARK_SIZE: f32 = 0.05;
const MAX_MARKS: usize = 128;

#[derive(Resource)]
struct Assets3 {
    tracer: Handle<StandardMaterial>,
    mark: Handle<StandardMaterial>,
    unit_cube: Handle<Mesh>,
    quad: Handle<Mesh>,
}

#[derive(Resource, Default)]
struct Marks {
    spawned: VecDeque<Entity>,
    local_shots: u32,
}

#[derive(Component)]
struct Tracer {
    from: Vec3,
    dir: Vec3,
    length: f32,
    travelled: f32,
}

fn setup(mut commands: Commands, mut meshes: ResMut<Assets<Mesh>>, mut materials: ResMut<Assets<StandardMaterial>>) {
    commands.insert_resource(Assets3 {
        tracer: materials.add(StandardMaterial {
            base_color: Color::srgba(1.0, 0.85, 0.5, 0.8),
            unlit: true,
            alpha_mode: AlphaMode::Add,
            ..default()
        }),
        mark: materials.add(StandardMaterial {
            base_color: Color::srgba(0.05, 0.04, 0.03, 0.85),
            unlit: true,
            alpha_mode: AlphaMode::Blend,
            depth_bias: 50.0,
            ..default()
        }),
        unit_cube: meshes.add(Cuboid::new(1.0, 1.0, 1.0)),
        quad: meshes.add(Circle::new(MARK_SIZE / 2.0)),
    });
}

fn spawn_effects(
    mut events: MessageReader<WeaponEvent>,
    local: Option<Single<Entity, With<LocalPlayer>>>,
    bodies: Query<&RigidBody>,
    assets: Res<Assets3>,
    mut marks: ResMut<Marks>,
    decals: Option<Res<crate::map::decal::ImpactDecals>>,
    mut commands: Commands,
) {
    let me = local.map(|l| *l);
    for e in events.read() {
        let WeaponEventKind::Shot { from, to, hit, normal } = &e.kind else {
            continue;
        };
        let mine = Some(e.owner) == me;
        let show = if mine {
            marks.local_shots += 1;
            marks.local_shots % LOCAL_TRACER_EVERY == 1
        } else {
            true
        };
        if show {
            let dir = (*to - *from).normalize_or_zero();
            // Offset the local player's start toward the gun side.
            let start = if mine {
                let right = dir.cross(Vec3::Y).normalize_or_zero();
                *from + right * LOCAL_OFFSET.x + Vec3::Y * LOCAL_OFFSET.y + dir * 0.5
            } else {
                *from + dir * 0.6
            };
            let length = (*to - start).length();
            commands.spawn((
                Tracer {
                    from: start,
                    dir: (*to - start).normalize_or_zero(),
                    length,
                    travelled: 0.0,
                },
                Mesh3d(assets.unit_cube.clone()),
                MeshMaterial3d(assets.tracer.clone()),
                Transform::from_translation(start).with_scale(Vec3::ZERO),
            ));
        }
        // Marks only on things that don't move, and only where the map
        // has no decals of its own.
        let (Some(hit), Some(n)) = (hit, normal) else { continue };
        if decals.is_some() {
            continue;
        }
        if bodies.get(*hit).is_ok_and(|b| !b.is_static()) {
            continue;
        }
        let mark = commands
            .spawn((
                Mesh3d(assets.quad.clone()),
                MeshMaterial3d(assets.mark.clone()),
                Transform::from_translation(*to + *n * 0.003).looking_to(-*n, any_up(*n)),
            ))
            .id();
        marks.spawned.push_back(mark);
        while marks.spawned.len() > MAX_MARKS {
            if let Some(old) = marks.spawned.pop_front() {
                commands.entity(old).despawn();
            }
        }
    }
}

fn any_up(n: Vec3) -> Vec3 {
    if n.y.abs() > 0.9 { Vec3::Z } else { Vec3::Y }
}

/// Move each streak along its path; despawn at the end.
fn move_tracers(mut tracers: Query<(Entity, &mut Tracer, &mut Transform)>, time: Res<Time>, mut commands: Commands) {
    for (e, mut t, mut tf) in &mut tracers {
        t.travelled += TRACER_SPEED * time.delta_secs();
        let head = t.travelled.min(t.length);
        let tail = (t.travelled - TRACER_LENGTH).max(0.0);
        if tail >= t.length {
            commands.entity(e).despawn();
            continue;
        }
        let mid = t.from + t.dir * (head + tail) / 2.0;
        *tf = Transform::from_translation(mid)
            .looking_to(t.dir, any_up(t.dir))
            .with_scale(Vec3::new(TRACER_WIDTH, TRACER_WIDTH, (head - tail).max(0.001)));
    }
}
