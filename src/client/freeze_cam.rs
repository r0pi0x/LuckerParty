//! The freeze cam's picture (`spectate::SpecPhase::FreezeCam`): once the
//! camera has travelled to the killer, the window's picture is taken and
//! held over the world (the world goes on underneath, unseen), a white
//! flash fades from it, the install's freeze sound plays
//! (`sound/ui/freeze_cam.wav`, `map::hud::UiSound::FreezeCam`), and the
//! game's freeze panel (`resource/ui/freezepanel_basic.res`) names the
//! killer. The flash's strength and length are guesses
//! (docs/tech-debt.md).

use bevy::{
    prelude::*,
    render::view::screenshot::{Screenshot, ScreenshotCaptured},
    window::PrimaryWindow,
};

use super::spectate::{SpecPhase, Spectator};
use crate::map::hud::ActiveHud;

/// The flash's starting opacity and how long it fades, seconds.
const FLASH_ALPHA: f32 = 0.7;
const FLASH_FADE: f32 = 0.35;

pub struct FreezeCamPlugin;

impl Plugin for FreezeCamPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Frozen>()
            .add_systems(Update, freeze.after(super::spectate::SpectateSet));
    }
}

/// The held picture: for which freeze (its start), when it was asked
/// for, and the picture once taken.
#[derive(Resource, Default)]
pub struct Frozen {
    pub since: Option<f64>,
    pub at: f64,
    pub image: Option<Handle<Image>>,
    shown: bool,
}

#[derive(Component)]
struct FreezeOverlay;

#[derive(Component)]
struct FreezeFlash;

#[allow(clippy::too_many_arguments, clippy::type_complexity)]
fn freeze(
    time: Res<Time>,
    spec: Res<Spectator>,
    mut frozen: ResMut<Frozen>,
    overlay: Query<Entity, With<FreezeOverlay>>,
    mut flash: Query<&mut BackgroundColor, With<FreezeFlash>>,
    window: Query<&Window, With<PrimaryWindow>>,
    who: Query<Option<&Name>>,
    hud: Option<Res<ActiveHud>>,
    fonts: Res<super::fonts::UiFonts>,
    menu: Option<Res<super::game_menu::GameMenu>>,
    clips: Option<ResMut<Assets<crate::map::live_sound::LiveClip>>>,
    mut commands: Commands,
) {
    let now = time.elapsed_secs_f64();
    let holding = match spec.phase {
        SpecPhase::FreezeCam {
            since, travel, killer, ..
        } if now - since >= travel as f64 => Some((since, killer)),
        _ => None,
    };
    let Some((since, killer)) = holding else {
        if frozen.since.is_some() {
            for e in &overlay {
                commands.entity(e).despawn();
            }
            *frozen = Frozen::default();
        }
        return;
    };
    if frozen.since != Some(since) {
        // Take the picture now: it arrives a frame or two later.
        for e in &overlay {
            commands.entity(e).despawn();
        }
        *frozen = Frozen {
            since: Some(since),
            at: now,
            ..default()
        };
        commands
            .spawn(Screenshot::primary_window())
            .observe(|shot: On<ScreenshotCaptured>, mut images: ResMut<Assets<Image>>, mut frozen: ResMut<Frozen>| {
                frozen.image = Some(images.add(shot.image.clone()));
            });
        // The freeze sound.
        let sound = menu
            .as_ref()
            .and_then(|m| m.ui.0.clone())
            .and_then(|ui| ui.sounds.get(&crate::map::hud::UiSound::FreezeCam).cloned());
        if let (Some(clip), Some(mut clips)) = (sound, clips) {
            let gains = std::sync::Arc::new(crate::map::live_sound::Gains::new(1.0, 1.0));
            let handle = clips.add(crate::map::live_sound::LiveClip::new(clip, gains, default()));
            commands.spawn((AudioPlayer(handle), PlaybackSettings::DESPAWN));
        }
        return;
    }
    if !frozen.shown
        && let Some(image) = frozen.image.clone()
    {
        frozen.shown = true;
        let root = commands
            .spawn((
                FreezeOverlay,
                ImageNode::new(image),
                Node {
                    position_type: PositionType::Absolute,
                    width: percent(100.0),
                    height: percent(100.0),
                    ..default()
                },
                GlobalZIndex(41),
            ))
            .id();
        commands.spawn((
            FreezeFlash,
            Node {
                position_type: PositionType::Absolute,
                width: percent(100.0),
                height: percent(100.0),
                ..default()
            },
            BackgroundColor(Color::srgba(1.0, 1.0, 1.0, FLASH_ALPHA)),
            ChildOf(root),
        ));
        let height = window.single().map_or(480.0, |w| w.height());
        let width = window.single().map_or(640.0, |w| w.width());
        let name = who.get(killer).ok().flatten().map_or("Player", |n| n.as_str()).to_string();
        if let Some(hud) = hud.as_deref() {
            panel(&mut commands, root, hud, &fonts, Vec2::new(width, height), &name);
        }
    }
    // The flash fades from when the picture shows.
    let t = (now - frozen.at) as f32;
    let a = FLASH_ALPHA * (1.0 - t / FLASH_FADE).clamp(0.0, 1.0);
    for mut c in &mut flash {
        c.set_if_neq(BackgroundColor(Color::srgba(1.0, 1.0, 1.0, a)));
    }
}

/// The game's freeze panel: its frame and, in it, the killer's name and
/// the words over it (the avatar, nemesis icon, screenshot hint and health
/// gauge are left out).
fn panel(
    commands: &mut Commands,
    root: Entity,
    hud: &ActiveHud,
    fonts: &super::fonts::UiFonts,
    size: Vec2,
    killer: &str,
) {
    let Some(menus) = hud.0.menus.as_ref() else { return };
    let Some((outer, inner)) = menus
        .freeze_panel
        .as_ref()
        .and_then(|(o, i)| Some((menus.layouts.get(o)?, menus.layouts.get(i)?)))
    else {
        return;
    };
    let painter = super::vgui::Painter::new(hud, fonts, size.y);
    let rects = outer.rects(Rect::from_corners(Vec2::ZERO, size), painter.scale);
    let Some((frame, r)) = outer
        .controls
        .iter()
        .zip(&rects)
        .find(|(c, _)| c.name.eq_ignore_ascii_case("FreezePanelBG"))
    else {
        return;
    };
    let bg = frame.bg.map_or(Color::srgba_u8(0, 0, 0, 192), |[r, g, b, a]| Color::srgba_u8(r, g, b, a));
    let e = commands
        .spawn((
            Node {
                position_type: PositionType::Absolute,
                left: px(r.min.x),
                top: px(r.min.y),
                width: px(r.width()),
                height: px(r.height()),
                ..default()
            },
            BackgroundColor(bg),
            ChildOf(root),
        ))
        .id();
    let words = freeze_words(&|t, f| menus.string(t, f).to_string(), killer);
    let mut shown = |c: &crate::map::hud::UiControl| {
        let mut s = super::vgui::Shown::of(c);
        match c.name.to_lowercase().as_str() {
            "killername" => s.text = Some(words.0.clone()),
            "infolabel1" => s.text = Some(words.1.clone()),
            "infolabel2" => s.text = Some(words.2.clone()),
            _ => s.visible = false,
        }
        s
    };
    painter.spawn(commands, e, r.size(), inner, super::vgui::VguiMenu::Spectator, &mut shown);
}

/// The freeze panel's three lines: the killer's name, then the game's
/// `FreezePanel_Killer1` and `FreezePanel_Killer2` ("KILLED YOU", "").
/// (Nemesis and revenge need the domination counts CS:S keeps; we don't.)
pub fn freeze_words(string: &dyn Fn(&str, &str) -> String, killer: &str) -> (String, String, String) {
    (
        killer.to_string(),
        string("#FreezePanel_Killer1", "KILLED YOU"),
        string("#FreezePanel_Killer2", ""),
    )
}
