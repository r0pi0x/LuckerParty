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
    /// Decryption keys by scheme name (Combat Arms: `V1`, `V2`).
    #[serde(default)]
    pub keys: BTreeMap<String, KeyConfig>,
}

/// A key and IV as hexadecimal text. Never logged or echoed in errors.
#[derive(Deserialize)]
pub struct KeyConfig {
    pub key: String,
    pub iv: String,
}

impl std::fmt::Debug for KeyConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("KeyConfig { .. }")
    }
}

/// Decode hex text into exactly `N` bytes. Errors never include the text.
pub fn hex_bytes<const N: usize>(text: &str, what: &str) -> Result<[u8; N], String> {
    let text = text.trim();
    if text.len() != N * 2 || !text.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(format!("{what}: expected {} hexadecimal characters", N * 2));
    }
    let mut out = [0u8; N];
    for (i, byte) in out.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&text[2 * i..2 * i + 2], 16).unwrap();
    }
    Ok(out)
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

impl LocalConfig {
    /// Keys configured for `game`, by scheme name.
    pub fn game_keys(&self, game: &str) -> Option<&BTreeMap<String, KeyConfig>> {
        self.games.get(game).map(|g| &g.keys)
    }
}

/// Where `dump` writes by default: a per-user data folder, never the repo.
pub fn default_dump_dir(game: &str) -> Option<PathBuf> {
    Some(dirs::data_local_dir()?.join("mashup").join("dump").join(game))
}
