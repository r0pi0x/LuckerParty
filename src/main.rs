use avian3d::prelude::*;
use bevy::prelude::*;
use mashup::{SimPlugins, client, greybox, movement, slots::Loadout};

fn main() {
    let args = client::Args::parse();
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
            SimPlugins,
            greybox::GreyboxMapPlugin,
        ))
        .insert_resource(Loadout {
            movement: movement::placeholder::ID,
        })
        .add_plugins(client::ClientPlugin { args })
        .run();
}
