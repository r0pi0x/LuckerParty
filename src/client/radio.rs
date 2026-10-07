//! The team radio: the three radio menus (Z, X, C, or `radio1`..`radio3`)
//! picked with the number keys, a console command per call (`coverme`,
//! `enemyspot`, ... from `map::radio::RadioCommands`), and what
//! `core::Radio` calls sound like to the local player: the call's sound
//! entry and a "Name (RADIO): text" chat line, for their own calls and
//! living teammates'.

use bevy::{prelude::*, window::CursorOptions};

use super::chat::ChatLine;
use crate::{
    console::{Command, Console, ConsoleAppExt},
    core::{Health, LocalPlayer, Radio, Team},
    map::{
        PlaySound,
        nav::NavMesh,
        radio::{RadioCommands, hears},
    },
};

pub struct RadioPlugin;

impl Plugin for RadioPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<RadioMenu>().add_message::<Radio>().add_systems(
            Update,
            (
                register_commands.run_if(resource_exists_and_changed::<RadioCommands>),
                (keys, draw).chain(),
                hear,
            ),
        );
        for n in 1..=3usize {
            let help = ["Radio commands", "Group radio commands", "Radio responses"][n - 1];
            app.console_command(
                &format!("radio{n}"),
                &format!("Open the {help} menu ({}).", ["Z", "X", "C"][n - 1]),
                move |w, _| {
                    open(w, Some(n - 1));
                    Ok(None)
                },
            );
        }
    }
}

/// Which radio menu is open (0-based).
#[derive(Resource, Default, Debug, Clone, Copy, PartialEq)]
pub struct RadioMenu(pub Option<usize>);

/// Open (or with the same menu open, close) a radio menu, closing the
/// other menus.
fn open(w: &mut World, menu: Option<usize>) {
    let mut m = w.resource_mut::<RadioMenu>();
    m.0 = if m.0 == menu { None } else { menu };
    if m.0.is_some() {
        close_others(w);
    }
}

/// Close the buy and team menus (a radio menu opened).
fn close_others(w: &mut World) {
    if let Some(mut b) = w.get_resource_mut::<super::buy_menu::BuyMenu>() {
        *b = default();
    }
    if let Some(mut t) = w.get_resource_mut::<super::team_menu::TeamMenu>() {
        t.0 = false;
    }
}

/// A console command per menu call, as the map's radio names them.
fn register_commands(radio: Res<RadioCommands>, mut console: ResMut<Console>) {
    for c in radio.commands.iter().filter(|c| c.menu.is_some()) {
        let name = c.command.clone();
        let label = c.variants.first().map(|v| v.1.clone()).unwrap_or_default();
        console.add_command(Command {
            name: name.clone(),
            help: format!("Radio: \"{label}\" to your team."),
            run: std::sync::Arc::new(move |w, _| {
                let player = w
                    .query_filtered::<Entity, With<LocalPlayer>>()
                    .iter(w)
                    .next()
                    .ok_or("no local player")?;
                w.write_message(Radio {
                    sender: player,
                    command: name.clone(),
                });
                Ok(None)
            }),
            complete: None,
        });
    }
}

const DIGITS: [KeyCode; 10] = [
    KeyCode::Digit0,
    KeyCode::Digit1,
    KeyCode::Digit2,
    KeyCode::Digit3,
    KeyCode::Digit4,
    KeyCode::Digit5,
    KeyCode::Digit6,
    KeyCode::Digit7,
    KeyCode::Digit8,
    KeyCode::Digit9,
];

fn keys(
    keys: Res<ButtonInput<KeyCode>>,
    cursor: Single<&CursorOptions>,
    menu: Res<RadioMenu>,
    radio: Option<Res<RadioCommands>>,
    mut console: ResMut<Console>,
    mut commands: Commands,
) {
    if !super::input::cursor_grabbed(&cursor) {
        return;
    }
    for (i, k) in [KeyCode::KeyZ, KeyCode::KeyX, KeyCode::KeyC].into_iter().enumerate() {
        if keys.just_pressed(k) {
            commands.queue(move |w: &mut World| open(w, Some(i)));
            return;
        }
    }
    let Some(open_menu) = menu.0 else { return };
    let Some(n) = DIGITS.iter().position(|k| keys.just_pressed(*k)) else {
        return;
    };
    if n == 0 {
        commands.queue(|w: &mut World| w.resource_mut::<RadioMenu>().0 = None);
        return;
    }
    if let Some(c) = radio.as_ref().and_then(|r| r.pick(open_menu, n)) {
        console.submit(c.command.clone());
        commands.queue(|w: &mut World| w.resource_mut::<RadioMenu>().0 = None);
    }
}

#[derive(Component)]
struct MenuText;

/// The open menu, as CS:S's HUD menu shows it: the title, the numbered
/// calls, "0. Exit".
fn menu_lines(radio: &RadioCommands, menu: usize) -> Option<(String, Vec<String>)> {
    let m = radio.menus.get(menu)?;
    let mut items: Vec<String> = if m.items.is_empty() {
        // No menu text: the calls' own words.
        radio
            .commands
            .iter()
            .filter_map(|c| Some((c.menu.filter(|x| x.0 == menu)?.1, c.variants.first()?.1.clone())))
            .map(|(n, l)| format!("{n}. \"{l}\""))
            .collect()
    } else {
        m.items.iter().map(|(n, l)| format!("{n}. \"{l}\"")).collect()
    };
    items.push(String::new());
    items.push("0. Exit".into());
    let title = if m.title.is_empty() {
        ["Radio Commands", "Group Radio Commands", "Radio Responses/Reports"][menu.min(2)].to_string()
    } else {
        m.title.clone()
    };
    Some((title, items))
}

fn draw(
    menu: Res<RadioMenu>,
    radio: Option<Res<RadioCommands>>,
    hud: Option<Res<crate::map::hud::ActiveHud>>,
    shown: Query<Entity, With<MenuText>>,
    windows: Query<&Window>,
    mut built: Local<Option<(Option<usize>, f32)>>,
    mut commands: Commands,
) {
    let h = windows.iter().next().map_or(480.0, |w| w.height());
    let key = (menu.0.filter(|_| radio.is_some()), h);
    if *built == Some(key) {
        return;
    }
    *built = Some(key);
    for e in &shown {
        commands.entity(e).despawn();
    }
    let (Some(open), Some(radio)) = (menu.0, radio) else {
        return;
    };
    let Some((title, items)) = menu_lines(&radio, open) else {
        return;
    };
    let scale = h / 480.0;
    let named = |n: &str, d: Color| hud.as_ref().and_then(|h| h.0.color(n)).unwrap_or(d);
    let title_color = named("MenuColor", Color::srgb_u8(233, 208, 173));
    let item_color = named("ItemColor", Color::srgba_u8(255, 167, 42, 200));
    let bg = named("MenuBoxBg", Color::srgba_u8(0, 0, 0, 100));
    // HudMenu's "Default" font: 12 at 480 lines.
    let size = FontSize::Px(12.0 * scale * 0.85);
    let root = commands
        .spawn((
            MenuText,
            Text::default(),
            TextFont {
                font_size: size,
                ..default()
            },
            BackgroundColor(bg),
            Node {
                position_type: PositionType::Absolute,
                left: px(8.0 * scale),
                top: Val::Percent(40.0),
                padding: UiRect::all(px(6.0 * scale)),
                border_radius: BorderRadius::all(px(4.0 * scale)),
                ..default()
            },
            GlobalZIndex(50),
        ))
        .id();
    commands.spawn((
        TextSpan::new(format!("{title}\n\n")),
        TextFont {
            font_size: size,
            ..default()
        },
        TextColor(title_color),
        ChildOf(root),
    ));
    commands.spawn((
        TextSpan::new(items.join("\n")),
        TextFont {
            font_size: size,
            ..default()
        },
        TextColor(item_color),
        ChildOf(root),
    ));
}

/// Play and print the radio calls the local player hears.
#[allow(clippy::type_complexity)]
fn hear(
    mut calls: MessageReader<Radio>,
    radio: Option<Res<RadioCommands>>,
    local: Query<(Entity, Option<&Team>), With<LocalPlayer>>,
    who: Query<(
        Option<&Name>,
        Option<&Team>,
        Option<&Health>,
        Has<LocalPlayer>,
        Option<&GlobalTransform>,
    )>,
    nav: Option<Res<NavMesh>>,
    mut play: MessageWriter<PlaySound>,
    mut chat: MessageWriter<ChatLine>,
    mut rng: Local<u64>,
) {
    let Some(radio) = radio else {
        calls.clear();
        return;
    };
    let Some((me, my_team)) = local.iter().next() else {
        calls.clear();
        return;
    };
    for call in calls.read() {
        let Some(c) = radio.get(&call.command) else { continue };
        let Ok((name, team, health, is_local, at)) = who.get(call.sender) else {
            continue;
        };
        let alive = health.is_none_or(|h| h.current > 0.0);
        if !hears(me, my_team.map(|t| t.0), call.sender, team.map(|t| t.0), alive) || c.variants.is_empty() {
            continue;
        }
        // xorshift: pick a variant.
        *rng = rng.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut x = *rng;
        x ^= x >> 30;
        x = x.wrapping_mul(0xBF58_476D_1CE4_E5B9);
        x ^= x >> 27;
        let (sound, text) = &c.variants[(x % c.variants.len() as u64) as usize];
        play.write(PlaySound::ui(sound.clone()));
        let sender = if is_local {
            "Player".to_string()
        } else {
            name.map_or_else(|| format!("{}", call.sender), |n| n.to_string())
        };
        let place = nav.as_ref().zip(at).and_then(|(nav, at)| {
            let area = nav.area_at(at.translation())?;
            nav.places.get(nav.areas[area].place?).cloned()
        });
        chat.write(ChatLine(radio.line(&sender, text, place.as_deref()), team.copied()));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::map::radio::{RadioCommand, RadioMenu as Menu};

    #[test]
    fn menu_text_from_the_game_or_the_calls() {
        let mut r = RadioCommands {
            commands: vec![RadioCommand {
                command: "coverme".into(),
                menu: Some((0, 1)),
                variants: vec![("Radio.CoverMe".into(), "Cover Me!".into())],
            }],
            menus: vec![Menu::default()],
            ..default()
        };
        let (title, items) = menu_lines(&r, 0).unwrap();
        assert_eq!(title, "Radio Commands");
        assert_eq!(items, ["1. \"Cover Me!\"", "", "0. Exit"]);
        r.menus[0] = Menu {
            title: "Radio Commands".into(),
            items: vec![(1, "Cover Me".into())],
        };
        assert_eq!(menu_lines(&r, 0).unwrap().1[0], "1. \"Cover Me\"");
        assert!(menu_lines(&r, 1).is_none());
    }
}
