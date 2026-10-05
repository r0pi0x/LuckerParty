//! Lighting data for things that aren't lightmapped (props, later
//! characters), per the public BSP v20 description: per-leaf ambient light
//! cubes (lumps 52/56, HDR 51/55) and the compiled light list ("world
//! lights", lump 15). All results are in engine axes and lightmap units
//! (1.0 shows a texture at its own brightness).

use bevy::prelude::*;
use vbsp::Bsp;

use super::bsp::{METERS_PER_UNIT, to_engine};

fn lump(bytes: &[u8], index: usize) -> &[u8] {
    let at = 8 + index * 16;
    let Some(entry) = bytes.get(at..at + 8) else { return &[] };
    let ofs = i32::from_le_bytes(entry[0..4].try_into().unwrap()).max(0) as usize;
    let len = i32::from_le_bytes(entry[4..8].try_into().unwrap()).max(0) as usize;
    bytes.get(ofs..ofs + len).unwrap_or(&[])
}

fn f32_at(b: &[u8], at: usize) -> f32 {
    f32::from_le_bytes(b[at..at + 4].try_into().unwrap())
}

fn i32_at(b: &[u8], at: usize) -> i32 {
    i32::from_le_bytes(b[at..at + 4].try_into().unwrap())
}

/// Ambient cube samples use a 0-255 scale (unlike lightmaps, which decode
/// with a further /255): fitted on de_dust2, where cube values match
/// lightmaps on surfaces facing away from the sun at a ratio of ~221.
fn rgbe(b: &[u8]) -> Vec3 {
    let scale = 2f32.powi(b[3] as i8 as i32);
    Vec3::new(b[0] as f32, b[1] as f32, b[2] as f32) * scale
}

fn src(x: f32, y: f32, z: f32) -> Vec3 {
    to_engine(vbsp::Vector { x, y, z }) / METERS_PER_UNIT
}

/// Light arriving from six directions: +X, -X, +Y, -Y, +Z, -Z (engine axes).
#[derive(Clone, Copy, Debug, Default)]
pub struct AmbientCube(pub [Vec3; 6]);

impl AmbientCube {
    /// Light on a surface with normal `n`: faces weighted by n squared.
    pub fn eval(&self, n: Vec3) -> Vec3 {
        let c = &self.0;
        let pick = |v: f32, pos: Vec3, neg: Vec3| if v >= 0.0 { pos } else { neg };
        pick(n.x, c[0], c[1]) * n.x * n.x + pick(n.y, c[2], c[3]) * n.y * n.y + pick(n.z, c[4], c[5]) * n.z * n.z
    }
}

struct Sample {
    position: Vec3,
    cube: AmbientCube,
}

/// A compiled light, already converted to engine space.
#[derive(Clone, Debug)]
pub enum WorldLight {
    /// The sun: light travels along `direction`.
    Sky { direction: Vec3, intensity: Vec3 },
    Point {
        position: Vec3,
        intensity: Vec3,
        atten: [f32; 3],
        radius: f32,
    },
    Spot {
        position: Vec3,
        direction: Vec3,
        intensity: Vec3,
        atten: [f32; 3],
        radius: f32,
        stopdot: f32,
        stopdot2: f32,
        exponent: f32,
    },
}

pub struct MapLighting {
    leaves: Vec<Vec<Sample>>,
    /// Lights not already folded into the ambient cubes.
    pub lights: Vec<WorldLight>,
}

/// Set on lights VRAD already added to the leaf ambient cubes.
const DWL_FLAGS_INAMBIENTCUBE: i32 = 1;

impl MapLighting {
    pub fn read(bsp: &Bsp, bytes: &[u8]) -> Self {
        let (mut index, mut samples) = (lump(bytes, 52), lump(bytes, 56));
        if index.is_empty() || samples.is_empty() {
            (index, samples) = (lump(bytes, 51), lump(bytes, 55));
        }
        let mut leaves = Vec::new();
        for (i, entry) in index.as_chunks::<4>().0.iter().enumerate() {
            let count = u16::from_le_bytes([entry[0], entry[1]]) as usize;
            let first = u16::from_le_bytes([entry[2], entry[3]]) as usize;
            let mut list = Vec::new();
            if let Some(leaf) = bsp.leaf(i) {
                let lo = Vec3::new(leaf.mins[0] as f32, leaf.mins[1] as f32, leaf.mins[2] as f32);
                let hi = Vec3::new(leaf.maxs[0] as f32, leaf.maxs[1] as f32, leaf.maxs[2] as f32);
                for s in samples.as_chunks::<28>().0.iter().skip(first).take(count) {
                    let cube: Vec<Vec3> = s[..24].as_chunks::<4>().0.iter().map(|c| rgbe(c)).collect();
                    let frac = Vec3::new(s[24] as f32, s[25] as f32, s[26] as f32) / 255.0;
                    let p = lo + (hi - lo) * frac;
                    // Source order +X -X +Y -Y +Z -Z; engine: X, Z up, -Y.
                    list.push(Sample {
                        position: to_engine(vbsp::Vector { x: p.x, y: p.y, z: p.z }),
                        cube: AmbientCube([cube[0], cube[1], cube[4], cube[5], cube[3], cube[2]]),
                    });
                }
            }
            leaves.push(list);
        }

        let mut lights = Vec::new();
        let mut wl = lump(bytes, 15);
        if wl.is_empty() {
            wl = lump(bytes, 54);
        }
        for l in wl.as_chunks::<88>().0 {
            let v = |at: usize| src(f32_at(l, at), f32_at(l, at + 4), f32_at(l, at + 8));
            let rgb = |at: usize| Vec3::new(f32_at(l, at), f32_at(l, at + 4), f32_at(l, at + 8));
            if i32_at(l, 76) & DWL_FLAGS_INAMBIENTCUBE != 0 {
                continue;
            }
            let position = v(0) * METERS_PER_UNIT;
            // Already in lightmap units: dust2's sun (2.25) matches sunlit
            // floor lightmap values (~2.8 including ambient).
            let intensity = rgb(12);
            let direction = v(24).normalize_or_zero();
            let atten = [f32_at(l, 64), f32_at(l, 68), f32_at(l, 72)];
            let radius = f32_at(l, 60);
            match i32_at(l, 40) {
                1 => lights.push(WorldLight::Point {
                    position,
                    intensity,
                    atten,
                    radius,
                }),
                2 => lights.push(WorldLight::Spot {
                    position,
                    direction,
                    intensity,
                    atten,
                    radius,
                    stopdot: f32_at(l, 48),
                    stopdot2: f32_at(l, 52),
                    exponent: f32_at(l, 56),
                }),
                3 => lights.push(WorldLight::Sky { direction, intensity }),
                _ => {}
            }
        }
        Self { leaves, lights }
    }

    /// The ambient cube at `p` (engine space): samples in the containing
    /// leaf, weighted by inverse squared distance.
    pub fn ambient_at(&self, bsp: &Bsp, p: Vec3) -> AmbientCube {
        let s = p / METERS_PER_UNIT;
        let point = vbsp::Vector {
            x: s.x,
            y: -s.z,
            z: s.y,
        };
        let Some(samples) = leaf_index(bsp, point).and_then(|i| self.leaves.get(i)) else {
            return AmbientCube::default();
        };
        let mut sum = [Vec3::ZERO; 6];
        let mut total = 0.0;
        for sample in samples {
            let w = 1.0 / (sample.position.distance_squared(p) + 0.01);
            for (acc, c) in sum.iter_mut().zip(sample.cube.0) {
                *acc += c * w;
            }
            total += w;
        }
        if total > 0.0 {
            sum.iter_mut().for_each(|c| *c /= total);
        }
        AmbientCube(sum)
    }
}

/// The leaf containing a Source-space point, walking the world's BSP tree.
pub fn leaf_index(bsp: &Bsp, p: vbsp::Vector) -> Option<usize> {
    let mut node = bsp.models.first()?.head_node;
    while node >= 0 {
        let n = bsp.nodes.get(node as usize)?;
        let plane = bsp.plane(n.plane_index as usize)?;
        let side = plane.normal.x * p.x + plane.normal.y * p.y + plane.normal.z * p.z - plane.dist;
        node = if side >= 0.0 { n.children[0] } else { n.children[1] };
    }
    Some((-node - 1) as usize)
}

/// Direct light from `light` on a surface at `p` with normal `n`, without
/// the shadow test. Returns the light and, for shadow tests, the direction
/// toward the light and how far it is (meters; infinite for the sun).
pub fn direct(light: &WorldLight, p: Vec3, n: Vec3) -> Option<(Vec3, Vec3, f32)> {
    let atten = |a: [f32; 3], d_m: f32| {
        let d = d_m / METERS_PER_UNIT;
        let f = a[0] + a[1] * d + a[2] * d * d;
        if f > 0.0 { 1.0 / f } else { 1.0 }
    };
    match light {
        WorldLight::Sky { direction, intensity } => {
            let to = -*direction;
            let dot = n.dot(to);
            (dot > 0.0).then(|| (*intensity * dot, to, f32::INFINITY))
        }
        WorldLight::Point {
            position,
            intensity,
            atten: a,
            radius,
        } => {
            let d = position.distance(p);
            let to = (*position - p) / d.max(1e-4);
            let dot = n.dot(to);
            if dot <= 0.0 || (*radius > 0.0 && d / METERS_PER_UNIT > *radius) {
                return None;
            }
            Some((*intensity * dot * atten(*a, d), to, d))
        }
        WorldLight::Spot {
            position,
            direction,
            intensity,
            atten: a,
            radius,
            stopdot,
            stopdot2,
            exponent,
        } => {
            let d = position.distance(p);
            let to = (*position - p) / d.max(1e-4);
            let dot = n.dot(to);
            if dot <= 0.0 || (*radius > 0.0 && d / METERS_PER_UNIT > *radius) {
                return None;
            }
            let dot2 = -to.dot(*direction);
            if dot2 <= *stopdot2 {
                return None;
            }
            let mut cone = if *exponent > 0.0 && *exponent != 1.0 {
                dot2.powf(*exponent)
            } else {
                dot2
            };
            if dot2 < *stopdot {
                cone *= (dot2 - stopdot2) / (stopdot - stopdot2);
            }
            Some((*intensity * dot * cone * atten(*a, d), to, d))
        }
    }
}

/// Geometry that blocks light, for shadow rays.
pub struct Occluders {
    /// Convex hulls (one compound) and triangle surfaces (separate: compound
    /// shapes can't contain triangle meshes).
    colliders: Vec<avian3d::prelude::Collider>,
}

impl Occluders {
    pub fn new(hulls: &[Vec<[f32; 3]>], triangles: (&[[f32; 3]], &[[u32; 3]])) -> Self {
        use avian3d::prelude::Collider;
        let parts: Vec<_> = hulls
            .iter()
            .filter_map(|h| Collider::convex_hull(h.iter().map(|p| Vec3::from(*p)).collect()))
            .map(|c| (Vec3::ZERO, Quat::IDENTITY, c))
            .collect();
        let mut colliders = vec![Collider::compound(parts)];
        if !triangles.1.is_empty() {
            colliders.push(Collider::trimesh(
                triangles.0.iter().map(|p| Vec3::from(*p)).collect(),
                triangles.1.to_vec(),
            ));
        }
        Self { colliders }
    }

    /// Whether anything blocks the segment from `p` along `dir` for `max`
    /// meters (infinite: up to the map's extent).
    pub fn blocked(&self, p: Vec3, dir: Vec3, max: f32) -> bool {
        let max = if max.is_finite() { max } else { 500.0 };
        self.colliders
            .iter()
            .any(|c| c.cast_ray(Vec3::ZERO, Quat::IDENTITY, p, dir, max, false).is_some())
    }
}

impl MapLighting {
    /// Total light on a surface at `p` with normal `n`: ambient cube plus
    /// shadow-tested direct light.
    pub fn light_at(&self, bsp: &Bsp, occluders: &Occluders, p: Vec3, n: Vec3) -> Vec3 {
        let p = p + n * 0.02;
        let mut light = self.ambient_at(bsp, p).eval(n);
        for l in &self.lights {
            if let Some((value, to, dist)) = direct(l, p, n)
                && value.max_element() > 0.002
                && !occluders.blocked(p, to, dist)
            {
                light += value;
            }
        }
        light
    }
}
