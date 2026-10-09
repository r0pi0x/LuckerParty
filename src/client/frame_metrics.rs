//! Per-frame metrics (docs/performance.md, "Load-independent metrics"):
//! the main world's and the render world's wall time, thread CPU time and
//! hardware counters (`crate::metrics`), GPU time, each schedule's share,
//! distributions over the last frames, and the hitch log
//! (`mashup_hitch_ratio`).
//!
//! The main world is measured between marks run between its schedules
//! (`MainScheduleOrder`), the render world between its `Render` sets, each
//! on its own thread.

use std::{
    sync::{Arc, Mutex},
    time::Instant,
};

use bevy::{
    app::MainScheduleOrder,
    diagnostic::DiagnosticsStore,
    ecs::{intern::Interned, schedule::ScheduleLabel},
    prelude::*,
    render::{Render, RenderApp, RenderSystems},
};

use crate::{
    console::resource_cvar,
    metrics::{Counts, Percentiles, Series, ThreadCounters, TickMetrics, Unit, WorkSeries, mark_schedule, short},
};

pub struct FrameMetricsPlugin;

/// A point between two main-world schedules: before the first (0), after
/// schedule i - 1 (i).
#[derive(ScheduleLabel, Clone, Debug, PartialEq, Eq, Hash)]
struct FrameMark(usize);

impl Plugin for FrameMetricsPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<FrameMetrics>().insert_resource(HitchRatio(2.0));
        resource_cvar::<HitchRatio, f32>(
            app,
            "mashup_hitch_ratio",
            "Log a frame taking this many times the median frame time, with its slowest schedules \
             (and spans, in a --features profile build); 0: off.",
            |h| &mut h.0,
        );
        let shared = RenderShared::default();
        app.insert_resource(shared.clone());
        if let Some(render) = app.get_sub_app_mut(RenderApp) {
            render.insert_resource(shared).init_resource::<RenderMarks>();
            let sets = [
                ("extract commands", RenderSystems::ExtractCommands),
                ("prepare meshes", RenderSystems::PrepareMeshes),
                ("create views", RenderSystems::CreateViews),
                ("specialize", RenderSystems::Specialize),
                ("prepare views", RenderSystems::PrepareViews),
                ("queue", RenderSystems::Queue),
                ("phase sort", RenderSystems::PhaseSort),
                ("prepare", RenderSystems::Prepare),
                ("render", RenderSystems::Render),
                ("cleanup", RenderSystems::Cleanup),
            ];
            // Start and end inside the first and last sets, as the
            // render-world time was always taken. Marks between the sets
            // (each set's share, for the hitch log) only with
            // `MASHUP_RENDER_PHASES=1`: with them, 2 of 5 cs_office
            // benches quit on a wgpu indirect-draw validation error
            // (docs/performance.md, "Load-independent metrics"); extra
            // systems reorder the single-threaded executor's ambiguous
            // render systems.
            render.add_systems(Render, render_start.in_set(RenderSystems::ExtractCommands));
            render.add_systems(Render, render_end.in_set(RenderSystems::Cleanup));
            let phases = std::env::var("MASHUP_RENDER_PHASES").as_deref() == Ok("1");
            for (i, (name, set)) in sets.iter().enumerate().filter(|_| phases) {
                let name: &'static str = name;
                let mark = move |mut m: ResMut<RenderMarks>| m.mark(name);
                match sets.get(i + 1) {
                    Some((_, next)) => render.add_systems(Render, mark.after(set.clone()).before(next.clone())),
                    // The last set ends at `render_end`.
                    None => render,
                };
            }
        }
    }

    fn finish(&self, app: &mut App) {
        // Every plugin has added its schedules by now: a mark before the
        // first and after each.
        let labels: Vec<Interned<dyn ScheduleLabel>> =
            std::mem::take(&mut app.world_mut().resource_mut::<MainScheduleOrder>().labels);
        let mut order = Vec::with_capacity(labels.len() * 2 + 1);
        let mut names = Vec::new();
        order.push(FrameMark(0).intern());
        for (i, l) in labels.iter().enumerate() {
            order.push(*l);
            order.push(FrameMark(i + 1).intern());
            names.push(format!("{l:?}"));
        }
        let last = labels.len();
        for i in 0..=last {
            app.add_schedule(mark_schedule(FrameMark(i), move |world: &mut World| {
                main_mark(world, i, last)
            }));
        }
        app.world_mut().resource_mut::<MainScheduleOrder>().labels = order;
        app.world_mut().resource_mut::<FrameMetrics>().schedule_names = names;
    }
}

/// `mashup_hitch_ratio`.
#[derive(Resource)]
pub struct HitchRatio(pub f32);

/// One finished frame's numbers.
#[derive(Clone, Copy, Debug, Default)]
pub struct FrameSample {
    /// Wall time from this frame's start to the next's, ms.
    pub frame_ms: f64,
    /// The main world's schedules (First to Last), wall ms, and its
    /// thread's counters.
    pub main_ms: f64,
    pub main: Counts,
    /// The main world's instructions less its simulation ticks': how many
    /// ticks a frame runs follows the frame time (and so the load), the
    /// rest doesn't. None without hardware counters.
    pub main_frame_instructions: Option<u64>,
    /// GPU ms (the top-level passes' latest timestamps), when known.
    pub gpu_ms: Option<f64>,
}

/// The render world's latest frame, as measured on its thread.
#[derive(Clone, Copy, Debug, Default)]
pub struct RenderSample {
    pub ms: f64,
    pub counts: Counts,
}

/// Recent frames' distributions and the latest frame.
#[derive(Resource)]
pub struct FrameMetrics {
    /// Wall time per frame (start to start), ms.
    pub frame_ms: Series,
    pub main: WorkSeries,
    pub render: WorkSeries,
    pub gpu_ms: Series,
    /// `FrameSample::main_frame_instructions`.
    pub main_frame_instructions: Series,
    /// The latest complete frame (main world) and render-world frame.
    pub last: FrameSample,
    pub last_render: Option<RenderSample>,
    /// Frames measured.
    pub frames: u64,
    counters: ThreadCounters,
    start: Option<(Instant, Counts)>,
    /// This frame's marks so far: time and thread CPU ns.
    marks: Vec<(Instant, u64)>,
    prev_ticks: u64,
    ticks_in_frame: u64,
    /// The ticks' totals when this frame started.
    tick_start: Option<Counts>,
    schedule_names: Vec<String>,
    /// Median frame time, refreshed every 30 frames.
    median_ms: f64,
    last_hitch_log: Option<Instant>,
    hitches_unlogged: u32,
    /// The latest hitches' frame numbers (`frames` when each ended).
    pub hitch_frames: Vec<u64>,
    /// Distribution lines, refreshed at most 4 times a second.
    stats: Vec<StatRow>,
    stats_at: Option<Instant>,
    #[cfg(feature = "profile")]
    prev_spans: Vec<(String, f64, u32)>,
}

impl Default for FrameMetrics {
    fn default() -> Self {
        Self {
            frame_ms: Series::new("frame ms", Unit::Ms),
            main: WorkSeries::default(),
            render: WorkSeries::default(),
            gpu_ms: Series::new("gpu ms", Unit::Ms),
            main_frame_instructions: Series::new("instructions w/o ticks", Unit::Count),
            tick_start: None,
            last: FrameSample::default(),
            last_render: None,
            frames: 0,
            counters: ThreadCounters::current(),
            start: None,
            marks: Vec::new(),
            prev_ticks: 0,
            ticks_in_frame: 0,
            schedule_names: Vec::new(),
            median_ms: 0.0,
            last_hitch_log: None,
            hitches_unlogged: 0,
            hitch_frames: Vec::new(),
            stats: Vec::new(),
            stats_at: None,
            #[cfg(feature = "profile")]
            prev_spans: Vec::new(),
        }
    }
}

/// One row of the distribution table: which world, which metric, its
/// percentiles and how to print them.
#[derive(Clone, Debug)]
pub struct StatRow {
    pub group: &'static str,
    pub name: &'static str,
    pub unit: Unit,
    pub p: Percentiles,
}

impl StatRow {
    pub fn format(&self, v: f64) -> String {
        match self.unit {
            Unit::Ms => format!("{v:.2}"),
            Unit::Count => short(v),
        }
    }
}

impl FrameMetrics {
    /// Every series by group, for tables.
    pub fn series<'a>(&'a self, ticks: Option<&'a TickMetrics>) -> Vec<(&'static str, &'a Series)> {
        let mut out = vec![("frame", &self.frame_ms)];
        out.extend(self.main.all().map(|s| ("main", s)));
        out.push(("main", &self.main_frame_instructions));
        out.extend(self.render.all().map(|s| ("render", s)));
        out.push(("gpu", &self.gpu_ms));
        if let Some(t) = ticks {
            out.extend(t.work.all().map(|s| ("tick", s)));
        }
        out
    }

    /// The distribution table (cached: sorting every series every frame
    /// would cost more than the readout is worth).
    pub fn stats(&mut self, ticks: Option<&TickMetrics>) -> &[StatRow] {
        let now = Instant::now();
        if self.stats_at.is_none_or(|t| (now - t).as_secs_f32() >= 0.25) {
            self.stats_at = Some(now);
            self.stats = self
                .series(ticks)
                .into_iter()
                .filter_map(|(group, s)| {
                    Some(StatRow {
                        group,
                        name: s.name,
                        unit: s.unit,
                        p: s.ring.percentiles()?,
                    })
                })
                .collect();
        }
        &self.stats
    }

    /// Readout lines: frame time percentiles and 1% low, then each world's
    /// CPU time and counters, the tick's and the GPU's.
    pub fn lines(&mut self, ticks: Option<&TickMetrics>) -> Vec<String> {
        let rows = self.stats(ticks).to_vec();
        let get = |g: &str, n: &str| rows.iter().find(|r| r.group == g && r.name == n);
        let mut lines = Vec::new();
        if let Some(f) = get("frame", "frame ms") {
            lines.push(format!(
                "frame ms ({}): {}; 1% low {:.0} fps",
                f.p.n,
                f.p.line(|v| format!("{v:.2}")),
                f.p.low_1pct_fps()
            ));
        }
        for group in ["main", "render"] {
            if let (Some(w), Some(c)) = (get(group, "wall ms"), get(group, "cpu ms")) {
                lines.push(format!(
                    "{group} ms p50 {:.2} p99 {:.2} p99.9 {:.2}; cpu p50 {:.2} p99 {:.2} p99.9 {:.2}",
                    w.p.p50, w.p.p99, w.p.p999, c.p.p50, c.p.p99, c.p.p999
                ));
            }
            if let (Some(i), Some(c), Some(m)) = (
                get(group, "instructions"),
                get(group, "cycles"),
                get(group, "cache misses"),
            ) {
                lines.push(format!(
                    "{group} instr p50 {} p99 {} p99.9 {}; IPC {:.2}; cache miss p50 {} p99 {}",
                    short(i.p.p50),
                    short(i.p.p99),
                    short(i.p.p999),
                    i.p.total / c.p.total.max(1.0),
                    short(m.p.p50),
                    short(m.p.p99)
                ));
            }
        }
        if let Some(i) = get("main", "instructions w/o ticks") {
            lines.push(format!(
                "main instr w/o ticks p50 {} p99 {} p99.9 {}",
                short(i.p.p50),
                short(i.p.p99),
                short(i.p.p999)
            ));
        }
        if let Some(g) = get("gpu", "gpu ms") {
            lines.push(format!(
                "gpu ms p50 {:.2} p99 {:.2} max {:.2}",
                g.p.p50, g.p.p99, g.p.max
            ));
        }
        if let (Some(c), i) = (get("tick", "cpu ms"), get("tick", "instructions")) {
            let mut l = format!("tick ({}) cpu ms p50 {:.2} p99 {:.2}", c.p.n, c.p.p50, c.p.p99);
            if let Some(i) = i {
                l += &format!("; instr p50 {} p99 {}", short(i.p.p50), short(i.p.p99));
            }
            lines.push(l);
        }
        if get("main", "instructions").is_none() && !rows.is_empty() {
            lines.push(format!("hardware counters: {}", crate::metrics::hw_status()));
        }
        lines
    }

    /// The main world's schedules, in order, as measured.
    pub fn schedule_names(&self) -> &[String] {
        &self.schedule_names
    }

    /// Forget every distribution (e.g. between benchmark views).
    pub fn clear(&mut self) {
        self.frame_ms.ring.clear();
        self.main.clear();
        self.render.clear();
        self.gpu_ms.ring.clear();
        self.main_frame_instructions.ring.clear();
        self.stats_at = None;
    }
}

/// The render world's measurements, handed to the main world.
#[derive(Resource, Clone, Default)]
pub struct RenderShared(Arc<Mutex<RenderFrames>>);

#[derive(Default)]
struct RenderFrames {
    samples: Vec<RenderSample>,
    /// The latest frame's sets: name, wall ms, CPU ms.
    phases: Vec<(&'static str, f64, f64)>,
}

#[derive(Resource, Default)]
struct RenderMarks {
    counters: ThreadCounters,
    start: Option<(Instant, Counts)>,
    marks: Vec<(&'static str, Instant, u64)>,
}

impl RenderMarks {
    fn mark(&mut self, name: &'static str) {
        let cpu = crate::metrics::counters::thread_cpu_ns().unwrap_or(0);
        self.marks.push((name, Instant::now(), cpu));
    }
}

fn render_start(mut m: ResMut<RenderMarks>) {
    let c = m.counters.read();
    m.start = Some((Instant::now(), c));
    m.marks.clear();
}

fn render_end(mut m: ResMut<RenderMarks>, shared: Res<RenderShared>, time: Option<Res<super::perf::RenderTime>>) {
    let c = m.counters.read();
    let Some((t, start)) = m.start.take() else { return };
    let ms = t.elapsed().as_secs_f64() * 1e3;
    if let Some(time) = time {
        time.set_ms(ms);
    }
    let mut phases = Vec::with_capacity(m.marks.len());
    let (mut at, mut cpu) = (t, start.cpu_ns);
    for &(name, i, c) in &m.marks {
        phases.push((name, (i - at).as_secs_f64() * 1e3, c.saturating_sub(cpu) as f64 / 1e6));
        (at, cpu) = (i, c);
    }
    if !m.marks.is_empty() {
        let end = Instant::now();
        let c_end = crate::metrics::counters::thread_cpu_ns().unwrap_or(cpu);
        phases.push((
            "cleanup",
            (end - at).as_secs_f64() * 1e3,
            c_end.saturating_sub(cpu) as f64 / 1e6,
        ));
    }
    let Ok(mut s) = shared.0.lock() else { return };
    s.samples.push(RenderSample { ms, counts: c - start });
    if s.samples.len() > 64 {
        s.samples.remove(0);
    }
    s.phases = phases;
}

fn main_mark(world: &mut World, i: usize, last: usize) {
    if i == 0 {
        frame_start(world);
        return;
    }
    let now = Instant::now();
    let mut m = world.resource_mut::<FrameMetrics>();
    let cpu = crate::metrics::counters::thread_cpu_ns().unwrap_or(0);
    m.marks.push((now, cpu));
    if i == last {
        frame_end(world, now);
    }
}

fn frame_start(world: &mut World) {
    let now = Instant::now();
    let ticks = world.get_resource::<TickMetrics>().map_or(0, |t| t.ticks);
    let tick_total = world.get_resource::<TickMetrics>().map(|t| t.total);
    let ratio = world.resource::<HitchRatio>().0;
    let shared = world.resource::<RenderShared>().clone();
    let mut m = world.resource_mut::<FrameMetrics>();
    // The render world's frames since the last look.
    if let Ok(mut s) = shared.0.lock() {
        for r in s.samples.drain(..) {
            m.render.push(r.ms, r.counts);
            m.last_render = Some(r);
        }
    }
    let c = m.counters.read();
    if let Some((t, _)) = m.start {
        let frame = (now - t).as_secs_f64() * 1e3;
        m.frame_ms.ring.push(frame);
        m.last.frame_ms = frame;
        m.frames += 1;
        m.ticks_in_frame = ticks - m.prev_ticks;
        if m.frames % 30 == 1 {
            m.median_ms = m.frame_ms.ring.percentiles().map_or(0.0, |p| p.p50);
        }
        #[cfg(feature = "profile")]
        {
            m.prev_spans = span_times::take();
        }
        if ratio > 0.0 && m.frames > 60 && m.median_ms > 0.0 && frame > ratio as f64 * m.median_ms {
            let phases = shared.0.lock().map(|s| s.phases.clone()).unwrap_or_default();
            hitch(&mut m, frame, now, &phases);
        }
    }
    m.prev_ticks = ticks;
    m.tick_start = tick_total;
    m.marks.clear();
    m.start = Some((now, c));
}

fn frame_end(world: &mut World, now: Instant) {
    let gpu = world.get_resource::<DiagnosticsStore>().and_then(gpu_latest_ms);
    let tick_total = world.get_resource::<TickMetrics>().map(|t| t.total);
    let mut m = world.resource_mut::<FrameMetrics>();
    let c = m.counters.read();
    let Some((t, start)) = m.start else { return };
    let main_ms = (now - t).as_secs_f64() * 1e3;
    let d = c - start;
    m.main.push(main_ms, d);
    m.last.main_ms = main_ms;
    m.last.main = d;
    m.last.gpu_ms = gpu;
    // Ticks before the first have no counts yet (`TickMetrics::total`).
    let ins = |c: Option<Counts>| c.and_then(|c| c.hw).map_or(0, |h| h.instructions);
    let ticks_ins = ins(tick_total).saturating_sub(ins(m.tick_start));
    m.last.main_frame_instructions = d.hw.map(|h| h.instructions.saturating_sub(ticks_ins));
    if let Some(i) = m.last.main_frame_instructions {
        m.main_frame_instructions.ring.push(i as f64);
    }
    if let Some(g) = gpu {
        m.gpu_ms.ring.push(g);
    }
}

/// GPU time of the latest measured frame: the top-level passes' latest
/// timestamps (not smoothed). None without GPU timestamps.
pub fn gpu_latest_ms(diagnostics: &DiagnosticsStore) -> Option<f64> {
    let mut any = false;
    let mut sum = 0.0;
    for d in diagnostics.iter() {
        if let Some(pass) = d
            .path()
            .as_str()
            .strip_prefix("render/")
            .and_then(|p| p.strip_suffix("/elapsed_gpu"))
            && !pass.contains('/')
        {
            any = true;
            sum += d.value().unwrap_or(0.0);
        }
    }
    any.then_some(sum)
}

/// Log a slow frame: how much slower than the median, the main world's
/// slowest schedules (wall and CPU: CPU far below wall means the thread
/// waited or was descheduled), the render world's slowest sets, and with
/// the profile feature the spans with the most self time.
fn hitch(m: &mut FrameMetrics, frame: f64, now: Instant, phases: &[(&'static str, f64, f64)]) {
    let n = m.frames;
    m.hitch_frames.push(n);
    if m.hitch_frames.len() > 16 {
        m.hitch_frames.remove(0);
    }
    if m.last_hitch_log.is_some_and(|t| (now - t).as_secs_f32() < 0.5) {
        m.hitches_unlogged += 1;
        return;
    }
    m.last_hitch_log = Some(now);
    let Some((start, start_counts)) = m.start else { return };
    let mut scheds: Vec<(String, f64, f64)> = Vec::new();
    let (mut at, mut cpu) = (start, start_counts.cpu_ns);
    for (i, &(t, c)) in m.marks.iter().enumerate() {
        let name = m
            .schedule_names
            .get(i)
            .cloned()
            .unwrap_or_else(|| format!("schedule {i}"));
        scheds.push((name, (t - at).as_secs_f64() * 1e3, c.saturating_sub(cpu) as f64 / 1e6));
        (at, cpu) = (t, c);
    }
    let main_end = m.marks.last().map_or(start, |l| l.0);
    let outside = (now - main_end).as_secs_f64() * 1e3;
    scheds.sort_by(|a, b| b.1.total_cmp(&a.1));
    let top: Vec<String> = scheds
        .iter()
        .take(4)
        .map(|(n, w, c)| format!("{n} {w:.2} (cpu {c:.2})"))
        .collect();
    let hw = m
        .last
        .main
        .hw
        .map(|h| {
            format!(
                ", {} instr, {} cache misses",
                short(h.instructions as f64),
                short(h.cache_misses as f64)
            )
        })
        .unwrap_or_default();
    let mut line = format!(
        "hitch: frame {frame:.2} ms = {:.1}x median {:.2}; main world {:.2} ms (cpu {:.2}{hw}, {} ticks): {}; \
         between frames {outside:.2} ms",
        frame / m.median_ms,
        m.median_ms,
        m.last.main_ms,
        m.last.main.cpu_ns as f64 / 1e6,
        m.ticks_in_frame,
        top.join(", ")
    );
    if let Some(r) = m.last_render {
        let mut p = phases.to_vec();
        p.sort_by(|a, b| b.1.total_cmp(&a.1));
        let top: Vec<String> = p
            .iter()
            .take(3)
            .map(|(n, w, c)| format!("{n} {w:.2} (cpu {c:.2})"))
            .collect();
        line += &format!(
            "; render world {:.2} ms (cpu {:.2}){}",
            r.ms,
            r.counts.cpu_ns as f64 / 1e6,
            if top.is_empty() {
                String::new()
            } else {
                format!(": {}", top.join(", "))
            }
        );
    }
    #[cfg(feature = "profile")]
    {
        let top: Vec<String> = m
            .prev_spans
            .iter()
            .take(8)
            .map(|(n, ms, k)| format!("{n} {ms:.2}{}", if *k > 1 { format!(" x{k}") } else { String::new() }))
            .collect();
        line += &format!("; spans by self ms: {}", top.join(", "));
    }
    if m.hitches_unlogged > 0 {
        line += &format!(" ({} more since the last)", m.hitches_unlogged);
        m.hitches_unlogged = 0;
    }
    info!("{line}");
}

/// `--features profile`: every span's self time per frame, by name (a
/// system's own name for Bevy's `system` spans), for the hitch log.
#[cfg(feature = "profile")]
pub mod span_times {
    use std::{
        cell::RefCell,
        collections::HashMap,
        sync::{Mutex, OnceLock},
        time::Instant,
    };

    use bevy::log::{
        tracing::{Subscriber, field, span},
        tracing_subscriber::{Layer, layer::Context, registry::LookupSpan},
    };

    static TIMES: OnceLock<Mutex<HashMap<String, (f64, u32)>>> = OnceLock::new();

    thread_local! {
        static STACK: RefCell<Vec<(Instant, f64)>> = const { RefCell::new(Vec::new()) };
    }

    struct Label(String);

    pub struct SpanTimes;

    impl<S: Subscriber + for<'a> LookupSpan<'a>> Layer<S> for SpanTimes {
        fn on_new_span(&self, attrs: &span::Attributes<'_>, id: &span::Id, ctx: Context<'_, S>) {
            struct Name(Option<String>);
            impl field::Visit for Name {
                fn record_str(&mut self, f: &field::Field, v: &str) {
                    if f.name() == "name" {
                        self.0 = Some(v.to_string());
                    }
                }
                fn record_debug(&mut self, f: &field::Field, v: &dyn std::fmt::Debug) {
                    if f.name() == "name" && self.0.is_none() {
                        self.0 = Some(format!("{v:?}"));
                    }
                }
            }
            let mut n = Name(None);
            attrs.record(&mut n);
            let label = n.0.unwrap_or_else(|| attrs.metadata().name().to_string());
            if let Some(span) = ctx.span(id) {
                span.extensions_mut().insert(Label(label));
            }
        }

        fn on_enter(&self, _: &span::Id, _: Context<'_, S>) {
            STACK.with(|s| s.borrow_mut().push((Instant::now(), 0.0)));
        }

        fn on_exit(&self, id: &span::Id, ctx: Context<'_, S>) {
            let Some((start, children)) = STACK.with(|s| s.borrow_mut().pop()) else {
                return;
            };
            let total = start.elapsed().as_secs_f64() * 1e3;
            STACK.with(|s| {
                if let Some(parent) = s.borrow_mut().last_mut() {
                    parent.1 += total;
                }
            });
            let Some(span) = ctx.span(id) else { return };
            let ext = span.extensions();
            let Some(label) = ext.get::<Label>() else { return };
            if let Ok(mut t) = TIMES.get_or_init(Default::default).lock() {
                let e = t.entry(label.0.clone()).or_default();
                e.0 += total - children;
                e.1 += 1;
            }
        }
    }

    /// The spans' self times since the last call, largest first.
    pub fn take() -> Vec<(String, f64, u32)> {
        let Ok(mut t) = TIMES.get_or_init(Default::default).lock() else {
            return Vec::new();
        };
        let mut v: Vec<(String, f64, u32)> = t.drain().map(|(k, (ms, n))| (k, ms, n)).collect();
        v.sort_by(|a, b| b.1.total_cmp(&a.1));
        v.truncate(16);
        v
    }
}
