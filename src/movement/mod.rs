//! Movement slot implementations that don't belong to any one game.
//! Game-specific movement (e.g. `combat_arms`) lives in that game's plugin.

pub mod noclip;
pub mod placeholder;

use bevy::prelude::*;

pub struct MovementPlugins;

impl Plugin for MovementPlugins {
    fn build(&self, app: &mut App) {
        app.add_plugins((placeholder::PlaceholderMovementPlugin, noclip::NoclipPlugin));
    }
}

/// Desired horizontal direction in world space from a move axis and yaw.
/// Length is at most 1.
pub fn wish_dir(move_axis: Vec2, yaw: Quat) -> Vec3 {
    (yaw * Vec3::new(move_axis.x, 0.0, -move_axis.y)).clamp_length_max(1.0)
}
