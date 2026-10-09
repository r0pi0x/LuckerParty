//! Baked lighting from BSP v20, per the public format description
//! (developer.valvesoftware.com/wiki/BSP_(Source), "Lighting"): each face
//! with lighting stores (size+1)^2 samples as RGB + shared signed exponent,
//! starting at the face's byte offset into the lighting lump. Faces' samples
//! are packed into one atlas.

use bevy::prelude::*;

use crate::map::{MapLightStyle, MapLightmap};

const LUMP_LIGHTING: usize = 8;
const LUMP_LIGHTING_HDR: usize = 53;
/// Luxels around each face's block, copied from its edge, so filtering
/// doesn't bleed into neighbours: 2 covers bicubic sampling's reach.
const PAD: u32 = 2;
const ATLAS_WIDTH: u32 = 1024;

const LUMP_FACES: usize = 7;
const LUMP_FACES_HDR: usize = 58;
/// Bytes per face (dface_t) and where its lighting offset sits.
const FACE_SIZE: usize = 56;
const FACE_LIGHT_OFS: usize = 20;

fn lump(bsp_bytes: &[u8], i: usize) -> &[u8] {
    let at = 8 + i * 16;
    let Some(entry) = bsp_bytes.get(at..at + 8) else {
        return &[];
    };
    let ofs = i32::from_le_bytes(entry[0..4].try_into().unwrap()).max(0) as usize;
    let len = i32::from_le_bytes(entry[4..8].try_into().unwrap()).max(0) as usize;
    bsp_bytes.get(ofs..ofs + len).unwrap_or(&[])
}

/// The raw lighting lump: LDR if present, else HDR.
pub fn lighting_lump(bsp_bytes: &[u8]) -> &[u8] {
    let ldr = lump(bsp_bytes, LUMP_LIGHTING);
    if ldr.is_empty() { lump(bsp_bytes, LUMP_LIGHTING_HDR) } else { ldr }
}

/// The HDR lighting lump (53), when the map has one that the faces we
/// read (lump 7) index: the HDR face lump (58) is absent or gives every
/// face the same lighting offset (true of every stock CS:S map that has
/// HDR lighting). Same luxel encoding as the LDR lump (RGB + shared
/// exponent, linear), per the public BSP v20 description.
pub fn hdr_lighting_lump(bsp_bytes: &[u8]) -> Option<&[u8]> {
    let hdr = lump(bsp_bytes, LUMP_LIGHTING_HDR);
    if hdr.is_empty() {
        return None;
    }
    let faces_hdr = lump(bsp_bytes, LUMP_FACES_HDR);
    if faces_hdr.is_empty() {
        return Some(hdr);
    }
    let offsets = |faces: &[u8]| -> Vec<[u8; 4]> {
        faces
            .chunks_exact(FACE_SIZE)
            .map(|f| f[FACE_LIGHT_OFS..FACE_LIGHT_OFS + 4].try_into().unwrap())
            .collect()
    };
    (offsets(faces_hdr) == offsets(lump(bsp_bytes, LUMP_FACES))).then_some(hdr)
}

/// One face's samples, linear RGB where 1.0 shows the texture unchanged.
pub struct FaceSamples {
    pub width: u32,
    pub height: u32,
    pub rgb: Vec<[f32; 3]>,
}

/// The face's first (style 0, unbumped) lightmap, if it has one.
pub fn face_samples(lump: &[u8], face: &vbsp::Face) -> Option<FaceSamples> {
    block(lump, face, 0)
}

/// For bump-mapped faces (texinfo flag SURF_BUMPLIGHT): the three
/// directional lightmaps that follow the unbumped one, one per basis
/// direction of Valve's radiosity normal mapping.
pub fn face_bumped_samples(lump: &[u8], face: &vbsp::Face, bumped: bool) -> Option<[FaceSamples; 3]> {
    if !bumped {
        return None;
    }
    Some([block(lump, face, 1)?, block(lump, face, 2)?, block(lump, face, 3)?])
}

/// A face's lighting with every light style that is lit at map start
/// added together: style 0, the animated presets (1-31, at their normal
/// brightness) and switchable styles (32+) unless `dark(style)` (their
/// lights start off). Each style stores the flat block and, for bumped
/// faces, the three directional blocks, one style after another. Returns
/// the flat block and the bumped pages.
pub fn face_samples_lit(
    lump: &[u8],
    face: &vbsp::Face,
    bumped: bool,
    dark: &dyn Fn(u8) -> bool,
) -> (Option<FaceSamples>, Option<[FaceSamples; 3]>) {
    let per_style = if bumped { 4 } else { 1 };
    let sum = |index: u32| -> Option<FaceSamples> {
        let mut out: Option<FaceSamples> = None;
        for (k, &style) in face.styles.iter().enumerate() {
            if style == 255 {
                break;
            }
            if style >= 32 && dark(style) {
                continue;
            }
            let b = block(lump, face, k as u32 * per_style + index)?;
            match &mut out {
                None => out = Some(b),
                Some(o) => o.rgb.iter_mut().zip(&b.rgb).for_each(|(a, b)| {
                    a[0] += b[0];
                    a[1] += b[1];
                    a[2] += b[2];
                }),
            }
        }
        // Lit only by lights that start off: black, not unlit.
        if out.is_none() {
            out = block(lump, face, index).map(|mut b| {
                b.rgb.fill([0.0; 3]);
                b
            });
        }
        out
    };
    let flat = sum(0);
    let pages = if bumped {
        (|| Some([sum(1)?, sum(2)?, sum(3)?]))()
    } else {
        None
    };
    (flat, pages)
}

/// A face's light styles other than the steady one (0) apart: animated
/// presets (1-31) and switchable styles (32+), each style's flat block
/// and, for bumped faces, its directional pages.
pub fn face_extra_styles(lump: &[u8], face: &vbsp::Face, bumped: bool) -> Vec<StyleSamples> {
    let per_style = if bumped { 4 } else { 1 };
    face.styles
        .iter()
        .enumerate()
        .take_while(|(_, s)| **s != 255)
        .filter(|(_, s)| **s != 0)
        .filter_map(|(k, &style)| {
            let k = k as u32 * per_style;
            let flat = block(lump, face, k)?;
            let pages = if bumped {
                Some([
                    block(lump, face, k + 1)?,
                    block(lump, face, k + 2)?,
                    block(lump, face, k + 3)?,
                ])
            } else {
                None
            };
            Some(StyleSamples { style, flat, pages })
        })
        .collect()
}

/// One light style's samples on one face.
pub struct StyleSamples {
    pub style: u8,
    pub flat: FaceSamples,
    pub pages: Option<[FaceSamples; 3]>,
}

fn block(lump: &[u8], face: &vbsp::Face, index: u32) -> Option<FaceSamples> {
    if face.light_offset < 0 || face.styles[0] == 255 {
        return None;
    }
    let width = (face.light_map_texture_size[0] + 1) as u32;
    let height = (face.light_map_texture_size[1] + 1) as u32;
    let start = face.light_offset as usize + (index * width * height * 4) as usize;
    let bytes = lump.get(start..start + (width * height * 4) as usize)?;
    let rgb = bytes
        .as_chunks::<4>()
        .0
        .iter()
        .map(|&[r, g, b, e]| {
            let scale = 2f32.powi(e as i8 as i32) / 255.0;
            [r as f32 * scale, g as f32 * scale, b as f32 * scale]
        })
        .collect();
    Some(FaceSamples { width, height, rgb })
}

/// The animated light style presets (`light` entities' "appearance"
/// 1-12, as the Valve Developer Wiki's light entity page lists them):
/// one letter a step, 'a' dark, 'm' as baked, 'z' about twice that.
pub const STYLE_PRESETS: [&str; 13] = [
    "m",
    "mmnmmommommnonmmonqnmmo",
    "abcdefghijklmnopqrstuvwxyzyxwvutsrqponmlkjihgfedcba",
    "mmmmmaaaaammmmmaaaaaabcdefgabcdefg",
    "mamamamamama",
    "jklmnopqrstuvwxyzyxwvutsrqponmlkj",
    "nmonqnmomnmomomno",
    "mmmaaaabcdefgmmmmaaaammmaamm",
    "mmmaaammmaaammmabcdefaaaammmmabcdefmmmaaaa",
    "aaaaaaaazzzzzzzz",
    "mmamammmmammamamaaamammma",
    "abcdefghijklmnopqrrqponmlkjihgfedcba",
    "mmnnmmnnnmmnn",
];

/// An animated style's brightness per step: the light's own `pattern`
/// when it has one, else the style's preset; styles 13-31 without one
/// stay as baked (no preset is documented for them).
pub fn style_pattern(style: u8, custom: Option<&str>) -> Vec<f32> {
    let letters = custom
        .map(str::trim)
        .filter(|p| !p.is_empty())
        .or_else(|| STYLE_PRESETS.get(style as usize).copied())
        .unwrap_or("m");
    letters
        .bytes()
        .map(|c| c.to_ascii_lowercase().clamp(b'a', b'z'))
        .map(|c| (c - b'a') as f32 / (b'm' - b'a') as f32)
        .collect()
}

/// Lightmap sample coordinates (in luxels, sample centers at integers) of a
/// point on the face's plane.
pub fn luxel_coords(texinfo: &vbsp::TextureInfo, face: &vbsp::Face, p: vbsp::Vector) -> Vec2 {
    let s = texinfo.light_map_scale;
    let t = texinfo.light_map_transform;
    Vec2::new(
        s[0] * p.x + s[1] * p.y + s[2] * p.z + s[3] - face.light_map_texture_min[0] as f32,
        t[0] * p.x + t[1] * p.y + t[2] * p.z + t[3] - face.light_map_texture_min[1] as f32,
    )
}

/// Shelf-packs face blocks into one atlas.
#[derive(Default)]
pub struct AtlasBuilder {
    blocks: Vec<FaceSamples>,
    /// Directional lightmaps per block, when bump-mapped.
    bumped: Vec<Option<[FaceSamples; 3]>>,
    /// Animated and switchable styles per block, and whether each is lit
    /// at start.
    styles: Vec<Vec<(StyleSamples, bool)>>,
}

/// Where a block landed: top-left of its samples, in luxels.
#[derive(Clone, Copy)]
pub struct Placement {
    pub origin: UVec2,
    pub size: UVec2,
}

impl AtlasBuilder {
    /// Add a block; returns its slot.
    pub fn add(&mut self, samples: FaceSamples) -> usize {
        self.add_bumped(samples, None)
    }

    pub fn add_bumped(&mut self, samples: FaceSamples, bumped: Option<[FaceSamples; 3]>) -> usize {
        self.blocks.push(samples);
        self.bumped.push(bumped);
        self.styles.push(Vec::new());
        self.blocks.len() - 1
    }

    /// A block's animated and switchable styles (`face_extra_styles`), lit
    /// at map start or not.
    pub fn set_styles(&mut self, slot: usize, styles: Vec<(StyleSamples, bool)>) {
        self.styles[slot] = styles;
    }

    /// Pack everything. Also returns a slot holding a single white (1.0)
    /// block for drawn faces without lighting.
    pub fn build(mut self) -> (MapLightmap, Vec<Placement>, usize) {
        let white = self.add(FaceSamples {
            width: 2,
            height: 2,
            rgb: vec![[1.0; 3]; 4],
        });
        let mut order: Vec<usize> = (0..self.blocks.len()).collect();
        order.sort_by_key(|&i| std::cmp::Reverse(self.blocks[i].height));

        let mut placements = vec![
            Placement {
                origin: UVec2::ZERO,
                size: UVec2::ZERO
            };
            self.blocks.len()
        ];
        let (mut x, mut y, mut shelf) = (0u32, 0u32, 0u32);
        for &i in &order {
            let b = &self.blocks[i];
            let (w, h) = (b.width + 2 * PAD, b.height + 2 * PAD);
            if x + w > ATLAS_WIDTH {
                x = 0;
                y += shelf;
                shelf = 0;
            }
            placements[i] = Placement {
                origin: UVec2::new(x + PAD, y + PAD),
                size: UVec2::new(b.width, b.height),
            };
            x += w;
            shelf = shelf.max(h);
        }
        let height = (y + shelf).max(1);

        let any_bumped = self.bumped.iter().any(Option::is_some);
        let blank = || vec![[0.0f32; 3]; (ATLAS_WIDTH * height) as usize];
        let mut rgb = blank();
        let mut bumped = any_bumped.then(|| [blank(), blank(), blank()]);
        // Copy samples, extending edge samples into the padding.
        let copy = |dest: &mut Vec<[f32; 3]>, b: &FaceSamples, p: &Placement| {
            for py in 0..b.height + 2 * PAD {
                for px in 0..b.width + 2 * PAD {
                    let sx = (px as i32 - PAD as i32).clamp(0, b.width as i32 - 1) as u32;
                    let sy = (py as i32 - PAD as i32).clamp(0, b.height as i32 - 1) as u32;
                    let ax = p.origin.x - PAD + px;
                    let ay = p.origin.y - PAD + py;
                    dest[(ay * ATLAS_WIDTH + ax) as usize] = b.rgb[(sy * b.width + sx) as usize];
                }
            }
        };
        for (i, (b, p)) in self.blocks.iter().zip(&placements).enumerate() {
            copy(&mut rgb, b, p);
            if let Some(layers) = bumped.as_mut() {
                for (k, layer) in layers.iter_mut().enumerate() {
                    // Unbumped faces show their flat lighting in every layer.
                    let src = self.bumped[i].as_ref().map(|d| &d[k]).unwrap_or(b);
                    copy(layer, src, p);
                }
            }
        }
        // Each style's share, texel by texel (padding included), block by
        // block.
        let mut styles: Vec<MapLightStyle> = Vec::new();
        for (i, list) in self.styles.iter().enumerate() {
            let p = &placements[i];
            for (st, on) in list {
                let k = match styles.iter().position(|s| s.style == st.style) {
                    Some(k) => k,
                    None => {
                        styles.push(MapLightStyle {
                            style: st.style,
                            on: *on,
                            bumped: any_bumped.then(|| [Vec::new(), Vec::new(), Vec::new()]),
                            ..default()
                        });
                        styles.len() - 1
                    }
                };
                let s = &mut styles[k];
                let b = &st.flat;
                s.rects.push([
                    p.origin.x - PAD,
                    p.origin.y - PAD,
                    b.width + 2 * PAD,
                    b.height + 2 * PAD,
                ]);
                for py in 0..b.height + 2 * PAD {
                    for px in 0..b.width + 2 * PAD {
                        let sx = (px as i32 - PAD as i32).clamp(0, b.width as i32 - 1) as u32;
                        let sy = (py as i32 - PAD as i32).clamp(0, b.height as i32 - 1) as u32;
                        let src = (sy * b.width + sx) as usize;
                        let ax = p.origin.x - PAD + px;
                        let ay = p.origin.y - PAD + py;
                        s.texels.push(ay * ATLAS_WIDTH + ax);
                        s.rgb.push(b.rgb[src]);
                        if let Some(pages) = s.bumped.as_mut() {
                            for (k, page) in pages.iter_mut().enumerate() {
                                // Unbumped faces show their flat lighting in every page.
                                let v = st.pages.as_ref().map_or(b.rgb[src], |d| d[k].rgb[src]);
                                page.push(v);
                            }
                        }
                    }
                }
            }
        }
        (
            MapLightmap {
                width: ATLAS_WIDTH,
                height,
                rgb,
                bumped,
                styles,
            },
            placements,
            white,
        )
    }
}

/// Normalized atlas UV for a luxel coordinate within a placed block.
pub fn atlas_uv(atlas: &MapLightmap, p: Placement, luxel: Vec2) -> [f32; 2] {
    // Clamp to the block's sample centers so filtering stays inside it.
    let max = (p.size.as_vec2() - 1.0).max(Vec2::ZERO);
    let c = luxel.clamp(Vec2::ZERO, max) + p.origin.as_vec2() + 0.5;
    [c.x / atlas.width as f32, c.y / atlas.height as f32]
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A BSP v20 file holding only the given lumps (index, bytes).
    fn bsp_with(lumps: &[(usize, Vec<u8>)]) -> Vec<u8> {
        let header = 8 + 64 * 16 + 4;
        let mut bytes = vec![0u8; header];
        bytes[0..4].copy_from_slice(b"VBSP");
        bytes[4..8].copy_from_slice(&20i32.to_le_bytes());
        for (i, data) in lumps {
            let at = 8 + i * 16;
            let ofs = bytes.len() as i32;
            bytes[at..at + 4].copy_from_slice(&ofs.to_le_bytes());
            bytes[at + 4..at + 8].copy_from_slice(&(data.len() as i32).to_le_bytes());
            bytes.extend_from_slice(data);
        }
        bytes
    }

    fn face_bytes(light_ofs: i32) -> Vec<u8> {
        let mut f = vec![0u8; FACE_SIZE];
        f[FACE_LIGHT_OFS..FACE_LIGHT_OFS + 4].copy_from_slice(&light_ofs.to_le_bytes());
        f
    }

    fn face(light_offset: i32, size: [i32; 2]) -> vbsp::Face {
        vbsp::Face {
            plane_num: 0,
            side: 0,
            on_node: 0,
            first_edge: 0,
            num_edges: 0,
            texture_info: 0,
            displacement_info: -1,
            surface_fog_volume_id: -1,
            styles: [0, 255, 255, 255],
            light_offset,
            area: 0.0,
            light_map_texture_min: [0, 0],
            light_map_texture_size: size,
            original_face: -1,
            primitive_count: 0,
            first_primitive_index: 0,
            smoothing_groups: 0,
        }
    }

    #[test]
    fn style_patterns_follow_the_presets() {
        // 'm' is as baked, 'a' dark, 'z' 25/12.
        assert_eq!(style_pattern(0, None), vec![1.0]);
        let strobe = style_pattern(4, None);
        assert_eq!(&strobe[..2], &[1.0, 0.0]);
        assert_eq!(style_pattern(9, None)[8], 25.0 / 12.0);
        assert_eq!(style_pattern(20, None), vec![1.0], "no preset");
        assert_eq!(style_pattern(1, Some("aZ")), vec![0.0, 25.0 / 12.0], "custom wins");
        assert_eq!(style_pattern(1, None).len(), 23);
    }

    #[test]
    fn hdr_lighting_is_the_hdr_lump_when_faces_agree() {
        let ldr = vec![1u8; 8];
        let hdr = vec![2u8; 8];
        // No HDR face lump: the faces' offsets index both.
        let b = bsp_with(&[(LUMP_LIGHTING, ldr.clone()), (LUMP_LIGHTING_HDR, hdr.clone())]);
        assert_eq!(hdr_lighting_lump(&b), Some(&hdr[..]));
        assert_eq!(lighting_lump(&b), &ldr[..], "LDR stays the default");
        // An HDR face lump with the same offsets.
        let faces = [face_bytes(0), face_bytes(4)].concat();
        let b = bsp_with(&[
            (LUMP_FACES, faces.clone()),
            (LUMP_LIGHTING, ldr.clone()),
            (LUMP_LIGHTING_HDR, hdr.clone()),
            (LUMP_FACES_HDR, faces.clone()),
        ]);
        assert_eq!(hdr_lighting_lump(&b), Some(&hdr[..]));
        // Different offsets: not usable with lump 7's faces.
        let other = [face_bytes(4), face_bytes(0)].concat();
        let b = bsp_with(&[(LUMP_FACES, faces), (LUMP_LIGHTING_HDR, hdr), (LUMP_FACES_HDR, other)]);
        assert_eq!(hdr_lighting_lump(&b), None);
        // No HDR lump at all.
        assert_eq!(hdr_lighting_lump(&bsp_with(&[(LUMP_LIGHTING, ldr)])), None);
    }

    #[test]
    fn hdr_luxels_decode_linear_and_unclamped() {
        // ColorRGBExp32: c * 2^e / 255, the same decode as LDR; HDR values
        // past the LDR encoding's ~4.0 ceiling come through as they are.
        let lump = [
            255u8,
            128,
            0,
            3, // (8, 4.016, 0)
            51,
            102,
            204,
            (-2i8) as u8, // (0.05, 0.1, 0.2)
            0,
            0,
            0,
            0,
            255,
            255,
            255,
            0,
        ];
        let s = face_samples(&lump, &face(0, [1, 1])).unwrap();
        assert_eq!((s.width, s.height), (2, 2));
        let close = |a: [f32; 3], b: [f32; 3]| a.iter().zip(b).all(|(x, y)| (x - y).abs() < 1e-4);
        assert!(close(s.rgb[0], [8.0, 128.0 * 8.0 / 255.0, 0.0]), "{:?}", s.rgb[0]);
        assert!(close(s.rgb[1], [0.05, 0.1, 0.2]), "{:?}", s.rgb[1]);
        assert!(close(s.rgb[3], [1.0, 1.0, 1.0]), "{:?}", s.rgb[3]);
    }
}
