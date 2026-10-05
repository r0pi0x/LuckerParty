//! Counter-Strike: Source (Steam app 240).

pub mod ambient;
pub mod bsp;
pub mod decals;
pub mod lightmap;
pub mod material;
pub mod mount;
pub mod movement;
pub mod overlays;
pub mod props;
pub mod ropes;
pub mod sky;
pub mod sprites;
pub mod vpk;

pub const GAME: &str = "cs_source";

/// CS:S's server tick: 0.015 s (66.67 Hz). Measured with the movement probe
/// (tools/css_probe): a dedicated server started with -tickrate 64 still
/// accelerates and applies friction in 0.015 s steps, so CS:S fixes it.
pub const TICK_INTERVAL: f64 = 0.015;
