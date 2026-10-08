//! Baked per-vertex static prop lighting (`.vhv`, "vertex header"): what
//! the map compiler writes into a map's pakfile for each static prop it lit
//! per vertex (`-StaticPropLighting`), as `sp_<n>.vhv` (LDR) and
//! `sp_hdr_<n>.vhv` (HDR), `<n>` the prop's index in the static prop lump.
//!
//! Layout (little endian, read from the files themselves; matches the
//! public descriptions of the format): a header of ten 32-bit words
//! (version 2, the model's checksum, vertex flags, bytes per vertex,
//! vertex count, mesh count, four unused), then one 28-byte record per
//! mesh (LOD, vertex count, byte offset of its vertices, four unused).
//! Every file in the cached community maps carries 4-byte colours (flags
//! 4). The meshes come body part by body part, model by model, LOD by
//! LOD; each holds one colour per vertex of that LOD's `.vtx` strip
//! groups, in their order (de_nuke's fuel cask: 1179, 908, 714, 508 and
//! 370 colours for its five LODs, its `.vtx` vertex counts). Bytes are
//! B, G, R, A (Direct3D's colour layout; read as R, G, B a bluish
//! surf_nebula came out reddish against its light probes).

/// A prop's baked colours: one per `.vtx` vertex of the model's LOD 0
/// meshes, in their order, as R, G, B (0..255, alpha dropped).
#[derive(Clone, Debug, PartialEq)]
pub struct VertexColors {
    /// The model checksum the file was baked for.
    pub checksum: u32,
    pub colors: Vec<[u8; 3]>,
}

const HEADER: usize = 40;
const MESH: usize = 28;
const VERSION: i32 = 2;

/// Parse a `.vhv` file.
pub fn parse(bytes: &[u8]) -> Result<VertexColors, String> {
    let word = |at: usize| -> Result<u32, String> {
        bytes
            .get(at..at + 4)
            .map(|b| u32::from_le_bytes(b.try_into().unwrap()))
            .ok_or_else(|| format!("truncated at {at}"))
    };
    let version = word(0)? as i32;
    if version != VERSION {
        return Err(format!("version {version}"));
    }
    let checksum = word(4)?;
    let size = word(12)? as usize;
    let meshes = word(20)? as usize;
    if size < 3 {
        return Err(format!("{size} bytes per vertex"));
    }
    let mut colors = Vec::new();
    for m in 0..meshes.min(4096) {
        let at = HEADER + m * MESH;
        let lod = word(at)?;
        if lod != 0 {
            continue;
        }
        let count = word(at + 4)? as usize;
        let offset = word(at + 8)? as usize;
        let data = bytes
            .get(offset..offset + count * size)
            .ok_or_else(|| format!("mesh {m}: {count} vertices past the end"))?;
        colors.extend(data.chunks_exact(size).map(|v| [v[2], v[1], v[0]]));
    }
    Ok(VertexColors { checksum, colors })
}

/// A colour's light, linear, in lightmap units (1.0 shows the texture
/// as it is): the overbright-2 gamma encoding lightmaps use
/// (specs/cs_source/shaders.md 4, "Static per-vertex lighting":
/// light = (2v)^2.2).
pub fn decode(c: [u8; 3]) -> [f32; 3] {
    c.map(|v| (2.0 * v as f32 / 255.0).powf(2.2))
}

/// The pakfile names of static prop `index`'s colours: the HDR file first
/// when `hdr`, then the other one (maps compiled for one mode only ship
/// one set: surf_holiday has only `sp_hdr_*`).
pub fn names(index: usize, hdr: bool) -> [String; 2] {
    let (ldr, hdr_name) = (format!("sp_{index}.vhv"), format!("sp_hdr_{index}.vhv"));
    if hdr { [hdr_name, ldr] } else { [ldr, hdr_name] }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file(checksum: u32, meshes: &[(u32, &[[u8; 4]])]) -> Vec<u8> {
        let mut b = Vec::new();
        let total: usize = meshes.iter().map(|m| m.1.len()).sum();
        for w in [2u32, checksum, 4, 4, total as u32, meshes.len() as u32, 0, 0, 0, 0] {
            b.extend_from_slice(&w.to_le_bytes());
        }
        let mut offset = 512u32;
        let mut data: Vec<u8> = Vec::new();
        for (lod, verts) in meshes {
            for w in [*lod, verts.len() as u32, offset, 0, 0, 0, 0] {
                b.extend_from_slice(&w.to_le_bytes());
            }
            offset += 4 * verts.len() as u32;
            data.extend(verts.iter().flatten());
        }
        b.resize(512, 0);
        b.extend(data);
        b
    }

    #[test]
    fn reads_lod0_colours_in_mesh_order() {
        let b = file(
            0xdead_beef,
            &[
                (0, &[[1, 2, 3, 255], [4, 5, 6, 255]]),
                (1, &[[9, 9, 9, 255]]),
                (0, &[[7, 8, 9, 255]]),
            ],
        );
        let v = parse(&b).unwrap();
        assert_eq!(v.checksum, 0xdead_beef);
        assert_eq!(v.colors, vec![[3, 2, 1], [6, 5, 4], [9, 8, 7]], "B, G, R in the file");
        assert!(parse(&b[..60]).is_err());
        let mut old = b.clone();
        old[0] = 1;
        assert!(parse(&old).is_err());
    }

    #[test]
    fn decodes_like_lightmaps() {
        // Byte 128 is about 1.0 (shaders.md's table: 1.0086).
        let l = decode([128, 0, 255]);
        assert!((l[0] - 1.0086).abs() < 1e-3, "{l:?}");
        assert_eq!(l[1], 0.0);
        assert!((l[2] - 2f32.powf(2.2)).abs() < 1e-4);
        assert_eq!(names(3, false), ["sp_3.vhv".to_string(), "sp_hdr_3.vhv".to_string()]);
        assert_eq!(names(3, true)[0], "sp_hdr_3.vhv");
    }
}
