//! Dynamic prop shadows as Source draws them (specs/cs_source/shadows_sky.md,
//! "Building one shadow" and "How a receiver is darkened"): each caster's
//! silhouette, seen along the map's shadow direction, goes into a coverage
//! texture; the shadow is clipped onto the world surfaces behind the
//! caster like a decal, and multiplies them toward the shadow colour with
//! a 5-tap blur, fading with distance and vanishing in fog. See shadow.wgsl.

use bevy::{
    asset::{RenderAssetUsages, embedded_asset},
    image::{ImageAddressMode, ImageFilterMode, ImageSampler, ImageSamplerDescriptor},
    mesh::{Indices, PrimitiveTopology},
    prelude::*,
    reflect::TypePath,
    render::render_resource::{AsBindGroup, Extent3d, TextureDimension, TextureFormat},
    shader::ShaderRef,
};

use super::{MapAlpha, MapData, MapModel, MapProp, MapShadows, MapTexture};

const METERS_PER_UNIT: f32 = 0.0254;
/// Added to each projected width, units.
const BLOAT: f32 = 4.0;
const TEXELS_PER_UNIT: f32 = 2.0;
const MIN_TEX: u32 = 16;
const MAX_TEX: u32 = 256;
const ATLAS_WIDTH: u32 = 1024;
/// Most a shadow is lightened at the far end of its reach.
const FALLOFF_AMOUNT: f32 = 240.0 / 255.0;
/// Shadow geometry sits this far off the surface it darkens (units).
const LIFT: f32 = 0.1;

/// One caster's projection (engine space, meters).
#[derive(Clone, Debug)]
pub struct ShadowFrame {
    pub origin: Vec3,
    pub x: Vec3,
    pub y: Vec3,
    /// Shadow direction (caster toward receiver).
    pub d: Vec3,
    pub size: Vec2,
    pub falloff_start: f32,
    pub max_dist: f32,
    /// The caster box's faces that face the light: receivers must lie
    /// behind them (n . p <= d).
    pub light_faces: Vec<(Vec3, f32)>,
}

impl ShadowFrame {
    /// The frame for a box (model-space bounds) placed at `translation`,
    /// `rotation`, casting along `d` for `distance` meters past itself.
    pub fn new(bounds: (Vec3, Vec3), translation: Vec3, rotation: Quat, d: Vec3, distance: f32) -> Self {
        let (lo, hi) = bounds;
        let s = hi - lo;
        let axes = [rotation * Vec3::X, rotation * Vec3::Y, rotation * Vec3::Z];
        let sizes = [s.x, s.y, s.z];
        // The longest of the box's axes projected across d sets y.
        let projected = (0..3).map(|i| (axes[i] - d * d.dot(axes[i])) * sizes[i]);
        let y = projected
            .max_by(|a, b| a.length_squared().total_cmp(&b.length_squared()))
            .unwrap_or(Vec3::X)
            .normalize_or(d.any_orthonormal_vector());
        let x = y.cross(d).normalize_or_zero();
        let bloat = BLOAT * METERS_PER_UNIT;
        let size = Vec2::new(
            (0..3).map(|i| sizes[i] * axes[i].dot(x).abs()).sum::<f32>() + bloat,
            (0..3).map(|i| sizes[i] * axes[i].dot(y).abs()).sum::<f32>() + bloat,
        );
        let centre = translation + rotation * ((lo + hi) / 2.0);
        let radius = s.length() / 2.0;
        // Back to the box corner furthest against d.
        let m = -(0..3).map(|i| sizes[i] / 2.0 * axes[i].dot(d).abs()).sum::<f32>();
        let origin = centre + d * m;
        let falloff_start = radius - m;
        let light_faces = (0..3)
            .flat_map(|i| [axes[i], -axes[i]].map(|n| (n, n.dot(centre) + sizes[i] / 2.0)))
            .filter(|(n, _)| n.dot(d) < -1e-4)
            .collect();
        Self {
            origin,
            x,
            y,
            d,
            size,
            falloff_start,
            max_dist: distance + falloff_start,
            light_faces,
        }
    }

    /// Shadow space: (x, y, along d), meters.
    pub fn local(&self, p: Vec3) -> Vec3 {
        let r = p - self.origin;
        Vec3::new(self.x.dot(r), self.y.dot(r), self.d.dot(r))
    }

    /// 0..1 texture coordinates within the caster's cell.
    fn uv(&self, p: Vec3) -> Vec2 {
        let l = self.local(p);
        Vec2::new(l.x / self.size.x + 0.5, l.y / self.size.y + 0.5)
    }

    /// Lightening from distance along d.
    fn fade(&self, z: f32) -> f32 {
        let span = (self.max_dist - self.falloff_start).max(1e-4);
        FALLOFF_AMOUNT * ((z - self.falloff_start) / span).clamp(0.0, 1.0)
    }
}

/// Cell size for a caster: 2 texels per unit of its largest dimension,
/// rounded up to a power of two, 16..256.
pub fn cell_size(bounds: (Vec3, Vec3)) -> u32 {
    let units = (bounds.1 - bounds.0).max_element() / METERS_PER_UNIT;
    ((TEXELS_PER_UNIT * units).ceil().max(1.0) as u32)
        .next_power_of_two()
        .clamp(MIN_TEX, MAX_TEX)
}

/// The caster's silhouette as coverage (0..1) in an `n` x `n` cell: its
/// triangles drawn orthographically along d, both faces, alpha-tested and
/// translucent parts by their texture alpha, overlaps saturating.
pub fn silhouette(
    model: &MapModel,
    textures: &[MapTexture],
    frame: &ShadowFrame,
    translation: Vec3,
    rotation: Quat,
    n: u32,
) -> Vec<f32> {
    let mut out = vec![0.0f32; (n * n) as usize];
    let scale = n as f32;
    for m in &model.meshes {
        // Additive glows and beams cast no shadow.
        if m.alpha == MapAlpha::Add {
            continue;
        }
        let texture = (m.alpha != MapAlpha::Opaque)
            .then_some(m.texture)
            .flatten()
            .map(|t| &textures[t]);
        for tri in m.indices.as_chunks::<3>().0 {
            let pts = tri.map(|i| {
                let p = translation + rotation * Vec3::from(m.positions[i as usize]);
                frame.uv(p) * scale
            });
            let uvs = tri.map(|i| Vec2::from(m.uvs.get(i as usize).copied().unwrap_or([0.0, 0.0])));
            let area = (pts[1] - pts[0]).perp_dot(pts[2] - pts[0]);
            if area.abs() < 1e-9 {
                continue;
            }
            let lo = pts[0].min(pts[1]).min(pts[2]).floor().max(Vec2::ZERO);
            let hi = pts[0].max(pts[1]).max(pts[2]).ceil().min(Vec2::splat(scale - 1.0));
            // Wholly outside the cell (a caster bigger than its box: some
            // community props), or not a number.
            if !(lo.x <= hi.x && lo.y <= hi.y) {
                continue;
            }
            for py in lo.y as u32..=hi.y.max(lo.y) as u32 {
                for px in lo.x as u32..=hi.x.max(lo.x) as u32 {
                    let c = Vec2::new(px as f32 + 0.5, py as f32 + 0.5);
                    let w0 = (pts[1] - c).perp_dot(pts[2] - c) / area;
                    let w1 = (pts[2] - c).perp_dot(pts[0] - c) / area;
                    let w2 = 1.0 - w0 - w1;
                    if w0 < 0.0 || w1 < 0.0 || w2 < 0.0 {
                        continue;
                    }
                    let alpha = texture.map_or(1.0, |t| {
                        let uv = uvs[0] * w0 + uvs[1] * w1 + uvs[2] * w2;
                        let tx = (uv.x.rem_euclid(1.0) * t.width as f32) as u32 % t.width.max(1);
                        let ty = (uv.y.rem_euclid(1.0) * t.height as f32) as u32 % t.height.max(1);
                        t.rgba8[((ty * t.width + tx) * 4 + 3) as usize] as f32 / 255.0
                    });
                    let cell = &mut out[(py * n + px) as usize];
                    *cell = (*cell + alpha).min(1.0);
                }
            }
        }
    }
    out
}

/// World triangles, bucketed on a horizontal grid for shadow-box queries.
pub struct Receivers {
    tris: Vec<[Vec3; 3]>,
    cell: f32,
    grid: std::collections::HashMap<(i32, i32), Vec<u32>>,
}

impl Receivers {
    const CELL: f32 = 4.0;

    /// The world's surfaces (not the 3D skybox, decals or overlays).
    pub fn new(data: &MapData) -> Self {
        let mut tris = Vec::new();
        for m in data
            .meshes
            .iter()
            .filter(|m| !m.skybox && m.entity.is_none() && !m.material.starts_with("decal:"))
        {
            for t in m.indices.as_chunks::<3>().0 {
                tris.push(t.map(|i| Vec3::from(m.positions[i as usize])));
            }
        }
        let mut grid: std::collections::HashMap<(i32, i32), Vec<u32>> = Default::default();
        for (i, t) in tris.iter().enumerate() {
            let lo = t[0].min(t[1]).min(t[2]);
            let hi = t[0].max(t[1]).max(t[2]);
            for gx in (lo.x / Self::CELL).floor() as i32..=(hi.x / Self::CELL).floor() as i32 {
                for gz in (lo.z / Self::CELL).floor() as i32..=(hi.z / Self::CELL).floor() as i32 {
                    grid.entry((gx, gz)).or_default().push(i as u32);
                }
            }
        }
        Self {
            tris,
            cell: Self::CELL,
            grid,
        }
    }

    fn near(&self, lo: Vec3, hi: Vec3) -> impl Iterator<Item = &[Vec3; 3]> {
        let mut ids: Vec<u32> = Vec::new();
        for gx in (lo.x / self.cell).floor() as i32..=(hi.x / self.cell).floor() as i32 {
            for gz in (lo.z / self.cell).floor() as i32..=(hi.z / self.cell).floor() as i32 {
                if let Some(v) = self.grid.get(&(gx, gz)) {
                    ids.extend(v);
                }
            }
        }
        ids.sort_unstable();
        ids.dedup();
        ids.into_iter().map(move |i| &self.tris[i as usize])
    }
}

/// Clip a convex polygon to n . p <= d.
fn clip(poly: Vec<Vec3>, n: Vec3, d: f32) -> Vec<Vec3> {
    let mut out = Vec::with_capacity(poly.len() + 2);
    for i in 0..poly.len() {
        let (a, b) = (poly[i], poly[(i + 1) % poly.len()]);
        let (da, db) = (n.dot(a) - d, n.dot(b) - d);
        if da <= 0.0 {
            out.push(a);
        }
        if (da < 0.0) != (db < 0.0) && (da - db).abs() > 1e-12 {
            out.push(a + (b - a) * (da / (da - db)));
        }
    }
    out
}

/// The shadow as geometry on the world: receiver triangles facing the
/// light, behind the caster, within its box and reach; atlas coordinates
/// in UV0 (cell `rect`: origin and size in 0..1) and the fade in the vertex
/// colour's red channel.
pub fn shadow_mesh(frame: &ShadowFrame, receivers: &Receivers, rect: (Vec2, Vec2)) -> Option<Mesh> {
    let (hx, hy) = (frame.size.x / 2.0, frame.size.y / 2.0);
    let corners = [-1.0f32, 1.0].into_iter().flat_map(|sx| {
        [-1.0f32, 1.0].into_iter().flat_map(move |sy| {
            [0.0, frame.max_dist].map(|z| frame.origin + frame.x * (sx * hx) + frame.y * (sy * hy) + frame.d * z)
        })
    });
    let (lo, hi) = corners.fold((Vec3::MAX, Vec3::MIN), |(a, b), c| (a.min(c), b.max(c)));
    let mut planes = vec![
        (frame.x, frame.x.dot(frame.origin) + hx),
        (-frame.x, -frame.x.dot(frame.origin) + hx),
        (frame.y, frame.y.dot(frame.origin) + hy),
        (-frame.y, -frame.y.dot(frame.origin) + hy),
        (-frame.d, -frame.d.dot(frame.origin)),
        (frame.d, frame.d.dot(frame.origin) + frame.max_dist),
    ];
    planes.extend(frame.light_faces.iter().copied());
    let (mut positions, mut uvs, mut colors, mut indices) = (Vec::new(), Vec::new(), Vec::new(), Vec::new());
    for tri in receivers.near(lo, hi) {
        let n = (tri[1] - tri[0]).cross(tri[2] - tri[0]).normalize_or_zero();
        if n.dot(frame.d) >= 0.0 {
            continue;
        }
        if tri[0].min(tri[1]).min(tri[2]).cmpgt(hi).any() || tri[0].max(tri[1]).max(tri[2]).cmplt(lo).any() {
            continue;
        }
        let mut poly = tri.to_vec();
        for (pn, pd) in &planes {
            poly = clip(poly, *pn, *pd);
            if poly.len() < 3 {
                break;
            }
        }
        if poly.len() < 3 {
            continue;
        }
        let base = positions.len() as u32;
        for p in &poly {
            let local = frame.local(*p);
            let uv = Vec2::new(local.x / frame.size.x + 0.5, local.y / frame.size.y + 0.5);
            positions.push((*p + n * (LIFT * METERS_PER_UNIT)).to_array());
            uvs.push((rect.0 + uv * rect.1).to_array());
            colors.push([frame.fade(local.z), 0.0, 0.0, 1.0]);
        }
        for i in 1..poly.len() as u32 - 1 {
            indices.extend([base, base + i, base + i + 1]);
        }
    }
    if indices.is_empty() {
        return None;
    }
    let normals = vec![[0.0, 1.0, 0.0]; positions.len()];
    let mut mesh = Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::RENDER_WORLD);
    mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, positions);
    mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, normals);
    mesh.insert_attribute(Mesh::ATTRIBUTE_UV_0, uvs);
    mesh.insert_attribute(Mesh::ATTRIBUTE_COLOR, colors);
    mesh.insert_indices(Indices::U32(indices));
    Some(mesh)
}

/// Coverage cells packed into one atlas (shelves of equal-height cells).
pub struct Atlas {
    pub width: u32,
    pub height: u32,
    pub coverage: Vec<f32>,
}

impl Atlas {
    /// Pack cells (size, coverage); returns each cell's (origin, size) in
    /// 0..1 atlas coordinates, inset half a texel against bleeding.
    pub fn pack(cells: &[(u32, Vec<f32>)]) -> (Self, Vec<(UVec2, (Vec2, Vec2))>) {
        let mut order: Vec<usize> = (0..cells.len()).collect();
        order.sort_by_key(|&i| std::cmp::Reverse(cells[i].0));
        let mut at = vec![UVec2::ZERO; cells.len()];
        let (mut x, mut y, mut shelf) = (0u32, 0u32, 0u32);
        for &i in &order {
            let n = cells[i].0;
            if x + n > ATLAS_WIDTH {
                x = 0;
                y += shelf;
                shelf = 0;
            }
            at[i] = UVec2::new(x, y);
            x += n;
            shelf = shelf.max(n);
        }
        let height = (y + shelf).max(1);
        let mut coverage = vec![0.0; (ATLAS_WIDTH * height) as usize];
        let mut rects = Vec::with_capacity(cells.len());
        for (i, (n, cov)) in cells.iter().enumerate() {
            for cy in 0..*n {
                for cx in 0..*n {
                    coverage[((at[i].y + cy) * ATLAS_WIDTH + at[i].x + cx) as usize] = cov[(cy * n + cx) as usize];
                }
            }
            let size = Vec2::new(ATLAS_WIDTH as f32, height as f32);
            let origin = (at[i].as_vec2() + 0.5) / size;
            let extent = Vec2::splat(*n as f32 - 1.0) / size;
            rects.push((at[i], (origin, extent)));
        }
        (
            Self {
                width: ATLAS_WIDTH,
                height,
                coverage,
            },
            rects,
        )
    }

    /// Replace one cell's coverage.
    pub fn write(&mut self, at: UVec2, n: u32, coverage: &[f32]) {
        for cy in 0..n {
            for cx in 0..n {
                self.coverage[((at.y + cy) * self.width + at.x + cx) as usize] = coverage[(cy * n + cx) as usize];
            }
        }
    }

    pub fn bytes(&self) -> Vec<u8> {
        self.coverage.iter().map(|c| texel_byte(*c)).collect()
    }

    /// Copy one cell's coverage into the atlas image's bytes (`bytes`'
    /// layout), leaving the rest alone: converting the whole atlas after
    /// each moved prop cost milliseconds a frame on cs_office.
    pub fn write_bytes(&self, bytes: &mut [u8], at: UVec2, n: u32) {
        for y in at.y..(at.y + n).min(self.height) {
            let row = (y * self.width) as usize;
            let (from, to) = (row + at.x as usize, row + (at.x + n).min(self.width) as usize);
            if to > bytes.len() {
                return;
            }
            for (b, c) in bytes[from..to].iter_mut().zip(&self.coverage[from..to]) {
                *b = texel_byte(*c);
            }
        }
    }

    pub fn image(&self) -> Image {
        let data = self.bytes();
        let mut image = Image::new(
            Extent3d {
                width: self.width,
                height: self.height,
                depth_or_array_layers: 1,
            },
            TextureDimension::D2,
            data,
            TextureFormat::R8Unorm,
            // Kept in the main world too: moved props redraw their cells
            // in place (`write_bytes`).
            RenderAssetUsages::default(),
        );
        image.sampler = ImageSampler::Descriptor(ImageSamplerDescriptor {
            address_mode_u: ImageAddressMode::ClampToEdge,
            address_mode_v: ImageAddressMode::ClampToEdge,
            mag_filter: ImageFilterMode::Linear,
            min_filter: ImageFilterMode::Linear,
            ..default()
        });
        image
    }
}

fn texel_byte(c: f32) -> u8 {
    (c.clamp(0.0, 1.0) * 255.0).round() as u8
}

/// One caster's place in the atlas.
#[derive(Clone, Copy, Debug)]
pub struct Cell {
    /// Index in `MapData::props`.
    pub prop: usize,
    pub size: u32,
    pub at: UVec2,
    pub rect: (Vec2, Vec2),
}

/// Everything needed to draw the map's prop shadows.
pub struct BuiltShadows {
    pub atlas: Atlas,
    pub cells: Vec<Cell>,
    /// Per caster prop: its index in `MapData::props` and its shadow mesh.
    pub meshes: Vec<(usize, Mesh)>,
    pub receivers: Receivers,
}

/// Build every caster's shadow (props marked `casts_shadow`, outside the
/// 3D skybox) where the map placed it.
pub fn build(data: &MapData, settings: &MapShadows) -> BuiltShadows {
    let receivers = Receivers::new(data);
    let (atlas, cells, meshes) = build_posed(data, settings, &receivers, &|i| {
        (data.props[i].translation, data.props[i].rotation)
    });
    BuiltShadows {
        atlas,
        cells,
        meshes,
        receivers,
    }
}

/// The map's casters (props marked `casts_shadow`, outside the 3D skybox).
pub fn casters(data: &MapData) -> impl Iterator<Item = (usize, &MapProp)> {
    data.props.iter().enumerate().filter(|(_, p)| p.casts_shadow && !p.skybox)
}

/// `build` with each caster where `pose` says it is now (translation,
/// rotation; physics props move) onto `receivers`.
#[allow(clippy::type_complexity)]
pub fn build_posed(
    data: &MapData,
    settings: &MapShadows,
    receivers: &Receivers,
    pose: &dyn Fn(usize) -> (Vec3, Quat),
) -> (Atlas, Vec<Cell>, Vec<(usize, Mesh)>) {
    let casters: Vec<(usize, &MapProp, ShadowFrame, (Vec3, Quat))> = casters(data)
        .map(|(i, p)| {
            let model = &data.models[p.model];
            let at = pose(i);
            let frame = ShadowFrame::new(model.bounds, at.0, at.1, settings.direction, settings.distance);
            (i, p, frame, at)
        })
        .collect();
    let cells: Vec<(u32, Vec<f32>)> = casters
        .iter()
        .map(|(_, p, frame, at)| {
            let model = &data.models[p.model];
            let n = cell_size(model.bounds);
            (n, silhouette(model, &data.textures, frame, at.0, at.1, n))
        })
        .collect();
    let (atlas, placed) = Atlas::pack(&cells);
    let cells: Vec<Cell> = casters
        .iter()
        .zip(&placed)
        .zip(&cells)
        .map(|(((i, ..), (at, rect)), (size, _))| Cell {
            prop: *i,
            size: *size,
            at: *at,
            rect: *rect,
        })
        .collect();
    let meshes = casters
        .iter()
        .zip(&cells)
        .filter_map(|((i, _, frame, _), cell)| shadow_mesh(frame, receivers, cell.rect).map(|m| (*i, m)))
        .collect();
    (atlas, cells, meshes)
}

// ---------------------------------------------------------------------------
// Shadow detail and blob shadows.

/// Video > Advanced's shadow detail, CS:S's cvars: `r_shadows` (0: no
/// dynamic shadows), `r_shadowrendertotexture` (0: every shadow a blob,
/// Low; 1: render-to-texture, Medium and High) and
/// `r_flashlightdepthtexture` (High; CS:S has no flashlights, so as
/// Medium here). Defaults: shadows on, render-to-texture, no flashlight
/// depth (the dialog's Medium; CS:S's own pick for the machine is to
/// confirm).
#[derive(Resource, Clone, Copy, Debug, PartialEq)]
pub struct ShadowSettings {
    pub shadows: u8,
    pub render_to_texture: u8,
    pub flashlight_depth: u8,
}

impl Default for ShadowSettings {
    fn default() -> Self {
        Self {
            shadows: 1,
            render_to_texture: 1,
            flashlight_depth: 0,
        }
    }
}

/// How dynamic shadows are drawn.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ShadowMode {
    Off,
    /// Source's blob shadows: a round dark patch under each caster,
    /// straight down.
    Blob,
    /// Render-to-texture: each caster's silhouette along the map's
    /// direction (characters: blobs along it, `character_frame`).
    Texture,
}

impl ShadowSettings {
    pub fn mode(&self) -> ShadowMode {
        if self.shadows == 0 {
            ShadowMode::Off
        } else if self.render_to_texture == 0 {
            ShadowMode::Blob
        } else {
            ShadowMode::Texture
        }
    }
}

/// Blob shadows' padding, and their least size, units (spec D:
/// blob_bloat).
const BLOB_BLOAT: f32 = 10.0;
/// A blob's origin is truncated to this grid, units (spec D).
const BLOB_SNAP: f32 = 0.5;
/// The blob picture's size, texels.
pub const BLOB_TEX: u32 = 32;
/// Straight down, engine space.
pub const DOWN: Vec3 = Vec3::NEG_Y;

impl ShadowFrame {
    /// A blob shadow's frame (spec D, "Blob shadows"): the box's widths
    /// across `d` plus `BLOB_BLOAT` (at least that), its origin backed up
    /// twice as far as a render-to-texture shadow's and snapped to
    /// `BLOB_SNAP`.
    pub fn blob(bounds: (Vec3, Vec3), translation: Vec3, rotation: Quat, d: Vec3, distance: f32) -> Self {
        let mut f = Self::new(bounds, translation, rotation, d, distance);
        let blob = BLOB_BLOAT * METERS_PER_UNIT;
        f.size = (f.size - Vec2::splat(BLOAT * METERS_PER_UNIT) + Vec2::splat(blob)).max(Vec2::splat(blob));
        let centre = translation + rotation * ((bounds.0 + bounds.1) / 2.0);
        // `new` backed up by m (negative along d): once more.
        let m = (f.origin - centre).dot(d);
        f.origin += d * m;
        f.falloff_start -= m;
        f.max_dist = distance + f.falloff_start;
        let snap = BLOB_SNAP * METERS_PER_UNIT;
        f.origin = (f.origin / snap).trunc() * snap;
        f
    }
}

/// The blob picture's coverage, `n` x `n`: full in the middle, fading to
/// nothing at the circle's edge (Source's is a fixed round texture; this
/// falloff is ours).
pub fn blob_coverage(n: u32) -> Vec<f32> {
    let n = n.max(2);
    (0..n * n)
        .map(|i| {
            let (x, y) = ((i % n) as f32 + 0.5, (i / n) as f32 + 0.5);
            let r = Vec2::new(x / n as f32 * 2.0 - 1.0, y / n as f32 * 2.0 - 1.0).length();
            let t = ((1.0 - r) / 0.6).clamp(0.0, 1.0);
            t * t * (3.0 - 2.0 * t)
        })
        .collect()
}

/// The blob picture as an atlas of one cell (its rect: the whole).
pub fn blob_atlas() -> Atlas {
    Atlas {
        width: BLOB_TEX,
        height: BLOB_TEX,
        coverage: blob_coverage(BLOB_TEX),
    }
}

/// The whole of the blob picture, as a cell's rect.
pub const BLOB_RECT: (Vec2, Vec2) = (Vec2::ZERO, Vec2::ONE);

/// A character's blob shadow frame: its collision box (`hull`, relative to
/// its origin; Source's player box when it has none) turned to its yaw,
/// cast straight down (Low) or along the map's direction (Medium, High:
/// our characters have no render-to-texture shadow; Source falls back to
/// blobs that way past its 32 a frame).
pub fn character_frame(hull: (Vec3, Vec3), at: Vec3, yaw: Quat, mode: ShadowMode, settings: &MapShadows) -> ShadowFrame {
    let hull = if (hull.1 - hull.0).length_squared() > 0.0 {
        hull
    } else {
        (
            Vec3::new(-16.0, 0.0, -16.0) * METERS_PER_UNIT,
            Vec3::new(16.0, 72.0, 16.0) * METERS_PER_UNIT,
        )
    };
    let d = if mode == ShadowMode::Blob { DOWN } else { settings.direction };
    ShadowFrame::blob(hull, at, yaw, d, settings.distance)
}

/// Whether a caster with these model bounds (meters) and an `n` x `n`
/// atlas cell moved far enough from `from` to `to` (translation, rotation)
/// for its shadow to change: some point of its box by more than a quarter
/// of one of its shadow's texels (at most its box plus `BLOAT` across `n`;
/// the receiver blurs over several).
/// The shadow is cast along the map's fixed direction, so only the
/// caster's own movement shifts it.
pub fn moved_visibly(bounds: (Vec3, Vec3), n: u32, from: (Vec3, Quat), to: (Vec3, Quat)) -> bool {
    let (lo, hi) = bounds;
    let texel = ((hi - lo).max_element().max(0.0) + BLOAT * METERS_PER_UNIT) / n.max(1) as f32;
    let radius = lo.length().max(hi.length());
    let turn = from.1.angle_between(to.1);
    from.0.distance(to.0) + turn * radius > texel / 4.0
}

/// A caster's shadow (silhouette into its atlas cell, and its mesh) for
/// where its prop is now.
pub fn rebuild(
    data: &MapData,
    settings: &MapShadows,
    receivers: &Receivers,
    atlas: &mut Atlas,
    cell: &Cell,
    translation: Vec3,
    rotation: Quat,
) -> Option<Mesh> {
    let model = &data.models[data.props[cell.prop].model];
    let frame = ShadowFrame::new(
        model.bounds,
        translation,
        rotation,
        settings.direction,
        settings.distance,
    );
    let coverage = silhouette(model, &data.textures, &frame, translation, rotation, cell.size);
    atlas.write(cell.at, cell.size, &coverage);
    shadow_mesh(&frame, receivers, cell.rect)
}

#[derive(Clone, Copy, Debug, Default, bevy::render::render_resource::ShaderType)]
pub struct ShadowParams {
    /// Shadow colour, linear.
    pub color: Vec4,
    /// One atlas texel (u, v).
    pub texel: Vec2,
    /// Range fog colour (w = 1 when on) and start, end, max density.
    pub fog_color: Vec4,
    pub fog_range: Vec4,
}

#[derive(Asset, TypePath, AsBindGroup, Debug, Clone)]
pub struct ShadowMaterial {
    #[uniform(0)]
    pub params: ShadowParams,
    #[texture(1)]
    #[sampler(2)]
    pub atlas: Handle<Image>,
}

impl Material for ShadowMaterial {
    fn fragment_shader() -> ShaderRef {
        "embedded://mashup/map/shadow.wgsl".into()
    }

    fn alpha_mode(&self) -> AlphaMode {
        AlphaMode::Multiply
    }

    /// Sort after every other see-through surface: Source draws shadows
    /// over the world with its decals and overlays already on it. (For
    /// custom materials this only moves the sort key, not the depth.)
    fn depth_bias(&self) -> f32 {
        1.0e6
    }
}

pub struct ShadowMaterialPlugin;

impl Plugin for ShadowMaterialPlugin {
    fn build(&self, app: &mut App) {
        embedded_asset!(app, "shadow.wgsl");
        app.add_plugins(MaterialPlugin::<ShadowMaterial>::default());
    }
}

/// The shadow colour as the receiver shader uses it: sRGB byte to linear
/// with the 2.2 curve (values from 0.95 up count as 1).
pub fn shadow_color(color: [u8; 3]) -> Vec4 {
    let c = color.map(|v| {
        let x = v as f32 / 255.0;
        if x >= 0.95 { 1.0 } else { x.powf(2.2) }
    });
    Vec4::new(c[0], c[1], c[2], 1.0)
}

/// What the map's dynamic shadows are made from (its props and
/// `shadow_control`, the world surfaces they fall on, the static parts'
/// root): kept so shadow detail can rebuild them at once
/// (`apply_shadow_detail`).
#[derive(Resource)]
pub(crate) struct ShadowSource {
    pub(super) data: std::sync::Arc<MapData>,
    pub(super) settings: MapShadows,
    pub(super) receivers: std::sync::Arc<Receivers>,
    pub(super) root: Entity,
    pub(super) fog_color: Vec4,
    pub(super) fog_range: Vec4,
    /// Tag shadows with the PVS clusters they touch (`vis`).
    pub(super) tag: bool,
    /// The mode built; None: not yet.
    pub(super) built: Option<ShadowMode>,
    /// The blob picture's material (characters' blobs; props' at Low).
    pub(super) blob_material: Option<Handle<ShadowMaterial>>,
}

/// Build the map's prop shadows as shadow detail asks: render-to-texture
/// (Medium, High), blobs straight down (Low) or none (`r_shadows 0`);
/// again, at once, when it changes (physics props where they are now,
/// those the logic hid hidden).
#[allow(clippy::too_many_arguments, clippy::type_complexity)]
pub(super) fn apply_shadow_detail(
    mut commands: Commands,
    source: Option<ResMut<ShadowSource>>,
    detail: Option<Res<ShadowSettings>>,
    old: Query<Entity, Or<(With<super::PropShadow>, With<CharacterBlob>)>>,
    moved: Query<(&super::PropIndex, &Transform), With<super::PhysicsProp>>,
    hidden: Query<&super::PropIndex, With<super::vis::LogicHidden>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut images: ResMut<Assets<Image>>,
    mut materials: Option<ResMut<Assets<ShadowMaterial>>>,
) {
    let (Some(mut source), Some(materials)) = (source, materials.as_mut()) else {
        return;
    };
    let mode = detail.map_or(ShadowMode::Texture, |d| d.mode());
    if source.built == Some(mode) {
        return;
    }
    for e in &old {
        commands.entity(e).despawn();
    }
    commands.remove_resource::<super::ShadowState>();
    source.built = Some(mode);
    source.blob_material = None;
    if mode == ShadowMode::Off {
        info!("prop shadows: off (r_shadows 0)");
        return;
    }
    let poses: std::collections::HashMap<usize, (Vec3, Quat)> =
        moved.iter().map(|(i, t)| (i.0, (t.translation, t.rotation))).collect();
    let hidden: std::collections::HashSet<usize> = hidden.iter().map(|i| i.0).collect();
    let data = source.data.clone();
    let pose = |i: usize| poses.get(&i).copied().unwrap_or((data.props[i].translation, data.props[i].rotation));
    let (color, fog_color, fog_range) = (shadow_color(source.settings.color), source.fog_color, source.fog_range);
    let params = move |texel: Vec2| ShadowParams {
        color,
        texel,
        fog_color,
        fog_range,
    };
    // The blob picture (characters' blobs in every mode).
    let blob = blob_atlas();
    let blob_image = images.add(blob.image());
    let blob_material = materials.add(ShadowMaterial {
        params: params(Vec2::splat(1.0 / BLOB_TEX as f32)),
        atlas: blob_image.clone(),
    });
    source.blob_material = Some(blob_material.clone());
    let (atlas, cells, built, material, image) = if mode == ShadowMode::Texture {
        let (atlas, cells, built) = build_posed(&data, &source.settings, &source.receivers, &pose);
        let image = images.add(atlas.image());
        let material = materials.add(ShadowMaterial {
            params: params(Vec2::new(1.0 / atlas.width as f32, 1.0 / atlas.height as f32)),
            atlas: image.clone(),
        });
        (atlas, cells, built, material, image)
    } else {
        let mut cells = Vec::new();
        let mut built = Vec::new();
        for (i, p) in casters(&data) {
            let at = pose(i);
            cells.push(Cell {
                prop: i,
                size: BLOB_TEX,
                at: UVec2::ZERO,
                rect: BLOB_RECT,
            });
            let frame = ShadowFrame::blob(data.models[p.model].bounds, at.0, at.1, DOWN, source.settings.distance);
            if let Some(mesh) = shadow_mesh(&frame, &source.receivers, BLOB_RECT) {
                built.push((i, mesh));
            }
        }
        (blob, cells, built, blob_material, blob_image)
    };
    info!("prop shadows ({mode:?}): {} casters reach the world", built.len());
    let visibility = data.visibility.as_deref().filter(|_| source.tag);
    let mut entities = std::collections::HashMap::new();
    for (prop, mesh) in built {
        // Physics props' shadows move with them: their clusters follow
        // the rebuilt mesh (`update_prop_shadows`).
        let clusters = match (visibility, bevy::camera::primitives::MeshAabb::compute_aabb(&mesh)) {
            (Some(v), Some(aabb)) => super::vis::box_clusters(v, Vec3::from(aabb.min()), Vec3::from(aabb.max())),
            _ => Vec::new(),
        };
        let mut e = commands.spawn((
            Name::new(format!("Shadow of prop {prop}")),
            super::MapPart,
            super::PropShadow { prop },
            Mesh3d(meshes.add(mesh)),
            MeshMaterial3d(material.clone()),
            bevy::light::NotShadowCaster,
            Transform::default(),
            ChildOf(source.root),
        ));
        if !clusters.is_empty() {
            e.insert(super::vis::VisClusters::new(clusters));
        }
        if hidden.contains(&prop) {
            e.insert((super::vis::LogicHidden, Visibility::Hidden));
        }
        entities.insert(prop, e.id());
    }
    commands.insert_resource(super::ShadowState {
        data: data.clone(),
        settings: source.settings.clone(),
        receivers: source.receivers.clone(),
        blob: mode == ShadowMode::Blob,
        atlas,
        atlas_image: image,
        material,
        built: cells.iter().map(|c| (c.prop, pose(c.prop))).collect(),
        cells: cells.into_iter().map(|c| (c.prop, c)).collect(),
        entities,
        root: source.root,
    });
}

/// A character's blob shadow: whose, and where it was drawn for
/// (position, yaw).
#[derive(Component, Debug)]
pub struct CharacterBlob {
    pub owner: Entity,
    drawn: (Vec3, f32),
}

/// Characters' blob shadows (Source's blob for players: our characters
/// have no render-to-texture shadow; straight down at Low, along the
/// map's direction at Medium and High): one per living character whose
/// body is drawn (not your own in first person, not one watched through
/// its eyes), its mesh rewritten in place when it moves.
#[allow(clippy::type_complexity)]
pub(super) fn update_character_blobs(
    mut commands: Commands,
    source: Option<Res<ShadowSource>>,
    detail: Option<Res<ShadowSettings>>,
    characters: Query<
        (
            Entity,
            &GlobalTransform,
            &crate::core::Intent,
            Option<&crate::core::MovementState>,
            Option<&crate::core::Health>,
            Option<&Children>,
        ),
        (Without<crate::core::Spectating>, Without<super::ragdoll::Ragdolled>),
    >,
    bodies: Query<&Visibility, (With<super::CharacterBody>, Without<CharacterBlob>)>,
    mut blobs: Query<(Entity, &mut CharacterBlob, &Mesh3d, &mut Visibility), Without<super::CharacterBody>>,
    mut meshes: ResMut<Assets<Mesh>>,
) {
    let mode = detail.map_or(ShadowMode::Texture, |d| d.mode());
    let material = source.as_ref().and_then(|s| s.blob_material.clone());
    let (Some(source), Some(material), false) = (source, material, mode == ShadowMode::Off) else {
        for (e, ..) in &blobs {
            commands.entity(e).despawn();
        }
        return;
    };
    let mut have: std::collections::HashMap<Entity, Entity> = blobs.iter().map(|(e, b, ..)| (b.owner, e)).collect();
    for (owner, at, intent, movement, health, children) in &characters {
        let alive = health.is_none_or(|h| h.current > 0.0);
        let drawn = children
            .into_iter()
            .flatten()
            .any(|c| bodies.get(*c).is_ok_and(|v| *v != Visibility::Hidden));
        if !alive || !drawn {
            continue;
        }
        let pose = (at.translation(), intent.yaw);
        let blob = have.remove(&owner);
        if let Some(Ok((_, b, ..))) = blob.map(|b| blobs.get(b))
            && b.drawn.0.distance(pose.0) < 0.005
            && (b.drawn.1 - pose.1).abs() < 0.03
        {
            continue;
        }
        let hull = movement.map_or((Vec3::ZERO, Vec3::ZERO), |m| (m.hull_min, m.hull_max));
        let frame = character_frame(hull, pose.0, Quat::from_rotation_y(pose.1), mode, &source.settings);
        let mesh = shadow_mesh(&frame, &source.receivers, BLOB_RECT);
        match (blob.and_then(|b| blobs.get_mut(b).ok()), mesh) {
            (Some((_, mut b, handle, mut vis)), Some(mesh)) => {
                b.drawn = pose;
                vis.set_if_neq(Visibility::Inherited);
                let _ = meshes.insert(handle.id(), mesh);
            }
            (Some((_, mut b, _, mut vis)), None) => {
                // Nothing under it within reach (jumping, falling).
                b.drawn = pose;
                vis.set_if_neq(Visibility::Hidden);
            }
            (None, Some(mesh)) => {
                commands.spawn((
                    Name::new("Character blob shadow"),
                    super::MapPart,
                    CharacterBlob { owner, drawn: pose },
                    Mesh3d(meshes.add(mesh)),
                    MeshMaterial3d(material.clone()),
                    bevy::light::NotShadowCaster,
                    Transform::default(),
                    Visibility::Inherited,
                    ChildOf(source.root),
                ));
            }
            (None, None) => {}
        }
    }
    // Characters gone, dead or not drawn.
    for (_, blob) in have {
        commands.entity(blob).despawn();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn jitter_keeps_the_shadow() {
        // A 32-unit crate: a 64-texel cell of about half a unit (1.3 cm)
        // each.
        let half = Vec3::splat(16.0 * METERS_PER_UNIT);
        let bounds = (-half, half);
        let n = cell_size(bounds);
        let at = (Vec3::new(1.0, 0.5, 2.0), Quat::from_rotation_y(0.4));
        let nudged = |dt: Vec3, turn: f32| (at.0 + dt, at.1 * Quat::from_rotation_x(turn));
        // Settling jitter seen on cs_office: 0.1 mm, 0.01 degrees.
        assert!(!moved_visibly(bounds, n, at, nudged(Vec3::splat(1e-4), 2e-4)));
        // A centimetre or a degree is a different shadow.
        assert!(moved_visibly(bounds, n, at, nudged(Vec3::X * 0.01, 0.0)));
        assert!(moved_visibly(bounds, n, at, nudged(Vec3::ZERO, 1f32.to_radians())));
    }

    #[test]
    fn redrawing_a_cell_matches_the_whole_atlas() {
        let cells = [(16, vec![0.25; 256]), (32, vec![0.5; 1024]), (16, vec![1.0; 256])];
        let (mut atlas, placed) = Atlas::pack(&cells);
        let mut bytes = atlas.bytes();
        let (at, n) = (placed[2].0, 16);
        atlas.write(at, n, &[0.75; 256]);
        atlas.write_bytes(&mut bytes, at, n);
        assert_eq!(bytes, atlas.bytes());
    }

    /// Blob shadows (spec D): round, at least 10 units across, under the
    /// caster straight down, reaching past it twice as far back.
    #[test]
    fn blobs_are_round_and_sized_as_source_s() {
        let c = blob_coverage(BLOB_TEX);
        let at = |x: u32, y: u32| c[(y * BLOB_TEX + x) as usize];
        assert!(at(16, 16) > 0.99 && at(0, 0) == 0.0 && at(0, 16) < 0.05, "full in the middle, none at the edge");
        assert!((at(8, 16) - at(16, 8)).abs() < 1e-6, "round");
        // A 2-unit pebble: the least size; a player box: its width plus 10.
        let u = METERS_PER_UNIT;
        let pebble = ShadowFrame::blob((Vec3::splat(-u), Vec3::splat(u)), Vec3::ZERO, Quat::IDENTITY, DOWN, 1.0);
        assert!((pebble.size - Vec2::splat(12.0 * u)).abs().max_element() < 1e-4, "{}", pebble.size / u);
        let hull = (Vec3::new(-16.0, 0.0, -16.0) * u, Vec3::new(16.0, 72.0, 16.0) * u);
        let player = ShadowFrame::blob(hull, Vec3::ZERO, Quat::IDENTITY, DOWN, 1.0);
        assert!((player.size - Vec2::splat(42.0 * u)).abs().max_element() < 1e-4, "{}", player.size / u);
        let rtt = ShadowFrame::new(hull, Vec3::ZERO, Quat::IDENTITY, DOWN, 1.0);
        let centre = Vec3::new(0.0, 36.0 * u, 0.0);
        assert!(((player.origin - centre).dot(DOWN) - 2.0 * (rtt.origin - centre).dot(DOWN)).abs() < BLOB_SNAP * u);
    }

    /// A tiny map: a floor and the shadow settings.
    fn floor_map() -> MapData {
        MapData {
            name: "test:blobs".into(),
            shadows: Some(MapShadows {
                direction: Vec3::new(0.3, -1.0, 0.0).normalize(),
                color: [100, 100, 100],
                distance: 2.0,
            }),
            meshes: vec![super::super::MapMesh {
                material: "test/floor".into(),
                positions: vec![[-10.0, 0.0, -10.0], [-10.0, 0.0, 10.0], [10.0, 0.0, 10.0], [10.0, 0.0, -10.0]],
                normals: vec![[0.0, 1.0, 0.0]; 4],
                uvs: vec![[0.0, 0.0]; 4],
                lightmap_uvs: vec![[0.0, 0.0]; 4],
                indices: vec![0, 1, 2, 0, 2, 3],
                color: [200, 180, 150],
                ..default()
            }],
            ..default()
        }
    }

    /// Characters whose body is drawn get a blob under them (straight down
    /// at Low, along the map's direction at Medium); none once the body is
    /// hidden (your own in first person) or `r_shadows 0`.
    #[test]
    fn drawn_characters_get_blob_shadows() {
        // Just the shadow systems over the map's floor (no simulation).
        let mut app = App::new();
        app.add_plugins((
            MinimalPlugins,
            TransformPlugin,
            AssetPlugin::default(),
            bevy::mesh::MeshPlugin,
        ))
        .init_asset::<Image>()
        .init_asset::<ShadowMaterial>()
        .add_systems(Update, (apply_shadow_detail, update_character_blobs).chain());
        let data = std::sync::Arc::new(floor_map());
        let root = app.world_mut().spawn((Transform::default(), Visibility::default())).id();
        app.insert_resource(ShadowSource {
            receivers: std::sync::Arc::new(Receivers::new(&data)),
            settings: data.shadows.clone().unwrap(),
            data,
            root,
            fog_color: Vec4::ZERO,
            fog_range: Vec4::ZERO,
            tag: false,
            built: None,
            blob_material: None,
        });
        struct Sim {
            app: App,
        }
        impl Sim {
            fn ticks(&mut self, n: usize) {
                for _ in 0..n {
                    self.app.update();
                }
            }
        }
        let mut sim = Sim { app };
        let at = Vec3::new(2.0, 0.05, 1.0);
        let character = sim
            .app
            .world_mut()
            .spawn((
                crate::core::Intent::default(),
                crate::core::Health::default(),
                Transform::from_translation(at),
            ))
            .id();
        let body = sim
            .app
            .world_mut()
            .spawn((
                super::super::CharacterBody {
                    model: 0,
                    joints: Vec::new(),
                    held: None,
                },
                Visibility::Inherited,
                ChildOf(character),
            ))
            .id();
        let blob_centre = |sim: &mut Sim| {
            let world = sim.app.world_mut();
            let found: Vec<Handle<Mesh>> = world
                .query::<(&CharacterBlob, &Mesh3d, &Visibility)>()
                .iter(world)
                .filter(|(_, _, v)| **v != Visibility::Hidden)
                .map(|(_, m, _)| m.0.clone())
                .collect();
            found.first().map(|m| {
                let mesh = world.resource::<Assets<Mesh>>().get(m).unwrap();
                Vec3::from(bevy::camera::primitives::MeshAabb::compute_aabb(mesh).unwrap().center)
            })
        };
        let set = |sim: &mut Sim, shadows: u8, rtt: u8| {
            sim.app.insert_resource(ShadowSettings {
                shadows,
                render_to_texture: rtt,
                flashlight_depth: 0,
            });
            sim.ticks(3);
        };
        set(&mut sim, 1, 0);
        let low = blob_centre(&mut sim).expect("a blob at Low");
        assert!(low.xz().distance(at.xz()) < 0.05, "straight down: {low}");
        set(&mut sim, 1, 1);
        let medium = blob_centre(&mut sim).expect("a blob at Medium");
        assert!(medium.x > at.x + 0.05, "along the map's direction: {medium}");
        sim.app.world_mut().entity_mut(body).insert(Visibility::Hidden);
        sim.ticks(2);
        assert!(blob_centre(&mut sim).is_none(), "no body drawn, no blob");
        sim.app.world_mut().entity_mut(body).insert(Visibility::Inherited);
        set(&mut sim, 0, 1);
        assert!(blob_centre(&mut sim).is_none(), "r_shadows 0");
    }
}
