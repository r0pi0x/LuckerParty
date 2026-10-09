//! What map entities put on the local player's screen
//! (specs/source/game_entities.md 3, 5): env_fade's fades (a full-screen
//! colour over the 3D view and HUD text, `logic::ScreenFades`) and
//! env_hudhint's key hint (the hint line, `chat::Hint`, with `%command%`
//! replaced by the key bound to it). A respawn clears the fades.

use bevy::prelude::*;

use crate::core::{Health, LocalPlayer};
use crate::logic::ScreenFades;

#[derive(Component)]
struct FadeNode;

pub struct MapScreenPlugin;

impl Plugin for MapScreenPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<ScreenFades>()
            .add_systems(Update, (clear_on_spawn, draw_fade, show_hint));
    }
}

/// The local player spawned (alive again): its fades go.
fn clear_on_spawn(
    local: Option<Single<&Health, With<LocalPlayer>>>,
    mut fades: ResMut<ScreenFades>,
    mut was_alive: Local<bool>,
) {
    let alive = local.is_some_and(|h| h.current > 0.0);
    if alive && !*was_alive && !fades.fades.is_empty() {
        fades.clear();
    }
    *was_alive = alive;
}

/// The fade as a full-screen colour (a modulating fade is drawn the same
/// way: spec open question 6).
fn draw_fade(
    time: Res<Time>,
    mut fades: ResMut<ScreenFades>,
    node: Query<(Entity, &mut BackgroundColor, &mut Visibility), With<FadeNode>>,
    mut commands: Commands,
) {
    let now = time.elapsed_secs_f64();
    let shown = if fades.fades.is_empty() {
        None
    } else {
        fades.bypass_change_detection().at(now)
    };
    let mut node = node;
    match (shown, node.single_mut()) {
        (Some((c, a, _)), Ok((_, mut bg, mut vis))) => {
            let color = Color::srgba_u8(c[0], c[1], c[2], a);
            if bg.0 != color {
                bg.0 = color;
            }
            vis.set_if_neq(Visibility::Inherited);
        }
        (Some((c, a, _)), Err(_)) => {
            commands.spawn((
                Name::new("Screen fade"),
                FadeNode,
                Node {
                    position_type: PositionType::Absolute,
                    left: Val::Px(0.0),
                    top: Val::Px(0.0),
                    width: Val::Percent(100.0),
                    height: Val::Percent(100.0),
                    ..default()
                },
                BackgroundColor(Color::srgba_u8(c[0], c[1], c[2], a)),
                // Over the 3D view and the game's text, under menus.
                GlobalZIndex(30),
                Pickable::IGNORE,
            ));
        }
        (None, Ok((_, _, mut vis))) => {
            vis.set_if_neq(Visibility::Hidden);
        }
        (None, Err(_)) => {}
    }
}

/// A key hint text as shown: a `#token` from the game's strings (unknown:
/// nothing), `%command%` replaced by the key bound to it.
pub fn hint_text(text: &str, token: impl Fn(&str) -> Option<String>, key: impl Fn(&str) -> Option<String>) -> String {
    let text = if text.starts_with('#') {
        token(text).unwrap_or_default()
    } else {
        text.to_string()
    };
    let mut out = String::new();
    let mut rest = text.as_str();
    while let Some(i) = rest.find('%') {
        out.push_str(&rest[..i]);
        let after = &rest[i + 1..];
        match after.find('%') {
            Some(j) => {
                let command = &after[..j];
                out.push_str(&key(command).unwrap_or_else(|| command.to_string()));
                rest = &after[j + 1..];
            }
            None => {
                out.push_str(&rest[i..]);
                rest = "";
            }
        }
    }
    out.push_str(rest);
    out
}

/// A new key hint goes to the hint line.
fn show_hint(
    fades: Res<ScreenFades>,
    console: Option<Res<crate::console::Console>>,
    hud: Option<Res<crate::map::hud::ActiveHud>>,
    mut hints: MessageWriter<super::chat::Hint>,
    mut last: Local<Option<f64>>,
) {
    let Some((text, at)) = fades.hint.as_ref() else { return };
    if *last == Some(*at) {
        return;
    }
    *last = Some(*at);
    let shown = hint_text(
        text,
        |t| {
            let t = t.trim().trim_start_matches('#').to_lowercase();
            hud.as_ref()?.0.menus.as_ref()?.strings.get(&t).cloned()
        },
        |c| {
            let binds = &console.as_ref()?.binds;
            super::binds::keys_for(binds, c).first().map(|k| super::binds::display(k))
        },
    );
    if !shown.is_empty() {
        hints.write(super::chat::Hint(shown));
    }
}
