//! Machine-local settings from `mashup.local.toml` (gitignored): install
//! paths (optional: `install` finds them otherwise) and Combat Arms keys.
//! See `mashup.local.example.toml`.

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
    /// The file it was read from.
    #[serde(skip)]
    pub file: Option<PathBuf>,
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
        let mut config: Self = toml::from_str(&text).map_err(|e| format!("{}: {e}", path.display()))?;
        config.file = Some(fs::canonicalize(&path).unwrap_or(path));
        Ok(config)
    }

    /// Install path for `game` (e.g. `cs_source`), with `~` expanded:
    /// this file's, else the one the user saved, else Steam's
    /// (`install::resolve`, which also says how it was found).
    pub fn game_path(&self, game: &str) -> Option<PathBuf> {
        super::install::resolve(self, game).install.map(|i| i.path)
    }
}

impl LocalConfig {
    /// Keys configured for `game`, by scheme name.
    pub fn game_keys(&self, game: &str) -> Option<&BTreeMap<String, KeyConfig>> {
        self.games.get(game).map(|g| &g.keys)
    }
}

/// Mashup's own content cache for a game (maps fetched or imported, laid
/// out like the game's download folder): a per-user data folder, never
/// the game install and never the repo (docs/plans/active/custom-maps.md).
pub fn content_dir(game: &str) -> Option<PathBuf> {
    Some(dirs::data_local_dir()?.join("mashup").join("content").join(game))
}

/// Where `dump` writes by default: a per-user data folder, never the repo.
pub fn default_dump_dir(game: &str) -> Option<PathBuf> {
    Some(dirs::data_local_dir()?.join("mashup").join("dump").join(game))
}
