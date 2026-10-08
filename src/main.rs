use avian3d::prelude::*;
use bevy::prelude::*;
use mashup::{
    SimPlugins, client, games, greybox,
    map::{MapDebugView, MapPlugin},
    slots::Loadout,
};

fn main() {
    let args = client::Args::parse();
    let mut app = App::new();
    match args.game_map() {
        // At the HDR level the console will start with (mat_hdr_level).
        Some(id) => match games::load_map_level(id, client::hdr::startup_level(&args.console)) {
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
            // The greybox (behind the main menu unless the run starts
            // playing), and map systems without a map, so `map <name>` can
            // load one.
            app.add_plugins((greybox::GreyboxMapPlugin, MapPlugin::empty()));
        }
    }
    app.add_plugins((
        DefaultPlugins
            .set(WindowPlugin {
                primary_window: Some(Window {
                    title: "mashup".into(),
                    // Physical pixels whatever the display's scaling.
                    resolution: args
                        .window
                        .map(|s| bevy::window::WindowResolution::new(s.x, s.y).with_scale_factor_override(1.0))
                        .unwrap_or_default(),
                    // A fixed size: tiling window managers float such
                    // windows instead of resizing them.
                    resizable: args.window.is_none(),
                    resize_constraints: args
                        .window
                        .map(|s| bevy::window::WindowResizeConstraints {
                            min_width: s.x as f32,
                            min_height: s.y as f32,
                            max_width: s.x as f32,
                            max_height: s.y as f32,
                        })
                        .unwrap_or_default(),
                    ..default()
                }),
                ..default()
            })
            // Warnings and errors also show in the in-game console.
            .set(bevy::log::LogPlugin {
                custom_layer: client::console::log_layer,
                ..default()
            }),
        // Rendering eases between ticks with `map::interp` (characters'
        // eyes and looks too), not avian's transform interpolation.
        PhysicsPlugins::default()
            .with_collision_hooks::<mashup::map::MapCollisionHooks>()
            .build()
            .disable::<PhysicsInterpolationPlugin>(),
        SimPlugins,
        mashup::games::cs_source::movement::SourceMovementPlugin,
        mashup::games::cs_source::weapons::CsWeaponsPlugin,
        mashup::games::cs_source::player_anim::PlayerAnimPlugin,
        mashup::games::cs_source::view_anim::ViewAnimPlugin,
        mashup::map::world_material::WorldMaterialPlugin,
        mashup::map::rope_material::RopeMaterialPlugin,
        mashup::map::sprite_material::SpriteMaterialPlugin,
        mashup::map::prop_material::PropMaterialPlugin,
        mashup::map::shadows::ShadowMaterialPlugin,
        mashup::map::decal::DecalMaterialPlugin,
        mashup::map::water::WaterMaterialPlugin,
        mashup::map::particles::ParticleMaterialPlugin,
    ))
    // A game's maps run at that game's server tick.
    .insert_resource(match args.game_map() {
        Some(id) if id.starts_with("cs_source:") => {
            Time::<Fixed>::from_seconds(mashup::games::cs_source::TICK_INTERVAL)
        }
        _ => Time::<Fixed>::from_hz(mashup::DEFAULT_TICK_HZ),
    })
    .insert_resource(movement_config(&args))
    .insert_resource(Loadout {
        movement: mashup::games::cs_source::movement::ID,
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
