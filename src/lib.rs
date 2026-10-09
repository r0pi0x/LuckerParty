//! Mashup prototype. Module layering is documented in docs/ARCHITECTURE.md
//! and enforced by tests/it/architecture.rs.

pub mod bot;
pub mod character;
pub mod client;
pub mod console;
pub mod core;
pub mod games;
pub mod greybox;
pub mod harness;
pub mod logic;
pub mod map;
pub mod metrics;
pub mod mount;
pub mod movement;
pub mod net;
pub mod objectives;
pub mod rules;
pub mod slots;
pub mod weapon;

use bevy::prelude::*;

/// Default fixed tick rate until a ruleset or movement spec sets one.
pub const DEFAULT_TICK_HZ: f64 = 64.0;

/// Everything the simulation needs, with no window, rendering or input.
/// Used by the game, headless tests and (later) a dedicated server.
pub struct SimPlugins;

impl Plugin for SimPlugins {
    fn build(&self, app: &mut App) {
        app.insert_resource(Time::<Fixed>::from_hz(DEFAULT_TICK_HZ))
            .add_plugins((core::CorePlugin, console::ConsolePlugin, movement::MovementPlugins))
            .add_plugins((
                weapon::WeaponPlugin,
                objectives::ObjectivesPlugin,
                rules::DeathmatchPlugin,
                bot::BotPlugin,
            ))
            .add_plugins((logic::LogicPlugin, metrics::TickMetricsPlugin));
    }
}

/// Put a map in place of the one playing, as the game's `map` does
/// without the view (the dedicated server's `changelevel`, tests): `data`
/// for map `id`, or the greybox (None); the tick its game runs at;
/// everyone respawned in a new game there. The fixed clock keeps running
/// (`core::set_tick_length`); a network server takes its clients along
/// (`net::maps`).
pub fn swap_map(world: &mut World, id: &str, data: Option<map::MapData>, tick: std::time::Duration) {
    match data {
        Some(data) => {
            greybox::unload(world);
            map::change_map(world, data, map::MapDebugView::Normal);
            world.insert_resource(map::LoadedMapName(id.to_string()));
        }
        None => {
            map::unload_map(world);
            greybox::unload(world);
            greybox::respawn(world);
            world.remove_resource::<map::ActiveMapLook>();
            world.insert_resource(map::LoadedMapName(net::GREYBOX.into()));
        }
    }
    core::set_tick_length(world, tick);
    rules::new_game(world);
}
