//! What a surface looks like at a point, for effects tinted by the surface
//! they hit (dust and debris of specs/cs_source/impact_effects.md section
//! 4, "Surface colour"): the surface texture's average colour and the
//! baked light there. World triangles carry their texture's average
//! colour and lightmap coordinates; props their model's texture average
//! and their light probe.

use std::{collections::HashMap, sync::Arc};

use bevy::prelude::*;

use super::{LightProbe, MapData, MapLightmap, MapMesh, MapTexture};

/// A surface's look at a point: light (linear, 1 = the texture's own
/// brightness) and base colour (the texture average, sRGB 0-1).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SurfaceLook {
    pub light: Vec3,
    pub base: Vec3,
}

struct Tri {
    corners: [Vec3; 3],
    normal: Vec3,
    lightmap: Option<[Vec2; 3]>,
    /// Average colour of the texture and (blended surfaces) of the second
    /// texture with per-corner weights.
    base: Vec3,
    blend: Option<(Vec3, [f32; 3])>,
}

/// Surface looks of the loaded map.
#[derive(Resource, Default)]
pub struct SurfaceColors {
    tris: Vec<Tri>,
    cells: HashMap<IVec3, Vec<u32>>,
    lightmap: Option<Arc<MapLightmap>>,
    light_scale: f32,
    /// Per prop: base colour, light probe, rotation.
    props: Vec<(Option<Vec3>, Option<LightProbe>, Quat)>,
}

const CELL: f32 = 1.0;
/// Triangles whose plane passes farther than this from the point take no
/// part (meters).
const REACH: f32 = 0.05;

fn cell(p: Vec3) -> IVec3 {
    (p / CELL).floor().as_ivec3()
}

/// A texture's average colour (sRGB 0-1), from its smallest stored level.
pub fn average_color(t: &MapTexture) -> Vec3 {
    let data = t.mips.last().filter(|m| !m.is_empty()).unwrap_or(&t.rgba8);
    let mut sum = Vec3::ZERO;
    let mut n = 0.0;
    for px in data.chunks_exact(4) {
        sum += Vec3::new(px[0] as f32, px[1] as f32, px[2] as f32);
        n += 1.0;
    }
    if n == 0.0 { Vec3::ONE } else { sum / n / 255.0 }
}

impl SurfaceColors {
    pub fn new(data: &MapData) -> Self {
        let mut averages: HashMap<usize, Vec3> = HashMap::new();
        let mut avg = |i: Option<usize>| -> Option<Vec3> {
            let i = i?;
            let t = data.textures.get(i)?;
            Some(*averages.entry(i).or_insert_with(|| average_color(t)))
        };
        let mut me = Self {
            // The plain lightmap only (the directional pages aren't needed).
            lightmap: data.lightmap.as_ref().map(|l| {
                Arc::new(MapLightmap {
                    width: l.width,
                    height: l.height,
                    rgb: l.rgb.clone(),
                    bumped: None,
                })
            }),
            light_scale: data.look.light_scale,
            ..default()
        };
        for m in data
            .meshes
            .iter()
            .filter(|m| !m.skybox && m.entity.is_none() && !m.material.starts_with("decal:"))
        {
            let base = avg(m.texture).unwrap_or_else(|| Vec3::from(m.color.map(|c| c as f32 / 255.0)));
            let second = m.blend.and_then(|b| avg(b.texture));
            me.add_mesh(m, base, second);
        }
        let model_colors: Vec<Option<Vec3>> = data
            .models
            .iter()
            .map(|model| model.meshes.iter().find_map(|m| avg(m.texture)))
            .collect();
        me.props = data
            .props
            .iter()
            .map(|p| {
                (
                    model_colors.get(p.model).copied().flatten(),
                    p.lighting.clone(),
                    p.rotation,
                )
            })
            .collect();
        me
    }

    fn add_mesh(&mut self, m: &MapMesh, base: Vec3, second: Option<Vec3>) {
        for t in m.indices.chunks_exact(3) {
            let idx = [t[0], t[1], t[2]].map(|i| i as usize);
            let corners = idx.map(|i| Vec3::from(m.positions[i]));
            let normal = (corners[1] - corners[0])
                .cross(corners[2] - corners[0])
                .normalize_or_zero();
            if normal == Vec3::ZERO {
                continue;
            }
            let lightmap =
                (m.lightmap_uvs.len() == m.positions.len()).then(|| idx.map(|i| Vec2::from(m.lightmap_uvs[i])));
            let blend = second
                .filter(|_| m.blend_weights.len() == m.positions.len())
                .map(|c| (c, idx.map(|i| m.blend_weights[i])));
            let i = self.tris.len() as u32;
            self.tris.push(Tri {
                corners,
                normal,
                lightmap,
                base,
                blend,
            });
            let (lo, hi) = (
                cell(corners[0].min(corners[1]).min(corners[2])),
                cell(corners[0].max(corners[1]).max(corners[2])),
            );
            for x in lo.x..=hi.x {
                for y in lo.y..=hi.y {
                    for z in lo.z..=hi.z {
                        self.cells.entry(IVec3::new(x, y, z)).or_default().push(i);
                    }
                }
            }
        }
    }

    /// The world surface at `p` facing `normal`: the nearest triangle
    /// containing the point (within a few centimetres of its plane).
    pub fn world(&self, p: Vec3, normal: Vec3) -> Option<SurfaceLook> {
        let list = self.cells.get(&cell(p))?;
        let mut best: Option<(f32, &Tri, Vec3)> = None;
        for &i in list {
            let t = &self.tris[i as usize];
            let dist = t.normal.dot(p - t.corners[0]).abs();
            if dist > REACH || t.normal.dot(normal) < 0.5 {
                continue;
            }
            let Some(w) = barycentric(t.corners, p - t.normal * t.normal.dot(p - t.corners[0])) else {
                continue;
            };
            if w.min_element() < -1e-3 {
                continue;
            }
            if best.is_none_or(|(d, ..)| dist < d) {
                best = Some((dist, t, w));
            }
        }
        let (_, t, w) = best?;
        let mut base = t.base;
        if let Some((second, weights)) = t.blend {
            let k = (weights[0] * w.x + weights[1] * w.y + weights[2] * w.z).clamp(0.0, 1.0);
            base = base.lerp(second, k);
        }
        let light = match (&self.lightmap, t.lightmap) {
            (Some(lm), Some(uv)) => sample(lm, uv[0] * w.x + uv[1] * w.y + uv[2] * w.z) * self.light_scale,
            _ => Vec3::ONE,
        };
        Some(SurfaceLook { light, base })
    }

    /// Prop `index` (in `MapData::props`) where its surface faces
    /// `normal` (world space).
    pub fn prop(&self, index: usize, normal: Vec3) -> Option<SurfaceLook> {
        let (base, probe, _) = self.props.get(index)?;
        let base = (*base)?;
        let light = probe
            .as_ref()
            .map_or(Vec3::ONE, |p| p.eval(normal.normalize_or_zero()) * self.light_scale);
        Some(SurfaceLook { light, base })
    }
}

/// Barycentric weights of `p` (in the triangle's plane).
fn barycentric([a, b, c]: [Vec3; 3], p: Vec3) -> Option<Vec3> {
    let (v0, v1, v2) = (b - a, c - a, p - a);
    let (d00, d01, d11, d20, d21) = (v0.dot(v0), v0.dot(v1), v1.dot(v1), v2.dot(v0), v2.dot(v1));
    let den = d00 * d11 - d01 * d01;
    if den.abs() < 1e-12 {
        return None;
    }
    let v = (d11 * d20 - d01 * d21) / den;
    let w = (d00 * d21 - d01 * d20) / den;
    Some(Vec3::new(1.0 - v - w, v, w))
}

/// The lightmap at `uv` (0..1 over the atlas), bilinear.
fn sample(lm: &MapLightmap, uv: Vec2) -> Vec3 {
    if lm.width == 0 || lm.height == 0 || lm.rgb.is_empty() {
        return Vec3::ONE;
    }
    let x = (uv.x * lm.width as f32 - 0.5).clamp(0.0, (lm.width - 1) as f32);
    let y = (uv.y * lm.height as f32 - 0.5).clamp(0.0, (lm.height - 1) as f32);
    let (x0, y0) = (x.floor() as u32, y.floor() as u32);
    let (x1, y1) = ((x0 + 1).min(lm.width - 1), (y0 + 1).min(lm.height - 1));
    let (fx, fy) = (x - x0 as f32, y - y0 as f32);
    let at = |x: u32, y: u32| Vec3::from(lm.rgb[(y * lm.width + x) as usize]);
    let top = at(x0, y0).lerp(at(x1, y0), fx);
    let bottom = at(x0, y1).lerp(at(x1, y1), fx);
    top.lerp(bottom, fy)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn floor_colour_and_light_at_a_point() {
        let mut data = MapData::default();
        data.textures.push(MapTexture {
            name: "t".into(),
            srgb: true,
            mips: vec![],
            width: 2,
            height: 1,
            rgba8: vec![255, 0, 0, 255, 0, 0, 255, 255],
        });
        data.lightmap = Some(MapLightmap {
            width: 1,
            height: 1,
            rgb: vec![[0.25, 0.5, 1.0]],
            bumped: None,
        });
        data.meshes.push(MapMesh {
            positions: vec![[-1.0, 0.0, -1.0], [-1.0, 0.0, 1.0], [1.0, 0.0, 1.0]],
            indices: vec![0, 1, 2],
            lightmap_uvs: vec![[0.5, 0.5]; 3],
            texture: Some(0),
            ..default()
        });
        let s = SurfaceColors::new(&data);
        let look = s.world(Vec3::new(-0.5, 0.01, 0.5), Vec3::Y).expect("floor");
        assert!(look.base.abs_diff_eq(Vec3::new(0.5, 0.0, 0.5), 1e-6), "{:?}", look.base);
        assert!(
            look.light.abs_diff_eq(Vec3::new(0.25, 0.5, 1.0), 1e-6),
            "{:?}",
            look.light
        );
        // Outside the triangle, or facing the other way: nothing.
        assert!(s.world(Vec3::new(0.5, 0.0, -0.5), Vec3::Y).is_none());
        assert!(s.world(Vec3::new(-0.5, 0.0, 0.5), Vec3::NEG_Y).is_none());
    }
}
