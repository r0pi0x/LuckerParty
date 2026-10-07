//! The radar, CS:S style: the map's overview picture under a square in the
//! HUD's `HudRadar` box, turned so the way you face is up, with you at the
//! centre; teammates always, enemies while you can see them (CS:S shows
//! enemies any teammate has spotted); your place name (from the nav mesh)
//! below it, as `HudLocation` does.

use avian3d::prelude::*;
use bevy::{
    asset::embedded_asset, prelude::*, reflect::TypePath, render::render_resource::AsBindGroup, shader::ShaderRef,
};

use crate::{
    core::{Health, Intent, LocalPlayer, Team},
    map::{
        hud::{ActiveHud, ActiveOverview, HudCoord},
        nav::NavMesh,
    },
    rules::Dead,
};

pub struct RadarPlugin;

impl Plugin for RadarPlugin {
    fn build(&self, app: &mut App) {
        embedded_asset!(app, "radar.wgsl");
        app.add_plugins(UiMaterialPlugin::<RadarMaterial>::default());
        app.add_systems(
            Update,
            (
                build.run_if(resource_exists_and_changed::<ActiveOverview>),
                teardown.run_if(resource_removed::<ActiveOverview>),
                update.run_if(resource_exists::<ActiveOverview>),
            ),
        );
    }
}

/// Map units across the radar: what it shows edge to edge (an assumption;
/// CS:S's radar range isn't in its data files).
const RADAR_RANGE_UNITS: f32 = 2200.0;
const DOT: f32 = 4.0;

/// The overview drawn around the player, turned and clipped in the shader
/// (UI clipping doesn't apply to rotated nodes).
#[derive(Asset, TypePath, AsBindGroup, Debug, Clone)]
struct RadarMaterial {
    /// Centre uv, span, angle, alpha (see radar.wgsl).
    #[uniform(0)]
    params: RadarParams,
    #[texture(1)]
    #[sampler(2)]
    overview: Handle<Image>,
}

#[derive(Clone, Copy, Debug, Default, bevy::render::render_resource::ShaderType)]
struct RadarParams {
    centre: Vec2,
    span: Vec2,
    angle: f32,
    alpha: f32,
    pad: Vec2,
}

impl UiMaterial for RadarMaterial {
    fn fragment_shader() -> ShaderRef {
        "embedded://mashup/client/radar.wgsl".into()
    }
}

#[derive(Component)]
struct RadarPart;
#[derive(Component)]
struct RadarFrame;
#[derive(Component)]
struct RadarPivot;
#[derive(Component)]
struct RadarMap;
#[derive(Component)]
struct RadarDot(Entity);
#[derive(Component)]
struct PlaceText;

fn build(
    overview: Res<ActiveOverview>,
    old: Query<Entity, With<RadarPart>>,
    mut materials: ResMut<Assets<RadarMaterial>>,
    mut commands: Commands,
) {
    for e in &old {
        commands.entity(e).despawn();
    }
    let frame = commands
        .spawn((
            RadarPart,
            RadarFrame,
            Node {
                position_type: PositionType::Absolute,
                overflow: Overflow::clip(),
                border: UiRect::all(px(1.0)),
                ..default()
            },
            BackgroundColor(Color::srgba(0.0, 0.0, 0.0, 0.45)),
            BorderColor::all(Color::srgba(1.0, 0.69, 0.0, 0.5)),
            GlobalZIndex(38),
        ))
        .id();
    commands.spawn((
        RadarPivot,
        // Over the map drawn by the material node.
        ZIndex(1),
        Node {
            position_type: PositionType::Absolute,
            left: percent(50.0),
            top: percent(50.0),
            width: px(0.0),
            height: px(0.0),
            ..default()
        },
        ChildOf(frame),
    ));
    commands.spawn((
        RadarMap,
        MaterialNode(materials.add(RadarMaterial {
            params: RadarParams {
                alpha: 0.85,
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
        ChildOf(frame),
    ));
    // You: a dot at the centre with a tick for where you face.
    commands.spawn((
        Node {
            position_type: PositionType::Absolute,
            left: percent(50.0),
            top: percent(50.0),
            width: px(DOT),
            height: px(DOT),
            margin: UiRect::new(px(-DOT / 2.0), px(0.0), px(-DOT / 2.0), px(0.0)),
            border_radius: BorderRadius::MAX,
            ..default()
        },
        BackgroundColor(Color::WHITE),
        ChildOf(frame),
    ));
    commands.spawn((
        RadarPart,
        PlaceText,
        Text::default(),
        TextFont {
            font_size: FontSize::Px(12.0),
            ..default()
        },
        TextColor(Color::srgb(1.0, 0.69, 0.0)),
        Node {
            position_type: PositionType::Absolute,
            ..default()
        },
        GlobalZIndex(38),
    ));
}

fn teardown(parts: Query<Entity, With<RadarPart>>, mut commands: Commands) {
    for e in &parts {
        commands.entity(e).despawn();
    }
}

#[allow(clippy::type_complexity, clippy::too_many_arguments)]
fn update(
    overview: Res<ActiveOverview>,
    hud: Option<Res<ActiveHud>>,
    nav: Option<Res<NavMesh>>,
    windows: Query<&Window>,
    me: Option<Single<(Entity, &GlobalTransform, &Intent, Option<&Team>, Has<Dead>), With<LocalPlayer>>>,
    others: Query<(Entity, &GlobalTransform, Option<&Team>, Option<&Health>), (With<Intent>, Without<LocalPlayer>)>,
    spatial: SpatialQuery,
    mut frame: Query<(&mut Node, &mut Visibility), (With<RadarFrame>, Without<RadarMap>, Without<PlaceText>)>,
    pivot: Query<Entity, With<RadarPivot>>,
    map: Query<&MaterialNode<RadarMaterial>, With<RadarMap>>,
    mut materials: ResMut<Assets<RadarMaterial>>,
    mut dots: Query<
        (Entity, &RadarDot, &mut Node, &mut BackgroundColor),
        (Without<RadarFrame>, Without<RadarMap>, Without<PlaceText>),
    >,
    mut place: Query<
        (&mut Text, &mut Node, &mut TextFont),
        (
            With<PlaceText>,
            Without<RadarFrame>,
            Without<RadarMap>,
            Without<RadarDot>,
        ),
    >,
    bombs: (
        Query<(Entity, &GlobalTransform), With<crate::objectives::bomb::PlantedBomb>>,
        Query<(Entity, &GlobalTransform, &crate::weapon::drop::Loose)>,
        Query<(), With<crate::objectives::bomb::C4>>,
        Res<crate::objectives::bomb::BombRules>,
    ),
    // Spectating: the watched player's radar.
    spectating: (
        Option<Res<super::spectate::Spectator>>,
        Query<(&GlobalTransform, &Intent, Option<&Team>, Option<&Dead>)>,
    ),
    mut commands: Commands,
) {
    let (Some(window), Ok((mut frame_node, mut frame_vis))) = (windows.iter().next(), frame.single_mut()) else {
        return;
    };
    let (w, h) = (window.width(), window.height());
    let scale = h / 480.0;
    // The HUD's radar box, else CS:S's (16, 16, 96 x 96).
    let (x, y, size) = hud
        .as_ref()
        .and_then(|hud| hud.0.panels.get("HudRadar"))
        .map_or((HudCoord::Start(16.0), HudCoord::Start(16.0), 96.0), |p| {
            (p.x, p.y, p.wide)
        });
    let (left, top, side) = (x.resolve(w, scale), y.resolve(h, scale), size * scale);
    frame_node.left = px(left);
    frame_node.top = px(top);
    frame_node.width = px(side);
    frame_node.height = px(side);
    let Some(me) = me else {
        *frame_vis = Visibility::Hidden;
        return;
    };
    let (me_entity, at, intent, team, dead) = *me;
    let (spectator, watched) = spectating;
    let target = spectator
        .filter(|s| s.phase == super::spectate::SpecPhase::Watching)
        .and_then(|s| s.target)
        .and_then(|t| watched.get(t).ok().map(|(g, i, tm, d)| (t, g, i, tm, d.is_some())));
    let (me_entity, at, intent, team, dead) = target.unwrap_or((me_entity, at, intent, team, dead));
    *frame_vis = if dead {
        Visibility::Hidden
    } else {
        Visibility::Inherited
    };
    let o = &overview.0;
    // Radar pixels per overview pixel.
    let k = side / (RADAR_RANGE_UNITS * 0.0254 / o.meters_per_pixel);
    let centre = o.pixel(at.translation());
    let Ok(pivot_entity) = pivot.single() else { return };
    // Clockwise by the look yaw puts the way you face at the top.
    let facing = if o.rotate {
        -intent.yaw - std::f32::consts::FRAC_PI_2
    } else {
        intent.yaw
    };
    // Only on a change: a mutable borrow alone re-prepares the material.
    let (centre_uv, span) = (centre / o.size, Vec2::splat(side / k) / o.size);
    if let Ok(m) = map.single()
        && materials
            .get(&m.0)
            .is_some_and(|m| (m.params.centre, m.params.span, m.params.angle) != (centre_uv, span, facing))
        && let Some(mut m) = materials.get_mut(&m.0)
    {
        m.params.centre = centre_uv;
        m.params.span = span;
        m.params.angle = facing;
    }
    // Dot offsets turn with the map.
    let turn = Rot2::radians(facing);
    // Dots: teammates always, enemies while in your sight.
    let eye = at.translation() + Vec3::Y * 0.6;
    let mut seen: Vec<(Entity, Vec2, Color)> = Vec::new();
    for (e, t, other_team, health) in &others {
        if e == me_entity || health.is_some_and(|h| h.current <= 0.0) {
            continue;
        }
        let friend = team.is_some() && other_team == team;
        let p = t.translation();
        let visible = friend || {
            let to = p + Vec3::Y * 0.5 - eye;
            Dir3::new(to).is_ok_and(|dir| {
                let filter = SpatialQueryFilter::from_excluded_entities([me_entity, e]).with_mask(crate::core::SOLID_LAYERS);
                spatial.cast_ray(eye, dir, to.length(), true, &filter).is_none()
            })
        };
        if !visible {
            continue;
        }
        let color = match other_team.map(|t| t.0) {
            Some(1) => Color::srgb(1.0, 0.3, 0.25),
            Some(2) => Color::srgb(0.45, 0.65, 1.0),
            _ => Color::srgb(0.9, 0.9, 0.9),
        };
        seen.push((e, turn * ((o.pixel(p) - centre) * k), color));
    }
    // The bomb: planted, for everyone; lying loose, for its carriers' team
    // (spec objectives.md Q12).
    let (planted, loose, c4s, rules) = bombs;
    let bomb_color = Color::srgb(1.0, 0.55, 0.1);
    for (e, t) in &planted {
        seen.push((e, turn * ((o.pixel(t.translation()) - centre) * k), bomb_color));
    }
    if team == Some(&rules.carrier_team) {
        for (e, t, l) in &loose {
            if c4s.contains(l.weapon) {
                seen.push((e, turn * ((o.pixel(t.translation()) - centre) * k), bomb_color));
            }
        }
    }
    for (dot, RadarDot(target), mut node, mut bg) in &mut dots {
        match seen.iter().position(|(e, ..)| e == target) {
            Some(i) => {
                let (_, at, color) = seen.swap_remove(i);
                node.left = px(at.x - DOT / 2.0);
                node.top = px(at.y - DOT / 2.0);
                bg.0 = color;
            }
            None => commands.entity(dot).despawn(),
        }
    }
    for (e, at, color) in seen {
        commands.spawn((
            RadarDot(e),
            Node {
                position_type: PositionType::Absolute,
                left: px(at.x - DOT / 2.0),
                top: px(at.y - DOT / 2.0),
                width: px(DOT),
                height: px(DOT),
                border_radius: BorderRadius::MAX,
                ..default()
            },
            BackgroundColor(color),
            ChildOf(pivot_entity),
        ));
    }
    // The place you're in, under the radar.
    if let Ok((mut text, mut node, mut font)) = place.single_mut() {
        let name = nav
            .as_ref()
            .and_then(|nav| {
                let area = nav.area_at(at.translation())?;
                nav.places.get(nav.areas[area].place?).cloned()
            })
            .unwrap_or_default();
        if text.0 != name {
            text.0 = name;
        }
        node.left = px(left);
        node.top = px(top + side + 2.0 * scale);
        font.font_size = FontSize::Px(9.0 * scale);
    }
}
