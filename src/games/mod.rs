//! One module per game. A game module never uses another game's module
//! (enforced by tests/architecture.rs).

pub mod combat_arms;
pub mod cs_source;

use crate::{map::MapData, mount::config::LocalConfig};

/// Load a map by namespaced ID, e.g. `cs_source:de_dust2`, from the
/// install configured in mashup.local.toml.
pub fn load_map(id: &str) -> Result<MapData, String> {
    let (game, name) = id
        .split_once(':')
        .ok_or_else(|| format!("map ID `{id}` should look like game:name"))?;
    let config = LocalConfig::load()?;
    let install = config
        .game_path(game)
        .ok_or_else(|| format!("no install path for `{game}` in mashup.local.toml"))?;
    match game {
        cs_source::GAME => {
            let mount = cs_source::mount::open(&install).map_err(|e| e.to_string())?;
            cs_source::bsp::load(&mount, name)
        }
        other => Err(format!("no map loader for `{other}` yet")),
    }
}
