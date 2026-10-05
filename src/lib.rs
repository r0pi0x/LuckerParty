//! Mashup prototype. Module layering is documented in docs/ARCHITECTURE.md
//! and enforced by tests/architecture.rs.

pub mod character;
pub mod client;
pub mod core;
pub mod games;
pub mod greybox;
pub mod harness;
pub mod map;
pub mod mount;
pub mod movement;
pub mod slots;

use bevy::prelude::*;

/// Default fixed tick rate until a ruleset or movement spec sets one.
pub const DEFAULT_TICK_HZ: f64 = 64.0;

/// Everything the simulation needs, with no window, rendering or input.
/// Used by the game, headless tests and (later) a dedicated server.
pub struct SimPlugins;

impl Plugin for SimPlugins {
    fn build(&self, app: &mut App) {
        app.insert_resource(Time::<Fixed>::from_hz(DEFAULT_TICK_HZ))
            .add_plugins((core::CorePlugin, movement::MovementPlugins));
    }
}
