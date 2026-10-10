//! The local player's HUD (crosshair, health, ammo, hit marker, killfeed,
//! sniper scope) and bodies for other characters until character models
//! exist.

use bevy::prelude::*;

use super::FirstPersonCamera;
use crate::{
    core::{Died, Health, Intent, LocalPlayer, Team},
    rules::{Dead, Score},
    weapon::{Hitscan, Inventory, Magazine, Weapon, WeaponEvent, WeaponEventKind, Zoomed},
};

pub struct HudPlugin;

impl Plugin for HudPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<HitMarker>()
            .init_resource::<Killfeed>()
            .init_resource::<CrosshairColor>()
            .add_systems(Startup, spawn_hud)
            .add_systems(
                Update,
                (
                    character_bodies,
                    show_bodies,
                    (hit_marker, killfeed, draw_hud, draw_crosshair, draw_scope).chain(),
                    game_scope.run_if(resource_exists_and_changed::<crate::map::hud::ActiveHud>),
                    screen_tints,
                ),
            );
        crosshair_cvars(app);
    }
}

/// The crosshair's look, CS:S's cvars (all archived, CS:S's defaults):
/// `cl_crosshaircolor` (presets 0-4, 5 custom: `cl_crosshaircolor_r`,
/// `_g`, `_b`), `cl_crosshairsize` (line length; 5 is ours unscaled),
/// `cl_crosshairthickness` (line width; 0.5 is ours), `cl_crosshairdot`
/// (a centre dot), `cl_crosshairscale` (0: our size; else the screen
/// height it is drawn for, lower is bigger: 1200 small, 768 medium, 600
/// large), `cl_crosshairusealpha` with `cl_crosshairalpha` (0-255) for
/// translucency, and `cl_dynamiccrosshair` (the gap follows the weapon's
/// spread; 0: a fixed gap). How size and thickness map to pixels is ours
/// (docs/plans/active/ui-parity.md: a reference capture to match).
#[derive(Resource, Clone, Copy, Debug, PartialEq)]
pub struct CrosshairColor {
    pub color: u8,
    pub scale: f32,
    pub alpha: u8,
    pub use_alpha: u8,
    pub dynamic: u8,
    pub size: f32,
    pub thickness: f32,
    pub dot: u8,
    pub r: u8,
    pub g: u8,
    pub b: u8,
}

impl Default for CrosshairColor {
    fn default() -> Self {
        Self {
            color: 0,
            scale: 0.0,
            alpha: 200,
            use_alpha: 0,
            dynamic: 1,
            size: 5.0,
            thickness: 0.5,
            dot: 0,
            r: 50,
            g: 250,
            b: 50,
        }
    }
}

/// `cl_crosshairsize` and `cl_crosshairthickness` drawing our lines
/// (`LINE_LENGTH`, `LINE_WIDTH`) at their own size.
const BASE_SIZE: f32 = 5.0;
const BASE_THICKNESS: f32 = 0.5;

/// The crosshair's cvars.
pub const CROSSHAIR_CVARS: [&str; 11] = [
    "cl_crosshaircolor",
    "cl_crosshairscale",
    "cl_crosshairalpha",
    "cl_crosshairusealpha",
    "cl_dynamiccrosshair",
    "cl_crosshairsize",
    "cl_crosshairthickness",
    "cl_crosshairdot",
    "cl_crosshaircolor_r",
    "cl_crosshaircolor_g",
    "cl_crosshaircolor_b",
];

pub(crate) fn crosshair_cvars(app: &mut App) {
    use crate::console::resource_cvar;
    resource_cvar::<CrosshairColor, u8>(
        app,
        "cl_crosshaircolor",
        "Crosshair colour: 0 green, 1 red, 2 blue, 3 yellow, 4 cyan, 5 custom (cl_crosshaircolor_r/g/b).",
        |c| &mut c.color,
    );
    resource_cvar::<CrosshairColor, f32>(
        app,
        "cl_crosshairscale",
        "Crosshair size: 0 ours; else the screen height it is drawn for (lower is bigger: 1200, 768, 600).",
        |c| &mut c.scale,
    );
    resource_cvar::<CrosshairColor, u8>(
        app,
        "cl_crosshairalpha",
        "Crosshair opacity (0-255) with cl_crosshairusealpha 1.",
        |c| &mut c.alpha,
    );
    resource_cvar::<CrosshairColor, u8>(
        app,
        "cl_crosshairusealpha",
        "1: draw the crosshair cl_crosshairalpha translucent.",
        |c| &mut c.use_alpha,
    );
    resource_cvar::<CrosshairColor, u8>(
        app,
        "cl_dynamiccrosshair",
        "1: the crosshair's gap follows the weapon's spread; 0: a fixed gap.",
        |c| &mut c.dynamic,
    );
    resource_cvar::<CrosshairColor, f32>(app, "cl_crosshairsize", "Crosshair line length (5: the usual).", |c| {
        &mut c.size
    });
    resource_cvar::<CrosshairColor, f32>(
        app,
        "cl_crosshairthickness",
        "Crosshair line width (0.5: the usual).",
        |c| &mut c.thickness,
    );
    resource_cvar::<CrosshairColor, u8>(app, "cl_crosshairdot", "1: a dot at the crosshair's centre.", |c| {
        &mut c.dot
    });
    resource_cvar::<CrosshairColor, u8>(app, "cl_crosshaircolor_r", "Custom crosshair red (0-255).", |c| &mut c.r);
    resource_cvar::<CrosshairColor, u8>(app, "cl_crosshaircolor_g", "Custom crosshair green (0-255).", |c| {
        &mut c.g
    });
    resource_cvar::<CrosshairColor, u8>(app, "cl_crosshaircolor_b", "Custom crosshair blue (0-255).", |c| &mut c.b);
    let mut console = app.world_mut().resource_mut::<crate::console::Console>();
    for name in CROSSHAIR_CVARS {
        console.archive(name);
    }
}

impl CrosshairColor {
    pub fn color(self) -> Color {
        let a = if self.use_alpha != 0 {
            self.alpha as f32 / 255.0
        } else {
            CROSSHAIR_COLOR.alpha()
        };
        match self.color {
            1 => Color::srgba(1.0, 0.3, 0.3, a),
            2 => Color::srgba(0.3, 0.3, 1.0, a),
            3 => Color::srgba(1.0, 1.0, 0.3, a),
            4 => Color::srgba(0.3, 1.0, 1.0, a),
            5 => Color::srgb_u8(self.r, self.g, self.b).with_alpha(a),
            _ => CROSSHAIR_COLOR.with_alpha(a),
        }
    }

    /// How much bigger than ours the lines are drawn in a window `height`
    /// pixels tall (`cl_crosshairscale`).
    pub fn size(self, height: f32) -> f32 {
        if self.scale > 0.0 {
            (height / self.scale).clamp(0.25, 4.0)
        } else {
            1.0
        }
    }

    /// The four lines and the dot (offset of each one's centre from the
    /// crosshair's centre, x right and y down, and its size; the dot's
    /// offset is zero, its size zero when off) for a spread gap in
    /// pixels, in a window `height` pixels tall.
    pub fn lines(self, spread_gap: f32, height: f32) -> [(Vec2, Vec2); 5] {
        let k = self.size(height);
        let gap = MIN_GAP * k + if self.dynamic != 0 { spread_gap } else { 0.0 };
        let length = (LINE_LENGTH * k * self.size.max(0.0) / BASE_SIZE).round();
        let width = (LINE_WIDTH * k * self.thickness.max(0.0) / BASE_THICKNESS).round().max(1.0);
        let out = gap + length / 2.0;
        let dot = if self.dot != 0 { Vec2::splat(width) } else { Vec2::ZERO };
        [
            (Vec2::new(out, 0.0), Vec2::new(length, width)),
            (Vec2::new(-out, 0.0), Vec2::new(length, width)),
            (Vec2::new(0.0, out), Vec2::new(width, length)),
            (Vec2::new(0.0, -out), Vec2::new(width, length)),
            (Vec2::ZERO, dot),
        ]
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
/// The sniper scope overlay (shown while the local player is scoped).
#[derive(Component)]
struct ScopeOverlay;
/// The scope's square middle (the ring, the lens and cross hairs).
#[derive(Component)]
struct ScopeSquare;
/// One of the four crosshair lines: its direction from the centre (zero:
/// the centre dot).
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

/// Our stand-in for CS:S's scope: black outside a circle filling the
/// screen's height, thin black cross hairs through it, black bars beside it.
fn scope_image() -> Image {
    use bevy::{
        asset::RenderAssetUsages,
        render::render_resource::{Extent3d, TextureDimension, TextureFormat},
    };
    const N: u32 = 512;
    let mut data = Vec::with_capacity((N * N * 4) as usize);
    let c = (N as f32 - 1.0) / 2.0;
    for y in 0..N {
        for x in 0..N {
            let (dx, dy) = (x as f32 - c, y as f32 - c);
            let r = (dx * dx + dy * dy).sqrt() / c;
            // Soft edge over about a pixel; the cross hairs one pixel wide.
            let outside = ((r - 0.99) * c).clamp(0.0, 1.0);
            let line = if dx.abs() < 0.75 || dy.abs() < 0.75 { 1.0 } else { 0.0 };
            let alpha = outside.max(line);
            data.extend_from_slice(&[0, 0, 0, (alpha * 255.0) as u8]);
        }
    }
    Image::new(
        Extent3d {
            width: N,
            height: N,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        data,
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::RENDER_WORLD,
    )
}

fn spawn_scope(commands: &mut Commands, images: &mut Assets<Image>) {
    let image = images.add(scope_image());
    let bar = || {
        (
            Node {
                flex_grow: 1.0,
                height: percent(100.0),
                ..default()
            },
            BackgroundColor(Color::BLACK),
        )
    };
    commands.spawn((
        ScopeOverlay,
        Node {
            position_type: PositionType::Absolute,
            width: percent(100.0),
            height: percent(100.0),
            flex_direction: FlexDirection::Row,
            ..default()
        },
        GlobalZIndex(39),
        Visibility::Hidden,
        children![
            bar(),
            (
                ScopeSquare,
                Node {
                    height: percent(100.0),
                    aspect_ratio: Some(1.0),
                    flex_shrink: 0.0,
                    ..default()
                },
                ImageNode::new(image),
            ),
            bar(),
        ],
    ));
}

/// Full-screen tints: grey inside smoke (specs/cs_source/grenades.md
/// 7.4), under the flash overlay (`senses`) and the HUD.
#[derive(Component, Clone, Copy, PartialEq)]
enum ScreenTint {
    Smoke,
}

fn spawn_tints(commands: &mut Commands) {
    for (tint, z) in [(ScreenTint::Smoke, 30)] {
        commands.spawn((
            tint,
            Node {
                position_type: PositionType::Absolute,
                width: percent(100.0),
                height: percent(100.0),
                ..default()
            },
            BackgroundColor(Color::NONE),
            Pickable::IGNORE,
            GlobalZIndex(z),
        ));
    }
}

/// The smoke clouds' grey (colour 0.3, alpha from the camera's place in
/// each cloud).
fn screen_tints(
    mut tints: Query<(&ScreenTint, &mut BackgroundColor)>,
    camera: Query<&GlobalTransform, With<FirstPersonCamera>>,
    clouds: Query<&crate::weapon::grenade::SmokeCloud>,
    time: Res<Time>,
) {
    let now = time.elapsed_secs_f64();
    let smoke = camera.iter().next().map_or(0.0, |c| {
        crate::weapon::grenade::smoke_fog_at(c.translation(), now, clouds.iter())
    });
    for (tint, mut color) in &mut tints {
        let want = match tint {
            ScreenTint::Smoke => Color::srgba(0.3, 0.3, 0.3, smoke),
        };
        if color.0 != want {
            color.0 = want;
        }
    }
}

fn spawn_hud(mut commands: Commands, mut images: ResMut<Assets<Image>>) {
    spawn_tints(&mut commands);
    spawn_scope(&mut commands, &mut images);
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
    for dir in [Vec2::X, Vec2::NEG_X, Vec2::Y, Vec2::NEG_Y, Vec2::ZERO] {
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

type Local<'a> = (
    &'a Health,
    Option<&'a crate::weapon::Armor>,
    Option<&'a Inventory>,
    Option<&'a Score>,
    Option<&'a Dead>,
);

fn draw_hud(
    player: Option<Single<Local, With<LocalPlayer>>>,
    weapons: Query<(&Weapon, Option<&Magazine>)>,
    mut health_text: Single<&mut Text, (With<HealthText>, Without<AmmoText>, Without<CenterText>)>,
    mut ammo_text: Single<&mut Text, (With<AmmoText>, Without<HealthText>, Without<CenterText>)>,
    mut center: Single<&mut Text, (With<CenterText>, Without<HealthText>, Without<AmmoText>)>,
    game_hud: Option<Res<crate::map::hud::ActiveHud>>,
    spectator: Option<Res<super::spectate::Spectator>>,
) {
    let Some(p) = player else { return };
    // The game's own HUD draws health, armour and ammo when there is one;
    // spectating, the spectator panel says who you watch instead.
    let spectating = spectator.is_some_and(|s| s.active());
    let plain = game_hud.is_none() && !spectating;
    let (health, armor, inv, score, dead) = *p;
    let score = score.copied().unwrap_or_default();
    let armor = match armor.filter(|a| a.amount > 0.0) {
        Some(a) => format!(
            "    {} {:.0}",
            if a.helmet { "[H]" } else { "[ ]" },
            (a.amount * 100.0).round()
        ),
        None => String::new(),
    };
    // (With the game's HUD, kills and deaths belong on a scoreboard.)
    health_text.0 = if !plain {
        String::new()
    } else {
        format!(
            "+ {:.0}{armor}    K {}  D {}",
            (health.current * 100.0).ceil(),
            score.kills,
            score.deaths
        )
    };
    ammo_text.0 = if !plain {
        String::new()
    } else {
        match inv.and_then(|i| i.active).and_then(|w| weapons.get(w).ok()) {
            Some((w, Some(m))) => format!("{}\n{} | {}", short(w.id), m.clip, m.reserve),
            Some((w, None)) => short(w.id).to_string(),
            None => String::new(),
        }
    };
    center.0 = if dead.is_some() && !spectating {
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
#[allow(clippy::type_complexity)]
fn draw_crosshair(
    player: Option<Single<(&Inventory, Option<&Dead>, Option<&Zoomed>), With<LocalPlayer>>>,
    (spectating, watched): (Option<Res<super::spectate::SpecView>>, Query<(&Inventory, Option<&Zoomed>)>),
    scans: Query<&Hitscan>,
    camera: Query<(&Camera, &Projection), With<FirstPersonCamera>>,
    mut lines: Query<(&CrosshairLine, &mut Node, &mut Visibility, &mut BackgroundColor)>,
    color: Res<CrosshairColor>,
) {
    let Some((cam, proj)) = camera.iter().next() else {
        return;
    };
    let Some(size) = cam.logical_viewport_size() else {
        return;
    };
    // Watching someone in first person: their weapon's crosshair (CS:S
    // draws the watched player's, in your own crosshair settings).
    let in_eye = spectating.and_then(|s| s.in_eye).and_then(|t| watched.get(t).ok());
    let (inv, dead, zoomed) = match (in_eye, player.map(|p| *p)) {
        (Some((i, z)), _) => (Some(i), None, z),
        (None, Some((i, d, z))) => (Some(i), d, z),
        (None, None) => (None, None, None),
    };
    // A scope draws its own cross hairs.
    let visible = inv.is_some() && dead.is_none() && !zoomed.is_some_and(|z| z.scope);
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
    let spread_gap = spread / (fov / 2.0).tan() * size.y / 2.0;
    let shapes = color.lines(spread_gap, size.y);
    let centre = size / 2.0;
    for (line, mut node, mut vis, mut bg) in &mut lines {
        bg.set_if_neq(BackgroundColor(color.color()));
        *vis = if visible {
            Visibility::Inherited
        } else {
            Visibility::Hidden
        };
        let d = line.0;
        // The line on that side (UI y grows downward).
        let side = Vec2::new(d.x, -d.y);
        let Some(&(offset, Vec2 { x: w, y: h })) = shapes.iter().find(|(o, _)| o.normalize_or_zero() == side) else {
            continue;
        };
        let c = centre + offset;
        node.left = px(c.x - w / 2.0);
        node.top = px(c.y - h / 2.0);
        node.width = px(w);
        node.height = px(h);
    }
}

/// The scope overlay while the local player looks through a sniper scope.
/// With the game's own scope textures (`scope_arc`, `scope_lens` HUD
/// sprites), draw the ring from four mirrored quarters, the lens tint
/// under it and thin black cross hairs, instead of our stand-in.
fn game_scope(
    hud: Res<crate::map::hud::ActiveHud>,
    square: Single<Entity, With<ScopeSquare>>,
    mut commands: Commands,
) {
    let sprite = |name: &str| {
        let s = hud.0.sprites.get(name)?;
        Some(hud.1.get(&s.texture)?.clone())
    };
    let Some(arc) = sprite("scope_arc") else { return };
    let square = *square;
    let mut e = commands.entity(square);
    e.remove::<ImageNode>().despawn_related::<Children>();
    let abs = |left: f32, top: f32, w: f32, h: f32| Node {
        position_type: PositionType::Absolute,
        left: percent(left),
        top: percent(top),
        width: percent(w),
        height: percent(h),
        ..default()
    };
    e.with_children(|c| {
        if let Some(lens) = sprite("scope_lens") {
            c.spawn((abs(0.0, 0.0, 100.0, 100.0), ImageNode::new(lens)));
        }
        // Stored as the bottom-right quarter.
        for (left, top, flip_x, flip_y) in [(50.0, 50.0, false, false), (0.0, 50.0, true, false), (50.0, 0.0, false, true), (0.0, 0.0, true, true)] {
            c.spawn((
                abs(left, top, 50.0, 50.0),
                ImageNode {
                    image: arc.clone(),
                    color: Color::BLACK,
                    flip_x,
                    flip_y,
                    ..default()
                },
            ));
        }
        c.spawn((abs(0.0, 50.0, 100.0, 0.0), Outline::new(px(0.5), px(0.0), Color::BLACK)));
        c.spawn((abs(50.0, 0.0, 0.0, 100.0), Outline::new(px(0.5), px(0.0), Color::BLACK)));
    });
}

/// Shown while the local player looks through a scope, or while watching
/// in first person someone who does.
fn draw_scope(
    player: Option<Single<Option<&Zoomed>, With<LocalPlayer>>>,
    (spectating, watched): (Option<Res<super::spectate::SpecView>>, Query<&Zoomed>),
    mut overlay: Query<&mut Visibility, With<ScopeOverlay>>,
) {
    let scoped = match spectating.and_then(|s| s.in_eye) {
        Some(t) => watched.get(t).is_ok_and(|z| z.scope),
        None => player.is_some_and(|z| z.is_some_and(|z| z.scope)),
    };
    let want = if scoped {
        Visibility::Inherited
    } else {
        Visibility::Hidden
    };
    for mut v in &mut overlay {
        if *v != want {
            *v = want;
        }
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
    for e in events.read().filter(|e| e.shown()) {
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
    game_hud: Option<Res<crate::map::hud::ActiveHud>>,
) {
    // The game's own HUD shows its death notices instead.
    if game_hud.is_some() {
        died.clear();
        text.0.clear();
        return;
    }
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

/// A character drawn as a capsule (`character_bodies`).
#[derive(Component)]
pub struct CapsuleBody;

/// A capsule for every character that isn't the local player, on maps
/// without character models; on maps with them (map::attach_bodies) the
/// capsules go. Characters outlive map changes (a network game's above
/// all: a client gets them before the server's map is in), so this
/// follows the map, not the character's arrival: a client that joined
/// de_dust2 from the greybox kept a capsule through every model (seen
/// in the network soak's screenshots).
#[allow(clippy::type_complexity)]
fn character_bodies(
    characters: Query<(Entity, &Team, Has<CapsuleBody>), (With<Intent>, Without<LocalPlayer>)>,
    models: Option<Res<crate::map::CharacterModels>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut commands: Commands,
) {
    for (e, team, capsule) in &characters {
        match (models.is_some(), capsule) {
            (true, true) => {
                commands
                    .entity(e)
                    .remove::<(CapsuleBody, Mesh3d, MeshMaterial3d<StandardMaterial>)>();
            }
            (false, false) => {
                let color = match team.0 {
                    0 => Color::srgb(0.35, 0.5, 0.9),
                    _ => Color::srgb(0.85, 0.4, 0.3),
                };
                let r = crate::character::CAPSULE_RADIUS;
                commands.entity(e).insert((
                    CapsuleBody,
                    Mesh3d(meshes.add(Capsule3d::new(r, crate::character::CAPSULE_HEIGHT - 2.0 * r))),
                    MeshMaterial3d(materials.add(StandardMaterial {
                        base_color: color,
                        perceptual_roughness: 0.8,
                        ..default()
                    })),
                ));
            }
            _ => {}
        }
    }
}

/// The dead aren't drawn, nor the one a spectator looks out of.
fn show_bodies(
    mut bodies: Query<(Entity, &mut Visibility, Has<Dead>), (With<Intent>, Without<LocalPlayer>)>,
    spectating: Option<Res<super::spectate::SpecView>>,
) {
    let in_eye = spectating.and_then(|s| s.in_eye);
    for (e, mut vis, dead) in &mut bodies {
        let want = if dead || in_eye == Some(e) {
            Visibility::Hidden
        } else {
            Visibility::Inherited
        };
        if *vis != want {
            *vis = want;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::console::Console;

    fn crosshair_after(line: &str) -> CrosshairColor {
        let mut app = App::new();
        app.add_plugins(crate::console::ConsolePlugin).init_resource::<CrosshairColor>();
        crosshair_cvars(&mut app);
        app.world_mut().resource_mut::<Console>().submit(line);
        app.update();
        *app.world().resource::<CrosshairColor>()
    }

    /// Capsules follow the map, not the characters' arrival: a character
    /// that came in on the greybox (no models) loses its capsule on a
    /// map with models, and gets one again back on the greybox.
    #[test]
    fn capsule_bodies_follow_the_map() {
        use bevy::ecs::system::RunSystemOnce;
        let mut world = World::new();
        world.init_resource::<Assets<Mesh>>();
        world.init_resource::<Assets<StandardMaterial>>();
        let other = world.spawn((Intent::default(), Team(1))).id();
        let me = world.spawn((Intent::default(), Team(2), LocalPlayer)).id();
        let mut run = |world: &mut World| world.run_system_once(character_bodies).unwrap();
        run(&mut world);
        assert!(world.get::<CapsuleBody>(other).is_some() && world.get::<Mesh3d>(other).is_some());
        assert!(world.get::<CapsuleBody>(me).is_none(), "not the local player");
        world.insert_resource(crate::map::CharacterModels(Default::default()));
        run(&mut world);
        assert!(world.get::<CapsuleBody>(other).is_none() && world.get::<Mesh3d>(other).is_none());
        world.remove_resource::<crate::map::CharacterModels>();
        run(&mut world);
        assert!(world.get::<CapsuleBody>(other).is_some());
    }

    #[test]
    fn the_default_crosshair_is_ours_unchanged() {
        let c = CrosshairColor::default();
        let l = c.lines(0.0, 720.0);
        assert_eq!(l[0], (Vec2::new(MIN_GAP + LINE_LENGTH / 2.0, 0.0), Vec2::new(LINE_LENGTH, LINE_WIDTH)));
        assert_eq!(l[4].1, Vec2::ZERO, "no dot");
    }

    #[test]
    fn size_thickness_and_dot_change_the_drawn_crosshair() {
        let base = CrosshairColor::default().lines(0.0, 720.0);
        let big = crosshair_after("cl_crosshairsize 10; cl_crosshairthickness 1; cl_crosshairdot 1").lines(0.0, 720.0);
        assert_eq!(big[0].1, Vec2::new(LINE_LENGTH * 2.0, LINE_WIDTH * 2.0), "twice as long and wide");
        assert!(big[0].0.x > base[0].0.x, "the line's centre moves out with its length");
        assert_eq!(big[2].1, Vec2::new(LINE_WIDTH * 2.0, LINE_LENGTH * 2.0));
        assert_eq!(big[4], (Vec2::ZERO, Vec2::splat(LINE_WIDTH * 2.0)), "a centre dot as wide as the lines");
        let none = crosshair_after("cl_crosshairsize 0").lines(0.0, 720.0);
        assert_eq!(none[0].1.x, 0.0, "size 0: no lines");
        let thin = crosshair_after("cl_crosshairthickness 0").lines(0.0, 720.0);
        assert_eq!(thin[0].1.y, 1.0, "at least a pixel wide");
    }

    #[test]
    fn custom_colour_and_translucency() {
        let c = crosshair_after("cl_crosshaircolor 5; cl_crosshaircolor_r 255; cl_crosshaircolor_g 0; cl_crosshaircolor_b 128");
        let rgba = c.color().to_srgba();
        assert_eq!((rgba.red, rgba.green), (1.0, 0.0));
        assert!((rgba.blue - 128.0 / 255.0).abs() < 1e-4);
        let preset = crosshair_after("cl_crosshaircolor 1").color().to_srgba();
        assert_eq!(preset.red, 1.0, "red preset");
        let faint = crosshair_after("cl_crosshairusealpha 1; cl_crosshairalpha 51").color();
        assert!((faint.alpha() - 0.2).abs() < 1e-4);
    }
}
