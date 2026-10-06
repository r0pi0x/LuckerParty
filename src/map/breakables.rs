//! Breakable brushes for any game, the drawing and physics side: gibs
//! (small models thrown when a brush breaks, flying with gravity, bouncing
//! off the world and fading out; they never block anything) and windows
//! made of panes (`BrushPanes` on a brush entity node: only the unbroken
//! panes are drawn and hit by shots). What breaks, and when, is the logic
//! layer's; games load the gib models (`MapData::gibs`) and draw the
//! shards of shattered glass (`GlassShatter`).
//! Engine space: meters, Y up.

use avian3d::prelude::*;
use bevy::{
    mesh::{Indices, VertexAttributeValues},
    prelude::*,
};

use super::{LightField, MapData, MapDebugView, MapModel, MapPart, prop_material::PropMaterial};

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
}

impl Default for MapGibPhysics {
    fn default() -> Self {
        Self {
            gravity: 9.81 / 2.0,
            bounce: 0.5,
            glass_bounce: 0.3,
            glass_alpha: 0.5,
            fade: 1.0,
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
}

/// Throw gibs from a gib list (a model is picked at random per piece).
#[derive(Message, Clone, Debug)]
pub struct SpawnGibs {
    pub set: String,
    /// Glass: translucent and less bouncy.
    pub glass: bool,
    pub pieces: Vec<GibPiece>,
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
    mut parts: Query<(&mut Mesh3d, Option<&PaneSource>)>,
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
                .insert(Collider::compound(shapes))
                .remove::<ColliderDisabled>();
        }
        let (Some(meshes), Some(children)) = (meshes.as_mut(), children) else {
            continue;
        };
        for child in children.iter() {
            let Ok((mut mesh3d, source)) = parts.get_mut(child) else {
                continue;
            };
            let source = match source {
                Some(s) => s.0.clone(),
                None => {
                    let s = mesh3d.0.clone();
                    commands.entity(child).insert(PaneSource(s.clone()));
                    s
                }
            };
            let Some(clipped) = meshes.get(&source).and_then(|m| clip_to_panes(m, panes)) else {
                continue;
            };
            let old = std::mem::replace(&mut mesh3d.0, meshes.add(clipped));
            if old != source {
                meshes.remove(&old);
            }
        }
    }
}

/// The part of a triangle mesh over unbroken panes: every triangle
/// clipped to each unbroken pane it overlaps (in the window's plane), its
/// float attributes interpolated.
pub fn clip_to_panes(mesh: &Mesh, panes: &BrushPanes) -> Option<Mesh> {
    let positions: Vec<Vec3> = match mesh.try_attribute(Mesh::ATTRIBUTE_POSITION).ok()? {
        VertexAttributeValues::Float32x3(v) => v.iter().map(|p| Vec3::from(*p)).collect(),
        _ => return None,
    };
    let indices: Vec<u32> = match mesh.try_indices_option().ok()? {
        Some(i) => i.iter().map(|i| i as u32).collect(),
        None => (0..positions.len() as u32).collect(),
    };
    let st: Vec<Vec2> = positions.iter().map(|p| panes.pane_at(*p)).collect();
    // Output vertices as weights over a source triangle.
    let mut out: Vec<([u32; 3], Vec3)> = Vec::new();
    let mut tris: Vec<[u32; 3]> = Vec::new();
    for t in indices.chunks_exact(3) {
        let tri = [t[0], t[1], t[2]];
        let pts = tri.map(|i| st[i as usize]);
        let lo = pts[0].min(pts[1]).min(pts[2]);
        let hi = pts[0].max(pts[1]).max(pts[2]);
        let c0 = (lo.x.floor().max(0.0) as usize).min(panes.cols);
        let c1 = (hi.x.ceil().max(0.0) as usize).min(panes.cols);
        let r0 = (lo.y.floor().max(0.0) as usize).min(panes.rows);
        let r1 = (hi.y.ceil().max(0.0) as usize).min(panes.rows);
        for r in r0..r1 {
            for c in c0..c1 {
                if !panes.whole(c, r) {
                    continue;
                }
                let poly = clip_triangle(
                    pts,
                    Vec2::new(c as f32, r as f32),
                    Vec2::new(c as f32 + 1.0, r as f32 + 1.0),
                );
                if poly.len() < 3 {
                    continue;
                }
                let base = out.len() as u32;
                out.extend(poly.iter().map(|w| (tri, *w)));
                for k in 1..poly.len() as u32 - 1 {
                    tris.push([base, base + k, base + k + 1]);
                }
            }
        }
    }
    let mut clipped = Mesh::new(mesh.primitive_topology(), mesh.asset_usage);
    for (attribute, values) in mesh.try_attributes().ok()? {
        let mix = |get: &dyn Fn(usize) -> Vec4| -> Vec<Vec4> {
            out.iter()
                .map(|(t, w)| get(t[0] as usize) * w.x + get(t[1] as usize) * w.y + get(t[2] as usize) * w.z)
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
    sets: Vec<(String, Vec<Vec<(Handle<Mesh>, Handle<PropMaterial>)>>)>,
    physics: MapGibPhysics,
    light_scale: f32,
}

pub(super) fn build_assets(
    data: &MapData,
    textures: &[Handle<Image>],
    view: MapDebugView,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<PropMaterial>,
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
                    model
                        .meshes
                        .iter()
                        .map(|m| {
                            let mut material = super::lit_prop_material(m, textures, data, view, false);
                            material.params.dynamic = 0.0;
                            (meshes.add(super::build_mesh(m, false)), materials.add(material))
                        })
                        .collect()
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

/// A flying (or resting) gib.
#[derive(Component)]
pub(super) struct FlyingGib {
    state: GibState,
    life: f32,
    glass: bool,
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

/// Spawn thrown gibs.
#[allow(clippy::too_many_arguments)]
pub(super) fn spawn_gibs(
    mut events: MessageReader<SpawnGibs>,
    assets: Option<Res<GibAssets>>,
    light_field: Option<Res<LightField>>,
    mut materials: Option<ResMut<Assets<PropMaterial>>>,
    mut dice: Local<u32>,
    mut commands: Commands,
) {
    let Some(assets) = assets else {
        events.clear();
        return;
    };
    for ev in events.read() {
        let Some((_, models)) = assets.sets.iter().find(|(n, _)| n.eq_ignore_ascii_case(&ev.set)) else {
            continue;
        };
        if models.is_empty() {
            continue;
        }
        for piece in &ev.pieces {
            *dice = dice.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            let parts = &models[(*dice >> 8) as usize % models.len()];
            let mut own = Vec::with_capacity(parts.len());
            if let Some(materials) = materials.as_mut() {
                let probe = light_field.as_ref().map(|f| (f.0.0)(piece.position));
                for (_, m) in parts {
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
            let rotation = Quat::from_rotation_y(*dice as f32 * 1e-3) * Quat::from_rotation_x(*dice as f32 * 7e-4);
            let state = GibState {
                position: piece.position,
                velocity: piece.velocity,
                rotation,
                spin: piece.spin,
                age: 0.0,
                resting: false,
            };
            commands
                .spawn((
                    Name::new("Gib"),
                    MapPart,
                    FlyingGib {
                        state,
                        life: piece.life,
                        glass: ev.glass,
                        materials: own.clone(),
                    },
                    Transform::from_translation(piece.position).with_rotation(rotation),
                    Visibility::default(),
                ))
                .with_children(|g| {
                    for ((mesh, _), material) in parts.iter().zip(own) {
                        g.spawn((
                            Mesh3d(mesh.clone()),
                            MeshMaterial3d(material),
                            bevy::light::NotShadowCaster,
                        ));
                    }
                });
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
    let filter = SpatialQueryFilter::from_excluded_entities(characters.iter());
    for (e, mut gib, mut transform) in &mut gibs {
        let bounce = if gib.glass { p.glass_bounce } else { p.bounce };
        let s = gib.state;
        let travel = s.velocity * dt;
        let hit = if s.resting || dt <= 0.0 {
            None
        } else {
            Dir3::new(travel).ok().and_then(|dir| {
                spatial
                    .cast_ray(s.position, dir, travel.length(), true, &filter)
                    .map(|h| (s.position + dir * h.distance + h.normal * 0.01, h.normal))
            })
        };
        gib_step(&mut gib.state, dt, hit, p.gravity, bounce);
        let Some(alpha) = gib_alpha(gib.state.age, gib.life, p.fade) else {
            commands.entity(e).try_despawn();
            continue;
        };
        if alpha < 1.0
            && let Some(materials) = materials.as_mut()
        {
            let base = if gib.glass { p.glass_alpha } else { 1.0 };
            for m in &gib.materials {
                if let Some(mut material) = materials.get_mut(m) {
                    material.alpha_mode = AlphaMode::Blend;
                    material.params.translucent = 1.0;
                    material.params.base_color.w = alpha * base;
                }
            }
        }
        transform.translation = gib.state.position;
        transform.rotation = gib.state.rotation;
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
    if let Some(c) = collider {
        e.insert(c);
    }
    for child in children {
        let Some(source) = world.get::<PaneSource>(child).map(|s| s.0.clone()) else {
            continue;
        };
        let old = world.get::<Mesh3d>(child).map(|m| m.0.clone());
        world.entity_mut(child).insert(Mesh3d(source.clone())).remove::<PaneSource>();
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
