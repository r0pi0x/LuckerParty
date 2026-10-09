//! The integration tests, as one test binary: each file here is a module,
//! so the game links once for all of them instead of once per file (see
//! docs/performance.md, "Test cycle"). Run one file's tests with
//! `cargo test --features dev --test it weapons::` (a name filter).
//!
//! A new test file goes here (or in `heavy/`) with a `mod` line below;
//! `architecture::tests_are_one_crate` fails for a file anywhere else in
//! `tests/` or one without its `mod` line.
//!
//! `heavy` holds the slow ones (real maps from the install, long bot
//! simulations): the fast tier skips them with `-- --skip heavy::`
//! (docs/OBSERVABILITY.md, section 1).

mod animation;
mod architecture;
mod bot_grenades;
mod cs_grenades;
mod cs_guns;
mod debug_ui;
mod impact_effects;
mod interpolation;
mod logic_physics;
mod map_logic;
mod map_sound_fx;
mod metrics;
mod mount_cs_source;
mod movement;
mod nav;
mod net;
mod net_interp;
mod net_prediction;
mod net_query;
mod net_rules;
mod net_weapons;
mod objectives;
mod phy_spec;
mod prediction;
mod radio;
mod ragdoll;
mod ragdoll_spec;
mod rope_spec;
mod rounds;
mod shader_spec;
mod shadow_spec;
mod source_movement;
mod source_movement_css;
mod source_movement_ladder_water;
mod source_movement_tricks;
mod spectate;
mod teams;
mod view_model_spec;
mod view_models;
mod wav_decode;
mod weapons;

// Combat Arms: local, git-ignored files (.gitignore), present where
// src/games/combat_arms/ is.
#[cfg(feature = "combat_arms")]
mod mount_combat_arms;
#[cfg(feature = "combat_arms")]
mod rez_format;

/// Slow tests: the fast tier (`-- --skip heavy::`) leaves them out.
mod heavy {
    mod bot_nav;
    mod bot_radio;
    mod bot_rounds;
    mod hud_layout;
    mod lightmap_probe;
    mod map_ambient;
    mod map_areaportals;
    mod map_breakables;
    mod map_brush_decals;
    mod map_brush_entities;
    mod map_community;
    mod map_de_dust2;
    mod map_de_nuke;
    mod map_de_nuke_doors;
    mod map_de_port;
    mod map_net_weapons;
    mod map_fire;
    mod map_hdr;
    mod map_loose;
    mod map_objectives;
    mod map_occluders;
    mod map_physics_shadow;
    mod map_prop_damage;
    mod map_props;
    mod map_sound_stock;
    mod map_stock;
    mod map_vis;
    mod map_visuals;
}
