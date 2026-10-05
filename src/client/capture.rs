//! `--screenshot` / `--frames`: run a fixed number of frames, optionally save
//! a screenshot, then exit. Lets an agent look at what the game renders.

use bevy::{
    prelude::*,
    render::view::screenshot::{Screenshot, ScreenshotCaptured, save_to_disk},
};

use super::ClientArgs;

const DEFAULT_FRAMES: u32 = 60;

pub struct CapturePlugin;

impl Plugin for CapturePlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Update, countdown);
    }
}

fn countdown(mut commands: Commands, args: Res<ClientArgs>, mut frame: Local<u32>, mut exit: MessageWriter<AppExit>) {
    let args = &args.0;
    if args.screenshot.is_none() && args.frames.is_none() {
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
