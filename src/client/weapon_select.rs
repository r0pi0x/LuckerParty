//! The game HUD's weapon selection (CS:S `HudWeaponSelection`) and pickup
//! history (`HudHistoryResource`), drawn as the game draws them
//! (measured on CS:S captures with `refcmp hudcmp`).
//!
//! Selection: a row of `MaxSlots` boxes centred at the top of the screen,
//! the highlighted slot's box large (`LargeBoxWide` x `LargeBoxTall`) and
//! the others small (`SmallBoxSize`), `BoxGap` apart. Only the large box
//! is drawn; each slot that holds something shows its number in its box's
//! corner (the highlighted slot's bright, the others at half), and the
//! large box the weapon's icon (the `CSweapons` font: the kill icon's
//! letter in capitals; the outline face, `CSType`'s, when it isn't the
//! weapon you hold) and its localised name (`#Cstrike_WPNHUD_*`).
//!
//! With `hud_fastswitch 0` (CS:S's default) a slot key opens the row on
//! that slot (again: the next weapon in it), attack takes the highlighted
//! weapon and attack2 closes it; with `hud_fastswitch 1` slot keys switch
//! at once and the row shows the drawn weapon for a moment. The row closes
//! itself after a while (`MENU_SECONDS`, ours).
//!
//! History: each weapon you get shows on the right, newest at the bottom
//! of the panel and older ones `history_gap` above, for
//! `HISTORY_SECONDS` (CS:S's `hud_drawhistory_time` default) and fading.

use std::collections::HashSet;

use bevy::{prelude::*, window::CursorOptions};

use super::{
    fonts::UiFonts,
    game_hud::{Drawn, scaled},
    hud_text::{Align, GlyphFont, HudText, width},
};
use crate::{
    console::{Console, ConsoleAppExt},
    core::LocalPlayer,
    map::hud::{ActiveHud, GameHud},
    weapon::{Inventory, Weapon, WeaponEvent, WeaponEventKind},
};

pub struct WeaponSelectPlugin;

impl Plugin for WeaponSelectPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<WeaponMenu>()
            .init_resource::<FastSwitch>()
            .init_resource::<History>()
            .add_systems(
                Update,
                (
                    menu_input.before(super::input::write_local_intent),
                    (track_pickups, show_deploys, draw)
                        .chain()
                        .after(menu_input)
                        .run_if(resource_exists::<ActiveHud>),
                ),
            );
        crate::console::resource_cvar::<FastSwitch, u8>(
            app,
            "hud_fastswitch",
            "0: a slot key opens the weapon selection, attack takes the weapon (CS:S's default); 1: slot keys \
             switch at once.",
            |f| &mut f.0,
        );
        app.world_mut().resource_mut::<Console>().archive("hud_fastswitch");
        for slot in 0..5u8 {
            app.console_command(
                &format!("slot{}", slot + 1),
                "Weapon slot (with hud_fastswitch 0: open the weapon selection on it).",
                move |w, _| {
                    w.resource_mut::<WeaponMenu>().pressed.push(slot);
                    Ok(None)
                },
            );
        }
    }
}

/// `hud_fastswitch`.
#[derive(Resource, Default)]
pub struct FastSwitch(pub u8);

/// Seconds the selection stays open waiting for a pick, and the last part
/// of that (or of a fast switch's showing) it fades over (ours; the game's
/// timing isn't in its data files).
const MENU_SECONDS: f32 = 3.0;
const SHOW_SECONDS: f32 = 1.5;
const FADE_SECONDS: f32 = 0.5;
/// Seconds a picked weapon's slot is held in the intent, so a fixed tick
/// sees it.
const SELECT_HOLD: f32 = 0.15;
/// Seconds a pickup shows (CS:S's `hud_drawhistory_time` default), the
/// last of which it fades over (ours).
const HISTORY_SECONDS: f32 = 5.0;
const HISTORY_FADE: f32 = 1.0;
/// The large box's darkening (measured: half the light behind it).
const LARGE_BOX_ALPHA: f32 = 0.5;
/// Unhighlighted slots' numbers draw at the panel's half alpha.
const DIM: f32 = 0.5;

/// The selection's state.
#[derive(Resource, Default)]
pub struct WeaponMenu {
    /// The highlighted weapon while the row shows.
    pub highlighted: Option<Entity>,
    /// Seconds the row has shown; None when hidden.
    shown_for: Option<f32>,
    /// Open for a pick (fast switch off), not just showing a switch.
    pub choosing: bool,
    /// The slot to select and how long it is still held in the intent.
    pub select: Option<(u8, f32)>,
    /// Attack has picked a weapon and is still held: no firing until it
    /// is let go.
    pub swallow_fire: bool,
    /// Slot keys work through the selection (fast switch off and the game's
    /// HUD has one); else they switch directly.
    pub takes_slots: bool,
    /// Slot commands run since last frame (console, binds of `slotN`).
    pressed: Vec<u8>,
}

impl WeaponMenu {
    /// The slot the input should select this frame.
    pub fn selected_slot(&self) -> Option<u8> {
        self.select.map(|s| s.0)
    }

    /// The row's opacity now (1 while showing, fading at the end), None
    /// when hidden.
    fn alpha(&self) -> Option<f32> {
        let t = self.shown_for?;
        let total = if self.choosing { MENU_SECONDS } else { SHOW_SECONDS };
        let left = total - t;
        (left > 0.0).then(|| (left / FADE_SECONDS).min(1.0))
    }
}

/// What a slot key does with the selection open on `current` (or closed):
/// the first weapon in `slot`, or the next one in it after `current`;
/// `held` lists (weapon, slot) in inventory order.
pub fn next_in_slot(held: &[(Entity, u8)], slot: u8, current: Option<Entity>) -> Option<Entity> {
    let in_slot: Vec<Entity> = held.iter().filter(|(_, s)| *s == slot).map(|(e, _)| *e).collect();
    match current.and_then(|c| in_slot.iter().position(|e| *e == c)) {
        Some(i) => Some(in_slot[(i + 1) % in_slot.len()]),
        None => in_slot.first().copied(),
    }
}

#[allow(clippy::too_many_arguments)]
fn menu_input(
    mut menu: ResMut<WeaponMenu>,
    fast: Res<FastSwitch>,
    hud: Option<Res<ActiveHud>>,
    console: Option<Res<Console>>,
    (keys, mouse): (Res<ButtonInput<KeyCode>>, Res<ButtonInput<MouseButton>>),
    cursor: Query<&CursorOptions>,
    player: Option<Single<&Inventory, With<LocalPlayer>>>,
    weapons: Query<&Weapon>,
    (buy, team, radio): (
        Option<Res<super::buy_menu::BuyMenu>>,
        Option<Res<super::team_menu::TeamMenu>>,
        Option<Res<super::radio::RadioMenu>>,
    ),
    time: Res<Time>,
) {
    let dt = time.delta_secs();
    menu.takes_slots = fast.0 == 0
        && hud
            .as_ref()
            .is_some_and(|h| h.0.panels.contains_key("HudWeaponSelection"));
    if let Some((slot, left)) = menu.select {
        menu.select = (left > dt).then_some((slot, left - dt));
    }
    if let Some(t) = menu.shown_for.as_mut() {
        *t += dt;
    }
    if menu.shown_for.is_some() && menu.alpha().is_none() {
        menu.shown_for = None;
        menu.choosing = false;
    }
    let empty = std::collections::BTreeMap::new();
    let binds = console.as_ref().map_or(&empty, |c| &c.binds);
    let playing = cursor.iter().next().is_some_and(super::input::cursor_grabbed)
        && !buy.is_some_and(|m| m.open)
        && !team.is_some_and(|m| m.0)
        && !radio.is_some_and(|m| m.0.is_some());
    let just = |c: &str| playing && super::binds::just_pressed(binds, &keys, &mouse, c);
    let attack_held = playing && super::binds::pressed(binds, &keys, &mouse, "+attack");
    if !attack_held {
        menu.swallow_fire = false;
    }
    let mut pressed = std::mem::take(&mut menu.pressed);
    if menu.takes_slots {
        pressed.extend((0..5u8).filter(|s| just(&format!("slot{}", s + 1))));
    } else if let Some(&slot) = pressed.last() {
        // Fast switch: the command switches directly, like the key.
        menu.select = Some((slot, SELECT_HOLD));
        return;
    }
    let Some(inv) = player else { return };
    let held: Vec<(Entity, u8)> = inv
        .weapons
        .iter()
        .filter_map(|e| weapons.get(*e).ok().map(|w| (*e, w.slot)))
        .collect();
    for slot in pressed {
        let current = menu.highlighted.filter(|_| menu.choosing);
        if let Some(next) = next_in_slot(&held, slot, current) {
            menu.highlighted = Some(next);
            menu.choosing = true;
            menu.shown_for = Some(0.0);
        }
    }
    if menu.choosing && menu.shown_for.is_some() {
        if just("+attack") {
            if let Some(slot) = menu
                .highlighted
                .and_then(|e| held.iter().find(|(w, _)| *w == e))
                .map(|(_, s)| *s)
            {
                menu.select = Some((slot, SELECT_HOLD));
            }
            menu.swallow_fire = true;
            menu.shown_for = None;
            menu.choosing = false;
        } else if just("+attack2") {
            menu.shown_for = None;
            menu.choosing = false;
        }
    }
}

/// With fast switch on, a weapon drawn shows the row on it for a moment.
fn show_deploys(
    mut events: MessageReader<WeaponEvent>,
    mut menu: ResMut<WeaponMenu>,
    player: Option<Single<Entity, With<LocalPlayer>>>,
) {
    let Some(me) = player.map(|p| *p) else {
        events.clear();
        return;
    };
    for e in events.read() {
        if e.owner == me && matches!(e.kind, WeaponEventKind::Deployed) && !menu.takes_slots {
            menu.highlighted = Some(e.weapon);
            menu.choosing = false;
            menu.shown_for = Some(0.0);
        }
    }
}

/// Weapons picked up, newest last, with seconds shown.
#[derive(Resource, Default)]
struct History {
    items: Vec<(&'static str, f32)>,
    known: HashSet<Entity>,
}

fn track_pickups(
    mut history: ResMut<History>,
    player: Option<Single<&Inventory, With<LocalPlayer>>>,
    weapons: Query<&Weapon>,
    time: Res<Time>,
) {
    let dt = time.delta_secs();
    history.items.retain_mut(|(_, t)| {
        *t += dt;
        *t < HISTORY_SECONDS
    });
    let Some(inv) = player else {
        history.known.clear();
        return;
    };
    let now: HashSet<Entity> = inv.weapons.iter().copied().collect();
    for e in &inv.weapons {
        if !history.known.contains(e)
            && let Ok(w) = weapons.get(*e)
        {
            history.items.push((w.id, 0.0));
        }
    }
    history.known = now;
}

/// A weapon's short name: `cs_source:weapon_usp` -> `usp`.
fn short(id: &str) -> &str {
    let name = id.rsplit(':').next().unwrap_or(id);
    name.strip_prefix("weapon_").unwrap_or(name)
}

/// The localised name's token of a CS:S weapon (`Cstrike_WPNHUD_*`).
fn name_token(short: &str) -> Option<&'static str> {
    Some(match short {
        "ak47" => "Cstrike_WPNHUD_AK47",
        "aug" => "Cstrike_WPNHUD_Aug",
        "awp" => "Cstrike_WPNHUD_AWP",
        "deagle" => "Cstrike_WPNHUD_DesertEagle",
        "elite" => "Cstrike_WPNHUD_Elites",
        "famas" => "Cstrike_WPNHUD_Famas",
        "fiveseven" => "Cstrike_WPNHUD_FiveSeven",
        "flashbang" => "Cstrike_WPNHUD_Flashbang",
        "g3sg1" => "Cstrike_WPNHUD_G3SG1",
        "galil" => "Cstrike_WPNHUD_Galil",
        "glock" => "Cstrike_WPNHUD_Glock18",
        "hegrenade" => "Cstrike_WPNHUD_HE_Grenade",
        "knife" => "Cstrike_WPNHUD_Knife",
        "m249" => "Cstrike_WPNHUD_M249",
        "m3" => "Cstrike_WPNHUD_m3",
        "m4a1" => "Cstrike_WPNHUD_M4A1",
        "mac10" => "Cstrike_WPNHUD_MAC10",
        "mp5navy" => "Cstrike_WPNHUD_MP5",
        "p228" => "Cstrike_WPNHUD_P228",
        "p90" => "Cstrike_WPNHUD_P90",
        "scout" => "Cstrike_WPNHUD_Scout",
        "sg550" => "Cstrike_WPNHUD_SG550",
        "sg552" => "Cstrike_WPNHUD_SG552",
        "smokegrenade" => "Cstrike_WPNHUD_Smoke_Grenade",
        "tmp" => "Cstrike_WPNHUD_Tmp",
        "ump45" => "Cstrike_WPNHUD_UMP45",
        "usp" => "Cstrike_WPNHUD_USP45",
        "xm1014" => "Cstrike_WPNHUD_xm1014",
        "c4" => "Cstrike_WPNHUD_C4",
        _ => return None,
    })
}

/// A weapon's letter in the weapon fonts: its kill icon's (`d_<weapon>`).
fn letter(hud: &GameHud, short: &str) -> Option<char> {
    hud.icons.get(&format!("d_{short}")).map(|(_, c)| *c)
}

/// What the row shows.
#[derive(Clone, Debug, PartialEq)]
pub struct Row {
    /// Slots that hold something.
    pub filled: Vec<u8>,
    /// The highlighted slot and weapon (short name), and whether it is
    /// the weapon held.
    pub highlighted: (u8, &'static str, bool),
}

fn rgba(hud: &GameHud, name: &str, fallback: [u8; 4], alpha: f32) -> Color {
    let c = hud.colors.get(name).copied().unwrap_or(fallback);
    Color::srgba_u8(c[0], c[1], c[2], (c[3] as f32 * alpha).round() as u8)
}

/// The selection row's parts in a `w` x `h` window at opacity `alpha`.
pub fn selection_layout(
    hud: &GameHud,
    font: &dyn Fn(&str) -> Option<(GlyphFont, f32)>,
    w: f32,
    h: f32,
    row: &Row,
    alpha: f32,
) -> Vec<Drawn> {
    let mut out = Vec::new();
    let Some(panel) = hud.panels.get("HudWeaponSelection") else {
        return out;
    };
    let s = h / 480.0;
    let num = |k: &str, d: f32| panel.num(k).unwrap_or(d);
    let small = scaled(num("SmallBoxSize", 60.0), s);
    let (large_w, large_h) = (
        scaled(num("LargeBoxWide", 108.0), s),
        scaled(num("LargeBoxTall", 80.0), s),
    );
    let gap = scaled(num("BoxGap", 8.0), s);
    let slots = num("MaxSlots", 5.0) as u8;
    let (hl_slot, weapon, held) = row.highlighted;
    let widths: Vec<f32> = (0..slots).map(|i| if i == hl_slot { large_w } else { small }).collect();
    let total = widths.iter().sum::<f32>() + gap * (slots.saturating_sub(1)) as f32;
    // Centred across the screen, at the panel's y.
    let mut x = ((w - total) / 2.0).trunc();
    let top = panel.y.resolve(h, s).trunc();
    let number_at = Vec2::new(
        scaled(num("SelectionNumberXPos", 4.0), s),
        scaled(num("SelectionNumberYPos", 4.0), s),
    );
    for (i, bw) in widths.iter().enumerate() {
        let i = i as u8;
        let corner = Vec2::new(x, top);
        if i == hl_slot {
            let rect = Rect::from_corners(corner, corner + Vec2::new(large_w, large_h));
            out.push(Drawn::Box(
                rect,
                Color::srgba(0.0, 0.0, 0.0, LARGE_BOX_ALPHA * alpha),
                4.0 * s,
            ));
            let letter = letter(hud, weapon);
            // The icon: the weapon font's capital, or the outline face when
            // it isn't the weapon held, its cell at (`IconXPos`,
            // `IconYPos`) in the box.
            let face = if held {
                font("CSweapons")
            } else {
                font("CSType").zip(font("CSweapons")).map(|(f, c)| (f.0, c.1))
            };
            if let (Some(c), Some((f, tall))) = (letter, face) {
                let text = if held {
                    c.to_ascii_uppercase()
                } else {
                    c.to_ascii_lowercase()
                }
                .to_string();
                out.push(Drawn::Text(HudText {
                    font: f,
                    tall,
                    color: rgba(hud, "FgColor", [255, 176, 0, 255], alpha),
                    at: Vec2::new(
                        x + scaled(num("IconXPos", 8.0), s),
                        top + scaled(num("IconYPos", 0.0), s),
                    ),
                    align: Align::Left,
                    text,
                }));
            }
            let name = name_token(weapon)
                .and_then(|t| hud.menus.as_ref().map(|m| m.string(t, "").to_string()))
                .filter(|n| !n.is_empty())
                .unwrap_or_else(|| weapon.to_uppercase());
            if let Some((f, tall)) = font("HudSelectionText") {
                out.push(Drawn::Text(HudText {
                    font: f,
                    tall,
                    text: name,
                    color: rgba(hud, "SelectionTextFg", [255, 220, 0, 200], alpha),
                    at: Vec2::new(x + (large_w / 2.0).trunc(), top + scaled(num("TextYPos", 68.0), s)),
                    align: Align::Centre,
                }));
            }
        }
        if row.filled.contains(&i)
            && let Some((f, tall)) = font("HudSelectionNumbers")
        {
            let dim = if i == hl_slot { 1.0 } else { DIM };
            out.push(Drawn::Text(HudText {
                font: f,
                tall,
                text: (i + 1).to_string(),
                color: rgba(hud, "SelectionNumberFg", [255, 220, 0, 200], alpha * dim),
                at: corner + number_at,
                align: Align::Left,
            }));
        }
        x += bw + gap;
    }
    out
}

/// The pickup history's icons, `items` oldest first with their opacity.
pub fn history_layout(
    hud: &GameHud,
    font: &dyn Fn(&str) -> Option<(GlyphFont, f32)>,
    w: f32,
    h: f32,
    items: &[(&str, f32)],
) -> Vec<Drawn> {
    let mut out = Vec::new();
    let (Some(panel), Some((f, tall))) = (hud.panels.get("HudHistoryResource"), font("CSweaponsSmall")) else {
        return out;
    };
    let s = h / 480.0;
    let gap = scaled(panel.num("history_gap").unwrap_or(50.0), s);
    let right = (panel.x.resolve(w, s) + panel.wide * s).trunc() - scaled(HISTORY_RIGHT, s);
    let bottom = (panel.y.resolve(h, s) + panel.tall * s).trunc();
    for (i, (weapon, alpha)) in items.iter().rev().enumerate() {
        let Some(c) = letter(hud, weapon) else { continue };
        let text = c.to_ascii_uppercase().to_string();
        let top = bottom - gap * (i as f32 + 1.0);
        if top < 0.0 {
            break;
        }
        out.push(Drawn::Text(HudText {
            font: f.clone(),
            tall,
            color: rgba(hud, "FgColor", [255, 176, 0, 255], *alpha),
            at: Vec2::new(right, top),
            align: Align::InkRight,
            text,
        }));
    }
    out
}

/// The history icons' ink's right edge from the panel's (virtual units;
/// measured on CS:S captures).
const HISTORY_RIGHT: f32 = 20.0;

/// A drawn part, by its place in the drawn list.
#[derive(Component)]
struct SelectPart(usize);

/// Draw the row and the history: one entity per drawn part, reused.
#[allow(clippy::too_many_arguments)]
fn draw(
    hud: Res<ActiveHud>,
    fonts: Res<UiFonts>,
    menu: Res<WeaponMenu>,
    history: Res<History>,
    windows: Query<&Window>,
    player: Option<Single<&Inventory, With<LocalPlayer>>>,
    weapons: Query<&Weapon>,
    mut parts: Query<(
        Entity,
        &SelectPart,
        &mut Node,
        Option<&mut HudText>,
        Option<&mut BackgroundColor>,
    )>,
    mut commands: Commands,
) {
    let Some(window) = windows.iter().next() else { return };
    let (w, h) = (window.width(), window.height());
    let font = |name: &str| fonts.hud_font(name, h);
    let mut drawn = Vec::new();
    if let (Some(alpha), Some(inv), Some(hl)) = (menu.alpha(), player.as_ref(), menu.highlighted)
        && let Ok(hw) = weapons.get(hl)
    {
        let filled: Vec<u8> = inv
            .weapons
            .iter()
            .filter_map(|e| weapons.get(*e).ok())
            .map(|w| w.slot)
            .collect();
        let row = Row {
            filled,
            highlighted: (hw.slot, short(hw.id), inv.active == Some(hl)),
        };
        drawn.extend(selection_layout(&hud.0, &font, w, h, &row, alpha));
    }
    let items: Vec<(&str, f32)> = history
        .items
        .iter()
        .map(|(id, t)| (short(id), ((HISTORY_SECONDS - t) / HISTORY_FADE).clamp(0.0, 1.0)))
        .collect();
    if player.is_some() {
        drawn.extend(history_layout(&hud.0, &font, w, h, &items));
    }
    // Boxes and texts need different entities (a box has a background, a
    // text a material): rebuild when the kinds change, else update.
    let kinds: Vec<bool> = drawn.iter().map(|d| matches!(d, Drawn::Text(_))).collect();
    let mut have: Vec<_> = parts.iter_mut().collect();
    let same_kinds = have.len() == kinds.len() && {
        have.sort_by_key(|(_, i, ..)| i.0);
        have.iter()
            .zip(&kinds)
            .all(|((_, _, _, t, _), is_text)| t.is_some() == *is_text)
    };
    if !same_kinds {
        for (e, ..) in &have {
            commands.entity(*e).despawn();
        }
        for (i, d) in drawn.into_iter().enumerate() {
            let node = Node {
                position_type: PositionType::Absolute,
                ..default()
            };
            match d {
                Drawn::Box(r, c, radius) => commands.spawn((
                    SelectPart(i),
                    Node {
                        left: px(r.min.x),
                        top: px(r.min.y),
                        width: px(r.width()),
                        height: px(r.height()),
                        border_radius: BorderRadius::all(px(radius)),
                        ..node
                    },
                    BackgroundColor(c),
                    GlobalZIndex(42),
                )),
                Drawn::Text(t) => commands.spawn((SelectPart(i), node, t, GlobalZIndex(43))),
            };
        }
        return;
    }
    for ((_, _, node, text, bg), d) in have.iter_mut().zip(drawn) {
        match d {
            Drawn::Box(r, c, radius) => {
                node.left = px(r.min.x);
                node.top = px(r.min.y);
                node.width = px(r.width());
                node.height = px(r.height());
                node.border_radius = BorderRadius::all(px(radius));
                if let Some(bg) = bg
                    && bg.0 != c
                {
                    bg.0 = c;
                }
            }
            Drawn::Text(t) => {
                if let Some(current) = text
                    && **current != t
                {
                    **current = t;
                }
            }
        }
    }
}

/// The width of a text in a scheme font (for tests and layout checks).
pub fn text_width(font: &dyn Fn(&str) -> Option<(GlyphFont, f32)>, name: &str, text: &str) -> Option<f32> {
    font(name).map(|(f, tall)| width(&f.data, tall, text))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slot_keys_pick_the_first_then_cycle() {
        let (a, b, c) = (
            Entity::from_raw_u32(1).unwrap(),
            Entity::from_raw_u32(2).unwrap(),
            Entity::from_raw_u32(3).unwrap(),
        );
        let held = [(a, 0), (b, 3), (c, 3)];
        assert_eq!(next_in_slot(&held, 0, None), Some(a));
        assert_eq!(next_in_slot(&held, 3, None), Some(b));
        assert_eq!(next_in_slot(&held, 3, Some(b)), Some(c));
        assert_eq!(next_in_slot(&held, 3, Some(c)), Some(b));
        // Open on another slot: that slot's first.
        assert_eq!(next_in_slot(&held, 3, Some(a)), Some(b));
        assert_eq!(next_in_slot(&held, 1, None), None);
    }

    #[test]
    fn weapon_names_and_tokens() {
        assert_eq!(short("cs_source:weapon_mp5navy"), "mp5navy");
        assert_eq!(name_token("usp"), Some("Cstrike_WPNHUD_USP45"));
        assert_eq!(name_token("hegrenade"), Some("Cstrike_WPNHUD_HE_Grenade"));
        assert_eq!(name_token("nope"), None);
    }

    #[test]
    fn the_row_fades_at_the_end() {
        let mut m = WeaponMenu {
            shown_for: Some(0.0),
            choosing: true,
            ..default()
        };
        assert_eq!(m.alpha(), Some(1.0));
        m.shown_for = Some(MENU_SECONDS - FADE_SECONDS / 2.0);
        assert!((m.alpha().unwrap() - 0.5).abs() < 1e-4);
        m.shown_for = Some(MENU_SECONDS);
        assert_eq!(m.alpha(), None);
        m.choosing = false;
        m.shown_for = Some(SHOW_SECONDS - 0.01);
        assert!(m.alpha().is_some());
    }
}
