//! Loose items lying in the world, for any game (dropped weapons): a held
//! model (`MapHeldModel`) as a small physics body that players walk
//! through, like Source's debris.

use std::collections::HashMap;

use avian3d::prelude::*;
use bevy::prelude::*;

use super::{MapHeldModel, MapPart, PhysicsProp, PushAway};

/// A loose item drawn with a held model (its `MapHeldModel` key).
/// Spawners give it a `Transform` and velocities; the map adds the body
/// and the meshes, and removes it with the map.
#[derive(Component, Clone, Debug, PartialEq)]
pub struct LooseItem(pub String);

/// Kilograms (a rifle is about 4; the game's own weight isn't known).
const MASS: f32 = 3.0;

#[derive(Resource, Default)]
pub(super) struct LooseAssets(pub(super) HashMap<String, LooseAsset>);

pub(super) struct LooseAsset {
    parts: Vec<(Handle<Mesh>, Handle<super::prop_material::PropMaterial>)>,
    /// From the held model's space (skeleton axes and units) to the item's
    /// frame, centred on its bounds.
    frame: Transform,
    half: Vec3,
}

/// A held model's loose form: `root` takes the skeleton's space to meters
/// (a character model's `root`; its translation is ignored).
pub(super) fn asset(
    held: &MapHeldModel,
    root: Transform,
    parts: Vec<(Handle<Mesh>, Handle<super::prop_material::PropMaterial>)>,
) -> LooseAsset {
    let to = Transform::from_rotation(root.rotation).with_scale(root.scale);
    let (lo, hi) = held
        .model
        .meshes
        .iter()
        .flat_map(|m| &m.positions)
        .map(|p| to.transform_point(Vec3::from(*p)))
        .fold((Vec3::splat(f32::MAX), Vec3::splat(f32::MIN)), |(lo, hi), p| {
            (lo.min(p), hi.max(p))
        });
    let (centre, half) = if lo.x <= hi.x {
        ((lo + hi) / 2.0, ((hi - lo) / 2.0).max(Vec3::splat(0.01)))
    } else {
        (Vec3::ZERO, Vec3::splat(0.1))
    };
    LooseAsset {
        parts,
        frame: to.with_translation(-centre),
        half,
    }
}

/// Give new loose items their body and meshes.
pub(super) fn attach_loose(
    items: Query<(Entity, &LooseItem), Without<RigidBody>>,
    assets: Option<Res<LooseAssets>>,
    mut materials: Option<ResMut<Assets<super::prop_material::PropMaterial>>>,
    mut commands: Commands,
) {
    let Some(assets) = assets else { return };
    for (e, item) in &items {
        let half = assets.0.get(&item.0).map_or(Vec3::splat(0.1), |a| a.half);
        let mut ent = commands.entity(e);
        ent.insert((
            RigidBody::Dynamic,
            Collider::cuboid(half.x * 2.0, half.y * 2.0, half.z * 2.0),
            Mass(MASS),
            Friction::new(0.8),
            Restitution::new(0.1),
            LinearDamping(0.1),
            AngularDamping(1.0),
            PhysicsProp {
                push: PushAway::Ignore,
                mass: MASS,
                bounds: (-half, half),
            },
            MapPart,
            Visibility::default(),
        ));
        if let Some(a) = assets.0.get(&item.0) {
            // Its own materials, lit where it lies (probe_lit).
            let own: Vec<_> = a
                .parts
                .iter()
                .map(|(_, m)| match materials.as_mut() {
                    Some(assets) => super::probe_lit::instance(assets, m),
                    None => m.clone(),
                })
                .collect();
            ent.insert(super::probe_lit::ProbeLit::new(own.clone(), Vec3::ZERO));
            ent.with_children(|c| {
                for ((mesh, _), material) in a.parts.iter().zip(own) {
                    c.spawn((Mesh3d(mesh.clone()), MeshMaterial3d(material), a.frame));
                }
            });
        }
    }
}

/// An item drawn with a held model's meshes (its `MapHeldModel` key) and
/// no physics body: something that moves itself (a thrown grenade).
/// Spawners give it a `Transform`; the map adds the meshes.
#[derive(Component, Clone, Debug, PartialEq)]
pub struct ShownItem(pub String);

/// A `ShownItem` whose model rests on its transform's origin (its
/// bounds' bottom there) instead of being centred on it: something set
/// down on the ground (a planted bomb).
/// The rotation turns the model first (one whose own up isn't the
/// held-model frame's).
#[derive(Component, Clone, Copy, Debug, Default)]
pub struct SitOnOrigin(pub Quat);

/// Marks a `ShownItem` whose meshes were added.
#[derive(Component)]
pub(super) struct ShownItemDrawn;

/// Give new shown items their meshes.
pub(super) fn attach_shown(
    items: Query<(Entity, &ShownItem, Option<&SitOnOrigin>), Without<ShownItemDrawn>>,
    assets: Option<Res<LooseAssets>>,
    mut materials: Option<ResMut<Assets<super::prop_material::PropMaterial>>>,
    mut commands: Commands,
) {
    let Some(assets) = assets else { return };
    for (e, item, sit) in &items {
        let mut ent = commands.entity(e);
        ent.insert((ShownItemDrawn, MapPart, Visibility::default()));
        if let Some(a) = assets.0.get(&item.0) {
            let mut frame = a.frame;
            if let Some(SitOnOrigin(turn)) = sit {
                let m = Mat3::from_quat(*turn);
                let half = m.x_axis.abs() * a.half.x + m.y_axis.abs() * a.half.y + m.z_axis.abs() * a.half.z;
                frame = Transform::from_rotation(*turn) * frame;
                frame.translation.y += half.y;
            }
            // Its own materials, lit where it is (probe_lit).
            let own: Vec<_> = a
                .parts
                .iter()
                .map(|(_, m)| match materials.as_mut() {
                    Some(assets) => super::probe_lit::instance(assets, m),
                    None => m.clone(),
                })
                .collect();
            ent.insert(super::probe_lit::ProbeLit::new(own.clone(), Vec3::ZERO));
            ent.with_children(|c| {
                for ((mesh, _), material) in a.parts.iter().zip(own) {
                    c.spawn((Mesh3d(mesh.clone()), MeshMaterial3d(material), frame));
                }
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::map::{MapMesh, MapModel};

    #[test]
    fn loose_items_are_centred_and_scaled() {
        let mut model = MapModel::default();
        model.meshes.push(MapMesh {
            positions: vec![[0.0, 0.0, 0.0], [40.0, 4.0, 2.0]],
            ..default()
        });
        let held = MapHeldModel {
            key: "gun".into(),
            model,
            bone: "b".into(),
            muzzle: None,
        };
        let a = asset(&held, Transform::from_scale(Vec3::splat(0.0254)), Vec::new());
        assert!((a.half - Vec3::new(0.508, 0.0508, 0.0254)).length() < 1e-5);
        // The model's far corner lands at +half.
        let corner = a.frame.transform_point(Vec3::new(40.0, 4.0, 2.0));
        assert!((corner - a.half).length() < 1e-5);
    }
}
