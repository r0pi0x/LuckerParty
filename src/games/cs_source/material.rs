//! Source materials (VMT) and their base textures (VTF) to neutral textures.
//! Looks in the map's embedded pak first (maps ship patched materials there),
//! then the game's search path.

use std::collections::HashMap;

use vbsp::Bsp;

use crate::{
    map::{MapAlpha, MapTexture},
    mount::{Mount, normalize},
};

/// Parse failures and missing `include` targets of patch materials.
#[derive(Debug)]
enum VmtError {
    Parse(vmt_parser::VdfError),
    Missing(String),
}

impl From<vmt_parser::VdfError> for VmtError {
    fn from(e: vmt_parser::VdfError) -> Self {
        Self::Parse(e)
    }
}

impl std::fmt::Display for VmtError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Parse(e) => write!(f, "{e}"),
            Self::Missing(path) => write!(f, "included {path} not found"),
        }
    }
}

pub struct Resolved {
    pub texture: Option<usize>,
    pub alpha: MapAlpha,
    pub double_sided: bool,
}

pub struct MaterialLoader<'a> {
    bsp: &'a Bsp,
    mount: &'a Mount,
    pub textures: Vec<MapTexture>,
    by_path: HashMap<String, Option<usize>>,
    /// Materials or textures that couldn't be loaded, with the reason.
    pub missing: Vec<String>,
}

impl<'a> MaterialLoader<'a> {
    pub fn new(bsp: &'a Bsp, mount: &'a Mount) -> Self {
        Self {
            bsp,
            mount,
            textures: Vec::new(),
            by_path: HashMap::new(),
            missing: Vec::new(),
        }
    }

    fn read(&self, path: &str) -> Option<Vec<u8>> {
        let path = normalize(path);
        if let Ok(Some(data)) = self.bsp.pack.get(&path) {
            return Some(data);
        }
        self.mount.read(&path).ok()
    }

    fn read_text(&self, path: &str) -> Option<String> {
        self.read(path).map(|b| String::from_utf8_lossy(&b).into_owned())
    }

    /// Resolve material `name` (as named in the BSP, without `materials/`).
    pub fn resolve(&mut self, name: &str) -> Resolved {
        let fallback = Resolved {
            texture: None,
            alpha: MapAlpha::Opaque,
            double_sided: false,
        };
        let vmt_path = format!("materials/{}.vmt", normalize(name));
        let Some(text) = self.read_text(&vmt_path) else {
            self.missing.push(format!("{vmt_path}: not found"));
            return fallback;
        };
        let material = vmt_parser::from_str(&text).map_err(VmtError::from).and_then(|m| {
            m.resolve(|include: &str| self.read_text(include).ok_or(VmtError::Missing(include.to_string())))
        });
        let material = match material {
            Ok(m) => m,
            Err(e) => {
                self.missing.push(format!("{vmt_path}: {e}"));
                return fallback;
            }
        };
        let alpha = if material.translucent() {
            MapAlpha::Blend
        } else if let Some(cutoff) = material.alpha_test() {
            MapAlpha::Mask(if cutoff > 0.0 { cutoff } else { 0.5 })
        } else {
            MapAlpha::Opaque
        };
        let texture = material.base_texture().and_then(|t| self.texture(t));
        Resolved {
            texture,
            alpha,
            double_sided: material.no_cull(),
        }
    }

    fn texture(&mut self, name: &str) -> Option<usize> {
        let path = format!("materials/{}.vtf", normalize(name).trim_end_matches(".vtf"));
        if let Some(cached) = self.by_path.get(&path) {
            return *cached;
        }
        let loaded = self.decode(&path);
        let index = match loaded {
            Ok(t) => {
                self.textures.push(t);
                Some(self.textures.len() - 1)
            }
            Err(e) => {
                self.missing.push(format!("{path}: {e}"));
                None
            }
        };
        self.by_path.insert(path, index);
        index
    }

    fn decode(&self, path: &str) -> Result<MapTexture, String> {
        let bytes = self.read(path).ok_or("not found")?;
        let vtf = vtf::from_bytes(&bytes).map_err(|e| e.to_string())?;
        let image = vtf.highres_image.decode(0).map_err(|e| e.to_string())?.to_rgba8();
        Ok(MapTexture {
            name: path.to_string(),
            width: image.width(),
            height: image.height(),
            rgba8: image.into_raw(),
        })
    }
}
