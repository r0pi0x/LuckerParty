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

/// The raw lighting lump: LDR if present, else HDR.
pub fn lighting_lump(bsp_bytes: &[u8]) -> &[u8] {
    let lump = |i: usize| -> &[u8] {
        let at = 8 + i * 16;
        let Some(entry) = bsp_bytes.get(at..at + 8) else {
            return &[];
        };
        let ofs = i32::from_le_bytes(entry[0..4].try_into().unwrap()).max(0) as usize;
        let len = i32::from_le_bytes(entry[4..8].try_into().unwrap()).max(0) as usize;
        bsp_bytes.get(ofs..ofs + len).unwrap_or(&[])
    };
    let ldr = lump(LUMP_LIGHTING);
    if ldr.is_empty() { lump(LUMP_LIGHTING_HDR) } else { ldr }
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

/// A face's switchable light styles (32+) apart: each style's flat block
/// and, for bumped faces, its directional pages.
pub fn face_switchable_styles(lump: &[u8], face: &vbsp::Face, bumped: bool) -> Vec<StyleSamples> {
    let per_style = if bumped { 4 } else { 1 };
    face.styles
        .iter()
        .enumerate()
        .take_while(|(_, s)| **s != 255)
        .filter(|(_, s)| **s >= 32)
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

/// One switchable style's samples on one face.
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
    /// Switchable styles per block, and whether each is lit at start.
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

    /// A block's switchable styles (`face_switchable_styles`), lit at map
    /// start or not.
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
        // Each switchable style's share, texel by texel (padding included).
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
