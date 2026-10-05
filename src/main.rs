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
        mashup::games::cs_source::movement::SourceMovementPlugin,
        mashup::map::world_material::WorldMaterialPlugin,
        mashup::map::rope_material::RopeMaterialPlugin,
        mashup::map::sprite_material::SpriteMaterialPlugin,
    ))
    // A game's maps run at that game's server tick.
    .insert_resource(match args.map.as_deref() {
        Some(id) if id.starts_with("cs_source:") => {
            Time::<Fixed>::from_seconds(mashup::games::cs_source::TICK_INTERVAL)
        }
        _ => Time::<Fixed>::from_hz(mashup::DEFAULT_TICK_HZ),
    })
    .insert_resource(movement_config(&args))
    .insert_resource(Loadout {
        movement: movement::placeholder::ID,
    })
    .add_plugins(client::ClientPlugin { args })
    .run();
}

/// Source movement settings: CS:S's, then `--exec` files, then `--cvar`s.
fn movement_config(args: &client::Args) -> mashup::games::cs_source::movement::SourceMovementConfig {
    let mut config = mashup::games::cs_source::movement::SourceMovementConfig::default();
    for file in &args.exec {
        match std::fs::read_to_string(file) {
            Ok(text) => {
                for problem in config.exec(&text) {
                    eprintln!("warning: {}: {problem}", file.display());
                }
            }
            Err(e) => {
                eprintln!("error: --exec {}: {e}", file.display());
                std::process::exit(2);
            }
        }
    }
    for (name, value) in &args.cvars {
        if let Err(e) = config.set_cvar(name, value) {
            eprintln!("error: --cvar {name}={value}: {e}");
            std::process::exit(2);
        }
    }
    config
}
