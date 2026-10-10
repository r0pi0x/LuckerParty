//! Models' levels of detail (Source's `.vtx` LODs): a prop whose model
//! has lower levels draws the one its size on screen asks for, as
//! Source's `r_lod -1` picks by a screen-size metric against each level's
//! switch point, never finer than Video > Advanced's model detail
//! (`r_rootlod`: High 0, Medium 1, Low 2). Each of a prop's meshes keeps
//! its levels' meshes (`LodMeshes`) and swaps its `Mesh3d` when the level
//! changes (`switch_lods`), so skins, body groups, fading and culling are
//! untouched. The metric is our reading (no spec): the camera's view
//! height at the model over its bounding sphere's diameter, so 100 / the
//! percent of the screen's height the model fills; docs/tech-debt.md.

use bevy::prelude::*;

use super::MapMesh;

/// One lower level of detail of a model: the metric it takes over at (the
/// `.vtx` switch point) and its meshes, by the slot of the LOD 0 mesh each
/// stands in for (None: none at this level; the finer level's is drawn).
#[derive(Clone, Debug, Default)]
pub struct MapLod {
    pub switch: f32,
    pub meshes: Vec<Option<MapMesh>>,
}

/// Video > Advanced's model detail and Source's LOD cvars: `r_rootlod`
/// (the finest level drawn: 0 High, 1 Medium, 2 Low) and `r_lod` (-1: by
/// the metric; n: always level n). Both apply at once (CS:S's
/// `r_rootlod` waits for the next map load).
#[derive(Resource, Clone, Copy, Debug, PartialEq)]
pub struct ModelSettings {
    pub root_lod: u8,
    pub lod: i32,
}

impl Default for ModelSettings {
    fn default() -> Self {
        Self { root_lod: 0, lod: -1 }
    }
}

/// The LOD metric of a model of bounding radius `radius` at `distance`
/// (any one length unit), seen with `tan_half_fov` the tangent of half
/// the vertical field of view: the view's height there over the model's
/// diameter (Source's fov 90 at 4:3: 0.75 x distance / radius).
pub fn metric(distance: f32, radius: f32, tan_half_fov: f32) -> f32 {
    distance * tan_half_fov / radius.max(1e-4)
}

/// The level drawn among `switches` (each level's switch point, LOD 0's
/// first): the last one whose switch point the metric has passed, or
/// `r_lod`'s when set, never finer than `r_rootlod`, at most the last.
pub fn pick(switches: &[f32], metric: f32, settings: &ModelSettings) -> usize {
    let last = switches.len().saturating_sub(1);
    let wanted = if settings.lod >= 0 {
        settings.lod as usize
    } else {
        switches.iter().rposition(|s| *s <= metric).unwrap_or(0)
    };
    wanted.max(settings.root_lod as usize).min(last)
}

/// A prop drawn with levels of detail: its model's switch points (0 for
/// LOD 0 first), its bounding sphere in model space and the level drawn.
#[derive(Component, Clone, Debug)]
pub struct LodGroup {
    pub switches: Vec<f32>,
    pub centre: Vec3,
    pub radius: f32,
    pub current: usize,
}

impl LodGroup {
    /// For a model's bounds (min, max) and its levels.
    pub fn new(bounds: (Vec3, Vec3), lods: &[MapLod]) -> Self {
        Self {
            switches: std::iter::once(0.0).chain(lods.iter().map(|l| l.switch)).collect(),
            centre: (bounds.0 + bounds.1) / 2.0,
            radius: ((bounds.1 - bounds.0).length() / 2.0).max(1e-3),
            current: 0,
        }
    }
}

/// One of a prop's meshes at each level (LOD 0 first).
#[derive(Component, Clone, Debug)]
pub struct LodMeshes(pub Vec<Handle<Mesh>>);

/// The camera props are seen from (the first-person camera, also when
/// spectating; `ViewModelAnchor`): each prop with levels draws the one
/// its metric and the settings ask for.
#[allow(clippy::type_complexity)]
pub(super) fn switch_lods(
    settings: Option<Res<ModelSettings>>,
    cameras: Query<(&GlobalTransform, &Projection), With<super::ViewModelAnchor>>,
    mut groups: Query<(&GlobalTransform, &mut LodGroup, &Children)>,
    mut parts: Query<(&LodMeshes, &mut Mesh3d)>,
) {
    let settings = settings.as_deref().copied().unwrap_or_default();
    let camera = cameras.iter().next().map(|(t, p)| {
        let tan = match p {
            Projection::Perspective(p) => (p.fov / 2.0).tan(),
            _ => 0.75,
        };
        (t.translation(), tan)
    });
    for (at, mut group, children) in &mut groups {
        let metric = camera.map_or(0.0, |(eye, tan)| {
            let scale = at.scale().max_element().max(1e-3);
            metric(
                eye.distance(at.transform_point(group.centre)),
                group.radius * scale,
                tan,
            )
        });
        let want = pick(&group.switches, metric, &settings);
        if want == group.current {
            continue;
        }
        group.current = want;
        for child in children.iter() {
            if let Ok((levels, mut mesh)) = parts.get_mut(child)
                && let Some(h) = levels.0.get(want.min(levels.0.len().saturating_sub(1)))
                && mesh.0 != *h
            {
                mesh.0 = h.clone();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn levels_switch_by_screen_size_never_finer_than_the_root() {
        // A barrel's levels (CS:S's barrelwarning: 0, 25, 35, 45, ...).
        let switches = [0.0, 25.0, 35.0, 45.0];
        let auto = ModelSettings::default();
        // 25 units across at fov 90: LOD 0 up close, LOD 1 from 0.75
        // d / r = 25, d = 417 units, LOD 3 far off.
        let r = 12.5;
        assert_eq!(pick(&switches, metric(100.0, r, 0.75), &auto), 0);
        assert_eq!(pick(&switches, metric(420.0, r, 0.75), &auto), 1);
        assert_eq!(pick(&switches, metric(5000.0, r, 0.75), &auto), 3);
        // Zoomed in (a scope's narrower view): finer again.
        assert_eq!(pick(&switches, metric(420.0, r, 0.2), &auto), 0);
        // Model detail Low: never finer than LOD 2; r_lod forces one.
        let low = ModelSettings { root_lod: 2, lod: -1 };
        assert_eq!(pick(&switches, metric(100.0, r, 0.75), &low), 2);
        assert_eq!(pick(&switches, metric(5000.0, r, 0.75), &low), 3);
        assert_eq!(pick(&switches, 0.0, &ModelSettings { root_lod: 0, lod: 1 }), 1);
        assert_eq!(pick(&[0.0], 1e6, &low), 0, "a model with one level");
    }
}
