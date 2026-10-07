//! The in-game menu (Esc, or `menu [page]`), styled after CS:S's game
//! menu: a list of entries at the left over the running game and a dark
//! translucent dialog per page. The game keeps running while it is open;
//! the mouse is free, so the local player's input is ignored
//! (`input::write_local_intent`). Every choice is a console line (`map`,
//! `bot_add`, `jointeam`, cvars ...), so the menu holds no game logic: the
//! model (`GameMenu::handle`) turns keys and clicks into those lines, and
//! is unit-tested without a window.

use bevy::{prelude::*, window::CursorOptions};

use crate::{
    console::{Console, ConsoleAppExt},
    core::{Intent, Team},
};

pub struct GameMenuPlugin;

impl Plugin for GameMenuPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<GameMenu>()
            .init_resource::<AfterLoad>()
            .init_resource::<RegrabCursor>()
            .add_systems(Startup, load_fonts)
            .add_systems(
                Update,
                (
                    keys.before(super::console::toggle),
                    pointer,
                    counts,
                    cursor,
                    after_load,
                    draw,
                )
                    .chain(),
            );
        app.console_command(
            "menu",
            "menu [main|newgame|maps|bots|team|options]: open the game menu (Esc) on a page.",
            |w, a| {
                let page = match a.first().map(|s| s.to_lowercase()).as_deref() {
                    None | Some("main") => Page::Main,
                    Some("newgame") => Page::NewGame,
                    Some("maps") => Page::Maps,
                    Some("bots") => Page::Bots,
                    Some("team") => Page::Team,
                    Some("options") | Some("settings") => Page::Settings,
                    Some(p) => return Err(format!("no menu page \"{p}\"")),
                };
                open_menu(w, page);
                Ok(None)
            },
        )
        .console_command("gameui_activate", "Open the game menu (Esc).", |w, _| {
            open_menu(w, Page::Main);
            Ok(None)
        })
        .console_command("gameui_hide", "Close the game menu.", |w, _| {
            w.resource_mut::<GameMenu>().open = false;
            w.resource_mut::<RegrabCursor>().0 = true;
            Ok(None)
        });
    }
}

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

/// How a setting's value changes.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum SettingKind {
    /// Steps of `step` between `min` and `max`, shown with `decimals`.
    Range {
        min: f32,
        max: f32,
        step: f32,
        decimals: usize,
    },
    /// Fixed values (as the cvar takes them) with their labels.
    Choice(&'static [(&'static str, &'static str)]),
}

/// A setting on the options page: one cvar (archived, so config.cfg keeps
/// it).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Setting {
    pub cvar: &'static str,
    pub label: &'static str,
    pub kind: SettingKind,
}

pub const SETTINGS: &[Setting] = &[
    Setting {
        cvar: "sensitivity",
        label: "Mouse sensitivity",
        kind: SettingKind::Range {
            min: 0.1,
            max: 20.0,
            step: 0.1,
            decimals: 1,
        },
    },
    Setting {
        cvar: "zoom_sensitivity_ratio",
        label: "Zoom sensitivity ratio",
        kind: SettingKind::Range {
            min: 0.1,
            max: 4.0,
            step: 0.1,
            decimals: 1,
        },
    },
    Setting {
        cvar: "volume",
        label: "Master volume",
        kind: SettingKind::Range {
            min: 0.0,
            max: 1.0,
            step: 0.05,
            decimals: 2,
        },
    },
    Setting {
        cvar: "viewmodel_fov",
        label: "View model FOV",
        kind: SettingKind::Range {
            min: 40.0,
            max: 90.0,
            step: 1.0,
            decimals: 0,
        },
    },
    Setting {
        cvar: "cl_righthand",
        label: "Weapon hand",
        kind: SettingKind::Choice(&[("0", "Left"), ("1", "Right")]),
    },
    Setting {
        cvar: "cl_crosshaircolor",
        label: "Crosshair colour",
        kind: SettingKind::Choice(&[
            ("0", "Green"),
            ("1", "Red"),
            ("2", "Blue"),
            ("3", "Yellow"),
            ("4", "Cyan"),
        ]),
    },
    Setting {
        cvar: "cl_showfps",
        label: "Show FPS",
        kind: SettingKind::Choice(&[("0", "Off"), ("1", "On"), ("2", "Detailed")]),
    },
    Setting {
        cvar: "mat_hdr_level",
        label: "High dynamic range (next map)",
        kind: SettingKind::Choice(&[("0", "None"), ("1", "Bloom"), ("2", "Full")]),
    },
];

impl Setting {
    /// The value `dir` steps from `current` (wrapping through choices,
    /// clamped in ranges).
    pub fn step(&self, current: &str, dir: i32) -> String {
        match self.kind {
            SettingKind::Range {
                min,
                max,
                step,
                decimals,
            } => {
                let v = current.trim().parse::<f32>().unwrap_or(min);
                // Onto the step grid, so 1.23 + 0.1 reads 1.3.
                let n = ((v - min) / step).round() + dir as f32;
                let v = (min + n * step).clamp(min, max);
                format!("{v:.decimals$}")
            }
            SettingKind::Choice(choices) => {
                let i = choices.iter().position(|(v, _)| *v == current.trim()).unwrap_or(0) as i32;
                let n = choices.len() as i32;
                choices[(i + dir).rem_euclid(n) as usize].0.to_string()
            }
        }
    }

    /// The value as shown.
    pub fn show(&self, value: &str) -> String {
        match self.kind {
            SettingKind::Range { decimals, .. } => value
                .trim()
                .parse::<f32>()
                .map_or(value.to_string(), |v| format!("{v:.decimals$}")),
            SettingKind::Choice(choices) => choices
                .iter()
                .find(|(v, _)| *v == value.trim())
                .map_or(value.to_string(), |(_, l)| l.to_string()),
        }
    }
}

/// The left-hand entries.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MainItem {
    Resume,
    NewGame,
    Bots,
    Team,
    Options,
    BugReport,
    Quit,
}

pub const MAIN: [(MainItem, &str); 7] = [
    (MainItem::Resume, "Resume Game"),
    (MainItem::NewGame, "New Game"),
    (MainItem::Bots, "Bots"),
    (MainItem::Team, "Team"),
    (MainItem::Options, "Options"),
    (MainItem::BugReport, "Report a Bug"),
    (MainItem::Quit, "Quit"),
];

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

/// A value a row changes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Field {
    Map,
    Mode,
    BotsT,
    BotsCt,
    Difficulty,
    /// Index into `SETTINGS`.
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
}

/// A row of a page.
#[derive(Clone, Debug, PartialEq)]
pub enum Row {
    Button { label: String, action: Action, enabled: bool },
    Value { label: String, value: String, field: Field },
    /// Text that can't be focused.
    Info(String),
}

impl Row {
    fn focusable(&self) -> bool {
        match self {
            Row::Button { enabled, .. } => *enabled,
            Row::Value { .. } => true,
            Row::Info(_) => false,
        }
    }
}

/// What a click or hover is on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Target {
    /// A left-hand entry (index into `MAIN`).
    Main(usize),
    /// A row of the open page.
    Row(usize),
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
    /// Esc: close.
    Close,
    Hover(Target),
    /// A click: on the row itself (0) or its `<` (-1) / `>` (+1) arrow.
    Click(Target, i32),
    /// A typed letter or digit: jumps the map list.
    Char(char),
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

/// The menu's state.
#[derive(Resource, Clone, Debug, Default, PartialEq)]
pub struct GameMenu {
    pub open: bool,
    pub page: Page,
    /// The focused row of the page (of `MAIN` on the main page).
    pub focus: usize,
    /// Loadable maps (`maps`).
    pub maps: Vec<String>,
    pub new_game: NewGame,
    /// Each setting's current value (`SETTINGS` order), None if its cvar
    /// isn't registered.
    pub values: Vec<Option<String>>,
    /// Bots in the game now: terrorists, counter-terrorists.
    pub bots: [usize; 2],
    /// Everyone in the game now, by team (auto-assign).
    pub players: [usize; 2],
}

/// Map list rows per column.
pub const MAP_ROWS: usize = 14;
/// Map list columns.
pub const MAP_COLUMNS: usize = 3;

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
    }

    /// The bots and players per team now (for the bots and team pages; a new
    /// game starts with as many bots).
    pub fn set_counts(&mut self, bots: [usize; 2], players: [usize; 2]) {
        self.bots = bots;
        self.players = players;
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
        let ng = &self.new_game;
        match self.page {
            Page::Main => MAIN.iter().map(|(item, label)| button(label, Action::Main(*item))).collect(),
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
                button("Back", Action::Back),
            ],
            Page::Maps => {
                let mut rows: Vec<Row> = self
                    .maps
                    .iter()
                    .enumerate()
                    .map(|(i, m)| button(m, Action::PickMap(i)))
                    .collect();
                rows.push(button("Back", Action::Back));
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
                button("Back", Action::Back),
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
                button("Back", Action::Back),
            ],
            Page::Settings => {
                let mut rows: Vec<Row> = SETTINGS
                    .iter()
                    .enumerate()
                    .map(|(i, s)| match self.values.get(i).cloned().flatten() {
                        Some(v) => value(s.label, s.show(&v), Field::Setting(i)),
                        None => Row::Info(format!("{}: not available", s.label)),
                    })
                    .collect();
                rows.push(Row::Info("Saved to config.cfg when you quit.".into()));
                rows.push(button("Back", Action::Back));
                rows
            }
        }
    }

    /// Apply an input; the console lines it produces.
    pub fn handle(&mut self, input: Input) -> Outcome {
        let mut out = Outcome::default();
        if !self.open {
            return out;
        }
        let rows = self.rows();
        match input {
            Input::Close => self.close(&mut out),
            Input::Back => self.back(),
            Input::Up => self.focus = self.first_focusable_from(self.focus + rows.len() - 1, -1),
            Input::Down => self.focus = self.first_focusable_from(self.focus + 1, 1),
            Input::Left | Input::Right => {
                let dir = if input == Input::Left { -1 } else { 1 };
                if self.page == Page::Maps {
                    // Across the columns.
                    let n = rows.len() as i32;
                    let to = self.focus as i32 + dir * MAP_ROWS as i32;
                    if (0..n).contains(&to) {
                        self.focus = to as usize;
                    }
                } else if let Some(Row::Value { field, .. }) = rows.get(self.focus) {
                    self.change(*field, dir, &mut out);
                }
            }
            Input::Activate => self.activate(self.focus, 0, &mut out),
            Input::Hover(Target::Main(i)) => {
                if self.page == Page::Main && i < MAIN.len() {
                    self.focus = i;
                }
            }
            Input::Hover(Target::Row(i)) => {
                if self.page != Page::Main && rows.get(i).is_some_and(Row::focusable) {
                    self.focus = i;
                }
            }
            Input::Click(Target::Main(i), _) => {
                if let Some((item, _)) = MAIN.get(i) {
                    if self.page == Page::Main {
                        self.focus = i;
                    }
                    self.main(*item, &mut out);
                }
            }
            Input::Click(Target::Row(i), step) => {
                if self.page != Page::Main && rows.get(i).is_some_and(Row::focusable) {
                    self.focus = i;
                    self.activate(i, step, &mut out);
                }
            }
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
        out
    }

    fn close(&mut self, out: &mut Outcome) {
        self.open = false;
        out.close = true;
    }

    /// Back to the main page, its entry focused (to the new game page from
    /// the map list).
    fn back(&mut self) {
        match self.page {
            Page::Main => {}
            Page::Maps => {
                self.page = Page::NewGame;
                self.focus = 0;
            }
            page => {
                self.focus = MAIN.iter().position(|(m, _)| m.page() == Some(page)).unwrap_or(0);
                self.page = Page::Main;
            }
        }
    }

    fn main(&mut self, item: MainItem, out: &mut Outcome) {
        if let Some(page) = item.page() {
            self.page = page;
            self.focus = self.first_focusable_from(0, 1);
            return;
        }
        match item {
            MainItem::Resume => self.close(out),
            MainItem::BugReport => {
                // Closed first, so the screenshot shows the game.
                self.close(out);
                out.lines.push("bugreport".into());
            }
            MainItem::Quit => out.lines.push("quit".into()),
            _ => {}
        }
    }

    /// Press row `i` (`step` -1/+1: its arrows).
    fn activate(&mut self, i: usize, step: i32, out: &mut Outcome) {
        let Some(row) = self.rows().into_iter().nth(i) else {
            return;
        };
        match row {
            Row::Button { enabled: false, .. } | Row::Info(_) => {}
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
                Action::Start => {
                    if let Some((lines, after)) = self.start_lines() {
                        out.lines = lines;
                        out.after_load = after;
                        self.close(out);
                    }
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
                let next = setting.step(current, dir);
                if setting.show(&next) != setting.show(current) {
                    out.lines.push(format!("{} {next}", setting.cvar));
                    self.values[i] = Some(next);
                }
            }
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

/// Open the menu on a page, reading the settings from the console.
fn open_menu(w: &mut World, page: Page) {
    let get = {
        let cvars: Vec<_> = {
            let console = w.resource::<Console>();
            SETTINGS
                .iter()
                .map(|s| s.cvar)
                .chain(["mashup_rounds", "bot_reaction"])
                .filter_map(|n| console.cvar(n).cloned())
                .collect()
        };
        cvars
            .into_iter()
            .filter_map(|c| (c.get)(w).map(|v| (c.name.clone(), v)))
            .collect::<std::collections::HashMap<_, _>>()
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
    let mut menu = w.resource_mut::<GameMenu>();
    menu.set_counts(bots, players);
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

/// Esc opens and closes the menu; arrows, Enter, Space, Backspace and
/// letters drive it. Runs before the console's toggle, so the Esc that
/// closes the console doesn't open the menu.
fn keys(
    keys: Res<ButtonInput<KeyCode>>,
    ui: Res<super::console::ConsoleUi>,
    mut menu: ResMut<GameMenu>,
    mut console: ResMut<Console>,
    mut after: ResMut<AfterLoad>,
    mut regrab: ResMut<RegrabCursor>,
    mut commands: Commands,
) {
    if ui.open {
        return;
    }
    if !menu.open {
        if keys.just_pressed(KeyCode::Escape) {
            commands.queue(|w: &mut World| open_menu(w, Page::Main));
        }
        return;
    }
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

/// Hovering focuses, clicking presses.
fn pointer(
    hits: Query<(&Interaction, &Hit), Changed<Interaction>>,
    mut menu: ResMut<GameMenu>,
    mut console: ResMut<Console>,
    mut after: ResMut<AfterLoad>,
    mut regrab: ResMut<RegrabCursor>,
) {
    if !menu.open {
        return;
    }
    let inputs: Vec<Input> = hits
        .iter()
        .filter_map(|(i, h)| match i {
            Interaction::Hovered => Some(Input::Hover(h.0)),
            Interaction::Pressed => Some(Input::Click(h.0, h.1)),
            Interaction::None => None,
        })
        .collect();
    for input in inputs {
        apply(input, &mut menu, &mut console, &mut after, &mut regrab);
    }
}

/// Bots and players per team, for the bots and team pages.
fn counts(
    mut menu: ResMut<GameMenu>,
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

/// The menu's text faces: the system's UI font (Tahoma, as CS:S's menus
/// use, on Windows), regular and bold; Bevy's own when none is found.
#[derive(Resource, Default)]
struct MenuFonts {
    text: Option<Handle<Font>>,
    bold: Option<Handle<Font>>,
}

fn load_fonts(mut fonts: ResMut<Assets<Font>>, mut commands: Commands) {
    let mut find = |names: &[&str]| super::vgui::system_font(&mut fonts, names);
    let text = find(&[
        "tahoma.ttf",
        "DejaVuSans.ttf",
        "LiberationSans-Regular.ttf",
        "NotoSans-Regular.ttf",
    ]);
    let bold = find(&[
        "tahomabd.ttf",
        "DejaVuSans-Bold.ttf",
        "LiberationSans-Bold.ttf",
        "NotoSans-Bold.ttf",
    ])
    .or_else(|| text.clone());
    commands.insert_resource(MenuFonts { text, bold });
}

#[derive(Component)]
struct MenuRoot;

/// Colours: the game's scheme where it has them.
struct Look {
    accent: Color,
    text: Color,
    /// Focused text, on the accent's tint.
    bright: Color,
    dim: Color,
    panel: Color,
    border: Color,
    focus: Color,
}

fn look(hud: Option<&crate::map::hud::ActiveHud>) -> Look {
    let scheme = |name: &str| hud.and_then(|h| h.0.color(name));
    let accent = scheme("FgColor")
        .map(|c| c.with_alpha(1.0))
        .unwrap_or(Color::srgb_u8(255, 176, 0));
    Look {
        accent,
        text: Color::srgb_u8(225, 225, 225),
        bright: Color::WHITE,
        dim: Color::srgb_u8(130, 130, 130),
        panel: Color::srgba_u8(28, 28, 28, 225),
        border: Color::srgba_u8(255, 255, 255, 40),
        focus: accent.with_alpha(0.12),
    }
}

#[allow(clippy::too_many_arguments)]
fn draw(
    menu: Res<GameMenu>,
    fonts: Option<Res<MenuFonts>>,
    hud: Option<Res<crate::map::hud::ActiveHud>>,
    shown: Query<Entity, With<MenuRoot>>,
    windows: Query<&Window>,
    mut last_height: Local<f32>,
    mut commands: Commands,
) {
    let height = windows.iter().next().map_or(480.0, |w| w.height());
    let resized = (height - *last_height).abs() > 0.5;
    if !menu.is_changed() && !resized && !hud.as_ref().is_some_and(|h| h.is_changed()) {
        return;
    }
    *last_height = height;
    for e in &shown {
        commands.entity(e).despawn();
    }
    if !menu.open {
        return;
    }
    let s = height / 480.0;
    let look = look(hud.as_deref());
    let fonts = fonts.as_deref();
    let font = |bold: bool, size: f32| TextFont {
        font: fonts
            .and_then(|f| if bold { f.bold.clone() } else { f.text.clone() })
            .unwrap_or_default()
            .into(),
        font_size: FontSize::Px(size * s),
        ..default()
    };
    let title_font = font(true, 24.0);

    let rows = menu.rows();
    let main_page = menu.page == Page::Main;
    commands
        .spawn((
            MenuRoot,
            Node {
                position_type: PositionType::Absolute,
                width: percent(100.0),
                height: percent(100.0),
                ..default()
            },
            BackgroundColor(Color::srgba(0.0, 0.0, 0.0, 0.35)),
            GlobalZIndex(45),
        ))
        .with_children(|root| {
            // The left-hand entries.
            root.spawn(Node {
                position_type: PositionType::Absolute,
                left: px(40.0 * s),
                bottom: px(70.0 * s),
                flex_direction: FlexDirection::Column,
                row_gap: px(3.0 * s),
                ..default()
            })
            .with_children(|col| {
                col.spawn((
                    Text::new("MASHUP"),
                    title_font,
                    TextColor(look.text),
                    Node {
                        margin: UiRect::bottom(px(14.0 * s)),
                        ..default()
                    },
                ));
                for (i, (item, label)) in MAIN.iter().enumerate() {
                    let current = !main_page && item.page() == Some(menu.page);
                    let focused = main_page && menu.focus == i;
                    let color = if focused {
                        look.bright
                    } else if current {
                        look.accent
                    } else {
                        look.text
                    };
                    col.spawn((
                        Hit(Target::Main(i), 0),
                        Button,
                        Interaction::default(),
                        Node {
                            padding: UiRect::axes(px(6.0 * s), px(2.0 * s)),
                            // Set apart from the game's entries.
                            margin: if *item == MainItem::BugReport {
                                UiRect::top(px(10.0 * s))
                            } else {
                                UiRect::ZERO
                            },
                            ..default()
                        },
                        BackgroundColor(if focused { look.focus } else { Color::NONE }),
                    ))
                    .with_child((Text::new(label.to_uppercase()), font(true, 12.0), TextColor(color)));
                }
            });
            if main_page {
                return;
            }
            // The page's dialog.
            let maps = menu.page == Page::Maps;
            let width = if maps { 470.0 } else { 300.0 };
            root.spawn((
                Node {
                    position_type: PositionType::Absolute,
                    left: px(190.0 * s),
                    top: px(70.0 * s),
                    width: px(width * s),
                    flex_direction: FlexDirection::Column,
                    padding: UiRect::all(px(10.0 * s)),
                    row_gap: px(2.0 * s),
                    border: UiRect::all(px(1.0)),
                    border_radius: BorderRadius::all(px(4.0 * s)),
                    ..default()
                },
                BackgroundColor(look.panel),
                BorderColor::all(look.border),
            ))
            .with_children(|dialog| {
                let title = match menu.page {
                    Page::Main => "",
                    Page::NewGame => "New Game",
                    Page::Maps => "Choose a Map",
                    Page::Bots => "Bots",
                    Page::Team => "Choose a Team",
                    Page::Settings => "Options",
                };
                dialog.spawn((
                    Text::new(title),
                    font(true, 11.0),
                    TextColor(look.accent),
                    Node {
                        margin: UiRect::bottom(px(8.0 * s)),
                        ..default()
                    },
                ));
                if maps {
                    map_grid(dialog, &menu, &rows, &look, &font, s);
                } else {
                    for (i, row) in rows.iter().enumerate() {
                        spawn_row(dialog, i, row, menu.focus == i, &look, &font, s);
                    }
                }
                let hint = if maps {
                    "Arrows select   Enter picks   a letter jumps   Backspace back   Esc closes"
                } else {
                    "Up/Down select   Left/Right change   Enter picks   Backspace back   Esc closes"
                };
                dialog.spawn((
                    Text::new(hint),
                    font(false, 7.0),
                    TextColor(look.dim),
                    Node {
                        margin: UiRect::top(px(8.0 * s)),
                        ..default()
                    },
                ));
            });
        });
}

/// One row of a dialog.
fn spawn_row(
    parent: &mut ChildSpawnerCommands,
    i: usize,
    row: &Row,
    focused: bool,
    look: &Look,
    font: &impl Fn(bool, f32) -> TextFont,
    s: f32,
) {
    let size = 9.5;
    let line = Node {
        width: percent(100.0),
        padding: UiRect::axes(px(6.0 * s), px(3.0 * s)),
        align_items: AlignItems::Center,
        border_radius: BorderRadius::all(px(2.0 * s)),
        ..default()
    };
    let bg = BackgroundColor(if focused { look.focus } else { Color::NONE });
    match row {
        Row::Info(text) => {
            parent.spawn((line, children![(Text::new(text.clone()), font(false, 8.5), TextColor(look.dim))]));
        }
        Row::Button { label, enabled, .. } => {
            let color = if !enabled {
                look.dim
            } else if focused {
                look.bright
            } else {
                look.text
            };
            parent
                .spawn((Hit(Target::Row(i), 0), Button, Interaction::default(), line, bg))
                .with_child((Text::new(label.clone()), font(false, size), TextColor(color)));
        }
        Row::Value { label, value, .. } => {
            let color = if focused { look.bright } else { look.text };
            parent
                .spawn((Hit(Target::Row(i), 0), Button, Interaction::default(), line, bg))
                .with_children(|r| {
                    r.spawn((
                        Text::new(label.clone()),
                        font(false, size),
                        TextColor(look.text),
                        Node {
                            flex_grow: 1.0,
                            ..default()
                        },
                    ));
                    let arrow = |r: &mut ChildSpawnerCommands, text: &str, step: i32| {
                        r.spawn((
                            Hit(Target::Row(i), step),
                            Button,
                            Interaction::default(),
                            Node {
                                padding: UiRect::axes(px(5.0 * s), px(0.0)),
                                ..default()
                            },
                        ))
                        .with_child((Text::new(text), font(true, size), TextColor(look.accent)));
                    };
                    arrow(r, "<", -1);
                    r.spawn((
                        Text::new(value.clone()),
                        font(true, size),
                        TextColor(color),
                        TextLayout::justify(Justify::Center),
                        Node {
                            min_width: px(95.0 * s),
                            justify_content: JustifyContent::Center,
                            ..default()
                        },
                    ));
                    arrow(r, ">", 1);
                });
        }
    }
}

/// The map list: columns of `MAP_ROWS`, a page of them around the focus.
fn map_grid(
    parent: &mut ChildSpawnerCommands,
    menu: &GameMenu,
    rows: &[Row],
    look: &Look,
    font: &impl Fn(bool, f32) -> TextFont,
    s: f32,
) {
    let per_page = MAP_ROWS * MAP_COLUMNS;
    let maps = rows.len() - 1;
    let page = menu.focus.min(maps.saturating_sub(1)) / per_page;
    let start = page * per_page;
    if maps == 0 {
        parent.spawn((Text::new("No maps found in the install."), font(false, 9.0), TextColor(look.dim)));
    }
    parent
        .spawn(Node {
            flex_direction: FlexDirection::Row,
            column_gap: px(4.0 * s),
            ..default()
        })
        .with_children(|grid| {
            for c in 0..MAP_COLUMNS {
                let from = start + c * MAP_ROWS;
                if from >= maps {
                    break;
                }
                grid.spawn(Node {
                    flex_direction: FlexDirection::Column,
                    width: px(148.0 * s),
                    ..default()
                })
                .with_children(|col| {
                    for i in from..(from + MAP_ROWS).min(maps) {
                        spawn_row(col, i, &rows[i], menu.focus == i, look, font, s);
                    }
                });
            }
        });
    if maps > per_page {
        parent.spawn((
            Text::new(format!(
                "Page {} of {} ({} maps)",
                page + 1,
                maps.div_ceil(per_page),
                maps
            )),
            font(false, 8.0),
            TextColor(look.dim),
        ));
    }
    // Back.
    spawn_row(parent, maps, &rows[maps], menu.focus == maps, look, font, s);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn menu() -> GameMenu {
        let mut m = GameMenu::default();
        let cvars = [
            ("sensitivity", "3"),
            ("zoom_sensitivity_ratio", "1.2"),
            ("volume", "1"),
            ("viewmodel_fov", "54"),
            ("cl_righthand", "1"),
            ("cl_showfps", "0"),
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

    #[test]
    fn opens_with_the_game_as_it_is() {
        let m = menu();
        assert!(m.open);
        assert_eq!((m.page, m.focus), (Page::Main, 0));
        assert_eq!(m.new_game.map, 1, "the loaded map");
        assert_eq!(m.new_game.mode, Mode::Rounds);
        assert_eq!(m.new_game.difficulty, NORMAL);
        // No crosshair colour cvar given: shown as not available.
        assert_eq!(m.values[5], None);
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
        press(&mut m, &[Input::Down, Input::Down, Input::Activate]);
        assert_eq!(m.page, Page::Bots);
        // The info line can't be focused.
        assert_eq!(m.focus, 1);
        press(&mut m, &[Input::Up]);
        assert_eq!(m.focus, 4, "wraps past the info line to Back");
        press(&mut m, &[Input::Back]);
        assert_eq!((m.page, m.focus), (Page::Main, 2), "back on its entry");
        // Up from the top wraps to Quit.
        press(&mut m, &[Input::Up, Input::Up, Input::Up]);
        assert_eq!(MAIN[m.focus].0, MainItem::Quit);
        assert_eq!(press(&mut m, &[Input::Activate]).lines, ["quit"]);
    }

    #[test]
    fn bots_and_team_pages_run_their_commands() {
        let mut m = menu();
        press(&mut m, &[Input::Click(Target::Main(2), 0)]);
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

        press(&mut m, &[Input::Click(Target::Main(3), 0)]);
        assert_eq!(m.page, Page::Team);
        let o = press(&mut m, &[Input::Down, Input::Activate]);
        assert_eq!(o.lines, ["jointeam 3"]);
        assert!(o.close && !m.open, "joining a team goes back to playing");

        // Auto-assign joins the smaller team.
        let mut m = menu();
        m.set_counts([0, 0], [3, 1]);
        press(&mut m, &[Input::Click(Target::Main(3), 0)]);
        assert_eq!(press(&mut m, &[Input::Click(Target::Row(2), 0)]).lines, ["jointeam 3"]);
    }

    #[test]
    fn a_new_game_loads_the_map_then_adds_bots() {
        let mut m = menu();
        press(&mut m, &[Input::Click(Target::Main(1), 0)]);
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
        press(&mut m, &[Input::Click(Target::Main(1), 0)]);
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
        press(&mut m, &[Input::Click(Target::Main(1), 0), Input::Activate]);
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
        press(&mut m, &[Input::Click(Target::Main(4), 0)]);
        assert_eq!((m.page, m.focus), (Page::Settings, 0));
        let o = press(&mut m, &[Input::Right, Input::Right, Input::Down, Input::Left]);
        assert_eq!(o.lines, ["sensitivity 3.1", "sensitivity 3.2", "zoom_sensitivity_ratio 1.1"]);
        // Volume stops at 1.
        let o = press(&mut m, &[Input::Down, Input::Right]);
        assert!(o.lines.is_empty(), "{:?}", o.lines);
        // Hand: a choice, wrapping; shown by name.
        let o = press(&mut m, &[Input::Click(Target::Row(4), 1)]);
        assert_eq!(o.lines, ["cl_righthand 0"]);
        assert!(matches!(&m.rows()[4], Row::Value { value, .. } if value == "Left"));
        // Crosshair colour isn't registered here: not focusable, skipped.
        press(&mut m, &[Input::Down]);
        assert_eq!(m.focus, 6);
        let o = press(&mut m, &[Input::Activate, Input::Activate]);
        assert_eq!(o.lines, ["cl_showfps 1", "cl_showfps 2"]);
    }

    #[test]
    fn steps_snap_to_the_grid() {
        let s = SETTINGS[0];
        assert_eq!(s.step("1.23", 1), "1.3");
        assert_eq!(s.step("0.1", -1), "0.1");
        assert_eq!(s.step("bad", 1), "0.2");
        assert_eq!(SETTINGS[2].step("0.5", 1), "0.55");
        assert_eq!(SETTINGS[2].show("0.5"), "0.50");
    }

    #[test]
    fn hover_moves_focus_only_on_its_page() {
        let mut m = menu();
        press(&mut m, &[Input::Hover(Target::Main(4))]);
        assert_eq!(m.focus, 4);
        press(&mut m, &[Input::Activate]);
        assert_eq!(m.page, Page::Settings);
        // The left-hand list doesn't take the focus from the dialog.
        press(&mut m, &[Input::Hover(Target::Main(1)), Input::Hover(Target::Row(3))]);
        assert_eq!(m.focus, 3);
    }

    #[test]
    fn escape_opens_and_closes_and_the_mouse_follows() {
        use bevy::window::CursorGrabMode;
        let mut app = App::new();
        app.add_plugins(crate::console::ConsolePlugin)
            .init_resource::<super::super::console::ConsoleUi>()
            .init_resource::<ButtonInput<KeyCode>>()
            .init_resource::<ButtonInput<MouseButton>>()
            .init_resource::<GameMenu>()
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

    #[test]
    fn bug_report_closes_first() {
        let mut m = menu();
        let o = press(&mut m, &[Input::Click(Target::Main(5), 0)]);
        assert_eq!(o.lines, ["bugreport"]);
        assert!(o.close && !m.open);
    }
}
