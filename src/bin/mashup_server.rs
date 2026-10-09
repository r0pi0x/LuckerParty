//! The dedicated server: the simulation and `net` with no window, Source's
//! srcds in spirit. Console on stdin (`status`, `bot_add`, `bot_quota`,
//! `changelevel`, `quit`, any cvar), output on stdout.
//!
//!   mashup_server [-port <n>] [+map <name>] [+maxplayers <n>] [+<command> ...]
//!
//! `+map greybox` (the default) or a CS:S map from the install
//! (`+map de_dust2`). Players join with `connect <ip>:<port>`;
//! `changelevel <map>` (or `map <map>`) moves them all to another map.

use std::{
    io::BufRead,
    sync::{Arc, Mutex, mpsc},
    time::Duration,
};

use bevy::{app::ScheduleRunnerPlugin, prelude::*};
use mashup::{
    console::{Console, ConsoleAppExt},
    games::{self, cs_source},
    greybox::GreyboxMapPlugin,
    harness,
    map::{LoadedMapName, MapPlugin},
    net::{self, NetPlugin, NetSettings},
    slots::Loadout,
};

const USAGE: &str = "\
usage: mashup_server [-port <n>] [+map <name>] [+maxplayers <n>] [+<command> [args...]]
  -port <n>        UDP port (hostport; default 27015)
  +map <name>      greybox (default) or a CS:S map from the install, e.g. de_dust2
  +<command>       any console command or cvar, e.g. +maxplayers 8 +bot_quota 4
Type console commands on stdin: status, bot_add, bot_kick, bot_quota, changelevel <map>, quit.";

struct Options {
    map: String,
    console: Vec<String>,
}

fn parse() -> Result<Options, String> {
    let mut map = net::GREYBOX.to_string();
    let mut console = Vec::new();
    let mut it = std::env::args().skip(1).peekable();
    while let Some(arg) = it.next() {
        if arg == "-h" || arg == "--help" {
            println!("{USAGE}");
            std::process::exit(0);
        }
        if arg == "-port" {
            let port = it.next().ok_or("-port: missing value")?;
            port.parse::<u16>().map_err(|_| format!("-port: not a port: {port}"))?;
            console.insert(0, format!("hostport {port}"));
            continue;
        }
        let Some(name) = arg.strip_prefix('+').filter(|n| !n.is_empty()) else {
            return Err(format!("unknown option {arg}"));
        };
        let mut words = vec![name.to_string()];
        while let Some(next) = it.peek() {
            if next.starts_with('+') && next.len() > 1 || next == "-port" {
                break;
            }
            words.push(it.next().unwrap());
        }
        if name.eq_ignore_ascii_case("map") {
            map = words.get(1).cloned().ok_or("+map: missing name")?;
        } else {
            console.push(
                words
                    .iter()
                    .map(|w| mashup::console::quote(w))
                    .collect::<Vec<_>>()
                    .join(" "),
            );
        }
    }
    Ok(Options { map, console })
}

fn main() {
    let options = parse().unwrap_or_else(|e| {
        eprintln!("{e}\n\n{USAGE}");
        std::process::exit(2);
    });
    let mut app = App::new();
    // A frame every millisecond at most: fixed ticks keep real time without
    // spinning a core.
    harness::add_headless_with(
        &mut app,
        MinimalPlugins.set(ScheduleRunnerPlugin::run_loop(Duration::from_millis(1))),
    );
    app.add_plugins(bevy::log::LogPlugin::default());
    let map_id = net::normalize_map(&options.map);
    if map_id == net::GREYBOX {
        // The map systems too: `changelevel` loads one later.
        app.add_plugins((GreyboxMapPlugin, MapPlugin::empty()))
            .insert_resource(Time::<Fixed>::from_hz(mashup::DEFAULT_TICK_HZ));
    } else {
        let id = if map_id.contains(':') {
            map_id.clone()
        } else {
            format!("{}:{map_id}", cs_source::GAME)
        };
        match games::load_map_level(&id, 0) {
            Ok(data) => {
                eprintln!(
                    "loaded {id}: {} spawns, {} warnings",
                    data.spawns.len(),
                    data.warnings.len()
                );
                app.add_plugins(MapPlugin::new(data))
                    .insert_resource(Time::<Fixed>::from_seconds(cs_source::TICK_INTERVAL))
                    .insert_resource(LoadedMapName(id));
            }
            Err(e) => {
                eprintln!("error: +map {id}: {e}");
                std::process::exit(1);
            }
        }
    }
    app.add_plugins((
        NetPlugin,
        cs_source::movement::SourceMovementPlugin,
        cs_source::weapons::CsWeaponsPlugin,
    ))
    .insert_resource(Loadout {
        movement: cs_source::movement::ID,
    })
    // The served map's file (clients without it download it), as the game
    // reads it.
    .insert_resource(net::maps::MapFiles {
        read: Arc::new(games::map_file_bytes),
        cache: mashup::mount::config::content_dir(cs_source::GAME),
    })
    .add_systems(Update, finish_level_change)
    .console_command(
        "changelevel",
        "changelevel <map>: move the server and its players to another map (greybox or a CS:S map).",
        |w, a| change_level(w, a.first().ok_or("changelevel <map>")?),
    )
    .console_command(
        "map",
        "map <map>: the same as changelevel on a dedicated server.",
        |w, a| change_level(w, a.first().ok_or("map <map>")?),
    )
    // A dedicated server takes eight players unless told otherwise.
    .insert_resource(NetSettings {
        maxplayers: 8,
        ..default()
    })
    .add_systems(Update, (read_stdin, start_serving, print_console).chain())
    .console_command("quit", "Stop the server.", |w, _| {
        net::disconnect(w, "Server shutting down.");
        w.write_message(AppExit::Success);
        Ok(None)
    })
    .console_command("exit", "Stop the server.", |w, _| {
        net::disconnect(w, "Server shutting down.");
        w.write_message(AppExit::Success);
        Ok(None)
    });
    {
        let mut console = app.world_mut().resource_mut::<Console>();
        for line in &options.console {
            console.submit(line.clone());
        }
    }
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        for line in std::io::stdin().lock().lines() {
            let Ok(line) = line else { break };
            if tx.send(line).is_err() {
                break;
            }
        }
    });
    app.insert_resource(Stdin(Mutex::new(rx)));
    app.run();
}

/// Lines typed on stdin.
#[derive(Resource)]
struct Stdin(Mutex<mpsc::Receiver<String>>);

/// A map loading in the background for `changelevel`.
#[derive(Resource)]
struct LevelLoad {
    id: String,
    task: bevy::tasks::Task<Result<mashup::map::MapData, String>>,
}

/// `changelevel <map>`: the greybox at once; a CS:S map loaded in the
/// background (the game plays on), then swapped in (`finish_level_change`).
fn change_level(world: &mut World, name: &str) -> Result<Option<String>, String> {
    let id = net::normalize_map(name);
    if id == net::GREYBOX {
        mashup::swap_map(
            world,
            net::GREYBOX,
            None,
            Duration::from_secs_f64(1.0 / mashup::DEFAULT_TICK_HZ),
        );
        return Ok(Some("changed level to the greybox".into()));
    }
    let id = if id.contains(':') {
        id
    } else {
        format!("{}:{id}", cs_source::GAME)
    };
    if world.contains_resource::<LevelLoad>() {
        return Err("a map is loading already".into());
    }
    // Clients hear of it now (`net::maps`: the map's name changes first).
    world.insert_resource(LoadedMapName(id.clone()));
    let task = bevy::tasks::AsyncComputeTaskPool::get().spawn({
        let id = id.clone();
        async move { games::load_map_level(&id, 0) }
    });
    world.insert_resource(LevelLoad { id: id.clone(), task });
    Ok(Some(format!("changing level to {id}...")))
}

fn finish_level_change(world: &mut World) {
    let Some(mut load) = world.get_resource_mut::<LevelLoad>() else {
        return;
    };
    let Some(result) = bevy::tasks::block_on(bevy::tasks::poll_once(&mut load.task)) else {
        return;
    };
    let id = load.id.clone();
    world.remove_resource::<LevelLoad>();
    match result {
        Ok(data) => {
            let summary = format!("loaded {id}: {} spawns", data.spawns.len());
            mashup::swap_map(
                world,
                &id,
                Some(data),
                Duration::from_secs_f64(cs_source::TICK_INTERVAL),
            );
            world.resource_mut::<Console>().info(summary);
        }
        Err(e) => {
            // Back to the map that plays: clients never left it.
            let back = world
                .get_resource::<mashup::map::MapFile>()
                .map(|f| format!("{}:{}", cs_source::GAME, f.name))
                .unwrap_or_else(|| net::GREYBOX.into());
            world.insert_resource(LoadedMapName(back));
            world
                .resource_mut::<Console>()
                .print(mashup::console::Level::Error, format!("changelevel {id}: {e}"));
        }
    }
}

fn read_stdin(stdin: Res<Stdin>, mut console: ResMut<Console>) {
    let rx = stdin.0.lock().unwrap();
    while let Ok(line) = rx.try_recv() {
        console.print(mashup::console::Level::Input, format!("] {line}"));
        console.submit(line);
    }
}

/// Listen once the command line's console lines have run (they set the
/// port and player count).
fn start_serving(world: &mut World, mut started: Local<bool>) {
    if *started {
        return;
    }
    *started = true;
    match net::server::listen(world) {
        Ok(addr) => {
            let max = world.resource::<NetSettings>().maxplayers;
            let map = net::current_map(world);
            world.resource_mut::<Console>().info(format!(
                "mashup dedicated server {}: {map} on UDP {addr}, {max} players",
                net::NET_VERSION
            ));
        }
        Err(e) => {
            eprintln!("error: {e}");
            world.write_message(AppExit::error());
        }
    }
}

/// Console output (command results, errors) to stdout.
fn print_console(console: Res<Console>, mut printed: Local<u64>) {
    let new = console.printed.saturating_sub(*printed) as usize;
    *printed = console.printed;
    let lines = &console.output;
    for line in &lines[lines.len().saturating_sub(new)..] {
        if line.level != mashup::console::Level::Input {
            println!("{}", line.text);
        }
    }
}
