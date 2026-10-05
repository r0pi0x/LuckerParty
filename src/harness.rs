//! Headless simulation for tests and tools: no window, no rendering, time
//! advanced by exact fixed ticks. Scenario tests in `tests/` build on this.

use std::time::Duration;

use avian3d::prelude::*;
use bevy::{prelude::*, time::TimeUpdateStrategy};

use crate::{
    DEFAULT_TICK_HZ, SimPlugins,
    character::character_bundle,
    core::{Intent, MovementState, SimTick, Team, Velocity},
    slots::set_movement,
};

pub struct Sim {
    pub app: App,
}

impl Sim {
    /// A headless app with the simulation plugins plus `plugins` (a map,
    /// and any game plugins whose implementations the test uses).
    pub fn new<M>(plugins: impl bevy::app::Plugins<M>) -> Self {
        let mut app = App::new();
        app.add_plugins((
            MinimalPlugins,
            TransformPlugin,
            // avian watches mesh assets even when no collider uses them.
            AssetPlugin::default(),
            bevy::mesh::MeshPlugin,
            // Interpolation eases rendering between ticks; headless, it
            // would make `Transform` show the previous tick.
            PhysicsPlugins::default()
                .build()
                .disable::<PhysicsInterpolationPlugin>(),
            SimPlugins,
        ))
        .add_plugins(plugins)
        .insert_resource(TimeUpdateStrategy::ManualDuration(Duration::from_secs_f64(
            1.0 / DEFAULT_TICK_HZ,
        )));
        app.finish();
        app.cleanup();
        // Startup, and one tick so the physics spatial index contains the map.
        app.update();
        let mut sim = Self { app };
        sim.ticks(1);
        sim
    }

    /// Run fixed ticks of `secs` from now on (e.g. a game's own tick).
    pub fn set_tick_interval(&mut self, secs: f64) {
        let step = Duration::from_secs_f64(secs);
        self.app.insert_resource(Time::<Fixed>::from_duration(step));
        self.app.insert_resource(TimeUpdateStrategy::ManualDuration(step));
    }

    pub fn tick_hz(&self) -> f64 {
        self.app
            .world()
            .resource::<Time<Fixed>>()
            .timestep()
            .as_secs_f64()
            .recip()
    }

    /// Advance exactly `n` fixed ticks.
    pub fn ticks(&mut self, n: u64) {
        let target = self.tick() + n;
        while self.tick() < target {
            self.app.update();
        }
    }

    /// Advance by whole ticks covering `secs` seconds.
    pub fn seconds(&mut self, secs: f64) {
        self.ticks((secs * self.tick_hz()).round() as u64);
    }

    pub fn tick(&self) -> u64 {
        self.app.world().resource::<SimTick>().0
    }

    pub fn spawn_character(&mut self, position: Vec3, movement: &'static str) -> Entity {
        let world = self.app.world_mut();
        let entity = world
            .spawn(character_bundle(Transform::from_translation(position), Team(0)))
            .id();
        set_movement(entity, movement).apply(world);
        entity
    }

    pub fn set_movement(&mut self, entity: Entity, movement: &'static str) {
        set_movement(entity, movement).apply(self.app.world_mut());
    }

    pub fn intent(&mut self, entity: Entity) -> Mut<'_, Intent> {
        self.app
            .world_mut()
            .get_mut::<Intent>(entity)
            .expect("entity has no Intent")
    }

    pub fn position(&self, entity: Entity) -> Vec3 {
        self.app
            .world()
            .get::<Transform>(entity)
            .expect("no Transform")
            .translation
    }

    pub fn velocity(&self, entity: Entity) -> Vec3 {
        self.app.world().get::<Velocity>(entity).expect("no Velocity").0
    }

    pub fn state(&self, entity: Entity) -> MovementState {
        self.app
            .world()
            .get::<MovementState>(entity)
            .expect("no MovementState")
            .clone()
    }
}
