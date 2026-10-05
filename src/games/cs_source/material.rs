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
    /// `$decalscale` (map units per texel) when the material is a decal.
    pub decal_scale: Option<f32>,
    /// `$bumpmap`: tangent-space normal map (linear texture).
    pub normal_map: Option<usize>,
    /// WorldVertexTransition: `$basetexture2`, `$bumpmap2`,
    /// `$blendmodulatetexture`.
    pub blend: Option<crate::map::MapBlend>,
    /// `$detail` with its scale, blend factor and mode (0 and 1 supported).
    pub detail: Option<crate::map::MapDetail>,
    /// UnlitGeneric: drawn at the texture's own brightness, unlit.
    pub unlit: bool,
    /// `$surfaceprop`: footstep and impact sounds, physics.
    pub surfaceprop: Option<String>,
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

    pub fn read(&self, path: &str) -> Option<Vec<u8>> {
        let path = normalize(path);
        if let Ok(Some(data)) = self.bsp.pack.get(&path) {
            return Some(data);
        }
        self.mount.read(&path).ok()
    }

    fn read_text(&self, path: &str) -> Option<String> {
        self.read(path).map(|b| String::from_utf8_lossy(&b).into_owned())
    }

    /// Resolve the first of `candidates` that exists (models list several
    /// material folders to search).
    pub fn resolve_any(&mut self, candidates: &[String]) -> Resolved {
        let found = candidates
            .iter()
            .find(|c| self.read(&format!("materials/{}.vmt", normalize(c))).is_some());
        match found {
            Some(name) => self.resolve(&name.clone()),
            None => {
                self.missing
                    .push(format!("material not found in any of {candidates:?}"));
                Resolved {
                    texture: None,
                    alpha: MapAlpha::Opaque,
                    double_sided: false,
                    decal_scale: None,
                    normal_map: None,
                    blend: None,
                    detail: None,
                    unlit: false,
                    surfaceprop: None,
                }
            }
        }
    }

    /// Resolve material `name` (as named in the BSP, without `materials/`).
    pub fn resolve(&mut self, name: &str) -> Resolved {
        let fallback = Resolved {
            texture: None,
            alpha: MapAlpha::Opaque,
            double_sided: false,
            decal_scale: None,
            normal_map: None,
            blend: None,
            detail: None,
            unlit: false,
            surfaceprop: None,
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
        } else if material.alpha_test().is_some() {
            // The parser defaults an absent $alphatestreference to 1.0, which
            // would keep only fully opaque texels; unset means 0.5 here
            // (specs/cs_source/shaders.md, open question 4).
            let reference = material_key::<f32>(&text, "$alphatestreference").filter(|r| *r > 0.0);
            MapAlpha::Mask(reference.unwrap_or(0.5))
        } else {
            MapAlpha::Opaque
        };
        // The parser's accessors skip some shaders' own fields (Cable).
        let (base, bump) = match &material {
            vmt_parser::material::Material::Cable(m) => (Some(m.base_texture.as_str()), m.bump_map.as_deref()),
            m => (m.base_texture(), m.bump_map()),
        };
        let texture = base.and_then(|t| self.texture(t, true));
        let normal_map = bump.and_then(|t| self.texture(t, false));
        let decal_scale = match &material {
            vmt_parser::material::Material::LightMappedGeneric(m) if m.decal => Some(m.decal_scale),
            _ => None,
        };
        let blend = match &material {
            vmt_parser::material::Material::WorldVertexTransition(m) => Some(crate::map::MapBlend {
                texture: self.texture(&m.base_texture2, true),
                normal_map: m.bump_map2.as_deref().and_then(|t| self.texture(t, false)),
                mask: m.blend_modulate_texture.as_deref().and_then(|t| self.texture(t, false)),
            }),
            _ => None,
        };
        // The parser defaults a missing $detailblendmode to 1; the game's
        // default is 0 (mod2x), so read the mode from the text.
        let detail_mode = detail_blend_mode(&text);
        let detail_source = match &material {
            vmt_parser::material::Material::LightMappedGeneric(m) => m
                .detail
                .as_deref()
                .map(|d| (d.to_string(), m.detail_scale.0, m.detail_blend_factor)),
            vmt_parser::material::Material::WorldVertexTransition(m) => m
                .detail
                .as_deref()
                .map(|d| (d.to_string(), m.detail_scale.0, m.detail_blend_factor)),
            _ => None,
        };
        let detail = detail_source
            .filter(|_| detail_mode <= 1)
            .and_then(|(name, scale, factor)| {
                // Mod2x uses the texel as stored; additive decodes sRGB.
                let texture = self.texture(&name, detail_mode == 1)?;
                Some(crate::map::MapDetail {
                    texture,
                    scale,
                    factor,
                    mode: detail_mode as u8,
                })
            });
        Resolved {
            texture,
            alpha,
            double_sided: material.no_cull(),
            decal_scale,
            normal_map,
            blend,
            detail,
            unlit: matches!(material, vmt_parser::material::Material::UnlitGeneric(_)),
            surfaceprop: material.surface_prop().map(str::to_lowercase),
        }
    }

    fn texture(&mut self, name: &str, srgb: bool) -> Option<usize> {
        let path = format!("materials/{}.vtf", normalize(name).trim_end_matches(".vtf"));
        let key = if srgb { path.clone() } else { format!("{path}#linear") };
        if let Some(cached) = self.by_path.get(&key) {
            return *cached;
        }
        let loaded = self.decode(&path).map(|t| MapTexture { srgb, ..t });
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
        self.by_path.insert(key, index);
        index
    }

    fn decode(&self, path: &str) -> Result<MapTexture, String> {
        let bytes = self.read(path).ok_or("not found")?;
        let vtf = vtf::from_bytes(&bytes).map_err(|e| e.to_string())?;
        let image = vtf.highres_image.decode(0).map_err(|e| e.to_string())?.to_rgba8();
        let mips = mip_levels(&bytes, &vtf.header).unwrap_or_default();
        Ok(MapTexture {
            name: path.to_string(),
            srgb: true,
            mips,
            width: image.width(),
            height: image.height(),
            rgba8: image.into_raw(),
        })
    }
}

/// The VTF's own smaller mip levels (1..n), decoded to RGBA8. VTF stores
/// levels smallest first, before the full-size image; this mirrors how the
/// `vtf` crate locates the full-size data, then walks back.
fn mip_levels(bytes: &[u8], header: &vtf::header::VTFHeader) -> Option<Vec<Vec<u8>>> {
    use vtf::{image::VTFImage, resources::ResourceType};

    if header.mipmap_count <= 1 || header.frames > 1 || header.depth > 1 {
        return None;
    }
    let format = header.highres_image_format;
    let lowres_offset = match header
        .resources
        .get_by_type(ResourceType::VTF_LEGACY_RSRC_LOW_RES_IMAGE)
    {
        Some(r) => r.data,
        None => header.header_size,
    };
    let data_start = match header.resources.get_by_type(ResourceType::VTF_LEGACY_RSRC_IMAGE) {
        Some(r) => r.data,
        None => {
            lowres_offset
                + header
                    .lowres_image_format
                    .frame_size(header.lowres_image_width as u32, header.lowres_image_height as u32)
                    .ok()?
        }
    } as usize;
    let size = |m: u32| -> Option<(u32, u32, usize)> {
        let (w, h) = ((header.width as u32 >> m).max(1), (header.height as u32 >> m).max(1));
        Some((w, h, format.frame_size(w, h).ok()? as usize))
    };
    let count = header.mipmap_count as u32;
    let mut out = Vec::new();
    for m in 1..count {
        // Offset of level m: all smaller levels come first.
        let mut offset = data_start;
        for smaller in (m + 1)..count {
            offset += size(smaller)?.2;
        }
        let (w, h, _) = size(m)?;
        let mut single = header.clone();
        single.mipmap_count = 1;
        let img = VTFImage::new(single, format, w as u16, h as u16, bytes, offset);
        out.push(img.decode(0).ok()?.to_rgba8().into_raw());
    }
    Some(out)
}

/// `$detailblendmode` from a material's text (0 when absent).
fn detail_blend_mode(text: &str) -> u32 {
    material_key(text, "$detailblendmode").unwrap_or(0)
}

/// A material key's value read from its text, when the parser's defaults
/// can't tell "absent" from a given value.
fn material_key<T: std::str::FromStr>(text: &str, key: &str) -> Option<T> {
    text.lines().find_map(|line| {
        let line = line.trim().to_ascii_lowercase();
        let rest = line
            .strip_prefix(&format!("\"{key}\""))
            .or_else(|| line.strip_prefix(key))?;
        rest.trim()
            .trim_matches('"')
            .split_whitespace()
            .next()?
            .trim_matches('"')
            .parse()
            .ok()
    })
}
