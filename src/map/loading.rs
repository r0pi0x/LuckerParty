//! How far a map load has come, for a loading screen: loaders call
//! `report` at their stages (from the loading thread), the client reads
//! `current` while it waits. One load runs at a time in the game; tests
//! loading maps side by side only overwrite each other's (unread) value.

use std::sync::Mutex;

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
