//! The 2D skybox: six textures named `skybox/<skyname><suffix>` from the
//! world's `skyname` key, as cube faces.

use vbsp::Bsp;

use super::material::MaterialLoader;
use crate::map::{MapData, MapSky};

/// Source face suffix -> engine cube face (+X, -X, +Y, -Y, +Z, -Z) and the
/// orientation (0-3: clockwise quarter turns; 4-7: the same after a
/// horizontal mirror) that places the image on that face. Fitted against the
/// real game with `refcmp skyfit`.
pub const FACES: [(&str, usize, u8); 6] = [
    // Fitted on de_dust2 (sky_dust). +X, -Z and up: per-face pixel fit
    // against the real game over sky pixels (refcmp skyfit), clear winners
    // of 48 candidates. -X and +Z: chosen by continuity across cube edges
    // with those three (the pixel fit was ambiguous between ft and lf).
    ("rt", 0, 0),
    ("ft", 1, 0),
    ("up", 2, 3),
    // Not visible in the fitted views; assumed to match "up".
    ("dn", 3, 3),
    ("lf", 4, 0),
    ("bk", 5, 0),
];

/// Face suffixes in the order `refcmp skyfit` numbers its candidates.
pub const SUFFIXES: [&str; 6] = ["rt", "lf", "ft", "bk", "up", "dn"];

pub fn add_sky(bsp: &Bsp, materials: &mut MaterialLoader, data: &mut MapData) {
    let Some(name) = bsp
        .entities
        .iter()
        .find(|e| e.prop("classname") == Some("worldspawn"))
        .and_then(|e| e.prop("skyname"))
    else {
        return;
    };
    // Calibration hook for `refcmp skyfit`: candidate N puts suffix N / 8
    // with orientation N % 8 on every face.
    let candidate: Option<usize> = std::env::var("MASHUP_SKY_CANDIDATE").ok().and_then(|v| v.parse().ok());
    let mapping: Vec<(&str, usize, u8)> = match candidate {
        Some(n) => (0..6)
            .map(|face| (SUFFIXES[(n / 8) % 6], face, (n % 8) as u8))
            .collect(),
        None => FACES.to_vec(),
    };
    let mut faces = [(usize::MAX, 0u8); 6];
    for (suffix, face, turns) in mapping {
        let r = materials.resolve(&format!("skybox/{name}{suffix}"));
        let Some(tex) = r.texture else {
            data.warnings.push(format!("sky {name}{suffix}: no texture"));
            return;
        };
        faces[face] = (tex, turns);
    }
    data.sky = Some(MapSky { faces });
}
