//! The main menu and the in-game menu (Esc, or `menu [page]`), one menu
//! drawn as CS:S's GameUI draws it. Out of a game (startup without a map,
//! after `disconnect`) it is the main menu: always open, over the game's
//! background picture, the game's title over its entries, in-game entries
//! hidden. In a game: the entries at the left over the darkened game, each
//! dialog a frame in the middle of the screen (title bar, raised borders,
//! the options' tabs), in the install's GameUI scheme (`map::hud::GameUi`:
//! `SourceScheme.res` colours, numbers and fonts, `GameMenu.res` entries
//! with ours added, the keyboard tab's `kb_act.lst` actions, localised
//! words, map thumbnails); without the install, in built-in colours and
//! words. The game keeps running while it is open; the mouse is free, so
//! the local player's input is ignored (`input::write_local_intent`). Every
//! choice is a console line (`map`, `bot_add`, `bind`, cvars ...), so the
//! menu holds no game logic: the model (`GameMenu::handle`) turns keys
//! and clicks into those lines, and is unit-tested without a window.

use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    sync::{Arc, Mutex},
};

use bevy::{
    input::mouse::{AccumulatedMouseScroll, MouseScrollUnit},
    prelude::*,
    text::LineBreak,
    ui::RelativeCursorPosition,
    window::CursorOptions,
};

use super::{
    binds,
    fonts::UiFonts,
    options::{SETTINGS, SettingKind, TABS, Tab},
};
use crate::{
    console::{Console, ConsoleAppExt},
    core::{Intent, Team},
    map::hud::{GameUi, KeyAction},
};

pub struct GameMenuPlugin;

impl Plugin for GameMenuPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<GameMenu>()
            .init_resource::<LoadingDetails>()
            .init_resource::<LoadTimeline>()
            .init_resource::<AfterLoad>()
            .init_resource::<RegrabCursor>()
            .init_resource::<MenuUi>()
            .add_systems(Startup, (start_loading_ui, start_session))
            .add_systems(
                Update,
                (
                    ui_loaded,
                    keys.before(super::console::toggle),
                    pointer,
                    sync,
                    cursor,
                    after_load,
                    menu_sounds,
                    draw,
                    loading_progress,
                )
                    .chain()
                    .in_set(MenuSystems),
            );
        crate::console::resource_cvar::<LoadingDetails, u8>(
            app,
            "mashup_loading_details",
            "1: the loading dialog adds a detailed view (each stage's time, bytes and percentage; the server's name, \
             map, players and ping).",
            |d| &mut d.0,
        );
        app.console_command(
            "menu",
            "menu [main|newgame|maps|bots|team|options|keyboard|mouse|audio|video|multiplayer]: open the game menu (Esc) on a page (options: on a tab).",
            |w, a| {
                let arg = a.first().map(|s| s.to_lowercase());
                let tab = TABS.iter().find(|(t, ..)| Some(t.page()) == arg.as_deref()).map(|(t, ..)| *t);
                let page = match arg.as_deref() {
                    _ if tab.is_some() => Page::Settings,
                    None | Some("main") => Page::Main,
                    Some("newgame") => Page::NewGame,
                    Some("maps") => Page::Maps,
                    Some("bots") => Page::Bots,
                    Some("team") => Page::Team,
                    Some("options") | Some("settings") => Page::Settings,
                    Some(p) => return Err(format!("no menu page \"{p}\"")),
                };
                open_menu(w, page);
                if let Some(tab) = tab {
                    w.resource_mut::<GameMenu>().set_tab(tab);
                }
                Ok(None)
            },
        )
        .console_command("gameui_activate", "Open the game menu (Esc).", |w, _| {
            open_menu(w, Page::Main);
            Ok(None)
        })
        .console_command("gameui_hide", "Close the game menu (in a game).", |w, _| {
            let mut menu = w.resource_mut::<GameMenu>();
            if !menu.in_game {
                return Err("the main menu stays open out of a game".into());
            }
            menu.open = false;
            w.resource_mut::<RegrabCursor>().0 = true;
            Ok(None)
        });
    }
}

/// The menu's systems (the server browser runs after them).
#[derive(SystemSet, Clone, Debug, PartialEq, Eq, Hash)]
pub struct MenuSystems;

// ---------------------------------------------------------------------------
// The model.

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Page {
    #[default]
    Main,
    NewGame,
    /// The map list, picked for a new game.
    Maps,
    Bots,
    Team,
    /// The options dialog (its tab: `GameMenu::tab`).
    Settings,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Mode {
    #[default]
    Deathmatch,
    Rounds,
}

/// Bot skill presets: reaction seconds, aim error degrees, turn rate
/// degrees per second (`bot_reaction`, `bot_aim_error`, `bot_turn_rate`).
/// Normal is the bots' default (`bot::BotConfig`).
pub const DIFFICULTIES: [(&str, f32, f32, f32); 4] = [
    ("Easy", 0.6, 5.0, 220.0),
    ("Normal", 0.35, 2.5, 360.0),
    ("Hard", 0.2, 1.5, 540.0),
    ("Expert", 0.12, 0.75, 720.0),
];
const NORMAL: usize = 1;
/// Bots per team a new game offers at most.
const MAX_BOTS: u8 = 15;

/// What a new game starts with.
#[derive(Clone, Debug, PartialEq)]
pub struct NewGame {
    /// Index into `GameMenu::maps`.
    pub map: usize,
    pub mode: Mode,
    pub bots_t: u8,
    pub bots_ct: u8,
    /// Index into `DIFFICULTIES`.
    pub difficulty: usize,
}

impl Default for NewGame {
    fn default() -> Self {
        Self {
            map: 0,
            mode: Mode::Deathmatch,
            bots_t: 0,
            bots_ct: 0,
            difficulty: NORMAL,
        }
    }
}

/// What a left-hand entry does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MainItem {
    Resume,
    /// Leave the game for the main menu (`disconnect`).
    Disconnect,
    /// The server browser (`openserverbrowser`).
    FindServers,
    /// The game's "Create Server": our new game page.
    NewGame,
    Bots,
    Team,
    Options,
    BugReport,
    Quit,
    /// Ours: a rounds game on `QUICK_MAP` with bots, at once.
    QuickStart,
    /// Ours: mashup's greybox test map (`map greybox`).
    Greybox,
    /// Ours: open the console (`toggleconsole`).
    Console,
}

/// The game's entries without its menu file, in CS:S's order and words:
/// item, text, shown only in a game.
pub const MAIN: [(MainItem, &str, bool); 7] = [
    (MainItem::Resume, "Resume Game", true),
    (MainItem::Disconnect, "Disconnect", true),
    (MainItem::FindServers, "Find Servers", false),
    (MainItem::NewGame, "Create Server", false),
    (MainItem::BugReport, "Report a Bug", false),
    (MainItem::Options, "Options", false),
    (MainItem::Quit, "Quit", false),
];

/// Ours, after the game's entries (and a gap): item, text, shown only in a
/// game.
pub const OURS: [(MainItem, &str, bool); 5] = [
    (MainItem::QuickStart, "Quick Start", false),
    (MainItem::Greybox, "Greybox Test Map", false),
    (MainItem::Bots, "Bots", true),
    (MainItem::Team, "Team", true),
    (MainItem::Console, "Console", false),
];

/// The map a quick start plays (rounds, `QUICK_BOTS` normal bots per
/// team: terrorists, counter-terrorists besides you); the first map when
/// the install lacks it.
pub const QUICK_MAP: &str = "de_dust2";
const QUICK_BOTS: (u8, u8) = (5, 4);

impl MainItem {
    fn page(self) -> Option<Page> {
        match self {
            MainItem::NewGame => Some(Page::NewGame),
            MainItem::Bots => Some(Page::Bots),
            MainItem::Team => Some(Page::Team),
            MainItem::Options => Some(Page::Settings),
            _ => None,
        }
    }
}

/// A left-hand entry: what it does, its text, whether a gap comes before
/// it (only in a game: `gap_in_game_only`), whether it shows only in a
/// game, and whether it can be pressed.
#[derive(Clone, Debug, PartialEq)]
pub struct MainEntry {
    pub item: MainItem,
    pub label: String,
    pub gap: bool,
    pub gap_in_game_only: bool,
    pub in_game_only: bool,
    pub enabled: bool,
}

impl MainEntry {
    fn new(item: MainItem, label: &str, in_game_only: bool) -> Self {
        Self {
            item,
            label: label.to_uppercase(),
            gap: false,
            gap_in_game_only: false,
            in_game_only,
            enabled: true,
        }
    }
}

/// Every left-hand entry, shown or not: the game's (`GameMenu.res`) that
/// mashup has, in its order and words (built-in ones in CS:S's order
/// without the file); then, after a gap, ours (quick
/// start, the greybox, bots, team, console). Entries mashup can't do
/// (player list, achievements, benchmark ...) are left out.
/// `GameMenu::entries` picks those shown in or out of a game.
pub fn main_entries(ui: Option<&GameUi>) -> Vec<MainEntry> {
    let builtin = |item: MainItem| {
        let (_, label, in_game) = MAIN.iter().find(|(m, ..)| *m == item).copied().unwrap_or((item, "", false));
        MainEntry::new(item, label, in_game)
    };
    let mut out: Vec<MainEntry> = Vec::new();
    match ui.filter(|u| !u.menu.is_empty()) {
        None => {
            out.extend(MAIN.iter().map(|(item, ..)| builtin(*item)));
            // CS:S's gap under Disconnect, shown in a game.
            out[2].gap = true;
            out[2].gap_in_game_only = true;
        }
        Some(ui) => {
            let mut gap: Option<bool> = None;
            for e in &ui.menu {
                if e.label.trim().is_empty() && e.command.trim().is_empty() {
                    if !out.is_empty() {
                        gap = Some(e.in_game_only);
                    }
                    continue;
                }
                let item = match e.command.trim().to_lowercase().as_str() {
                    "resumegame" => MainItem::Resume,
                    "disconnect" => MainItem::Disconnect,
                    "openserverbrowser" => MainItem::FindServers,
                    "opennewgamedialog" | "opencreatemultiplayergamedialog" => MainItem::NewGame,
                    "openoptionsdialog" => MainItem::Options,
                    "quit" | "quitnoconfirm" => MainItem::Quit,
                    "engine bug" => MainItem::BugReport,
                    _ => continue,
                };
                if out.iter().any(|m| m.item == item) {
                    continue;
                }
                let mut entry = MainEntry::new(item, "", e.in_game_only || builtin(item).in_game_only);
                entry.label = e.label.clone();
                if let Some(in_game) = gap.take() {
                    entry.gap = true;
                    entry.gap_in_game_only = in_game;
                }
                out.push(entry);
            }
            // Whatever the file lacks, where CS:S has it.
            for (i, (item, ..)) in MAIN.iter().enumerate() {
                if out.iter().any(|m| m.item == *item) {
                    continue;
                }
                let at = match i {
                    0 => 0,
                    1 => out.iter().position(|m| m.item == MainItem::Resume).map_or(0, |p| p + 1),
                    _ => out.iter().position(|m| m.item == MainItem::Quit).unwrap_or(out.len()),
                };
                out.insert(at, builtin(*item));
            }
        }
    }
    for (k, (item, label, in_game)) in OURS.iter().enumerate() {
        let mut e = MainEntry::new(*item, label, *in_game);
        e.gap = k == 0;
        out.push(e);
    }
    out
}

/// A value a row changes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Field {
    Map,
    Mode,
    BotsT,
    BotsCt,
    Difficulty,
    /// Index into `options::SETTINGS`.
    Setting(usize),
}

/// What pressing a button row does.
#[derive(Clone, Debug, PartialEq)]
pub enum Action {
    Main(MainItem),
    Back,
    Start,
    PickMap(usize),
    /// Run a console line; close the menu too when set.
    Run(String, bool),
    /// Wait for a key for the selected keyboard action.
    EditKey,
    /// Unbind the selected keyboard action's keys.
    ClearKey,
    /// Every bind back to the defaults.
    DefaultBinds,
}

/// A row of a page.
#[derive(Clone, Debug, PartialEq)]
pub enum Row {
    Button { label: String, action: Action, enabled: bool },
    Value { label: String, value: String, field: Field },
    /// A keyboard action: its description, console line and keys (as
    /// shown); `known`: mashup has the command.
    Bind { label: String, command: String, keys: String, known: bool },
    /// A section heading in a list.
    Heading(String),
    /// Text that can't be focused.
    Info(String),
}

impl Row {
    fn focusable(&self) -> bool {
        match self {
            Row::Button { enabled, .. } => *enabled,
            Row::Value { .. } | Row::Bind { .. } => true,
            Row::Info(_) | Row::Heading(_) => false,
        }
    }
}

/// What a click or hover is on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Target {
    /// A left-hand entry (index into `GameMenu::main`).
    Main(usize),
    /// A row of the open page.
    Row(usize),
    /// An options tab (index into `options::TABS`).
    Tab(usize),
    /// A list's scroll bar (the click's step says how far).
    Scroll,
    /// The loading dialog's Cancel button.
    Cancel,
}

/// A key or pointer event for the menu.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Input {
    Up,
    Down,
    Left,
    Right,
    /// Enter or Space.
    Activate,
    /// Backspace: back to the main page.
    Back,
    /// Esc: close (or stop waiting for a key).
    Close,
    Hover(Target),
    /// A click: on the row itself (0) or its `<` (-1) / `>` (+1) arrow; on
    /// the scroll bar, rows to scroll.
    Click(Target, i32),
    /// A typed letter or digit: jumps the map list.
    Char(char),
    /// Tab (+1) / Shift+Tab (-1): the next options tab.
    NextTab(i32),
    /// The wheel over a list: rows to scroll.
    Scroll(i32),
    /// A slider row pressed or dragged at a position (0 to 1).
    Slide(usize, f32),
    /// The key, button or wheel notch pressed while waiting for one (a
    /// Source key name).
    BindKey(&'static str),
}

/// What an input asks for besides the menu's own state.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Outcome {
    /// Console lines to run now.
    pub lines: Vec<String>,
    /// Console lines to run once the map that `lines` loads is in.
    pub after_load: Vec<String>,
    /// The menu closed: back to playing.
    pub close: bool,
}

/// The game's GameUI look and words, shared (compared by identity).
#[derive(Clone, Default)]
pub struct UiText(pub Option<Arc<GameUi>>);

impl PartialEq for UiText {
    fn eq(&self, other: &Self) -> bool {
        match (&self.0, &other.0) {
            (Some(a), Some(b)) => Arc::ptr_eq(a, b),
            (None, None) => true,
            _ => false,
        }
    }
}

impl std::fmt::Debug for UiText {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(if self.0.is_some() { "GameUi" } else { "none" })
    }
}

/// The menu's state.
#[derive(Resource, Clone, Debug, Default, PartialEq)]
pub struct GameMenu {
    pub open: bool,
    /// A map is loaded and played: false at the main menu (startup with no
    /// map, after `disconnect`), where the menu stays open over the game's
    /// background and in-game entries are hidden.
    pub in_game: bool,
    /// A map the main menu started is loading (its name), or a server is
    /// being joined (its address, then its map): the loading dialog
    /// shows that and the menu takes no input until the map is in.
    pub loading: Option<String>,
    /// The loading dialog shows why joining (or the game) failed, in the
    /// game's words (`net_failure_text`), until it's closed.
    pub failure: Option<String>,
    pub page: Page,
    /// The focused row of the page (of `main` on the main page).
    pub focus: usize,
    /// Loadable maps (`maps`).
    pub maps: Vec<String>,
    pub new_game: NewGame,
    /// Each setting's current value (`options::SETTINGS` order), None if
    /// its cvar isn't registered.
    pub values: Vec<Option<String>>,
    /// Bots in the game now: terrorists, counter-terrorists.
    pub bots: [usize; 2],
    /// Everyone in the game now, by team (auto-assign).
    pub players: [usize; 2],
    /// The options dialog's tab.
    pub tab: Tab,
    /// The left-hand entries.
    pub main: Vec<MainEntry>,
    /// The game's look and words, when the install has them.
    pub ui: UiText,
    /// The binds now (key -> line).
    pub binds: BTreeMap<String, String>,
    /// Console commands and cvars that exist (lower case), to tell the
    /// keyboard actions mashup has from those it lacks.
    pub known: BTreeSet<String>,
    /// The keyboard action waiting for a key.
    pub capture: Option<String>,
    /// The keyboard list's selected row (Edit key / Clear key act on it).
    pub key_row: Option<usize>,
    /// The first row of the open list shown (keyboard actions, maps).
    pub scroll: usize,
    /// Window sizes the video tab offers.
    pub resolutions: Vec<String>,
}

/// Map list rows shown at once.
pub const MAP_ROWS: usize = 16;
/// Keyboard list rows shown at once.
pub const KEY_ROWS: usize = 14;
/// Buttons under the keyboard list (Use defaults, Edit key, Clear key).
const KEY_BUTTONS: usize = 3;

/// The keyboard list without the game's `kb_act.lst`: what mashup does.
const OUR_ACTIONS: &[(&str, &str)] = &[
    ("", "Movement"),
    ("+forward", "Move forward"),
    ("+back", "Move back"),
    ("+moveleft", "Move left (strafe)"),
    ("+moveright", "Move right (strafe)"),
    ("+speed", "Walk"),
    ("+jump", "Jump"),
    ("+duck", "Duck"),
    ("", "Combat"),
    ("+attack", "Fire"),
    ("+attack2", "Weapon special function"),
    ("+reload", "Reload weapon"),
    ("lastinv", "Last weapon used"),
    ("drop", "Drop weapon"),
    ("", "Communication"),
    ("radio1", "Standard radio messages"),
    ("radio2", "Group radio messages"),
    ("radio3", "Report radio messages"),
    ("messagemode", "Chat message"),
    ("messagemode2", "Team message"),
    ("", "Menus"),
    ("buymenu", "Buy menu"),
    ("chooseteam", "Select team"),
    ("+showscores", "Display multiplayer scores"),
    ("slot1", "Menu item 1"),
    ("slot2", "Menu item 2"),
    ("slot3", "Menu item 3"),
    ("slot4", "Menu item 4"),
    ("slot5", "Menu item 5"),
    ("", "Miscellaneous"),
    ("+use", "Use items"),
    ("bug", "Report a bug"),
    ("+freelook", "Free look"),
];

impl GameMenu {
    /// Open on a page with the current settings: `get` reads a cvar,
    /// `current_map` is the loaded map's name.
    pub fn open(
        &mut self,
        page: Page,
        maps: Vec<String>,
        current_map: Option<&str>,
        get: impl Fn(&str) -> Option<String>,
    ) {
        self.open = true;
        self.page = page;
        self.focus = 0;
        self.capture = None;
        self.scroll = 0;
        if self.main.is_empty() {
            self.main = main_entries(self.ui.0.as_deref());
        }
        self.values = SETTINGS.iter().map(|s| get(s.cvar)).collect();
        let num = |name: &str| get(name).and_then(|v| v.trim().parse::<f32>().ok());
        self.new_game.map = current_map
            .and_then(|m| maps.iter().position(|n| n == m))
            .unwrap_or(0);
        self.maps = maps;
        self.new_game.mode = if num("mashup_rounds").unwrap_or(0.0) != 0.0 {
            Mode::Rounds
        } else {
            Mode::Deathmatch
        };
        // The preset nearest the bots' reaction now.
        if let Some(r) = num("bot_reaction") {
            self.new_game.difficulty = (0..DIFFICULTIES.len())
                .min_by(|&a, &b| {
                    (DIFFICULTIES[a].1 - r)
                        .abs()
                        .total_cmp(&(DIFFICULTIES[b].1 - r).abs())
                })
                .unwrap_or(NORMAL);
        }
        // As many bots as there are now (`set_counts` first).
        self.new_game.bots_t = self.bots[0].min(MAX_BOTS as usize) as u8;
        self.new_game.bots_ct = self.bots[1].min(MAX_BOTS as usize) as u8;
        if page == Page::Maps {
            self.focus = self.new_game.map;
        }
        self.focus = self.first_focusable_from(self.focus, 1);
        self.key_row = None;
        self.select_focused_key();
        self.keep_visible();
    }

    /// The game's look and words (None: ours).
    pub fn set_ui(&mut self, ui: Option<Arc<GameUi>>) {
        self.ui = UiText(ui);
        self.main = main_entries(self.ui.0.as_deref());
        self.focus = self.focus.min(self.main.len().saturating_sub(1));
    }

    /// The binds now and the commands that exist.
    pub fn set_binds(&mut self, binds: BTreeMap<String, String>, known: BTreeSet<String>) {
        self.binds = binds;
        self.known = known;
    }

    /// Show an options tab.
    pub fn set_tab(&mut self, tab: Tab) {
        self.tab = tab;
        self.capture = None;
        self.scroll = 0;
        self.key_row = None;
        self.focus = self.first_focusable_from(0, 1);
        self.select_focused_key();
        self.keep_visible();
    }

    /// The bots and players per team now (for the bots and team pages; a new
    /// game starts with as many bots).
    pub fn set_counts(&mut self, bots: [usize; 2], players: [usize; 2]) {
        self.bots = bots;
        self.players = players;
    }

    /// The game's text for a `#token`, else `ours`.
    pub fn text(&self, token: &str, ours: &str) -> String {
        self.ui
            .0
            .as_ref()
            .and_then(|u| u.string(token))
            .map_or_else(|| ours.to_string(), str::to_string)
    }

    fn game_text(&self, token: &str) -> Option<String> {
        self.ui.0.as_ref().and_then(|u| u.string(token)).map(str::to_string)
    }

    /// The keyboard list: the game's actions, then ours it lacks.
    pub fn key_actions(&self) -> Vec<KeyAction> {
        let ours = |command: &str, label: &str| KeyAction::Action {
            command: command.to_string(),
            label: label.to_string(),
        };
        let Some(ui) = self.ui.0.as_ref().filter(|u| !u.actions.is_empty()) else {
            return OUR_ACTIONS
                .iter()
                .map(|(c, l)| {
                    if c.is_empty() {
                        KeyAction::Section(l.to_string())
                    } else {
                        ours(c, l)
                    }
                })
                .collect();
        };
        let mut list = ui.actions.clone();
        let has = |c: &str| {
            list.iter()
                .any(|a| matches!(a, KeyAction::Action { command, .. } if command.eq_ignore_ascii_case(c)))
        };
        let missing: Vec<KeyAction> = OUR_ACTIONS
            .iter()
            .filter(|(c, _)| !c.is_empty() && !has(c))
            .map(|(c, l)| ours(c, l))
            .collect();
        if !missing.is_empty() {
            list.push(KeyAction::Section("mashup".into()));
            list.extend(missing);
        }
        list
    }

    /// Whether mashup has a keyboard action's command.
    fn known_command(&self, line: &str) -> bool {
        let first = line.split_whitespace().next().unwrap_or_default().to_lowercase();
        binds::is_polled(line) || self.known.contains(&first)
    }

    /// The left-hand entries shown: in a game all, at the main menu those
    /// not only for a game; gaps resolved likewise.
    pub fn entries(&self) -> Vec<MainEntry> {
        let all = if self.main.is_empty() {
            main_entries(self.ui.0.as_deref())
        } else {
            self.main.clone()
        };
        let mut shown: Vec<MainEntry> = Vec::new();
        for mut e in all.into_iter().filter(|e| self.in_game || !e.in_game_only) {
            e.gap = e.gap && !shown.is_empty() && (self.in_game || !e.gap_in_game_only);
            shown.push(e);
        }
        shown
    }

    /// A map is in: playing it. Closes the menu (the main menu's, or a
    /// load's); true if it was open.
    pub fn enter_game(&mut self) -> bool {
        self.in_game = true;
        self.loading = None;
        let was_open = self.open;
        self.open = false;
        self.capture = None;
        was_open
    }

    /// Out of the game: the main menu, on its first entry.
    pub fn leave_game(&mut self) {
        self.in_game = false;
        self.loading = None;
        self.failure = None;
        self.open = true;
        self.page = Page::Main;
        self.capture = None;
        self.scroll = 0;
        self.focus = self.first_focusable_from(0, 1);
    }

    /// Start the new game set up on the new game page: from a game, close
    /// (the old map plays until the new one is in); from the main menu,
    /// show it loading.
    fn start(&mut self, out: &mut Outcome) {
        let Some((lines, after)) = self.start_lines() else {
            return;
        };
        out.lines = lines;
        out.after_load = after;
        if self.in_game {
            self.close(out);
        } else {
            self.loading = self.maps.get(self.new_game.map).cloned();
        }
    }

    /// The quick start's settings on the new game page.
    fn quick_setup(&mut self) {
        let map = self.maps.iter().position(|m| m == QUICK_MAP).unwrap_or(0);
        self.new_game = NewGame {
            map,
            mode: Mode::Rounds,
            bots_t: QUICK_BOTS.0,
            bots_ct: QUICK_BOTS.1,
            difficulty: NORMAL,
        };
    }

    /// The rows of the open page (the left-hand entries on the main page).
    pub fn rows(&self) -> Vec<Row> {
        let button = |label: &str, action: Action| Row::Button {
            label: label.to_string(),
            action,
            enabled: true,
        };
        let value = |label: &str, value: String, field: Field| Row::Value {
            label: label.to_string(),
            value,
            field,
        };
        let ok = || button(&self.text("#GameUI_OK", "OK"), Action::Back);
        let ng = &self.new_game;
        match self.page {
            Page::Main => self
                .entries()
                .iter()
                .map(|e| Row::Button {
                    label: e.label.clone(),
                    action: Action::Main(e.item),
                    enabled: e.enabled,
                })
                .collect(),
            Page::NewGame => vec![
                value(
                    "Map",
                    self.maps.get(ng.map).cloned().unwrap_or_else(|| "(no maps found)".into()),
                    Field::Map,
                ),
                value(
                    "Mode",
                    match ng.mode {
                        Mode::Deathmatch => "Deathmatch",
                        Mode::Rounds => "Rounds",
                    }
                    .into(),
                    Field::Mode,
                ),
                value("Terrorist bots", ng.bots_t.to_string(), Field::BotsT),
                value("Counter-terrorist bots", ng.bots_ct.to_string(), Field::BotsCt),
                value("Bot difficulty", DIFFICULTIES[ng.difficulty].0.into(), Field::Difficulty),
                Row::Button {
                    label: "Start".into(),
                    action: Action::Start,
                    enabled: !self.maps.is_empty(),
                },
                button(&self.text("#GameUI_Cancel", "Back"), Action::Back),
            ],
            Page::Maps => {
                let mut rows: Vec<Row> = self
                    .maps
                    .iter()
                    .enumerate()
                    .map(|(i, m)| button(m, Action::PickMap(i)))
                    .collect();
                rows.push(button(&self.text("#GameUI_Cancel", "Back"), Action::Back));
                rows
            }
            Page::Bots => vec![
                Row::Info(format!(
                    "In the game: {} terrorist and {} counter-terrorist bots",
                    self.bots[0], self.bots[1]
                )),
                button("Add a terrorist bot", Action::Run("bot_add 1".into(), false)),
                button("Add a counter-terrorist bot", Action::Run("bot_add 2".into(), false)),
                button("Kick all bots", Action::Run("bot_kick".into(), false)),
                ok(),
            ],
            Page::Team => vec![
                button("Terrorists", Action::Run("jointeam 2".into(), true)),
                button("Counter-Terrorists", Action::Run("jointeam 3".into(), true)),
                button(
                    "Auto-assign",
                    Action::Run(
                        format!(
                            "jointeam {}",
                            super::team_menu::auto_team(self.players[0], self.players[1])
                        ),
                        true,
                    ),
                ),
                button(&self.text("#GameUI_Cancel", "Back"), Action::Back),
            ],
            Page::Settings if self.tab == Tab::Keyboard => {
                let mut rows: Vec<Row> = self
                    .key_actions()
                    .into_iter()
                    .map(|a| match a {
                        KeyAction::Section(t) => Row::Heading(t),
                        KeyAction::Action { command, label } => Row::Bind {
                            keys: binds::keys_for(&self.binds, &command)
                                .iter()
                                .map(|k| binds::display(k))
                                .collect::<Vec<_>>()
                                .join(", "),
                            known: self.known_command(&command),
                            label,
                            command,
                        },
                    })
                    .collect();
                let selected = self.key_row.is_some_and(|r| matches!(rows.get(r), Some(Row::Bind { .. })));
                rows.push(button(&self.text("#GameUI_UseDefaults", "Use Defaults"), Action::DefaultBinds));
                rows.push(Row::Button {
                    label: self.text("#GameUI_SetNewKey", "Edit key"),
                    action: Action::EditKey,
                    enabled: selected,
                });
                rows.push(Row::Button {
                    label: self.text("#GameUI_ClearKey", "Clear Key"),
                    action: Action::ClearKey,
                    enabled: selected,
                });
                rows.push(ok());
                rows
            }
            Page::Settings => {
                let text = |t: &str| self.game_text(t);
                let mut rows: Vec<Row> = SETTINGS
                    .iter()
                    .enumerate()
                    .filter(|(_, s)| s.tab == self.tab)
                    .map(|(i, s)| {
                        let label = s.token.and_then(|t| self.game_text(t)).unwrap_or_else(|| s.label.to_string());
                        match self.values.get(i).cloned().flatten() {
                            Some(v) => value(&label, s.show(&v, &text), Field::Setting(i)),
                            None => Row::Info(format!("{label}: not available")),
                        }
                    })
                    .collect();
                rows.push(ok());
                rows
            }
        }
    }

    /// Apply an input; the console lines it produces.
    pub fn handle(&mut self, input: Input) -> Outcome {
        let mut out = Outcome::default();
        if self.open && self.failure.is_some() {
            // The failure's dialog: Close (its button, Esc, Enter) leaves
            // the main menu.
            if matches!(input, Input::Click(Target::Cancel, _) | Input::Close | Input::Activate) {
                self.failure = None;
                self.loading = None;
            }
            return out;
        }
        if self.open && self.loading.is_some() && matches!(input, Input::Click(Target::Cancel, _)) {
            // Stop the load: back at the main menu (`disconnect` drops it).
            self.loading = None;
            out.lines.push("disconnect".into());
            return out;
        }
        if !self.open || self.loading.is_some() {
            return out;
        }
        if let Some(command) = self.capture.clone() {
            // Waiting for a key: it, or Esc to give up.
            match input {
                Input::BindKey(key) => {
                    let line = binds::rebind_line(&self.binds, &command, key);
                    self.binds.retain(|_, v| !v.trim().eq_ignore_ascii_case(command.trim()));
                    self.binds.insert(key.to_string(), command);
                    out.lines.push(line);
                    self.capture = None;
                }
                Input::Close => self.capture = None,
                _ => {}
            }
            return out;
        }
        let rows = self.rows();
        match input {
            // The main menu stays: Esc only closes its dialogs.
            Input::Close if !self.in_game => self.back(),
            Input::Close => self.close(&mut out),
            Input::Back => self.back(),
            Input::Up => self.focus = self.first_focusable_from(self.focus + rows.len() - 1, -1),
            Input::Down => self.focus = self.first_focusable_from(self.focus + 1, 1),
            Input::Left | Input::Right => {
                let dir = if input == Input::Left { -1 } else { 1 };
                if self.page == Page::Maps {
                    // A screenful at a time.
                    let n = self.maps.len() as i32;
                    if self.focus < self.maps.len() && n > 0 {
                        self.focus = (self.focus as i32 + dir * MAP_ROWS as i32).clamp(0, n - 1) as usize;
                    }
                } else if let Some(Row::Value { field, .. }) = rows.get(self.focus) {
                    self.change(*field, dir, &mut out);
                }
            }
            Input::Activate => self.activate(self.focus, 0, &mut out),
            Input::Hover(Target::Main(i)) => {
                if self.page == Page::Main && rows.get(i).is_some_and(Row::focusable) {
                    self.focus = i;
                }
            }
            Input::Hover(Target::Row(i)) => {
                // Lists select by click, not by hover.
                let list = matches!(rows.get(i), Some(Row::Bind { .. }));
                if self.page != Page::Main && !list && rows.get(i).is_some_and(Row::focusable) {
                    self.focus = i;
                }
            }
            Input::Hover(_) => {}
            Input::Click(Target::Main(i), _) => {
                if let Some(e) = self.entries().get(i).filter(|e| e.enabled) {
                    if self.page == Page::Main {
                        self.focus = i;
                    }
                    self.main(e.item, &mut out);
                }
            }
            Input::Click(Target::Row(i), step) => {
                if self.page != Page::Main && rows.get(i).is_some_and(Row::focusable) {
                    // A list row: the first click selects, the next edits.
                    if matches!(rows[i], Row::Bind { .. }) && self.key_row != Some(i) {
                        self.focus = i;
                        self.key_row = Some(i);
                    } else {
                        self.focus = i;
                        self.activate(i, step, &mut out);
                    }
                }
            }
            Input::Click(Target::Tab(i), _) => {
                if self.page == Page::Settings
                    && let Some((tab, ..)) = TABS.get(i)
                {
                    self.set_tab(*tab);
                }
            }
            // Only while loading (above).
            Input::Click(Target::Cancel, _) => {}
            Input::Click(Target::Scroll, step) | Input::Scroll(step) => {
                let (len, shown) = self.list();
                self.scroll = (self.scroll as i32 + step).clamp(0, len.saturating_sub(shown) as i32) as usize;
                return out;
            }
            Input::NextTab(dir) => {
                if self.page == Page::Settings {
                    self.set_tab(self.tab.step(dir));
                }
            }
            Input::Slide(i, f) => {
                if let Some(Row::Value {
                    field: Field::Setting(s),
                    ..
                }) = rows.get(i)
                    && let Some(v) = SETTINGS[*s].at_fraction(f)
                {
                    self.focus = i;
                    self.set_value(*s, v, &mut out);
                }
            }
            Input::BindKey(_) => {}
            Input::Char(c) => {
                // Jump to the next map starting with it.
                let on_map = self.page == Page::Maps
                    || (self.page == Page::NewGame && matches!(rows.get(self.focus), Some(Row::Value { field: Field::Map, .. })));
                if on_map && !self.maps.is_empty() {
                    let c = c.to_ascii_lowercase();
                    let from = if self.page == Page::Maps {
                        self.focus
                    } else {
                        self.new_game.map
                    };
                    let n = self.maps.len();
                    if let Some(i) = (1..=n)
                        .map(|k| (from + k) % n)
                        .find(|&i| self.maps[i].to_ascii_lowercase().starts_with(c))
                    {
                        if self.page == Page::Maps {
                            self.focus = i;
                        } else {
                            self.new_game.map = i;
                        }
                    }
                }
            }
        }
        self.select_focused_key();
        self.keep_visible();
        out
    }

    /// The focused keyboard action is the selected one.
    fn select_focused_key(&mut self) {
        if self.page == Page::Settings
            && self.tab == Tab::Keyboard
            && matches!(self.rows().get(self.focus), Some(Row::Bind { .. }))
        {
            self.key_row = Some(self.focus);
        }
    }

    /// The open list's length and rows shown (0, 0 without one).
    pub fn list(&self) -> (usize, usize) {
        match self.page {
            Page::Maps => (self.maps.len(), MAP_ROWS),
            Page::Settings if self.tab == Tab::Keyboard => (self.rows().len() - KEY_BUTTONS - 1, KEY_ROWS),
            _ => (0, 0),
        }
    }

    /// Scroll the open list so the focused row shows.
    fn keep_visible(&mut self) {
        let (len, shown) = self.list();
        if shown == 0 {
            self.scroll = 0;
            return;
        }
        if self.focus < len {
            if self.focus < self.scroll {
                self.scroll = self.focus;
            } else if self.focus >= self.scroll + shown {
                self.scroll = self.focus + 1 - shown;
            }
        }
        self.scroll = self.scroll.min(len.saturating_sub(shown));
    }

    fn close(&mut self, out: &mut Outcome) {
        self.open = false;
        self.capture = None;
        out.close = true;
    }

    /// Back to the main page, its entry focused (to the new game page from
    /// the map list).
    fn back(&mut self) {
        self.capture = None;
        self.scroll = 0;
        match self.page {
            Page::Main => {}
            Page::Maps => {
                self.page = Page::NewGame;
                self.focus = 0;
            }
            page => {
                self.focus = self.entries().iter().position(|e| e.item.page() == Some(page)).unwrap_or(0);
                self.page = Page::Main;
            }
        }
    }

    fn main(&mut self, item: MainItem, out: &mut Outcome) {
        if let Some(page) = item.page() {
            self.page = page;
            self.scroll = 0;
            self.key_row = None;
            self.focus = self.first_focusable_from(0, 1);
            self.select_focused_key();
            return;
        }
        match item {
            MainItem::Resume => self.close(out),
            MainItem::Disconnect => {
                out.lines.push("disconnect".into());
                self.leave_game();
            }
            MainItem::BugReport => {
                // Closed first in a game, so the screenshot shows it.
                if self.in_game {
                    self.close(out);
                }
                out.lines.push("bugreport".into());
            }
            MainItem::Quit => out.lines.push("quit".into()),
            MainItem::QuickStart => {
                self.quick_setup();
                self.start(out);
            }
            MainItem::Greybox => {
                out.lines.push(format!("map {}", super::console::GREYBOX));
                if self.in_game {
                    self.close(out);
                } else {
                    self.loading = Some(super::console::GREYBOX.into());
                }
            }
            MainItem::Console => out.lines.push("toggleconsole".into()),
            MainItem::FindServers => out.lines.push("openserverbrowser".into()),
            MainItem::NewGame | MainItem::Bots | MainItem::Team | MainItem::Options => {}
        }
    }

    /// Press row `i` (`step` -1/+1: its arrows).
    fn activate(&mut self, i: usize, step: i32, out: &mut Outcome) {
        let Some(row) = self.rows().into_iter().nth(i) else {
            return;
        };
        match row {
            Row::Button { enabled: false, .. } | Row::Info(_) | Row::Heading(_) => {}
            Row::Bind { command, .. } => {
                self.key_row = Some(i);
                self.capture = Some(command);
            }
            Row::Value { field, .. } => {
                if step == 0 && field == Field::Map && !self.maps.is_empty() {
                    self.page = Page::Maps;
                    self.focus = self.new_game.map;
                } else {
                    self.change(field, if step == 0 { 1 } else { step }, out);
                }
            }
            Row::Button { action, .. } => match action {
                Action::Main(item) => self.main(item, out),
                Action::Back => self.back(),
                Action::PickMap(m) => {
                    self.new_game.map = m;
                    self.page = Page::NewGame;
                    self.focus = 0;
                }
                Action::Run(line, close) => {
                    out.lines.push(line);
                    if close {
                        self.close(out);
                    }
                }
                Action::Start => self.start(out),
                Action::EditKey => {
                    if let Some(Row::Bind { command, .. }) = self.key_row.and_then(|r| self.rows().into_iter().nth(r)) {
                        self.capture = Some(command);
                    }
                }
                Action::ClearKey => {
                    if let Some(Row::Bind { command, .. }) = self.key_row.and_then(|r| self.rows().into_iter().nth(r))
                        && let Some(line) = binds::clear_line(&self.binds, &command)
                    {
                        self.binds.retain(|_, v| !v.trim().eq_ignore_ascii_case(command.trim()));
                        out.lines.push(line);
                    }
                }
                Action::DefaultBinds => {
                    binds::bind_defaults(&mut self.binds, true);
                    out.lines.push("binddefaults".into());
                }
            },
        }
    }

    /// Step a value; settings apply at once.
    fn change(&mut self, field: Field, dir: i32, out: &mut Outcome) {
        let ng = &mut self.new_game;
        let bots = |n: u8| (n as i32 + dir).clamp(0, MAX_BOTS as i32) as u8;
        match field {
            Field::Map => {
                if !self.maps.is_empty() {
                    ng.map = (ng.map as i32 + dir).rem_euclid(self.maps.len() as i32) as usize;
                }
            }
            Field::Mode => {
                ng.mode = match ng.mode {
                    Mode::Deathmatch => Mode::Rounds,
                    Mode::Rounds => Mode::Deathmatch,
                }
            }
            Field::BotsT => ng.bots_t = bots(ng.bots_t),
            Field::BotsCt => ng.bots_ct = bots(ng.bots_ct),
            Field::Difficulty => {
                ng.difficulty = (ng.difficulty as i32 + dir).clamp(0, DIFFICULTIES.len() as i32 - 1) as usize
            }
            Field::Setting(i) => {
                let (Some(setting), Some(Some(current))) = (SETTINGS.get(i), self.values.get(i)) else {
                    return;
                };
                let next = setting.step(current, dir, &self.resolutions);
                self.set_value(i, next, out);
            }
        }
    }

    /// Set setting `i` to `next` (a line when that changes what it shows).
    fn set_value(&mut self, i: usize, next: String, out: &mut Outcome) {
        let (Some(setting), Some(Some(current))) = (SETTINGS.get(i), self.values.get(i)) else {
            return;
        };
        let none = |_: &str| None;
        let changed = match setting.kind {
            SettingKind::Negate => next.trim() != current.trim(),
            _ => setting.show(&next, &none) != setting.show(current, &none),
        };
        if changed {
            out.lines.push(format!("{} {}", setting.cvar, crate::console::quote(&next)));
            self.values[i] = Some(next);
        }
    }

    /// A new game's console lines: settings and the map now, the bots once
    /// the map is in (they spawn at its spawn points).
    pub fn start_lines(&self) -> Option<(Vec<String>, Vec<String>)> {
        let ng = &self.new_game;
        let map = self.maps.get(ng.map)?;
        let (_, reaction, aim, turn) = DIFFICULTIES[ng.difficulty];
        let lines = vec![
            "bot_kick".to_string(),
            format!("mashup_rounds {}", (ng.mode == Mode::Rounds) as u8),
            format!("bot_reaction {reaction}"),
            format!("bot_aim_error {aim}"),
            format!("bot_turn_rate {turn}"),
            format!("map {}", crate::console::quote(map)),
        ];
        let after = std::iter::repeat_n("bot_add 1".to_string(), ng.bots_t as usize)
            .chain(std::iter::repeat_n("bot_add 2".to_string(), ng.bots_ct as usize))
            .collect();
        Some((lines, after))
    }

    /// The first focusable row from `i` on, stepping by `dir` (wrapping).
    fn first_focusable_from(&self, i: usize, dir: i32) -> usize {
        let rows = self.rows();
        let n = rows.len();
        if n == 0 {
            return 0;
        }
        (0..n)
            .map(|k| (i as i32 + dir * k as i32).rem_euclid(n as i32) as usize)
            .find(|&r| rows[r].focusable())
            .unwrap_or(0)
    }
}

// ---------------------------------------------------------------------------
// The client systems.

/// Lines waiting for a map load the menu started (`map` loads in the
/// background, `console::finish_map_load` swaps it in).
#[derive(Resource, Default)]
struct AfterLoad {
    lines: Vec<String>,
    /// Frames waited for the load to start; it started once seen.
    waited: u32,
    started: bool,
}

/// The menu closed: grab the mouse again once the button that closed it is
/// up (else the click would fire the gun).
#[derive(Resource, Default)]
struct RegrabCursor(bool);

/// The install's GameUI look, read in the background at startup, and its
/// map thumbnails as images.
#[derive(Resource, Default)]
struct MenuUi {
    loading: Option<Arc<Mutex<Option<Option<GameUi>>>>>,
    ui: Option<Arc<GameUi>>,
    /// By map: the picture and its height over its width.
    thumbs: HashMap<String, (Handle<Image>, f32)>,
    /// The main menu's backgrounds: 4:3, widescreen.
    backgrounds: [Option<Handle<Image>>; 2],
    /// The title's font, its height in scheme pixels and its line height
    /// in ems.
    title_font: Option<(Handle<Font>, f32, f32)>,
}

impl MenuUi {
    /// The main menu background for a screen of this size.
    fn background(&self, size: Vec2) -> Option<Handle<Image>> {
        let ui = self.ui.as_ref()?;
        let pic = ui.background_for(size.x / size.y.max(1.0))?;
        let i = ui.background_wide.as_ref().is_some_and(|w| std::ptr::eq(w, pic)) as usize;
        self.backgrounds[i].clone()
    }
}

/// A decoded picture as a UI image.
pub(super) fn ui_image(pic: &crate::map::hud::UiImage, images: &mut Assets<Image>) -> Handle<Image> {
    use bevy::{
        asset::RenderAssetUsages,
        render::render_resource::{Extent3d, TextureDimension, TextureFormat},
    };
    images.add(Image::new(
        Extent3d {
            width: pic.width,
            height: pic.height,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        pic.rgba8.clone(),
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::RENDER_WORLD,
    ))
}

/// Read the GameUI files on a thread (the install's archives).
fn start_loading_ui(mut ui: ResMut<MenuUi>) {
    let slot = Arc::new(Mutex::new(None));
    ui.loading = Some(slot.clone());
    std::thread::spawn(move || {
        let loaded = crate::mount::config::LocalConfig::load()
            .ok()
            .and_then(|c| c.game_path(crate::games::cs_source::GAME))
            .and_then(|p| crate::games::cs_source::mount::open(&p).ok())
            .and_then(|m| crate::games::cs_source::gameui::load(&m));
        if let Ok(mut s) = slot.lock() {
            *s = Some(loaded);
        }
    });
}

/// Take the GameUI look once read.
fn ui_loaded(
    mut ui: ResMut<MenuUi>,
    mut menu: ResMut<GameMenu>,
    mut images: ResMut<Assets<Image>>,
    mut fonts: ResMut<Assets<Font>>,
    mut ui_fonts: ResMut<UiFonts>,
) {
    let Some(slot) = ui.loading.clone() else { return };
    let Some(loaded) = slot.lock().ok().and_then(|mut s| s.take()) else {
        return;
    };
    ui.loading = None;
    let Some(game_ui) = loaded else {
        info!("game menu: no GameUI files in the install, built-in look");
        return;
    };
    for (map, pic) in &game_ui.thumbnails {
        let handle = ui_image(pic, &mut images);
        ui.thumbs
            .insert(map.clone(), (handle, pic.height as f32 / pic.width.max(1) as f32));
    }
    ui.backgrounds = [&game_ui.background, &game_ui.background_wide].map(|p| p.as_ref().map(|p| ui_image(p, &mut images)));
    ui.title_font = game_ui
        .title_font
        .as_ref()
        .map(|(bytes, tall)| {
            let line_per_em = super::fonts::line_per_em(bytes).unwrap_or(1.2);
            (fonts.add(Font::from_bytes(bytes.to_vec())), *tall, line_per_em)
        });
    ui_fonts.set_source(&game_ui);
    info!(
        "game menu: GameUI look ({} entries, {} keyboard actions, {} option pages, {} map thumbnails, \
         {} main menu backgrounds, title {:?}{})",
        game_ui.menu.len(),
        game_ui.actions.len(),
        game_ui.options.len(),
        game_ui.thumbnails.len(),
        ui.backgrounds.iter().flatten().count(),
        game_ui.title,
        if ui.title_font.is_some() { " in its font" } else { "" },
    );
    let game_ui = Arc::new(game_ui);
    ui.ui = Some(game_ui.clone());
    menu.set_ui(Some(game_ui));
}

/// Open the menu on a page, reading the settings from the console.
fn open_menu(w: &mut World, page: Page) {
    let (get, binds, known) = {
        let cvars: Vec<_> = {
            let console = w.resource::<Console>();
            SETTINGS
                .iter()
                .map(|s| s.cvar)
                .chain(["mashup_rounds", "bot_reaction"])
                .filter_map(|n| console.cvar(n).cloned())
                .collect()
        };
        let console = w.resource::<Console>();
        let binds = console.binds.clone();
        let known: BTreeSet<String> = console.names().into_iter().map(|n| n.to_lowercase()).collect();
        let values = cvars
            .into_iter()
            .filter_map(|c| (c.get)(w).map(|v| (c.name.clone(), v)))
            .collect::<HashMap<_, _>>();
        (values, binds, known)
    };
    let current = w
        .get_resource::<crate::map::LoadedMapName>()
        .map(|m| m.0.rsplit(':').next().unwrap_or(&m.0).to_string());
    let maps = super::console::map_names();
    let (mut bots, mut players) = ([0; 2], [0; 2]);
    let mut q = w.query_filtered::<(&Team, Has<crate::bot::Bot>), With<Intent>>();
    for (team, bot) in q.iter(w) {
        if let Some(i) = [1, 2].iter().position(|t| *t == team.0) {
            players[i] += 1;
            bots[i] += bot as usize;
        }
    }
    let mut monitors = w.query_filtered::<&bevy::window::Monitor, With<bevy::window::PrimaryMonitor>>();
    let modes: Vec<(UVec2, Vec<UVec2>)> = monitors
        .iter(w)
        .map(|m| {
            (
                UVec2::new(m.physical_width, m.physical_height),
                m.video_modes.iter().map(|v| v.physical_size).collect(),
            )
        })
        .collect();
    let mut menu = w.resource_mut::<GameMenu>();
    menu.set_counts(bots, players);
    menu.set_binds(binds, known);
    menu.resolutions = super::options::resolutions(&modes);
    menu.open(page, maps, current.as_deref(), |n| get.get(n).cloned());
    // Menus close each other.
    if let Some(mut b) = w.get_resource_mut::<super::buy_menu::BuyMenu>() {
        *b = default();
    }
    if let Some(mut t) = w.get_resource_mut::<super::team_menu::TeamMenu>() {
        t.0 = false;
    }
    if let Some(mut r) = w.get_resource_mut::<super::radio::RadioMenu>() {
        r.0 = None;
    }
    w.resource_mut::<RegrabCursor>().0 = false;
}

/// The menu open (on its main page unless it is open already): for the
/// server browser, which shows over it.
pub(super) fn open_main(w: &mut World) {
    if !w.resource::<GameMenu>().open {
        open_menu(w, Page::Main);
    }
}

/// A map is in (`map`, `map greybox`): playing it, the menu closed.
pub(super) fn entered_game(w: &mut World) {
    let Some(mut menu) = w.get_resource_mut::<GameMenu>() else { return };
    if menu.enter_game()
        && let Some(mut regrab) = w.get_resource_mut::<RegrabCursor>()
    {
        regrab.0 = true;
    }
}

/// Out of the game (`disconnect`): the main menu.
pub(super) fn left_game(w: &mut World) {
    let Some(mut menu) = w.get_resource_mut::<GameMenu>() else { return };
    menu.in_game = false;
    menu.loading = None;
    menu.failure = None;
    open_menu(w, Page::Main);
}

/// Joining a server, or following its map change (`client::net`): the
/// loading dialog over the main menu's background (`label`: the address,
/// then the map), as the game shows it, until the map is in
/// (`entered_game`) or it fails (`show_failure`).
pub(super) fn joining(w: &mut World, label: &str) {
    let Some(mut menu) = w.get_resource_mut::<GameMenu>() else { return };
    if menu.loading.as_deref() == Some(label) && menu.open && menu.failure.is_none() {
        return;
    }
    menu.loading = Some(label.to_string());
    menu.failure = None;
    menu.in_game = false;
    menu.open = true;
    menu.page = Page::Main;
    menu.capture = None;
}

/// Joining (or the game) failed: the loading dialog says why, in the
/// game's words, until it's closed.
pub(super) fn show_failure(w: &mut World) {
    let Some(failure) = w
        .get_resource::<crate::net::client::JoinProgress>()
        .and_then(|p| p.failure.clone())
    else {
        return;
    };
    let Some(mut menu) = w.get_resource_mut::<GameMenu>() else { return };
    let text = net_failure_text(&menu, &failure.0, &failure.1);
    menu.failure = Some(text);
    menu.loading = Some(String::new());
    menu.open = true;
    menu.page = Page::Main;
}

/// Why joining failed, as the game words it (its GameUI strings, `%s1`
/// filled in), else the server's or our own words.
pub fn net_failure_text(menu: &GameMenu, kind: &crate::net::client::JoinFailure, reason: &str) -> String {
    use crate::net::client::JoinFailure as F;
    let game = |token: &str| menu.game_text(token).map(|t| t.trim_end().to_string());
    let text = match kind {
        F::Full => game("#GameUI_ServerRejectServerFull"),
        F::OldServer => game("#GameUI_ServerRejectOldVersion"),
        F::NewServer => game("#GameUI_ServerRejectNewVersion"),
        F::Timeout => game("#GameUI_ServerConnectionTimeout"),
        F::Download { file, error } => {
            let token = match error.as_str() {
                "File does not exist" => "#GameUI_DownloadFailedFileNotFound",
                "Connection closed by remote host" => "#GameUI_DownloadFailedConClosed",
                "Invalid URL" => "#GameUI_DownloadFailedBadURL",
                "Only HTTP is supported" => "#GameUI_DownloadFailedBadProtocol",
                "Cannot get file info from server" => "#GameUI_DownloadFailedNoHeaders",
                "File has no data" => "#GameUI_DownloadFailedZeroLen",
                e if e.starts_with("Cannot connect to server") => "#GameUI_DownloadFailedCantConnect",
                _ => "",
            };
            let generic = game("#GameUI_DownloadFailed").map(|t| format!("{}:\n{error}", t.replace("%s1", file)));
            if token.is_empty() {
                generic
            } else {
                game(token).map(|t| t.replace("%s1", file)).or(generic)
            }
        }
        F::Dropped => game("#GameUI_DisconnectedFromServerExtended").map(|t| t.replace("%s1", reason)),
        _ => None,
    };
    text.unwrap_or_else(|| reason.to_string())
}

/// A `map` load failed: the main menu takes input again.
pub(super) fn map_load_failed(w: &mut World) {
    if let Some(mut menu) = w.get_resource_mut::<GameMenu>() {
        menu.loading = None;
    }
}

/// At startup: playing when the command line gives a map or places the
/// player (`Args::starts_in_game`), else the main menu.
fn start_session(args: Option<Res<super::ClientArgs>>, mut menu: ResMut<GameMenu>, mut commands: Commands) {
    menu.in_game = args.is_some_and(|a| a.0.starts_in_game());
    if !menu.in_game {
        commands.queue(|w: &mut World| open_menu(w, Page::Main));
    }
}

/// Run an input through the model; apply what it asks for.
fn apply(input: Input, menu: &mut ResMut<GameMenu>, console: &mut Console, after: &mut AfterLoad, regrab: &mut RegrabCursor) {
    let mut next = (**menu).clone();
    let out = next.handle(input);
    if next != **menu {
        **menu = next;
    }
    for line in out.lines {
        console.submit(line);
    }
    if !out.after_load.is_empty() {
        *after = AfterLoad {
            lines: out.after_load,
            ..default()
        };
    }
    if out.close {
        regrab.0 = true;
    }
}

const LETTERS: [(KeyCode, char); 36] = [
    (KeyCode::KeyA, 'a'),
    (KeyCode::KeyB, 'b'),
    (KeyCode::KeyC, 'c'),
    (KeyCode::KeyD, 'd'),
    (KeyCode::KeyE, 'e'),
    (KeyCode::KeyF, 'f'),
    (KeyCode::KeyG, 'g'),
    (KeyCode::KeyH, 'h'),
    (KeyCode::KeyI, 'i'),
    (KeyCode::KeyJ, 'j'),
    (KeyCode::KeyK, 'k'),
    (KeyCode::KeyL, 'l'),
    (KeyCode::KeyM, 'm'),
    (KeyCode::KeyN, 'n'),
    (KeyCode::KeyO, 'o'),
    (KeyCode::KeyP, 'p'),
    (KeyCode::KeyQ, 'q'),
    (KeyCode::KeyR, 'r'),
    (KeyCode::KeyS, 's'),
    (KeyCode::KeyT, 't'),
    (KeyCode::KeyU, 'u'),
    (KeyCode::KeyV, 'v'),
    (KeyCode::KeyW, 'w'),
    (KeyCode::KeyX, 'x'),
    (KeyCode::KeyY, 'y'),
    (KeyCode::KeyZ, 'z'),
    (KeyCode::Digit0, '0'),
    (KeyCode::Digit1, '1'),
    (KeyCode::Digit2, '2'),
    (KeyCode::Digit3, '3'),
    (KeyCode::Digit4, '4'),
    (KeyCode::Digit5, '5'),
    (KeyCode::Digit6, '6'),
    (KeyCode::Digit7, '7'),
    (KeyCode::Digit8, '8'),
    (KeyCode::Digit9, '9'),
];

/// Esc opens and closes the menu; arrows, Enter, Space, Backspace, Tab and
/// letters drive it; while a keyboard action waits for a key, the next
/// key, button or wheel notch is its new key. Runs before the console's
/// toggle, so the Esc that closes the console doesn't open the menu.
#[allow(clippy::too_many_arguments)]
fn keys(
    keys: Res<ButtonInput<KeyCode>>,
    (mouse, scroll): (Option<Res<ButtonInput<MouseButton>>>, Option<Res<AccumulatedMouseScroll>>),
    ui: Res<super::console::ConsoleUi>,
    mut menu: ResMut<GameMenu>,
    mut console: ResMut<Console>,
    mut after: ResMut<AfterLoad>,
    mut regrab: ResMut<RegrabCursor>,
    mut commands: Commands,
    browser: Option<Res<super::server_browser::ServerBrowser>>,
) {
    // The server browser over the menu takes the keys.
    if ui.open || browser.is_some_and(|b| b.open) {
        return;
    }
    if !menu.open {
        if keys.just_pressed(KeyCode::Escape) && menu.loading.is_none() {
            commands.queue(|w: &mut World| open_menu(w, Page::Main));
        }
        return;
    }
    if menu.capture.is_some() {
        let input = if keys.just_pressed(KeyCode::Escape) {
            Some(Input::Close)
        } else {
            let none = ButtonInput::<MouseButton>::default();
            binds::first_pressed(&keys, mouse.as_deref().unwrap_or(&none), scroll.as_deref()).map(Input::BindKey)
        };
        if let Some(input) = input {
            apply(input, &mut menu, &mut console, &mut after, &mut regrab);
        }
        return;
    }
    let shift = keys.pressed(KeyCode::ShiftLeft) || keys.pressed(KeyCode::ShiftRight);
    let mut inputs = Vec::new();
    for (key, input) in [
        (KeyCode::Escape, Input::Close),
        (KeyCode::ArrowUp, Input::Up),
        (KeyCode::ArrowDown, Input::Down),
        (KeyCode::ArrowLeft, Input::Left),
        (KeyCode::ArrowRight, Input::Right),
        (KeyCode::Enter, Input::Activate),
        (KeyCode::NumpadEnter, Input::Activate),
        (KeyCode::Space, Input::Activate),
        (KeyCode::Backspace, Input::Back),
        (KeyCode::Tab, Input::NextTab(if shift { -1 } else { 1 })),
        (KeyCode::PageUp, Input::Scroll(-(KEY_ROWS as i32))),
        (KeyCode::PageDown, Input::Scroll(KEY_ROWS as i32)),
    ] {
        if keys.just_pressed(key) {
            inputs.push(input);
        }
    }
    inputs.extend(LETTERS.iter().filter(|(k, _)| keys.just_pressed(*k)).map(|(_, c)| Input::Char(*c)));
    for input in inputs {
        apply(input, &mut menu, &mut console, &mut after, &mut regrab);
    }
}

/// What a UI node stands for: a row (step 0) or one of its arrows.
#[derive(Component, Clone, Copy)]
struct Hit(Target, i32);

/// A slider's track: pressing or dragging along it sets its row's value.
#[derive(Component, Clone, Copy)]
struct SliderTrack(usize);

/// A list the wheel scrolls.
#[derive(Component)]
struct WheelList;

/// Hovering focuses, clicking presses, the wheel scrolls a list under the
/// mouse, a pressed slider follows the mouse.
#[allow(clippy::too_many_arguments)]
fn pointer(
    hits: Query<(&Interaction, &Hit), Changed<Interaction>>,
    sliders: Query<(&Interaction, &SliderTrack, &RelativeCursorPosition)>,
    lists: Query<&RelativeCursorPosition, With<WheelList>>,
    scroll: Option<Res<AccumulatedMouseScroll>>,
    mut menu: ResMut<GameMenu>,
    mut console: ResMut<Console>,
    mut after: ResMut<AfterLoad>,
    mut regrab: ResMut<RegrabCursor>,
    browser: Option<Res<super::server_browser::ServerBrowser>>,
) {
    if !menu.open || menu.capture.is_some() || browser.is_some_and(|b| b.open) {
        return;
    }
    let mut inputs: Vec<Input> = hits
        .iter()
        .filter_map(|(i, h)| match i {
            Interaction::Hovered => Some(Input::Hover(h.0)),
            Interaction::Pressed => Some(Input::Click(h.0, h.1)),
            Interaction::None => None,
        })
        .collect();
    for (i, track, at) in &sliders {
        if *i == Interaction::Pressed
            && let Some(p) = at.normalized
        {
            inputs.push(Input::Slide(track.0, p.x + 0.5));
        }
    }
    if let Some(scroll) = scroll
        && scroll.delta.y != 0.0
        && lists.iter().any(|l| l.cursor_over())
    {
        let notches = match scroll.unit {
            MouseScrollUnit::Line => scroll.delta.y.round(),
            MouseScrollUnit::Pixel => (scroll.delta.y / 40.0).round(),
        } as i32;
        inputs.push(Input::Scroll(-notches * 3));
    }
    for input in inputs {
        apply(input, &mut menu, &mut console, &mut after, &mut regrab);
    }
}

/// Bots and players per team (bots and team pages), and the binds (the
/// keyboard tab), while open.
fn sync(
    mut menu: ResMut<GameMenu>,
    console: Res<Console>,
    characters: Query<(&Team, Has<crate::bot::Bot>), With<Intent>>,
) {
    if !menu.open {
        return;
    }
    let (mut bots, mut players) = ([0; 2], [0; 2]);
    for (team, bot) in &characters {
        let Some(i) = [1, 2].iter().position(|t| *t == team.0) else {
            continue;
        };
        players[i] += 1;
        if bot {
            bots[i] += 1;
        }
    }
    if menu.bots != bots || menu.players != players {
        menu.set_counts(bots, players);
    }
    if menu.binds != console.binds {
        menu.binds = console.binds.clone();
    }
}

/// The mouse is free while the menu is open (whoever grabbed it); once
/// closed, grabbed again when the mouse button is up.
fn cursor(
    menu: Res<GameMenu>,
    mut regrab: ResMut<RegrabCursor>,
    mouse: Res<ButtonInput<MouseButton>>,
    ui: Res<super::console::ConsoleUi>,
    cursor: Option<Single<&mut CursorOptions>>,
) {
    let Some(mut cursor) = cursor else { return };
    if menu.open {
        if super::input::cursor_grabbed(&cursor) {
            super::input::release_cursor(&mut cursor);
        }
    } else if regrab.0 && !mouse.pressed(MouseButton::Left) {
        regrab.0 = false;
        if !ui.open {
            super::input::capture_cursor(&mut cursor);
        }
    }
}

/// Run a new game's bot lines once its map is in (or if the load never
/// started: the `map` line failed).
fn after_load(w: &mut World) {
    // A load started at the main menu (a menu, the console): its dialog.
    // A server's own map change too (its players load it as well): the
    // dialog over the game, as theirs (single player keeps playing the
    // old map until the new one is in).
    let serving = w.get_resource::<crate::core::NetRole>() == Some(&crate::core::NetRole::Server);
    if let Some(map) = super::console::loading_map(w)
        && let Some(mut menu) = w.get_resource_mut::<GameMenu>()
        && (!menu.in_game || serving)
        && menu.loading.is_none()
    {
        if menu.in_game {
            menu.in_game = false;
            menu.open = true;
            menu.page = Page::Main;
        }
        menu.loading = Some(map);
    }
    let loading = super::console::map_loading(w);
    let mut after = w.resource_mut::<AfterLoad>();
    if after.lines.is_empty() {
        return;
    }
    if loading {
        after.started = true;
        return;
    }
    after.waited += 1;
    // The `map` line runs in the next frame's queue.
    if !after.started && after.waited < 10 {
        return;
    }
    let lines = std::mem::take(&mut after.lines);
    let mut console = w.resource_mut::<Console>();
    for line in lines {
        console.submit(line);
    }
}

// ---------------------------------------------------------------------------
// Drawing.

#[derive(Component)]
struct MenuRoot;

/// Colours, sizes and fonts: the GameUI scheme's, else built in.
pub(super) struct Look<'a> {
    pub ui: Option<&'a GameUi>,
    pub fonts: &'a UiFonts,
    /// Pixels per scheme pixel (GameUI is drawn in screen pixels; larger
    /// windows scale it up).
    pub s: f32,
    pub height: f32,
    pub accent: Color,
}

impl<'a> Look<'a> {
    pub(super) fn color(&self, name: &str, fallback: [u8; 4]) -> Color {
        let [r, g, b, a] = self.ui.and_then(|u| u.color(name)).unwrap_or(fallback);
        Color::srgba_u8(r, g, b, a)
    }

    pub(super) fn number(&self, name: &str, fallback: f32) -> f32 {
        self.ui.and_then(|u| u.numbers.get(name).copied()).unwrap_or(fallback)
    }

    /// Built in colours stand in for the scheme's: a darker panel, our
    /// accent for selections.
    pub(super) fn has_scheme(&self) -> bool {
        self.ui.is_some_and(|u| !u.colors.is_empty())
    }

    /// A GameUI scheme font (`Default`, `UiBold`, `MenuLarge`) at this
    /// window's size: `fallback` scheme pixels tall and bold when the
    /// scheme lacks it.
    pub(super) fn font(&self, name: &str, fallback: (f32, bool)) -> TextFont {
        self.fonts.source(name, self.height, self.s, fallback)
    }

    pub(super) fn frame_bg(&self) -> Color {
        if self.has_scheme() {
            self.color("Frame.BgColor", [160, 160, 160, 128])
        } else {
            Color::srgba_u8(28, 28, 28, 230)
        }
    }

    pub(super) fn bright(&self) -> Color {
        self.color("Border.Bright", [200, 200, 200, 196])
    }

    pub(super) fn dark(&self) -> Color {
        self.color("Border.Dark", [40, 40, 40, 196])
    }

    pub(super) fn text(&self) -> Color {
        self.color("Label.TextColor", [221, 221, 221, 255])
    }

    pub(super) fn dull(&self) -> Color {
        self.color("Label.TextDullColor", [190, 190, 190, 255])
    }

    pub(super) fn disabled(&self) -> Color {
        self.color("Label.DisabledFgColor1", [117, 117, 117, 255])
    }

    pub(super) fn white(&self) -> Color {
        self.color("Label.TextBrightColor", [255, 255, 255, 255])
    }

    pub(super) fn selected_bg(&self) -> Color {
        if self.has_scheme() {
            self.color("SectionedListPanel.SelectedBgColor", [255, 155, 0, 255])
        } else {
            self.accent
        }
    }

    pub(super) fn selected_text(&self) -> Color {
        self.color("SectionedListPanel.SelectedTextColor", [0, 0, 0, 255])
    }

    pub(super) fn sunken_bg(&self) -> Color {
        self.color("TextEntry.BgColor", [0, 0, 0, 128])
    }

    pub(super) fn px(&self, v: f32) -> Val {
        px((v * self.s).round())
    }
}

/// An absolutely placed box, in scheme pixels, inside `parent`.
pub(super) fn place(look: &Look, x: f32, y: f32, w: f32, h: f32) -> Node {
    Node {
        position_type: PositionType::Absolute,
        left: look.px(x),
        top: look.px(y),
        width: look.px(w),
        height: look.px(h),
        ..default()
    }
}

/// Raised (lit top-left) or sunken (lit bottom-right) VGUI borders.
pub(super) fn bevel(look: &Look, raised: bool) -> BorderColor {
    let (a, b) = if raised {
        (look.bright(), look.dark())
    } else {
        (look.dark(), look.bright())
    };
    BorderColor {
        top: a,
        left: a,
        bottom: b,
        right: b,
    }
}

/// Text in a box: one line, vertically centred, `align` -1 left, 0
/// centre, 1 right.
#[allow(clippy::too_many_arguments)]
pub(super) fn label(
    commands: &mut Commands,
    parent: Entity,
    look: &Look,
    (x, y, w, h): (f32, f32, f32, f32),
    text: &str,
    font: TextFont,
    color: Color,
    align: i32,
) -> Entity {
    let node = Node {
        display: Display::Flex,
        align_items: AlignItems::Center,
        justify_content: match align {
            -1 => JustifyContent::FlexStart,
            0 => JustifyContent::Center,
            _ => JustifyContent::FlexEnd,
        },
        overflow: Overflow::clip(),
        ..place(look, x, y, w, h)
    };
    let e = commands.spawn((node, ChildOf(parent))).id();
    commands.spawn((
        Text::new(text),
        font,
        TextColor(color),
        TextLayout::new(Justify::Left, LineBreak::NoWrap),
        ChildOf(e),
    ));
    e
}

/// A VGUI button: raised borders, its text, lit when focused.
#[allow(clippy::too_many_arguments)]
fn button(
    commands: &mut Commands,
    parent: Entity,
    look: &Look,
    rect: (f32, f32, f32, f32),
    text: &str,
    hit: Hit,
    focused: bool,
    enabled: bool,
) -> Entity {
    let (x, y, w, h) = rect;
    let bg = look.color("Button.BgColor", [0, 0, 0, 0]);
    let focus_bg = if look.has_scheme() {
        Color::srgba(1.0, 1.0, 1.0, 0.12)
    } else {
        look.accent.with_alpha(0.15)
    };
    let e = commands
        .spawn((
            Node {
                border: UiRect::all(px(1.0)),
                ..place(look, x, y, w, h)
            },
            bevel(look, true),
            BackgroundColor(if focused { focus_bg } else { bg }),
            hit,
            Button,
            Interaction::default(),
            ChildOf(parent),
        ))
        .id();
    let color = if !enabled {
        look.disabled()
    } else if focused {
        look.white()
    } else {
        look.color("Button.TextColor", [255, 255, 255, 255])
    };
    label(commands, e, look, (6.0, 0.0, w - 12.0, h - 2.0), text, look.font("Default", (16.0, false)), color, -1);
    e
}

#[allow(clippy::too_many_arguments)]
fn draw(
    menu: Res<GameMenu>,
    fonts: Res<UiFonts>,
    menu_ui: Option<Res<MenuUi>>,
    hud: Option<Res<crate::map::hud::ActiveHud>>,
    details: Res<LoadingDetails>,
    shown: Query<Entity, With<MenuRoot>>,
    windows: Query<&Window>,
    mut last_size: Local<Vec2>,
    mut commands: Commands,
) {
    let size = windows
        .iter()
        .next()
        .map_or(Vec2::new(640.0, 480.0), |w| Vec2::new(w.width(), w.height()));
    let resized = (size - *last_size).abs().max_element() > 0.5;
    if !menu.is_changed() && !resized && !hud.as_ref().is_some_and(|h| h.is_changed()) && !details.is_changed() {
        return;
    }
    *last_size = size;
    for e in &shown {
        commands.entity(e).despawn();
    }
    if !menu.open {
        return;
    }
    let accent = hud
        .as_ref()
        .and_then(|h| h.0.color("FgColor"))
        .map(|c| c.with_alpha(1.0))
        .unwrap_or(Color::srgb_u8(255, 176, 0));
    let look = Look {
        ui: menu.ui.0.as_deref(),
        fonts: &fonts,
        s: (size.y / 720.0).clamp(0.6, 3.0),
        height: size.y,
        accent,
    };
    let thumbs = menu_ui.as_ref().map(|u| &u.thumbs);
    // In a game the game shows through, darkened (GameUI's backdrop); at
    // the main menu the game's background picture covers the screen (or
    // black without it).
    let backdrop = if !menu.in_game {
        Color::BLACK
    } else if look.has_scheme() {
        look.color("MainMenu.Backdrop", [0, 0, 0, 156])
    } else {
        Color::srgba(0.0, 0.0, 0.0, 0.35)
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
            BackgroundColor(backdrop),
            // Over the HUD (40-45), under the scoreboard and the console.
            GlobalZIndex(46),
        ))
        .id();
    if !menu.in_game
        && let Some(image) = menu_ui.as_ref().and_then(|u| u.background(size))
    {
        commands.spawn((
            Node {
                position_type: PositionType::Absolute,
                width: percent(100.0),
                height: percent(100.0),
                ..default()
            },
            ImageNode {
                image,
                image_mode: NodeImageMode::Stretch,
                ..default()
            },
            ChildOf(root),
        ));
    }
    let title_font = menu_ui.as_ref().and_then(|u| u.title_font.clone());
    if let Some(failure) = &menu.failure {
        failure_frame(&mut commands, root, &menu, &look, size, failure);
        return;
    }
    if let Some(map) = &menu.loading {
        loading_frame(&mut commands, root, &menu, &look, size, map);
        if details.0 != 0 {
            details_panel(&mut commands, root, &look, size);
        }
        return;
    }
    main_list(&mut commands, root, &menu, &look, size, title_font);
    if menu.page == Page::Main {
        return;
    }
    let (w, h) = match menu.page {
        Page::Settings | Page::Maps => (532.0, 410.0),
        Page::NewGame => (560.0, 280.0),
        _ => (360.0, 230.0),
    };
    let title = match menu.page {
        Page::Main => String::new(),
        Page::NewGame => menu.text("#GameUI_CreateServer", "Create Server"),
        Page::Maps => "Choose a Map".into(),
        Page::Bots => "Bots".into(),
        Page::Team => "Choose a Team".into(),
        Page::Settings => menu.text("#GameUI_Options", "Options"),
    };
    let frame = frame(&mut commands, root, &look, size, (w, h), &title);
    let rows = menu.rows();
    match menu.page {
        Page::Settings => {
            tabs(&mut commands, frame, &look, &menu, w);
            let content = commands
                .spawn((
                    Node {
                        border: UiRect::all(px(1.0)),
                        ..place(&look, 8.0, 56.0, w - 16.0, h - 56.0 - 40.0)
                    },
                    bevel(&look, true),
                    ChildOf(frame),
                ))
                .id();
            if menu.tab == Tab::Keyboard {
                keyboard_tab(&mut commands, content, &look, &menu, &rows);
            } else {
                if menu.tab == Tab::Multiplayer {
                    setting_rows(&mut commands, content, &look, &menu, &rows, (20.0, 14.0, w - 56.0 - 104.0));
                    crosshair_preview(&mut commands, content, &look, &menu, (w - 16.0 - 104.0, 14.0));
                } else {
                    setting_rows(&mut commands, content, &look, &menu, &rows, (20.0, 14.0, w - 56.0));
                }
            }
            let ok = rows.len() - 1;
            button(
                &mut commands,
                frame,
                &look,
                (w - 8.0 - 80.0, h - 8.0 - 24.0, 80.0, 24.0),
                &label_of(&rows[ok]),
                Hit(Target::Row(ok), 0),
                menu.focus == ok,
                true,
            );
            if menu.tab != Tab::Keyboard {
                label(
                    &mut commands,
                    frame,
                    &look,
                    (12.0, h - 8.0 - 24.0, w - 120.0, 24.0),
                    "Changes apply at once; config.cfg keeps them.",
                    look.font("DefaultSmall", (13.0, false)),
                    look.dull(),
                    -1,
                );
            }
        }
        Page::Maps => map_list(&mut commands, frame, &look, &menu, &rows, thumbs, (w, h)),
        _ => {
            if menu.page == Page::NewGame {
                setting_rows(&mut commands, frame, &look, &menu, &rows, (16.0, 36.0, w - 32.0 - 152.0));
                if let Some(map) = menu.maps.get(menu.new_game.map) {
                    thumbnail(&mut commands, frame, &look, thumbs, map, (w - 16.0 - 136.0, 36.0, 136.0));
                }
            } else {
                setting_rows(&mut commands, frame, &look, &menu, &rows, (16.0, 36.0, w - 32.0));
            }
        }
    }
}

fn label_of(row: &Row) -> String {
    match row {
        Row::Button { label, .. } | Row::Value { label, .. } | Row::Bind { label, .. } => label.clone(),
        Row::Heading(t) | Row::Info(t) => t.clone(),
    }
}

/// The left-hand entries, as GameUI's game menu: at the left inset, one
/// entry per `MainMenu.MenuItemHeight`, the bottom one a fixed distance up
/// from the screen's foot, the game's title over them (its title font,
/// else the menu's); greyed entries in the disabled colour.
fn main_list(
    commands: &mut Commands,
    root: Entity,
    menu: &GameMenu,
    look: &Look,
    size: Vec2,
    title_font: Option<(Handle<Font>, f32, f32)>,
) {
    let entries = menu.entries();
    let item_h = look.number("MainMenu.MenuItemHeight", 22.0);
    let inset = look.number("MainMenu.Inset", 32.0);
    let gaps = entries.iter().filter(|e| e.gap).count() as f32;
    let total = item_h * (entries.len() as f32 + gaps * 0.5);
    let bottom = size.y / look.s - 72.0;
    let top = bottom - total;
    let font = look.font("MenuLarge", (12.0, true));
    let normal = look.color("MainMenu.TextColor", [255, 255, 255, 255]);
    let armed = if look.has_scheme() {
        look.color("MainMenu.ArmedTextColor", [200, 200, 200, 255])
    } else {
        look.accent
    };
    let current_color = look.color("MainMenu.DepressedTextColor", [192, 186, 80, 255]);
    // The title over the entries: the game's lines, bottom one last.
    let lines: Vec<String> = match menu.ui.0.as_ref().filter(|u| !u.title.is_empty()) {
        Some(ui) => ui.title.clone(),
        None => vec!["MASHUP".into()],
    };
    let (title_text, line_h) = match title_font {
        Some((handle, tall, line_per_em)) => (
            TextFont {
                font: handle.into(),
                font_size: FontSize::Px((tall * look.s / line_per_em).max(1.0)),
                ..default()
            },
            tall,
        ),
        None => (
            TextFont {
                font_size: FontSize::Px(34.0 * look.s),
                ..look.font("MenuLarge", (12.0, true))
            },
            48.0,
        ),
    };
    // Lines after the first sit close under it (a subtitle).
    let step = line_h * 0.6;
    let title_top = top - item_h - line_h - step * (lines.len() as f32 - 1.0);
    for (k, line) in lines.iter().enumerate() {
        label(
            commands,
            root,
            look,
            (inset, title_top + k as f32 * step, size.x / look.s - inset, line_h),
            line,
            title_text.clone(),
            normal,
            -1,
        );
    }
    let mut y = top;
    for (i, e) in entries.iter().enumerate() {
        if e.gap {
            y += item_h * 0.5;
        }
        let main_page = menu.page == Page::Main;
        let current = !main_page && e.item.page() == Some(menu.page);
        let focused = main_page && menu.focus == i && e.enabled;
        // GameUI lights the entry under the mouse in its armed colour.
        let color = if !e.enabled {
            look.disabled()
        } else if current {
            current_color
        } else if focused {
            armed
        } else {
            normal
        };
        let e_node = commands
            .spawn((
                place(look, inset - 4.0, y, 260.0, item_h),
                Hit(Target::Main(i), 0),
                Button,
                Interaction::default(),
                BackgroundColor(if focused && !look.has_scheme() {
                    look.accent.with_alpha(0.12)
                } else {
                    Color::NONE
                }),
                ChildOf(root),
            ))
            .id();
        label(commands, e_node, look, (4.0, 0.0, 256.0, item_h), &e.label.to_uppercase(), font.clone(), color, -1);
        y += item_h;
    }
}

/// A map the main menu started is loading: GameUI's loading dialog (its
/// layout from the install, `GameUi::loading`: the frame, the stage line,
/// the progress bar, Cancel), the menu's entries hidden. The stage and
/// the bar follow the load (`loading_progress`).
fn loading_frame(commands: &mut Commands, root: Entity, menu: &GameMenu, look: &Look, size: Vec2, map: &str) {
    let layout = look.ui.and_then(|u| u.loading.as_ref());
    let at = |name: &str, fallback: (f32, f32, f32, f32)| {
        layout
            .and_then(|l| l.get(name))
            .map_or(fallback, |c| (start(c.x), start(c.y), c.wide, c.tall))
    };
    fn start(c: crate::map::hud::HudCoord) -> f32 {
        match c {
            crate::map::hud::HudCoord::Start(v) | crate::map::hud::HudCoord::Centre(v) | crate::map::hud::HudCoord::End(v) => v,
        }
    }
    let (_, _, w, h) = at("LoadingDialog", (0.0, 0.0, 380.0, 112.0));
    let title = menu.text("#GameUI_Loading", "Loading...");
    let f = frame(commands, root, look, size, (w, h), &title);
    let info = label(
        commands,
        f,
        look,
        at("InfoLabel", (20.0, 34.0, 340.0, 24.0)),
        &loading_text(menu, crate::map::loading::current(), map),
        look.font("Default", (16.0, false)),
        look.text(),
        -1,
    );
    commands.entity(info).insert(LoadingInfo(map.to_string()));
    // The bar: sunken, filled with the scheme's progress colour.
    let (x, y, bw, bh) = at("Progress", (20.0, 64.0, 260.0, 24.0));
    let bar = commands
        .spawn((
            Node {
                border: UiRect::all(px(1.0)),
                padding: UiRect::all(look.px(2.0)),
                ..place(look, x, y, bw, bh)
            },
            bevel(look, false),
            BackgroundColor(look.color("ProgressBar.BgColor", [0, 0, 0, 128])),
            ChildOf(f),
        ))
        .id();
    let fraction = crate::map::loading::current().map_or(0.0, |p| p.fraction);
    commands.spawn((
        LoadingBar,
        Node {
            width: percent(100.0 * fraction),
            height: percent(100.0),
            ..default()
        },
        BackgroundColor(look.color("ProgressBar.FgColor", [216, 222, 211, 255])),
        ChildOf(bar),
    ));
    let cancel = layout.and_then(|l| l.get("CancelButton"));
    let text = cancel.map_or_else(|| menu.text("#GameUI_Cancel", "Cancel"), |c| c.text.clone());
    button(
        commands,
        f,
        look,
        at("CancelButton", (288.0, 64.0, 72.0, 24.0)),
        &text,
        Hit(Target::Cancel, 0),
        false,
        true,
    );
}

/// The loading dialog's stage line: the stage the load reported, as the
/// game words it (`LoadingProgress_LoadMap`: "Loading world..."), else
/// "Loading <map> ...".
fn loading_text(menu: &GameMenu, progress: Option<crate::map::loading::LoadProgress>, map: &str) -> String {
    match progress {
        Some(p) => menu.text(&format!("#{}", p.stage), p.stage),
        None => menu.text("#GameUI_LoadingFilename", "Loading %s1 ...").replace("%s1", map),
    }
}

/// What the loading dialog shows now: its stage line and the bar's
/// fraction. Joining a server: its stages (connecting, retrieving server
/// info, downloading the map with its progress, the map's own load), as
/// the game words them; a map loading: its stages; a server starting:
/// "Starting local game server..." until the map reports.
pub fn dialog_state(
    menu: &GameMenu,
    join: Option<&crate::net::client::JoinProgress>,
    progress: Option<crate::map::loading::LoadProgress>,
    hosting: bool,
    map: &str,
) -> (String, f32) {
    use crate::net::client::JoinStage as S;
    // Joined too: the frame before the dialog closes.
    if let Some(j) = join.filter(|j| j.stage != crate::net::client::JoinStage::Idle && j.failure.is_none()) {
        let (token, ours) = j.stage.token();
        let stage = menu.text(&format!("#{token}"), ours);
        return match j.stage {
            S::LoadingMap => match progress {
                Some(p) => (menu.text(&format!("#{}", p.stage), p.stage), p.fraction),
                None => (stage, 0.0),
            },
            S::Downloading => {
                let f = j
                    .download
                    .as_ref()
                    .filter(|d| d.total > 0)
                    .map_or(0.0, |d| (d.done as f64 / d.total as f64) as f32);
                (stage, f.clamp(0.0, 1.0))
            }
            S::Connecting => (stage, 0.02),
            S::ServerInfo | S::ChangingLevel => (stage, 0.05),
            _ => (stage, 0.08),
        };
    }
    if hosting && progress.is_none() {
        return (
            menu.text("#LoadingProgress_SpawningServer", "Starting local game server..."),
            0.0,
        );
    }
    (
        loading_text(menu, progress, map),
        progress.map_or(0.0, |p| p.fraction),
    )
}

/// `mashup_loading_details`: the loading dialog's detailed view (each
/// stage's time, bytes and percentage; the server's name, map, players
/// and ping). Off by default (the game's dialog alone).
#[derive(Resource, Clone, Copy, Debug, Default, PartialEq)]
pub struct LoadingDetails(pub u8);

/// The stages the loading dialog went through this time, for the
/// detailed view: each one's name, when it started (real s) and its
/// latest progress text.
#[derive(Resource, Clone, Debug, Default)]
pub struct LoadTimeline {
    pub stages: Vec<(String, f64, String)>,
}

impl LoadTimeline {
    /// The detailed view's lines at `now`: a stage's time runs to the
    /// next one's start (the last to now).
    pub fn lines(&self, now: f64) -> Vec<String> {
        let mut out = Vec::new();
        for (i, (name, start, progress)) in self.stages.iter().enumerate() {
            let end = self.stages.get(i + 1).map_or(now, |s| s.1);
            out.push(format!("{name:<32} {:>7.2} s   {progress}", (end - start).max(0.0)));
        }
        out
    }
}

/// Bytes as the detailed view shows them.
fn bytes(n: u64) -> String {
    if n >= 1024 * 1024 {
        format!("{:.1} MB", n as f64 / (1024.0 * 1024.0))
    } else {
        format!("{:.0} KB", n as f64 / 1024.0)
    }
}

/// The detailed view's stage now: its name and progress text.
fn detail_stage(
    join: Option<&crate::net::client::JoinProgress>,
    progress: Option<crate::map::loading::LoadProgress>,
) -> (String, String) {
    use crate::net::client::JoinStage as S;
    let map_stage = |p: crate::map::loading::LoadProgress| {
        (
            format!("loading the map: {}", p.stage.trim_start_matches("LoadingProgress_")),
            format!("{:.0}%", p.fraction * 100.0),
        )
    };
    match join.filter(|j| j.stage != S::Idle && j.failure.is_none()) {
        Some(j) if j.stage == S::Downloading => {
            let text = j.download.as_ref().map_or(String::new(), |d| {
                if d.total > 0 {
                    format!(
                        "{} of {} ({:.0}%) of {} from {}",
                        bytes(d.done),
                        bytes(d.total),
                        d.done as f64 * 100.0 / d.total as f64,
                        d.file,
                        d.from
                    )
                } else {
                    format!("{} of {} from {}", bytes(d.done), d.file, d.from)
                }
            });
            (j.stage.name().to_string(), text)
        }
        Some(j) if j.stage == S::LoadingMap => progress.map_or_else(|| (j.stage.name().into(), String::new()), map_stage),
        Some(j) => (j.stage.name().to_string(), String::new()),
        None => progress.map_or_else(|| ("starting".into(), String::new()), map_stage),
    }
}

/// The detailed view's text: the server, then the stages.
#[derive(Component)]
struct LoadingDetail;

/// The detailed view under the dialog (`mashup_loading_details 1`).
fn details_panel(commands: &mut Commands, root: Entity, look: &Look, size: Vec2) {
    let (w, h) = (560.0, 190.0);
    let x = ((size.x / look.s - w) / 2.0).max(0.0);
    let y = ((size.y / look.s) / 2.0 + 70.0).max(0.0);
    let panel = commands
        .spawn((
            Node {
                border: UiRect::all(px(1.0)),
                padding: UiRect::all(look.px(8.0)),
                ..place(look, x, y, w, h)
            },
            bevel(look, false),
            BackgroundColor(look.sunken_bg()),
            ChildOf(root),
        ))
        .id();
    commands.spawn((
        LoadingDetail,
        Text::new(""),
        look.font("DefaultFixed", (13.0, false)),
        TextColor(look.text()),
        TextLayout::new(Justify::Left, LineBreak::NoWrap),
        ChildOf(panel),
    ));
}

/// Why joining failed, in the loading dialog: "Disconnected", the reason
/// (wrapped), Close.
fn failure_frame(commands: &mut Commands, root: Entity, menu: &GameMenu, look: &Look, size: Vec2, text: &str) {
    let (w, h) = (380.0, 150.0);
    let title = menu.text("#GameUI_Disconnected", "Disconnected");
    let f = frame(commands, root, look, size, (w, h), &title);
    let body = commands
        .spawn((
            Node {
                overflow: Overflow::clip(),
                ..place(look, 20.0, 34.0, w - 40.0, h - 34.0 - 40.0)
            },
            ChildOf(f),
        ))
        .id();
    commands.spawn((
        Text::new(text),
        look.font("Default", (16.0, false)),
        TextColor(look.text()),
        TextLayout::new(Justify::Left, LineBreak::WordBoundary),
        ChildOf(body),
    ));
    let close = menu.text("#GameUI_Close", "Close");
    button(
        commands,
        f,
        look,
        (w - 20.0 - 72.0, h - 34.0, 72.0, 24.0),
        &close,
        Hit(Target::Cancel, 0),
        true,
        true,
    );
}

/// The loading dialog's stage line (the map's name).
#[derive(Component)]
struct LoadingInfo(String);

/// The loading dialog's bar fill.
#[derive(Component)]
struct LoadingBar;

/// The loading dialog follows the load's progress (a map's, joining a
/// server's), and the detailed view times its stages.
#[allow(clippy::too_many_arguments)]
fn loading_progress(
    menu: Res<GameMenu>,
    join: Option<Res<crate::net::client::JoinProgress>>,
    settings: Option<Res<crate::net::NetSettings>>,
    role: Option<Res<crate::core::NetRole>>,
    client: Option<Res<bevy_replicon_renet::RenetClient>>,
    time: Res<Time<Real>>,
    mut timeline: ResMut<LoadTimeline>,
    infos: Query<(&LoadingInfo, &Children)>,
    mut texts: Query<&mut Text, Without<LoadingDetail>>,
    mut details: Query<&mut Text, With<LoadingDetail>>,
    mut bars: Query<&mut Node, With<LoadingBar>>,
) {
    let now = time.elapsed_secs_f64();
    if menu.loading.is_none() || menu.failure.is_some() {
        timeline.stages.clear();
        return;
    }
    let progress = crate::map::loading::current();
    let hosting = settings.as_ref().is_some_and(|s| s.maxplayers > 1)
        && role.as_deref().is_none_or(|r| *r != crate::core::NetRole::Client)
        && join.as_ref().is_none_or(|j| !j.showing());
    let label = infos.iter().next().map_or(String::new(), |(i, _)| i.0.clone());
    let (line, fraction) = dialog_state(&menu, join.as_deref(), progress, hosting, &label);
    for mut node in &mut bars {
        let w = percent(100.0 * fraction);
        if node.width != w {
            node.width = w;
        }
    }
    for (_, children) in &infos {
        for c in children.iter() {
            if let Ok(mut t) = texts.get_mut(c)
                && t.0 != line
            {
                t.0 = line.clone();
            }
        }
    }
    // The timeline, for the detailed view.
    let (stage, detail) = detail_stage(join.as_deref(), progress);
    match timeline.stages.last_mut() {
        Some(last) if last.0 == stage => last.2 = detail,
        _ => timeline.stages.push((stage, now, detail)),
    }
    if details.is_empty() {
        return;
    }
    let mut lines = Vec::new();
    if let Some(j) = join.as_deref().filter(|j| j.showing()) {
        let server = j.server.clone().unwrap_or_default();
        let ping = client.as_ref().map_or(0.0, |c| c.rtt() * 1000.0);
        lines.push(format!(
            "server  {}  {}",
            if server.name.is_empty() { "(asking)" } else { &server.name },
            j.address.map_or(String::new(), |a| a.to_string())
        ));
        lines.push(format!(
            "map     {}   players {}/{}   ping {ping:.0} ms",
            j.map.as_deref().unwrap_or("?"),
            server.players,
            server.max_players
        ));
    } else {
        lines.push(format!("map     {label}"));
    }
    lines.push(String::new());
    lines.extend(timeline.lines(now));
    let total = timeline.stages.first().map_or(0.0, |s| now - s.1);
    lines.push(format!("{:<32} {total:>7.2} s", "total"));
    let text = lines.join("\n");
    for mut t in &mut details {
        if t.0 != text {
            t.0 = text.clone();
        }
    }
}

/// The interface's sounds (`GameUi::sounds`): the rollover as the
/// pointer comes onto another entry, the click as one is pressed, the
/// release as it is let go.
fn menu_sounds(
    menu: Res<GameMenu>,
    menu_ui: Option<Res<MenuUi>>,
    hits: Query<(&Interaction, &Hit), Changed<Interaction>>,
    clips: Option<ResMut<Assets<crate::map::live_sound::LiveClip>>>,
    mut last: Local<(Option<Target>, Option<Target>)>,
    mut commands: Commands,
) {
    let (Some(ui), Some(mut clips)) = (menu_ui.and_then(|u| u.ui.clone()), clips) else {
        return;
    };
    if !menu.open {
        *last = (None, None);
        return;
    }
    use crate::map::hud::UiSound;
    let mut play = |sound: UiSound| {
        if let Some(clip) = ui.sounds.get(&sound) {
            let gains = std::sync::Arc::new(crate::map::live_sound::Gains::new(1.0, 1.0));
            let handle = clips.add(crate::map::live_sound::LiveClip::new(clip.clone(), gains, default()));
            commands.spawn((AudioPlayer(handle), PlaybackSettings::DESPAWN));
        }
    };
    let (hovered, pressed) = &mut *last;
    for (interaction, hit) in &hits {
        match interaction {
            Interaction::Hovered => {
                if *pressed == Some(hit.0) {
                    play(UiSound::Release);
                } else if *hovered != Some(hit.0) {
                    play(UiSound::Rollover);
                }
                *hovered = Some(hit.0);
                *pressed = None;
            }
            Interaction::Pressed => {
                play(UiSound::Click);
                *pressed = Some(hit.0);
            }
            // Off it: coming back rolls over again (a redraw replaces the
            // node without this, so the entry under the pointer is quiet).
            Interaction::None if *hovered == Some(hit.0) => *hovered = None,
            Interaction::None => {}
        }
    }
}

/// A GameUI frame centred on the screen: its background, raised borders,
/// title. Returns the frame (children placed in scheme pixels from its
/// corner).
pub(super) fn frame(commands: &mut Commands, root: Entity, look: &Look, size: Vec2, (w, h): (f32, f32), title: &str) -> Entity {
    let x = ((size.x / look.s - w) / 2.0).max(0.0);
    let y = ((size.y / look.s - h) / 2.0).max(0.0);
    let e = commands
        .spawn((
            Node {
                border: UiRect::all(px(1.0)),
                ..place(look, x, y, w, h)
            },
            bevel(look, true),
            BackgroundColor(look.frame_bg()),
            ChildOf(root),
        ))
        .id();
    let inset = look.number("Frame.TitleTextInsetX", 16.0);
    label(
        commands,
        e,
        look,
        (inset, 4.0, w - inset - 32.0, 22.0),
        title,
        look.font("UiBold", (12.0, true)),
        look.color("FrameTitleBar.TextColor", [255, 255, 255, 255]),
        -1,
    );
    e
}

/// The options' tabs over the page (PropertySheet): raised, the open one
/// joined to the page below.
fn tabs(commands: &mut Commands, frame: Entity, look: &Look, menu: &GameMenu, w: f32) {
    let mut x = 8.0;
    let font = look.font("Default", (16.0, false));
    let tab_w = ((w - 16.0) / TABS.len() as f32).min(96.0);
    for (i, (tab, token, ours)) in TABS.iter().enumerate() {
        let open = *tab == menu.tab;
        let (y, h) = if open { (30.0, 27.0) } else { (33.0, 23.0) };
        let e = commands
            .spawn((
                Node {
                    border: UiRect {
                        left: px(1.0),
                        right: px(1.0),
                        top: px(1.0),
                        bottom: px(0.0),
                    },
                    ..place(look, x, y, tab_w - 2.0, h)
                },
                bevel(look, true),
                BackgroundColor(if open { look.frame_bg() } else { Color::NONE }),
                Hit(Target::Tab(i), 0),
                Button,
                Interaction::default(),
                ZIndex(if open { 2 } else { 0 }),
                ChildOf(frame),
            ))
            .id();
        let color = if open {
            look.color("PropertySheet.SelectedTextColor", [255, 255, 255, 255])
        } else {
            look.color("PropertySheet.TextColor", [221, 221, 221, 255])
        };
        label(commands, e, look, (0.0, 0.0, tab_w - 4.0, h - 1.0), &menu.text(token, ours), font.clone(), color, 0);
        x += tab_w;
    }
}

/// A scroll bar `h` tall at (x, y): arrows at the ends, a thumb for the
/// shown part; clicks scroll a row (arrows) or a page (the track).
fn scroll_bar(commands: &mut Commands, parent: Entity, look: &Look, (x, y, h): (f32, f32, f32), (len, shown, first): (usize, usize, usize)) {
    let w = look.number("ScrollBar.Wide", 17.0);
    let fg = look.color("ScrollBarButton.FgColor", [255, 255, 255, 255]);
    let track = commands
        .spawn((place(look, x, y, w, h), BackgroundColor(look.sunken_bg()), ChildOf(parent)))
        .id();
    let font = look.font("Marlett", (12.0, false));
    for (ty, text, step) in [(0.0, "\u{25B2}", -1), (h - w, "\u{25BC}", 1)] {
        let b = commands
            .spawn((
                Node {
                    border: UiRect::all(px(1.0)),
                    ..place(look, 0.0, ty, w, w)
                },
                bevel(look, true),
                Hit(Target::Scroll, step),
                Button,
                Interaction::default(),
                ChildOf(track),
            ))
            .id();
        label(commands, b, look, (0.0, 0.0, w - 2.0, w - 2.0), text, font.clone(), fg, 0);
    }
    let room = h - 2.0 * w;
    if len > shown && room > 8.0 {
        let thumb_h = (room * shown as f32 / len as f32).max(10.0);
        let at = w + (room - thumb_h) * first as f32 / (len - shown) as f32;
        let page = shown as i32;
        commands.spawn((
            place(look, 0.0, w, w, at - w),
            Hit(Target::Scroll, -page),
            Button,
            Interaction::default(),
            ChildOf(track),
        ));
        commands.spawn((
            place(look, 0.0, at + thumb_h, w, h - w - at - thumb_h),
            Hit(Target::Scroll, page),
            Button,
            Interaction::default(),
            ChildOf(track),
        ));
        commands.spawn((
            Node {
                border: UiRect::all(px(1.0)),
                ..place(look, 1.0, at, w - 2.0, thumb_h)
            },
            bevel(look, true),
            BackgroundColor(look.color("ScrollBarSlider.BgColor", [255, 255, 255, 64])),
            ChildOf(track),
        ));
    }
}

/// The keyboard tab: the action list (sections, action and key columns,
/// a scroll bar) where the game's layout puts it, and its buttons.
fn keyboard_tab(commands: &mut Commands, content: Entity, look: &Look, menu: &GameMenu, rows: &[Row]) {
    let layout = menu.ui.0.as_ref().and_then(|u| u.options.get("keyboard"));
    let rect = |name: &str, fallback: (f32, f32, f32, f32)| {
        layout
            .and_then(|l| l.get(name))
            .map_or(fallback, |c| {
                let num = |h: crate::map::hud::HudCoord| match h {
                    crate::map::hud::HudCoord::Start(v) => v,
                    _ => 0.0,
                };
                (num(c.x), num(c.y), c.wide, c.tall)
            })
    };
    let (lx, ly, lw, lh) = rect("listpanel_keybindlist", (8.0, 10.0, 480.0, 258.0));
    let (len, shown) = menu.list();
    let list = commands
        .spawn((
            Node {
                border: UiRect::all(px(1.0)),
                ..place(look, lx, ly, lw, lh)
            },
            bevel(look, false),
            BackgroundColor(look.color("SectionedListPanel.BgColor", [0, 0, 0, 128])),
            RelativeCursorPosition::default(),
            WheelList,
            ChildOf(content),
        ))
        .id();
    let bar_w = look.number("ScrollBar.Wide", 17.0);
    let row_h = ((lh - 22.0) / KEY_ROWS as f32).floor();
    let key_x = (lw - bar_w) * 0.62;
    let header = look.font("DefaultBold", (16.0, true));
    let text_font = look.font("Default", (16.0, false));
    let header_color = look.color("SectionedListPanel.HeaderTextColor", [255, 255, 255, 255]);
    label(commands, list, look, (6.0, 1.0, key_x, 20.0), &menu.text("#GameUI_Action", "Action"), header.clone(), header_color, -1);
    label(
        commands,
        list,
        look,
        (key_x, 1.0, lw - bar_w - key_x, 20.0),
        &menu.text("#GameUI_KeyButton", "Key/Button"),
        header.clone(),
        header_color,
        -1,
    );
    commands.spawn((
        place(look, 2.0, 21.0, lw - bar_w - 6.0, 1.0),
        BackgroundColor(look.color("SectionedListPanel.DividerColor", [0, 0, 0, 255])),
        ChildOf(list),
    ));
    let text = look.color("SectionedListPanel.TextColor", [190, 190, 190, 255]);
    for (k, i) in (menu.scroll..(menu.scroll + shown).min(len)).enumerate() {
        let y = 22.0 + k as f32 * row_h;
        let width = lw - bar_w - 4.0;
        match &rows[i] {
            Row::Heading(t) => {
                label(commands, list, look, (6.0, y, width, row_h), t, header.clone(), header_color, -1);
            }
            Row::Bind { label: name, keys, known, .. } => {
                let selected = menu.key_row == Some(i);
                let waiting = selected && menu.capture.is_some();
                let bg = if selected { look.selected_bg() } else { Color::NONE };
                let fg = if selected {
                    look.selected_text()
                } else if *known {
                    text
                } else {
                    look.disabled()
                };
                let e = commands
                    .spawn((
                        place(look, 1.0, y, width, row_h),
                        BackgroundColor(bg),
                        Hit(Target::Row(i), 0),
                        Button,
                        Interaction::default(),
                        ChildOf(list),
                    ))
                    .id();
                label(commands, e, look, (16.0, 0.0, key_x - 20.0, row_h), name, text_font.clone(), fg, -1);
                let keys = if waiting { "Press a key (Esc: cancel)".to_string() } else { keys.clone() };
                label(commands, e, look, (key_x - 1.0, 0.0, width - key_x, row_h), &keys, text_font.clone(), fg, -1);
            }
            _ => {}
        }
    }
    scroll_bar(commands, list, look, (lw - bar_w - 2.0, 0.0, lh - 2.0), (len, shown, menu.scroll));
    // The buttons, where the game's layout has them.
    let names = ["Defaults", "ChangeKeyButton", "ClearKeyButton"];
    let fallback = [(8.0, 276.0, 134.0, 24.0), (272.0, 276.0, 106.0, 24.0), (384.0, 276.0, 105.0, 24.0)];
    for (k, name) in names.iter().enumerate() {
        let i = len + k;
        let Row::Button { label: text, enabled, .. } = &rows[i] else { continue };
        button(commands, content, look, rect(name, fallback[k]), text, Hit(Target::Row(i), 0), menu.focus == i, *enabled);
    }
}

/// Rows of labelled controls (sliders, check boxes, choices, buttons) in a
/// column at (x, y), `w` wide; the last row (OK / Back) on the options
/// page is the dialog's own button, drawn by the caller.
fn setting_rows(commands: &mut Commands, parent: Entity, look: &Look, menu: &GameMenu, rows: &[Row], (x, y, w): (f32, f32, f32)) {
    let row_h = 28.0;
    let font = look.font("Default", (16.0, false));
    let controls = if menu.page == Page::Settings { rows.len() - 1 } else { rows.len() };
    let label_w = (w * 0.45).round();
    let control_w = (w - label_w).min(220.0);
    for (i, row) in rows.iter().enumerate().take(controls) {
        let ry = y + i as f32 * row_h;
        let focused = menu.focus == i;
        match row {
            Row::Info(t) | Row::Heading(t) => {
                label(commands, parent, look, (x, ry, w, row_h - 4.0), t, font.clone(), look.dull(), -1);
            }
            Row::Button { label: text, enabled, .. } => {
                button(commands, parent, look, (x, ry, w.min(260.0), 24.0), text, Hit(Target::Row(i), 0), focused, *enabled);
            }
            Row::Bind { .. } => {}
            Row::Value { label: text, value, field } => {
                let color = if focused { look.white() } else { look.text() };
                label(commands, parent, look, (x, ry, label_w, 24.0), text, font.clone(), color, -1);
                let cx = x + label_w;
                let kind = match field {
                    Field::Setting(s) => Some(SETTINGS[*s].kind),
                    _ => None,
                };
                match kind {
                    Some(SettingKind::Range { .. }) => {
                        let Field::Setting(s) = field else { unreachable!() };
                        let f = menu.values[*s].as_deref().and_then(|v| SETTINGS[*s].fraction(v)).unwrap_or(0.0);
                        slider(commands, parent, look, (cx, ry, control_w - 48.0), i, f, focused);
                        label(commands, parent, look, (cx + control_w - 44.0, ry, 44.0, 24.0), value, font.clone(), color, 1);
                    }
                    Some(SettingKind::Toggle | SettingKind::Negate) => {
                        let Field::Setting(s) = field else { unreachable!() };
                        let on = menu.values[*s].as_deref().is_some_and(|v| SETTINGS[*s].checked(v));
                        check_box(commands, parent, look, (cx, ry), i, on, focused);
                    }
                    _ => combo(commands, parent, look, (cx, ry, control_w), i, value, focused),
                }
            }
        }
    }
}

/// A slider: its track and nob; pressing or dragging sets the value.
fn slider(commands: &mut Commands, parent: Entity, look: &Look, (x, y, w): (f32, f32, f32), row: usize, f: f32, focused: bool) {
    let e = commands
        .spawn((
            place(look, x, y, w, 24.0),
            SliderTrack(row),
            RelativeCursorPosition::default(),
            Button,
            Interaction::default(),
            BackgroundColor(if focused { Color::srgba(1.0, 1.0, 1.0, 0.06) } else { Color::NONE }),
            ChildOf(parent),
        ))
        .id();
    commands.spawn((
        Node {
            border: UiRect::all(px(1.0)),
            ..place(look, 4.0, 10.0, w - 8.0, 4.0)
        },
        bevel(look, false),
        BackgroundColor(look.color("Slider.TrackColor", [31, 31, 31, 255])),
        ChildOf(e),
    ));
    let nob_x = 4.0 + (w - 8.0 - 8.0) * f;
    commands.spawn((
        Node {
            border: UiRect::all(px(1.0)),
            ..place(look, nob_x, 3.0, 8.0, 18.0)
        },
        bevel(look, true),
        BackgroundColor(look.color("Slider.NobColor", [108, 108, 108, 255])),
        ChildOf(e),
    ));
}

/// A check box: a sunken square, ticked when on.
fn check_box(commands: &mut Commands, parent: Entity, look: &Look, (x, y): (f32, f32), row: usize, on: bool, focused: bool) {
    let e = commands
        .spawn((
            Node {
                border: UiRect::all(px(1.0)),
                ..place(look, x, y + 4.0, 16.0, 16.0)
            },
            BorderColor {
                top: look.color("CheckButton.Border1", [40, 40, 40, 196]),
                left: look.color("CheckButton.Border1", [40, 40, 40, 196]),
                bottom: look.color("CheckButton.Border2", [200, 200, 200, 196]),
                right: look.color("CheckButton.Border2", [200, 200, 200, 196]),
            },
            BackgroundColor(if focused {
                Color::srgba(1.0, 1.0, 1.0, 0.15)
            } else {
                look.color("CheckButton.BgColor", [0, 0, 0, 128])
            }),
            Hit(Target::Row(row), 0),
            Button,
            Interaction::default(),
            ChildOf(parent),
        ))
        .id();
    if on {
        commands.spawn((
            place(look, 3.0, 3.0, 8.0, 8.0),
            BackgroundColor(look.color("CheckButton.Check", [255, 255, 255, 255])),
            ChildOf(e),
        ));
    }
}

/// A choice, as a combo box: a sunken box with the value, arrows either
/// side stepping it.
fn combo(commands: &mut Commands, parent: Entity, look: &Look, (x, y, w): (f32, f32, f32), row: usize, value: &str, focused: bool) {
    let e = commands
        .spawn((
            Node {
                border: UiRect::all(px(1.0)),
                ..place(look, x, y, w, 24.0)
            },
            bevel(look, false),
            BackgroundColor(if focused {
                Color::srgba(1.0, 1.0, 1.0, 0.12)
            } else {
                look.sunken_bg()
            }),
            Hit(Target::Row(row), 0),
            Button,
            Interaction::default(),
            ChildOf(parent),
        ))
        .id();
    let font = look.font("Default", (16.0, false));
    let color = look.color("TextEntry.TextColor", [221, 221, 221, 255]);
    label(commands, e, look, (20.0, 0.0, w - 42.0, 22.0), value, font.clone(), color, 0);
    let arrow = look.color("ComboBoxButton.ArrowColor", [190, 190, 190, 255]);
    for (ax, text, step) in [(0.0, "\u{25C4}", -1), (w - 20.0, "\u{25BA}", 1)] {
        let a = commands
            .spawn((
                place(look, ax, 0.0, 18.0, 22.0),
                Hit(Target::Row(row), step),
                Button,
                Interaction::default(),
                ChildOf(e),
            ))
            .id();
        label(commands, a, look, (0.0, 0.0, 18.0, 22.0), text, font.clone(), arrow, 0);
    }
}

/// The crosshair as the multiplayer tab's settings draw it, on a dark
/// square.
fn crosshair_preview(commands: &mut Commands, parent: Entity, look: &Look, menu: &GameMenu, (x, y): (f32, f32)) {
    let get = |cvar: &str| {
        SETTINGS
            .iter()
            .position(|s| s.cvar == cvar)
            .and_then(|i| menu.values.get(i).cloned().flatten())
            .and_then(|v| v.trim().parse::<f32>().ok())
    };
    let defaults = super::hud::CrosshairColor::default();
    let c = super::hud::CrosshairColor {
        color: get("cl_crosshaircolor").map_or(defaults.color, |v| v as u8),
        scale: get("cl_crosshairscale").unwrap_or(defaults.scale),
        alpha: get("cl_crosshairalpha").map_or(defaults.alpha, |v| v as u8),
        use_alpha: get("cl_crosshairusealpha").map_or(defaults.use_alpha, |v| v as u8),
        dynamic: get("cl_dynamiccrosshair").map_or(defaults.dynamic, |v| v as u8),
    };
    let side = 88.0;
    let e = commands
        .spawn((
            Node {
                border: UiRect::all(px(1.0)),
                ..place(look, x, y, side, side)
            },
            bevel(look, false),
            BackgroundColor(Color::srgb(0.25, 0.27, 0.3)),
            ChildOf(parent),
        ))
        .id();
    // As at this window's height, in screen pixels.
    let centre = Vec2::splat(side * look.s / 2.0);
    for (offset, size) in c.lines(4.0, look.height) {
        let at = centre + offset - size / 2.0;
        commands.spawn((
            Node {
                position_type: PositionType::Absolute,
                left: px(at.x.round()),
                top: px(at.y.round()),
                width: px(size.x),
                height: px(size.y),
                ..default()
            },
            BackgroundColor(c.color()),
            ChildOf(e),
        ));
    }
}

/// A map's thumbnail (`menu_thumb_<map>`, else the game's default one),
/// `side` scheme pixels wide at (x, y), as tall as its picture.
fn thumbnail(
    commands: &mut Commands,
    parent: Entity,
    look: &Look,
    thumbs: Option<&HashMap<String, (Handle<Image>, f32)>>,
    map: &str,
    (x, y, side): (f32, f32, f32),
) {
    let Some((image, aspect)) = thumbs.and_then(|t| t.get(&map.to_lowercase()).or_else(|| t.get("default"))) else {
        return;
    };
    commands.spawn((
        Node {
            border: UiRect::all(px(1.0)),
            ..place(look, x, y, side, (side * aspect).round())
        },
        bevel(look, false),
        ImageNode::new(image.clone()),
        ChildOf(parent),
    ));
}

/// The map list: a scrolled list with a scroll bar, the focused map's
/// thumbnail beside it, Back under it.
#[allow(clippy::too_many_arguments)]
fn map_list(
    commands: &mut Commands,
    frame: Entity,
    look: &Look,
    menu: &GameMenu,
    rows: &[Row],
    thumbs: Option<&HashMap<String, (Handle<Image>, f32)>>,
    (w, h): (f32, f32),
) {
    let (len, shown) = menu.list();
    let font = look.font("Default", (16.0, false));
    let (lx, ly, lw) = (12.0, 36.0, 300.0);
    let row_h = 20.0;
    let lh = shown as f32 * row_h + 4.0;
    let list = commands
        .spawn((
            Node {
                border: UiRect::all(px(1.0)),
                ..place(look, lx, ly, lw, lh)
            },
            bevel(look, false),
            BackgroundColor(look.color("ListPanel.BgColor", [0, 0, 0, 128])),
            RelativeCursorPosition::default(),
            WheelList,
            ChildOf(frame),
        ))
        .id();
    let bar_w = look.number("ScrollBar.Wide", 17.0);
    if len == 0 {
        label(commands, list, look, (8.0, 4.0, lw - 16.0, row_h), "No maps found in the install.", font.clone(), look.dull(), -1);
    }
    for (k, i) in (menu.scroll..(menu.scroll + shown).min(len)).enumerate() {
        let focused = menu.focus == i;
        let chosen = menu.new_game.map == i;
        let e = commands
            .spawn((
                place(look, 1.0, 1.0 + k as f32 * row_h, lw - bar_w - 4.0, row_h),
                BackgroundColor(if focused { look.selected_bg() } else { Color::NONE }),
                Hit(Target::Row(i), 0),
                Button,
                Interaction::default(),
                ChildOf(list),
            ))
            .id();
        let color = if focused {
            look.selected_text()
        } else if chosen {
            look.white()
        } else {
            look.color("ListPanel.TextColor", [221, 221, 221, 255])
        };
        label(commands, e, look, (6.0, 0.0, lw - bar_w - 12.0, row_h), &label_of(&rows[i]), font.clone(), color, -1);
    }
    scroll_bar(commands, list, look, (lw - bar_w - 2.0, 0.0, lh - 2.0), (len, shown, menu.scroll));
    if let Some(map) = menu.maps.get(menu.focus.min(len.saturating_sub(1))) {
        thumbnail(commands, frame, look, thumbs, map, (lx + lw + 16.0, ly, (w - lx - lw - 32.0).min(192.0)));
        label(
            commands,
            frame,
            look,
            (lx + lw + 16.0, ly + 160.0, w - lx - lw - 32.0, 24.0),
            map,
            look.font("DefaultBold", (16.0, true)),
            look.white(),
            -1,
        );
    }
    let hint = if len > shown {
        format!("{len} maps; wheel or the bar scrolls, a letter jumps")
    } else {
        format!("{len} maps; a letter jumps")
    };
    label(commands, frame, look, (lx, ly + lh + 6.0, lw, 20.0), &hint, look.font("DefaultSmall", (13.0, false)), look.dull(), -1);
    // Back.
    let back = rows.len() - 1;
    button(
        commands,
        frame,
        look,
        (w - 8.0 - 80.0, h - 8.0 - 24.0, 80.0, 24.0),
        &label_of(&rows[back]),
        Hit(Target::Row(back), 0),
        menu.focus == back,
        true,
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::map::hud::GameUiItem;

    /// The menu opened in a game (Esc).
    fn menu() -> GameMenu {
        let mut m = GameMenu {
            in_game: true,
            ..default()
        };
        let cvars = [
            ("sensitivity", "3"),
            ("m_pitch", "0.022"),
            ("zoom_sensitivity_ratio", "1.2"),
            ("volume", "1"),
            ("viewmodel_fov", "54"),
            ("cl_righthand", "1"),
            ("cl_showfps", "0"),
            ("mat_vsync", "1"),
            ("mashup_rounds", "1"),
            ("bot_reaction", "0.35"),
        ];
        m.open(
            Page::Main,
            vec!["cs_office".into(), "de_dust2".into(), "de_nuke".into()],
            Some("de_dust2"),
            |n| cvars.iter().find(|(k, _)| *k == n).map(|(_, v)| v.to_string()),
        );
        m
    }

    /// The menu at startup: the main menu, out of a game.
    fn main_menu() -> GameMenu {
        let mut m = GameMenu::default();
        m.open(Page::Main, vec!["cs_office".into(), "de_dust2".into(), "de_nuke".into()], None, |_| None);
        m
    }

    #[test]
    fn cancel_stops_a_load_and_nothing_else_works_meanwhile() {
        let mut m = main_menu();
        m.loading = Some("de_dust2".into());
        let before = m.clone();
        assert!(m.handle(Input::Click(Target::Main(0), 0)).lines.is_empty());
        assert_eq!(m, before, "the dialog takes no other input");
        let out = m.handle(Input::Click(Target::Cancel, 0));
        assert_eq!(out.lines, vec!["disconnect".to_string()]);
        assert!(m.loading.is_none() && m.open && !m.in_game);
    }

    #[test]
    fn the_loading_line_follows_the_load() {
        let m = main_menu();
        assert_eq!(loading_text(&m, None, "de_nuke"), "Loading de_nuke ...");
        let p = crate::map::loading::LoadProgress {
            fraction: 0.5,
            stage: "LoadingProgress_LoadResources",
        };
        // Without the game's strings, the token itself.
        assert_eq!(loading_text(&m, Some(p), "de_nuke"), "LoadingProgress_LoadResources");
    }

    #[test]
    fn joining_shows_its_stages_and_the_download() {
        use crate::net::client::{DownloadInfo, JoinProgress, JoinStage};
        let m = main_menu();
        let mut j = JoinProgress {
            stage: JoinStage::Connecting,
            ..default()
        };
        assert_eq!(dialog_state(&m, Some(&j), None, false, "x").0, "Connecting to server...");
        j.stage = JoinStage::ServerInfo;
        assert_eq!(dialog_state(&m, Some(&j), None, false, "x").0, "Retrieving server info...");
        j.stage = JoinStage::ChangingLevel;
        assert_eq!(dialog_state(&m, Some(&j), None, false, "x").0, "Server is changing level...");
        j.stage = JoinStage::Downloading;
        j.download = Some(DownloadInfo {
            file: "maps/a.bsp".into(),
            from: "server".into(),
            done: 250,
            total: 1000,
        });
        assert_eq!(
            dialog_state(&m, Some(&j), None, false, "x"),
            ("Verifying and downloading resources...".to_string(), 0.25)
        );
        // Loading the map: the map's own stages.
        j.stage = JoinStage::LoadingMap;
        let p = crate::map::loading::LoadProgress {
            fraction: 0.5,
            stage: "LoadingProgress_LoadMap",
        };
        assert_eq!(dialog_state(&m, Some(&j), Some(p), false, "x").1, 0.5);
        // A host starting its server, before the map reports.
        assert_eq!(
            dialog_state(&m, None, None, true, "de_dust2").0,
            "Starting local game server..."
        );
        assert_eq!(dialog_state(&m, None, None, false, "de_dust2").0, "Loading de_dust2 ...");
        let (stage, text) = detail_stage(Some(&JoinProgress {
            stage: JoinStage::Downloading,
            download: j.download.clone(),
            ..default()
        }), None);
        assert_eq!(stage, "downloading the map");
        assert_eq!(text, "0 KB of 1 KB (25%) of maps/a.bsp from server");
    }

    #[test]
    fn a_failure_shows_until_closed() {
        use crate::net::client::JoinFailure;
        let mut m = main_menu();
        assert_eq!(net_failure_text(&m, &JoinFailure::Full, "Server is full."), "Server is full.");
        m.failure = Some("Server is full.".into());
        m.loading = Some(String::new());
        assert!(m.handle(Input::Click(Target::Main(0), 0)).lines.is_empty());
        assert!(m.failure.is_some(), "only Close closes it");
        let out = m.handle(Input::Click(Target::Cancel, 0));
        assert!(out.lines.is_empty(), "already disconnected");
        assert!(m.failure.is_none() && m.loading.is_none() && m.open);
    }

    /// A shown entry's index.
    fn at(m: &GameMenu, item: MainItem) -> usize {
        m.entries().iter().position(|e| e.item == item).unwrap()
    }

    /// Click a shown entry.
    fn click(m: &mut GameMenu, item: MainItem) -> Outcome {
        let i = at(m, item);
        press(m, &[Input::Click(Target::Main(i), 0)])
    }

    fn shown(m: &GameMenu) -> Vec<MainItem> {
        m.entries().iter().map(|e| e.item).collect()
    }

    fn press(m: &mut GameMenu, inputs: &[Input]) -> Outcome {
        let mut all = Outcome::default();
        for i in inputs {
            let o = m.handle(*i);
            all.lines.extend(o.lines);
            all.after_load.extend(o.after_load);
            all.close |= o.close;
        }
        all
    }

    fn setting(cvar: &str) -> usize {
        SETTINGS.iter().position(|s| s.cvar == cvar).unwrap()
    }

    /// The row of the open page with this setting.
    fn row_of(m: &GameMenu, cvar: &str) -> usize {
        let i = setting(cvar);
        m.rows()
            .iter()
            .position(|r| matches!(r, Row::Value { field: Field::Setting(s), .. } if *s == i))
            .unwrap()
    }

    #[test]
    fn opens_with_the_game_as_it_is() {
        let m = menu();
        assert!(m.open);
        assert_eq!((m.page, m.focus), (Page::Main, 0));
        assert_eq!(m.new_game.map, 1, "the loaded map");
        assert_eq!(m.new_game.mode, Mode::Rounds);
        assert_eq!(m.new_game.difficulty, NORMAL);
        // No crosshair colour cvar given: shown as not available.
        assert_eq!(m.values[setting("cl_crosshaircolor")], None);
        assert_eq!(m.entries().len(), MAIN.len() + OURS.len());
    }

    #[test]
    fn in_game_entries_show_only_in_a_game() {
        use MainItem::*;
        let m = menu();
        assert_eq!(
            shown(&m),
            [Resume, Disconnect, FindServers, NewGame, BugReport, Options, Quit, QuickStart, Greybox, Bots, Team, Console]
        );
        let e = m.entries();
        assert!(e[2].gap && e[2].enabled, "a gap under Disconnect, then Find Servers");
        assert!(e[7].gap, "ours after a gap");
        let m = main_menu();
        assert_eq!(shown(&m), [FindServers, NewGame, BugReport, Options, Quit, QuickStart, Greybox, Console]);
        let e = m.entries();
        assert!(!e[0].gap, "no gap over the first entry");
        assert_eq!(m.focus, 0, "the first entry: Find Servers");
        assert_eq!(e[1].label, "CREATE SERVER");
    }

    #[test]
    fn find_servers_opens_the_server_browser() {
        let mut m = main_menu();
        let find = at(&m, MainItem::FindServers);
        let o = press(&mut m, &[Input::Hover(Target::Main(find)), Input::Click(Target::Main(find), 0)]);
        assert_eq!(o.lines, ["openserverbrowser"]);
        assert_eq!(m.focus, find);
        assert!(m.open && m.page == Page::Main, "the browser shows over the menu");
        // Up from it wraps to Console.
        press(&mut m, &[Input::Up]);
        assert_eq!(m.entries()[m.focus].item, MainItem::Console);
    }

    #[test]
    fn the_main_menu_stays_open() {
        let mut m = main_menu();
        let o = press(&mut m, &[Input::Close]);
        assert!(!o.close && m.open && m.page == Page::Main, "Esc does nothing there");
        // Esc closes a dialog, back to the main menu.
        click(&mut m, MainItem::Options);
        assert_eq!(m.page, Page::Settings);
        let o = press(&mut m, &[Input::Close]);
        assert!(!o.close && m.open && m.page == Page::Main);
        // A bug report from it keeps it open.
        let o = click(&mut m, MainItem::BugReport);
        assert_eq!(o.lines, ["bugreport"]);
        assert!(m.open && !o.close);
        // The console entry opens the console over it.
        let o = click(&mut m, MainItem::Console);
        assert_eq!(o.lines, ["toggleconsole"]);
        assert!(m.open);
    }

    #[test]
    fn create_server_starts_a_map_from_the_main_menu() {
        let mut m = main_menu();
        click(&mut m, MainItem::NewGame);
        assert_eq!(m.page, Page::NewGame);
        let start = m.rows().iter().position(|r| matches!(r, Row::Button { action: Action::Start, .. })).unwrap();
        let o = press(&mut m, &[Input::Click(Target::Row(start), 0)]);
        assert_eq!(o.lines.last().map(String::as_str), Some("map cs_office"));
        // The menu stays up, loading, and takes no input until the map is in.
        assert!(!o.close && m.open);
        assert_eq!(m.loading.as_deref(), Some("cs_office"));
        assert_eq!(press(&mut m, &[Input::Close, Input::Activate]), Outcome::default());
        assert!(m.enter_game(), "the load closes the open menu");
        assert!(m.in_game && !m.open && m.loading.is_none());
        // Esc now opens the in-game menu, with Resume.
        m.open(Page::Main, vec!["cs_office".into()], Some("cs_office"), |_| None);
        assert_eq!(m.entries()[m.focus].item, MainItem::Resume);
    }

    #[test]
    fn quick_start_and_the_greybox() {
        let mut m = main_menu();
        let o = click(&mut m, MainItem::QuickStart);
        assert_eq!(
            o.lines,
            ["bot_kick", "mashup_rounds 1", "bot_reaction 0.35", "bot_aim_error 2.5", "bot_turn_rate 360", "map de_dust2"]
        );
        assert_eq!(o.after_load.len(), (QUICK_BOTS.0 + QUICK_BOTS.1) as usize);
        assert_eq!(m.loading.as_deref(), Some(QUICK_MAP));
        let mut m = main_menu();
        let o = click(&mut m, MainItem::Greybox);
        assert_eq!(o.lines, ["map greybox"]);
        assert_eq!(m.loading.as_deref(), Some("greybox"));
        // From a game, the greybox closes the menu at once.
        let mut m = menu();
        let o = click(&mut m, MainItem::Greybox);
        assert!(o.close && !m.open && o.lines == ["map greybox"]);
    }

    /// `disconnect` and `map greybox` on a world (headless): the map gives
    /// way to the greybox, the menu follows.
    #[test]
    fn disconnect_and_the_greybox_on_the_world() {
        let mut app = App::new();
        app.add_plugins(crate::console::ConsolePlugin)
            .insert_resource(GameMenu {
                in_game: true,
                ..default()
            })
            .init_resource::<RegrabCursor>()
            .insert_resource(crate::map::LoadedMapName("cs_source:de_dust2".into()));
        let w = app.world_mut();
        super::super::console::load_greybox(w);
        left_game(w);
        let parts = w
            .query_filtered::<(), With<crate::greybox::GreyboxPart>>()
            .iter(w)
            .count();
        assert!(parts > 0, "the greybox is back behind the main menu");
        assert_eq!(w.resource::<crate::map::LoadedMapName>().0, "greybox");
        let m = w.resource::<GameMenu>();
        assert!(m.open && !m.in_game && m.page == Page::Main);
        assert!(!shown(m).contains(&MainItem::Resume));
        // The greybox from the main menu: in a game, the menu closed and
        // the mouse grabbed again.
        w.resource_mut::<GameMenu>().loading = Some("greybox".into());
        entered_game(w);
        let m = w.resource::<GameMenu>();
        assert!(m.in_game && !m.open && m.loading.is_none());
        assert!(w.resource::<RegrabCursor>().0);
        // A failed load gives the main menu back.
        let mut m = w.resource_mut::<GameMenu>();
        m.leave_game();
        m.loading = Some("nope".into());
        map_load_failed(w);
        assert!(w.resource::<GameMenu>().loading.is_none());
    }

    #[test]
    fn disconnect_returns_to_the_main_menu() {
        let mut m = menu();
        let o = click(&mut m, MainItem::Disconnect);
        assert_eq!(o.lines, ["disconnect"]);
        assert!(!o.close && m.open && !m.in_game && m.page == Page::Main);
        assert!(!shown(&m).contains(&MainItem::Resume) && !shown(&m).contains(&MainItem::Disconnect));
        assert!(m.entries()[m.focus].enabled);
        // Esc no longer leaves the menu.
        press(&mut m, &[Input::Close]);
        assert!(m.open);
    }

    #[test]
    fn normal_difficulty_is_the_bots_default() {
        let c = crate::bot::BotConfig::default();
        let (_, r, a, t) = DIFFICULTIES[NORMAL];
        assert_eq!((r, a, t), (c.reaction, c.aim_error, c.turn_rate));
    }

    #[test]
    fn escape_and_resume_close_with_no_lines() {
        let mut m = menu();
        let o = m.handle(Input::Close);
        assert!(o.close && o.lines.is_empty() && !m.open);
        let mut m = menu();
        let o = m.handle(Input::Activate);
        assert!(o.close && o.lines.is_empty() && !m.open);
        // A closed menu ignores input.
        assert_eq!(m.handle(Input::Activate), Outcome::default());
    }

    #[test]
    fn keys_move_through_pages_and_back() {
        let mut m = menu();
        click(&mut m, MainItem::Bots);
        assert_eq!(m.page, Page::Bots);
        // The info line can't be focused.
        assert_eq!(m.focus, 1);
        press(&mut m, &[Input::Up]);
        assert_eq!(m.focus, 4, "wraps past the info line to OK");
        press(&mut m, &[Input::Back]);
        assert_eq!((m.page, m.focus), (Page::Main, at(&m, MainItem::Bots)), "back on its entry");
        // Up past ours to Quit.
        press(&mut m, &[Input::Up, Input::Up, Input::Up]);
        assert_eq!(m.entries()[m.focus].item, MainItem::Quit);
        assert_eq!(press(&mut m, &[Input::Activate]).lines, ["quit"]);
    }

    #[test]
    fn bots_and_team_pages_run_their_commands() {
        let mut m = menu();
        click(&mut m, MainItem::Bots);
        assert_eq!(m.page, Page::Bots);
        let o = press(
            &mut m,
            &[
                Input::Click(Target::Row(1), 0),
                Input::Click(Target::Row(2), 0),
                Input::Click(Target::Row(3), 0),
            ],
        );
        assert_eq!(o.lines, ["bot_add 1", "bot_add 2", "bot_kick"]);
        assert!(!o.close && m.open, "adding bots keeps the menu open");
        // Info rows ignore clicks.
        assert!(press(&mut m, &[Input::Click(Target::Row(0), 0)]).lines.is_empty());

        click(&mut m, MainItem::Team);
        assert_eq!(m.page, Page::Team);
        let o = press(&mut m, &[Input::Down, Input::Activate]);
        assert_eq!(o.lines, ["jointeam 3"]);
        assert!(o.close && !m.open, "joining a team goes back to playing");

        // Auto-assign joins the smaller team.
        let mut m = menu();
        m.set_counts([0, 0], [3, 1]);
        click(&mut m, MainItem::Team);
        assert_eq!(press(&mut m, &[Input::Click(Target::Row(2), 0)]).lines, ["jointeam 3"]);
    }

    #[test]
    fn a_new_game_loads_the_map_then_adds_bots() {
        let mut m = menu();
        click(&mut m, MainItem::NewGame);
        assert_eq!(m.page, Page::NewGame);
        // Map: one to the right (de_nuke); mode back to deathmatch; 2 T,
        // 1 CT; hard.
        let o = press(
            &mut m,
            &[
                Input::Right,
                Input::Down,
                Input::Right,
                Input::Down,
                Input::Right,
                Input::Right,
                Input::Down,
                Input::Click(Target::Row(3), 1),
                Input::Click(Target::Row(4), 1),
            ],
        );
        assert!(o.lines.is_empty(), "nothing runs before Start: {:?}", o.lines);
        assert_eq!(
            m.new_game,
            NewGame {
                map: 2,
                mode: Mode::Deathmatch,
                bots_t: 2,
                bots_ct: 1,
                difficulty: 2,
            }
        );
        let o = press(&mut m, &[Input::Click(Target::Row(5), 0)]);
        assert_eq!(
            o.lines,
            [
                "bot_kick",
                "mashup_rounds 0",
                "bot_reaction 0.2",
                "bot_aim_error 1.5",
                "bot_turn_rate 540",
                "map de_nuke",
            ]
        );
        assert_eq!(o.after_load, ["bot_add 1", "bot_add 1", "bot_add 2"]);
        assert!(o.close && !m.open);
    }

    #[test]
    fn bot_counts_and_difficulty_stay_in_range() {
        let mut m = menu();
        click(&mut m, MainItem::NewGame);
        for _ in 0..3 {
            press(&mut m, &[Input::Click(Target::Row(2), -1), Input::Click(Target::Row(4), -1)]);
        }
        assert_eq!((m.new_game.bots_t, m.new_game.difficulty), (0, 0));
        for _ in 0..40 {
            press(&mut m, &[Input::Click(Target::Row(2), 1), Input::Click(Target::Row(4), 1)]);
        }
        assert_eq!((m.new_game.bots_t, m.new_game.difficulty), (MAX_BOTS, 3));
    }

    #[test]
    fn the_map_list_picks_by_key_letter_and_click() {
        let mut m = menu();
        { click(&mut m, MainItem::NewGame); press(&mut m, &[Input::Activate]) };
        assert_eq!((m.page, m.focus), (Page::Maps, 1), "on the chosen map");
        press(&mut m, &[Input::Char('c'), Input::Activate]);
        assert_eq!((m.page, m.new_game.map), (Page::NewGame, 0));
        press(&mut m, &[Input::Click(Target::Row(0), 0), Input::Click(Target::Row(2), 0)]);
        assert_eq!((m.page, m.new_game.map), (Page::NewGame, 2));
        // Letters on the map row jump too.
        press(&mut m, &[Input::Char('d')]);
        assert_eq!(m.new_game.map, 1);
        // Backspace from the map list goes back to the new game page.
        press(&mut m, &[Input::Activate, Input::Back]);
        assert_eq!(m.page, Page::NewGame);
    }

    #[test]
    fn long_map_lists_scroll_with_the_focus_wheel_and_bar() {
        let mut m = GameMenu::default();
        let maps: Vec<String> = (0..50).map(|i| format!("map{i:02}")).collect();
        m.open(Page::Maps, maps, Some("map45"), |_| None);
        assert_eq!(m.focus, 45);
        assert_eq!(m.scroll, 45 + 1 - MAP_ROWS, "the chosen map shows");
        press(&mut m, &[Input::Scroll(-100)]);
        assert_eq!((m.scroll, m.focus), (0, 45), "the wheel scrolls without moving the focus");
        press(&mut m, &[Input::Click(Target::Scroll, MAP_ROWS as i32)]);
        assert_eq!(m.scroll, MAP_ROWS);
        press(&mut m, &[Input::Scroll(1000)]);
        assert_eq!(m.scroll, 50 - MAP_ROWS, "not past the end");
        // Left and right go a screenful at a time.
        press(&mut m, &[Input::Left]);
        assert_eq!(m.focus, 45 - MAP_ROWS);
        press(&mut m, &[Input::Left, Input::Left, Input::Left]);
        assert_eq!((m.focus, m.scroll), (0, 0));
        // A click picks.
        press(&mut m, &[Input::Click(Target::Row(3), 0)]);
        assert_eq!((m.page, m.new_game.map), (Page::NewGame, 3));
    }

    #[test]
    fn no_maps_no_start() {
        let mut m = GameMenu::default();
        m.open(Page::NewGame, Vec::new(), None, |_| None);
        let start = m.rows().iter().position(|r| matches!(r, Row::Button { action: Action::Start, .. }));
        let o = press(&mut m, &[Input::Click(Target::Row(start.unwrap()), 0)]);
        assert!(o.lines.is_empty() && m.open);
        // Activating the map row doesn't open an empty list.
        press(&mut m, &[Input::Click(Target::Row(0), 0)]);
        assert_eq!(m.page, Page::NewGame);
    }

    #[test]
    fn settings_set_their_cvars_at_once() {
        let mut m = menu();
        click(&mut m, MainItem::Options);
        assert_eq!((m.page, m.tab), (Page::Settings, Tab::Keyboard));
        press(&mut m, &[Input::NextTab(1)]);
        assert_eq!((m.tab, m.focus), (Tab::Mouse, 0));
        let o = press(&mut m, &[Input::Right, Input::Right, Input::Down, Input::Activate, Input::Down, Input::Left]);
        assert_eq!(
            o.lines,
            ["sensitivity 3.1", "sensitivity 3.2", "m_pitch -0.022", "zoom_sensitivity_ratio 1.1"]
        );
        // A slider pressed halfway.
        let o = press(&mut m, &[Input::Slide(0, 1.0), Input::Slide(0, 1.0)]);
        assert_eq!(o.lines, ["sensitivity 20.0"], "once: dragging to the same place sets nothing");
        // Audio: volume stops at 1.
        press(&mut m, &[Input::Click(Target::Tab(2), 0)]);
        assert_eq!(m.tab, Tab::Audio);
        let o = press(&mut m, &[Input::Right]);
        assert!(o.lines.is_empty(), "{:?}", o.lines);
        // Multiplayer: hand, a choice, wrapping; shown by name.
        press(&mut m, &[Input::NextTab(1), Input::NextTab(1)]);
        assert_eq!(m.tab, Tab::Multiplayer);
        let hand = row_of(&m, "cl_righthand");
        let o = press(&mut m, &[Input::Click(Target::Row(hand), 1)]);
        assert_eq!(o.lines, ["cl_righthand 0"]);
        assert!(matches!(&m.rows()[hand], Row::Value { value, .. } if value == "Left"));
        // Crosshair colour isn't registered here: not focusable, skipped.
        assert!(matches!(m.rows()[0], Row::Info(_)));
        assert_eq!(m.focus, hand);
        // Video: a check box flips.
        press(&mut m, &[Input::NextTab(-1)]);
        let vsync = row_of(&m, "mat_vsync");
        let o = press(&mut m, &[Input::Click(Target::Row(vsync), 0), Input::Click(Target::Row(vsync), 0)]);
        assert_eq!(o.lines, ["mat_vsync 0", "mat_vsync 1"]);
        // OK goes back.
        let ok = m.rows().len() - 1;
        press(&mut m, &[Input::Click(Target::Row(ok), 0)]);
        assert_eq!(m.page, Page::Main);
    }

    fn keyboard() -> GameMenu {
        let mut m = menu();
        let mut b = BTreeMap::new();
        binds::bind_defaults(&mut b, true);
        m.set_binds(b, BTreeSet::from(["bugreport".to_string()]));
        click(&mut m, MainItem::Options);
        m
    }

    fn bind_row(m: &GameMenu, command: &str) -> usize {
        m.rows()
            .iter()
            .position(|r| matches!(r, Row::Bind { command: c, .. } if c == command))
            .unwrap()
    }

    #[test]
    fn the_keyboard_tab_lists_actions_with_their_keys() {
        let m = keyboard();
        let rows = m.rows();
        assert!(matches!(&rows[0], Row::Heading(t) if t == "Movement"));
        assert!(matches!(&rows[bind_row(&m, "+jump")], Row::Bind { keys, known: true, .. } if keys == "MWHEELDOWN, MWHEELUP, SPACE"));
        assert_eq!(m.focus, 1, "the first action, past the heading");
        assert_eq!(m.list(), (rows.len() - 4, KEY_ROWS));
    }

    #[test]
    fn rebinding_through_the_keyboard_tab() {
        let mut m = keyboard();
        let jump = bind_row(&m, "+jump");
        // One click selects, the next waits for a key.
        press(&mut m, &[Input::Click(Target::Row(jump), 0)]);
        assert_eq!((m.key_row, m.capture.as_deref()), (Some(jump), None));
        press(&mut m, &[Input::Click(Target::Row(jump), 0)]);
        assert_eq!(m.capture.as_deref(), Some("+jump"));
        // Keys while waiting go to the bind, not the menu.
        let o = press(&mut m, &[Input::Down, Input::BindKey("f")]);
        assert_eq!(o.lines, ["unbind mwheeldown; unbind mwheelup; unbind space; bind f +jump"]);
        assert_eq!(m.capture, None);
        assert!(matches!(&m.rows()[jump], Row::Bind { keys, .. } if keys == "F"));
        // Esc while waiting gives up, leaving the menu open.
        press(&mut m, &[Input::Activate]);
        assert!(m.capture.is_some());
        let o = press(&mut m, &[Input::Close]);
        assert!(o.lines.is_empty() && m.open && m.capture.is_none());
        // Edit key and Clear key act on the selected action.
        let duck = bind_row(&m, "+duck");
        press(&mut m, &[Input::Click(Target::Row(duck), 0)]);
        let rows = m.rows();
        let edit = rows.iter().position(|r| matches!(r, Row::Button { action: Action::EditKey, .. })).unwrap();
        let clear = edit + 1;
        let o = press(&mut m, &[Input::Click(Target::Row(clear), 0)]);
        assert_eq!(o.lines, ["unbind ctrl"]);
        press(&mut m, &[Input::Click(Target::Row(edit), 0)]);
        assert_eq!(m.capture.as_deref(), Some("+duck"));
        let o = press(&mut m, &[Input::BindKey("mouse4")]);
        assert_eq!(o.lines, ["bind mouse4 +duck"]);
        // Use Defaults puts every bind back.
        let o = press(&mut m, &[Input::Click(Target::Row(edit - 1), 0)]);
        assert_eq!(o.lines, ["binddefaults"]);
        assert_eq!(m.binds.get("space").map(String::as_str), Some("+jump"));
        assert_eq!(m.binds.get("mouse4"), None);
    }

    #[test]
    fn the_keyboard_list_scrolls_to_the_focus() {
        let mut m = keyboard();
        for _ in 0..20 {
            press(&mut m, &[Input::Down]);
        }
        let (len, shown) = m.list();
        assert!(m.focus < len);
        assert!(m.scroll <= m.focus && m.focus < m.scroll + shown, "{} in {}+{shown}", m.focus, m.scroll);
    }

    #[test]
    fn the_games_menu_entries_with_ours_added() {
        let item = |label: &str, command: &str| GameUiItem {
            label: label.into(),
            command: command.into(),
            in_game_only: false,
        };
        let ui = GameUi {
            menu: vec![
                item("RESUME GAME", "ResumeGame"),
                item("DISCONNECT", "Disconnect"),
                item("", ""),
                item("FIND SERVERS", "OpenServerBrowser"),
                item("CREATE SERVER", "OpenCreateMultiplayerGameDialog"),
                item("REPORT BUG", "engine bug"),
                item("OPTIONS", "OpenOptionsDialog"),
                item("QUIT", "Quit"),
            ],
            strings: HashMap::from([("gameui_gamemenu_newgame".to_string(), "NEW GAME".to_string())]),
            ..default()
        };
        let e = main_entries(Some(&ui));
        let items: Vec<MainItem> = e.iter().map(|e| e.item).collect();
        use MainItem::*;
        assert_eq!(
            items,
            [Resume, Disconnect, FindServers, NewGame, BugReport, Options, Quit, QuickStart, Greybox, Bots, Team, Console]
        );
        assert_eq!(e[3].label, "CREATE SERVER", "the game's words");
        assert!(e[2].gap && !e[2].gap_in_game_only, "the file's blank entry");
        assert!(e[0].in_game_only && e[1].in_game_only, "resume and disconnect only in a game");
        assert_eq!(e[4].label, "REPORT BUG");
        // Without the file: CS:S's entries, ours.
        assert_eq!(main_entries(None).len(), MAIN.len() + OURS.len());
        let mut m = menu();
        m.set_ui(Some(Arc::new(ui)));
        press(&mut m, &[Input::Click(Target::Main(5), 0)]);
        assert_eq!(m.page, Page::Settings);
        press(&mut m, &[Input::Back]);
        assert_eq!(m.focus, 5, "back on its entry");
    }

    #[test]
    fn the_install_menu_file_hides_its_gap_out_of_a_game() {
        let item = |label: &str, command: &str, in_game_only: bool| GameUiItem {
            label: label.into(),
            command: command.into(),
            in_game_only,
        };
        // As CS:S's GameMenu.res: the blank entry is only for a game.
        let ui = GameUi {
            menu: vec![
                item("RESUME GAME", "ResumeGame", true),
                item("DISCONNECT", "Disconnect", true),
                item("PLAYERS", "OpenPlayerListDialog", true),
                item("", "", true),
                item("FIND SERVERS", "OpenServerBrowser", false),
                item("CREATE SERVER", "OpenCreateMultiplayerGameDialog", false),
                item("ACHIEVEMENTS", "OpenCSAchievementsDialog", false),
                item("QUIT", "Quit", false),
            ],
            ..default()
        };
        let mut m = main_menu();
        m.set_ui(Some(Arc::new(ui)));
        let e = m.entries();
        assert_eq!(e[0].item, MainItem::FindServers);
        assert!(!e[0].gap);
        m.in_game = true;
        let e = m.entries();
        assert_eq!(e[2].item, MainItem::FindServers);
        assert!(e[2].gap);
        assert!(!e.iter().any(|e| e.label == "ACHIEVEMENTS" || e.label == "PLAYERS"), "what mashup lacks is left out");
    }

    #[test]
    fn hover_moves_focus_only_on_its_page() {
        let mut m = menu();
        let options = at(&m, MainItem::Options);
        press(&mut m, &[Input::Hover(Target::Main(options))]);
        assert_eq!(m.focus, options);
        press(&mut m, &[Input::Activate, Input::NextTab(1)]);
        assert_eq!(m.page, Page::Settings);
        // The left-hand list doesn't take the focus from the dialog.
        press(&mut m, &[Input::Hover(Target::Main(1)), Input::Hover(Target::Row(2))]);
        assert_eq!(m.focus, 2);
    }

    #[test]
    fn escape_opens_and_closes_and_the_mouse_follows() {
        use bevy::window::CursorGrabMode;
        let mut app = App::new();
        app.add_plugins(crate::console::ConsolePlugin)
            .init_resource::<super::super::console::ConsoleUi>()
            .init_resource::<ButtonInput<KeyCode>>()
            .init_resource::<ButtonInput<MouseButton>>()
            .insert_resource(GameMenu {
                in_game: true,
                ..default()
            })
            .init_resource::<AfterLoad>()
            .init_resource::<RegrabCursor>()
            .add_systems(
                Update,
                (
                    keys.before(super::super::console::toggle),
                    super::super::console::toggle,
                    cursor,
                )
                    .chain(),
            );
        app.world_mut()
            .spawn((super::super::console::ConsoleRoot, Visibility::Hidden));
        let window = app
            .world_mut()
            .spawn(CursorOptions {
                grab_mode: CursorGrabMode::Locked,
                ..default()
            })
            .id();
        let press = |app: &mut App, key: KeyCode| {
            app.world_mut().resource_mut::<ButtonInput<KeyCode>>().press(key);
            app.update();
            let mut keys = app.world_mut().resource_mut::<ButtonInput<KeyCode>>();
            keys.release(key);
            keys.clear();
        };
        let grabbed = |app: &App| app.world().get::<CursorOptions>(window).unwrap().grab_mode != CursorGrabMode::None;
        let open = |app: &App| app.world().resource::<GameMenu>().open;

        press(&mut app, KeyCode::Escape);
        assert!(open(&app) && !grabbed(&app), "Esc opens the menu and frees the mouse");
        press(&mut app, KeyCode::Escape);
        assert!(!open(&app) && grabbed(&app), "Esc closes it and grabs the mouse");

        // Esc in the console only closes the console.
        press(&mut app, KeyCode::Backquote);
        press(&mut app, KeyCode::Escape);
        assert!(!open(&app) && !app.world().resource::<super::super::console::ConsoleUi>().open);

        // Resume clicked: the mouse is grabbed once the button is up, so
        // the click doesn't fire.
        press(&mut app, KeyCode::Escape);
        assert!(open(&app));
        app.world_mut()
            .resource_mut::<ButtonInput<MouseButton>>()
            .press(MouseButton::Left);
        press(&mut app, KeyCode::Enter);
        assert!(!open(&app) && !grabbed(&app));
        app.world_mut()
            .resource_mut::<ButtonInput<MouseButton>>()
            .release(MouseButton::Left);
        app.update();
        assert!(grabbed(&app));
    }

    /// Rebinding in the open menu with real keys: the console's binds
    /// change, and the new key then drives the action (headless).
    #[test]
    fn a_key_rebound_in_the_options_drives_the_action() {
        let mut app = App::new();
        app.add_plugins(crate::console::ConsolePlugin)
            .init_resource::<super::super::console::ConsoleUi>()
            .init_resource::<ButtonInput<KeyCode>>()
            .insert_resource(GameMenu {
                in_game: true,
                ..default()
            })
            .init_resource::<AfterLoad>()
            .init_resource::<RegrabCursor>()
            .add_systems(Update, keys.before(super::super::console::toggle))
            .add_systems(Update, super::super::console::toggle);
        super::super::binds::test_binds(&mut app);
        binds::commands(&mut app);
        app.world_mut()
            .spawn((super::super::console::ConsoleRoot, Visibility::Hidden));
        let tap = |app: &mut App, key: KeyCode| {
            app.world_mut().resource_mut::<ButtonInput<KeyCode>>().press(key);
            app.update();
            let mut keys = app.world_mut().resource_mut::<ButtonInput<KeyCode>>();
            keys.release(key);
            keys.clear();
            app.update();
        };
        // Open on the options (the keyboard tab), as `menu keyboard` does.
        app.world_mut().commands().queue(|w: &mut World| {
            open_menu(w, Page::Settings);
        });
        app.update();
        assert!(app.world().resource::<GameMenu>().open);
        // To "Jump", Enter to wait for a key, then K.
        let jump = bind_row(app.world().resource::<GameMenu>(), "+jump");
        for _ in 1..jump {
            tap(&mut app, KeyCode::ArrowDown);
        }
        assert_eq!(app.world().resource::<GameMenu>().focus, jump);
        tap(&mut app, KeyCode::Enter);
        assert_eq!(app.world().resource::<GameMenu>().capture.as_deref(), Some("+jump"));
        tap(&mut app, KeyCode::KeyK);
        let console = app.world().resource::<Console>();
        assert_eq!(console.binds.get("k").map(String::as_str), Some("+jump"));
        assert_eq!(console.binds.get("space"), None);
        assert!(console.dirty, "saved to config.cfg on quit");
        // K now holds jump; Space doesn't.
        let mut keys = ButtonInput::<KeyCode>::default();
        let mouse = ButtonInput::<MouseButton>::default();
        keys.press(KeyCode::KeyK);
        assert!(binds::pressed(&console.binds, &keys, &mouse, "+jump"));
        keys.release(KeyCode::KeyK);
        keys.press(KeyCode::Space);
        assert!(!binds::pressed(&console.binds, &keys, &mouse, "+jump"));
    }

    #[test]
    fn bug_report_closes_first() {
        let mut m = menu();
        let o = click(&mut m, MainItem::BugReport);
        assert_eq!(o.lines, ["bugreport"]);
        assert!(o.close && !m.open);
    }
}
