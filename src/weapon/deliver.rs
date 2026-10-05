//! Deliveries: hitscan shots (spec 4) and melee swings (spec 6.1), and the
//! damage they deal. Damage to one target within one firing call is summed
//! and applied once (spec 4.5).

use avian3d::prelude::*;
use bevy::prelude::*;

use super::{DamageEffect, Hitscan, Swing, WeaponEvent, WeaponEventKind};
use crate::{
    core::{Damage, Health, Hitgroup, Intent},
    map::PlaySound,
};

#[derive(bevy::ecs::query::QueryData)]
pub(super) struct Target {
    entity: Entity,
    transform: &'static Transform,
    aabb: Option<&'static ColliderAabb>,
    health: Option<&'static Health>,
    intent: Option<&'static Intent>,
}

/// The world access deliveries need.
#[derive(bevy::ecs::system::SystemParam)]
pub(super) struct World<'w, 's> {
    spatial: SpatialQuery<'w, 's>,
    targets: Query<'w, 's, Target>,
    colliders: Query<'w, 's, &'static ColliderOf>,
    bodies: Query<'w, 's, (&'static RigidBody, Forces)>,
    damage: MessageWriter<'w, Damage>,
    pub events: MessageWriter<'w, WeaponEvent>,
    pub play: MessageWriter<'w, PlaySound>,
}

/// Everything one firing call needs.
pub(super) struct Shot<'a, 'w, 's> {
    pub owner: Entity,
    pub weapon: Entity,
    pub eye: Vec3,
    pub aim: Quat,
    pub seed: u32,
    pub w: &'a mut World<'w, 's>,
}

/// One hit to apply.
struct Hit {
    entity: Entity,
    point: Vec3,
    dir: Vec3,
    distance: f32,
    normal: Vec3,
}

impl Shot<'_, '_, '_> {
    fn filter(&self) -> SpatialQueryFilter {
        SpatialQueryFilter::from_excluded_entities([self.owner])
    }

    /// The body a collider belongs to (props may have child colliders).
    fn body(&self, collider: Entity) -> Entity {
        self.w.colliders.get(collider).map_or(collider, |c| c.body)
    }

    fn trace(&self, dir: Dir3, range: f32) -> Option<Hit> {
        let hit = self.w.spatial.cast_ray(self.eye, dir, range, true, &self.filter())?;
        Some(Hit {
            entity: self.body(hit.entity),
            point: self.eye + *dir * hit.distance,
            dir: *dir,
            distance: hit.distance,
            normal: hit.normal,
        })
    }

    /// Fire every pellet of one shot.
    pub fn fire(&mut self, scan: &Hitscan, effect: &DamageEffect) {
        let mut total: Vec<(Entity, f32, Hitgroup, Vec3, Vec3)> = Vec::new();
        for pellet in 0..scan.pellets.max(1) {
            let mut rng = Rng::new(self.seed.wrapping_add(1 + pellet));
            let x = rng.range(-0.5, 0.5) + rng.range(-0.5, 0.5);
            let y = rng.range(-0.5, 0.5) + rng.range(-0.5, 0.5);
            let dir = spread_dir(self.aim, scan.spread, x, y);
            let hit = self.trace(dir, scan.range);
            self.w.events.write(WeaponEvent {
                owner: self.owner,
                weapon: self.weapon,
                kind: WeaponEventKind::Shot {
                    from: self.eye,
                    to: hit.as_ref().map_or(self.eye + *dir * scan.range, |h| h.point),
                    hit: hit.as_ref().map(|h| h.entity),
                    normal: hit.as_ref().map(|h| h.normal),
                },
            });
            let Some(hit) = hit else { continue };
            self.push(hit.entity, hit.point, hit.dir * effect.impulse);
            let Ok(t) = self.w.targets.get(hit.entity) else {
                continue;
            };
            if t.health.is_none() {
                continue;
            }
            let group = t.aabb.map_or(Hitgroup::Generic, |b| hitgroup_at(b, hit.point));
            let amount = effect.amount
                * falloff(effect.falloff, effect.falloff_step, hit.distance)
                * effect.hitgroups.get(group);
            match total.iter_mut().find(|t| t.0 == hit.entity) {
                Some(t) => {
                    t.1 += amount;
                    t.2 = group;
                    t.3 = hit.point;
                }
                None => total.push((hit.entity, amount, group, hit.point, hit.dir)),
            }
        }
        for (target, amount, hitgroup, point, dir) in total {
            self.apply(target, amount, hitgroup, point, dir);
        }
    }

    fn apply(&mut self, target: Entity, amount: f32, hitgroup: Hitgroup, point: Vec3, dir: Vec3) {
        self.w.damage.write(Damage {
            target,
            attacker: Some(self.owner),
            amount,
            point,
            dir,
            hitgroup,
        });
        self.w.events.write(WeaponEvent {
            owner: self.owner,
            weapon: self.weapon,
            kind: WeaponEventKind::Hit {
                target,
                amount,
                hitgroup,
            },
        });
    }

    /// Give a dynamic body an impulse at a point.
    fn push(&mut self, body: Entity, at: Vec3, impulse: Vec3) {
        if impulse == Vec3::ZERO {
            return;
        }
        if let Ok((rb, mut forces)) = self.w.bodies.get_mut(body)
            && rb.is_dynamic()
        {
            forces.apply_linear_impulse_at_point(impulse, at);
        }
    }

    /// One swing (spec 6.1): a line, then a box when the line misses.
    /// Returns whether it hit anything.
    pub fn swing(&mut self, swing: &Swing, secondary: bool) -> bool {
        let forward = self.aim * Vec3::NEG_Z;
        let dir = Dir3::new(forward).unwrap_or(Dir3::NEG_Z);
        let mut hit = self.trace(dir, swing.range);
        if hit.is_none()
            && let Some(half) = swing.hull
        {
            // Box swept to the range minus its corner distance, counting
            // only targets roughly in front.
            let reach = (swing.range - half * 3f32.sqrt()).max(0.0);
            let shape = Collider::cuboid(half * 2.0, half * 2.0, half * 2.0);
            let config = ShapeCastConfig::from_max_distance(reach);
            if let Some(h) = self
                .w
                .spatial
                .cast_shape(&shape, self.eye, Quat::IDENTITY, dir, &config, &self.filter())
            {
                let entity = self.body(h.entity);
                let origin = self.w.targets.get(entity).map_or(h.point1, |t| t.transform.translation);
                let to = (origin - self.eye).normalize_or_zero();
                if to.dot(forward) >= swing.facing_cos {
                    hit = Some(Hit {
                        entity,
                        point: h.point1,
                        dir: forward,
                        distance: h.distance,
                        normal: -h.normal1,
                    });
                }
            }
        }
        self.w.events.write(WeaponEvent {
            owner: self.owner,
            weapon: self.weapon,
            kind: WeaponEventKind::Swing {
                hit: hit.is_some(),
                secondary,
            },
        });
        let Some(hit) = hit else {
            if let Some(s) = &swing.sound_miss {
                self.w.play.write(PlaySound::at(s.clone(), self.eye));
            }
            return false;
        };
        let (alive, yaw, aabb) = match self.w.targets.get(hit.entity) {
            Ok(t) => (t.health.is_some(), t.intent.map(|i| i.yaw_rotation()), t.aabb.copied()),
            Err(_) => (false, None, None),
        };
        let mut amount = swing.damage;
        if let (Some(back), Some(yaw)) = (swing.backstab, yaw) {
            // Backstab: the target faces away from the swing.
            let facing = (yaw * Vec3::NEG_Z).with_y(0.0).normalize_or_zero();
            if facing.dot(forward.with_y(0.0).normalize_or_zero()) > BACKSTAB_COS {
                amount = back;
            }
        }
        self.push(hit.entity, hit.point, hit.dir * amount * swing.force);
        let sound = if alive {
            &swing.sound_hit
        } else {
            &swing.sound_hit_world
        };
        if let Some(s) = sound {
            self.w.play.write(PlaySound::at(s.clone(), hit.point));
        }
        if alive {
            let group = aabb.map_or(Hitgroup::Generic, |b| hitgroup_at(&b, hit.point));
            self.apply(hit.entity, amount, group, hit.point, hit.dir);
        }
        true
    }
}

/// Facing test for backstabs: the target's view within ≈45° of the
/// attacker's (unmeasured; spec Q9).
const BACKSTAB_COS: f32 = 0.7071;

/// Aim direction bent by spread factors `x`, `y` (spec 4.3, CS template):
/// normalize(forward + x·s·right + y·s·up).
pub fn spread_dir(aim: Quat, spread: f32, x: f32, y: f32) -> Dir3 {
    let f = aim * Vec3::NEG_Z;
    let r = aim * Vec3::X;
    let u = aim * Vec3::Y;
    Dir3::new(f + r * (x * spread) + u * (y * spread)).unwrap_or(Dir3::NEG_Z)
}

/// Damage scale after `distance`: base^(distance / step).
pub fn falloff(base: f32, step: f32, distance: f32) -> f32 {
    if step <= 0.0 {
        return 1.0;
    }
    base.powf(distance / step)
}

/// Hitgroup from where on a body's bounds a hit landed, until bodies carry
/// real hitboxes: height bands from CS:S's player hitboxes (head and neck
/// above ~83 % of the height, chest to 63 %, stomach to 47 %, legs below),
/// arms where the hit is far off the body's axis at chest height.
pub fn hitgroup_at(b: &ColliderAabb, point: Vec3) -> Hitgroup {
    let h = (b.max.y - b.min.y).max(1e-3);
    let frac = (point.y - b.min.y) / h;
    let centre = (b.min + b.max) / 2.0;
    let half_w = ((b.max.x - b.min.x).min(b.max.z - b.min.z) / 2.0).max(1e-3);
    let off = Vec2::new(point.x - centre.x, point.z - centre.z).length() / half_w;
    match frac {
        f if f >= 0.83 => Hitgroup::Head,
        f if f >= 0.47 && off > 0.9 => Hitgroup::LeftArm,
        f if f >= 0.63 => Hitgroup::Chest,
        f if f >= 0.47 => Hitgroup::Stomach,
        _ => Hitgroup::LeftLeg,
    }
}

/// Small deterministic generator for spread (Source's own generator is
/// not reproduced yet: spec Q1).
struct Rng(u64);

impl Rng {
    fn new(seed: u32) -> Self {
        Self(seed as u64 ^ 0x9E37_79B9_7F4A_7C15)
    }

    fn next(&mut self) -> f32 {
        // splitmix64
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^= z >> 31;
        (z >> 40) as f32 / (1u64 << 24) as f32
    }

    fn range(&mut self, lo: f32, hi: f32) -> f32 {
        lo + (hi - lo) * self.next()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn falloff_matches_spec_t12() {
        // AK-47 at 1000 units with RangeModifier 0.98 per 500 units.
        assert!((36.0 * falloff(0.98, 500.0, 1000.0) - 34.5744).abs() < 1e-3);
    }

    #[test]
    fn spread_bends_right_for_positive_x() {
        // Spec T2: x = 0.5, s = 0.01 -> 0.2865 degrees to the right.
        let d = spread_dir(Quat::IDENTITY, 0.01, 0.5, 0.0);
        let angle = d.x.atan2(-d.z).to_degrees();
        assert!((angle - 0.2865).abs() < 1e-3, "{angle}");
    }

    #[test]
    fn hitgroups_by_height() {
        let b = ColliderAabb {
            min: Vec3::new(-0.4, 0.0, -0.4),
            max: Vec3::new(0.4, 1.8, 0.4),
        };
        assert_eq!(hitgroup_at(&b, Vec3::new(0.0, 1.7, 0.0)), Hitgroup::Head);
        assert_eq!(hitgroup_at(&b, Vec3::new(0.0, 1.3, 0.1)), Hitgroup::Chest);
        assert_eq!(hitgroup_at(&b, Vec3::new(0.0, 1.0, 0.1)), Hitgroup::Stomach);
        assert_eq!(hitgroup_at(&b, Vec3::new(0.0, 0.5, 0.1)), Hitgroup::LeftLeg);
        assert_eq!(hitgroup_at(&b, Vec3::new(0.39, 1.2, 0.0)), Hitgroup::LeftArm);
    }

    #[test]
    fn rng_is_uniform_enough() {
        let mut r = Rng::new(7);
        let mean = (0..10000).map(|_| r.next()).sum::<f32>() / 10000.0;
        assert!((mean - 0.5).abs() < 0.02);
    }
}
