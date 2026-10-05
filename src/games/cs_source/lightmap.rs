//! Baked lighting from BSP v20, per the public format description
//! (developer.valvesoftware.com/wiki/BSP_(Source), "Lighting"): each face
//! with lighting stores (size+1)^2 samples as RGB + shared signed exponent,
//! starting at the face's byte offset into the lighting lump. Faces' samples
//! are packed into one atlas.

use bevy::prelude::*;

use crate::map::MapLightmap;

const LUMP_LIGHTING: usize = 8;
const LUMP_LIGHTING_HDR: usize = 53;
/// Empty luxels around each face's block so filtering doesn't bleed.
const PAD: u32 = 1;
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
    if face.light_offset < 0 || face.styles[0] == 255 {
        return None;
    }
    let width = (face.light_map_texture_size[0] + 1) as u32;
    let height = (face.light_map_texture_size[1] + 1) as u32;
    let start = face.light_offset as usize;
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
        self.blocks.push(samples);
        self.blocks.len() - 1
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

        let mut rgb = vec![[0.0f32; 3]; (ATLAS_WIDTH * height) as usize];
        for (b, p) in self.blocks.iter().zip(&placements) {
            // Copy samples, extending edge samples into the padding.
            for py in 0..b.height + 2 * PAD {
                for px in 0..b.width + 2 * PAD {
                    let sx = (px as i32 - PAD as i32).clamp(0, b.width as i32 - 1) as u32;
                    let sy = (py as i32 - PAD as i32).clamp(0, b.height as i32 - 1) as u32;
                    let ax = p.origin.x - PAD + px;
                    let ay = p.origin.y - PAD + py;
                    rgb[(ay * ATLAS_WIDTH + ax) as usize] = b.rgb[(sy * b.width + sx) as usize];
                }
            }
        }
        (
            MapLightmap {
                width: ATLAS_WIDTH,
                height,
                rgb,
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
