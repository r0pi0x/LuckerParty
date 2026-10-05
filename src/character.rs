//! Spawning characters and the first-person camera.

use avian3d::prelude::*;
use bevy::prelude::*;

use crate::{
    core::{Health, Intent, LocalPlayer, MovementState, SpawnPoint, Team, Velocity},
    slots::{Loadout, set_movement},
};

/// Standing collision capsule, shared by movement implementations until a
/// game's spec says otherwise. 1.8 m tall, 0.8 m wide, origin at its center.
pub const CAPSULE_RADIUS: f32 = 0.4;
pub const CAPSULE_HEIGHT: f32 = 1.8;

#[derive(Component)]
pub struct FirstPersonCamera;

pub struct CharacterPlugin;

impl Plugin for CharacterPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(PostStartup, spawn_local_player)
            .add_systems(Update, follow_eye);
    }
}

/// The components every character has, whatever controls it.
pub fn character_bundle(transform: Transform, team: Team) -> impl Bundle {
    (
        Name::new("Character"),
        transform,
        Intent::default(),
        Velocity::default(),
        MovementState::default(),
        Health::default(),
        team,
        // The camera is a child; children need a visible parent.
        Visibility::default(),
        RigidBody::Kinematic,
        Collider::capsule(CAPSULE_RADIUS, CAPSULE_HEIGHT - 2.0 * CAPSULE_RADIUS),
        // Movement implementations move the character; avian must not.
        CustomPositionIntegration,
        // Movement runs at a fixed tick; smooth it for rendering. Rotation is
        // not eased: the camera takes look angles straight from `Intent`.
        TranslationInterpolation,
    )
}

fn spawn_local_player(mut commands: Commands, spawns: Query<&Transform, With<SpawnPoint>>, loadout: Res<Loadout>) {
    let at = spawns.iter().next().copied().unwrap_or_default();
    let player = commands
        .spawn((character_bundle(at, Team(0)), LocalPlayer))
        .with_child((
            FirstPersonCamera,
            Camera3d::default(),
            Projection::Perspective(PerspectiveProjection {
                fov: 74f32.to_radians(),
                ..default()
            }),
        ))
        .id();
    commands.queue(set_movement(player, loadout.movement));
}

/// Place the camera at the movement implementation's eye position and aim it
/// along the look angles.
fn follow_eye(
    players: Query<(&Intent, &MovementState, &Children), With<LocalPlayer>>,
    mut cameras: Query<&mut Transform, With<FirstPersonCamera>>,
) {
    for (intent, state, children) in &players {
        let mut cams = cameras.iter_many_mut(children);
        while let Some(mut cam) = cams.fetch_next() {
            cam.translation = state.eye_offset;
            cam.rotation = intent.look_rotation();
        }
    }
}
