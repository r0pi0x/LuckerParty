//! Finding a game's install, in this order:
//!
//! 1. its `path` in `mashup.local.toml` (`config::LocalConfig`: an
//!    explicit path always wins, even one that doesn't check out);
//! 2. the path saved in the user's settings file (`settings_path`: the
//!    first-run dialog and `mashup_install` write it), while it still
//!    checks out;
//! 3. Steam: each Steam root (`steam_roots`; `$MASHUP_STEAM_ROOT`
//!    replaces the usual places), its libraries
//!    (`steamapps/libraryfolders.vdf`, old and new formats), the library
//!    with the app's `appmanifest_<id>.acf` (fully installed), its
//!    `steamapps/common/<installdir>` holding the files the game needs.
//!
//! The Steam build id comes with it (from the manifest) for the mount
//! doctor. Only games with an entry in `STEAM_GAMES` are looked for on
//! Steam; the table is data (app id, required files), so this layer
//! stays below the game modules.

use std::{
    collections::BTreeMap,
    fmt, fs,
    path::{Path, PathBuf},
    sync::Mutex,
};

use serde::{Deserialize, Serialize};

use super::{config::LocalConfig, keyvalues};

/// A game Steam installs: our game id, its Steam app, the files (relative
/// to the install folder) that show it is that game.
#[derive(Clone, Copy, Debug)]
pub struct SteamGame {
    pub game: &'static str,
    pub app_id: u32,
    pub name: &'static str,
    pub required: &'static [&'static str],
}

pub const STEAM_GAMES: &[SteamGame] = &[SteamGame {
    game: "cs_source",
    app_id: 240,
    name: "Counter-Strike: Source",
    required: &["cstrike/cstrike_pak_dir.vpk", "hl2/hl2_misc_dir.vpk"],
}];

pub fn steam_game(game: &str) -> Option<&'static SteamGame> {
    STEAM_GAMES.iter().find(|g| g.game == game)
}

/// Replaces the usual Steam roots (paths joined as `PATH` is; empty: no
/// Steam at all), for tests and for trying the first-run dialog.
pub const STEAM_ROOT_ENV: &str = "MASHUP_STEAM_ROOT";
/// Replaces the user's settings file.
pub const SETTINGS_ENV: &str = "MASHUP_SETTINGS";

/// How an install was found.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Found {
    /// `mashup.local.toml` (the file, when known).
    Config(Option<PathBuf>),
    /// The user's settings file.
    Saved(PathBuf),
    /// Steam: the library's number in `root`'s `libraryfolders.vdf` (0:
    /// the Steam folder itself) and its folder.
    Steam { root: PathBuf, library: usize, library_path: PathBuf },
}

impl fmt::Display for Found {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Found::Config(Some(file)) => write!(f, "config {}", file.display()),
            Found::Config(None) => f.write_str("config"),
            Found::Saved(file) => write!(f, "saved settings {}", file.display()),
            Found::Steam { library, library_path, .. } => {
                write!(f, "Steam library {library} ({})", library_path.display())
            }
        }
    }
}

/// A game's install folder, how it was found, its Steam build.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Install {
    pub path: PathBuf,
    pub found: Found,
    pub build_id: Option<String>,
}

/// The outcome of a search: the install, and what was tried on the way
/// (paths that didn't check out, libraries without the app).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Resolved {
    pub install: Option<Install>,
    pub notes: Vec<String>,
}

impl Resolved {
    /// One line for the log and the console.
    pub fn describe(&self, name: &str) -> String {
        match &self.install {
            Some(i) => {
                let build = i.build_id.as_deref().map(|b| format!(", build {b}")).unwrap_or_default();
                format!("{name} install: {} (from {}{build})", i.path.display(), i.found)
            }
            None if self.notes.is_empty() => format!("{name} install: none found"),
            None => format!("{name} install: none found ({})", self.notes.join("; ")),
        }
    }
}

// ---------------------------------------------------------------------------
// Checking a folder.

/// Whether `path` holds `game`'s files: Ok, or what's missing.
pub fn check(path: &Path, game: &SteamGame) -> Result<(), String> {
    if !path.is_dir() {
        return Err(format!("{}: not a folder", path.display()));
    }
    let missing: Vec<&str> = game.required.iter().copied().filter(|f| !path.join(f).is_file()).collect();
    if missing.is_empty() {
        Ok(())
    } else {
        Err(format!("{} has no {}: not a {} folder", path.display(), missing.join(" or "), game.name))
    }
}

/// `~` at the start means the home folder.
pub fn expand_home(path: &Path) -> PathBuf {
    match path.strip_prefix("~") {
        Ok(rest) => dirs::home_dir().map_or_else(|| path.to_path_buf(), |h| h.join(rest)),
        Err(_) => path.to_path_buf(),
    }
}

// ---------------------------------------------------------------------------
// Steam.

/// A Steam app manifest's facts.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Manifest {
    pub install_dir: String,
    pub build_id: Option<String>,
    pub state_flags: u32,
}

/// `StateFlags` bit: fully installed.
const FULLY_INSTALLED: u32 = 4;

impl Manifest {
    pub fn fully_installed(&self) -> bool {
        self.state_flags & FULLY_INSTALLED != 0
    }
}

/// Read an `appmanifest_<id>.acf`.
pub fn read_manifest(path: &Path) -> Option<Manifest> {
    let text = fs::read_to_string(path).ok()?;
    let root = keyvalues::parse(&text);
    let state = root.get("AppState")?;
    Some(Manifest {
        install_dir: state.value("installdir")?.to_string(),
        build_id: state.value("buildid").map(String::from),
        state_flags: state.value("StateFlags").and_then(|v| v.trim().parse().ok()).unwrap_or(0),
    })
}

/// A Steam library: its number in `libraryfolders.vdf`, its folder, the
/// apps it lists (none in the old format).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Library {
    pub index: usize,
    pub path: PathBuf,
    pub apps: Vec<u32>,
}

/// The libraries of a Steam root: the root itself (library 0) and those
/// `steamapps/libraryfolders.vdf` names. Both formats: the current one
/// (`"1" { "path" "..." "apps" { "240" "..." } }`) and the old one
/// (`"1" "D:\\SteamLibrary"`).
pub fn libraries(root: &Path) -> Vec<Library> {
    let mut out: Vec<Library> = Vec::new();
    let text = ["steamapps/libraryfolders.vdf", "config/libraryfolders.vdf"]
        .iter()
        .find_map(|f| fs::read_to_string(root.join(f)).ok())
        .unwrap_or_default();
    let kv = keyvalues::parse(&text);
    let top = kv.children().first().map(|(_, v)| v);
    for (key, node) in top.map(keyvalues::Node::children).unwrap_or(&[]) {
        let Ok(index) = key.trim().parse::<usize>() else {
            continue;
        };
        let (path, apps) = match node {
            keyvalues::Node::Value(p) => (p.clone(), Vec::new()),
            keyvalues::Node::Block(_) => {
                let Some(p) = node.value("path") else { continue };
                let apps = node
                    .get("apps")
                    .map_or(Vec::new(), |a| a.children().iter().filter_map(|(k, _)| k.trim().parse().ok()).collect());
                (p.to_string(), apps)
            }
        };
        out.push(Library { index, path: PathBuf::from(path), apps });
    }
    if !out.iter().any(|l| same_dir(&l.path, root)) {
        out.insert(0, Library { index: 0, path: root.to_path_buf(), apps: Vec::new() });
    }
    out
}

fn same_dir(a: &Path, b: &Path) -> bool {
    a == b || matches!((fs::canonicalize(a), fs::canonicalize(b)), (Ok(x), Ok(y)) if x == y)
}

/// Look for `game` in the libraries of `roots`.
pub fn find_in_steam(roots: &[PathBuf], game: &SteamGame) -> Resolved {
    let mut notes = Vec::new();
    let manifest_name = format!("appmanifest_{}.acf", game.app_id);
    for root in roots {
        let mut libs = libraries(root);
        // Libraries that list the app first; the manifests decide.
        libs.sort_by_key(|l| !l.apps.contains(&game.app_id));
        for lib in libs {
            let listed = lib.apps.contains(&game.app_id);
            let steamapps = lib.path.join("steamapps");
            let Some(manifest) = read_manifest(&steamapps.join(&manifest_name)) else {
                if listed {
                    notes.push(format!(
                        "Steam library {} lists app {} but has no {manifest_name}",
                        lib.index, game.app_id
                    ));
                }
                continue;
            };
            if !manifest.fully_installed() {
                notes.push(format!(
                    "Steam library {}: {} isn't fully installed (StateFlags {})",
                    lib.index, game.name, manifest.state_flags
                ));
                continue;
            }
            let path = steamapps.join("common").join(&manifest.install_dir);
            if let Err(e) = check(&path, game) {
                notes.push(format!("Steam library {}: {e}", lib.index));
                continue;
            }
            return Resolved {
                install: Some(Install {
                    path,
                    found: Found::Steam { root: root.clone(), library: lib.index, library_path: lib.path.clone() },
                    build_id: manifest.build_id,
                }),
                notes,
            };
        }
    }
    if roots.is_empty() {
        notes.push("no Steam folder found".into());
    } else if notes.is_empty() {
        notes.push(format!("no Steam library has {} (app {})", game.name, game.app_id));
    }
    Resolved { install: None, notes }
}

/// Steam folders to search: `$MASHUP_STEAM_ROOT` if set, else where
/// Steam lives on this system; existing ones, each once.
pub fn steam_roots() -> Vec<PathBuf> {
    let candidates = match std::env::var_os(STEAM_ROOT_ENV) {
        Some(v) => std::env::split_paths(&v).filter(|p| !p.as_os_str().is_empty()).collect(),
        None => platform_steam_roots(),
    };
    let mut out: Vec<PathBuf> = Vec::new();
    let mut seen = Vec::new();
    for c in candidates {
        let Ok(canon) = fs::canonicalize(&c) else { continue };
        if canon.is_dir() && !seen.contains(&canon) {
            seen.push(canon);
            out.push(c);
        }
    }
    out
}

/// Where Steam installs itself on this system (existing or not).
#[allow(clippy::vec_init_then_push)]
fn platform_steam_roots() -> Vec<PathBuf> {
    let mut out = Vec::new();
    #[cfg(windows)]
    {
        out.extend(windows_registry_steam_path());
        if let Some(pf) = std::env::var_os("ProgramFiles(x86)") {
            out.push(PathBuf::from(pf).join("Steam"));
        }
        out.push(PathBuf::from(r"C:\Program Files (x86)\Steam"));
    }
    #[cfg(target_os = "macos")]
    if let Some(home) = dirs::home_dir() {
        out.push(home.join("Library/Application Support/Steam"));
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    if let Some(home) = dirs::home_dir() {
        out.push(home.join(".steam/steam"));
        out.push(home.join(".steam/root"));
        out.push(home.join(".local/share/Steam"));
        // Flatpak.
        out.push(home.join(".var/app/com.valvesoftware.Steam/data/Steam"));
        out.push(home.join(".var/app/com.valvesoftware.Steam/.local/share/Steam"));
    }
    out
}

/// `HKCU\Software\Valve\Steam` `SteamPath`.
#[cfg(windows)]
fn windows_registry_steam_path() -> Option<PathBuf> {
    use windows_sys::Win32::System::Registry::{HKEY_CURRENT_USER, RRF_RT_REG_SZ, RegGetValueW};
    let wide = |s: &str| s.encode_utf16().chain(Some(0)).collect::<Vec<u16>>();
    let (key, value) = (wide(r"Software\Valve\Steam"), wide("SteamPath"));
    let mut len: u32 = 0;
    // SAFETY: the key and value names are NUL-terminated UTF-16; a null
    // buffer asks only for the size, written to `len`.
    let err = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            key.as_ptr(),
            value.as_ptr(),
            RRF_RT_REG_SZ,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            &mut len,
        )
    };
    if err != 0 || len == 0 {
        return None;
    }
    let mut buf = vec![0u16; (len as usize).div_ceil(2)];
    let mut len = (buf.len() * 2) as u32;
    // SAFETY: `buf` holds `len` bytes.
    let err = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            key.as_ptr(),
            value.as_ptr(),
            RRF_RT_REG_SZ,
            std::ptr::null_mut(),
            buf.as_mut_ptr().cast(),
            &mut len,
        )
    };
    if err != 0 {
        return None;
    }
    let n = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
    let path = String::from_utf16_lossy(&buf[..n]);
    (!path.is_empty()).then(|| PathBuf::from(path))
}

/// The Steam build of an install found some other way: its library's
/// manifest, when it sits in `<library>/steamapps/common/<dir>`.
pub fn build_id_near(path: &Path, game: &SteamGame) -> Option<String> {
    let path = fs::canonicalize(path).ok()?;
    let steamapps = path.parent()?.parent()?;
    read_manifest(&steamapps.join(format!("appmanifest_{}.acf", game.app_id)))?.build_id
}

// ---------------------------------------------------------------------------
// The user's settings.

/// The user's own mashup files (outside any checkout): Linux
/// `~/.local/share/mashup`, Windows `%APPDATA%\mashup` (the cfg folder,
/// `console::cfg_dir`, is its `cfg/`).
pub fn user_dir() -> Option<PathBuf> {
    let base = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("APPDATA").map(PathBuf::from))
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/share")))?;
    Some(base.join("mashup"))
}

/// The settings file: `$MASHUP_SETTINGS`, else `settings.toml` in
/// `user_dir`.
pub fn settings_path() -> Option<PathBuf> {
    std::env::var_os(SETTINGS_ENV).map(PathBuf::from).or_else(|| Some(user_dir()?.join("settings.toml")))
}

/// What the settings file holds: install paths by game, as
/// `mashup.local.toml` names them (never keys).
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
pub struct Settings {
    #[serde(default)]
    pub games: BTreeMap<String, SavedGame>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct SavedGame {
    pub path: PathBuf,
}

impl Settings {
    /// The file's settings; none if it's missing or unreadable.
    pub fn load(path: &Path) -> Self {
        fs::read_to_string(path).ok().and_then(|t| toml::from_str(&t).ok()).unwrap_or_default()
    }

    pub fn save(&self, path: &Path) -> Result<(), String> {
        let text = toml::to_string(self).map_err(|e| e.to_string())?;
        if let Some(dir) = path.parent() {
            fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        }
        let text = format!("# mashup's own settings (written by the game; see mashup.local.example.toml).\n{text}");
        fs::write(path, text).map_err(|e| format!("{}: {e}", path.display()))
    }
}

/// Remember (or with None forget) `game`'s install in the settings file.
pub fn save_install(settings: &Path, game: &str, path: Option<&Path>) -> Result<(), String> {
    let mut s = Settings::load(settings);
    match path {
        Some(p) => {
            s.games.insert(game.to_string(), SavedGame { path: p.to_path_buf() });
        }
        None => {
            s.games.remove(game);
        }
    }
    s.save(settings)
}

// ---------------------------------------------------------------------------
// The whole search.

/// Find `game`'s install: config, then the settings file, then Steam
/// (`steam`: the roots to search, or None to skip Steam).
pub fn resolve_with(config: &LocalConfig, settings: Option<&Path>, steam: Option<&[PathBuf]>, game: &str) -> Resolved {
    let sg = steam_game(game);
    let with_build =
        |path: PathBuf, found: Found| Install { build_id: sg.and_then(|g| build_id_near(&path, g)), path, found };
    let mut notes = Vec::new();
    if let Some(g) = config.games.get(game) {
        let path = expand_home(&g.path);
        if let Some(sg) = sg
            && let Err(e) = check(&path, sg)
        {
            notes.push(format!("config path doesn't check out: {e}"));
        }
        return Resolved { install: Some(with_build(path, Found::Config(config.file.clone()))), notes };
    }
    if let Some(file) = settings
        && let Some(saved) = Settings::load(file).games.get(game)
    {
        let path = expand_home(&saved.path);
        match sg.map_or(Ok(()), |sg| check(&path, sg)) {
            Ok(()) => {
                return Resolved { install: Some(with_build(path, Found::Saved(file.to_path_buf()))), notes };
            }
            Err(e) => notes.push(format!("saved path doesn't check out: {e}")),
        }
    }
    let (Some(sg), Some(roots)) = (sg, steam) else {
        notes.push(format!("{NO_PATH} for `{game}` in mashup.local.toml"));
        return Resolved { install: None, notes };
    };
    let mut found = find_in_steam(roots, sg);
    notes.append(&mut found.notes);
    found.notes = notes;
    found
}

const NO_PATH: &str = "no install path";

/// Steam searches made this run, by game (the folders don't move while
/// the game runs; `forget_steam` searches again).
static STEAM_FOUND: Mutex<BTreeMap<String, Resolved>> = Mutex::new(BTreeMap::new());

/// `resolve_with` from the environment: the settings file
/// (`settings_path`), the Steam roots (`steam_roots`, searched once per
/// run).
pub fn resolve(config: &LocalConfig, game: &str) -> Resolved {
    let settings = settings_path();
    let mut r = resolve_with(config, settings.as_deref(), None, game);
    let Some(sg) = steam_game(game).filter(|_| r.install.is_none()) else {
        return r;
    };
    let steam = STEAM_FOUND
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .entry(game.to_string())
        .or_insert_with(|| find_in_steam(&steam_roots(), sg))
        .clone();
    r.notes.retain(|n| !n.starts_with(NO_PATH));
    r.notes.extend(steam.notes);
    r.install = steam.install;
    r
}

/// Search Steam again on the next `resolve` (after the user installs or
/// moves the game).
pub fn forget_steam() {
    STEAM_FOUND.lock().unwrap_or_else(|e| e.into_inner()).clear();
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A scratch folder, removed when dropped.
    struct Scratch(PathBuf);

    impl Scratch {
        fn new(name: &str) -> Self {
            let p = std::env::temp_dir().join(format!("mashup-install-{name}-{}", std::process::id()));
            let _ = fs::remove_dir_all(&p);
            fs::create_dir_all(&p).unwrap();
            Self(p)
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn write(path: &Path, text: &str) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, text).unwrap();
    }

    fn css() -> &'static SteamGame {
        steam_game("cs_source").unwrap()
    }

    /// A CS:S install in `library` (its files as empty stand-ins) with
    /// its manifest.
    fn install_css(library: &Path, flags: u32) -> PathBuf {
        let dir = library.join("steamapps/common/Counter-Strike Source");
        for f in css().required {
            write(&dir.join(f), "");
        }
        write(
            &library.join("steamapps/appmanifest_240.acf"),
            &format!(
                "\"AppState\"\n{{\n\t\"appid\"\t\t\"240\"\n\t\"StateFlags\"\t\t\"{flags}\"\n\t\"installdir\"\t\t\"Counter-Strike Source\"\n\t\"buildid\"\t\t\"12345\"\n}}\n"
            ),
        );
        dir
    }

    fn vdf_path(p: &Path) -> String {
        p.display().to_string().replace('\\', "\\\\")
    }

    #[test]
    fn finds_the_app_in_the_second_library() {
        let s = Scratch::new("two-libs");
        let root = s.0.join("Steam");
        let second = s.0.join("Games/SteamLibrary");
        fs::create_dir_all(root.join("steamapps")).unwrap();
        let dir = install_css(&second, 4);
        write(
            &root.join("steamapps/libraryfolders.vdf"),
            &format!(
                "\"libraryfolders\"\n{{\n\t\"0\"\n\t{{\n\t\t\"path\"\t\t\"{}\"\n\t\t\"apps\"\n\t\t{{\n\t\t\t\"228980\"\t\t\"1\"\n\t\t}}\n\t}}\n\t\"1\"\n\t{{\n\t\t\"path\"\t\t\"{}\"\n\t\t\"apps\"\n\t\t{{\n\t\t\t\"240\"\t\t\"4775539566\"\n\t\t}}\n\t}}\n}}\n",
                vdf_path(&root),
                vdf_path(&second)
            ),
        );
        let r = find_in_steam(&[root.clone()], css());
        let i = r.install.clone().expect("found");
        assert_eq!(i.path, dir);
        assert_eq!(i.build_id.as_deref(), Some("12345"));
        assert!(matches!(i.found, Found::Steam { library: 1, .. }), "{:?}", i.found);
        assert!(r.notes.is_empty(), "{:?}", r.notes);
        assert!(r.describe("CS:S").contains("Steam library 1"), "{}", r.describe("CS:S"));
    }

    #[test]
    fn old_format_library_folders() {
        let s = Scratch::new("old-format");
        let root = s.0.join("Steam");
        let second = s.0.join("lib2");
        fs::create_dir_all(root.join("steamapps")).unwrap();
        let dir = install_css(&second, 4);
        write(
            &root.join("steamapps/libraryfolders.vdf"),
            &format!(
                "\"LibraryFolders\"\n{{\n\t\"TimeNextStatsReport\"\t\t\"1600000000\"\n\t\"ContentStatsID\"\t\t\"-123\"\n\t\"1\"\t\t\"{}\"\n}}\n",
                vdf_path(&second)
            ),
        );
        let libs = libraries(&root);
        assert_eq!(libs.len(), 2);
        assert_eq!((libs[0].index, &libs[0].path), (0, &root));
        assert_eq!((libs[1].index, &libs[1].path), (1, &second));
        let i = find_in_steam(&[root], css()).install.expect("found");
        assert_eq!(i.path, dir);
        assert!(matches!(i.found, Found::Steam { library: 1, .. }));
    }

    #[test]
    fn windows_paths_with_escaped_backslashes() {
        let s = Scratch::new("windows-paths");
        write(
            &s.0.join("steamapps/libraryfolders.vdf"),
            "\"libraryfolders\"\n{\n\t\"0\"\n\t{\n\t\t\"path\"\t\t\"C:\\\\Program Files (x86)\\\\Steam\"\n\t}\n\t\"1\"\n\t{\n\t\t\"path\"\t\t\"D:\\\\SteamLibrary\"\n\t\t\"apps\"\n\t\t{\n\t\t\t\"240\"\t\t\"1\"\n\t\t}\n\t}\n}\n",
        );
        let libs = libraries(&s.0);
        let paths: Vec<String> = libs.iter().map(|l| l.path.display().to_string()).collect();
        assert!(paths.contains(&r"C:\Program Files (x86)\Steam".to_string()), "{paths:?}");
        assert!(paths.contains(&r"D:\SteamLibrary".to_string()), "{paths:?}");
        assert_eq!(libs.iter().find(|l| l.index == 1).unwrap().apps, vec![240]);
    }

    #[test]
    fn missing_manifest_is_not_found_and_said() {
        let s = Scratch::new("no-manifest");
        write(
            &s.0.join("steamapps/libraryfolders.vdf"),
            &format!(
                "\"libraryfolders\" {{ \"0\" {{ \"path\" \"{}\" \"apps\" {{ \"240\" \"1\" }} }} }}",
                vdf_path(&s.0)
            ),
        );
        let r = find_in_steam(&[s.0.clone()], css());
        assert!(r.install.is_none());
        assert!(r.notes.iter().any(|n| n.contains("no appmanifest_240.acf")), "{:?}", r.notes);
    }

    #[test]
    fn partially_installed_is_skipped() {
        let s = Scratch::new("partial");
        install_css(&s.0, 1026);
        let r = find_in_steam(&[s.0.clone()], css());
        assert!(r.install.is_none());
        assert!(r.notes.iter().any(|n| n.contains("isn't fully installed")), "{:?}", r.notes);
        // An update in progress on an installed game keeps the bit.
        install_css(&s.0, 4 | 2);
        assert!(find_in_steam(&[s.0.clone()], css()).install.is_some());
    }

    #[test]
    fn a_folder_without_the_games_files_is_rejected() {
        let s = Scratch::new("no-files");
        install_css(&s.0, 4);
        fs::remove_file(s.0.join("steamapps/common/Counter-Strike Source/hl2/hl2_misc_dir.vpk")).unwrap();
        let r = find_in_steam(&[s.0.clone()], css());
        assert!(r.install.is_none());
        assert!(r.notes.iter().any(|n| n.contains("hl2/hl2_misc_dir.vpk")), "{:?}", r.notes);
        assert!(find_in_steam(&[], css()).notes.iter().any(|n| n.contains("no Steam folder")));
    }

    #[test]
    fn config_then_saved_then_steam() {
        let s = Scratch::new("precedence");
        let steam = s.0.join("steam");
        let steam_dir = install_css(&steam, 4);
        let saved_dir = install_css(&s.0.join("saved"), 4);
        let settings = s.0.join("user/settings.toml");
        let roots = [steam.clone()];

        // Nothing configured or saved: Steam.
        let none = LocalConfig::default();
        let r = resolve_with(&none, Some(&settings), Some(&roots), "cs_source");
        assert_eq!(r.install.as_ref().unwrap().path, steam_dir);

        // Saved beats Steam.
        save_install(&settings, "cs_source", Some(&saved_dir)).unwrap();
        let r = resolve_with(&none, Some(&settings), Some(&roots), "cs_source");
        let i = r.install.unwrap();
        assert_eq!(i.path, saved_dir);
        assert_eq!(i.found, Found::Saved(settings.clone()));
        assert_eq!(i.build_id.as_deref(), Some("12345"), "the saved install's manifest");

        // The config beats both, even a path that doesn't check out.
        let config: LocalConfig = toml::from_str("[games.cs_source]\npath = \"/nowhere/css\"").unwrap();
        let r = resolve_with(&config, Some(&settings), Some(&roots), "cs_source");
        assert_eq!(r.install.unwrap().path, PathBuf::from("/nowhere/css"));
        assert!(r.notes.iter().any(|n| n.contains("config path")));

        // A saved path that no longer checks out falls through to Steam.
        save_install(&settings, "cs_source", Some(&s.0.join("gone"))).unwrap();
        let r = resolve_with(&none, Some(&settings), Some(&roots), "cs_source");
        assert_eq!(r.install.unwrap().path, steam_dir);
        assert!(r.notes.iter().any(|n| n.contains("saved path")));

        // Forgetting keeps other games' entries.
        save_install(&settings, "other", Some(Path::new("/x"))).unwrap();
        save_install(&settings, "cs_source", None).unwrap();
        let kept = Settings::load(&settings);
        assert!(kept.games.contains_key("other") && !kept.games.contains_key("cs_source"));

        // A game Steam doesn't have here: config only.
        let r = resolve_with(&none, Some(&settings), Some(&roots), "cs_source_server");
        assert!(r.install.is_none());
    }
}
