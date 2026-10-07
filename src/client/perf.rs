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
            .init_resource::<Churn>()
            .insert_resource(HostTimescale(1.0))
            .add_systems(PreStartup, timescale_from_command_line)
            .add_systems(First, apply_timescale.after(bevy::time::TimeSystems))
            .add_systems(Startup, spawn_overlay)
            .add_systems(First, start_frame)
            .add_systems(
                Last,
                (record_triangles, count_churn, end_frame, draw_overlay.run_if(|p: Res<Perf>| p.show > 0), hide_overlay).chain(),
            );
        resource_cvar::<Perf, u8>(
            app,
            "mashup_perf",
            "1: performance overlay (frame times, CPU, GPU passes, meshes, visibility, per-frame changes); \
             2: also every render pass; 3: also log what writes transforms, each second.",
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
        if app.world().contains_resource::<crate::map::vis::PortalsOpenAll>() {
            resource_cvar::<crate::map::vis::PortalsOpenAll, u8>(
                app,
                "r_portalsopenall",
                "1: treat every areaportal as open and don't clip the view through them (PVS culling only).",
                |v| &mut v.0,
            );
        }
        water_cvars(app);
    }
}

/// Source's water settings (specs/cs_source/water.md section 1) on
/// `map::water::WaterSettings`.
fn water_cvars(app: &mut App) {
    use crate::map::water::WaterSettings;
    app.init_resource::<WaterSettings>();
    resource_cvar::<WaterSettings, u8>(
        app,
        "r_waterforceexpensive",
        "1: planar water reflections even where a material turns $forceexpensive off.",
        |s| &mut s.force_expensive,
    );
    resource_cvar::<WaterSettings, u8>(
        app,
        "r_waterforcereflectentities",
        "1: water reflections show models (props, players) as well as the world.",
        |s| &mut s.reflect_entities,
    );
    resource_cvar::<WaterSettings, u8>(app, "r_WaterDrawReflection", "0: no planar water reflections.", |s| {
        &mut s.draw_reflection
    });
    resource_cvar::<WaterSettings, u8>(
        app,
        "r_WaterDrawRefraction",
        "0: water shows nothing below its surface (cubemap only).",
        |s| &mut s.draw_refraction,
    );
    resource_cvar::<WaterSettings, u8>(app, "mat_drawwater", "0: hide water surfaces.", |s| &mut s.draw_water);
    let mut console = app.world_mut().resource_mut::<crate::console::Console>();
    for name in ["r_waterforceexpensive", "r_waterforcereflectentities"] {
        console.archive(name);
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

/// What changes each frame, averaged over about a second: transforms
/// written (each dirties its tree for transform propagation; one under the
/// map's root dirties the map's), and mesh and material assets modified
/// (each is prepared for the GPU again, and any modified mesh makes every
/// mesh entity re-check its pipeline). Writes that store the same value
/// count too: avoid them.
#[derive(Resource, Default)]
struct Churn {
    frames: u32,
    started: Option<Instant>,
    sums: [u32; 4],
    names: std::collections::HashMap<String, u32>,
    /// Last second's averages: transforms, of them under the map's root,
    /// meshes, materials.
    shown: [f32; 4],
}

#[allow(clippy::too_many_arguments)]
fn count_churn(
    perf: Res<Perf>,
    mut churn: ResMut<Churn>,
    changed: Query<Entity, Changed<Transform>>,
    parents: Query<&ChildOf>,
    names: Query<&Name>,
    map_parts: Query<(), With<crate::map::MapPart>>,
    mut meshes: MessageReader<AssetEvent<Mesh>>,
    mut props: MessageReader<AssetEvent<crate::map::prop_material::PropMaterial>>,
    mut standard: MessageReader<AssetEvent<StandardMaterial>>,
    mut sprites: MessageReader<AssetEvent<crate::map::sprite_material::SpriteMaterial>>,
    mut particles: MessageReader<AssetEvent<crate::map::particles::ParticleDrawMaterial>>,
    mut decals: MessageReader<AssetEvent<crate::map::decal::DecalMaterial>>,
) {
    fn modified<A: Asset>(events: &mut MessageReader<AssetEvent<A>>) -> u32 {
        events.read().filter(|e| matches!(e, AssetEvent::Modified { .. })).count() as u32
    }
    let mesh_count = modified(&mut meshes);
    let material_count = modified(&mut props)
        + modified(&mut standard)
        + modified(&mut sprites)
        + modified(&mut particles)
        + modified(&mut decals);
    if perf.show == 0 {
        churn.started = None;
        return;
    }
    let churn = &mut *churn;
    let (mut total, mut in_map) = (0, 0);
    for e in &changed {
        total += 1;
        // The root of its tree, and the nearest name on the way up.
        let (mut root, mut label) = (e, names.get(e).ok());
        while let Ok(child_of) = parents.get(root) {
            root = child_of.parent();
            label = label.or_else(|| names.get(root).ok());
        }
        let label = label.map(|n| n.as_str().to_string());
        if root != e && map_parts.contains(root) {
            in_map += 1;
        }
        if perf.show >= 3 {
            let label: String = label.unwrap_or_else(|| "?".into()).chars().filter(|c| !c.is_ascii_digit()).collect();
            *churn.names.entry(label).or_default() += 1;
        }
    }
    churn.frames += 1;
    for (s, v) in churn.sums.iter_mut().zip([total, in_map, mesh_count, material_count]) {
        *s += v;
    }
    let now = Instant::now();
    let started = *churn.started.get_or_insert(now);
    if (now - started).as_secs_f32() < 1.0 {
        return;
    }
    let n = churn.frames.max(1) as f32;
    churn.shown = churn.sums.map(|s| s as f32 / n);
    if perf.show >= 3 {
        let mut names: Vec<_> = churn.names.drain().collect();
        names.sort_by(|a, b| b.1.cmp(&a.1));
        let top: Vec<String> = names.iter().take(12).map(|(k, v)| format!("{k}: {:.1}", *v as f32 / n)).collect();
        info!(
            "per frame: {:.1} transforms written ({:.1} under the map's root), {:.1} meshes and {:.1} materials modified; \
             transforms by name: {}",
            churn.shown[0],
            churn.shown[1],
            churn.shown[2],
            churn.shown[3],
            top.join(", ")
        );
    }
    churn.frames = 0;
    churn.sums = [0; 4];
    churn.started = Some(now);
}

#[allow(clippy::too_many_arguments)]
fn draw_overlay(
    churn: Res<Churn>,
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
    let [t, m, meshes_changed, materials] = churn.shown;
    lines.push(format!(
        "per frame: {t:.0} transforms written ({m:.0} under the map), {meshes_changed:.1} meshes, {materials:.1} materials modified"
    ));
    if vis.cluster.is_some() && vis.areas > 1 {
        lines.push(format!(
            "areas: in {}, reaching {}/{}, {} areaportals closed",
            vis.area, vis.visible_areas, vis.areas, vis.closed_portals
        ));
    }
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
