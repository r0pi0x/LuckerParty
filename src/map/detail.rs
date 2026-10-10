//! Detail sprites (`MapDetailProps`: Source detail props' grass and weeds)
//! drawn in a few merged meshes, one per ground cell, each culled whole by
//! distance (`VisibilityRange`), the view frustum and the map's
//! visibility. The vertex shader (detail.wgsl) turns facing sprites to the
//! view, sways tops in the map's wind and fades each sprite by its
//! distance (dithered out, as faded props are); its fragment is the
//! sprite sheet x the sprite's baked light, alpha tested, then fogged.
//!
//! Vertex layout (one mesh holds both kinds): POSITION the sprite's foot;
//! TANGENT xyz the corner's offset from it (fixed quads) and w its kind
//! (0 fixed, 1 facing the view, 2 turning about the vertical only); UV_1
//! the corner across and up (facing kinds); COLOR its light and, in
//! alpha, how much the corner sways.

use bevy::{
    asset::{RenderAssetUsages, embedded_asset},
    camera::{primitives::Aabb, visibility::VisibilityRange},
    mesh::Indices,
    prelude::*,
    reflect::TypePath,
    render::render_resource::{AsBindGroup, PrimitiveTopology},
    shader::ShaderRef,
};

use super::{MapDetailProps, fog::FogUniform};

/// Fade (start, gone) without an env_detail_controller, meters: the
/// game's `cl_detaildist` 1200 less `cl_detailfade` 400, to 1200 units.
pub const DEFAULT_FADE: (f32, f32) = (800.0 * 0.0254, 1200.0 * 0.0254);
/// Ground cells merged into one mesh, meters (1024 units).
const CELL: f32 = 1024.0 * 0.0254;
/// How far a sway moves a sprite's top at full sway (meters) per m/s of
/// wind (our choice: about 7 units in env_wind's usual 20-30 units/s).
const SWAY_PER_SPEED: f32 = 0.25;

#[derive(Clone, Copy, Debug, Default, bevy::render::render_resource::ShaderType)]
pub struct DetailParams {
    /// Fade start and end, meters.
    pub fade: Vec4,
    /// Wind direction (x, z), how far the tops move (meters) and how fast
    /// (radians per second).
    pub wind: Vec4,
    pub fog: FogUniform,
}

#[derive(Asset, TypePath, AsBindGroup, Debug, Clone)]
pub struct DetailMaterial {
    #[uniform(0)]
    pub params: DetailParams,
    #[texture(1)]
    #[sampler(2)]
    pub texture: Option<Handle<Image>>,
}

impl Material for DetailMaterial {
    fn vertex_shader() -> ShaderRef {
        "embedded://mashup/map/detail.wgsl".into()
    }

    fn fragment_shader() -> ShaderRef {
        "embedded://mashup/map/detail.wgsl".into()
    }

    fn specialize(
        _pipeline: &bevy::pbr::MaterialPipeline,
        descriptor: &mut bevy::render::render_resource::RenderPipelineDescriptor,
        _layout: &bevy::mesh::MeshVertexBufferLayoutRef,
        _key: bevy::pbr::MaterialPipelineKey<Self>,
    ) -> Result<(), bevy::render::render_resource::SpecializedMeshPipelineError> {
        descriptor.primitive.cull_mode = None;
        Ok(())
    }

    fn enable_prepass() -> bool {
        false
    }

    fn enable_shadows() -> bool {
        false
    }
}

/// The fade (start, end) a map's detail sprites use, meters.
pub fn fade(props: &MapDetailProps) -> (f32, f32) {
    props.fade.unwrap_or(DEFAULT_FADE)
}

/// How much of a sprite shows at `dist` meters from the view (1 near, 0
/// past the end; linear between), as detail.wgsl computes it.
pub fn fade_amount(dist: f32, (start, end): (f32, f32)) -> f32 {
    if end <= start {
        return if dist < end { 1.0 } else { 0.0 };
    }
    ((end - dist) / (end - start)).clamp(0.0, 1.0)
}

/// The map's detail sprites as one mesh per ground cell, with the cell's
/// bounds (corners included).
pub fn cell_meshes(props: &MapDetailProps) -> Vec<(Mesh, Vec3, Vec3)> {
    let mut cells: std::collections::BTreeMap<(i32, i32), Vec<usize>> = Default::default();
    for (i, q) in props.quads.iter().enumerate() {
        let key = ((q.origin.x / CELL).floor() as i32, (q.origin.z / CELL).floor() as i32);
        cells.entry(key).or_default().push(i);
    }
    cells
        .into_values()
        .map(|members| {
            let n = members.len() * 4;
            let mut positions = Vec::with_capacity(n);
            let mut tangents = Vec::with_capacity(n);
            let mut corners = Vec::with_capacity(n);
            let mut uvs = Vec::with_capacity(n);
            let mut colors = Vec::with_capacity(n);
            let mut indices = Vec::with_capacity(members.len() * 6);
            let (mut lo, mut hi) = (Vec3::splat(f32::MAX), Vec3::splat(f32::MIN));
            for i in members {
                let q = &props.quads[i];
                let base = positions.len() as u32;
                let (kind, across_up) = match q.billboard {
                    None => (0.0, [Vec2::ZERO; 4]),
                    Some(b) => (
                        if b.vertical { 2.0 } else { 1.0 },
                        [
                            Vec2::new(b.lo.x, b.lo.y),
                            Vec2::new(b.lo.x, b.hi.y),
                            Vec2::new(b.hi.x, b.hi.y),
                            Vec2::new(b.hi.x, b.lo.y),
                        ],
                    ),
                };
                // How high each corner is, for the sway (the foot stays).
                let heights: [f32; 4] = match q.billboard {
                    None => q.corners.map(|c| c.y),
                    Some(_) => across_up.map(|c| c.y),
                };
                let top = heights.iter().copied().fold(0.0f32, f32::max).max(1e-4);
                let reach = match q.billboard {
                    None => q.corners.iter().map(|c| c.length()).fold(0.0, f32::max),
                    Some(_) => across_up.iter().map(|c| c.length()).fold(0.0, f32::max),
                };
                lo = lo.min(q.origin - Vec3::splat(reach));
                hi = hi.max(q.origin + Vec3::splat(reach));
                for k in 0..4 {
                    positions.push(q.origin.to_array());
                    let off = q.corners[k];
                    tangents.push([off.x, off.y, off.z, kind]);
                    corners.push(across_up[k].to_array());
                    uvs.push(q.uv[k].to_array());
                    let sway = q.sway * (heights[k] / top).clamp(0.0, 1.0);
                    colors.push([q.light[0], q.light[1], q.light[2], sway]);
                }
                indices.extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
            }
            let mut mesh = Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::RENDER_WORLD);
            mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, vec![[0.0f32, 1.0, 0.0]; positions.len()]);
            mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, positions);
            mesh.insert_attribute(Mesh::ATTRIBUTE_TANGENT, tangents);
            mesh.insert_attribute(Mesh::ATTRIBUTE_UV_0, uvs);
            mesh.insert_attribute(Mesh::ATTRIBUTE_UV_1, corners);
            mesh.insert_attribute(Mesh::ATTRIBUTE_COLOR, colors);
            mesh.insert_indices(Indices::U32(indices));
            (mesh, lo, hi)
        })
        .collect()
}

/// The material for a map's detail sprites.
pub fn material(props: &MapDetailProps, texture: Handle<Image>, fog: FogUniform) -> DetailMaterial {
    let (start, end) = fade(props);
    let wind = props.wind.map_or(Vec4::ZERO, |(dir, speed)| {
        Vec4::new(dir.x, dir.z, speed * SWAY_PER_SPEED, 1.5)
    });
    DetailMaterial {
        params: DetailParams {
            fade: Vec4::new(start, end, 0.0, 0.0),
            wind,
            fog,
        },
        texture: Some(texture),
    }
}

/// Spawn the map's detail sprites under `parent`; `tag` adds the
/// visibility clusters of a cell's bounds.
#[allow(clippy::too_many_arguments)]
pub fn spawn(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<DetailMaterial>,
    props: &MapDetailProps,
    texture: Handle<Image>,
    fog: FogUniform,
    parent: Entity,
    mut tag: impl FnMut(&mut EntityCommands, Vec3, Vec3),
) {
    let material = materials.add(material(props, texture, fog));
    let (_, end) = fade(props);
    for (i, (mesh, lo, hi)) in cell_meshes(props).into_iter().enumerate() {
        let half = (hi - lo) / 2.0;
        let mut e = commands.spawn((
            Name::new(format!("Detail sprites {i}")),
            super::MapPart,
            Mesh3d(meshes.add(mesh)),
            MeshMaterial3d(material.clone()),
            // Positions are the sprites' feet: the bounds take the corners.
            Aabb::from_min_max(lo, hi),
            // Past the fade's end nothing of the cell shows (measured from
            // the centre of its bounds, so its far corner is covered).
            VisibilityRange {
                start_margin: 0.0..0.0,
                end_margin: (end + half.length())..(end + half.length()),
                use_aabb: true,
            },
            bevy::light::NotShadowCaster,
            Transform::default(),
            ChildOf(parent),
        ));
        tag(&mut e, lo, hi);
    }
}

/// Keep the detail materials' fog with the scene's.
fn apply_fog(fog: Option<Res<super::fog::SceneFog>>, mut materials: ResMut<Assets<DetailMaterial>>) {
    let Some(fog) = fog.filter(|f| f.is_changed()) else {
        return;
    };
    let ids: Vec<_> = materials.ids().collect();
    for id in ids {
        if materials.get(id).is_some_and(|m| m.params.fog != fog.0)
            && let Some(mut m) = materials.get_mut(id)
        {
            m.params.fog = fog.0;
        }
    }
}

/// Registers the detail sprite material. Needs rendering.
pub struct DetailMaterialPlugin;

impl Plugin for DetailMaterialPlugin {
    fn build(&self, app: &mut App) {
        embedded_asset!(app, "detail.wgsl");
        app.add_plugins((super::fog::FogShaderPlugin, MaterialPlugin::<DetailMaterial>::default()))
            .add_systems(Update, apply_fog);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::map::{DetailBillboard, DetailQuad};

    fn quad(x: f32, billboard: bool) -> DetailQuad {
        DetailQuad {
            origin: Vec3::new(x, 0.0, 0.0),
            corners: [
                Vec3::new(-0.1, 0.0, 0.0),
                Vec3::new(-0.1, 0.3, 0.0),
                Vec3::new(0.1, 0.3, 0.0),
                Vec3::new(0.1, 0.0, 0.0),
            ],
            billboard: billboard.then_some(DetailBillboard {
                vertical: true,
                lo: Vec2::new(-0.2, 0.0),
                hi: Vec2::new(0.2, 0.5),
            }),
            uv: [Vec2::ZERO, Vec2::Y, Vec2::ONE, Vec2::X],
            light: [1.0, 0.5, 0.25],
            sway: 1.0,
        }
    }

    #[test]
    fn sprites_merge_per_cell_with_bounds_holding_their_corners() {
        let props = MapDetailProps {
            texture: 0,
            quads: vec![quad(0.5, false), quad(1.0, true), quad(CELL * 3.5, false)],
            fade: None,
            wind: None,
        };
        let cells = cell_meshes(&props);
        assert_eq!(cells.len(), 2, "two cells");
        let (mesh, lo, hi) = &cells[0];
        assert_eq!(mesh.count_vertices(), 8);
        assert!(hi.y >= 0.5 - 1e-6 && lo.x <= 0.5 - 0.1, "{lo} {hi}");
        let Some(bevy::mesh::VertexAttributeValues::Float32x4(t)) = mesh.attribute(Mesh::ATTRIBUTE_TANGENT) else {
            panic!("tangents");
        };
        assert_eq!(t[0][3], 0.0, "fixed");
        assert_eq!(t[4][3], 2.0, "turns about the vertical");
        let Some(bevy::mesh::VertexAttributeValues::Float32x4(c)) = mesh.attribute(Mesh::ATTRIBUTE_COLOR) else {
            panic!("colours");
        };
        assert_eq!(c[0][3], 0.0, "the foot doesn't sway");
        assert_eq!(c[1][3], 1.0, "the top does");
    }

    #[test]
    fn fades_between_start_and_end() {
        assert_eq!(fade_amount(1.0, (2.0, 4.0)), 1.0);
        assert_eq!(fade_amount(3.0, (2.0, 4.0)), 0.5);
        assert_eq!(fade_amount(5.0, (2.0, 4.0)), 0.0);
        let props = MapDetailProps {
            texture: 0,
            quads: vec![],
            fade: None,
            wind: None,
        };
        assert_eq!(fade(&props), DEFAULT_FADE);
    }
}
