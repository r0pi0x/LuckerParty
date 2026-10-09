//! The game's own HUD when the map provides one (`map::hud::ActiveHud`; for
//! CS:S, read from its HUD files): health, armour and ammo panels with the
//! game's fonts and icon glyphs, and the death notices, laid out in the
//! virtual 640x480 screen scaled by the window height as Source's
//! proportional HUD is. Without one, `hud` draws the plain HUD.

use std::collections::HashMap;

use bevy::prelude::*;

use super::{
    fonts::UiFonts,
    hud_text::{Align, GlyphFont, HudText, width},
};

use crate::{
    core::{Damage, Died, Health, Hitgroup, LocalPlayer, Team},
    map::hud::{ActiveHud, GameHud, HudCoord, HudPanel},
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

/// Everything this HUD spawned.
#[derive(Component)]
struct GameHudPart;

#[derive(Component, Clone, Copy, PartialEq, Eq, Debug)]
pub enum Part {
    Panel(PanelKind),
    Icon(PanelKind),
    Digits(PanelKind),
    Digits2(PanelKind),
    Bar,
    Notices,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum PanelKind {
    Health,
    Armor,
    Ammo,
    /// Money (rounds).
    Account,
    /// The round clock (rounds).
    Timer,
}

impl PanelKind {
    const ALL: [PanelKind; 5] = [
        PanelKind::Health,
        PanelKind::Armor,
        PanelKind::Ammo,
        PanelKind::Account,
        PanelKind::Timer,
    ];

    pub fn name(self) -> &'static str {
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

/// The health panel's flash when health drops: the game's
/// `HealthTookDamage` HUD animation (scripts/HudAnimations.txt): its
/// colour goes to `HudIcon_Red` over 0.1 s, then pulses 4 times toward
/// `OrangeDim` over a second. (Its blur is left out.)
#[derive(Default)]
struct HealthFlash {
    last: Option<f32>,
    since: Option<f32>,
}

impl HealthFlash {
    const RISE: f32 = 0.1;
    const PULSE: f32 = 1.0;
    const PULSES: f32 = 4.0;

    /// The health colour now, while flashing; `health` normalized.
    fn colour(&mut self, health: Option<f32>, now: f32, red: Color, dim: Color) -> Option<Color> {
        if let (Some(h), Some(last)) = (health, self.last)
            && h < last
            && h > 0.0
        {
            self.since = Some(now);
        }
        self.last = health;
        let t = now - self.since?;
        let mix = |a: Color, b: Color, k: f32| -> Color {
            let (a, b) = (a.to_srgba(), b.to_srgba());
            Color::Srgba(a.mix(&b, k.clamp(0.0, 1.0)))
        };
        if t < Self::RISE {
            Some(mix(dim, red, t / Self::RISE))
        } else if t < Self::RISE + Self::PULSE {
            // VGUI's pulse: toward the target and back, `PULSES` times.
            let k = 0.5 - 0.5 * ((t - Self::RISE) / Self::PULSE * Self::PULSES * std::f32::consts::TAU).cos();
            Some(mix(red, dim, k))
        } else {
            self.since = None;
            None
        }
    }
}

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

fn build(hud: Res<ActiveHud>, old: Query<Entity, With<GameHudPart>>, mut commands: Commands) {
    for e in &old {
        commands.entity(e).despawn();
    }
    let absolute = || Node {
        position_type: PositionType::Absolute,
        ..default()
    };
    for kind in PanelKind::ALL {
        if !hud.0.panels.contains_key(kind.name()) {
            continue;
        }
        commands.spawn((
            GameHudPart,
            Part::Panel(kind),
            absolute(),
            BackgroundColor(Color::NONE),
            GlobalZIndex(39),
        ));
        commands.spawn((GameHudPart, Part::Icon(kind), absolute(), GlobalZIndex(40)));
        commands.spawn((GameHudPart, Part::Digits(kind), absolute(), GlobalZIndex(40)));
        if kind == PanelKind::Ammo {
            commands.spawn((GameHudPart, Part::Digits2(kind), absolute(), GlobalZIndex(40)));
            commands.spawn((
                GameHudPart,
                Part::Bar,
                absolute(),
                BackgroundColor(Color::NONE),
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
    who: Query<(
        Option<&Name>,
        Option<&Team>,
        Has<LocalPlayer>,
        Option<&Inventory>,
        Option<&crate::net::NetCharacter>,
    )>,
    weapons: Query<&Weapon>,
    mut notices: ResMut<DeathNotices>,
    settings: Option<Res<crate::net::NetSettings>>,
    time: Res<Time>,
) {
    let name = |e: Entity| match who.get(e) {
        Ok((name, team, local, _, net)) => (
            super::shown_name(e, local, net, name, settings.as_deref()),
            team.copied(),
        ),
        Err(_) => (format!("{e}"), None),
    };
    for d in died.read() {
        let attacker = d.attacker.filter(|a| *a != d.entity);
        // A thrown grenade names itself; otherwise what the killer holds.
        let weapon = d
            .damage
            .weapon
            .filter(|_| attacker.is_some())
            .or_else(|| {
                attacker
                    .and_then(|a| who.get(a).ok())
                    .and_then(|(_, _, _, inv, _)| inv?.active)
                    .and_then(|w| weapons.get(w).ok())
                    .map(|w| w.id)
            })
            .and_then(|id| {
                let short = id.rsplit([':', '_']).next().unwrap_or(id);
                hud.0.icons.get(&format!("d_{short}")).cloned()
            });
        // (A network client has no damage messages: the killing hit's.)
        let headshot = hits
            .0
            .get(&d.entity)
            .map_or(d.damage.hitgroup == Hitgroup::Head, |(_, g)| *g == Hitgroup::Head);
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
pub(super) fn clock_text(seconds: f32) -> String {
    let s = seconds.ceil() as u32;
    format!("{}:{:02}", s / 60, s % 60)
}

/// What the panels show; None hides a panel.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct HudValues {
    /// Health points (alive), and the colour it is drawn in.
    pub health: Option<(f32, Color)>,
    /// Armour points and helmet (alive; shown at 0 too, as CS:S does).
    pub armor: Option<(f32, bool)>,
    /// Clip and reserve of the drawn weapon.
    pub ammo: Option<(u32, u32)>,
    /// Its ammo type's icon (`ammo_45`).
    pub ammo_icon: Option<&'static str>,
    pub money: Option<u32>,
    /// Round seconds left.
    pub clock: Option<f32>,
}

/// One drawn part of a panel.
#[derive(Clone, Debug, PartialEq)]
pub enum Drawn {
    /// A filled box (pixels), its colour and corner radius.
    Box(Rect, Color, f32),
    Text(HudText),
}

/// Source's proportional value: virtual units (480 lines) to whole pixels,
/// truncated as VGUI does.
pub fn scaled(v: f32, scale: f32) -> f32 {
    (v * scale).trunc()
}

/// A panel's box in a `w` x `h` window, whole pixels: `xpos`/`ypos` from
/// the left (top), centre (`c`) or right (`r`) edge, sizes scaled.
pub fn panel_rect(panel: &HudPanel, w: f32, h: f32) -> Rect {
    let s = h / 480.0;
    let at = |c: HudCoord, length: f32| match c {
        HudCoord::Start(v) => scaled(v, s),
        HudCoord::Centre(v) => (length / 2.0).trunc() + scaled(v, s),
        HudCoord::End(v) => length - scaled(v, s),
    };
    let min = Vec2::new(at(panel.x, w), at(panel.y, h));
    Rect::from_corners(min, min + Vec2::new(scaled(panel.wide, s), scaled(panel.tall, s)))
}

/// An ammo type's icon as CS:S's ammo panel draws it: a glyph of the
/// kill-icon face (`csd.ttf`, the `CSTypeDeath` font's file); None for
/// types it has none for (buckshot), which draw the sprite.
pub fn ammo_glyph(icon: &str) -> Option<char> {
    Some(match icon {
        "ammo_45" => 'M',
        "ammo_556" => 'N',
        "ammo_9mm" => 'R',
        "ammo_57" => 'S',
        "ammo_357" => 'T',
        "ammo_50" => 'U',
        "ammo_762" => 'V',
        "ammo_338" => 'W',
        _ => return None,
    })
}

/// The ammo glyph's cell height and its offset from the panel's
/// `icon_xpos`/`icon_ypos`, virtual units: measured on CS:S captures (the
/// weapon scripts that size it are encrypted).
const AMMO_GLYPH_TALL: f32 = 55.0;
const AMMO_GLYPH_OFFSET: Vec2 = Vec2::new(1.5, 4.5);

/// The corner radius of the panels' rounded boxes (VGUI's
/// `PaintBackgroundType 2`), virtual units (measured on CS:S captures).
const PANEL_CORNER: f32 = 4.0;

/// Each part the HUD panels draw for `values` in a `w` x `h` window: the
/// panels' boxes, icons (the scheme's `Icons` glyphs) and numbers
/// (`HudNumbers`), placed as CS:S places them. `font` resolves a scheme
/// font to a glyph font and its cell height in pixels (`UiFonts::hud_font`).
pub fn layout(
    hud: &GameHud,
    font: &dyn Fn(&str) -> Option<(GlyphFont, f32)>,
    w: f32,
    h: f32,
    values: &HudValues,
) -> Vec<(Part, Drawn)> {
    let s = h / 480.0;
    // Panels draw in the panel colour (`Panel.FgColor`: the dim orange,
    // added to the screen).
    let fg = hud
        .color("Panel.FgColor")
        .or_else(|| hud.color("FgColor"))
        .unwrap_or(Color::srgba_u8(255, 176, 0, 120));
    let mut out = Vec::new();
    for kind in PanelKind::ALL {
        let Some(panel) = hud.panels.get(kind.name()) else { continue };
        let shown = match kind {
            PanelKind::Health => values.health.is_some(),
            PanelKind::Armor => values.armor.is_some(),
            PanelKind::Ammo => values.ammo.is_some(),
            PanelKind::Account => values.money.is_some(),
            PanelKind::Timer => values.clock.is_some(),
        };
        if !shown {
            continue;
        }
        let rect = panel_rect(panel, w, h);
        if let Some(bg) = panel.background {
            out.push((Part::Panel(kind), Drawn::Box(rect, color(bg), PANEL_CORNER * s)));
        }
        let colour = match (kind, values.health) {
            (PanelKind::Health, Some((_, c))) => c,
            _ => fg,
        };
        let at = |v: Vec2| rect.min + Vec2::new(scaled(v.x, s), scaled(v.y, s));
        let mut text = |part: Part, font_name: &str, text: String, pos: Vec2, align: Align| {
            if let Some((font, tall)) = font(font_name) {
                out.push((
                    part,
                    Drawn::Text(HudText {
                        font,
                        tall,
                        text,
                        color: colour,
                        at: pos,
                        align,
                    }),
                ));
            }
        };
        // Numbers of up to three digits sit right-aligned in a three-digit
        // field starting at their position (CS:S's armour "0" sits where
        // health's last digit does).
        let field = |at: Vec2| {
            let digits = font("HudNumbers").map_or(0.0, |(f, tall)| width(&f.data, tall, "000"));
            at + Vec2::X * digits
        };
        let icon_name = match kind {
            PanelKind::Health => Some("health_icon"),
            PanelKind::Armor => Some(if values.armor.is_some_and(|a| a.1) {
                "shield_kevlar"
            } else {
                "shield"
            }),
            PanelKind::Account => Some("dollar_sign"),
            PanelKind::Timer => Some("timer_icon"),
            PanelKind::Ammo => None,
        };
        if let Some((icon_font, ch)) = icon_name.and_then(|n| hud.icons.get(n)) {
            text(Part::Icon(kind), icon_font, ch.to_string(), at(panel.icon), Align::Left);
        }
        match kind {
            PanelKind::Health => {
                let v = values.health.map_or(0.0, |h| h.0).ceil();
                text(Part::Digits(kind), "HudNumbers", format!("{v:.0}"), field(at(panel.digit)), Align::Right);
            }
            PanelKind::Armor => {
                let v = values.armor.map_or(0.0, |a| a.0).round();
                text(Part::Digits(kind), "HudNumbers", format!("{v:.0}"), field(at(panel.digit)), Align::Right);
            }
            PanelKind::Ammo => {
                let (clip, reserve) = values.ammo.unwrap_or_default();
                text(Part::Digits(kind), "HudNumbers", clip.to_string(), field(at(panel.digit)), Align::Right);
                text(
                    Part::Digits2(kind),
                    "HudNumbers",
                    reserve.to_string(),
                    field(at(panel.digit2)),
                    Align::Right,
                );
                // The bar between clip and reserve.
                let num = |k: &str, d: f32| panel.num(k).unwrap_or(d);
                let min = at(Vec2::new(num("bar_xpos", 53.0), num("bar_ypos", 3.0)));
                let size = Vec2::new(scaled(num("bar_width", 2.0), s), scaled(num("bar_height", 20.0), s));
                out.push((Part::Bar, Drawn::Box(Rect::from_corners(min, min + size), fg, 0.0)));
            }
            PanelKind::Account => {
                let v = values.money.unwrap_or(0);
                // Right-aligned: the digits end at their position.
                text(Part::Digits(kind), "HudNumbers", v.to_string(), at(panel.digit), Align::Right);
            }
            PanelKind::Timer => {
                let v = values.clock.map_or(String::new(), clock_text);
                text(Part::Digits(kind), "HudNumbers", v, at(panel.digit), Align::Left);
            }
        }
        if kind == PanelKind::Ammo
            && let Some(c) = values.ammo_icon.and_then(ammo_glyph)
            && let Some((f, _)) = font("CSTypeDeath")
        {
            out.push((
                Part::Icon(kind),
                Drawn::Text(HudText {
                    font: f,
                    tall: super::hud_text::proportional_tall(AMMO_GLYPH_TALL, h),
                    text: c.to_string(),
                    color: colour,
                    at: at(panel.icon + AMMO_GLYPH_OFFSET),
                    align: Align::Left,
                }),
            ));
        }
    }
    out
}

/// Lay the HUD out for the window and fill in the values.
#[allow(clippy::type_complexity)]
fn update(
    hud: Res<ActiveHud>,
    fonts: Res<UiFonts>,
    windows: Query<&Window>,
    player: Option<Single<LocalState, With<LocalPlayer>>>,
    weapons: Query<(&Weapon, Option<&Magazine>)>,
    notices: Res<DeathNotices>,
    rounds: Option<Res<RoundState>>,
    bomb: Option<Res<crate::objectives::bomb::BombState>>,
    (time, clock_now): (Res<Time<Fixed>>, Res<Time>),
    bars: Option<Res<super::spectate::SpectatorBarsUp>>,
    mut built: Local<(u64, f32)>,
    mut flash: Local<HealthFlash>,
    mut parts: Query<(
        Entity,
        &Part,
        &mut Node,
        Option<&mut HudText>,
        Option<&mut BackgroundColor>,
        &mut Visibility,
    )>,
    mut commands: Commands,
) {
    let Some(window) = windows.iter().next() else {
        return;
    };
    let (w, h) = (window.width(), window.height());
    let scale = h / 480.0;
    let hud = &hud.0;
    let fg = hud.color("Panel.FgColor").unwrap_or(Color::srgba_u8(255, 176, 0, 120));
    let warn = hud.color("HudIcon_Red").unwrap_or(Color::srgb_u8(160, 0, 0));
    let health_now = player.as_ref().map(|p| p.0.current);
    let health_colour = flash.colour(
        health_now,
        clock_now.elapsed_secs(),
        warn,
        hud.color("OrangeDim").unwrap_or(fg),
    );
    let (health, armor, active, dead, money) = match &player {
        Some(p) => {
            let (h, a, inv, dead, money) = **p;
            (
                Some(h.current * 100.0),
                a.map(|a| (a.amount * 100.0, a.helmet && a.amount > 0.0)),
                inv.and_then(|i| i.active).and_then(|e| weapons.get(e).ok()),
                dead,
                money.map(|m| m.0),
            )
        }
        None => (None, None, None, true, None),
    };
    // A planted bomb replaces the round clock (spec objectives.md 4,
    // *hyp.*: the scenario icon shows instead).
    let planted = bomb.is_some_and(|b| b.planted.is_some());
    let clock = rounds
        .as_ref()
        .and_then(|r| r.clock(time.elapsed_secs_f64()))
        .filter(|_| !planted);
    let ammo = active.and_then(|(_, m)| m).map(|m| (m.clip, m.reserve));
    let ammo_icon = active.and_then(|(w, _)| super::hud_sprites::ammo_icon(w.id));
    let bars_top = bars.and_then(|b| b.0);
    // Spectating with the game's bars: they show the clock; the money goes
    // with the rest of the player's HUD.
    let spectating = bars_top.is_some();
    let low = health.is_some_and(|v| v <= LOW_HEALTH);
    let values = HudValues {
        health: health.filter(|_| !dead).map(|v| {
            let c = match health_colour {
                Some(c) => c,
                None if low => warn,
                None => fg,
            };
            (v, c)
        }),
        armor: if dead { None } else { Some(armor.unwrap_or((0.0, false))) },
        ammo: ammo.filter(|_| !dead),
        ammo_icon,
        money: money.filter(|_| !spectating),
        clock: clock.filter(|_| !spectating),
    };
    let drawn = layout(hud, &|name| fonts.hud_font(name, h), w, h, &values);
    for (entity, part, mut node, text, background, mut vis) in &mut parts {
        if *part == Part::Notices {
            let Some(panel) = hud.panels.get("HudDeathNotice") else {
                continue;
            };
            node.right = px(w - (panel.x.resolve(w, scale) + panel.wide * scale));
            // Below the spectator's top bar while it shows.
            node.top = px(panel.y.resolve(h, scale).max(bars_top.map_or(0.0, |t| t + 4.0 * scale)));
            if *built != (notices.1, scale) {
                *built = (notices.1, scale);
                rebuild_notices(entity, &notices, hud, &fonts, scale, &mut commands);
            }
            continue;
        }
        let item = drawn.iter().find(|(p, _)| p == part).map(|(_, d)| d);
        *vis = if item.is_some() {
            Visibility::Inherited
        } else {
            Visibility::Hidden
        };
        match item {
            Some(Drawn::Box(r, c, radius)) => {
                node.left = px(r.min.x);
                node.top = px(r.min.y);
                node.width = px(r.width());
                node.height = px(r.height());
                node.border_radius = BorderRadius::all(px(*radius));
                if let Some(mut b) = background
                    && b.0 != *c
                {
                    b.0 = *c;
                }
            }
            Some(Drawn::Text(t)) => match text {
                Some(mut current) => {
                    if *current != *t {
                        *current = t.clone();
                    }
                }
                None => {
                    commands.entity(entity).insert(t.clone());
                }
            },
            None => {}
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
    fonts: Res<UiFonts>,
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
    let text = end.reason.text(end.winner);
    let h = windows.iter().next().map_or(480.0, |w| w.height());
    for e in &banner {
        commands.entity(e).despawn();
    }
    commands.spawn((
        RoundBanner,
        Text::new(text),
        // A centre print: the client scheme's CenterPrintText.
        fonts.client("CenterPrintText", h, 18.0),
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

/// The notice lines: attacker, weapon glyph, headshot glyph, victim, in
/// team colours, right-justified.
fn rebuild_notices(
    root: Entity,
    notices: &DeathNotices,
    hud: &GameHud,
    fonts: &UiFonts,
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
    // HudDeathNotice's TextFont: the client scheme's Default.
    let text_font = fonts.client(
        hud.panels
            .get("HudDeathNotice")
            .and_then(|p| p.keys.get("textfont"))
            .map_or("Default", String::as_str),
        scale * 480.0,
        12.0,
    );
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
                None => e.insert(text_font.clone()),
            };
        };
        if let Some((name, team)) = &n.attacker {
            word(name.clone(), team_color(*team), None);
        }
        let glyph = |name: &str| hud.icons.get(name).cloned();
        let weapon = n.weapon.clone().or_else(|| glyph("d_skull_cs"));
        if let Some((font, ch)) = weapon {
            word(ch.to_string(), Color::WHITE, fonts.game(&font));
        }
        if n.headshot
            && let Some((font, ch)) = glyph("d_headshot")
        {
            word(ch.to_string(), Color::WHITE, fonts.game(&font));
        }
        word(n.victim.0.clone(), team_color(n.victim.1), None);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn health_flashes_red_when_it_drops() {
        let (red, dim) = (Color::srgb(1.0, 0.0, 0.0), Color::srgb(1.0, 0.7, 0.0));
        let mut f = HealthFlash::default();
        assert_eq!(f.colour(Some(1.0), 0.0, red, dim), None);
        // A drop starts it: red after the 0.1 s rise.
        assert_eq!(f.colour(Some(0.8), 1.0, red, dim), Some(dim));
        let at_rise = f.colour(Some(0.8), 1.1, red, dim).unwrap().to_srgba();
        assert!((at_rise.red - 1.0).abs() < 1e-4 && at_rise.green < 1e-4, "{at_rise:?}");
        // Half a pulse later (1/8 s): the dim colour.
        let mid = f.colour(Some(0.8), 1.225, red, dim).unwrap().to_srgba();
        assert!((mid.green - 0.7).abs() < 1e-3, "{mid:?}");
        // Over after a second of pulses; healing never flashes.
        assert_eq!(f.colour(Some(0.8), 2.2, red, dim), None);
        assert_eq!(f.colour(Some(1.0), 2.3, red, dim), None);
    }

    #[test]
    fn round_clock_reads_like_the_game() {
        assert_eq!(clock_text(300.0), "5:00");
        assert_eq!(clock_text(59.2), "1:00");
        assert_eq!(clock_text(5.0), "0:05");
    }
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
            keys: Default::default(),
        };
        // 1440 lines: 3 pixels per unit.
        let s = 1440.0 / 480.0;
        assert_eq!(p.x.resolve(2560.0, s), 2560.0 - 471.0);
        assert_eq!(p.y.resolve(1440.0, s), 1338.0);
    }
}
