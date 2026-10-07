//! The team menu (M, or `chooseteam`): 1 terrorists, 2 counter-terrorists,
//! 5 auto-assign (the smaller team), 0 closes; joins through `jointeam`.

use bevy::{prelude::*, window::CursorOptions};

use crate::{
    console::{Console, ConsoleAppExt},
    core::{Intent, Team},
};

pub struct TeamMenuPlugin;

impl Plugin for TeamMenuPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<TeamMenu>().add_systems(Update, (keys, draw).chain());
        app.console_command("chooseteam", "Open the team menu (M).", |w, _| {
            w.resource_mut::<TeamMenu>().0 = true;
            Ok(None)
        });
    }
}

/// Whether the team menu is open.
#[derive(Resource, Default, Debug, Clone, Copy, PartialEq)]
pub struct TeamMenu(pub bool);

/// Auto-assign: the team with fewer players (terrorists on a tie), as
/// Source's `jointeam` number.
pub(super) fn auto_team(t: usize, ct: usize) -> u8 {
    if ct < t { 3 } else { 2 }
}

fn keys(
    keys: Res<ButtonInput<KeyCode>>,
    cursor: Single<&CursorOptions>,
    mut menu: ResMut<TeamMenu>,
    mut console: ResMut<Console>,
    teams: Query<&Team, With<Intent>>,
    mut others: (ResMut<super::buy_menu::BuyMenu>, ResMut<super::radio::RadioMenu>),
) {
    if !super::input::cursor_grabbed(&cursor) {
        return;
    }
    if keys.just_pressed(KeyCode::KeyM) {
        menu.0 = !menu.0;
        if menu.0 {
            // Menus close each other.
            *others.0 = default();
            others.1.0 = None;
        }
        return;
    }
    if !menu.0 {
        return;
    }
    let pick = if keys.just_pressed(KeyCode::Digit1) {
        Some(2)
    } else if keys.just_pressed(KeyCode::Digit2) {
        Some(3)
    } else if keys.just_pressed(KeyCode::Digit5) {
        let count = |n: u8| teams.iter().filter(|t| t.0 == n).count();
        Some(auto_team(count(1), count(2)))
    } else {
        None
    };
    if let Some(team) = pick {
        console.submit(format!("jointeam {team}"));
        menu.0 = false;
    } else if keys.just_pressed(KeyCode::Digit0) {
        menu.0 = false;
    }
}

#[derive(Component)]
struct MenuText;

fn draw(
    menu: Res<TeamMenu>,
    shown: Query<Entity, With<MenuText>>,
    windows: Query<&Window>,
    mut commands: Commands,
) {
    if !menu.is_changed() {
        return;
    }
    for e in &shown {
        commands.entity(e).despawn();
    }
    if !menu.0 {
        return;
    }
    let scale = windows.iter().next().map_or(1.0, |w| w.height() / 480.0);
    commands.spawn((
        MenuText,
        Text::new("SELECT A TEAM\n\n1  Terrorists\n2  Counter-Terrorists\n\n5  Auto-assign\n\n0  Close"),
        TextFont {
            font_size: FontSize::Px(11.0 * scale),
            ..default()
        },
        TextColor(Color::srgb_u8(255, 176, 0)),
        BackgroundColor(Color::srgba(0.0, 0.0, 0.0, 0.6)),
        Node {
            position_type: PositionType::Absolute,
            left: Val::Percent(4.0),
            top: Val::Percent(42.0),
            padding: UiRect::all(Val::Px(10.0 * scale)),
            ..default()
        },
        GlobalZIndex(50),
    ));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn auto_assign_picks_the_smaller_team() {
        assert_eq!(auto_team(2, 1), 3);
        assert_eq!(auto_team(1, 2), 2);
        assert_eq!(auto_team(1, 1), 2);
    }
}
