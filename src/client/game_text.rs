//! Map HUD messages (game_text; specs/source/entity_io.md, "game_text"):
//! one message per channel from `logic::HudMessages`, faded or scanned out
//! and placed as the client does. Rebuilt every frame (six channels at
//! most).

use bevy::{prelude::*, window::PrimaryWindow};

use crate::logic::{HudMessage, HudMessages};

/// Default HUD message font size at a 480-pixel-high screen (Trebuchet
/// 24), scaled with the window height like Source's proportional HUD.
const FONT_PX_AT_480: f32 = 24.0;

#[derive(Component)]
struct GameTextNode;

pub struct GameTextPlugin;

impl Plugin for GameTextPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<HudMessages>().add_systems(Update, draw);
    }
}

fn color(c: Vec3, a: f32) -> Color {
    Color::srgba(c.x, c.y, c.z, a)
}

fn draw(
    mut commands: Commands,
    mut messages: ResMut<HudMessages>,
    time: Res<Time>,
    nodes: Query<Entity, With<GameTextNode>>,
    window: Query<&Window, With<PrimaryWindow>>,
) {
    for e in &nodes {
        commands.entity(e).despawn();
    }
    let now = time.elapsed_secs_f64();
    let height = window.single().map_or(720.0, |w| w.height());
    for slot in messages.channels.iter_mut() {
        let Some((m, start)) = slot else { continue };
        let t = (now - *start) as f32;
        if t > m.lifetime() {
            *slot = None;
            continue;
        }
        spawn_message(&mut commands, m, t, height);
    }
}

fn spawn_message(commands: &mut Commands, m: &HudMessage, t: f32, height: f32) {
    let opacity = m.opacity(t);
    let main = Vec3::from(m.color.map(|v| v as f32 / 255.0));
    // Where the block sits: a full-screen flex box aligned per x and y.
    let (justify, left, right) = if m.x == -1.0 {
        (JustifyContent::Center, 0.0, 0.0)
    } else if m.x >= 0.0 {
        (JustifyContent::FlexStart, m.x * 100.0, 0.0)
    } else {
        (JustifyContent::FlexEnd, 0.0, -m.x * 100.0)
    };
    let (align, top, bottom) = if m.y == -1.0 {
        (AlignItems::Center, 0.0, 0.0)
    } else if m.y >= 0.0 {
        (AlignItems::FlexStart, m.y * 100.0, 0.0)
    } else {
        (AlignItems::FlexEnd, 0.0, -m.y * 100.0)
    };
    let text_justify = if m.x == -1.0 { Justify::Center } else { Justify::Left };
    let font = TextFont {
        font_size: FontSize::Px(FONT_PX_AT_480 * height / 480.0),
        ..default()
    };
    let root = commands
        .spawn((
            GameTextNode,
            Node {
                position_type: PositionType::Absolute,
                width: percent(100.0),
                height: percent(100.0),
                justify_content: justify,
                align_items: align,
                padding: UiRect {
                    left: percent(left),
                    right: percent(right),
                    top: percent(top),
                    bottom: percent(bottom),
                },
                ..default()
            },
            GlobalZIndex(45),
        ))
        .id();
    let text = commands
        .spawn((
            Text::default(),
            font.clone(),
            TextLayout::justify(text_justify),
            TextColor(color(main, opacity)),
            ChildOf(root),
        ))
        .id();
    let shown = m.shown_text();
    if m.effect == 2 {
        // Scan out: each character appears in colour 2 and blends to the
        // colour; ones not yet shown keep their place, invisible.
        for (k, ch) in shown.chars().enumerate() {
            let c = m.scan_color(k, t).map_or(Color::NONE, |c| color(c, opacity));
            commands.spawn((TextSpan::new(ch.to_string()), font.clone(), TextColor(c), ChildOf(text)));
        }
    } else {
        commands.spawn((TextSpan::new(shown), font, TextColor(color(main, opacity)), ChildOf(text)));
    }
}
