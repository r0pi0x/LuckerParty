//! A map's entity list as neutral data, for the logic layer: keyvalues as
//! the game stores them, plus the volumes of brush entities. Brush entities
//! that can move or toggle (`MapEntity::mover`) get their own node
//! (`MapBrushEntity`) with their meshes and a collider, which the logic
//! layer moves.
//!
//! Entity space: the game's own units, Z up, right-handed (Hammer/Source):
//! (x, y, z) is engine (x, z, -y) times `MapData::entity_scale` meters.

use std::sync::Arc;

use bevy::prelude::*;

/// One convex volume of a brush entity, relative to the entity's origin
/// and unrotated, in entity space: planes (outward normal, distance) and
/// corner points.
#[derive(Clone, Debug, Default)]
pub struct MapHull {
    pub planes: Vec<(Vec3, f32)>,
    pub points: Vec<Vec3>,
}

/// A map entity: its keyvalues in the order the map lists them (repeated
/// keys kept: outputs are repeated keys) and, for brush entities, its
/// volumes.
#[derive(Clone, Debug, Default)]
pub struct MapEntity {
    pub keyvalues: Vec<(String, String)>,
    /// Player-solid volumes (doors, buttons) or, for triggers, the trigger
    /// volume.
    pub hulls: Vec<MapHull>,
    /// Drawn and collided with through its own node (`MapBrushEntity`),
    /// not baked into the world, so it can move or toggle.
    pub mover: bool,
    /// A rigid body (func_physbox, specs/source/physics_brushes.md 2):
    /// its node is simulated by the physics instead of moved by the logic.
    pub physics: Option<super::MapPhysics>,
}

impl MapEntity {
    /// The first value of a key (case-insensitive).
    pub fn get(&self, key: &str) -> Option<&str> {
        self.keyvalues
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(key))
            .map(|(_, v)| v.as_str())
    }

    pub fn classname(&self) -> &str {
        self.get("classname").unwrap_or("")
    }

    /// "x y z" keyvalue as a vector (missing parts 0).
    pub fn vector(&self, key: &str) -> Vec3 {
        self.get(key).map_or(Vec3::ZERO, parse_vector)
    }

    /// Origin (entity space).
    pub fn origin(&self) -> Vec3 {
        self.vector("origin")
    }

    /// Angles (pitch, yaw, roll), degrees.
    pub fn angles(&self) -> Vec3 {
        self.vector("angles")
    }
}

/// Keyvalues a game's loader adds to a prop entity from its model (they
/// are not in the map): the model's health (Source `prop_data`, or the
/// template it names), and a model door's sound entries (Source
/// `door_options` for its skin: starting to move, fully open, fully
/// closed).
pub const PROP_HEALTH_KEY: &str = "mashup_prop_health";
/// The rest of a prop's damage rules from its model (Source prop data,
/// specs/source/prop_damage.md 2): damage multipliers "bullets club
/// blast"; how many pieces it breaks into; interactions (space separated:
/// flammable, explosive_resist, ignite_halfhealth, explode_fire,
/// firstimpact_break); explosion "damage radius" and its sound; impact
/// damage table name; "1" for a client-only prop (multiplayer physics
/// mode 3); the break sound entry; its mass (kg).
pub const PROP_DAMAGE_KEY: &str = "mashup_prop_damage";
pub const PROP_PIECES_KEY: &str = "mashup_prop_pieces";
pub const PROP_INTERACTIONS_KEY: &str = "mashup_prop_interactions";
pub const PROP_EXPLODE_KEY: &str = "mashup_prop_explode";
pub const PROP_EXPLODE_SOUND_KEY: &str = "mashup_prop_explode_sound";
pub const PROP_TABLE_KEY: &str = "mashup_prop_damage_table";
pub const PROP_CLIENT_KEY: &str = "mashup_prop_client";
pub const PROP_BREAK_SOUND_KEY: &str = "mashup_prop_break_sound";
pub const PROP_MASS_KEY: &str = "mashup_prop_mass";
pub const DOOR_MOVE_KEY: &str = "mashup_door_move";
pub const DOOR_OPEN_KEY: &str = "mashup_door_open";
pub const DOOR_CLOSE_KEY: &str = "mashup_door_close";
/// A model door's handle sounds for its `hardware` type (Source
/// `door_options` "hardwareN"): used while locked, and unlocked.
pub const DOOR_LOCKED_KEY: &str = "mashup_door_locked";
pub const DOOR_UNLOCKED_KEY: &str = "mashup_door_unlocked";

/// "x y z" (or "[x y z]") as a vector; missing or bad parts are 0.
pub fn parse_vector(s: &str) -> Vec3 {
    let mut v = [0.0f32; 3];
    for (i, p) in s.trim_matches(|c| c == '[' || c == ']').split_whitespace().take(3).enumerate() {
        v[i] = p.parse().unwrap_or(0.0);
    }
    Vec3::from(v)
}

/// Entity-space position to engine space.
pub fn entity_to_engine(v: Vec3, scale: f32) -> Vec3 {
    Vec3::new(v.x, v.z, -v.y) * scale
}

/// Engine-space position to entity space.
pub fn engine_to_entity(v: Vec3, scale: f32) -> Vec3 {
    Vec3::new(v.x, -v.z, v.y) / scale
}

/// Rotation of Source-style angles (pitch about Y, yaw about Z, roll about
/// X), in entity space.
pub fn entity_rotation(angles: Vec3) -> Quat {
    Quat::from_rotation_z(angles.y.to_radians())
        * Quat::from_rotation_y(angles.x.to_radians())
        * Quat::from_rotation_x(angles.z.to_radians())
}

/// An entity-space rotation as the same rotation in engine space (the axis
/// change is itself a rotation, so the angle stays and the axis maps).
pub fn rotation_to_engine(q: Quat) -> Quat {
    Quat::from_xyzw(q.x, q.z, -q.y, q.w)
}

/// A brush entity's collider: its volumes as convex hulls, in the
/// node's space (engine axes, unrotated). None without volumes.
pub fn brush_collider(e: &MapEntity, scale: f32) -> Option<avian3d::prelude::Collider> {
    use avian3d::prelude::Collider;
    let hulls: Vec<_> = e
        .hulls
        .iter()
        .filter_map(|h| Collider::convex_hull(h.points.iter().map(|p| entity_to_engine(*p, scale)).collect()))
        .map(|c| (Vec3::ZERO, Quat::IDENTITY, c))
        .collect();
    (!hulls.is_empty()).then(|| Collider::compound(hulls))
}

/// The loaded map's entities (from `MapData::entities`), for the logic
/// layer; replaced when a map loads, removed when it unloads.
#[derive(Resource, Clone, Debug)]
pub struct MapEntities {
    pub entities: Arc<Vec<MapEntity>>,
    /// Meters per entity-space unit.
    pub scale: f32,
}

/// Fire a map entity's output (by index into `MapEntities`), e.g. a bomb
/// target's `BombExplode`: game rules outside the logic layer ask for it,
/// the logic layer (if loaded) fires it with `activator`.
#[derive(Message, Clone, Debug, PartialEq)]
pub struct FireEntityOutput {
    pub map_index: usize,
    pub output: String,
    pub activator: Option<Entity>,
}

/// The entity named by a `parentname` value (an attachment after a comma
/// is left out).
pub fn parent_name(value: &str) -> &str {
    value.split(',').next().unwrap_or("").trim()
}

/// Whether map entities of this class move by means outside the logic
/// layer and carry what is parented to them: placed weapons (`weapon_*`,
/// which lie loose and are picked up; the weapon layer moves them).
pub fn anchor_class(classname: &str) -> bool {
    classname.to_ascii_lowercase().starts_with("weapon_")
}

/// Which map entities are anchors (`anchor_class`, with a name some other
/// entity is parented to), by index: each gets a node (`MapAnchor`) that
/// what is parented to it rides.
pub fn anchor_entities(entities: &[MapEntity]) -> Vec<bool> {
    let parents: std::collections::HashSet<String> = entities
        .iter()
        .filter_map(|e| e.get("parentname"))
        .map(|p| parent_name(p).to_ascii_lowercase())
        .filter(|p| !p.is_empty())
        .collect();
    entities
        .iter()
        .map(|e| {
            anchor_class(e.classname())
                && e.get("targetname")
                    .is_some_and(|n| !n.trim().is_empty() && parents.contains(&n.trim().to_ascii_lowercase()))
        })
        .collect()
}

/// The node of an anchor entity (`anchor_entities`), by index into
/// `MapEntities`: props and brushes parented to the entity ride it; the
/// layer that moves the entity (the weapon layer) places it.
#[derive(Component, Clone, Copy, Debug)]
pub struct MapAnchor(pub usize);

/// Where the anchor entities are this tick (engine space), by index into
/// `MapEntities`, as whatever moves them wrote it: the logic layer moves
/// the logic entities parented to them along.
#[derive(Resource, Clone, Debug, Default, PartialEq)]
pub struct EntityAnchors(pub Vec<(usize, Transform)>);

/// Where `EntityAnchors` is written each tick (after movement, before the
/// logic's touches and queue).
#[derive(SystemSet, Debug, Clone, PartialEq, Eq, Hash)]
pub struct AnchorSet;

/// The node of a mover brush entity (`MapEntity::mover`): index into
/// `MapEntities`. Its transform places the entity (origin and angles in
/// engine space); meshes and collider are its children/itself.
#[derive(Component, Clone, Copy, Debug)]
pub struct MapBrushEntity(pub usize);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rotation_maps_between_spaces() {
        let scale = 0.0254;
        let q = entity_rotation(Vec3::new(10.0, 35.0, -20.0));
        let v = Vec3::new(3.0, -4.0, 5.0);
        let a = entity_to_engine(q * v, scale);
        let b = rotation_to_engine(q) * entity_to_engine(v, scale);
        assert!(a.distance(b) < 1e-5, "{a} vs {b}");
        assert!(engine_to_entity(entity_to_engine(v, scale), scale).distance(v) < 1e-4);
    }
}
