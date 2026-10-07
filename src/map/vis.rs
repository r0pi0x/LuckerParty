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
//! Areaportals (Source's func_areaportal, func_areaportalwindow) divide a
//! map into areas: a closed portal (a shut door) hides the areas only
//! reachable through it, and an open one is looked through, so only what
//! its opening's screen rectangle covers is reached beyond it
//! (`MapAreas::flood`). Map parts are drawn when a camera's cluster can
//! potentially see one of their clusters and that cluster lies in an area
//! the camera reaches.
//!
//! `r_novis 1` turns culling off, `r_portalsopenall 1` area culling only.
//! Outside the map (a point in solid or in no cluster) everything is drawn.

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
    /// Areas and the portals between them; empty for maps without them.
    pub areas: MapAreas,
}

impl MapVisibility {
    /// The leaf containing `p`, or None outside the tree.
    pub fn leaf_at(&self, p: Vec3) -> Option<usize> {
        if self.nodes.is_empty() {
            return None;
        }
        let mut node = 0i32;
        while node >= 0 {
            let &(plane, children) = self.nodes.get(node as usize)?;
            let (n, d) = self.planes[plane];
            node = if n.dot(p) - d >= 0.0 { children[0] } else { children[1] };
        }
        Some((-node - 1) as usize)
    }

    /// The cluster containing `p`, or None in solid or outside the map.
    pub fn cluster_at(&self, p: Vec3) -> Option<u32> {
        let c = *self.leaf_clusters.get(self.leaf_at(p)?)?;
        (c >= 0).then_some(c as u32)
    }

    /// The area containing `p` (0: none, or a map without areas).
    pub fn area_at(&self, p: Vec3) -> u16 {
        self.leaf_at(p)
            .and_then(|l| self.areas.leaf_areas.get(l).copied())
            .unwrap_or(0)
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

/// A map's areas: groups of leaves that areaportals separate (Source's
/// areas lump; area 0 is "none"). Each portal joins two areas through a
/// convex opening.
#[derive(Clone, Debug, Default)]
pub struct MapAreas {
    /// Each leaf's area, indexed like `MapVisibility::leaf_clusters`.
    pub leaf_areas: Vec<u16>,
    pub portals: Vec<AreaPortal>,
    /// Per area: (portal, the area on its other side).
    pub links: Vec<Vec<(u32, u16)>>,
    /// Per cluster: the areas its leaves are in, sorted.
    pub cluster_areas: Vec<Vec<u16>>,
}

/// An opening between two areas.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct AreaPortal {
    /// What map logic calls it by (Source: the portal entity's
    /// `portalnumber`).
    pub key: u16,
    pub areas: [u16; 2],
    /// The opening, a convex polygon (engine space, meters); empty when
    /// unknown (then looked through whole).
    pub polygon: Vec<Vec3>,
    /// A window (func_areaportalwindow): the distance from the opening
    /// beyond which the game closes it and draws the window's brush
    /// opaque instead (meters). Not applied yet: the window brush doesn't
    /// fade in, so closing it would show a hole (docs/tech-debt.md).
    pub fade: Option<f32>,
    /// What closes it can be seen through (a door with a window): never
    /// closed, so the glass doesn't show a hole.
    pub see_through: bool,
}

/// A rectangle in normalized device coordinates: min x, min y, max x,
/// max y.
pub type ScreenRect = [f32; 4];

const FULL_SCREEN: ScreenRect = [-1.0, -1.0, 1.0, 1.0];

/// Slack around a portal's projected rectangle (NDC), for rasterization.
const PORTAL_MARGIN: f32 = 0.01;

impl MapAreas {
    /// Build the links and each cluster's areas. `leaf_clusters` as in
    /// `MapVisibility`.
    pub fn new(leaf_areas: Vec<u16>, leaf_clusters: &[i32], cluster_count: usize, portals: Vec<AreaPortal>) -> Self {
        let area_count = portals
            .iter()
            .flat_map(|p| p.areas)
            .chain(leaf_areas.iter().copied())
            .max()
            .map_or(0, |a| a as usize + 1);
        let mut links = vec![Vec::new(); area_count];
        for (i, p) in portals.iter().enumerate() {
            let [a, b] = p.areas;
            links[a as usize].push((i as u32, b));
            links[b as usize].push((i as u32, a));
        }
        let mut cluster_areas: Vec<Vec<u16>> = vec![Vec::new(); cluster_count];
        for (leaf, &c) in leaf_clusters.iter().enumerate() {
            if let (Ok(c), Some(&a)) = (usize::try_from(c), leaf_areas.get(leaf))
                && let Some(areas) = cluster_areas.get_mut(c)
                && !areas.contains(&a)
            {
                areas.push(a);
            }
        }
        for a in &mut cluster_areas {
            a.sort_unstable();
        }
        Self {
            leaf_areas,
            portals,
            links,
            cluster_areas,
        }
    }

    pub fn area_count(&self) -> usize {
        self.links.len()
    }

    /// Mark portals with a see-through surface in their opening (glass,
    /// grates: any mesh not opaque with a triangle within `SEE_THROUGH_REACH`
    /// of the opening's box) as `see_through`: closing them would show a
    /// hole through it.
    pub fn mark_see_through(&mut self, meshes: &[MapMesh]) {
        const SEE_THROUGH_REACH: f32 = 0.25;
        for portal in &mut self.portals {
            let Some(first) = portal.polygon.first() else { continue };
            let (lo, hi) = portal.polygon.iter().fold((*first, *first), |(a, b), p| (a.min(*p), b.max(*p)));
            let (lo, hi) = (lo - SEE_THROUGH_REACH, hi + SEE_THROUGH_REACH);
            portal.see_through = meshes
                .iter()
                .filter(|m| !m.skybox && m.alpha != super::MapAlpha::Opaque)
                .any(|m| {
                    m.indices.chunks_exact(3).any(|t| {
                        let p = [0, 1, 2].map(|k| Vec3::from(m.positions[t[k] as usize]));
                        let (a, b) = (p[0].min(p[1]).min(p[2]), p[0].max(p[1]).max(p[2]));
                        a.cmple(hi).all() && b.cmpge(lo).all()
                    })
                });
        }
    }

    /// The areas a camera in area `from` can see into: those reached
    /// through portals `open` passes. With `clip_from_world` (the camera's
    /// view projection), a portal is looked through only where its screen
    /// rectangle overlaps the rectangle it was reached by, starting from
    /// the whole screen. Area 0, or one out of range, reaches everything;
    /// area 0 (leaves in no area) is always reached.
    pub fn flood(&self, from: u16, open: &dyn Fn(&AreaPortal) -> bool, clip_from_world: Option<Mat4>) -> Vec<bool> {
        let n = self.area_count();
        if from == 0 || from as usize >= n {
            return vec![true; n];
        }
        let mut rects: Vec<Option<ScreenRect>> = vec![None; n];
        rects[from as usize] = Some(FULL_SCREEN);
        let mut stack = vec![from];
        // Rectangles only grow, so this ends; the budget is a guard.
        let mut budget = 64 * n + 64;
        while let Some(a) = stack.pop() {
            if budget == 0 {
                warn!("areaportal flood gave up; drawing every area");
                return vec![true; n];
            }
            budget -= 1;
            let Some(rect) = rects[a as usize] else { continue };
            for &(portal, to) in &self.links[a as usize] {
                let portal = &self.portals[portal as usize];
                if !open(portal) {
                    continue;
                }
                let through = match clip_from_world {
                    Some(m) => portal_rect(&portal.polygon, m).and_then(|r| intersect(rect, r)),
                    None => Some(rect),
                };
                let Some(through) = through else { continue };
                let grown = match rects[to as usize] {
                    Some(old) => union(old, through),
                    None => through,
                };
                if rects[to as usize] != Some(grown) {
                    rects[to as usize] = Some(grown);
                    stack.push(to);
                }
            }
        }
        let mut seen: Vec<bool> = rects.iter().map(Option::is_some).collect();
        seen[0] = true;
        seen
    }

    /// The clusters with a leaf in a `reached` area (or in area 0, or with
    /// no known area), as a bit set of `cluster_count` bits.
    pub fn cluster_mask(&self, reached: &[bool], cluster_count: usize) -> Vec<u64> {
        let mut mask = vec![0u64; cluster_count.div_ceil(64)];
        for c in 0..cluster_count {
            let on = match self.cluster_areas.get(c) {
                Some(areas) if !areas.is_empty() => {
                    areas.iter().any(|&a| a == 0 || reached.get(a as usize).is_none_or(|r| *r))
                }
                _ => true,
            };
            if on {
                mask[c / 64] |= 1 << (c % 64);
            }
        }
        mask
    }
}

/// A polygon's screen rectangle under `clip_from_world` (with a margin),
/// clamped to the screen; None when it is wholly behind the eye or off
/// screen. A polygon crossing the eye's plane (or an empty one) covers the
/// whole screen.
pub fn portal_rect(polygon: &[Vec3], clip_from_world: Mat4) -> Option<ScreenRect> {
    if polygon.is_empty() {
        return Some(FULL_SCREEN);
    }
    let (mut lo, mut hi) = (Vec2::MAX, Vec2::MIN);
    let mut behind = 0;
    for p in polygon {
        let c = clip_from_world * p.extend(1.0);
        if c.w <= 1e-4 {
            behind += 1;
            continue;
        }
        let ndc = c.truncate().truncate() / c.w;
        lo = lo.min(ndc);
        hi = hi.max(ndc);
    }
    if behind == polygon.len() {
        return None;
    }
    if behind > 0 {
        return Some(FULL_SCREEN);
    }
    intersect(
        FULL_SCREEN,
        [lo.x - PORTAL_MARGIN, lo.y - PORTAL_MARGIN, hi.x + PORTAL_MARGIN, hi.y + PORTAL_MARGIN],
    )
}

fn intersect(a: ScreenRect, b: ScreenRect) -> Option<ScreenRect> {
    let r = [a[0].max(b[0]), a[1].max(b[1]), a[2].min(b[2]), a[3].min(b[3])];
    (r[0] <= r[2] && r[1] <= r[3]).then_some(r)
}

fn union(a: ScreenRect, b: ScreenRect) -> ScreenRect {
    [a[0].min(b[0]), a[1].min(b[1]), a[2].max(b[2]), a[3].max(b[3])]
}

/// Areaportals map logic closed (by key); every other portal is open.
/// Written by the logic (linked doors, Open/Close inputs), read by `cull`.
#[derive(Resource, Clone, Debug, Default, PartialEq, Eq)]
pub struct AreaPortalStates {
    /// Sorted.
    pub closed: Vec<u16>,
}

impl AreaPortalStates {
    pub fn is_open(&self, key: u16) -> bool {
        self.closed.binary_search(&key).is_err()
    }
}

/// `r_portalsopenall`: 1 treats every areaportal as open and doesn't look
/// through them (PVS culling only).
#[derive(Resource, Default, Clone, Copy)]
pub struct PortalsOpenAll(pub u8);

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

/// A map part game logic removed or turned off (a broken or killed prop):
/// culling keeps it hidden. Whoever adds it hides the part; whoever
/// removes it shows it again.
#[derive(Component, Clone, Copy, Debug, Default)]
pub struct LogicHidden;

/// Hidden when the camera is farther than this from the entity's origin,
/// meters (Source props' `fademaxdist`).
#[derive(Component, Clone, Copy, Debug)]
pub struct FadeDistance(pub f32);

/// A prop mesh's fade band (Source's `fademindist` to `fademaxdist`,
/// meters) as a Bevy visibility range: drawn fully nearer than `near`,
/// dithered out (screen-door, in the opaque passes: no sorting) toward
/// `far`, gone beyond it. The distance is from the view to the mesh's
/// origin (the prop's), per view. None without a band (`near` negative,
/// "use fademaxdist", or not below `far`): the prop pops at `far`.
/// Prop shaders apply the dither (`prop.wgsl`, `prop_prepass.wgsl`).
pub fn fade_band(near: f32, far: f32) -> Option<bevy::camera::visibility::VisibilityRange> {
    (near >= 0.0 && near < far).then(|| bevy::camera::visibility::VisibilityRange {
        start_margin: 0.0..0.0,
        end_margin: near..far,
        use_aabb: false,
    })
}

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
    /// Clusters drawn from: potentially visible from a camera's cluster
    /// and in an area it reaches.
    pub visible_clusters: usize,
    pub clusters: usize,
    /// The main camera's area (0: none) and how many areas it reaches.
    pub area: u16,
    pub visible_areas: usize,
    pub areas: usize,
    /// Areaportals closed by map logic.
    pub closed_portals: usize,
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

/// What one camera draws from: the clusters potentially visible from its
/// cluster that lie in an area it reaches through open areaportals (a bit
/// set), its area and how many areas it reaches. None outside the map.
/// `clip_from_world` narrows the areas to those seen through portal
/// openings; `open_all` ignores areaportals.
pub fn camera_clusters(
    v: &MapVisibility,
    eye: Vec3,
    clip_from_world: Option<Mat4>,
    states: &AreaPortalStates,
    open_all: bool,
) -> Option<(Vec<u64>, u16, usize)> {
    let leaf = v.leaf_at(eye)?;
    let cluster = usize::try_from(*v.leaf_clusters.get(leaf)?).ok()?;
    let mut row = v.visible.get(cluster)?.clone();
    let areas = &v.areas;
    let area = areas.leaf_areas.get(leaf).copied().unwrap_or(0);
    if open_all || area == 0 || areas.portals.is_empty() {
        return Some((row, area, areas.area_count()));
    }
    let open = |p: &AreaPortal| p.see_through || states.is_open(p.key);
    let reached = areas.flood(area, &open, clip_from_world);
    for (w, m) in row.iter_mut().zip(areas.cluster_mask(&reached, v.cluster_count)) {
        *w &= m;
    }
    Some((row, area, reached.iter().filter(|r| **r).count()))
}

/// Each frame: find what each camera draws from (`camera_clusters`); when
/// that (or `r_novis`, or the set of tagged parts) changes, show the parts
/// in those clusters and hide the rest. Parts with a fade distance are
/// checked every frame.
#[allow(clippy::type_complexity, clippy::too_many_arguments)]
pub(crate) fn cull(
    vis: Option<Res<ActiveVisibility>>,
    novis: Res<NoVis>,
    open_all: Option<Res<PortalsOpenAll>>,
    portal_states: Option<Res<AreaPortalStates>>,
    cameras: Query<
        (&GlobalTransform, &Camera, Option<&Projection>, Has<super::water::WaterReflectionCamera>),
        (With<Camera3d>, Without<super::SkyboxCamera>, Without<super::ViewModelCamera>),
    >,
    mut queries: ParamSet<(
        Query<(), Added<VisClusters>>,
        Query<(
            &mut VisClusters,
            Option<&mut Visibility>,
            Has<super::GlowSprite>,
            Option<(&FadeDistance, &GlobalTransform)>,
            Has<LogicHidden>,
        )>,
    )>,
    mut stats: ResMut<VisStats>,
    mut last: Local<Option<Option<Vec<u64>>>>,
) {
    // Every view that draws the map: the main one and, while it draws, the
    // water's mirrored reflection camera (whole areas: its projection is
    // mirrored).
    let active: Vec<(Vec3, Option<Mat4>, bool)> = cameras
        .iter()
        .filter(|(_, c, _, _)| c.is_active)
        .map(|(t, _, projection, reflection)| {
            let clip = projection
                .filter(|_| !reflection)
                .map(|p| p.get_clip_from_view() * Mat4::from(t.affine().inverse()));
            (t.translation(), clip, reflection)
        })
        .collect();
    let eye = active.iter().find(|(_, _, r)| !r).map(|(p, _, _)| *p);
    let default_states = AreaPortalStates::default();
    let states = portal_states.as_deref().unwrap_or(&default_states);
    let open_all = open_all.is_some_and(|o| o.0 != 0);
    // The union of the views' clusters; any view outside the map (or
    // r_novis 1) draws everything: None.
    let mut main = None;
    let drawn: Option<Vec<u64>> = match (&vis, novis.0) {
        (Some(v), 0) if !active.is_empty() => {
            let mut union = vec![0u64; v.0.cluster_count.div_ceil(64)];
            let mut inside = true;
            for (p, clip, reflection) in &active {
                let Some((row, area, areas)) = camera_clusters(&v.0, *p, *clip, states, open_all) else {
                    inside = false;
                    break;
                };
                if !reflection {
                    main = Some((v.0.cluster_at(*p), area, areas));
                }
                for (u, w) in union.iter_mut().zip(row) {
                    *u |= w;
                }
            }
            inside.then_some(union)
        }
        _ => None,
    };
    let changed = last.as_ref() != Some(&drawn) || !queries.p0().is_empty();
    stats.clusters = vis.as_ref().map_or(0, |v| v.0.cluster_count);
    stats.areas = vis.as_ref().map_or(0, |v| v.0.areas.area_count());
    stats.closed_portals = states.closed.len();
    let main = main.filter(|_| drawn.is_some());
    stats.cluster = main.and_then(|m| m.0);
    stats.area = main.map_or(0, |m| m.1);
    stats.visible_areas = main.map_or(stats.areas, |m| m.2);
    if changed {
        debug!("visibility: camera at {eye:?}, area {}, {} areas reached", stats.area, stats.visible_areas);
        stats.visible_clusters = match &drawn {
            Some(d) => d.iter().map(|w| w.count_ones() as usize).sum(),
            None => stats.clusters,
        };
    }
    let in_set = |clusters: &[u32]| match &drawn {
        Some(d) => {
            clusters.is_empty()
                || clusters
                    .iter()
                    .any(|&c| d.get(c as usize / 64).is_some_and(|w| w >> (c % 64) & 1 != 0))
        }
        None => true,
    };
    let (mut total, mut shown) = (0, 0);
    for (mut part, visibility, glow, fade, removed) in &mut queries.p1() {
        if !changed && fade.is_none() {
            total += 1;
            shown += part.potentially_visible as usize;
            continue;
        }
        let near = match (fade, eye) {
            (Some((FadeDistance(far), at)), Some(eye)) => at.translation().distance(eye) <= *far,
            _ => true,
        };
        let on = near && in_set(&part.clusters);
        total += 1;
        shown += on as usize;
        if part.potentially_visible != on {
            part.potentially_visible = on;
        }
        if !glow && let Some(mut v) = visibility {
            v.set_if_neq(if on && !removed { Visibility::Inherited } else { Visibility::Hidden });
        }
    }
    *last = Some(drawn);
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
            areas: MapAreas::default(),
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

    /// A square opening (side 2) centred at `c`, facing along z.
    fn opening(c: Vec3) -> Vec<Vec3> {
        [(-1.0, -1.0), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)]
            .map(|(x, y)| c + Vec3::new(x, y, 0.0))
            .to_vec()
    }

    fn portal(key: u16, a: u16, b: u16, polygon: Vec<Vec3>) -> AreaPortal {
        AreaPortal {
            key,
            areas: [a, b],
            polygon,
            fade: None,
            see_through: false,
        }
    }

    /// Areas 1 - 2 - 3 in a row (portals 10 and 11) and 4 off area 1
    /// (portal 12); one leaf, and one cluster, per area 1..=4.
    fn row_of_areas(polygons: [Vec<Vec3>; 3]) -> MapAreas {
        let [a, b, c] = polygons;
        MapAreas::new(
            vec![1, 2, 3, 4],
            &[0, 1, 2, 3],
            4,
            vec![portal(10, 1, 2, a), portal(11, 2, 3, b), portal(12, 1, 4, c)],
        )
    }

    fn reached(r: &[bool]) -> Vec<usize> {
        (0..r.len()).filter(|&a| r[a]).collect()
    }

    #[test]
    fn flood_follows_open_portals() {
        let areas = row_of_areas([Vec::new(), Vec::new(), Vec::new()]);
        assert_eq!(areas.area_count(), 5);
        let all = |_: &AreaPortal| true;
        assert_eq!(reached(&areas.flood(1, &all, None)), vec![0, 1, 2, 3, 4]);
        // Closing 11 cuts off area 3 only; closing 10 cuts off 2 and 3.
        let shut = |keys: &'static [u16]| move |p: &AreaPortal| !keys.contains(&p.key);
        assert_eq!(reached(&areas.flood(1, &shut(&[11]), None)), vec![0, 1, 2, 4]);
        assert_eq!(reached(&areas.flood(1, &shut(&[10]), None)), vec![0, 1, 4]);
        assert_eq!(reached(&areas.flood(3, &shut(&[10]), None)), vec![0, 2, 3]);
        // Portals work both ways; area 0 (or none) reaches everything.
        assert_eq!(reached(&areas.flood(4, &all, None)), vec![0, 1, 2, 3, 4]);
        assert_eq!(reached(&areas.flood(0, &shut(&[10, 11, 12]), None)), vec![0, 1, 2, 3, 4]);
        assert_eq!(reached(&areas.flood(9, &shut(&[10, 11, 12]), None)), vec![0, 1, 2, 3, 4]);
        // Cluster k + 1 is in area k + 1.
        let mask = areas.cluster_mask(&areas.flood(1, &shut(&[11]), None), 4);
        assert_eq!(mask, vec![0b1011]);
    }

    #[test]
    fn flood_looks_through_portal_openings() {
        // Camera at the origin looking down -z, 90 degree field of view.
        let projection = PerspectiveProjection {
            fov: std::f32::consts::FRAC_PI_2,
            aspect_ratio: 1.0,
            ..default()
        };
        use bevy::camera::CameraProjection;
        let clip = projection.get_clip_from_view() * Mat4::IDENTITY;
        let all = |_: &AreaPortal| true;
        // In front: through 10 then 11 (straight ahead, further on);
        // portal 12 is behind the camera.
        let ahead = row_of_areas([
            opening(Vec3::new(0.0, 0.0, -5.0)),
            opening(Vec3::new(0.0, 0.0, -10.0)),
            opening(Vec3::new(0.0, 0.0, 5.0)),
        ]);
        assert_eq!(reached(&ahead.flood(1, &all, Some(clip))), vec![0, 1, 2, 3]);
        // 11 off to the side, outside the view: area 3 isn't seen.
        let aside = row_of_areas([
            opening(Vec3::new(0.0, 0.0, -5.0)),
            opening(Vec3::new(40.0, 0.0, -10.0)),
            opening(Vec3::new(0.0, 0.0, 5.0)),
        ]);
        assert_eq!(reached(&aside.flood(1, &all, Some(clip))), vec![0, 1, 2]);
        // 11 in view, but not through 10's opening (10 on the left, 11 on
        // the right): narrowed away.
        let narrowed = row_of_areas([
            opening(Vec3::new(-3.0, 0.0, -5.0)),
            opening(Vec3::new(6.0, 0.0, -10.0)),
            opening(Vec3::new(0.0, 0.0, 5.0)),
        ]);
        assert_eq!(reached(&narrowed.flood(1, &all, Some(clip))), vec![0, 1, 2]);
        // Without the projection, all of them.
        assert_eq!(reached(&narrowed.flood(1, &all, None)), vec![0, 1, 2, 3, 4]);
        // A portal straddling the eye's plane covers the whole screen.
        let straddle = row_of_areas([
            vec![Vec3::new(-1.0, -1.0, 1.0), Vec3::new(1.0, -1.0, -1.0), Vec3::new(0.0, 1.0, 0.0)],
            opening(Vec3::new(0.0, 0.0, -10.0)),
            Vec::new(),
        ]);
        assert_eq!(reached(&straddle.flood(1, &all, Some(clip))), vec![0, 1, 2, 3, 4]);
    }

    /// `two_rooms` with areas: cluster 0 in area 1, cluster 1 in area 2,
    /// joined by portal 7 (an opening at x = 0).
    fn two_areas(fade: Option<f32>) -> MapVisibility {
        let mut v = two_rooms();
        // Cluster 1 sees cluster 0 too here.
        v.visible = vec![vec![0b11], vec![0b11]];
        let mut p = portal(7, 1, 2, vec![Vec3::new(0.0, -1.0, -1.0), Vec3::new(0.0, 1.0, -1.0), Vec3::new(0.0, 1.0, 1.0)]);
        p.fade = fade;
        v.areas = MapAreas::new(vec![1, 2, 0], &v.leaf_clusters, 2, vec![p]);
        v
    }

    #[test]
    fn closed_portals_cut_the_pvs() {
        let v = two_areas(None);
        let at = Vec3::new(1.0, 0.0, 0.0);
        assert_eq!(v.area_at(at), 1);
        let open = AreaPortalStates::default();
        let closed = AreaPortalStates { closed: vec![7] };
        assert_eq!(camera_clusters(&v, at, None, &open, false).unwrap(), (vec![0b11], 1, 3));
        assert_eq!(camera_clusters(&v, at, None, &closed, false).unwrap(), (vec![0b01], 1, 2));
        // r_portalsopenall: the PVS alone.
        assert_eq!(camera_clusters(&v, at, None, &closed, true).unwrap().0, vec![0b11]);
        // A window stays open at any distance (its fade isn't applied).
        let w = two_areas(Some(2.0));
        let far = Vec3::new(5.0, 0.0, 0.0);
        assert_eq!(camera_clusters(&w, far, None, &open, false).unwrap().0, vec![0b11]);
        // A door with a window: seen through even when closed.
        let mut glass = two_areas(None);
        glass.areas.portals[0].see_through = true;
        assert_eq!(camera_clusters(&glass, at, None, &closed, false).unwrap().0, vec![0b11]);
        // Outside the map: None.
        assert!(camera_clusters(&v, Vec3::new(20.0, 0.0, 0.0), None, &open, false).is_none());
    }

    #[test]
    fn culling_hides_what_a_closed_portal_cuts_off() {
        let mut app = App::new();
        app.init_resource::<NoVis>()
            .init_resource::<VisStats>()
            .insert_resource(ActiveVisibility(std::sync::Arc::new(two_areas(None))))
            .add_systems(Update, cull);
        app.world_mut()
            .spawn((Camera3d::default(), GlobalTransform::from_translation(Vec3::new(1.0, 0.0, 0.0))));
        let far = app.world_mut().spawn((VisClusters::new(vec![1]), Visibility::default())).id();
        let shown = |app: &App| app.world().get::<Visibility>(far) == Some(&Visibility::Inherited);
        app.update();
        assert!(shown(&app), "no portal states: open");
        app.insert_resource(AreaPortalStates { closed: vec![7] });
        app.update();
        assert!(!shown(&app), "closed");
        assert_eq!(app.world().resource::<VisStats>().closed_portals, 1);
        app.insert_resource(PortalsOpenAll(1));
        app.update();
        assert!(shown(&app), "r_portalsopenall 1");
        app.insert_resource(PortalsOpenAll(0));
        app.update();
        assert!(!shown(&app));
        app.insert_resource(AreaPortalStates::default());
        app.update();
        assert!(shown(&app), "opened again");
    }

    #[test]
    fn glass_in_an_opening_keeps_its_portal_open() {
        let mut areas = row_of_areas([
            opening(Vec3::new(0.0, 0.0, -5.0)),
            opening(Vec3::new(0.0, 0.0, -10.0)),
            opening(Vec3::new(0.0, 0.0, 5.0)),
        ]);
        let pane = |z: f32, alpha: super::super::MapAlpha| MapMesh {
            positions: vec![[-0.5, -0.5, z], [0.5, -0.5, z], [0.0, 0.5, z]],
            indices: vec![0, 1, 2],
            alpha,
            ..default()
        };
        // Glass just behind portal 11's opening; an opaque pane in 10's.
        areas.mark_see_through(&[pane(-10.1, super::super::MapAlpha::Blend), pane(-5.0, super::super::MapAlpha::Opaque)]);
        let marked: Vec<u16> = areas.portals.iter().filter(|p| p.see_through).map(|p| p.key).collect();
        assert_eq!(marked, vec![11]);
    }

    #[test]
    fn fade_bands_need_a_near_distance_below_the_far_one() {
        let band = fade_band(10.0, 20.0).unwrap();
        assert_eq!(band.end_margin, 10.0..20.0);
        assert!(fade_band(-1.0, 20.0).is_none());
        assert!(fade_band(20.0, 20.0).is_none());
        assert!(fade_band(0.0, 20.0).is_some());
    }
}
