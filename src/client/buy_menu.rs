//! The buy menu (B, or `buymenu`): categories, then items with their
//! prices, picked with the number keys, as CS:S's menu can be driven from
//! the keyboard. Buying goes through the `buy` command, so the money, the
//! buy period and team limits apply; items you can't buy now are dimmed.

use bevy::prelude::*;
use bevy::window::CursorOptions;

use crate::{
    console::ConsoleAppExt,
    core::{LocalPlayer, Team},
    weapon::{
        Armor, Inventory, Weapon, WeaponRegistry,
        economy::{BuyWindow, Money, Prices},
    },
};

pub struct BuyMenuPlugin;

impl Plugin for BuyMenuPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<BuyMenu>()
            .init_resource::<BuyNotice>()
            .add_systems(Update, ((keys, draw).chain(), notice));
        app.console_command(
            "buymenu",
            "Open or close the buy menu (B); buymenu <n> opens category n (1 pistols, 2 rifles, 3 equipment).",
            |w, a| {
                let category = a.first().and_then(|n| n.parse::<usize>().ok()).filter(|n| (1..=CATEGORIES.len()).contains(n));
                let mut m = w.resource_mut::<BuyMenu>();
                m.open = category.is_some() || !m.open;
                m.category = category.map(|n| n - 1);
                Ok(None)
            },
        );
    }
}

/// Whether the menu is open, and which category is shown.
#[derive(Resource, Default, Debug, Clone, PartialEq)]
pub struct BuyMenu {
    pub open: bool,
    pub category: Option<usize>,
}

/// One line of the menu: what `buy` gets, its label and price.
#[derive(Clone, Debug, PartialEq)]
struct Item {
    buy: String,
    label: String,
    price: u32,
    team: Option<u8>,
}

const CATEGORIES: [&str; 3] = ["Pistols", "Rifles", "Equipment"];

/// A weapon's display name from its ID: `cs_source:weapon_m4a1` -> M4A1.
fn label(id: &str) -> String {
    id.rsplit(':').next().unwrap_or(id).trim_start_matches("weapon_").to_uppercase()
}

/// The items of a category, cheapest first.
fn items(category: usize, prices: &Prices, slots: &[(&'static str, u8)]) -> Vec<Item> {
    let mut out: Vec<Item> = match category {
        0 | 1 => slots
            .iter()
            .filter(|(_, slot)| *slot == if category == 0 { 1 } else { 0 })
            .filter_map(|(id, _)| {
                Some(Item {
                    buy: id.to_string(),
                    label: label(id),
                    price: *prices.weapons.get(id)?,
                    team: prices.team_only.get(id).copied(),
                })
            })
            .collect(),
        _ => vec![
            Item {
                buy: "vest".into(),
                label: "Kevlar".into(),
                price: prices.vest,
                team: None,
            },
            Item {
                buy: "vesthelm".into(),
                label: "Kevlar + Helmet".into(),
                price: prices.vest_helmet,
                team: None,
            },
        ],
    };
    out.sort_by_key(|i| (i.price, i.label.clone()));
    out
}

use crate::weapon::economy::weapon_slots;

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

#[allow(clippy::too_many_arguments)]
fn keys(
    keys: Res<ButtonInput<KeyCode>>,
    cursor: Single<&CursorOptions>,
    mut menu: ResMut<BuyMenu>,
    prices: Res<Prices>,
    registry: Res<WeaponRegistry>,
    mut slots: Local<Option<Vec<(&'static str, u8)>>>,
    mut commands: Commands,
) {
    if !super::input::cursor_grabbed(&cursor) {
        return;
    }
    if keys.just_pressed(KeyCode::KeyB) {
        menu.open = !menu.open;
        menu.category = None;
        return;
    }
    if !menu.open {
        return;
    }
    let Some(n) = DIGITS.iter().position(|k| keys.just_pressed(*k)) else {
        return;
    };
    let slots = slots.get_or_insert_with(|| weapon_slots(&registry));
    match (menu.category, n) {
        (_, 0) => *menu = BuyMenu::default(),
        (None, n) if n <= CATEGORIES.len() => menu.category = Some(n - 1),
        (Some(c), n) => {
            if let Some(item) = items(c, &prices, slots).get(n - 1) {
                // Bought, or why not, across the screen (as CS:S says
                // "You have insufficient funds!").
                let what = item.buy.clone();
                commands.queue(move |w: &mut World| {
                    let Some(player) = w.query_filtered::<Entity, With<LocalPlayer>>().iter(w).next() else {
                        return;
                    };
                    if let Err(why) = crate::weapon::economy::buy(w, player, &what) {
                        let now = w.resource::<Time>().elapsed_secs();
                        *w.resource_mut::<BuyNotice>() = BuyNotice(Some((why, now + 2.5)));
                    }
                });
                *menu = BuyMenu::default();
            }
        }
        _ => {}
    }
}

#[derive(Component)]
struct MenuText;

/// Why the last buy failed, until a time (seconds).
#[derive(Resource, Default)]
struct BuyNotice(Option<(String, f32)>);

#[derive(Component)]
struct NoticeText;

fn notice(
    mut notice: ResMut<BuyNotice>,
    time: Res<Time>,
    shown: Query<(Entity, &Text), With<NoticeText>>,
    windows: Query<&Window>,
    mut commands: Commands,
) {
    let now = time.elapsed_secs();
    if notice.0.as_ref().is_some_and(|(_, until)| now > *until) {
        notice.0 = None;
    }
    let want = notice.0.as_ref().map(|(m, _)| m.clone());
    let current = shown.iter().next();
    if current.map(|(_, t)| t.0.clone()) == want {
        return;
    }
    for (e, _) in &shown {
        commands.entity(e).despawn();
    }
    let Some(text) = want else { return };
    let scale = windows.iter().next().map_or(1.0, |w| w.height() / 480.0);
    let mut chars = text.chars();
    let text: String = chars.next().map(|c| c.to_uppercase().chain(chars).collect()).unwrap_or_default();
    commands.spawn((
        NoticeText,
        Text::new(format!("{text}!")),
        TextFont {
            font_size: FontSize::Px(12.0 * scale),
            ..default()
        },
        TextColor(Color::WHITE),
        TextShadow::default(),
        Node {
            position_type: PositionType::Absolute,
            top: Val::Percent(62.0),
            width: Val::Percent(100.0),
            justify_content: JustifyContent::Center,
            ..default()
        },
        TextLayout::justify(Justify::Center),
        GlobalZIndex(45),
    ));
}

#[allow(clippy::too_many_arguments)]
fn draw(
    menu: Res<BuyMenu>,
    prices: Res<Prices>,
    registry: Res<WeaponRegistry>,
    window: Res<BuyWindow>,
    player: Option<Single<(Option<&Money>, Option<&Team>, Option<&Inventory>, Option<&Armor>), With<LocalPlayer>>>,
    weapons: Query<&Weapon>,
    windows: Query<&Window>,
    mut text: Query<(Entity, &mut Text, &mut TextFont, &mut Node), With<MenuText>>,
    mut slots: Local<Option<Vec<(&'static str, u8)>>>,
    mut commands: Commands,
) {
    if !menu.open {
        for (e, ..) in &text {
            commands.entity(e).despawn();
        }
        return;
    }
    let (money, team, inv, armor) = player.map(|p| *p).unwrap_or((None, None, None, None));
    let held: Vec<&str> = inv
        .map(|i| i.weapons.iter().filter_map(|w| weapons.get(*w).ok()).map(|w| w.id).collect())
        .unwrap_or_default();
    let slots = slots.get_or_insert_with(|| weapon_slots(&registry));
    let mut lines = vec![match money {
        Some(m) => format!("BUY MENU   ${}", m.0),
        None => "BUY MENU   (free)".to_string(),
    }];
    if let Err(why) = &window.0 {
        lines.push(format!("({why})"));
    }
    lines.push(String::new());
    match menu.category {
        None => {
            for (i, c) in CATEGORIES.iter().enumerate() {
                lines.push(format!("{}  {c}", i + 1));
            }
        }
        Some(c) => {
            lines.push(CATEGORIES[c].to_uppercase());
            for (i, item) in items(c, &prices, slots).iter().enumerate() {
                let have = held.contains(&item.buy.as_str())
                    || (item.buy == "vest" && armor.is_some_and(|a| a.amount >= 1.0));
                let wrong_team = item.team.is_some_and(|t| team.is_some_and(|my| my.0 != t));
                let broke = money.is_some_and(|m| m.0 < item.price);
                let note = if wrong_team {
                    "  (other team)"
                } else if have {
                    "  (owned)"
                } else if broke {
                    "  (can't afford)"
                } else {
                    ""
                };
                lines.push(format!("{}  {}   ${}{note}", i + 1, item.label, item.price));
            }
        }
    }
    lines.push(String::new());
    lines.push("0  Close".into());
    let body = lines.join("\n");
    let scale = windows.iter().next().map_or(1.0, |w| w.height() / 480.0);
    match text.single_mut() {
        Ok((_, mut t, mut f, _)) => {
            if t.0 != body {
                t.0 = body;
            }
            f.font_size = FontSize::Px(11.0 * scale);
        }
        Err(_) => {
            commands.spawn((
                MenuText,
                Text::new(body),
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
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn menu_items_by_category_cheapest_first() {
        let mut p = Prices {
            vest: 650,
            vest_helmet: 1000,
            ..default()
        };
        p.weapons.insert("cs_source:weapon_usp", 500);
        p.weapons.insert("cs_source:weapon_glock", 400);
        p.weapons.insert("cs_source:weapon_ak47", 2500);
        p.team_only.insert("cs_source:weapon_ak47", 1);
        let slots = [
            ("cs_source:weapon_usp", 1),
            ("cs_source:weapon_glock", 1),
            ("cs_source:weapon_ak47", 0),
            ("cs_source:weapon_knife", 2),
        ];
        let pistols = items(0, &p, &slots);
        assert_eq!(
            pistols.iter().map(|i| i.label.as_str()).collect::<Vec<_>>(),
            ["GLOCK", "USP"]
        );
        let rifles = items(1, &p, &slots);
        assert_eq!(rifles[0].team, Some(1));
        assert_eq!(items(2, &p, &slots)[1].buy, "vesthelm");
    }
}
