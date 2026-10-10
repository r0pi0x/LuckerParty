//! The scoreboard, CS:S style: held with Tab (or `+showscores`). With the
//! game's layout (`map::hud::GameMenus::scoreboard`, CS:S's
//! `resource/ui/scoreboard.res`) it is drawn in the game's look: its
//! background picture, the map name, each team's name, players alive and
//! rounds won, the column headings, and a row per player placed on the
//! layout's first-row cells (`CTPlayerName0`, `TPlayerStatus0`, ...),
//! counter-terrorists left and terrorists right; the status column shows
//! the game's own icons (`hud/scoreboard_dead`, and to teammates
//! `scoreboard_bomb` by the carrier and `scoreboard_defuser` by a player
//! with a kit; spec objectives.md: the bomb for Ts only), the latency
//! column "BOT" for bots and each player's ping (a network game's
//! `net::NetScore`, which also brings scores and the bomb and kit
//! markers; 0 in single player), the local
//! player's row lit with `scoreboard-select`. Without the layout: two
//! plain team columns with the status in words.

use bevy::prelude::*;

use super::vgui::{Painter, Shown, VguiMenu};
use crate::{
    core::{Intent, LocalPlayer, Team},
    map::hud::{ActiveHud, GameMenus, UiControl},
    objectives::bomb::C4,
    rules::{Dead, Score},
    weapon::{Weapon, economy::DefuseKit},
};

pub struct ScoreboardPlugin;

impl Plugin for ScoreboardPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, spawn).add_systems(Update, update);
    }
}

#[derive(Component)]
struct Board;

/// The game-look scoreboard's root (the whole window).
#[derive(Component)]
struct GameBoard;

#[derive(Component)]
struct Column(u8);

const T_RED: Color = Color::srgb(1.0, 0.25, 0.25);
const CT_BLUE: Color = Color::srgb(0.6, 0.8, 1.0);
/// Virtual units from one player row to the next. The layout gives only
/// the first row (14 tall); 16 fills its 272-unit player area with 16
/// rows (a guess, not measured).
pub const ROW_PITCH: f32 = 16.0;

fn spawn(mut commands: Commands) {
    let board = commands
        .spawn((
            Board,
            Node {
                position_type: PositionType::Absolute,
                left: percent(15.0),
                right: percent(15.0),
                top: percent(12.0),
                padding: UiRect::all(px(12.0)),
                column_gap: px(16.0),
                border_radius: BorderRadius::all(px(6.0)),
                ..default()
            },
            BackgroundColor(Color::srgba(0.0, 0.0, 0.0, 0.75)),
            GlobalZIndex(60),
            Visibility::Hidden,
        ))
        .id();
    for team in [1u8, 2] {
        commands.spawn((
            Column(team),
            Node {
                flex_direction: FlexDirection::Column,
                flex_grow: 1.0,
                flex_basis: px(0.0),
                row_gap: px(3.0),
                ..default()
            },
            ChildOf(board),
        ));
    }
    commands.spawn((
        GameBoard,
        Node {
            position_type: PositionType::Absolute,
            width: percent(100.0),
            height: percent(100.0),
            ..default()
        },
        GlobalZIndex(60),
        Visibility::Hidden,
    ));
}

/// One row: name, kills, deaths, latency, status; the first cell in
/// `fonts.0`, the others in `fonts.1`.
fn row(
    commands: &mut Commands,
    parent: Entity,
    cells: [String; 5],
    color: Color,
    highlight: bool,
    fonts: (&TextFont, &TextFont),
) {
    let r = commands
        .spawn((
            Node {
                flex_direction: FlexDirection::Row,
                padding: UiRect::axes(px(6.0), px(2.0)),
                ..default()
            },
            BackgroundColor(if highlight {
                Color::srgba(1.0, 1.0, 1.0, 0.12)
            } else {
                Color::NONE
            }),
            ChildOf(parent),
        ))
        .id();
    for (i, text) in cells.into_iter().enumerate() {
        commands.spawn((
            Text::new(text),
            if i == 0 { fonts.0.clone() } else { fonts.1.clone() },
            TextColor(color),
            TextLayout::justify(if i == 0 { Justify::Left } else { Justify::Right }),
            Node {
                width: if i == 0 { percent(48.0) } else { percent(13.0) },
                ..default()
            },
            ChildOf(r),
        ));
    }
}

/// What the status column shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    None,
    Dead,
    Bomb,
    Defuser,
}

impl Status {
    /// The plain board's word.
    fn word(self) -> &'static str {
        match self {
            Status::None => "",
            Status::Dead => "DEAD",
            Status::Bomb => "BOMB",
            Status::Defuser => "DEFUSER",
        }
    }

    /// The game's icon (a `GameHud::sprites` key).
    pub fn icon(self) -> Option<&'static str> {
        match self {
            Status::None => None,
            Status::Dead => Some("scoreboard_dead"),
            Status::Bomb => Some("scoreboard_bomb"),
            Status::Defuser => Some("scoreboard_defuser"),
        }
    }
}

/// The status column: dead, else for a teammate the bomb (carrying it) or
/// the defuser (has a kit); the other team's are hidden.
pub fn status(dead: bool, carrier: bool, kit: bool, teammate: bool) -> Status {
    match (dead, teammate && carrier, teammate && kit) {
        (true, ..) => Status::Dead,
        (_, true, _) => Status::Bomb,
        (_, _, true) => Status::Defuser,
        _ => Status::None,
    }
}

/// The latency column: "BOT" for bots (as CS:S shows them), else the
/// player's ping, ms (0 in single player and for a listen server's host).
pub fn latency(bot: bool, ping: u16) -> String {
    if bot { "BOT".into() } else { ping.to_string() }
}

/// Whether the board shows when held: only in a game, never under the
/// game menu (the main menu, the Esc menu or the loading dialog: GameUI
/// has the keys there, and the last game's players may still be about).
pub fn shown(held: bool, menu: Option<&super::game_menu::GameMenu>) -> bool {
    held && menu.is_none_or(|m| m.in_game && !m.open && m.loading.is_none() && m.failure.is_none())
}

/// A scoreboard row.
#[derive(Clone, Debug, PartialEq)]
struct Row {
    /// 1 terrorists, 2 counter-terrorists (and anyone else).
    column: u8,
    name: String,
    kills: i32,
    deaths: u32,
    dead: bool,
    local: bool,
    bot: bool,
    ping: u16,
    status: Status,
}

/// What the board shows; redrawn when it changes.
#[derive(Clone, Debug, PartialEq)]
struct Shot {
    rows: Vec<Row>,
    wins: Option<[u32; 2]>,
    map: String,
    size: Vec2,
    game_look: bool,
}

#[allow(clippy::type_complexity)]
fn update(
    keys: Res<ButtonInput<KeyCode>>,
    (binds, mouse): (Option<Res<crate::console::Console>>, Option<Res<ButtonInput<MouseButton>>>),
    held: Res<super::console::HeldActions>,
    console: Option<Res<super::console::ConsoleUi>>,
    mut boards: Query<(&mut Visibility, Has<GameBoard>), Or<(With<Board>, With<GameBoard>)>>,
    roots: (Query<Entity, With<GameBoard>>, Query<(Entity, &Column)>),
    players: Query<
        (
            Entity,
            Option<&Name>,
            Option<&Team>,
            Option<&Score>,
            Has<Dead>,
            Has<LocalPlayer>,
            Has<DefuseKit>,
            Has<crate::bot::Bot>,
            Option<&crate::net::NetScore>,
            Option<&crate::net::NetCharacter>,
        ),
        (With<Intent>, Without<crate::objectives::hostages::Hostage>),
    >,
    bombs: Query<&Weapon, With<C4>>,
    windows: Query<&Window>,
    (fonts, hud, map, settings): (
        Res<super::fonts::UiFonts>,
        Option<Res<ActiveHud>>,
        Option<Res<crate::map::LoadedMapName>>,
        Option<Res<crate::net::NetSettings>>,
    ),
    (rounds, menu): (
        Option<Res<crate::rules::rounds::RoundState>>,
        Option<Res<super::game_menu::GameMenu>>,
    ),
    mut last: Local<Option<Shot>>,
    mut commands: Commands,
) {
    let typing = console.is_some_and(|c| c.open);
    let tab = match (&binds, &mouse) {
        (Some(b), Some(m)) => super::binds::pressed(&b.binds, &keys, m, "+showscores"),
        _ => false,
    };
    let show = shown((tab && !typing) || held.showscores, menu.as_deref());
    if !show && last.is_some() {
        // Hidden: its rows go (the next game draws its own; a map
        // unloading takes the last game's with it).
        for e in roots.0.iter().chain(roots.1.iter().map(|(e, _)| e)) {
            commands.entity(e).despawn_related::<Children>();
        }
    }
    let layout = hud
        .as_ref()
        .and_then(|h| h.0.menus.as_ref())
        .and_then(|m| Some((m, m.layouts.get(m.scoreboard.as_ref()?)?)));
    let game_look = layout.is_some();
    for (mut vis, is_game) in &mut boards {
        vis.set_if_neq(if show && is_game == game_look {
            Visibility::Inherited
        } else {
            Visibility::Hidden
        });
    }
    if !show {
        *last = None;
        return;
    }
    let my_team = players.iter().find(|p| p.5).and_then(|p| p.2.copied());
    let carriers: Vec<Entity> = bombs.iter().filter_map(|w| w.owner).collect();
    // Rows by team, most kills first, then fewest deaths.
    let mut rows: Vec<Row> = players
        .iter()
        .map(|(e, name, team, score, dead, local, kit, bot, net, character)| {
            let s = score.copied().unwrap_or_default();
            // A network game: the server's line (bots, the bomb, kits and
            // ping as it knows them) and names.
            let flag = |f: u8| net.is_some_and(|n| n.flags & f != 0);
            let bot = bot || flag(crate::net::score_flags::BOT);
            let carrier = carriers.contains(&e) || flag(crate::net::score_flags::BOMB);
            let kit = kit || flag(crate::net::score_flags::KIT);
            let name = super::shown_name(e, local, character, name, settings.as_deref());
            Row {
                column: if team.is_some_and(|t| t.0 == 1) { 1 } else { 2 },
                name,
                kills: s.kills,
                deaths: s.deaths,
                dead,
                local,
                bot,
                ping: net.map_or(0, |n| n.ping),
                status: status(
                    dead,
                    carrier,
                    kit,
                    team.is_some() && team.copied() == my_team,
                ),
            }
        })
        .collect();
    rows.sort_by(|a, b| {
        a.column
            .cmp(&b.column)
            .then(b.kills.cmp(&a.kills))
            .then(a.deaths.cmp(&b.deaths))
            .then(a.name.cmp(&b.name))
    });
    // Rounds won, while rounds are played.
    let wins = rounds
        .filter(|r| r.phase != crate::rules::rounds::Phase::Off)
        .map(|r| r.wins);
    let size = windows
        .iter()
        .next()
        .map_or(Vec2::new(640.0, 480.0), |w| Vec2::new(w.width(), w.height()));
    let map = map.map_or_else(String::new, |m| m.0.rsplit(':').next().unwrap_or(&m.0).to_string());
    let shot = Shot {
        rows,
        wins,
        map,
        size,
        game_look,
    };
    if last.as_ref() == Some(&shot) {
        return;
    }
    if let (Some((menus, layout)), Some(hud), Ok(root)) = (layout, hud.as_deref(), roots.0.single()) {
        commands.entity(root).despawn_related::<Children>();
        let painter = Painter::new(hud, &fonts, size.y);
        draw_game(&mut commands, root, &painter, menus, layout, &shot);
    } else {
        draw_plain(&mut commands, &roots.1, &fonts, &shot);
    }
    *last = Some(shot);
}

/// The game-look board: the layout's controls with its `%...%` labels
/// filled in, then a row per player on the first-row cells.
fn draw_game(
    commands: &mut Commands,
    root: Entity,
    painter: &Painter,
    menus: &GameMenus,
    layout: &crate::map::hud::UiLayout,
    shot: &Shot,
) {
    let alive = |team: u8| shot.rows.iter().filter(|r| r.column == team && !r.dead).count();
    let wins = |team: u8| shot.wins.map_or(0, |w| w[if team == 1 { 0 } else { 1 }]);
    let mut shown = |c: &UiControl| {
        let mut s = Shown::of(c);
        let name = c.name.to_lowercase();
        let fill = match c.text.trim().to_lowercase().as_str() {
            "%mapname%" => Some(shot.map.clone()),
            "%ct_teamname%" => Some(menus.string("Cstrike_ScoreBoard_CT", "COUNTER-TERRORISTS").to_string()),
            "%t_teamname%" => Some(menus.string("Cstrike_ScoreBoard_Ter", "TERRORISTS").to_string()),
            "%ct_alivecount%" => Some(alive(2).to_string()),
            "%t_alivecount%" => Some(alive(1).to_string()),
            "%ct_totalteamscore%" => Some(wins(2).to_string()),
            "%t_totalteamscore%" => Some(wins(1).to_string()),
            "%spectators%" => Some(menus.string("Cstrike_Scoreboard_NoSpectators", "No Spectators").to_string()),
            t if t.starts_with('%') => Some(String::new()),
            _ => None,
        };
        s.text = fill;
        // No time limit to show beside the clock (the map's time left).
        if name == "icon_clock" || name == "winconditionlabel" {
            s.visible = false;
        }
        s
    };
    painter.spawn(commands, root, shot.size, layout, VguiMenu::Scoreboard, &mut shown);

    let rects = layout.rects(Rect::from_corners(Vec2::ZERO, shot.size), painter.scale);
    let cell = |name: &str| {
        layout
            .controls
            .iter()
            .position(|c| c.name.eq_ignore_ascii_case(name))
            .map(|i| (&layout.controls[i], rects[i]))
    };
    for (team, prefix) in [(2u8, "CT"), (1u8, "T")] {
        let heading = |col: &str| cell(&format!("{prefix}Player{col}Label")).and_then(|(c, _)| c.fg);
        let [r, g, b, a] = heading("").unwrap_or(if team == 1 { [240, 90, 90, 255] } else { [150, 200, 255, 255] });
        let color = Color::srgba_u8(r, g, b, a);
        let font = painter.font(Some("ScoreboardBody_1"));
        let name_cell = cell(&format!("{prefix}PlayerName0"));
        let latency_cell = cell(&format!("{prefix}PlayerLatency0"));
        for (i, row) in shot.rows.iter().filter(|r| r.column == team).enumerate() {
            let dy = i as f32 * ROW_PITCH * painter.scale;
            let at = |r: Rect| Node {
                position_type: PositionType::Absolute,
                left: px(r.min.x),
                top: px(r.min.y + dy),
                width: px(r.width()),
                height: px(r.height()),
                display: Display::Flex,
                align_items: AlignItems::Center,
                ..default()
            };
            if row.local
                && let (Some((_, from)), Some((_, to))) = (name_cell, latency_cell)
                && let Some(image) = sprite(painter, "scoreboard_select")
            {
                let r = Rect::from_corners(from.min, Vec2::new(to.max.x, from.max.y));
                commands.spawn((at(r), image, ZIndex(5), ChildOf(root)));
            }
            let text_cells = [
                ("Name", row.name.clone(), JustifyContent::FlexStart),
                ("Score", row.kills.to_string(), JustifyContent::Center),
                ("Deaths", row.deaths.to_string(), JustifyContent::Center),
                ("Latency", latency(row.bot, row.ping), JustifyContent::Center),
            ];
            for (col, text, justify) in text_cells {
                let Some((_, r)) = cell(&format!("{prefix}Player{col}0")) else {
                    continue;
                };
                let node = Node {
                    justify_content: justify,
                    overflow: Overflow::clip(),
                    ..at(r)
                };
                commands
                    .spawn((node, ZIndex(10), ChildOf(root)))
                    .with_child((Text::new(text), font.clone(), TextColor(color)));
            }
            if let Some(icon) = row.status.icon()
                && let Some((_, r)) = cell(&format!("{prefix}PlayerStatus0"))
                && let Some(mut image) = sprite(painter, icon)
            {
                image.color = color;
                commands.spawn((at(r), image, ZIndex(10), ChildOf(root)));
            }
        }
    }
}

/// A HUD sprite as an image node.
fn sprite(painter: &Painter, name: &str) -> Option<ImageNode> {
    let s = painter.hud.sprites.get(name)?;
    let [x, y, w, h] = s.rect;
    Some(ImageNode {
        image: painter.images.get(&s.texture)?.clone(),
        rect: Some(Rect::new(x, y, x + w, y + h)),
        ..default()
    })
}

/// The plain board: a column per team.
fn draw_plain(commands: &mut Commands, columns: &Query<(Entity, &Column)>, fonts: &super::fonts::UiFonts, shot: &Shot) {
    // The client scheme's scoreboard fonts: team names, column titles,
    // player rows.
    let h = shot.size.y;
    let team_font = fonts.client("ScoreboardTeamName", h, 14.0);
    let column_font = fonts.client("ScoreboardColumns", h, 8.0);
    let body_font = fonts.client("ScoreboardBody_1", h, 10.0);
    for (column, Column(team)) in columns {
        commands.entity(column).despawn_related::<Children>();
        let (title, color) = if *team == 1 {
            ("Terrorists", T_RED)
        } else {
            ("Counter-Terrorists", CT_BLUE)
        };
        let members: Vec<_> = shot.rows.iter().filter(|r| r.column == *team).collect();
        row(
            commands,
            column,
            [
                match shot.wins {
                    Some(w) => format!("{title}  ({})   {}", members.len(), w[if *team == 1 { 0 } else { 1 }]),
                    None => format!("{title}  ({})", members.len()),
                },
                "Score".into(),
                "Deaths".into(),
                "Latency".into(),
                String::new(),
            ],
            color,
            false,
            (&team_font, &column_font),
        );
        for r in members {
            row(
                commands,
                column,
                [
                    r.name.clone(),
                    r.kills.to_string(),
                    r.deaths.to_string(),
                    latency(r.bot, r.ping),
                    r.status.word().into(),
                ],
                if r.dead { color.with_alpha(0.5) } else { color },
                r.local,
                (&body_font, &body_font),
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_icons_only_for_teammates() {
        assert_eq!(status(true, false, true, true), Status::Dead);
        assert_eq!(status(false, true, false, true), Status::Bomb);
        assert_eq!(status(false, true, false, false), Status::None);
        assert_eq!(status(false, false, true, true), Status::Defuser);
        assert_eq!(status(false, false, true, false), Status::None);
        assert_eq!(Status::Bomb.icon(), Some("scoreboard_bomb"));
        assert_eq!(Status::Dead.icon(), Some("scoreboard_dead"));
        assert_eq!(Status::Defuser.icon(), Some("scoreboard_defuser"));
        assert_eq!(Status::None.icon(), None);
    }

    /// Held Tab shows the board in a game only: not over the main menu,
    /// the Esc menu or the loading dialog, and its rows go when it hides.
    #[test]
    fn the_board_shows_only_in_a_game_and_drops_its_rows() {
        use super::super::{console::HeldActions, fonts::UiFonts, game_menu::GameMenu};
        let mut app = App::new();
        let mut menu = GameMenu::default();
        menu.in_game = true;
        menu.open = false;
        app.init_resource::<ButtonInput<KeyCode>>()
            .init_resource::<UiFonts>()
            .insert_resource(HeldActions {
                showscores: true,
                ..default()
            })
            .insert_resource(menu)
            .add_plugins(ScoreboardPlugin);
        for (name, team) in [("Alice", 1), ("Bob", 2)] {
            app.world_mut().spawn((Name::new(name), Team(team), Intent::default()));
        }
        let rows = |app: &mut App| -> usize {
            let mut q = app.world_mut().query_filtered::<&Children, With<Column>>();
            q.iter(app.world()).map(|c| c.len()).sum()
        };
        let visible = |app: &mut App| -> bool {
            let mut q = app.world_mut().query_filtered::<&Visibility, With<Board>>();
            q.iter(app.world()).any(|v| *v != Visibility::Hidden)
        };
        app.update();
        app.update();
        assert!(visible(&mut app), "held in a game");
        assert!(rows(&mut app) >= 4, "a title and a row per team");
        // A map loading from the main menu (the old game's players still
        // about): hidden, its rows gone.
        app.world_mut().resource_mut::<GameMenu>().loading = Some("de_dust2".into());
        app.update();
        app.update();
        assert!(!visible(&mut app));
        assert_eq!(rows(&mut app), 0, "the last game's rows are gone");
        // The main menu (out of a game) and the Esc menu: hidden too.
        let mut menu = app.world_mut().resource_mut::<GameMenu>();
        menu.loading = None;
        menu.in_game = false;
        menu.open = true;
        app.update();
        assert!(!visible(&mut app));
        let mut menu = app.world_mut().resource_mut::<GameMenu>();
        menu.in_game = true;
        app.update();
        assert!(!visible(&mut app), "under the Esc menu");
        app.world_mut().resource_mut::<GameMenu>().open = false;
        app.update();
        assert!(visible(&mut app), "back in the game");
    }

    #[test]
    fn bots_show_bot_for_latency() {
        assert_eq!(latency(true, 40), "BOT");
        assert_eq!(latency(false, 0), "0");
        assert_eq!(latency(false, 87), "87");
    }
}
