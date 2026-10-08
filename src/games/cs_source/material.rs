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

/// A packfile's entries by lower-case name (with `/` separators).
fn pack_names(pack: &vbsp::Packfile) -> HashMap<String, String> {
    let zip = pack.clone().into_zip();
    let zip = zip.lock().unwrap_or_else(|e| e.into_inner());
    zip.file_names()
        .map(|n| (n.replace('\\', "/").to_lowercase(), n.to_string()))
        .collect()
}

pub struct MaterialLoader<'a> {
    bsp: &'a Bsp,
    mount: &'a Mount,
    pub textures: Vec<MapTexture>,
    pub cubemaps: Vec<crate::map::MapCubemap>,
    by_path: HashMap<String, Option<usize>>,
    cube_by_path: HashMap<String, Option<usize>>,
    /// The map's packed files by lower-case path: maps pack names in any
    /// case (`legomg/Shotgun.vmt`) and the game finds them whatever case
    /// the map's own references use.
    pack_names: HashMap<String, String>,
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
            pack_names: pack_names(&bsp.pack),
            missing: Vec::new(),
        }
    }

    /// A file from the map's pakfile only (any case).
    pub fn read_packed(&self, path: &str) -> Option<Vec<u8>> {
        let path = normalize(path);
        if let Ok(Some(data)) = self.bsp.pack.get(&path) {
            return Some(data);
        }
        self.pack_names
            .get(&path.to_lowercase())
            .and_then(|n| self.bsp.pack.get(n).ok().flatten())
    }

    pub fn read(&self, path: &str) -> Option<Vec<u8>> {
        let path = normalize(path);
        if let Ok(Some(data)) = self.bsp.pack.get(&path) {
            return Some(data);
        }
        if let Some(name) = self.pack_names.get(&path.to_lowercase()) {
            match self.bsp.pack.get(name) {
                Ok(Some(data)) => return Some(data),
                Err(_) => return self.packed_lzma(name),
                Ok(None) => {}
            }
        }
        self.mount.read(&path).ok()
    }

    /// A packed file the zip reader's LZMA decoder refuses ("stream is
    /// corrupted": surf_demise's 78 MB HDR sky cubemap, which the game
    /// and other zip readers decode), decoded from its raw entry with
    /// lzma-rs. Zip's LZMA entries: a version (2 bytes), the property
    /// size (2, always 5), the properties (5), then the stream.
    fn packed_lzma(&self, name: &str) -> Option<Vec<u8>> {
        use std::io::Read;
        let zip = self.bsp.pack.clone().into_zip();
        let mut zip = zip.lock().unwrap_or_else(|e| e.into_inner());
        let index = (0..zip.len()).find(|&i| zip.by_index_raw(i).is_ok_and(|f| f.name() == name))?;
        let mut entry = zip.by_index_raw(index).ok()?;
        let size = entry.size();
        let mut raw = Vec::new();
        entry.read_to_end(&mut raw).ok()?;
        if raw.get(2..4)? != [5, 0] {
            return None;
        }
        let mut out = Vec::with_capacity(size as usize);
        lzma_rs::lzma_decompress_with_options(
            &mut std::io::Cursor::new(&raw[4..]),
            &mut out,
            &lzma_rs::decompress::Options {
                unpacked_size: lzma_rs::decompress::UnpackedSize::UseProvided(Some(size)),
                allow_incomplete: false,
                memlimit: None,
            },
        )
        .ok()?;
        (out.len() as u64 == size).then_some(out)
    }

    /// A packed file under `materials/maps/<any folder>/` named `file`
    /// (lower case): a renamed map's own files sit under its original
    /// name. The path without `materials/` and the extension.
    pub fn packed_map_file(&self, file: &str) -> Option<String> {
        let mut found: Vec<&String> = self
            .pack_names
            .keys()
            .filter(|k| k.starts_with("materials/maps/") && k.rsplit('/').next() == Some(file))
            .collect();
        found.sort();
        found
            .first()
            .and_then(|k| k.strip_prefix("materials/"))
            .map(|k| k.trim_end_matches(".vtf").to_string())
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
        let (text, mut stand_in) = stand_in_shader(&text);
        match stand_in {
            Some(StandIn::WindowImposter) => return self.window_imposter(&vmt_path, &text, fallback),
            Some(StandIn::Reflective) => return self.reflective(&text, fallback),
            _ => {}
        }
        // Strictly first; what the parser refuses is read again leniently,
        // the way the game reads it (`lenient_vmt`).
        let material = self
            .parse_vmt(&text, false)
            .or_else(|e| self.parse_vmt(&text, true).map_err(|_| e))
            .map(|(m, included)| {
                // A patch of a ShatteredGlass material (a window's
                // cubemap-patched `$crackmaterial`) reads as one.
                if included == Some(StandIn::ShatteredGlass) {
                    stand_in = included;
                }
                m
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
        // Envmap-only surfaces (no `$basetexture`) have black albedo
        // (specs/cs_source/shaders.md 2): surf_demise's ramps glow with
        // a tinted sky cubemap through translucent marble.
        let generic = matches!(
            material,
            vmt_parser::material::Material::LightMappedGeneric(_)
                | vmt_parser::material::Material::VertexLitGeneric(_)
                | vmt_parser::material::Material::UnlitGeneric(_)
        );
        if generic && base.is_none() && texture.is_none() && self.keys(&text, 0).contains_key("$envmap") {
            texture = Some(self.solid([0.0; 3]));
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
            .filter(|_| detail_mode <= 4 && stand_in != Some(StandIn::ShatteredGlass))
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

    /// A material's text parsed, patches resolved through their included
    /// material (whose unknown shader gets its stand-in too, returned);
    /// `lenient`: both texts through `lenient_vmt` first.
    fn parse_vmt(
        &self,
        text: &str,
        lenient: bool,
    ) -> Result<(vmt_parser::material::Material, Option<StandIn>), VmtError> {
        let fix = |t: &str| if lenient { lenient_vmt(t) } else { t.to_string() };
        let included_stand_in = std::cell::Cell::new(None);
        let material = vmt_parser::from_str(&fix(text)).map_err(VmtError::from).and_then(|m| {
            m.resolve(|include: &str| {
                let included = self.read_text(include).ok_or(VmtError::Missing(include.to_string()))?;
                let (included, stand_in) = stand_in_shader(&included);
                included_stand_in.set(stand_in);
                Ok(fix(&included))
            })
        })?;
        Ok((material, included_stand_in.get()))
    }

    /// WindowImposter ("fake sky" windows on surf maps): the `$envmap`
    /// cubemap seen through the surface, unlit, tinted by `$color`
    /// (`MapEnvmap::imposter`). The shader isn't specified
    /// (specs/cs_source/shaders.md, open question 15): drawn as if the
    /// cubemap were infinitely far, like a sky.
    fn window_imposter(&mut self, vmt_path: &str, text: &str, fallback: Resolved) -> Resolved {
        let keys = self.keys(text, 0);
        let Some(name) = keys.get("$envmap").cloned() else {
            self.missing.push(format!("{vmt_path}: WindowImposter without $envmap"));
            return fallback;
        };
        let Some(cube) = self.cubemap(&name) else {
            return fallback;
        };
        Resolved {
            texture: Some(self.solid([0.0; 3])),
            unlit: true,
            envmap: Some(crate::map::MapEnvmap {
                cubemap: Some(cube),
                mask: crate::map::EnvmapMask::None,
                tint: tint(&keys).unwrap_or([1.0; 3]),
                contrast: 0.0,
                saturation: 1.0,
                fresnel: 1.0,
                imposter: true,
            }),
            ..fallback
        }
    }

    /// LightmappedReflective (glass that shows the scene reflected in real
    /// time, and refracted behind it): an opaque lightmapped surface of its
    /// `$refracttint`, with `$envmap` reflections if it names one. A
    /// stand-in (specs/cs_source/shaders.md, open question 15).
    fn reflective(&mut self, text: &str, fallback: Resolved) -> Resolved {
        let keys = self.keys(text, 0);
        let colour = keys.get("$refracttint").and_then(|v| vector(v)).unwrap_or([0.5; 3]);
        Resolved {
            texture: Some(self.solid(colour)),
            surfaceprop: keys.get("$surfaceprop").map(|s| s.to_lowercase()),
            envmap: self.envmap(text, false),
            ..fallback
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

    /// A key of material `name` (as `resolve` takes it), e.g.
    /// `$crackmaterial`: lower-case key, the value as written.
    pub fn material_value(&self, name: &str, key: &str) -> Option<String> {
        let text = self.read_text(&format!("materials/{}.vmt", normalize(name)))?;
        self.keys(&text, 0).remove(&key.to_lowercase())
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
            imposter: false,
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
                let image = decode_rgba8(&vtf.highres_image, 0)?;
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
                    let image = decode_full(&bytes, &vtf, f)?;
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
        let image = decode_full(&bytes, &vtf, 0)?;
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

/// One frame of a VTF image as RGBA8: the `vtf` crate's decoders, plus the
/// uncompressed formats it leaves out (community maps use them: ABGR8888,
/// ARGB8888, BGRX8888, I8, IA88, A8, the 16-bit packed formats and the
/// "bluescreen" ones, whose pure blue texels are transparent). Channel
/// orders follow the public VTF format description.
pub fn decode_rgba8(img: &vtf::image::VTFImage<'_>, frame: u32) -> Result<image::RgbaImage, String> {
    match img.decode(frame) {
        Ok(i) => return Ok(i.to_rgba8()),
        Err(vtf::Error::UnsupportedImageFormat(_)) => {}
        Err(e) => return Err(e.to_string()),
    }
    let bytes = img.get_frame(frame).map_err(|e| e.to_string())?;
    let (w, h) = (img.width as u32, img.height as u32);
    let rgba = convert_texels(img.format, bytes).ok_or_else(|| format!("Decoding {} images is not supported", img.format))?;
    image::RgbaImage::from_raw(w, h, rgba).ok_or_else(|| "bad image size".to_string())
}

/// Bytes per texel of the uncompressed formats whose size the `vtf` crate
/// doesn't know (so it can't find their data).
fn extra_texel_bytes(format: vtf::ImageFormat) -> Option<usize> {
    use vtf::ImageFormat as F;
    match format {
        F::Bgrx8888 => Some(4),
        F::Rgb888Bluescreen | F::Bgr888Bluescreen => Some(3),
        F::Bgr565 | F::Bgrx5551 | F::Bgra4444 | F::Bgra5551 => Some(2),
        _ => None,
    }
}

/// A frame of a VTF's full-size image as RGBA8 (`decode_rgba8`), also for
/// the formats the `vtf` crate can't locate: their data found here, as in
/// `frame_mip_levels` (smaller levels first, each holding every frame).
fn decode_full(bytes: &[u8], vtf: &vtf::vtf::VTF<'_>, frame: u32) -> Result<image::RgbaImage, String> {
    let header = &vtf.header;
    let format = header.highres_image_format;
    let Some(texel) = extra_texel_bytes(format).filter(|_| header.depth <= 1) else {
        return decode_rgba8(&vtf.highres_image, frame);
    };
    let start = match header
        .resources
        .get_by_type(vtf::resources::ResourceType::VTF_LEGACY_RSRC_IMAGE)
    {
        Some(r) => r.data as usize,
        None => {
            header.header_size as usize
                + header
                    .lowres_image_format
                    .frame_size(header.lowres_image_width as u32, header.lowres_image_height as u32)
                    .unwrap_or(0) as usize
        }
    };
    let (w, h) = (header.width as usize, header.height as usize);
    let level = |m: usize| (w >> m).max(1) * (h >> m).max(1) * texel;
    let frames = header.frames.max(1) as usize;
    let at = start + (1..header.mipmap_count.max(1) as usize).map(|m| level(m) * frames).sum::<usize>() + frame as usize * level(0);
    let data = bytes.get(at..at + level(0)).ok_or("image data truncated")?;
    let rgba = convert_texels(format, data).ok_or("unreadable format")?;
    image::RgbaImage::from_raw(w as u32, h as u32, rgba).ok_or_else(|| "bad image size".to_string())
}

/// Uncompressed VTF texels to RGBA8 (None: a format this doesn't read).
pub fn convert_texels(format: vtf::ImageFormat, bytes: &[u8]) -> Option<Vec<u8>> {
    use vtf::ImageFormat as F;
    let five = |v: u16| ((v & 31) as u32 * 255 / 31) as u8;
    let six = |v: u16| ((v & 63) as u32 * 255 / 63) as u8;
    let four = |v: u16| ((v & 15) as u32 * 17) as u8;
    let words = || bytes.chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]]));
    let out: Vec<[u8; 4]> = match format {
        F::Abgr8888 => bytes.chunks_exact(4).map(|p| [p[3], p[2], p[1], p[0]]).collect(),
        F::Argb8888 => bytes.chunks_exact(4).map(|p| [p[1], p[2], p[3], p[0]]).collect(),
        F::Bgrx8888 => bytes.chunks_exact(4).map(|p| [p[2], p[1], p[0], 255]).collect(),
        F::I8 => bytes.iter().map(|&l| [l, l, l, 255]).collect(),
        F::Ia88 => bytes.chunks_exact(2).map(|p| [p[0], p[0], p[0], p[1]]).collect(),
        F::A8 => bytes.iter().map(|&a| [0, 0, 0, a]).collect(),
        F::Rgb888Bluescreen | F::Bgr888Bluescreen => bytes
            .chunks_exact(3)
            .map(|p| {
                let (r, g, b) = if format == F::Rgb888Bluescreen { (p[0], p[1], p[2]) } else { (p[2], p[1], p[0]) };
                if (r, g, b) == (0, 0, 255) { [0, 0, 0, 0] } else { [r, g, b, 255] }
            })
            .collect(),
        F::Rgb565 => words().map(|v| [five(v), six(v >> 5), five(v >> 11), 255]).collect(),
        F::Bgr565 => words().map(|v| [five(v >> 11), six(v >> 5), five(v), 255]).collect(),
        F::Bgra4444 => words().map(|v| [four(v >> 8), four(v >> 4), four(v), four(v >> 12)]).collect(),
        F::Bgrx5551 | F::Bgra5551 => words()
            .map(|v| {
                let a = if format == F::Bgrx5551 || v & 0x8000 != 0 { 255 } else { 0 };
                [five(v >> 10), five(v >> 5), five(v), a]
            })
            .collect(),
        // Half floats, linear light (HDR cubemaps a WindowImposter or an
        // envmap-only surface names in LDR too): clamped, sRGB-encoded
        // for the 8-bit sRGB texture.
        F::Rgba16161616f => bytes
            .chunks_exact(8)
            .map(|p| {
                let c = |k: usize| half::f16::from_le_bytes([p[2 * k], p[2 * k + 1]]).to_f32();
                let srgb = |v: f32| {
                    let v = if v.is_finite() { v.clamp(0.0, 1.0) } else { 0.0 };
                    let e = if v <= 0.0031308 { v * 12.92 } else { 1.055 * v.powf(1.0 / 2.4) - 0.055 };
                    (e * 255.0).round() as u8
                };
                [srgb(c(0)), srgb(c(1)), srgb(c(2)), (c(3).clamp(0.0, 1.0) * 255.0).round() as u8]
            })
            .collect(),
        _ => return None,
    };
    Some(out.into_iter().flatten().collect())
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
        out.push(decode_rgba8(&img, 0).ok()?.into_raw());
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
    /// ShatteredGlass (a breakable window's `$crackmaterial`): a
    /// translucent LightmappedGeneric showing its crack texture (`$detail`,
    /// the same image as its "dummy" base in stock materials) as the base;
    /// its per-pane proxy isn't modelled (the panes are cut from the mesh).
    ShatteredGlass,
    /// LightmappedReflective (reflective glass, no base texture): drawn by
    /// `MaterialLoader::reflective`.
    Reflective,
    /// WindowImposter: drawn by `MaterialLoader::window_imposter`.
    WindowImposter,
    /// A DirectX-level variant name (`Refract_DX90`) read as its shader.
    DxVariant,
}

/// Shaders the VMT parser knows (its `Material` variants).
const PARSER_SHADERS: &[&str] = &[
    "lightmappedgeneric",
    "vertexlitgeneric",
    "vertexlitgeneric_dx6",
    "unlitgeneric",
    "unlittwotexture",
    "water",
    "worldvertextransition",
    "eyerefract",
    "subrect",
    "sprite",
    "spritecard",
    "cable",
    "refract",
    "modulate",
    "decalmodulate",
    "sky",
    "replacements",
    "patch",
];

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
        "shatteredglass" => (StandIn::ShatteredGlass, ""),
        s if s.starts_with("decalbasetimeslightmap") => {
            (StandIn::Decal, "\n\"$decal\" \"1\"\n\"$translucent\" \"1\"\n")
        }
        "lightmappedreflective" => (StandIn::Reflective, ""),
        "windowimposter" => (StandIn::WindowImposter, ""),
        s if dx_variant_of(s).is_some() => (StandIn::DxVariant, ""),
        _ => return (Cow::Borrowed(text), None),
    };
    let rest = &text[start + usize::from(quoted) + len..];
    // Extra keys go just inside the first block.
    let rest = match rest.find('{') {
        Some(i) if !extra.is_empty() => format!("{}{{{extra}{}", &rest[..i], &rest[i + 1..]),
        _ => rest.to_string(),
    };
    let quote = if quoted { "\"" } else { "" };
    let shader = match stand_in {
        StandIn::Model => "VertexLitGeneric",
        StandIn::DxVariant => dx_variant_of(&shader).unwrap_or("LightmappedGeneric"),
        _ => "LightmappedGeneric",
    };
    (
        Cow::Owned(format!("{}{quote}{shader}{rest}", &text[..start])),
        Some(stand_in),
    )
}

/// A shader name with a DirectX-level suffix (`refract_dx90`,
/// `unlitgeneric_dx8`) whose base name the parser knows: that name.
fn dx_variant_of(shader: &str) -> Option<&'static str> {
    let (base, level) = shader.rsplit_once("_dx")?;
    if PARSER_SHADERS.contains(&shader) || level.is_empty() || !level.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    PARSER_SHADERS.iter().copied().find(|s| *s == base)
}

/// A material's text rewritten the way the game reads what the VMT parser
/// refuses (community maps' materials, found by the map sweep):
/// - unbalanced braces: missing `}` at the end are added, extra ones and
///   a key without a value dropped;
/// - in the shader's block, conditional blocks (`">=DX90" { ... }`) and
///   the DirectX 9 fallback block (`<shader>_dx9`) count as their keys
///   when they hold at DirectX level 9 (overriding), and are dropped
///   otherwise, like other DirectX-level blocks (`<shader>_dx6`);
/// - a key given twice: the last one counts;
/// - WorldVertexTransition without `$basetexture2` is LightmappedGeneric
///   (one layer; what the map compiler itself writes for such a
///   material's brush faces, in the `_wvt_patch` materials it packs);
/// - `$detailscale` with more numbers than two keeps the first two
///   (`"[9 9 9]"`); `{r g b}` colours become `[r g b]` / 255, without a
///   fourth number;
/// - texture transforms the parser can't read (`"11"`) are dropped
///   (identity).
///
/// The result is re-quoted text without comments.
fn lenient_vmt(text: &str) -> String {
    enum Node {
        Pair(String, String),
        Block(String, Vec<Node>),
    }
    fn block(t: &[String], i: &mut usize) -> Vec<Node> {
        let mut out = Vec::new();
        while *i < t.len() {
            let tok = &t[*i];
            *i += 1;
            match tok.as_str() {
                "}" => return out,
                // A block without a name: its keys belong here.
                "{" => out.extend(block(t, i)),
                _ => match t.get(*i).map(String::as_str) {
                    Some("{") => {
                        *i += 1;
                        out.push(Node::Block(tok.clone(), block(t, i)));
                    }
                    Some("}") | None => {}
                    Some(value) => {
                        out.push(Node::Pair(tok.clone(), value.to_string()));
                        *i += 1;
                    }
                },
            }
        }
        out
    }
    /// Whether a block in the shader's block applies at DirectX level 9
    /// (Some(true): its keys count; Some(false): dropped; None: kept).
    fn applies(name: &str) -> Option<bool> {
        const LEVEL: u32 = 95;
        let n = name.to_ascii_lowercase();
        let ops: [(&str, fn(u32, u32) -> bool); 4] = [
            (">=dx", |l, x| l >= x),
            ("<=dx", |l, x| l <= x),
            (">dx", |l, x| l > x),
            ("<dx", |l, x| l < x),
        ];
        for (op, holds) in ops {
            if let Some(level) = n.strip_prefix(op).and_then(|x| x.parse::<u32>().ok()) {
                return Some(holds(LEVEL, level));
            }
        }
        let (_, level) = n.rsplit_once("_dx")?;
        match level {
            _ if n.contains("hdr") => Some(false),
            "9" | "90" | "95" => Some(true),
            l if !l.is_empty() && l.bytes().all(|b| b.is_ascii_digit()) => Some(false),
            _ => None,
        }
    }
    fn numbers(v: &str) -> Vec<f32> {
        v.trim()
            .trim_start_matches(['[', '{'])
            .trim_end_matches([']', '}'])
            .split_whitespace()
            .filter_map(|x| x.parse().ok())
            .collect()
    }
    fn value(key: &str, value: &str) -> Option<String> {
        let key = key.to_ascii_lowercase();
        if key == "$detailscale" {
            return match numbers(value).as_slice() {
                [] => None,
                [a] => Some(format!("{a}")),
                [a, b, ..] => Some(format!("[{a} {b}]")),
            };
        }
        if key.starts_with('$') && key.ends_with("transform") {
            let readable = value
                .to_ascii_lowercase()
                .parse::<vmt_parser::TextureTransform>()
                .is_ok();
            return readable.then(|| value.to_string());
        }
        if value.trim_start().starts_with('{') {
            let n: Vec<String> = numbers(value).iter().take(3).map(|x| format!("{}", x / 255.0)).collect();
            return Some(format!("[{}]", n.join(" ")));
        }
        Some(value.to_string())
    }
    fn quote(s: &str) -> String {
        format!("\"{}\"", s.replace('"', ""))
    }
    fn emit(nodes: Vec<Node>, out: &mut String) {
        let mut last: HashMap<String, usize> = HashMap::new();
        for (k, n) in nodes.iter().enumerate() {
            if let Node::Pair(key, _) = n {
                last.insert(key.to_ascii_lowercase(), k);
            }
        }
        for (k, n) in nodes.into_iter().enumerate() {
            match n {
                Node::Pair(key, v) => {
                    if last.get(&key.to_ascii_lowercase()) == Some(&k)
                        && let Some(v) = value(&key, &v)
                    {
                        out.push_str(&format!("{} {}\n", quote(&key), quote(&v)));
                    }
                }
                Node::Block(name, kids) => {
                    out.push_str(&format!("{} {{\n", quote(&name)));
                    emit(kids, out);
                    out.push_str("}\n");
                }
            }
        }
    }
    let t = super::surfaceprops::tokens(text);
    let has_base2 = t.iter().any(|s| s.eq_ignore_ascii_case("$basetexture2"));
    let mut i = 0;
    let mut out = String::new();
    for node in block(&t, &mut i) {
        let Node::Block(shader, kids) = node else {
            emit(vec![node], &mut out);
            continue;
        };
        let mut body = Vec::new();
        for kid in kids {
            match kid {
                Node::Block(name, inner) => match applies(&name) {
                    Some(true) => body.extend(inner),
                    Some(false) => {}
                    None => body.push(Node::Block(name, inner)),
                },
                pair => body.push(pair),
            }
        }
        let shader = if shader.eq_ignore_ascii_case("worldvertextransition") && !has_base2 {
            "LightmappedGeneric".to_string()
        } else {
            shader
        };
        emit(vec![Node::Block(shader, body)], &mut out);
    }
    out
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
        Ok(decode_rgba8(&img, 0)?.into_raw())
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

    /// Uncompressed formats the `vtf` crate leaves out (community maps'
    /// textures, found by the map sweep), by the public format's channel
    /// orders.
    #[test]
    fn uncompressed_formats_convert() {
        use vtf::ImageFormat as F;
        assert_eq!(convert_texels(F::Abgr8888, &[4, 3, 2, 1]), Some(vec![1, 2, 3, 4]));
        assert_eq!(convert_texels(F::Argb8888, &[4, 1, 2, 3]), Some(vec![1, 2, 3, 4]));
        assert_eq!(convert_texels(F::Bgrx8888, &[3, 2, 1, 0]), Some(vec![1, 2, 3, 255]));
        assert_eq!(convert_texels(F::I8, &[7]), Some(vec![7, 7, 7, 255]));
        assert_eq!(convert_texels(F::Ia88, &[7, 9]), Some(vec![7, 7, 7, 9]));
        assert_eq!(
            convert_texels(F::Bgr888Bluescreen, &[255, 0, 0, 1, 2, 3]),
            Some(vec![0, 0, 0, 0, 3, 2, 1, 255])
        );
        // BGR565: red in the top five bits.
        assert_eq!(convert_texels(F::Bgr565, &0xF800u16.to_le_bytes()), Some(vec![255, 0, 0, 255]));
        assert_eq!(convert_texels(F::Bgra5551, &0x801Fu16.to_le_bytes()), Some(vec![0, 0, 255, 255]));
        assert_eq!(convert_texels(F::Bgra4444, &0xF0F0u16.to_le_bytes()), Some(vec![0, 255, 0, 255]));
        assert_eq!(convert_texels(F::Uv88, &[1, 2]), None);
    }

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

        let glass = "\"ShatteredGlass\" { \"$basetexture\" \"glass/x\" \"$envmap\" \"env_cubemap\" }";
        let (text, s) = stand_in_shader(glass);
        assert_eq!(s, Some(StandIn::ShatteredGlass));
        assert_eq!(vmt_parser::from_str(&text).expect("parses").base_texture(), Some("glass/x"));

        let refract = "\"Refract_DX90\" { \"$normalmap\" \"w/n\" \"$refractamount\" \"0.05\" }";
        let (text, s) = stand_in_shader(refract);
        assert_eq!(s, Some(StandIn::DxVariant));
        assert!(matches!(
            vmt_parser::from_str(&text).expect("parses"),
            vmt_parser::material::Material::Refract(_)
        ));
        assert_eq!(dx_variant_of("vertexlitgeneric_dx6"), None);
        assert_eq!(dx_variant_of("unlitgeneric_dx8"), Some("unlitgeneric"));
        assert!(matches!(stand_in_shader("WindowImposter { $envmap x }").1, Some(StandIn::WindowImposter)));
    }

    /// What the parser refuses on community maps (the map sweep), read
    /// the game's way.
    #[test]
    fn lenient_reading() {
        use vmt_parser::material::Material;
        let strict = |t: &str| vmt_parser::from_str(t);
        let lenient = |t: &str| vmt_parser::from_str(&lenient_vmt(t)).expect("lenient parse");
        // A vec3 where the parser wants a vec2.
        let detail = "\"LightmappedGeneric\"\n{\n\t\"$basetexture\" \"b/x\"\n\t\"$detail\" \"detail\\plaster\"\n\t\"$detailscale\" \"[9 9 9]\" // comment\n\t$color \"{225 215 215}\"\n}\n";
        assert!(strict(detail).is_err());
        match lenient(detail) {
            Material::LightMappedGeneric(m) => assert_eq!(m.detail_scale.0, [9.0, 9.0]),
            m => panic!("{m:?}"),
        }
        // WorldVertexTransition without its second texture: one layer.
        let wvt = "WorldVertexTransition\n{\n\t$basetexture \"elly/snow\"\n\t$bumpmap \"elly/snow_n\"\n\t$translucent \"1\"\n}\n";
        assert!(strict(wvt).is_err());
        match lenient(wvt) {
            Material::LightMappedGeneric(m) => {
                assert_eq!(m.base_texture, "elly/snow");
                assert!(m.translucent);
            }
            m => panic!("{m:?}"),
        }
        // A transform the parser can't read is dropped; a full one stays.
        let transform = "\"WorldVertexTransition\" { \"$basetexture\" \"a\" \"$basetexture2\" \"b\" \"$basetexturetransform\" \"11\" }";
        assert!(strict(transform).is_err());
        assert!(matches!(lenient(transform), Material::WorldVertexTransition(_)));
        let full = "LightmappedGeneric { $basetexture a $basetexturetransform \"center .5 .5 scale 2 2 rotate 0 translate 0 0\" }";
        match lenient(full) {
            Material::LightMappedGeneric(m) => assert_eq!(m.base_texture_transform.scale, [2.0, 2.0]),
            m => panic!("{m:?}"),
        }
        // A missing closing brace (the proxies block's).
        let unclosed = "\"UnlitGeneric\"\n{\n\t\"$baseTexture\" \"m/laser\"\n\t\"$translucent\" 1\n\t\"Proxies\"\n\t{\n\t\t\"AnimatedTexture\"\n\t\t{\n\t\t\t\"animatedtexturevar\" \"$basetexture\"\n\t\t}\n}\n";
        assert!(strict(unclosed).is_err());
        assert!(matches!(lenient(unclosed), Material::UnlitGeneric(_)));
        // Conditional and DirectX-level blocks: the DX9 ones count.
        let blocks = "// comment\n\"LightmappedGeneric\"\n{\n\t\"LightmappedGeneric_DX6\" { \"$fallbackmaterial\" \"n/x_dx70\" }\n\t\"$envmap\" \"env_cubemap\"\n\t\"$fogcolor\" \"{29 99 39}\"\n\t\">=DX90\"\n\t{\n\t\t\"$basetexture\" \"Nature/slime\"\n\t\t\"Proxies\" { \"TextureScroll\" { \"texturescrollvar\" \"$bumptransform\" } }\n\t}\n\t\"<DX90\" { \"$basetexture\" \"Nature/old\" }\n}\n";
        assert!(strict(blocks).is_err());
        match lenient(blocks) {
            Material::LightMappedGeneric(m) => assert_eq!(m.base_texture, "nature/slime"),
            m => panic!("{m:?}"),
        }
        // A key given twice; a four-number colour in braces.
        let twice = "VertexlitGeneric { $basetexture \"m/b\" \"$envmap\" \"env_cubemap\" \"$envmap\" \"env_cubemap\" }";
        assert!(strict(twice).is_err());
        assert!(matches!(lenient(twice), Material::VertexLitGeneric(_)));
        let water = "\"Water\" { \"$normalmap\" \"dev/water_normal\" \"$fogcolor\" \"{55 248 7 200}\" \"$fogenable\" 1 }";
        assert!(strict(water).is_err());
        match lenient(water) {
            Material::Water(w) => assert!((w.fog_color.0[1] - 248.0 / 255.0).abs() < 1e-4),
            m => panic!("{m:?}"),
        }
        // Materials the parser takes read the same leniently.
        let fine = "\"LightmappedGeneric\" { \"$basetexture\" \"x/y\" \"$detailscale\" \"4\" \"$surfaceprop\" \"metal\" }";
        assert_eq!(
            format!("{:?}", strict(fine).unwrap()),
            format!("{:?}", lenient(fine))
        );
    }
}
