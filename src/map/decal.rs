//! Runtime decals (bullet holes, knife slashes) for any game: the map
//! carries decal definitions grouped by name (`MapDecals`); a `PlaceDecal`
//! message projects one onto the world triangles near the hit point,
//! clipped to the decal's rectangle, as the game projects decals onto the
//! faces they touch (specs/cs_source/overlays_decals.md). Drawn with
//! `DecalMaterial`: the surface is multiplied by the decal (modulate 2x).
//! A hit prop or brush entity (door, breakable, func_brush) takes the decal
//! on its own triangles, as its child: it moves with it, and goes when the
//! entity is removed or broken (and at a round restart, which re-creates
//! brush entities). Characters take no decals yet. They stay until the
//! map changes, as in the game; `ClearDecals` (the `r_cleardecals`
//! command) removes them all, and `DecalSettings::round_clear`
//! (`mashup_round_cleardecals 1`) at every round start.

use std::collections::{HashMap, HashSet, VecDeque};

use bevy::{
    asset::{RenderAssetUsages, embedded_asset},
    mesh::{Indices, PrimitiveTopology},
    prelude::*,
    reflect::TypePath,
    render::render_resource::{AsBindGroup, BlendComponent, BlendFactor, BlendOperation, BlendState, ColorWrites},
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
/// How far below the decal its depth-only copy lies (meters): that copy
/// (0.30 units up) keeps overlays (up to 0.25) off impacts while every
/// impact's colour (0.35) still passes over every other impact's depth.
const DEPTH_DROP: f32 = 0.05 * 0.0254;
const CELL: f32 = 1.0;

/// Triangles (corners and normal) in a uniform grid; those spanning more
/// than `BIG_CELLS` cells in a list of their own with their bounds (a surf
/// map's 100 m ramp filled millions of cells: a minute and gigabytes).
#[derive(Default)]
pub(super) struct TriSet {
    tris: Vec<[Vec3; 4]>,
    cells: HashMap<IVec3, Vec<u32>>,
    big: Vec<(u32, Vec3, Vec3)>,
}

/// Grid cells a triangle may span before it goes in the big list.
const BIG_CELLS: i32 = 64;

/// What decals can land on: the world's triangles, each prop model's in
/// its own space, and each brush entity's in its node's space.
#[derive(Resource, Default)]
pub(super) struct DecalSurfaces {
    world: TriSet,
    models: Vec<TriSet>,
    /// Prop index -> model index.
    props: Vec<usize>,
    /// Map entity index -> its brush model's triangles.
    entities: HashMap<usize, TriSet>,
}

/// Remove every runtime decal (`r_cleardecals`).
#[derive(Message, Clone, Copy, Debug, Default)]
pub struct ClearDecals;

/// Decal options: `round_clear` (`mashup_round_cleardecals`, default 0)
/// removes every runtime decal at each round start. The game keeps them
/// until the map changes.
#[derive(Resource, Default)]
pub struct DecalSettings {
    pub round_clear: u8,
}

/// A placed runtime decal (its colour part; the depth-only copy is its
/// child).
#[derive(Component)]
pub struct RuntimeDecal;

fn cell(p: Vec3) -> IVec3 {
    (p / CELL).floor().as_ivec3()
}

impl DecalSurfaces {
    pub(super) fn new(data: &super::MapData) -> Self {
        let mut by_entity: HashMap<usize, Vec<&MapMesh>> = HashMap::new();
        for m in &data.meshes {
            if let Some(e) = m.entity {
                by_entity.entry(e).or_default().push(m);
            }
        }
        Self {
            world: TriSet::new(&data.meshes),
            models: data.models.iter().map(|m| TriSet::new(&m.meshes)).collect(),
            props: data.props.iter().map(|p| p.model).collect(),
            entities: by_entity
                .into_iter()
                .map(|(e, list)| (e, TriSet::from_meshes(list.into_iter(), true)))
                .collect(),
        }
    }
}

impl TriSet {
    /// Opaque surfaces (not the 3D skybox, other decals or overlays,
    /// unlit or blended surfaces), without brush entities' meshes (local to
    /// their entity, not world space).
    pub(super) fn new(meshes: &[MapMesh]) -> Self {
        Self::from_meshes(meshes.iter(), false)
    }

    /// The opaque surfaces of `meshes`, which are a brush entity's own
    /// (node space) with `entity`, else world or model meshes.
    fn from_meshes<'a>(meshes: impl Iterator<Item = &'a MapMesh>, entity: bool) -> Self {
        let mut out = Self::default();
        for m in meshes {
            if m.skybox
                || m.entity.is_some() != entity
                || m.pane_look != super::PaneLook::Whole
                || m.unlit
                || m.material.starts_with("decal:")
                || matches!(m.alpha, super::MapAlpha::Blend | super::MapAlpha::Add)
            {
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
                let span = hi - lo + IVec3::ONE;
                if span.x * span.y * span.z > BIG_CELLS {
                    out.big.push((i, a.min(b).min(c), a.max(b).max(c)));
                    continue;
                }
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
        let (qlo, qhi) = (p - Vec3::splat(r), p + Vec3::splat(r));
        seen.extend(
            self.big
                .iter()
                .filter(|(_, lo, hi)| lo.cmple(qhi + Vec3::splat(CELL)).all() && hi.cmpge(qlo - Vec3::splat(CELL)).all())
                .map(|(i, ..)| *i),
        );
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
    mesh.insert_attribute(
        Mesh::ATTRIBUTE_UV_0,
        uvs.into_iter().map(|v| v.to_array()).collect::<Vec<_>>(),
    );
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
    /// (decal index, depth only) -> material.
    materials: HashMap<(usize, bool), Handle<DecalMaterial>>,
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
    brushes: Query<(&super::MapBrushEntity, &GlobalTransform, Has<super::vis::LogicHidden>)>,
    parents: Query<&ChildOf>,
    assets: Option<ResMut<DecalAssets>>,
    meshes: Option<ResMut<Assets<Mesh>>>,
    materials: Option<ResMut<Assets<DecalMaterial>>>,
    fog: Option<Res<super::fog::SceneFog>>,
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
        // On a prop: its model, in its own space, as its child; on a brush
        // entity: its brushes, in its node's space, as the node's child. A
        // hit collider may be the prop or a child of it.
        let hit = |e: Entity| std::iter::once(e).chain(parents.get(e).ok().map(|c| c.parent()));
        let prop = ask.target.and_then(|t| {
            hit(t).find_map(|e| props.get(e).ok().map(|(i, at)| (e, i.0, at.affine().inverse())))
        });
        let brush = ask
            .target
            .filter(|_| prop.is_none())
            .and_then(|t| hit(t).find_map(|e| brushes.get(e).ok().map(|(b, at, gone)| (e, b.0, at, gone))));
        // A removed or broken brush entity takes none (nor does the world
        // behind where it was).
        if brush.is_some_and(|b| b.3) {
            continue;
        }
        let to_local = prop
            .map(|p| p.2)
            .or(brush.map(|b| b.2.affine().inverse()));
        let set = match (prop, brush) {
            (Some((_, index, _)), _) => surfaces.props.get(index).and_then(|m| surfaces.models.get(*m)),
            (None, Some((_, index, _, _))) => surfaces.entities.get(&index),
            (None, None) => Some(&surfaces.world),
        };
        let Some(set) = set else { continue };
        let parent = prop.map(|p| p.0).or(brush.map(|b| b.0));
        let projected = match to_local {
            Some(to_local) => {
                let local = |v: Vec3| to_local.transform_vector3(v).normalize_or_zero();
                project(
                    set,
                    &decal,
                    to_local.transform_point3(ask.point),
                    local(ask.normal),
                    local(right),
                    local(down),
                )
                .map(|m| (m, parent))
            }
            None => project(set, &decal, ask.point, ask.normal, right, down).map(|m| (m, None)),
        };
        let Some((mesh, parent)) = projected else {
            continue;
        };
        let Some(texture) = assets.textures.get(decal.texture).cloned() else {
            continue;
        };
        let mut material = |depth_only: bool| match assets.materials.get(&(index, depth_only)) {
            Some(m) => m.clone(),
            None => {
                let m = materials.add(DecalMaterial {
                    params: Vec4::new(
                        if decal.blend == DecalBlend::Modulate2x {
                            1.0
                        } else {
                            0.0
                        },
                        0.0,
                        0.0,
                        0.0,
                    ),
                    texture: texture.clone(),
                    fog: fog.as_ref().map_or_else(Default::default, |f| f.0),
                    depth_only,
                });
                assets.materials.insert((index, depth_only), m.clone());
                m
            }
        };
        let (colour, depth) = (material(false), material(true));
        let mesh = meshes.add(mesh);
        // The decal's surface normal where it lies (the prop's space for
        // decals on props).
        let drop = match to_local {
            Some(to_local) => to_local.transform_vector3(ask.normal).normalize_or_zero(),
            None => ask.normal.normalize_or_zero(),
        } * -DEPTH_DROP;
        let mut decal_entity = commands.spawn((
            Name::new("Decal"),
            RuntimeDecal,
            Mesh3d(mesh.clone()),
            MeshMaterial3d(colour),
            Transform::default(),
            Visibility::default(),
            MapPart,
            children![(
                Name::new("Decal depth"),
                Mesh3d(mesh),
                MeshMaterial3d(depth),
                Transform::from_translation(drop),
                Visibility::default(),
                MapPart,
            )],
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

/// Decals on brush entities go with them: when the logic removes one
/// (killed, broken: `vis::LogicHidden` added to its node) and at a round
/// restart, which re-creates them.
#[allow(clippy::type_complexity)]
pub(super) fn drop_brush_decals(
    gone: Query<&Children, (With<super::MapBrushEntity>, Added<super::vis::LogicHidden>)>,
    nodes: Query<&Children, With<super::MapBrushEntity>>,
    decals: Query<(), With<RuntimeDecal>>,
    restarts: Option<Res<crate::core::RoundRestarts>>,
    mut seen: Local<Option<u32>>,
    assets: Option<ResMut<DecalAssets>>,
    mut commands: Commands,
) {
    let count = restarts.map_or(0, |r| r.0);
    let restart = seen.replace(count).is_some_and(|s| s != count);
    let mut removed = Vec::new();
    let lists: Vec<&Children> = if restart { nodes.iter().collect() } else { gone.iter().collect() };
    for children in lists {
        for child in children.iter().filter(|c| decals.contains(*c)) {
            commands.entity(child).try_despawn();
            removed.push(child);
        }
    }
    if let Some(mut assets) = assets.filter(|_| !removed.is_empty()) {
        assets.placed.retain(|e| !removed.contains(e));
    }
}

/// `ClearDecals`, and round starts with `DecalSettings::round_clear` on:
/// every runtime decal goes (world, props and brush entities alike).
pub(super) fn clear_decals(
    mut asks: MessageReader<ClearDecals>,
    settings: Option<Res<DecalSettings>>,
    restarts: Option<Res<crate::core::RoundRestarts>>,
    mut seen: Local<Option<u32>>,
    decals: Query<Entity, With<RuntimeDecal>>,
    assets: Option<ResMut<DecalAssets>>,
    mut commands: Commands,
) {
    let count = restarts.map_or(0, |r| r.0);
    let restart = seen.replace(count).is_some_and(|s| s != count);
    let asked = asks.read().count() > 0;
    if !asked && !(restart && settings.is_some_and(|s| s.round_clear != 0)) {
        return;
    }
    for e in &decals {
        commands.entity(e).try_despawn();
    }
    if let Some(mut assets) = assets {
        assets.placed.clear();
    }
}

/// Each decal is drawn twice (see `DecalMaterial::specialize`): its
/// colour, and a depth-only copy just below it.
#[derive(Asset, TypePath, AsBindGroup, Debug, Clone)]
#[bind_group_data(DecalMaterialKey)]
pub struct DecalMaterial {
    /// x: 1 for modulate 2x, 0 for a stain.
    #[uniform(0)]
    pub params: Vec4,
    #[texture(1)]
    #[sampler(2)]
    pub texture: Handle<Image>,
    /// World and water fog (`fog::SceneFog`): fogged decals fade to
    /// neutral.
    #[uniform(3)]
    pub fog: super::fog::FogUniform,
    /// Writes depth only (no colour): the copy that keeps overlays off.
    pub depth_only: bool,
}

/// Pipeline key data: the depth-only copy has its own pipeline.
#[repr(C)]
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct DecalMaterialKey {
    depth_only: bool,
}

impl From<&DecalMaterial> for DecalMaterialKey {
    fn from(m: &DecalMaterial) -> Self {
        Self { depth_only: m.depth_only }
    }
}

impl Material for DecalMaterial {
    fn fragment_shader() -> ShaderRef {
        "embedded://mashup/map/decal.wgsl".into()
    }

    /// Drawn right after the opaque world (alpha-mask phase), before
    /// anything see-through, so particles, smoke, dust and muzzle flashes
    /// draw over impacts rather than being sorted against them.
    fn alpha_mode(&self) -> AlphaMode {
        AlphaMode::Mask(0.001)
    }

    /// Kept out of the depth prepass (main views get one on maps with
    /// water): written there, a decal's lifted depth would make the
    /// surface under it fail the depth test in the main pass, and the
    /// decal would then multiply whatever was drawn before (the sky).
    fn enable_prepass() -> bool {
        false
    }

    fn enable_shadows() -> bool {
        false
    }

    /// The framebuffer times twice the shader's output, without depth
    /// writes: every impact's colour passes over every other's, and the
    /// product doesn't depend on the order they are drawn in, which is the
    /// game's result of applying them in creation order (they all
    /// multiply). The depth-only copy (0.05 units lower, discarding the
    /// same neutral texels) writes depth where the decal shows, so the
    /// map's overlays and decals (lifted less) never draw over an impact,
    /// whichever draws first. (Impacts writing depth themselves made
    /// overlapping ones hide each other by draw order, which flickered.)
    fn specialize(
        _pipeline: &bevy::pbr::MaterialPipeline,
        descriptor: &mut bevy::render::render_resource::RenderPipelineDescriptor,
        _layout: &bevy::mesh::MeshVertexBufferLayoutRef,
        key: bevy::pbr::MaterialPipelineKey<Self>,
    ) -> Result<(), bevy::render::render_resource::SpecializedMeshPipelineError> {
        let depth_only = key.bind_group_data.depth_only;
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
                if depth_only {
                    target.write_mask = ColorWrites::empty();
                }
            }
        }
        if let Some(depth) = descriptor.depth_stencil.as_mut() {
            depth.depth_write_enabled = Some(depth_only);
        }
        Ok(())
    }
}

pub struct DecalMaterialPlugin;

impl Plugin for DecalMaterialPlugin {
    fn build(&self, app: &mut App) {
        embedded_asset!(app, "decal.wgsl");
        app.add_plugins((super::fog::FogShaderPlugin, MaterialPlugin::<DecalMaterial>::default()));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn impacts_stay_out_of_the_depth_prepass() {
        // Water maps' prepass would otherwise hide the wall under a decal.
        assert!(!<DecalMaterial as Material>::enable_prepass());
    }

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
        assert!(
            right.abs_diff_eq(Vec3::X, 1e-6) && down.abs_diff_eq(Vec3::NEG_Y, 1e-6),
            "{right} {down}"
        );
        let m = project(&s, &decal(0.1), Vec3::new(0.3, 0.2, 0.0), Vec3::Z, right, down).expect("a decal");
        let p = positions(&m);
        let (lo, hi) = p
            .iter()
            .fold((Vec3::MAX, Vec3::MIN), |(lo, hi), v| (lo.min(*v), hi.max(*v)));
        assert!(lo.abs_diff_eq(Vec3::new(0.25, 0.15, LIFT), 1e-4), "{lo}");
        assert!(hi.abs_diff_eq(Vec3::new(0.35, 0.25, LIFT), 1e-4), "{hi}");
        // Texture coordinates stay inside the atlas rectangle.
        let Some(bevy::mesh::VertexAttributeValues::Float32x2(uv)) = m.attribute(Mesh::ATTRIBUTE_UV_0) else {
            panic!("no uvs")
        };
        assert!(
            uv.iter()
                .all(|t| (0.5 - 1e-5..=0.75 + 1e-5).contains(&t[0]) && (0.25 - 1e-5..=0.5 + 1e-5).contains(&t[1])),
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
