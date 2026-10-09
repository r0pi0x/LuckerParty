//! What the game does about the network (`net`): load the server's map
//! when it asks (by name, or a copy it downloaded), show joining and the
//! server's map changes in the loading dialog (`game_menu`), leave the
//! menu once joined, and come back to the main menu (a fresh local player
//! on the greybox) when dropped, with why in the dialog.

use std::sync::Arc;

use bevy::prelude::*;

use crate::{
    console::{Console, Level},
    core::{LocalPlayer, NetRole},
    net::{NetEvent, NetSettings, maps::MapFiles},
};

pub struct ClientNetPlugin;

impl Plugin for ClientNetPlugin {
    fn build(&self, app: &mut App) {
        // Map files as the game loads them (the install, its downloads,
        // mashup's content cache), for the server's offer and the client's
        // checks and downloads.
        let game = crate::games::cs_source::GAME;
        app.insert_resource(MapFiles {
            read: Arc::new(crate::games::map_file_bytes),
            cache: crate::mount::config::content_dir(game),
        })
        .add_systems(Update, react);
    }
}

fn react(world: &mut World, mut cursor: Local<bevy::ecs::message::MessageCursor<NetEvent>>) {
    let events: Vec<NetEvent> = cursor.read(world.resource::<Messages<NetEvent>>()).cloned().collect();
    for e in events {
        match e {
            NetEvent::Listening(addr) => {
                let map = crate::net::current_map(world);
                world.resource_mut::<Console>().info(format!(
                    "Hosting {map} on UDP port {} (players join with: connect <your ip>:{})",
                    addr.port(),
                    addr.port()
                ));
            }
            NetEvent::Connecting(addr) => {
                world.resource_mut::<Console>().info(format!("Connecting to {addr}..."));
                super::game_menu::joining(world, &addr.to_string());
            }
            NetEvent::LoadMap { map, file } => {
                super::console::load_server_map(world, &map, file);
                let name = crate::net::maps::map_name(&map).to_string();
                super::game_menu::joining(world, &name);
            }
            NetEvent::ChangingLevel(map) => {
                world
                    .resource_mut::<Console>()
                    .info(format!("The server is changing level to {map}..."));
                let name = crate::net::maps::map_name(&map).to_string();
                super::game_menu::joining(world, &name);
            }
            NetEvent::Joined { map } => {
                world
                    .resource_mut::<Console>()
                    .info(format!("Joined the game on {map}."));
                super::game_menu::entered_game(world);
            }
            NetEvent::Disconnected(reason) => {
                world
                    .resource_mut::<Console>()
                    .print(Level::Warn, format!("Disconnected: {reason}"));
                back_to_menu(world);
            }
        }
    }
}

/// After leaving a server: our own player back on the greybox behind the
/// main menu, as `disconnect` leaves single player; the dialog shows why
/// if the game ended on its own (`net::client::JoinProgress::failure`).
fn back_to_menu(world: &mut World) {
    let failed = world
        .get_resource::<crate::net::client::JoinProgress>()
        .is_some_and(|p| p.failure.is_some());
    let has_player = world
        .query_filtered::<(), With<LocalPlayer>>()
        .iter(world)
        .next()
        .is_some();
    if has_player {
        if failed {
            super::game_menu::left_game(world);
            super::game_menu::show_failure(world);
        }
        return;
    }
    super::console::load_greybox(world);
    super::game_menu::left_game(world);
    if failed {
        super::game_menu::show_failure(world);
    }
    if let Err(e) = world.run_system_cached(super::spawn_local_player) {
        error!("spawning the local player: {e}");
    }
}

/// A map finished loading (or the greybox came back): host it when
/// `maxplayers` asks for other players (Source's `maxplayers 8; map x`).
pub(super) fn listen_if_hosting(world: &mut World) {
    let standalone = world
        .get_resource::<NetRole>()
        .is_none_or(|r| *r == NetRole::Standalone);
    let wanted = world.get_resource::<NetSettings>().is_some_and(|s| s.maxplayers > 1);
    if !standalone || !wanted {
        return;
    }
    if let Err(e) = crate::net::server::listen(world) {
        world
            .resource_mut::<Console>()
            .print(Level::Error, format!("Can't host: {e}"));
    }
}
