//! Moving models lit as Source lights them: from the map's baked light at
//! their position (`LightField`: ambient cube and nearby lights), re-sampled
//! when they move, instead of real-time lights and shadow maps. Character
//! bodies, the weapons they hold and loose items use it; each has its own
//! material instances.

use bevy::prelude::*;

use super::{LightField, prop_material::PropMaterial};

/// Lit from the light field at this entity's position (plus `offset`).
#[derive(Component, Clone, Debug, Default)]
pub struct ProbeLit {
    pub materials: Vec<Handle<PropMaterial>>,
    /// Added to the entity's position before sampling (e.g. a body's
    /// centre above its feet).
    pub offset: Vec3,
    lit_at: Option<Vec3>,
}

impl ProbeLit {
    pub fn new(materials: Vec<Handle<PropMaterial>>, offset: Vec3) -> Self {
        Self {
            materials,
            offset,
            lit_at: None,
        }
    }
}

/// The map's light scale for probe lighting (`MapData::look.light_scale`).
#[derive(Resource, Clone, Copy, Debug)]
pub struct ProbeLightScale(pub f32);

/// Re-sample after moving this far, meters.
const RELIGHT_DISTANCE: f32 = 0.25;

/// A copy of a shared material for one model instance.
pub fn instance(materials: &mut Assets<PropMaterial>, shared: &Handle<PropMaterial>) -> Handle<PropMaterial> {
    match materials.get(shared).cloned() {
        Some(m) => materials.add(m),
        None => shared.clone(),
    }
}

pub(super) fn relight(
    field: Option<Res<LightField>>,
    scale: Option<Res<ProbeLightScale>>,
    mut lit: Query<(&mut ProbeLit, &GlobalTransform)>,
    materials: Option<ResMut<Assets<PropMaterial>>>,
) {
    let (Some(field), Some(mut materials)) = (field, materials) else { return };
    let scale = scale.map_or(1.0, |s| s.0);
    for (mut l, at) in &mut lit {
        let p = at.translation() + l.offset;
        if l.lit_at.is_some_and(|q| q.distance(p) < RELIGHT_DISTANCE) {
            continue;
        }
        let probe = (field.0.0)(p);
        for h in &l.materials {
            if let Some(mut m) = materials.get_mut(h)
                && m.params.probe > 0.5
            {
                m.params.set_probe(&probe, scale);
            }
        }
        l.lit_at = Some(p);
    }
}
