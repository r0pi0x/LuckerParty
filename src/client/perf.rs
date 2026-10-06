//! Performance tools: the `mashup_perf` overlay (frame times, main-world
//! CPU time, GPU time per render pass, entity and mesh counts, visibility
//! culling), `r_novis`, and `--bench` (time each `--views` view instead of
//! capturing it; see docs/performance.md).

use std::{collections::VecDeque, time::Instant};

use bevy::{
    camera::primitives::Aabb,
    diagnostic::{DiagnosticsStore, EntityCountDiagnosticsPlugin},
    prelude::*,
    render::diagnostic::RenderDiagnosticsPlugin,
};

use crate::{
    console::resource_cvar,
    map::vis::{NoVis, VisClusters, VisStats},
};

pub struct PerfPlugin;

impl Plugin for PerfPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins((EntityCountDiagnosticsPlugin::default(), RenderDiagnosticsPlugin))
            .init_resource::<Perf>()
            .init_resource::<FrameTimes>()
            .init_resource::<MeshTriangles>()
            .insert_resource(HostTimescale(1.0))
            .add_systems(PreStartup, timescale_from_command_line)
            .add_systems(First, apply_timescale.after(bevy::time::TimeSystems))
            .add_systems(Startup, spawn_overlay)
            .add_systems(First, start_frame)
            .add_systems(
                Last,
                (record_triangles, end_frame, draw_overlay.run_if(|p: Res<Perf>| p.show > 0), hide_overlay).chain(),
            );
        resource_cvar::<Perf, u8>(
            app,
            "mashup_perf",
            "1: performance overlay (frame times, CPU, GPU passes, meshes, visibility); 2: also every render pass.",
            |p| &mut p.show,
        );
        resource_cvar::<HostTimescale, f32>(
            app,
            "host_timescale",
            "Game time speed (1 normal); 0 freezes simulation, physics and effects (for identical captures).",
            |t| &mut t.0,
        );
        if app.world().contains_resource::<NoVis>() {
            resource_cvar::<NoVis, u8>(
                app,
                "r_novis",
                "1: draw every map part, ignoring the map's visibility data (prop fade distances still apply).",
                |v| &mut v.0,
            );
        }
    }
}

/// `host_timescale`: how fast game (virtual) time runs; 0 pauses it.
#[derive(Resource)]
struct HostTimescale(f32);

/// A `+host_timescale` on the command line holds from the first frame
/// (console lines run a frame or two later), so a frozen run is the same
/// every time.
fn timescale_from_command_line(args: Res<super::ClientArgs>, mut scale: ResMut<HostTimescale>) {
    for line in &args.0.console {
        let mut words = line.split_whitespace();
        if words.next() == Some("host_timescale")
            && let Some(v) = words.next().and_then(|v| v.trim_matches('"').parse().ok())
        {
            scale.0 = v;
        }
    }
}

fn apply_timescale(
    scale: Res<HostTimescale>,
    mut time: ResMut<Time<Virtual>>,
    physics: Option<ResMut<Time<avian3d::prelude::Physics>>>,
) {
    use avian3d::prelude::PhysicsTime;
    if !scale.is_changed() {
        return;
    }
    if scale.0 <= 0.0 {
        time.pause();
    } else {
        time.unpause();
        time.set_relative_speed(scale.0);
    }
    if let Some(mut physics) = physics {
        if scale.0 <= 0.0 {
            physics.pause();
        } else {
            physics.unpause();
        }
    }
}

#[derive(Resource, Default)]
struct Perf {
    show: u8,
}

/// Recent frames: wall time between frames and the main world's CPU time
/// (First to Last), seconds, over about the last second.
#[derive(Resource, Default)]
pub struct FrameTimes {
    started: Option<Instant>,
    last_start: Option<Instant>,
    pub frames: VecDeque<(f32, f32)>,
}

impl FrameTimes {
    /// Average, 95th percentile and maximum frame time, and the average
    /// main-world CPU time, milliseconds.
    pub fn summary(&self) -> (f32, f32, f32, f32) {
        let mut v: Vec<f32> = self.frames.iter().map(|f| f.0).collect();
        v.sort_by(f32::total_cmp);
        let n = v.len().max(1) as f32;
        let avg = v.iter().sum::<f32>() / n;
        let p95 = v.get(((v.len() as f32 * 0.95) as usize).min(v.len().saturating_sub(1))).copied().unwrap_or(0.0);
        let max = v.last().copied().unwrap_or(0.0);
        let cpu = self.frames.iter().map(|f| f.1).sum::<f32>() / n;
        (avg * 1e3, p95 * 1e3, max * 1e3, cpu * 1e3)
    }
}

fn start_frame(mut times: ResMut<FrameTimes>) {
    let now = Instant::now();
    times.last_start = times.started.replace(now);
}

fn end_frame(mut times: ResMut<FrameTimes>) {
    let (Some(start), Some(prev)) = (times.started, times.last_start) else {
        return;
    };
    let frame = (start - prev).as_secs_f32();
    let cpu = start.elapsed().as_secs_f32();
    times.frames.push_back((frame, cpu));
    while times.frames.iter().map(|f| f.0).sum::<f32>() > 1.0 && times.frames.len() > 1 {
        times.frames.pop_front();
    }
}

#[derive(Component)]
struct PerfText;

fn spawn_overlay(mut commands: Commands) {
    commands.spawn((
        PerfText,
        Text::default(),
        TextFont {
            font_size: FontSize::Px(13.0),
            ..default()
        },
        TextColor(Color::srgb(1.0, 1.0, 0.6)),
        Node {
            position_type: PositionType::Absolute,
            top: px(8.0),
            right: px(8.0),
            ..default()
        },
        Visibility::Hidden,
    ));
}

/// Triangles per mesh asset, noted when a mesh is added or changed (the
/// render world takes the data of render-only meshes once extracted).
#[derive(Resource, Default)]
pub struct MeshTriangles(std::collections::HashMap<AssetId<Mesh>, usize>);

fn record_triangles(
    mut events: MessageReader<AssetEvent<Mesh>>,
    assets: Res<Assets<Mesh>>,
    mut triangles: ResMut<MeshTriangles>,
) {
    for e in events.read() {
        match e {
            AssetEvent::Added { id } | AssetEvent::Modified { id } => {
                if let Some(m) = assets.get(*id)
                    && let Ok(Some(i)) = m.try_indices_option()
                {
                    triangles.0.insert(*id, i.len() / 3);
                }
            }
            AssetEvent::Removed { id } | AssetEvent::Unused { id } => {
                triangles.0.remove(id);
            }
            _ => {}
        }
    }
}

/// GPU time of a frame, ms: the top-level render passes' timestamps
/// (smoothed). None without GPU timestamps (not Vulkan/DX12).
pub fn gpu_ms(diagnostics: &DiagnosticsStore) -> Option<f64> {
    let mut any = false;
    let mut sum = 0.0;
    for d in diagnostics.iter() {
        if let Some(pass) = d.path().as_str().strip_prefix("render/").and_then(|p| p.strip_suffix("/elapsed_gpu"))
            && !pass.contains('/')
        {
            any = true;
            sum += d.smoothed().unwrap_or(0.0);
        }
    }
    any.then_some(sum)
}

/// CPU time this process has used so far, all threads (seconds); None
/// where it isn't read (only Linux so far). Unlike frame times, it barely
/// moves with other programs' load.
pub fn process_cpu_seconds() -> Option<f64> {
    #[cfg(target_os = "linux")]
    {
        // /proc/self/stat: utime and stime are fields 14 and 15, in clock
        // ticks (100 per second on Linux).
        let stat = std::fs::read_to_string("/proc/self/stat").ok()?;
        let rest = &stat[stat.rfind(')')? + 2..];
        let fields: Vec<&str> = rest.split_whitespace().collect();
        let ticks: f64 = fields.get(11)?.parse::<f64>().ok()? + fields.get(12)?.parse::<f64>().ok()?;
        Some(ticks / 100.0)
    }
    #[cfg(not(target_os = "linux"))]
    {
        None
    }
}

/// Mesh counts this frame: total, drawn in some view, and their triangles.
pub fn mesh_counts(
    meshes: &Query<(&Mesh3d, &ViewVisibility), With<Aabb>>,
    triangles: &MeshTriangles,
) -> (usize, usize, usize) {
    let (mut total, mut drawn, mut tris) = (0, 0, 0);
    for (mesh, seen) in meshes {
        total += 1;
        if seen.get() {
            drawn += 1;
            tris += triangles.0.get(&mesh.0.id()).copied().unwrap_or(0);
        }
    }
    (total, drawn, tris)
}

#[allow(clippy::too_many_arguments)]
fn draw_overlay(
    perf: Res<Perf>,
    times: Res<FrameTimes>,
    diagnostics: Res<DiagnosticsStore>,
    vis: Res<VisStats>,
    parts: Query<(), With<VisClusters>>,
    meshes: Query<(&Mesh3d, &ViewVisibility), With<Aabb>>,
    assets: Res<MeshTriangles>,
    mut text: Single<(&mut Text, &mut Visibility), With<PerfText>>,
) {
    let (avg, p95, max, cpu) = times.summary();
    let mut lines = vec![
        format!("{:.0} fps  frame {avg:.2} ms (p95 {p95:.2}, max {max:.2})", 1e3 / avg.max(1e-3)),
        format!("main world CPU {cpu:.2} ms"),
    ];
    // GPU and render-world CPU time per render pass (Vulkan/DX12 give GPU
    // timestamps; elsewhere only CPU).
    let mut passes: Vec<(String, f64, f64)> = Vec::new();
    for d in diagnostics.iter() {
        let path = d.path().as_str();
        if let Some(pass) = path.strip_prefix("render/").and_then(|p| p.strip_suffix("/elapsed_gpu")) {
            let cpu = diagnostics
                .get(&bevy::diagnostic::DiagnosticPath::new(format!("render/{pass}/elapsed_cpu")))
                .and_then(|c| c.smoothed())
                .unwrap_or(0.0);
            passes.push((pass.to_string(), d.smoothed().unwrap_or(0.0), cpu));
        }
    }
    passes.sort_by(|a, b| b.1.total_cmp(&a.1));
    if !passes.is_empty() {
        let gpu = gpu_ms(&diagnostics).unwrap_or(0.0);
        lines.push(format!("GPU {gpu:.2} ms (top-level passes)"));
        let shown = if perf.show > 1 { passes.len() } else { 6 };
        for (pass, g, c) in passes.iter().take(shown) {
            lines.push(format!("  {pass}: gpu {g:.2} cpu {c:.2}"));
        }
    }
    let entities = diagnostics
        .get(&EntityCountDiagnosticsPlugin::ENTITY_COUNT)
        .and_then(|d| d.value())
        .unwrap_or(0.0);
    let (total, drawn, triangles) = mesh_counts(&meshes, &assets);
    lines.push(format!("entities {entities:.0}  meshes {drawn}/{total} drawn, {}k triangles", triangles / 1000));
    match vis.cluster {
        Some(c) => lines.push(format!(
            "vis: cluster {c}, sees {}/{} clusters, {}/{} map parts",
            vis.visible_clusters, vis.clusters, vis.visible_parts, vis.parts
        )),
        None if vis.clusters > 0 => lines.push(format!("vis: off or outside the map ({} parts all drawn)", parts.iter().count())),
        None => lines.push("vis: map has no visibility data".into()),
    }
    let (text, visibility) = &mut *text;
    text.0 = lines.join("\n");
    visibility.set_if_neq(Visibility::Inherited);
}

/// Hide the overlay when `mashup_perf` goes back to 0.
fn hide_overlay(perf: Res<Perf>, mut text: Query<&mut Visibility, With<PerfText>>) {
    if perf.show == 0 {
        for mut v in &mut text {
            v.set_if_neq(Visibility::Hidden);
        }
    }
}
