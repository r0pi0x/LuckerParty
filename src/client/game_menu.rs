//! The main menu and the in-game menu (Esc, or `menu [page]`), one menu
//! drawn as CS:S's GameUI draws it. Out of a game (startup without a map,
//! after `disconnect`) it is the main menu: always open, over the game's
//! background picture, the game's title over its entries, in-game entries
//! hidden. In a game: the entries at the left over the darkened game. Each
//! dialog is a VGUI frame (`widgets`: moved by its title bar, brought to
//! the front by a click, closed from its X) laid out from the install's
//! layouts: the options (`OptionsSub*.res` per tab, OK / Cancel / Apply,
//! the keyboard and video tabs' Advanced dialogs), Create Server (its
//! Server, Game and Bot pages: `CreateMultiplayerGame*Page.res`, the Game
//! page's options from `cfg/settings.scr`); controls the game has and
//! mashup lacks are drawn greyed. Ours (settings CS:S's options don't
//! have) are in our own dialog, Lucker Party Options, from our entries.
//! In the install's GameUI scheme (`map::hud::GameUi`); without the
//! install, in built-in colours and words, controls in a column. The game
//! keeps running while it is open; the mouse is free, so the local
//! player's input is ignored (`input::write_local_intent`). Every choice
//! is a console line (`map`, `bot_add`, `bind`, cvars ...), so the menu
//! holds no game logic: the model (`GameMenu::handle`) turns keys and
//! clicks into those lines, and is unit-tested without a window.

use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    sync::{Arc, Mutex},
};

use bevy::{
    input::{
        ButtonState,
        keyboard::{Key, KeyboardInput},
        mouse::AccumulatedMouseScroll,
    },
    prelude::*,
    text::LineBreak,
    ui::RelativeCursorPosition,
    window::CursorOptions,
};

pub(super) use super::widgets::{Look, bevel, label, place};
use super::{
    binds,
    fonts::UiFonts,
    options::{Place, SETTINGS, SettingKind, TABS, Tab, VALUE_ENTRIES},
    widgets::{
        self, ButtonState as Btn, Caret, ComboEvent, ComboKey, ComboList, Edit, FrameSpec, SliderDrag, SliderOwner,
        SliderTrack, WindowId, Windows,
    },
};
use crate::{
    console::{Console, ConsoleAppExt},
    core::{Intent, Team},
    map::hud::{GameUi, KeyAction, ServerSettingKind, UiControl, UiKind, UiLayout},
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
                    text_keys,
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
            "menu [main|newgame|game|bot|bots|team|options|keyboard|mouse|audio|video|multiplayer|advanced|\
             videoadvanced|extras]: open the game menu (Esc) on a page (newgame: Create Server, game and bot its \
             other pages; options: on a tab; advanced: the keyboard tab's Advanced dialog, videoadvanced the video \
             tab's; extras: Lucker Party Options).",
            |w, a| {
                let arg = a.first().map(|s| s.to_lowercase());
                let tab = TABS.iter().find(|(t, ..)| Some(t.page()) == arg.as_deref()).map(|(t, ..)| *t);
                let page = match arg.as_deref() {
                    _ if tab.is_some() => Page::Settings,
                    None | Some("main") => Page::Main,
                    Some("newgame" | "createserver" | "game" | "bot") => Page::NewGame,
                    Some("bots") => Page::Bots,
                    Some("team") => Page::Team,
                    Some("options" | "settings" | "advanced" | "videoadvanced") => Page::Settings,
                    Some("extras") => Page::Extras,
                    Some(p) => return Err(format!("no menu page \"{p}\"")),
                };
                open_menu(w, page);
                let mut menu = w.resource_mut::<GameMenu>();
                if let Some(tab) = tab {
                    menu.set_tab(tab);
                }
                match arg.as_deref() {
                    Some("game") => menu.set_create_tab(1),
                    Some("bot") => menu.set_create_tab(2),
                    Some("advanced") => {
                        menu.set_tab(Tab::Keyboard);
                        menu.press_action(&Action::Advanced);
                    }
                    Some("videoadvanced") => {
                        menu.set_tab(Tab::Video);
                        menu.press_action(&Action::VideoAdvanced);
                    }
                    _ => {}
                }
                Ok(None)
            },
        )
        .console_command(
            "menuinput",
            "menuinput <input>: drive the open game menu as keys and clicks would (screenshots, tests): \
             focus <cvar|map|bots|botcount|botteam|difficulty> | open (the focused combo box's list) | \
             down | up | enter | space | escape | tab | backtab | type <text> | pick <n> | sheet <n>; prints \
             the focused control.",
            |w, a| {
                let input = menu_input(&w.resource::<GameMenu>(), a)?;
                let mut menu = w.resource_mut::<GameMenu>();
                let mut next = menu.clone();
                let out = next.handle(input);
                *menu = next;
                let state = menu.describe();
                let mut console = w.resource_mut::<Console>();
                for line in out.lines {
                    console.submit(line);
                }
                Ok(Some(state))
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
    /// Create Server (its page: `GameMenu::create_tab`).
    NewGame,
    Bots,
    Team,
    /// The options dialog (its tab: `GameMenu::tab`).
    Settings,
    /// The keyboard tab's Advanced dialog (over the options).
    KeyboardAdvanced,
    /// The video tab's Advanced dialog (over the options).
    VideoAdvanced,
    /// Ours: Lucker Party Options (what CS:S's options don't have).
    Extras,
}

impl Page {
    /// Its frame (`widgets::Windows` keeps where it was dragged).
    pub fn window(self) -> WindowId {
        match self {
            Page::Main => "main",
            Page::NewGame => "createserver",
            Page::Bots => "bots",
            Page::Team => "team",
            Page::Settings => "options",
            Page::KeyboardAdvanced => "keyboardadvanced",
            Page::VideoAdvanced => "videoadvanced",
            Page::Extras => "extras",
        }
    }

    /// A dialog over the options (modal to them).
    fn over_options(self) -> bool {
        matches!(self, Page::KeyboardAdvanced | Page::VideoAdvanced)
    }
}

/// Bot skill presets: reaction seconds, aim error degrees, turn rate
/// degrees per second (`bot_reaction`, `bot_aim_error`, `bot_turn_rate`).
/// Normal is the bots' default (`bot::BotConfig`). Create Server's four
/// difficulty buttons pick one (`#Cstrike_Bot_Difficulty0..3`).
pub const DIFFICULTIES: [(&str, f32, f32, f32); 4] = [
    ("Easy", 0.6, 5.0, 220.0),
    ("Normal", 0.35, 2.5, 360.0),
    ("Hard", 0.2, 1.5, 540.0),
    ("Expert", 0.12, 0.75, 720.0),
];
const NORMAL: usize = 1;
/// Bots a new game offers at most (the box takes two digits).
const MAX_BOTS: u8 = 30;

/// Create Server's pages: the token of each tab and our word.
pub const CREATE_TABS: [(&str, &str); 3] = [("#GameUI_Server", "Server"), ("#GameUI_Game", "Game"), ("", "Bot")];

/// Which team bots join (Create Server's Bot page): any (split between
/// them), terrorists, counter-terrorists.
pub const BOT_TEAMS: [(&str, &str); 3] = [
    ("", "Any"),
    ("#Cstrike_Team_T", "Terrorists"),
    ("#Cstrike_Team_CT", "Counter-Terrorists"),
];

/// A cvar Create Server sets when it starts (the Game page's options
/// from the install's `settings.scr`, the Bot page's cvars, ours): its
/// value shown and the one before.
#[derive(Clone, Debug, PartialEq)]
pub struct ServerCvar {
    pub cvar: String,
    pub label: String,
    pub kind: ServerSettingKind,
    pub value: String,
    /// Its value when the dialog opened (None: mashup lacks it, greyed).
    pub before: Option<String>,
    /// The page it's on (index into `CREATE_TABS`).
    pub page: usize,
    /// The layout control showing it (Bot page), else a row of the Game
    /// page's list.
    pub field: Option<&'static str>,
}

/// The Bot page's cvars, by its layout's controls (fieldName, cvar, kind).
const BOT_CVARS: [(&str, &str, &str); 2] = [
    ("BotPrefixEntry", "bot_prefix", "#CStrike_Bot_NamePrefix"),
    ("BotJoinAfterPlayerCheck", "bot_join_after_player", "#CStrike_Bot_JoinAfterPlayer"),
];

/// Ours on the Game page, after the game's own: rounds or deathmatch.
const ROUNDS_CVAR: &str = "mashup_rounds";

/// What a new game starts with.
#[derive(Clone, Debug, PartialEq)]
pub struct NewGame {
    /// Index into `GameMenu::maps`.
    pub map: usize,
    /// Include bots, how many (typed), which team they join (`BOT_TEAMS`).
    pub bots_on: bool,
    pub bot_count: String,
    pub bot_team: usize,
    /// Index into `DIFFICULTIES`.
    pub difficulty: usize,
    /// The Game and Bot pages' cvars.
    pub cvars: Vec<ServerCvar>,
}

impl Default for NewGame {
    fn default() -> Self {
        Self {
            map: 0,
            bots_on: false,
            bot_count: "0".into(),
            bot_team: 0,
            difficulty: NORMAL,
            cvars: Vec::new(),
        }
    }
}

impl NewGame {
    /// Bots per team: terrorists, counter-terrorists (any team: split,
    /// the odd one a terrorist).
    pub fn split(&self) -> (u8, u8) {
        if !self.bots_on {
            return (0, 0);
        }
        let n = self.bot_count.trim().parse::<u8>().unwrap_or(0).min(MAX_BOTS);
        match self.bot_team {
            1 => (n, 0),
            2 => (0, n),
            _ => (n.div_ceil(2), n / 2),
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
    /// Create Server.
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
    /// Ours: Lucker Party Options.
    Extras,
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
pub const OURS: [(MainItem, &str, bool); 6] = [
    (MainItem::QuickStart, "Quick Start", false),
    (MainItem::Greybox, "Greybox Test Map", false),
    (MainItem::Bots, "Bots", true),
    (MainItem::Team, "Team", true),
    (MainItem::Extras, "Lucker Party Options", false),
    (MainItem::Console, "Console", false),
];

/// The map a quick start plays (rounds, `QUICK_BOTS` normal bots: five
/// terrorists and four counter-terrorists besides you); the first map
/// when the install lacks it.
pub const QUICK_MAP: &str = "de_dust2";
const QUICK_BOTS: u8 = 9;

impl MainItem {
    fn page(self) -> Option<Page> {
        match self {
            MainItem::NewGame => Some(Page::NewGame),
            MainItem::Bots => Some(Page::Bots),
            MainItem::Team => Some(Page::Team),
            MainItem::Options => Some(Page::Settings),
            MainItem::Extras => Some(Page::Extras),
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
/// without the file); then, after a gap, ours (quick start, the greybox,
/// bots, team, our options, console). Entries mashup can't do (player
/// list, achievements, benchmark ...) are left out. `GameMenu::entries`
/// picks those shown in or out of a game.
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

/// A value a control changes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Field {
    /// Create Server's Server page: the map, include bots, how many, the
    /// difficulty buttons (0 to 3).
    Map,
    BotsOn,
    BotCount,
    Difficulty(usize),
    /// Its Bot page: which team bots join.
    BotTeam,
    /// A cvar Create Server sets (index into `NewGame::cvars`).
    ServerCvar(usize),
    /// Index into `options::SETTINGS`.
    Setting(usize),
    /// The text entry showing a slider setting's value (`VALUE_ENTRIES`).
    SettingText(usize),
}

/// How a control shows its value.
#[derive(Clone, Debug, PartialEq)]
pub enum Control {
    /// A combo box: its entries and the one shown (None: none of them,
    /// `text` shown).
    Combo {
        entries: Vec<String>,
        selected: Option<usize>,
        text: String,
    },
    Check(bool),
    /// One of a group of radio buttons.
    Radio(bool),
    Slider {
        fraction: f32,
        text: String,
    },
    /// A text entry (`numeric`: digits only; at most `max` chars).
    Text {
        text: String,
        numeric: bool,
        max: usize,
    },
}

/// What pressing a button row does.
#[derive(Clone, Debug, PartialEq)]
pub enum Action {
    Main(MainItem),
    /// Back to the main page (Bots, Team).
    Back,
    /// Create Server's Start.
    Start,
    /// Run a console line; close the menu too when set.
    Run(String, bool),
    /// Wait for a key for the selected keyboard action.
    EditKey,
    /// Unbind the selected keyboard action's keys.
    ClearKey,
    /// Every bind back to the defaults.
    DefaultBinds,
    /// The keyboard tab's Advanced dialog.
    Advanced,
    /// The video tab's Advanced dialog.
    VideoAdvanced,
    /// A dialog's OK: keep its changes, close it.
    Ok,
    /// A dialog's Cancel (Esc, its X): its changes undone, closed.
    Cancel,
    /// The options' Apply: keep the changes so far, stay open.
    Apply,
}

/// A row of a page: its controls in tab order.
#[derive(Clone, Debug, PartialEq)]
pub enum Row {
    Button { label: String, action: Action, enabled: bool },
    Control { label: String, field: Field, control: Control },
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
            Row::Control { .. } | Row::Bind { .. } => true,
            Row::Info(_) | Row::Heading(_) => false,
        }
    }

    fn field(&self) -> Option<Field> {
        match self {
            Row::Control { field, .. } => Some(*field),
            _ => None,
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
    /// A tab of the open dialog's property sheet (options, Create Server).
    Tab(usize),
    /// A list's scroll bar (the click's step says how far).
    Scroll,
    /// The loading dialog's Cancel button.
    Cancel,
    /// An entry of the open combo box's list.
    ComboItem(usize),
    /// Anywhere outside the open combo box's list (closes it).
    Outside,
    /// The open dialog's close box (its X).
    Close,
}

/// A key or pointer event for the menu.
#[derive(Clone, Debug, PartialEq)]
pub enum Input {
    Up,
    Down,
    Left,
    Right,
    /// Enter: the focused button or list row, else the dialog's default
    /// button (OK, Start).
    Activate,
    /// Space: the focused control (ticks a check box, opens a combo box,
    /// presses a button).
    Space,
    /// Backspace: back to the main page.
    Back,
    /// Esc: cancel (close a combo box's list, stop waiting for a key,
    /// cancel the dialog, close the menu in a game).
    Close,
    Hover(Target),
    /// A click: on a row, a tab, the scroll bar (its step: rows), a combo
    /// box's list entry, outside an open list.
    Click(Target, i32),
    /// A typed letter or digit: jumps a combo box's list or the focused
    /// one.
    Char(char),
    /// Text typed or pasted into the focused text entry.
    Type(String),
    /// An editing key in the focused text entry.
    Edit(Edit),
    /// Ctrl+C / Ctrl+X in the focused text entry.
    Copy,
    Cut,
    /// The caret placed in a text entry row (a click on its char).
    Caret(usize, usize),
    /// Ctrl+Tab (+1) / Ctrl+Shift+Tab (-1): the property sheet's next tab.
    NextTab(i32),
    /// Tab (+1) / Shift+Tab (-1): the next control in the tab order.
    Focus(i32),
    /// A row focused (scripts: `menuinput focus`).
    FocusRow(usize),
    /// The wheel over a list or an open combo box's list: rows to scroll.
    Scroll(i32),
    /// The wheel over a combo box row: steps it.
    Wheel(usize, i32),
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
    /// Text for the clipboard (copied or cut).
    pub clipboard: Option<String>,
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
    /// Create Server's page (index into `CREATE_TABS`).
    pub create_tab: usize,
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
    /// The first row of the open list shown (keyboard actions, Create
    /// Server's Game page).
    pub scroll: usize,
    /// Window sizes the video tab offers.
    pub resolutions: Vec<String>,
    /// The options' settings as they were when it opened or was last
    /// applied (`SETTINGS` index, value): its Cancel puts them back.
    pub options_before: Vec<(usize, Option<String>)>,
    /// The same for the Advanced dialog open over it.
    pub advanced_before: Vec<(usize, Option<String>)>,
    /// The open combo box's list (its owner: the row).
    pub combo: Option<ComboList>,
    /// The focused text entry's caret and selection.
    pub caret: Caret,
    /// A value entry's text as typed (it sets its setting once it reads
    /// as one), while focused.
    pub editing: Option<(Field, String)>,
}

/// Keyboard list rows shown at once.
pub const KEY_ROWS: usize = 14;
/// Buttons under the keyboard list (Use defaults, Edit key, Clear key,
/// Advanced).
const KEY_BUTTONS: usize = 4;
/// Rows of Create Server's Game page list shown at once.
pub const GAME_ROWS: usize = 12;

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

/// The layout name of the open page (`GameUi::options`).
fn layout_name(page: Page, tab: Tab, create_tab: usize) -> Option<&'static str> {
    match page {
        Page::Settings => Some(tab.page()),
        Page::KeyboardAdvanced => Some("keyboard_advanced"),
        Page::VideoAdvanced => Some("video_advanced"),
        Page::NewGame => Some(["create_server", "create_game", "create_bot"][create_tab.min(2)]),
        _ => None,
    }
}

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
        self.focus = 0;
        self.capture = None;
        self.scroll = 0;
        self.combo = None;
        if self.main.is_empty() {
            self.main = main_entries(self.ui.0.as_deref());
        }
        self.values = SETTINGS.iter().map(|s| get(s.cvar)).collect();
        let num = |name: &str| get(name).and_then(|v| v.trim().parse::<f32>().ok());
        self.new_game.map = current_map
            .and_then(|m| maps.iter().position(|n| n == m))
            .unwrap_or(0);
        self.maps = maps;
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
        let bots = (self.bots[0] + self.bots[1]).min(MAX_BOTS as usize);
        self.new_game.bots_on = bots > 0;
        self.new_game.bot_count = bots.to_string();
        self.new_game.bot_team = match self.bots {
            [_, 0] if bots > 0 => 1,
            [0, _] if bots > 0 => 2,
            _ => 0,
        };
        self.new_game.cvars = self.server_cvars(&get);
        self.go_to(page);
    }

    /// Show a page, its first control focused (the options remember what
    /// they were, for Cancel).
    fn go_to(&mut self, page: Page) {
        if matches!(page, Page::Settings | Page::Extras) && self.page != page && !self.page.over_options() {
            self.options_before = self.snapshot(|_| true);
        }
        self.page = page;
        self.scroll = 0;
        self.combo = None;
        self.key_row = None;
        self.focus = self.first_focusable_from(0, 1);
        self.select_focused_key();
        self.focus_changed();
        self.keep_visible();
    }

    /// Settings' values now (those `which` picks).
    fn snapshot(&self, which: impl Fn(&super::options::Setting) -> bool) -> Vec<(usize, Option<String>)> {
        SETTINGS
            .iter()
            .enumerate()
            .filter(|(_, s)| which(s))
            .map(|(i, _)| (i, self.values.get(i).cloned().flatten()))
            .collect()
    }

    /// Create Server's cvars: the install's Game page options (else none),
    /// ours, the Bot page's; each with its value now.
    fn server_cvars(&self, get: &dyn Fn(&str) -> Option<String>) -> Vec<ServerCvar> {
        let mut out: Vec<ServerCvar> = Vec::new();
        if let Some(ui) = self.ui.0.as_ref() {
            for s in &ui.server_settings {
                let before = get(&s.cvar);
                out.push(ServerCvar {
                    cvar: s.cvar.clone(),
                    label: s.label.clone(),
                    kind: s.kind.clone(),
                    value: before.clone().unwrap_or_else(|| s.default.clone()),
                    before,
                    page: 1,
                    field: None,
                });
            }
        }
        let before = get(ROUNDS_CVAR);
        out.push(ServerCvar {
            cvar: ROUNDS_CVAR.into(),
            label: "Rounds (off: deathmatch; Lucker Party)".into(),
            kind: ServerSettingKind::Bool,
            value: before.clone().unwrap_or_else(|| "0".into()),
            before,
            page: 1,
            field: None,
        });
        for (field, cvar, token) in BOT_CVARS {
            let before = get(cvar);
            out.push(ServerCvar {
                cvar: cvar.into(),
                label: self.text(token, cvar),
                kind: if cvar == "bot_prefix" {
                    ServerSettingKind::Text
                } else {
                    ServerSettingKind::Bool
                },
                value: before.clone().unwrap_or_default(),
                before,
                page: 2,
                field: Some(field),
            });
        }
        out
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
        if self.page != Page::Settings {
            return;
        }
        self.tab = tab;
        self.capture = None;
        self.combo = None;
        self.scroll = 0;
        self.key_row = None;
        self.focus = self.first_focusable_from(0, 1);
        self.select_focused_key();
        self.focus_changed();
        self.keep_visible();
    }

    /// Show a page of Create Server.
    pub fn set_create_tab(&mut self, tab: usize) {
        if self.page != Page::NewGame {
            return;
        }
        self.create_tab = tab.min(CREATE_TABS.len() - 1);
        self.combo = None;
        self.scroll = 0;
        self.focus = self.first_focusable_from(0, 1);
        self.focus_changed();
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

    /// The open page's layout from the install.
    pub fn layout(&self) -> Option<&UiLayout> {
        let name = layout_name(self.page, self.tab, self.create_tab)?;
        self.ui.0.as_ref()?.options.get(name)
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
        self.combo = None;
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
        self.combo = None;
        self.scroll = 0;
        self.focus = self.first_focusable_from(0, 1);
    }

    /// Start the new game set up in Create Server: from a game, close (the
    /// old map plays until the new one is in); from the main menu, show it
    /// loading.
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
            self.page = Page::Main;
        }
    }

    /// The quick start's settings in Create Server.
    fn quick_setup(&mut self) {
        let map = self.maps.iter().position(|m| m == QUICK_MAP).unwrap_or(0);
        self.new_game.map = map;
        self.new_game.bots_on = true;
        self.new_game.bot_count = QUICK_BOTS.to_string();
        self.new_game.bot_team = 0;
        self.new_game.difficulty = NORMAL;
        // Rounds; the rest as they are.
        for c in &mut self.new_game.cvars {
            c.value = if c.cvar == ROUNDS_CVAR {
                "1".into()
            } else {
                c.before.clone().unwrap_or_default()
            };
        }
    }

    /// The value of a cvar Create Server sets, as shown.
    fn server_cvar(&self, cvar: &str) -> Option<&ServerCvar> {
        self.new_game.cvars.iter().find(|c| c.cvar == cvar)
    }

    /// A control row (its label from the game, else ours).
    fn control_row(&self, field: Field) -> Row {
        let text = |t: &str| self.game_text(t);
        let ng = &self.new_game;
        let (label, control) = match field {
            Field::Map => (
                self.text("#GameUI_Map", "Map"),
                Control::Combo {
                    entries: self.maps.clone(),
                    selected: (!self.maps.is_empty()).then_some(ng.map),
                    text: self.maps.get(ng.map).cloned().unwrap_or_else(|| "(no maps found)".into()),
                },
            ),
            Field::BotsOn => (
                self.text("#Cstrike_Bot_IncludeBots", "Include CPU players (bots) in this game"),
                Control::Check(ng.bots_on),
            ),
            Field::BotCount => (
                self.text("#Cstrike_Bot_NumberOfBots", "Number of CPU players"),
                Control::Text {
                    text: ng.bot_count.clone(),
                    numeric: true,
                    max: 2,
                },
            ),
            Field::Difficulty(k) => (
                self.text(&format!("#Cstrike_Bot_Difficulty{k}"), DIFFICULTIES[k].0),
                Control::Radio(ng.difficulty == k),
            ),
            Field::BotTeam => {
                let entries: Vec<String> = BOT_TEAMS.iter().map(|(t, o)| self.text(t, o)).collect();
                (
                    self.text("#CStrike_Bot_JoinTeam", "Bots join team"),
                    Control::Combo {
                        text: entries[ng.bot_team].clone(),
                        entries,
                        selected: Some(ng.bot_team),
                    },
                )
            }
            Field::ServerCvar(i) => {
                let c = &ng.cvars[i];
                let control = match &c.kind {
                    ServerSettingKind::Bool => Control::Check(c.value.trim().parse::<f32>().is_ok_and(|v| v != 0.0)),
                    ServerSettingKind::List(items) => {
                        let entries: Vec<String> = items.iter().map(|(l, _)| self.text(l, l)).collect();
                        let selected = items.iter().position(|(_, v)| v.trim() == c.value.trim());
                        Control::Combo {
                            text: selected.map_or(c.value.clone(), |s| entries[s].clone()),
                            entries,
                            selected,
                        }
                    }
                    ServerSettingKind::Number { .. } => Control::Text {
                        text: c.value.clone(),
                        numeric: true,
                        max: 8,
                    },
                    ServerSettingKind::Text => Control::Text {
                        text: c.value.clone(),
                        numeric: false,
                        max: 64,
                    },
                };
                (c.label.clone(), control)
            }
            Field::Setting(i) | Field::SettingText(i) => {
                let s = &SETTINGS[i];
                let label = s.token.and_then(|t| self.game_text(t)).unwrap_or_else(|| s.label.to_string());
                let v = self.values.get(i).cloned().flatten().unwrap_or_default();
                let shown = s.show(&v, &text);
                let as_combo = self.shown_as_combo(i);
                let control = if field == Field::SettingText(i) {
                    Control::Text {
                        text: match &self.editing {
                            Some((f, typed)) if *f == field => typed.clone(),
                            _ => shown,
                        },
                        numeric: true,
                        max: 8,
                    }
                } else if as_combo {
                    let entries: Vec<String> =
                        s.entries(&self.resolutions, &text).into_iter().map(|(_, l)| l).collect();
                    let selected = s.entry_index(&v, &self.resolutions);
                    Control::Combo {
                        text: selected.map_or(shown, |k| entries[k].clone()),
                        selected,
                        entries,
                    }
                } else {
                    match s.kind {
                        SettingKind::Range { .. } => Control::Slider {
                            fraction: s.fraction(&v).unwrap_or(0.0),
                            text: shown,
                        },
                        _ => Control::Check(s.checked(&v)),
                    }
                };
                (label, control)
            }
        };
        Row::Control { label, field, control }
    }

    /// Whether setting `i` shows as a combo box: choices, window sizes,
    /// and a check box setting the game's layout shows as one (VSync).
    fn shown_as_combo(&self, i: usize) -> bool {
        let s = &SETTINGS[i];
        match s.kind {
            SettingKind::Choice(_) | SettingKind::Resolution => true,
            SettingKind::Toggle => s
                .field
                .and_then(|f| self.settings_layout(s.place)?.get(f))
                .is_some_and(|c| class_of(c).eq_ignore_ascii_case("ComboBox")),
            _ => false,
        }
    }

    /// The layout a place's settings are on.
    fn settings_layout(&self, place: Place) -> Option<&UiLayout> {
        let name = match place {
            Place::Options(tab) => tab.page(),
            Place::KeyboardAdvanced => "keyboard_advanced",
            Place::VideoAdvanced => "video_advanced",
            Place::Extras => return None,
        };
        self.ui.0.as_ref()?.options.get(name)
    }

    /// The settings of a place that mashup has, in its layout's order
    /// (else ours), with the text entries showing slider values.
    fn place_rows(&self, place: Place) -> Vec<Row> {
        let mut fields: Vec<(usize, Field)> = Vec::new();
        let layout = self.settings_layout(place);
        let order = |name: &str| {
            layout
                .and_then(|l| l.controls.iter().position(|c| c.name.eq_ignore_ascii_case(name)))
                .unwrap_or(usize::MAX)
        };
        for (i, s) in SETTINGS.iter().enumerate() {
            if s.place != place || self.values.get(i).cloned().flatten().is_none() {
                continue;
            }
            // In a layout without its control: not shown (the game has no
            // place for it).
            if layout.is_some() && s.field.is_some_and(|f| order(f) == usize::MAX) {
                continue;
            }
            fields.push((s.field.map_or(i, order), Field::Setting(i)));
            if let Some((entry, _)) = VALUE_ENTRIES.iter().find(|(_, cvar)| *cvar == s.cvar)
                && (layout.is_none() || order(entry) != usize::MAX)
            {
                fields.push((order(entry), Field::SettingText(i)));
            }
        }
        if layout.is_some() {
            fields.sort_by_key(|(o, _)| *o);
        }
        // Without the layout the value entry is the slider's own number.
        fields
            .into_iter()
            .filter(|(_, f)| layout.is_some() || !matches!(f, Field::SettingText(_)))
            .map(|(_, f)| self.control_row(f))
            .collect()
    }

    /// The rows of the open page (the left-hand entries on the main page),
    /// in tab order.
    pub fn rows(&self) -> Vec<Row> {
        let button = |label: &str, action: Action| Row::Button {
            label: label.to_string(),
            action,
            enabled: true,
        };
        let ok = || button(&self.text("#GameUI_OK", "OK"), Action::Ok);
        let cancel = || button(&self.text("#GameUI_Cancel", "Cancel"), Action::Cancel);
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
            Page::NewGame => {
                let mut rows: Vec<Row> = match self.create_tab {
                    0 => {
                        let mut r = vec![self.control_row(Field::Map), self.control_row(Field::BotsOn)];
                        if self.new_game.bots_on {
                            r.push(self.control_row(Field::BotCount));
                            r.extend((0..DIFFICULTIES.len()).map(|k| self.control_row(Field::Difficulty(k))));
                        }
                        r
                    }
                    page => {
                        let mut r = Vec::new();
                        if page == 2 {
                            r.push(self.control_row(Field::BotTeam));
                        }
                        for (i, c) in self.new_game.cvars.iter().enumerate() {
                            if c.page == page && c.before.is_some() {
                                r.push(self.control_row(Field::ServerCvar(i)));
                            }
                        }
                        r
                    }
                };
                rows.push(Row::Button {
                    label: self.text("#GameUI_Start", "Start"),
                    action: Action::Start,
                    enabled: !self.maps.is_empty(),
                });
                rows.push(cancel());
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
                button(&self.text("#GameUI_Close", "Close"), Action::Back),
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
                button(&self.text("#GameUI_Cancel", "Cancel"), Action::Back),
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
                rows.push(button(&self.text("#GameUI_AdvancedEllipsis", "Advanced..."), Action::Advanced));
                rows.extend(self.dialog_buttons());
                rows
            }
            Page::Settings => {
                let mut rows = self.place_rows(Place::Options(self.tab));
                if self.tab == Tab::Video {
                    // Its Advanced... button, where the layout has it.
                    let at = self.layout().and_then(|l| l.controls.iter().position(|c| c.name == "AdvancedButton"));
                    let b = button(&self.text("#GameUI_AdvancedEllipsis", "Advanced..."), Action::VideoAdvanced);
                    let before = at.map_or(rows.len(), |at| {
                        rows.iter()
                            .position(|r| self.row_order(r) > at)
                            .unwrap_or(rows.len())
                    });
                    rows.insert(before, b);
                }
                rows.extend(self.dialog_buttons());
                rows
            }
            Page::KeyboardAdvanced | Page::VideoAdvanced => {
                let place = if self.page == Page::KeyboardAdvanced {
                    Place::KeyboardAdvanced
                } else {
                    Place::VideoAdvanced
                };
                let mut rows = self.place_rows(place);
                rows.push(ok());
                rows.push(cancel());
                rows
            }
            Page::Extras => {
                let mut rows = self.place_rows(Place::Extras);
                for (i, s) in SETTINGS.iter().enumerate() {
                    if s.place == Place::Extras && self.values.get(i).cloned().flatten().is_none() {
                        rows.push(Row::Info(format!("{}: not available", s.label)));
                    }
                }
                rows.push(ok());
                rows.push(cancel());
                rows
            }
        }
    }

    /// The options' OK, Cancel and Apply (Apply greyed with nothing to
    /// apply).
    fn dialog_buttons(&self) -> Vec<Row> {
        let changed = self.options_before.iter().any(|(i, v)| self.values.get(*i) != Some(v));
        vec![
            Row::Button {
                label: self.text("#GameUI_OK", "OK"),
                action: Action::Ok,
                enabled: true,
            },
            Row::Button {
                label: self.text("#GameUI_Cancel", "Cancel"),
                action: Action::Cancel,
                enabled: true,
            },
            Row::Button {
                label: self.text("#GameUI_Apply", "Apply"),
                action: Action::Apply,
                enabled: changed,
            },
        ]
    }

    /// Where a row's control is in the open page's layout (its index).
    fn row_order(&self, row: &Row) -> usize {
        let Some(layout) = self.layout() else { return usize::MAX };
        let name = match row {
            Row::Control { field, .. } => self.field_name(*field),
            Row::Button {
                action: Action::VideoAdvanced,
                ..
            } => Some("AdvancedButton"),
            _ => None,
        };
        name.and_then(|n| layout.controls.iter().position(|c| c.name.eq_ignore_ascii_case(n)))
            .unwrap_or(usize::MAX)
    }

    /// The layout control showing a field (its `fieldName`).
    pub fn field_name(&self, field: Field) -> Option<&'static str> {
        const SKILL: [&str; 4] = ["SkillLevel0", "SkillLevel1", "SkillLevel2", "SkillLevel3"];
        match field {
            Field::Map => Some("MapList"),
            Field::BotsOn => Some("EnableBotsCheck"),
            Field::BotCount => Some("BotQuotaCombo"),
            Field::Difficulty(k) => SKILL.get(k).copied(),
            Field::BotTeam => Some("BotJoinTeamCombo"),
            Field::ServerCvar(i) => self.new_game.cvars.get(i).and_then(|c| c.field),
            Field::Setting(i) => SETTINGS[i].field,
            Field::SettingText(i) => VALUE_ENTRIES.iter().find(|(_, c)| *c == SETTINGS[i].cvar).map(|(f, _)| *f),
        }
    }

    /// The default button of the open page (Enter presses it).
    fn default_action(&self) -> Option<Action> {
        match self.page {
            Page::NewGame => Some(Action::Start),
            Page::Settings | Page::KeyboardAdvanced | Page::VideoAdvanced | Page::Extras => Some(Action::Ok),
            _ => None,
        }
    }

    /// The open page's state in words (`menuinput`).
    pub fn describe(&self) -> String {
        let rows = self.rows();
        let focused = rows.get(self.focus).map(|r| match r {
            Row::Control { label, control, .. } => format!("{label}: {control:?}"),
            other => label_of(other),
        });
        let combo = self.combo.map_or(String::new(), |c| {
            format!(", list open on entry {} of {}", c.highlight, c.len)
        });
        format!("{:?}, focus {} ({}){combo}", self.page, self.focus, focused.unwrap_or_default())
    }

    /// Apply an input; the console lines it produces.
    pub fn handle(&mut self, input: Input) -> Outcome {
        let mut out = Outcome::default();
        if self.open && self.failure.is_some() {
            // The failure's dialog: Close (its button, Esc, Enter) leaves
            // the main menu.
            if matches!(
                input,
                Input::Click(Target::Cancel | Target::Close, _) | Input::Close | Input::Activate
            ) {
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
        if self.combo.is_some() {
            self.handle_combo(input, &mut out);
            return out;
        }
        let rows = self.rows();
        let focused = rows.get(self.focus).cloned();
        let text_focused = matches!(focused, Some(Row::Control { control: Control::Text { .. }, .. }));
        match input {
            Input::Close if self.page != Page::Main && self.default_action().is_some() => {
                self.press_action_into(&Action::Cancel, &mut out)
            }
            Input::Click(Target::Close, _) => {
                if self.default_action().is_some() {
                    self.press_action_into(&Action::Cancel, &mut out);
                } else {
                    self.back();
                }
            }
            // The main menu stays: Esc only closes its dialogs.
            Input::Close if !self.in_game => self.back(),
            Input::Close => self.close(&mut out),
            Input::Back if !text_focused => self.back(),
            Input::Back => self.edit(Edit::Backspace, &mut out),
            Input::Up | Input::Down => {
                let dir = if input == Input::Up { -1 } else { 1 };
                match focused.as_ref() {
                    Some(Row::Control {
                        field,
                        control: Control::Combo { .. } | Control::Slider { .. },
                        ..
                    }) if self.page != Page::Main => self.change(*field, dir, false, &mut out),
                    _ => {
                        let n = rows.len().max(1);
                        self.focus = self.first_focusable_from((self.focus + if dir < 0 { n - 1 } else { 1 }) % n, dir);
                        self.focus_changed();
                    }
                }
            }
            Input::Left | Input::Right => {
                let dir = if input == Input::Left { -1 } else { 1 };
                match focused.as_ref() {
                    Some(Row::Control {
                        control: Control::Text { .. },
                        ..
                    }) => self.edit(if dir < 0 { Edit::Left(false) } else { Edit::Right(false) }, &mut out),
                    Some(Row::Control { field, control, .. }) if !matches!(control, Control::Check(_) | Control::Radio(_)) => {
                        self.change(*field, dir, false, &mut out)
                    }
                    _ => {}
                }
            }
            Input::Activate => match focused.as_ref() {
                Some(Row::Button { .. } | Row::Bind { .. }) => self.activate(self.focus, &mut out),
                _ if self.page == Page::Main => self.activate(self.focus, &mut out),
                _ => {
                    if let Some(action) = self.default_action() {
                        self.press_action_into(&action, &mut out);
                    }
                }
            },
            Input::Space => {
                if text_focused {
                    self.edit(Edit::Insert(" ".into()), &mut out);
                } else {
                    self.activate(self.focus, &mut out);
                }
            }
            Input::Hover(Target::Main(i)) => {
                if self.page == Page::Main && rows.get(i).is_some_and(Row::focusable) {
                    self.focus = i;
                }
            }
            // Dialogs focus by click (and Tab), not by hover.
            Input::Hover(_) => {}
            Input::Click(Target::Main(i), _) => {
                if let Some(e) = self.entries().get(i).filter(|e| e.enabled) {
                    if self.page == Page::Main {
                        self.focus = i;
                    }
                    self.main(e.item, &mut out);
                }
            }
            Input::Click(Target::Row(i), _) => {
                if self.page != Page::Main && rows.get(i).is_some_and(Row::focusable) {
                    // A list row: the first click selects, the next edits.
                    if matches!(rows[i], Row::Bind { .. }) && self.key_row != Some(i) {
                        self.focus = i;
                        self.key_row = Some(i);
                    } else {
                        let was = self.focus;
                        self.focus = i;
                        if was != i {
                            self.focus_changed();
                        }
                        // A click into a text entry focuses it (`Caret`
                        // places the caret).
                        if !matches!(rows[i], Row::Control { control: Control::Text { .. }, .. }) {
                            self.activate(i, &mut out);
                        }
                    }
                }
            }
            Input::Click(Target::Tab(i), _) => match self.page {
                Page::Settings => {
                    if let Some((tab, ..)) = TABS.get(i) {
                        self.set_tab(*tab);
                    }
                }
                Page::NewGame => self.set_create_tab(i),
                _ => {}
            },
            // Only while loading (above) or with a list open.
            Input::Click(Target::Cancel | Target::ComboItem(_) | Target::Outside, _) => {}
            Input::Click(Target::Scroll, step) | Input::Scroll(step) => {
                let (len, shown) = self.list();
                self.scroll = (self.scroll as i32 + step).clamp(0, len.saturating_sub(shown) as i32) as usize;
                return out;
            }
            Input::Wheel(i, notches) => {
                if let Some(Row::Control {
                    field,
                    control: Control::Combo { .. },
                    ..
                }) = rows.get(i)
                {
                    self.focus = i;
                    self.change(*field, -notches, false, &mut out);
                }
            }
            Input::NextTab(dir) => match self.page {
                Page::Settings => self.set_tab(self.tab.step(dir)),
                Page::NewGame => {
                    let n = CREATE_TABS.len() as i32;
                    self.set_create_tab((self.create_tab as i32 + dir).rem_euclid(n) as usize)
                }
                _ => {}
            },
            Input::Focus(dir) => {
                if self.page != Page::Main {
                    let order: Vec<usize> = (0..rows.len()).filter(|&i| rows[i].focusable()).collect();
                    if let Some(next) = widgets::focus_step(&order, Some(self.focus), dir) {
                        self.focus = next;
                        self.focus_changed();
                    }
                } else {
                    let n = rows.len().max(1);
                    self.focus = self.first_focusable_from((self.focus as i32 + dir).rem_euclid(n as i32) as usize, dir);
                }
            }
            Input::FocusRow(i) => {
                if rows.get(i).is_some_and(Row::focusable) && self.focus != i {
                    self.focus = i;
                    self.focus_changed();
                }
            }
            Input::Slide(i, f) => {
                if let Some(Row::Control {
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
                if text_focused {
                    self.edit(Edit::Insert(c.to_string()), &mut out);
                } else if let Some(Row::Control {
                    field,
                    control: Control::Combo { entries, selected, .. },
                    ..
                }) = focused.as_ref()
                    && let Some(k) = jump(entries, selected.unwrap_or(0), c)
                {
                    self.pick(*field, k, &mut out);
                }
            }
            Input::Type(s) => {
                if text_focused {
                    self.edit(Edit::Insert(s), &mut out);
                }
            }
            Input::Edit(e) => {
                if text_focused {
                    self.edit(e, &mut out);
                }
            }
            Input::Copy | Input::Cut => {
                if let Some(Row::Control {
                    control: Control::Text { text, .. },
                    ..
                }) = focused.as_ref()
                {
                    let selected = self.caret.selected(text).to_string();
                    if !selected.is_empty() {
                        out.clipboard = Some(selected);
                        if input == Input::Cut {
                            self.edit(Edit::Delete, &mut out);
                        }
                    }
                }
            }
            Input::Caret(i, at) => {
                if matches!(rows.get(i), Some(Row::Control { control: Control::Text { .. }, .. })) {
                    if self.focus != i {
                        self.focus = i;
                        self.focus_changed();
                    }
                    self.caret = Caret { at, anchor: at };
                }
            }
        }
        self.select_focused_key();
        self.keep_visible();
        out
    }

    /// Input while a combo box's list is open: it takes the keys, the
    /// pointer on its entries and the wheel; a click elsewhere closes it.
    fn handle_combo(&mut self, input: Input, out: &mut Outcome) {
        let Some(mut list) = self.combo else { return };
        let event = match input {
            Input::Up => list.key(ComboKey::Up),
            Input::Down => list.key(ComboKey::Down),
            Input::Activate | Input::Space => list.key(ComboKey::Enter),
            Input::Close | Input::Back => list.key(ComboKey::Escape),
            Input::Hover(Target::ComboItem(k)) => {
                list.hover(k);
                ComboEvent::Open
            }
            Input::Click(Target::ComboItem(k), _) => ComboEvent::Pick(k),
            Input::Click(..) | Input::Focus(_) | Input::NextTab(_) => ComboEvent::Close,
            Input::Scroll(n) => {
                list.wheel(n);
                ComboEvent::Open
            }
            Input::Char(c) => {
                if let Some(Row::Control {
                    control: Control::Combo { entries, .. },
                    ..
                }) = self.rows().get(list.owner)
                    && let Some(k) = jump(entries, list.highlight, c)
                {
                    list.select(k);
                }
                ComboEvent::Open
            }
            _ => ComboEvent::Open,
        };
        match event {
            ComboEvent::Open => self.combo = Some(list),
            ComboEvent::Close => self.combo = None,
            ComboEvent::Pick(k) => {
                self.combo = None;
                if let Some(field) = self.rows().get(list.owner).and_then(Row::field) {
                    self.pick(field, k, out);
                }
            }
        }
    }

    /// The focus moved: a text entry selects all its text (VGUI), a list
    /// open closes.
    fn focus_changed(&mut self) {
        self.combo = None;
        self.editing = None;
        if let Some(Row::Control {
            control: Control::Text { text, .. },
            ..
        }) = self.rows().get(self.focus)
        {
            let n = text.chars().count();
            self.caret = Caret { at: n, anchor: 0 };
        }
    }

    /// An edit to the focused text entry; the value follows.
    fn edit(&mut self, edit: Edit, out: &mut Outcome) {
        let rows = self.rows();
        let Some(Row::Control {
            field,
            control: Control::Text { text, numeric, max },
            ..
        }) = rows.get(self.focus)
        else {
            return;
        };
        let mut text = text.clone();
        let edit = match edit {
            Edit::Insert(s) if *numeric => Edit::Insert(s.chars().filter(|c| c.is_ascii_digit() || *c == '.').collect()),
            e => e,
        };
        let mut caret = self.caret;
        let changed = caret.edit(&mut text, edit, 256);
        if text.chars().count() > *max {
            return;
        }
        self.caret = caret;
        if changed {
            self.set_text(*field, text, out);
        }
    }

    /// A text entry's new text.
    fn set_text(&mut self, field: Field, text: String, out: &mut Outcome) {
        match field {
            Field::BotCount => self.new_game.bot_count = text,
            Field::ServerCvar(i) => self.new_game.cvars[i].value = text,
            Field::SettingText(i) => {
                self.editing = Some((field, text.clone()));
                // Set once it reads as a number in range.
                if let (SettingKind::Range { min, max, .. }, Ok(v)) = (SETTINGS[i].kind, text.trim().parse::<f32>())
                    && (min..=max).contains(&v)
                {
                    self.set_value(i, text.trim().to_string(), out);
                }
            }
            _ => {}
        }
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
            Page::Settings if self.tab == Tab::Keyboard => (self.rows().len() - KEY_BUTTONS - 3, KEY_ROWS),
            Page::NewGame if self.create_tab == 1 => (self.rows().len() - 2, GAME_ROWS),
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

    /// Put settings back as they were.
    fn restore(&mut self, before: Vec<(usize, Option<String>)>, out: &mut Outcome) {
        for (s, before) in before {
            if let Some(v) = before
                && self.values.get(s) != Some(&Some(v.clone()))
            {
                self.set_value(s, v, out);
            }
        }
    }

    fn close(&mut self, out: &mut Outcome) {
        self.open = false;
        self.capture = None;
        self.combo = None;
        out.close = true;
    }

    /// Back to the main page, its entry focused (from an Advanced dialog:
    /// the options, its button focused).
    fn back(&mut self) {
        self.capture = None;
        self.combo = None;
        self.scroll = 0;
        match self.page {
            Page::Main => {}
            Page::KeyboardAdvanced | Page::VideoAdvanced => {
                let action = if self.page == Page::KeyboardAdvanced {
                    Action::Advanced
                } else {
                    Action::VideoAdvanced
                };
                self.page = Page::Settings;
                self.focus = self
                    .rows()
                    .iter()
                    .position(|r| matches!(r, Row::Button { action: a, .. } if *a == action))
                    .unwrap_or(0);
            }
            page => {
                self.focus = self.entries().iter().position(|e| e.item.page() == Some(page)).unwrap_or(0);
                self.page = Page::Main;
            }
        }
    }

    fn main(&mut self, item: MainItem, out: &mut Outcome) {
        if let Some(page) = item.page() {
            // Another dialog in its place: changes made so far stay (CS:S
            // keeps both open; here the one shown gives way).
            if page != self.page {
                self.advanced_before.clear();
                self.options_before.clear();
            }
            if page == Page::NewGame {
                self.create_tab = 0;
            }
            if self.page != page {
                self.go_to(page);
            }
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
            MainItem::NewGame | MainItem::Bots | MainItem::Team | MainItem::Options | MainItem::Extras => {}
        }
    }

    /// Press row `i` (Space, a click): a button presses, a check box
    /// ticks, a radio button picks, a combo box opens its list, a list row
    /// waits for a key.
    fn activate(&mut self, i: usize, out: &mut Outcome) {
        let Some(row) = self.rows().into_iter().nth(i) else {
            return;
        };
        match row {
            Row::Button { enabled: false, .. } | Row::Info(_) | Row::Heading(_) => {}
            Row::Bind { command, .. } => {
                self.key_row = Some(i);
                self.capture = Some(command);
            }
            Row::Control { field, control, .. } => match control {
                Control::Combo { entries, selected, .. } => {
                    if !entries.is_empty() {
                        self.combo = Some(ComboList::open(i, entries.len(), selected.unwrap_or(0)));
                    }
                }
                Control::Check(_) | Control::Radio(_) => self.change(field, 1, true, out),
                Control::Slider { .. } | Control::Text { .. } => {}
            },
            Row::Button { action, .. } => self.press_action_into(&action, out),
        }
    }

    /// Press a button by what it does (`menu advanced` opens a dialog so).
    pub fn press_action(&mut self, action: &Action) -> Outcome {
        let mut out = Outcome::default();
        self.press_action_into(action, &mut out);
        out
    }

    fn press_action_into(&mut self, action: &Action, out: &mut Outcome) {
        match action.clone() {
            Action::Main(item) => self.main(item, out),
            Action::Back => self.back(),
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
            Action::Advanced | Action::VideoAdvanced => {
                let (page, place) = if *action == Action::Advanced {
                    (Page::KeyboardAdvanced, Place::KeyboardAdvanced)
                } else {
                    (Page::VideoAdvanced, Place::VideoAdvanced)
                };
                self.advanced_before = self.snapshot(|s| s.place == place);
                self.go_to(page);
            }
            Action::Ok => match self.page {
                Page::KeyboardAdvanced | Page::VideoAdvanced => {
                    self.advanced_before.clear();
                    self.back();
                }
                _ => {
                    self.options_before.clear();
                    self.back();
                }
            },
            Action::Cancel => match self.page {
                Page::KeyboardAdvanced | Page::VideoAdvanced => {
                    let before = std::mem::take(&mut self.advanced_before);
                    self.restore(before, out);
                    self.back();
                }
                Page::Settings | Page::Extras => {
                    let before = std::mem::take(&mut self.options_before);
                    self.restore(before, out);
                    self.back();
                }
                _ => self.back(),
            },
            Action::Apply => {
                self.options_before = self.snapshot(|_| true);
            }
        }
    }

    /// Step a value (`dir`), a check box ticked or radio picked; `wrap`:
    /// choices wrap round (else stop at their ends, as a combo box
    /// stepped). Settings apply at once (the options' Cancel puts them
    /// back).
    fn change(&mut self, field: Field, dir: i32, wrap: bool, out: &mut Outcome) {
        let ng = &mut self.new_game;
        match field {
            Field::Map => {
                if !self.maps.is_empty() {
                    let k = widgets::combo_step(ng.map, self.maps.len(), dir);
                    ng.map = k;
                }
            }
            Field::BotsOn => ng.bots_on = !ng.bots_on,
            Field::BotCount => {}
            Field::Difficulty(k) => ng.difficulty = k,
            Field::BotTeam => ng.bot_team = widgets::combo_step(ng.bot_team, BOT_TEAMS.len(), dir),
            Field::ServerCvar(i) => {
                let c = &mut ng.cvars[i];
                match &c.kind {
                    ServerSettingKind::Bool => {
                        let on = c.value.trim().parse::<f32>().is_ok_and(|v| v != 0.0);
                        c.value = if on { "0" } else { "1" }.into();
                    }
                    ServerSettingKind::List(items) => {
                        let at = items.iter().position(|(_, v)| v.trim() == c.value.trim()).unwrap_or(0);
                        c.value = items[widgets::combo_step(at, items.len(), dir)].1.clone();
                    }
                    _ => {}
                }
            }
            Field::Setting(i) | Field::SettingText(i) => {
                let (Some(setting), Some(Some(current))) = (SETTINGS.get(i), self.values.get(i)) else {
                    return;
                };
                let next = setting.step_with(current, dir, &self.resolutions, wrap);
                self.set_value(i, next, out);
            }
        }
    }

    /// Pick entry `k` of a combo box.
    fn pick(&mut self, field: Field, k: usize, out: &mut Outcome) {
        match field {
            Field::Map => {
                if k < self.maps.len() {
                    self.new_game.map = k;
                }
            }
            Field::BotTeam => self.new_game.bot_team = k.min(BOT_TEAMS.len() - 1),
            Field::ServerCvar(i) => {
                if let ServerSettingKind::List(items) = &self.new_game.cvars[i].kind
                    && let Some((_, v)) = items.get(k)
                {
                    self.new_game.cvars[i].value = v.clone();
                }
            }
            Field::Setting(i) => {
                let none = |_: &str| None;
                if let Some((v, _)) = SETTINGS[i].entries(&self.resolutions, &none).into_iter().nth(k) {
                    self.set_value(i, v, out);
                }
            }
            _ => {}
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
        let rounds = self
            .server_cvar(ROUNDS_CVAR)
            .is_some_and(|c| c.value.trim().parse::<f32>().is_ok_and(|v| v != 0.0));
        let mut lines = vec!["bot_kick".to_string(), format!("{ROUNDS_CVAR} {}", rounds as u8)];
        // The Game and Bot pages' cvars that changed.
        for c in &ng.cvars {
            if c.cvar != ROUNDS_CVAR && c.before.as_ref().is_some_and(|b| b.trim() != c.value.trim()) {
                lines.push(format!("{} {}", c.cvar, crate::console::quote(&c.value)));
            }
        }
        lines.extend([
            format!("bot_reaction {reaction}"),
            format!("bot_aim_error {aim}"),
            format!("bot_turn_rate {turn}"),
            format!("map {}", crate::console::quote(map)),
        ]);
        let (t, ct) = ng.split();
        let after = std::iter::repeat_n("bot_add 1".to_string(), t as usize)
            .chain(std::iter::repeat_n("bot_add 2".to_string(), ct as usize))
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

/// The next entry after `from` starting with `c` (wrapping), as VGUI's
/// combo boxes jump on a typed letter.
fn jump(entries: &[String], from: usize, c: char) -> Option<usize> {
    let n = entries.len();
    let c = c.to_ascii_lowercase();
    (1..=n)
        .map(|k| (from + k) % n)
        .find(|&i| entries[i].to_ascii_lowercase().starts_with(c))
}

/// A layout control's VGUI class (`ComboBox`, `CCvarSlider` ...).
pub(super) fn class_of(c: &UiControl) -> &str {
    match &c.kind {
        UiKind::Other(class) => class,
        UiKind::Label => "Label",
        UiKind::Button => "Button",
        UiKind::Image => "ImagePanel",
        UiKind::RichText => "RichText",
        UiKind::Panel => "Panel",
        UiKind::Divider => "Divider",
        UiKind::Frame => "Frame",
    }
}

/// `menuinput`'s words as an input.
fn menu_input(menu: &GameMenu, a: &[String]) -> Result<Input, String> {
    let rows = menu.rows();
    let word = a.first().map(|s| s.to_lowercase());
    match word.as_deref() {
        Some("focus") => {
            let what = a.get(1).map(|s| s.to_lowercase()).ok_or("focus <cvar|map|bots|botcount|botteam|difficulty>")?;
            let i = rows
                .iter()
                .position(|r| match r.field() {
                    Some(Field::Map) => what == "map",
                    Some(Field::BotsOn) => what == "bots",
                    Some(Field::BotCount) => what == "botcount",
                    Some(Field::BotTeam) => what == "botteam",
                    Some(Field::Difficulty(_)) => what == "difficulty",
                    Some(Field::ServerCvar(i)) => menu.new_game.cvars[i].cvar == what,
                    Some(Field::Setting(i)) => SETTINGS[i].cvar == what,
                    _ => false,
                })
                .ok_or(format!("no control \"{what}\" on this page"))?;
            Ok(Input::FocusRow(i))
        }
        Some("open") => Ok(Input::Space),
        Some("down") => Ok(Input::Down),
        Some("up") => Ok(Input::Up),
        Some("enter") => Ok(Input::Activate),
        Some("space") => Ok(Input::Space),
        Some("escape") => Ok(Input::Close),
        Some("tab") => Ok(Input::Focus(1)),
        Some("backtab") => Ok(Input::Focus(-1)),
        Some("type") => Ok(Input::Type(a[1..].join(" "))),
        Some("pick") => Ok(Input::Click(
            Target::ComboItem(a.get(1).and_then(|n| n.parse().ok()).ok_or("pick <n>")?),
            0,
        )),
        Some("sheet") => Ok(Input::Click(Target::Tab(a.get(1).and_then(|n| n.parse().ok()).ok_or("sheet <n>")?), 0)),
        _ => Err("menuinput focus <control> | open | down | up | enter | space | escape | tab | backtab | type <text> | pick <n> | sheet <n>".into()),
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

/// The install's GameUI look, read in the background at startup.
#[derive(Resource, Default)]
struct MenuUi {
    loading: Option<Arc<Mutex<Option<Option<GameUi>>>>>,
    ui: Option<Arc<GameUi>>,
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

/// The install changed (the first-run dialog, `mashup_install`): read
/// its GameUI look again, and the open page's maps.
pub(super) fn reload_install(w: &mut World) {
    if let Err(e) = w.run_system_cached(start_loading_ui) {
        warn!("game menu: reading the GameUI look again: {e}");
    }
    let menu = w.resource::<GameMenu>();
    if menu.open && menu.loading.is_none() {
        let page = menu.page;
        open_menu(w, page);
    }
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
        "game menu: GameUI look ({} entries, {} keyboard actions, {} option and Create Server pages, {} server \
         options, {} main menu backgrounds, title {:?}{})",
        game_ui.menu.len(),
        game_ui.actions.len(),
        game_ui.options.len(),
        game_ui.server_settings.len(),
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
        let mut names: Vec<String> = SETTINGS
            .iter()
            .map(|s| s.cvar.to_string())
            .chain([ROUNDS_CVAR.to_string(), "bot_reaction".into()])
            .chain(BOT_CVARS.iter().map(|(_, c, _)| c.to_string()))
            .collect();
        if let Some(ui) = w.get_resource::<MenuUi>().and_then(|u| u.ui.clone()) {
            names.extend(ui.server_settings.iter().map(|s| s.cvar.clone()));
        }
        let cvars: Vec<_> = {
            let console = w.resource::<Console>();
            names.iter().filter_map(|n| console.cvar(n).cloned()).collect()
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
    if page != Page::Main
        && let Some(mut windows) = w.get_resource_mut::<Windows>()
    {
        windows.raise(page.window());
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
    menu.combo = None;
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
    if let Some(text) = out.clipboard
        && let Err(e) = arboard::Clipboard::new().and_then(|mut c| c.set_text(text))
    {
        warn!("clipboard: {e}");
    }
}

/// Whether the menu's dialog takes the keys: no other window over it
/// (the server browser in front, the first-run dialog).
fn menu_has_keys(
    menu: &GameMenu,
    browser: Option<&super::server_browser::ServerBrowser>,
    first_run: Option<&super::first_run::FirstRun>,
    windows: Option<&Windows>,
) -> bool {
    if first_run.is_some_and(|f| f.open) {
        return false;
    }
    if !browser.is_some_and(|b| b.open) {
        return true;
    }
    // The browser and a dialog of the menu both open: the front one.
    menu.page != Page::Main
        && windows.is_some_and(|w| w.front(&[super::server_browser::WINDOW, menu.page.window()]) == Some(menu.page.window()))
}

/// The focused row is a text entry (typing goes to it: `text_keys`).
fn typing(menu: &GameMenu) -> bool {
    menu.combo.is_none()
        && matches!(
            menu.rows().get(menu.focus),
            Some(Row::Control {
                control: Control::Text { .. },
                ..
            })
        )
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

/// Esc opens and closes the menu; arrows, Enter, Space, Backspace, Tab
/// (Ctrl+Tab: the property sheet's tabs) and letters drive it; while a
/// keyboard action waits for a key, the next key, button or wheel notch
/// is its new key. Typing into a text entry is `text_keys`'. Runs before
/// the console's toggle, so the Esc that closes the console doesn't open
/// the menu.
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
    (browser, first_run, windows): (
        Option<Res<super::server_browser::ServerBrowser>>,
        Option<Res<super::first_run::FirstRun>>,
        Option<Res<Windows>>,
    ),
) {
    if ui.open || !menu_has_keys(&menu, browser.as_deref(), first_run.as_deref(), windows.as_deref()) {
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
    let shift = keys.any_pressed([KeyCode::ShiftLeft, KeyCode::ShiftRight]);
    let ctrl = keys.any_pressed([KeyCode::ControlLeft, KeyCode::ControlRight]);
    let tab = if shift { -1 } else { 1 };
    let text = typing(&menu);
    let mut inputs = Vec::new();
    for (key, input) in [
        (KeyCode::Escape, Some(Input::Close)),
        (KeyCode::ArrowUp, Some(Input::Up)),
        (KeyCode::ArrowDown, Some(Input::Down)),
        (KeyCode::ArrowLeft, (!text).then_some(Input::Left)),
        (KeyCode::ArrowRight, (!text).then_some(Input::Right)),
        (KeyCode::Enter, Some(Input::Activate)),
        (KeyCode::NumpadEnter, Some(Input::Activate)),
        (KeyCode::Space, (!text).then_some(Input::Space)),
        (KeyCode::Backspace, (!text).then_some(Input::Back)),
        (KeyCode::Tab, Some(if ctrl { Input::NextTab(tab) } else { Input::Focus(tab) })),
        (KeyCode::PageUp, Some(Input::Scroll(-(KEY_ROWS as i32)))),
        (KeyCode::PageDown, Some(Input::Scroll(KEY_ROWS as i32))),
    ] {
        if keys.just_pressed(key)
            && let Some(input) = input
        {
            inputs.push(input);
        }
    }
    if !text && !ctrl {
        inputs.extend(LETTERS.iter().filter(|(k, _)| keys.just_pressed(*k)).map(|(_, c)| Input::Char(*c)));
    }
    for input in inputs {
        apply(input, &mut menu, &mut console, &mut after, &mut regrab);
    }
}

/// Typing into the focused text entry: characters, Backspace, Delete,
/// the arrows, Home and End (Shift selects), Ctrl+A, Ctrl+C, Ctrl+X,
/// Ctrl+V.
#[allow(clippy::too_many_arguments)]
fn text_keys(
    mut events: MessageReader<KeyboardInput>,
    held: Res<ButtonInput<KeyCode>>,
    ui: Res<super::console::ConsoleUi>,
    mut menu: ResMut<GameMenu>,
    mut console: ResMut<Console>,
    mut after: ResMut<AfterLoad>,
    mut regrab: ResMut<RegrabCursor>,
    (browser, first_run, windows): (
        Option<Res<super::server_browser::ServerBrowser>>,
        Option<Res<super::first_run::FirstRun>>,
        Option<Res<Windows>>,
    ),
) {
    let live = menu.open
        && !ui.open
        && menu.capture.is_none()
        && typing(&menu)
        && menu_has_keys(&menu, browser.as_deref(), first_run.as_deref(), windows.as_deref());
    if !live {
        events.clear();
        return;
    }
    let shift = held.any_pressed([KeyCode::ShiftLeft, KeyCode::ShiftRight]);
    let ctrl = held.any_pressed([KeyCode::ControlLeft, KeyCode::ControlRight, KeyCode::SuperLeft, KeyCode::SuperRight]);
    for e in events.read() {
        if e.state != ButtonState::Pressed {
            continue;
        }
        let input = match (&e.logical_key, e.key_code) {
            (_, KeyCode::KeyA) if ctrl => Input::Edit(Edit::SelectAll),
            (_, KeyCode::KeyC) if ctrl => Input::Copy,
            (_, KeyCode::KeyX) if ctrl => Input::Cut,
            (_, KeyCode::KeyV) if ctrl => {
                let text = arboard::Clipboard::new().and_then(|mut c| c.get_text()).unwrap_or_default();
                Input::Type(text.lines().next().unwrap_or("").to_string())
            }
            (_, KeyCode::ArrowLeft) => Input::Edit(Edit::Left(shift)),
            (_, KeyCode::ArrowRight) => Input::Edit(Edit::Right(shift)),
            (_, KeyCode::Home) => Input::Edit(Edit::Home(shift)),
            (_, KeyCode::End) => Input::Edit(Edit::End(shift)),
            (_, KeyCode::Delete) => Input::Edit(Edit::Delete),
            (Key::Backspace, _) => Input::Edit(Edit::Backspace),
            (Key::Space, _) => Input::Type(" ".into()),
            (Key::Character(s), _) if !ctrl => Input::Type(s.to_string()),
            _ => continue,
        };
        apply(input, &mut menu, &mut console, &mut after, &mut regrab);
    }
}

/// What a UI node stands for: a row (step 0) or one of its parts.
#[derive(Component, Clone, Copy)]
struct Hit(Target, i32);

/// A combo box row the wheel steps.
#[derive(Component, Clone, Copy)]
struct WheelCombo(usize);

/// A text entry row: a click places its caret.
#[derive(Component, Clone)]
struct TextHit {
    row: usize,
    shown: String,
    width: f32,
}

/// A list the wheel scrolls.
#[derive(Component)]
struct WheelList;

/// Hovering focuses, clicking presses, the wheel scrolls a list under the
/// mouse (or an open combo box's list, or steps a combo box), a held
/// slider follows the mouse.
#[allow(clippy::too_many_arguments)]
fn pointer(
    hits: Query<(&Interaction, &Hit), Changed<Interaction>>,
    texts: Query<(&Interaction, &TextHit, &RelativeCursorPosition), Changed<Interaction>>,
    lists: Query<&RelativeCursorPosition, With<WheelList>>,
    combos: Query<(&WheelCombo, &RelativeCursorPosition)>,
    scroll: Option<Res<AccumulatedMouseScroll>>,
    slider: Res<SliderDrag>,
    fonts: Res<UiFonts>,
    windows_q: Query<&Window>,
    mut menu: ResMut<GameMenu>,
    mut console: ResMut<Console>,
    mut after: ResMut<AfterLoad>,
    mut regrab: ResMut<RegrabCursor>,
    (browser, first_run): (Option<Res<super::server_browser::ServerBrowser>>, Option<Res<super::first_run::FirstRun>>),
) {
    if !menu.open || menu.capture.is_some() || first_run.is_some_and(|f| f.open) {
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
    let height = windows_q.iter().next().map_or(720.0, Window::height);
    for (i, hit, at) in &texts {
        if *i == Interaction::Pressed
            && let Some(p) = at.normalized
        {
            let look = Look {
                ui: menu.ui.0.as_deref(),
                fonts: &fonts,
                s: (height / 720.0).clamp(0.6, 3.0),
                height,
                accent: Color::WHITE,
            };
            let caret = (menu.focus == hit.row).then_some(menu.caret);
            let at = widgets::caret_from_click(&fonts, &look.default_font(), look.s, &hit.shown, hit.width, p.x + 0.5, caret);
            inputs.push(Input::Click(Target::Row(hit.row), 0));
            inputs.push(Input::Caret(hit.row, at));
        }
    }
    if let Some((SliderOwner::Menu, row, f)) = slider.at
        && slider.is_changed()
    {
        inputs.push(Input::Slide(row, f));
    }
    if let Some(scroll) = scroll
        && scroll.delta.y != 0.0
    {
        let notches = widgets::wheel_notches(&scroll);
        if menu.combo.is_some() {
            inputs.push(Input::Scroll(-notches));
        } else if lists.iter().any(|l| l.cursor_over()) {
            inputs.push(Input::Scroll(-notches * 3));
        } else if let Some((c, _)) = combos.iter().find(|(_, r)| r.cursor_over()) {
            inputs.push(Input::Wheel(c.0, notches));
        }
    }
    // The browser in front of the dialog: its own (Bevy's picking already
    // stops at the frame in front).
    let _ = browser;
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

/// What a page is drawn with: the root (open lists' outside clicks), the
/// look, the menu and its rows.
struct Ctx<'a> {
    root: Entity,
    look: &'a Look<'a>,
    menu: &'a GameMenu,
    rows: &'a [Row],
}

/// A dialog's size in scheme pixels: the layout's frame when it has one.
fn dialog_size(menu: &GameMenu, page: Page) -> (f32, f32) {
    match page {
        // CS:S's options and Create Server dialogs: their pages
        // (`OptionsSubMultiplayer.res`: 496 x 314; Create Server's: 332 x
        // 364) in a property sheet, the buttons under it.
        Page::Settings => (512.0, 406.0),
        Page::NewGame => (348.0, 460.0),
        // The layout reaches to 264 x 124.
        Page::KeyboardAdvanced => (280.0, 134.0),
        Page::VideoAdvanced => menu
            .ui
            .0
            .as_ref()
            .and_then(|u| u.options.get("video_advanced"))
            .and_then(|l| l.get("OptionsSubVideoAdvancedDlg"))
            .map_or((482.0, 358.0), |c| (c.wide, c.tall)),
        Page::Extras => {
            let n = menu.rows().len() as f32;
            (420.0, 44.0 + n * 28.0 + 12.0)
        }
        _ => (360.0, 230.0),
    }
}

#[allow(clippy::too_many_arguments)]
fn draw(
    menu: Res<GameMenu>,
    fonts: Res<UiFonts>,
    menu_ui: Option<Res<MenuUi>>,
    hud: Option<Res<crate::map::hud::ActiveHud>>,
    details: Res<LoadingDetails>,
    shown: Query<Entity, With<MenuRoot>>,
    windows_q: Query<&Window>,
    mut windows: ResMut<Windows>,
    mut last: Local<(Vec2, Option<Page>)>,
    mut commands: Commands,
) {
    let size = windows_q
        .iter()
        .next()
        .map_or(Vec2::new(640.0, 480.0), |w| Vec2::new(w.width(), w.height()));
    let resized = (size - last.0).abs().max_element() > 0.5;
    if !menu.is_changed() && !resized && !hud.as_ref().is_some_and(|h| h.is_changed()) && !details.is_changed() {
        return;
    }
    // A dialog opened (or another in its place): in front.
    let page = (menu.open && menu.page != Page::Main).then_some(menu.page);
    if page != last.1
        && let Some(p) = page
    {
        windows.raise(p.window());
    }
    *last = (size, page);
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
            // Over the HUD (40-45), under the frames (`widgets::FRAME_Z`),
            // the scoreboard and the console.
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
        failure_frame(&mut commands, root, &menu, &look, failure);
        return;
    }
    if let Some(map) = &menu.loading {
        loading_frame(&mut commands, root, &menu, &look, map);
        if details.0 != 0 {
            details_panel(&mut commands, root, &look, size);
        }
        return;
    }
    main_list(&mut commands, root, &menu, &look, size, title_font);
    if menu.page == Page::Main {
        return;
    }
    let rows = menu.rows();
    let ctx = Ctx {
        root,
        look: &look,
        menu: &menu,
        rows: &rows,
    };
    // An Advanced dialog shows over the options.
    if menu.page.over_options() {
        let mut under = menu.clone();
        under.page = Page::Settings;
        under.combo = None;
        under.focus = usize::MAX;
        let under_rows = under.rows();
        let c = Ctx {
            root,
            look: &look,
            menu: &under,
            rows: &under_rows,
        };
        dialog(&mut commands, &c, Page::Settings);
    }
    dialog(&mut commands, &ctx, menu.page);
}

/// One of the menu's dialogs in its frame.
fn dialog(commands: &mut Commands, ctx: &Ctx, page: Page) {
    let (look, menu) = (ctx.look, ctx.menu);
    let (w, h) = dialog_size(menu, page);
    let title = match page {
        Page::Main => String::new(),
        Page::NewGame => menu.text("#GameUI_CreateServer", "Create Server"),
        Page::Bots => "Bots".into(),
        Page::Team => "Choose a Team".into(),
        Page::Settings => menu.text("#GameUI_Options", "Options"),
        Page::KeyboardAdvanced => menu.text("#GameUI_KeyboardAdvanced_Title", "Keyboard - Advanced"),
        Page::VideoAdvanced => menu.text("#GameUI_VideoAdvanced_Title", "Video - Advanced"),
        Page::Extras => "Lucker Party Options".into(),
    };
    let mut spec = FrameSpec::new(page.window());
    if page.over_options() {
        spec = spec.modal();
    }
    let parts = widgets::frame(commands, ctx.root, look, spec, (w, h), &title);
    let frame = parts.frame;
    if let Some(close) = parts.close {
        commands.entity(close).insert(Hit(Target::Close, 0));
    }
    match page {
        Page::Settings => {
            let names: Vec<(String, bool)> = TABS.iter().map(|(_, t, o)| (menu.text(t, o), true)).collect();
            widgets::tabs(commands, frame, look, (8.0, 30.0), &names, menu.tab.index(), Some(96.0), |i| Hit(Target::Tab(i), 0));
            let content = sheet_page(commands, frame, look, (8.0, 57.0, w - 16.0, h - 57.0 - 36.0));
            if menu.tab == Tab::Keyboard {
                keyboard_tab(commands, content, look, menu, ctx.rows);
            } else {
                page_controls(commands, ctx, content, (w - 16.0, h - 57.0 - 36.0));
            }
            dialog_buttons(commands, ctx, frame, (w, h), &[Action::Ok, Action::Cancel, Action::Apply]);
        }
        Page::NewGame => {
            let names: Vec<(String, bool)> = CREATE_TABS.iter().map(|(t, o)| (menu.text(t, o), true)).collect();
            widgets::tabs(commands, frame, look, (8.0, 30.0), &names, menu.create_tab, None, |i| Hit(Target::Tab(i), 0));
            let size = (w - 16.0, h - 57.0 - 38.0);
            let content = sheet_page(commands, frame, look, (8.0, 57.0, size.0, size.1));
            if menu.create_tab == 1 {
                game_options(commands, ctx, content, size);
            } else {
                page_controls(commands, ctx, content, size);
            }
            dialog_buttons(commands, ctx, frame, (w, h), &[Action::Start, Action::Cancel]);
        }
        Page::KeyboardAdvanced | Page::VideoAdvanced => {
            page_controls(commands, ctx, frame, (w, h));
        }
        Page::Extras => {
            column(commands, ctx, frame, (16.0, 36.0, w - 32.0), true);
            dialog_buttons(commands, ctx, frame, (w, h), &[Action::Ok, Action::Cancel]);
        }
        _ => column(commands, ctx, frame, (16.0, 36.0, w - 32.0), false),
    }
}

/// A property sheet's page: a raised box under its tabs.
fn sheet_page(commands: &mut Commands, frame: Entity, look: &Look, (x, y, w, h): (f32, f32, f32, f32)) -> Entity {
    commands
        .spawn((
            Node {
                border: UiRect::all(px(1.0)),
                ..place(look, x, y, w, h)
            },
            bevel(look, true),
            ChildOf(frame),
        ))
        .id()
}

/// A dialog's buttons at its bottom right (OK, Cancel, Apply; Start,
/// Cancel), the default one ringed.
fn dialog_buttons(commands: &mut Commands, ctx: &Ctx, frame: Entity, (w, h): (f32, f32), actions: &[Action]) {
    let default = ctx.menu.default_action();
    let mut x = w - 8.0 - (72.0 + 6.0) * actions.len() as f32 + 6.0;
    for action in actions {
        if let Some((i, Row::Button { label, enabled, .. })) = ctx
            .rows
            .iter()
            .enumerate()
            .find(|(_, r)| matches!(r, Row::Button { action: a, .. } if a == action))
        {
            let state = Btn::enabled(*enabled)
                .focused(ctx.menu.focus == i)
                .default_button(default.as_ref() == Some(action));
            widgets::button(commands, frame, ctx.look, (x, h - 8.0 - 24.0, 72.0, 24.0), label, state, 0, Hit(Target::Row(i), 0));
        }
        x += 78.0;
    }
}

fn label_of(row: &Row) -> String {
    match row {
        Row::Button { label, .. } | Row::Control { label, .. } | Row::Bind { label, .. } => label.clone(),
        Row::Heading(t) | Row::Info(t) => t.clone(),
    }
}

/// A row's control at a box (scheme pixels in `parent`): a combo box (its
/// list when open), check box, radio button, slider (its Low / High words
/// under it when the layout gives them), text entry. `text`: a check
/// box's words (the layout's), else none.
#[allow(clippy::too_many_arguments)]
fn control(
    commands: &mut Commands,
    ctx: &Ctx,
    parent: Entity,
    i: usize,
    row: &Row,
    rect: (f32, f32, f32, f32),
    text: Option<&str>,
    layout: Option<&UiControl>,
) {
    let (look, menu) = (ctx.look, ctx.menu);
    let Row::Control { label: name, control, .. } = row else {
        return;
    };
    let focused = menu.focus == i;
    let (x, y, w, h) = rect;
    match control {
        Control::Combo { entries, text: shown, .. } => {
            let open = menu.combo.filter(|c| c.owner == i);
            let ch = h.min(24.0);
            widgets::combo_box(
                commands,
                parent,
                look,
                (x, y, w, ch),
                shown,
                true,
                open.is_some(),
                focused,
                (Hit(Target::Row(i), 0), WheelCombo(i), RelativeCursorPosition::default()),
            );
            if let Some(list) = open {
                widgets::combo_popup(
                    commands,
                    parent,
                    ctx.root,
                    look,
                    (x, y, w, ch),
                    entries,
                    &list,
                    |k| Hit(Target::ComboItem(k), 0),
                    Hit(Target::Outside, 0),
                );
            }
        }
        Control::Check(on) => {
            widgets::check_button(
                commands,
                parent,
                look,
                rect,
                text.unwrap_or(""),
                *on,
                true,
                focused,
                Hit(Target::Row(i), 0),
            );
        }
        Control::Radio(on) => {
            widgets::radio_button(
                commands,
                parent,
                look,
                rect,
                text.unwrap_or(name),
                *on,
                true,
                focused,
                Hit(Target::Row(i), 0),
            );
        }
        Control::Slider { fraction, .. } => {
            let track_h = 24.0;
            widgets::slider(
                commands,
                parent,
                look,
                (x, y, w, track_h),
                *fraction,
                11,
                true,
                SliderTrack {
                    owner: SliderOwner::Menu,
                    id: i,
                    inset: 0.0,
                },
            );
            if focused {
                commands.spawn((
                    place(look, x, y, w, track_h),
                    Outline::new(px(1.0), px(0.0), look.dark()),
                    ChildOf(parent),
                ));
            }
            // VGUI's slider words under its ends (`leftText`, `rightText`).
            let words = |key: &str| {
                layout
                    .and_then(|c| c.keys.get(key))
                    .map(|t| match t.strip_prefix('#') {
                        Some(_) => menu.text(t, ""),
                        None => t.clone(),
                    })
                    .filter(|t| !t.is_empty() && t.parse::<f32>().is_err())
            };
            if h >= 36.0 {
                let font = look.font("DefaultSmall", (13.0, false));
                if let Some(t) = words("lefttext") {
                    label(commands, parent, look, (x, y + track_h, w / 2.0, 14.0), &t, font.clone(), look.text(), -1);
                }
                if let Some(t) = words("righttext") {
                    label(commands, parent, look, (x + w / 2.0, y + track_h, w / 2.0, 14.0), &t, font, look.text(), 1);
                }
            }
        }
        Control::Text { text: value, .. } => {
            let caret = focused.then_some(menu.caret);
            widgets::text_entry(
                commands,
                parent,
                look,
                (x, y, w, h.min(24.0)),
                value,
                caret,
                false,
                true,
                (
                    Hit(Target::Row(i), 0),
                    TextHit {
                        row: i,
                        shown: value.clone(),
                        width: w,
                    },
                ),
            );
        }
    }
}

/// Whether a layout control is a page or frame part, not a control drawn
/// on it.
fn frame_part(c: &UiControl, (w, h): (f32, f32)) -> bool {
    let class = class_of(c);
    let x = coord(c.x);
    let y = coord(c.y);
    matches!(c.kind, UiKind::Frame)
        || matches!(class, "BuildModeDialog" | "Menu" | "FrameSystemButton" | "CPanelListPanel")
        || c.name.starts_with("frame_")
        || (c.wide >= w - 24.0 && c.tall >= h - 64.0)
        || x >= w
        || y >= h
}

/// The open page's controls where the game's layout puts them: those
/// mashup has as working controls, the rest greyed (labels as they are);
/// without the layout, a column.
fn page_controls(commands: &mut Commands, ctx: &Ctx, parent: Entity, (w, h): (f32, f32)) {
    let (look, menu) = (ctx.look, ctx.menu);
    let Some(layout) = menu.layout() else {
        column(commands, ctx, parent, (20.0, 14.0, w - 40.0), menu.page.over_options());
        return;
    };
    let rects = layout.rects(Rect::new(0.0, 0.0, w, h), 1.0);
    let font = look.default_font();
    // Each row's control, by its field's name.
    let row_at = |name: &str| {
        ctx.rows.iter().position(|r| match r {
            Row::Control { field, .. } => menu.field_name(*field).is_some_and(|f| f.eq_ignore_ascii_case(name)),
            Row::Button {
                action: Action::VideoAdvanced,
                ..
            } => name == "AdvancedButton",
            Row::Button {
                action: Action::Ok, ..
            } => name == "Button1" && menu.page.over_options(),
            Row::Button {
                action: Action::Cancel,
                ..
            } => name == "Button2" && menu.page.over_options(),
            _ => false,
        })
    };
    let drawn: Vec<Rect> = layout
        .controls
        .iter()
        .zip(&rects)
        .filter(|(c, _)| row_at(&c.name).is_some())
        .map(|(_, r)| *r)
        .collect();
    // Controls whose label the install has no words for (VR mode): left
    // out with it, as the game hides what it doesn't offer.
    let unworded: Vec<&str> = layout
        .controls
        .iter()
        .filter(|c| c.text.starts_with('#'))
        .filter_map(|c| c.keys.get("associate").map(String::as_str))
        .collect();
    for (c, r) in layout.controls.iter().zip(&rects) {
        let rect = (r.min.x, r.min.y, r.width(), r.height());
        let row = row_at(&c.name);
        if frame_part(c, (w, h)) || (!c.visible && row.is_none()) {
            continue;
        }
        if row.is_none() && unworded.iter().any(|n| n.eq_ignore_ascii_case(&c.name)) {
            continue;
        }
        let class = class_of(c);
        if let Some(i) = row {
            match &ctx.rows[i] {
                Row::Button { label: text, enabled, action } => {
                    let state = Btn::enabled(*enabled)
                        .focused(menu.focus == i)
                        .default_button(matches!(action, Action::Ok));
                    widgets::button(commands, parent, look, rect, text, state, 0, Hit(Target::Row(i), 0));
                }
                row => {
                    let text = (!c.text.is_empty() && !c.text.starts_with('#')).then_some(c.text.as_str());
                    control(commands, ctx, parent, i, row, rect, text, Some(c));
                }
            }
            continue;
        }
        let text = if c.text.starts_with('#') { "" } else { c.text.as_str() };
        match class.to_ascii_lowercase().as_str() {
            "label" => {
                // Not over a control we draw (the video Advanced dialog's
                // note sits where its HDR box is).
                if text.is_empty() || drawn.iter().any(|d| !d.intersect(*r).is_empty()) {
                    continue;
                }
                let color = if c.dull { look.dull() } else { look.text() };
                let f = c.font.as_deref().map_or_else(|| font.clone(), |n| look.font(n, (13.0, false)));
                let align = match c.align {
                    crate::map::hud::UiAlign::East | crate::map::hud::UiAlign::NorthEast | crate::map::hud::UiAlign::SouthEast => 1,
                    crate::map::hud::UiAlign::Center | crate::map::hud::UiAlign::North | crate::map::hud::UiAlign::South => 0,
                    _ => -1,
                };
                if c.wrap || r.height() > 30.0 {
                    let e = commands.spawn((place(look, rect.0, rect.1, rect.2, rect.3), ChildOf(parent))).id();
                    commands.spawn((
                        Text::new(text),
                        f,
                        TextColor(color),
                        TextLayout::new(Justify::Left, LineBreak::WordBoundary),
                        ChildOf(e),
                    ));
                } else {
                    label(commands, parent, look, rect, text, f, color, align);
                }
            }
            "divider" => {
                commands.spawn((
                    Node {
                        border: UiRect::all(px(1.0)),
                        ..place(look, rect.0, rect.1, rect.2, 2.0)
                    },
                    bevel(look, false),
                    ChildOf(parent),
                ));
            }
            "button" | "urlbutton" | "togglebutton" => {
                if !text.is_empty() {
                    widgets::button(commands, parent, look, rect, text, Btn::enabled(false), 0, ());
                }
            }
            // Without words (tokens this install lacks) a greyed box says
            // nothing: left out.
            "checkbutton" | "ccvartogglecheckbutton" | "ccvarnegatecheckbutton" if !text.is_empty() => {
                widgets::check_button(commands, parent, look, rect, text, false, false, false, ());
            }
            "radiobutton" if !text.is_empty() => {
                widgets::radio_button(commands, parent, look, rect, text, false, false, false, ());
            }
            "combobox" | "clabeledcommandcombobox" => {
                widgets::combo_box(commands, parent, look, (rect.0, rect.1, rect.2, rect.3.min(24.0)), "", false, false, false, ());
            }
            "ccvarslider" | "slider" => {
                widgets::slider(
                    commands,
                    parent,
                    look,
                    (rect.0, rect.1, rect.2, 24.0),
                    0.0,
                    11,
                    false,
                    SliderTrack {
                        owner: SliderOwner::Menu,
                        id: usize::MAX,
                        inset: 0.0,
                    },
                );
            }
            "textentry" => {
                widgets::text_entry(commands, parent, look, (rect.0, rect.1, rect.2, rect.3.min(24.0)), "", None, false, false, ());
            }
            "crosshairimagepanelcs" => crosshair_preview(commands, parent, look, menu, rect),
            "imagepanel" => {
                commands.spawn((
                    Node {
                        border: UiRect::all(px(1.0)),
                        ..place(look, rect.0, rect.1, rect.2, rect.3)
                    },
                    bevel(look, false),
                    BackgroundColor(look.sunken_bg()),
                    ChildOf(parent),
                ));
            }
            _ => {}
        }
    }
}

/// Rows of labelled controls in a column at (x, y), `w` wide: ours (Lucker
/// Party Options), the Bots and Team pages, and pages without the
/// install's layout; `buttons`: the dialog draws its own buttons.
fn column(commands: &mut Commands, ctx: &Ctx, parent: Entity, (x, y, w): (f32, f32, f32), buttons: bool) {
    let (look, menu) = (ctx.look, ctx.menu);
    let row_h = 28.0;
    let font = look.font("Default", (16.0, false));
    let label_w = (w * 0.45).round();
    let control_w = (w - label_w).min(220.0);
    let mut ry = y;
    for (i, row) in ctx.rows.iter().enumerate() {
        match row {
            Row::Info(t) | Row::Heading(t) => {
                label(commands, parent, look, (x, ry, w, row_h - 4.0), t, font.clone(), look.dull(), -1);
            }
            Row::Button { label: text, enabled, action } => {
                if buttons && matches!(action, Action::Ok | Action::Cancel | Action::Apply | Action::Start) {
                    continue;
                }
                let state = Btn::enabled(*enabled).focused(menu.focus == i);
                widgets::button(commands, parent, look, (x, ry, w.min(260.0), 24.0), text, state, -1, Hit(Target::Row(i), 0));
            }
            Row::Bind { .. } => continue,
            Row::Control { label: text, control: c, .. } => {
                if matches!(c, Control::Check(_) | Control::Radio(_)) {
                    control(commands, ctx, parent, i, row, (x, ry, w, 24.0), Some(text), None);
                } else {
                    label(commands, parent, look, (x, ry, label_w, 24.0), text, font.clone(), look.text(), -1);
                    let slider = matches!(c, Control::Slider { .. });
                    let cw = if slider { control_w - 48.0 } else { control_w };
                    control(commands, ctx, parent, i, row, (x + label_w, ry, cw, 24.0), None, None);
                    if let Control::Slider { text: value, .. } = c {
                        label(commands, parent, look, (x + label_w + control_w - 44.0, ry, 44.0, 24.0), value, font.clone(), look.text(), 1);
                    }
                }
            }
        }
        ry += row_h;
    }
}

/// Create Server's Game page: the game's server options (`settings.scr`)
/// in a scrolled list, each its label and control (CS:S's
/// `CPanelListPanel`, where the layout puts it).
fn game_options(commands: &mut Commands, ctx: &Ctx, parent: Entity, (w, h): (f32, f32)) {
    let (look, menu) = (ctx.look, ctx.menu);
    let at = menu
        .layout()
        .and_then(|l| l.get("GameOptions"))
        .map(|c| (coord(c.x), coord(c.y), c.wide, c.tall));
    let (lx, ly, lw, lh) = at.unwrap_or((10.0, 12.0, w - 20.0, h - 24.0));
    let list = commands
        .spawn((
            Node {
                border: UiRect::all(px(1.0)),
                overflow: Overflow::clip(),
                ..place(look, lx, ly, lw, lh)
            },
            bevel(look, false),
            RelativeCursorPosition::default(),
            WheelList,
            ChildOf(parent),
        ))
        .id();
    let (len, shown) = menu.list();
    let bar_w = look.number("ScrollBar.Wide", 17.0);
    let row_h = ((lh - 4.0) / shown as f32).floor();
    let font = look.default_font();
    let inner = lw - bar_w - 4.0;
    let label_w = (inner * 0.5).round();
    for (k, i) in (menu.scroll..(menu.scroll + shown).min(len)).enumerate() {
        let y = 2.0 + k as f32 * row_h;
        let row = &ctx.rows[i];
        if let Row::Control { label: text, control: c, .. } = row {
            if matches!(c, Control::Check(_)) {
                control(commands, ctx, list, i, row, (4.0, y, inner - 4.0, 24.0), Some(text), None);
            } else {
                label(commands, list, look, (6.0, y, label_w - 8.0, 24.0), text, font.clone(), look.text(), -1);
                control(commands, ctx, list, i, row, (label_w, y, inner - label_w, 24.0), None, None);
            }
        }
    }
    scroll_bar(commands, list, look, (lw - bar_w - 2.0, 0.0, lh - 2.0), (len, shown, menu.scroll));
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

/// A layout coordinate from its start edge (dialog layouts use plain
/// numbers).
fn coord(c: crate::map::hud::HudCoord) -> f32 {
    match c {
        crate::map::hud::HudCoord::Start(v) | crate::map::hud::HudCoord::Centre(v) | crate::map::hud::HudCoord::End(v) => v,
    }
}

/// A map the main menu started is loading: GameUI's loading dialog (its
/// layout from the install, `GameUi::loading`: the frame, the stage line,
/// the progress bar, Cancel), the menu's entries hidden. The stage and
/// the bar follow the load (`loading_progress`).
fn loading_frame(commands: &mut Commands, root: Entity, menu: &GameMenu, look: &Look, map: &str) {
    let layout = look.ui.and_then(|u| u.loading.as_ref());
    let at = |name: &str, fallback: (f32, f32, f32, f32)| {
        layout
            .and_then(|l| l.get(name))
            .map_or(fallback, |c| (coord(c.x), coord(c.y), c.wide, c.tall))
    };
    let (_, _, w, h) = at("LoadingDialog", (0.0, 0.0, 380.0, 112.0));
    let title = menu.text("#GameUI_Loading", "Loading...");
    let f = widgets::frame(commands, root, look, FrameSpec::new("loading").no_close(), (w, h), &title).frame;
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
    widgets::button(
        commands,
        f,
        look,
        at("CancelButton", (288.0, 64.0, 72.0, 24.0)),
        &text,
        Btn::enabled(true).default_button(true),
        0,
        Hit(Target::Cancel, 0),
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
fn failure_frame(commands: &mut Commands, root: Entity, menu: &GameMenu, look: &Look, text: &str) {
    let (w, h) = (380.0, 150.0);
    let title = menu.text("#GameUI_Disconnected", "Disconnected");
    let parts = widgets::frame(commands, root, look, FrameSpec::new("loading"), (w, h), &title);
    if let Some(close) = parts.close {
        commands.entity(close).insert(Hit(Target::Close, 0));
    }
    let body = commands
        .spawn((
            Node {
                overflow: Overflow::clip(),
                ..place(look, 20.0, 34.0, w - 40.0, h - 34.0 - 40.0)
            },
            ChildOf(parts.frame),
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
    widgets::button(
        commands,
        parts.frame,
        look,
        (w - 20.0 - 72.0, h - 34.0, 72.0, 24.0),
        &close,
        Btn::enabled(true).default_button(true),
        0,
        Hit(Target::Cancel, 0),
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
/// pointer comes onto an entry or button, the click as one is pressed,
/// the release as it is let go.
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
        // The outside of an open list isn't a button.
        if matches!(hit.0, Target::Outside) {
            continue;
        }
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
            .map_or(fallback, |c| (coord(c.x), coord(c.y), c.wide, c.tall))
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
    let names = ["Defaults", "ChangeKeyButton", "ClearKeyButton", "KeyAdvancedButton"];
    let fallback = [
        (8.0, 276.0, 134.0, 24.0),
        (272.0, 276.0, 106.0, 24.0),
        (384.0, 276.0, 105.0, 24.0),
        (148.0, 276.0, 111.0, 24.0),
    ];
    for (k, name) in names.iter().enumerate() {
        let i = len + k;
        let Row::Button { label: text, enabled, .. } = &rows[i] else { continue };
        let state = Btn::enabled(*enabled).focused(menu.focus == i);
        widgets::button(commands, content, look, rect(name, fallback[k]), text, state, 0, Hit(Target::Row(i), 0));
    }
}

/// The crosshair as the multiplayer tab's settings draw it, in the
/// layout's crosshair box.
fn crosshair_preview(commands: &mut Commands, parent: Entity, look: &Look, menu: &GameMenu, (x, y, w, h): (f32, f32, f32, f32)) {
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
    let e = commands
        .spawn((
            Node {
                border: UiRect::all(px(1.0)),
                ..place(look, x, y, w, h)
            },
            bevel(look, false),
            BackgroundColor(Color::srgb(0.25, 0.27, 0.3)),
            ChildOf(parent),
        ))
        .id();
    // As at this window's height, in screen pixels.
    let centre = Vec2::new(w, h) * look.s / 2.0;
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::client::options::setting_index;
    use crate::map::hud::{GameUiItem, ServerSetting};

    const CVARS: [(&str, &str); 14] = [
        ("sensitivity", "3"),
        ("m_pitch", "0.022"),
        ("zoom_sensitivity_ratio", "1.2"),
        ("volume", "1"),
        ("viewmodel_fov", "54"),
        ("cl_righthand", "1"),
        ("cl_showfps", "0"),
        ("mat_vsync", "1"),
        ("mat_antialias", "4"),
        ("mashup_resolution", ""),
        ("mashup_rounds", "1"),
        ("bot_reaction", "0.35"),
        ("hud_fastswitch", "0"),
        ("con_enable", "1"),
    ];

    fn get(n: &str) -> Option<String> {
        CVARS
            .iter()
            .chain(&[("hostname", "Mine"), ("mp_friendlyfire", "0"), ("bot_prefix", "")])
            .find(|(k, _)| *k == n)
            .map(|(_, v)| v.to_string())
    }

    /// The menu opened in a game (Esc).
    fn menu() -> GameMenu {
        let mut m = GameMenu {
            in_game: true,
            ..default()
        };
        m.open(Page::Main, vec!["cs_office".into(), "de_dust2".into(), "de_nuke".into()], Some("de_dust2"), get);
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
            let o = m.handle(i.clone());
            all.lines.extend(o.lines);
            all.after_load.extend(o.after_load);
            all.close |= o.close;
            all.clipboard = o.clipboard.or(all.clipboard);
        }
        all
    }

    /// Click the row `row` finds.
    fn click_on(m: &mut GameMenu, row: impl Fn(&GameMenu) -> usize) -> Outcome {
        let i = row(m);
        press(m, &[Input::Click(Target::Row(i), 0)])
    }

    fn setting(cvar: &str) -> usize {
        SETTINGS.iter().position(|s| s.cvar == cvar).unwrap()
    }

    /// The row of the open page with this field.
    fn row_with(m: &GameMenu, field: Field) -> usize {
        m.rows().iter().position(|r| r.field() == Some(field)).unwrap_or_else(|| panic!("no {field:?}"))
    }

    fn row_of(m: &GameMenu, cvar: &str) -> usize {
        row_with(m, Field::Setting(setting(cvar)))
    }

    fn button(m: &GameMenu, action: Action) -> usize {
        m.rows()
            .iter()
            .position(|r| matches!(r, Row::Button { action: a, .. } if *a == action))
            .unwrap()
    }

    fn control(m: &GameMenu, i: usize) -> Control {
        match &m.rows()[i] {
            Row::Control { control, .. } => control.clone(),
            r => panic!("{r:?}"),
        }
    }

    #[test]
    fn opens_with_the_game_as_it_is() {
        let m = menu();
        assert!(m.open);
        assert_eq!((m.page, m.focus), (Page::Main, 0));
        assert_eq!(m.new_game.map, 1, "the loaded map");
        assert_eq!(m.new_game.difficulty, NORMAL);
        assert!(!m.new_game.bots_on, "no bots in the game now");
        // No crosshair colour cvar given: not available.
        assert_eq!(m.values[setting("cl_crosshaircolor")], None);
        assert_eq!(m.entries().len(), MAIN.len() + OURS.len());
    }

    #[test]
    fn in_game_entries_show_only_in_a_game() {
        use MainItem::*;
        let m = menu();
        assert_eq!(
            shown(&m),
            [Resume, Disconnect, FindServers, NewGame, BugReport, Options, Quit, QuickStart, Greybox, Bots, Team, Extras, Console]
        );
        let e = m.entries();
        assert!(e[2].gap && e[2].enabled, "a gap under Disconnect, then Find Servers");
        assert!(e[7].gap, "ours after a gap");
        let m = main_menu();
        assert_eq!(shown(&m), [FindServers, NewGame, BugReport, Options, Quit, QuickStart, Greybox, Extras, Console]);
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
        // Esc cancels a dialog, back to the main menu.
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
        assert_eq!((m.page, m.create_tab), (Page::NewGame, 0));
        // Enter: Start, the default button.
        let o = press(&mut m, &[Input::Activate]);
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
        assert_eq!(o.after_load.len(), QUICK_BOTS as usize);
        assert_eq!(o.after_load.iter().filter(|l| *l == "bot_add 1").count(), 5, "five terrorists, four CTs");
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
        assert_eq!(m.focus, 4, "wraps past the info line to Close");
        press(&mut m, &[Input::Focus(1)]);
        assert_eq!(m.focus, 1, "Tab wraps too");
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

    /// Create Server driven as CS:S's: the map from its drop-down, bots
    /// included, how many typed, a difficulty button, the Bot page's team,
    /// the Game page's options; Start sets what changed, loads the map,
    /// then adds the bots.
    #[test]
    fn create_server_by_its_controls() {
        let mut m = menu();
        let ui = GameUi {
            server_settings: vec![
                ServerSetting {
                    cvar: "hostname".into(),
                    label: "Server Name".into(),
                    kind: ServerSettingKind::Text,
                    default: "Counter-Strike Source".into(),
                },
                ServerSetting {
                    cvar: "mp_friendlyfire".into(),
                    label: "Friendly fire".into(),
                    kind: ServerSettingKind::Bool,
                    default: "0".into(),
                },
                ServerSetting {
                    cvar: "mp_timelimit".into(),
                    label: "Time limit".into(),
                    kind: ServerSettingKind::Number { min: Some(0.0), max: None },
                    default: "20".into(),
                },
            ],
            ..default()
        };
        m.set_ui(Some(Arc::new(ui)));
        m.open(Page::Main, vec!["cs_office".into(), "de_dust2".into(), "de_nuke".into()], Some("de_dust2"), get);
        click(&mut m, MainItem::NewGame);
        // The map: a click opens its list on the map now, Down and Enter
        // pick the next.
        let map = row_with(&m, Field::Map);
        press(&mut m, &[Input::Click(Target::Row(map), 0)]);
        assert_eq!(m.combo.map(|c| (c.owner, c.highlight, c.len)), Some((map, 1, 3)));
        press(&mut m, &[Input::Down, Input::Activate]);
        assert_eq!((m.new_game.map, m.combo), (2, None));
        // Include bots: their count and difficulty show.
        assert!(m.rows().iter().all(|r| r.field() != Some(Field::BotCount)));
        click_on(&mut m, |m| row_with(m, Field::BotsOn));
        let count = row_with(&m, Field::BotCount);
        // Tab to the count (its text selected), type over it; only digits.
        press(&mut m, &[Input::Focus(1)]);
        assert_eq!(m.focus, count);
        press(&mut m, &[Input::Type("3x".into())]);
        assert_eq!(m.new_game.bot_count, "3");
        press(&mut m, &[Input::Type("45".into())]);
        assert_eq!(m.new_game.bot_count, "3", "two digits at most");
        click_on(&mut m, |m| row_with(m, Field::Difficulty(2)));
        assert_eq!(m.new_game.difficulty, 2);
        assert_eq!(control(&m, row_with(&m, Field::Difficulty(2))), Control::Radio(true));
        // The Bot page: counter-terrorists only.
        press(&mut m, &[Input::Click(Target::Tab(2), 0)]);
        assert_eq!(m.create_tab, 2);
        let team = row_with(&m, Field::BotTeam);
        press(&mut m, &[Input::Click(Target::Row(team), 0), Input::Click(Target::ComboItem(2), 0)]);
        assert_eq!(m.new_game.bot_team, 2);
        // The Game page: friendly fire on (Ctrl+Tab back to it).
        press(&mut m, &[Input::NextTab(-1)]);
        assert_eq!(m.create_tab, 1);
        let rows = m.rows();
        let ff = rows
            .iter()
            .position(|r| matches!(r, Row::Control { label, .. } if label == "Friendly fire"))
            .unwrap();
        assert!(!rows.iter().any(|r| matches!(r, Row::Control { label, .. } if label == "Time limit")), "mashup lacks it");
        let o = press(&mut m, &[Input::Click(Target::Row(ff), 0)]);
        assert!(o.lines.is_empty(), "nothing runs before Start: {:?}", o.lines);
        let o = click_on(&mut m, |m| button(m, Action::Start));
        assert_eq!(
            o.lines,
            [
                "bot_kick",
                "mashup_rounds 1",
                "mp_friendlyfire 1",
                "bot_reaction 0.2",
                "bot_aim_error 1.5",
                "bot_turn_rate 540",
                "map de_nuke",
            ]
        );
        assert_eq!(o.after_load, ["bot_add 2", "bot_add 2", "bot_add 2"]);
        assert!(o.close && !m.open);
    }

    #[test]
    fn create_servers_combo_boxes_take_the_wheel_letters_and_escape() {
        let mut m = menu();
        click(&mut m, MainItem::NewGame);
        let map = row_with(&m, Field::Map);
        // Closed and focused: Up/Down and the wheel step it, no wrapping.
        press(&mut m, &[Input::FocusRow(map), Input::Down, Input::Down, Input::Down]);
        assert_eq!(m.new_game.map, 2);
        press(&mut m, &[Input::Wheel(map, 1)]);
        assert_eq!(m.new_game.map, 1);
        // A letter jumps.
        press(&mut m, &[Input::Char('c')]);
        assert_eq!(m.new_game.map, 0);
        // Open: hover highlights, Esc closes without a pick, a click outside
        // too.
        press(&mut m, &[Input::Space, Input::Hover(Target::ComboItem(2))]);
        assert_eq!(m.combo.map(|c| c.highlight), Some(2));
        press(&mut m, &[Input::Close]);
        assert_eq!((m.combo, m.new_game.map, m.page), (None, 0, Page::NewGame), "Esc closed only the list");
        press(&mut m, &[Input::Space, Input::Click(Target::Outside, 0)]);
        assert_eq!((m.combo, m.new_game.map), (None, 0));
        // Its wheel scrolls the list, not the value.
        press(&mut m, &[Input::Space, Input::Scroll(1)]);
        assert_eq!(m.new_game.map, 0);
        press(&mut m, &[Input::Char('d'), Input::Activate]);
        assert_eq!(m.new_game.map, 1, "a letter in the open list, Enter picks");
    }

    #[test]
    fn no_maps_no_start() {
        let mut m = GameMenu::default();
        m.open(Page::NewGame, Vec::new(), None, |_| None);
        let start = button(&m, Action::Start);
        let o = press(&mut m, &[Input::Click(Target::Row(start), 0)]);
        assert!(o.lines.is_empty() && m.open);
        // The empty map box doesn't open a list.
        press(&mut m, &[Input::Click(Target::Row(0), 0)]);
        assert_eq!((m.page, m.combo), (Page::NewGame, None));
    }

    #[test]
    fn options_ok_cancel_and_apply() {
        let mut m = menu();
        click(&mut m, MainItem::Options);
        assert_eq!((m.page, m.tab), (Page::Settings, Tab::Keyboard));
        let apply = button(&m, Action::Apply);
        assert!(matches!(m.rows()[apply], Row::Button { enabled: false, .. }), "nothing to apply yet");
        press(&mut m, &[Input::NextTab(1)]);
        assert_eq!(m.tab, Tab::Mouse);
        // Reverse mouse, then the sensitivity slider pressed at its end and
        // dragged: once per change.
        let reverse = row_of(&m, "m_pitch");
        let o = press(&mut m, &[Input::Click(Target::Row(reverse), 0)]);
        assert_eq!(o.lines, ["m_pitch -0.022"]);
        let slider = row_of(&m, "sensitivity");
        let o = press(&mut m, &[Input::Slide(slider, 1.0), Input::Slide(slider, 1.0), Input::Slide(slider, 0.0)]);
        assert_eq!(o.lines, ["sensitivity 20.0", "sensitivity 0.1"]);
        // Focused, Right steps it.
        assert_eq!(press(&mut m, &[Input::Right]).lines, ["sensitivity 0.2"]);
        // Apply keeps them; Cancel puts back only what changed after.
        let apply = button(&m, Action::Apply);
        assert!(matches!(m.rows()[apply], Row::Button { enabled: true, .. }));
        press(&mut m, &[Input::Click(Target::Row(apply), 0)]);
        assert!(matches!(m.rows()[apply], Row::Button { enabled: false, .. }), "applied");
        assert_eq!(press(&mut m, &[Input::FocusRow(slider), Input::Right]).lines, ["sensitivity 0.3"]);
        let o = click_on(&mut m, |m| button(m, Action::Cancel));
        assert_eq!(o.lines, ["sensitivity 0.2"]);
        assert_eq!(m.page, Page::Main);
        // OK keeps; Esc is Cancel.
        click(&mut m, MainItem::Options);
        press(&mut m, &[Input::Click(Target::Tab(1), 0)]);
        let reverse = row_of(&m, "m_pitch");
        press(&mut m, &[Input::Click(Target::Row(reverse), 0)]);
        assert!(click_on(&mut m, |m| button(m, Action::Ok)).lines.is_empty());
        click(&mut m, MainItem::Options);
        press(&mut m, &[Input::Click(Target::Tab(1), 0), Input::Click(Target::Row(reverse), 0)]);
        assert_eq!(press(&mut m, &[Input::Close]).lines, ["m_pitch 0.022"], "back to what OK kept");
        // The close box is Cancel too.
        click(&mut m, MainItem::Options);
        press(&mut m, &[Input::Click(Target::Tab(1), 0), Input::Click(Target::Row(reverse), 0)]);
        assert_eq!(press(&mut m, &[Input::Click(Target::Close, 0)]).lines, ["m_pitch 0.022"]);
        assert!(m.open && m.in_game, "the menu stays");
    }

    #[test]
    fn video_options_and_its_advanced_dialog() {
        let mut m = menu();
        m.resolutions = vec!["1920x1080".into(), "1280x720".into()];
        click(&mut m, MainItem::Options);
        press(&mut m, &[Input::Click(Target::Tab(3), 0)]);
        assert_eq!(m.tab, Tab::Video);
        // The resolution's drop-down lists the window sizes.
        let res = row_of(&m, "mashup_resolution");
        press(&mut m, &[Input::Click(Target::Row(res), 0)]);
        assert_eq!(m.combo.map(|c| c.len), Some(2));
        let o = press(&mut m, &[Input::Click(Target::ComboItem(1), 0)]);
        assert_eq!(o.lines, ["mashup_resolution 1280x720"]);
        // Advanced...: vsync as a drop-down, antialiasing; Cancel puts them
        // back, the options stay open.
        let adv = button(&m, Action::VideoAdvanced);
        press(&mut m, &[Input::Click(Target::Row(adv), 0)]);
        assert_eq!(m.page, Page::VideoAdvanced);
        let aa = row_of(&m, "mat_antialias");
        assert!(matches!(control(&m, aa), Control::Combo { selected: Some(2), .. }));
        let o = press(&mut m, &[Input::Click(Target::Row(aa), 0), Input::Up, Input::Up, Input::Activate]);
        assert_eq!(o.lines, ["mat_antialias 0"]);
        let o = click_on(&mut m, |m| button(m, Action::Cancel));
        assert_eq!(o.lines, ["mat_antialias 4"]);
        assert_eq!((m.page, m.focus), (Page::Settings, adv));
        // Show FPS and the hand aren't CS:S's: in our own dialog.
        assert!(m.rows().iter().all(|r| r.field() != Some(Field::Setting(setting("cl_showfps")))));
        click(&mut m, MainItem::Extras);
        assert_eq!(m.page, Page::Extras);
        let hand = row_of(&m, "cl_righthand");
        let o = press(&mut m, &[Input::Click(Target::Row(hand), 0), Input::Click(Target::ComboItem(0), 0)]);
        assert_eq!(o.lines, ["cl_righthand 0"]);
        assert!(matches!(control(&m, hand), Control::Combo { text, .. } if text == "Left"));
    }

    #[test]
    fn the_sensitivity_text_entry_beside_its_slider() {
        // With the game's mouse layout: the slider and the text entry.
        let mut layout = UiLayout::default();
        for (name, kind) in [("ReverseMouse", "CCvarNegateCheckButton"), ("Slider", "CCvarSlider"), ("SensitivityLabel", "TextEntry")] {
            layout
                .controls
                .push(UiControl::new(name, UiKind::Other(kind.into()), 0.0, 0.0, 100.0, 24.0));
        }
        let ui = GameUi {
            options: HashMap::from([("mouse".to_string(), layout)]),
            ..default()
        };
        let mut m = menu();
        m.set_ui(Some(Arc::new(ui)));
        click(&mut m, MainItem::Options);
        press(&mut m, &[Input::Click(Target::Tab(1), 0)]);
        let entry = row_with(&m, Field::SettingText(setting("sensitivity")));
        assert!(matches!(control(&m, entry), Control::Text { text, .. } if text == "3.0"));
        // A click places the caret; the text typed sets the cvar once it
        // reads as a number in range.
        press(&mut m, &[Input::Click(Target::Row(entry), 0), Input::Caret(entry, 3), Input::Edit(Edit::Home(true))]);
        assert_eq!(m.caret, Caret { at: 0, anchor: 3 });
        let o = press(&mut m, &[Input::Copy]);
        assert_eq!(o.clipboard.as_deref(), Some("3.0"));
        let o = press(&mut m, &[Input::Type("2".into()), Input::Type("5".into())]);
        assert_eq!(o.lines, ["sensitivity 2"], "25 is out of range");
        assert!(matches!(control(&m, entry), Control::Text { text, .. } if text == "25"), "as typed");
        assert_eq!(m.values[setting("sensitivity")].as_deref(), Some("2"));
        // Without the layout no entry: the slider shows the number.
        let mut m = menu();
        click(&mut m, MainItem::Options);
        press(&mut m, &[Input::Click(Target::Tab(1), 0)]);
        assert!(m.rows().iter().all(|r| !matches!(r.field(), Some(Field::SettingText(_)))));
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
        assert_eq!(m.list(), (rows.len() - 7, KEY_ROWS));
    }

    /// Keyboard > Advanced...: CS:S's two check boxes (fast weapon switch,
    /// developer console) in its order; ticking applies at once, Cancel
    /// (or Esc) puts them back, OK keeps them.
    #[test]
    fn the_keyboard_tabs_advanced_dialog() {
        let mut m = keyboard();
        let at = button(&m, Action::Advanced);
        press(&mut m, &[Input::Click(Target::Row(at), 0)]);
        assert_eq!(m.page, Page::KeyboardAdvanced);
        let rows = m.rows();
        let labels: Vec<String> = rows.iter().map(label_of).collect();
        assert_eq!(labels, ["Fast weapon switch", "Enable developer console", "OK", "Cancel"]);
        assert!(matches!(&rows[0], Row::Control { control: Control::Check(false), .. }), "{:?}", rows[0]);
        assert_eq!(m.focus, 0);
        let o = press(&mut m, &[Input::Space]);
        assert_eq!(o.lines, ["hud_fastswitch 1"]);
        let o = press(&mut m, &[Input::Click(Target::Row(1), 0)]);
        assert_eq!(o.lines, ["con_enable 0"]);
        // Cancel: both back.
        let o = press(&mut m, &[Input::Click(Target::Row(3), 0)]);
        assert_eq!(o.lines, ["hud_fastswitch 0", "con_enable 1"]);
        assert_eq!((m.page, m.tab, m.focus), (Page::Settings, Tab::Keyboard, at));
        // Enter on the check box: OK, the default button, keeps it.
        press(&mut m, &[Input::Activate, Input::Space]);
        let o = press(&mut m, &[Input::Activate]);
        assert!(o.lines.is_empty());
        assert_eq!(m.page, Page::Settings);
        assert_eq!(m.values[setting_index("hud_fastswitch").unwrap()].as_deref(), Some("1"));
        // Esc on the dialog cancels it, leaving the options open.
        press(&mut m, &[Input::Activate, Input::Focus(1), Input::Space]);
        let o = press(&mut m, &[Input::Close]);
        assert_eq!(o.lines, ["con_enable 1"]);
        assert!(m.open && m.page == Page::Settings);
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
        let edit = button(&m, Action::EditKey);
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
            [Resume, Disconnect, FindServers, NewGame, BugReport, Options, Quit, QuickStart, Greybox, Bots, Team, Extras, Console]
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
    fn hover_moves_focus_only_on_the_main_page() {
        let mut m = menu();
        let options = at(&m, MainItem::Options);
        press(&mut m, &[Input::Hover(Target::Main(options))]);
        assert_eq!(m.focus, options);
        press(&mut m, &[Input::Activate, Input::NextTab(1)]);
        assert_eq!(m.page, Page::Settings);
        let focus = m.focus;
        // Neither the left-hand list nor a hover takes the dialog's focus.
        press(&mut m, &[Input::Hover(Target::Main(1)), Input::Hover(Target::Row(focus + 1))]);
        assert_eq!(m.focus, focus);
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

    #[test]
    fn menuinput_words() {
        let mut m = menu();
        click(&mut m, MainItem::NewGame);
        assert_eq!(menu_input(&m, &["focus".into(), "map".into()]), Ok(Input::FocusRow(0)));
        assert_eq!(menu_input(&m, &["open".into()]), Ok(Input::Space));
        assert_eq!(menu_input(&m, &["pick".into(), "2".into()]), Ok(Input::Click(Target::ComboItem(2), 0)));
        assert!(menu_input(&m, &["focus".into(), "nothing".into()]).is_err());
        assert!(m.describe().starts_with("NewGame, focus 0"));
    }
}
