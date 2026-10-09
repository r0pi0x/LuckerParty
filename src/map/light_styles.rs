//! Light styles at run time: switchable styles (Source 32+, lights'
//! TurnOn/TurnOff through `LightStyles`) rebuild the lightmap images; the
//! animated ones (Source 1-31: flicker, pulse, strobe presets, or a
//! light's own pattern) step through their pattern ten times a second and
//! rewrite only the face blocks they light, straight into the GPU textures
//! (`LightmapUploads`: one buffer upload a step, copied block by block on
//! the GPU), so a flickering light costs its blocks a step instead of
//! whole atlases.

use std::{collections::HashMap, sync::Arc};

use bevy::{
    prelude::*,
    render::{
        Render, RenderApp, RenderSystems,
        extract_plugin::{ExtractSchedule, MainWorld},
        render_asset::RenderAssets,
        render_resource::{
            Buffer, BufferDescriptor, BufferUsages, CommandEncoderDescriptor, Extent3d, Origin3d, TexelCopyBufferInfo,
            TexelCopyBufferLayout, TexelCopyTextureInfo, TextureAspect,
        },
        renderer::{RenderDevice, RenderQueue},
        texture::GpuImage,
    },
};

use super::{
    LightStyles, LightmapLayers, MapLightStyle, MapLightmap, source_ldr_bump_texels, source_ldr_texel, world_material,
};

/// A face block's atlas rectangle and the styles lighting it (style
/// index, offset of the block in that style's texels).
type Block = ([u32; 4], Vec<(usize, usize)>);

/// The lightmap images of a map whose light styles change, and the
/// brightness each style shows in them now.
#[derive(Resource)]
pub(super) struct StyledLightmaps {
    pub atlas: Arc<MapLightmap>,
    pub source_ldr: bool,
    pub plain: Option<Handle<Image>>,
    pub world: Option<Handle<Image>>,
    pub bumped: Option<[Handle<Image>; 3]>,
    /// Each of the atlas's styles' brightness in the images now.
    levels: Vec<f32>,
    /// The face blocks animated styles light: each block's rectangle and
    /// every style lighting it (index into the atlas's styles, offset of
    /// the block in that style's texels).
    blocks: Vec<Block>,
    /// Per style: the `blocks` it lights (animated styles only).
    style_blocks: Vec<Vec<usize>>,
    /// Encoded texels of the states each block has shown (the levels of
    /// the styles lighting it), every layer's in turn: patterns loop
    /// through a few levels, so after their first loop a step costs
    /// copies, not relighting (which took ~0.1-0.2 G instructions a step
    /// on mg_jacks_multigames_v1).
    states: Vec<Vec<(Vec<f32>, Arc<[u8]>)>>,
    /// Bytes in `states`, kept under `STATE_CACHE_BYTES`.
    state_bytes: usize,
}

/// At most this many bytes of encoded block states are kept per map;
/// states beyond it are worked out each time they are shown.
const STATE_CACHE_BYTES: usize = 128 << 20;

impl StyledLightmaps {
    pub fn new(
        atlas: Arc<MapLightmap>,
        source_ldr: bool,
        plain: Option<Handle<Image>>,
        world: Option<Handle<Image>>,
        bumped: Option<[Handle<Image>; 3]>,
    ) -> Self {
        let levels = atlas.styles.iter().map(MapLightStyle::start).collect();
        // Blocks animated styles light, with every style sharing them.
        let mut index: HashMap<[u32; 4], usize> = HashMap::new();
        let mut blocks: Vec<Block> = Vec::new();
        for s in atlas.styles.iter().filter(|s| !s.pattern.is_empty()) {
            for r in &s.rects {
                index.entry(*r).or_insert_with(|| {
                    blocks.push((*r, Vec::new()));
                    blocks.len() - 1
                });
            }
        }
        let mut style_blocks = vec![Vec::new(); atlas.styles.len()];
        for (k, s) in atlas.styles.iter().enumerate() {
            let mut offset = 0;
            for r in &s.rects {
                if let Some(&b) = index.get(r) {
                    blocks[b].1.push((k, offset));
                    if !s.pattern.is_empty() {
                        style_blocks[k].push(b);
                    }
                }
                offset += (r[2] * r[3]) as usize;
            }
        }
        Self {
            atlas,
            source_ldr,
            plain,
            world,
            bumped,
            levels,
            states: vec![Vec::new(); blocks.len()],
            blocks,
            style_blocks,
            state_bytes: 0,
        }
    }

    /// The images light styles write into, and how each stores its
    /// texels.
    fn layers(&self) -> Vec<(AssetId<Image>, Layer)> {
        let mut layers = Vec::new();
        if let Some(h) = &self.plain {
            layers.push((h.id(), Layer::F16));
        }
        if let Some(h) = &self.world {
            layers.push((h.id(), if self.source_ldr { Layer::Ldr } else { Layer::F16 }));
        }
        if let Some(handles) = &self.bumped
            && self.atlas.bumped.is_some()
        {
            for (k, h) in handles.iter().enumerate() {
                layers.push((
                    h.id(),
                    if self.source_ldr {
                        Layer::LdrPage(k)
                    } else {
                        Layer::F16Page(k)
                    },
                ));
            }
        }
        layers
    }

    /// The levels a block's light depends on now.
    fn state(&self, block: usize) -> Vec<f32> {
        self.blocks[block].1.iter().map(|&(s, _)| self.levels[s]).collect()
    }

    /// A block's texels at the current levels, encoded for each of
    /// `layers` in turn (rows tightly packed).
    fn encode_block(&self, block: usize, layers: &[(AssetId<Image>, Layer)]) -> Vec<u8> {
        let (flat, pages) = self.block_light(block);
        // Source's LDR pages are encoded together, once per texel.
        let bumped_ldr: Vec<[[u8; 3]; 3]> = match pages.as_ref().filter(|_| self.source_ldr) {
            Some(p) => (0..flat.len())
                .map(|i| source_ldr_bump_texels(flat[i], [p[0][i], p[1][i], p[2][i]]))
                .collect(),
            None => Vec::new(),
        };
        let mut out = Vec::with_capacity(flat.len() * layers.iter().map(|l| l.1.bytes_per_texel()).sum::<usize>());
        for (_, layer) in layers {
            let at = out.len();
            out.resize(at + flat.len() * layer.bytes_per_texel(), 0);
            encode(*layer, &flat, pages.as_ref(), &bumped_ldr, &mut out[at..]);
        }
        out
    }

    /// One block's light with the styles at `levels`: the flat page and
    /// the three directional ones (when the atlas has them), row by row.
    #[allow(clippy::type_complexity)]
    fn block_light(&self, block: usize) -> (Vec<[f32; 3]>, Option<[Vec<[f32; 3]>; 3]>) {
        let ([x, y, w, h], users) = &self.blocks[block];
        let l = &self.atlas;
        let texel = |i: usize| ((y + i as u32 / w) * l.width + x + i as u32 % w) as usize;
        let n = (w * h) as usize;
        let mut flat: Vec<[f32; 3]> = (0..n).map(|i| l.rgb[texel(i)]).collect();
        let mut pages = l
            .bumped
            .as_ref()
            .map(|b| std::array::from_fn::<_, 3, _>(|k| (0..n).map(|i| b[k][texel(i)]).collect::<Vec<_>>()));
        let add = |dst: &mut [f32; 3], v: [f32; 3], k: f32| {
            for c in 0..3 {
                dst[c] = (dst[c] + k * v[c]).max(0.0);
            }
        };
        for &(s, offset) in users {
            let style = &l.styles[s];
            let k = self.levels[s] - style.start();
            if k == 0.0 {
                continue;
            }
            for i in 0..n {
                add(&mut flat[i], style.rgb[offset + i], k);
                if let Some(pages) = pages.as_mut() {
                    for (p, page) in pages.iter_mut().enumerate() {
                        let v = style
                            .bumped
                            .as_ref()
                            .map_or(style.rgb[offset + i], |b| b[p][offset + i]);
                        add(&mut page[i], v, k);
                    }
                }
            }
        }
        (flat, pages)
    }
}

/// Texel bytes of the plain lightmap layer (`lightmap_image`'s format).
pub(super) fn f16_texels(rgb: &[[f32; 3]]) -> Vec<u8> {
    let mut data = Vec::with_capacity(rgb.len() * 8);
    for [r, g, b] in rgb {
        for c in [*r, *g, *b, 1.0] {
            data.extend_from_slice(&half::f16::from_f32(c).to_le_bytes());
        }
    }
    data
}

/// Texel bytes of the world layer in Source's LDR encoding
/// (`source_lightmap_image`'s format).
pub(super) fn source_ldr_texels(rgb: &[[f32; 3]]) -> Vec<u8> {
    let mut data = Vec::with_capacity(rgb.len() * 4);
    for [r, g, b] in rgb {
        data.extend([source_ldr_texel(*r), source_ldr_texel(*g), source_ldr_texel(*b), 255]);
    }
    data
}

/// Texel bytes of the three directional pages in Source's LDR encoding.
pub(super) fn source_ldr_page_texels(flat: &[[f32; 3]], pages: &[Vec<[f32; 3]>; 3]) -> [Vec<u8>; 3] {
    let mut texels: [Vec<u8>; 3] = std::array::from_fn(|_| Vec::with_capacity(flat.len() * 4));
    for (i, f) in flat.iter().enumerate() {
        let enc = source_ldr_bump_texels(*f, [pages[0][i], pages[1][i], pages[2][i]]);
        for (t, [r, g, b]) in texels.iter_mut().zip(enc) {
            t.extend([r, g, b, 255]);
        }
    }
    texels
}

/// Rebuild the lightmap images when switchable light styles change
/// (`LightStyles`): the atlas at map start, plus or minus each style
/// switched since (Source lights' TurnOn/TurnOff), animated ones at the
/// brightness they show now.
pub(super) fn relight(
    styles: Option<Res<LightStyles>>,
    maps: Option<ResMut<StyledLightmaps>>,
    images: Option<ResMut<Assets<Image>>>,
    world_materials: Option<ResMut<Assets<world_material::WorldMaterial>>>,
    standard: Option<ResMut<Assets<StandardMaterial>>>,
) {
    let (Some(styles), Some(mut maps), Some(mut images)) = (styles, maps, images) else {
        return;
    };
    if !styles.is_changed() {
        return;
    }
    let l = maps.atlas.clone();
    let want: Vec<f32> = l
        .styles
        .iter()
        .zip(&maps.levels)
        .map(|(s, now)| {
            if !s.pattern.is_empty() {
                return *now;
            }
            let on = styles
                .styles
                .iter()
                .find(|(n, _)| *n == s.style)
                .map_or(s.on, |(_, on)| *on);
            if on { 1.0 } else { 0.0 }
        })
        .collect();
    if maps.levels == want {
        return;
    }
    let level = |s: &MapLightStyle| {
        l.styles
            .iter()
            .position(|o| std::ptr::eq(o, s))
            .map_or(s.start(), |k| want[k])
    };
    let (rgb, bumped) = l.relit(&level);
    maps.levels = want;
    let built = LightmapLayers::build(&rgb, bumped.as_ref(), l.width, l.height, maps.source_ldr);
    let mut put = |handle: &Option<Handle<Image>>, image: Image| {
        if let Some(h) = handle {
            let _ = images.insert(h.id(), image);
        }
    };
    put(&maps.plain, built.plain);
    put(&maps.world, built.world);
    if let (Some(handles), Some(pages)) = (&maps.bumped, built.bumped) {
        for (h, p) in handles.iter().zip(pages) {
            let _ = images.insert(h.id(), p);
        }
    }
    // Materials bound to the old images are prepared again.
    if let Some(mut m) = world_materials {
        let ids: Vec<_> = m.ids().collect();
        for id in ids {
            let _ = m.get_mut(id);
        }
    }
    if let Some(mut m) = standard {
        let ids: Vec<_> = m.ids().collect();
        for id in ids {
            let _ = m.get_mut(id);
        }
    }
    info!("lightmaps relit: {:?}", styles.styles);
}

/// One frame's writes into lightmap textures: every changed block of
/// every image, packed into one buffer (`bytes`), and where each block
/// goes. The render world uploads the buffer once and copies the blocks
/// out of it on the GPU: a texture write per block (each with its own
/// staging buffer inside wgpu) cost several milliseconds a step.
#[derive(Resource, Default)]
pub struct LightmapUploads {
    pub bytes: Vec<u8>,
    pub copies: Vec<LightmapCopy>,
}

/// One block of one image in `LightmapUploads::bytes`.
#[derive(Clone, Debug, PartialEq)]
pub struct LightmapCopy {
    pub image: AssetId<Image>,
    /// x, y, width, height in the image.
    pub rect: [u32; 4],
    /// Where its first row starts in the buffer, and the distance between
    /// its rows (a multiple of 256, as buffer-to-texture copies require).
    pub offset: u64,
    pub bytes_per_row: u32,
}

/// How an image stores its texels.
#[derive(Clone, Copy, PartialEq)]
enum Layer {
    /// RGBA16F of the flat light.
    F16,
    /// Source's LDR RGBA8 of the flat light.
    Ldr,
    /// RGBA16F of directional page k.
    F16Page(usize),
    /// Source's LDR RGBA8 of directional page k (encoded with the flat
    /// light and the other pages).
    LdrPage(usize),
}

impl Layer {
    fn bytes_per_texel(self) -> usize {
        match self {
            Layer::F16 | Layer::F16Page(_) => 8,
            Layer::Ldr | Layer::LdrPage(_) => 4,
        }
    }
}

/// Row alignment of buffer-to-texture copies (wgpu's
/// `COPY_BYTES_PER_ROW_ALIGNMENT`).
const ROW_ALIGN: usize = 256;

/// Place blocks (width, height) on shelves `width` texels wide, tallest
/// first: each block's position, and the rows used.
fn shelf_pack(sizes: &[(u32, u32)], width: u32) -> (Vec<(u32, u32)>, u32) {
    let mut order: Vec<usize> = (0..sizes.len()).collect();
    order.sort_by_key(|&i| (std::cmp::Reverse(sizes[i].1), std::cmp::Reverse(sizes[i].0), i));
    let mut at = vec![(0, 0); sizes.len()];
    let (mut x, mut y, mut shelf) = (0, 0, 0);
    for i in order {
        let (w, h) = sizes[i];
        if x + w > width && x > 0 {
            (x, y, shelf) = (0, y + shelf, 0);
        }
        at[i] = (x, y);
        x += w;
        shelf = shelf.max(h);
    }
    (at, y + shelf)
}

/// Step animated light styles: each one whose pattern moved on rewrites
/// the blocks it lights (all styles sharing a block included), packed
/// into `LightmapUploads`. Block states not seen before are worked out on
/// the compute task pool and kept (`StyledLightmaps::states`).
pub(super) fn animate(time: Res<Time>, maps: Option<ResMut<StyledLightmaps>>, mut uploads: ResMut<LightmapUploads>) {
    // What the render world didn't take last frame (no renderer: headless)
    // is stale by now.
    uploads.bytes.clear();
    uploads.copies.clear();
    let Some(mut maps) = maps else { return };
    let t = time.elapsed_secs();
    let atlas = maps.atlas.clone();
    let mut dirty: Vec<usize> = Vec::new();
    for (k, s) in atlas.styles.iter().enumerate() {
        if s.pattern.is_empty() {
            continue;
        }
        let now = s.at(t);
        if now != maps.levels[k] {
            maps.levels[k] = now;
            dirty.extend_from_slice(&maps.style_blocks[k]);
        }
    }
    if dirty.is_empty() {
        return;
    }
    dirty.sort_unstable();
    dirty.dedup();
    let layers = maps.layers();
    if layers.is_empty() {
        return;
    }
    // Each block's encoded texels: kept, or worked out now.
    let keys: Vec<Vec<f32>> = dirty.iter().map(|&b| maps.state(b)).collect();
    let mut texels: Vec<Option<Arc<[u8]>>> = dirty
        .iter()
        .zip(&keys)
        .map(|(&b, key)| maps.states[b].iter().find(|(k, _)| k == key).map(|(_, t)| t.clone()))
        .collect();
    let missing: Vec<usize> = (0..dirty.len()).filter(|&i| texels[i].is_none()).collect();
    if !missing.is_empty() {
        let encoded: Vec<(usize, Vec<u8>)> = {
            let maps = &*maps;
            let work = |list: &[usize]| -> Vec<(usize, Vec<u8>)> {
                list.iter()
                    .map(|&i| (i, maps.encode_block(dirty[i], &layers)))
                    .collect()
            };
            match bevy::tasks::ComputeTaskPool::try_get().filter(|_| missing.len() > 8) {
                Some(pool) => {
                    let chunk = missing.len().div_ceil(pool.thread_num().max(1) * 4).max(4);
                    pool.scope(|s| {
                        for list in missing.chunks(chunk) {
                            s.spawn(async move { work(list) });
                        }
                    })
                    .into_iter()
                    .flatten()
                    .collect()
                }
                None => work(&missing),
            }
        };
        for (i, bytes) in encoded {
            let bytes: Arc<[u8]> = bytes.into();
            if maps.state_bytes + bytes.len() <= STATE_CACHE_BYTES {
                maps.state_bytes += bytes.len();
                let b = dirty[i];
                let key = keys[i].clone();
                maps.states[b].push((key, bytes.clone()));
            }
            texels[i] = Some(bytes);
        }
    }
    // Blocks side by side (or stacked) in the atlas go as one rectangle:
    // a copy costs the render thread ~10k instructions however small.
    let rects: Vec<[u32; 4]> = dirty.iter().map(|&b| maps.blocks[b].0).collect();
    let merged = merge_rects(&rects);
    // The same packing in every image's section of the buffer.
    let sizes: Vec<(u32, u32)> = merged.iter().map(|(r, _)| (r[2], r[3])).collect();
    let widest = sizes.iter().map(|s| s.0).max().unwrap_or(1);
    let width = widest.max(atlas.width.min(1024));
    let (at, rows) = shelf_pack(&sizes, width);
    let pitches: Vec<usize> = layers
        .iter()
        .map(|(_, l)| (width as usize * l.bytes_per_texel()).next_multiple_of(ROW_ALIGN))
        .collect();
    let mut starts = Vec::with_capacity(layers.len());
    let mut total = 0;
    for p in &pitches {
        starts.push(total);
        total += p * rows as usize;
    }
    let mut bytes = std::mem::take(&mut uploads.bytes);
    bytes.resize(total, 0);
    let mut copies = std::mem::take(&mut uploads.copies);
    for ((rect, members), &(sx, sy)) in merged.iter().zip(&at) {
        for &(i, dx, dy) in members {
            let Some(src) = &texels[i] else { continue };
            let (w, h) = (rects[i][2] as usize, rects[i][3] as usize);
            let mut from = 0;
            for ((_, layer), (&pitch, &start)) in layers.iter().zip(pitches.iter().zip(&starts)) {
                let bpt = layer.bytes_per_texel();
                let row_bytes = w * bpt;
                let offset = start + (sy + dy) as usize * pitch + (sx + dx) as usize * bpt;
                for row in 0..h {
                    bytes[offset + row * pitch..][..row_bytes].copy_from_slice(&src[from..from + row_bytes]);
                    from += row_bytes;
                }
            }
        }
        for ((image, layer), (&pitch, &start)) in layers.iter().zip(pitches.iter().zip(&starts)) {
            copies.push(LightmapCopy {
                image: *image,
                rect: *rect,
                offset: (start + sy as usize * pitch + sx as usize * layer.bytes_per_texel()) as u64,
                bytes_per_row: pitch as u32,
            });
        }
    }
    uploads.bytes = bytes;
    uploads.copies = copies;
}

/// Join rectangles (x, y, width, height; not overlapping) that touch
/// along a whole side: runs side by side with the same rows, then runs
/// stacked with the same columns. Each result lists its rectangles
/// (index, offset in it).
fn merge_rects(rects: &[[u32; 4]]) -> Vec<([u32; 4], Vec<(usize, u32, u32)>)> {
    let mut order: Vec<usize> = (0..rects.len()).collect();
    order.sort_by_key(|&i| (rects[i][1], rects[i][3], rects[i][0]));
    let mut rows: Vec<([u32; 4], Vec<(usize, u32, u32)>)> = Vec::new();
    for i in order {
        let [x, y, w, h] = rects[i];
        match rows.last_mut() {
            Some((r, members)) if r[1] == y && r[3] == h && r[0] + r[2] == x => {
                members.push((i, r[2], 0));
                r[2] += w;
            }
            _ => rows.push(([x, y, w, h], vec![(i, 0, 0)])),
        }
    }
    rows.sort_by_key(|(r, _)| (r[0], r[2], r[1]));
    let mut out: Vec<([u32; 4], Vec<(usize, u32, u32)>)> = Vec::new();
    for (r, members) in rows {
        match out.last_mut() {
            Some((o, list)) if o[0] == r[0] && o[2] == r[2] && o[1] + o[3] == r[1] => {
                let dy = o[3];
                list.extend(members.into_iter().map(|(i, dx, _)| (i, dx, dy)));
                o[3] += r[3];
            }
            _ => out.push((r, members)),
        }
    }
    out
}

/// Encode a block's texels (its flat light, directional pages and their
/// Source LDR encoding) into `dst` in `layer`'s format.
fn encode(
    layer: Layer,
    flat: &[[f32; 3]],
    pages: Option<&[Vec<[f32; 3]>; 3]>,
    bumped_ldr: &[[[u8; 3]; 3]],
    dst: &mut [u8],
) {
    let f16 = |dst: &mut [u8], src: &[[f32; 3]]| {
        for (d, [r, g, b]) in dst.chunks_exact_mut(8).zip(src) {
            for (c, v) in [*r, *g, *b, 1.0].into_iter().enumerate() {
                d[c * 2..c * 2 + 2].copy_from_slice(&half::f16::from_f32(v).to_le_bytes());
            }
        }
    };
    match layer {
        Layer::F16 => f16(dst, flat),
        Layer::F16Page(k) => f16(dst, pages.map_or(flat, |p| &p[k][..])),
        Layer::Ldr => {
            for (d, [r, g, b]) in dst.chunks_exact_mut(4).zip(flat) {
                d.copy_from_slice(&[source_ldr_texel(*r), source_ldr_texel(*g), source_ldr_texel(*b), 255]);
            }
        }
        Layer::LdrPage(k) => {
            for (d, t) in dst.chunks_exact_mut(4).zip(bumped_ldr) {
                let [r, g, b] = t[k];
                d.copy_from_slice(&[r, g, b, 255]);
            }
        }
    }
}

/// The render side: uploads move to the render world each frame and are
/// copied into the images' textures once they are prepared. (Set up in
/// `finish`: the map plugin may be added before the renderer.)
pub(super) struct LightStylesPlugin;

impl Plugin for LightStylesPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<LightmapUploads>();
    }

    fn finish(&self, app: &mut App) {
        let Some(render_app) = app.get_sub_app_mut(RenderApp) else {
            return;
        };
        render_app
            .init_resource::<LightmapUploads>()
            .add_systems(ExtractSchedule, extract_uploads)
            .add_systems(Render, write_uploads.in_set(RenderSystems::PrepareResources));
    }
}

/// Move the main world's uploads over (after any the render world hasn't
/// written yet, so later steps land last).
fn extract_uploads(mut main: ResMut<MainWorld>, mut render: ResMut<LightmapUploads>) {
    let Some(mut u) = main.get_resource_mut::<LightmapUploads>() else {
        return;
    };
    if u.copies.is_empty() {
        return;
    }
    if render.copies.is_empty() {
        // Swapped, so both sides keep their allocations.
        std::mem::swap(&mut render.bytes, &mut u.bytes);
        std::mem::swap(&mut render.copies, &mut u.copies);
        return;
    }
    let base = render.bytes.len() as u64;
    let LightmapUploads { mut bytes, copies } = std::mem::take(&mut *u);
    render.bytes.append(&mut bytes);
    render.copies.extend(copies.into_iter().map(|c| LightmapCopy {
        offset: c.offset + base,
        ..c
    }));
}

/// Upload the frame's packed blocks into one GPU buffer (kept, grown when
/// needed) and copy each block from it into its texture.
fn write_uploads(
    mut uploads: ResMut<LightmapUploads>,
    images: Res<RenderAssets<GpuImage>>,
    device: Res<RenderDevice>,
    queue: Res<RenderQueue>,
    mut buffer: Local<Option<Buffer>>,
) {
    if uploads.copies.is_empty() {
        return;
    }
    let uploads = &mut *uploads;
    // Buffer writes come in multiples of 4 bytes.
    let len = uploads.bytes.len().next_multiple_of(4);
    uploads.bytes.resize(len, 0);
    let size = len as u64;
    if buffer.as_ref().is_none_or(|b| b.size() < size) {
        *buffer = Some(device.create_buffer(&BufferDescriptor {
            label: Some("lightmap uploads"),
            size: size.next_power_of_two(),
            usage: BufferUsages::COPY_DST | BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        }));
    }
    let Some(buffer) = buffer.as_ref() else { return };
    queue.write_buffer(buffer, 0, &uploads.bytes);
    let mut encoder = device.create_command_encoder(&CommandEncoderDescriptor {
        label: Some("lightmap uploads"),
    });
    for c in uploads.copies.drain(..) {
        // Not on the GPU yet: the image's own upload carries the light at
        // the time it was built; the next step patches it.
        let Some(gpu) = images.get(c.image) else { continue };
        let [x, y, w, h] = c.rect;
        let size = gpu.texture_descriptor.size;
        if x + w > size.width || y + h > size.height || w * h == 0 {
            continue;
        }
        encoder.copy_buffer_to_texture(
            TexelCopyBufferInfo {
                buffer,
                layout: TexelCopyBufferLayout {
                    offset: c.offset,
                    bytes_per_row: Some(c.bytes_per_row),
                    rows_per_image: Some(h),
                },
            },
            TexelCopyTextureInfo {
                texture: &gpu.texture,
                mip_level: 0,
                origin: Origin3d { x, y, z: 0 },
                aspect: TextureAspect::All,
            },
            Extent3d {
                width: w,
                height: h,
                depth_or_array_layers: 1,
            },
        );
    }
    queue.submit([encoder.finish()]);
    uploads.bytes.clear();
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A 4x1 atlas: texel 0 lit steadily, texels 1-2 a flickering style's
    /// block (with a switchable style on texel 2 too), texel 3 another
    /// block of the switchable style.
    fn atlas() -> MapLightmap {
        MapLightmap {
            width: 4,
            height: 1,
            rgb: vec![[0.5; 3], [1.0; 3], [1.5; 3], [0.25; 3]],
            bumped: None,
            styles: vec![
                MapLightStyle {
                    style: 4,
                    on: true,
                    texels: vec![1, 2],
                    rgb: vec![[0.5; 3], [0.5; 3]],
                    pattern: vec![1.0, 0.0],
                    rects: vec![[1, 0, 2, 1]],
                    ..default()
                },
                MapLightStyle {
                    style: 32,
                    on: true,
                    texels: vec![1, 2, 3],
                    rgb: vec![[0.0; 3], [0.25; 3], [0.25; 3]],
                    rects: vec![[1, 0, 2, 1], [3, 0, 1, 1]],
                    ..default()
                },
            ],
        }
    }

    #[test]
    fn animated_blocks_follow_every_style_lighting_them() {
        let mut maps = StyledLightmaps::new(Arc::new(atlas()), false, None, None, None);
        assert_eq!(maps.blocks.len(), 1, "only the animated style's block");
        assert_eq!(maps.style_blocks[0], vec![0]);
        // Dark step of the flicker, switchable light off.
        maps.levels = vec![0.0, 0.0];
        let (flat, pages) = maps.block_light(0);
        assert!(pages.is_none());
        assert_eq!(flat, vec![[0.5; 3], [0.75; 3]]);
        // Matches the whole-atlas rebuild.
        let full = maps.atlas.relit(&|_| 0.0).0;
        assert_eq!(&full[1..3], &flat[..]);
        // Bright step, 'z'.
        maps.levels = vec![25.0 / 12.0, 1.0];
        let (flat, _) = maps.block_light(0);
        assert!((flat[0][0] - (1.0 + 0.5 * 13.0 / 12.0)).abs() < 1e-5, "{flat:?}");
    }

    #[test]
    fn animate_packs_the_blocks_whose_style_stepped() {
        use bevy::ecs::system::RunSystemOnce;
        let mut world = World::new();
        world.insert_resource(Time::<()>::default());
        world.init_resource::<LightmapUploads>();
        world.insert_resource(StyledLightmaps::new(
            Arc::new(atlas()),
            true,
            Some(Handle::default()),
            Some(Handle::default()),
            None,
        ));
        let step = |world: &mut World, secs: f32| {
            world
                .resource_mut::<Time>()
                .advance_by(std::time::Duration::from_secs_f32(secs));
            world.run_system_once(animate).unwrap();
            world.resource::<LightmapUploads>().copies.len()
        };
        // Still on the first step ('m'): nothing to write.
        assert_eq!(step(&mut world, 0.05), 0);
        // Dark step: the block, in both layers, from one buffer.
        assert_eq!(step(&mut world, 0.1), 2);
        let maps = world.resource::<StyledLightmaps>();
        let (flat, _) = maps.block_light(0);
        let u = world.resource::<LightmapUploads>();
        let [f16, ldr] = [&u.copies[0], &u.copies[1]];
        assert_eq!(f16.rect, [1, 0, 2, 1]);
        assert_eq!(ldr.rect, [1, 0, 2, 1]);
        for c in [f16, ldr] {
            assert_eq!(c.bytes_per_row % 256, 0, "copy rows are 256-byte aligned");
        }
        let at = |c: &LightmapCopy, n: usize| &u.bytes[c.offset as usize..c.offset as usize + n];
        assert_eq!(at(f16, 16), &f16_texels(&flat)[..], "f16 RGBA");
        assert_eq!(at(ldr, 8), &source_ldr_texels(&flat)[..], "Source LDR RGBA8");
        // Same step: nothing new (last frame's uploads dropped).
        assert_eq!(step(&mut world, 0.01), 0);
        assert!(world.resource::<LightmapUploads>().bytes.is_empty());
    }

    #[test]
    fn shelf_packing_keeps_blocks_apart_and_dense() {
        let sizes: Vec<(u32, u32)> = (0..200).map(|i| (3 + i * 7 % 29, 2 + i * 5 % 17)).collect();
        let (at, rows) = shelf_pack(&sizes, 128);
        let area: u32 = sizes.iter().map(|(w, h)| w * h).sum();
        for (i, (&(x, y), &(w, h))) in at.iter().zip(&sizes).enumerate() {
            assert!(x + w <= 128 && y + h <= rows);
            for (&(x2, y2), &(w2, h2)) in at[i + 1..].iter().zip(&sizes[i + 1..]) {
                assert!(
                    x + w <= x2 || x2 + w2 <= x || y + h <= y2 || y2 + h2 <= y,
                    "blocks overlap"
                );
            }
        }
        assert!(area as f32 / (128 * rows) as f32 > 0.75, "{area} texels in {rows} rows");
    }

    #[test]
    fn kept_states_skip_relighting() {
        use bevy::ecs::system::RunSystemOnce;
        let mut world = World::new();
        world.insert_resource(Time::<()>::default());
        world.init_resource::<LightmapUploads>();
        world.insert_resource(StyledLightmaps::new(
            Arc::new(atlas()),
            true,
            None,
            Some(Handle::default()),
            None,
        ));
        let mut step = |secs: f32| {
            world
                .resource_mut::<Time>()
                .advance_by(std::time::Duration::from_secs_f32(secs));
            world.run_system_once(animate).unwrap();
            let maps = world.resource::<StyledLightmaps>();
            let u = world.resource::<LightmapUploads>();
            (
                u.copies.len(),
                maps.states[0].len(),
                maps.state_bytes,
                u.bytes[u.copies[0].offset as usize],
            )
        };
        // Dark, bright, dark again: two states, the third step a copy of
        // the first one's bytes.
        let dark = step(0.15);
        let bright = step(0.1);
        let again = step(0.1);
        assert_eq!((dark.0, dark.1), (1, 1), "one copy, the plain image left out");
        assert_eq!(bright.1, 2);
        assert_eq!((again.1, again.2), (2, bright.2), "nothing new kept");
        assert_eq!(again.3, dark.3);
        assert_ne!(bright.3, dark.3);
    }

    #[test]
    fn touching_blocks_merge_into_one_copy() {
        // A row of three (one a different height), one stacked under the
        // first two, one apart.
        let rects = [[0, 0, 4, 2], [4, 0, 3, 2], [7, 0, 2, 3], [0, 2, 7, 1], [20, 20, 2, 2]];
        let merged = merge_rects(&rects);
        let mut got: Vec<_> = merged.iter().map(|(r, m)| (*r, m.len())).collect();
        got.sort();
        assert_eq!(got, vec![([0, 0, 7, 3], 3), ([7, 0, 2, 3], 1), ([20, 20, 2, 2], 1)]);
        let (_, members) = merged.iter().find(|(r, _)| r[2] == 7).unwrap();
        let mut members = members.clone();
        members.sort();
        assert_eq!(members, vec![(0, 0, 0), (1, 4, 0), (3, 0, 2)]);
    }

    #[test]
    fn source_ldr_tables_match_the_formula() {
        for i in 0..20000 {
            let l = i as f32 * 0.00025 - 0.5;
            let q = (l * 1024.0).round().clamp(0.0, 4095.0);
            assert_eq!(
                source_ldr_texel(l),
                (255.0 * 0.5 * (q / 1024.0).powf(1.0 / 2.2)).round() as u8,
                "{l}"
            );
        }
    }

    #[test]
    fn patterns_step_ten_times_a_second() {
        let s = &atlas().styles[0];
        assert_eq!(s.at(0.05), 1.0);
        assert_eq!(s.at(0.15), 0.0);
        assert_eq!(s.at(0.25), 1.0);
        assert_eq!(atlas().styles[1].at(3.0), 1.0, "switchable: its start");
    }
}
