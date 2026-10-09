//! The menu's model: its controls, inputs and state (`GameMenu`), turned
//! into console lines by `GameMenu::handle`; no drawing, no Bevy systems.

use super::*;

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
    pub(super) fn focusable(&self) -> bool {
        match self {
            Row::Button { enabled, .. } => *enabled,
            Row::Control { .. } | Row::Bind { .. } => true,
            Row::Info(_) | Row::Heading(_) => false,
        }
    }

    pub(super) fn field(&self) -> Option<Field> {
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
pub(super) const KEY_BUTTONS: usize = 4;
/// Rows of Create Server's Game page list shown at once.
pub const GAME_ROWS: usize = 12;

/// The layout name of the open page (`GameUi::options`).
pub(super) fn layout_name(page: Page, tab: Tab, create_tab: usize) -> Option<&'static str> {
    match page {
        Page::Settings => Some(tab.page()),
        Page::KeyboardAdvanced => Some("keyboard_advanced"),
        Page::VideoAdvanced => Some("video_advanced"),
        Page::NewGame => Some(["create_server", "create_game", "create_bot"][create_tab.min(2)]),
        _ => None,
    }
}

/// The next entry after `from` starting with `c` (wrapping), as VGUI's
/// combo boxes jump on a typed letter.
pub(super) fn jump(entries: &[String], from: usize, c: char) -> Option<usize> {
    let n = entries.len();
    let c = c.to_ascii_lowercase();
    (1..=n)
        .map(|k| (from + k) % n)
        .find(|&i| entries[i].to_ascii_lowercase().starts_with(c))
}

/// A layout control's VGUI class (`ComboBox`, `CCvarSlider` ...).
pub(in crate::client) fn class_of(c: &UiControl) -> &str {
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
pub(super) fn menu_input(menu: &GameMenu, a: &[String]) -> Result<Input, String> {
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
    pub(super) fn go_to(&mut self, page: Page) {
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

    pub(super) fn game_text(&self, token: &str) -> Option<String> {
        self.ui.0.as_ref().and_then(|u| u.string(token)).map(str::to_string)
    }

    /// The open page's layout from the install.
    pub fn layout(&self) -> Option<&UiLayout> {
        let name = layout_name(self.page, self.tab, self.create_tab)?;
        self.ui.0.as_ref()?.options.get(name)
    }

    /// A control row (its label from the game, else ours).
    pub(super) fn control_row(&self, field: Field) -> Row {
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

    /// The rows of the open page (the left-hand entries on the main page),
    /// in tab order.
    pub fn rows(&self) -> Vec<Row> {
        match self.page {
            Page::Main => self.main_rows(),
            Page::NewGame => self.create_server_rows(),
            Page::Bots => self.bots_rows(),
            Page::Team => self.team_rows(),
            Page::Settings if self.tab == Tab::Keyboard => self.keyboard_rows(),
            Page::Settings => self.options_rows(),
            Page::KeyboardAdvanced | Page::VideoAdvanced => self.advanced_rows(),
            Page::Extras => self.extras_rows(),
        }
    }

    /// A button row that can be pressed.
    pub(super) fn button_row(label: &str, action: Action) -> Row {
        Row::Button {
            label: label.to_string(),
            action,
            enabled: true,
        }
    }

    /// A dialog's OK and Cancel rows.
    pub(super) fn ok_row(&self) -> Row {
        Self::button_row(&self.text("#GameUI_OK", "OK"), Action::Ok)
    }

    pub(super) fn cancel_row(&self) -> Row {
        Self::button_row(&self.text("#GameUI_Cancel", "Cancel"), Action::Cancel)
    }

    /// Where a row's control is in the open page's layout (its index).
    pub(super) fn row_order(&self, row: &Row) -> usize {
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
    pub(super) fn default_action(&self) -> Option<Action> {
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
    pub(super) fn handle_combo(&mut self, input: Input, out: &mut Outcome) {
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
    pub(super) fn focus_changed(&mut self) {
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
    pub(super) fn edit(&mut self, edit: Edit, out: &mut Outcome) {
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
    pub(super) fn set_text(&mut self, field: Field, text: String, out: &mut Outcome) {
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
    pub(super) fn select_focused_key(&mut self) {
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
    pub(super) fn keep_visible(&mut self) {
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

    pub(super) fn close(&mut self, out: &mut Outcome) {
        self.open = false;
        self.capture = None;
        self.combo = None;
        out.close = true;
    }

    /// Back to the main page, its entry focused (from an Advanced dialog:
    /// the options, its button focused).
    pub(super) fn back(&mut self) {
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

    /// Press row `i` (Space, a click): a button presses, a check box
    /// ticks, a radio button picks, a combo box opens its list, a list row
    /// waits for a key.
    pub(super) fn activate(&mut self, i: usize, out: &mut Outcome) {
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

    pub(super) fn press_action_into(&mut self, action: &Action, out: &mut Outcome) {
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
    pub(super) fn change(&mut self, field: Field, dir: i32, wrap: bool, out: &mut Outcome) {
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
    pub(super) fn pick(&mut self, field: Field, k: usize, out: &mut Outcome) {
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

    /// The first focusable row from `i` on, stepping by `dir` (wrapping).
    pub(super) fn first_focusable_from(&self, i: usize, dir: i32) -> usize {
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
