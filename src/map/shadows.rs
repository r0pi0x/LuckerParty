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
            .filter(|m| !m.skybox && !m.material.starts_with("decal:"))
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
        self.coverage
            .iter()
            .map(|c| (c.clamp(0.0, 1.0) * 255.0).round() as u8)
            .collect()
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
            RenderAssetUsages::RENDER_WORLD,
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
/// 3D skybox).
pub fn build(data: &MapData, settings: &MapShadows) -> BuiltShadows {
    let casters: Vec<(usize, &MapProp, ShadowFrame)> = data
        .props
        .iter()
        .enumerate()
        .filter(|(_, p)| p.casts_shadow && !p.skybox)
        .map(|(i, p)| {
            let model = &data.models[p.model];
            let frame = ShadowFrame::new(
                model.bounds,
                p.translation,
                p.rotation,
                settings.direction,
                settings.distance,
            );
            (i, p, frame)
        })
        .collect();
    let cells: Vec<(u32, Vec<f32>)> = casters
        .iter()
        .map(|(_, p, frame)| {
            let model = &data.models[p.model];
            let n = cell_size(model.bounds);
            (
                n,
                silhouette(model, &data.textures, frame, p.translation, p.rotation, n),
            )
        })
        .collect();
    let (atlas, placed) = Atlas::pack(&cells);
    let receivers = Receivers::new(data);
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
        .filter_map(|((i, _, frame), cell)| shadow_mesh(frame, &receivers, cell.rect).map(|m| (*i, m)))
        .collect();
    BuiltShadows {
        atlas,
        cells,
        meshes,
        receivers,
    }
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
