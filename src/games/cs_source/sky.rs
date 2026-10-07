//! The 2D skybox: six textures named `skybox/<skyname><suffix>` from the
//! world's `skyname` key, as cube faces.

use std::sync::Arc;

use vbsp::Bsp;

use super::material::MaterialLoader;
use crate::map::{MapData, MapHdrImage, MapSky};

/// Source face suffix -> cubemap layer (+X, -X, +Y, -Y, +Z, -Z as stored)
/// and the orientation (0-3: clockwise quarter turns; 4-7: the same after a
/// horizontal mirror) that places the image on that layer.
///
/// Fitted with `refcmp skyconv` on de_dust2 (sky_dust): it first measures
/// how Bevy's skybox samples the cube (it flips z: looking toward -Z shows
/// the +Z layer), then fits each layer's texture to the real game's sky.
/// Errors per layer: rt 0.006, lf 0.037, up 0.048, bk 0.006, ft 0.030.
pub const FACES: [(&str, usize, u8); 6] = [
    ("rt", 0, 0),
    ("lf", 1, 0),
    ("up", 2, 3),
    // Not visible in the fitted views; assumed to match "up".
    ("dn", 3, 3),
    ("bk", 4, 0),
    ("ft", 5, 0),
];

/// The sky, and with `hdr` (mat_hdr_level 2) its HDR faces: the engine
/// prefers `skybox/<skyname>_hdr<suffix>` materials when HDR is on, and a
/// Sky material's `$hdrbasetexture` (or `$hdrcompressedtexture`) over its
/// `$basetexture` (specs/cs_source/shaders.md section 6). Without HDR faces
/// for all six sides the LDR sky shows.
pub fn add_sky(bsp: &Bsp, materials: &mut MaterialLoader, data: &mut MapData, hdr: bool) {
    let Some(name) = bsp
        .entities
        .iter()
        .find(|e| e.prop("classname") == Some("worldspawn"))
        .and_then(|e| e.prop("skyname"))
    else {
        return;
    };
    let mut faces = [(usize::MAX, 0u8); 6];
    for (suffix, face, turns) in FACES {
        let r = materials.resolve(&format!("skybox/{name}{suffix}"));
        let Some(tex) = r.texture else {
            data.warnings.push(format!("sky {name}{suffix}: no texture"));
            return;
        };
        faces[face] = (tex, turns);
    }
    let hdr_faces = if hdr { hdr_faces(materials, name, data) } else { None };
    data.sky = Some(MapSky { faces, hdr: hdr_faces });
}

fn hdr_faces(materials: &mut MaterialLoader, name: &str, data: &mut MapData) -> Option<Arc<[MapHdrImage; 6]>> {
    let mut out: [MapHdrImage; 6] = Default::default();
    for (suffix, face, _) in FACES {
        let candidates = [format!("skybox/{name}_hdr{suffix}"), format!("skybox/{name}{suffix}")];
        let material = candidates
            .iter()
            .find(|m| materials.read(&format!("materials/{m}.vmt")).is_some())?;
        match materials.hdr_sky_texture(material) {
            Ok(image) => out[face] = image,
            Err(e) => {
                data.warnings.push(format!("HDR sky {material}: {e}"));
                return None;
            }
        }
    }
    Some(Arc::new(out))
}
