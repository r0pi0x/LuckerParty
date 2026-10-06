//! The components every character has, whatever controls it.

use avian3d::prelude::*;
use bevy::prelude::*;

use crate::{
    core::{BaseVelocity, EntityGravity, Health, Intent, MovementState, Team, Velocity},
    slots::set_movement,
};

/// Standing collision capsule, shared by movement implementations until a
/// game's spec says otherwise. 1.8 m tall, 0.8 m wide, origin at its center.
pub const CAPSULE_RADIUS: f32 = 0.4;
pub const CAPSULE_HEIGHT: f32 = 1.8;

pub fn character_bundle(transform: Transform, team: Team) -> impl Bundle {
    (
        Name::new("Character"),
        transform,
        Intent::default(),
        Velocity::default(),
        MovementState::default(),
        Health::default(),
        team,
        (BaseVelocity::default(), EntityGravity::default()),
        // A camera or model may be attached as a child; children need a
        // visible parent.
        Visibility::default(),
        RigidBody::Kinematic,
        Collider::capsule(CAPSULE_RADIUS, CAPSULE_HEIGHT - 2.0 * CAPSULE_RADIUS),
        // Ragdolls pass through characters.
        CollisionLayers::new(LayerMask::DEFAULT, crate::core::SOLID_LAYERS),
        // Movement implementations move the character; avian must not.
        CustomPositionIntegration,
        // Movement runs at a fixed tick; smooth it for rendering. Rotation is
        // not eased: cameras take look angles straight from `Intent`.
        TranslationInterpolation,
    )
}

/// Spawn a character using Movement implementation `movement`.
pub fn spawn_character(commands: &mut Commands, transform: Transform, team: Team, movement: &'static str) -> Entity {
    let entity = commands.spawn(character_bundle(transform, team)).id();
    commands.queue(set_movement(entity, movement));
    entity
}
