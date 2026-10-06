//! Visibility sets for any game whose maps carry precomputed visibility
//! (Source's PVS, Quake-family maps): the map is divided into clusters, a
//! point query finds the camera's cluster, and each cluster lists the
//! clusters potentially visible from anywhere inside it. Map parts are
//! tagged with the clusters they touch (`VisClusters`) and hidden while
//! none of those is potentially visible from the camera's cluster.
//!
//! World meshes are merged per material by importers; `split_mesh` cuts
//! them into spatial chunks so each chunk has tight bounds (frustum
//! culling) and its own cluster list (visibility culling).
//!
//! `r_novis 1` turns culling off. Outside the map (a point in solid or in
//! no cluster) everything is drawn.

use bevy::prelude::*;

use super::MapMesh;

/// A map's visibility description, engine space (meters).
#[derive(Clone, Debug, Default)]
pub struct MapVisibility {
    /// Splitting planes: normal and distance (`n . p = d`).
    pub planes: Vec<(Vec3, f32)>,
    /// Plane index and children (front, back); a negative child `c` is
    /// leaf `-c - 1`.
    pub nodes: Vec<(usize, [i32; 2])>,
    /// Each leaf's cluster; negative for solid leaves and leaves outside
    /// every cluster.
    pub leaf_clusters: Vec<i32>,
    pub cluster_count: usize,
    /// Per cluster: the potentially visible clusters, as a bit set of
    /// `cluster_count` bits in 64-bit words. A cluster always sees itself.
    pub visible: Vec<Vec<u64>>,
}

impl MapVisibility {
    /// The cluster containing `p`, or None in solid or outside the map.
    pub fn cluster_at(&self, p: Vec3) -> Option<u32> {
        if self.nodes.is_empty() {
            return None;
        }
        let mut node = 0i32;
        while node >= 0 {
            let &(plane, children) = self.nodes.get(node as usize)?;
            let (n, d) = self.planes[plane];
            node = if n.dot(p) - d >= 0.0 { children[0] } else { children[1] };
        }
        let c = *self.leaf_clusters.get((-node - 1) as usize)?;
        (c >= 0).then_some(c as u32)
    }

    /// Every cluster whose leaves an axis-aligned box touches (a superset:
    /// a box straddling a plane, within `EPSILON`, visits both sides).
    pub fn clusters_in_box(&self, min: Vec3, max: Vec3, out: &mut Vec<u32>) {
        const EPSILON: f32 = 0.01;
        if self.nodes.is_empty() {
            return;
        }
        let centre = (min + max) * 0.5;
        let half = (max - min) * 0.5;
        let mut stack = vec![0i32];
        while let Some(node) = stack.pop() {
            if node < 0 {
                if let Some(&c) = self.leaf_clusters.get((-node - 1) as usize)
                    && c >= 0
                    && !out.contains(&(c as u32))
                {
                    out.push(c as u32);
                }
                continue;
            }
            let Some(&(plane, children)) = self.nodes.get(node as usize) else {
                continue;
            };
            let (n, d) = self.planes[plane];
            let dist = n.dot(centre) - d;
            let radius = n.abs().dot(half) + EPSILON;
            if dist > -radius {
                stack.push(children[0]);
            }
            if dist < radius {
                stack.push(children[1]);
            }
        }
    }

    /// Whether `to` is potentially visible from `from`.
    pub fn sees(&self, from: u32, to: u32) -> bool {
        self.visible
            .get(from as usize)
            .and_then(|row| row.get(to as usize / 64))
            .is_some_and(|w| w >> (to % 64) & 1 != 0)
    }

    /// Whether any of `clusters` is potentially visible from `from` (no
    /// clusters: always).
    pub fn sees_any(&self, from: u32, clusters: &[u32]) -> bool {
        if clusters.is_empty() {
            return true;
        }
        clusters.iter().any(|&c| self.sees(from, c))
    }
}

/// Decompress one run-length encoded visibility row (Quake/Source PVS: a
/// zero byte is followed by a count of zero bytes; other bytes are eight
/// clusters' bits, lowest bit first) into `cluster_count` bits.
pub fn decompress_row(data: &[u8], cluster_count: usize) -> Vec<u64> {
    let bytes_needed = cluster_count.div_ceil(8);
    let mut row = vec![0u64; cluster_count.div_ceil(64)];
    let (mut i, mut byte) = (0, 0);
    while byte < bytes_needed && i < data.len() {
        if data[i] == 0 {
            byte += data.get(i + 1).copied().unwrap_or(0) as usize;
            i += 2;
            continue;
        }
        for bit in 0..8 {
            let c = byte * 8 + bit;
            if c < cluster_count && data[i] >> bit & 1 != 0 {
                row[c / 64] |= 1 << (c % 64);
            }
        }
        byte += 1;
        i += 1;
    }
    row
}

/// Clusters a map part touches; it is drawn only while one of them is
/// potentially visible from the camera's cluster. Entities without it are
/// always drawn. The culling system owns their `Visibility` (except glow
/// sprites, which read `potentially_visible`).
#[derive(Component, Clone, Debug)]
pub struct VisClusters {
    pub clusters: Box<[u32]>,
    pub potentially_visible: bool,
}

impl VisClusters {
    pub fn new(clusters: Vec<u32>) -> Self {
        Self {
            clusters: clusters.into_boxed_slice(),
            potentially_visible: true,
        }
    }
}

/// Hidden when the camera is farther than this from the entity's origin,
/// meters (Source props' `fademaxdist`).
#[derive(Component, Clone, Copy, Debug)]
pub struct FadeDistance(pub f32);

/// The loaded map's visibility, when it has one.
#[derive(Resource, Clone)]
pub struct ActiveVisibility(pub std::sync::Arc<MapVisibility>);

/// `r_novis`: 1 draws everything (no visibility culling).
#[derive(Resource, Default, Clone, Copy)]
pub struct NoVis(pub u8);

/// What visibility culling did this frame, for `mashup_perf`.
#[derive(Resource, Default, Clone, Debug)]
pub struct VisStats {
    /// The camera's cluster (None: outside the map or culling off).
    pub cluster: Option<u32>,
    pub parts: usize,
    pub visible_parts: usize,
    /// Clusters potentially visible from the camera's.
    pub visible_clusters: usize,
    pub clusters: usize,
}

/// The world mesh chunk size, meters (a cube's edge). Chunks are cells of
/// this grid, per material: small enough that frustum and visibility
/// culling skip most of a map, large enough to keep draws few.
pub const CHUNK_SIZE: f32 = 512.0 * 0.0254;

/// The chunk size to spawn with: `CHUNK_SIZE`, or MASHUP_CHUNK_SIZE (in
/// Source units, for tuning measurements).
pub fn chunk_size() -> f32 {
    std::env::var("MASHUP_CHUNK_SIZE")
        .ok()
        .and_then(|v| v.parse::<f32>().ok())
        .filter(|v| *v > 0.0)
        .map_or(CHUNK_SIZE, |units| units * 0.0254)
}

/// Split a mesh into chunks by the grid cell of each triangle's centroid,
/// each with the clusters its triangles touch. With no visibility, or a
/// mesh in the 3D skybox, the mesh comes back whole with no clusters.
pub fn split_mesh(mesh: &MapMesh, vis: Option<&MapVisibility>, size: f32) -> Vec<(MapMesh, Vec<u32>)> {
    let Some(vis) = vis.filter(|_| !mesh.skybox) else {
        return vec![(mesh.clone(), Vec::new())];
    };
    let mut cells: std::collections::BTreeMap<[i32; 3], (Vec<u32>, Vec<u32>)> = Default::default();
    for tri in mesh.indices.chunks_exact(3) {
        let p = [0, 1, 2].map(|k| Vec3::from(mesh.positions[tri[k] as usize]));
        let centroid = (p[0] + p[1] + p[2]) / 3.0;
        let cell = (centroid / size).floor().as_ivec3().to_array();
        let (tris, clusters) = cells.entry(cell).or_default();
        tris.extend_from_slice(tri);
        let min = p[0].min(p[1]).min(p[2]);
        let max = p[0].max(p[1]).max(p[2]);
        vis.clusters_in_box(min, max, clusters);
    }
    // The mesh without its per-vertex data, copied into every chunk.
    let mut template = mesh.clone();
    template.indices.clear();
    template.positions.clear();
    template.normals.clear();
    template.uvs.clear();
    template.joints.clear();
    template.joint_weights.clear();
    template.lightmap_uvs.clear();
    template.blend_weights.clear();
    cells
        .into_values()
        .map(|(tris, mut clusters)| {
            clusters.sort_unstable();
            (sub_mesh(mesh, &template, &tris), clusters)
        })
        .collect()
}

/// The triangles `indices` of `mesh` as a mesh of their own (vertices
/// renumbered, every per-vertex attribute kept). `template` is the mesh
/// with its per-vertex data emptied (everything else is copied from it).
fn sub_mesh(mesh: &MapMesh, template: &MapMesh, indices: &[u32]) -> MapMesh {
    let mut remap: std::collections::HashMap<u32, u32> = Default::default();
    let mut used: Vec<usize> = Vec::new();
    let new_indices = indices
        .iter()
        .map(|&i| {
            *remap.entry(i).or_insert_with(|| {
                used.push(i as usize);
                used.len() as u32 - 1
            })
        })
        .collect();
    fn pick<T: Clone>(v: &[T], used: &[usize]) -> Vec<T> {
        if v.is_empty() {
            return Vec::new();
        }
        used.iter().map(|&i| v[i].clone()).collect()
    }
    MapMesh {
        positions: pick(&mesh.positions, &used),
        normals: pick(&mesh.normals, &used),
        uvs: pick(&mesh.uvs, &used),
        joints: pick(&mesh.joints, &used),
        joint_weights: pick(&mesh.joint_weights, &used),
        indices: new_indices,
        lightmap_uvs: pick(&mesh.lightmap_uvs, &used),
        blend_weights: pick(&mesh.blend_weights, &used),
        ..template.clone()
    }
}

/// Move a mesh's vertices so its bounds' centre is the origin; returns
/// that centre (where to place it).
pub fn recentre(mesh: &mut MapMesh) -> Vec3 {
    let (lo, hi) = mesh
        .positions
        .iter()
        .fold((Vec3::MAX, Vec3::MIN), |(a, b), p| (a.min(Vec3::from(*p)), b.max(Vec3::from(*p))));
    if lo.x > hi.x {
        return Vec3::ZERO;
    }
    let centre = (lo + hi) / 2.0;
    for p in &mut mesh.positions {
        *p = (Vec3::from(*p) - centre).to_array();
    }
    centre
}

/// The clusters a box touches, for tagging map parts.
pub fn box_clusters(vis: &MapVisibility, min: Vec3, max: Vec3) -> Vec<u32> {
    let mut out = Vec::new();
    vis.clusters_in_box(min, max, &mut out);
    out.sort_unstable();
    out
}

/// Each frame: find the camera's cluster; when it (or `r_novis`, or the
/// set of tagged parts) changes, show the parts it can potentially see and
/// hide the rest. Parts with a fade distance are checked every frame.
#[allow(clippy::type_complexity)]
pub(crate) fn cull(
    vis: Option<Res<ActiveVisibility>>,
    novis: Res<NoVis>,
    cameras: Query<
        (&GlobalTransform, &Camera, Has<super::water::WaterReflectionCamera>),
        (With<Camera3d>, Without<super::SkyboxCamera>, Without<super::ViewModelCamera>),
    >,
    mut queries: ParamSet<(
        Query<(), Added<VisClusters>>,
        Query<(
            &mut VisClusters,
            Option<&mut Visibility>,
            Has<super::GlowSprite>,
            Option<(&FadeDistance, &GlobalTransform)>,
        )>,
    )>,
    mut stats: ResMut<VisStats>,
    mut last: Local<Option<Option<Vec<u32>>>>,
) {
    let active: Vec<(Vec3, bool)> = cameras
        .iter()
        .filter(|(_, c, _)| c.is_active)
        .map(|(t, _, reflection)| (t.translation(), reflection))
        .collect();
    // The main view's eye (fade distances), and the clusters of every view
    // that draws the map: the main one and, while it draws, the water's
    // mirrored reflection camera. Any view outside the map (or r_novis 1)
    // draws everything: None.
    let eye = active.iter().find(|(_, r)| !r).map(|(p, _)| *p);
    let clusters: Option<Vec<u32>> = match (&vis, novis.0) {
        (Some(v), 0) if !active.is_empty() => active.iter().map(|(p, _)| v.0.cluster_at(*p)).collect(),
        _ => None,
    };
    let changed = last.as_ref() != Some(&clusters) || !queries.p0().is_empty();
    *last = Some(clusters.clone());
    let main = eye.and_then(|e| vis.as_ref().and_then(|v| v.0.cluster_at(e))).filter(|_| clusters.is_some());
    stats.cluster = main;
    stats.clusters = vis.as_ref().map_or(0, |v| v.0.cluster_count);
    if changed {
        debug!("visibility: camera at {eye:?}, view clusters {clusters:?}");
        stats.visible_clusters = match (main, &vis) {
            (Some(c), Some(v)) => v.0.visible[c as usize].iter().map(|w| w.count_ones() as usize).sum(),
            _ => stats.clusters,
        };
    }
    let (mut total, mut shown) = (0, 0);
    for (mut part, visibility, glow, fade) in &mut queries.p1() {
        if !changed && fade.is_none() {
            total += 1;
            shown += part.potentially_visible as usize;
            continue;
        }
        let in_pvs = match (&clusters, &vis) {
            (Some(from), Some(v)) => from.iter().any(|c| v.0.sees_any(*c, &part.clusters)),
            _ => true,
        };
        let near = match (fade, eye) {
            (Some((FadeDistance(far), at)), Some(eye)) => at.translation().distance(eye) <= *far,
            _ => true,
        };
        let on = in_pvs && near;
        total += 1;
        shown += on as usize;
        if part.potentially_visible != on {
            part.potentially_visible = on;
        }
        if !glow && let Some(mut v) = visibility {
            v.set_if_neq(if on { Visibility::Inherited } else { Visibility::Hidden });
        }
    }
    if changed {
        debug!("visibility: {shown} of {total} parts potentially visible");
    }
    stats.parts = total;
    stats.visible_parts = shown;
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Two rooms split by the plane x = 0: 0 <= x < 10 is cluster 0, x < 0
    /// cluster 1, x >= 10 is outside (solid). 0 sees 1, 1 sees only itself.
    fn two_rooms() -> MapVisibility {
        MapVisibility {
            planes: vec![(Vec3::X, 0.0), (Vec3::X, 10.0)],
            nodes: vec![(0, [1, -2]), (1, [-3, -1])],
            leaf_clusters: vec![0, 1, -1],
            cluster_count: 2,
            visible: vec![vec![0b11], vec![0b10]],
        }
    }

    #[test]
    fn finds_clusters_and_sets() {
        let v = two_rooms();
        assert_eq!(v.cluster_at(Vec3::new(1.0, 0.0, 0.0)), Some(0));
        assert_eq!(v.cluster_at(Vec3::new(-1.0, 0.0, 0.0)), Some(1));
        assert_eq!(v.cluster_at(Vec3::new(20.0, 0.0, 0.0)), None);
        assert!(v.sees(0, 1) && !v.sees(1, 0));
        assert_eq!(box_clusters(&v, Vec3::splat(-1.0), Vec3::splat(1.0)), vec![0, 1]);
        assert_eq!(box_clusters(&v, Vec3::splat(0.5), Vec3::splat(1.0)), vec![0]);
    }

    #[test]
    fn decompresses_run_length_rows() {
        // 20 clusters: 0x05 (clusters 0, 2), then one zero byte skipped,
        // then 0x08 (cluster 19).
        let row = decompress_row(&[0x05, 0x00, 0x01, 0x08], 20);
        let set: Vec<usize> = (0..20).filter(|&c| row[0] >> c & 1 != 0).collect();
        assert_eq!(set, vec![0, 2, 19]);
    }

    #[test]
    fn splits_meshes_by_cell_keeping_every_triangle() {
        let v = two_rooms();
        let mut mesh = MapMesh::default();
        for x in [-3.0f32, -1.0, 1.0, 3.0] {
            let base = mesh.positions.len() as u32;
            mesh.positions.extend([[x, 0.0, 0.0], [x + 0.5, 0.0, 0.0], [x, 0.5, 0.0]]);
            mesh.uvs.extend([[0.0, 0.0]; 3]);
            mesh.indices.extend([base, base + 1, base + 2]);
        }
        let chunks = split_mesh(&mesh, Some(&v), 2.0);
        assert_eq!(chunks.len(), 4);
        assert_eq!(chunks.iter().map(|(m, _)| m.indices.len()).sum::<usize>(), 12);
        for (m, clusters) in &chunks {
            assert_eq!(m.positions.len(), 3);
            assert_eq!(m.uvs.len(), 3);
            let x = m.positions[0][0];
            assert_eq!(clusters, &vec![if x < 0.0 { 1 } else { 0 }]);
        }
    }

    #[test]
    fn culling_follows_the_camera_and_shows_everything_outside() {
        let mut app = App::new();
        app.init_resource::<NoVis>()
            .init_resource::<VisStats>()
            .insert_resource(ActiveVisibility(std::sync::Arc::new(two_rooms())))
            .add_systems(Update, cull);
        let at = |x: f32| GlobalTransform::from_translation(Vec3::new(x, 0.0, 0.0));
        let camera = app.world_mut().spawn((Camera3d::default(), at(-1.0))).id();
        let in_0 = app.world_mut().spawn((VisClusters::new(vec![0]), Visibility::default())).id();
        let in_1 = app.world_mut().spawn((VisClusters::new(vec![1]), Visibility::default())).id();
        let shown = |app: &App| {
            [in_0, in_1].map(|e| app.world().get::<Visibility>(e) == Some(&Visibility::Inherited))
        };
        // From cluster 1 only cluster 1 is potentially visible.
        app.update();
        assert_eq!(shown(&app), [false, true]);
        // From cluster 0 both are.
        *app.world_mut().get_mut::<GlobalTransform>(camera).unwrap() = at(1.0);
        app.update();
        assert_eq!(shown(&app), [true, true]);
        // Back in 1, then outside the map: everything is drawn.
        *app.world_mut().get_mut::<GlobalTransform>(camera).unwrap() = at(-1.0);
        app.update();
        assert_eq!(shown(&app), [false, true]);
        *app.world_mut().get_mut::<GlobalTransform>(camera).unwrap() = at(20.0);
        app.update();
        assert_eq!(shown(&app), [true, true]);
        // r_novis 1 draws everything too.
        *app.world_mut().get_mut::<GlobalTransform>(camera).unwrap() = at(-1.0);
        app.update();
        assert_eq!(shown(&app), [false, true]);
        app.world_mut().resource_mut::<NoVis>().0 = 1;
        app.update();
        assert_eq!(shown(&app), [true, true]);
    }
}
