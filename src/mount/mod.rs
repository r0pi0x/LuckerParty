//! Shared mount plumbing: a read-only virtual filesystem over a game's
//! archives and loose files, plus local configuration and finding the
//! installs (`install`: config, saved settings, Steam libraries).
//! Game-specific archive formats live in `games/<name>/`.

pub mod config;
pub mod install;
pub mod keyvalues;

use std::{
    collections::BTreeMap,
    fs, io,
    path::{Path, PathBuf},
};

/// One file visible through a mount.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    /// Normalized path: lowercase, `/` separators, no leading slash.
    pub path: String,
    pub size: u64,
}

/// A source of files: an archive or a loose directory.
pub trait FileSource: Send + Sync {
    /// Human-readable name for listings and errors.
    fn name(&self) -> String;
    fn entries(&self) -> Vec<Entry>;
    /// `None` if this source doesn't have `path` (already normalized).
    fn read(&self, path: &str) -> Option<io::Result<Vec<u8>>>;
}

/// Normalize a path the way Source and LithTech resolve them:
/// case-insensitive, either slash.
pub fn normalize(path: &str) -> String {
    // Doubled separators occur in game data (e.g. a model's texture
    // directory `models\props_junk\\`); file systems ignore them, so do we.
    // So are `.` components (static prop names like `./models/...` on
    // de_nuke and de_train).
    let path = path.replace('\\', "/").to_lowercase();
    path.split('/')
        .filter(|p| !p.is_empty() && *p != ".")
        .collect::<Vec<_>>()
        .join("/")
}

/// Layers searched in order; the first layer that has a file wins, like a
/// game's search path.
#[derive(Default)]
pub struct Mount {
    layers: Vec<Box<dyn FileSource>>,
}

impl Mount {
    pub fn push(&mut self, layer: impl FileSource + 'static) {
        self.layers.push(Box::new(layer));
    }

    pub fn layers(&self) -> impl Iterator<Item = &dyn FileSource> {
        self.layers.iter().map(|l| l.as_ref())
    }

    pub fn read(&self, path: &str) -> io::Result<Vec<u8>> {
        let path = normalize(path);
        self.layers
            .iter()
            .find_map(|l| l.read(&path))
            .unwrap_or_else(|| Err(io::Error::new(io::ErrorKind::NotFound, format!("not in mount: {path}"))))
    }

    /// Every visible file, sorted by path, with the layer it comes from.
    /// Files shadowed by an earlier layer are omitted.
    pub fn entries(&self) -> Vec<(Entry, usize)> {
        let mut seen = BTreeMap::new();
        for (i, layer) in self.layers.iter().enumerate() {
            for e in layer.entries() {
                seen.entry(e.path.clone()).or_insert((e, i));
            }
        }
        seen.into_values().collect()
    }
}

/// Loose files under a directory.
pub struct LooseDir {
    root: PathBuf,
    /// Lowercase extensions to hide, e.g. archives that are mounted as their
    /// own layers.
    hidden_exts: Vec<&'static str>,
}

impl LooseDir {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self {
            root: root.into(),
            hidden_exts: Vec::new(),
        }
    }

    pub fn hiding(mut self, exts: &[&'static str]) -> Self {
        self.hidden_exts.extend_from_slice(exts);
        self
    }

    fn hidden(&self, path: &str) -> bool {
        path.rsplit_once('.')
            .is_some_and(|(_, ext)| self.hidden_exts.contains(&ext))
    }

    fn walk(dir: &Path, root: &Path, out: &mut Vec<Entry>) {
        let Ok(rd) = fs::read_dir(dir) else { return };
        for entry in rd.flatten() {
            let path = entry.path();
            let Ok(meta) = entry.metadata() else { continue };
            if meta.is_dir() {
                Self::walk(&path, root, out);
            } else if let Ok(rel) = path.strip_prefix(root) {
                out.push(Entry {
                    path: normalize(&rel.to_string_lossy()),
                    size: meta.len(),
                });
            }
        }
    }
}

impl FileSource for LooseDir {
    fn name(&self) -> String {
        self.root.display().to_string()
    }

    fn entries(&self) -> Vec<Entry> {
        let mut out = Vec::new();
        Self::walk(&self.root, &self.root, &mut out);
        out.retain(|e| !self.hidden(&e.path));
        out
    }

    fn read(&self, path: &str) -> Option<io::Result<Vec<u8>>> {
        if self.hidden(path) {
            return None;
        }
        // Fast path: exact case. Otherwise match case-insensitively.
        let direct = self.root.join(path);
        if direct.is_file() {
            return Some(fs::read(direct));
        }
        let mut dir = self.root.clone();
        for part in path.split('/') {
            let found = fs::read_dir(&dir)
                .ok()?
                .flatten()
                .find(|e| e.file_name().to_string_lossy().eq_ignore_ascii_case(part))?;
            dir = found.path();
        }
        dir.is_file().then(|| fs::read(dir))
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn normalize_drops_empty_and_dot_components() {
        assert_eq!(super::normalize("./Models\\props//a.MDL"), "models/props/a.mdl");
    }
}
