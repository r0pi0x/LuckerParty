//! Entities parented to something the ECS moves (`map::entities::
//! EntityAnchors`: placed weapons, which lie loose and are picked up):
//! each keeps its placement relative to its parent from map load, as
//! Source's parenting does (the child's local transform is taken at
//! spawn), and moves with it. Brushes (`Attached`) and triggers move
//! their volumes along; other entities their origin and angles (aim
//! points, game_ui's facing check, explosions).

use bevy::prelude::*;

use super::classes::Class;
use super::world::{EntId, LogicWorld, place_hull};
use crate::map::entities::{anchor_class, entity_rotation, parent_name};

/// A child's place relative to its parent (entity space).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Follow {
    pub parent: EntId,
    pub offset: Vec3,
    pub rotation: Quat,
}

/// Source-style angles (pitch, yaw, roll; degrees) of a rotation in
/// entity space (the inverse of `entity_rotation`).
pub fn angles_of(q: Quat) -> Vec3 {
    let (yaw, pitch, roll) = q.to_euler(EulerRot::ZYX);
    Vec3::new(pitch.to_degrees(), yaw.to_degrees(), roll.to_degrees())
}

impl LogicWorld {
    /// Link every entity parented (directly, or through other children)
    /// to something that moves (an anchor entity, `map::entities::
    /// anchor_class`; a physics brush; a mover) to its parent: its place
    /// relative to the parent now. Run after activation.
    pub fn link_anchored(&mut self) {
        let ids = self.ids();
        // The first entity of each name.
        let mut by_name: std::collections::HashMap<String, EntId> = std::collections::HashMap::new();
        for &id in &ids {
            let e = self.get(id).unwrap();
            if !e.targetname.is_empty() {
                by_name.entry(e.targetname.to_ascii_lowercase()).or_insert(id);
            }
        }
        let parent_of = |w: &LogicWorld, id: EntId| -> Option<EntId> {
            let p = w.get(id)?.kv("parentname").map(parent_name)?;
            by_name.get(&p.to_ascii_lowercase()).copied().filter(|p| *p != id)
        };
        // Entities something else moves: placed weapons and physics
        // brushes (the physics), movers (their own logic).
        let moves = |w: &LogicWorld, id: EntId| {
            w.get(id).is_some_and(|e| {
                anchor_class(&e.classname)
                    || super::prop_damage::is_physbox(&e.classname)
                    || super::movers::pusher(&e.class).is_some()
            })
        };
        // How many moving ancestors each entity has (0: stays put).
        let mut depth: std::collections::HashMap<EntId, u32> = std::collections::HashMap::new();
        fn depth_of(
            w: &LogicWorld,
            id: EntId,
            parent_of: &dyn Fn(&LogicWorld, EntId) -> Option<EntId>,
            moves: &dyn Fn(&LogicWorld, EntId) -> bool,
            memo: &mut std::collections::HashMap<EntId, u32>,
            guard: u32,
        ) -> u32 {
            if let Some(d) = memo.get(&id) {
                return *d;
            }
            let d = match parent_of(w, id) {
                Some(p) if guard < 32 => {
                    let pd = depth_of(w, p, parent_of, moves, memo, guard + 1);
                    if pd > 0 || moves(w, p) { pd + 1 } else { 0 }
                }
                _ => 0,
            };
            memo.insert(id, d);
            d
        }
        let mut children: Vec<(u32, EntId, EntId)> = Vec::new();
        for &id in &ids {
            let d = depth_of(self, id, &parent_of, &moves, &mut depth, 0);
            if d == 0 {
                continue;
            }
            let parent = parent_of(self, id).unwrap();
            let e = self.get(id).unwrap();
            let parent_moves_itself = self
                .get(parent)
                .is_some_and(|p| super::movers::pusher(&p.class).is_some());
            // Parented brushes and breakables follow a mover their own
            // way (`movers::follow_parents`), and props ride the mover's
            // node (the bridge).
            let own_way = super::movers::attached(&e.class).is_some_and(|a| a.parent.is_some())
                || (parent_moves_itself && matches!(e.class, Class::Prop(_) | Class::PropDoor(_)));
            if !own_way {
                children.push((d, id, parent));
            }
        }
        // Parents before children.
        children.sort_by_key(|(d, ..)| *d);
        for (_, id, parent) in children {
            let (po, pa) = self.parent_pose(parent);
            let prot = entity_rotation(pa);
            let e = self.get(id).unwrap();
            let follow = Follow {
                parent,
                offset: prot.inverse() * (e.origin - po),
                rotation: prot.inverse() * entity_rotation(e.angles),
            };
            self.follows.push((id, follow));
        }
    }

    /// Where a parent is now: a mover's pusher pose, else its origin and
    /// angles.
    pub fn parent_pose(&self, id: EntId) -> (Vec3, Vec3) {
        match self.get(id) {
            Some(e) => match super::movers::pusher(&e.class) {
                Some(p) => (p.origin, p.angles),
                None => (e.origin, e.angles),
            },
            None => (Vec3::ZERO, Vec3::ZERO),
        }
    }

    /// An anchor entity is here now (entity space; the host's `EntityAnchors`).
    pub fn set_anchor(&mut self, id: EntId, origin: Vec3, angles: Vec3) {
        if let Some(e) = self.get_mut(id) {
            e.origin = origin;
            e.angles = angles;
        }
    }

    /// Move every linked child to its parent's pose (parents before
    /// children: `link_anchored` keeps them in that order). A mover child
    /// (a door on a lift) is carried along by the parent's move, its own
    /// motion kept: translation only (tech-debt: a turning parent doesn't
    /// turn a mover child).
    pub fn follow_anchors(&mut self) {
        let follows = self.follows.clone();
        for (id, f) in follows {
            if self.get(f.parent).is_none() {
                continue;
            }
            let (po, pa) = self.parent_pose(f.parent);
            let parent_velocity = self
                .get(f.parent)
                .and_then(|p| super::movers::pusher(&p.class))
                .map_or(Vec3::ZERO, |p| p.velocity + p.carry);
            let prot = entity_rotation(pa);
            let origin = po + prot * f.offset;
            let rotation = prot * f.rotation;
            let Some(e) = self.get(id) else { continue };
            if e.origin.distance_squared(origin) < 1e-6 && (entity_rotation(e.angles).dot(rotation)).abs() > 1.0 - 1e-7
            {
                // A still parent carries nothing.
                if let Some(e) = self.get_mut(id) {
                    super::movers::shift(&mut e.class, Vec3::ZERO, parent_velocity);
                }
                continue;
            }
            let angles = angles_of(rotation);
            let hulls = e.hulls.clone();
            let delta = origin - e.origin;
            let e = self.get_mut(id).unwrap();
            e.origin = origin;
            if super::movers::shift(&mut e.class, delta, parent_velocity) {
                self.refresh_solid(id);
                continue;
            }
            e.angles = angles;
            match &mut e.class {
                Class::Trigger(t) => {
                    t.brushes = hulls.iter().map(|h| place_hull(h, rotation, origin)).collect();
                }
                Class::Attached(a) => {
                    a.push.origin = origin;
                    a.push.angles = angles;
                }
                Class::Breakable(b) => {
                    b.attach.push.origin = origin;
                    b.attach.push.angles = angles;
                }
                _ => {}
            }
            self.refresh_solid(id);
        }
    }

    /// Whether `id` follows an anchored parent.
    pub fn follows_anchor(&self, id: EntId) -> bool {
        self.follows.iter().any(|(c, _)| *c == id)
    }

    /// Whether `id` is carried by a physics brush (through its parents):
    /// its own collider must not collide with that body.
    pub fn rides_body(&self, id: EntId) -> bool {
        let mut at = id;
        for _ in 0..16 {
            let Some((_, f)) = self.follows.iter().find(|(c, _)| *c == at) else {
                return false;
            };
            if self
                .get(f.parent)
                .is_some_and(|p| super::prop_damage::is_physbox(&p.classname))
            {
                return true;
            }
            at = f.parent;
        }
        false
    }
}
