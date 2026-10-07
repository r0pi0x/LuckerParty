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
        app.init_resource::<BuyMenu>().add_systems(Update, (keys, draw).chain());
        app.console_command(
            "buymenu",
            "Open or close the buy menu (B); buymenu <n> opens category n (1 pistols, 2 rifles, 3 equipment).",
            |w, a| {
                let category = a.first().and_then(|n| n.parse::<usize>().ok()).filter(|n| (1..=CATEGORIES.len()).contains(n));
                let mut m = w.resource_mut::<BuyMenu>();
                m.open = category.is_some() || !m.open;
                m.category = category.map(|n| n - 1);
                if m.open {
                    close_others(w);
                }
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
    let name = id.rsplit(':').next().unwrap_or(id).trim_start_matches("weapon_").to_uppercase();
    match name.strip_suffix("GRENADE") {
        Some(kind) if !kind.is_empty() => format!("{kind} GRENADE"),
        _ => name,
    }
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
        _ => {
            let mut out = vec![
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
            ];
            // Grenades (slot 3) are equipment.
            out.extend(slots.iter().filter(|(_, slot)| *slot == 3).filter_map(|(id, _)| {
                Some(Item {
                    buy: id.to_string(),
                    label: label(id),
                    price: *prices.weapons.get(id)?,
                    team: prices.team_only.get(id).copied(),
                })
            }));
            out
        }
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

/// Close the team and radio menus (the buy menu opened).
fn close_others(w: &mut World) {
    if let Some(mut t) = w.get_resource_mut::<super::team_menu::TeamMenu>() {
        t.0 = false;
    }
    if let Some(mut r) = w.get_resource_mut::<super::radio::RadioMenu>() {
        r.0 = None;
    }
}

/// Why the local player can't buy now (out of the buy time or a buy
/// zone), as the hint CS:S shows when the buy key is pressed.
fn cannot_buy(w: &mut World) -> Option<String> {
    let player = w.query_filtered::<Entity, With<LocalPlayer>>().iter(w).next()?;
    // Without money, buying is free and anywhere.
    w.get::<Money>(player)?;
    if let Err(why) = &w.resource::<BuyWindow>().0 {
        return Some(why.clone());
    }
    (!crate::weapon::economy::in_buy_zone(w, player)).then(|| crate::weapon::economy::NOT_IN_BUY_ZONE.to_string())
}

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
        if menu.open {
            *menu = BuyMenu::default();
        } else {
            commands.queue(|w: &mut World| {
                if let Some(why) = cannot_buy(w) {
                    w.write_message(super::chat::Hint(why));
                    return;
                }
                *w.resource_mut::<BuyMenu>() = BuyMenu {
                    open: true,
                    category: None,
                };
                close_others(w);
            });
        }
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
                // Why not, as a hint (as CS:S says "You have
                // insufficient funds.").
                let what = item.buy.clone();
                commands.queue(move |w: &mut World| {
                    let Some(player) = w.query_filtered::<Entity, With<LocalPlayer>>().iter(w).next() else {
                        return;
                    };
                    if let Err(why) = crate::weapon::economy::buy(w, player, &what) {
                        w.write_message(super::chat::Hint(why));
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
            ("cs_source:weapon_hegrenade", 3),
            ("cs_source:weapon_flashbang", 3),
        ];
        p.weapons.insert("cs_source:weapon_hegrenade", 300);
        p.weapons.insert("cs_source:weapon_flashbang", 200);
        let pistols = items(0, &p, &slots);
        assert_eq!(
            pistols.iter().map(|i| i.label.as_str()).collect::<Vec<_>>(),
            ["GLOCK", "USP"]
        );
        let rifles = items(1, &p, &slots);
        assert_eq!(rifles[0].team, Some(1));
        let equipment = items(2, &p, &slots);
        assert_eq!(
            equipment.iter().map(|i| i.label.as_str()).collect::<Vec<_>>(),
            ["FLASHBANG", "HE GRENADE", "Kevlar", "Kevlar + Helmet"]
        );
        assert_eq!(equipment[3].buy, "vesthelm");
    }
}
