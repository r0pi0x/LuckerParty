//! Copies of map nodes made at run time (a point_template's brush and
//! prop members spawned again: specs/source/viewcontrol_and_templates.md
//! 2.2): the node and everything under it cloned, without the physics
//! engine's per-body bookkeeping (island and tree keys: the copy gets its
//! own when its body and colliders are added), drawn by its own meshes
//! (never merged), visible wherever it is (the original's visibility
//! clusters don't apply), and not replicated (a network client doesn't
//! see copies yet). `MapCopy` marks them so a round restart removes them.

use avian3d::collider_tree::ColliderTreeProxyKey;
use avian3d::dynamics::solver::islands::BodyIslandNode;
use bevy::prelude::*;

use super::merge::{MergedBrush, MergedPiece};
use super::vis::{FadeDistance, Occludee, VisClusters};

/// A node copied at run time.
#[derive(Component, Clone, Copy, Debug)]
pub struct MapCopy;

/// Copy `node` (and its children) at `pose`; returns the copy.
pub fn copy_node(world: &mut World, node: Entity, pose: Transform) -> Option<Entity> {
    world.get_entity(node).ok()?;
    let copy = world.entity_mut(node).clone_and_spawn_with_opt_out(|b| {
        b.linked_cloning(true)
            .deny::<(BodyIslandNode, ColliderTreeProxyKey)>()
            .deny::<(MergedBrush, VisClusters, Occludee, FadeDistance)>()
            .deny::<bevy_replicon::prelude::Replicated>();
    });
    // Pieces drawn merged in the original are drawn by themselves here.
    show_merged_pieces(world, node, copy);
    let mut e = world.entity_mut(copy);
    e.insert((MapCopy, pose, Visibility::Inherited));
    if let Some(mut p) = e.get_mut::<avian3d::prelude::Position>() {
        p.0 = pose.translation;
    }
    if let Some(mut r) = e.get_mut::<avian3d::prelude::Rotation>() {
        *r = avian3d::prelude::Rotation::from(pose.rotation);
    }
    Some(copy)
}

/// Walk the original and its copy together: a copied piece the original
/// draws merged is shown.
fn show_merged_pieces(world: &mut World, original: Entity, copy: Entity) {
    let a: Vec<Entity> = world.get::<Children>(original).map(|c| c.to_vec()).unwrap_or_default();
    let b: Vec<Entity> = world.get::<Children>(copy).map(|c| c.to_vec()).unwrap_or_default();
    for (x, y) in a.into_iter().zip(b) {
        if world.get::<MergedPiece>(x).is_some()
            && let Ok(mut e) = world.get_entity_mut(y)
        {
            e.remove::<MergedPiece>().insert(Visibility::Inherited);
        }
        show_merged_pieces(world, x, y);
    }
}
