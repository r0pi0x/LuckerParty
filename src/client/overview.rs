//! The spectators' map overview (Source's `overview_mode`, picked from the
//! spectator menu's Options > Overview): the map's overview picture
//! (`resource/overviews/<map>.txt` and its texture, `map::hud::ActiveOverview`,
//! the radar's) in a small box at the top left (1) or across the screen
//! between the spectator bars (2), with the players who may be watched on
//! it in their team's colours, their names (`overview_names`), health
//! (`overview_health`) and where they have been (`overview_tracks`);
//! `overview_zoom` zooms toward the watched player (or the free camera),
//! `overview_locked 0` turns it with the view as the radar does. A map
//! without an overview shows none (the menu's Small and Large are greyed).
//!
//! From the install's menu files and general CS:S knowledge; the boxes'
//! sizes, the defaults and the track length are guesses
//! (docs/tech-debt.md, spectating).

use std::collections::{HashMap, VecDeque};

use bevy::prelude::*;

use super::radar::{RadarMaterial, RadarParams};
use super::spectate::{SpecPhase, SpecView, Spectator};
use crate::{
    console::{ConsoleAppExt, resource_cvar},
    core::{Health, Intent, Team},
    map::hud::{ActiveOverview, MapOverview},
};

/// The overview drawn (with `OverviewStatePlugin`).
pub struct OverviewPlugin;

impl Plugin for OverviewPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(OverviewStatePlugin)
            .add_systems(Update, draw.after(zoom).after(super::spectate::SpectateSet));
    }
}

/// The overview's cvars, `overview_zoom` and the zoom moving (no window
/// needed: tests).
pub struct OverviewStatePlugin;

impl Plugin for OverviewStatePlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Update, zoom);
        overview_cvars(app);
    }
}

fn overview_cvars(app: &mut App) {
    app.init_resource::<OverviewSettings>();
    resource_cvar::<OverviewSettings, u8>(
        app,
        "overview_mode",
        "Spectating: the map overview: 0 none, 1 a small map, 2 a large map.",
        |o| &mut o.mode,
    );
    resource_cvar::<OverviewSettings, u8>(
        app,
        "overview_locked",
        "1: the spectators' overview doesn't turn with the view (as drawn).",
        |o| &mut o.locked,
    );
    resource_cvar::<OverviewSettings, u8>(
        app,
        "overview_names",
        "1: player names on the spectators' overview.",
        |o| &mut o.names,
    );
    resource_cvar::<OverviewSettings, u8>(
        app,
        "overview_health",
        "1: player health on the spectators' overview.",
        |o| &mut o.health,
    );
    resource_cvar::<OverviewSettings, u8>(
        app,
        "overview_tracks",
        "1: where players have been, on the spectators' overview.",
        |o| &mut o.tracks,
    );
    app.console_command(
        "overview_zoom",
        "overview_zoom <zoom> [seconds] [rel]: zoom the spectators' overview (1: the whole map; rel: times the zoom now) \
         over that many seconds.",
        |w, a| {
            let zoom: f32 = a
                .first()
                .and_then(|z| z.parse().ok())
                .ok_or("overview_zoom <zoom> [seconds] [rel]")?;
            let time: f32 = a.get(1).and_then(|t| t.parse().ok()).unwrap_or(0.0);
            let rel = a.iter().any(|x| x.eq_ignore_ascii_case("rel"));
            let mut o = w.resource_mut::<OverviewSettings>();
            o.set_zoom(zoom, time, rel);
            Ok(None)
        },
    );
    for name in [
        "overview_mode",
        "overview_locked",
        "overview_names",
        "overview_health",
        "overview_tracks",
    ] {
        app.world_mut().resource_mut::<crate::console::Console>().archive(name);
    }
}

/// Most and least zoom (1: the whole map in the box).
pub const ZOOM_RANGE: (f32, f32) = (1.0, 10.0);

/// The overview's settings (Source's cvars; defaults are guesses: off, not
/// turning, names, health and tracks shown).
#[derive(Resource, Clone, Copy, Debug, PartialEq)]
pub struct OverviewSettings {
    pub mode: u8,
    pub locked: u8,
    pub names: u8,
    pub health: u8,
    pub tracks: u8,
    /// The zoom now, and where it is going (zoom, per second toward it).
    pub zoom: f32,
    pub zoom_to: f32,
    pub zoom_rate: f32,
}

impl Default for OverviewSettings {
    fn default() -> Self {
        Self {
            mode: 0,
            locked: 1,
            names: 1,
            health: 1,
            tracks: 1,
            zoom: 1.0,
            zoom_to: 1.0,
            zoom_rate: 0.0,
        }
    }
}

impl OverviewSettings {
    /// `overview_zoom`: to `zoom` (times the target now with `rel`) over
    /// `time` seconds.
    pub fn set_zoom(&mut self, zoom: f32, time: f32, rel: bool) {
        let to = if rel { self.zoom_to * zoom } else { zoom };
        self.zoom_to = to.clamp(ZOOM_RANGE.0, ZOOM_RANGE.1);
        if time <= 0.0 {
            self.zoom = self.zoom_to;
            self.zoom_rate = 0.0;
        } else {
            self.zoom_rate = (self.zoom_to - self.zoom).abs() / time;
        }
    }

    /// A frame of `dt` seconds toward the zoom asked for.
    pub fn step(&mut self, dt: f32) {
        if self.zoom_rate <= 0.0 {
            self.zoom = self.zoom_to;
            return;
        }
        let d = self.zoom_to - self.zoom;
        let step = self.zoom_rate * dt;
        if d.abs() <= step {
            self.zoom = self.zoom_to;
            self.zoom_rate = 0.0;
        } else {
            self.zoom += step * d.signum();
        }
    }
}

/// What part of the overview a box shows: the overview pixel at its
/// centre and box pixels per overview pixel. At zoom 1 the whole picture
/// fits the box, centred; zoomed in it moves toward `focus` (an overview
/// pixel), all the way at infinite zoom.
pub fn view(o: &MapOverview, focus: Vec2, zoom: f32, size: Vec2) -> (Vec2, f32) {
    let zoom = zoom.max(ZOOM_RANGE.0);
    let fit = (size / o.size.max(Vec2::ONE)).min_element();
    let middle = o.size / 2.0;
    (middle + (focus - middle) * (1.0 - 1.0 / zoom), fit * zoom)
}

/// Where an overview pixel falls in the box (pixels from its top left),
/// given the view's centre, scale and turn (clockwise radians).
pub fn place(pixel: Vec2, centre: Vec2, k: f32, angle: f32, size: Vec2) -> Vec2 {
    size / 2.0 + Rot2::radians(angle) * ((pixel - centre) * k)
}

/// Seconds between track points, and how many are kept per player.
const TRACK_EVERY: f32 = 0.5;
const TRACK_POINTS: usize = 12;
/// Marker size, 480-line pixels.
const DOT: f32 = 6.0;

fn zoom(mut o: ResMut<OverviewSettings>, time: Res<Time>) {
    if o.zoom != o.zoom_to {
        o.step(time.delta_secs());
    }
}

#[derive(Component)]
struct OverviewRoot;
#[derive(Component)]
struct OverviewMap;
#[derive(Component)]
struct OverviewMarks;

/// One player on the overview: where (box pixels), colour, its text.
#[derive(Clone, Debug, PartialEq)]
struct Mark {
    at: Vec2,
    color: [u8; 4],
    text: String,
    health: Option<u8>,
    watched: bool,
}

#[derive(Clone, Debug, Default, PartialEq)]
struct Shot {
    marks: Vec<Mark>,
    tracks: Vec<(Vec2, [u8; 4])>,
    rect: Rect,
}

#[allow(clippy::too_many_arguments, clippy::type_complexity)]
fn draw(
    settings: Res<OverviewSettings>,
    overview: Option<Res<ActiveOverview>>,
    (spec, spec_view): (Res<Spectator>, Res<SpecView>),
    bars: Res<super::spectate::SpectatorBarsUp>,
    windows: Query<&Window>,
    who: Query<(
        &GlobalTransform,
        Option<&Intent>,
        Option<&Team>,
        Option<&Health>,
        Option<&Name>,
    )>,
    mut roots: Query<(&mut Node, &mut Visibility, &mut BackgroundColor), With<OverviewRoot>>,
    map: Query<&MaterialNode<RadarMaterial>, With<OverviewMap>>,
    marks_q: Query<Entity, With<OverviewMarks>>,
    mut materials: ResMut<Assets<RadarMaterial>>,
    fonts: Res<super::fonts::UiFonts>,
    time: Res<Time>,
    mut trails: Local<(HashMap<Entity, VecDeque<Vec3>>, f32)>,
    mut last: Local<Option<Shot>>,
    mut commands: Commands,
) {
    let shown = settings.mode > 0 && spec.phase == SpecPhase::Watching;
    let (Some(overview), Some(window), true) = (overview, windows.iter().next(), shown) else {
        for (_, mut vis, _) in &mut roots {
            vis.set_if_neq(Visibility::Hidden);
        }
        trails.0.clear();
        *last = None;
        return;
    };
    let (w, h) = (window.width(), window.height());
    let s = h / 480.0;
    let top = bars.0.unwrap_or(0.0);
    let bottom = if bars.0.is_some() { 428.0 * s } else { h };
    let rect = if settings.mode >= 2 {
        Rect::new(0.0, top, w, bottom)
    } else {
        Rect::from_corners(
            Vec2::new(8.0 * s, top + 8.0 * s),
            Vec2::new(8.0 * s + 192.0 * s, top + 8.0 * s + 144.0 * s),
        )
    };
    let Ok((mut node, mut vis, mut backdrop)) = roots.single_mut() else {
        // First use: the box, the map in it and the marks over it.
        let root = commands
            .spawn((
                OverviewRoot,
                Node {
                    position_type: PositionType::Absolute,
                    overflow: Overflow::clip(),
                    ..default()
                },
                BackgroundColor(Color::srgba(0.0, 0.0, 0.0, 0.6)),
                Visibility::Hidden,
                // Under the spectator bars (43) and menu.
                GlobalZIndex(42),
            ))
            .id();
        commands.spawn((
            OverviewMap,
            MaterialNode(materials.add(RadarMaterial {
                params: RadarParams {
                    alpha: 0.9,
                    ..default()
                },
                overview: overview.1.clone(),
            })),
            Node {
                position_type: PositionType::Absolute,
                width: percent(100.0),
                height: percent(100.0),
                ..default()
            },
            ChildOf(root),
        ));
        commands.spawn((
            OverviewMarks,
            Node {
                position_type: PositionType::Absolute,
                width: percent(100.0),
                height: percent(100.0),
                ..default()
            },
            ZIndex(1),
            ChildOf(root),
        ));
        return;
    };
    vis.set_if_neq(Visibility::Inherited);
    // The large map over a darker backdrop (the picture is see-through
    // off the map).
    backdrop.set_if_neq(BackgroundColor(Color::srgba(
        0.0,
        0.0,
        0.0,
        if settings.mode >= 2 { 0.85 } else { 0.6 },
    )));
    if node.left != px(rect.min.x)
        || node.top != px(rect.min.y)
        || node.width != px(rect.width())
        || node.height != px(rect.height())
    {
        node.left = px(rect.min.x);
        node.top = px(rect.min.y);
        node.width = px(rect.width());
        node.height = px(rect.height());
    }
    let o = &overview.0;
    let size = rect.size();
    // What it follows: the watched player, else the camera.
    let target = spec.target.and_then(|t| who.get(t).ok());
    let (focus, yaw) = match (target, spec_view.pose) {
        (Some((g, intent, ..)), _) => (g.translation(), intent.map_or(0.0, |i| i.yaw)),
        (None, Some((at, rot))) => (at, rot.to_euler(EulerRot::YXZ).0),
        (None, None) => (Vec3::ZERO, 0.0),
    };
    let (centre, k) = view(o, o.pixel(focus), settings.zoom, size);
    let angle = super::radar::radar_angle(o.rotate, settings.locked != 0, yaw);
    let params = RadarParams {
        centre: centre / o.size,
        span: size / k / o.size,
        angle,
        alpha: 0.9,
        size,
    };
    if let Ok(m) = map.single()
        && materials
            .get(&m.0)
            .is_some_and(|m| m.params != params || m.overview != overview.1)
        && let Some(mut m) = materials.get_mut(&m.0)
    {
        m.params = params;
        m.overview = overview.1.clone();
    }
    // Tracks: each player's last points.
    trails.1 += time.delta_secs();
    let sample = trails.1 >= TRACK_EVERY;
    if sample {
        trails.1 = 0.0;
    }
    let color = |team: Option<&Team>| match team.map(|t| t.0) {
        Some(1) => [255, 64, 64, 255],
        Some(2) => [153, 204, 255, 255],
        _ => [204, 204, 204, 255],
    };
    let mut shot = Shot { rect, ..default() };
    trails.0.retain(|e, _| spec.watchable.contains(e));
    for e in &spec.watchable {
        let Ok((g, _, team, health, name)) = who.get(*e) else {
            continue;
        };
        let p = g.translation();
        let at = place(o.pixel(p), centre, k, angle, size);
        let c = color(team);
        let trail = trails.0.entry(*e).or_default();
        if sample {
            trail.push_back(p);
            while trail.len() > TRACK_POINTS {
                trail.pop_front();
            }
        }
        if settings.tracks != 0 {
            for (i, q) in trail.iter().enumerate() {
                let fade = (64 + 128 * (i + 1) / TRACK_POINTS) as u8;
                let t = place(o.pixel(*q), centre, k, angle, size).round();
                shot.tracks.push((t, [c[0], c[1], c[2], fade]));
            }
        }
        shot.marks.push(Mark {
            at: at.round(),
            color: c,
            text: if settings.names != 0 {
                name.map_or_else(String::new, |n| n.as_str().to_string())
            } else {
                String::new()
            },
            health: (settings.health != 0).then(|| (health.map_or(0.0, |h| h.current) * 100.0).clamp(0.0, 100.0) as u8),
            watched: spec.target == Some(*e),
        });
    }
    if last.as_ref() == Some(&shot) {
        return;
    }
    let Ok(marks) = marks_q.single() else { return };
    commands.entity(marks).despawn_related::<Children>();
    let font = fonts.client("DefaultVerySmall", h, 8.0);
    for (at, c) in &shot.tracks {
        let d = 2.0 * s;
        commands.spawn((
            Node {
                position_type: PositionType::Absolute,
                left: px(at.x - d / 2.0),
                top: px(at.y - d / 2.0),
                width: px(d),
                height: px(d),
                ..default()
            },
            BackgroundColor(Color::srgba_u8(c[0], c[1], c[2], c[3])),
            ChildOf(marks),
        ));
    }
    for m in &shot.marks {
        let d = DOT * s * if m.watched { 1.5 } else { 1.0 };
        let [r, g, b, a] = m.color;
        commands.spawn((
            Node {
                position_type: PositionType::Absolute,
                left: px(m.at.x - d / 2.0),
                top: px(m.at.y - d / 2.0),
                width: px(d),
                height: px(d),
                border: UiRect::all(px(1.0)),
                border_radius: BorderRadius::MAX,
                ..default()
            },
            BackgroundColor(Color::srgba_u8(r, g, b, a)),
            BorderColor::all(if m.watched { Color::WHITE } else { Color::BLACK }),
            ChildOf(marks),
        ));
        let mut below = m.at.y + d / 2.0 + 1.0;
        if let Some(hp) = m.health {
            // A health bar under the dot.
            let (bw, bh) = (16.0 * s, 2.0 * s);
            let bar = commands
                .spawn((
                    Node {
                        position_type: PositionType::Absolute,
                        left: px(m.at.x - bw / 2.0),
                        top: px(below),
                        width: px(bw),
                        height: px(bh),
                        ..default()
                    },
                    BackgroundColor(Color::srgba(0.0, 0.0, 0.0, 0.7)),
                    ChildOf(marks),
                ))
                .id();
            commands.spawn((
                Node {
                    width: percent(hp as f32),
                    height: percent(100.0),
                    ..default()
                },
                BackgroundColor(Color::srgb(0.3, 0.9, 0.3)),
                ChildOf(bar),
            ));
            below += bh + 1.0;
        }
        if !m.text.is_empty() {
            commands.spawn((
                Node {
                    position_type: PositionType::Absolute,
                    left: px(m.at.x - 60.0 * s),
                    top: px(below),
                    width: px(120.0 * s),
                    justify_content: JustifyContent::Center,
                    ..default()
                },
                ChildOf(marks),
                children![(
                    Text::new(m.text.clone()),
                    font.clone(),
                    TextColor(Color::srgba_u8(r, g, b, a)),
                    TextShadow::default(),
                    TextLayout::new(Justify::Center, bevy::text::LineBreak::NoWrap),
                )],
            ));
        }
    }
    *last = Some(shot);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn overview() -> MapOverview {
        MapOverview {
            texture: 0,
            origin: Vec2::ZERO,
            meters_per_pixel: 1.0,
            rotate: false,
            size: Vec2::new(1024.0, 1024.0),
        }
    }

    #[test]
    fn zoom_one_shows_the_whole_map_and_zooming_follows_the_player() {
        let o = overview();
        let box_size = Vec2::new(256.0, 192.0);
        let player = Vec2::new(900.0, 100.0);
        let (centre, k) = view(&o, player, 1.0, box_size);
        assert_eq!(centre, Vec2::splat(512.0), "the whole map, centred");
        assert!((k * 1024.0 - 192.0).abs() < 1e-3, "it fits the box's shorter side");
        let (centre, k2) = view(&o, player, 4.0, box_size);
        assert!((k2 / k - 4.0).abs() < 1e-4);
        assert!(
            centre.distance(player) < Vec2::splat(512.0).distance(player),
            "toward the player"
        );
        // The player stays in the box zoomed in.
        let at = place(player, centre, k2, 0.0, box_size);
        assert!(
            at.x >= 0.0 && at.x <= box_size.x && at.y >= 0.0 && at.y <= box_size.y,
            "{at}"
        );
    }

    #[test]
    fn zoom_commands_are_relative_timed_and_clamped() {
        let mut o = OverviewSettings::default();
        o.set_zoom(1.1, 0.0, true);
        assert!((o.zoom - 1.1).abs() < 1e-5);
        o.set_zoom(0.5, 0.0, true);
        assert_eq!(o.zoom, ZOOM_RANGE.0, "not past the whole map");
        o.set_zoom(3.0, 1.0, false);
        o.step(0.5);
        assert!((o.zoom - 2.0).abs() < 1e-4, "half way after half the time: {}", o.zoom);
        o.step(0.6);
        assert_eq!(o.zoom, 3.0);
    }
}
