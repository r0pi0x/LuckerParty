//! Create Server: its Server, Game and Bot pages, and the console lines
//! a new game starts with.

use super::*;

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
pub(super) const NORMAL: usize = 1;
/// Bots a new game offers at most (the box takes two digits).
pub(super) const MAX_BOTS: u8 = 30;

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
pub(super) const BOT_CVARS: [(&str, &str, &str); 2] = [
    ("BotPrefixEntry", "bot_prefix", "#CStrike_Bot_NamePrefix"),
    ("BotJoinAfterPlayerCheck", "bot_join_after_player", "#CStrike_Bot_JoinAfterPlayer"),
];

/// Ours on the Game page, after the game's own: rounds or deathmatch.
pub(super) const ROUNDS_CVAR: &str = "mashup_rounds";

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

/// Create Server's Game page: the game's server options (`settings.scr`)
/// in a scrolled list, each its label and control (CS:S's
/// `CPanelListPanel`, where the layout puts it).
pub(super) fn game_options(commands: &mut Commands, ctx: &Ctx, parent: Entity, (w, h): (f32, f32)) {
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

impl GameMenu {
    /// Create Server's cvars: the install's Game page options (else none),
    /// ours, the Bot page's; each with its value now.
    pub(super) fn server_cvars(&self, get: &dyn Fn(&str) -> Option<String>) -> Vec<ServerCvar> {
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

    /// Start the new game set up in Create Server: from a game, close (the
    /// old map plays until the new one is in); from the main menu, show it
    /// loading.
    pub(super) fn start(&mut self, out: &mut Outcome) {
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
    pub(super) fn quick_setup(&mut self) {
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
    pub(super) fn server_cvar(&self, cvar: &str) -> Option<&ServerCvar> {
        self.new_game.cvars.iter().find(|c| c.cvar == cvar)
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
}

impl GameMenu {
    /// Create Server's rows: the open page's controls, Start, Cancel.
    pub(super) fn create_server_rows(&self) -> Vec<Row> {
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
        rows.push(self.cancel_row());
        rows
    }
}
