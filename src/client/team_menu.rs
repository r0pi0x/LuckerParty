//! The team menu (M, or `chooseteam`): terrorists, counter-terrorists or
//! auto-assign (the smaller team), joined through `jointeam`. With the
//! game's own layout (`map::hud::GameMenus::team`, CS:S's
//! `resource/ui/teammenu.res`) it is drawn as the game draws it
//! (`super::vgui`): the team buttons with their number keys, clickable
//! with the mouse (free while it is open), beside the map's description.
//! Spectating has no team here yet, so its button is greyed, and the VIP
//! button (VIP maps only) is hidden. Without the layout, a plain list: 1
//! terrorists, 2 counter-terrorists, 5 auto-assign, 0 closes.

use bevy::{prelude::*, window::CursorOptions};

use super::fonts::UiFonts;
use super::vgui::{Painter, Shown, VguiButton, VguiMenu, VguiOpen};
use crate::{
    console::{Console, ConsoleAppExt},
    core::{Intent, Team},
    map::hud::{ActiveHud, GameMenus, UiControl, UiKind, UiLayout},
};

pub struct TeamMenuPlugin;

impl Plugin for TeamMenuPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<TeamMenu>()
            .add_systems(Update, (keys, pointer, draw, draw_vgui).chain());
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

/// The game's team menu layout, when it has one.
fn layout(menus: &GameMenus) -> Option<&UiLayout> {
    menus.layouts.get(menus.team.as_ref()?)
}

/// What we make of the game's controls: the map's description in its
/// `MapInfo`; no VIP button (VIP maps only), no spectating (no spectator
/// team yet).
fn shown(c: &UiControl, map_info: Option<&str>) -> Shown {
    let mut s = Shown::of(c);
    if c.kind == UiKind::RichText && c.name.eq_ignore_ascii_case("MapInfo") {
        s.text = map_info.map(str::to_string);
    }
    if c.name.eq_ignore_ascii_case("vipbutton") {
        s.visible = false;
    }
    if c.command.as_deref().is_some_and(|cmd| cmd.trim() == "jointeam 1") {
        s.enabled = false;
    }
    s
}

/// The console line a button's command becomes (None: just close):
/// `jointeam 0` is auto-assign, picked here from the team sizes.
fn line(command: &str, count: impl Fn(u8) -> usize) -> Option<String> {
    let command = command.trim();
    if command.eq_ignore_ascii_case("vguicancel") {
        None
    } else if command == "jointeam 0" {
        Some(format!("jointeam {}", auto_team(count(1), count(2))))
    } else {
        Some(command.to_string())
    }
}

#[allow(clippy::too_many_arguments)]
fn keys(
    (keys, mouse): (Res<ButtonInput<KeyCode>>, Res<ButtonInput<MouseButton>>),
    cursor: Single<&CursorOptions>,
    (open, ui, chat, game_menu): (
        Res<VguiOpen>,
        Option<Res<super::console::ConsoleUi>>,
        Option<Res<super::chat::ChatInput>>,
        Option<Res<super::game_menu::GameMenu>>,
    ),
    mut menu: ResMut<TeamMenu>,
    mut console: ResMut<Console>,
    teams: Query<&Team, With<Intent>>,
    hud: Option<Res<ActiveHud>>,
    mut others: (ResMut<super::buy_menu::BuyMenu>, ResMut<super::radio::RadioMenu>),
) {
    if !super::vgui::keys_live(&cursor, &open, ui.as_deref(), chat.as_deref(), game_menu.as_deref()) {
        return;
    }
    if super::binds::just_pressed(&console.binds, &keys, &mouse, "chooseteam") {
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
    let count = |n: u8| teams.iter().filter(|t| t.0 == n).count();
    // The game's own layout: the button with that hotkey.
    if let Some(menus) = hud.as_ref().and_then(|h| h.0.menus.as_ref())
        && let Some(page) = layout(menus)
    {
        let Some(key) = super::vgui::digit(&keys) else {
            return;
        };
        let info = menus.map_info.as_deref();
        if let Some(command) = super::vgui::hotkey_button(page, key, &mut |c| shown(c, info)).and_then(|c| c.command.clone()) {
            if let Some(l) = line(&command, count) {
                console.submit(l);
            }
            menu.0 = false;
        }
        return;
    }
    let pick = if keys.just_pressed(KeyCode::Digit1) {
        Some(2)
    } else if keys.just_pressed(KeyCode::Digit2) {
        Some(3)
    } else if keys.just_pressed(KeyCode::Digit5) {
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

/// Clicking one of the game-look menu's buttons presses it.
fn pointer(
    buttons: Query<(&Interaction, &VguiButton), Changed<Interaction>>,
    mut menu: ResMut<TeamMenu>,
    mut console: ResMut<Console>,
    teams: Query<&Team, With<Intent>>,
) {
    let count = |n: u8| teams.iter().filter(|t| t.0 == n).count();
    for (interaction, b) in &buttons {
        if b.menu != VguiMenu::Team || !menu.0 || *interaction != Interaction::Pressed || !b.enabled {
            continue;
        }
        if let Some(command) = &b.command {
            if let Some(l) = line(command, count) {
                console.submit(l);
            }
            menu.0 = false;
            return;
        }
    }
}

#[derive(Component)]
struct MenuText;

fn draw(
    menu: Res<TeamMenu>,
    shown: Query<Entity, With<MenuText>>,
    windows: Query<&Window>,
    hud: Option<Res<ActiveHud>>,
    fonts: Res<UiFonts>,
    mut commands: Commands,
) {
    if !menu.is_changed() {
        return;
    }
    for e in &shown {
        commands.entity(e).despawn();
    }
    // The game's own look draws it instead (`draw_vgui`).
    let game_look = hud.as_ref().and_then(|h| h.0.menus.as_ref()).and_then(layout).is_some();
    if !menu.0 || game_look {
        return;
    }
    let h = windows.iter().next().map_or(480.0, |w| w.height());
    let scale = h / 480.0;
    commands.spawn((
        MenuText,
        Text::new("SELECT A TEAM\n\n1  Terrorists\n2  Counter-Terrorists\n\n5  Auto-assign\n\n0  Close"),
        // As a HudMenu: the client scheme's Default.
        fonts.client("Default", h, 12.0),
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

/// Everything the game-look menu spawned.
#[derive(Component)]
struct VguiRoot;

#[allow(clippy::too_many_arguments)]
fn draw_vgui(
    menu: Res<TeamMenu>,
    hud: Option<Res<ActiveHud>>,
    fonts: Res<UiFonts>,
    windows: Query<&Window>,
    roots: Query<Entity, With<VguiRoot>>,
    mut open: ResMut<VguiOpen>,
    mut drawn: Local<Option<Vec2>>,
    mut commands: Commands,
) {
    let menus = hud.as_ref().and_then(|h| h.0.menus.as_ref()).filter(|_| menu.0);
    let page = menus.and_then(layout);
    let (Some(hud), Some(menus), Some(page), Some(window)) = (hud.as_ref(), menus, page, windows.iter().next()) else {
        for e in &roots {
            commands.entity(e).despawn();
        }
        *drawn = None;
        let closed = VguiOpen { team: false, ..*open };
        open.set_if_neq(closed);
        return;
    };
    let shown_now = VguiOpen { team: true, ..*open };
    open.set_if_neq(shown_now);
    let size = Vec2::new(window.width(), window.height());
    if *drawn == Some(size) && !roots.is_empty() && !hud.is_changed() {
        return;
    }
    *drawn = Some(size);
    for e in &roots {
        commands.entity(e).despawn();
    }
    let painter = Painter::new(hud, &fonts, size.y);
    let root = commands
        .spawn((
            VguiRoot,
            Node {
                position_type: PositionType::Absolute,
                width: Val::Percent(100.0),
                height: Val::Percent(100.0),
                ..default()
            },
            BackgroundColor(painter.color("Frame.BgColor", [0, 0, 0, 196])),
            GlobalZIndex(50),
        ))
        .id();
    let a = super::vgui::area(size.x, size.y);
    let area = commands
        .spawn((
            Node {
                position_type: PositionType::Absolute,
                left: px(a.min.x),
                top: px(a.min.y),
                width: px(a.width()),
                height: px(a.height()),
                ..default()
            },
            ChildOf(root),
        ))
        .id();
    let info = menus.map_info.as_deref();
    painter.spawn(&mut commands, area, a.size(), page, VguiMenu::Team, &mut |c| shown(c, info));
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

    /// Lines `jointeam` got.
    #[derive(Resource, Default)]
    struct Joined(Vec<String>);

    fn app(game_look: bool) -> App {
        use crate::map::hud::GameHud;
        use std::collections::HashMap;
        let mut app = App::new();
        app.add_plugins(crate::console::ConsolePlugin)
            .init_resource::<ButtonInput<KeyCode>>()
            .init_resource::<TeamMenu>()
            .init_resource::<VguiOpen>()
            .init_resource::<Joined>()
            .init_resource::<super::super::buy_menu::BuyMenu>()
            .init_resource::<super::super::radio::RadioMenu>()
            .add_systems(Update, keys)
            .console_command("jointeam", "", |w, a| {
                w.resource_mut::<Joined>().0.push(a.join(" "));
                Ok(None)
            });
        super::super::binds::test_binds(&mut app);
        if game_look {
            let button = |name: &str, key: char, command: &str| {
                let mut c = UiControl::new(name, UiKind::Button, 76.0, 116.0, 148.0, 20.0);
                c.hotkey = Some(key);
                c.command = Some(command.into());
                c
            };
            let page = UiLayout {
                controls: vec![
                    button("terbutton", '1', "jointeam 2"),
                    button("ctbutton", '2', "jointeam 3"),
                    button("autobutton", '5', "jointeam 0"),
                    button("specbutton", '6', "jointeam 1"),
                    button("CancelButton", '0', "vguicancel"),
                ],
            };
            let menus = GameMenus {
                layouts: HashMap::from([("resource/ui/teammenu.res".to_string(), page)]),
                team: Some("resource/ui/teammenu.res".into()),
                ..default()
            };
            let hud = GameHud {
                menus: Some(menus),
                ..default()
            };
            app.insert_resource(ActiveHud(std::sync::Arc::new(hud), HashMap::new()));
        }
        app.world_mut().spawn(CursorOptions {
            grab_mode: bevy::window::CursorGrabMode::Locked,
            ..default()
        });
        // Two terrorists, no CTs: auto-assign picks the CTs.
        app.world_mut().spawn_batch([(Intent::default(), Team(1)), (Intent::default(), Team(1))]);
        app
    }

    fn press(app: &mut App, key: KeyCode) {
        app.world_mut().resource_mut::<ButtonInput<KeyCode>>().press(key);
        app.update();
        let mut keys = app.world_mut().resource_mut::<ButtonInput<KeyCode>>();
        keys.release(key);
        keys.clear();
        app.update(); // the console runs the line
    }

    #[test]
    fn both_looks_join_with_number_keys() {
        for game_look in [true, false] {
            let mut app = app(game_look);
            press(&mut app, KeyCode::KeyM);
            assert!(app.world().resource::<TeamMenu>().0);
            if game_look {
                // The game's menu frees the mouse; its keys still work, and
                // spectating (no team for it yet) does nothing.
                app.world_mut().resource_mut::<VguiOpen>().team = true;
                let mut q = app.world_mut().query::<&mut CursorOptions>();
                super::super::input::release_cursor(&mut q.single_mut(app.world_mut()).unwrap());
                press(&mut app, KeyCode::Digit6);
                assert!(app.world().resource::<TeamMenu>().0);
            }
            press(&mut app, KeyCode::Digit5);
            assert!(!app.world().resource::<TeamMenu>().0);
            if game_look {
                app.world_mut().resource_mut::<VguiOpen>().team = false;
                let mut q = app.world_mut().query::<&mut CursorOptions>();
                super::super::input::capture_cursor(&mut q.single_mut(app.world_mut()).unwrap());
            }
            press(&mut app, KeyCode::KeyM);
            press(&mut app, KeyCode::Digit1);
            assert_eq!(app.world().resource::<Joined>().0, ["3", "2"], "game look: {game_look}");
        }
    }

    #[test]
    fn game_buttons_become_console_lines() {
        let count = |n: u8| if n == 1 { 3 } else { 1 };
        assert_eq!(line("jointeam 2", count).as_deref(), Some("jointeam 2"));
        assert_eq!(line("jointeam 0", count).as_deref(), Some("jointeam 3"), "fewer CTs");
        assert_eq!(line("vguicancel", count), None);
        let mut spec = UiControl::new("specbutton", UiKind::Button, 0.0, 0.0, 1.0, 1.0);
        spec.command = Some("jointeam 1".into());
        assert!(!shown(&spec, None).enabled);
        assert!(!shown(&UiControl::new("vipbutton", UiKind::Button, 0.0, 0.0, 1.0, 1.0), None).visible);
        let info = UiControl::new("MapInfo", UiKind::RichText, 0.0, 0.0, 1.0, 1.0);
        assert_eq!(shown(&info, Some("Dust II")).text.as_deref(), Some("Dust II"));
    }
}
