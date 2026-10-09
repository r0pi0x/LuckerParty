//! How far a map load has come, for a loading screen: loaders call
//! `report` at their stages (from the loading thread), the client reads
//! `current` while it waits. One load runs at a time in the game; tests
//! loading maps side by side only overwrite each other's (unread) value.

use std::{sync::Mutex, time::Instant};

/// A load's progress: the fraction done (0..1) and the stage, as a
/// localisation token the game words (`LoadingProgress_LoadMap`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LoadProgress {
    pub fraction: f32,
    pub stage: &'static str,
}

static PROGRESS: Mutex<Option<LoadProgress>> = Mutex::new(None);

/// A loader reached `stage` with `fraction` of the work done.
pub fn report(fraction: f32, stage: &'static str) {
    *PROGRESS.lock().unwrap_or_else(|e| e.into_inner()) = Some(LoadProgress {
        fraction: fraction.clamp(0.0, 1.0),
        stage,
    });
}

/// A new load starts (nothing done yet).
pub fn reset() {
    *PROGRESS.lock().unwrap_or_else(|e| e.into_inner()) = None;
}

/// The latest report since the last `reset`.
pub fn current() -> Option<LoadProgress> {
    *PROGRESS.lock().unwrap_or_else(|e| e.into_inner())
}

/// Where a load's time goes: a loader calls `lap` after each stage; the
/// stages land in `MapData::load_times` (and the log), for finding what
/// makes a map slow to load (docs/OBSERVABILITY.md, "Load times").
pub struct LoadTimer {
    start: Instant,
    last: Instant,
    pub stages: Vec<(&'static str, f32)>,
}

impl Default for LoadTimer {
    fn default() -> Self {
        let now = Instant::now();
        Self {
            start: now,
            last: now,
            stages: Vec::new(),
        }
    }
}

impl LoadTimer {
    /// `stage` ended now: the seconds since the last lap.
    pub fn lap(&mut self, stage: &'static str) {
        let now = Instant::now();
        self.stages.push((stage, (now - self.last).as_secs_f32()));
        self.last = now;
    }

    /// Seconds since the start.
    pub fn total(&self) -> f32 {
        self.start.elapsed().as_secs_f32()
    }

    /// One line: the total, then the stages over 50 ms, slowest first.
    pub fn summary(&self) -> String {
        let mut slow: Vec<_> = self.stages.iter().filter(|s| s.1 >= 0.05).collect();
        slow.sort_by(|a, b| b.1.total_cmp(&a.1));
        let parts: Vec<String> = slow.iter().map(|(n, t)| format!("{n} {t:.2}")).collect();
        format!("{:.2} s ({})", self.total(), parts.join(", "))
    }
}
