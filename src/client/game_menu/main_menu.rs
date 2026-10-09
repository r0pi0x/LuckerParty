//! The main menu: the left-hand entries (the game's, then ours) and what
//! each does.

use super::*;

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
pub(super) const QUICK_BOTS: u8 = 9;

impl MainItem {
    pub(super) fn page(self) -> Option<Page> {
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
    pub(super) fn new(item: MainItem, label: &str, in_game_only: bool) -> Self {
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

/// The left-hand entries, as GameUI's game menu: at the left inset, one
/// entry per `MainMenu.MenuItemHeight`, the bottom one a fixed distance up
/// from the screen's foot, the game's title over them (its title font,
/// else the menu's); greyed entries in the disabled colour.
pub(super) fn main_list(
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

impl GameMenu {
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

    pub(super) fn main(&mut self, item: MainItem, out: &mut Outcome) {
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
                out.lines.push(format!("map {}", crate::client::console::GREYBOX));
                if self.in_game {
                    self.close(out);
                } else {
                    self.loading = Some(crate::client::console::GREYBOX.into());
                }
            }
            MainItem::Console => out.lines.push("toggleconsole".into()),
            MainItem::FindServers => out.lines.push("openserverbrowser".into()),
            MainItem::NewGame | MainItem::Bots | MainItem::Team | MainItem::Options | MainItem::Extras => {}
        }
    }
}

impl GameMenu {
    /// The main page's rows: the left-hand entries shown.
    pub(super) fn main_rows(&self) -> Vec<Row> {
        self.entries()
            .iter()
            .map(|e| Row::Button {
                label: e.label.clone(),
                action: Action::Main(e.item),
                enabled: e.enabled,
            })
            .collect()
    }
}
