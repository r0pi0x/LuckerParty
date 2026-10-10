//! The spectator menu: duck while spectating (dead, or on the spectator
//! team) opens it, as CS:S's does (`spec_menu [0|1]` too); duck again, or
//! its Options > Close, shuts it. While it is open the mouse is free. It
//! is CS:S's bottom bar (`resource/ui/bottomspectator.res`, read from the
//! install at run time): three drop-down lists over the spectator bar,
//! drawn with the shared widgets (`client::widgets`), and the `<` `>`
//! buttons either side of the middle one:
//!
//! - Options (`settingscombo`): the entries of the install's
//!   `resource/spectatormenu.res` (Close; Settings: the overview's No
//!   Rotation, Show Names, Show Health, Show Tracks; Overview: No / Small /
//!   Large Map, Zoom In / Out; Auto Director; Show Scores), a submenu
//!   opening beside its entry, a toggle ticked while its cvar is 1;
//!   entries mashup can't do (no such command or cvar: the auto director)
//!   greyed, and the overview's without a map overview.
//! - The players who may be watched (`playercombo`), the watched one shown;
//!   `specprev` / `specnext` step through them.
//! - The camera (`viewcombo`): the install's `resource/spectatormodes.res`
//!   (First Person, Chase Camera, Free Look), the mode now shown.
//!
//! The lists open upward (the bar is at the bottom of the screen). Without
//! the install, the same controls at the file's places with our words.

use std::collections::{HashMap, HashSet};

use bevy::{prelude::*, window::CursorOptions};

use super::spectate::{SpecMode, SpecPhase, Spectator};
use super::vgui::VguiOpen;
use super::widgets::{self, ComboList, Look};
use crate::{
    console::Console,
    core::{Health, Team},
    map::hud::{CommandMenuItem, UiLayout},
};

pub struct SpectatorMenuPlugin;

impl Plugin for SpectatorMenuPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<SpectatorMenu>()
            .init_resource::<MenuLists>()
            .init_resource::<VguiOpen>()
            .init_resource::<CvarStates>()
            .add_systems(
                Update,
                (toggle, read_cvars, click, hover, keys, draw)
                    .chain()
                    .before(super::spectate::SpectateSet),
            );
        crate::console::ConsoleAppExt::console_command(
            app,
            "spec_menu",
            "spec_menu [0|1]: close or open the spectator menu (toggles without a number).",
            |w, a| {
                let open = match a.first().map(String::as_str) {
                    None => !w.resource::<SpectatorMenu>().0,
                    Some("0") => false,
                    Some("1") => true,
                    _ => return Err("spec_menu [0|1]".into()),
                };
                w.resource_mut::<SpectatorMenu>().0 = open;
                Ok(None)
            },
        );
        crate::console::ConsoleAppExt::console_command(
            app,
            "togglescores",
            "Show or hide the scoreboard (as holding +showscores does).",
            |w, _| {
                if let Some(mut h) = w.get_resource_mut::<super::console::HeldActions>() {
                    h.showscores = !h.showscores;
                }
                Ok(None)
            },
        );
        // `spectatormenu.res`'s spec_menuinput-free driving, for tests and
        // screenshots: open a list, pick an entry.
        crate::console::ConsoleAppExt::console_command(
            app,
            "spec_menuinput",
            "spec_menuinput <options|players|view> [entry [subentry]]: open one of the spectator menu's lists, or pick \
             its entry (0 first), in a submenu its subentry.",
            |w, a| {
                let list = match a.first().map(|s| s.to_lowercase()).as_deref() {
                    Some("options") => List::Options,
                    Some("players") => List::Players,
                    Some("view") => List::View,
                    _ => return Err("spec_menuinput <options|players|view> [entry [subentry]]".into()),
                };
                let nums: Vec<usize> = a[1..].iter().filter_map(|n| n.parse().ok()).collect();
                w.resource_mut::<SpectatorMenu>().0 = true;
                let mut lists = w.resource_mut::<MenuLists>();
                lists.queued.push(Hit::Box(list));
                match nums.as_slice() {
                    [] => {}
                    [i] => lists.queued.push(Hit::Row(list, *i)),
                    [i, j, ..] => {
                        lists.queued.push(Hit::Row(list, *i));
                        lists.queued.push(Hit::Sub(*j));
                    }
                }
                Ok(None)
            },
        );
    }
}

/// Whether the spectator menu is open.
#[derive(Resource, Default, Debug, Clone, Copy, PartialEq)]
pub struct SpectatorMenu(pub bool);

/// The menu's drop-down lists.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum List {
    Options,
    Players,
    View,
}

impl List {
    fn owner(self) -> usize {
        self as usize
    }
}

/// The open list (`ComboList::owner`: `List as usize`) and the open
/// submenu (its `owner`: the options entry it belongs to).
#[derive(Resource, Default, Debug, Clone, PartialEq)]
pub struct MenuLists {
    pub open: Option<ComboList>,
    pub sub: Option<ComboList>,
    /// Clicks waiting (`spec_menuinput`).
    queued: Vec<Hit>,
}

/// What a click lands on.
#[derive(Component, Clone, Copy, Debug, PartialEq)]
enum Hit {
    Box(List),
    Row(List, usize),
    Sub(usize),
    Prev,
    Next,
    /// Outside the open list: closes it.
    Outside,
}

/// The cvars and commands the options' entries use: which exist and the
/// toggles' values (read from the console each frame the menu is open).
#[derive(Resource, Default, Debug, Clone, PartialEq)]
struct CvarStates {
    known: HashSet<String>,
    on: HashMap<String, bool>,
    overview: bool,
}

/// One entry as shown: its words, whether it works, whether it opens a
/// submenu.
#[derive(Clone, Debug, PartialEq)]
pub struct Entry {
    pub text: String,
    pub enabled: bool,
    pub submenu: bool,
}

/// Whether an options entry works here: its command or toggle exists, and
/// the overview's need a map overview (`overview_mode 0` always works).
fn works(item: &CommandMenuItem, known: &HashSet<String>, overview: bool) -> bool {
    if !item.items.is_empty() {
        return item.items.iter().any(|i| works(i, known, overview));
    }
    let line = item.command.as_deref().or(item.toggle.as_deref()).unwrap_or("");
    let word = line.split_whitespace().next().unwrap_or("").to_lowercase();
    if word.is_empty() || !known.contains(&word) {
        return false;
    }
    let off = line.split_whitespace().nth(1) == Some("0");
    !(word.starts_with("overview_") && !overview && !(word == "overview_mode" && off))
}

/// The options list's (or a submenu's) entries as shown.
pub fn entries(
    items: &[CommandMenuItem],
    states: &HashMap<String, bool>,
    known: &HashSet<String>,
    overview: bool,
) -> Vec<Entry> {
    items
        .iter()
        .map(|i| {
            let on = i.toggle.as_ref().and_then(|t| states.get(&t.to_lowercase())).copied();
            let text = match on {
                // VGUI's check mark beside a ticked toggle.
                Some(true) => format!("\u{221A} {}", i.label),
                Some(false) => format!("    {}", i.label),
                None if !i.items.is_empty() => format!("{}  \u{25BA}", i.label),
                None => i.label.clone(),
            };
            Entry {
                text,
                enabled: works(i, known, overview),
                submenu: !i.items.is_empty(),
            }
        })
        .collect()
}

/// The console line an options entry runs: a toggle flips its cvar.
pub fn line(item: &CommandMenuItem, on: Option<bool>) -> Option<String> {
    if let Some(t) = &item.toggle {
        return Some(format!("{t} {}", if on == Some(true) { 0 } else { 1 }));
    }
    item.command.clone()
}

/// Our words when the install lacks its menu files.
fn fallback_options() -> Vec<CommandMenuItem> {
    let item =
        |label: &str, command: Option<&str>, toggle: Option<&str>, items: Vec<CommandMenuItem>| CommandMenuItem {
            label: label.into(),
            command: command.map(String::from),
            toggle: toggle.map(String::from),
            items,
        };
    vec![
        item("Close", Some("spec_menu 0"), None, vec![]),
        item(
            "Settings",
            None,
            None,
            vec![
                item("No Rotation", None, Some("overview_locked"), vec![]),
                item("Show Names", None, Some("overview_names"), vec![]),
                item("Show Health", None, Some("overview_health"), vec![]),
                item("Show Tracks", None, Some("overview_tracks"), vec![]),
            ],
        ),
        item(
            "Overview",
            None,
            None,
            vec![
                item("No Map", Some("overview_mode 0"), None, vec![]),
                item("Small Map", Some("overview_mode 1"), None, vec![]),
                item("Large Map", Some("overview_mode 2"), None, vec![]),
                item("Zoom In", Some("overview_zoom 1.1 0.1 rel"), None, vec![]),
                item("Zoom Out", Some("overview_zoom 0.9 0.1 rel"), None, vec![]),
            ],
        ),
        item("Show Scores", Some("togglescores"), None, vec![]),
    ]
}

fn fallback_modes() -> Vec<CommandMenuItem> {
    [SpecMode::InEye, SpecMode::Chase, SpecMode::Roaming]
        .map(|m| CommandMenuItem {
            label: m.name().into(),
            command: Some(format!("spec_mode {}", m.number())),
            ..default()
        })
        .into()
}

/// The camera list's entry for a mode (its `spec_mode` number).
fn mode_row(modes: &[CommandMenuItem], mode: SpecMode) -> Option<usize> {
    let want = format!("spec_mode {}", mode.number());
    modes
        .iter()
        .position(|m| m.command.as_deref().map(str::trim) == Some(want.as_str()))
}

/// The bottom bar's controls in 640 x 480 units when the install lacks
/// `bottomspectator.res` (its numbers).
fn fallback_layout() -> UiLayout {
    use crate::map::hud::{HudCoord, UiControl, UiKind};
    let c = |name: &str, kind: UiKind, x: HudCoord, w: f32| {
        let mut c = UiControl::new(name, kind, 0.0, 14.0, w, 22.0);
        c.x = x;
        c
    };
    let combo = || UiKind::Other("ComboBox".into());
    UiLayout {
        controls: vec![
            UiControl::new("specmenu", UiKind::Frame, 0.0, 428.0, 640.0, 55.0),
            c("settingscombo", combo(), HudCoord::Start(5.0), 126.0),
            c("playercombo", combo(), HudCoord::Centre(-141.0), 282.0),
            c("viewcombo", combo(), HudCoord::End(131.0), 126.0),
            c("specprev", UiKind::Button, HudCoord::Centre(-166.0), 20.0),
            c("specnext", UiKind::Button, HudCoord::Centre(146.0), 20.0),
        ],
    }
}

#[allow(clippy::too_many_arguments)]
fn toggle(
    (keys, mouse): (Res<ButtonInput<KeyCode>>, Res<ButtonInput<MouseButton>>),
    cursor: Option<Single<&CursorOptions>>,
    (ui, chat, game_menu): (
        Option<Res<super::console::ConsoleUi>>,
        Option<Res<super::chat::ChatInput>>,
        Option<Res<super::game_menu::GameMenu>>,
    ),
    console: Res<Console>,
    spec: Res<Spectator>,
    mut menu: ResMut<SpectatorMenu>,
    mut lists: ResMut<MenuLists>,
    mut open: ResMut<VguiOpen>,
    team_menu: Option<Res<super::team_menu::TeamMenu>>,
) {
    let watching = spec.phase == SpecPhase::Watching;
    if !watching || team_menu.is_some_and(|t| t.0) {
        menu.set_if_neq(SpectatorMenu(false));
    } else if let Some(cursor) = cursor
        && super::vgui::keys_live(&cursor, &open, ui.as_deref(), chat.as_deref(), game_menu.as_deref())
        && super::binds::just_pressed(&console.binds, &keys, &mouse, "+duck")
    {
        menu.0 = !menu.0;
    }
    if !menu.0 && (lists.open.is_some() || lists.sub.is_some()) {
        lists.open = None;
        lists.sub = None;
    }
    let want = VguiOpen {
        spectator: menu.0,
        ..*open
    };
    open.set_if_neq(want);
}

/// The toggles' values and which commands exist, while open.
fn read_cvars(world: &mut World) {
    if !world.resource::<SpectatorMenu>().0 {
        return;
    }
    let toggles: Vec<String> = options_of(world)
        .iter()
        .flat_map(|i| std::iter::once(i).chain(i.items.iter()))
        .filter_map(|i| i.toggle.clone())
        .collect();
    let console = world.resource::<Console>();
    let known: HashSet<String> = console.names().into_iter().map(|n| n.to_lowercase()).collect();
    let getters: Vec<(String, crate::console::GetFn)> = toggles
        .iter()
        .filter_map(|t| console.cvar(t).map(|c| (t.to_lowercase(), c.get.clone())))
        .collect();
    let on = getters
        .into_iter()
        .map(|(name, get)| {
            let v = get(world).unwrap_or_default();
            (name, v.trim().parse::<f32>().is_ok_and(|v| v != 0.0))
        })
        .collect();
    let states = CvarStates {
        known,
        on,
        overview: world.contains_resource::<crate::map::hud::ActiveOverview>(),
    };
    let mut current = world.resource_mut::<CvarStates>();
    if *current != states {
        *current = states;
    }
}

/// The options' entries: the install's, else ours.
fn options_of(world: &World) -> Vec<CommandMenuItem> {
    world
        .get_resource::<crate::map::hud::ActiveHud>()
        .and_then(|h| h.0.menus.as_ref())
        .map(|m| m.spectator_options.clone())
        .filter(|o| !o.is_empty())
        .unwrap_or_else(fallback_options)
}

/// The menus' entries (install's or ours) as one value for the systems.
fn menus_of(hud: Option<&crate::map::hud::ActiveHud>) -> (Vec<CommandMenuItem>, Vec<CommandMenuItem>) {
    let menus = hud.and_then(|h| h.0.menus.as_ref());
    let options = menus
        .map(|m| m.spectator_options.clone())
        .filter(|o| !o.is_empty())
        .unwrap_or_else(fallback_options);
    let modes = menus
        .map(|m| m.spectator_modes.clone())
        .filter(|o| !o.is_empty())
        .unwrap_or_else(fallback_modes);
    (options, modes)
}

/// Clicks: open and close the lists, pick entries.
#[allow(clippy::too_many_arguments)]
fn click(
    buttons: Query<(&Interaction, &Hit), Changed<Interaction>>,
    menu: Res<SpectatorMenu>,
    mut lists: ResMut<MenuLists>,
    mut spec: ResMut<Spectator>,
    mut console: ResMut<Console>,
    states: Res<CvarStates>,
    hud: Option<Res<crate::map::hud::ActiveHud>>,
) {
    if !menu.0 {
        lists.queued.clear();
        return;
    }
    let mut hits: Vec<Hit> = std::mem::take(&mut lists.queued);
    hits.extend(
        buttons
            .iter()
            .filter(|(i, _)| **i == Interaction::Pressed)
            .map(|(_, h)| *h),
    );
    if hits.is_empty() {
        return;
    }
    let (options, modes) = menus_of(hud.as_deref());
    let rows = |list: List, spec: &Spectator| match list {
        List::Options => options.len(),
        List::Players => spec.watchable.len(),
        List::View => modes.len(),
    };
    for hit in hits {
        match hit {
            Hit::Box(list) => {
                let was = lists.open.map(|l| l.owner);
                lists.sub = None;
                lists.open = if was == Some(list.owner()) {
                    None
                } else {
                    let selected = match list {
                        List::Players => spec
                            .target
                            .and_then(|t| spec.watchable.iter().position(|e| *e == t))
                            .unwrap_or(0),
                        List::View => mode_row(&modes, spec.effective_mode()).unwrap_or(0),
                        List::Options => 0,
                    };
                    Some(ComboList::open(list.owner(), rows(list, &spec), selected))
                };
            }
            Hit::Outside => {
                lists.open = None;
                lists.sub = None;
            }
            Hit::Prev => spec.pending.target_step = -1,
            Hit::Next => spec.pending.target_step = 1,
            Hit::Row(List::Options, i) => {
                let Some(item) = options.get(i) else { continue };
                if !item.items.is_empty() {
                    if works(item, &states.known, states.overview) {
                        lists.sub = Some(submenu(i, item.items.len()));
                        if let Some(l) = lists.open.as_mut() {
                            l.hover(i);
                        }
                    }
                    continue;
                }
                if works(item, &states.known, states.overview) {
                    let on = item
                        .toggle
                        .as_ref()
                        .and_then(|t| states.on.get(&t.to_lowercase()))
                        .copied();
                    if let Some(l) = line(item, on) {
                        console.submit(l);
                    }
                }
                lists.open = None;
                lists.sub = None;
            }
            Hit::Sub(j) => {
                let Some(item) = lists
                    .sub
                    .and_then(|s| options.get(s.owner))
                    .and_then(|p| p.items.get(j))
                else {
                    continue;
                };
                if works(item, &states.known, states.overview) {
                    let on = item
                        .toggle
                        .as_ref()
                        .and_then(|t| states.on.get(&t.to_lowercase()))
                        .copied();
                    if let Some(l) = line(item, on) {
                        console.submit(l);
                    }
                    lists.open = None;
                    lists.sub = None;
                }
            }
            Hit::Row(List::Players, i) => {
                if let Some(e) = spec.watchable.get(i).copied() {
                    spec.pending.pick = Some(e);
                }
                lists.open = None;
            }
            Hit::Row(List::View, i) => {
                if let Some(c) = modes.get(i).and_then(|m| m.command.clone()) {
                    console.submit(c);
                }
                lists.open = None;
            }
        }
    }
    if lists.open.is_none() {
        lists.sub = None;
    }
}

/// A submenu just opened beside options entry `owner`: nothing lit yet.
fn submenu(owner: usize, len: usize) -> ComboList {
    ComboList {
        owner,
        len,
        highlight: usize::MAX,
        first: 0,
    }
}

/// The pointer over a list's entry highlights it (an options entry with a
/// submenu opens it, as VGUI's cascading menus).
fn hover(
    rows: Query<(&Interaction, &Hit), Changed<Interaction>>,
    mut lists: ResMut<MenuLists>,
    hud: Option<Res<crate::map::hud::ActiveHud>>,
    states: Res<CvarStates>,
) {
    for (interaction, hit) in &rows {
        if *interaction != Interaction::Hovered {
            continue;
        }
        match *hit {
            Hit::Row(list, i) if lists.open.is_some_and(|l| l.owner == list.owner()) => {
                if lists.open.is_some_and(|l| l.highlight != i)
                    && let Some(l) = lists.open.as_mut()
                {
                    l.hover(i);
                }
                if list == List::Options {
                    let (options, _) = menus_of(hud.as_deref());
                    let sub = options
                        .get(i)
                        .filter(|o| !o.items.is_empty() && works(o, &states.known, states.overview))
                        .map(|o| submenu(i, o.items.len()));
                    if lists.sub.map(|s| s.owner) != sub.map(|s| s.owner) {
                        lists.sub = sub;
                    }
                }
            }
            Hit::Sub(j) => {
                if lists.sub.is_some_and(|s| s.highlight != j)
                    && let Some(s) = lists.sub.as_mut()
                {
                    s.hover(j);
                }
            }
            _ => {}
        }
    }
}

/// Esc closes an open list.
fn keys(keys: Res<ButtonInput<KeyCode>>, mut lists: ResMut<MenuLists>) {
    if lists.open.is_some() && keys.just_pressed(KeyCode::Escape) {
        lists.open = None;
        lists.sub = None;
    }
}

#[derive(Component)]
struct MenuRoot;

/// What the menu shows; redrawn when it changes.
#[derive(Clone, Debug, PartialEq)]
struct Shot {
    players: Vec<String>,
    target: Option<usize>,
    mode: SpecMode,
    size: Vec2,
    lists: MenuLists,
    states: CvarStates,
}

#[allow(clippy::too_many_arguments, clippy::type_complexity)]
fn draw(
    menu: Res<SpectatorMenu>,
    lists: Res<MenuLists>,
    states: Res<CvarStates>,
    spec: Res<Spectator>,
    who: Query<(Option<&Name>, Option<&Team>, Option<&Health>)>,
    roots: Query<Entity, With<MenuRoot>>,
    windows: Query<&Window>,
    fonts: Option<Res<super::fonts::UiFonts>>,
    hud: Option<Res<crate::map::hud::ActiveHud>>,
    game_menu: Option<Res<super::game_menu::GameMenu>>,
    mut last: Local<Option<Shot>>,
    mut commands: Commands,
) {
    let Some(fonts) = fonts.filter(|_| menu.0) else {
        for e in &roots {
            commands.entity(e).despawn();
        }
        *last = None;
        return;
    };
    let size = windows
        .iter()
        .next()
        .map_or(Vec2::new(640.0, 480.0), |w| Vec2::new(w.width(), w.height()));
    let players: Vec<String> = spec
        .watchable
        .iter()
        .map(|e| {
            let (name, _, _) = who.get(*e).unwrap_or((None, None, None));
            name.map_or_else(|| "Player".to_string(), |n| n.as_str().to_string())
        })
        .collect();
    let shot = Shot {
        target: spec.target.and_then(|t| spec.watchable.iter().position(|e| *e == t)),
        players,
        mode: spec.effective_mode(),
        size,
        lists: MenuLists {
            queued: Vec::new(),
            ..lists.clone()
        },
        states: states.clone(),
    };
    if last.as_ref() == Some(&shot) && !roots.is_empty() {
        return;
    }
    for e in &roots {
        commands.entity(e).despawn();
    }
    let game_menus = hud.as_ref().and_then(|h| h.0.menus.as_ref());
    let text = |token: &str, fallback: &str| {
        game_menus.map_or_else(|| fallback.to_string(), |m| m.string(token, fallback).to_string())
    };
    let (options, modes) = menus_of(hud.as_deref());
    let layout = game_menus
        .and_then(|m| m.layouts.get(m.spectator_menu.as_ref()?))
        .cloned()
        .unwrap_or_else(fallback_layout);
    // Laid out in 480-line units (the HUD's proportional layout); the
    // widgets draw in `look.s` pixels per unit, at the game menu's size
    // for their text (GameUI's scheme pixels).
    let hud_s = size.y / 480.0;
    let s = (size.y / 720.0).clamp(0.6, 3.0);
    let look = Look {
        ui: game_menu.as_ref().and_then(|m| m.ui.0.as_deref()),
        fonts: &fonts,
        s,
        height: size.y,
        accent: Color::srgb_u8(255, 176, 0),
    };
    let rects = layout.rects(Rect::from_corners(Vec2::ZERO, size), hud_s);
    let frame = layout
        .controls
        .iter()
        .position(|c| c.name.eq_ignore_ascii_case("specmenu"))
        .map_or(428.0 * hud_s, |i| rects[i].min.y);
    let rect = |name: &str| -> Option<(f32, f32, f32, f32)> {
        let i = layout.controls.iter().position(|c| c.name.eq_ignore_ascii_case(name))?;
        let r = rects[i];
        // Controls are placed inside the frame (its top), in the
        // widgets' units.
        Some((r.min.x / s, (r.min.y + frame) / s, r.width() / s, r.height() / s))
    };
    let root = commands
        .spawn((
            MenuRoot,
            Node {
                position_type: PositionType::Absolute,
                width: percent(100.0),
                height: percent(100.0),
                ..default()
            },
            // Over the spectator bars (43) and the overview (42).
            GlobalZIndex(52),
        ))
        .id();
    let open = |list: List| shot.lists.open.filter(|l| l.owner == list.owner());
    // Options: its value is the list's name.
    let option_entries = entries(&options, &states.on, &states.known, states.overview);
    let player_entries: Vec<String> = shot.players.clone();
    let mode_entries: Vec<String> = modes.iter().map(|m| m.label.clone()).collect();
    let mode_now = mode_row(&modes, shot.mode)
        .and_then(|i| mode_entries.get(i).cloned())
        .unwrap_or_else(|| shot.mode.name().to_string());
    let player_now = shot
        .target
        .filter(|_| shot.mode != SpecMode::Roaming)
        .and_then(|i| {
            let e = spec.watchable[i];
            let (name, _, health) = who.get(e).ok()?;
            let name = name.map_or("Player", |n| n.as_str());
            let hp = health.map_or(0.0, |h| (h.current * 100.0).ceil().max(0.0));
            Some(
                text("Spec_PlayerItem_Health", "%s1 (%s2)")
                    .replace("%s1", name)
                    .replace("%s2", &format!("{hp:.0}")),
            )
        })
        .unwrap_or_default();
    for (list, name, value) in [
        (List::Options, "settingscombo", text("Spec_Options", "Options")),
        (List::Players, "playercombo", player_now),
        (List::View, "viewcombo", mode_now),
    ] {
        let Some(r) = rect(name) else { continue };
        let enabled = list != List::Players || !player_entries.is_empty();
        widgets::combo_box(
            &mut commands,
            root,
            &look,
            r,
            &value,
            enabled,
            open(list).is_some(),
            false,
            Hit::Box(list),
        );
    }
    for (name, label, hit) in [("specprev", "<", Hit::Prev), ("specnext", ">", Hit::Next)] {
        if let Some(r) = rect(name) {
            widgets::button(
                &mut commands,
                root,
                &look,
                r,
                label,
                widgets::ButtonState::enabled(!player_entries.is_empty()),
                0,
                hit,
            );
        }
    }
    // The open list, upward from its box.
    let row_h = 20.0;
    if let Some(l) = shot.lists.open {
        let list = [List::Options, List::Players, List::View][l.owner.min(2)];
        let name = ["settingscombo", "playercombo", "viewcombo"][l.owner.min(2)];
        let words: Vec<String> = match list {
            List::Options => option_entries.iter().map(|e| e.text.clone()).collect(),
            List::Players => player_entries.clone(),
            List::View => mode_entries.clone(),
        };
        if let Some((x, y, w, h)) = rect(name) {
            let tall = l.rows() as f32 * row_h + 2.0;
            let top = y - tall;
            widgets::combo_popup(
                &mut commands,
                root,
                root,
                &look,
                (x, top - h, w, h),
                &words,
                &l,
                |i| Hit::Row(list, i),
                Hit::Outside,
            );
            // Entries that don't work: engraved over theirs (the popup
            // draws every entry alike).
            if list == List::Options {
                grey_rows(&mut commands, root, &look, (x, top, w), &l, &option_entries);
            }
            // The submenu, beside its entry, kept on screen.
            if let (List::Options, Some(sub)) = (list, shot.lists.sub)
                && let Some(parent) = options.get(sub.owner)
            {
                let sub_entries = entries(&parent.items, &states.on, &states.known, states.overview);
                let words: Vec<String> = sub_entries.iter().map(|e| e.text.clone()).collect();
                let sub_w = 140.0;
                let sub_tall = sub.rows() as f32 * row_h + 2.0;
                let row_top = top + (sub.owner.saturating_sub(l.first)) as f32 * row_h;
                let sub_top = row_top.min(size.y / s - sub_tall);
                let sub_x = (x + w).min(size.x / s - sub_w);
                let shown = sub;
                widgets::combo_popup(
                    &mut commands,
                    root,
                    root,
                    &look,
                    (sub_x, sub_top - h, sub_w, h),
                    &words,
                    &shown,
                    |j| Hit::Sub(j),
                    Hit::Outside,
                );
                grey_rows(
                    &mut commands,
                    root,
                    &look,
                    (sub_x, sub_top, sub_w),
                    &shown,
                    &sub_entries,
                );
            }
        }
    }
    *last = Some(shot);
}

/// Disabled entries of an open list drawn engraved over the list's own
/// row (which then takes no click: covered).
fn grey_rows(
    commands: &mut Commands,
    root: Entity,
    look: &Look,
    (x, top, w): (f32, f32, f32),
    l: &ComboList,
    rows: &[Entry],
) {
    let row_h = 20.0;
    // As the popup paints its menu: a solid backing, the menu's colour.
    let backing = {
        let c = look.frame_bg().to_srgba();
        Color::srgb(c.red * 0.45, c.green * 0.45, c.blue * 0.45)
    };
    let bg = look.color("Menu.BgColor", [40, 40, 40, 255]);
    for (k, i) in (l.first..(l.first + l.rows()).min(rows.len())).enumerate() {
        if rows[i].enabled {
            continue;
        }
        let under = commands
            .spawn((
                widgets::place(look, x + 1.0, top + 1.0 + k as f32 * row_h, w - 2.0, row_h),
                BackgroundColor(backing),
                bevy::ui::FocusPolicy::Block,
                Interaction::default(),
                GlobalZIndex(widgets::POPUP_Z + 1),
                ChildOf(root),
            ))
            .id();
        let cover = commands
            .spawn((
                Node {
                    position_type: PositionType::Absolute,
                    width: percent(100.0),
                    height: percent(100.0),
                    ..default()
                },
                BackgroundColor(bg),
                ChildOf(under),
            ))
            .id();
        widgets::engraved(
            commands,
            cover,
            look,
            (3.0, 0.0, w - 10.0, row_h),
            &rows[i].text,
            look.default_font(),
            -1,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn known(names: &[&str]) -> HashSet<String> {
        names.iter().map(|n| n.to_string()).collect()
    }

    #[test]
    fn options_tick_toggles_open_submenus_and_grey_what_mashup_lacks() {
        let mut options = fallback_options();
        options.push(CommandMenuItem {
            label: "Auto Director".into(),
            toggle: Some("spec_autodirector".into()),
            ..default()
        });
        let k = known(&[
            "spec_menu",
            "overview_locked",
            "overview_names",
            "overview_mode",
            "overview_zoom",
            "togglescores",
        ]);
        let on = HashMap::from([
            ("overview_names".to_string(), true),
            ("overview_locked".to_string(), false),
        ]);
        let top = entries(&options, &on, &k, true);
        assert_eq!(top[0].text, "Close");
        assert!(top[1].submenu && top[1].text.starts_with("Settings"));
        assert!(!top.last().unwrap().enabled, "no auto director in mashup");
        let settings = entries(&options[1].items, &on, &k, true);
        assert!(settings[1].text.starts_with('\u{221A}'), "Show Names ticked");
        assert!(!settings[0].text.starts_with('\u{221A}'));
        assert_eq!(
            line(&options[1].items[1], Some(true)).as_deref(),
            Some("overview_names 0")
        );
        assert_eq!(
            line(&options[1].items[0], Some(false)).as_deref(),
            Some("overview_locked 1")
        );
        // Without a map overview only "No Map" works.
        let overview = entries(&options[2].items, &on, &k, false);
        assert!(overview[0].enabled && !overview[1].enabled && !overview[2].enabled && !overview[3].enabled);
        assert!(entries(&options[2].items, &on, &k, true).iter().all(|e| e.enabled));
    }

    #[test]
    fn the_camera_list_follows_the_mode() {
        let modes = fallback_modes();
        assert_eq!(mode_row(&modes, SpecMode::Chase), Some(1));
        assert_eq!(modes[2].command.as_deref(), Some("spec_mode 6"));
    }
}
