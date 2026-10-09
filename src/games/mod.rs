//! One module per game. A game module never uses another game's module
//! (enforced by tests/it/architecture.rs).

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

/// `load_map_level` from a file of the map (a downloaded copy), its
/// content from the game's install.
pub fn load_map_file(id: &str, file: &std::path::Path, hdr_level: u8) -> Result<MapData, String> {
    let (game, name) = id
        .split_once(':')
        .ok_or_else(|| format!("map ID `{id}` should look like game:name"))?;
    let config = LocalConfig::load()?;
    let install = config
        .game_path(game)
        .ok_or_else(|| format!("no install path for `{game}` in mashup.local.toml"))?;
    let bytes = std::fs::read(file).map_err(|e| format!("{}: {e}", file.display()))?;
    match game {
        cs_source::GAME => {
            let mount = cs_source::mount::open(&install).map_err(|e| e.to_string())?;
            cs_source::bsp::load_level_bytes(&mount, name, bytes, hdr_level)
        }
        other => Err(format!("no map loader for `{other}` yet")),
    }
}

/// The bytes of map `id`'s file as `load_map` reads it (the install, its
/// downloads, mashup's content cache), for the network's map offer and
/// hash checks (`net::maps::MapFiles`). None if there's none.
pub fn map_file_bytes(id: &str) -> Option<Vec<u8>> {
    let (game, name) = id.split_once(':')?;
    let install = LocalConfig::load().ok()?.game_path(game)?;
    match game {
        cs_source::GAME => cs_source::mount::open(&install)
            .ok()?
            .read(&format!("maps/{name}.bsp"))
            .ok(),
        _ => None,
    }
}
