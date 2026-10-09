//! Beams and glows from map entities, for any game (specs/source/
//! visual_entities.md): point_spotlight's shaft and halo (2), env_laser and
//! env_beam's beam between two points (3, 4.1), env_lightglow's
//! screen-sized glow (6). Beams are drawn by `BeamMaterial` (beam.wgsl);
//! halos and light glows are glow sprites (`GlowSprite`, faded by how
//! much of their occlusion proxy is seen) whose colour and size follow
//! the eye each frame. The logic switches them through `EntityPart`.

use bevy::{
    asset::{RenderAssetUsages, embedded_asset},
    mesh::Indices,
    prelude::*,
    reflect::TypePath,
    render::render_resource::{AsBindGroup, BlendComponent, BlendFactor, BlendOperation, BlendState, PrimitiveTopology},
    shader::ShaderRef,
};

use super::sprite_material::SpriteMaterial;

/// A beam between two points (engine space, meters).
#[derive(Clone, Debug, PartialEq)]
pub struct MapBeam {
    /// The entity (index into `MapData::entities`) the logic switches it
    /// by.
    pub entity: Option<usize>,
    pub start: Vec3,
    pub end: Vec3,
    /// Widths at the start and end, meters (the strip is twice as wide).
    pub width: f32,
    pub end_width: f32,
    /// Colour x brightness, 0..1.
    pub color: Vec3,
    /// Shading: 1 out, 2 in, 3 both, 0 none; over `fade` meters (0: the
    /// whole beam).
    pub shade: u8,
    pub fade: f32,
    /// Index into `MapData::textures`.
    pub texture: usize,
    /// A spotlight's: the near-axis fade width (meters) and its halo.
    pub spot: Option<SpotHalo>,
    pub start_on: bool,
}

/// A point_spotlight's halo at the beam's start (2.3).
#[derive(Clone, Debug, PartialEq)]
pub struct SpotHalo {
    pub texture: usize,
    /// The spotlight's width W, meters.
    pub width: f32,
    /// rendercolor / 255.
    pub color: Vec3,
    /// The halo scale (60) in meters: half-size h x this.
    pub scale: f32,
    /// Occlusion proxy, meters.
    pub proxy: f32,
}

/// An env_lightglow (6): engine space, meters.
#[derive(Clone, Debug, PartialEq)]
pub struct MapGlow {
    pub entity: Option<usize>,
    pub position: Vec3,
    pub texture: usize,
    /// rendercolor / 255.
    pub color: Vec3,
    /// Half extents (horizontal, vertical) of the quad 100 units ahead,
    /// meters.
    pub half: Vec2,
    /// MinDist, MaxDist, OuterMaxDist, meters.
    pub min: f32,
    pub max: f32,
    pub outer: f32,
    /// "Visible only from front": its forward direction.
    pub front: Option<Vec3>,
    pub proxy: f32,
}

/// A burst of sparks (env_spark; visual_entities.md 8): where (engine
/// space), its direction (zero: none) and magnitude; a game draws it.
#[derive(Message, Clone, Copy, Debug, PartialEq)]
pub struct SparkBurst {
    pub at: Vec3,
    pub dir: Vec3,
    pub magnitude: f32,
}

/// 100 units, the distance the glow's quad sits ahead of the eye.
pub const GLOW_DISTANCE_UNITS: f32 = 100.0;

#[derive(Clone, Copy, Debug, Default, bevy::render::render_resource::ShaderType)]
pub struct BeamParams {
    pub start: Vec4,
    pub end: Vec4,
    pub color: Vec4,
    pub widths: Vec4,
}

#[derive(Asset, TypePath, AsBindGroup, Debug, Clone)]
pub struct BeamMaterial {
    #[uniform(0)]
    pub params: BeamParams,
    #[texture(1)]
    #[sampler(2)]
    pub texture: Option<Handle<Image>>,
}

impl Material for BeamMaterial {
    fn vertex_shader() -> ShaderRef {
        "embedded://mashup/map/beam.wgsl".into()
    }

    fn fragment_shader() -> ShaderRef {
        "embedded://mashup/map/beam.wgsl".into()
    }

    fn alpha_mode(&self) -> AlphaMode {
        AlphaMode::Add
    }

    fn specialize(
        _pipeline: &bevy::pbr::MaterialPipeline,
        descriptor: &mut bevy::render::render_resource::RenderPipelineDescriptor,
        _layout: &bevy::mesh::MeshVertexBufferLayoutRef,
        _key: bevy::pbr::MaterialPipelineKey<Self>,
    ) -> Result<(), bevy::render::render_resource::SpecializedMeshPipelineError> {
        descriptor.primitive.cull_mode = None;
        // Added to the scene (source + destination).
        let add = BlendComponent {
            src_factor: BlendFactor::One,
            dst_factor: BlendFactor::One,
            operation: BlendOperation::Add,
        };
        if let Some(fragment) = descriptor.fragment.as_mut() {
            for target in fragment.targets.iter_mut().flatten() {
                target.blend = Some(BlendState { color: add, alpha: add });
            }
        }
        if let Some(depth) = descriptor.depth_stencil.as_mut() {
            depth.depth_write_enabled = Some(false);
        }
        Ok(())
    }
}

/// A beam's strip: four vertices, spread by the vertex shader.
pub fn beam_mesh() -> Mesh {
    let mut mesh = Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::default());
    mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, vec![[0.0f32; 3]; 4]);
    mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, vec![[0.0f32, 1.0, 0.0]; 4]);
    mesh.insert_attribute(
        Mesh::ATTRIBUTE_UV_0,
        vec![[0.0f32, 0.0], [1.0, 0.0], [0.0, 1.0], [1.0, 1.0]],
    );
    // The shader writes the vertex colour (the shading along the beam).
    mesh.insert_attribute(Mesh::ATTRIBUTE_COLOR, vec![[1.0f32; 4]; 4]);
    mesh.insert_indices(Indices::U32(vec![0, 2, 1, 1, 2, 3]));
    mesh
}

impl MapBeam {
    pub fn params(&self) -> BeamParams {
        BeamParams {
            start: self.start.extend(1.0),
            end: self.end.extend(1.0),
            color: self.color.extend(self.shade as f32),
            widths: Vec4::new(
                self.width,
                self.end_width,
                self.fade,
                self.spot.as_ref().map_or(0.0, |s| s.width),
            ),
        }
    }
}

/// A spotlight halo's look from `eye` (2.3): half-size h x scale, h from 1
/// (eye 4W or more from the axis) to 2 (within W/2); colour x (2k)²
/// clamped, k the cosine between the beam and the eye; none from behind.
pub fn halo_look(halo: &SpotHalo, origin: Vec3, dir: Vec3, eye: Vec3) -> (f32, Vec3) {
    let rel = eye - origin;
    let e = (rel - dir * rel.dot(dir)).length();
    let w = halo.width.max(1e-6);
    let h = (1.0 + (4.0 * w - e) / (3.5 * w)).clamp(1.0, 2.0);
    let k = dir.dot(rel.normalize_or_zero());
    if k <= 0.0 {
        return (h * halo.scale, Vec3::ZERO);
    }
    (h * halo.scale, halo.color * (2.0 * k).powi(2).clamp(0.0, 1.0))
}

/// A light glow's distance fade (6): from MinDist (0) to MaxDist (1),
/// then to 0 at OuterMaxDist when that is past MaxDist.
pub fn glow_fade(glow: &MapGlow, dist: f32) -> f32 {
    let remap = |x: f32, a: f32, b: f32| {
        if b == a {
            if x >= b { 1.0 } else { 0.0 }
        } else {
            ((x - a) / (b - a)).clamp(0.0, 1.0)
        }
    };
    if glow.outer > glow.max && dist > glow.max {
        1.0 - remap(dist, glow.max, glow.outer)
    } else {
        remap(dist, glow.min, glow.max)
    }
}

/// What follows the eye on a halo or glow sprite.
#[derive(Component, Clone, Debug)]
pub(super) enum EyeGlow {
    Halo { halo: SpotHalo, dir: Vec3 },
    Light(MapGlow),
}

/// Halos and light glows follow the eye: their base colour (before the
/// occlusion fade, `GlowSprite`) and their size.
pub(super) fn follow_eye(
    cameras: Query<
        &GlobalTransform,
        (
            With<Camera3d>,
            Without<super::SkyboxCamera>,
            Without<super::ViewModelCamera>,
            Without<super::water::WaterReflectionCamera>,
        ),
    >,
    mut glows: Query<(&GlobalTransform, &EyeGlow, &mut super::GlowSprite, &MeshMaterial3d<SpriteMaterial>)>,
    materials: Option<ResMut<Assets<SpriteMaterial>>>,
) {
    let (Some(eye), Some(mut materials)) = (cameras.iter().next(), materials) else {
        return;
    };
    let eye = eye.translation();
    for (at, look, mut sprite, material) in &mut glows {
        let p = at.translation();
        let (size, color) = match look {
            EyeGlow::Halo { halo, dir } => {
                let (half, color) = halo_look(halo, p, *dir, eye);
                (Vec2::splat(half * 2.0), color)
            }
            EyeGlow::Light(g) => {
                let dist = eye.distance(p);
                let mut color = g.color * glow_fade(g, dist);
                if let Some(front) = g.front
                    && (eye - p).normalize_or_zero().dot(front) < 0.0
                {
                    color = Vec3::ZERO;
                }
                // The same size on screen at any distance: the quad 100
                // units ahead, scaled out to where the glow is.
                let ahead = GLOW_DISTANCE_UNITS * 0.0254;
                (g.half * 2.0 * dist / ahead, color)
            }
        };
        let base = color.extend(1.0);
        if sprite.color != base {
            sprite.color = base;
        }
        if materials.get(&material.0).is_some_and(|m| m.params.size != size)
            && let Some(mut m) = materials.get_mut(&material.0)
        {
            m.params.size = size;
        }
    }
}

pub struct BeamMaterialPlugin;

impl Plugin for BeamMaterialPlugin {
    fn build(&self, app: &mut App) {
        embedded_asset!(app, "beam.wgsl");
        app.add_plugins(MaterialPlugin::<BeamMaterial>::default());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn halo_by_eye() {
        let halo = SpotHalo {
            texture: 0,
            width: 50.0,
            color: Vec3::ONE,
            scale: 60.0,
            proxy: 2.0,
        };
        // Units here: the spec's case (W 50).
        let (half, c) = halo_look(&halo, Vec3::ZERO, Vec3::X, Vec3::new(100.0, 25.0, 0.0));
        // 25 from the axis: h 2; in front, k ≈ 0.97 → (2k)² clamps to 1.
        assert!((half - 120.0).abs() < 1e-3);
        assert_eq!(c, Vec3::ONE);
        let (_, behind) = halo_look(&halo, Vec3::ZERO, Vec3::X, Vec3::new(-100.0, 0.0, 0.0));
        assert_eq!(behind, Vec3::ZERO);
        // k = 0.25: (2k)² = 0.25.
        let eye = Vec3::new(0.25, (1.0f32 - 0.0625).sqrt(), 0.0) * 400.0;
        let (_, quarter) = halo_look(&halo, Vec3::ZERO, Vec3::X, eye);
        assert!((quarter.x - 0.25).abs() < 1e-3, "{quarter}");
    }

    #[test]
    fn glow_distance_fade() {
        let g = MapGlow {
            entity: None,
            position: Vec3::ZERO,
            texture: 0,
            color: Vec3::ONE,
            half: Vec2::ONE,
            min: 100.0,
            max: 500.0,
            outer: 0.0,
            front: None,
            proxy: 2.0,
        };
        assert!((glow_fade(&g, 300.0) - 0.5).abs() < 1e-6);
        let outer = MapGlow { outer: 1000.0, ..g.clone() };
        assert!((glow_fade(&outer, 750.0) - 0.5).abs() < 1e-6);
        assert_eq!(glow_fade(&outer, 1200.0), 0.0);
    }
}
