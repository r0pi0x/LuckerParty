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
pub mod breakables;
mod bridge;
pub mod classes;
pub mod hud;
pub mod movers;
pub mod props;
pub mod triggers;
pub mod value;
pub mod world;

#[cfg(test)]
mod tests;

pub use bridge::{Logic, LogicPlugin, LogicSet};
pub use hud::{HudMessage, HudMessages};
pub use value::Value;
pub use world::{BrushCollision, Collision, Delivery, Effect, EntId, LogicWorld, NoCollision, Player, Who};
