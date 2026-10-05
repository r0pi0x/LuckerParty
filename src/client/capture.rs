//! Agent-facing capture modes:
//! - `--screenshot` / `--frames`: run a fixed number of frames, optionally
//!   save a screenshot of the window, then exit.
//! - `--views <file.json> --capture-dir <dir>`: visit a list of camera views
//!   and save one PNG per view, rendered off-screen at a fixed size so the
//!   result doesn't depend on how the window manager sizes the window.

use std::path::PathBuf;

use bevy::{
    camera::RenderTarget,
    prelude::*,
    render::{
        render_resource::TextureFormat,
        view::screenshot::{Screenshot, ScreenshotCaptured, save_to_disk},
    },
};
use serde::Deserialize;

use super::{ClientArgs, FirstPersonCamera};
use crate::core::{Intent, LocalPlayer, Velocity};

const DEFAULT_FRAMES: u32 = 60;
/// Size of `--views` captures (CS:S comparisons use the same).
pub const VIEW_SIZE: UVec2 = UVec2::new(1280, 720);
/// Frames to let a view settle (interpolation, streaming) before capture.
const SETTLE_FRAMES: u32 = 30;

/// One camera view, in engine space: eye position (meters) and look angles
/// in degrees (yaw 0 = -Z, positive pitch looks up).
#[derive(Clone, Debug, Deserialize)]
pub struct View {
    pub name: String,
    pub position: [f32; 3],
    pub yaw: f32,
    pub pitch: f32,
}

#[derive(Resource)]
struct ViewRun {
    views: Vec<View>,
    dir: PathBuf,
    target: Handle<Image>,
    index: usize,
    frame: u32,
    waiting: bool,
}

#[derive(Component)]
struct ViewShot;

pub struct CapturePlugin;

impl Plugin for CapturePlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Update, (countdown, start_views, run_views.after(start_views)));
    }
}

fn countdown(mut commands: Commands, args: Res<ClientArgs>, mut frame: Local<u32>, mut exit: MessageWriter<AppExit>) {
    let args = &args.0;
    if args.views.is_some() || (args.screenshot.is_none() && args.frames.is_none()) {
        return;
    }
    *frame += 1;
    if *frame != args.frames.unwrap_or(DEFAULT_FRAMES) {
        return;
    }
    match &args.screenshot {
        Some(path) => {
            info!("saving screenshot to {}", path.display());
            commands
                .spawn(Screenshot::primary_window())
                .observe(save_to_disk(path.clone()))
                .observe(|_: On<ScreenshotCaptured>, mut exit: MessageWriter<AppExit>| {
                    exit.write(AppExit::Success);
                });
        }
        None => {
            exit.write(AppExit::Success);
        }
    }
}

fn start_views(
    mut commands: Commands,
    args: Res<ClientArgs>,
    run: Option<Res<ViewRun>>,
    camera: Option<Single<Entity, With<FirstPersonCamera>>>,
    mut images: ResMut<Assets<Image>>,
    mut exit: MessageWriter<AppExit>,
) {
    let (Some(path), None, Some(camera)) = (&args.0.views, run, camera) else {
        return;
    };
    let views: Vec<View> = match std::fs::read_to_string(path)
        .map_err(|e| e.to_string())
        .and_then(|t| serde_json::from_str(&t).map_err(|e| e.to_string()))
    {
        Ok(v) => v,
        Err(e) => {
            error!("--views {}: {e}", path.display());
            exit.write(AppExit::error());
            return;
        }
    };
    let dir = args.0.capture_dir.clone().unwrap_or_else(|| PathBuf::from("."));
    if let Err(e) = std::fs::create_dir_all(&dir) {
        error!("--capture-dir {}: {e}", dir.display());
        exit.write(AppExit::error());
        return;
    }
    let target = images.add(Image::new_target_texture(
        VIEW_SIZE.x,
        VIEW_SIZE.y,
        TextureFormat::Rgba8Unorm,
        Some(TextureFormat::Rgba8UnormSrgb),
    ));
    commands
        .entity(*camera)
        .insert(RenderTarget::Image(target.clone().into()));
    info!("capturing {} views into {}", views.len(), dir.display());
    commands.insert_resource(ViewRun {
        views,
        dir,
        target,
        index: 0,
        frame: 0,
        waiting: false,
    });
}

fn run_views(
    mut commands: Commands,
    run: Option<ResMut<ViewRun>>,
    mut player: Query<(&mut Transform, &mut Intent, &mut Velocity), With<LocalPlayer>>,
    mut exit: MessageWriter<AppExit>,
) {
    let Some(mut run) = run else { return };
    if run.waiting {
        return;
    }
    let Some(view) = run.views.get(run.index).cloned() else {
        info!("all views captured");
        exit.write(AppExit::Success);
        return;
    };
    let Ok((mut transform, mut intent, mut velocity)) = player.single_mut() else {
        return;
    };
    // Hold the camera on the view every frame (movement must be noclip).
    transform.translation = Vec3::from(view.position);
    intent.yaw = view.yaw.to_radians();
    intent.pitch = view.pitch.to_radians();
    intent.move_axis = Vec2::ZERO;
    velocity.0 = Vec3::ZERO;
    run.frame += 1;
    if run.frame < SETTLE_FRAMES {
        return;
    }
    run.waiting = true;
    let path = run.dir.join(format!("{}.png", view.name));
    commands
        .spawn((ViewShot, Screenshot::image(run.target.clone())))
        .observe(save_to_disk(path))
        .observe(|_: On<ScreenshotCaptured>, mut run: ResMut<ViewRun>| {
            run.index += 1;
            run.frame = 0;
            run.waiting = false;
        });
}
