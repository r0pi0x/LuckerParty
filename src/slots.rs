//! Slot registries. A game plugin registers its implementations here; the
//! loadout picks one per slot by ID. Nothing outside a game plugin names that
//! plugin's types.

use bevy::prelude::*;

/// One Movement implementation. Each implementation is driven by a marker
/// component: its systems only act on entities that have it, so swapping
/// movement is "remove one marker, insert another".
pub struct MovementImpl {
    /// Namespaced ID, e.g. `combat_arms:movement`.
    pub id: &'static str,
    pub insert: fn(&mut EntityWorldMut),
    pub remove: fn(&mut EntityWorldMut),
}

#[derive(Resource, Default)]
pub struct MovementRegistry(pub Vec<MovementImpl>);

impl MovementRegistry {
    pub fn get(&self, id: &str) -> Option<&MovementImpl> {
        self.0.iter().find(|m| m.id == id)
    }

    /// The implementation registered after `id`, wrapping around.
    pub fn next_after(&self, id: &str) -> Option<&MovementImpl> {
        let i = self.0.iter().position(|m| m.id == id)?;
        self.0.get((i + 1) % self.0.len())
    }
}

/// Which Movement implementation an entity currently uses.
#[derive(Component, Debug, Clone, Copy)]
pub struct MovementSlot(pub &'static str);

/// The match loadout: one implementation ID per slot. Owned by the server in
/// multiplayer (README, "Multiplayer").
#[derive(Resource, Debug, Clone)]
pub struct Loadout {
    pub movement: &'static str,
}

pub trait RegisterSlots {
    /// Register a Movement implementation driven by marker component `M`.
    fn register_movement<M: Component + Default>(&mut self, id: &'static str) -> &mut Self;
}

impl RegisterSlots for App {
    fn register_movement<M: Component + Default>(&mut self, id: &'static str) -> &mut Self {
        fn insert<M: Component + Default>(e: &mut EntityWorldMut) {
            e.insert(M::default());
        }
        fn remove<M: Component>(e: &mut EntityWorldMut) {
            e.remove::<M>();
        }
        self.init_resource::<MovementRegistry>()
            .world_mut()
            .resource_mut::<MovementRegistry>()
            .0
            .push(MovementImpl {
                id,
                insert: insert::<M>,
                remove: remove::<M>,
            });
        self
    }
}

/// Switch `entity` to the Movement implementation `id`. Velocity and
/// movement state carry over; the new implementation takes it from there.
pub fn set_movement(entity: Entity, id: &'static str) -> impl Command {
    move |world: &mut World| {
        let registry = world.resource::<MovementRegistry>();
        let Some(new) = registry.get(id) else {
            warn!("unknown movement implementation {id}");
            return;
        };
        let insert = new.insert;
        let removes: Vec<_> = registry.0.iter().map(|m| m.remove).collect();
        let Ok(mut e) = world.get_entity_mut(entity) else {
            return;
        };
        for remove in removes {
            remove(&mut e);
        }
        insert(&mut e);
        e.insert(MovementSlot(id));
        info!("{entity}: movement -> {id}");
    }
}
