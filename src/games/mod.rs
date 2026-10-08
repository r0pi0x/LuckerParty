//! One module per game. A game module never uses another game's module
//! (enforced by tests/architecture.rs).

#[cfg(feature = "combat_arms")]
pub mod combat_arms;
pub mod cs_source;

use crate::{map::MapData, mount::config::LocalConfig};

/// Load a map by namespaced ID, e.g. `cs_source:de_dust2`, from the
/// install configured in mashup.local.toml.
pub fn load_map(id: &str) -> Result<MapData, String> {
    load_map_level(id, 0)
}

/// `load_map` at a Source `mat_hdr_level` (0 LDR, 1 bloom, 2 HDR); games
/// without HDR data ignore it.
pub fn load_map_level(id: &str, hdr_level: u8) -> Result<MapData, String> {
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
            cs_source::bsp::load_level(&mount, name, hdr_level)
        }
        other => Err(format!("no map loader for `{other}` yet")),
    }
}
