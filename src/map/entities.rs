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
