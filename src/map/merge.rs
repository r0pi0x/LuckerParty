//! Brush entities drawn together: maps with hundreds of breakables or
//! doors (a minigame map's wall of blocks) gave each entity a mesh per
//! material, and each mesh is a draw and a main-world entity of its own.
//! While a brush entity is where the map put it, whole and shown, its
//! opaque and alpha-tested meshes are drawn as part of a combined mesh
//! per material and chunk (`MergedChunk`), and its own meshes
//! (`MergedPiece`) stay hidden. When it moves, breaks, is removed or
//! turned off by the logic (`vis::LogicHidden`), or shows broken panes
//! (`BrushPanes`), its triangles leave the combined mesh and its own
//! meshes are drawn again; when it is back as it started (a closed door,
//! a round restart) it rejoins. The node itself (collider, logic, culling,
//! decals) is unchanged either way.

use std::{collections::HashMap, ops::Range};

use bevy::{mesh::Indices, prelude::*};

use super::{
    BrushPanes, MapMesh,
    vis::{self, MapVisibility},
};

/// A brush entity node with meshes in combined chunks.
#[derive(Component, Clone, Debug)]
pub struct MergedBrush {
    /// Where the map placed it: it is drawn merged only while here.
    pub home: Transform,
    /// The combined meshes holding its triangles.
    pub chunks: Vec<Entity>,
    /// Drawn through the chunks (its own meshes hidden).
    pub merged: bool,
}

/// One of a merged brush entity's own meshes (hidden while it is merged).
#[derive(Component, Clone, Copy, Debug)]
pub struct MergedPiece;

/// A combined mesh of brush entities sharing a material in one chunk:
/// each node's index range in the full index list. Hidden (`LogicHidden`)
/// while none of them is merged.
#[derive(Component, Clone, Debug)]
pub struct MergedChunk {
    pub parts: Vec<(Entity, Range<usize>)>,
    pub indices: Vec<u32>,
}

/// Whether a brush entity mesh may be drawn merged: in the world (not the
/// 3D skybox, not water) and not blended (blended meshes are sorted one by
/// one; merged, their order would change).
pub(super) fn mergeable(m: &MapMesh) -> bool {
    !m.skybox
        && m.water.is_none()
        && !matches!(m.alpha, super::MapAlpha::Blend | super::MapAlpha::Add)
        && m.normals.len() == m.positions.len()
        && m.uvs.len() == m.positions.len()
}

/// Off with `MASHUP_MERGE_BRUSHES=0` (every brush entity draws its own
/// meshes), for measurements.
pub(super) fn enabled() -> bool {
    std::env::var("MASHUP_MERGE_BRUSHES").map_or(true, |v| v != "0")
}

/// Collects brush entity meshes into combined ones while the map spawns.
/// `K` is the material (with whatever else must match to draw together).
pub(super) struct Merger<K> {
    size: f32,
    groups: Vec<(K, IVec3, MapMesh, Vec<(Entity, Range<usize>)>, Vec<(Vec3, Vec3)>)>,
    index: HashMap<(K, IVec3), usize>,
}

impl<K: Clone + Eq + std::hash::Hash> Merger<K> {
    pub(super) fn new(size: f32) -> Self {
        Self {
            size,
            groups: Vec::new(),
            index: HashMap::new(),
        }
    }

    /// Add node `node`'s mesh `m` (node space) placed at `home`, grouped
    /// by `key` and the chunk of the node's `centre` (world).
    pub(super) fn add(&mut self, key: K, node: Entity, home: &Transform, centre: Vec3, m: &MapMesh) {
        let cell = (centre / self.size).floor().as_ivec3();
        let i = *self.index.entry((key.clone(), cell)).or_insert_with(|| {
            let mut template = m.clone();
            template.entity = None;
            template.positions.clear();
            template.normals.clear();
            template.uvs.clear();
            template.lightmap_uvs.clear();
            template.blend_weights.clear();
            template.indices.clear();
            self.groups.push((key, cell, template, Vec::new(), Vec::new()));
            self.groups.len() - 1
        });
        let (_, _, out, parts, boxes) = &mut self.groups[i];
        let base = out.positions.len() as u32;
        let start = out.indices.len();
        let (mut lo, mut hi) = (Vec3::MAX, Vec3::MIN);
        for p in &m.positions {
            let w = home.transform_point(Vec3::from(*p));
            lo = lo.min(w);
            hi = hi.max(w);
            out.positions.push(w.to_array());
        }
        for n in &m.normals {
            out.normals.push((home.rotation * Vec3::from(*n)).to_array());
        }
        out.uvs.extend_from_slice(&m.uvs);
        if m.lightmap_uvs.len() == m.positions.len() {
            out.lightmap_uvs.extend_from_slice(&m.lightmap_uvs);
        }
        if m.blend_weights.len() == m.positions.len() {
            out.blend_weights.extend_from_slice(&m.blend_weights);
        }
        out.indices.extend(m.indices.iter().map(|i| i + base));
        match parts.last_mut() {
            Some((n, r)) if *n == node && r.end == start => r.end = out.indices.len(),
            _ => parts.push((node, start..out.indices.len())),
        }
        if lo.x <= hi.x {
            boxes.push((lo, hi));
        }
    }

    /// The combined meshes: (key, mesh recentred, its centre, the nodes'
    /// index ranges, the clusters its parts touch).
    pub(super) fn finish(
        self,
        vis: Option<&MapVisibility>,
    ) -> Vec<(K, MapMesh, Vec3, Vec<(Entity, Range<usize>)>, Vec<u32>)> {
        self.groups
            .into_iter()
            .map(|(key, _, mut mesh, parts, boxes)| {
                let clusters = match vis {
                    Some(v) => {
                        let mut out = Vec::new();
                        for (lo, hi) in boxes {
                            v.clusters_in_box(lo, hi, &mut out);
                        }
                        out.sort_unstable();
                        out.dedup();
                        out
                    }
                    None => Vec::new(),
                };
                let centre = vis::recentre(&mut mesh);
                (key, mesh, centre, parts, clusters)
            })
            .collect()
    }
}

/// Close enough to where the map put it to draw merged.
fn at_home(t: &Transform, home: &Transform) -> bool {
    t.translation.distance_squared(home.translation) < 1e-8 && t.rotation.angle_between(home.rotation) < 1e-4
}

/// Each frame: brush entities that left their merged state (moved,
/// removed, broken panes) are drawn from their own meshes and their
/// triangles leave the combined meshes; those back home rejoin.
#[allow(clippy::type_complexity)]
pub(super) fn sync_merged_brushes(
    mut nodes: Query<(
        &Transform,
        Has<vis::LogicHidden>,
        Has<BrushPanes>,
        &mut MergedBrush,
        &Children,
    )>,
    mut pieces: Query<&mut Visibility, (With<MergedPiece>, Without<MergedChunk>)>,
    mut chunks: Query<(
        &MergedChunk,
        &Mesh3d,
        &mut Visibility,
        Option<&vis::VisClusters>,
        Has<vis::LogicHidden>,
    )>,
    meshes: Option<ResMut<Assets<Mesh>>>,
    mut commands: Commands,
    mut dirty: Local<Vec<Entity>>,
) {
    let Some(mut meshes) = meshes else { return };
    dirty.clear();
    for (t, hidden, panes, mut merged, children) in &mut nodes {
        let want = !hidden && !panes && at_home(t, &merged.home);
        if merged.merged == want {
            continue;
        }
        merged.merged = want;
        dirty.extend(merged.chunks.iter().copied());
        for child in children.iter() {
            if let Ok(mut v) = pieces.get_mut(child) {
                v.set_if_neq(if want {
                    Visibility::Hidden
                } else {
                    Visibility::Inherited
                });
            }
        }
    }
    dirty.sort_unstable();
    dirty.dedup();
    for chunk in dirty.iter().copied() {
        let Ok((c, mesh, mut visibility, clusters, was_hidden)) = chunks.get_mut(chunk) else {
            continue;
        };
        let indices: Vec<u32> = c
            .parts
            .iter()
            .filter(|(n, _)| nodes.get(*n).is_ok_and(|(_, _, _, m, _)| m.merged))
            .flat_map(|(_, r)| c.indices[r.clone()].iter().copied())
            .collect();
        if indices.is_empty() {
            // Nothing left to draw: hidden, its last indices kept (an
            // empty index buffer isn't worth a special case).
            if !was_hidden {
                commands.entity(chunk).insert(vis::LogicHidden);
            }
            visibility.set_if_neq(Visibility::Hidden);
            continue;
        }
        if was_hidden {
            commands.entity(chunk).remove::<vis::LogicHidden>();
            let culled = clusters.is_some_and(|v| !v.potentially_visible);
            visibility.set_if_neq(if culled {
                Visibility::Hidden
            } else {
                Visibility::Inherited
            });
        }
        if let Some(mut m) = meshes.get_mut(&mesh.0) {
            m.insert_indices(Indices::U32(indices));
        }
    }
}
