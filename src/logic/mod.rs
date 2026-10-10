//! Map logic for any game whose maps carry Source-style entities
//! (specs/source/entity_io.md, triggers.md, doors_buttons.md): named
//! entities firing outputs into inputs through a time-ordered event queue,
//! logic entities, triggers, moving brushes and +use.
//!
//! `world` is plain data driven tick by tick (tests use it directly);
//! `bridge` connects it to the ECS: it reads the map's entities
//! (`map::MapEntities`), hands it the characters each tick, moves the
//! mover nodes (`map::MapBrushEntity`, with `core::MovingSolid` for
//! movement) and applies its effects (damage, HUD text, sounds, allowed
//! commands). Games feed it through `MapData::entities`.

pub mod ambient;
pub mod audit;
pub mod anchors;
pub mod beams;
pub mod breakables;
pub mod camera;
mod bridge;
pub mod classes;
pub mod community;
pub mod fire;
pub mod game;
pub mod hud;
pub mod movers;
pub mod physics;
pub mod prop_damage;
pub mod props;
pub mod templates;
pub mod triggers;
pub mod value;
pub mod visuals;
pub mod world;

#[cfg(test)]
mod tests;
#[cfg(test)]
mod community_tests;

pub(crate) use bridge::set_node_shown;
pub use bridge::{CarriedMover, Logic, LogicPlugin, LogicSet, brush_to_engine, carry_player, mover_brushes_at};
pub use hud::{HudEvent, HudMessage, HudMessages, HudShow, MapShake, ScreenFade, ScreenFades};
pub use value::Value;
pub use world::{BrushCollision, Collision, Delivery, Effect, EntId, LogicWorld, NoCollision, Player, Who};
