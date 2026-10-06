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
    // Maps and content fetched from servers (the game's own download
    // folder, read-only) and mashup's cache, last: they never replace the
    // game's own files.
    mount.push(LooseDir::new(cstrike.join("download")).hiding(&["vpk"]));
    if let Some(cache) = crate::mount::config::content_dir(super::GAME) {
        mount.push(LooseDir::new(cache).hiding(&["vpk"]));
    }
    Ok(mount)
}

/// Copy a map (`.bsp`, or `.bsp.bz2` as servers send them) into mashup's
/// content cache as `maps/<name>.bsp`, and note it in `index.toml` (size,
/// FNV-1a 64 hash, where it came from). Returns the map's name.
pub fn import_map(file: &Path) -> Result<String, String> {
    let cache = crate::mount::config::content_dir(super::GAME).ok_or("no per-user data folder")?;
    import_map_into(file, &cache)
}

/// `import_map` into the content folder `cache`.
pub fn import_map_into(file: &Path, cache: &Path) -> Result<String, String> {
    let bytes = std::fs::read(file).map_err(|e| format!("{}: {e}", file.display()))?;
    let file_name = file
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or_default()
        .to_lowercase();
    let (bsp, name) = if let Some(stem) = file_name.strip_suffix(".bsp.bz2") {
        let mut out = Vec::new();
        use std::io::Read;
        bzip2::read::BzDecoder::new(&bytes[..])
            .read_to_end(&mut out)
            .map_err(|e| format!("{}: not a bzip2 file ({e})", file.display()))?;
        (out, stem.to_string())
    } else if let Some(stem) = file_name.strip_suffix(".bsp") {
        (bytes, stem.to_string())
    } else {
        return Err(format!("{}: expected a .bsp or .bsp.bz2 file", file.display()));
    };
    if bsp.get(0..4) != Some(b"VBSP") {
        return Err(format!("{}: not a Source map (no VBSP header)", file.display()));
    }
    if name.is_empty() || name.contains(['/', '\\']) {
        return Err(format!("{}: bad map name", file.display()));
    }
    let maps = cache.join("maps");
    std::fs::create_dir_all(&maps).map_err(|e| format!("{}: {e}", maps.display()))?;
    let target = maps.join(format!("{name}.bsp"));
    std::fs::write(&target, &bsp).map_err(|e| format!("{}: {e}", target.display()))?;
    let hash = bsp.iter().fold(0xcbf2_9ce4_8422_2325u64, |h, b| {
        (h ^ *b as u64).wrapping_mul(0x0100_0000_01b3)
    });
    let entry = format!(
        "[[map]]\nname = {name:?}\nsize = {}\nfnv64 = \"{hash:016x}\"\nfrom = {:?}\n\n",
        bsp.len(),
        file.display().to_string()
    );
    use std::io::Write;
    std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(cache.join("index.toml"))
        .and_then(|mut f| f.write_all(entry.as_bytes()))
        .map_err(|e| format!("index.toml: {e}"))?;
    Ok(name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn imports_plain_and_bzip2_maps() {
        let dir = std::env::temp_dir().join(format!("mashup-import-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let bsp = [b"VBSP".as_slice(), &[20, 0, 0, 0], &[7u8; 64]].concat();
        let plain = dir.join("mg_Test.bsp");
        std::fs::write(&plain, &bsp).unwrap();
        let mut packed = Vec::new();
        {
            use std::io::Write;
            let mut enc = bzip2::write::BzEncoder::new(&mut packed, bzip2::Compression::default());
            enc.write_all(&bsp).unwrap();
            enc.finish().unwrap();
        }
        let bz = dir.join("deathrun_x.bsp.bz2");
        std::fs::write(&bz, &packed).unwrap();
        let cache = dir.join("cache");
        assert_eq!(import_map_into(&plain, &cache).unwrap(), "mg_test");
        assert_eq!(import_map_into(&bz, &cache).unwrap(), "deathrun_x");
        assert_eq!(std::fs::read(cache.join("maps/deathrun_x.bsp")).unwrap(), bsp);
        let index = std::fs::read_to_string(cache.join("index.toml")).unwrap();
        assert_eq!(index.matches("[[map]]").count(), 2);
        // Not a map.
        let junk = dir.join("junk.bsp");
        std::fs::write(&junk, b"nope").unwrap();
        assert!(import_map_into(&junk, &cache).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
