//! Light styles at run time: switchable styles (Source 32+, lights'
//! TurnOn/TurnOff through `LightStyles`) rebuild the lightmap images; the
//! animated ones (Source 1-31: flicker, pulse, strobe presets, or a
//! light's own pattern) step through their pattern ten times a second and
//! rewrite only the face blocks they light, straight into the GPU textures
//! (`LightmapPatches`), so a flickering light costs a few small uploads a
//! step instead of whole atlases.

use std::{collections::HashMap, sync::Arc};

use bevy::{
    prelude::*,
    render::{
        Render, RenderApp, RenderSystems,
        extract_plugin::{ExtractSchedule, MainWorld},
        render_asset::RenderAssets,
        render_resource::{Extent3d, Origin3d, TexelCopyBufferLayout, TexelCopyTextureInfo, TextureAspect},
        renderer::RenderQueue,
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
}

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
            blocks,
            style_blocks,
        }
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

/// Texel rectangles to write into lightmap textures this frame.
#[derive(Resource, Default)]
pub struct LightmapPatches(pub Vec<LightmapPatch>);

/// New texels for one rectangle of one image (tightly packed rows, in
/// the image's own format).
pub struct LightmapPatch {
    pub image: AssetId<Image>,
    /// x, y, width, height.
    pub rect: [u32; 4],
    pub bytes: Vec<u8>,
}

/// Step animated light styles: each one whose pattern moved on rewrites
/// the blocks it lights (all styles sharing a block included).
pub(super) fn animate(time: Res<Time>, maps: Option<ResMut<StyledLightmaps>>, mut patches: ResMut<LightmapPatches>) {
    // What the render world didn't take last frame (no renderer: headless)
    // is stale by now.
    patches.0.clear();
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
            let blocks = maps.style_blocks[k].clone();
            dirty.extend(blocks);
        }
    }
    if dirty.is_empty() {
        return;
    }
    dirty.sort_unstable();
    dirty.dedup();
    for b in dirty {
        let rect = maps.blocks[b].0;
        let (flat, pages) = maps.block_light(b);
        if let Some(h) = &maps.plain {
            patches.0.push(LightmapPatch {
                image: h.id(),
                rect,
                bytes: f16_texels(&flat),
            });
        }
        if let Some(h) = &maps.world {
            let bytes = if maps.source_ldr {
                source_ldr_texels(&flat)
            } else {
                f16_texels(&flat)
            };
            patches.0.push(LightmapPatch {
                image: h.id(),
                rect,
                bytes,
            });
        }
        if let (Some(handles), Some(pages)) = (&maps.bumped, pages) {
            let bytes: [Vec<u8>; 3] = if maps.source_ldr {
                source_ldr_page_texels(&flat, &pages)
            } else {
                std::array::from_fn(|k| f16_texels(&pages[k]))
            };
            for (h, bytes) in handles.iter().zip(bytes) {
                patches.0.push(LightmapPatch {
                    image: h.id(),
                    rect,
                    bytes,
                });
            }
        }
    }
}

/// The render side: patches move to the render world each frame and are
/// written into the images' textures once they are prepared. (Set up in
/// `finish`: the map plugin may be added before the renderer.)
pub(super) struct LightStylesPlugin;

impl Plugin for LightStylesPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<LightmapPatches>();
    }

    fn finish(&self, app: &mut App) {
        let Some(render_app) = app.get_sub_app_mut(RenderApp) else {
            return;
        };
        render_app
            .init_resource::<LightmapPatches>()
            .add_systems(ExtractSchedule, extract_patches)
            .add_systems(Render, write_patches.in_set(RenderSystems::PrepareResources));
    }
}

fn extract_patches(mut main: ResMut<MainWorld>, mut render: ResMut<LightmapPatches>) {
    if let Some(mut p) = main.get_resource_mut::<LightmapPatches>()
        && !p.0.is_empty()
    {
        render.0.append(&mut p.0);
    }
}

fn write_patches(mut patches: ResMut<LightmapPatches>, images: Res<RenderAssets<GpuImage>>, queue: Res<RenderQueue>) {
    for p in patches.0.drain(..) {
        // Not on the GPU yet: the image's own upload carries the light at
        // the time it was built; the next step patches it.
        let Some(gpu) = images.get(p.image) else { continue };
        let [x, y, w, h] = p.rect;
        let size = gpu.texture_descriptor.size;
        if x + w > size.width || y + h > size.height || w * h == 0 {
            continue;
        }
        let bytes_per_texel = p.bytes.len() as u32 / (w * h);
        queue.write_texture(
            TexelCopyTextureInfo {
                texture: &gpu.texture,
                mip_level: 0,
                origin: Origin3d { x, y, z: 0 },
                aspect: TextureAspect::All,
            },
            &p.bytes,
            TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(w * bytes_per_texel),
                rows_per_image: Some(h),
            },
            Extent3d {
                width: w,
                height: h,
                depth_or_array_layers: 1,
            },
        );
    }
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
    fn animate_patches_the_blocks_whose_style_stepped() {
        use bevy::ecs::system::RunSystemOnce;
        let mut world = World::new();
        world.insert_resource(Time::<()>::default());
        world.init_resource::<LightmapPatches>();
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
            world.resource::<LightmapPatches>().0.len()
        };
        // Still on the first step ('m'): nothing to write.
        assert_eq!(step(&mut world, 0.05), 0);
        // Dark step: the block, in both layers.
        assert_eq!(step(&mut world, 0.1), 2);
        let patches = world.resource::<LightmapPatches>();
        assert_eq!(patches.0[0].rect, [1, 0, 2, 1]);
        assert_eq!(patches.0[0].bytes.len(), 2 * 8, "f16 RGBA");
        assert_eq!(patches.0[1].bytes.len(), 2 * 4, "Source LDR RGBA8");
        // Same step: nothing new (last frame's patches dropped).
        assert_eq!(step(&mut world, 0.01), 0);
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
