//! Load-independent performance metrics (docs/performance.md,
//! "Load-independent metrics"): per-thread CPU time and hardware counters
//! (`counters`), distributions of the last N values (`dist`), and the
//! simulation tick's own numbers (`TickMetricsPlugin`).
//!
//! Wall-clock times move with whatever else the machine runs; a thread's
//! CPU time moves less (not with waiting, but with cache and frequency
//! effects), and its user-space instruction count hardly at all: compare
//! instructions across runs, times within one.

pub mod counters;
pub mod dist;

use std::time::Instant;

use bevy::{
    app::FixedMainScheduleOrder,
    ecs::schedule::{ScheduleLabel, SingleThreadedExecutor},
    prelude::*,
};

pub use counters::{AllThreads, Counts, Hw, ThreadCounters, hw_status};
pub use dist::{Percentiles, Ring, short};

/// Values kept per series: about 17 s at 240 fps, a minute of ticks at
/// 66 Hz; enough for a p99.9 of 4 values.
pub const RING: usize = 4096;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Unit {
    Ms,
    Count,
}

/// One metric's recent values.
#[derive(Clone, Debug)]
pub struct Series {
    pub name: &'static str,
    pub unit: Unit,
    pub ring: Ring,
}

impl Series {
    pub fn new(name: &'static str, unit: Unit) -> Self {
        Self {
            name,
            unit,
            ring: Ring::new(RING),
        }
    }

    pub fn format(&self, v: f64) -> String {
        match self.unit {
            Unit::Ms => format!("{v:.2}"),
            Unit::Count => short(v),
        }
    }

    /// `name p50 .. p90 .. p99 .. p99.9 .. max ..`, or None while empty.
    pub fn line(&self) -> Option<String> {
        let p = self.ring.percentiles()?;
        Some(format!("{} {}", self.name, p.line(|v| self.format(v))))
    }
}

/// A stretch of work on one thread, repeated (a frame of a world, a
/// tick): its wall time and what the thread's counters say.
#[derive(Clone, Debug)]
pub struct WorkSeries {
    pub wall_ms: Series,
    pub cpu_ms: Series,
    pub instructions: Series,
    pub cycles: Series,
    pub cache_misses: Series,
    pub branch_misses: Series,
}

impl Default for WorkSeries {
    fn default() -> Self {
        Self {
            wall_ms: Series::new("wall ms", Unit::Ms),
            cpu_ms: Series::new("cpu ms", Unit::Ms),
            instructions: Series::new("instructions", Unit::Count),
            cycles: Series::new("cycles", Unit::Count),
            cache_misses: Series::new("cache misses", Unit::Count),
            branch_misses: Series::new("branch misses", Unit::Count),
        }
    }
}

impl WorkSeries {
    pub fn push(&mut self, wall_ms: f64, d: Counts) {
        self.wall_ms.ring.push(wall_ms);
        self.cpu_ms.ring.push(d.cpu_ns as f64 / 1e6);
        if let Some(hw) = d.hw {
            self.instructions.ring.push(hw.instructions as f64);
            self.cycles.ring.push(hw.cycles as f64);
            self.cache_misses.ring.push(hw.cache_misses as f64);
            self.branch_misses.ring.push(hw.branch_misses as f64);
        }
    }

    pub fn all(&self) -> [&Series; 6] {
        [
            &self.wall_ms,
            &self.cpu_ms,
            &self.instructions,
            &self.cycles,
            &self.cache_misses,
            &self.branch_misses,
        ]
    }

    pub fn clear(&mut self) {
        for s in [
            &mut self.wall_ms,
            &mut self.cpu_ms,
            &mut self.instructions,
            &mut self.cycles,
            &mut self.cache_misses,
            &mut self.branch_misses,
        ] {
            s.ring.clear();
        }
    }

    /// Instructions per cycle over the kept values.
    pub fn ipc(&self) -> Option<f64> {
        let i: f64 = self.instructions.ring.values().iter().sum();
        let c: f64 = self.cycles.ring.values().iter().sum();
        (c > 0.0).then(|| i / c)
    }

    /// Two short lines for a readout: CPU time, and the hardware counters
    /// (or why there are none).
    pub fn summary(&self, label: &str) -> Vec<String> {
        let mut lines = Vec::new();
        let n = self.wall_ms.ring.len();
        if let (Some(w), Some(c)) = (self.wall_ms.ring.percentiles(), self.cpu_ms.ring.percentiles()) {
            lines.push(format!(
                "{label} ({n}): wall p50 {:.2} p99 {:.2} max {:.2} ms; cpu p50 {:.2} p99 {:.2} max {:.2} ms",
                w.p50, w.p99, w.max, c.p50, c.p99, c.max
            ));
        }
        match (
            self.instructions.ring.percentiles(),
            self.cache_misses.ring.percentiles(),
        ) {
            (Some(i), Some(m)) => lines.push(format!(
                "{label} instr p50 {} p99 {} p99.9 {} max {}; IPC {:.2}; cache miss p50 {} p99 {}",
                short(i.p50),
                short(i.p99),
                short(i.p999),
                short(i.max),
                self.ipc().unwrap_or(0.0),
                short(m.p50),
                short(m.p99)
            )),
            _ if n > 0 => lines.push(format!("{label} hardware counters: {}", hw_status())),
            _ => {}
        }
        lines
    }
}

/// Each simulation tick's (`FixedMain`'s) wall time and main-thread
/// counters, measured from just before `FixedFirst` to just after
/// `FixedLast`. Work handed to other threads (a multi-threaded executor,
/// parallel queries) isn't in the counters.
#[derive(Resource, Default)]
pub struct TickMetrics {
    pub work: WorkSeries,
    /// Ticks measured since the start, and their counts summed.
    pub ticks: u64,
    pub total: Counts,
    counters: ThreadCounters,
    start: Option<(Instant, Counts)>,
    /// The latest tick's numbers (wall ms, counts).
    pub last: Option<(f64, Counts)>,
}

pub struct TickMetricsPlugin;

#[derive(ScheduleLabel, Clone, Debug, PartialEq, Eq, Hash)]
struct TickStart;

#[derive(ScheduleLabel, Clone, Debug, PartialEq, Eq, Hash)]
struct TickEnd;

impl Plugin for TickMetricsPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<TickMetrics>();
        // Exclusive systems run on the thread running the schedule (the
        // main thread), which is the thread measured.
        app.add_schedule(mark_schedule(TickStart, tick_start));
        app.add_schedule(mark_schedule(TickEnd, tick_end));
    }

    fn finish(&self, app: &mut App) {
        // After every plugin added its fixed schedules: first and last.
        let mut order = app.world_mut().resource_mut::<FixedMainScheduleOrder>();
        order.labels.insert(0, TickStart.intern());
        order.labels.push(TickEnd.intern());
    }
}

/// A schedule of one system on one thread (a measuring point).
pub fn mark_schedule<M>(
    label: impl ScheduleLabel,
    system: impl IntoScheduleConfigs<bevy::ecs::system::ScheduleSystem, M>,
) -> Schedule {
    let mut s = Schedule::new(label);
    s.set_executor(SingleThreadedExecutor::new());
    s.add_systems(system);
    s
}

fn tick_start(world: &mut World) {
    let mut m = world.resource_mut::<TickMetrics>();
    let c = m.counters.read();
    m.start = Some((Instant::now(), c));
}

fn tick_end(world: &mut World) {
    let mut m = world.resource_mut::<TickMetrics>();
    let c = m.counters.read();
    let Some((t, start)) = m.start.take() else { return };
    let wall = t.elapsed().as_secs_f64() * 1e3;
    let d = c - start;
    m.work.push(wall, d);
    m.ticks += 1;
    m.total = if m.ticks == 1 { d } else { m.total + d };
    m.last = Some((wall, d));
}
