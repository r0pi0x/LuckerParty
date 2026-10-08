//! The objectives on the HUD (specs/cs_source/objectives.md, HUD table):
//! the carried-bomb / defusal-kit / rescue-zone icon (`HudC4`,
//! `HudDefuser`, `HudHostageRescueZone`), the scenario icon (a planted
//! bomb, the hostages left: `HudScenarioIcon`), the planting and defusing
//! progress bar (`HudProgressBar`), the game's messages (centre text for
//! everyone, hints for the local player, chat lines for the team), the
//! game's deny sound when the local player's +use finds nothing (neither a
//! map door or button nor the bomb or a hostage; spec 5, step 4), and
//! `mashup_objectives 1`, a debug readout of the bomb and hostages.

use bevy::prelude::*;

use super::{
    chat::{ChatLine, Hint},
    fonts::UiFonts,
};
use crate::{
    console::resource_cvar,
    core::{Health, LocalPlayer, MovementState, Team},
    map::hud::{ActiveHud, HudCoord},
    objectives::{
        MapKind, MapObjectives, ObjectiveEvent,
        bomb::{Arming, BombRules, BombState, PlantedBomb, hull},
        hostages::{Hostage, HostagePenalty, HostageTally},
    },
    weapon::economy::DefuseKit,
};

pub struct ObjectivesHudPlugin;

impl Plugin for ObjectivesHudPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Overlay>()
            .init_resource::<Centre>()
            .add_message::<crate::weapon::drop::UsedPickup>()
            .add_systems(Startup, spawn)
            .add_systems(Update, (messages, status, progress, centre, overlay))
            .add_systems(
                FixedUpdate,
                use_deny
                    .after(crate::core::SimSet::Weapons)
                    .after(crate::logic::LogicSet::Pre),
            );
        resource_cvar::<Overlay, u8>(
            app,
            "mashup_objectives",
            "1: show the bomb's timer and defuse and the hostages' state (debug).",
            |o| &mut o.0,
        );
    }
}

#[derive(Resource, Default)]
struct Overlay(u8);

/// The centre message and until when it shows.
#[derive(Resource, Default)]
struct Centre(Option<(String, f32)>);

/// Seconds a centre message stays.
const CENTRE_SECONDS: f32 = 4.0;

#[derive(Component)]
enum Part {
    Status,
    Scenario,
    Frame,
    Centre,
    Overlay,
}

fn spawn(mut commands: Commands, fonts: Res<UiFonts>) {
    let abs = || Node {
        position_type: PositionType::Absolute,
        ..default()
    };
    for part in [Part::Status, Part::Scenario] {
        commands.spawn((
            part,
            Text::default(),
            TextColor(Color::WHITE),
            abs(),
            Visibility::Hidden,
            GlobalZIndex(40),
        ));
    }
    commands
        .spawn((
            Part::Frame,
            abs(),
            BorderColor::all(Color::srgb_u8(255, 30, 13)),
            BackgroundColor(Color::srgba(0.0, 0.0, 0.0, 0.3)),
            Visibility::Hidden,
            GlobalZIndex(42),
        ))
        .with_children(|f| {
            f.spawn((
                FillMarker,
                Node {
                    height: Val::Percent(100.0),
                    ..default()
                },
                BackgroundColor(Color::srgb_u8(255, 30, 13)),
            ));
        });
    commands.spawn((
        Part::Centre,
        Text::default(),
        TextColor(Color::WHITE),
        TextShadow::default(),
        TextLayout::justify(Justify::Center),
        Node {
            position_type: PositionType::Absolute,
            top: Val::Percent(22.0),
            width: Val::Percent(100.0),
            justify_content: JustifyContent::Center,
            ..default()
        },
        Visibility::Hidden,
        GlobalZIndex(45),
    ));
    commands.spawn((
        Part::Overlay,
        Text::default(),
        fonts.debug(13.0),
        TextColor(Color::srgb(1.0, 1.0, 0.6)),
        TextShadow::default(),
        Node {
            position_type: PositionType::Absolute,
            left: Val::Px(8.0),
            top: Val::Percent(44.0),
            ..default()
        },
        Visibility::Hidden,
        GlobalZIndex(60),
    ));
}

/// The game's words for what happened: centre text for everyone, hints
/// to the local player, chat lines to the terrorists.
#[allow(clippy::too_many_arguments)]
fn messages(
    mut events: MessageReader<ObjectiveEvent>,
    mut penalties: MessageReader<HostagePenalty>,
    local: Option<Single<(Entity, Option<&Team>), With<LocalPlayer>>>,
    names: Query<(Option<&Name>, Has<LocalPlayer>)>,
    rules: Res<BombRules>,
    mut centre: ResMut<Centre>,
    mut hints: MessageWriter<Hint>,
    mut chat: MessageWriter<ChatLine>,
    time: Res<Time>,
) {
    let now = time.elapsed_secs();
    let (me, team) = local.map_or((Entity::PLACEHOLDER, None), |l| (l.0, l.1.copied()));
    let name = |e: Entity| match names.get(e) {
        Ok((_, true)) => "Player".to_string(),
        Ok((Some(n), _)) => n.to_string(),
        _ => format!("{e}"),
    };
    let carriers = team == Some(rules.carrier_team);
    let mut say = |text: &str| centre.0 = Some((text.to_string(), now + CENTRE_SECONDS));
    for ev in events.read() {
        use ObjectiveEvent as E;
        let hint: Option<String> = match ev {
            E::GotBomb { who } if *who == me => {
                Some("You have the bomb. Find the target zone or DROP the bomb for another Terrorist.".into())
            }
            E::PickedUpBomb { who } => {
                if carriers && *who != me {
                    chat.write(ChatLine::plain(format!("{} picked up the bomb.", name(*who))));
                }
                (*who == me).then(|| "You picked up the bomb.".into())
            }
            E::DroppedBomb { who } => {
                if carriers {
                    chat.write(ChatLine::plain(format!("{} dropped the bomb.", name(*who))));
                }
                None
            }
            E::PlantRefused { who, reason } | E::DefuseRefused { who, reason } if *who == me => {
                Some(reason.to_string())
            }
            E::AbortPlant { who, left_zone: true } if *who == me => {
                Some("Arming sequence canceled. C4 can only be placed at a bomb target.".into())
            }
            E::Planted { .. } => {
                say("The bomb has been planted.");
                None
            }
            E::BeginDefuse { who, kit } if *who == me => Some(if *kit {
                "Defusing bomb WITH defuse kit.".into()
            } else {
                "Defusing bomb WITHOUT defuse kit.".into()
            }),
            E::Defused { .. } => {
                say("The bomb has been defused.");
                None
            }
            E::PickedUpKit { who } if *who == me => Some("You picked up a defuse kit!".into()),
            E::HostageFollows { leader, .. } if *leader == me => Some(
                "Lead the hostage to the rescue point! You may USE the hostage again to stop him from following."
                    .into(),
            ),
            E::HostageRefused { who } if *who == me => Some("Only Counter-Terrorists can move the hostages.".into()),
            E::HostageHurt { attacker, .. } if *attacker == Some(me) => Some("You injured a hostage!".into()),
            E::HostageKilled { attacker, .. } if *attacker == Some(me) => Some("You killed a hostage!".into()),
            _ => None,
        };
        if let Some(h) = hint {
            hints.write(Hint(h));
        }
    }
    for p in penalties.read() {
        match p {
            HostagePenalty::Warning { who } if *who == me => {
                hints.write(Hint(
                    "If you kill one more hostage, you will be removed from the server.".into(),
                ));
            }
            HostagePenalty::Removed { who } => {
                chat.write(ChatLine::plain(format!(
                    "{} was removed for killing too many hostages.",
                    name(*who)
                )));
            }
            _ => {}
        }
    }
}

fn panel_box(hud: Option<&ActiveHud>, name: &str, fallback: (HudCoord, HudCoord, f32, f32), w: f32, h: f32) -> Rect {
    let scale = h / 480.0;
    let (x, y, wide, tall) = hud
        .and_then(|hud| hud.0.panels.get(name))
        .map_or(fallback, |p| (p.x, p.y, p.wide, p.tall));
    let min = Vec2::new(x.resolve(w, scale), y.resolve(h, scale));
    Rect::from_corners(min, min + Vec2::new(wide, tall) * scale)
}

/// The status icon and the scenario icon.
#[allow(clippy::type_complexity, clippy::too_many_arguments)]
fn status(
    hud: Option<Res<ActiveHud>>,
    fonts: Res<UiFonts>,
    windows: Query<&Window>,
    local: Option<
        Single<
            (
                Entity,
                &Transform,
                Option<&MovementState>,
                Option<&Team>,
                Option<&Health>,
                Has<DefuseKit>,
            ),
            With<LocalPlayer>,
        >,
    >,
    world_bits: (Res<MapObjectives>, Res<BombState>, Res<HostageTally>, Res<BombRules>),
    inventories: Query<&crate::weapon::Inventory>,
    bombs: Query<(), With<crate::objectives::bomb::C4>>,
    mut parts: Query<(
        &Part,
        &mut Text,
        &mut TextFont,
        &mut TextColor,
        &mut Node,
        &mut Visibility,
    )>,
    time: Res<Time>,
) {
    let (objectives, bomb, tally, rules) = world_bits;
    let Some(window) = windows.iter().next() else { return };
    let (w, h) = (window.width(), window.height());
    let hud_ref = hud.as_deref();
    let color = |name: &str, fallback: Color| hud_ref.and_then(|h| h.0.color(name)).unwrap_or(fallback);
    let glyph = |name: &str| -> Option<(Handle<Font>, f32, char)> {
        let (font, ch) = hud_ref?.0.icons.get(name)?.clone();
        let (handle, tall) = fonts.game(&font)?;
        Some((handle, tall, ch))
    };
    let flash = (time.elapsed_secs() * 4.0) as u32 % 2 == 0;
    let green = color("HudIcon_Green", Color::srgb_u8(0, 160, 0));
    let red = color("HudIcon_Red", Color::srgb_u8(160, 0, 0));
    // Status: the bomb (red-flashing in a target), else the kit, else the
    // rescue zone (flashing) for a defender standing in one.
    let status = local.as_ref().and_then(|l| {
        let (e, t, state, team, health, kit) = **l;
        if health.is_some_and(|h| h.current <= 0.0) {
            return None;
        }
        let (lo, hi) = hull(t, state);
        let carrying = inventories
            .get(e)
            .is_ok_and(|inv| inv.weapons.iter().any(|w| bombs.contains(*w)));
        if carrying {
            let inside = objectives.bomb_target_at(lo, hi).is_some();
            return Some(("HudC4", "c4", if inside && flash { red } else { green }));
        }
        if kit {
            return Some(("HudDefuser", "defuser", green));
        }
        if team == Some(&rules.defuser_team) && objectives.rescue_zones.iter().any(|z| z.overlaps(lo, hi)) {
            return Some((
                "HudHostageRescueZone",
                "hostage_rescue",
                if flash { red } else { green },
            ));
        }
        None
    });
    // Scenario: the planted bomb, or a glyph per hostage left.
    let scenario = if bomb.planted.is_some() {
        glyph("scenario_c4").map(|g| (g, 1))
    } else if objectives.kind() == MapKind::Hostage && tally.total > 0 {
        let left = tally.total.saturating_sub(tally.rescued + tally.killed) as usize;
        glyph("scenario_hostage").map(|g| (g, left))
    } else {
        None
    };
    let scale = h / 480.0;
    for (part, mut text, mut font, mut tc, mut node, mut vis) in &mut parts {
        let want = match part {
            Part::Status => status.and_then(|(panel, icon, c)| {
                let (handle, tall, ch) = glyph(icon)?;
                let r = panel_box(
                    hud_ref,
                    panel,
                    (HudCoord::Start(16.0), HudCoord::Start(240.0), 40.0, 40.0),
                    w,
                    h,
                );
                Some((r, handle, tall, ch.to_string(), c))
            }),
            Part::Scenario => scenario.as_ref().map(|((handle, tall, ch), n)| {
                let r = panel_box(
                    hud_ref,
                    "HudScenarioIcon",
                    (HudCoord::Centre(110.0), HudCoord::Start(443.0), 40.0, 44.0),
                    w,
                    h,
                );
                (
                    r,
                    handle.clone(),
                    *tall,
                    ch.to_string().repeat(*n),
                    color("Hostage_Yellow", Color::srgba_u8(255, 176, 0, 120)),
                )
            }),
            _ => continue,
        };
        let Some((r, handle, tall, s, c)) = want else {
            *vis = Visibility::Hidden;
            continue;
        };
        *vis = Visibility::Inherited;
        node.left = Val::Px(r.min.x);
        node.top = Val::Px(r.min.y);
        if text.0 != s {
            text.0 = s;
        }
        font.font = handle.into();
        font.font_size = FontSize::Px(tall * scale);
        tc.0 = c;
    }
}

/// The progress bar while the local player arms or defuses.
#[allow(clippy::type_complexity)]
fn progress(
    hud: Option<Res<ActiveHud>>,
    windows: Query<&Window>,
    local: Option<Single<(Entity, Option<&Arming>), With<LocalPlayer>>>,
    bombs: Query<&PlantedBomb>,
    rules: Res<BombRules>,
    fixed: Res<Time<Fixed>>,
    mut frame: Query<(&Part, &mut Node, &mut Visibility)>,
    mut fill: Query<&mut Node, (With<FillMarker>, Without<Part>)>,
) {
    let Some(window) = windows.iter().next() else { return };
    let (w, h) = (window.width(), window.height());
    let now = fixed.elapsed_secs_f64();
    let share = local.as_ref().and_then(|l| {
        let (me, arming) = **l;
        if let Some(a) = arming {
            return Some(((now - a.since) / rules.plant_time as f64) as f32);
        }
        bombs
            .iter()
            .filter_map(|b| b.defuse)
            .find(|d| d.who == me)
            .map(|d| ((now - d.started) / (d.ends - d.started)) as f32)
    });
    let r = panel_box(
        hud.as_deref(),
        "HudProgressBar",
        (HudCoord::Centre(-150.0), HudCoord::Start(300.0), 300.0, 15.0),
        w,
        h,
    );
    for (part, mut node, mut vis) in &mut frame {
        if !matches!(part, Part::Frame) {
            continue;
        }
        *vis = if share.is_some() {
            Visibility::Inherited
        } else {
            Visibility::Hidden
        };
        node.left = Val::Px(r.min.x);
        node.top = Val::Px(r.min.y);
        node.width = Val::Px(r.width());
        node.height = Val::Px(r.height());
        node.border = UiRect::all(Val::Px((h / 480.0).max(1.0)));
    }
    for mut node in &mut fill {
        node.width = Val::Percent(share.unwrap_or(0.0).clamp(0.0, 1.0) * 100.0);
    }
}

#[derive(Component)]
struct FillMarker;

fn centre(
    mut state: ResMut<Centre>,
    windows: Query<&Window>,
    mut parts: Query<(&Part, &mut Text, &mut TextFont, &mut Visibility)>,
    time: Res<Time>,
    fonts: Res<UiFonts>,
) {
    let now = time.elapsed_secs();
    if state.0.as_ref().is_some_and(|(_, until)| now > *until) {
        state.0 = None;
    }
    // Centre print: the client scheme's CenterPrintText (Trebuchet).
    let want = fonts.client("CenterPrintText", windows.iter().next().map_or(480.0, |w| w.height()), 18.0);
    for (part, mut text, mut font, mut vis) in &mut parts {
        if !matches!(part, Part::Centre) {
            continue;
        }
        match &state.0 {
            Some((s, _)) => {
                if text.0 != *s {
                    text.0 = s.clone();
                }
                if *font != want {
                    *font = want.clone();
                }
                *vis = Visibility::Inherited;
            }
            None => *vis = Visibility::Hidden,
        }
    }
}

/// `mashup_objectives 1`: the bomb (where, time left, defuse progress)
/// and the hostages (leader, health), as text.
#[allow(clippy::type_complexity)]
fn overlay(
    show: Res<Overlay>,
    bombs: Query<(&PlantedBomb, &Transform)>,
    hostages: Query<(Entity, &Hostage, &Transform, &Health)>,
    carriers: Query<(Entity, Option<&Name>, &crate::weapon::Inventory)>,
    c4s: Query<(), With<crate::objectives::bomb::C4>>,
    state: (Res<BombState>, Res<HostageTally>),
    fixed: Res<Time<Fixed>>,
    mut parts: Query<(&Part, &mut Text, &mut Visibility)>,
) {
    let mut lines = Vec::new();
    if show.0 != 0 {
        let now = fixed.elapsed_secs_f64();
        let (bomb_state, tally) = state;
        for (e, name, inv) in &carriers {
            if inv.weapons.iter().any(|w| c4s.contains(*w)) {
                lines.push(format!(
                    "bomb carried by {}",
                    name.map_or(format!("{e}"), |n| n.to_string())
                ));
            }
        }
        for (b, t) in &bombs {
            let p = t.translation / 0.0254;
            lines.push(format!(
                "bomb planted at ({:.0} {:.0} {:.0}) site {:?}: {:.2} s left{}",
                p.x,
                -p.z,
                p.y,
                b.site,
                (b.explode_at - now).max(0.0),
                if b.defused { ", DEFUSED" } else { "" }
            ));
            if let Some(d) = b.defuse {
                lines.push(format!(
                    "  defusing {} ({}): {:.2} / {:.2} s",
                    d.who,
                    if d.kit { "kit" } else { "no kit" },
                    now - d.started,
                    d.ends - d.started
                ));
            }
        }
        if let Some(o) = bomb_state.outcome {
            lines.push(format!("bomb: {o:?}"));
        }
        if tally.total > 0 {
            lines.push(format!(
                "hostages: {} of {} rescued, {} killed",
                tally.rescued, tally.total, tally.killed
            ));
        }
        for (e, h, t, health) in &hostages {
            lines.push(format!(
                "  {e}: health {:.0}, leader {:?}, at {:.1} {:.1} {:.1}",
                health.current * 100.0,
                h.leader,
                t.translation.x,
                t.translation.y,
                t.translation.z
            ));
        }
    }
    for (part, mut text, mut vis) in &mut parts {
        if !matches!(part, Part::Overlay) {
            continue;
        }
        if lines.is_empty() {
            *vis = Visibility::Hidden;
            continue;
        }
        *vis = Visibility::Inherited;
        let s = lines.join("\n");
        if text.0 != s {
            text.0 = s;
        }
    }
}

/// Whether an objective event says `me` used something by pressing +use.
fn used(e: &ObjectiveEvent, me: Entity) -> bool {
    match e {
        ObjectiveEvent::BeginDefuse { who, .. }
        | ObjectiveEvent::DefuseRefused { who, .. }
        | ObjectiveEvent::HostageRefused { who } => *who == me,
        ObjectiveEvent::HostageFollows { leader, .. } | ObjectiveEvent::HostageStops { leader, .. } => *leader == me,
        _ => false,
    }
}

/// The deny sound for a +use press of the living local player that found
/// nothing: no map entity (`logic`'s use presses), no objective and no
/// weapon taken (`mashup_usepickup`).
#[allow(clippy::too_many_arguments)]
fn use_deny(
    local: Query<(Entity, &crate::core::Intent, Option<&Health>), With<LocalPlayer>>,
    logic: Option<Res<crate::logic::Logic>>,
    sounds: Option<Res<crate::map::RoundSounds>>,
    mut events: MessageReader<ObjectiveEvent>,
    mut picks: MessageReader<crate::weapon::drop::UsedPickup>,
    mut play: MessageWriter<crate::map::PlaySound>,
    mut was: Local<bool>,
) {
    let Some((me, intent, health)) = local.iter().next() else {
        events.clear();
        picks.clear();
        return;
    };
    let objective = events.read().fold(false, |found, e| found || used(e, me))
        | picks.read().fold(false, |found, p| found || p.who == me);
    let pressed = intent.use_key && !*was && health.is_none_or(|h| h.current > 0.0);
    *was = intent.use_key;
    if !pressed {
        return;
    }
    let map = logic.is_some_and(|l| l.world.use_presses.iter().any(|(e, found)| *e == me && *found));
    if !map
        && !objective
        && let Some(entry) = sounds.and_then(|s| s.use_deny.clone())
    {
        play.write(crate::map::PlaySound::ui(entry));
    }
}
