//! Counter-Strike: Source (Steam app 240).

pub mod ambient;
pub mod anim;
pub mod breakables;
pub mod bsp;
pub mod decals;
pub mod dust;
pub mod fire;
pub mod grenades;
pub mod hostage_anim;
pub mod hud;
pub mod impact_effects;
pub mod impacts;
pub mod lightmap;
pub mod material;
pub mod mount;
pub mod movement;
pub mod nav;
pub mod objectives;
pub mod overlays;
pub mod phy;
pub mod player_anim;
pub mod propdata;
pub mod props;
pub mod pushaway;
pub mod radio;
pub mod ropes;
pub mod sky;
pub mod sound;
pub mod soundscape;
pub mod sprites;
pub mod surfaceprops;
pub mod view_anim;
pub mod view_motion;
pub mod vpk;
pub mod water;
pub mod wav;
pub mod weapons;

pub const GAME: &str = "cs_source";

/// CS:S's server tick: 0.015 s (66.67 Hz). Measured with the movement probe
/// (tools/css_probe): a dedicated server started with -tickrate 64 still
/// accelerates and applies friction in 0.015 s steps, so CS:S fixes it.
pub const TICK_INTERVAL: f64 = 0.015;
