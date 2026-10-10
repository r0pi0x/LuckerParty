//! color_correction (no spec; public entity docs): a colour lookup table
//! (a `.raw` file of 32×32×32 RGB bytes, red fastest, from the map's pak
//! or the install) weighted by the eye's distance from the entity, faded in
//! and out by Enable/Disable. Each frame the weighted tables blend into one
//! (what isn't covered stays as it is: the identity table), kept in
//! `ColorCorrectionLut`; the client applies it to the finished frame as a
//! post-process (`client::color_correction`, `mat_colorcorrection`).

use std::sync::Arc;

use bevy::{
    asset::RenderAssetUsages,
    prelude::*,
    render::render_resource::{Extent3d, TextureDimension, TextureFormat},
};

/// A table's size per channel.
pub const LUT_SIZE: usize = 32;
/// A `.raw` table's length in bytes.
pub const LUT_BYTES: usize = LUT_SIZE * LUT_SIZE * LUT_SIZE * 3;

/// A placed color_correction.
#[derive(Clone, Debug, PartialEq)]
pub struct MapColorCorrection {
    /// Index into `MapData::entities`.
    pub entity: Option<usize>,
    /// Engine space, meters.
    pub position: Vec3,
    /// Full weight within `min_falloff`, none past `max_falloff`, linear
    /// between (meters); a negative `max_falloff`: full weight everywhere.
    pub min_falloff: f32,
    pub max_falloff: f32,
    pub max_weight: f32,
    /// Seconds to fade in (Enable) and out (Disable).
    pub fade_in: f32,
    pub fade_out: f32,
    /// While it has weight, the others have none.
    pub exclusive: bool,
    pub start_on: bool,
    /// `LUT_BYTES` bytes, entry r + 32 g + 1024 b.
    pub lut: Arc<Vec<u8>>,
}

impl MapColorCorrection {
    /// Its weight at `distance` from the eye at full strength (before the
    /// fade).
    pub fn weight_at(&self, distance: f32) -> f32 {
        if self.max_falloff < 0.0 || distance <= self.min_falloff.max(0.0) {
            return self.max_weight;
        }
        if distance >= self.max_falloff {
            return 0.0;
        }
        let span = (self.max_falloff - self.min_falloff.max(0.0)).max(1e-6);
        self.max_weight * (1.0 - (distance - self.min_falloff.max(0.0)) / span)
    }
}

/// A color_correction in the world: how far faded in (0..1).
#[derive(Component, Clone, Debug)]
pub struct ColorCorrectionVolume {
    pub cc: MapColorCorrection,
    pub fade: f32,
}

/// The blended table this frame (a 32³ RGBA8 3D image) and whether any
/// table has weight (no pass otherwise).
#[derive(Resource, Clone, Debug)]
pub struct ColorCorrectionLut {
    pub image: Handle<Image>,
    pub active: bool,
    /// The weights the image was blended with.
    weights: Vec<f32>,
}

/// The identity table's value for index `i` of channel steps.
fn identity(i: usize) -> f32 {
    i as f32 * 255.0 / (LUT_SIZE - 1) as f32
}

/// Blend tables by weight over the identity (weights summing past 1 are
/// scaled down to 1), as RGBA8 texels in the table's order.
pub fn blend(tables: &[(&[u8], f32)]) -> Vec<u8> {
    let total: f32 = tables.iter().map(|(_, w)| w.max(0.0)).sum();
    let scale = if total > 1.0 { 1.0 / total } else { 1.0 };
    let rest = (1.0 - total * scale).max(0.0);
    let n = LUT_SIZE * LUT_SIZE * LUT_SIZE;
    let mut out = Vec::with_capacity(n * 4);
    for i in 0..n {
        let (r, g, b) = (i % LUT_SIZE, (i / LUT_SIZE) % LUT_SIZE, i / (LUT_SIZE * LUT_SIZE));
        let mut c = [identity(r) * rest, identity(g) * rest, identity(b) * rest];
        for (lut, w) in tables {
            let w = w.max(0.0) * scale;
            for (k, v) in c.iter_mut().enumerate() {
                *v += lut.get(i * 3 + k).copied().unwrap_or(0) as f32 * w;
            }
        }
        out.extend(c.map(|v| v.round().clamp(0.0, 255.0) as u8));
        out.push(255);
    }
    out
}

fn lut_image(data: Vec<u8>) -> Image {
    let mut image = Image::new(
        Extent3d {
            width: LUT_SIZE as u32,
            height: LUT_SIZE as u32,
            depth_or_array_layers: LUT_SIZE as u32,
        },
        TextureDimension::D3,
        data,
        TextureFormat::Rgba8Unorm,
        RenderAssetUsages::RENDER_WORLD | RenderAssetUsages::MAIN_WORLD,
    );
    image.sampler = bevy::image::ImageSampler::linear();
    image
}

/// Spawn the map's color corrections (an `EntityPart` each: the logic
/// turns them on and off).
pub(super) fn spawn(commands: &mut Commands, data: &super::MapData, root: Entity) {
    for (i, cc) in data.color_corrections.iter().enumerate() {
        let mut e = commands.spawn((
            Name::new(format!("Color correction {i}")),
            super::MapPart,
            ColorCorrectionVolume {
                cc: cc.clone(),
                fade: if cc.start_on { 1.0 } else { 0.0 },
            },
            super::emitters::FollowsEntity,
            Transform::from_translation(cc.position),
            ChildOf(root),
        ));
        if let Some(entity) = cc.entity {
            e.insert(super::EntityPart {
                entity,
                on: cc.start_on,
                exists: true,
            });
        }
    }
}

/// Fade each color correction toward on or off, weigh it by the eye's
/// distance, and blend the tables when the weights change.
pub(super) fn update(
    time: Res<Time>,
    eyes: super::emitters::EyeQuery,
    mut volumes: Query<(&mut ColorCorrectionVolume, &GlobalTransform, Option<&super::EntityPart>)>,
    lut: Option<ResMut<ColorCorrectionLut>>,
    images: Option<ResMut<Assets<Image>>>,
    mut commands: Commands,
) {
    if volumes.is_empty() {
        if lut.is_some() {
            commands.remove_resource::<ColorCorrectionLut>();
        }
        return;
    }
    let Some(mut images) = images else { return };
    let dt = time.delta_secs();
    let eye = eyes.iter().next().map(|g| g.translation());
    let mut weights = Vec::new();
    for (mut v, at, part) in &mut volumes {
        let on = part.map_or(v.cc.start_on, |p| p.on && p.exists);
        let target = if on { 1.0 } else { 0.0 };
        let time = if on { v.cc.fade_in } else { v.cc.fade_out };
        v.fade = if time <= 0.0 {
            target
        } else if v.fade < target {
            (v.fade + dt / time).min(target)
        } else {
            (v.fade - dt / time).max(target)
        };
        let distance = eye.map_or(0.0, |e| e.distance(at.translation()));
        weights.push((v.cc.weight_at(distance) * v.fade, v.cc.exclusive));
    }
    // An exclusive one with weight silences the others.
    if let Some(i) = weights.iter().position(|(w, x)| *x && *w > 0.0) {
        for (j, w) in weights.iter_mut().enumerate() {
            if j != i {
                w.0 = 0.0;
            }
        }
    }
    let weights: Vec<f32> = weights.into_iter().map(|(w, _)| w).collect();
    let active = weights.iter().any(|w| *w > 1.0 / 512.0);
    let changed = lut.as_ref().is_none_or(|l| {
        l.weights.len() != weights.len() || l.weights.iter().zip(&weights).any(|(a, b)| (a - b).abs() > 1.0 / 512.0)
    });
    if !changed {
        if let Some(mut l) = lut
            && l.active != active
        {
            l.active = active;
        }
        return;
    }
    let tables: Vec<(&[u8], f32)> = volumes
        .iter()
        .zip(&weights)
        .map(|((v, ..), w)| (v.cc.lut.as_slice(), *w))
        .collect();
    let data = blend(&tables);
    match lut {
        Some(mut l) => {
            if let Some(mut image) = images.get_mut(&l.image) {
                image.data = Some(data);
            }
            l.weights = weights;
            l.active = active;
        }
        None => {
            commands.insert_resource(ColorCorrectionLut {
                image: images.add(lut_image(data)),
                active,
                weights,
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn table(f: impl Fn(usize, usize, usize) -> [u8; 3]) -> Vec<u8> {
        let mut out = Vec::with_capacity(LUT_BYTES);
        for b in 0..LUT_SIZE {
            for g in 0..LUT_SIZE {
                for r in 0..LUT_SIZE {
                    out.extend(f(r, g, b));
                }
            }
        }
        out
    }

    #[test]
    fn tables_blend_over_the_identity() {
        let red = table(|_, _, _| [255, 0, 0]);
        // No weight: the identity (red fastest).
        let none = blend(&[(&red, 0.0)]);
        assert_eq!(&none[4 * 31..4 * 32], &[255, 0, 0, 255]);
        assert_eq!(&none[4 * 32 * 31..4 * 32 * 31 + 4], &[0, 255, 0, 255]);
        // Half weight: halfway to red.
        let half = blend(&[(&red, 0.5)]);
        assert_eq!(&half[0..4], &[128, 0, 0, 255]);
        // Weights past 1 are scaled down: no identity left.
        let blue = table(|_, _, _| [0, 0, 255]);
        let both = blend(&[(&red, 1.0), (&blue, 1.0)]);
        assert_eq!(&both[0..4], &[128, 0, 128, 255]);
    }

    #[test]
    fn weight_falls_off_with_distance() {
        let cc = MapColorCorrection {
            entity: None,
            position: Vec3::ZERO,
            min_falloff: 1.0,
            max_falloff: 3.0,
            max_weight: 0.8,
            fade_in: 0.0,
            fade_out: 0.0,
            exclusive: false,
            start_on: true,
            lut: Arc::new(Vec::new()),
        };
        assert_eq!(cc.weight_at(0.5), 0.8);
        assert!((cc.weight_at(2.0) - 0.4).abs() < 1e-6);
        assert_eq!(cc.weight_at(4.0), 0.0);
        // No far falloff: everywhere.
        let everywhere = MapColorCorrection {
            max_falloff: -1.0,
            ..cc
        };
        assert_eq!(everywhere.weight_at(1000.0), 0.8);
    }
}
