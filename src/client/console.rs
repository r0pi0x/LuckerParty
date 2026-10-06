//! The in-game console (docs/plans/active/console.md): toggled with `~`.
//! Drop-down output with colours by severity, scrollback, a filter and
//! timestamps; an input line with Tab completion (names, cvar values,
//! command arguments; repeated Tab cycles), ranked fuzzy suggestions as
//! you type, history (Up/Down, persisted, Ctrl+R reverse search), and the
//! usual editing keys. Binds run when it's closed (`+action` while held).
//! Also the CS:S-style overlays (cl_showpos, cl_showfps, snd_show, watch)
//! and the client's commands (noclip, getpos/setpos, kill, map, quit, ...).

use std::{
    collections::VecDeque,
    sync::{Arc, Mutex, OnceLock},
};

use bevy::{
    app::AppExit,
    input::{
        ButtonState,
        keyboard::{Key, KeyboardInput},
        mouse::{MouseScrollUnit, MouseWheel},
    },
    log::{BoxedLayer, tracing_subscriber::Layer},
    prelude::*,
    window::CursorOptions,
};

use super::FirstPersonCamera;
use crate::{
    console::{Console, ConsoleAppExt, Level, cfg_dir, parse, resource_cvar},
    core::{LocalPlayer, MovementState, SpawnPoint, Velocity},
    map::PlaySound,
    slots::{MovementSlot, set_movement},
};

pub struct ConsoleUiPlugin;

impl Plugin for ConsoleUiPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<ConsoleUi>()
            .init_resource::<Overlays>()
            .init_resource::<HeldActions>()
            .init_resource::<SoundMarks>()
            .add_systems(Startup, (spawn_ui, load_history, autoexec).chain())
            .add_systems(
                Update,
                (
                    toggle,
                    edit.run_if(|ui: Res<ConsoleUi>| ui.open),
                    run_binds.run_if(|ui: Res<ConsoleUi>| !ui.open),
                    drain_log,
                    record_sounds,
                    draw_console,
                    draw_overlays,
                    draw_notify,
                    finish_map_load,
                    draw_sound_marks,
                )
                    .chain(),
            )
            .add_systems(Update, overlay_text)
            .add_systems(Last, save_on_exit);
        overlay_cvars(app);
        client_commands(app);
    }
}

/// Whether the console is open, and its input state.
#[derive(Resource, Default)]
pub struct ConsoleUi {
    pub open: bool,
    input: String,
    /// Cursor position in chars.
    cursor: usize,
    history: Vec<String>,
    /// Position while browsing history (None: editing a new line).
    browsing: Option<usize>,
    /// Tab cycling: the candidates and which one is shown, and the input
    /// they complete.
    cycle: Option<(Vec<String>, usize, String)>,
    /// Ctrl+R: the query and how many matches back.
    search: Option<(String, usize)>,
    /// Lines scrolled up from the bottom.
    scroll: usize,
    pub filter: String,
    pub timestamps: bool,
}

const HISTORY_MAX: usize = 1000;
const VISIBLE_LINES: usize = 28;
const SUGGESTIONS: usize = 8;

#[derive(Component)]
struct ConsoleRoot;
#[derive(Component)]
struct ConsoleOutput;
#[derive(Component)]
struct ConsoleInput;
#[derive(Component)]
struct ConsoleSuggest;
#[derive(Component)]
struct OverlayText;

fn spawn_ui(mut commands: Commands) {
    let font = |size: f32| TextFont {
        font_size: FontSize::Px(size),
        ..default()
    };
    commands
        .spawn((
            ConsoleRoot,
            Node {
                position_type: PositionType::Absolute,
                top: px(0.0),
                left: px(0.0),
                width: percent(100.0),
                height: percent(50.0),
                flex_direction: FlexDirection::Column,
                justify_content: JustifyContent::FlexEnd,
                padding: UiRect::all(px(8.0)),
                ..default()
            },
            BackgroundColor(Color::srgba(0.05, 0.05, 0.07, 0.88)),
            GlobalZIndex(100),
            Visibility::Hidden,
        ))
        .with_children(|c| {
            c.spawn((
                ConsoleOutput,
                Text::default(),
                font(14.0),
                Node {
                    overflow: Overflow::clip(),
                    ..default()
                },
            ));
            c.spawn((
                ConsoleInput,
                Text::default(),
                font(15.0),
                TextColor(Color::srgb(1.0, 0.95, 0.6)),
                Node {
                    margin: UiRect::top(px(4.0)),
                    ..default()
                },
            ));
        });
    commands.spawn((
        ConsoleSuggest,
        Text::default(),
        font(14.0),
        Node {
            position_type: PositionType::Absolute,
            top: percent(50.0),
            left: px(8.0),
            padding: UiRect::all(px(6.0)),
            ..default()
        },
        BackgroundColor(Color::srgba(0.1, 0.1, 0.14, 0.95)),
        GlobalZIndex(101),
        Visibility::Hidden,
    ));
    commands.spawn((
        NotifyText,
        Text::default(),
        font(14.0),
        TextColor(Color::srgb(0.95, 0.95, 0.85)),
        Node {
            position_type: PositionType::Absolute,
            // Below the debug overlay's five lines.
            top: px(125.0),
            left: px(8.0),
            ..default()
        },
        GlobalZIndex(50),
        Visibility::Hidden,
    ));
    commands.spawn((
        OverlayText,
        Text::default(),
        font(14.0),
        TextColor(Color::srgb(0.9, 0.9, 0.9)),
        Node {
            position_type: PositionType::Absolute,
            top: px(8.0),
            right: px(10.0),
            ..default()
        },
        GlobalZIndex(50),
    ));
}

fn toggle(
    keys: Res<ButtonInput<KeyCode>>,
    mut ui: ResMut<ConsoleUi>,
    mut root: Single<&mut Visibility, With<ConsoleRoot>>,
    mut cursor: Single<&mut CursorOptions>,
) {
    let close = ui.open && keys.just_pressed(KeyCode::Escape) && ui.search.is_none();
    if keys.just_pressed(KeyCode::Backquote) || close {
        ui.open = !ui.open;
        **root = if ui.open {
            Visibility::Visible
        } else {
            Visibility::Hidden
        };
        if ui.open {
            // Typing shouldn't move the player: the game reads input only
            // while the mouse is grabbed.
            super::input::release_cursor(&mut cursor);
        } else if !close {
            // Closed with the console key: straight back to playing. (Escape
            // leaves the mouse free, as it does outside the console.)
            super::input::capture_cursor(&mut cursor);
        }
    }
}

/// Every completion candidate for the current input: names when on the
/// first word, else the command's arguments.
fn candidates(world: &mut World, input: &str) -> (String, Vec<(String, String)>) {
    let ends_space = input.ends_with(' ');
    let words: Vec<&str> = input.split_whitespace().collect();
    let (head, token) = if words.len() <= 1 && !ends_space {
        (String::new(), words.first().copied().unwrap_or(""))
    } else {
        let token = if ends_space {
            ""
        } else {
            words.last().copied().unwrap_or("")
        };
        let keep = if ends_space { words.len() } else { words.len() - 1 };
        (format!("{} ", words[..keep].join(" ")), token)
    };
    let console = world.resource::<Console>();
    let mut out: Vec<(String, String)> = Vec::new();
    if head.is_empty() {
        for v in console.cvars() {
            out.push((v.name.clone(), v.help.clone()));
        }
        for c in console.commands() {
            out.push((c.name.clone(), c.help.clone()));
        }
        for (a, body) in &console.aliases {
            out.push((a.clone(), format!("alias: {body}")));
        }
    } else {
        let cmd = words[0].to_lowercase();
        if let Some(v) = console.cvar(&cmd) {
            out.extend(v.values.iter().map(|x| (x.clone(), String::new())));
            out.push((v.default.clone(), "default".into()));
        }
        match cmd.as_str() {
            "help" | "reset" | "toggle" | "incrementvar" | "watch" | "unwatch" => {
                out.extend(console.cvars().map(|v| (v.name.clone(), v.help.clone())));
                if cmd == "help" {
                    out.extend(console.commands().map(|c| (c.name.clone(), c.help.clone())));
                }
            }
            "exec" | "execifexists" => {
                if let Some(dir) = cfg_dir()
                    && let Ok(files) = std::fs::read_dir(dir)
                {
                    out.extend(files.flatten().filter_map(|f| {
                        let n = f.file_name().to_string_lossy().to_string();
                        n.ends_with(".cfg")
                            .then(|| (n.trim_end_matches(".cfg").to_string(), String::new()))
                    }));
                }
            }
            "bind" | "unbind" if words.len() <= 2 && !(words.len() == 2 && ends_space) => {
                out.extend(KEY_NAMES.iter().map(|(_, n)| (n.to_string(), String::new())));
                out.extend(
                    [
                        "mouse1",
                        "mouse2",
                        "mouse3",
                        "mouse4",
                        "mouse5",
                        "mwheelup",
                        "mwheeldown",
                    ]
                    .map(|n| (n.to_string(), String::new())),
                );
            }
            "bind" => {
                out.extend(ACTIONS.iter().map(|a| (format!("+{a}"), String::new())));
                out.extend(console.commands().map(|c| (c.name.clone(), c.help.clone())));
            }
            "map" => out.extend(map_names().iter().map(|m| (m.clone(), String::new()))),
            "noclip" | "movement" => {}
            _ => {}
        }
        if let Some(c) = console.command(&cmd).cloned()
            && let Some(complete) = c.complete
        {
            out.extend(complete(world, token).into_iter().map(|x| (x, String::new())));
        }
    }
    let mut ranked: Vec<(i32, String, String)> = out
        .into_iter()
        .filter_map(|(n, h)| score(&n, token).map(|s| (s, n, h)))
        .collect();
    ranked.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.cmp(&b.1)));
    ranked.dedup_by(|a, b| a.1 == b.1);
    (head, ranked.into_iter().map(|(_, n, h)| (n, h)).collect())
}

/// Fuzzy rank (lower is better): prefix, then substring, then the letters
/// in order; None when it doesn't match.
fn score(name: &str, query: &str) -> Option<i32> {
    let (n, q) = (name.to_lowercase(), query.to_lowercase());
    if q.is_empty() {
        return Some(1000);
    }
    if n.starts_with(&q) {
        return Some(n.len() as i32 - q.len() as i32);
    }
    if let Some(at) = n.find(&q) {
        return Some(100 + at as i32);
    }
    let mut gaps = 0;
    let mut chars = n.chars();
    for qc in q.chars() {
        let mut skipped = 0;
        loop {
            match chars.next() {
                Some(c) if c == qc => break,
                Some(_) => skipped += 1,
                None => return None,
            }
        }
        gaps += skipped;
    }
    Some(300 + gaps)
}

/// Text editing, completion, history and search while open.
fn edit(world: &mut World) {
    let events: Vec<KeyboardInput> = world
        .resource_mut::<Messages<KeyboardInput>>()
        .drain()
        .filter(|e| e.state == ButtonState::Pressed)
        .collect();
    let wheel: Vec<MouseWheel> = world.resource_mut::<Messages<MouseWheel>>().drain().collect();
    let keys = world.resource::<ButtonInput<KeyCode>>().clone();
    let ctrl = keys.pressed(KeyCode::ControlLeft) || keys.pressed(KeyCode::ControlRight);
    let shift = keys.pressed(KeyCode::ShiftLeft) || keys.pressed(KeyCode::ShiftRight);
    for w in wheel {
        let lines = match w.unit {
            MouseScrollUnit::Line => w.y * 3.0,
            MouseScrollUnit::Pixel => w.y / 16.0,
        };
        let mut ui = world.resource_mut::<ConsoleUi>();
        ui.scroll = (ui.scroll as f32 + lines).max(0.0) as usize;
    }
    for e in events {
        if e.key_code == KeyCode::Backquote {
            continue;
        }
        let mut ui = world.resource_mut::<ConsoleUi>();
        // Reverse search mode.
        if let Some((query, back)) = ui.search.clone() {
            match (&e.logical_key, e.key_code) {
                (_, KeyCode::KeyR) if ctrl => ui.search = Some((query, back + 1)),
                (_, KeyCode::Escape) => ui.search = None,
                (_, KeyCode::Enter) | (_, KeyCode::ArrowLeft) | (_, KeyCode::ArrowRight) | (_, KeyCode::Tab) => {
                    let found = search(&ui.history, &query, back);
                    ui.search = None;
                    if let Some(found) = found {
                        ui.cursor = found.chars().count();
                        ui.input = found;
                    }
                    if e.key_code == KeyCode::Enter {
                        submit(world);
                    }
                }
                (_, KeyCode::Backspace) => {
                    let mut q = query;
                    q.pop();
                    ui.search = Some((q, 0));
                }
                (Key::Character(s), _) if !ctrl => ui.search = Some((format!("{query}{s}"), 0)),
                (Key::Space, _) => ui.search = Some((format!("{query} "), 0)),
                _ => {}
            }
            continue;
        }
        let chars: Vec<char> = ui.input.chars().collect();
        let cur = ui.cursor.min(chars.len());
        let mut reset_cycle = true;
        match (&e.logical_key, e.key_code) {
            (_, KeyCode::Enter) | (_, KeyCode::NumpadEnter) => {
                submit(world);
                continue;
            }
            (_, KeyCode::KeyR) if ctrl => ui.search = Some((String::new(), 0)),
            (_, KeyCode::KeyL) if ctrl => {
                world.resource_mut::<Console>().output.clear();
                continue;
            }
            (_, KeyCode::KeyU) if ctrl => {
                ui.input = chars[cur..].iter().collect();
                ui.cursor = 0;
            }
            (_, KeyCode::KeyW) if ctrl => {
                let mut start = cur;
                while start > 0 && chars[start - 1] == ' ' {
                    start -= 1;
                }
                while start > 0 && chars[start - 1] != ' ' {
                    start -= 1;
                }
                ui.input = chars[..start].iter().chain(&chars[cur..]).collect();
                ui.cursor = start;
            }
            (_, KeyCode::KeyA) if ctrl => ui.cursor = 0,
            (_, KeyCode::KeyE) if ctrl => ui.cursor = chars.len(),
            (_, KeyCode::Backspace) if cur > 0 => {
                ui.input = chars[..cur - 1].iter().chain(&chars[cur..]).collect();
                ui.cursor = cur - 1;
            }
            (_, KeyCode::Delete) if cur < chars.len() => {
                ui.input = chars[..cur].iter().chain(&chars[cur + 1..]).collect();
            }
            (_, KeyCode::ArrowLeft) if ctrl => {
                let mut c = cur;
                while c > 0 && chars[c - 1] == ' ' {
                    c -= 1;
                }
                while c > 0 && chars[c - 1] != ' ' {
                    c -= 1;
                }
                ui.cursor = c;
            }
            (_, KeyCode::ArrowRight) if ctrl => {
                let mut c = cur;
                while c < chars.len() && chars[c] != ' ' {
                    c += 1;
                }
                while c < chars.len() && chars[c] == ' ' {
                    c += 1;
                }
                ui.cursor = c;
            }
            (_, KeyCode::ArrowLeft) => ui.cursor = cur.saturating_sub(1),
            (_, KeyCode::ArrowRight) => ui.cursor = (cur + 1).min(chars.len()),
            (_, KeyCode::Home) => ui.cursor = 0,
            (_, KeyCode::End) => ui.cursor = chars.len(),
            (_, KeyCode::PageUp) => ui.scroll += VISIBLE_LINES / 2,
            (_, KeyCode::PageDown) => ui.scroll = ui.scroll.saturating_sub(VISIBLE_LINES / 2),
            (_, KeyCode::ArrowUp) | (_, KeyCode::ArrowDown) => {
                if ui.history.is_empty() {
                    continue;
                }
                let n = ui.history.len();
                let next = match (ui.browsing, e.key_code == KeyCode::ArrowUp) {
                    (None, true) => Some(n - 1),
                    (None, false) => None,
                    (Some(i), true) => Some(i.saturating_sub(1)),
                    (Some(i), false) if i + 1 < n => Some(i + 1),
                    (Some(_), false) => None,
                };
                ui.browsing = next;
                ui.input = next.map(|i| ui.history[i].clone()).unwrap_or_default();
                ui.cursor = ui.input.chars().count();
            }
            (_, KeyCode::Tab) => {
                reset_cycle = false;
                let input = ui.input.clone();
                let step = if shift { -1 } else { 1 };
                if let Some((list, i, base)) = ui.cycle.clone()
                    && !list.is_empty()
                {
                    let next = (i as i64 + step).rem_euclid(list.len() as i64) as usize;
                    let (head, _) = split_last(&base);
                    ui.input = format!("{head}{}", list[next]);
                    ui.cursor = ui.input.chars().count();
                    ui.cycle = Some((list, next, base));
                } else {
                    let (head, list) = candidates(world, &input);
                    let mut ui = world.resource_mut::<ConsoleUi>();
                    if let Some(first) = list.first() {
                        // One match completes and adds a space.
                        let single = list.len() == 1;
                        ui.input = format!("{head}{}{}", first.0, if single { " " } else { "" });
                        ui.cursor = ui.input.chars().count();
                        ui.cycle = (!single).then(|| (list.into_iter().map(|x| x.0).collect(), 0, input));
                    }
                }
            }
            (Key::Character(s), _) if !ctrl => {
                let mut c = chars.clone();
                let ins: Vec<char> = s.chars().collect();
                c.splice(cur..cur, ins.iter().copied());
                ui.input = c.into_iter().collect();
                ui.cursor = cur + ins.len();
            }
            (Key::Space, _) => {
                let mut c = chars.clone();
                c.insert(cur, ' ');
                ui.input = c.into_iter().collect();
                ui.cursor = cur + 1;
            }
            _ => continue,
        }
        if reset_cycle {
            world.resource_mut::<ConsoleUi>().cycle = None;
        }
    }
}

/// Split off the token being completed.
fn split_last(input: &str) -> (String, String) {
    match input.rfind(' ') {
        Some(i) => (input[..=i].to_string(), input[i + 1..].to_string()),
        None => (String::new(), input.to_string()),
    }
}

/// The `back`-th most recent history entry containing `query`.
fn search(history: &[String], query: &str, back: usize) -> Option<String> {
    history.iter().rev().filter(|h| h.contains(query)).nth(back).cloned()
}

fn submit(world: &mut World) {
    let mut ui = world.resource_mut::<ConsoleUi>();
    let line = std::mem::take(&mut ui.input);
    ui.cursor = 0;
    ui.browsing = None;
    ui.cycle = None;
    ui.scroll = 0;
    if !line.trim().is_empty() && ui.history.last() != Some(&line) {
        ui.history.push(line.clone());
        if ui.history.len() > HISTORY_MAX {
            ui.history.remove(0);
        }
        save_history(&ui.history);
    }
    let mut console = world.resource_mut::<Console>();
    console.print(Level::Input, format!("] {line}"));
    console.submit(line);
}

fn history_path() -> Option<std::path::PathBuf> {
    cfg_dir().map(|d| d.join("history.txt"))
}

fn save_history(history: &[String]) {
    if let Some(p) = history_path() {
        let _ = std::fs::create_dir_all(p.parent().unwrap());
        let _ = std::fs::write(p, history.join("\n"));
    }
}

fn load_history(mut ui: ResMut<ConsoleUi>) {
    if let Some(text) = history_path().and_then(|p| std::fs::read_to_string(p).ok()) {
        ui.history = text.lines().filter(|l| !l.is_empty()).map(String::from).collect();
    }
}

/// config.cfg (binds, aliases, changed cvars) then autoexec.cfg, then the
/// command line's `+commands`.
fn autoexec(
    mut console: ResMut<Console>,
    mut ui: ResMut<ConsoleUi>,
    args: Res<super::ClientArgs>,
    mut root: Query<&mut Visibility, With<ConsoleRoot>>,
) {
    console.submit("execifexists config.cfg; execifexists autoexec.cfg");
    for line in &args.0.console {
        console.submit(line.clone());
    }
    if args.0.console_open {
        ui.open = true;
        for mut v in &mut root {
            *v = Visibility::Visible;
        }
    }
}

/// Save binds and changed cvars when the game closes.
fn save_on_exit(mut exits: MessageReader<AppExit>, args: Res<super::ClientArgs>, mut commands: Commands) {
    // Automated runs (screenshots, view captures) leave the config alone.
    let automated = args.0.screenshot.is_some() || args.0.views.is_some();
    if exits.read().next().is_some() && !automated {
        commands.queue(|w: &mut World| {
            if w.resource::<Console>().dirty {
                crate::console::execute(w, &["host_writeconfig".to_string()], 0);
            }
        });
    }
}

fn draw_console(
    console: Res<Console>,
    mut ui: ResMut<ConsoleUi>,
    output: Single<Entity, With<ConsoleOutput>>,
    mut input: Single<&mut Text, (With<ConsoleInput>, Without<ConsoleOutput>)>,
    mut suggest: Single<
        (&mut Text, &mut Visibility),
        (With<ConsoleSuggest>, Without<ConsoleInput>, Without<ConsoleOutput>),
    >,
    mut commands: Commands,
    mut last: Local<(usize, usize, String, bool, f64)>,
) {
    if !ui.open {
        *suggest.1 = Visibility::Hidden;
        return;
    }
    // Output, filtered and scrolled, coloured by severity (rebuilt on change).
    let first_time = console.output.first().map_or(0.0, |l| l.time);
    let key = (
        console.output.len(),
        ui.scroll,
        ui.filter.clone(),
        ui.timestamps,
        first_time,
    );
    if *last != key {
        *last = key;
        let lines: Vec<_> = console
            .output
            .iter()
            .filter(|l| ui.filter.is_empty() || l.text.to_lowercase().contains(&ui.filter.to_lowercase()))
            .collect();
        let max_scroll = lines.len().saturating_sub(VISIBLE_LINES);
        ui.scroll = ui.scroll.min(max_scroll);
        let end = lines.len() - ui.scroll;
        let start = end.saturating_sub(VISIBLE_LINES);
        commands.entity(*output).despawn_related::<Children>();
        commands.entity(*output).with_children(|c| {
            for l in &lines[start..end] {
                let color = match l.level {
                    Level::Input => Color::srgb(0.6, 0.8, 1.0),
                    Level::Info => Color::srgb(0.85, 0.85, 0.85),
                    Level::Warn => Color::srgb(1.0, 0.8, 0.3),
                    Level::Error => Color::srgb(1.0, 0.4, 0.35),
                };
                let stamp = if ui.timestamps {
                    format!("[{:7.2}] ", l.time)
                } else {
                    String::new()
                };
                c.spawn((TextSpan::new(format!("{stamp}{}\n", l.text)), TextColor(color)));
            }
            if ui.scroll > 0 {
                c.spawn((
                    TextSpan::new(format!("-- {} more below (PageDown) --\n", ui.scroll)),
                    TextColor(Color::srgb(0.5, 0.5, 0.5)),
                ));
            }
        });
    }
    // Input line with a cursor, or the reverse search.
    input.0 = match &ui.search {
        Some((q, back)) => {
            let found = search(&ui.history, q, *back).unwrap_or_default();
            format!("(reverse-i-search)`{q}': {found}")
        }
        None => {
            let chars: Vec<char> = ui.input.chars().collect();
            let cur = ui.cursor.min(chars.len());
            let before: String = chars[..cur].iter().collect();
            let after: String = chars[cur..].iter().collect();
            format!("] {before}|{after}")
        }
    };
}

/// Suggestions under the console as you type (needs the world to read
/// cvar values).
fn overlay_text(world: &mut World) {
    let (open, input, searching) = {
        let ui = world.resource::<ConsoleUi>();
        (ui.open, ui.input.clone(), ui.search.is_some())
    };
    let mut text = String::new();
    if open && !searching && !input.trim().is_empty() {
        let (_, list) = candidates(world, &input);
        let console_cvars: Vec<_> = list
            .iter()
            .take(SUGGESTIONS)
            .map(|(n, h)| {
                let value = world.resource::<Console>().cvar(n).cloned();
                (n.clone(), h.clone(), value)
            })
            .collect();
        for (n, h, cvar) in console_cvars {
            let value = cvar.and_then(|v| {
                let now = (v.get)(world)?;
                Some(if now != v.default {
                    format!(" = {now} *")
                } else {
                    format!(" = {now}")
                })
            });
            let help: String = h.chars().take(70).collect();
            text += &format!("{n}{}   {help}\n", value.unwrap_or_default());
        }
        if list.len() > SUGGESTIONS {
            text += &format!("... {} more (Tab cycles)", list.len() - SUGGESTIONS);
        }
    }
    let mut q = world.query_filtered::<(&mut Text, &mut Visibility), With<ConsoleSuggest>>();
    if let Ok((mut t, mut v)) = q.single_mut(world) {
        *v = if text.is_empty() {
            Visibility::Hidden
        } else {
            Visibility::Visible
        };
        t.0 = text.trim_end().to_string();
    }
    // Watches: each watched cvar's value.
    let watches = world.resource::<Console>().watches.clone();
    let mut lines = Vec::new();
    for name in watches {
        let cvar = world.resource::<Console>().cvar(&name).cloned();
        let value = cvar.and_then(|v| (v.get)(world)).unwrap_or_else(|| "?".into());
        lines.push(format!("{name}: {value}"));
    }
    world.resource_mut::<Overlays>().watch_text = lines.join("\n");
}

// ------------------------------------------------------------- overlays

#[derive(Resource)]
pub struct Overlays {
    pub showpos: u8,
    pub showfps: u8,
    pub snd_show: u8,
    /// `developer`: 1 shows console output top left for a few seconds.
    pub developer: u8,
    /// Notify lines and seconds left, and console lines already taken.
    notify: VecDeque<(String, f32)>,
    notify_seen: u64,
    watch_text: String,
    frames: VecDeque<f32>,
}

impl Default for Overlays {
    fn default() -> Self {
        Self {
            showpos: 0,
            showfps: 0,
            snd_show: 0,
            developer: 0,
            notify: VecDeque::new(),
            notify_seen: 0,
            watch_text: String::new(),
            frames: VecDeque::new(),
        }
    }
}

/// Source's con_notifytime and notify line count.
const NOTIFY_SECONDS: f32 = 8.0;
const NOTIFY_LINES: usize = 8;

#[derive(Component)]
struct NotifyText;

/// `developer 1`: new console lines show top left for a few seconds.
fn draw_notify(
    time: Res<Time>,
    console: Res<Console>,
    ui: Res<ConsoleUi>,
    mut o: ResMut<Overlays>,
    mut text: Single<(&mut Text, &mut Visibility), With<NotifyText>>,
) {
    let new = console.printed.saturating_sub(o.notify_seen) as usize;
    o.notify_seen = console.printed;
    if o.developer > 0 {
        let start = console.output.len().saturating_sub(new);
        for line in &console.output[start..] {
            if line.level != crate::console::Level::Input {
                o.notify.push_back((line.text.clone(), NOTIFY_SECONDS));
            }
        }
    }
    let dt = time.delta_secs();
    o.notify.retain_mut(|(_, left)| {
        *left -= dt;
        *left > 0.0
    });
    while o.notify.len() > NOTIFY_LINES {
        o.notify.pop_front();
    }
    let (t, vis) = &mut *text;
    // Hidden while the console itself is open, or when off.
    let show = o.developer > 0 && !ui.open && !o.notify.is_empty();
    **vis = if show {
        Visibility::Inherited
    } else {
        Visibility::Hidden
    };
    if show {
        t.0 = o.notify.iter().map(|(l, _)| l.as_str()).collect::<Vec<_>>().join("\n");
    }
}

fn overlay_cvars(app: &mut App) {
    resource_cvar::<Overlays, u8>(
        app,
        "developer",
        "1: show console output at the top left for a few seconds.",
        |o| &mut o.developer,
    );
    resource_cvar::<Overlays, u8>(
        app,
        "cl_showpos",
        "1: show map, position, angles and velocity (CS:S units on CS:S maps).",
        |o| &mut o.showpos,
    );
    resource_cvar::<Overlays, u8>(
        app,
        "cl_showfps",
        "1: frame rate; 2: also frame time min/avg/max over the last second.",
        |o| &mut o.showfps,
    );
    resource_cvar::<Overlays, u8>(
        app,
        "snd_show",
        "1: mark where sounds play, with their names, for a few seconds.",
        |o| &mut o.snd_show,
    );
    app.console_cvar(
        "con_timestamps",
        "1: time stamps on console lines.",
        "0",
        |w| {
            w.get_resource::<ConsoleUi>()
                .map(|u| if u.timestamps { "1" } else { "0" }.to_string())
        },
        |w, v| {
            w.resource_mut::<ConsoleUi>().timestamps = v.trim() != "0";
            Ok(())
        },
    );
    app.world_mut().resource_mut::<Console>().archive("con_timestamps");
}

fn draw_overlays(
    time: Res<Time>,
    mut o: ResMut<Overlays>,
    mut text: Single<&mut Text, With<OverlayText>>,
    player: Option<Single<(&Transform, &Velocity, &crate::core::Intent), With<LocalPlayer>>>,
    map: Option<Res<crate::map::ActiveMapLook>>,
    console: Res<Console>,
) {
    let _ = map;
    let dt = time.delta_secs();
    o.frames.push_back(dt);
    while o.frames.iter().sum::<f32>() > 1.0 && o.frames.len() > 1 {
        o.frames.pop_front();
    }
    let mut lines = Vec::new();
    if o.showfps > 0 {
        let n = o.frames.len().max(1) as f32;
        let avg = o.frames.iter().sum::<f32>() / n;
        lines.push(format!("{:.0} fps", 1.0 / avg.max(1e-6)));
        if o.showfps > 1 {
            let min = o.frames.iter().cloned().fold(f32::MAX, f32::min);
            let max = o.frames.iter().cloned().fold(0.0f32, f32::max);
            lines.push(format!(
                "frame {:.1} / {:.1} / {:.1} ms",
                min * 1e3,
                avg * 1e3,
                max * 1e3
            ));
        }
    }
    if o.showpos > 0
        && let Some(p) = &player
    {
        let (t, v, i) = **p;
        let units = |x: Vec3| Vec3::new(x.x, -x.z, x.y) / 0.0254;
        let pos = units(t.translation);
        let vel = units(v.0);
        lines.push(format!("pos: {:.2} {:.2} {:.2}", pos.x, pos.y, pos.z));
        lines.push(format!(
            "ang: {:.2} {:.2} 0.00",
            -i.pitch.to_degrees(),
            (i.yaw.to_degrees() + 90.0).rem_euclid(360.0)
        ));
        lines.push(format!("vel: {:.2} (xy {:.2})", vel.length(), vel.truncate().length()));
    }
    if !o.watch_text.is_empty() {
        lines.push(o.watch_text.clone());
    }
    let _ = console;
    text.0 = lines.join("\n");
}

/// Sounds marked on screen for `snd_show`.
#[derive(Resource, Default)]
struct SoundMarks(Vec<(String, Vec3, f32, Entity)>);

#[derive(Component)]
struct SoundMark;

const MARK_SECONDS: f32 = 3.0;

fn record_sounds(
    mut sounds: MessageReader<PlaySound>,
    o: Res<Overlays>,
    mut marks: ResMut<SoundMarks>,
    mut commands: Commands,
) {
    for s in sounds.read() {
        if o.snd_show == 0 {
            continue;
        }
        let Some(at) = s.at else { continue };
        let e = commands
            .spawn((
                SoundMark,
                Text::new(s.entry.clone()),
                TextFont {
                    font_size: FontSize::Px(13.0),
                    ..default()
                },
                TextColor(Color::srgb(0.5, 1.0, 0.6)),
                Node {
                    position_type: PositionType::Absolute,
                    ..default()
                },
            ))
            .id();
        marks.0.push((s.entry.clone(), at, MARK_SECONDS, e));
    }
}

fn draw_sound_marks(
    time: Res<Time>,
    mut marks: ResMut<SoundMarks>,
    camera: Query<(&Camera, &GlobalTransform), With<FirstPersonCamera>>,
    mut nodes: Query<(&mut Node, &mut TextColor, &mut Visibility), With<SoundMark>>,
    mut commands: Commands,
) {
    let cam = camera.iter().next();
    marks.0.retain_mut(|(_, at, left, e)| {
        *left -= time.delta_secs();
        if *left <= 0.0 {
            commands.entity(*e).despawn();
            return false;
        }
        if let (Some((c, t)), Ok((mut node, mut color, mut vis))) = (cam, nodes.get_mut(*e)) {
            match c.world_to_viewport(t, *at) {
                Ok(p) => {
                    node.left = px(p.x);
                    node.top = px(p.y);
                    *vis = Visibility::Visible;
                    color.0 = color.0.with_alpha(*left / MARK_SECONDS);
                }
                Err(_) => *vis = Visibility::Hidden,
            }
        }
        true
    });
}

// ------------------------------------------------------------- log

static LOG: OnceLock<Arc<Mutex<Vec<(Level, String)>>>> = OnceLock::new();

/// A tracing layer that copies warnings and errors into the console.
pub fn log_layer(_: &mut App) -> Option<BoxedLayer> {
    struct ToConsole(Arc<Mutex<Vec<(Level, String)>>>);
    impl<S: bevy::log::tracing::Subscriber> Layer<S> for ToConsole {
        fn on_event(
            &self,
            event: &bevy::log::tracing::Event<'_>,
            _: bevy::log::tracing_subscriber::layer::Context<'_, S>,
        ) {
            let level = match *event.metadata().level() {
                bevy::log::tracing::Level::ERROR => Level::Error,
                bevy::log::tracing::Level::WARN => Level::Warn,
                bevy::log::tracing::Level::INFO if event.metadata().target().starts_with("mashup") => Level::Info,
                _ => return,
            };
            struct Msg(String);
            impl bevy::log::tracing::field::Visit for Msg {
                fn record_debug(&mut self, f: &bevy::log::tracing::field::Field, v: &dyn std::fmt::Debug) {
                    if f.name() == "message" {
                        self.0 = format!("{v:?}");
                    }
                }
            }
            let mut m = Msg(String::new());
            event.record(&mut m);
            if let Ok(mut q) = self.0.lock() {
                q.push((level, m.0));
            }
        }
    }
    let queue = LOG.get_or_init(Default::default).clone();
    Some(Box::new(ToConsole(queue)))
}

fn drain_log(mut console: ResMut<Console>) {
    let Some(q) = LOG.get() else { return };
    let lines: Vec<_> = q.lock().map(|mut q| std::mem::take(&mut *q)).unwrap_or_default();
    for (level, text) in lines {
        console.print(level, text);
    }
}

// ------------------------------------------------------------- binds

/// Actions held through binds (`+jump`, ...), read with the keys.
#[derive(Resource, Default, Debug)]
pub struct HeldActions {
    pub jump: bool,
    pub duck: bool,
    pub speed: bool,
    pub forward: bool,
    pub back: bool,
    pub moveleft: bool,
    pub moveright: bool,
    pub attack: bool,
    pub attack2: bool,
    pub reload: bool,
    /// The use key (`+use`): doors, buttons.
    pub use_key: bool,
    /// The scoreboard (`+showscores`, Tab in CS:S).
    pub showscores: bool,
}

const ACTIONS: &[&str] = &[
    "jump",
    "duck",
    "speed",
    "forward",
    "back",
    "moveleft",
    "moveright",
    "attack",
    "attack2",
    "reload",
    "use",
    "showscores",
];

/// Source key names.
const KEY_NAMES: &[(KeyCode, &str)] = &[
    (KeyCode::KeyA, "a"),
    (KeyCode::KeyB, "b"),
    (KeyCode::KeyC, "c"),
    (KeyCode::KeyD, "d"),
    (KeyCode::KeyE, "e"),
    (KeyCode::KeyF, "f"),
    (KeyCode::KeyG, "g"),
    (KeyCode::KeyH, "h"),
    (KeyCode::KeyI, "i"),
    (KeyCode::KeyJ, "j"),
    (KeyCode::KeyK, "k"),
    (KeyCode::KeyL, "l"),
    (KeyCode::KeyM, "m"),
    (KeyCode::KeyN, "n"),
    (KeyCode::KeyO, "o"),
    (KeyCode::KeyP, "p"),
    (KeyCode::KeyQ, "q"),
    (KeyCode::KeyR, "r"),
    (KeyCode::KeyS, "s"),
    (KeyCode::KeyT, "t"),
    (KeyCode::KeyU, "u"),
    (KeyCode::KeyV, "v"),
    (KeyCode::KeyW, "w"),
    (KeyCode::KeyX, "x"),
    (KeyCode::KeyY, "y"),
    (KeyCode::KeyZ, "z"),
    (KeyCode::Digit0, "0"),
    (KeyCode::Digit1, "1"),
    (KeyCode::Digit2, "2"),
    (KeyCode::Digit3, "3"),
    (KeyCode::Digit4, "4"),
    (KeyCode::Digit5, "5"),
    (KeyCode::Digit6, "6"),
    (KeyCode::Digit7, "7"),
    (KeyCode::Digit8, "8"),
    (KeyCode::Digit9, "9"),
    (KeyCode::Space, "space"),
    (KeyCode::Enter, "enter"),
    (KeyCode::Tab, "tab"),
    (KeyCode::ShiftLeft, "shift"),
    (KeyCode::ControlLeft, "ctrl"),
    (KeyCode::AltLeft, "alt"),
    (KeyCode::ArrowUp, "uparrow"),
    (KeyCode::ArrowDown, "downarrow"),
    (KeyCode::ArrowLeft, "leftarrow"),
    (KeyCode::ArrowRight, "rightarrow"),
    (KeyCode::F1, "f1"),
    (KeyCode::F2, "f2"),
    (KeyCode::F3, "f3"),
    (KeyCode::F4, "f4"),
    (KeyCode::F5, "f5"),
    (KeyCode::F6, "f6"),
    (KeyCode::F7, "f7"),
    (KeyCode::F8, "f8"),
    (KeyCode::F9, "f9"),
    (KeyCode::F10, "f10"),
    (KeyCode::F11, "f11"),
    (KeyCode::F12, "f12"),
    (KeyCode::Minus, "-"),
    (KeyCode::Equal, "="),
    (KeyCode::BracketLeft, "["),
    (KeyCode::BracketRight, "]"),
    (KeyCode::Backslash, "\\"),
    (KeyCode::Semicolon, "semicolon"),
    (KeyCode::Quote, "'"),
    (KeyCode::Comma, ","),
    (KeyCode::Period, "."),
    (KeyCode::Slash, "/"),
];

/// Run bound commands: on press, and the `-` half of `+` actions on
/// release; wheel notches run once.
fn run_binds(
    keys: Res<ButtonInput<KeyCode>>,
    mouse: Res<ButtonInput<MouseButton>>,
    mut wheel: MessageReader<MouseWheel>,
    mut console: ResMut<Console>,
) {
    if console.binds.is_empty() {
        wheel.clear();
        return;
    }
    let mut events: Vec<(String, bool)> = Vec::new();
    for (code, name) in KEY_NAMES {
        if keys.just_pressed(*code) {
            events.push((name.to_string(), true));
        }
        if keys.just_released(*code) {
            events.push((name.to_string(), false));
        }
    }
    for (b, name) in [
        (MouseButton::Left, "mouse1"),
        (MouseButton::Right, "mouse2"),
        (MouseButton::Middle, "mouse3"),
        (MouseButton::Back, "mouse4"),
        (MouseButton::Forward, "mouse5"),
    ] {
        if mouse.just_pressed(b) {
            events.push((name.into(), true));
        }
        if mouse.just_released(b) {
            events.push((name.into(), false));
        }
    }
    for w in wheel.read() {
        let name = if w.y > 0.0 { "mwheelup" } else { "mwheeldown" };
        events.push((name.into(), true));
        events.push((name.into(), false));
    }
    for (key, pressed) in events {
        let Some(cmd) = console.binds.get(&key).cloned() else {
            continue;
        };
        if pressed {
            console.submit(cmd);
        } else {
            // Releasing runs the "-" half of each "+" command.
            let releases: Vec<String> = parse(&cmd)
                .into_iter()
                .filter(|w| w[0].starts_with('+'))
                .map(|w| format!("-{}", &w[0][1..]))
                .collect();
            if !releases.is_empty() {
                console.submit(releases.join("; "));
            }
        }
    }
}

// ------------------------------------------------------------- commands

/// Maps in the CS:S install (for `map` completion), found once.
/// Map names found by `map_names`, until an import changes them.
static MAP_NAMES: Mutex<Option<Vec<String>>> = Mutex::new(None);

/// Every map the game can load: the install's, its downloads and
/// mashup's cache (cached; `import` refreshes it).
fn map_names() -> Vec<String> {
    let mut cached = MAP_NAMES.lock().unwrap_or_else(|e| e.into_inner());
    cached
        .get_or_insert_with(|| {
            let Some(path) = crate::mount::config::LocalConfig::load()
                .ok()
                .and_then(|c| c.game_path(crate::games::cs_source::GAME))
            else {
                return Vec::new();
            };
            let Ok(mount) = crate::games::cs_source::mount::open(&path) else {
                return Vec::new();
            };
            let mut names: Vec<String> = mount
                .entries()
                .into_iter()
                .filter_map(|(e, _)| {
                    e.path
                        .strip_prefix("maps/")
                        .and_then(|p| p.strip_suffix(".bsp"))
                        .filter(|p| !p.contains('/'))
                        .map(String::from)
                })
                .collect();
            names.sort();
            names.dedup();
            names
        })
        .clone()
}

/// Where a map comes from: the game's own content, its download folder, or
/// mashup's cache.
fn map_source(name: &str) -> &'static str {
    let file = format!("maps/{name}.bsp");
    let in_dir = |dir: Option<std::path::PathBuf>| dir.is_some_and(|d| d.join(&file).is_file());
    let install = crate::mount::config::LocalConfig::load()
        .ok()
        .and_then(|c| c.game_path(crate::games::cs_source::GAME));
    if in_dir(crate::mount::config::content_dir(crate::games::cs_source::GAME)) {
        "imported"
    } else if in_dir(install.map(|p| p.join("cstrike/download"))) {
        "downloaded"
    } else {
        "game"
    }
}

/// A map loading in the background for the `map` command.
#[derive(Resource)]
struct MapLoad {
    id: String,
    task: bevy::tasks::Task<Result<crate::map::MapData, String>>,
}

/// Swap in a map once its background load finishes.
fn finish_map_load(w: &mut World) {
    let Some(mut load) = w.get_resource_mut::<MapLoad>() else {
        return;
    };
    let Some(result) = bevy::tasks::block_on(bevy::tasks::poll_once(&mut load.task)) else {
        return;
    };
    let id = load.id.clone();
    w.remove_resource::<MapLoad>();
    let data = match result {
        Ok(d) => d,
        Err(e) => {
            w.resource_mut::<Console>()
                .print(crate::console::Level::Error, format!("map {id}: {e}"));
            return;
        }
    };
    let summary = format!(
        "loaded {id}: {} triangles, {} spawns",
        data.triangle_count(),
        data.spawns.len()
    );
    crate::greybox::unload(w);
    crate::map::change_map(w, data, crate::map::MapDebugView::Normal);
    w.insert_resource(Time::<Fixed>::from_seconds(crate::games::cs_source::TICK_INTERVAL));
    // Follow the map's presentation (Source LDR: no tonemapping).
    let tonemapping = w.resource::<crate::map::ActiveMapLook>().0.tonemapping;
    let cams: Vec<Entity> = w
        .query_filtered::<Entity, With<super::FirstPersonCamera>>()
        .iter(w)
        .collect();
    for c in cams {
        let t = if tonemapping {
            bevy::core_pipeline::tonemapping::Tonemapping::default()
        } else {
            bevy::core_pipeline::tonemapping::Tonemapping::None
        };
        w.entity_mut(c).insert(t);
    }
    crate::rules::respawn_everyone(w);
    w.resource_mut::<Console>().info(summary);
}

fn local_player(w: &mut World) -> Result<Entity, String> {
    let mut q = w.query_filtered::<Entity, With<LocalPlayer>>();
    q.single(w).map_err(|_| "no local player".to_string())
}

fn client_commands(app: &mut App) {
    for a in ACTIONS {
        let a = *a;
        for (sign, on) in [("+", true), ("-", false)] {
            app.console_command(
                &format!("{sign}{a}"),
                &format!("{} {a} (for binds).", if on { "Start" } else { "Stop" }),
                move |w, _| {
                    let mut h = w.resource_mut::<HeldActions>();
                    match a {
                        "jump" => h.jump = on,
                        "duck" => h.duck = on,
                        "speed" => h.speed = on,
                        "forward" => h.forward = on,
                        "back" => h.back = on,
                        "moveleft" => h.moveleft = on,
                        "moveright" => h.moveright = on,
                        "attack2" => h.attack2 = on,
                        "reload" => h.reload = on,
                        "use" => h.use_key = on,
                        "showscores" => h.showscores = on,
                        _ => h.attack = on,
                    }
                    Ok(None)
                },
            );
        }
    }
    app.console_command(
        "noclip",
        "Fly through walls (toggles back to the movement you had).",
        |w, _| {
            let p = local_player(w)?;
            let current = w.get::<MovementSlot>(p).map(|s| s.0).ok_or("no movement")?;
            let back = w.get_resource::<NoclipBack>().map(|b| b.0);
            let next = if current == crate::movement::noclip::ID {
                back.unwrap_or(crate::movement::placeholder::ID)
            } else {
                w.insert_resource(NoclipBack(current));
                crate::movement::noclip::ID
            };
            set_movement(p, next).apply(w);
            Ok(Some(format!(
                "noclip {}",
                if next == crate::movement::noclip::ID {
                    "ON"
                } else {
                    "OFF"
                }
            )))
        },
    )
    .console_command(
        "getpos",
        "Your position and view, as a setpos/setang line (CS:S units).",
        |w, _| {
            let p = local_player(w)?;
            let t = *w.get::<Transform>(p).ok_or("no transform")?;
            let i = w.get::<crate::core::Intent>(p).ok_or("no intent")?;
            let s = Vec3::new(t.translation.x, -t.translation.z, t.translation.y) / 0.0254;
            Ok(Some(format!(
                "setpos {:.2} {:.2} {:.2};setang {:.2} {:.2} 0.00",
                s.x,
                s.y,
                s.z,
                -i.pitch.to_degrees(),
                (i.yaw.to_degrees() + 90.0).rem_euclid(360.0)
            )))
        },
    )
    .console_command(
        "setpos",
        "setpos <x> <y> <z>: move there (CS:S units, the view's origin as getpos prints it).",
        |w, a| {
            let [x, y, z] = a else {
                return Err("setpos <x> <y> <z>".into());
            };
            let n = |s: &String| s.parse::<f32>().map_err(|_| format!("bad number \"{s}\""));
            let v = Vec3::new(n(x)?, n(z)?, -n(y)?) * 0.0254;
            let p = local_player(w)?;
            w.get_mut::<Transform>(p).ok_or("no transform")?.translation = v;
            w.get_mut::<Velocity>(p).ok_or("no velocity")?.0 = Vec3::ZERO;
            Ok(None)
        },
    )
    .console_command(
        "setang",
        "setang <pitch> <yaw>: look there (degrees, CS:S convention).",
        |w, a| {
            let [pitch, yaw, ..] = a else {
                return Err("setang <pitch> <yaw>".into());
            };
            let n = |s: &String| s.parse::<f32>().map_err(|_| format!("bad number \"{s}\""));
            let (pitch, yaw) = (n(pitch)?, n(yaw)?);
            let p = local_player(w)?;
            let mut i = w.get_mut::<crate::core::Intent>(p).ok_or("no intent")?;
            i.pitch = -pitch.to_radians();
            i.yaw = (yaw - 90.0).to_radians();
            Ok(None)
        },
    )
    .console_command("god", "Toggle taking no damage.", |w, _| {
        let p = local_player(w)?;
        let on = w.get::<crate::core::God>(p).is_none();
        if on {
            w.entity_mut(p).insert(crate::core::God);
        } else {
            w.entity_mut(p).remove::<crate::core::God>();
        }
        Ok(Some(format!("godmode {}", if on { "ON" } else { "OFF" })))
    })
    .console_command("kill", "Respawn at a spawn point.", |w, _| {
        let p = local_player(w)?;
        let mut q = w.query_filtered::<&Transform, With<SpawnPoint>>();
        let spawn = q.iter(w).next().copied().ok_or("no spawn points")?;
        w.get_mut::<Transform>(p).ok_or("no transform")?.translation = spawn.translation + Vec3::Y * 1.0;
        w.get_mut::<Velocity>(p).ok_or("no velocity")?.0 = Vec3::ZERO;
        let _ = w.get::<MovementState>(p);
        Ok(None)
    })
    .console_command(
        "maps",
        "maps [filter]: list the maps that can be loaded (the game's, its downloads, imported).",
        |_, a| {
            let filter = a.first().map(|f| f.to_lowercase()).unwrap_or_default();
            let lines: Vec<String> = map_names()
                .into_iter()
                .filter(|m| m.contains(&filter))
                .map(|m| {
                    let from = map_source(&m);
                    format!(
                        "{m}{}",
                        if from == "game" {
                            String::new()
                        } else {
                            format!("  ({from})")
                        }
                    )
                })
                .collect();
            Ok(Some(if lines.is_empty() {
                "no maps match".into()
            } else {
                format!("{}\n{} maps", lines.join("\n"), lines.len())
            }))
        },
    )
    .console_command(
        "import",
        "import <file.bsp|file.bsp.bz2>: copy a map into mashup's content cache (never the game folder).",
        |_, a| {
            let path = a.join(" ");
            if path.is_empty() {
                return Err("import <file.bsp|file.bsp.bz2>".into());
            }
            let name = crate::games::cs_source::mount::import_map(std::path::Path::new(&path))?;
            *MAP_NAMES.lock().unwrap_or_else(|e| e.into_inner()) = None;
            Ok(Some(format!("imported {name}; load it with: map {name}")))
        },
    )
    .console_command(
        "map",
        "map <name>: load a CS:S map (Tab lists the install's maps).",
        |w, a| {
            let name = a.first().ok_or("map <name>")?;
            if !map_names().is_empty() && !map_names().iter().any(|m| m == name) {
                return Err(format!("no map \"{name}\" in the install"));
            }
            // Load in the background, then swap in place (finish_map_load):
            // everyone respawns at the new spawn points.
            let id = format!("cs_source:{name}");
            let task = bevy::tasks::AsyncComputeTaskPool::get().spawn({
                let id = id.clone();
                async move { crate::games::load_map(&id) }
            });
            w.insert_resource(MapLoad { id: id.clone(), task });
            Ok(Some(format!("loading {id}...")))
        },
    )
    .console_command("quit", "Quit (binds and changed cvars are saved).", |w, _| {
        w.write_message(AppExit::Success);
        Ok(None)
    })
    .console_command("exit", "Quit.", |w, _| {
        w.write_message(AppExit::Success);
        Ok(None)
    })
    .console_command(
        "con_filter",
        "con_filter [text]: show only lines containing text (none: all).",
        |w, a| {
            w.resource_mut::<ConsoleUi>().filter = a.join(" ");
            Ok(None)
        },
    )
    .console_command(
        "con_dump",
        "con_dump [file]: write the console output to the cfg folder (condump.txt).",
        |w, a| {
            let name = a.first().map_or("condump.txt", |s| s.as_str());
            let text: Vec<String> = w.resource::<Console>().output.iter().map(|l| l.text.clone()).collect();
            let path = cfg_dir().ok_or("no config folder")?.join(name);
            std::fs::create_dir_all(path.parent().unwrap()).map_err(|e| e.to_string())?;
            std::fs::write(&path, text.join("\n")).map_err(|e| e.to_string())?;
            Ok(Some(format!("wrote {}", path.display())))
        },
    )
    .console_command(
        "con_copy",
        "con_copy [lines]: copy the console output (or its last lines) to the clipboard.",
        |w, a| {
            let output = &w.resource::<Console>().output;
            let n = match a.first() {
                Some(n) => n.parse::<usize>().map_err(|_| format!("bad line count \"{n}\""))?,
                None => output.len(),
            };
            let text: Vec<String> = output.iter().skip(output.len().saturating_sub(n)).map(|l| l.text.clone()).collect();
            let count = text.len();
            arboard::Clipboard::new()
                .and_then(|mut c| c.set_text(text.join("\n")))
                .map_err(|e| format!("clipboard: {e}"))?;
            Ok(Some(format!("copied {count} lines")))
        },
    );
}

/// Remote protocol method `mashup/console`: params `{"line": "..."}` runs
/// the line now and returns the output lines it printed.
#[cfg(feature = "dev")]
pub fn remote_exec(
    bevy::ecs::system::In(params): bevy::ecs::system::In<Option<serde_json::Value>>,
    world: &mut World,
) -> bevy::remote::BrpResult {
    let line = params
        .as_ref()
        .and_then(|p| p.get("line"))
        .and_then(|l| l.as_str())
        .ok_or(bevy::remote::BrpError {
            code: bevy::remote::error_codes::INVALID_PARAMS,
            message: "expected {\"line\": \"command\"}".into(),
            data: None,
        })?
        .to_string();
    let start = world.resource::<Console>().output.len();
    {
        let mut c = world.resource_mut::<Console>();
        c.print(Level::Input, format!("] {line}"));
        c.submit(line);
    }
    crate::console::run_queue(world);
    let out: Vec<String> = world.resource::<Console>().output[start + 1..]
        .iter()
        .map(|l| l.text.clone())
        .collect();
    Ok(serde_json::json!({ "output": out }))
}

/// The movement to go back to when noclip turns off.
#[derive(Resource)]
struct NoclipBack(&'static str);

#[cfg(test)]
mod tests {

    #[test]
    fn closing_with_the_console_key_captures_the_mouse() {
        use bevy::window::{CursorGrabMode, CursorOptions};
        let mut app = App::new();
        app.init_resource::<ConsoleUi>()
            .init_resource::<ButtonInput<KeyCode>>()
            .add_systems(Update, toggle);
        app.world_mut().spawn((ConsoleRoot, Visibility::Hidden));
        let window = app.world_mut().spawn(CursorOptions::default()).id();
        let press = |app: &mut App, key: KeyCode| {
            let mut keys = app.world_mut().resource_mut::<ButtonInput<KeyCode>>();
            keys.clear();
            keys.press(key);
            app.update();
            app.world_mut().resource_mut::<ButtonInput<KeyCode>>().release(key);
        };
        let grabbed = |app: &App| app.world().get::<CursorOptions>(window).unwrap().grab_mode != CursorGrabMode::None;
        press(&mut app, KeyCode::Backquote);
        assert!(app.world().resource::<ConsoleUi>().open && !grabbed(&app));
        press(&mut app, KeyCode::Backquote);
        assert!(
            !app.world().resource::<ConsoleUi>().open && grabbed(&app),
            "tilde should recapture"
        );
        press(&mut app, KeyCode::Backquote);
        press(&mut app, KeyCode::Escape);
        assert!(
            !app.world().resource::<ConsoleUi>().open && !grabbed(&app),
            "escape leaves the mouse free"
        );
    }

    use super::*;
    use crate::console::ConsolePlugin;

    #[test]
    fn fuzzy_ranking() {
        // Prefix beats substring beats scattered letters.
        let mut names = vec!["sv_airaccelerate", "sv_accelerate", "cl_showpos", "sv_autobunnyhopping"];
        names.sort_by_key(|n| score(n, "sv_a").unwrap_or(i32::MAX));
        assert_eq!(names[0], "sv_accelerate");
        assert!(score("sv_enablebunnyhopping", "bunny").unwrap() < score("sv_enablebunnyhopping", "sebh").unwrap());
        assert!(score("sv_enablebunnyhopping", "sebh").is_some(), "letters in order");
        assert!(score("cl_showpos", "xyz").is_none());
    }

    #[test]
    fn completion_candidates() {
        let mut app = App::new();
        app.add_plugins(ConsolePlugin);
        app.init_resource::<HeldActions>();
        client_commands(&mut app);
        app.console_cvar(
            "sv_enablebunnyhopping",
            "bhop",
            "0",
            |_| Some("0".into()),
            |_, _| Ok(()),
        );
        let w = app.world_mut();
        let (head, list) = candidates(w, "sv_enab");
        assert_eq!(head, "");
        assert_eq!(list[0].0, "sv_enablebunnyhopping");
        // A boolean cvar offers 0 and 1.
        let (head, list) = candidates(w, "sv_enablebunnyhopping ");
        assert_eq!(head, "sv_enablebunnyhopping ");
        assert!(list.iter().any(|(v, _)| v == "1"));
        // bind: key names, then actions.
        let (_, keys) = candidates(w, "bind mwh");
        assert!(keys.iter().any(|(k, _)| k == "mwheeldown"));
        let (_, actions) = candidates(w, "bind space +ju");
        assert_eq!(actions[0].0, "+jump");
        // Commands registered by the client are names too.
        let (_, list) = candidates(w, "noc");
        assert_eq!(list[0].0, "noclip");
    }

    #[test]
    fn reverse_search_and_tokens() {
        let h = vec!["sv_gravity 800".to_string(), "noclip".into(), "sv_gravity 400".into()];
        assert_eq!(search(&h, "grav", 0).as_deref(), Some("sv_gravity 400"));
        assert_eq!(search(&h, "grav", 1).as_deref(), Some("sv_gravity 800"));
        assert_eq!(search(&h, "grav", 2), None);
        assert_eq!(
            split_last("bind space +j"),
            ("bind space ".to_string(), "+j".to_string())
        );
    }
}
