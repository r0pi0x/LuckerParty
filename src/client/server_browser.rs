//! The server browser (Find Servers, `openserverbrowser`), drawn as
//! CS:S's: a frame over the menu with the Internet, Favorites, History
//! and Lan tabs, the server list (password and bot icons, name, game,
//! players, map, latency; History adds when last played), sorting by a
//! column header, filters (map, latency, max players, has users playing,
//! not full, no password), refresh, Add a Server, Connect (double-click
//! a row too) and the password dialog; laid out from the install's
//! `servers/*.res` (`GameUi::servers`), in its words and icons; built-in
//! sizes and words without the install.
//!
//! Servers are found with `net::query`: the Lan tab scans
//! `net_lan_ports`; Favorites and History ask each saved address.
//! Internet stays greyed until there is a master server to list servers
//! (docs/plans/active/multiplayer.md, slice 8). Favorites and History
//! live in the cfg folder (`serverbrowser.vdf`); a server joined goes
//! into History. Joining is the console's `connect` (with `password`
//! first when the server wants one): the browser closes and the game's
//! connect flow takes over.
//!
//! As the game menu, the model (`ServerBrowser::handle`) turns clicks and
//! keys into console lines and queries, unit-tested without a window.

use std::{
    net::SocketAddr,
    path::{Path, PathBuf},
    sync::Arc,
};

use bevy::{
    input::{
        ButtonState,
        keyboard::{Key, KeyboardInput},
        mouse::AccumulatedMouseScroll,
    },
    prelude::*,
    ui::RelativeCursorPosition,
};

use super::{
    fonts::UiFonts,
    game_menu::GameMenu,
    widgets::{self, Caret, ComboEvent, ComboKey, ComboList, Edit, FrameSpec, Look, Windows, bevel, label, place},
};
use crate::{
    console::{Console, ConsoleAppExt},
    map::hud::{GameUi, UiControl, UiLayout},
    net::{
        NET_VERSION,
        query::{LanSettings, QueryResult, QueryState, ServerQueries},
    },
};

pub struct ServerBrowserPlugin;

impl Plugin for ServerBrowserPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<ServerBrowser>()
            .init_resource::<BrowserIcons>()
            .init_resource::<RequestInbox>()
            .add_systems(Startup, load_saved)
            .add_systems(
                Update,
                (
                    record_history,
                    sync_results,
                    follow_size,
                    keys,
                    pointer,
                    run_requests,
                    draw,
                )
                    .chain()
                    .after(super::game_menu::MenuSystems),
            );
        app.console_command(
            "openserverbrowser",
            "openserverbrowser [lan|favorites|history]: open Find Servers (the server browser).",
            |w, a| {
                let tab = match a.first().map(|s| s.to_lowercase()).as_deref() {
                    None => None,
                    Some("lan") => Some(Tab::Lan),
                    Some("favorites" | "favourites") => Some(Tab::Favorites),
                    Some("history") => Some(Tab::History),
                    Some("internet") => return Err("the Internet tab needs a master server (not yet)".into()),
                    Some(t) => return Err(format!("no tab \"{t}\"")),
                };
                super::game_menu::open_main(w);
                let out = w.resource_mut::<ServerBrowser>().open(tab);
                w.resource_mut::<RequestInbox>().0.extend(out.requests);
                Ok(None)
            },
        )
        .console_command(
            "serverbrowser",
            "serverbrowser <input>: drive the open server browser as a click or key would (testing): \
             row <n> | doubleclick <n> | connect | refresh | quickrefresh | addserver | filters | \
             sort <name|players|map|latency|...> | tab <lan|favorites|history> | type <text> | enter | escape | \
             delete | down | up | latency (opens its list) | hover <n> | pick <n> | check <notfull|notempty|\
             nopassword> | map | maxplayers (their text entries) | next (Tab) | space; prints its rows.",
            |w, a| {
                let input = browser_input(a)?;
                let out = w.resource_mut::<ServerBrowser>().handle(input);
                let mut console = w.resource_mut::<Console>();
                for line in out.lines {
                    console.submit(line);
                }
                w.resource_mut::<RequestInbox>().0.extend(out.requests);
                let b = w.resource::<ServerBrowser>();
                if out.save {
                    save(b);
                }
                Ok(Some(describe_rows(b)))
            },
        );
    }
}

// ---------------------------------------------------------------------------
// The model.

/// The browser's tabs, in CS:S's order (the ones mashup has).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Tab {
    Internet,
    Favorites,
    History,
    #[default]
    Lan,
}

/// Tab, its `#token`, our word for it.
pub const TABS: [(Tab, &str, &str); 4] = [
    (Tab::Internet, "#ServerBrowser_InternetTab", "Internet"),
    (Tab::Favorites, "#ServerBrowser_FavoritesTab", "Favorites"),
    (Tab::History, "#ServerBrowser_HistoryTab", "History"),
    (Tab::Lan, "#ServerBrowser_LanTab", "Lan"),
];

impl Tab {
    /// Whether it can be opened: Internet needs a master server.
    pub fn enabled(self) -> bool {
        self != Tab::Internet
    }
}

/// The list's columns.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Column {
    Password,
    Bots,
    Name,
    Game,
    Players,
    Map,
    Latency,
    LastPlayed,
}

/// Latency filter choices, ms (0: all), and their words.
pub const LATENCIES: [(u32, &str, &str); 7] = [
    (0, "#ServerBrowser_All", "<All>"),
    (50, "#ServerBrowser_LessThan50", "< 50"),
    (100, "#ServerBrowser_LessThan100", "< 100"),
    (150, "#ServerBrowser_LessThan150", "< 150"),
    (250, "#ServerBrowser_LessThan250", "< 250"),
    (350, "#ServerBrowser_LessThan350", "< 350"),
    (600, "#ServerBrowser_LessThan600", "< 600"),
];

/// A saved server (favourite or history).
#[derive(Clone, Debug, PartialEq)]
pub struct SavedServer {
    pub addr: SocketAddr,
    /// Its name when last heard.
    pub name: String,
    /// When last joined, Unix seconds (History; 0: never).
    pub last_played: u64,
}

/// The filters (`Filters` toggles them shown).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Filters {
    pub shown: bool,
    /// Map names containing this (any case).
    pub map: String,
    /// Servers taking at most this many players ("" or 0: any).
    pub max_players: String,
    /// Index into `LATENCIES`.
    pub latency: usize,
    pub not_empty: bool,
    pub not_full: bool,
    pub no_password: bool,
}

impl Filters {
    fn max_players(&self) -> Option<u32> {
        self.max_players.trim().parse::<u32>().ok().filter(|n| *n > 0)
    }

    /// Whether a row passes. A server that hasn't answered passes (it is
    /// listed as not responding in Favorites and History).
    pub fn pass(&self, row: &ServerRow) -> bool {
        if !row.responded {
            return true;
        }
        let map = self.map.trim().to_lowercase();
        let limit = LATENCIES[self.latency.min(LATENCIES.len() - 1)].0;
        (map.is_empty() || row.map.to_lowercase().contains(&map))
            && self.max_players().is_none_or(|n| row.max_players <= n)
            && (limit == 0 || row.ping.is_some_and(|p| p < limit))
            && (!self.not_empty || row.players > row.bots)
            && (!self.not_full || row.players < row.max_players)
            && (!self.no_password || !row.password)
    }

    pub fn active(&self) -> bool {
        !self.map.trim().is_empty()
            || self.max_players().is_some()
            || self.latency > 0
            || self.not_empty
            || self.not_full
            || self.no_password
    }
}

/// A row of the list.
#[derive(Clone, Debug, PartialEq)]
pub struct ServerRow {
    pub addr: SocketAddr,
    pub name: String,
    pub game: String,
    pub map: String,
    pub players: u32,
    pub max_players: u32,
    pub bots: u32,
    pub password: bool,
    pub ping: Option<u32>,
    /// It answered (else: asking, or not responding).
    pub responded: bool,
    /// Asked and given up on.
    pub not_responding: bool,
    /// Another build: joining would be refused.
    pub other_version: bool,
    pub last_played: u64,
}

impl Default for ServerRow {
    fn default() -> Self {
        Self {
            addr: SocketAddr::from(([0, 0, 0, 0], 0)),
            name: String::new(),
            game: String::new(),
            map: String::new(),
            players: 0,
            max_players: 0,
            bots: 0,
            password: false,
            ping: None,
            responded: false,
            not_responding: false,
            other_version: false,
            last_played: 0,
        }
    }
}

/// A text box that takes typing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Field {
    MapFilter,
    MaxPlayers,
    AddAddress,
    Password,
}

/// A dialog over the browser.
#[derive(Clone, Debug, Default, PartialEq)]
pub enum Dialog {
    #[default]
    None,
    /// Add a Server: the address typed, the address being looked at
    /// (`Find games at this address...`) and whether its row is selected.
    AddServer {
        text: String,
        looked_up: Option<SocketAddr>,
        selected: bool,
        error: Option<String>,
    },
    /// The server wants a password.
    Password {
        addr: SocketAddr,
        name: String,
        text: String,
    },
}

/// What a click is on.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Target {
    Tab(Tab),
    Header(Column),
    /// A row of the list (index into `rows`).
    Row(usize),
    Scroll(i32),
    Button(ButtonId),
    Field(Field),
    /// The latency filter's combo box (a click opens its list).
    Latency,
    /// An entry of the open list.
    ComboItem(usize),
    /// Anywhere outside the open list (closes it).
    Outside,
    Check(Check),
}

/// A control the keyboard is on (other than a text entry, `typing`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Focus {
    List,
    Latency,
    Check(Check),
    Button(ButtonId),
}

/// The browser's frame (`widgets::Windows`).
pub const WINDOW: widgets::WindowId = "servers";
/// Its dialogs' frames.
const ADD_WINDOW: widgets::WindowId = "addserver";
const PASSWORD_WINDOW: widgets::WindowId = "password";

/// The browser's smallest size, scheme pixels (`CServerBrowserDialog`'s
/// minimum: its layout's own size).
pub const MIN_SIZE: Vec2 = Vec2::new(640.0, 384.0);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Check {
    NotEmpty,
    NotFull,
    NoPassword,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ButtonId {
    Connect,
    Refresh,
    QuickRefresh,
    AddServer,
    Filters,
    Close,
    /// Add server dialog: add the typed address to favourites.
    AddOk,
    /// Add server dialog: ask the typed address.
    AddFind,
    /// Add server dialog: add the found server to favourites.
    AddSelected,
    AddCancel,
    PasswordConnect,
    PasswordCancel,
}

/// A key or pointer event.
#[derive(Clone, Debug, PartialEq)]
pub enum Input {
    Click(Target),
    /// A second click on the same row soon after: join it.
    DoubleClick(usize),
    Up,
    Down,
    Enter,
    Escape,
    Delete,
    Backspace,
    Type(String),
    Scroll(i32),
    /// The pointer over a control (an open list's entries highlight).
    Hover(Target),
    /// Tab (+1) / Shift+Tab (-1): the next control.
    Tab(i32),
    /// Space: the focused control (ticks, opens the list, presses).
    Space,
    /// An editing key in the text entry typed into.
    Edit(Edit),
    /// Ctrl+C / Ctrl+X there.
    Copy,
    Cut,
    /// A text entry clicked at a char.
    Caret(Field, usize),
    /// A column's header edge dragged: its width (scheme pixels).
    ColumnWidth(Column, f32),
}

/// Queries the browser asks for.
#[derive(Clone, Debug, PartialEq)]
pub enum Request {
    /// Scan the LAN (`net_lan_ports`).
    ScanLan,
    /// Ask these servers.
    Query(Vec<SocketAddr>),
}

/// What an input asks for besides the browser's own state.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Outcome {
    pub lines: Vec<String>,
    pub requests: Vec<Request>,
    /// Favourites or history changed: write them out.
    pub save: bool,
    /// Text for the clipboard (copied or cut).
    pub clipboard: Option<String>,
}

/// Rows shown at once (without and with the filters), as the list's
/// height allows at `ROW_H`.
pub const ROW_H: f32 = 18.0;
pub const HEADER_H: f32 = 20.0;
/// History and favourites kept at most.
pub const MAX_SAVED: usize = 100;

/// The browser's state.
#[derive(Resource, Clone, Debug, Default, PartialEq)]
pub struct ServerBrowser {
    pub open: bool,
    pub tab: Tab,
    pub favorites: Vec<SavedServer>,
    pub history: Vec<SavedServer>,
    pub filters: Filters,
    /// Sort column and descending.
    pub sort: Option<(Column, bool)>,
    /// The selected server.
    pub selected: Option<SocketAddr>,
    /// The first row shown.
    pub scroll: usize,
    pub dialog: Dialog,
    /// The text box typing goes into.
    pub typing: Option<Field>,
    /// Its caret and selection.
    pub caret: Caret,
    /// The control the keyboard is on otherwise (Tab moves it).
    pub focus: Option<Focus>,
    /// The latency filter's open list.
    pub combo: Option<ComboList>,
    /// Columns given a width by dragging their header's edge (scheme
    /// pixels).
    pub widths: Vec<(Column, f32)>,
    /// How much bigger than its layout the frame was made (scheme pixels):
    /// the list grows, the buttons and filters follow its corners.
    pub grow: Vec2,
    /// The open tab's rows, filtered and sorted (`set_results`).
    pub rows: Vec<ServerRow>,
    /// Rows before filtering (for "Servers (n)").
    pub total: usize,
    /// Queries under way.
    pub refreshing: bool,
    /// Results for the add server dialog's address.
    pub found: Option<ServerRow>,
    /// The game's words, when the install has them.
    pub ui: super::game_menu::UiText,
}

impl ServerBrowser {
    /// The game's text for a `#token`, else `ours`.
    pub fn text(&self, token: &str, ours: &str) -> String {
        self.ui
            .0
            .as_ref()
            .and_then(|u| u.string(token))
            .map_or_else(|| ours.to_string(), str::to_string)
    }

    /// Open (on a tab, else the last one); the tab refreshes.
    pub fn open(&mut self, tab: Option<Tab>) -> Outcome {
        self.open = true;
        self.dialog = Dialog::None;
        self.typing = None;
        let mut out = Outcome::default();
        self.set_tab(tab.unwrap_or(self.tab), &mut out);
        out
    }

    fn set_tab(&mut self, tab: Tab, out: &mut Outcome) {
        if !tab.enabled() {
            return;
        }
        self.tab = tab;
        self.scroll = 0;
        self.selected = None;
        self.refresh(out);
    }

    fn saved(&self) -> &[SavedServer] {
        match self.tab {
            Tab::Favorites => &self.favorites,
            Tab::History => &self.history,
            _ => &[],
        }
    }

    fn refresh(&mut self, out: &mut Outcome) {
        match self.tab {
            Tab::Lan => out.requests.push(Request::ScanLan),
            Tab::Favorites | Tab::History => {
                let addrs: Vec<SocketAddr> = self.saved().iter().map(|s| s.addr).collect();
                if !addrs.is_empty() {
                    out.requests.push(Request::Query(addrs));
                }
            }
            Tab::Internet => {}
        }
        self.refreshing = !out.requests.is_empty();
    }

    /// Rows for the open tab from what the queries have heard.
    pub fn set_results(&mut self, results: &[QueryResult], busy: bool) {
        let row_of = |r: &QueryResult, saved: Option<&SavedServer>| -> ServerRow {
            let mut row = ServerRow {
                addr: r.addr,
                name: saved.map(|s| s.name.clone()).unwrap_or_default(),
                last_played: saved.map_or(0, |s| s.last_played),
                ..default()
            };
            match &r.state {
                QueryState::Answered { info, ping_ms } => {
                    row.name = info.name.clone();
                    row.game = info.game.clone();
                    row.map = info.map.clone();
                    row.players = info.players as u32;
                    row.max_players = info.max_players as u32;
                    row.bots = info.bots as u32;
                    row.password = info.password;
                    row.ping = Some(*ping_ms);
                    row.responded = true;
                    row.other_version = info.version != NET_VERSION;
                }
                QueryState::NoResponse => row.not_responding = true,
                QueryState::Pending => {}
            }
            row
        };
        let mut rows: Vec<ServerRow> = match self.tab {
            Tab::Lan => results
                .iter()
                .filter(|r| r.lan && matches!(r.state, QueryState::Answered { .. }))
                .map(|r| row_of(r, None))
                .collect(),
            Tab::Favorites | Tab::History => self
                .saved()
                .iter()
                .map(|s| {
                    let r = results
                        .iter()
                        .find(|r| r.addr == s.addr)
                        .cloned()
                        .unwrap_or(QueryResult {
                            addr: s.addr,
                            state: QueryState::Pending,
                            lan: false,
                        });
                    row_of(&r, Some(s))
                })
                .collect(),
            Tab::Internet => Vec::new(),
        };
        // Names learned go into the saved lists.
        for row in rows.iter().filter(|r| r.responded) {
            for s in self.favorites.iter_mut().chain(self.history.iter_mut()) {
                if s.addr == row.addr && s.name != row.name {
                    s.name = row.name.clone();
                }
            }
        }
        if let Dialog::AddServer {
            looked_up: Some(addr), ..
        } = &self.dialog
        {
            self.found = results.iter().find(|r| r.addr == *addr).map(|r| row_of(r, None));
        }
        self.total = rows.len();
        rows.retain(|r| self.filters.pass(r));
        self.sort_rows(&mut rows);
        self.rows = rows;
        self.refreshing = busy;
        self.scroll = self.scroll.min(self.rows.len().saturating_sub(self.shown_rows()));
    }

    fn sort_rows(&self, rows: &mut [ServerRow]) {
        // Unsorted: by latency, as Source lists (unanswered last).
        let (column, desc) = self.sort.unwrap_or((Column::Latency, false));
        rows.sort_by(|a, b| {
            use std::cmp::Ordering;
            let by = match column {
                Column::Password => a.password.cmp(&b.password),
                Column::Bots => (a.bots > 0).cmp(&(b.bots > 0)),
                Column::Name => a.name.to_lowercase().cmp(&b.name.to_lowercase()),
                Column::Game => a.game.cmp(&b.game),
                Column::Players => a.players.cmp(&b.players),
                Column::Map => a.map.to_lowercase().cmp(&b.map.to_lowercase()),
                Column::Latency => match (a.ping, b.ping) {
                    (Some(x), Some(y)) => x.cmp(&y),
                    (Some(_), None) => Ordering::Less,
                    (None, Some(_)) => Ordering::Greater,
                    (None, None) => Ordering::Equal,
                },
                Column::LastPlayed => a.last_played.cmp(&b.last_played),
            };
            let by = if desc { by.reverse() } else { by };
            by.then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
                .then_with(|| a.addr.cmp(&b.addr))
        });
    }

    /// The list's columns on the open tab: column, header, width (0: the
    /// rest).
    pub fn columns(&self) -> Vec<(Column, String, f32)> {
        let mut c = vec![
            (Column::Password, String::new(), 16.0),
            (Column::Bots, String::new(), 16.0),
            (
                Column::Name,
                self.text("#ServerBrowser_ServersCount", "Servers (%s1)")
                    .replace("%s1", &self.total.to_string()),
                0.0,
            ),
            (Column::Game, self.text("#ServerBrowser_Game", "Game"), 112.0),
            (Column::Players, self.text("#ServerBrowser_Players", "Players"), 55.0),
            (Column::Map, self.text("#ServerBrowser_Map", "Map"), 90.0),
            (Column::Latency, self.text("#ServerBrowser_Latency", "Latency"), 55.0),
        ];
        if self.tab == Tab::History {
            c.push((
                Column::LastPlayed,
                self.text("#ServerBrowser_LastPlayed", "Last played"),
                110.0,
            ));
        }
        c
    }

    /// Rows the list shows at once.
    pub fn shown_rows(&self) -> usize {
        let list_h = if self.filters.shown { 134.0 } else { 226.0 } + self.grow.y;
        ((list_h - HEADER_H - 2.0) / ROW_H).floor().max(1.0) as usize
    }

    /// A column's width: dragged, else its own (0: the rest).
    pub fn column_width(&self, column: Column, own: f32) -> f32 {
        self.widths.iter().find(|(c, _)| *c == column).map_or(own, |(_, w)| *w)
    }

    /// The controls in tab order (`tabPosition`): the page's buttons,
    /// the filters when shown, the list.
    fn tab_order(&self) -> Vec<Option<Focus>> {
        let mut order = vec![
            Some(Focus::Button(ButtonId::Connect)),
            Some(Focus::Button(ButtonId::Refresh)),
            Some(Focus::Button(if self.tab == Tab::Favorites {
                ButtonId::AddServer
            } else {
                ButtonId::QuickRefresh
            })),
            Some(Focus::Button(ButtonId::Filters)),
        ];
        if self.filters.shown {
            order.extend([
                None, // the map text entry
                None, // the max players one
                Some(Focus::Latency),
                Some(Focus::Check(Check::NotFull)),
                Some(Focus::Check(Check::NotEmpty)),
                Some(Focus::Check(Check::NoPassword)),
            ]);
        }
        order.push(Some(Focus::List));
        order
    }

    /// Tab from the control on now.
    fn tab_focus(&mut self, dir: i32) {
        // Text entries stand in the order as None slots, in turn.
        let order = self.tab_order();
        let fields = [Field::MapFilter, Field::MaxPlayers];
        let slots: Vec<(Option<Focus>, Option<Field>)> = {
            let mut k = 0;
            order
                .iter()
                .map(|o| match o {
                    Some(f) => (Some(*f), None),
                    None => {
                        k += 1;
                        (None, Some(fields[k - 1]))
                    }
                })
                .collect()
        };
        let now = slots
            .iter()
            .position(|(f, t)| (f.is_some() && *f == self.focus) || (t.is_some() && *t == self.typing));
        let indices: Vec<usize> = (0..slots.len()).collect();
        let next = widgets::focus_step(&indices, now, dir).unwrap_or(0);
        let (focus, typing) = slots[next];
        self.focus = focus;
        self.set_typing(typing);
    }

    /// Typing goes into a text entry now (all its text selected, as
    /// VGUI's on focus), or none.
    fn set_typing(&mut self, field: Option<Field>) {
        self.typing = field;
        if field.is_some() {
            self.focus = None;
            let n = self.field_mut().map_or(0, |t| t.chars().count());
            self.caret = Caret { at: n, anchor: 0 };
        }
    }

    /// The line for an empty list.
    pub fn empty_text(&self) -> Option<String> {
        if !self.rows.is_empty() || self.refreshing {
            return None;
        }
        Some(match self.tab {
            Tab::Lan => self.text(
                "#ServerBrowser_NoLanServers",
                "There are no servers running on your local network.",
            ),
            Tab::Favorites => self.text(
                "#ServerBrowser_NoFavoriteServers",
                "You currently have no favorite servers selected.",
            ),
            Tab::History => self.text(
                "#ServerBrowser_NoServersPlayed",
                "No servers have been played recently.",
            ),
            Tab::Internet => String::new(),
        })
    }

    /// The status line under the tabs.
    pub fn status(&self) -> String {
        if self.refreshing {
            self.text("#ServerBrowser_RefreshingServerList", "Refreshing server list...")
        } else {
            String::new()
        }
    }

    /// The filters in words (the line next to the Filters button).
    pub fn filter_text(&self) -> String {
        let f = &self.filters;
        let mut parts = Vec::new();
        if !f.map.trim().is_empty() {
            parts.push(format!(
                "{} {}",
                self.text("#ServerBrowser_Map", "Map").to_lowercase(),
                f.map.trim()
            ));
        }
        if f.latency > 0 {
            let (_, token, ours) = LATENCIES[f.latency.min(LATENCIES.len() - 1)];
            parts.push(format!(
                "{} {}",
                self.text("#ServerBrowser_FilterDescLatency", "latency"),
                self.text(token, ours)
            ));
        }
        if let Some(n) = f.max_players() {
            parts.push(format!(
                "{} <= {n}",
                self.text("#ServerBrowser_FilterDescMaxPlayers", "max players")
            ));
        }
        if f.not_full {
            parts.push(self.text("#ServerBrowser_FilterDescNotFull", "is not full"));
        }
        if f.not_empty {
            parts.push(self.text("#ServerBrowser_FilterDescNotEmpty", "is not empty"));
        }
        if f.no_password {
            parts.push(self.text("#ServerBrowser_FilterDescNoPassword", "has no password"));
        }
        parts.join("; ")
    }

    fn selected_row(&self) -> Option<&ServerRow> {
        self.selected.and_then(|a| self.rows.iter().find(|r| r.addr == a))
    }

    /// Join a server: the password dialog first when it wants one.
    fn join(&mut self, addr: SocketAddr, out: &mut Outcome) {
        let row = self
            .rows
            .iter()
            .find(|r| r.addr == addr)
            .cloned()
            .or_else(|| self.found.clone());
        if row.as_ref().is_some_and(|r| r.password) {
            self.dialog = Dialog::Password {
                addr,
                name: row.map(|r| r.name).unwrap_or_default(),
                text: String::new(),
            };
            self.set_typing(Some(Field::Password));
            return;
        }
        self.connect(addr, None, out);
    }

    fn connect(&mut self, addr: SocketAddr, password: Option<&str>, out: &mut Outcome) {
        if let Some(p) = password {
            out.lines.push(format!("password {}", crate::console::quote(p)));
        }
        out.lines.push(format!("connect {addr}"));
        self.open = false;
        self.dialog = Dialog::None;
        self.typing = None;
    }

    /// Add a server to the favourites (moved to the end if there).
    pub fn add_favorite(&mut self, addr: SocketAddr, name: &str) {
        self.favorites.retain(|s| s.addr != addr);
        self.favorites.push(SavedServer {
            addr,
            name: name.to_string(),
            last_played: 0,
        });
        if self.favorites.len() > MAX_SAVED {
            self.favorites.remove(0);
        }
    }

    /// A server joined: first in the history.
    pub fn played(&mut self, addr: SocketAddr, name: &str, now: u64) {
        let name = self
            .history
            .iter()
            .chain(&self.favorites)
            .find(|s| s.addr == addr && name.is_empty())
            .map_or(name.to_string(), |s| s.name.clone());
        self.history.retain(|s| s.addr != addr);
        self.history.insert(
            0,
            SavedServer {
                addr,
                name,
                last_played: now,
            },
        );
        self.history.truncate(MAX_SAVED);
    }

    /// Apply an input; what it asks for.
    pub fn handle(&mut self, input: Input) -> Outcome {
        let mut out = Outcome::default();
        if !self.open {
            return out;
        }
        if self.dialog != Dialog::None {
            self.handle_dialog(input, &mut out);
            return out;
        }
        if self.combo.is_some() {
            self.handle_combo(input);
            return out;
        }
        match input {
            Input::Click(Target::Tab(tab)) => self.set_tab(tab, &mut out),
            Input::Click(Target::Header(column)) => {
                self.sort = match self.sort {
                    Some((c, desc)) if c == column => Some((c, !desc)),
                    _ => Some((column, false)),
                };
                let mut rows = std::mem::take(&mut self.rows);
                self.sort_rows(&mut rows);
                self.rows = rows;
            }
            Input::Click(Target::Row(i)) => {
                self.set_typing(None);
                self.focus = Some(Focus::List);
                if let Some(r) = self.rows.get(i) {
                    self.selected = Some(r.addr);
                }
            }
            Input::DoubleClick(i) => {
                if let Some(addr) = self.rows.get(i).map(|r| r.addr) {
                    self.selected = Some(addr);
                    self.join(addr, &mut out);
                }
            }
            Input::Click(Target::Scroll(step)) | Input::Scroll(step) => {
                let max = self.rows.len().saturating_sub(self.shown_rows());
                self.scroll = (self.scroll as i32 + step).clamp(0, max as i32) as usize;
            }
            Input::Click(Target::Button(b)) => {
                self.focus = Some(Focus::Button(b));
                self.set_typing(None);
                self.button(b, &mut out);
            }
            Input::Click(Target::Field(f)) => {
                if self.typing != Some(f) {
                    self.set_typing(Some(f));
                }
            }
            Input::Caret(f, at) => {
                if self.typing == Some(f) {
                    self.caret = Caret { at, anchor: at };
                }
            }
            Input::Click(Target::Latency) => {
                self.set_typing(None);
                self.focus = Some(Focus::Latency);
                self.combo = Some(ComboList::open(0, LATENCIES.len(), self.filters.latency));
            }
            Input::Click(Target::Check(c)) => {
                self.set_typing(None);
                self.focus = Some(Focus::Check(c));
                self.toggle(c);
            }
            Input::Click(Target::ComboItem(_) | Target::Outside) | Input::Hover(_) => {}
            Input::ColumnWidth(column, w) => {
                let w = w.max(16.0).round();
                self.widths.retain(|(c, _)| *c != column);
                self.widths.push((column, w));
            }
            Input::Tab(dir) => self.tab_focus(dir),
            Input::Space => match (self.typing, self.focus) {
                (Some(_), _) => self.edit(Edit::Insert(" ".into())),
                (None, Some(Focus::Latency)) => {
                    self.combo = Some(ComboList::open(0, LATENCIES.len(), self.filters.latency));
                }
                (None, Some(Focus::Check(c))) => self.toggle(c),
                (None, Some(Focus::Button(b))) => self.button(b, &mut out),
                _ => {}
            },
            Input::Up | Input::Down if self.focus == Some(Focus::Latency) => {
                let dir = if input == Input::Up { -1 } else { 1 };
                self.filters.latency = widgets::combo_step(self.filters.latency, LATENCIES.len(), dir);
            }
            Input::Up | Input::Down => {
                self.set_typing(None);
                self.focus = Some(Focus::List);
                let n = self.rows.len();
                if n > 0 {
                    let at = self.selected.and_then(|a| self.rows.iter().position(|r| r.addr == a));
                    let next = match (at, input == Input::Down) {
                        (None, _) => 0,
                        (Some(i), true) => (i + 1).min(n - 1),
                        (Some(i), false) => i.saturating_sub(1),
                    };
                    self.selected = Some(self.rows[next].addr);
                    let shown = self.shown_rows();
                    if next < self.scroll {
                        self.scroll = next;
                    } else if next >= self.scroll + shown {
                        self.scroll = next + 1 - shown;
                    }
                }
            }
            Input::Enter => {
                if self.typing.is_some() {
                    self.set_typing(None);
                } else if let Some(Focus::Button(b)) = self.focus {
                    self.button(b, &mut out);
                } else if let Some(addr) = self.selected_row().map(|r| r.addr) {
                    // Connect, the default button.
                    self.join(addr, &mut out);
                }
            }
            Input::Escape => {
                if self.typing.is_some() {
                    self.set_typing(None);
                } else {
                    self.open = false;
                }
            }
            Input::Delete if self.typing.is_some() => self.edit(Edit::Delete),
            Input::Delete => {
                // Ours: Delete takes the selected server off Favorites or
                // History (CS:S: its right-click menu).
                if let Some(addr) = self.selected {
                    let list = match self.tab {
                        Tab::Favorites => Some(&mut self.favorites),
                        Tab::History => Some(&mut self.history),
                        _ => None,
                    };
                    if let Some(list) = list {
                        list.retain(|s| s.addr != addr);
                        self.rows.retain(|r| r.addr != addr);
                        self.total = self.total.saturating_sub(1);
                        self.selected = None;
                        out.save = true;
                    }
                }
            }
            Input::Backspace => self.edit(Edit::Backspace),
            Input::Type(t) => self.edit(Edit::Insert(t)),
            Input::Edit(e) => self.edit(e),
            Input::Copy | Input::Cut => self.copy(input == Input::Cut, &mut out),
        }
        out
    }

    /// A check box ticked or not.
    fn toggle(&mut self, c: Check) {
        let f = &mut self.filters;
        match c {
            Check::NotEmpty => f.not_empty = !f.not_empty,
            Check::NotFull => f.not_full = !f.not_full,
            Check::NoPassword => f.no_password = !f.no_password,
        }
    }

    /// The latency list open: its keys, the pointer on it, the wheel; a
    /// click elsewhere closes it.
    fn handle_combo(&mut self, input: Input) {
        let Some(mut list) = self.combo else { return };
        let event = match input {
            Input::Up => list.key(ComboKey::Up),
            Input::Down => list.key(ComboKey::Down),
            Input::Enter | Input::Space => list.key(ComboKey::Enter),
            Input::Escape => list.key(ComboKey::Escape),
            Input::Hover(Target::ComboItem(k)) => {
                list.hover(k);
                ComboEvent::Open
            }
            Input::Click(Target::ComboItem(k)) => ComboEvent::Pick(k),
            Input::Click(_) | Input::Tab(_) => ComboEvent::Close,
            Input::Scroll(n) => {
                list.wheel(n);
                ComboEvent::Open
            }
            _ => ComboEvent::Open,
        };
        match event {
            ComboEvent::Open => self.combo = Some(list),
            ComboEvent::Close => self.combo = None,
            ComboEvent::Pick(k) => {
                self.combo = None;
                self.filters.latency = k.min(LATENCIES.len() - 1);
            }
        }
    }

    /// An edit to the text entry typed into.
    fn edit(&mut self, edit: Edit) {
        let mut caret = self.caret;
        if let Some(f) = self.field_mut() {
            caret.edit(f, edit, 128);
        }
        self.caret = caret;
    }

    /// Copy (or cut) the typed text's selection.
    fn copy(&mut self, cut: bool, out: &mut Outcome) {
        // A password never leaves its box.
        if self.typing == Some(Field::Password) {
            return;
        }
        let caret = self.caret;
        let Some(text) = self.field_mut().map(|t| caret.selected(t).to_string()) else {
            return;
        };
        if !text.is_empty() {
            out.clipboard = Some(text);
            if cut {
                self.edit(Edit::Delete);
            }
        }
    }
    fn field_mut(&mut self) -> Option<&mut String> {
        match self.typing? {
            Field::MapFilter => Some(&mut self.filters.map),
            Field::MaxPlayers => Some(&mut self.filters.max_players),
            Field::AddAddress => match &mut self.dialog {
                Dialog::AddServer { text, .. } => Some(text),
                _ => None,
            },
            Field::Password => match &mut self.dialog {
                Dialog::Password { text, .. } => Some(text),
                _ => None,
            },
        }
    }

    fn button(&mut self, b: ButtonId, out: &mut Outcome) {
        match b {
            ButtonId::Connect => {
                if let Some(addr) = self.selected_row().map(|r| r.addr) {
                    self.join(addr, out);
                }
            }
            ButtonId::Refresh => self.refresh(out),
            ButtonId::QuickRefresh => {
                let addrs: Vec<SocketAddr> = self.rows.iter().map(|r| r.addr).collect();
                if !addrs.is_empty() {
                    out.requests.push(Request::Query(addrs));
                    self.refreshing = true;
                }
            }
            ButtonId::AddServer => {
                self.dialog = Dialog::AddServer {
                    text: String::new(),
                    looked_up: None,
                    selected: false,
                    error: None,
                };
                self.found = None;
                self.set_typing(Some(Field::AddAddress));
            }
            ButtonId::Filters => {
                self.filters.shown = !self.filters.shown;
                self.typing = None;
            }
            ButtonId::Close => self.open = false,
            _ => {}
        }
    }

    fn handle_dialog(&mut self, input: Input, out: &mut Outcome) {
        let close = |b: &mut Self| {
            b.dialog = Dialog::None;
            b.typing = None;
            b.found = None;
        };
        match input {
            Input::Escape
            | Input::Click(Target::Button(ButtonId::AddCancel))
            | Input::Click(Target::Button(ButtonId::PasswordCancel))
            | Input::Click(Target::Button(ButtonId::Close)) => close(self),
            Input::Type(t) => self.edit(Edit::Insert(t)),
            Input::Backspace => self.edit(Edit::Backspace),
            Input::Delete => self.edit(Edit::Delete),
            Input::Edit(e) => self.edit(e),
            Input::Space => self.edit(Edit::Insert(" ".into())),
            Input::Copy | Input::Cut => self.copy(input == Input::Cut, out),
            Input::Click(Target::Field(f)) => {
                if self.typing != Some(f) {
                    self.set_typing(Some(f));
                }
            }
            Input::Caret(f, at) => {
                if self.typing == Some(f) {
                    self.caret = Caret { at, anchor: at };
                }
            }
            Input::Enter | Input::Click(Target::Button(ButtonId::AddOk | ButtonId::PasswordConnect)) => {
                match self.dialog.clone() {
                    Dialog::AddServer { text, .. } => match crate::net::parse_address(&text) {
                        Ok(addr) => {
                            self.add_favorite(addr, "");
                            out.save = true;
                            close(self);
                            if self.tab == Tab::Favorites {
                                self.refresh(out);
                            }
                        }
                        Err(_) => {
                            let e = self.ui_error();
                            if let Dialog::AddServer { error, .. } = &mut self.dialog {
                                *error = Some(e);
                            }
                        }
                    },
                    Dialog::Password { addr, text, .. } => self.connect(addr, Some(&text), out),
                    Dialog::None => {}
                }
            }
            Input::Click(Target::Button(ButtonId::AddFind)) => {
                if let Dialog::AddServer {
                    text, looked_up, error, ..
                } = &mut self.dialog
                {
                    match crate::net::parse_address(text) {
                        Ok(addr) => {
                            *looked_up = Some(addr);
                            *error = None;
                            out.requests.push(Request::Query(vec![addr]));
                        }
                        Err(_) => *error = Some(String::new()),
                    }
                }
                if matches!(&self.dialog, Dialog::AddServer { error: Some(e), .. } if e.is_empty()) {
                    let e = self.ui_error();
                    if let Dialog::AddServer { error, .. } = &mut self.dialog {
                        *error = Some(e);
                    }
                }
            }
            Input::Click(Target::Row(0)) => {
                if let Dialog::AddServer { selected, .. } = &mut self.dialog {
                    *selected = self.found.is_some();
                }
            }
            // Ours: double-click the server found to join it (a connect
            // to an address).
            Input::DoubleClick(0) => {
                if let Some(addr) = self.found.as_ref().map(|r| r.addr) {
                    self.join(addr, out);
                }
            }
            Input::Click(Target::Button(ButtonId::AddSelected)) => {
                if let (Dialog::AddServer { selected: true, .. }, Some(row)) = (&self.dialog, self.found.clone()) {
                    self.add_favorite(row.addr, &row.name);
                    out.save = true;
                    close(self);
                    if self.tab == Tab::Favorites {
                        self.refresh(out);
                    }
                }
            }
            _ => {}
        }
    }

    fn ui_error(&self) -> String {
        self.text(
            "#ServerBrowser_AddServerError",
            "The server IP address you entered is invalid.",
        )
    }

    // --- Saving.

    /// Favourites and history as a KeyValues file (`serverbrowser.vdf`).
    pub fn saved_text(&self) -> String {
        let mut out = String::from("\"ServerBrowser\"\n{\n");
        for (name, list) in [("Favorites", &self.favorites), ("History", &self.history)] {
            out += &format!("\t\"{name}\"\n\t{{\n");
            for (i, s) in list.iter().enumerate() {
                out += &format!(
                    "\t\t\"{}\"\n\t\t{{\n\t\t\t\"name\"\t\t\"{}\"\n\t\t\t\"address\"\t\t\"{}\"\n\t\t\t\"lastplayed\"\t\t\"{}\"\n\t\t}}\n",
                    i + 1,
                    escape(&s.name),
                    s.addr,
                    s.last_played
                );
            }
            out += "\t}\n";
        }
        out += "}\n";
        out
    }

    /// Read favourites and history from `saved_text`'s format (what it
    /// can; the rest is left out).
    pub fn load_text(&mut self, text: &str) {
        let tokens = crate::mount::keyvalues::tokens(text);
        let mut i = 0;
        let mut path: Vec<String> = Vec::new();
        let mut current: Vec<(String, String)> = Vec::new();
        let (mut fav, mut hist) = (Vec::new(), Vec::new());
        let mut pending_key: Option<String> = None;
        while i < tokens.len() {
            let t = &tokens[i];
            i += 1;
            match t.as_str() {
                "{" => {
                    path.push(pending_key.take().unwrap_or_default());
                    current.clear();
                }
                "}" => {
                    if path.len() == 3 {
                        let get = |k: &str| {
                            current
                                .iter()
                                .find(|(a, _)| a.eq_ignore_ascii_case(k))
                                .map(|(_, v)| v.clone())
                        };
                        if let Some(addr) = get("address").and_then(|a| a.parse::<SocketAddr>().ok()) {
                            let s = SavedServer {
                                addr,
                                name: get("name").unwrap_or_default(),
                                last_played: get("lastplayed").and_then(|v| v.parse().ok()).unwrap_or(0),
                            };
                            match path[1].to_lowercase().as_str() {
                                "favorites" => fav.push(s),
                                "history" => hist.push(s),
                                _ => {}
                            }
                        }
                    }
                    path.pop();
                }
                _ => {
                    if let Some(key) = pending_key.take() {
                        current.push((key, t.clone()));
                    } else {
                        pending_key = Some(t.clone());
                    }
                }
            }
        }
        fav.truncate(MAX_SAVED);
        hist.truncate(MAX_SAVED);
        self.favorites = fav;
        self.history = hist;
    }

    pub fn save_to(&self, path: &Path) -> std::io::Result<()> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        std::fs::write(path, self.saved_text())
    }

    pub fn load_from(&mut self, path: &Path) {
        if let Ok(text) = std::fs::read_to_string(path) {
            self.load_text(&text);
        }
    }
}

/// Where favourites and history are kept: the cfg folder's
/// `serverbrowser.vdf`.
pub fn saved_path() -> Option<PathBuf> {
    crate::console::cfg_dir().map(|d| d.join("serverbrowser.vdf"))
}

fn escape(s: &str) -> String {
    s.replace('\\', "\\\\").replace('"', "\\\"")
}

/// "5:04 PM"-like time a server was last played: here, how long ago.
pub fn last_played_text(then: u64, now: u64) -> String {
    if then == 0 {
        return String::new();
    }
    let ago = now.saturating_sub(then);
    match ago {
        0..60 => "just now".into(),
        60..3600 => format!("{} min ago", ago / 60),
        3600..86400 => format!("{} h ago", ago / 3600),
        _ => format!("{} days ago", ago / 86400),
    }
}

fn unix_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

/// `serverbrowser`'s words as an input.
fn browser_input(a: &[String]) -> Result<Input, String> {
    let n = |i: usize| a.get(i).and_then(|v| v.parse::<usize>().ok()).ok_or("a row number");
    let button = |b| Ok(Input::Click(Target::Button(b)));
    match a.first().map(|s| s.to_lowercase()).as_deref() {
        Some("row") => Ok(Input::Click(Target::Row(n(1)?))),
        Some("doubleclick") => Ok(Input::DoubleClick(n(1)?)),
        Some("connect") => button(ButtonId::Connect),
        Some("refresh") => button(ButtonId::Refresh),
        Some("quickrefresh") => button(ButtonId::QuickRefresh),
        Some("addserver") => button(ButtonId::AddServer),
        Some("filters") => button(ButtonId::Filters),
        Some("find") => button(ButtonId::AddFind),
        Some("addselected") => button(ButtonId::AddSelected),
        Some("type") => Ok(Input::Type(a[1..].join(" "))),
        Some("latency") => Ok(Input::Click(Target::Latency)),
        Some("pick") => Ok(Input::Click(Target::ComboItem(n(1)?))),
        Some("hover") => Ok(Input::Hover(Target::ComboItem(n(1)?))),
        Some("check") => match a.get(1).map(|s| s.to_lowercase()).as_deref() {
            Some("notfull") => Ok(Input::Click(Target::Check(Check::NotFull))),
            Some("notempty") => Ok(Input::Click(Target::Check(Check::NotEmpty))),
            Some("nopassword") => Ok(Input::Click(Target::Check(Check::NoPassword))),
            _ => Err("check notfull|notempty|nopassword".into()),
        },
        Some("map") => Ok(Input::Click(Target::Field(Field::MapFilter))),
        Some("maxplayers") => Ok(Input::Click(Target::Field(Field::MaxPlayers))),
        Some("next") => Ok(Input::Tab(1)),
        Some("space") => Ok(Input::Space),
        Some("enter") => Ok(Input::Enter),
        Some("escape") => Ok(Input::Escape),
        Some("delete") => Ok(Input::Delete),
        Some("down") => Ok(Input::Down),
        Some("up") => Ok(Input::Up),
        Some("tab") => match a.get(1).map(|s| s.to_lowercase()).as_deref() {
            Some("lan") => Ok(Input::Click(Target::Tab(Tab::Lan))),
            Some("favorites" | "favourites") => Ok(Input::Click(Target::Tab(Tab::Favorites))),
            Some("history") => Ok(Input::Click(Target::Tab(Tab::History))),
            _ => Err("tab lan|favorites|history".into()),
        },
        Some("sort") => {
            let c = match a.get(1).map(|s| s.to_lowercase()).as_deref() {
                Some("password") => Column::Password,
                Some("bots") => Column::Bots,
                Some("name") => Column::Name,
                Some("game") => Column::Game,
                Some("players") => Column::Players,
                Some("map") => Column::Map,
                Some("latency") => Column::Latency,
                Some("lastplayed") => Column::LastPlayed,
                _ => return Err("sort name|players|map|latency|password|bots|game|lastplayed".into()),
            };
            Ok(Input::Click(Target::Header(c)))
        }
        _ => Err("serverbrowser row <n> | doubleclick <n> | connect | refresh | ... (help serverbrowser)".into()),
    }
}

/// The rows as `serverbrowser` prints them.
fn describe_rows(b: &ServerBrowser) -> String {
    let mut out = vec![format!(
        "{} tab, {} of {} servers{}{}",
        TABS.iter().find(|t| t.0 == b.tab).map_or("", |t| t.2),
        b.rows.len(),
        b.total,
        if b.refreshing { ", refreshing" } else { "" },
        match &b.dialog {
            Dialog::None => String::new(),
            Dialog::AddServer { .. } => ", add server dialog".into(),
            Dialog::Password { .. } => ", password dialog".into(),
        }
    )];
    for (i, r) in b.rows.iter().enumerate() {
        out.push(format!(
            "{}{i}: {}  {}  {}/{} ({} bots)  {}  {}{}",
            if b.selected == Some(r.addr) { "*" } else { " " },
            r.addr,
            r.name,
            r.players,
            r.max_players,
            r.bots,
            r.map,
            r.ping.map_or("-".into(), |p| format!("{p} ms")),
            if r.password { "  password" } else { "" }
        ));
    }
    out.join("\n")
}

// ---------------------------------------------------------------------------
// The client systems.

fn load_saved(mut browser: ResMut<ServerBrowser>) {
    if let Some(path) = saved_path() {
        browser.load_from(&path);
    }
}

fn save(browser: &ServerBrowser) {
    if let Some(path) = saved_path()
        && let Err(e) = browser.save_to(&path)
    {
        warn!("server browser: saving {}: {e}", path.display());
    }
}

/// A server joined goes first in the history.
fn record_history(
    mut events: MessageReader<crate::net::NetEvent>,
    addr: Option<Res<crate::net::client::ServerAddress>>,
    queries: Res<ServerQueries>,
    mut browser: ResMut<ServerBrowser>,
) {
    for e in events.read() {
        if let crate::net::NetEvent::Joined { .. } = e
            && let Some(addr) = &addr
        {
            let name = match queries.result(addr.0).map(|r| &r.state) {
                Some(QueryState::Answered { info, .. }) => info.name.clone(),
                _ => String::new(),
            };
            browser.played(addr.0, &name, unix_now());
            save(&browser);
        }
    }
}

/// The open tab's rows follow the queries; the game's words come with
/// the menu's.
fn sync_results(
    queries: Res<ServerQueries>,
    menu: Res<GameMenu>,
    mut browser: ResMut<ServerBrowser>,
    mut seen: Local<Option<u64>>,
) {
    if browser.ui != menu.ui {
        browser.ui = menu.ui.clone();
    }
    if !browser.open {
        *seen = None;
        return;
    }
    let busy = queries.busy();
    if *seen != Some(queries.generation) || browser.refreshing != busy || browser.is_changed() {
        *seen = Some(queries.generation);
        let mut next = browser.clone();
        next.set_results(&queries.results, busy);
        if next != *browser {
            *browser = next;
        }
    }
}

fn apply(input: Input, browser: &mut ResMut<ServerBrowser>, console: &mut Console, inbox: &mut RequestInbox) {
    let mut next = (**browser).clone();
    let out = next.handle(input);
    if next != **browser {
        **browser = next;
    }
    for line in out.lines {
        console.submit(line);
    }
    inbox.0.extend(out.requests);
    if out.save {
        save(browser);
    }
    if let Some(text) = out.clipboard
        && let Err(e) = arboard::Clipboard::new().and_then(|mut c| c.set_text(text))
    {
        warn!("clipboard: {e}");
    }
}

/// Queries the browser asked for.
fn run_requests(mut inbox: ResMut<RequestInbox>, lan: Res<LanSettings>, mut queries: ResMut<ServerQueries>) {
    for r in inbox.0.drain(..) {
        match r {
            Request::ScanLan => queries.scan_lan(lan.port_range(), &lan.broadcast_addresses()),
            Request::Query(addrs) => {
                for a in addrs {
                    queries.query(a);
                }
            }
        }
    }
}

/// Requests from input systems, run by `run_requests`.
#[derive(Resource, Default)]
struct RequestInbox(Vec<Request>);

/// Whether the browser takes the keys: open, in front of the menu's
/// dialog (when one is open too), no first-run dialog over it.
fn has_keys(
    browser: &ServerBrowser,
    menu: Option<&GameMenu>,
    windows: Option<&Windows>,
    first_run: Option<&super::first_run::FirstRun>,
) -> bool {
    if !browser.open || first_run.is_some_and(|f| f.open) {
        return false;
    }
    match menu.filter(|m| m.open && m.page != super::game_menu::Page::Main) {
        Some(m) => windows.is_none_or(|w| w.front(&[WINDOW, m.page.window()]) == Some(WINDOW)),
        None => true,
    }
}

/// Keys: typing and editing in the focused box (Shift selects, Ctrl+A,
/// Ctrl+C, Ctrl+X, Ctrl+V), Tab, Space, arrows, Enter, Esc, Delete, F5.
#[allow(clippy::too_many_arguments)]
fn keys(
    mut events: MessageReader<KeyboardInput>,
    held: Res<ButtonInput<KeyCode>>,
    console_ui: Res<super::console::ConsoleUi>,
    mut browser: ResMut<ServerBrowser>,
    mut console: ResMut<Console>,
    mut inbox: ResMut<RequestInbox>,
    (menu, windows, first_run): (
        Option<Res<GameMenu>>,
        Option<Res<Windows>>,
        Option<Res<super::first_run::FirstRun>>,
    ),
) {
    if console_ui.open || !has_keys(&browser, menu.as_deref(), windows.as_deref(), first_run.as_deref()) {
        events.clear();
        return;
    }
    let shift = held.any_pressed([KeyCode::ShiftLeft, KeyCode::ShiftRight]);
    let ctrl = held.any_pressed([
        KeyCode::ControlLeft,
        KeyCode::ControlRight,
        KeyCode::SuperLeft,
        KeyCode::SuperRight,
    ]);
    for e in events.read() {
        if e.state != ButtonState::Pressed {
            continue;
        }
        let typing = browser.typing.is_some();
        let input = match (&e.logical_key, e.key_code) {
            (_, KeyCode::Enter | KeyCode::NumpadEnter) => Input::Enter,
            (_, KeyCode::Escape) => Input::Escape,
            (_, KeyCode::Tab) => Input::Tab(if shift { -1 } else { 1 }),
            (_, KeyCode::ArrowUp) => Input::Up,
            (_, KeyCode::ArrowDown) => Input::Down,
            (_, KeyCode::ArrowLeft) if typing => Input::Edit(Edit::Left(shift)),
            (_, KeyCode::ArrowRight) if typing => Input::Edit(Edit::Right(shift)),
            (_, KeyCode::Home) if typing => Input::Edit(Edit::Home(shift)),
            (_, KeyCode::End) if typing => Input::Edit(Edit::End(shift)),
            (_, KeyCode::KeyA) if typing && ctrl => Input::Edit(Edit::SelectAll),
            (_, KeyCode::KeyC) if typing && ctrl => Input::Copy,
            (_, KeyCode::KeyX) if typing && ctrl => Input::Cut,
            (_, KeyCode::KeyV) if typing && ctrl => {
                let text = arboard::Clipboard::new()
                    .and_then(|mut c| c.get_text())
                    .unwrap_or_default();
                Input::Type(text.lines().next().unwrap_or("").to_string())
            }
            (_, KeyCode::PageUp) => Input::Scroll(-(browser.shown_rows() as i32)),
            (_, KeyCode::PageDown) => Input::Scroll(browser.shown_rows() as i32),
            (_, KeyCode::Delete) => Input::Delete,
            (_, KeyCode::F5) => Input::Click(Target::Button(ButtonId::Refresh)),
            (Key::Backspace, _) => Input::Backspace,
            (Key::Space, _) => Input::Space,
            (Key::Character(s), _) if typing && !ctrl => Input::Type(s.to_string()),
            _ => continue,
        };
        apply(input, &mut browser, &mut console, &mut inbox);
    }
}

/// What a UI node stands for.
#[derive(Component, Clone, Copy)]
struct Hit(Target);

/// The list (the wheel scrolls it).
#[derive(Component)]
struct WheelList;

/// A text entry: a click places its caret at the char under it.
#[derive(Component, Clone)]
struct TextHit {
    field: Field,
    shown: String,
    width: f32,
}

/// A column header's right edge: dragging it sizes the column.
#[derive(Component, Clone, Copy)]
struct ColumnGrip {
    column: Column,
    width: f32,
}

/// Clicks (a second click on a row soon after the first joins it), the
/// pointer over an open list, a click into a text entry, a column's
/// edge dragged, the wheel over the list (or the open list).
#[allow(clippy::too_many_arguments)]
fn pointer(
    hits: Query<(&Interaction, &Hit), Changed<Interaction>>,
    texts: Query<(&Interaction, &TextHit, &RelativeCursorPosition), Changed<Interaction>>,
    grips: Query<(&Interaction, &ColumnGrip), Changed<Interaction>>,
    lists: Query<&RelativeCursorPosition, With<WheelList>>,
    scroll: Option<Res<AccumulatedMouseScroll>>,
    mouse: Res<ButtonInput<MouseButton>>,
    window: Query<&Window, With<bevy::window::PrimaryWindow>>,
    fonts: Res<UiFonts>,
    time: Res<Time<Real>>,
    mut browser: ResMut<ServerBrowser>,
    mut console: ResMut<Console>,
    mut inbox: ResMut<RequestInbox>,
    mut last_click: Local<Option<(usize, f64)>>,
    mut sizing: Local<Option<(Column, f32, f32)>>,
) {
    if !browser.open {
        *sizing = None;
        return;
    }
    let win = window.iter().next();
    let height = win.map_or(720.0, Window::height);
    let scale = (height / 720.0).clamp(0.6, 3.0);
    let now = time.elapsed_secs_f64();
    let mut inputs = Vec::new();
    for (i, hit) in &hits {
        match i {
            Interaction::Hovered => inputs.push(Input::Hover(hit.0)),
            Interaction::Pressed => {
                if let Target::Row(r) = hit.0 {
                    if last_click.is_some_and(|(at, t)| at == r && now - t < 0.4) {
                        inputs.push(Input::DoubleClick(r));
                        *last_click = None;
                        continue;
                    }
                    *last_click = Some((r, now));
                }
                inputs.push(Input::Click(hit.0));
            }
            Interaction::None => {}
        }
    }
    for (i, hit, at) in &texts {
        if *i == Interaction::Pressed
            && let Some(p) = at.normalized
        {
            let font = fonts.source("Default", height, scale, (16.0, false));
            let caret = (browser.typing == Some(hit.field)).then_some(browser.caret);
            let at = widgets::caret_from_click(&fonts, &font, scale, &hit.shown, hit.width, p.x + 0.5, caret);
            inputs.push(Input::Caret(hit.field, at));
        }
    }
    // A column's edge: held, it follows the pointer.
    let x = win.and_then(Window::cursor_position).map(|p| p.x);
    for (i, grip) in &grips {
        if *i == Interaction::Pressed
            && let Some(x) = x
        {
            *sizing = Some((grip.column, grip.width, x));
        }
    }
    if let Some((column, width, from)) = *sizing {
        if !mouse.pressed(MouseButton::Left) {
            *sizing = None;
        } else if let Some(x) = x {
            let w = (width + (x - from) / scale).round();
            if browser.column_width(column, -1.0) != w {
                inputs.push(Input::ColumnWidth(column, w));
            }
        }
    }
    if let Some(scroll) = scroll
        && scroll.delta.y != 0.0
    {
        let notches = widgets::wheel_notches(&scroll);
        if browser.combo.is_some() {
            inputs.push(Input::Scroll(-notches));
        } else if lists.iter().any(|l| l.cursor_over()) {
            inputs.push(Input::Scroll(-notches * 3));
        }
    }
    for input in inputs {
        apply(input, &mut browser, &mut console, &mut inbox);
    }
}

/// The frame made bigger: the list grows with it.
fn follow_size(windows: Res<Windows>, mut browser: ResMut<ServerBrowser>, mut seen: Local<u64>) {
    if windows.resized == *seen {
        return;
    }
    *seen = windows.resized;
    let grow = windows
        .size(WINDOW)
        .map_or(Vec2::ZERO, |s| (s - MIN_SIZE).max(Vec2::ZERO));
    if browser.grow != grow {
        browser.grow = grow;
    }
}

// ---------------------------------------------------------------------------
// Drawing.

#[derive(Component)]
struct BrowserRoot;

/// The column icons as images (from `GameUi::server_icons`).
#[derive(Resource, Default)]
struct BrowserIcons {
    from: Option<Arc<GameUi>>,
    icons: std::collections::HashMap<String, Handle<Image>>,
}

/// How a control follows its parent's corners as the frame grows
/// (VGUI's `pinCorner`: 0 top left, 1 top right, 2 bottom left, 3 bottom
/// right; `autoResize`: 0 none, 1 right, 2 down, 3 both).
fn grown(rect: (f32, f32, f32, f32), pin: u8, resize: u8, grow: Vec2) -> (f32, f32, f32, f32) {
    let (mut x, mut y, mut w, mut h) = rect;
    if pin == 1 || pin == 3 {
        x += grow.x;
    }
    if pin >= 2 {
        y += grow.y;
    }
    if resize & 1 != 0 {
        w += grow.x;
    }
    if resize & 2 != 0 {
        h += grow.y;
    }
    (x, y, w, h)
}

/// A layout's control box (x, y, wide, tall) by name, else `fallback`;
/// moved and sized as the frame grew (its `pinCorner` and `autoResize`,
/// else `fallback_pin`'s).
fn rect_in(
    layout: Option<&UiLayout>,
    name: &str,
    fallback: (f32, f32, f32, f32),
    fallback_pin: (u8, u8),
    grow: Vec2,
) -> (f32, f32, f32, f32) {
    let c = layout.and_then(|l| l.get(name));
    let num = |key: &str, or: u8| {
        c.and_then(|c| c.keys.get(key))
            .and_then(|v| v.trim().parse::<u8>().ok())
            .unwrap_or(if c.is_some() { 0 } else { or })
    };
    let rect = rect_of(layout, name, fallback);
    grown(
        rect,
        num("pincorner", fallback_pin.0),
        num("autoresize", fallback_pin.1),
        grow,
    )
}

/// A layout's control box (x, y, wide, tall) by name, else `fallback`.
fn rect_of(layout: Option<&UiLayout>, name: &str, fallback: (f32, f32, f32, f32)) -> (f32, f32, f32, f32) {
    layout.and_then(|l| l.get(name)).map_or(fallback, |c| {
        let v = |h: crate::map::hud::HudCoord| match h {
            crate::map::hud::HudCoord::Start(v)
            | crate::map::hud::HudCoord::Centre(v)
            | crate::map::hud::HudCoord::End(v) => v,
        };
        (v(c.x), v(c.y), c.wide, c.tall)
    })
}

fn control<'a>(layout: Option<&'a UiLayout>, name: &str) -> Option<&'a UiControl> {
    layout.and_then(|l| l.get(name))
}

/// A control's text from the layout, else the token's, else ours.
fn text_of(b: &ServerBrowser, layout: Option<&UiLayout>, name: &str, token: &str, ours: &str) -> String {
    control(layout, name)
        .map(|c| c.text.clone())
        .filter(|t| !t.is_empty() && !t.starts_with('#'))
        .unwrap_or_else(|| b.text(token, ours))
}

/// A VGUI button that reports `target` (focused: ringed; `default`: the
/// dialog's Enter button; `latched`: a toggle button held down).
#[allow(clippy::too_many_arguments)]
fn button(
    commands: &mut Commands,
    parent: Entity,
    look: &Look,
    rect: (f32, f32, f32, f32),
    text: &str,
    target: Target,
    b: &ServerBrowser,
    (enabled, default, latched): (bool, bool, bool),
) {
    let focused = matches!(target, Target::Button(id) if b.focus == Some(Focus::Button(id)));
    let state = widgets::ButtonState::enabled(enabled)
        .focused(focused)
        .default_button(default)
        .latched(latched);
    widgets::button(commands, parent, look, rect, text, state, -1, Hit(target));
}

/// A text entry that takes typing, with its caret when typed into.
#[allow(clippy::too_many_arguments)]
fn text_box(
    commands: &mut Commands,
    parent: Entity,
    look: &Look,
    rect: (f32, f32, f32, f32),
    text: &str,
    field: Field,
    b: &ServerBrowser,
    hidden: bool,
) {
    let caret = (b.typing == Some(field)).then_some(b.caret);
    let shown = if hidden {
        "*".repeat(text.chars().count())
    } else {
        text.to_string()
    };
    widgets::text_entry(
        commands,
        parent,
        look,
        rect,
        text,
        caret,
        hidden,
        true,
        (
            Hit(Target::Field(field)),
            TextHit {
                field,
                shown,
                width: rect.2,
            },
        ),
    );
}

fn icon(
    commands: &mut Commands,
    parent: Entity,
    look: &Look,
    (x, y, w, h): (f32, f32, f32, f32),
    image: Option<&Handle<Image>>,
    fallback: &str,
    color: Color,
) {
    match image {
        Some(image) => {
            commands.spawn((
                place(
                    look,
                    x + (w - 16.0).max(0.0) / 2.0,
                    y + (h - 16.0).max(0.0) / 2.0,
                    w.min(16.0),
                    h.min(16.0),
                ),
                ImageNode::new(image.clone()),
                ChildOf(parent),
            ));
        }
        None => {
            label(
                commands,
                parent,
                look,
                (x, y, w, h),
                fallback,
                look.font("DefaultSmall", (12.0, false)),
                color,
                0,
            );
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn draw(
    browser: Res<ServerBrowser>,
    fonts: Res<UiFonts>,
    hud: Option<Res<crate::map::hud::ActiveHud>>,
    shown: Query<Entity, With<BrowserRoot>>,
    windows_q: Query<&Window>,
    mut windows: ResMut<Windows>,
    mut icons: ResMut<BrowserIcons>,
    mut images: ResMut<Assets<Image>>,
    mut last: Local<(Vec2, bool, bool)>,
    mut last_minute: Local<u64>,
    mut commands: Commands,
) {
    let size = windows_q
        .iter()
        .next()
        .map_or(Vec2::new(640.0, 480.0), |w| Vec2::new(w.width(), w.height()));
    let resized = (size - last.0).abs().max_element() > 0.5;
    // History's "min ago" moves on.
    let minute = unix_now() / 60;
    let ticked = browser.open && browser.tab == Tab::History && minute != *last_minute;
    if !browser.is_changed() && !resized && !ticked {
        return;
    }
    // Opened, or a dialog opened: in front.
    let dialog = browser.dialog != Dialog::None;
    if browser.open && !last.1 {
        windows.raise(WINDOW);
    }
    if dialog && !last.2 {
        windows.raise(match browser.dialog {
            Dialog::AddServer { .. } => ADD_WINDOW,
            _ => PASSWORD_WINDOW,
        });
    }
    *last = (size, browser.open, dialog);
    *last_minute = minute;
    for e in &shown {
        commands.entity(e).despawn();
    }
    if !browser.open {
        return;
    }
    // The icons, once the game's look is in.
    let ui = browser.ui.0.clone();
    if let Some(ui) = &ui
        && !icons.from.as_ref().is_some_and(|f| Arc::ptr_eq(f, ui))
    {
        icons.icons = ui
            .server_icons
            .iter()
            .map(|(k, pic)| (k.clone(), super::game_menu::ui_image(pic, &mut images)))
            .collect();
        icons.from = Some(ui.clone());
    }
    let accent = hud
        .as_ref()
        .and_then(|h| h.0.color("FgColor"))
        .map(|c| c.with_alpha(1.0))
        .unwrap_or(Color::srgb_u8(255, 176, 0));
    let look = Look {
        ui: ui.as_deref(),
        fonts: &fonts,
        s: (size.y / 720.0).clamp(0.6, 3.0),
        height: size.y,
        accent,
    };
    let root = commands
        .spawn((
            BrowserRoot,
            Node {
                position_type: PositionType::Absolute,
                width: percent(100.0),
                height: percent(100.0),
                ..default()
            },
            // The frames inside stack with the menu's (`widgets`); the
            // root itself catches nothing.
            bevy::ui::FocusPolicy::Pass,
            GlobalZIndex(47),
        ))
        .id();
    let layouts = ui.as_ref().map(|u| &u.servers);
    let dialog = layouts.and_then(|l| l.get("dialog"));
    let page = layouts.and_then(|l| l.get(if browser.filters.shown { "page_filters" } else { "page" }));
    let grow = browser.grow;
    // The frame: the file's size, wide enough for its tabs; bigger as
    // the user made it.
    let (_, _, fw, fh) = rect_of(dialog, "CServerBrowserDialog", (0.0, 0.0, 640.0, 384.0));
    let (tx, ty, tw, th) = rect_in(dialog, "GameTabs", (8.0, 44.0, 624.0, 306.0), (0, 3), grow);
    let (w, h) = (
        fw.max(tx + tw - grow.x + 8.0) + grow.x,
        fh.max(ty + th - grow.y + 34.0) + grow.y,
    );
    let title = browser.text("#ServerBrowser_Servers", "Servers");
    let parts = widgets::frame(
        &mut commands,
        root,
        &look,
        FrameSpec::new(WINDOW).sizeable(MIN_SIZE),
        (w, h),
        &title,
    );
    let f = parts.frame;
    if let Some(close) = parts.close {
        commands.entity(close).insert(Hit(Target::Button(ButtonId::Close)));
    }
    // The tabs over the page.
    let names: Vec<(String, bool)> = TABS
        .iter()
        .map(|(t, token, ours)| (browser.text(token, ours), t.enabled()))
        .collect();
    let open = TABS.iter().position(|t| t.0 == browser.tab).unwrap_or(0);
    widgets::tabs(&mut commands, f, &look, (tx, ty), &names, open, None, |i| {
        Hit(Target::Tab(TABS[i].0))
    });
    // The page: its box, then the list and buttons inside it.
    let (_, py, _, ph) = rect_of(page, "InternetGames", (0.0, 28.0, 624.0, 278.0));
    let page_e = commands
        .spawn((
            Node {
                border: UiRect::all(px(1.0)),
                ..place(&look, tx, ty + py, tw, ph + grow.y)
            },
            bevel(&look, true),
            BackgroundColor(Color::NONE),
            ChildOf(f),
        ))
        .id();
    let list_rect = rect_in(
        page,
        "gamelist",
        (0.0, 8.0, 624.0, if browser.filters.shown { 134.0 } else { 226.0 }),
        (0, 3),
        grow,
    );
    server_list(&mut commands, page_e, &look, &browser, &icons, list_rect);
    page_buttons(&mut commands, page_e, &look, &browser, page);
    if browser.filters.shown {
        filter_controls(&mut commands, page_e, root, &look, &browser, page);
    }
    // The status line.
    let status = rect_in(dialog, "StatusLabel", (11.0, 356.0, 544.0, 24.0), (2, 1), grow);
    label(
        &mut commands,
        f,
        &look,
        status,
        &browser.status(),
        look.default_font(),
        look.text(),
        -1,
    );
    match &browser.dialog {
        Dialog::None => {}
        Dialog::AddServer { .. } => add_server_dialog(&mut commands, root, &look, &browser, &icons),
        Dialog::Password { .. } => password_dialog(&mut commands, root, &look, &browser),
    }
}

/// The list: column headers (sorting, their edges sizing them), rows, a
/// scroll bar, the empty list's line.
fn server_list(
    commands: &mut Commands,
    parent: Entity,
    look: &Look,
    b: &ServerBrowser,
    icons: &BrowserIcons,
    (lx, ly, lw, lh): (f32, f32, f32, f32),
) {
    let list = commands
        .spawn((
            Node {
                border: UiRect::all(px(1.0)),
                overflow: Overflow::clip(),
                ..place(look, lx, ly, lw, lh)
            },
            bevel(look, false),
            BackgroundColor(look.color("ListPanel.BgColor", [0, 0, 0, 128])),
            RelativeCursorPosition::default(),
            WheelList,
            ChildOf(parent),
        ))
        .id();
    if b.focus == Some(Focus::List) {
        commands
            .entity(list)
            .insert(Outline::new(px(1.0), px(0.0), look.dark()));
    }
    let bar_w = look.number("ScrollBar.Wide", 17.0);
    let inner = lw - 2.0 - bar_w;
    let columns: Vec<(Column, String, f32)> = b
        .columns()
        .into_iter()
        .map(|(c, t, w)| (c, t, b.column_width(c, w)))
        .collect();
    let fixed: f32 = columns.iter().map(|c| c.2).sum();
    let widths: Vec<f32> = columns
        .iter()
        .map(|c| if c.2 == 0.0 { (inner - fixed).max(60.0) } else { c.2 })
        .collect();
    let font = look.default_font();
    let small = look.font("DefaultSmall", (13.0, false));
    // Headers.
    let mut x = 0.0;
    for ((column, text, _), cw) in columns.iter().zip(&widths) {
        let e = commands
            .spawn((
                Node {
                    border: UiRect::all(px(1.0)),
                    ..place(look, x, 0.0, *cw, HEADER_H)
                },
                bevel(look, true),
                BackgroundColor(look.color("Button.BgColor", [0, 0, 0, 0])),
                Hit(Target::Header(*column)),
                Button,
                Interaction::default(),
                ChildOf(list),
            ))
            .id();
        match column {
            Column::Password => icon(
                commands,
                e,
                look,
                (0.0, 0.0, cw - 2.0, HEADER_H - 2.0),
                icons.icons.get("password_column"),
                "P",
                look.text(),
            ),
            Column::Bots => icon(
                commands,
                e,
                look,
                (0.0, 0.0, cw - 2.0, HEADER_H - 2.0),
                icons.icons.get("bots_column"),
                "B",
                look.text(),
            ),
            _ => {
                label(
                    commands,
                    e,
                    look,
                    (4.0, 0.0, cw - 20.0, HEADER_H - 2.0),
                    text,
                    small.clone(),
                    look.text(),
                    -1,
                );
                // The sort arrow at the header's right.
                if let Some((c, desc)) = b.sort
                    && c == *column
                {
                    let arrow = if desc { "\u{25BC}" } else { "\u{25B2}" };
                    label(
                        commands,
                        e,
                        look,
                        (cw - 16.0, 0.0, 12.0, HEADER_H - 2.0),
                        arrow,
                        look.font("Marlett", (8.0, false)),
                        look.text(),
                        0,
                    );
                }
            }
        }
        // Its right edge sizes it (not the one that takes the rest).
        if !matches!(column, Column::Password | Column::Bots | Column::Name) {
            commands.spawn((
                place(look, x + cw - 3.0, 0.0, 6.0, HEADER_H),
                ColumnGrip {
                    column: *column,
                    width: *cw,
                },
                Button,
                Interaction::default(),
                ZIndex(3),
                ChildOf(list),
            ));
        }
        x += cw;
    }
    // Rows.
    let shown = b.shown_rows();
    let now = unix_now();
    for (k, row) in b.rows.iter().enumerate().skip(b.scroll).take(shown) {
        let y = HEADER_H + (k - b.scroll) as f32 * ROW_H;
        let selected = b.selected == Some(row.addr);
        let e = commands
            .spawn((
                place(look, 0.0, y, inner, ROW_H),
                BackgroundColor(if selected {
                    look.color("ListPanel.SelectedBgColor", [255, 155, 0, 255])
                } else {
                    Color::NONE
                }),
                Hit(Target::Row(k)),
                Button,
                Interaction::default(),
                ChildOf(list),
            ))
            .id();
        let color = if selected {
            look.color("ListPanel.SelectedTextColor", [0, 0, 0, 255])
        } else if row.responded && !row.other_version {
            look.color("ListPanel.TextColor", [221, 221, 221, 255])
        } else {
            look.dull()
        };
        let mut x = 0.0;
        for ((column, ..), cw) in columns.iter().zip(&widths) {
            let cell = (x + 4.0, 0.0, cw - 8.0, ROW_H);
            let text = match column {
                Column::Password => {
                    if row.password {
                        icon(
                            commands,
                            e,
                            look,
                            (x, 0.0, *cw, ROW_H),
                            icons.icons.get("password"),
                            "*",
                            color,
                        );
                    }
                    None
                }
                Column::Bots => {
                    if row.bots > 0 {
                        icon(
                            commands,
                            e,
                            look,
                            (x, 0.0, *cw, ROW_H),
                            icons.icons.get("bots"),
                            "b",
                            color,
                        );
                    }
                    None
                }
                Column::Name => Some(if row.name.is_empty() && !row.responded {
                    if row.not_responding {
                        format!(
                            "{} {}",
                            row.addr,
                            b.text("#ServerBrowser_NotResponding", "< not responding >")
                        )
                    } else {
                        row.addr.to_string()
                    }
                } else {
                    row.name.clone()
                }),
                Column::Game => Some(row.game.clone()),
                Column::Players => row.responded.then(|| format!("{} / {}", row.players, row.max_players)),
                Column::Map => Some(row.map.clone()),
                Column::Latency => Some(match row.ping {
                    Some(p) => p.to_string(),
                    None if row.not_responding => String::new(),
                    None => b.text("#ServerBrowser_PendingPing", "<pending>"),
                }),
                Column::LastPlayed => Some(last_played_text(row.last_played, now)),
            };
            if let Some(t) = text {
                label(commands, e, look, cell, &t, font.clone(), color, -1);
            }
            x += cw;
        }
    }
    if let Some(t) = b.empty_text() {
        label(
            commands,
            list,
            look,
            (0.0, HEADER_H + 8.0, inner, ROW_H * 2.0),
            &t,
            font.clone(),
            look.color("ListPanel.EmptyListInfoTextColor", [221, 221, 221, 255]),
            0,
        );
    }
    scroll_bar(
        commands,
        list,
        look,
        (inner, 0.0, lh - 2.0),
        (b.rows.len(), shown, b.scroll),
    );
}

/// A scroll bar `h` tall at (x, y).
fn scroll_bar(
    commands: &mut Commands,
    parent: Entity,
    look: &Look,
    (x, y, h): (f32, f32, f32),
    (len, shown, first): (usize, usize, usize),
) {
    let w = look.number("ScrollBar.Wide", 17.0);
    let track = commands
        .spawn((
            place(look, x, y, w, h),
            BackgroundColor(look.sunken_bg()),
            ChildOf(parent),
        ))
        .id();
    let font = look.font("Marlett", (12.0, false));
    let fg = look.color("ScrollBarButton.FgColor", [255, 255, 255, 255]);
    for (ty, text, step) in [(0.0, "\u{25B2}", -1), (h - w, "\u{25BC}", 1)] {
        let e = commands
            .spawn((
                Node {
                    border: UiRect::all(px(1.0)),
                    ..place(look, 0.0, ty, w, w)
                },
                bevel(look, true),
                Hit(Target::Scroll(step)),
                Button,
                Interaction::default(),
                ChildOf(track),
            ))
            .id();
        label(
            commands,
            e,
            look,
            (0.0, 0.0, w - 2.0, w - 2.0),
            text,
            font.clone(),
            fg,
            0,
        );
    }
    let room = h - 2.0 * w;
    if len > shown && room > 8.0 {
        let thumb = (room * shown as f32 / len as f32).max(10.0);
        let at = w + (room - thumb) * first as f32 / (len - shown) as f32;
        commands.spawn((
            Node {
                border: UiRect::all(px(1.0)),
                ..place(look, 1.0, at, w - 2.0, thumb)
            },
            bevel(look, true),
            BackgroundColor(look.color("ScrollBarSlider.BgColor", [255, 255, 255, 64])),
            ChildOf(track),
        ));
    }
}

/// The page's buttons: those the open tab shows, where the file puts
/// them (following the frame's bottom right corner as it grows).
fn page_buttons(commands: &mut Commands, parent: Entity, look: &Look, b: &ServerBrowser, page: Option<&UiLayout>) {
    let g = b.grow;
    let has_selection = b.selected.is_some_and(|a| b.rows.iter().any(|r| r.addr == a));
    let connect = rect_in(page, "ConnectButton", (550.0, 244.0, 67.0, 24.0), (3, 0), g);
    button(
        commands,
        parent,
        look,
        connect,
        &text_of(b, page, "ConnectButton", "#ServerBrowser_Connect", "Connect"),
        Target::Button(ButtonId::Connect),
        b,
        (has_selection, true, false),
    );
    // Internet's "Refresh all" is the others' "Refresh".
    let refresh = rect_in(page, "RefreshButton", (453.0, 244.0, 95.0, 24.0), (3, 0), g);
    let refresh_text = if b.tab == Tab::Internet {
        text_of(b, page, "RefreshButton", "#ServerBrowser_RefreshAll", "Refresh all")
    } else {
        b.text("#ServerBrowser_Refresh", "Refresh")
    };
    button(
        commands,
        parent,
        look,
        refresh,
        &refresh_text,
        Target::Button(ButtonId::Refresh),
        b,
        (true, false, false),
    );
    let right_x = if b.tab == Tab::Favorites {
        let add = rect_in(page, "AddServerButton", (349.0, 244.0, 100.0, 24.0), (3, 0), g);
        button(
            commands,
            parent,
            look,
            add,
            &text_of(b, page, "AddServerButton", "#ServerBrowser_AddServer", "Add a Server"),
            Target::Button(ButtonId::AddServer),
            b,
            (true, false, false),
        );
        add.0
    } else {
        let quick = rect_in(page, "RefreshQuickButton", (345.0, 244.0, 105.0, 24.0), (3, 0), g);
        button(
            commands,
            parent,
            look,
            quick,
            &text_of(
                b,
                page,
                "RefreshQuickButton",
                "#ServerBrowser_RefreshQuick",
                "Quick refresh",
            ),
            Target::Button(ButtonId::QuickRefresh),
            b,
            (!b.rows.is_empty(), false, false),
        );
        quick.0
    };
    let filter = rect_in(page, "Filter", (140.0, 244.0, 108.0, 24.0), (2, 0), g);
    // The file puts Filters after the simplified list box, which only
    // Internet has: here it starts at the left.
    let filter = (8.0, filter.1, filter.2, filter.3);
    button(
        commands,
        parent,
        look,
        filter,
        &text_of(b, page, "Filter", "#ServerBrowser_Filters", "Filters"),
        Target::Button(ButtonId::Filters),
        b,
        (true, false, b.filters.shown),
    );
    if b.filters.active() {
        let (_, y, _, h) = rect_in(page, "FilterString", (250.0, 244.0, 90.0, 24.0), (2, 1), g);
        let x = filter.0 + filter.2 + 4.0;
        label(
            commands,
            parent,
            look,
            (x, y, (right_x - x - 4.0).max(20.0), h),
            &b.filter_text(),
            look.font("DefaultSmall", (13.0, false)),
            look.dull(),
            -1,
        );
    }
}

/// The filters (shown): game, location and anti-cheat as they are
/// (greyed: LAN servers have none of those), map, latency (a drop-down),
/// max players, the three checks; under the list, following the frame's
/// bottom as it grows.
fn filter_controls(
    commands: &mut Commands,
    parent: Entity,
    root: Entity,
    look: &Look,
    b: &ServerBrowser,
    page: Option<&UiLayout>,
) {
    let g = b.grow;
    let at = |name: &str, fallback: (f32, f32, f32, f32)| rect_in(page, name, fallback, (2, 0), g);
    let font = look.default_font();
    for (name, token, ours, fallback) in [
        (
            "GameFilterLabel",
            "#ServerBrowser_Game",
            "Game",
            (12.0, 150.0, 44.0, 24.0),
        ),
        (
            "LocationFilterLabel",
            "#ServerBrowser_Location",
            "Location",
            (234.0, 180.0, 72.0, 24.0),
        ),
        ("MapFilterLabel", "#ServerBrowser_Map", "Map", (12.0, 180.0, 44.0, 24.0)),
        (
            "MaxPlayerFilterLabel",
            "#ServerBrowser_MaxPlayer",
            "Max player count",
            (12.0, 210.0, 144.0, 24.0),
        ),
        (
            "PingFilterLabel",
            "#ServerBrowser_Latency",
            "Latency",
            (234.0, 150.0, 72.0, 24.0),
        ),
        (
            "SecureFilterLabel",
            "#ServerBrowser_AntiCheat",
            "Anti-cheat",
            (236.0, 210.0, 72.0, 24.0),
        ),
    ] {
        let greyed = matches!(name, "LocationFilterLabel" | "SecureFilterLabel");
        let (rect, text) = (at(name, fallback), text_of(b, page, name, token, ours));
        if greyed {
            widgets::engraved(commands, parent, look, rect, &text, font.clone(), 1);
        } else {
            label(commands, parent, look, rect, &text, font.clone(), look.text(), 1);
        }
    }
    let all = b.text("#ServerBrowser_All", "<All>");
    for (name, fallback) in [
        ("GameFilter", (60.0, 150.0, 164.0, 24.0)),
        ("LocationFilter", (311.0, 180.0, 112.0, 24.0)),
        ("SecureFilter", (311.0, 210.0, 112.0, 24.0)),
    ] {
        widgets::combo_box(
            commands,
            parent,
            look,
            at(name, fallback),
            &all,
            false,
            false,
            false,
            (),
        );
    }
    let (_, token, ours) = LATENCIES[b.filters.latency.min(LATENCIES.len() - 1)];
    let ping = at("PingFilter", (311.0, 150.0, 112.0, 24.0));
    widgets::combo_box(
        commands,
        parent,
        look,
        ping,
        &b.text(token, ours),
        true,
        b.combo.is_some(),
        b.focus == Some(Focus::Latency),
        Hit(Target::Latency),
    );
    if let Some(list) = &b.combo {
        let entries: Vec<String> = LATENCIES.iter().map(|(_, t, o)| b.text(t, o)).collect();
        widgets::combo_popup(
            commands,
            parent,
            root,
            look,
            ping,
            &entries,
            list,
            |k| Hit(Target::ComboItem(k)),
            Hit(Target::Outside),
        );
    }
    text_box(
        commands,
        parent,
        look,
        at("MapFilter", (60.0, 180.0, 164.0, 24.0)),
        &b.filters.map,
        Field::MapFilter,
        b,
        false,
    );
    text_box(
        commands,
        parent,
        look,
        at("MaxPlayerFilter", (160.0, 210.0, 64.0, 24.0)),
        &b.filters.max_players,
        Field::MaxPlayers,
        b,
        false,
    );
    for (name, token, ours, fallback, on, which) in [
        (
            "ServerEmptyFilterCheck",
            "#ServerBrowser_HasUsersPlaying",
            "Has users playing",
            (436.0, 170.0, 184.0, 24.0),
            b.filters.not_empty,
            Check::NotEmpty,
        ),
        (
            "ServerFullFilterCheck",
            "#ServerBrowser_ServerNotFull",
            "Server not full",
            (436.0, 150.0, 184.0, 24.0),
            b.filters.not_full,
            Check::NotFull,
        ),
        (
            "NoPasswordFilterCheck",
            "#ServerBrowser_IsNotPasswordProtected",
            "Is not password protected",
            (436.0, 190.0, 184.0, 24.0),
            b.filters.no_password,
            Check::NoPassword,
        ),
    ] {
        let (x, y, w, h) = at(name, fallback);
        widgets::check_button(
            commands,
            parent,
            look,
            (x, y, w.min(624.0 + g.x - x - 4.0), h),
            &text_of(b, page, name, token, ours),
            on,
            true,
            b.focus == Some(Focus::Check(which)),
            Hit(Target::Check(which)),
        );
    }
}

/// A dialog frame of a layout (modal over the browser), its size from the
/// layout, its close box Cancel.
fn dialog_frame(
    commands: &mut Commands,
    root: Entity,
    look: &Look,
    (id, layout, name): (widgets::WindowId, Option<&UiLayout>, &str),
    fallback: (f32, f32),
    title: &str,
) -> Entity {
    let (_, _, w, h) = rect_of(layout, name, (0.0, 0.0, fallback.0, fallback.1));
    let parts = widgets::frame(commands, root, look, FrameSpec::new(id).modal(), (w, h), title);
    if let Some(close) = parts.close {
        commands.entity(close).insert(Hit(Target::Button(ButtonId::Close)));
    }
    parts.frame
}

/// Add a Server: the address box, add it or find games at it, the
/// server found (select it to add it; ours: double-click joins it).
fn add_server_dialog(commands: &mut Commands, root: Entity, look: &Look, b: &ServerBrowser, icons: &BrowserIcons) {
    let Dialog::AddServer {
        text,
        selected,
        error,
        looked_up,
    } = &b.dialog
    else {
        return;
    };
    let layout = b.ui.0.as_ref().and_then(|u| u.servers.get("add"));
    let title = control(layout, "DialogAddServer")
        .and_then(|c| c.keys.get("title"))
        .map(|t| b.text(t, "Add Server - Servers"))
        .unwrap_or_else(|| b.text("#ServerBrowser_AddServersTitle", "Add Server - Servers"));
    let f = dialog_frame(
        commands,
        root,
        look,
        (ADD_WINDOW, layout, "DialogAddServer"),
        (572.0, 390.0),
        &title,
    );
    let font = look.default_font();
    label(
        commands,
        f,
        look,
        rect_of(layout, "InfoLabel", (22.0, 46.0, 330.0, 20.0)),
        &text_of(
            b,
            layout,
            "InfoLabel",
            "#ServerBrowser_EnterIPofServerToAdd",
            "Enter the IP address of the server you wish to add.",
        ),
        look.font("UiBold", (12.0, true)),
        look.text(),
        -1,
    );
    text_box(
        commands,
        f,
        look,
        rect_of(layout, "ServerNameText", (20.0, 74.0, 330.0, 24.0)),
        text,
        Field::AddAddress,
        b,
        false,
    );
    let has_text = !text.trim().is_empty();
    button(
        commands,
        f,
        look,
        rect_of(layout, "OKButton", (356.0, 74.0, 190.0, 24.0)),
        &text_of(
            b,
            layout,
            "OKButton",
            "#ServerBrowser_AddAddressToFavorites",
            "Add this address to favorites",
        ),
        Target::Button(ButtonId::AddOk),
        b,
        (has_text, true, false),
    );
    button(
        commands,
        f,
        look,
        rect_of(layout, "TestServersButton", (356.0, 102.0, 190.0, 24.0)),
        &text_of(
            b,
            layout,
            "TestServersButton",
            "#ServerBrowser_FindGames",
            "Find games at this address...",
        ),
        Target::Button(ButtonId::AddFind),
        b,
        (has_text, false, false),
    );
    button(
        commands,
        f,
        look,
        rect_of(layout, "CancelButton", (482.0, 131.0, 64.0, 24.0)),
        &text_of(b, layout, "CancelButton", "#ServerBrowser_Cancel", "Cancel"),
        Target::Button(ButtonId::AddCancel),
        b,
        (true, false, false),
    );
    // The examples, one per line (or the address's error).
    let (ex, ey, ew, _) = rect_of(layout, "ExampleLabel", (22.0, 106.0, 328.0, 74.0));
    let examples = match error {
        Some(e) => vec![e.clone()],
        None => text_of(
            b,
            layout,
            "ExampleLabel",
            "#ServerBrowser_Examples",
            "Examples:\ntfc.valvesoftware.com\ncounterstrike.speakeasy.net:27016\n205.158.143.200:27015",
        )
        .replace("\\n", "\n")
        .lines()
        .map(String::from)
        .collect(),
    };
    for (k, line) in examples.iter().enumerate() {
        label(
            commands,
            f,
            look,
            (ex, ey + k as f32 * 16.0, ew, 16.0),
            line,
            font.clone(),
            look.dull(),
            -1,
        );
    }
    // The servers found: a tab over a list.
    let (gx, gy, gw, gh) = rect_of(layout, "GameTabs", (20.0, 175.0, 526.0, 150.0));
    let tab_text = b
        .text("#ServerBrowser_ServersCount", "Servers (%s1)")
        .replace("%s1", &b.found.iter().filter(|r| r.responded).count().to_string());
    widgets::tabs(
        commands,
        f,
        look,
        (gx, gy + 1.0),
        &[(tab_text, true)],
        0,
        Some(110.0),
        |_| (),
    );
    let (_, sy, _, sh) = rect_of(layout, "Servers", (0.0, 28.0, 526.0, 122.0));
    let list = commands
        .spawn((
            Node {
                border: UiRect::all(px(1.0)),
                ..place(look, gx, gy + sy, gw, sh.min(gh - sy))
            },
            bevel(look, false),
            BackgroundColor(look.color("ListPanel.BgColor", [0, 0, 0, 128])),
            ChildOf(f),
        ))
        .id();
    let headers = [
        (
            b.text("#ServerBrowser_Servers", "Servers"),
            gw - 2.0 - 112.0 - 55.0 - 90.0 - 55.0,
        ),
        (b.text("#ServerBrowser_Game", "Game"), 112.0),
        (b.text("#ServerBrowser_Players", "Players"), 55.0),
        (b.text("#ServerBrowser_Map", "Map"), 90.0),
        (b.text("#ServerBrowser_Latency", "Latency"), 55.0),
    ];
    let small = look.font("DefaultSmall", (13.0, false));
    let mut x = 0.0;
    for (t, cw) in &headers {
        let e = commands
            .spawn((
                Node {
                    border: UiRect::all(px(1.0)),
                    ..place(look, x, 0.0, *cw, HEADER_H)
                },
                bevel(look, true),
                ChildOf(list),
            ))
            .id();
        label(
            commands,
            e,
            look,
            (4.0, 0.0, cw - 8.0, HEADER_H - 2.0),
            t,
            small.clone(),
            look.text(),
            -1,
        );
        x += cw;
    }
    if let (Some(row), Some(_)) = (&b.found, looked_up) {
        let color = if *selected {
            look.color("ListPanel.SelectedTextColor", [0, 0, 0, 255])
        } else {
            look.color("ListPanel.TextColor", [221, 221, 221, 255])
        };
        let e = commands
            .spawn((
                place(look, 0.0, HEADER_H, gw - 2.0, ROW_H),
                BackgroundColor(if *selected {
                    look.color("ListPanel.SelectedBgColor", [255, 155, 0, 255])
                } else {
                    Color::NONE
                }),
                Hit(Target::Row(0)),
                Button,
                Interaction::default(),
                ChildOf(list),
            ))
            .id();
        let name = if row.responded {
            row.name.clone()
        } else if row.not_responding {
            b.text("#ServerBrowser_ServerNotResponding", "Server is not responding.")
        } else {
            b.text("#ServerBrowser_RefreshingServerList", "Refreshing server list...")
        };
        let cells = [
            name,
            row.game.clone(),
            if row.responded {
                format!("{} / {}", row.players, row.max_players)
            } else {
                String::new()
            },
            row.map.clone(),
            row.ping.map(|p| p.to_string()).unwrap_or_default(),
        ];
        let mut x = 0.0;
        for (t, (_, cw)) in cells.iter().zip(&headers) {
            label(
                commands,
                e,
                look,
                (x + 4.0, 0.0, cw - 8.0, ROW_H),
                t,
                font.clone(),
                color,
                -1,
            );
            x += cw;
        }
        if row.password {
            icon(
                commands,
                e,
                look,
                (gw - 20.0, 0.0, 16.0, ROW_H),
                icons.icons.get("password"),
                "*",
                color,
            );
        }
    }
    button(
        commands,
        f,
        look,
        rect_of(layout, "SelectedOKButton", (316.0, 340.0, 230.0, 24.0)),
        &text_of(
            b,
            layout,
            "SelectedOKButton",
            "#ServerBrowser_AddSelectedToFavorites",
            "Add selected game server to favorites",
        ),
        Target::Button(ButtonId::AddSelected),
        b,
        (*selected && b.found.as_ref().is_some_and(|r| r.responded), false, false),
    );
}

/// The server wants a password: its name, the password box, Connect.
fn password_dialog(commands: &mut Commands, root: Entity, look: &Look, b: &ServerBrowser) {
    let Dialog::Password { name, text, .. } = &b.dialog else {
        return;
    };
    let layout = b.ui.0.as_ref().and_then(|u| u.servers.get("password"));
    let title = control(layout, "DialogServerPassword")
        .and_then(|c| c.keys.get("title"))
        .map(|t| b.text(t, "Server Requires Password"))
        .unwrap_or_else(|| b.text("#ServerBrowser_ServerRequiresPasswordTitle", "Server Requires Password"));
    let f = dialog_frame(
        commands,
        root,
        look,
        (PASSWORD_WINDOW, layout, "DialogServerPassword"),
        (290.0, 176.0),
        &title,
    );
    let font = look.default_font();
    label(
        commands,
        f,
        look,
        rect_of(layout, "GameLabel", (20.0, 42.0, 252.0, 24.0)),
        name,
        font.clone(),
        look.dull(),
        -1,
    );
    label(
        commands,
        f,
        look,
        rect_of(layout, "InfoLabel", (20.0, 68.0, 252.0, 24.0)),
        &text_of(
            b,
            layout,
            "InfoLabel",
            "#ServerBrowser_PasswordRequired",
            "This server requires a password to join.",
        ),
        font.clone(),
        look.text(),
        -1,
    );
    label(
        commands,
        f,
        look,
        rect_of(layout, "PasswordLabel", (20.0, 95.0, 66.0, 24.0)),
        &text_of(b, layout, "PasswordLabel", "#ServerBrowser_PasswordLabel", "Password"),
        font.clone(),
        look.text(),
        1,
    );
    text_box(
        commands,
        f,
        look,
        rect_of(layout, "PasswordEntry", (86.0, 96.0, 186.0, 24.0)),
        text,
        Field::Password,
        b,
        true,
    );
    button(
        commands,
        f,
        look,
        rect_of(layout, "ConnectButton", (116.0, 136.0, 74.0, 24.0)),
        &text_of(b, layout, "ConnectButton", "#ServerBrowser_Connect", "Connect"),
        Target::Button(ButtonId::PasswordConnect),
        b,
        (true, true, false),
    );
    button(
        commands,
        f,
        look,
        rect_of(layout, "CancelButton", (198.0, 136.0, 74.0, 24.0)),
        &text_of(b, layout, "CancelButton", "#ServerBrowser_Cancel", "Cancel"),
        Target::Button(ButtonId::PasswordCancel),
        b,
        (true, false, false),
    );
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::net::query::ServerInfo;

    fn addr(port: u16) -> SocketAddr {
        SocketAddr::from(([192, 168, 1, 5], port))
    }

    fn answered(port: u16, name: &str, map: &str, players: u8, bots: u8, ping: u32, password: bool) -> QueryResult {
        QueryResult {
            addr: addr(port),
            state: QueryState::Answered {
                info: ServerInfo {
                    name: name.into(),
                    map: map.into(),
                    folder: "mashup".into(),
                    game: "Lucker Party".into(),
                    players,
                    max_players: 8,
                    bots,
                    password,
                    version: NET_VERSION.into(),
                    ..default()
                },
                ping_ms: ping,
            },
            lan: true,
        }
    }

    fn lan_browser() -> (ServerBrowser, Vec<QueryResult>) {
        let mut b = ServerBrowser::default();
        let out = b.open(Some(Tab::Lan));
        assert_eq!(out.requests, [Request::ScanLan]);
        let results = vec![
            answered(27031, "Beta", "de_dust2", 3, 2, 40, false),
            answered(27032, "alpha", "cs_office", 8, 0, 12, true),
            answered(27033, "Gamma", "de_dust2", 0, 0, 90, false),
        ];
        b.set_results(&results, false);
        (b, results)
    }

    fn names(b: &ServerBrowser) -> Vec<&str> {
        b.rows.iter().map(|r| r.name.as_str()).collect()
    }

    #[test]
    fn lan_rows_sort_by_latency_then_by_a_header() {
        let (mut b, _) = lan_browser();
        assert_eq!(names(&b), ["alpha", "Beta", "Gamma"], "by latency at first");
        b.handle(Input::Click(Target::Header(Column::Name)));
        assert_eq!(names(&b), ["alpha", "Beta", "Gamma"]);
        b.handle(Input::Click(Target::Header(Column::Name)));
        assert_eq!(names(&b), ["Gamma", "Beta", "alpha"], "again: reversed");
        b.handle(Input::Click(Target::Header(Column::Players)));
        assert_eq!(names(&b), ["Gamma", "Beta", "alpha"]);
        assert_eq!(b.columns()[2].1, "Servers (3)");
    }

    #[test]
    fn filters_as_the_original_offers() {
        let (mut b, results) = lan_browser();
        b.filters.no_password = true;
        b.set_results(&results, false);
        assert_eq!(names(&b), ["Beta", "Gamma"]);
        b.filters = Filters {
            not_empty: true,
            ..default()
        };
        b.set_results(&results, false);
        assert_eq!(
            names(&b),
            ["alpha", "Beta"],
            "bots alone aren't users; 3 players with 2 bots is"
        );
        b.filters = Filters {
            not_full: true,
            map: "DUST".into(),
            ..default()
        };
        b.set_results(&results, false);
        assert_eq!(names(&b), ["Beta", "Gamma"]);
        b.filters = Filters {
            latency: 1,
            ..default()
        };
        b.set_results(&results, false);
        assert_eq!(names(&b), ["alpha", "Beta"], "< 50 ms");
        assert_eq!(b.total, 3, "the header counts them all");
        assert!(b.filters.active());
        assert_eq!(b.filter_text(), "latency < 50");
    }

    #[test]
    fn connect_and_double_click_join() {
        let (mut b, _) = lan_browser();
        assert!(
            b.handle(Input::Click(Target::Button(ButtonId::Connect)))
                .lines
                .is_empty(),
            "nothing selected"
        );
        b.handle(Input::Click(Target::Row(1)));
        let out = b.handle(Input::Click(Target::Button(ButtonId::Connect)));
        assert_eq!(out.lines, ["connect 192.168.1.5:27031"]);
        assert!(!b.open, "the browser closes: the game connects");
        let (mut b, _) = lan_browser();
        let out = b.handle(Input::DoubleClick(2));
        assert_eq!(out.lines, ["connect 192.168.1.5:27033"]);
    }

    #[test]
    fn a_password_server_asks_for_it_first() {
        let (mut b, _) = lan_browser();
        let out = b.handle(Input::DoubleClick(0));
        assert!(out.lines.is_empty());
        assert!(matches!(&b.dialog, Dialog::Password { name, .. } if name == "alpha"));
        assert_eq!(b.typing, Some(Field::Password));
        b.handle(Input::Type("se".into()));
        b.handle(Input::Type("cret".into()));
        let out = b.handle(Input::Enter);
        assert_eq!(out.lines, ["password secret", "connect 192.168.1.5:27032"]);
        // Cancelled: nothing.
        let (mut b, _) = lan_browser();
        b.handle(Input::DoubleClick(0));
        assert!(b.handle(Input::Escape).lines.is_empty());
        assert_eq!(b.dialog, Dialog::None);
        assert!(b.open);
    }

    #[test]
    fn internet_is_greyed() {
        let mut b = ServerBrowser::default();
        b.open(Some(Tab::Lan));
        let out = b.handle(Input::Click(Target::Tab(Tab::Internet)));
        assert_eq!(b.tab, Tab::Lan);
        assert!(out.requests.is_empty());
    }

    #[test]
    fn add_a_server_to_favorites() {
        let mut b = ServerBrowser::default();
        let out = b.open(Some(Tab::Favorites));
        assert!(out.requests.is_empty(), "no favourites to ask");
        b.set_results(&[], false);
        assert!(b.empty_text().unwrap().contains("no favorite"));
        b.handle(Input::Click(Target::Button(ButtonId::AddServer)));
        assert_eq!(b.typing, Some(Field::AddAddress));
        b.handle(Input::Type("not an address!".into()));
        let out = b.handle(Input::Click(Target::Button(ButtonId::AddOk)));
        assert!(!out.save);
        assert!(matches!(&b.dialog, Dialog::AddServer { error: Some(_), .. }));
        for _ in 0..20 {
            b.handle(Input::Backspace);
        }
        b.handle(Input::Type("10.0.0.7:27040".into()));
        let out = b.handle(Input::Click(Target::Button(ButtonId::AddFind)));
        let a: SocketAddr = "10.0.0.7:27040".parse().unwrap();
        assert_eq!(out.requests, [Request::Query(vec![a])]);
        let out = b.handle(Input::Enter);
        assert!(out.save);
        assert_eq!(b.dialog, Dialog::None);
        assert_eq!(b.favorites.len(), 1);
        assert_eq!(out.requests, [Request::Query(vec![a])], "the tab refreshes");
        // Pending, then not responding: listed by address.
        b.set_results(&[], true);
        assert_eq!(b.rows.len(), 1);
        assert!(!b.rows[0].responded);
        b.set_results(
            &[QueryResult {
                addr: a,
                state: QueryState::NoResponse,
                lan: false,
            }],
            false,
        );
        assert!(b.rows[0].not_responding);
        // Delete takes it off.
        b.handle(Input::Down);
        let out = b.handle(Input::Delete);
        assert!(out.save && b.favorites.is_empty());
    }

    #[test]
    fn history_first_played_first() {
        let mut b = ServerBrowser::default();
        b.played(addr(1), "one", 100);
        b.played(addr(2), "two", 200);
        b.played(addr(1), "", 300);
        assert_eq!(b.history.len(), 2);
        assert_eq!(
            (b.history[0].addr, b.history[0].name.as_str(), b.history[0].last_played),
            (addr(1), "one", 300)
        );
        assert_eq!(last_played_text(300, 300 + 125), "2 min ago");
        assert_eq!(last_played_text(0, 5), "");
    }

    #[test]
    fn saved_lists_round_trip() {
        let mut b = ServerBrowser::default();
        b.add_favorite(addr(27031), "A \"quoted\" name \\ too");
        b.add_favorite("[::1]:27040".parse().unwrap(), "");
        b.played(addr(27032), "History one", 1_700_000_000);
        let text = b.saved_text();
        let mut c = ServerBrowser::default();
        c.load_text(&text);
        assert_eq!(c.favorites, b.favorites);
        assert_eq!(c.history, b.history);
        // Junk and missing parts: what can be read.
        let mut d = ServerBrowser::default();
        d.load_text("\"ServerBrowser\" { \"Favorites\" { \"1\" { \"address\" \"nope\" } \"2\" { \"address\" \"1.2.3.4:5\" } } // c\n");
        assert_eq!(d.favorites.len(), 1);
        assert_eq!(d.favorites[0].addr, "1.2.3.4:5".parse::<SocketAddr>().unwrap());
    }
}
