//! Contacts only where something uses them.
//!
//! avian computes contacts for every pair of overlapping colliders except
//! static-static, every physics step. Between two bodies that aren't
//! dynamic (a character's capsule against the world, a door against its
//! frame) they never become constraints: only collision events could use
//! them, and those need `CollisionEventsEnabled` on one side. Characters
//! (our own movement, not avian's) and movers carry
//! `ActiveCollisionHooks::FILTER_PAIRS`, and `MapCollisionHooks` drops
//! such pairs before the narrow phase.

use avian3d::prelude::*;
use bevy::{ecs::system::SystemParam, prelude::*};

/// Put on a collider whose pairs with other non-dynamic bodies are
/// dropped (see the module docs).
pub fn hooks() -> ActiveCollisionHooks {
    ActiveCollisionHooks::FILTER_PAIRS
}

#[derive(SystemParam)]
pub struct UsedContacts<'w, 's> {
    colliders: Query<'w, 's, (Option<&'static ColliderOf>, Has<CollisionEventsEnabled>)>,
    bodies: Query<'w, 's, &'static RigidBody>,
}

impl UsedContacts<'_, '_> {
    /// Whether contacts between colliders `a` and `b` can be used: one
    /// belongs to a dynamic body (or to none), or reports its contacts.
    pub fn used(&self, a: Entity, b: Entity) -> bool {
        let used = |e: Entity| {
            let Ok((of, events)) = self.colliders.get(e) else {
                return true;
            };
            events || of.is_none_or(|of| self.bodies.get(of.body).is_ok_and(|b| b.is_dynamic()))
        };
        used(a) || used(b)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn app() -> App {
        let mut app = App::new();
        app.add_plugins((
            MinimalPlugins,
            TransformPlugin,
            AssetPlugin::default(),
            bevy::mesh::MeshPlugin,
            PhysicsPlugins::default().with_collision_hooks::<super::super::MapCollisionHooks>(),
        ))
        .insert_resource(bevy::time::TimeUpdateStrategy::ManualDuration(Duration::from_secs_f64(
            1.0 / 64.0,
        )));
        app.finish();
        app.cleanup();
        app.update();
        app
    }

    /// Whether avian keeps a contact pair for the two (touching or not).
    fn paired(app: &App, a: Entity, b: Entity) -> bool {
        app.world().resource::<ContactGraph>().contains(a, b)
    }

    #[test]
    fn non_dynamic_pairs_are_dropped_unless_reported() {
        let mut app = app();
        let floor = app
            .world_mut()
            .spawn((RigidBody::Static, Collider::cuboid(10.0, 1.0, 10.0), Transform::default()))
            .id();
        // A character standing in the floor, a door in it, a reporting
        // static prop, and a crate resting on it.
        let character = app
            .world_mut()
            .spawn((RigidBody::Kinematic, Collider::capsule(0.4, 1.0), hooks(), Transform::from_xyz(0.0, 1.0, 0.0)))
            .id();
        let door = app
            .world_mut()
            .spawn((RigidBody::Kinematic, Collider::cuboid(1.0, 2.0, 0.1), hooks(), Transform::from_xyz(3.0, 1.0, 0.0)))
            .id();
        let reporting = app
            .world_mut()
            .spawn((
                RigidBody::Static,
                Collider::cuboid(1.0, 1.0, 1.0),
                CollisionEventsEnabled,
                Transform::from_xyz(3.0, 1.0, 0.5),
            ))
            .id();
        let crate_ = app
            .world_mut()
            .spawn((RigidBody::Dynamic, Collider::cuboid(1.0, 1.0, 1.0), Transform::from_xyz(0.5, 1.0, 0.0)))
            .id();
        for _ in 0..3 {
            app.update();
        }
        assert!(!paired(&app, character, floor), "character and world");
        assert!(!paired(&app, door, floor), "door and world");
        assert!(paired(&app, door, reporting), "a reporting collider keeps its contacts");
        assert!(paired(&app, character, crate_), "a character pushes a crate");
        assert!(paired(&app, crate_, floor), "the crate rests on the world");
    }
}
