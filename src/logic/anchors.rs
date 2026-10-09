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
    /// to an anchor entity (`map::entities::anchor_class`) to its parent:
    /// its place relative to the parent now. Run after activation.
    pub fn link_anchored(&mut self) {
        let ids = self.ids();
        // Anchored entities first, then their children's children.
        // Placed weapons, and physics brushes (the physics moves them).
        let mut linked: Vec<EntId> = ids
            .iter()
            .copied()
            .filter(|id| {
                self.get(*id)
                    .is_some_and(|e| anchor_class(&e.classname) || super::prop_damage::is_physbox(&e.classname))
            })
            .collect();
        let mut frontier = linked.clone();
        while !frontier.is_empty() {
            let mut next = Vec::new();
            for parent in frontier {
                let Some(name) = self.get(parent).map(|e| e.targetname.clone()).filter(|n| !n.is_empty()) else {
                    continue;
                };
                let (po, pa) = {
                    let p = self.get(parent).unwrap();
                    (p.origin, p.angles)
                };
                let prot = entity_rotation(pa);
                for &id in &ids {
                    if linked.contains(&id) {
                        continue;
                    }
                    let Some(e) = self.get(id) else { continue };
                    let Some(p) = e.kv("parentname").map(parent_name) else { continue };
                    if !p.eq_ignore_ascii_case(&name) {
                        continue;
                    }
                    let follow = Follow {
                        parent,
                        offset: prot.inverse() * (e.origin - po),
                        rotation: prot.inverse() * entity_rotation(e.angles),
                    };
                    self.follows.push((id, follow));
                    linked.push(id);
                    next.push(id);
                }
            }
            frontier = next;
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
    /// children: `link_anchored` keeps them in that order).
    pub fn follow_anchors(&mut self) {
        let follows = self.follows.clone();
        for (id, f) in follows {
            let Some((po, pa)) = self.get(f.parent).map(|p| (p.origin, p.angles)) else {
                continue;
            };
            let prot = entity_rotation(pa);
            let origin = po + prot * f.offset;
            let rotation = prot * f.rotation;
            let Some(e) = self.get(id) else { continue };
            if e.origin.distance_squared(origin) < 1e-6 && (entity_rotation(e.angles).dot(rotation)).abs() > 1.0 - 1e-7 {
                continue;
            }
            let angles = angles_of(rotation);
            let hulls = e.hulls.clone();
            let e = self.get_mut(id).unwrap();
            e.origin = origin;
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
