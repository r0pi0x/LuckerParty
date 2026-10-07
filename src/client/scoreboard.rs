//! The scoreboard, CS:S style: held with Tab (or `+showscores`), two team
//! columns (Terrorists, Counter-Terrorists, then anyone else) listing each
//! player's name, kills, deaths, latency (0 until networking) and whether
//! they're dead, the local player highlighted.

use bevy::prelude::*;

use crate::{
    core::{Intent, LocalPlayer, Team},
    rules::{Dead, Score},
};

pub struct ScoreboardPlugin;

impl Plugin for ScoreboardPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, spawn).add_systems(Update, update);
    }
}

#[derive(Component)]
struct Board;

#[derive(Component)]
struct Column(u8);

const T_RED: Color = Color::srgb(1.0, 0.25, 0.25);
const CT_BLUE: Color = Color::srgb(0.6, 0.8, 1.0);

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
}

/// One row: name, kills, deaths, latency, status.
fn row(commands: &mut Commands, parent: Entity, cells: [String; 5], color: Color, highlight: bool, size: f32) {
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
            TextFont {
                font_size: FontSize::Px(size),
                ..default()
            },
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

#[allow(clippy::type_complexity)]
fn update(
    keys: Res<ButtonInput<KeyCode>>,
    held: Res<super::console::HeldActions>,
    console: Option<Res<super::console::ConsoleUi>>,
    mut board: Single<&mut Visibility, With<Board>>,
    columns: Query<(Entity, &Column)>,
    players: Query<
        (
            Entity,
            Option<&Name>,
            Option<&Team>,
            Option<&Score>,
            Has<Dead>,
            Has<LocalPlayer>,
        ),
        With<Intent>,
    >,
    windows: Query<&Window>,
    rounds: Option<Res<crate::rules::rounds::RoundState>>,
    mut last: Local<Option<(Vec<(u8, String, u32, u32, bool, bool)>, Option<[u32; 2]>)>>,
    mut commands: Commands,
) {
    let typing = console.is_some_and(|c| c.open);
    let show = (keys.pressed(KeyCode::Tab) && !typing) || held.showscores;
    **board = if show {
        Visibility::Inherited
    } else {
        Visibility::Hidden
    };
    if !show {
        *last = None;
        return;
    }
    // Rows by team, most kills first, then fewest deaths.
    let mut rows: Vec<(u8, String, u32, u32, bool, bool)> = players
        .iter()
        .map(|(e, name, team, score, dead, local)| {
            let s = score.copied().unwrap_or_default();
            let name = if local {
                "Player".to_string()
            } else {
                name.map_or_else(|| format!("{e}"), |n| n.to_string())
            };
            // Team 1 terrorists; everyone else in the CT column.
            let column = if team.is_some_and(|t| t.0 == 1) { 1 } else { 2 };
            (column, name, s.kills, s.deaths, dead, local)
        })
        .collect();
    rows.sort_by(|a, b| {
        a.0.cmp(&b.0)
            .then(b.2.cmp(&a.2))
            .then(a.3.cmp(&b.3))
            .then(a.1.cmp(&b.1))
    });
    // Rounds won, while rounds are played.
    let wins = rounds
        .filter(|r| r.phase != crate::rules::rounds::Phase::Off)
        .map(|r| r.wins);
    let key = (rows, wins);
    if last.as_ref() == Some(&key) {
        return;
    }
    let rows = &key.0;
    let size = windows
        .iter()
        .next()
        .map_or(16.0, |w| (w.height() / 60.0).clamp(12.0, 26.0));
    for (column, Column(team)) in &columns {
        commands.entity(column).despawn_related::<Children>();
        let (title, color) = if *team == 1 {
            ("Terrorists", T_RED)
        } else {
            ("Counter-Terrorists", CT_BLUE)
        };
        let members: Vec<_> = rows.iter().filter(|r| r.0 == *team).collect();
        row(
            &mut commands,
            column,
            [
                match wins {
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
            size * 1.15,
        );
        for (_, name, kills, deaths, dead, local) in members {
            let status = if *dead { "DEAD" } else { "" };
            row(
                &mut commands,
                column,
                // Latency: everyone is local until networking (0 ms).
                [name.clone(), kills.to_string(), deaths.to_string(), "0".into(), status.into()],
                if *dead { color.with_alpha(0.5) } else { color },
                *local,
                size,
            );
        }
    }
    *last = Some(key);
}
