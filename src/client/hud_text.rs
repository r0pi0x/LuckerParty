//! Text drawn the way Source's HUD draws it: the game's font glyphs
//! rasterized here, placed by Source's rules, and blended additively for
//! the scheme's `additive` fonts (the HUD numbers and icon fonts) or
//! alpha blended for the others. Bevy's own text can't do either: it lays
//! a line out by its own metrics and only alpha blends.
//!
//! Source's rules, as measured against CS:S captures (`refcmp hudcmp`;
//! docs/OBSERVABILITY.md): a font's `tall` is its cell height in pixels,
//! the em is that over the face's Windows line height (OS/2 `usWinAscent +
//! usWinDescent`), and the baseline sits `tall · ascender / (ascender −
//! descender)` (hhea) below the cell's top; glyphs advance by their
//! advance widths rounded to whole pixels, and the cell starts on a whole
//! pixel. Coverage is drawn as is: additive text adds `colour · alpha ·
//! coverage` to what is under it.

use std::{
    collections::HashMap,
    hash::{Hash, Hasher},
    sync::Arc,
};

use ab_glyph::{Font as _, FontRef, PxScale, point};
use bevy::{
    asset::{RenderAssetUsages, embedded_asset},
    prelude::*,
    reflect::TypePath,
    render::render_resource::{
        AsBindGroup, BlendComponent, BlendFactor, BlendOperation, BlendState, Extent3d, RenderPipelineDescriptor,
        TextureDimension, TextureFormat,
    },
    shader::ShaderRef,
    ui_render::ui_material::UiMaterialKey,
};

pub struct HudTextPlugin;

impl Plugin for HudTextPlugin {
    fn build(&self, app: &mut App) {
        embedded_asset!(app, "hud_text.wgsl");
        app.add_plugins(UiMaterialPlugin::<HudGlyphMaterial>::default())
            .init_resource::<GlyphCache>()
            .add_systems(PostUpdate, draw.before(bevy::ui::UiSystems::Prepare));
    }
}

/// A font's bytes and how Source blends it.
#[derive(Clone, Debug)]
pub struct GlyphFont {
    pub data: Arc<Vec<u8>>,
    /// The scheme's `additive 1`.
    pub additive: bool,
}

impl PartialEq for GlyphFont {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.data, &other.data) && self.additive == other.additive
    }
}

/// Which point of the text `HudText::at` is.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Align {
    /// The cell's top-left corner.
    #[default]
    Left,
    /// The top of the advance's middle.
    Centre,
    /// The top-right corner (the end of the last advance).
    Right,
    /// The top of the ink's right edge (the pickup history's icons line up
    /// by their ink).
    InkRight,
}

/// One line of HUD text: what, in which font and colour, where (window
/// pixels, logical). Its entity also needs a `Node` (absolute) and gets a
/// `MaterialNode<HudGlyphMaterial>`.
#[derive(Component, Clone, Debug, PartialEq)]
pub struct HudText {
    pub font: GlyphFont,
    /// Cell height, window pixels.
    pub tall: f32,
    pub text: String,
    pub color: Color,
    pub at: Vec2,
    pub align: Align,
}

/// A face's numbers for Source's layout (font units).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FaceMetrics {
    pub units_per_em: f32,
    /// OS/2 usWinAscent + usWinDescent (hhea's span without OS/2).
    pub win_line: f32,
    /// hhea ascender and descender (descender negative).
    pub ascender: f32,
    pub descender: f32,
}

impl FaceMetrics {
    pub fn of(data: &[u8]) -> Option<Self> {
        let font = FontRef::try_from_slice(data).ok()?;
        let units_per_em = font.units_per_em()?;
        let (ascender, descender) = hhea(data).unwrap_or((font.ascent_unscaled(), font.descent_unscaled()));
        let win_line = super::fonts::line_per_em(data).map_or(ascender - descender, |l| l * units_per_em);
        Some(Self {
            units_per_em,
            win_line,
            ascender,
            descender,
        })
    }

    /// Em size in pixels for a cell `tall` pixels high: whole pixels
    /// (rounded down), as the game sizes its fonts.
    pub fn em(&self, tall: f32) -> f32 {
        (tall * self.units_per_em / self.win_line).floor().max(1.0)
    }

    /// The baseline's distance below the cell's top.
    pub fn baseline(&self, tall: f32) -> f32 {
        tall * self.ascender / (self.ascender - self.descender)
    }
}

/// hhea ascender and descender (font units).
fn hhea(ttf: &[u8]) -> Option<(f32, f32)> {
    let u16_at = |o: usize| ttf.get(o..o + 2).map(|b| u16::from_be_bytes([b[0], b[1]]));
    let u32_at = |o: usize| ttf.get(o..o + 4).map(|b| u32::from_be_bytes([b[0], b[1], b[2], b[3]]));
    let tables = u16_at(4)? as usize;
    let at = (0..tables)
        .map(|i| 12 + i * 16)
        .find(|&o| ttf.get(o..o + 4) == Some(b"hhea".as_slice()))
        .and_then(|o| u32_at(o + 8))? as usize;
    let a = u16_at(at + 4)? as i16 as f32;
    let d = u16_at(at + 6)? as i16 as f32;
    (a > d).then_some((a, d))
}

/// Text laid out in pixels: each glyph's pen x (cell-relative) and the
/// total advance, all whole pixels.
pub fn advances(data: &[u8], tall: f32, text: &str) -> (Vec<f32>, f32) {
    let (Ok(font), Some(m)) = (FontRef::try_from_slice(data), FaceMetrics::of(data)) else {
        return (Vec::new(), 0.0);
    };
    let k = m.em(tall) / m.units_per_em;
    let mut pen = 0.0;
    let mut out = Vec::new();
    for c in text.chars() {
        out.push(pen);
        pen += (font.h_advance_unscaled(font.glyph_id(c)) * k).round();
    }
    (out, pen)
}

/// The width of `text` (whole pixels).
pub fn width(data: &[u8], tall: f32, text: &str) -> f32 {
    advances(data, tall, text).1
}

/// Coverage of `text` drawn with its cell's top-left at (`pad`, `pad`):
/// width, height, one byte per pixel.
pub struct Raster {
    pub width: u32,
    pub height: u32,
    pub coverage: Vec<u8>,
    /// The cell's top-left inside the raster.
    pub pad: u32,
    pub advance: f32,
    /// The ink's right edge from the cell's left (coverage of half or more).
    pub ink_right: f32,
}

pub fn rasterize(data: &[u8], tall: f32, text: &str) -> Option<Raster> {
    let font = FontRef::try_from_slice(data).ok()?;
    let m = FaceMetrics::of(data)?;
    let em = m.em(tall);
    let scale = PxScale::from(em * font.height_unscaled() / m.units_per_em);
    let (pens, advance) = advances(data, tall, text);
    // Room for glyphs that reach past their cell (italic overhang,
    // icon fonts' wide glyphs).
    let pad = (tall * 0.5).ceil().max(2.0) as u32;
    let (w, h) = (advance.ceil() as u32 + 2 * pad, tall.ceil() as u32 + 2 * pad);
    let mut coverage = vec![0u8; (w * h) as usize];
    let base = pad as f32 + m.baseline(tall).round();
    for (c, pen) in text.chars().zip(pens) {
        let glyph = font
            .glyph_id(c)
            .with_scale_and_position(scale, point(pad as f32 + pen, base));
        let Some(outlined) = font.outline_glyph(glyph) else {
            continue;
        };
        let b = outlined.px_bounds();
        outlined.draw(|x, y, v| {
            let (px, py) = (b.min.x as i32 + x as i32, b.min.y as i32 + y as i32);
            if px >= 0 && py >= 0 && (px as u32) < w && (py as u32) < h {
                let i = (py as u32 * w + px as u32) as usize;
                let v = (v.clamp(0.0, 1.0) * 255.0).round() as u8;
                coverage[i] = coverage[i].saturating_add(v);
            }
        });
    }
    let ink_right = (0..w)
        .rev()
        .find(|x| (0..h).any(|y| coverage[(y * w + x) as usize] >= 128))
        .map_or(advance, |x| (x + 1) as f32 - pad as f32);
    Some(Raster {
        width: w,
        height: h,
        coverage,
        pad,
        advance,
        ink_right,
    })
}

/// The cell's top-left corner (whole pixels) of text `advance` wide (its
/// ink ending `ink_right` from the cell's left) whose `align` point is at
/// `at`.
pub fn cell_corner(at: Vec2, align: Align, advance: f32, ink_right: f32) -> Vec2 {
    let left = match align {
        Align::Left => at.x,
        Align::Centre => at.x - (advance / 2.0).floor(),
        Align::Right => at.x - advance,
        Align::InkRight => at.x - ink_right,
    };
    Vec2::new(left.floor(), at.y.floor())
}

/// Where a text's ink lands on the screen (pixels whose coverage is at
/// least half), at scale factor 1; None when it draws nothing.
pub fn ink_rect(text: &HudText) -> Option<Rect> {
    let tall = text.tall.round();
    let r = rasterize(&text.font.data, tall, &text.text)?;
    let corner = cell_corner(text.at, text.align, r.advance, r.ink_right) - Vec2::splat(r.pad as f32);
    let mut b: Option<Rect> = None;
    for y in 0..r.height {
        for x in 0..r.width {
            if r.coverage[(y * r.width + x) as usize] >= 128 {
                let p = corner + Vec2::new(x as f32, y as f32);
                let px = Rect::from_corners(p, p + Vec2::ONE);
                b = Some(b.map_or(px, |b| b.union(px)));
            }
        }
    }
    b
}

/// A HUD font file's cell height in a window `height` pixels tall: its
/// 480-line height scaled, in whole pixels (VGUI's proportional sizes).
pub fn proportional_tall(tall: f32, height: f32) -> f32 {
    (tall * height / 480.0).trunc()
}

/// One of the game's HUD font files (`GameHud::fonts`) by scheme name, as
/// drawn in a window `height` pixels tall.
pub fn game_font(hud: &crate::map::hud::GameHud, name: &str, height: f32) -> Option<(GlyphFont, f32)> {
    let f = hud.fonts.get(name)?;
    Some((
        GlyphFont {
            data: f.data.clone(),
            additive: f.additive,
        },
        proportional_tall(f.tall, height),
    ))
}

/// A glyph image drawn in a colour, added to or blended over the screen.
#[derive(Asset, TypePath, AsBindGroup, Debug, Clone)]
#[bind_group_data(HudGlyphKey)]
pub struct HudGlyphMaterial {
    #[uniform(0)]
    pub params: GlyphParams,
    #[texture(1)]
    #[sampler(2)]
    pub coverage: Handle<Image>,
    pub additive: bool,
}

/// The tint, and the part of the image drawn (uv offset and size; a
/// sprite's rectangle of its sheet).
#[derive(Clone, Copy, Debug, PartialEq, bevy::render::render_resource::ShaderType)]
pub struct GlyphParams {
    pub color: LinearRgba,
    pub uv: Vec4,
}

impl GlyphParams {
    /// The whole image in `color`.
    pub fn whole(color: Color) -> Self {
        Self {
            color: color.to_linear(),
            uv: Vec4::new(0.0, 0.0, 1.0, 1.0),
        }
    }
}

#[repr(C)]
#[derive(Eq, PartialEq, Hash, Copy, Clone)]
pub struct HudGlyphKey {
    additive: bool,
}

impl From<&HudGlyphMaterial> for HudGlyphKey {
    fn from(m: &HudGlyphMaterial) -> Self {
        Self { additive: m.additive }
    }
}

impl UiMaterial for HudGlyphMaterial {
    fn fragment_shader() -> ShaderRef {
        "embedded://mashup/client/hud_text.wgsl".into()
    }

    fn specialize(descriptor: &mut RenderPipelineDescriptor, key: UiMaterialKey<Self>) {
        if key.bind_group_data.additive
            && let Some(fragment) = descriptor.fragment.as_mut()
        {
            for target in fragment.targets.iter_mut().flatten() {
                target.blend = Some(BlendState {
                    color: BlendComponent {
                        src_factor: BlendFactor::SrcAlpha,
                        dst_factor: BlendFactor::One,
                        operation: BlendOperation::Add,
                    },
                    alpha: BlendComponent {
                        src_factor: BlendFactor::Zero,
                        dst_factor: BlendFactor::One,
                        operation: BlendOperation::Add,
                    },
                });
            }
        }
    }
}

/// Coverage images by font, size and text, so a clock ticking or a
/// panel redrawn doesn't rasterize again.
#[derive(Resource, Default)]
struct GlyphCache(HashMap<u64, (Handle<Image>, u32, f32, f32)>);

fn key(font: &GlyphFont, tall: f32, text: &str) -> u64 {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    (Arc::as_ptr(&font.data) as usize).hash(&mut h);
    tall.to_bits().hash(&mut h);
    text.hash(&mut h);
    h.finish()
}

/// Rasterize changed texts at the window's physical resolution and place
/// their nodes on whole physical pixels.
#[allow(clippy::type_complexity)]
fn draw(
    windows: Query<&Window>,
    mut texts: Query<(Entity, Ref<HudText>, &mut Node, Option<&MaterialNode<HudGlyphMaterial>>)>,
    mut cache: ResMut<GlyphCache>,
    mut images: ResMut<Assets<Image>>,
    mut materials: ResMut<Assets<HudGlyphMaterial>>,
    mut last_scale: Local<f32>,
    mut commands: Commands,
) {
    let sf = windows.iter().next().map_or(1.0, |w| w.scale_factor());
    let rescaled = *last_scale != sf;
    *last_scale = sf;
    if cache.0.len() > 512 {
        cache.0.clear();
    }
    for (entity, text, mut node, material) in &mut texts {
        if !text.is_changed() && !rescaled && material.is_some() {
            continue;
        }
        let tall = (text.tall * sf).round();
        let k = key(&text.font, tall, &text.text);
        let (image, pad, advance, ink_right) = match cache.0.get(&k) {
            Some(c) => c.clone(),
            None => {
                let Some(r) = rasterize(&text.font.data, tall, &text.text) else {
                    continue;
                };
                let mut data = Vec::with_capacity(r.coverage.len() * 4);
                for c in &r.coverage {
                    data.extend_from_slice(&[255, 255, 255, *c]);
                }
                let mut image = Image::new(
                    Extent3d {
                        width: r.width,
                        height: r.height,
                        depth_or_array_layers: 1,
                    },
                    TextureDimension::D2,
                    data,
                    TextureFormat::Rgba8Unorm,
                    RenderAssetUsages::RENDER_WORLD | RenderAssetUsages::MAIN_WORLD,
                );
                // Drawn pixel for pixel.
                image.sampler = bevy::image::ImageSampler::nearest();
                let image = images.add(image);
                let entry = (image, r.pad, r.advance, r.ink_right);
                cache.0.insert(k, entry.clone());
                entry
            }
        };
        let size = images.get(&image).map_or(UVec2::ZERO, |i| i.size());
        // The cell's corner on a whole physical pixel.
        let Vec2 { x: left, y: top } = cell_corner(text.at * sf, text.align, advance, ink_right);
        node.position_type = PositionType::Absolute;
        node.left = px((left - pad as f32) / sf);
        node.top = px((top - pad as f32) / sf);
        node.width = px(size.x as f32 / sf);
        node.height = px(size.y as f32 / sf);
        let want = HudGlyphMaterial {
            params: GlyphParams::whole(text.color),
            coverage: image,
            additive: text.font.additive,
        };
        let same = material
            .and_then(|m| materials.get(&m.0))
            .is_some_and(|m| m.coverage == want.coverage && m.params == want.params && m.additive == want.additive);
        if !same {
            commands.entity(entity).insert(MaterialNode(materials.add(want)));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A font with one square glyph, for layout without a real face.
    fn font_file() -> Option<Vec<u8>> {
        // DejaVu or Liberation, where installed (Linux CI and dev boxes);
        // skipped elsewhere.
        [
            "/usr/share/fonts/TTF/DejaVuSans.ttf",
            "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf",
            "/usr/share/fonts/liberation/LiberationSans-Regular.ttf",
            "C:\\Windows\\Fonts\\verdana.ttf",
        ]
        .iter()
        .find_map(|p| std::fs::read(p).ok())
    }

    #[test]
    fn source_cell_metrics() {
        // Cstrike.ttf's numbers: em 1000, Windows line 985 + 144, hhea
        // 985 / -226; HudNumbers is 28 tall at 480 lines, 63 at 1080.
        let m = FaceMetrics {
            units_per_em: 1000.0,
            win_line: 1129.0,
            ascender: 985.0,
            descender: -226.0,
        };
        assert_eq!(m.em(63.0), 55.0);
        assert!((m.baseline(63.0) - 51.24).abs() < 0.01);
    }

    #[test]
    fn rasterized_text_sits_on_whole_pixel_advances() {
        let Some(data) = font_file() else { return };
        let (pens, total) = advances(&data, 20.0, "100");
        assert_eq!(pens.len(), 3);
        assert!(pens.iter().all(|p| p.fract() == 0.0) && total.fract() == 0.0);
        assert_eq!(pens[1] - pens[0], pens[2] - pens[1], "same digit, same advance");
        let r = rasterize(&data, 20.0, "1").unwrap();
        assert!(r.coverage.iter().any(|c| *c > 200), "ink drawn");
        assert_eq!(r.height, 20 + 2 * r.pad);
    }
}
