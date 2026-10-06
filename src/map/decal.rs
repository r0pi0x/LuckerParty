//! Runtime decals (bullet holes, knife slashes) for any game: the map
//! carries decal definitions grouped by name (`MapDecals`); a `PlaceDecal`
//! message projects one onto the world triangles near the hit point,
//! clipped to the decal's rectangle, as the game projects decals onto the
//! faces they touch (specs/cs_source/overlays_decals.md). Drawn with
//! `DecalMaterial`: the surface is multiplied by the decal (modulate 2x).
//! Props and characters take no decals yet.

use std::collections::{HashMap, HashSet, VecDeque};

use bevy::{
    asset::{RenderAssetUsages, embedded_asset},
    mesh::{Indices, PrimitiveTopology},
    prelude::*,
    reflect::TypePath,
    render::render_resource::{AsBindGroup, BlendComponent, BlendFactor, BlendOperation, BlendState},
    shader::ShaderRef,
};

use super::{MapMesh, MapPart};

/// One decal image: a rectangle of a texture (often a decal atlas) and its
/// size on surfaces.
#[derive(Clone, Debug)]
pub struct MapDecal {
    /// Index into `MapData::textures`.
    pub texture: usize,
    pub uv_min: Vec2,
    pub uv_max: Vec2,
    /// Width and height, meters.
    pub size: Vec2,
    pub blend: DecalBlend,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DecalBlend {
    /// The surface times 2 x texel (0.5 grey leaves it unchanged).
    Modulate2x,
    /// Alpha-blended decals (lit by the surface in the game); drawn as a
    /// stain for now.
    Alpha,
}

/// The map's runtime decals: definitions, named groups of weighted
/// variants, and the group a surface material letter takes when shot.
#[derive(Clone, Debug, Default)]
pub struct MapDecals {
    pub decals: Vec<MapDecal>,
    /// Group name -> (weight, index into `decals`).
    pub groups: HashMap<String, Vec<(f32, usize)>>,
    /// Surface game material letter -> group name.
    pub by_material: HashMap<char, String>,
}

/// Put a decal from `group` on the world at `point`, on the surface with
/// `normal`; `dir` is the direction of whatever made it. `spin` turns it
/// randomly in the surface plane (bullet holes); otherwise walls keep it
/// upright. With a `target` prop (or one of its colliders) the decal goes
/// on that prop's model and moves with it; otherwise on the world.
#[derive(Message, Clone, Debug)]
pub struct PlaceDecal {
    pub target: Option<Entity>,
    pub group: DecalGroup,
    pub point: Vec3,
    pub normal: Vec3,
    pub dir: Vec3,
    pub spin: bool,
}

/// Which decals to choose from.
#[derive(Clone, Debug)]
pub enum DecalGroup {
    /// A named group (e.g. the knife's slashes).
    Named(String),
    /// What a surface with this game material letter takes when shot.
    Material(char),
}

/// Present while the loaded map has impact decals (games' placeholder hit
/// marks stand down).
#[derive(Resource)]
pub struct ImpactDecals;

/// Most decals kept; the oldest goes first. (The game's limit is a
/// client setting, see the spec's open questions.)
pub const MAX_DECALS: usize = 512;
/// Triangles whose plane passes farther than this from the point (meters)
/// take no part of the decal.
const REACH: f32 = 0.1;
/// Lift off the surface (meters): above the map's own decals (0.15 units)
/// and overlays (up to 0.25 units), so impacts draw on top of them.
const LIFT: f32 = 0.35 * 0.0254;
const CELL: f32 = 1.0;

/// Triangles (corners and normal) in a uniform grid.
#[derive(Default)]
pub(super) struct TriSet {
    tris: Vec<[Vec3; 4]>,
    cells: HashMap<IVec3, Vec<u32>>,
}

/// What decals can land on: the world's triangles, and each prop model's
/// in its own space.
#[derive(Resource, Default)]
pub(super) struct DecalSurfaces {
    world: TriSet,
    models: Vec<TriSet>,
    /// Prop index -> model index.
    props: Vec<usize>,
}

fn cell(p: Vec3) -> IVec3 {
    (p / CELL).floor().as_ivec3()
}

impl DecalSurfaces {
    pub(super) fn new(data: &super::MapData) -> Self {
        Self {
            world: TriSet::new(&data.meshes),
            models: data.models.iter().map(|m| TriSet::new(&m.meshes)).collect(),
            props: data.props.iter().map(|p| p.model).collect(),
        }
    }
}

impl TriSet {
    /// Opaque surfaces (not the 3D skybox, other decals or overlays,
    /// unlit or blended surfaces).
    pub(super) fn new(meshes: &[MapMesh]) -> Self {
        let mut out = Self::default();
        for m in meshes {
            if m.skybox || m.unlit || m.material.starts_with("decal:") || matches!(m.alpha, super::MapAlpha::Blend | super::MapAlpha::Add) {
                continue;
            }
            for t in m.indices.chunks_exact(3) {
                let [a, b, c] = [t[0], t[1], t[2]].map(|i| Vec3::from(m.positions[i as usize]));
                let n = (b - a).cross(c - a).normalize_or_zero();
                if n == Vec3::ZERO {
                    continue;
                }
                let i = out.tris.len() as u32;
                out.tris.push([a, b, c, n]);
                let (lo, hi) = (cell(a.min(b).min(c)), cell(a.max(b).max(c)));
                for x in lo.x..=hi.x {
                    for y in lo.y..=hi.y {
                        for z in lo.z..=hi.z {
                            out.cells.entry(IVec3::new(x, y, z)).or_default().push(i);
                        }
                    }
                }
            }
        }
        out
    }

    /// Triangles with a bounding cell within `r` of `p`.
    fn near(&self, p: Vec3, r: f32) -> Vec<u32> {
        let (lo, hi) = (cell(p - Vec3::splat(r)), cell(p + Vec3::splat(r)));
        let mut seen = HashSet::new();
        for x in lo.x..=hi.x {
            for y in lo.y..=hi.y {
                for z in lo.z..=hi.z {
                    if let Some(list) = self.cells.get(&IVec3::new(x, y, z)) {
                        seen.extend(list.iter().copied());
                    }
                }
            }
        }
        let mut v: Vec<u32> = seen.into_iter().collect();
        v.sort_unstable();
        v
    }
}

/// Decal axes on a surface: right and down as seen from in front. Walls
/// stay upright; floors and ceilings align to world X (the rule map decals
/// follow; CS:S's bullet holes aren't turned either).
pub fn basis(normal: Vec3) -> (Vec3, Vec3) {
    let n = normal.normalize_or_zero();
    let up = if n.y.abs() < 0.7 { Vec3::Y } else { Vec3::X };
    let right = up.cross(n).normalize_or_zero();
    let down = right.cross(n).normalize_or_zero();
    (right, down)
}

/// Clip a polygon (position, uv) to 0 <= u, v <= 1.
fn clip(mut poly: Vec<(Vec3, Vec2)>) -> Vec<(Vec3, Vec2)> {
    for (axis, keep_above) in [(0, true), (0, false), (1, true), (1, false)] {
        let inside = |p: &(Vec3, Vec2)| if keep_above { p.1[axis] >= 0.0 } else { p.1[axis] <= 1.0 };
        let edge = if keep_above { 0.0 } else { 1.0 };
        let mut out = Vec::with_capacity(poly.len() + 4);
        for i in 0..poly.len() {
            let a = poly[i];
            let b = poly[(i + 1) % poly.len()];
            if inside(&a) {
                out.push(a);
            }
            if inside(&a) != inside(&b) {
                let t = (edge - a.1[axis]) / (b.1[axis] - a.1[axis]);
                out.push((a.0.lerp(b.0, t), a.1.lerp(b.1, t)));
            }
        }
        poly = out;
        if poly.len() < 3 {
            return poly;
        }
    }
    poly
}

/// The decal's mesh on the surfaces near `point`: positions, normals and
/// texture coordinates within `decal`'s rectangle. None when nothing is
/// in reach.
pub(super) fn project(
    surfaces: &TriSet,
    decal: &MapDecal,
    point: Vec3,
    normal: Vec3,
    right: Vec3,
    down: Vec3,
) -> Option<Mesh> {
    let n = normal.normalize_or_zero();
    let r = decal.size.max_element() * 0.75 + REACH;
    let (mut pos, mut nor, mut uvs, mut local, mut idx) = (Vec::new(), Vec::new(), Vec::new(), Vec::new(), Vec::new());
    for i in surfaces.near(point, r) {
        let [a, b, c, tn] = surfaces.tris[i as usize];
        if tn.dot(n) < 0.5 || tn.dot(point - a).abs() > REACH {
            continue;
        }
        let uv = |p: Vec3| {
            let d = p - point;
            Vec2::new(d.dot(right) / decal.size.x + 0.5, d.dot(down) / decal.size.y + 0.5)
        };
        let poly = clip(vec![(a, uv(a)), (b, uv(b)), (c, uv(c))]);
        if poly.len() < 3 {
            continue;
        }
        let base = pos.len() as u32;
        for (p, t) in &poly {
            pos.push((*p + tn * LIFT).to_array());
            nor.push(tn.to_array());
            uvs.push(decal.uv_min + (decal.uv_max - decal.uv_min) * *t);
            local.push(t.to_array());
        }
        for k in 1..poly.len() as u32 - 1 {
            idx.extend([base, base + k, base + k + 1]);
        }
    }
    if idx.is_empty() {
        return None;
    }
    let mut mesh = Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::default());
    mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, pos);
    mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, nor);
    mesh.insert_attribute(Mesh::ATTRIBUTE_UV_0, uvs.into_iter().map(|v| v.to_array()).collect::<Vec<_>>());
    // Position within the decal (0..1), for the shader's edge fade.
    mesh.insert_attribute(Mesh::ATTRIBUTE_UV_1, local);
    mesh.insert_indices(Indices::U32(idx));
    Some(mesh)
}

/// The loaded map's decal definitions, their textures and materials, and
/// the decals placed so far.
#[derive(Resource)]
pub(super) struct DecalAssets {
    pub(super) data: MapDecals,
    /// Every map texture, by `MapData::textures` index.
    pub(super) textures: Vec<Handle<Image>>,
    materials: HashMap<usize, Handle<DecalMaterial>>,
    placed: VecDeque<Entity>,
    rng: u64,
}

impl DecalAssets {
    pub(super) fn new(data: MapDecals, textures: Vec<Handle<Image>>) -> Self {
        Self {
            data,
            textures,
            materials: HashMap::new(),
            placed: VecDeque::new(),
            rng: 0x9e37_79b9_7f4a_7c15,
        }
    }

    fn random(&mut self) -> f32 {
        self.rng ^= self.rng << 13;
        self.rng ^= self.rng >> 7;
        self.rng ^= self.rng << 17;
        (self.rng >> 40) as f32 / (1u64 << 24) as f32
    }

    /// A variant of `group`, by weight.
    fn pick(&mut self, group: &DecalGroup) -> Option<usize> {
        let name = match group {
            DecalGroup::Named(n) => n.to_lowercase(),
            DecalGroup::Material(c) => self.data.by_material.get(&c.to_ascii_uppercase())?.clone(),
        };
        let list = self.data.groups.get(&name)?.clone();
        let total: f32 = list.iter().map(|(w, _)| w.max(0.0)).sum();
        let mut x = self.random() * total;
        for (w, d) in &list {
            x -= w.max(0.0);
            if x <= 0.0 {
                return Some(*d);
            }
        }
        list.last().map(|(_, d)| *d)
    }
}

/// Place the decals asked for this frame.
pub(super) fn place_decals(
    mut asks: MessageReader<PlaceDecal>,
    surfaces: Option<Res<DecalSurfaces>>,
    props: Query<(&super::PropIndex, &GlobalTransform)>,
    parents: Query<&ChildOf>,
    assets: Option<ResMut<DecalAssets>>,
    meshes: Option<ResMut<Assets<Mesh>>>,
    materials: Option<ResMut<Assets<DecalMaterial>>>,
    mut commands: Commands,
) {
    let (Some(surfaces), Some(mut assets), Some(mut meshes), Some(mut materials)) =
        (surfaces, assets, meshes, materials)
    else {
        asks.clear();
        return;
    };
    for ask in asks.read() {
        let Some(index) = assets.pick(&ask.group) else { continue };
        let decal = assets.data.decals[index].clone();
        let (mut right, mut down) = basis(ask.normal);
        if ask.spin {
            let turn = Quat::from_axis_angle(ask.normal.normalize_or_zero(), assets.random() * std::f32::consts::TAU);
            (right, down) = (turn * right, turn * down);
        }
        // On a prop: its model, in its own space, as its child. A hit
        // collider may be the prop or a child of it.
        let prop = ask.target.and_then(|t| {
            std::iter::once(t)
                .chain(parents.get(t).ok().map(|c| c.parent()))
                .find_map(|e| props.get(e).ok().map(|(i, at)| (e, i.0, at.affine().inverse())))
        });
        let projected = match prop {
            Some((entity, index, to_local)) => {
                let Some(set) = surfaces.props.get(index).and_then(|m| surfaces.models.get(*m)) else {
                    continue;
                };
                let local = |v: Vec3| to_local.transform_vector3(v).normalize_or_zero();
                project(
                    set,
                    &decal,
                    to_local.transform_point3(ask.point),
                    local(ask.normal),
                    local(right),
                    local(down),
                )
                .map(|m| (m, Some(entity)))
            }
            None => project(&surfaces.world, &decal, ask.point, ask.normal, right, down).map(|m| (m, None)),
        };
        let Some((mesh, parent)) = projected else {
            continue;
        };
        let Some(texture) = assets.textures.get(decal.texture).cloned() else {
            continue;
        };
        let material = match assets.materials.get(&index) {
            Some(m) => m.clone(),
            None => {
                let m = materials.add(DecalMaterial {
                    params: Vec4::new(if decal.blend == DecalBlend::Modulate2x { 1.0 } else { 0.0 }, 0.0, 0.0, 0.0),
                    texture,
                });
                assets.materials.insert(index, m.clone());
                m
            }
        };
        let mut decal_entity = commands.spawn((
            Name::new("Decal"),
            Mesh3d(meshes.add(mesh)),
            MeshMaterial3d(material),
            Transform::default(),
            Visibility::default(),
            MapPart,
        ));
        if let Some(p) = parent {
            decal_entity.insert(ChildOf(p));
        }
        let e = decal_entity.id();
        assets.placed.push_back(e);
        while assets.placed.len() > MAX_DECALS {
            if let Some(old) = assets.placed.pop_front() {
                commands.entity(old).try_despawn();
            }
        }
    }
}

#[derive(Asset, TypePath, AsBindGroup, Debug, Clone)]
pub struct DecalMaterial {
    /// x: 1 for modulate 2x, 0 for a stain.
    #[uniform(0)]
    pub params: Vec4,
    #[texture(1)]
    #[sampler(2)]
    pub texture: Handle<Image>,
}

impl Material for DecalMaterial {
    fn fragment_shader() -> ShaderRef {
        "embedded://mashup/map/decal.wgsl".into()
    }

    fn alpha_mode(&self) -> AlphaMode {
        AlphaMode::Blend
    }

    /// Sorted after the map's own decals and overlays (same surfaces,
    /// lower lift), so impacts draw on top of them.
    fn depth_bias(&self) -> f32 {
        1000.0
    }

    /// The framebuffer times twice the shader's output; no depth writes.
    fn specialize(
        _pipeline: &bevy::pbr::MaterialPipeline,
        descriptor: &mut bevy::render::render_resource::RenderPipelineDescriptor,
        _layout: &bevy::mesh::MeshVertexBufferLayoutRef,
        _key: bevy::pbr::MaterialPipelineKey<Self>,
    ) -> Result<(), bevy::render::render_resource::SpecializedMeshPipelineError> {
        // src x dst + dst x src = 2 x src x dst.
        let multiply = BlendComponent {
            src_factor: BlendFactor::Dst,
            dst_factor: BlendFactor::Src,
            operation: BlendOperation::Add,
        };
        let keep = BlendComponent {
            src_factor: BlendFactor::Zero,
            dst_factor: BlendFactor::One,
            operation: BlendOperation::Add,
        };
        if let Some(fragment) = descriptor.fragment.as_mut() {
            for target in fragment.targets.iter_mut().flatten() {
                target.blend = Some(BlendState {
                    color: multiply,
                    alpha: keep,
                });
            }
        }
        if let Some(depth) = descriptor.depth_stencil.as_mut() {
            depth.depth_write_enabled = Some(false);
        }
        Ok(())
    }
}

pub struct DecalMaterialPlugin;

impl Plugin for DecalMaterialPlugin {
    fn build(&self, app: &mut App) {
        embedded_asset!(app, "decal.wgsl");
        app.add_plugins(MaterialPlugin::<DecalMaterial>::default());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wall() -> TriSet {
        // A 4 m square wall facing +Z at z = 0, two triangles.
        let m = MapMesh {
            positions: vec![[-2.0, -2.0, 0.0], [2.0, -2.0, 0.0], [2.0, 2.0, 0.0], [-2.0, 2.0, 0.0]],
            indices: vec![0, 1, 2, 0, 2, 3],
            ..default()
        };
        TriSet::new(&[m])
    }

    fn decal(size: f32) -> MapDecal {
        MapDecal {
            texture: 0,
            uv_min: Vec2::new(0.5, 0.25),
            uv_max: Vec2::new(0.75, 0.5),
            size: Vec2::splat(size),
            blend: DecalBlend::Modulate2x,
        }
    }

    fn positions(m: &Mesh) -> Vec<Vec3> {
        match m.attribute(Mesh::ATTRIBUTE_POSITION) {
            Some(bevy::mesh::VertexAttributeValues::Float32x3(v)) => v.iter().map(|p| Vec3::from(*p)).collect(),
            _ => vec![],
        }
    }

    #[test]
    fn decal_is_clipped_to_its_rectangle_on_the_surface() {
        let s = wall();
        let (right, down) = basis(Vec3::Z);
        assert!(right.abs_diff_eq(Vec3::X, 1e-6) && down.abs_diff_eq(Vec3::NEG_Y, 1e-6), "{right} {down}");
        let m = project(&s, &decal(0.1), Vec3::new(0.3, 0.2, 0.0), Vec3::Z, right, down).expect("a decal");
        let p = positions(&m);
        let (lo, hi) = p.iter().fold((Vec3::MAX, Vec3::MIN), |(lo, hi), v| (lo.min(*v), hi.max(*v)));
        assert!(lo.abs_diff_eq(Vec3::new(0.25, 0.15, LIFT), 1e-4), "{lo}");
        assert!(hi.abs_diff_eq(Vec3::new(0.35, 0.25, LIFT), 1e-4), "{hi}");
        // Texture coordinates stay inside the atlas rectangle.
        let Some(bevy::mesh::VertexAttributeValues::Float32x2(uv)) = m.attribute(Mesh::ATTRIBUTE_UV_0) else {
            panic!("no uvs")
        };
        assert!(
            uv.iter().all(|t| (0.5 - 1e-5..=0.75 + 1e-5).contains(&t[0]) && (0.25 - 1e-5..=0.5 + 1e-5).contains(&t[1])),
            "{uv:?}"
        );
    }

    #[test]
    fn decal_over_an_edge_is_cut_off() {
        let s = wall();
        let (right, down) = basis(Vec3::Z);
        let m = project(&s, &decal(0.2), Vec3::new(1.95, 0.0, 0.0), Vec3::Z, right, down).unwrap();
        let max_x = positions(&m).iter().map(|p| p.x).fold(f32::MIN, f32::max);
        assert!((max_x - 2.0).abs() < 1e-4, "{max_x}");
    }

    #[test]
    fn nothing_out_of_reach_or_facing_away() {
        let s = wall();
        let (right, down) = basis(Vec3::Z);
        assert!(project(&s, &decal(0.1), Vec3::new(0.0, 0.0, 0.5), Vec3::Z, right, down).is_none());
        assert!(project(&s, &decal(0.1), Vec3::ZERO, Vec3::X, Vec3::Z, Vec3::NEG_Y).is_none());
    }
}
