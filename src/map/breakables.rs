//! Breakable brushes for any game, the drawing and physics side: gibs
//! (small models thrown when a brush breaks, flying with gravity, bouncing
//! off the world and fading out; a broken prop's pieces are physics
//! bodies that tumble and come to rest; neither blocks characters or
//! shots) and windows
//! made of panes (`BrushPanes` on a brush entity node: only the unbroken
//! panes are drawn and hit by shots). What breaks, and when, is the logic
//! layer's; games load the gib models (`MapData::gibs`) and draw the
//! shards of shattered glass (`GlassShatter`, `GlassImpact`).
//! Engine space: meters, Y up.

use avian3d::prelude::*;
use bevy::{
    mesh::{Indices, VertexAttributeValues},
    prelude::*,
};

use super::{LightField, MapData, MapDebugView, MapModel, MapPart, PaneLook, prop_material::PropMaterial};

/// A gib list: the models one break picks from, by name (Source: the
/// propdata "BreakableModels" lists, "MetalChunks"...).
#[derive(Clone, Debug)]
pub struct MapGibSet {
    pub name: String,
    pub models: Vec<MapModel>,
}

/// How gibs move and look.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MapGibPhysics {
    /// Downward acceleration (m/s²).
    pub gravity: f32,
    /// Velocity kept per bounce, and for glass gibs.
    pub bounce: f32,
    pub glass_bounce: f32,
    /// Opacity of glass gibs (0..1).
    pub glass_alpha: f32,
    /// Seconds a gib takes to fade out after its life.
    pub fade: f32,
    /// Downward acceleration of a broken prop's pieces (m/s²).
    pub prop_gravity: f32,
}

impl Default for MapGibPhysics {
    fn default() -> Self {
        Self {
            gravity: 9.81 / 2.0,
            bounce: 0.5,
            glass_bounce: 0.3,
            glass_alpha: 0.5,
            fade: 1.0,
            prop_gravity: 9.81,
        }
    }
}

/// One piece to throw.
#[derive(Clone, Debug, PartialEq)]
pub struct GibPiece {
    pub position: Vec3,
    pub velocity: Vec3,
    /// Angular velocity (rad/s, an axis times its rate).
    pub spin: Vec3,
    /// Seconds before fading.
    pub life: f32,
    /// Which model of the list (None: one at random).
    pub model: Option<usize>,
    /// Its rotation (None: a random one). `position` is then where the
    /// model's origin goes, not its centre.
    pub rotation: Option<Quat>,
    /// Stays where it is (a piece the prop holds in place).
    pub frozen: bool,
    /// Its skin family (one its model lacks shows skin 0).
    pub skin: i32,
    /// Fades out between these distances from the view (meters).
    pub fade_dist: Option<(f32, f32)>,
}

impl GibPiece {
    /// A piece thrown at a point, spinning, picked at random.
    pub fn thrown(position: Vec3, velocity: Vec3, spin: Vec3, life: f32) -> Self {
        Self {
            position,
            velocity,
            spin,
            life,
            model: None,
            rotation: None,
            frozen: false,
            skin: 0,
            fade_dist: None,
        }
    }
}

/// Throw gibs from a gib list (a model is picked at random per piece).
#[derive(Message, Clone, Debug)]
pub struct SpawnGibs {
    pub set: String,
    /// Glass: translucent and less bouncy.
    pub glass: bool,
    /// A broken prop's pieces: they fall at `MapGibPhysics::prop_gravity`.
    pub prop: bool,
    pub pieces: Vec<GibPiece>,
    /// Pieces that shatter on the first thing they touch (falling window
    /// panes, with `prop`): the pane's size and the shards' size
    /// (meters).
    pub shatters: Option<(Vec2, f32)>,
}

/// The most gibs alive at once (Source `cl_phys_props_max`); more are not
/// spawned.
pub const MAX_GIBS: usize = 300;

/// What a prop model breaks into (Source: the `.phy` break pieces, else
/// chunks of a gib list).
#[derive(Clone, Debug, PartialEq)]
pub enum MapBreak {
    /// One piece per entry, each a model of gib list `set`.
    Pieces { set: String, pieces: Vec<MapBreakPiece> },
    /// `count` chunks of gib list `set`, picked among its first
    /// `size_limit + 1` models; each lives a time in `life`.
    Chunks {
        set: String,
        count: usize,
        size_limit: usize,
        life: (f32, f32),
        /// Each list model's bounds centre (its model space).
        centres: Vec<Vec3>,
        /// The chunks' skin family (prop data `breakable_skin`).
        skin: i32,
    },
}

/// One break piece of a prop model.
#[derive(Clone, Debug, PartialEq)]
pub struct MapBreakPiece {
    /// Index in its gib list.
    pub model: usize,
    /// Where its origin sits in the prop's model space (meters).
    pub offset: Vec3,
    /// Its model's bounds centre (its model space).
    pub centre: Vec3,
    /// Seconds before it fades; infinite: never.
    pub life: f32,
    /// Outward speed added (m/s).
    pub burst: f32,
    pub frozen: bool,
    /// Fades out between these distances from the view (meters; the
    /// break block's `fademindist`, `fademaxdist`).
    pub fade_dist: Option<(f32, f32)>,
}

/// A prop broke: throw what its model breaks into (`MapModel::breaks`)
/// from where it was, moving as it moved.
#[derive(Message, Clone, Debug)]
pub struct BreakProp {
    /// Index into `MapData::props` (`PropIndex`).
    pub prop: usize,
    pub transform: Transform,
    pub velocity: Vec3,
    /// Angular velocity (rad/s, an axis times its rate).
    pub spin: Vec3,
    /// Its skin family now (None: the one it was placed with).
    pub skin: Option<i32>,
}

/// Every prop's break pieces, bounds and placed skin, from its model
/// (also headless).
#[derive(Resource, Default)]
pub(super) struct PropBreaks(Vec<(Option<MapBreak>, (Vec3, Vec3), i32)>);

pub(super) fn prop_breaks(data: &MapData) -> PropBreaks {
    PropBreaks(
        data.props
            .iter()
            .map(|p| {
                let m = &data.models[p.model];
                (m.breaks.clone(), m.bounds, p.skin)
            })
            .collect(),
    )
}

/// Template chunks fly out at 100 units/s (in meters/s).
const CHUNK_BURST: f32 = 100.0 * 0.0254;
/// Model pieces' velocity is jittered by this fraction either way.
const PIECE_JITTER: f32 = 0.025;

/// The pieces a broken prop throws (prop_damage.md 7.3, the client
/// branch): its model's break pieces at its pose, in its skin, moving
/// with it (±2.5 %) plus their burst outward and spinning as it spun, or
/// chunks (in the prop data's skin, not spinning) at random points of its
/// box (the smallest axis at its middle) flying out at 100 units/s.
/// `random` gives uniform numbers in [0, 1).
#[allow(clippy::too_many_arguments)]
pub fn break_pieces(
    breaks: &MapBreak,
    bounds: (Vec3, Vec3),
    transform: &Transform,
    velocity: Vec3,
    spin: Vec3,
    skin: i32,
    random: &mut dyn FnMut() -> f32,
) -> Vec<GibPiece> {
    let origin = transform.translation;
    let rot = transform.rotation;
    match breaks {
        MapBreak::Pieces { pieces, .. } => pieces
            .iter()
            .map(|p| {
                let at = origin + rot * p.offset;
                let mid = at + rot * p.centre;
                let out = if at.distance_squared(origin) > 1e-8 {
                    at - origin
                } else {
                    mid - origin
                };
                let jitter = 1.0 + (random() * 2.0 - 1.0) * PIECE_JITTER;
                let v = if p.frozen {
                    Vec3::ZERO
                } else {
                    velocity * jitter + out.normalize_or_zero() * p.burst
                };
                GibPiece {
                    position: at,
                    velocity: v,
                    spin: if p.frozen { Vec3::ZERO } else { spin },
                    life: p.life,
                    model: Some(p.model),
                    rotation: Some(rot),
                    frozen: p.frozen,
                    skin,
                    fade_dist: p.fade_dist,
                }
            })
            .collect(),
        MapBreak::Chunks {
            count,
            size_limit,
            life,
            centres,
            skin: chunk_skin,
            ..
        } => {
            let (lo, hi) = bounds;
            let size = hi - lo;
            let smallest = if size.x <= size.y && size.x <= size.z {
                0
            } else if size.y <= size.z {
                1
            } else {
                2
            };
            (0..*count)
                .map(|_| {
                    let mut local = Vec3::new(
                        lo.x + size.x * random(),
                        lo.y + size.y * random(),
                        lo.z + size.z * random(),
                    );
                    local[smallest] = (lo[smallest] + hi[smallest]) / 2.0;
                    let at = origin + rot * local;
                    let pick = ((random() * (*size_limit + 1) as f32) as usize)
                        .min(*size_limit)
                        .min(centres.len().saturating_sub(1));
                    GibPiece {
                        // The chunk's centre goes there.
                        position: at - rot * centres.get(pick).copied().unwrap_or_default(),
                        velocity: velocity + (at - origin).normalize_or_zero() * CHUNK_BURST,
                        spin: Vec3::ZERO,
                        life: life.0 + (life.1 - life.0) * random(),
                        model: Some(pick),
                        rotation: Some(rot),
                        frozen: false,
                        skin: *chunk_skin,
                        fade_dist: None,
                    }
                })
                .collect()
        }
    }
}

/// Turn `BreakProp`s into gibs.
pub(super) fn break_props(
    mut events: MessageReader<BreakProp>,
    breaks: Option<Res<PropBreaks>>,
    mut gibs: MessageWriter<SpawnGibs>,
    mut dice: Local<u32>,
) {
    let Some(breaks) = breaks else {
        events.clear();
        return;
    };
    let mut random = || {
        *dice = dice.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        (*dice >> 8) as f32 / (1u32 << 24) as f32
    };
    for ev in events.read() {
        let Some((Some(b), bounds, placed_skin)) = breaks.0.get(ev.prop) else {
            continue;
        };
        let set = match b {
            MapBreak::Pieces { set, .. } | MapBreak::Chunks { set, .. } => set.clone(),
        };
        let skin = ev.skin.unwrap_or(*placed_skin);
        let pieces = break_pieces(b, *bounds, &ev.transform, ev.velocity, ev.spin, skin, &mut random);
        if !pieces.is_empty() {
            gibs.write(SpawnGibs {
                set,
                glass: false,
                prop: true,
                pieces,
                shatters: None,
            });
        }
    }
}

/// A pane of glass (or tile) shattered: its centre and normal, its size
/// (meters) and the push its shards get (m/s). Games draw the shards.
#[derive(Message, Clone, Debug)]
pub struct GlassShatter {
    pub at: Vec3,
    pub normal: Vec3,
    pub size: Vec2,
    pub velocity: Vec3,
    pub tile: bool,
    /// The shards' size (meters): small for one pane, large for a blast's
    /// burst.
    pub shard: f32,
}

/// A collapsing pane drops a falling piece (Source: the pane piece
/// model): the pane's reference corner, its axes (along the columns, the
/// rows, the window's normal), its size (meters), the body variant (the
/// `PANE_PIECES` gib list's model) and its spin (rad/s). The piece falls,
/// and shatters (`GlassShatter`) on the first thing it touches.
#[derive(Message, Clone, Debug)]
pub struct FallingPane {
    pub at: Vec3,
    pub axes: [Vec3; 3],
    pub size: Vec2,
    pub body: usize,
    pub spin: Vec3,
    /// The shards' size when it shatters (meters).
    pub shard: f32,
}

/// The gib list of falling pane pieces, one model per body variant.
pub const PANE_PIECES: &str = "PanePieces";
/// The falling pane piece's body variants (random body 0..2).
pub const PANE_PIECE_BODIES: usize = 3;

/// A bullet or club hit shattered the window pane it hit: the hit point
/// and the trace normal (towards the shooter). Games draw the glass
/// impact (specs/cs_source/impact_effects.md section 9).
#[derive(Message, Clone, Copy, Debug)]
pub struct GlassImpact {
    pub at: Vec3,
    pub normal: Vec3,
}

/// A window on a brush entity node, as a grid of panes, in the node's own
/// space: the lower-left corner, one pane's steps along the columns and
/// rows, and the brush's depth behind the face (`depth.0..depth.1` along
/// the face normal from the corner). Only unbroken panes are drawn and
/// collide (when `active`: the window broke; until then the brush is
/// whole).
#[derive(Component, Clone, Debug, PartialEq)]
pub struct BrushPanes {
    pub corner: Vec3,
    pub u: Vec3,
    pub v: Vec3,
    pub depth: (f32, f32),
    pub cols: usize,
    pub rows: usize,
    pub broken: Vec<bool>,
}

impl BrushPanes {
    pub fn normal(&self) -> Vec3 {
        self.u.cross(self.v).normalize_or_zero()
    }

    /// Pane coordinates of a node-space point.
    pub fn pane_at(&self, p: Vec3) -> Vec2 {
        let d = p - self.corner;
        Vec2::new(
            d.dot(self.u) / self.u.length_squared().max(1e-12),
            d.dot(self.v) / self.v.length_squared().max(1e-12),
        )
    }

    pub fn whole(&self, c: usize, r: usize) -> bool {
        !self.broken[r * self.cols + c]
    }

    /// One box per unbroken pane: (centre, rotation, size), node space.
    pub fn boxes(&self) -> Vec<(Vec3, Quat, Vec3)> {
        let n = self.normal();
        let (ud, vd) = (self.u.normalize_or_zero(), self.v.normalize_or_zero());
        // Panes are rectangles; keep the basis orthonormal anyway.
        let vd = (vd - n * vd.dot(n) - ud * vd.dot(ud)).normalize_or_zero();
        let rot = Quat::from_mat3(&Mat3::from_cols(ud, vd, n));
        let thick = (self.depth.1 - self.depth.0).abs().max(1e-3);
        let mid = (self.depth.0 + self.depth.1) / 2.0;
        let mut out = Vec::new();
        for r in 0..self.rows {
            for c in 0..self.cols {
                if !self.whole(c, r) {
                    continue;
                }
                let centre = self.corner + self.u * (c as f32 + 0.5) + self.v * (r as f32 + 0.5) + n * mid;
                out.push((centre, rot, Vec3::new(self.u.length(), self.v.length(), thick)));
            }
        }
        out
    }
}

/// A mesh drawn under a node with `BrushPanes`: its whole (unclipped)
/// mesh, from which the unbroken panes are cut.
#[derive(Component)]
pub(super) struct PaneSource(Handle<Mesh>);

/// Keep a window's collider and meshes to its unbroken panes.
#[allow(clippy::type_complexity)]
pub(super) fn update_panes(
    nodes: Query<(Entity, &BrushPanes, Option<&Children>), Changed<BrushPanes>>,
    mut parts: Query<(&mut Mesh3d, Option<&PaneSource>, Option<&PanePart>, &mut Visibility)>,
    meshes: Option<ResMut<Assets<Mesh>>>,
    mut commands: Commands,
) {
    let mut meshes = meshes;
    for (node, panes, children) in &nodes {
        let boxes = panes.boxes();
        if boxes.is_empty() {
            commands.entity(node).insert(ColliderDisabled);
        } else {
            let shapes = boxes
                .iter()
                .map(|(c, r, s)| (*c, *r, Collider::cuboid(s.x, s.y, s.z)))
                .collect();
            commands
                .entity(node)
                .insert((
                    Collider::compound(shapes),
                    // Non-solid once broken: shots still hit the panes,
                    // physics bodies pass (and shatter them by touch).
                    CollisionLayers::new(LayerMask::DEFAULT, LayerMask::NONE),
                ))
                .remove::<ColliderDisabled>();
        }
        let (Some(meshes), Some(children)) = (meshes.as_mut(), children) else {
            continue;
        };
        // With a broken look the face as placed gives way to it.
        let looks: Vec<PaneLook> = children
            .iter()
            .filter_map(|c| parts.get(c).ok().map(|p| p.2.map_or(PaneLook::Whole, |l| l.0)))
            .collect();
        let cracked = looks.contains(&PaneLook::Cracked);
        let edges = looks.iter().filter(|l| matches!(l, PaneLook::Edge(_))).count() as u8;
        for child in children.iter() {
            let Ok((mut mesh3d, source, look, mut visibility)) = parts.get_mut(child) else {
                continue;
            };
            let look = look.map_or(PaneLook::Whole, |l| l.0);
            let source = match source {
                Some(s) => s.0.clone(),
                None => {
                    let s = mesh3d.0.clone();
                    commands.entity(child).insert(PaneSource(s.clone()));
                    s
                }
            };
            if look == PaneLook::Whole && cracked {
                visibility.set_if_neq(Visibility::Hidden);
                continue;
            }
            let Some(clipped) = meshes.get(&source).and_then(|m| match look {
                PaneLook::Edge(k) => clip_to_rects(m, panes, &edge_strips(panes, k, edges)),
                _ => clip_to_panes(m, panes),
            }) else {
                continue;
            };
            // Nothing left of it: hidden (an empty mesh upsets the
            // renderer's allocator).
            if clipped.indices().is_none_or(|i| i.is_empty()) {
                visibility.set_if_neq(Visibility::Hidden);
                continue;
            }
            visibility.set_if_neq(Visibility::Inherited);
            let old = std::mem::replace(&mut mesh3d.0, meshes.add(clipped));
            if old != source {
                meshes.remove(&old);
            }
        }
    }
}

/// A window's broken look on one of its meshes (`MapMesh::pane_look`):
/// hidden until the window breaks.
#[derive(Component, Clone, Copy, Debug)]
pub struct PanePart(pub PaneLook);

/// How far (in panes) a jagged edge piece reaches into the broken pane
/// beside it (ours: the spec leaves edge pieces to the client).
pub const EDGE_DEPTH: f32 = 0.5;

/// A rectangle of pane space to cut a mesh to; `uv` maps a point `p` of
/// it to texture coordinates ((p − o)·x, (p − o)·y) instead of
/// interpolating the mesh's.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PaneRect {
    pub lo: Vec2,
    pub hi: Vec2,
    pub uv: Option<(Vec2, Vec2, Vec2)>,
}

/// The edge strips of edge material `k` of `kinds`: along each side of an
/// unbroken pane whose neighbour inside the window broke, reaching
/// `EDGE_DEPTH` into the neighbour; its texture's u runs along the side
/// and its bottom (v = 1) lies on the pane. Each side picks one of the `kinds` edge
/// materials by its place (ours).
pub fn edge_strips(panes: &BrushPanes, k: u8, kinds: u8) -> Vec<PaneRect> {
    let (cols, rows) = (panes.cols as i32, panes.rows as i32);
    let broken = |c: i32, r: i32| c >= 0 && r >= 0 && c < cols && r < rows && !panes.whole(c as usize, r as usize);
    let d = EDGE_DEPTH;
    let mut out = Vec::new();
    for r in 0..rows {
        for c in 0..cols {
            if !panes.whole(c as usize, r as usize) {
                continue;
            }
            let (x, y) = (c as f32, r as f32);
            // (neighbour, rect, origin, u axis, v axis)
            let sides = [
                (
                    (c + 1, r),
                    Vec2::new(x + 1.0, y),
                    Vec2::new(x + 1.0 + d, y + 1.0),
                    Vec2::new(x + 1.0 + d, y),
                    Vec2::Y,
                    Vec2::NEG_X / d,
                ),
                (
                    (c - 1, r),
                    Vec2::new(x - d, y),
                    Vec2::new(x, y + 1.0),
                    Vec2::new(x - d, y + 1.0),
                    Vec2::NEG_Y,
                    Vec2::X / d,
                ),
                (
                    (c, r + 1),
                    Vec2::new(x, y + 1.0),
                    Vec2::new(x + 1.0, y + 1.0 + d),
                    Vec2::new(x + 1.0, y + 1.0 + d),
                    Vec2::NEG_X,
                    Vec2::NEG_Y / d,
                ),
                (
                    (c, r - 1),
                    Vec2::new(x, y - d),
                    Vec2::new(x + 1.0, y),
                    Vec2::new(x, y - d),
                    Vec2::X,
                    Vec2::Y / d,
                ),
            ];
            for (i, ((nc, nr), lo, hi, o, ux, vy)) in sides.into_iter().enumerate() {
                if !broken(nc, nr) {
                    continue;
                }
                let pick = (c as u32).wrapping_mul(7) ^ (r as u32).wrapping_mul(13) ^ (i as u32).wrapping_mul(5);
                if kinds == 0 || (pick % kinds as u32) as u8 != k {
                    continue;
                }
                out.push(PaneRect {
                    lo,
                    hi,
                    uv: Some((o, ux, vy)),
                });
            }
        }
    }
    out
}

/// The part of a triangle mesh over unbroken panes: every triangle
/// clipped to each unbroken pane it overlaps (in the window's plane), its
/// float attributes interpolated.
pub fn clip_to_panes(mesh: &Mesh, panes: &BrushPanes) -> Option<Mesh> {
    let mut rects = Vec::new();
    for r in 0..panes.rows {
        for c in 0..panes.cols {
            if panes.whole(c, r) {
                let lo = Vec2::new(c as f32, r as f32);
                rects.push(PaneRect {
                    lo,
                    hi: lo + Vec2::ONE,
                    uv: None,
                });
            }
        }
    }
    clip_to_rects(mesh, panes, &rects)
}

/// A triangle mesh cut to rectangles of pane space: every triangle
/// clipped to each rectangle it overlaps (in the window's plane), its
/// float attributes interpolated, its texture coordinates the
/// rectangle's own where it has them.
pub fn clip_to_rects(mesh: &Mesh, panes: &BrushPanes, rects: &[PaneRect]) -> Option<Mesh> {
    let positions: Vec<Vec3> = match mesh.try_attribute(Mesh::ATTRIBUTE_POSITION).ok()? {
        VertexAttributeValues::Float32x3(v) => v.iter().map(|p| Vec3::from(*p)).collect(),
        _ => return None,
    };
    let indices: Vec<u32> = match mesh.try_indices_option().ok()? {
        Some(i) => i.iter().map(|i| i as u32).collect(),
        None => (0..positions.len() as u32).collect(),
    };
    let st: Vec<Vec2> = positions.iter().map(|p| panes.pane_at(*p)).collect();
    // Output vertices as weights over a source triangle, and their own
    // texture coordinates.
    let mut out: Vec<([u32; 3], Vec3, Option<Vec2>)> = Vec::new();
    let mut tris: Vec<[u32; 3]> = Vec::new();
    for t in indices.chunks_exact(3) {
        let tri = [t[0], t[1], t[2]];
        let pts = tri.map(|i| st[i as usize]);
        let lo = pts[0].min(pts[1]).min(pts[2]);
        let hi = pts[0].max(pts[1]).max(pts[2]);
        for rect in rects {
            if rect.hi.x <= lo.x || rect.lo.x >= hi.x || rect.hi.y <= lo.y || rect.lo.y >= hi.y {
                continue;
            }
            let poly = clip_triangle(pts, rect.lo, rect.hi);
            if poly.len() < 3 {
                continue;
            }
            let base = out.len() as u32;
            out.extend(poly.iter().map(|w| {
                let uv = rect.uv.map(|(o, x, y)| {
                    let p = pts[0] * w.x + pts[1] * w.y + pts[2] * w.z - o;
                    Vec2::new(p.dot(x), p.dot(y))
                });
                (tri, *w, uv)
            }));
            for k in 1..poly.len() as u32 - 1 {
                tris.push([base, base + k, base + k + 1]);
            }
        }
    }
    let mut clipped = Mesh::new(mesh.primitive_topology(), mesh.asset_usage);
    for (attribute, values) in mesh.try_attributes().ok()? {
        let mix = |get: &dyn Fn(usize) -> Vec4| -> Vec<Vec4> {
            out.iter()
                .map(|(t, w, _)| get(t[0] as usize) * w.x + get(t[1] as usize) * w.y + get(t[2] as usize) * w.z)
                .collect()
        };
        let values: VertexAttributeValues = match values {
            VertexAttributeValues::Float32x2(v) => mix(&|i| Vec2::from(v[i]).extend(0.0).extend(0.0))
                .into_iter()
                .map(|x| [x.x, x.y])
                .collect::<Vec<_>>()
                .into(),
            VertexAttributeValues::Float32x3(v) => mix(&|i| Vec3::from(v[i]).extend(0.0))
                .into_iter()
                .map(|x| [x.x, x.y, x.z])
                .collect::<Vec<_>>()
                .into(),
            VertexAttributeValues::Float32x4(v) => mix(&|i| Vec4::from(v[i]))
                .into_iter()
                .map(|x| x.to_array())
                .collect::<Vec<_>>()
                .into(),
            _ => continue,
        };
        clipped.insert_attribute(attribute.clone(), values);
    }
    if out.iter().any(|(_, _, uv)| uv.is_some()) {
        let uvs: Vec<[f32; 2]> = out.iter().map(|(_, _, uv)| uv.unwrap_or_default().to_array()).collect();
        clipped.insert_attribute(Mesh::ATTRIBUTE_UV_0, uvs);
    }
    clipped.insert_indices(Indices::U32(tris.into_iter().flatten().collect()));
    Some(clipped)
}

/// A triangle (2D points) clipped to a rectangle: the polygon's corners as
/// barycentric weights over the triangle.
fn clip_triangle(pts: [Vec2; 3], lo: Vec2, hi: Vec2) -> Vec<Vec3> {
    let mut poly: Vec<Vec3> = vec![Vec3::X, Vec3::Y, Vec3::Z];
    let at = |w: Vec3| pts[0] * w.x + pts[1] * w.y + pts[2] * w.z;
    // Half-planes: (axis, bound, keep below?).
    for (axis, bound, below) in [(0, lo.x, false), (0, hi.x, true), (1, lo.y, false), (1, hi.y, true)] {
        let inside = |w: Vec3| {
            let v = at(w)[axis];
            if below { v <= bound + 1e-5 } else { v >= bound - 1e-5 }
        };
        let mut next = Vec::with_capacity(poly.len() + 2);
        for i in 0..poly.len() {
            let (a, b) = (poly[i], poly[(i + 1) % poly.len()]);
            let (ia, ib) = (inside(a), inside(b));
            if ia {
                next.push(a);
            }
            if ia != ib {
                let (va, vb) = (at(a)[axis], at(b)[axis]);
                let t = ((bound - va) / (vb - va)).clamp(0.0, 1.0);
                next.push(a + (b - a) * t);
            }
        }
        poly = next;
        if poly.len() < 3 {
            return Vec::new();
        }
    }
    poly
}

// ---------------------------------------------------------------- gibs

/// Gib models ready to draw, and how they move.
#[derive(Resource)]
pub(super) struct GibAssets {
    sets: Vec<(String, Vec<GibModel>)>,
    physics: MapGibPhysics,
    light_scale: f32,
}

/// A gib model's meshes (none headless), each skin family's materials
/// for them when it has several, its bounds' centre, its smallest half
/// size (model space), and the body a prop's piece gets: its collider
/// about the centre, mass (kg), damping and surface.
struct GibModel {
    parts: Vec<(Handle<Mesh>, Handle<PropMaterial>)>,
    skins: Vec<Vec<Handle<PropMaterial>>>,
    centre: Vec3,
    radius: f32,
    body: GibBody,
}

#[derive(Clone)]
struct GibBody {
    collider: Collider,
    mass: f32,
    damping: (f32, f32),
    /// Surface property (impact sounds).
    surface: Option<String>,
}

/// A piece's body: its collision model's convex pieces (Source's client
/// prop pieces are physics objects), else its bounds' box; placed about
/// the bounds' centre.
fn gib_body(model: &MapModel) -> GibBody {
    let (lo, hi) = model.bounds;
    let centre = (lo + hi) / 2.0;
    let hulls: Vec<(Vec3, Quat, Collider)> = model
        .collision
        .iter()
        .flat_map(|c| &c.pieces)
        .filter_map(|p| Collider::convex_hull(p.points.clone()))
        .map(|hull| (-centre, Quat::IDENTITY, hull))
        .collect();
    let collider = if hulls.is_empty() {
        let size = (hi - lo).max(Vec3::splat(0.02));
        Collider::cuboid(size.x, size.y, size.z)
    } else {
        Collider::compound(hulls)
    };
    let c = model.collision.as_ref();
    GibBody {
        collider,
        mass: c.map_or(1.0, |c| c.mass).max(0.1),
        damping: c.map_or((0.0, 0.0), |c| (c.damping, c.rotdamping)),
        surface: c
            .map(|c| c.surfaceprop.clone())
            .or_else(|| model.surfaceprop.clone())
            .map(|s| s.trim().to_ascii_lowercase())
            .filter(|s| !s.is_empty()),
    }
}

/// Gib models, and their meshes when rendering (`render`).
pub(super) fn build_assets(
    data: &MapData,
    textures: &[Handle<Image>],
    view: MapDebugView,
    mut render: Option<(&mut Assets<Mesh>, &mut Assets<PropMaterial>)>,
) -> Option<GibAssets> {
    if data.gibs.is_empty() {
        return None;
    }
    let sets = data
        .gibs
        .iter()
        .map(|s| {
            let models = s
                .models
                .iter()
                .map(|model| {
                    let parts = match render.as_mut() {
                        Some((meshes, materials)) => model
                            .meshes
                            .iter()
                            .map(|m| {
                                let mut material = super::lit_prop_material(m, textures, data, view, false);
                                material.params.dynamic = 0.0;
                                (meshes.add(super::build_mesh(m, false)), materials.add(material))
                            })
                            .collect(),
                        None => Vec::new(),
                    };
                    let skins = match render.as_mut() {
                        Some((_, materials)) => model
                            .skins
                            .iter()
                            .map(|looks| {
                                model
                                    .meshes
                                    .iter()
                                    .zip(looks)
                                    .map(|(m, look)| {
                                        let mut material =
                                            super::lit_prop_material(&look.apply(m), textures, data, view, false);
                                        material.params.dynamic = 0.0;
                                        materials.add(material)
                                    })
                                    .collect()
                            })
                            .collect(),
                        None => Vec::new(),
                    };
                    let (lo, hi) = model.bounds;
                    GibModel {
                        parts,
                        skins,
                        centre: (lo + hi) / 2.0,
                        radius: ((hi - lo) / 2.0).min_element().max(0.0),
                        body: gib_body(model),
                    }
                })
                .collect();
            (s.name.clone(), models)
        })
        .collect();
    Some(GibAssets {
        sets,
        physics: data.gib_physics.unwrap_or_default(),
        light_scale: data.look.light_scale,
    })
}

/// A broken prop's piece with a surface: a client-side physics object
/// whose contacts make impact sounds.
#[derive(Component, Clone, Copy, Debug)]
pub struct PieceBody;

/// A flying (or resting) gib.
#[derive(Component)]
pub(super) struct FlyingGib {
    state: GibState,
    life: f32,
    glass: bool,
    /// Falls at the prop pieces' gravity.
    prop: bool,
    /// Kept this far off what it bounces on (its smallest half size).
    radius: f32,
    /// A physics body moves it (a prop's piece), not `state`.
    body: bool,
    materials: Vec<Handle<PropMaterial>>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GibState {
    pub position: Vec3,
    pub velocity: Vec3,
    pub rotation: Quat,
    pub spin: Vec3,
    pub age: f32,
    pub resting: bool,
}

/// One step of a gib: fall, spin and move, or bounce off `hit` (point and
/// normal of the first surface on the way), keeping `bounce` of its
/// velocity; it comes to rest on a floor once slow.
pub fn gib_step(s: &mut GibState, dt: f32, hit: Option<(Vec3, Vec3)>, gravity: f32, bounce: f32) {
    s.age += dt;
    if s.resting {
        return;
    }
    s.rotation = (Quat::from_scaled_axis(s.spin * dt) * s.rotation).normalize();
    match hit {
        None => {
            s.position += s.velocity * dt;
            s.velocity.y -= gravity * dt;
        }
        Some((point, normal)) => {
            s.position = point;
            s.velocity = (s.velocity - 2.0 * s.velocity.dot(normal) * normal) * bounce;
            s.spin *= bounce;
            if normal.y > 0.7 && s.velocity.length() < gravity * 0.1 {
                s.velocity = Vec3::ZERO;
                s.spin = Vec3::ZERO;
                s.resting = true;
            }
        }
    }
}

/// A gib's opacity at `age` (fading out over `fade` after `life`); None
/// once gone.
pub fn gib_alpha(age: f32, life: f32, fade: f32) -> Option<f32> {
    if age < life {
        return Some(1.0);
    }
    let a = 1.0 - (age - life) / fade.max(1e-6);
    (a > 0.0).then_some(a)
}

/// Spawn thrown gibs (at most `MAX_GIBS` alive).
#[allow(clippy::too_many_arguments)]
pub(super) fn spawn_gibs(
    mut events: MessageReader<SpawnGibs>,
    assets: Option<Res<GibAssets>>,
    light_field: Option<Res<LightField>>,
    mut materials: Option<ResMut<Assets<PropMaterial>>>,
    alive: Query<(), With<FlyingGib>>,
    gravity: Option<Res<Gravity>>,
    mut dice: Local<u32>,
    mut commands: Commands,
) {
    let Some(assets) = assets else {
        events.clear();
        return;
    };
    // Prop pieces fall at their own gravity.
    let world_gravity = gravity.map_or(9.81, |g| g.0.length());
    let gravity_scale = if world_gravity > 0.0 {
        assets.physics.prop_gravity / world_gravity
    } else {
        1.0
    };
    let mut count = alive.iter().count();
    for ev in events.read() {
        let Some((_, models)) = assets.sets.iter().find(|(n, _)| n.eq_ignore_ascii_case(&ev.set)) else {
            continue;
        };
        if models.is_empty() {
            continue;
        }
        for piece in &ev.pieces {
            if count >= MAX_GIBS {
                break;
            }
            count += 1;
            *dice = dice.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            let model = &models[piece.model.map_or((*dice >> 8) as usize, |m| m.min(models.len() - 1)) % models.len()];
            let parts = &model.parts;
            // Where it's lit and starts: its centre.
            let (rotation, centre) = match piece.rotation {
                Some(r) => (r, piece.position + r * model.centre),
                None => (
                    Quat::from_rotation_y(*dice as f32 * 1e-3) * Quat::from_rotation_x(*dice as f32 * 7e-4),
                    piece.position,
                ),
            };
            let offset = if piece.rotation.is_some() {
                -model.centre
            } else {
                Vec3::ZERO
            };
            let mut own = Vec::with_capacity(parts.len());
            // Its skin's materials (a skin it lacks shows skin 0).
            let skin = usize::try_from(piece.skin).ok().and_then(|s| model.skins.get(s));
            if let Some(materials) = materials.as_mut() {
                let probe = light_field.as_ref().map(|f| (f.0.0)(centre));
                for (i, (_, m)) in parts.iter().enumerate() {
                    let m = skin.and_then(|s| s.get(i)).unwrap_or(m);
                    let Some(mut material) = materials.get(m).cloned() else {
                        continue;
                    };
                    if let Some(probe) = &probe {
                        material.params.set_probe(probe, assets.light_scale);
                    }
                    if ev.glass {
                        material.alpha_mode = AlphaMode::Blend;
                        material.params.translucent = 1.0;
                        material.params.base_color.w = assets.physics.glass_alpha;
                    }
                    own.push(materials.add(material));
                }
            }
            let state = GibState {
                position: centre,
                velocity: piece.velocity,
                rotation,
                spin: piece.spin,
                age: 0.0,
                resting: piece.frozen,
            };
            // A prop's pieces are physics bodies (Source's client props):
            // they tumble, collide with the world, props and each other,
            // and come to rest; characters and shots pass through them
            // (the ragdoll layer: debris). Brush gibs fly on their own.
            let body = ev.prop;
            let mut gib = commands.spawn((
                Name::new("Gib"),
                MapPart,
                FlyingGib {
                    state,
                    life: piece.life,
                    glass: ev.glass,
                    prop: ev.prop,
                    radius: if ev.prop { model.radius } else { 0.0 },
                    body,
                    materials: own.clone(),
                },
                Transform::from_translation(centre).with_rotation(rotation),
                Visibility::default(),
            ));
            if body {
                let b = &model.body;
                // Its contacts make impact sounds (a client-side physics
                // object's, specs/cs_source/sounds.md 4).
                if let Some(surface) = &b.surface {
                    gib.insert((crate::core::PropSurface(surface.clone()), PieceBody, CollisionEventsEnabled));
                }
                // Before its collider: the contact tree reads it when the
                // collider joins.
                if let Some((size, shard)) = ev.shatters {
                    gib.insert((PanePiece { size, shard }, CollisionEventsEnabled));
                }
                gib.insert((
                    if piece.frozen {
                        RigidBody::Static
                    } else {
                        RigidBody::Dynamic
                    },
                    b.collider.clone(),
                    CollisionLayers::new(crate::core::RAGDOLL_LAYER, LayerMask::DEFAULT | crate::core::RAGDOLL_LAYER),
                    Mass(b.mass),
                    LinearVelocity(piece.velocity),
                    AngularVelocity(piece.spin),
                    LinearDamping(b.damping.0),
                    AngularDamping(b.damping.1.max(0.1)),
                    Friction::new(0.8),
                    Restitution::new(0.25),
                    GravityScale(gravity_scale),
                    MaxLinearSpeed(2000.0 * 0.0254),
                    MaxAngularSpeed(3600f32.to_radians()),
                ));
            }
            // Fade distances from the break block (a band dithers out;
            // equal ones pop).
            let range = piece.fade_dist.map(|(near, far)| {
                super::vis::fade_band(near, far)
                    .unwrap_or_else(|| bevy::camera::visibility::VisibilityRange::abrupt(0.0, far))
            });
            gib.with_children(|g| {
                for ((mesh, _), material) in parts.iter().zip(own) {
                    let mut c = g.spawn((
                        Mesh3d(mesh.clone()),
                        MeshMaterial3d(material),
                        Transform::from_translation(offset),
                        bevy::light::NotShadowCaster,
                    ));
                    if let Some(range) = &range {
                        c.insert(range.clone());
                    }
                }
            });
        }
    }
}

/// A falling window pane piece: it shatters on the first thing it
/// touches (the pane's size and the shards', meters).
#[derive(Component, Clone, Copy, Debug)]
pub struct PanePiece {
    pub size: Vec2,
    pub shard: f32,
}

/// Seconds a falling pane piece lasts if it never touches anything.
const PANE_PIECE_LIFE: f32 = 5.0;

/// Falling panes become pane pieces: physics debris of the
/// `PANE_PIECES` list in the pane's orientation, falling from its
/// corner (the model's origin there) and spinning.
pub(super) fn fall_panes(mut events: MessageReader<FallingPane>, mut gibs: MessageWriter<SpawnGibs>) {
    for ev in events.read() {
        let [u, v, n] = ev.axes.map(Vec3::normalize_or_zero);
        // The piece model is a square in its own Y/Z plane (Source axes;
        // engine -Z/Y), X its normal: those go onto the pane's axes.
        let rotation = Quat::from_mat3(&Mat3::from_cols(n, v, -u));
        let mut piece = GibPiece::thrown(ev.at, Vec3::ZERO, ev.spin, PANE_PIECE_LIFE);
        piece.model = Some(ev.body);
        piece.rotation = Some(rotation);
        gibs.write(SpawnGibs {
            set: PANE_PIECES.to_string(),
            glass: false,
            prop: true,
            pieces: vec![piece],
            shatters: Some((ev.size, ev.shard)),
        });
    }
}

/// Pane pieces that touched something shatter there.
pub(super) fn shatter_pane_pieces(
    mut started: MessageReader<CollisionStart>,
    pieces: Query<(&PanePiece, &Position, &Rotation, &LinearVelocity)>,
    mut shatter: MessageWriter<GlassShatter>,
    mut commands: Commands,
    mut done: Local<Vec<Entity>>,
) {
    done.clear();
    for s in started.read() {
        for e in [s.body1.unwrap_or(s.collider1), s.body2.unwrap_or(s.collider2)] {
            let Ok((piece, at, rot, v)) = pieces.get(e) else { continue };
            if done.contains(&e) {
                continue;
            }
            done.push(e);
            shatter.write(GlassShatter {
                at: at.0,
                normal: rot.0 * Vec3::X,
                size: piece.size,
                velocity: v.0,
                tile: false,
                shard: piece.shard,
            });
            commands.entity(e).try_despawn();
        }
    }
}

/// Move gibs, bounce them off the world (not characters), fade them out.
pub(super) fn fly_gibs(
    time: Res<Time>,
    assets: Option<Res<GibAssets>>,
    spatial: SpatialQuery,
    characters: Query<Entity, With<crate::core::Intent>>,
    mut gibs: Query<(Entity, &mut FlyingGib, &mut Transform)>,
    mut materials: Option<ResMut<Assets<PropMaterial>>>,
    mut commands: Commands,
) {
    let Some(assets) = assets else { return };
    let p = assets.physics;
    let dt = time.delta_secs().min(0.1);
    let filter = SpatialQueryFilter::from_excluded_entities(characters.iter()).with_mask(crate::core::NOT_SHADOW);
    for (e, mut gib, mut transform) in &mut gibs {
        if gib.body {
            // Its body moves it; it only fades.
            gib.state.age += dt;
            if gib_alpha(gib.state.age, gib.life, p.fade).is_none() {
                commands.entity(e).try_despawn();
                continue;
            }
            fade(&gib, p, materials.as_deref_mut());
            continue;
        }
        let bounce = if gib.glass { p.glass_bounce } else { p.bounce };
        let gravity = if gib.prop { p.prop_gravity } else { p.gravity };
        let s = gib.state;
        let r = gib.radius;
        let travel = s.velocity * dt;
        let hit = if s.resting || dt <= 0.0 {
            None
        } else {
            Dir3::new(travel).ok().and_then(|dir| {
                spatial
                    .cast_ray(s.position, dir, travel.length() + r, true, &filter)
                    .map(|h| {
                        let back = (h.distance - r).max(0.0);
                        (s.position + dir * back + h.normal * 0.01, h.normal)
                    })
            })
        };
        gib_step(&mut gib.state, dt, hit, gravity, bounce);
        let Some(alpha) = gib_alpha(gib.state.age, gib.life, p.fade) else {
            commands.entity(e).try_despawn();
            continue;
        };
        if alpha < 1.0 {
            fade(&gib, p, materials.as_deref_mut());
        }
        if transform.translation != gib.state.position || transform.rotation != gib.state.rotation {
            transform.translation = gib.state.position;
            transform.rotation = gib.state.rotation;
        }
    }
}

/// A fading gib's materials take its opacity.
fn fade(gib: &FlyingGib, p: MapGibPhysics, materials: Option<&mut Assets<PropMaterial>>) {
    let Some(alpha) = gib_alpha(gib.state.age, gib.life, p.fade).filter(|a| *a < 1.0) else {
        return;
    };
    let Some(materials) = materials else { return };
    let base = if gib.glass { p.glass_alpha } else { 1.0 };
    for m in &gib.materials {
        if let Some(mut material) = materials.get_mut(m) {
            material.alpha_mode = AlphaMode::Blend;
            material.params.translucent = 1.0;
            material.params.base_color.w = alpha * base;
        }
    }
}

/// A round restart (`core::RoundRestarts` counted up): gibs and
/// particles (glass shards) go, and windows drawn as panes are whole
/// again: their meshes uncut and their collider the brush's volumes. The
/// logic layer puts the nodes themselves back (shown, solid, placed).
pub(super) fn round_restart(world: &mut World, mut seen: Local<Option<u32>>) {
    let count = world.get_resource::<crate::core::RoundRestarts>().map_or(0, |r| r.0);
    if seen.replace(count).is_none_or(|s| s == count) {
        return;
    }
    let gibs: Vec<Entity> = world.query_filtered::<Entity, With<FlyingGib>>().iter(world).collect();
    for g in gibs {
        world.entity_mut(g).despawn();
    }
    if let Some(mut p) = world.get_resource_mut::<super::particles::Particles>() {
        p.groups.clear();
    }
    let windows: Vec<(Entity, usize)> = world
        .query_filtered::<(Entity, &super::MapBrushEntity), With<BrushPanes>>()
        .iter(world)
        .map(|(e, n)| (e, n.0))
        .collect();
    for (node, index) in windows {
        make_whole(world, node, index);
    }
}

/// A window node back to its whole brush.
fn make_whole(world: &mut World, node: Entity, index: usize) {
    let collider = world
        .get_resource::<super::MapEntities>()
        .and_then(|m| Some(super::entities::brush_collider(m.entities.get(index)?, m.scale)))
        .flatten();
    let children: Vec<Entity> = world
        .get::<Children>(node)
        .map(|c| c.iter().collect())
        .unwrap_or_default();
    let mut e = world.entity_mut(node);
    e.remove::<(BrushPanes, ColliderDisabled)>();
    e.insert(CollisionLayers::default());
    if let Some(c) = collider {
        e.insert(c);
    }
    for child in children {
        let Some(source) = world.get::<PaneSource>(child).map(|s| s.0.clone()) else {
            continue;
        };
        // The face as placed again, its broken looks hidden.
        let want = if world.get::<PanePart>(child).is_some() {
            Visibility::Hidden
        } else {
            Visibility::Inherited
        };
        if let Some(mut v) = world.get_mut::<Visibility>(child) {
            v.set_if_neq(want);
        }
        let old = world.get::<Mesh3d>(child).map(|m| m.0.clone());
        world
            .entity_mut(child)
            .insert(Mesh3d(source.clone()))
            .remove::<PaneSource>();
        if let (Some(old), Some(mut meshes)) = (old, world.get_resource_mut::<Assets<Mesh>>())
            && old != source
        {
            meshes.remove(&old);
        }
    }
}

/// Remove gibs (map unload).
pub(super) fn unload(world: &mut World) {
    world.remove_resource::<GibAssets>();
    let gibs: Vec<Entity> = world.query_filtered::<Entity, With<FlyingGib>>().iter(world).collect();
    for g in gibs {
        world.entity_mut(g).despawn();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn panes(cols: usize, rows: usize) -> BrushPanes {
        BrushPanes {
            corner: Vec3::ZERO,
            u: Vec3::X,
            v: Vec3::Y,
            depth: (-0.1, 0.0),
            cols,
            rows,
            broken: vec![false; cols * rows],
        }
    }

    #[test]
    fn clipping_keeps_only_unbroken_panes() {
        // A 2x1 quad over two panes; break the right one.
        let mut mesh = Mesh::new(
            bevy::mesh::PrimitiveTopology::TriangleList,
            bevy::asset::RenderAssetUsages::default(),
        );
        mesh.insert_attribute(
            Mesh::ATTRIBUTE_POSITION,
            vec![[0.0, 0.0, 0.0], [2.0, 0.0, 0.0], [2.0, 1.0, 0.0], [0.0, 1.0, 0.0]],
        );
        mesh.insert_attribute(
            Mesh::ATTRIBUTE_UV_0,
            vec![[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]],
        );
        mesh.insert_indices(Indices::U32(vec![0, 1, 2, 0, 2, 3]));
        let mut p = panes(2, 1);
        let whole = clip_to_panes(&mesh, &p).unwrap();
        let area = |m: &Mesh| {
            let Some(VertexAttributeValues::Float32x3(pos)) = m.attribute(Mesh::ATTRIBUTE_POSITION) else {
                panic!()
            };
            let idx: Vec<usize> = m.indices().unwrap().iter().collect();
            idx.chunks(3)
                .map(|t| {
                    let [a, b, c] = [t[0], t[1], t[2]].map(|i| Vec3::from(pos[i]));
                    (b - a).cross(c - a).length() / 2.0
                })
                .sum::<f32>()
        };
        assert!((area(&whole) - 2.0).abs() < 1e-4);
        p.broken[1] = true;
        let half = clip_to_panes(&mesh, &p).unwrap();
        assert!((area(&half) - 1.0).abs() < 1e-4, "{}", area(&half));
        // UVs follow: nothing past u = 0.5 is left.
        let Some(VertexAttributeValues::Float32x2(uv)) = half.attribute(Mesh::ATTRIBUTE_UV_0) else {
            panic!()
        };
        assert!(uv.iter().all(|u| u[0] <= 0.5 + 1e-5));
    }

    #[test]
    fn pane_boxes_cover_unbroken_panes() {
        let mut p = panes(3, 2);
        assert_eq!(p.boxes().len(), 6);
        p.broken[0] = true;
        let boxes = p.boxes();
        assert_eq!(boxes.len(), 5);
        let (c, _, s) = boxes[0];
        assert!((c - Vec3::new(1.5, 0.5, -0.05)).length() < 1e-5, "{c}");
        assert!((s - Vec3::new(1.0, 1.0, 0.1)).length() < 1e-5);
    }

    #[test]
    fn gibs_fall_bounce_and_rest() {
        let mut s = GibState {
            position: Vec3::Y,
            velocity: Vec3::ZERO,
            rotation: Quat::IDENTITY,
            spin: Vec3::ZERO,
            age: 0.0,
            resting: false,
        };
        gib_step(&mut s, 0.1, None, 5.0, 0.3);
        assert!((s.velocity.y + 0.5).abs() < 1e-5);
        gib_step(&mut s, 0.1, Some((Vec3::ZERO, Vec3::Y)), 1.0, 0.3);
        assert!(
            (s.velocity.y - 0.15).abs() < 1e-5,
            "bounced up at 0.3: {}",
            s.velocity.y
        );
        s.velocity = Vec3::new(0.0, -0.1, 0.0);
        gib_step(&mut s, 0.1, Some((Vec3::ZERO, Vec3::Y)), 5.0, 0.3);
        assert!(s.resting);
        assert_eq!(gib_alpha(1.0, 2.5, 1.0), Some(1.0));
        assert_eq!(gib_alpha(3.0, 2.5, 1.0), Some(0.5));
        assert_eq!(gib_alpha(3.6, 2.5, 1.0), None);
    }
}
