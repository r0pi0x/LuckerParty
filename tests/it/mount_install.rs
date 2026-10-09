//! Finding the CS:S install without `mashup.local.toml`: a fake Steam
//! folder (its library list, the app's manifest, a stand-in install)
//! is found and mounted (`mount::install`, `games::cs_source::mount`).

use std::{
    fs,
    path::{Path, PathBuf},
};

use mashup::{
    games::cs_source,
    mount::{
        config::LocalConfig,
        install::{self, Found},
    },
};

struct Scratch(PathBuf);

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn write(path: &Path, bytes: &[u8]) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, bytes).unwrap();
}

/// An empty VPK directory file (version 1, an empty tree).
fn empty_vpk() -> Vec<u8> {
    let mut v = Vec::new();
    v.extend(0x55aa_1234u32.to_le_bytes());
    v.extend(1u32.to_le_bytes());
    v.extend(1u32.to_le_bytes());
    v.push(0);
    v
}

#[test]
fn an_install_in_a_steam_library_is_found_and_mounted() {
    let s = Scratch(std::env::temp_dir().join(format!("mashup-it-install-{}", std::process::id())));
    let _ = fs::remove_dir_all(&s.0);
    let root = s.0.join("Steam");
    let library = s.0.join("SteamLibrary");
    let css = library.join("steamapps/common/Counter-Strike Source");
    write(&css.join("cstrike/cstrike_pak_dir.vpk"), &empty_vpk());
    write(&css.join("hl2/hl2_misc_dir.vpk"), &empty_vpk());
    write(&css.join("cstrike/cfg/found.txt"), b"from the fake install");
    write(
        &library.join("steamapps/appmanifest_240.acf"),
        b"\"AppState\" { \"appid\" \"240\" \"StateFlags\" \"4\" \"installdir\" \"Counter-Strike Source\" \"buildid\" \"777\" }",
    );
    let vdf = |p: &Path| p.display().to_string().replace('\\', "\\\\");
    write(
        &root.join("steamapps/libraryfolders.vdf"),
        format!(
            "\"libraryfolders\" {{ \"0\" {{ \"path\" \"{}\" }} \"1\" {{ \"path\" \"{}\" \"apps\" {{ \"240\" \"1\" }} }} }}",
            vdf(&root),
            vdf(&library)
        )
        .as_bytes(),
    );

    // No config, nothing saved: Steam's library 1.
    let settings = s.0.join("user/settings.toml");
    let r = install::resolve_with(&LocalConfig::default(), Some(&settings), Some(&[root.clone()]), cs_source::GAME);
    let found = r.install.expect("found through the Steam folder");
    assert_eq!(found.path, css);
    assert_eq!(found.build_id.as_deref(), Some("777"));
    assert!(matches!(found.found, Found::Steam { library: 1, .. }), "{:?}", found.found);

    // And it's what the game mounts.
    let mount = cs_source::mount::open(&found.path).expect("mounts");
    assert_eq!(mount.read("cfg/found.txt").unwrap(), b"from the fake install");

    // Saved by the first-run dialog: used next time, Steam or not.
    install::save_install(&settings, cs_source::GAME, Some(&css)).unwrap();
    let r = install::resolve_with(&LocalConfig::default(), Some(&settings), Some(&[]), cs_source::GAME);
    assert_eq!(r.install.unwrap().found, Found::Saved(settings.clone()));
}

#[test]
fn settings_live_beside_the_cfg_folder() {
    assert_eq!(
        mashup::console::cfg_dir(),
        install::user_dir().map(|d| d.join("cfg")),
        "console::cfg_dir and mount::install::user_dir share their base"
    );
}
