//! Fires and flames as the map logic says they burn (specs/source/fire.md
//! 2.6, 4.4): placed fires (env_fire) by their floor point and size, and
//! flames on entities by the box of what burns. The logic layer writes
//! `MapFires` each tick; a game draws them its own way (CS:S: the
//! `env_fire_*` and `burning_character` particle systems). Engine space.

use bevy::prelude::*;

/// What burns now.
#[derive(Resource, Clone, Debug, Default, PartialEq)]
pub struct MapFires {
    pub fires: Vec<FireLook>,
    pub flames: Vec<FlameLook>,
}

/// A lit placed fire.
#[derive(Clone, Debug, PartialEq)]
pub struct FireLook {
    /// Stable while it stays lit; a fire lit again gets a new key.
    pub key: u64,
    /// The floor point it burns on, meters.
    pub at: Vec3,
    /// Its size in the game's units (`firesize`).
    pub size: f32,
    /// Flames without smoke.
    pub smokeless: bool,
}

/// A flame on an entity.
#[derive(Clone, Debug, PartialEq)]
pub struct FlameLook {
    pub key: u64,
    /// The burning entity, when it has a node.
    pub follow: Option<Entity>,
    /// Its box now (world, meters).
    pub min: Vec3,
    pub max: Vec3,
}
