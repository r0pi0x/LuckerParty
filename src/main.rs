mod character;
mod core;
mod debug;
mod greybox;
mod input;
mod movement;
mod slots;

use avian3d::prelude::*;
use bevy::prelude::*;

fn main() {
    App::new()
        .add_plugins((
            DefaultPlugins.set(WindowPlugin {
                primary_window: Some(Window {
                    title: "mashup".into(),
                    ..default()
                }),
                ..default()
            }),
            PhysicsPlugins::default(),
        ))
        // Placeholder tick rate until a ruleset or movement spec sets one.
        .insert_resource(Time::<Fixed>::from_hz(64.0))
        .insert_resource(slots::Loadout {
            movement: movement::placeholder::ID,
        })
        .add_plugins((
            core::CorePlugin,
            input::LocalInputPlugin,
            character::CharacterPlugin,
            movement::MovementPlugins,
            greybox::GreyboxMapPlugin,
            debug::DebugPlugin,
        ))
        .run();
}
