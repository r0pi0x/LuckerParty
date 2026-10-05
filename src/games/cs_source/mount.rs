//! The CS:S search path: what the game itself would see.

use std::{io, path::Path};

use super::vpk::Vpk;
use crate::mount::{LooseDir, Mount};

/// Mount a CS:S install (the folder containing `cstrike/` and `hl2/`).
/// Order follows the game's search path: CS:S content first, then the shared
/// HL2 content it depends on; loose files before archives at each level.
pub fn open(install: &Path) -> io::Result<Mount> {
    let cstrike = install.join("cstrike");
    let hl2 = install.join("hl2");
    if !cstrike.is_dir() {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            format!("{}: no cstrike/ folder; is this a CS:S install?", install.display()),
        ));
    }
    let mut mount = Mount::default();
    // VPK files are mounted as their own layers, not as loose files.
    mount.push(LooseDir::new(cstrike.join("custom")).hiding(&["vpk"]));
    mount.push(LooseDir::new(&cstrike).hiding(&["vpk"]));
    mount.push(Vpk::open(cstrike.join("cstrike_pak_dir.vpk"))?);
    for name in ["hl2_textures", "hl2_sound_vo_english", "hl2_sound_misc", "hl2_misc"] {
        let dir = hl2.join(format!("{name}_dir.vpk"));
        if dir.is_file() {
            mount.push(Vpk::open(dir)?);
        }
    }
    mount.push(LooseDir::new(&hl2).hiding(&["vpk"]));
    Ok(mount)
}
