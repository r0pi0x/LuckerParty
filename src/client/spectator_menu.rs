//! The spectator menu: duck while spectating (dead, or on the spectator
//! team) opens it, as CS:S's does (`resource/ui/bottomspectator.res`'s
//! player and view lists; `spec_menu [0|1]` too); duck again or Close shuts it. While it is open
//! the mouse is free: click a player to watch them, or a camera mode
//! (first person, chase camera, free look), or Join a team (the team
//! menu). Our look: two plain lists above the spectator bars, in the
//! client scheme's font and the game's mode names; CS:S draws them as
//! drop-down lists in the bottom bar.

use bevy::{prelude::*, window::CursorOptions};

use super::spectate::{SpecMode, SpecPhase, Spectator};
use super::vgui::VguiOpen;
use crate::{
    console::Console,
    core::{Health, Spectating, Team},
};

pub struct SpectatorMenuPlugin;

impl Plugin for SpectatorMenuPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<SpectatorMenu>()
            .add_systems(Update, (toggle, click, draw).chain().before(super::spectate::SpectateSet));
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
    }
}

/// Whether the spectator menu is open.
#[derive(Resource, Default, Debug, Clone, Copy, PartialEq)]
pub struct SpectatorMenu(pub bool);

/// What a menu button does.
#[derive(Component, Clone, Copy, Debug, PartialEq)]
enum Item {
    Watch(Entity),
    Mode(SpecMode),
    /// The team menu (and close this one).
    Teams,
    Close,
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
    let want = VguiOpen {
        spectator: menu.0,
        ..*open
    };
    open.set_if_neq(want);
}

fn click(
    buttons: Query<(&Interaction, &Item), Changed<Interaction>>,
    mut menu: ResMut<SpectatorMenu>,
    mut spec: ResMut<Spectator>,
    team_menu: Option<ResMut<super::team_menu::TeamMenu>>,
) {
    if !menu.0 {
        return;
    }
    let mut team_menu = team_menu;
    for (interaction, item) in &buttons {
        if *interaction != Interaction::Pressed {
            continue;
        }
        match *item {
            Item::Watch(e) => spec.pending.pick = Some(e),
            Item::Mode(m) => spec.pending.set_mode = Some(m),
            Item::Teams => {
                menu.0 = false;
                if let Some(t) = team_menu.as_mut() {
                    t.0 = true;
                }
            }
            Item::Close => menu.0 = false,
        }
    }
}

#[derive(Component)]
struct MenuRoot;

/// What the menu shows; redrawn when it changes.
#[derive(Clone, Debug, PartialEq)]
struct Shot {
    players: Vec<(Entity, String, Option<Team>)>,
    target: Option<Entity>,
    mode: SpecMode,
    height: f32,
    spectator: bool,
}

#[allow(clippy::too_many_arguments, clippy::type_complexity)]
fn draw(
    menu: Res<SpectatorMenu>,
    spec: Res<Spectator>,
    who: Query<(Option<&Name>, Option<&Team>, Option<&Health>)>,
    local: Option<Single<Has<Spectating>, With<crate::core::LocalPlayer>>>,
    roots: Query<Entity, With<MenuRoot>>,
    windows: Query<&Window>,
    fonts: Res<super::fonts::UiFonts>,
    hud: Option<Res<crate::map::hud::ActiveHud>>,
    mut last: Local<Option<Shot>>,
    mut commands: Commands,
) {
    if !menu.0 {
        for e in &roots {
            commands.entity(e).despawn();
        }
        *last = None;
        return;
    }
    let height = windows.iter().next().map_or(480.0, |w| w.height());
    let players = spec
        .watchable
        .iter()
        .filter_map(|e| {
            let (name, team, _) = who.get(*e).ok()?;
            Some((*e, name.map_or_else(|| "Player".to_string(), |n| n.as_str().to_string()), team.copied()))
        })
        .collect();
    let shot = Shot {
        players,
        target: spec.target,
        mode: spec.effective_mode(),
        height,
        spectator: local.is_some_and(|s| *s),
    };
    if last.as_ref() == Some(&shot) && !roots.is_empty() {
        return;
    }
    for e in &roots {
        commands.entity(e).despawn();
    }
    let menus = hud.as_ref().and_then(|h| h.0.menus.as_ref());
    let text = |token: &str, fallback: &str| {
        menus
            .as_ref()
            .map_or_else(|| fallback.to_string(), |m| m.string(token, fallback).to_string())
    };
    let scale = height / 480.0;
    let font = fonts.client("Default", height, 12.0);
    let amber = Color::srgb_u8(255, 176, 0);
    let root = commands
        .spawn((
            MenuRoot,
            Node {
                position_type: PositionType::Absolute,
                bottom: Val::Percent(14.0),
                width: Val::Percent(100.0),
                justify_content: JustifyContent::Center,
                column_gap: Val::Px(12.0 * scale),
                ..default()
            },
            GlobalZIndex(52),
        ))
        .id();
    let column = |commands: &mut Commands, title: String| {
        let c = commands
            .spawn((
                Node {
                    flex_direction: FlexDirection::Column,
                    padding: UiRect::all(Val::Px(6.0 * scale)),
                    row_gap: Val::Px(2.0 * scale),
                    min_width: Val::Px(140.0 * scale),
                    ..default()
                },
                BackgroundColor(Color::srgba(0.0, 0.0, 0.0, 0.75)),
                ChildOf(root),
            ))
            .id();
        commands.spawn((Text::new(title), font.clone(), TextColor(amber), ChildOf(c)));
        c
    };
    let button = |commands: &mut Commands, parent: Entity, label: String, item: Item, on: bool, color: Color| {
        commands
            .spawn((
                Button,
                item,
                Node {
                    padding: UiRect::axes(Val::Px(4.0 * scale), Val::Px(1.0 * scale)),
                    ..default()
                },
                BackgroundColor(if on {
                    Color::srgba(1.0, 1.0, 1.0, 0.18)
                } else {
                    Color::NONE
                }),
                ChildOf(parent),
            ))
            .with_child((Text::new(label), font.clone(), TextColor(color)));
    };
    let players = column(&mut commands, text("Cstrike_Spec_Players", "Players"));
    if shot.players.is_empty() {
        let none = text("Cstrike_TitlesTXT_Spec_NoPlayers", "No Players to Spectate");
        commands.spawn((Text::new(none), font.clone(), TextColor(Color::WHITE), ChildOf(players)));
    }
    for (e, name, team) in &shot.players {
        let color = match team.map(|t| t.0) {
            Some(1) => Color::srgb(1.0, 0.25, 0.25),
            Some(2) => Color::srgb(0.6, 0.8, 1.0),
            _ => Color::WHITE,
        };
        let on = shot.target == Some(*e) && shot.mode != SpecMode::Roaming;
        button(&mut commands, players, name.clone(), Item::Watch(*e), on, color);
    }
    let views = column(&mut commands, text("Cstrike_Spec_View", "Camera"));
    for (mode, token) in [
        (SpecMode::InEye, "Spec_Mode3"),
        (SpecMode::Chase, "Spec_Mode4"),
        (SpecMode::Roaming, "Spec_Mode5"),
    ] {
        let on = shot.mode == mode;
        button(&mut commands, views, text(token, mode.name()), Item::Mode(mode), on, Color::WHITE);
    }
    if shot.spectator {
        button(&mut commands, views, text("Cstrike_Join_Team", "Join a team"), Item::Teams, false, amber);
    }
    button(&mut commands, views, text("GameUI_Close", "Close"), Item::Close, false, amber);
    *last = Some(shot);
}
