//! The local player's HUD (crosshair, health, ammo, hit marker, killfeed)
//! and bodies for other characters until character models exist.

use bevy::prelude::*;

use super::FirstPersonCamera;
use crate::{
    core::{Died, Health, Intent, LocalPlayer, Team},
    rules::{Dead, Score},
    weapon::{Hitscan, Inventory, Magazine, Weapon, WeaponEvent, WeaponEventKind},
};

pub struct HudPlugin;

impl Plugin for HudPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<HitMarker>()
            .init_resource::<Killfeed>()
            .add_systems(Startup, spawn_hud)
            .add_systems(
                Update,
                (
                    character_bodies,
                    show_bodies,
                    (hit_marker, killfeed, draw_hud, draw_crosshair).chain(),
                ),
            );
    }
}

#[derive(Component)]
struct HealthText;
#[derive(Component)]
struct AmmoText;
#[derive(Component)]
struct HitMarkerText;
#[derive(Component)]
struct KillfeedText;
#[derive(Component)]
struct CenterText;
/// One of the four crosshair lines: its direction from the centre.
#[derive(Component)]
struct CrosshairLine(Vec2);

/// Seconds the hit marker stays up, and whether the last hit killed.
#[derive(Resource, Default)]
struct HitMarker {
    left: f32,
}

#[derive(Resource, Default)]
struct Killfeed(Vec<(String, f32)>);

const HUD_COLOR: Color = Color::srgba(1.0, 0.86, 0.45, 0.9);
const CROSSHAIR_COLOR: Color = Color::srgba(0.3, 1.0, 0.3, 0.9);
const LINE_LENGTH: f32 = 7.0;
const LINE_WIDTH: f32 = 2.0;
const MIN_GAP: f32 = 3.0;
const MARKER_SECONDS: f32 = 0.2;
const FEED_SECONDS: f32 = 6.0;

fn spawn_hud(mut commands: Commands) {
    let font = |size: f32| TextFont {
        font_size: FontSize::Px(size),
        ..default()
    };
    commands.spawn((
        HealthText,
        Text::default(),
        font(28.0),
        TextColor(HUD_COLOR),
        Node {
            position_type: PositionType::Absolute,
            bottom: px(14.0),
            left: px(20.0),
            ..default()
        },
        GlobalZIndex(40),
    ));
    commands.spawn((
        AmmoText,
        Text::default(),
        font(28.0),
        TextColor(HUD_COLOR),
        TextLayout::justify(Justify::Right),
        Node {
            position_type: PositionType::Absolute,
            bottom: px(14.0),
            right: px(20.0),
            ..default()
        },
        GlobalZIndex(40),
    ));
    commands.spawn((
        KillfeedText,
        Text::default(),
        font(15.0),
        TextColor(Color::srgb(0.95, 0.95, 0.95)),
        TextLayout::justify(Justify::Right),
        Node {
            position_type: PositionType::Absolute,
            top: px(90.0),
            right: px(10.0),
            ..default()
        },
        GlobalZIndex(40),
    ));
    commands.spawn((
        CenterText,
        Text::default(),
        font(22.0),
        TextColor(Color::srgb(1.0, 0.4, 0.35)),
        TextLayout::justify(Justify::Center),
        Node {
            position_type: PositionType::Absolute,
            top: percent(60.0),
            width: percent(100.0),
            ..default()
        },
        GlobalZIndex(40),
    ));
    commands.spawn((
        HitMarkerText,
        Text::new("x"),
        font(26.0),
        TextColor(Color::srgb(1.0, 1.0, 1.0)),
        TextLayout::justify(Justify::Center),
        Node {
            position_type: PositionType::Absolute,
            top: percent(50.0),
            width: percent(100.0),
            margin: UiRect::top(px(-17.0)),
            ..default()
        },
        GlobalZIndex(41),
        Visibility::Hidden,
    ));
    for dir in [Vec2::X, Vec2::NEG_X, Vec2::Y, Vec2::NEG_Y] {
        commands.spawn((
            CrosshairLine(dir),
            Node {
                position_type: PositionType::Absolute,
                ..default()
            },
            BackgroundColor(CROSSHAIR_COLOR),
            GlobalZIndex(40),
        ));
    }
}

type Local<'a> = (&'a Health, Option<&'a Inventory>, Option<&'a Score>, Option<&'a Dead>);

fn draw_hud(
    player: Option<Single<Local, With<LocalPlayer>>>,
    weapons: Query<(&Weapon, Option<&Magazine>)>,
    mut health_text: Single<&mut Text, (With<HealthText>, Without<AmmoText>, Without<CenterText>)>,
    mut ammo_text: Single<&mut Text, (With<AmmoText>, Without<HealthText>, Without<CenterText>)>,
    mut center: Single<&mut Text, (With<CenterText>, Without<HealthText>, Without<AmmoText>)>,
) {
    let Some(p) = player else { return };
    let (health, inv, score, dead) = *p;
    let score = score.copied().unwrap_or_default();
    health_text.0 = format!(
        "+ {:.0}    K {}  D {}",
        (health.current * 100.0).ceil(),
        score.kills,
        score.deaths
    );
    ammo_text.0 = match inv.and_then(|i| i.active).and_then(|w| weapons.get(w).ok()) {
        Some((w, Some(m))) => format!("{}\n{} | {}", short(w.id), m.clip, m.reserve),
        Some((w, None)) => short(w.id).to_string(),
        None => String::new(),
    };
    center.0 = if dead.is_some() {
        "You died. Respawning...".into()
    } else {
        String::new()
    };
}

/// `cs_source:weapon_ak47` -> `ak47`.
fn short(id: &str) -> &str {
    let id = id.rsplit(':').next().unwrap_or(id);
    id.strip_prefix("weapon_").unwrap_or(id)
}

/// Crosshair gap from the held weapon's spread at the camera's field of
/// view.
fn draw_crosshair(
    player: Option<Single<(&Inventory, Option<&Dead>), With<LocalPlayer>>>,
    scans: Query<&Hitscan>,
    camera: Query<(&Camera, &Projection), With<FirstPersonCamera>>,
    mut lines: Query<(&CrosshairLine, &mut Node, &mut Visibility)>,
) {
    let Some((cam, proj)) = camera.iter().next() else {
        return;
    };
    let Some(size) = cam.logical_viewport_size() else {
        return;
    };
    let (inv, dead) = player.map(|p| *p).unzip();
    let visible = inv.is_some() && dead.flatten().is_none();
    let spread = inv
        .and_then(|i| i.active)
        .and_then(|w| scans.get(w).ok())
        .map_or(0.0, |s| match s.spread {
            crate::weapon::SpreadShape::Template(s) => s,
            crate::weapon::SpreadShape::Disc { inaccuracy, spread } => inaccuracy + spread,
        });
    let fov = match proj {
        Projection::Perspective(p) => p.fov,
        _ => 1.0,
    };
    let gap = MIN_GAP + spread / (fov / 2.0).tan() * size.y / 2.0;
    let centre = size / 2.0;
    for (line, mut node, mut vis) in &mut lines {
        *vis = if visible {
            Visibility::Inherited
        } else {
            Visibility::Hidden
        };
        let d = line.0;
        let (w, h) = if d.x != 0.0 {
            (LINE_LENGTH, LINE_WIDTH)
        } else {
            (LINE_WIDTH, LINE_LENGTH)
        };
        // Line centre: gap plus half its length out from the centre (UI y
        // grows downward).
        let c = centre + Vec2::new(d.x, -d.y) * (gap + LINE_LENGTH / 2.0);
        node.left = px(c.x - w / 2.0);
        node.top = px(c.y - h / 2.0);
        node.width = px(w);
        node.height = px(h);
    }
}

fn hit_marker(
    mut events: MessageReader<WeaponEvent>,
    local: Option<Single<Entity, With<LocalPlayer>>>,
    mut marker: ResMut<HitMarker>,
    mut text: Single<(&mut Visibility, &mut TextColor), With<HitMarkerText>>,
    time: Res<Time>,
) {
    let me = local.map(|l| *l);
    for e in events.read() {
        if Some(e.owner) == me && matches!(e.kind, WeaponEventKind::Hit { .. }) {
            marker.left = MARKER_SECONDS;
        }
    }
    marker.left = (marker.left - time.delta_secs()).max(0.0);
    let (vis, color) = &mut *text;
    **vis = if marker.left > 0.0 {
        Visibility::Inherited
    } else {
        Visibility::Hidden
    };
    color.0 = Color::srgba(1.0, 1.0, 1.0, (marker.left / MARKER_SECONDS).min(1.0));
}

fn killfeed(
    mut died: MessageReader<Died>,
    names: Query<(Option<&Name>, Has<LocalPlayer>)>,
    mut feed: ResMut<Killfeed>,
    mut text: Single<&mut Text, With<KillfeedText>>,
    time: Res<Time>,
) {
    let name = |e: Entity| match names.get(e) {
        Ok((_, true)) => "You".to_string(),
        Ok((Some(n), _)) => n.to_string(),
        _ => format!("{e}"),
    };
    for d in died.read() {
        let line = match d.attacker.filter(|a| *a != d.entity) {
            Some(a) => format!("{} killed {}", name(a), name(d.entity)),
            None => format!("{} died", name(d.entity)),
        };
        feed.0.push((line, FEED_SECONDS));
    }
    let dt = time.delta_secs();
    feed.0.retain_mut(|(_, left)| {
        *left -= dt;
        *left > 0.0
    });
    if feed.0.len() > 6 {
        let extra = feed.0.len() - 6;
        feed.0.drain(..extra);
    }
    text.0 = feed.0.iter().map(|(l, _)| l.as_str()).collect::<Vec<_>>().join("\n");
}

/// A capsule for every character that isn't the local player.
fn character_bodies(
    new: Query<(Entity, &Team), (Added<Intent>, Without<LocalPlayer>)>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut commands: Commands,
) {
    for (e, team) in &new {
        let color = match team.0 {
            0 => Color::srgb(0.35, 0.5, 0.9),
            _ => Color::srgb(0.85, 0.4, 0.3),
        };
        let r = crate::character::CAPSULE_RADIUS;
        commands.entity(e).insert((
            Mesh3d(meshes.add(Capsule3d::new(r, crate::character::CAPSULE_HEIGHT - 2.0 * r))),
            MeshMaterial3d(materials.add(StandardMaterial {
                base_color: color,
                perceptual_roughness: 0.8,
                ..default()
            })),
        ));
    }
}

/// The dead aren't drawn.
fn show_bodies(mut bodies: Query<(&mut Visibility, Has<Dead>), (With<Mesh3d>, With<Intent>, Without<LocalPlayer>)>) {
    for (mut vis, dead) in &mut bodies {
        let want = if dead {
            Visibility::Hidden
        } else {
            Visibility::Inherited
        };
        if *vis != want {
            *vis = want;
        }
    }
}
