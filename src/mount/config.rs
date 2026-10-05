//! Machine-local settings from `mashup.local.toml` (gitignored): install
//! paths, and later Combat Arms keys. See `mashup.local.example.toml`.

use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
};

use serde::Deserialize;

pub const FILE_NAME: &str = "mashup.local.toml";

#[derive(Debug, Default, Deserialize)]
pub struct LocalConfig {
    #[serde(default)]
    pub games: BTreeMap<String, GameConfig>,
}

#[derive(Debug, Deserialize)]
pub struct GameConfig {
    pub path: PathBuf,
}

impl LocalConfig {
    /// Load from `$MASHUP_CONFIG`, else `./mashup.local.toml`, else the one
    /// in the source checkout (for `cargo run`/`cargo test`). Missing file
    /// means an empty config.
    pub fn load() -> Result<Self, String> {
        let candidates = [
            std::env::var_os("MASHUP_CONFIG").map(PathBuf::from),
            Some(PathBuf::from(FILE_NAME)),
            Some(Path::new(env!("CARGO_MANIFEST_DIR")).join(FILE_NAME)),
        ];
        let Some(path) = candidates.into_iter().flatten().find(|p| p.is_file()) else {
            return Ok(Self::default());
        };
        let text = fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
        toml::from_str(&text).map_err(|e| format!("{}: {e}", path.display()))
    }

    /// Install path for `game` (e.g. `cs_source`), with `~` expanded.
    pub fn game_path(&self, game: &str) -> Option<PathBuf> {
        let path = &self.games.get(game)?.path;
        Some(match path.strip_prefix("~") {
            Ok(rest) => dirs::home_dir()?.join(rest),
            Err(_) => path.clone(),
        })
    }
}

/// Where `dump` writes by default: a per-user data folder, never the repo.
pub fn default_dump_dir(game: &str) -> Option<PathBuf> {
    Some(dirs::data_local_dir()?.join("mashup").join("dump").join(game))
}
