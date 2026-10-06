//! Agent-facing capture modes:
//! - `--screenshot` / `--frames`: run a fixed number of frames, optionally
//!   save a screenshot of the window, then exit.
//! - `screenshot <file.png>` (console, also over the remote console):
//!   save the window now, so a live run can be photographed step by step.
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
use crate::console::ConsoleAppExt;
use crate::core::{Intent, LocalPlayer, Velocity};

const DEFAULT_FRAMES: u32 = 60;
/// Size of `--views` captures (CS:S comparisons use the same).
pub const VIEW_SIZE: UVec2 = UVec2::new(1280, 720);
/// Frames to let a view settle (interpolation, streaming) before capture.
const SETTLE_FRAMES: u32 = 30;
/// The first view settles longer: shader pipelines compile in the
/// background during the first frames, and meshes wait for them.
const FIRST_SETTLE_FRAMES: u32 = 150;
/// Frames timed per view by `--bench`, after settling.
const BENCH_FRAMES: u32 = 200;

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
    /// `--bench`: this view's frame times (seconds) and the results so far.
    times: Vec<f32>,
    /// Process CPU time when this view's timing started.
    cpu_start: Option<f64>,
    results: Vec<BenchRow>,
}

struct BenchRow {
    name: String,
    avg: f32,
    p95: f32,
    max: f32,
    meshes: (usize, usize, usize),
    parts: (usize, usize),
    /// Process CPU (all threads) and GPU ms per frame, when known.
    cpu: Option<f32>,
    gpu: Option<f32>,
}

#[derive(Component)]
struct ViewShot;

pub struct CapturePlugin;

impl Plugin for CapturePlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Update, (countdown, start_views, run_views.after(start_views)));
        app.console_command(
            "screenshot",
            "screenshot <file.png>: save the window as it is now.",
            |w, a| {
                let path = PathBuf::from(a.first().ok_or("screenshot <file.png>")?);
                w.spawn(Screenshot::primary_window()).observe(save_to_disk(path.clone()));
                Ok(Some(format!("saving {}", path.display())))
            },
        );
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
    mut windows: Query<&mut Window>,
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
    if args.0.bench {
        // Time what the GPU can do, not the display's refresh.
        for mut w in &mut windows {
            w.present_mode = bevy::window::PresentMode::AutoNoVsync;
        }
        info!("timing {} views", views.len());
    } else {
        info!("capturing {} views into {}", views.len(), dir.display());
    }
    commands.insert_resource(ViewRun {
        views,
        dir,
        target,
        index: 0,
        frame: 0,
        waiting: false,
        times: Vec::new(),
        cpu_start: None,
        results: Vec::new(),
    });
}

#[allow(clippy::too_many_arguments)]
fn run_views(
    mut commands: Commands,
    args: Res<ClientArgs>,
    run: Option<ResMut<ViewRun>>,
    mut player: Query<(&mut Transform, &mut Intent, &mut Velocity), With<LocalPlayer>>,
    mut exit: MessageWriter<AppExit>,
    time: Res<Time<Real>>,
    meshes: Query<(&Mesh3d, &ViewVisibility), With<bevy::camera::primitives::Aabb>>,
    assets: Res<super::perf::MeshTriangles>,
    vis: Res<crate::map::vis::VisStats>,
    diagnostics: Res<bevy::diagnostic::DiagnosticsStore>,
) {
    let Some(mut run) = run else { return };
    if run.waiting {
        return;
    }
    let Some(view) = run.views.get(run.index).cloned() else {
        if args.0.bench {
            print_bench(&run.results);
        } else {
            info!("all views captured");
        }
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
    let settle = if run.index == 0 { FIRST_SETTLE_FRAMES } else { SETTLE_FRAMES };
    if run.frame < settle {
        return;
    }
    if args.0.bench {
        // Frame times as seen at this point of each frame: the time since
        // the previous frame started.
        if run.times.is_empty() {
            run.cpu_start = super::perf::process_cpu_seconds();
        }
        run.times.push(time.delta_secs());
        if run.frame < settle + BENCH_FRAMES {
            return;
        }
        let mut t = std::mem::take(&mut run.times);
        t.sort_by(f32::total_cmp);
        let avg = t.iter().sum::<f32>() / t.len() as f32;
        let row = BenchRow {
            name: view.name.clone(),
            avg: avg * 1e3,
            p95: t[(t.len() * 95 / 100).min(t.len() - 1)] * 1e3,
            max: t[t.len() - 1] * 1e3,
            meshes: super::perf::mesh_counts(&meshes, &assets),
            parts: (vis.visible_parts, vis.parts),
            cpu: run
                .cpu_start
                .zip(super::perf::process_cpu_seconds())
                .map(|(a, b)| ((b - a) * 1e3 / t.len() as f64) as f32),
            gpu: super::perf::gpu_ms(&diagnostics).map(|g| g as f32),
        };
        run.results.push(row);
        run.index += 1;
        run.frame = 0;
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

/// Print `--bench` results: one row per view, then the averages. Frame
/// times follow machine load; process CPU per frame (all threads) and GPU
/// time much less.
fn print_bench(rows: &[BenchRow]) {
    let opt = |v: Option<f32>| v.map_or("n/a".to_string(), |v| format!("{v:.2}"));
    println!(
        "{:<24} {:>8} {:>8} {:>8} {:>8} {:>8} {:>10} {:>9} {:>11}",
        "view", "avg ms", "p95 ms", "max ms", "cpu ms", "gpu ms", "meshes", "tris (k)", "vis parts"
    );
    for r in rows {
        println!(
            "{:<24} {:>8.2} {:>8.2} {:>8.2} {:>8} {:>8} {:>10} {:>9} {:>11}",
            r.name,
            r.avg,
            r.p95,
            r.max,
            opt(r.cpu),
            opt(r.gpu),
            format!("{}/{}", r.meshes.1, r.meshes.0),
            r.meshes.2 / 1000,
            format!("{}/{}", r.parts.0, r.parts.1),
        );
    }
    let n = rows.len().max(1) as f32;
    let mean = |f: &dyn Fn(&BenchRow) -> f32| rows.iter().map(f).sum::<f32>() / n;
    let mean_opt = |f: &dyn Fn(&BenchRow) -> Option<f32>| {
        rows.iter().map(f).collect::<Option<Vec<f32>>>().map(|v| v.iter().sum::<f32>() / n)
    };
    println!(
        "{:<24} {:>8.2} {:>8.2} {:>8} {:>8} {:>8} {:>10.0} {:>9.0}",
        "MEAN",
        mean(&|r| r.avg),
        mean(&|r| r.p95),
        "",
        opt(mean_opt(&|r| r.cpu)),
        opt(mean_opt(&|r| r.gpu)),
        mean(&|r| r.meshes.1 as f32),
        mean(&|r| r.meshes.2 as f32) / 1000.0,
    );
}
