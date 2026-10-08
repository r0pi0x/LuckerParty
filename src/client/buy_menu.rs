//! The buy menu (B, or `buymenu`). With the game's own menu layouts
//! (`map::hud::GameMenus`, CS:S's VGUI pages) it is drawn as the game
//! draws it (`super::vgui`): the team's main page of categories, each
//! category's page of items, and the hovered item's picture, price and
//! details; the mouse is free while it is open, buttons are clicked or
//! pressed with their number keys, items you can't buy are greyed. Without
//! them, a plain list: categories, then items with their prices, picked
//! with the number keys; a game's direct keys (CS:S's 6 and 7, primary and
//! secondary ammo) buy at once. Buying goes through `economy::buy`, so the
//! money, the buy period and team limits apply.

use bevy::prelude::*;
use bevy::window::CursorOptions;

use super::vgui::{Painter, Shown, VguiButton, VguiFonts, VguiMenu, VguiOpen};
use crate::{
    console::{Console, ConsoleAppExt},
    core::{LocalPlayer, Team},
    map::hud::{ActiveHud, GameMenus, UiControl, UiKind, UiLayout, layout_key},
    weapon::{
        Armor, Inventory, Weapon, WeaponRegistry,
        economy::{AMMO_BUYS, BuyWindow, Money, Prices},
    },
};

pub struct BuyMenuPlugin;

impl Plugin for BuyMenuPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<BuyMenu>()
            .add_systems(Update, (keys, pointer, draw, draw_vgui).chain());
        app.console_command(
            "buymenu",
            "Open or close the buy menu (B); buymenu <n> opens the category on key n (CS:S's menu: 1 pistols, 2 shotguns, 3 SMGs, 4 rifles, 5 machine guns, 6 equipment; its plain one: 6 primary ammo, 7 secondary ammo, 8 equipment).",
            |w, a| {
                let n = a.first().and_then(|n| n.parse::<u8>().ok());
                let team = local_team(w);
                // The game's own pages: the main page's button on that key.
                if let Some(page) = w
                    .get_resource::<ActiveHud>()
                    .and_then(|h| Some((h.0.menus.as_ref()?, main_page(h.0.menus.as_ref()?, team)?)))
                    .map(|(menus, main)| {
                        n.and_then(|n| char::from_digit(n as u32, 10))
                            .and_then(|k| main.controls.iter().find(|c| c.kind == UiKind::Button && c.hotkey == Some(k)))
                            .and_then(|c| c.command.as_deref().map(layout_key))
                            .filter(|p| menus.layouts.contains_key(p))
                    })
                {
                    let mut m = w.resource_mut::<BuyMenu>();
                    m.open = page.is_some() || !m.open;
                    m.page = page;
                    m.hover = None;
                    if m.open {
                        close_others(w);
                    }
                    return Ok(None);
                }
                let slots = weapon_slots(w.resource::<WeaponRegistry>());
                let menu = categories(w.resource::<Prices>(), &slots, None);
                let category = n.filter(|n| menu.iter().any(|c| c.key == *n && c.direct.is_none()));
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

/// Whether the menu is open, and which category is shown: by its key in
/// the plain menu, by its layout (`GameMenus::layouts` key) in the game's
/// own; and the button the pointer is over there.
#[derive(Resource, Default, Debug, Clone, PartialEq)]
pub struct BuyMenu {
    pub open: bool,
    pub category: Option<u8>,
    pub page: Option<String>,
    pub hover: Option<String>,
}

fn local_team(w: &mut World) -> Option<u8> {
    w.query_filtered::<&Team, With<LocalPlayer>>().iter(w).next().map(|t| t.0)
}

/// The game's main buy page for `team`, when it has its own menus.
fn main_page(menus: &GameMenus, team: Option<u8>) -> Option<&UiLayout> {
    menus.layouts.get(menus.buy_page(team)?)
}

/// The page shown: the open category's, else the main one.
fn current_page<'a>(menus: &'a GameMenus, menu: &BuyMenu, team: Option<u8>) -> Option<&'a UiLayout> {
    menu.page
        .as_ref()
        .and_then(|p| menus.layouts.get(p))
        .or_else(|| main_page(menus, team))
}

/// What the buy button's `buy X` names, as `Prices` knows it: `vest`,
/// `vesthelm`, `defuser` or a weapon ID.
fn priced(what: &str, registry: &WeaponRegistry) -> Option<String> {
    let what = what.to_lowercase();
    match what.as_str() {
        "vest" | "vesthelm" | "defuser" => Some(what),
        _ => registry
            .find(&format!("weapon_{what}"))
            .or_else(|| registry.find(&what))
            .map(|d| d.id.to_string()),
    }
}

/// What the player's state makes of the game's buttons.
struct BuyState<'a> {
    prices: &'a Prices,
    registry: &'a WeaponRegistry,
    console: &'a Console,
    money: Option<u32>,
    team: Option<u8>,
    /// The weapons carried: (slot, ID).
    held: Vec<(u8, &'static str)>,
}

impl BuyState<'_> {
    /// Price of what a `buy X` command gets, if it is for sale to us.
    /// Ammo (`primammo`, `secammo`) costs a box for the weapon carried in
    /// its slot.
    fn price(&self, command: &str) -> Option<u32> {
        let what = command.strip_prefix("buy ")?.trim();
        if let Some((_, slot, _)) = AMMO_BUYS.iter().find(|(n, ..)| n.eq_ignore_ascii_case(what)) {
            return self
                .held
                .iter()
                .find_map(|(s, id)| (*s == *slot).then(|| self.prices.ammo.get(id))?)
                .map(|b| b.price);
        }
        let id = priced(what, self.registry)?;
        if self
            .prices
            .team_only
            .get(id.as_str())
            .is_some_and(|t| self.team.is_some_and(|me| me != *t))
        {
            return None;
        }
        self.prices.of(&id)
    }

    /// A button works when it opens a page, cancels, buys something for
    /// sale that we can afford, or runs a command this game has.
    fn shown(&self, c: &UiControl) -> Shown {
        let mut s = Shown::of(c);
        if c.kind != UiKind::Button || !s.enabled {
            return s;
        }
        s.enabled = match c.command.as_deref().map(str::trim) {
            None => false,
            Some(cmd) if cmd.to_lowercase().ends_with(".res") || cmd.eq_ignore_ascii_case("vguicancel") => true,
            Some(cmd) if cmd.starts_with("buy ") => self
                .price(cmd)
                .is_some_and(|p| self.money.is_none_or(|m| m >= p)),
            Some(cmd) => cmd
                .split_whitespace()
                .next()
                .is_some_and(|name| self.console.command(name).is_some()),
        };
        s
    }
}

/// The weapons in an inventory: (slot, ID).
fn held_slots(inv: Option<&Inventory>, weapons: &Query<&Weapon>) -> Vec<(u8, &'static str)> {
    inv.map(|i| i.weapons.iter().filter_map(|w| weapons.get(*w).ok()).map(|w| (w.slot, w.id)).collect())
        .unwrap_or_default()
}

/// What pressing one of the game's buttons does.
#[derive(Debug, PartialEq)]
enum Action {
    /// Open another page (layout key).
    Page(String),
    /// Buy this (as `buy` takes it) and close.
    Buy(String),
    Close,
    /// Run a console line and close.
    Console(String),
}

fn action(command: &str) -> Action {
    let command = command.trim();
    if command.to_lowercase().ends_with(".res") {
        Action::Page(layout_key(command))
    } else if command.eq_ignore_ascii_case("vguicancel") {
        Action::Close
    } else if let Some(what) = command.strip_prefix("buy ") {
        Action::Buy(what.trim().to_string())
    } else {
        Action::Console(command.to_string())
    }
}

fn apply(action: Action, menu: &mut BuyMenu, console: &mut Console, commands: &mut Commands) {
    match action {
        Action::Page(p) => {
            menu.page = Some(p);
            menu.hover = None;
        }
        Action::Buy(what) => {
            buy_now(commands, what);
            *menu = BuyMenu::default();
        }
        Action::Close => *menu = BuyMenu::default(),
        Action::Console(line) => {
            console.submit(line);
            *menu = BuyMenu::default();
        }
    }
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

/// What else is reading the keyboard (console, chat, game menu).
type Typing<'a> = (
    Option<Res<'a, super::console::ConsoleUi>>,
    Option<Res<'a, super::chat::ChatInput>>,
    Option<Res<'a, super::game_menu::GameMenu>>,
);

#[allow(clippy::too_many_arguments)]
fn keys(
    (keys, mouse): (Res<ButtonInput<KeyCode>>, Res<ButtonInput<MouseButton>>),
    cursor: Single<&CursorOptions>,
    (open, typing): (Res<VguiOpen>, Typing),
    mut menu: ResMut<BuyMenu>,
    (prices, registry, hud, mut console): (Res<Prices>, Res<WeaponRegistry>, Option<Res<ActiveHud>>, ResMut<Console>),
    (player, weapons): (
        Option<Single<(Option<&Team>, Option<&Money>, Option<&Inventory>), With<LocalPlayer>>>,
        Query<&Weapon>,
    ),
    mut slots: Local<Option<Vec<(&'static str, u8)>>>,
    mut commands: Commands,
) {
    let (ui, chat, game_menu) = typing;
    if !super::vgui::keys_live(&cursor, &open, ui.as_deref(), chat.as_deref(), game_menu.as_deref()) {
        return;
    }
    if super::binds::just_pressed(&console.binds, &keys, &mouse, "buymenu") {
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
                    ..default()
                };
                close_others(w);
            });
        }
        return;
    }
    if !menu.open {
        return;
    }
    let team = player.as_ref().and_then(|p| p.0).map(|t| t.0);
    // The game's own pages: the button with that hotkey.
    if let Some(menus) = hud.as_ref().and_then(|h| h.0.menus.as_ref())
        && let Some(page) = current_page(menus, &menu, team)
    {
        let Some(key) = super::vgui::digit(&keys) else {
            return;
        };
        let state = BuyState {
            prices: &prices,
            registry: &registry,
            console: &console,
            money: player.as_ref().and_then(|p| p.1).map(|m| m.0),
            team,
            held: held_slots(player.as_ref().and_then(|p| p.2), &weapons),
        };
        let pressed = super::vgui::hotkey_button(page, key, &mut |c| state.shown(c)).and_then(|c| c.command.clone());
        if let Some(command) = pressed {
            apply(action(&command), &mut menu, &mut console, &mut commands);
        }
        return;
    }
    let Some(n) = DIGITS.iter().position(|k| keys.just_pressed(*k)) else {
        return;
    };
    let slots = slots.get_or_insert_with(|| weapon_slots(&registry));
    let all = categories(&prices, slots, team);
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
    hud: Option<Res<ActiveHud>>,
    mut commands: Commands,
) {
    let (money, team, inv, armor) = player.map(|p| *p).unwrap_or((None, None, None, None));
    // The game's own look draws it instead (`draw_vgui`).
    let game_look = hud
        .as_ref()
        .and_then(|h| h.0.menus.as_ref())
        .is_some_and(|m| main_page(m, team.map(|t| t.0)).is_some());
    if !menu.open || game_look {
        for (e, ..) in &text {
            commands.entity(e).despawn();
        }
        return;
    }
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

/// The game-look menu's buttons: the pointer over one shows its item;
/// a click presses it.
fn pointer(
    buttons: Query<(&Interaction, &VguiButton), Changed<Interaction>>,
    mut menu: ResMut<BuyMenu>,
    mut console: ResMut<Console>,
    mut commands: Commands,
) {
    for (interaction, b) in &buttons {
        if b.menu != VguiMenu::Buy || !menu.open {
            continue;
        }
        match interaction {
            Interaction::Hovered => {
                if menu.hover.as_deref() != Some(b.name.as_str()) {
                    menu.hover = Some(b.name.clone());
                }
            }
            Interaction::Pressed => {
                if b.enabled
                    && let Some(command) = &b.command
                {
                    apply(action(command), &mut menu, &mut console, &mut commands);
                    return;
                }
            }
            Interaction::None => {}
        }
    }
}

/// Everything the game-look menu spawned.
#[derive(Component)]
struct VguiRoot;

/// What the game-look menu shows now, to redraw on change.
#[derive(Default)]
struct Drawn {
    page: String,
    /// The page's item panel (`ItemInfo`) and its size.
    info: Option<(Entity, Vec2)>,
    info_shown: Option<String>,
}

/// The picture the game's menu shows for a `buy` name (`ak47`): the image
/// of the description panel of the button that buys it.
fn item_image<'a>(menus: &'a GameMenus, what: &str) -> Option<&'a String> {
    let command = format!("buy {what}");
    menus
        .layouts
        .values()
        .flat_map(|l| &l.controls)
        .filter(|c| c.command.as_deref().is_some_and(|c| c.eq_ignore_ascii_case(&command)))
        .find_map(|c| menus.layouts.get(c.info.as_ref()?)?.get("classimage")?.image.as_ref())
}

#[allow(clippy::too_many_arguments)]
fn draw_vgui(
    menu: Res<BuyMenu>,
    hud: Option<Res<ActiveHud>>,
    fonts: Option<Res<VguiFonts>>,
    (prices, registry, console): (Res<Prices>, Res<WeaponRegistry>, Res<Console>),
    player: Option<Single<(Option<&Money>, Option<&Team>, Option<&Inventory>), With<LocalPlayer>>>,
    weapons: Query<&Weapon>,
    windows: Query<&Window>,
    roots: Query<Entity, With<VguiRoot>>,
    mut open: ResMut<VguiOpen>,
    mut drawn: Local<Drawn>,
    mut slots: Local<Option<Vec<(&'static str, u8)>>>,
    mut commands: Commands,
) {
    let (money, team, inv) = player.map(|p| *p).unwrap_or((None, None, None));
    let (money, team) = (money.map(|m| m.0), team.map(|t| t.0));
    let menus = hud.as_ref().and_then(|h| h.0.menus.as_ref());
    let page = menus.filter(|_| menu.open).and_then(|m| current_page(m, &menu, team));
    let (Some(hud), Some(menus), Some(page), Some(window)) = (hud.as_ref(), menus, page, windows.iter().next()) else {
        for e in &roots {
            commands.entity(e).despawn();
        }
        *drawn = Drawn::default();
        let closed = VguiOpen { buy: false, ..*open };
        open.set_if_neq(closed);
        return;
    };
    let shown = VguiOpen { buy: true, ..*open };
    open.set_if_neq(shown);
    let slots = slots.get_or_insert_with(|| weapon_slots(&registry));
    let mut held: Vec<(u8, &str)> = inv
        .map(|i| i.weapons.iter().filter_map(|w| weapons.get(*w).ok()).map(|w| w.id).collect::<Vec<_>>())
        .unwrap_or_default()
        .into_iter()
        .filter_map(|id| Some((slots.iter().find(|(s, _)| *s == id)?.1, id)))
        .collect();
    held.sort();
    let (w, h) = (window.width(), window.height());
    let key = format!("{:?}|{team:?}|{money:?}|{w}x{h}|{held:?}", menu.page);
    let state = BuyState {
        prices: &prices,
        registry: &registry,
        console: &console,
        money,
        team,
        held: held_slots(inv, &weapons),
    };
    let painter = Painter::new(hud, menus, fonts.as_deref(), h);
    if drawn.page != key || roots.is_empty() {
        for e in &roots {
            commands.entity(e).despawn();
        }
        // The frame darkens the whole screen; the pages lay out in the
        // virtual 640x480 screen.
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
        let a = super::vgui::area(w, h);
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
        let parts = painter.spawn(&mut commands, area, a.size(), page, VguiMenu::Buy, &mut |c| state.shown(c));
        // The current loadout: the held primary and secondary weapons'
        // pictures, side by side.
        if let Some(&(panel, size)) = parts.get("loadoutpanel") {
            let cell = Vec2::new(size.x / 2.0, size.x / 4.0);
            let mut x = 0.0;
            for (slot, id) in held.iter().filter(|(s, _)| *s <= 1) {
                let what = id.rsplit(':').next().unwrap_or(id).trim_start_matches("weapon_");
                let Some(sprite) = item_image(menus, what).and_then(|i| hud.0.sprites.get(i)) else {
                    continue;
                };
                let Some(image) = hud.1.get(&sprite.texture) else { continue };
                let [sx, sy, sw, sh] = sprite.rect;
                commands.spawn((
                    Node {
                        position_type: PositionType::Absolute,
                        left: px(x),
                        top: px(if *slot == 0 { 0.0 } else { (size.y - cell.y).max(0.0) }),
                        width: px(cell.x),
                        height: px(cell.y),
                        ..default()
                    },
                    ImageNode {
                        image: image.clone(),
                        rect: Some(Rect::new(sx, sy, sx + sw, sy + sh)),
                        ..default()
                    },
                    ChildOf(panel),
                ));
                x += cell.x;
            }
        }
        drawn.page = key;
        drawn.info = parts.get("iteminfo").copied();
        drawn.info_shown = None;
    }
    // The item's description: the hovered button's, else the page's first.
    let item = menu
        .hover
        .as_deref()
        .and_then(|name| page.controls.iter().find(|c| c.name == name && c.info.is_some()))
        .or_else(|| {
            page.controls
                .iter()
                .find(|c| c.kind == UiKind::Button && c.info.is_some() && state.shown(c).visible)
        });
    let info = item.and_then(|c| Some((c.info.clone()?, c.command.clone())));
    let info_key = info.as_ref().map(|(k, cmd)| format!("{k}|{cmd:?}"));
    if drawn.info_shown != info_key {
        drawn.info_shown = info_key;
        if let Some((panel, size)) = drawn.info {
            commands.entity(panel).despawn_related::<Children>();
            if let Some((info, command)) = info
                && let Some(layout) = menus.layouts.get(&info)
            {
                // Today's price rather than the file's.
                let price = command.as_deref().and_then(|c| state.price(c));
                painter.spawn(&mut commands, panel, size, layout, VguiMenu::Buy, &mut |c| {
                    let mut s = Shown::of(c);
                    if c.name.eq_ignore_ascii_case("price")
                        && let Some(p) = price
                    {
                        s.text = Some(format!(": ${p}"));
                    }
                    s
                });
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::map::hud::GameHud;
    use bevy::window::CursorGrabMode;
    use std::collections::HashMap;

    fn button(name: &str, key: char, command: &str) -> UiControl {
        let mut c = UiControl::new(name, UiKind::Button, 52.0, 116.0, 170.0, 20.0);
        c.hotkey = Some(key);
        c.text = format!("{key} {name}");
        c.command = Some(command.into());
        c
    }

    /// A game's own buy pages, as a game's files would give them.
    fn game_menus() -> GameMenus {
        let main = UiLayout {
            controls: vec![
                button("equipment", '6', "Resource/UI/BuyEquipment_CT.res"),
                button("autobuy", 'a', "autobuy"),
                button("CancelButton", '0', "vguicancel"),
            ],
        };
        let equipment = UiLayout {
            controls: vec![
                button("kevlar", '1', "buy vest"),
                button("kevlar_helmet", '2', "buy vesthelm"),
                button("nightvision", '7', "buy nvgs"),
                button("CancelButton", '0', "vguicancel"),
            ],
        };
        GameMenus {
            layouts: HashMap::from([
                ("resource/ui/buymenu_ct.res".to_string(), main),
                ("resource/ui/buyequipment_ct.res".to_string(), equipment),
            ]),
            buy: HashMap::from([(2, "resource/ui/buymenu_ct.res".to_string())]),
            ..default()
        }
    }

    /// The buy menu's key handling with a local CT holding $800.
    fn app(game_look: bool) -> (App, Entity, Entity) {
        let mut app = App::new();
        app.add_plugins(crate::console::ConsolePlugin)
            .add_message::<super::super::chat::Hint>()
            .init_resource::<ButtonInput<KeyCode>>()
            .init_resource::<BuyMenu>()
            .init_resource::<VguiOpen>()
            .init_resource::<WeaponRegistry>()
            .init_resource::<BuyWindow>()
            .insert_resource(Prices {
                vest: 650,
                vest_helmet: 1000,
                helmet: 350,
                ..default()
            })
            .add_systems(Update, keys);
        super::super::binds::test_binds(&mut app);
        if game_look {
            let hud = GameHud {
                menus: Some(game_menus()),
                ..default()
            };
            app.insert_resource(ActiveHud(std::sync::Arc::new(hud), HashMap::new()));
        }
        let window = app
            .world_mut()
            .spawn(CursorOptions {
                grab_mode: CursorGrabMode::Locked,
                ..default()
            })
            .id();
        let player = app.world_mut().spawn((LocalPlayer, Team(2), Money(800))).id();
        (app, player, window)
    }

    fn press(app: &mut App, key: KeyCode) {
        app.world_mut().resource_mut::<ButtonInput<KeyCode>>().press(key);
        app.update();
        let mut keys = app.world_mut().resource_mut::<ButtonInput<KeyCode>>();
        keys.release(key);
        keys.clear();
    }

    #[test]
    fn the_games_own_pages_buy_with_number_keys_while_the_mouse_is_free() {
        let (mut app, player, window) = app(true);
        press(&mut app, KeyCode::KeyB);
        assert!(app.world().resource::<BuyMenu>().open);
        // Open, the menu frees the mouse (as `draw_vgui` and the cursor
        // system do in the game); the keys still work.
        app.world_mut().resource_mut::<VguiOpen>().buy = true;
        super::super::input::release_cursor(&mut app.world_mut().get_mut::<CursorOptions>(window).unwrap());
        press(&mut app, KeyCode::Digit6);
        assert_eq!(
            app.world().resource::<BuyMenu>().page.as_deref(),
            Some("resource/ui/buyequipment_ct.res")
        );
        // Kevlar + helmet ($1000) is greyed out at $800, night vision isn't
        // sold here: nothing happens.
        press(&mut app, KeyCode::Digit2);
        press(&mut app, KeyCode::Digit7);
        assert!(app.world().resource::<BuyMenu>().open);
        assert_eq!(app.world().get::<Money>(player).unwrap().0, 800);
        press(&mut app, KeyCode::Digit1);
        assert!(!app.world().resource::<BuyMenu>().open, "buying closes the menu");
        assert_eq!(app.world().get::<Money>(player).unwrap().0, 150);
        assert!(app.world().get::<Armor>(player).is_some_and(|a| a.amount >= 1.0));
        // 0 cancels.
        press(&mut app, KeyCode::KeyB);
        press(&mut app, KeyCode::Digit0);
        assert!(!app.world().resource::<BuyMenu>().open);
    }

    #[test]
    fn without_the_games_pages_the_plain_menu_buys_with_number_keys() {
        let (mut app, player, _) = app(false);
        press(&mut app, KeyCode::KeyB);
        // Pistols, rifles, equipment; equipment's first item is Kevlar.
        press(&mut app, KeyCode::Digit3);
        assert_eq!(app.world().resource::<BuyMenu>().category, Some(3));
        press(&mut app, KeyCode::Digit1);
        assert!(!app.world().resource::<BuyMenu>().open);
        assert_eq!(app.world().get::<Money>(player).unwrap().0, 150);
    }

    #[test]
    fn game_buttons_work_when_for_sale_affordable_or_known() {
        let mut console = Console::default();
        console.add_command(crate::console::Command {
            name: "rebuy".into(),
            help: String::new(),
            run: std::sync::Arc::new(|_, _| Ok(None)),
            complete: None,
        });
        let (prices, registry) = (
            Prices {
                vest: 650,
                vest_helmet: 1000,
                ..default()
            },
            WeaponRegistry::default(),
        );
        let state = BuyState {
            prices: &prices,
            registry: &registry,
            console: &console,
            money: Some(800),
            team: Some(2),
            held: Vec::new(),
        };
        let enabled = |c: &str| state.shown(&button("b", '1', c)).enabled;
        assert!(enabled("buy vest"));
        assert!(!enabled("buy vesthelm"), "can't afford");
        assert!(!enabled("buy nvgs"), "not sold");
        assert!(enabled("Resource/UI/BuyPistols_CT.res"));
        assert!(enabled("vguicancel"));
        assert!(enabled("rebuy"));
        assert!(!enabled("autobuy"), "no such command");
        assert_eq!(action("Resource\\UI/X.res"), Action::Page("resource/ui/x.res".into()));
        assert_eq!(action("buy ak47"), Action::Buy("ak47".into()));
        // Ammo: a box for the gun in that slot, when one is carried.
        assert!(!enabled("buy primammo"), "no primary weapon");
        let mut prices = prices.clone();
        prices.ammo.insert(
            "cs_source:weapon_ak47",
            crate::weapon::economy::AmmoBox { price: 80, rounds: 30 },
        );
        let armed = BuyState {
            prices: &prices,
            held: vec![(0, "cs_source:weapon_ak47")],
            ..state
        };
        assert_eq!(armed.price("buy primammo"), Some(80));
        assert!(armed.shown(&button("b", '6', "buy primammo")).enabled);
        assert!(!armed.shown(&button("b", '7', "buy secammo")).enabled, "no pistol");
    }

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
