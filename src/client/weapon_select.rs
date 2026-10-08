//! The game HUD's weapon selection (CS:S `HudWeaponSelection`): when the
//! local player switches weapons, a row of boxes at the top right, one per
//! slot that holds something; the drawn weapon's slot is large with its
//! icon (the game's weapon font) and name, the others small with their
//! number. It fades out after a moment, as with CS:S's fast switch.

use bevy::prelude::*;

use super::fonts::UiFonts;
use crate::{
    core::LocalPlayer,
    map::hud::ActiveHud,
    weapon::{Inventory, Weapon, WeaponEvent, WeaponEventKind},
};

pub struct WeaponSelectPlugin;

impl Plugin for WeaponSelectPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Shown>()
            .add_systems(Update, (rebuild, fade).chain().run_if(resource_exists::<ActiveHud>));
    }
}

/// Seconds the row stays, and the last part of that it fades over (our
/// choice; the game's timing isn't in its data files).
const SHOW_SECONDS: f32 = 1.5;
const FADE_SECONDS: f32 = 0.5;

/// Seconds left showing.
#[derive(Resource, Default)]
struct Shown(f32);

#[derive(Component)]
struct SelectPart;

/// A weapon's icon: its kill icon (`d_<weapon>` in mod_textures.txt: font
/// and glyph). CS:S's selection draws the weapon scripts' own icons
/// (another font, letters not shared); those scripts are encrypted.
fn glyph(hud: &crate::map::hud::GameHud, id: &str) -> Option<(String, char)> {
    let short = id.rsplit([':', '_']).next().unwrap_or(id);
    hud.icons.get(&format!("d_{short}")).cloned()
}

/// The boxes left to right: (slot, large, weapon id of a large box).
fn boxes(slots: &[(u8, &'static str)], active: Option<&'static str>, max: u8) -> Vec<(u8, Option<&'static str>)> {
    (0..max)
        .filter(|s| slots.iter().any(|(slot, _)| slot == s))
        .map(|s| {
            let drawn = active.filter(|a| slots.iter().any(|(slot, id)| *slot == s && id == a));
            (s, drawn)
        })
        .collect()
}

#[allow(clippy::too_many_arguments)]
fn rebuild(
    mut events: MessageReader<WeaponEvent>,
    hud: Res<ActiveHud>,
    fonts: Res<UiFonts>,
    player: Option<Single<(Entity, &Inventory), With<LocalPlayer>>>,
    weapons: Query<&Weapon>,
    windows: Query<&Window>,
    old: Query<Entity, With<SelectPart>>,
    mut shown: ResMut<Shown>,
    mut commands: Commands,
) {
    let Some(player) = player else {
        events.clear();
        return;
    };
    let (me, inv) = *player;
    let deployed = events
        .read()
        .any(|e| e.owner == me && matches!(e.kind, WeaponEventKind::Deployed));
    if !deployed {
        return;
    }
    let (Some(panel), Some(window)) = (hud.0.panels.get("HudWeaponSelection"), windows.iter().next()) else {
        return;
    };
    for e in &old {
        commands.entity(e).despawn();
    }
    shown.0 = SHOW_SECONDS;
    let (w, h) = (window.width(), window.height());
    let scale = h / 480.0;
    let small = panel.num("SmallBoxSize").unwrap_or(60.0);
    let (large_w, large_h) = (panel.num("LargeBoxWide").unwrap_or(108.0), panel.num("LargeBoxTall").unwrap_or(80.0));
    let gap = panel.num("BoxGap").unwrap_or(8.0);
    let number_at = Vec2::new(
        panel.num("SelectionNumberXPos").unwrap_or(4.0),
        panel.num("SelectionNumberYPos").unwrap_or(4.0),
    );
    let icon_at = Vec2::new(panel.num("IconXPos").unwrap_or(8.0), panel.num("IconYPos").unwrap_or(0.0));
    let text_y = panel.num("TextYPos").unwrap_or(68.0);
    let max = panel.num("MaxSlots").unwrap_or(5.0) as u8;
    let fg = hud.0.color("SelectionTextFg").or(hud.0.color("FgColor")).unwrap_or(Color::srgb_u8(255, 176, 0));
    let bg = hud.0.color("SelectionBoxBg").unwrap_or(Color::srgba(0.0, 0.0, 0.0, 0.31));
    let held: Vec<(u8, &'static str)> = inv
        .weapons
        .iter()
        .filter_map(|e| weapons.get(*e).ok())
        .map(|w| (w.slot, w.id))
        .collect();
    let active = inv.active.and_then(|e| weapons.get(e).ok()).map(|w| w.id);
    let row = boxes(&held, active, max);
    // Right-aligned in the panel's area, top at its y.
    let total: f32 = row.iter().map(|(_, big)| if big.is_some() { large_w } else { small }).sum::<f32>()
        + gap * row.len().saturating_sub(1) as f32;
    let right = panel.x.resolve(w, scale) + panel.wide * scale;
    let top = panel.y.resolve(h, scale);
    let mut x = right - total * scale;
    // A scheme font: the game's font file, else its system face.
    let font = |name: &str, fallback: f32| match fonts.game(name) {
        Some((handle, tall)) => TextFont {
            font: handle.into(),
            font_size: FontSize::Px(tall * scale),
            ..default()
        },
        None => fonts.client(name, h, fallback),
    };
    for (slot, big) in row {
        let (bw, bh) = if big.is_some() { (large_w, large_h) } else { (small, small) };
        let mut b = commands.spawn((
            SelectPart,
            Node {
                position_type: PositionType::Absolute,
                left: px(x),
                top: px(top),
                width: px(bw * scale),
                height: px(bh * scale),
                border_radius: BorderRadius::all(px(4.0 * scale)),
                ..default()
            },
            BackgroundColor(bg),
            GlobalZIndex(42),
        ));
        b.with_children(|c| {
            let text = |s: String, at: Vec2, font: TextFont| {
                (
                    SelectPart,
                    Text::new(s),
                    font,
                    TextColor(fg),
                    Node {
                        position_type: PositionType::Absolute,
                        left: px(at.x * scale),
                        top: px(at.y * scale),
                        ..default()
                    },
                )
            };
            c.spawn(text((slot + 1).to_string(), number_at, font("HudSelectionNumbers", 11.0)));
            if let Some(id) = big {
                if let Some((font_name, ch)) = glyph(&hud.0, id) {
                    // Sized to fill the box above the name.
                    let f = TextFont {
                        font_size: FontSize::Px(40.0 * scale),
                        ..font(&font_name, 40.0)
                    };
                    c.spawn(text(ch.to_string(), icon_at + Vec2::new(10.0, 20.0), f));
                }
                let name = id.rsplit(':').next().unwrap_or(id).trim_start_matches("weapon_").to_uppercase();
                c.spawn(text(name, Vec2::new(icon_at.x, text_y), font("HudSelectionText", 8.0)));
            }
        });
        x += (bw + gap) * scale;
    }
}

fn fade(
    time: Res<Time>,
    mut shown: ResMut<Shown>,
    parts: Query<Entity, (With<SelectPart>, Without<ChildOf>)>,
    mut colors: Query<(Option<&mut BackgroundColor>, Option<&mut TextColor>), With<SelectPart>>,
    mut commands: Commands,
) {
    if shown.0 <= 0.0 {
        return;
    }
    shown.0 -= time.delta_secs();
    if shown.0 <= 0.0 {
        for e in &parts {
            commands.entity(e).despawn();
        }
        return;
    }
    let a = (shown.0 / FADE_SECONDS).min(1.0);
    for (bg, fg) in &mut colors {
        if let Some(mut bg) = bg {
            bg.0.set_alpha(0.31 * a);
        }
        if let Some(mut fg) = fg {
            fg.0.set_alpha(a);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn boxes_for_held_slots_with_the_drawn_one_large() {
        let held = [(2, "g:weapon_knife"), (1, "g:weapon_usp"), (0, "g:weapon_ak47")];
        let row = boxes(&held, Some("g:weapon_usp"), 5);
        assert_eq!(row, vec![(0, None), (1, Some("g:weapon_usp")), (2, None)]);
        // Nothing in slot 1: skipped.
        let row = boxes(&held[..1], Some("g:weapon_knife"), 5);
        assert_eq!(row, vec![(2, Some("g:weapon_knife"))]);
    }
}
