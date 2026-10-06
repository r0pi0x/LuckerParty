//! The game's own HUD when the map provides one (`map::hud::ActiveHud`; for
//! CS:S, read from its HUD files): health, armour and ammo panels with the
//! game's fonts and icon glyphs, and the death notices, laid out in the
//! virtual 640x480 screen scaled by the window height as Source's
//! proportional HUD is. Without one, `hud` draws the plain HUD.

use std::collections::HashMap;

use bevy::prelude::*;

use crate::{
    core::{Damage, Died, Health, Hitgroup, LocalPlayer, Team},
    map::hud::{ActiveHud, GameHud},
    rules::Dead,
    rules::rounds::RoundState,
    weapon::{Armor, Inventory, Magazine, Weapon, economy::Money},
};

pub struct GameHudPlugin;

impl Plugin for GameHudPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<DeathNotices>()
            .init_resource::<LastHits>()
            .add_systems(
                Update,
                (
                    build.run_if(resource_exists_and_changed::<ActiveHud>),
                    teardown.run_if(resource_removed::<ActiveHud>),
                    (remember_hits, death_notices, update)
                        .chain()
                        .run_if(resource_exists::<ActiveHud>),
                    round_banner,
                ),
            );
    }
}

/// Font handles made from the HUD's font files, by the game's font name.
#[derive(Resource, Default)]
struct HudFonts(HashMap<String, (Handle<Font>, f32)>);

/// Everything this HUD spawned.
#[derive(Component)]
struct GameHudPart;

#[derive(Component, Clone, Copy, PartialEq, Eq)]
enum Part {
    Panel(PanelKind),
    Icon(PanelKind),
    Digits(PanelKind),
    Digits2(PanelKind),
    Bar,
    Notices,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum PanelKind {
    Health,
    Armor,
    Ammo,
    /// Money (rounds).
    Account,
    /// The round clock (rounds).
    Timer,
}

impl PanelKind {
    fn name(self) -> &'static str {
        match self {
            PanelKind::Health => "HudHealth",
            PanelKind::Armor => "HudArmor",
            PanelKind::Ammo => "HudAmmo",
            PanelKind::Account => "HudAccount",
            PanelKind::Timer => "HudRoundTimer",
        }
    }
}

/// Health at or below this shows in the warning colour (an assumption: the
/// game's own threshold isn't in its data files).
const LOW_HEALTH: f32 = 25.0;
/// Seconds a death notice stays (CS:S's `hud_deathnotice_time` default).
const NOTICE_SECONDS: f32 = 6.0;
const MAX_NOTICES: usize = 4;
const NOTICE_LINE: f32 = 22.0;

/// A kill to show: attacker, victim, weapon icon glyph, headshot.
struct Notice {
    attacker: Option<(String, Option<Team>)>,
    victim: (String, Option<Team>),
    weapon: Option<(String, char)>,
    headshot: bool,
    left: f32,
}

/// The notices shown, and a counter bumped whenever they change.
#[derive(Resource, Default)]
struct DeathNotices(Vec<Notice>, u64);

/// Each character's last damage: who, and where it hit.
#[derive(Resource, Default)]
struct LastHits(HashMap<Entity, (Option<Entity>, Hitgroup)>);

fn color(c: [u8; 4]) -> Color {
    Color::srgba_u8(c[0], c[1], c[2], c[3])
}

fn build(
    hud: Res<ActiveHud>,
    mut fonts: ResMut<Assets<Font>>,
    old: Query<Entity, With<GameHudPart>>,
    mut commands: Commands,
) {
    for e in &old {
        commands.entity(e).despawn();
    }
    let handles = hud
        .0
        .fonts
        .iter()
        .map(|(name, f)| (name.clone(), (fonts.add(Font::from_bytes(f.data.to_vec())), f.tall)))
        .collect();
    commands.insert_resource(HudFonts(handles));
    let fg = hud.0.color("FgColor").unwrap_or(Color::srgb_u8(255, 176, 0));
    let spawn_text = |commands: &mut Commands, part: Part| {
        commands.spawn((
            GameHudPart,
            part,
            Text::default(),
            TextColor(fg),
            Node {
                position_type: PositionType::Absolute,
                ..default()
            },
            GlobalZIndex(40),
        ));
    };
    for kind in [
        PanelKind::Health,
        PanelKind::Armor,
        PanelKind::Ammo,
        PanelKind::Account,
        PanelKind::Timer,
    ] {
        let Some(panel) = hud.0.panels.get(kind.name()) else {
            continue;
        };
        commands.spawn((
            GameHudPart,
            Part::Panel(kind),
            Node {
                position_type: PositionType::Absolute,
                ..default()
            },
            BackgroundColor(panel.background.map(color).unwrap_or(Color::NONE)),
            GlobalZIndex(39),
        ));
        spawn_text(&mut commands, Part::Icon(kind));
        spawn_text(&mut commands, Part::Digits(kind));
        if kind == PanelKind::Ammo {
            spawn_text(&mut commands, Part::Digits2(kind));
            commands.spawn((
                GameHudPart,
                Part::Bar,
                Node {
                    position_type: PositionType::Absolute,
                    ..default()
                },
                BackgroundColor(fg),
                GlobalZIndex(40),
            ));
        }
    }
    commands.spawn((
        GameHudPart,
        Part::Notices,
        Node {
            position_type: PositionType::Absolute,
            flex_direction: FlexDirection::Column,
            align_items: AlignItems::FlexEnd,
            ..default()
        },
        GlobalZIndex(40),
    ));
}

fn teardown(parts: Query<Entity, With<GameHudPart>>, mut commands: Commands) {
    for e in &parts {
        commands.entity(e).despawn();
    }
    commands.remove_resource::<HudFonts>();
}

fn remember_hits(mut damage: MessageReader<Damage>, mut hits: ResMut<LastHits>) {
    for d in damage.read() {
        hits.0.insert(d.target, (d.attacker, d.hitgroup));
    }
}

#[allow(clippy::type_complexity)]
fn death_notices(
    mut died: MessageReader<Died>,
    hud: Res<ActiveHud>,
    hits: Res<LastHits>,
    who: Query<(Option<&Name>, Option<&Team>, Has<LocalPlayer>, Option<&Inventory>)>,
    weapons: Query<&Weapon>,
    mut notices: ResMut<DeathNotices>,
    time: Res<Time>,
) {
    let name = |e: Entity| match who.get(e) {
        Ok((name, team, local, _)) => (
            if local {
                "Player".to_string()
            } else {
                name.map_or_else(|| format!("{e}"), |n| n.to_string())
            },
            team.copied(),
        ),
        Err(_) => (format!("{e}"), None),
    };
    for d in died.read() {
        let attacker = d.attacker.filter(|a| *a != d.entity);
        let weapon = attacker
            .and_then(|a| who.get(a).ok())
            .and_then(|(.., inv)| inv?.active)
            .and_then(|w| weapons.get(w).ok())
            .and_then(|w| {
                let short = w.id.rsplit([':', '_']).next().unwrap_or(w.id);
                hud.0.icons.get(&format!("d_{short}")).cloned()
            });
        let headshot = hits.0.get(&d.entity).is_some_and(|(_, g)| *g == Hitgroup::Head);
        notices.0.push(Notice {
            attacker: attacker.map(name),
            victim: name(d.entity),
            weapon,
            headshot,
            left: NOTICE_SECONDS,
        });
        notices.1 += 1;
    }
    let dt = time.delta_secs();
    let before = notices.0.len();
    notices.0.retain_mut(|n| {
        n.left -= dt;
        n.left > 0.0
    });
    if notices.0.len() > MAX_NOTICES {
        let extra = notices.0.len() - MAX_NOTICES;
        notices.0.drain(..extra);
    }
    if notices.0.len() != before {
        notices.1 += 1;
    }
}

type LocalState<'a> = (
    &'a Health,
    Option<&'a Armor>,
    Option<&'a Inventory>,
    Has<Dead>,
    Option<&'a Money>,
);

/// The round clock as the game shows it: minutes and seconds, rounded up.
fn clock_text(seconds: f32) -> String {
    let s = seconds.ceil() as u32;
    format!("{}:{:02}", s / 60, s % 60)
}

/// Lay the HUD out for the window and fill in the values.
#[allow(clippy::type_complexity)]
fn update(
    hud: Res<ActiveHud>,
    fonts: Option<Res<HudFonts>>,
    windows: Query<&Window>,
    player: Option<Single<LocalState, With<LocalPlayer>>>,
    weapons: Query<(&Weapon, Option<&Magazine>)>,
    notices: Res<DeathNotices>,
    rounds: Option<Res<RoundState>>,
    time: Res<Time<Fixed>>,
    mut built: Local<(u64, f32)>,
    mut parts: Query<(
        Entity,
        &Part,
        &mut Node,
        Option<&mut Text>,
        Option<&mut TextFont>,
        Option<&mut TextColor>,
        &mut Visibility,
    )>,
    mut commands: Commands,
) {
    let (Some(fonts), Some(window)) = (fonts, windows.iter().next()) else {
        return;
    };
    let (w, h) = (window.width(), window.height());
    let scale = h / 480.0;
    let hud = &hud.0;
    let fg = hud.color("FgColor").unwrap_or(Color::srgb_u8(255, 176, 0));
    let warn = Color::srgb_u8(255, 0, 0);
    let font = |name: &str| fonts.0.get(name).cloned();
    let (health, armor, active, dead, money) = match &player {
        Some(p) => {
            let (h, a, inv, dead, money) = **p;
            (
                Some(h.current * 100.0),
                a.filter(|a| a.amount > 0.0).map(|a| (a.amount * 100.0, a.helmet)),
                inv.and_then(|i| i.active).and_then(|e| weapons.get(e).ok()),
                dead,
                money.map(|m| m.0),
            )
        }
        None => (None, None, None, true, None),
    };
    let clock = rounds.as_ref().and_then(|r| r.clock(time.elapsed_secs_f64()));
    let ammo = active.and_then(|(_, m)| m).map(|m| (m.clip, m.reserve));
    for (entity, part, mut node, text, text_font, text_color, mut vis) in &mut parts {
        let kind = match part {
            Part::Panel(k) | Part::Icon(k) | Part::Digits(k) | Part::Digits2(k) => Some(*k),
            Part::Bar => Some(PanelKind::Ammo),
            Part::Notices => None,
        };
        let shown = match kind {
            Some(PanelKind::Health) => !dead,
            Some(PanelKind::Armor) => !dead && armor.is_some(),
            Some(PanelKind::Ammo) => !dead && ammo.is_some(),
            Some(PanelKind::Account) => money.is_some(),
            Some(PanelKind::Timer) => clock.is_some(),
            None => true,
        };
        *vis = if shown {
            Visibility::Inherited
        } else {
            Visibility::Hidden
        };
        if !shown {
            continue;
        }
        if *part == Part::Notices {
            let Some(panel) = hud.panels.get("HudDeathNotice") else {
                continue;
            };
            node.right = px(w - (panel.x.resolve(w, scale) + panel.wide * scale));
            node.top = px(panel.y.resolve(h, scale));
            if *built != (notices.1, scale) {
                *built = (notices.1, scale);
                rebuild_notices(entity, &notices, hud, &fonts, scale, &mut commands);
            }
            continue;
        }
        let Some(kind) = kind else { continue };
        let Some(panel) = hud.panels.get(kind.name()) else {
            continue;
        };
        let origin = Vec2::new(panel.x.resolve(w, scale), panel.y.resolve(h, scale));
        let place = |node: &mut Node, at: Vec2| {
            node.left = px(origin.x + at.x * scale);
            node.top = px(origin.y + at.y * scale);
        };
        let low = health.is_some_and(|v| v <= LOW_HEALTH);
        match part {
            Part::Panel(_) => {
                place(&mut node, Vec2::ZERO);
                node.width = px(panel.wide * scale);
                node.height = px(panel.tall * scale);
                node.border_radius = BorderRadius::all(px(4.0 * scale));
            }
            Part::Bar => {
                // The divider between clip and reserve, as in the game.
                let x = (panel.digit.x + panel.digit2.x) / 2.0 + 2.0;
                place(&mut node, Vec2::new(x, 4.0));
                node.width = px(2.0 * scale);
                node.height = px((panel.tall - 8.0) * scale);
            }
            Part::Icon(_) | Part::Digits(_) | Part::Digits2(_) => {
                let (font_name, value, at) = match (part, kind) {
                    (Part::Icon(PanelKind::Health), _) => icon(hud, "health_icon"),
                    (Part::Icon(PanelKind::Armor), _) => icon(
                        hud,
                        if armor.is_some_and(|a| a.1) {
                            "shield_kevlar"
                        } else {
                            "shield"
                        },
                    ),
                    (Part::Digits(PanelKind::Health), _) => (
                        "HudNumbers".into(),
                        format!("{:.0}", health.unwrap_or(0.0).ceil()),
                        panel.digit,
                    ),
                    (Part::Digits(PanelKind::Armor), _) => (
                        "HudNumbers".into(),
                        format!("{:.0}", armor.map_or(0.0, |a| a.0).round()),
                        panel.digit,
                    ),
                    (Part::Digits(PanelKind::Ammo), _) => (
                        "HudNumbers".into(),
                        ammo.map_or(String::new(), |a| a.0.to_string()),
                        panel.digit,
                    ),
                    (Part::Icon(PanelKind::Account), _) => icon(hud, "dollar_sign"),
                    (Part::Icon(PanelKind::Timer), _) => icon(hud, "timer_icon"),
                    (Part::Digits(PanelKind::Account), _) => (
                        "HudNumbers".into(),
                        money.map_or(String::new(), |m| m.to_string()),
                        panel.digit,
                    ),
                    (Part::Digits(PanelKind::Timer), _) => (
                        "HudNumbers".into(),
                        clock.map_or(String::new(), clock_text),
                        panel.digit,
                    ),
                    (Part::Digits2(_), _) => (
                        "HudNumbers".into(),
                        ammo.map_or(String::new(), |a| a.1.to_string()),
                        panel.digit2,
                    ),
                    _ => (String::new(), String::new(), Vec2::ZERO),
                };
                let at = if matches!(part, Part::Icon(_)) { panel.icon } else { at };
                place(&mut node, at);
                // The account's digits end at their position (right-aligned).
                if kind == PanelKind::Account && matches!(part, Part::Digits(_)) {
                    node.right = px(w - (origin.x + at.x * scale));
                    node.left = Val::Auto;
                }
                if let Some(mut t) = text
                    && t.0 != value
                {
                    t.0 = value;
                }
                if let (Some(mut tf), Some((handle, tall))) = (text_font, font(&font_name)) {
                    tf.font = handle.into();
                    tf.font_size = FontSize::Px(tall * scale);
                }
                if let Some(mut c) = text_color {
                    c.0 = if kind == PanelKind::Health && low { warn } else { fg };
                }
            }
            Part::Notices => {}
        }
    }
}

/// The round's result across the screen ("Terrorists Win!") until the
/// next round starts.
#[derive(Component)]
struct RoundBanner;

fn round_banner(
    mut ended: MessageReader<crate::rules::rounds::RoundEnded>,
    rounds: Option<Res<RoundState>>,
    banner: Query<Entity, With<RoundBanner>>,
    windows: Query<&Window>,
    sounds: Option<Res<crate::map::RoundSounds>>,
    mut play: MessageWriter<crate::map::PlaySound>,
    mut was_live: Local<bool>,
    mut commands: Commands,
) {
    use crate::rules::rounds::{ATTACKERS, DEFENDERS, Phase};
    // "Let's go!" as a round goes live.
    let live = rounds.as_ref().is_some_and(|r| matches!(r.phase, Phase::Live { .. }));
    if live
        && !*was_live
        && let Some(s) = sounds.as_ref().filter(|s| !s.start.is_empty())
    {
        let n = rounds.as_ref().map_or(0, |r| r.number as usize);
        play.write(crate::map::PlaySound::ui(s.start[n % s.start.len()].clone()));
    }
    *was_live = live;
    let over = rounds.is_some_and(|r| matches!(r.phase, Phase::Over { .. }));
    if !over {
        for e in &banner {
            commands.entity(e).despawn();
        }
    }
    let Some(end) = ended.read().last() else { return };
    if let Some(entry) = sounds.as_ref().and_then(|s| match end.winner {
        Some(ATTACKERS) => s.attackers_win.clone(),
        Some(DEFENDERS) => s.defenders_win.clone(),
        _ => s.draw.clone(),
    }) {
        play.write(crate::map::PlaySound::ui(entry));
    }
    let text = match end.winner {
        Some(ATTACKERS) => "Terrorists Win!",
        Some(DEFENDERS) => "Counter-Terrorists Win!",
        _ => "Round Draw!",
    };
    let scale = windows.iter().next().map_or(1.0, |w| w.height() / 480.0);
    for e in &banner {
        commands.entity(e).despawn();
    }
    commands.spawn((
        RoundBanner,
        Text::new(text),
        TextFont {
            font_size: FontSize::Px(18.0 * scale),
            ..default()
        },
        TextColor(Color::WHITE),
        TextShadow::default(),
        Node {
            position_type: PositionType::Absolute,
            top: Val::Percent(28.0),
            width: Val::Percent(100.0),
            justify_content: JustifyContent::Center,
            ..default()
        },
        TextLayout::justify(Justify::Center),
        GlobalZIndex(45),
    ));
}

/// An icon's font and glyph (and an unused offset).
fn icon(hud: &GameHud, name: &str) -> (String, String, Vec2) {
    match hud.icons.get(name) {
        Some((font, ch)) => (font.clone(), ch.to_string(), Vec2::ZERO),
        None => (String::new(), String::new(), Vec2::ZERO),
    }
}

/// The notice lines: attacker, weapon glyph, headshot glyph, victim, in
/// team colours, right-justified.
fn rebuild_notices(
    root: Entity,
    notices: &DeathNotices,
    hud: &GameHud,
    fonts: &HudFonts,
    scale: f32,
    commands: &mut Commands,
) {
    commands.entity(root).despawn_related::<Children>();
    let team_color = |t: Option<Team>| {
        match t.map(|t| t.0) {
            Some(1) => hud.color("T_Red"),
            Some(2) => hud.color("CT_Blue"),
            _ => None,
        }
        .unwrap_or(Color::WHITE)
    };
    let text_size = 9.0 * scale;
    for n in &notices.0 {
        let row = commands
            .spawn((
                Node {
                    flex_direction: FlexDirection::Row,
                    align_items: AlignItems::Center,
                    height: px(NOTICE_LINE * scale),
                    padding: UiRect::horizontal(px(4.0 * scale)),
                    column_gap: px(4.0 * scale),
                    border_radius: BorderRadius::all(px(3.0 * scale)),
                    ..default()
                },
                BackgroundColor(Color::srgba(0.0, 0.0, 0.0, 0.4)),
                ChildOf(root),
            ))
            .id();
        let mut word = |text: String, color: Color, font: Option<(Handle<Font>, f32)>| {
            let mut e = commands.spawn((Text::new(text), TextColor(color), ChildOf(row)));
            match font {
                // Icon fonts draw their glyphs high in the line: lower
                // them to sit with the names.
                Some((handle, tall)) => e.insert((
                    TextFont {
                        font: handle.into(),
                        font_size: FontSize::Px(tall * scale * 0.5),
                        ..default()
                    },
                    Node {
                        margin: UiRect::top(px(tall * scale * 0.25)),
                        ..default()
                    },
                )),
                None => e.insert(TextFont {
                    font_size: FontSize::Px(text_size),
                    ..default()
                }),
            };
        };
        if let Some((name, team)) = &n.attacker {
            word(name.clone(), team_color(*team), None);
        }
        let glyph = |name: &str| hud.icons.get(name).cloned();
        let weapon = n.weapon.clone().or_else(|| glyph("d_skull_cs"));
        if let Some((font, ch)) = weapon {
            word(ch.to_string(), Color::WHITE, fonts.0.get(&font).cloned());
        }
        if n.headshot
            && let Some((font, ch)) = glyph("d_headshot")
        {
            word(ch.to_string(), Color::WHITE, fonts.0.get(&font).cloned());
        }
        word(n.victim.0.clone(), team_color(n.victim.1), None);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_clock_reads_like_the_game() {
        assert_eq!(clock_text(300.0), "5:00");
        assert_eq!(clock_text(59.2), "1:00");
        assert_eq!(clock_text(5.0), "0:05");
    }
    use crate::map::hud::{HudCoord, HudPanel};

    #[test]
    fn panels_scale_with_window_height() {
        let p = HudPanel {
            x: HudCoord::End(157.0),
            y: HudCoord::Start(446.0),
            wide: 142.0,
            tall: 25.0,
            background: None,
            icon: Vec2::ZERO,
            digit: Vec2::new(8.0, -4.0),
            digit2: Vec2::ZERO,
        };
        // 1440 lines: 3 pixels per unit.
        let s = 1440.0 / 480.0;
        assert_eq!(p.x.resolve(2560.0, s), 2560.0 - 471.0);
        assert_eq!(p.y.resolve(1440.0, s), 1338.0);
    }
}
