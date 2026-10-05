use avian3d::prelude::*;
use bevy::prelude::*;
use mashup::{
    SimPlugins, client, games, greybox,
    map::{MapDebugView, MapPlugin},
    movement,
    slots::Loadout,
};

fn main() {
    let args = client::Args::parse();
    let mut app = App::new();
    match &args.map {
        Some(id) => match games::load_map(id) {
            Ok(data) => {
                eprintln!(
                    "loaded {}: {} triangles, {} textures, {} spawns, {} warnings",
                    data.name,
                    data.triangle_count(),
                    data.textures.len(),
                    data.spawns.len(),
                    data.warnings.len()
                );
                for w in &data.warnings {
                    eprintln!("  warning: {w}");
                }
                let view = match (args.debug_view.as_deref(), args.lightmap_only) {
                    (Some("lighting"), _) => MapDebugView::Lighting { scale: 0.25 },
                    (Some("albedo"), _) => MapDebugView::Albedo,
                    (Some(other), _) => {
                        eprintln!("error: --debug-view {other}: expected lighting or albedo");
                        std::process::exit(2);
                    }
                    (None, true) => MapDebugView::Lighting { scale: 1.0 },
                    (None, false) => MapDebugView::Normal,
                };
                app.add_plugins(MapPlugin {
                    view,
                    ..MapPlugin::new(data)
                });
            }
            Err(e) => {
                eprintln!("error: --map {id}: {e}");
                std::process::exit(1);
            }
        },
        None => {
            app.add_plugins(greybox::GreyboxMapPlugin);
        }
    }
    app.add_plugins((
        DefaultPlugins.set(WindowPlugin {
            primary_window: Some(Window {
                title: "mashup".into(),
                ..default()
            }),
            ..default()
        }),
        PhysicsPlugins::default(),
        SimPlugins,
    ))
    .insert_resource(Loadout {
        movement: movement::placeholder::ID,
    })
    .add_plugins(client::ClientPlugin { args })
    .run();
}
