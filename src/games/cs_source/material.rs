//! Source materials (VMT) and their base textures (VTF) to neutral textures.
//! Looks in the map's embedded pak first (maps ship patched materials there),
//! then the game's search path.

use std::collections::HashMap;

use bevy::math::Vec2;

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
    /// `$envmap` with a baked cubemap.
    pub envmap: Option<crate::map::MapEnvmap>,
    /// `$color` x `$color2` (linear multiplier; the `srgb?` variants, which
    /// the game uses as it renders in sRGB, win).
    pub tint: Option<[f32; 3]>,
}

pub struct MaterialLoader<'a> {
    bsp: &'a Bsp,
    mount: &'a Mount,
    pub textures: Vec<MapTexture>,
    pub cubemaps: Vec<crate::map::MapCubemap>,
    by_path: HashMap<String, Option<usize>>,
    cube_by_path: HashMap<String, Option<usize>>,
    /// Materials or textures that couldn't be loaded, with the reason.
    pub missing: Vec<String>,
}

impl<'a> MaterialLoader<'a> {
    pub fn new(bsp: &'a Bsp, mount: &'a Mount) -> Self {
        Self {
            bsp,
            mount,
            textures: Vec::new(),
            cubemaps: Vec::new(),
            by_path: HashMap::new(),
            cube_by_path: HashMap::new(),
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

    pub(crate) fn read_text(&self, path: &str) -> Option<String> {
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
                    envmap: None,
                    tint: None,
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
            envmap: None,
            tint: None,
        };
        let vmt_path = format!("materials/{}.vmt", normalize(name));
        let Some(text) = self.read_text(&vmt_path) else {
            self.missing.push(format!("{vmt_path}: not found"));
            return fallback;
        };
        let (text, stand_in) = stand_in_shader(&text);
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
        // `$additive` (light glows and beams): the parser keeps it only for
        // SpriteCard, so read it from the text (and patch materials' keys).
        let additive = self.keys(&text, 0).get("$additive").is_some_and(|v| v.trim() != "0");
        let alpha = if additive {
            MapAlpha::Add
        } else if material.translucent() {
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
        // The parser's accessors skip some shaders' own fields (Cable, and
        // Sky: the HDR-capable skies on de_train, de_nuke, de_dust and
        // cs_militia, whose LDR texture is `$basetexture`).
        let (base, bump) = match &material {
            vmt_parser::material::Material::Cable(m) => (Some(m.base_texture.as_str()), m.bump_map.as_deref()),
            vmt_parser::material::Material::Sky(m) => (Some(m.base_texture.as_str()), None),
            // Water's `$bumpmap` is a DuDv refraction map (unsupported
            // format, and not a normal map); `$normalmap` is the normal map.
            vmt_parser::material::Material::Water(m) => (m.base_texture.as_deref(), m.normal_map.as_deref()),
            m => (m.base_texture(), m.bump_map()),
        };
        let mut texture = base.and_then(|t| self.texture(t, true));
        // Water without a base texture (de_aztec's canals) shows what's
        // below through its fog in the game; without a Water shader, draw
        // its fog colour (plus `$envmap` reflections below) instead of a
        // debug colour.
        if let (None, vmt_parser::material::Material::Water(w)) = (texture, &material) {
            texture = Some(self.solid(w.fog_color.0));
        }
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
        // WorldTwoTextureBlend (specs/cs_source/shaders_two_texture_blend.md):
        // mode 3 lerps the detail over the base, mode 4 is the "2x grime
        // mask" (`$detail_alpha_mask_base_texture 1`, every stock aztec wall).
        let detail_mode = match stand_in {
            Some(StandIn::TwoTextureBlend) => {
                if material_key::<u32>(&text, "$detail_alpha_mask_base_texture") == Some(1) {
                    4
                } else {
                    3
                }
            }
            _ => detail_blend_mode(&text),
        };
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
            .filter(|_| detail_mode <= 4)
            .and_then(|(name, scale, factor)| {
                // Mod2x and WorldTwoTextureBlend use the texel as stored;
                // additive and translucent decode sRGB.
                let texture = self.texture(&name, matches!(detail_mode, 1 | 2))?;
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
            envmap: self.envmap(&text, normal_map.is_some()),
            tint: tint(&self.keys(&text, 0)),
        }
    }

    /// A particle or effect material (specs/cs_source/impact_effects.md):
    /// its base texture, blend (`$additive`), whether vertex colour and
    /// alpha apply (`$vertexcolor`, `$vertexalpha`; SpriteCard always
    /// takes both), `$ignorez`, and the texture's sprite sheet if it has one.
    pub fn particle(&mut self, name: &str) -> Option<crate::map::particles::ParticleMaterial> {
        use crate::map::particles::{ParticleBlend, ParticleMaterial};
        let text = self.read_text(&format!("materials/{}.vmt", normalize(name).trim_end_matches(".vmt")))?;
        let shader = super::surfaceprops::tokens(&text)
            .first()
            .map(|s| s.to_lowercase())
            .unwrap_or_default();
        let keys = self.keys(&text, 0);
        let flag = |k: &str| keys.get(k).is_some_and(|v| v.trim() != "0");
        let sprite_card = shader == "spritecard";
        let base = keys.get("$basetexture").cloned()?;
        let texture = self.texture(&base, true);
        let sequences = self
            .read(&format!("materials/{}.vtf", normalize(&base).trim_end_matches(".vtf")))
            .and_then(|b| sheet(&b))
            .unwrap_or_default();
        Some(ParticleMaterial {
            name: name.to_lowercase(),
            texture,
            blend: if flag("$additive") {
                ParticleBlend::Additive
            } else {
                ParticleBlend::Alpha
            },
            vertex_color: sprite_card || flag("$vertexcolor"),
            vertex_alpha: sprite_card || flag("$vertexalpha"),
            no_depth: flag("$ignorez"),
            sequences,
        })
    }

    /// A material's keys (lower-case), with "patch" materials merged over
    /// the material they include; nested blocks (proxies) are skipped.
    /// A decal material as runtime decals draw it: a `Subrect` of a decal
    /// atlas (its pixel rectangle), or a whole decal texture. World size is
    /// the rectangle's texels x `$decalscale` (specs/cs_source/
    /// overlays_decals.md, "Decal size").
    pub fn decal(&mut self, name: &str) -> Option<crate::map::decal::MapDecal> {
        use crate::map::decal::{DecalBlend, MapDecal};
        let vmt = |n: &str| format!("materials/{}.vmt", normalize(n).trim_end_matches(".vmt"));
        let shader = |text: &str| {
            super::surfaceprops::tokens(text)
                .first()
                .map(|s| s.to_lowercase())
                .unwrap_or_default()
        };
        let text = self.read_text(&vmt(name))?;
        let keys = self.keys(&text, 0);
        let float = |k: &HashMap<String, String>, key: &str| k.get(key).and_then(|v| v.trim().parse::<f32>().ok());
        let pair = |k: &HashMap<String, String>, key: &str| -> Option<Vec2> {
            let v: Vec<f32> = k.get(key)?.split_whitespace().filter_map(|x| x.parse().ok()).collect();
            (v.len() == 2).then(|| Vec2::new(v[0], v[1]))
        };
        let (atlas_keys, atlas_shader, rect) = if shader(&text) == "subrect" {
            let atlas = self.read_text(&vmt(keys.get("$material")?))?;
            let rect = (pair(&keys, "$pos")?, pair(&keys, "$size")?);
            (self.keys(&atlas, 0), shader(&atlas), Some(rect))
        } else {
            (keys.clone(), shader(&text), None)
        };
        let texture = self.texture(atlas_keys.get("$basetexture")?, true)?;
        let t = &self.textures[texture];
        let full = Vec2::new(t.width as f32, t.height as f32);
        let (pos, size) = rect.unwrap_or((Vec2::ZERO, full));
        let scale = float(&keys, "$decalscale")
            .or_else(|| float(&atlas_keys, "$decalscale"))
            .unwrap_or(1.0);
        Some(MapDecal {
            texture,
            uv_min: pos / full,
            uv_max: (pos + size) / full,
            size: size * scale * super::bsp::METERS_PER_UNIT,
            blend: if atlas_shader == "decalmodulate" {
                DecalBlend::Modulate2x
            } else {
                DecalBlend::Alpha
            },
        })
    }

    fn keys(&self, text: &str, depth: u32) -> HashMap<String, String> {
        let t = super::surfaceprops::tokens(text);
        let mut out = HashMap::new();
        if t.first().is_some_and(|s| s.eq_ignore_ascii_case("patch")) {
            let include = t
                .windows(2)
                .find(|w| w[0].eq_ignore_ascii_case("include"))
                .map(|w| w[1].clone());
            if let Some(inc) = include.filter(|_| depth < 8)
                && let Some(inner) = self.read_text(&inc)
            {
                out = self.keys(&inner, depth + 1);
            }
            // Keys inside "replace" / "insert" blocks override.
            let mut i = 0;
            while i < t.len() {
                if (t[i].eq_ignore_ascii_case("replace") || t[i].eq_ignore_ascii_case("insert"))
                    && t.get(i + 1).is_some_and(|s| s == "{")
                {
                    i += 2;
                    while i + 1 < t.len() && t[i] != "}" {
                        out.insert(t[i].to_lowercase(), t[i + 1].clone());
                        i += 2;
                    }
                }
                i += 1;
            }
            return out;
        }
        // shader { key value ... }: the first block's own keys, then the
        // DirectX 9 fallback block's (`<shader>_dx9`), which LDR CS:S uses
        // at DX level 90 and up; other nested blocks (proxies, HDR) are
        // skipped.
        let mut level = 0;
        let mut i = 0;
        let mut dx9 = HashMap::new();
        let mut in_dx9 = false;
        while i < t.len() {
            match t[i].as_str() {
                "{" => level += 1,
                "}" => {
                    level -= 1;
                    if level <= 1 {
                        in_dx9 = false;
                    }
                }
                key if level == 1 && t.get(i + 1).is_some_and(|v| v == "{") => {
                    let k = key.to_lowercase();
                    in_dx9 = k.ends_with("_dx9") && !k.contains("hdr");
                }
                key if t.get(i + 1).is_some_and(|v| v != "{" && v != "}") => {
                    if level == 1 {
                        out.insert(key.to_lowercase(), t[i + 1].clone());
                    } else if level == 2 && in_dx9 {
                        dx9.insert(key.to_lowercase(), t[i + 1].clone());
                    }
                    i += 1;
                }
                _ => {}
            }
            i += 1;
        }
        out.extend(dx9);
        out
    }

    /// `$envmap` and its modifiers, when it names a baked cubemap. The
    /// world shader's fast path (spec Quirks) is applied here.
    fn envmap(&mut self, text: &str, bumped: bool) -> Option<crate::map::MapEnvmap> {
        use crate::map::EnvmapMask;
        let keys = self.keys(text, 0);
        let name = keys.get("$envmap")?.clone();
        // Unpatched "env_cubemap": the engine picks the nearest of the
        // map's cubemaps per object at run time.
        let cubemap = if name.eq_ignore_ascii_case("env_cubemap") {
            None
        } else {
            Some(self.cubemap(&name)?)
        };
        let num = |k: &str, d: f32| keys.get(k).and_then(|v| vector(v)).map_or(d, |v| v[0]);
        let flag = |k: &str| keys.get(k).is_some_and(|v| v.trim() != "0");
        let mask = if flag("$normalmapalphaenvmapmask") && bumped {
            EnvmapMask::NormalAlpha
        } else if flag("$basealphaenvmapmask") {
            EnvmapMask::BaseAlphaInverted
        } else if let Some(m) = keys.get("$envmapmask").filter(|_| !bumped).cloned() {
            self.texture(&m, false).map_or(EnvmapMask::None, EnvmapMask::Texture)
        } else {
            EnvmapMask::None
        };
        let tint = keys.get("$envmaptint").and_then(|v| vector(v)).unwrap_or([1.0; 3]);
        let (mut contrast, mut saturation, mut fresnel) = (
            num("$envmapcontrast", 0.0),
            num("$envmapsaturation", 1.0),
            num("$fresnelreflection", 1.0),
        );
        let honoured = (contrast > 0.0 && contrast != 1.0 && saturation != 1.0) || fresnel != 1.0;
        if !honoured {
            contrast = if contrast == 1.0 { 1.0 } else { 0.0 };
            saturation = 1.0;
            fresnel = 1.0;
        }
        Some(crate::map::MapEnvmap {
            cubemap,
            mask,
            tint,
            contrast,
            saturation,
            fresnel,
        })
    }

    /// A cubemap texture's six faces (largest mip; sRGB).
    pub fn cubemap(&mut self, name: &str) -> Option<usize> {
        let path = format!("materials/{}.vtf", normalize(name).trim_end_matches(".vtf"));
        if let Some(c) = self.cube_by_path.get(&path) {
            return *c;
        }
        let decoded = self
            .read(&path)
            .ok_or_else(|| "not found".to_string())
            .and_then(|b| cube_faces(&b));
        let index = match decoded {
            Ok((size, faces)) => {
                self.cubemaps.push(crate::map::MapCubemap {
                    name: path.clone(),
                    size,
                    faces,
                });
                Some(self.cubemaps.len() - 1)
            }
            Err(e) => {
                self.missing.push(format!("{path}: {e}"));
                None
            }
        };
        self.cube_by_path.insert(path, index);
        index
    }

    /// A sky material's HDR texture as linear RGB (specs/cs_source/shaders.md
    /// section 6 and Constants): `$hdrbasetexture`, read linear, integer
    /// 16-bit formats times 16; or `$hdrcompressedtexture`, RGB times alpha
    /// times 8 (decoded per texel, before filtering, as the game does).
    pub fn hdr_sky_texture(&mut self, material: &str) -> Result<crate::map::MapHdrImage, String> {
        let text = self
            .read_text(&format!("materials/{}.vmt", normalize(material)))
            .ok_or("not found")?;
        let (name, compressed) = match material_key::<String>(&text, "$hdrbasetexture") {
            Some(n) => (n, false),
            None => (
                material_key::<String>(&text, "$hdrcompressedtexture").ok_or("no HDR texture")?,
                true,
            ),
        };
        let path = format!("materials/{}.vtf", normalize(&name).trim_end_matches(".vtf"));
        let bytes = self.read(&path).ok_or(format!("{path}: not found"))?;
        let vtf = vtf::from_bytes(&bytes).map_err(|e| e.to_string())?;
        let (width, height) = (vtf.header.width as u32, vtf.header.height as u32);
        let rgb: Vec<[f32; 3]> = match (compressed, vtf.header.highres_image_format) {
            (false, vtf::ImageFormat::Rgba16161616) => {
                let raw = vtf.highres_image.get_frame(0).map_err(|e| e.to_string())?;
                let c = |b: &[u8]| u16::from_le_bytes([b[0], b[1]]) as f32 / 65535.0 * 16.0;
                raw.as_chunks::<8>().0.iter().map(|p| [c(&p[0..]), c(&p[2..]), c(&p[4..])]).collect()
            }
            (false, vtf::ImageFormat::Rgba16161616f) => {
                let raw = vtf.highres_image.get_frame(0).map_err(|e| e.to_string())?;
                let c = |b: &[u8]| half::f16::from_le_bytes([b[0], b[1]]).to_f32();
                raw.as_chunks::<8>().0.iter().map(|p| [c(&p[0..]), c(&p[2..]), c(&p[4..])]).collect()
            }
            (true, _) => {
                let image = vtf.highres_image.decode(0).map_err(|e| e.to_string())?.to_rgba8();
                image
                    .pixels()
                    .map(|p| {
                        let s = p[3] as f32 / 255.0 * 8.0;
                        [0, 1, 2].map(|i| p[i] as f32 / 255.0 * s)
                    })
                    .collect()
            }
            (false, other) => return Err(format!("{path}: HDR format {other:?} not supported")),
        };
        if rgb.len() != (width * height) as usize {
            return Err(format!("{path}: {} texels for {width}x{height}", rgb.len()));
        }
        Ok(crate::map::MapHdrImage { width, height, rgb })
    }

    /// A 1x1 texture of a colour (0-1, gamma space).
    fn solid(&mut self, rgb: [f32; 3]) -> usize {
        let px = rgb.map(|c| (c.clamp(0.0, 1.0) * 255.0).round() as u8);
        let key = format!("solid:{px:?}");
        if let Some(Some(i)) = self.by_path.get(&key) {
            return *i;
        }
        self.textures.push(MapTexture {
            name: key.clone(),
            srgb: true,
            mips: Vec::new(),
            width: 1,
            height: 1,
            rgba8: vec![px[0], px[1], px[2], 255],
        });
        let index = self.textures.len() - 1;
        self.by_path.insert(key, Some(index));
        index
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

    /// Every frame of an animated texture (one, for a still one), each with
    /// its own mip levels.
    pub fn texture_frames(&mut self, name: &str, srgb: bool) -> Vec<usize> {
        let path = format!("materials/{}.vtf", normalize(name).trim_end_matches(".vtf"));
        let decoded = (|| -> Result<Vec<MapTexture>, String> {
            let bytes = self.read(&path).ok_or("not found")?;
            let vtf = vtf::from_bytes(&bytes).map_err(|e| e.to_string())?;
            (0..vtf.header.frames.max(1) as u32)
                .map(|f| {
                    let image = vtf.highres_image.decode(f).map_err(|e| e.to_string())?.to_rgba8();
                    Ok(MapTexture {
                        name: format!("{path}#{f}"),
                        srgb,
                        mips: frame_mip_levels(&bytes, &vtf.header, f).unwrap_or_default(),
                        width: image.width(),
                        height: image.height(),
                        rgba8: image.into_raw(),
                    })
                })
                .collect()
        })();
        match decoded {
            Ok(frames) => frames
                .into_iter()
                .map(|t| {
                    self.textures.push(t);
                    self.textures.len() - 1
                })
                .collect(),
            Err(e) => {
                self.missing.push(format!("{path}: {e}"));
                Vec::new()
            }
        }
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
    if header.frames > 1 {
        return None;
    }
    frame_mip_levels(bytes, header, 0)
}

/// `mip_levels` of one frame of an animated texture (each level holds
/// every frame in turn).
fn frame_mip_levels(bytes: &[u8], header: &vtf::header::VTFHeader, frame: u32) -> Option<Vec<Vec<u8>>> {
    use vtf::{image::VTFImage, resources::ResourceType};

    let frames = header.frames.max(1) as usize;
    if header.mipmap_count <= 1 || header.depth > 1 || frame as usize >= frames {
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
            offset += size(smaller)?.2 * frames;
        }
        let (w, h, level_size) = size(m)?;
        offset += frame as usize * level_size;
        let mut single = header.clone();
        single.mipmap_count = 1;
        let img = VTFImage::new(single, format, w as u16, h as u16, bytes, offset);
        out.push(img.decode(0).ok()?.to_rgba8().into_raw());
    }
    Some(out)
}

/// Shaders the VMT parser doesn't know, read as the closest one it does.
#[derive(Clone, Copy, Debug, PartialEq)]
enum StandIn {
    /// WorldTwoTextureBlend (de_aztec's walls): LightmappedGeneric whose
    /// `$detail` is a second texture blended over the base by the detail's
    /// alpha (detail blend mode 2). The aztec materials all set
    /// `$detail_alpha_mask_base_texture 1`; its absence isn't modelled.
    TwoTextureBlend,
    /// Eyes and Teeth (HL2 faces: the hostages): a plain VertexLitGeneric
    /// model surface; the iris, glint and mouth darkening aren't modelled.
    Model,
    /// Decal shaders with their own names
    /// (DecalBaseTimesLightmapAlphaBlendSelfIllum on de_nuke): a translucent
    /// LightmappedGeneric decal. Self-illumination isn't modelled.
    Decal,
}

/// `text` with an unknown shader name replaced by its stand-in.
fn stand_in_shader(text: &str) -> (std::borrow::Cow<'_, str>, Option<StandIn>) {
    use std::borrow::Cow;
    let trimmed = text.trim_start_matches(|c: char| c.is_whitespace() || c == '\u{feff}');
    let start = text.len() - trimmed.len();
    let quoted = trimmed.starts_with('"');
    let body = if quoted { &trimmed[1..] } else { trimmed };
    let len = body
        .find(|c: char| c == '"' || c == '{' || c.is_whitespace())
        .unwrap_or(body.len());
    let shader = body[..len].to_ascii_lowercase();
    let (stand_in, extra) = match shader.as_str() {
        "worldtwotextureblend" => (StandIn::TwoTextureBlend, ""),
        "eyes" | "teeth" => (StandIn::Model, ""),
        s if s.starts_with("decalbasetimeslightmap") => {
            (StandIn::Decal, "\n\"$decal\" \"1\"\n\"$translucent\" \"1\"\n")
        }
        _ => return (Cow::Borrowed(text), None),
    };
    let rest = &text[start + usize::from(quoted) + len..];
    // Extra keys go just inside the first block.
    let rest = match rest.find('{') {
        Some(i) if !extra.is_empty() => format!("{}{{{extra}{}", &rest[..i], &rest[i + 1..]),
        _ => rest.to_string(),
    };
    let quote = if quoted { "\"" } else { "" };
    let shader = if stand_in == StandIn::Model {
        "VertexLitGeneric"
    } else {
        "LightmappedGeneric"
    };
    (
        Cow::Owned(format!("{}{quote}{shader}{rest}", &text[..start])),
        Some(stand_in),
    )
}

/// `$detailblendmode` from a material's text (0 when absent).
fn detail_blend_mode(text: &str) -> u32 {
    material_key(text, "$detailblendmode").unwrap_or(0)
}

/// `$color` x `$color2` from a material's keys, preferring their `srgb?`
/// forms: `[r g b]` as given, `{r g b}` as 0..255 bytes (gamma, decoded).
/// None when neither is set or both are white.
fn tint(keys: &HashMap<String, String>) -> Option<[f32; 3]> {
    let vector = |name: &str| -> Option<[f32; 3]> {
        let v = keys.get(&format!("srgb?{name}")).or_else(|| keys.get(name))?.trim();
        let bytes = v.starts_with('{');
        let n: Vec<f32> = v
            .trim_matches(|c| c == '[' || c == ']' || c == '{' || c == '}')
            .split_whitespace()
            .filter_map(|x| x.parse().ok())
            .collect();
        let n: [f32; 3] = match n.as_slice() {
            [a, b, c] => [*a, *b, *c],
            [a] => [*a, *a, *a],
            _ => return None,
        };
        Some(if bytes { n.map(|x| (x / 255.0).powf(2.2)) } else { n })
    };
    let (a, b) = (vector("$color"), vector("$color2"));
    let t = match (a, b) {
        (None, None) => return None,
        (Some(a), None) | (None, Some(a)) => a,
        (Some(a), Some(b)) => [a[0] * b[0], a[1] * b[1], a[2] * b[2]],
    };
    (t != [1.0, 1.0, 1.0]).then_some(t)
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

/// A VTF cubemap's six faces at full size, RGBA8. VTF stores mips smallest
/// first, each mip holding every frame's faces; versions before 7.5 add a
/// seventh (sphere map) face unless the first frame is 0xFFFF.
fn cube_faces(bytes: &[u8]) -> Result<(u32, [Vec<u8>; 6]), String> {
    use vtf::{image::VTFImage, resources::ResourceType};
    const ENVMAP: u32 = 0x4000;
    let vtf = vtf::from_bytes(bytes).map_err(|e| e.to_string())?;
    let header = &vtf.header;
    if header.flags & ENVMAP == 0 {
        return Err("not a cubemap".into());
    }
    let minor = header.version[1];
    let first_frame = u16::from_le_bytes([bytes[26], bytes[27]]);
    let faces: usize = if minor < 5 && first_frame != 0xFFFF { 7 } else { 6 };
    let format = header.highres_image_format;
    let lowres_offset = match header
        .resources
        .get_by_type(ResourceType::VTF_LEGACY_RSRC_LOW_RES_IMAGE)
    {
        Some(r) => r.data,
        None => header.header_size,
    };
    let data_start = match header.resources.get_by_type(ResourceType::VTF_LEGACY_RSRC_IMAGE) {
        Some(r) => r.data as usize,
        None => {
            lowres_offset as usize
                + header
                    .lowres_image_format
                    .frame_size(header.lowres_image_width as u32, header.lowres_image_height as u32)
                    .unwrap_or(0) as usize
        }
    };
    let size = |m: u32| -> Result<usize, String> {
        let (w, h) = ((header.width as u32 >> m).max(1), (header.height as u32 >> m).max(1));
        format.frame_size(w, h).map(|s| s as usize).map_err(|e| e.to_string())
    };
    let frames = header.frames.max(1) as usize;
    let mut offset = data_start;
    for m in 1..header.mipmap_count.max(1) as u32 {
        offset += size(m)? * frames * faces;
    }
    let face_size = size(0)?;
    if offset + face_size * 6 > bytes.len() {
        return Err("cubemap data truncated".into());
    }
    let (w, h) = (header.width as u32, header.height as u32);
    let mut single = header.clone();
    single.mipmap_count = 1;
    let face = |k: usize| -> Result<Vec<u8>, String> {
        let img = VTFImage::new(
            single.clone(),
            format,
            w as u16,
            h as u16,
            bytes,
            offset + k * face_size,
        );
        Ok(img.decode(0).map_err(|e| e.to_string())?.to_rgba8().into_raw())
    };
    Ok((w, [face(0)?, face(1)?, face(2)?, face(3)?, face(4)?, face(5)?]))
}

/// "[a b c]", "a b c" or "a" as three numbers (a scalar repeats).
fn vector(v: &str) -> Option<[f32; 3]> {
    let parts: Vec<f32> = v
        .trim()
        .trim_start_matches(['[', '{'])
        .trim_end_matches([']', '}'])
        .split_whitespace()
        .filter_map(|p| p.parse().ok())
        .collect();
    match parts.as_slice() {
        [a] => Some([*a; 3]),
        [a, b, c, ..] => Some([*a, *b, *c]),
        _ => None,
    }
}

/// A VTF's sprite sheet (resource tag 0x10): per sequence, its frames'
/// texture rectangles (first image of each frame). None without one.
pub fn sheet(bytes: &[u8]) -> Option<Vec<Vec<[f32; 4]>>> {
    let u32_at = |o: usize| {
        bytes
            .get(o..o + 4)
            .map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    };
    let f32_at = |o: usize| u32_at(o).map(f32::from_bits);
    // Resources exist from version 7.3: count at 68, entries from 80.
    if u32_at(4)? != 7 || u32_at(8)? < 3 {
        return None;
    }
    let count = u32_at(68)? as usize;
    let offset = (0..count.min(32)).find_map(|i| {
        let e = 80 + i * 8;
        let tag = bytes.get(e..e + 3)?;
        if tag == [0x10, 0, 0] { u32_at(e + 4) } else { None }
    })? as usize;
    // A size, then the sheet: version, sequence count, then sequences.
    let mut o = offset + 4;
    let version = u32_at(o)?;
    let images = if version == 0 { 1 } else { 4 };
    let sequences = u32_at(o + 4)? as usize;
    o += 8;
    let mut out: Vec<Vec<[f32; 4]>> = Vec::new();
    for _ in 0..sequences.min(64) {
        let number = u32_at(o)? as usize;
        let frames = u32_at(o + 8)? as usize;
        o += 16;
        let mut list = Vec::new();
        for _ in 0..frames.min(512) {
            // Duration, then the images' rectangles.
            let r = o + 4;
            list.push([f32_at(r)?, f32_at(r + 4)?, f32_at(r + 8)?, f32_at(r + 12)?]);
            o += 4 + 16 * images;
        }
        if number < 64 {
            if out.len() <= number {
                out.resize(number + 1, Vec::new());
            }
            out[number] = list;
        }
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn colour_multipliers() {
        let keys = |pairs: &[(&str, &str)]| -> HashMap<String, String> {
            pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect()
        };
        // de_nuke's warehouse light cones: the srgb? form wins.
        assert_eq!(
            tint(&keys(&[("srgb?$color2", "[.4 .4 .4]"), ("$color2", "[.9 .9 .9]")])),
            Some([0.4, 0.4, 0.4])
        );
        // Braces are gamma bytes; $color and $color2 multiply.
        let t = tint(&keys(&[("$color", "{255 128 0}"), ("$color2", "[0.5 1 1]")])).unwrap();
        assert!((t[0] - 0.5).abs() < 1e-6 && (t[1] - (128.0f32 / 255.0).powf(2.2)).abs() < 1e-6 && t[2] == 0.0);
        assert_eq!(tint(&keys(&[("$color", "[1 1 1]")])), None);
        assert_eq!(tint(&keys(&[])), None);
    }

    #[test]
    fn unknown_shaders_parse_as_their_stand_in() {
        let wttb = "\"WorldTwoTextureBlend\"\n{\n\t\"$basetexture\" \"a/base\"\n\t\"$detail\" \"a/detail\"\n}\n";
        let (text, s) = stand_in_shader(wttb);
        assert_eq!(s, Some(StandIn::TwoTextureBlend));
        let m = vmt_parser::from_str(&text).expect("parses");
        assert_eq!(m.base_texture(), Some("a/base"));

        let decal = "DecalBaseTimesLightmapAlphaBlendSelfIllum { \"$basetexture\" \"d/x\" \"$decalscale\" 0.25 }";
        let (text, s) = stand_in_shader(decal);
        assert_eq!(s, Some(StandIn::Decal));
        let m = vmt_parser::from_str(&text).expect("parses");
        assert!(m.translucent());
        assert!(matches!(m, vmt_parser::material::Material::LightMappedGeneric(ref g) if g.decal));

        let known = "\"LightmappedGeneric\" { \"$basetexture\" \"x\" }";
        assert!(matches!(stand_in_shader(known), (std::borrow::Cow::Borrowed(_), None)));
    }
}
