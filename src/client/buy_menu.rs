//! The buy menu (B, or `buymenu`): categories, then items with their
//! prices, picked with the number keys, as CS:S's menu can be driven from
//! the keyboard; a game's direct keys (CS:S's 6 and 7, primary and
//! secondary ammo) buy at once. Buying goes through the `buy` command, so the money, the
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
            "Open or close the buy menu (B); buymenu <n> opens the category on key n (CS:S: 1 pistols, 2 shotguns, 3 SMGs, 4 rifles, 5 machine guns, 6 primary ammo, 7 secondary ammo, 8 equipment).",
            |w, a| {
                let slots = weapon_slots(w.resource::<WeaponRegistry>());
                let menu = categories(w.resource::<Prices>(), &slots, None);
                let category = a
                    .first()
                    .and_then(|n| n.parse::<u8>().ok())
                    .filter(|n| menu.iter().any(|c| c.key == *n && c.direct.is_none()));
                let mut m = w.resource_mut::<BuyMenu>();
                m.open = category.is_some() || !m.open;
                m.category = category;
                if m.open {
                    close_others(w);
                }
                Ok(None)
            },
        );
    }
}

/// Whether the menu is open, and which category is shown (by its key).
#[derive(Resource, Default, Debug, Clone, PartialEq)]
pub struct BuyMenu {
    pub open: bool,
    pub category: Option<u8>,
}

/// One line of the menu: what `buy` gets, its label and price.
#[derive(Clone, Debug, PartialEq)]
struct Item {
    buy: String,
    label: String,
    price: u32,
    team: Option<u8>,
}

/// One category: its number key and name, and its items (or what it
/// buys at once).
#[derive(Clone, Debug, PartialEq)]
struct Category {
    key: u8,
    name: String,
    items: Vec<Item>,
    direct: Option<String>,
}

/// A weapon's display name from its ID: `cs_source:weapon_m4a1` -> M4A1.
fn label(id: &str) -> String {
    let name = id.rsplit(':').next().unwrap_or(id).trim_start_matches("weapon_").to_uppercase();
    match name.strip_suffix("GRENADE") {
        Some(kind) if !kind.is_empty() => format!("{kind} GRENADE"),
        _ => name,
    }
}

/// The menu: the game's own layout (`Prices::menu`), else pistols,
/// primaries (cheapest first) and equipment by slot. Items only another
/// team may buy are left out when `team` is known (as CS:S shows each
/// team its own list).
fn categories(prices: &Prices, slots: &[(&'static str, u8)], team: Option<u8>) -> Vec<Category> {
    let item = |buy: &str, label: String| -> Option<Item> {
        Some(Item {
            buy: buy.to_string(),
            label,
            price: prices.of(buy)?,
            team: prices.team_only.get(buy).copied(),
        })
    };
    let mut out: Vec<Category> = if prices.menu.is_empty() {
        let by_slot = |slot: u8| -> Vec<Item> {
            let mut items: Vec<Item> = slots
                .iter()
                .filter(|(_, s)| *s == slot)
                .filter_map(|(id, _)| item(id, label(id)))
                .collect();
            items.sort_by_key(|i| (i.price, i.label.clone()));
            items
        };
        let mut equipment: Vec<Item> = [("vest", "Kevlar"), ("vesthelm", "Kevlar + Helmet")]
            .into_iter()
            .filter_map(|(b, l)| item(b, l.into()))
            .collect();
        // Grenades (slot 3) are equipment.
        equipment.extend(by_slot(3));
        equipment.sort_by_key(|i| (i.price, i.label.clone()));
        vec![
            Category {
                key: 1,
                name: "Pistols".into(),
                items: by_slot(1),
                direct: None,
            },
            Category {
                key: 2,
                name: "Rifles".into(),
                items: by_slot(0),
                direct: None,
            },
            Category {
                key: 3,
                name: "Equipment".into(),
                items: equipment,
                direct: None,
            },
        ]
    } else {
        prices
            .menu
            .iter()
            .map(|c| Category {
                key: c.key,
                name: c.name.into(),
                items: c.items.iter().filter_map(|i| item(i.buy, i.label.into())).collect(),
                direct: c.direct.map(str::to_string),
            })
            .collect()
    };
    for c in &mut out {
        c.items.retain(|i| i.team.is_none() || team.is_none() || i.team == team);
    }
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
    team: Option<Single<&Team, With<LocalPlayer>>>,
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
    let all = categories(&prices, slots, team.map(|t| t.0));
    match (menu.category, n) {
        (_, 0) => *menu = BuyMenu::default(),
        (None, n) => match all.iter().find(|c| c.key as usize == n) {
            Some(Category { direct: Some(what), .. }) => {
                buy_now(&mut commands, what.clone());
                *menu = BuyMenu::default();
            }
            Some(_) => menu.category = Some(n as u8),
            None => {}
        },
        (Some(c), n) => {
            let items = all.iter().find(|x| x.key == c).map(|x| x.items.as_slice()).unwrap_or_default();
            if let Some(item) = items.get(n - 1) {
                buy_now(&mut commands, item.buy.clone());
                *menu = BuyMenu::default();
            }
        }
    }
}

/// Buy `what` for the local player; why not, as a hint (as CS:S says
/// "You have insufficient funds.").
fn buy_now(commands: &mut Commands, what: String) {
    commands.queue(move |w: &mut World| {
        let Some(player) = w.query_filtered::<Entity, With<LocalPlayer>>().iter(w).next() else {
            return;
        };
        if let Err(why) = crate::weapon::economy::buy(w, player, &what) {
            w.write_message(super::chat::Hint(why));
        }
    });
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
    let all = categories(&prices, slots, team.map(|t| t.0));
    match menu
        .category
        .and_then(|c| all.iter().find(|x| x.key == c && x.direct.is_none()))
    {
        None => {
            for c in &all {
                lines.push(format!("{}  {}", c.key, c.name));
            }
        }
        Some(c) => {
            lines.push(c.name.to_uppercase());
            for (i, item) in c.items.iter().enumerate() {
                let have = held.contains(&item.buy.as_str())
                    || (item.buy == "vest" && armor.is_some_and(|a| a.amount >= 1.0));
                let broke = money.is_some_and(|m| m.0 < item.price);
                let note = if have {
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
        let menu = categories(&p, &slots, None);
        assert_eq!(menu.iter().map(|c| c.key).collect::<Vec<_>>(), [1, 2, 3]);
        let labels = |c: &Category| c.items.iter().map(|i| i.label.clone()).collect::<Vec<_>>();
        assert_eq!(labels(&menu[0]), ["GLOCK", "USP"]);
        assert_eq!(menu[1].items[0].team, Some(1));
        assert_eq!(
            labels(&menu[2]),
            ["FLASHBANG", "HE GRENADE", "Kevlar", "Kevlar + Helmet"]
        );
        assert_eq!(menu[2].items[3].buy, "vesthelm");
        // A CT doesn't see the terrorists' rifle.
        assert!(categories(&p, &slots, Some(2))[1].items.is_empty());
    }

    #[test]
    fn a_games_own_layout_keeps_its_keys_and_order() {
        use crate::weapon::economy::{BuyCategory, BuyItem};
        let mut p = Prices {
            vest: 650,
            ..default()
        };
        p.weapons.insert("g:weapon_b", 100);
        p.weapons.insert("g:weapon_a", 200);
        p.menu = vec![
            BuyCategory {
                key: 4,
                direct: None,
                name: "Rifles",
                items: vec![
                    BuyItem {
                        buy: "g:weapon_a",
                        label: "A",
                    },
                    BuyItem {
                        buy: "g:weapon_b",
                        label: "B",
                    },
                ],
            },
            BuyCategory {
                key: 8,
                direct: None,
                name: "Equipment",
                items: vec![BuyItem {
                    buy: "vest",
                    label: "Kevlar",
                }],
            },
            BuyCategory {
                key: 6,
                direct: Some("primammo"),
                name: "Primary Ammo",
                items: Vec::new(),
            },
        ];
        let menu = categories(&p, &[], None);
        assert_eq!((menu[0].key, menu[1].key), (4, 8));
        assert_eq!(menu[2].direct.as_deref(), Some("primammo"));
        assert_eq!((menu[0].items[0].price, menu[0].items[1].label.as_str()), (200, "B"));
        assert_eq!(menu[1].items[0].price, 650);
    }
}
