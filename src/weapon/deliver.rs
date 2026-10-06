//! Deliveries: hitscan shots (spec 4) and melee swings (spec 6.1), and the
//! damage they deal. Damage to one target within one firing call is summed
//! and applied once (spec 4.5). Bullets of weapons with `Penetration` go on
//! through walls and characters (measured M13).

use avian3d::prelude::*;
use bevy::prelude::*;

use super::{
    Armor, DamageEffect, Hitscan, PassMaterials, Penetration, SpreadShape, Swing, WeaponEvent, WeaponEventKind,
};
use crate::{
    core::{Damage, DamageKind, Damageable, Health, Hitboxes, Hitgroup, Intent, SOLID_LAYERS},
    map::{
        PlaySound, PropSurface,
        sound::{SoundBank, SurfaceGrid},
    },
};

/// How far past a boundary a bullet steps before looking again, m.
const STEP: f32 = 1e-3;
/// How far from a hit point a world triangle may be to give its surface, m.
const SURFACE_REACH: f32 = 0.3;

#[derive(bevy::ecs::query::QueryData)]
pub(super) struct Target {
    entity: Entity,
    transform: &'static Transform,
    aabb: Option<&'static ColliderAabb>,
    health: Option<&'static Health>,
    damageable: Option<&'static Damageable>,
    intent: Option<&'static Intent>,
    hitboxes: Option<&'static Hitboxes>,
}

/// The world access deliveries need.
#[derive(bevy::ecs::system::SystemParam)]
pub(super) struct World<'w, 's> {
    spatial: SpatialQuery<'w, 's>,
    targets: Query<'w, 's, Target>,
    colliders: Query<'w, 's, &'static ColliderOf>,
    bodies: Query<'w, 's, (&'static RigidBody, Forces)>,
    damage: MessageWriter<'w, Damage>,
    armor: Query<'w, 's, &'static mut Armor>,
    props: Query<'w, 's, &'static PropSurface>,
    bank: Option<Res<'w, SoundBank>>,
    grid: Option<Res<'w, SurfaceGrid>>,
    materials: Res<'w, PassMaterials>,
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
    /// The body hit, and its collider (props may have child colliders).
    entity: Entity,
    collider: Entity,
    point: Vec3,
    dir: Vec3,
    /// From the eye.
    distance: f32,
    normal: Vec3,
    /// The hitbox's group, when the target has hitboxes.
    group: Option<Hitgroup>,
}

impl Shot<'_, '_, '_> {
    fn filter(&self) -> SpatialQueryFilter {
        SpatialQueryFilter::from_excluded_entities([self.owner]).with_mask(SOLID_LAYERS)
    }

    /// The body a collider belongs to (props may have child colliders).
    fn body(&self, collider: Entity) -> Entity {
        self.w.colliders.get(collider).map_or(collider, |c| c.body)
    }

    /// The first thing along the ray between `start` and `end` meters from
    /// the eye, leaving out `skip`. Characters with hitboxes are hit only
    /// where a hitbox is (shots pass through the rest of their hull, and
    /// hitboxes poking out of it still count, as in Source); the nearest
    /// box entered decides the hitgroup.
    fn trace(&self, dir: Dir3, start: f32, end: f32, skip: &[Entity]) -> Option<Hit> {
        let from = self.eye + *dir * start;
        let boxed: Vec<Entity> = self
            .w
            .targets
            .iter()
            .filter(|t| t.hitboxes.is_some() && t.entity != self.owner)
            .map(|t| t.entity)
            .collect();
        // The dead are left out entirely (their body is a ragdoll, which
        // bullets pass through, or hidden).
        let dead = self
            .w
            .targets
            .iter()
            .filter(|t| t.intent.is_some() && t.health.is_some_and(|h| h.current <= 0.0))
            .map(|t| t.entity);
        let filter = SpatialQueryFilter::from_excluded_entities(
            std::iter::once(self.owner)
                .chain(boxed.iter().copied())
                .chain(dead)
                .chain(skip.iter().copied()),
        )
        .with_mask(SOLID_LAYERS);
        let boxed: Vec<Entity> = boxed
            .into_iter()
            .filter(|e| self.w.targets.get(*e).ok().and_then(|t| t.health).is_none_or(|h| h.current > 0.0))
            .collect();
        let mut best = self
            .w
            .spatial
            .cast_ray(from, dir, end - start, true, &filter)
            .map(|hit| Hit {
                entity: self.body(hit.entity),
                collider: hit.entity,
                point: from + *dir * hit.distance,
                dir: *dir,
                distance: start + hit.distance,
                normal: hit.normal,
                group: None,
            });
        for e in boxed.into_iter().filter(|e| !skip.contains(e)) {
            let Some((o, d)) = self.local_ray(e, from, dir) else {
                continue;
            };
            let Some(boxes) = self.w.targets.get(e).ok().and_then(|t| t.hitboxes) else {
                continue;
            };
            let limit = best.as_ref().map_or(end, |b| b.distance) - start;
            let nearest = boxes
                .0
                .iter()
                .filter_map(|b| Some((b.ray_entry(o, d)?, b.group)))
                .filter(|(t, _)| *t < limit)
                .min_by(|a, b| a.0.total_cmp(&b.0));
            if let Some((distance, group)) = nearest {
                best = Some(Hit {
                    entity: e,
                    collider: e,
                    point: from + *dir * distance,
                    dir: *dir,
                    distance: start + distance,
                    normal: -*dir,
                    group: Some(group),
                });
            }
        }
        best
    }

    /// A ray in a character's hitbox frame: feet at the origin, facing -Z.
    fn local_ray(&self, character: Entity, origin: Vec3, dir: Dir3) -> Option<(Vec3, Vec3)> {
        let t = self.w.targets.get(character).ok()?;
        let feet = t
            .aabb
            .map_or(t.transform.translation, |b| t.transform.translation.with_y(b.min.y));
        let turn = t.intent.map_or(Quat::IDENTITY, |i| i.yaw_rotation()).inverse();
        Some((turn * (origin - feet), turn * *dir))
    }

    /// Whether what was hit is a character (bullets pass those by
    /// `PassMaterials::character`).
    fn is_character(&self, body: Entity) -> bool {
        self.w
            .targets
            .get(body)
            .is_ok_and(|t| t.intent.is_some() || t.hitboxes.is_some())
    }

    /// Where the bullet leaves a character it hit, as a distance from the
    /// eye: past its last hitbox along the ray, or out of its collider.
    fn character_exit(&self, hit: &Hit) -> Option<f32> {
        let dir = Dir3::new(hit.dir).ok()?;
        if let Some(boxes) = self.w.targets.get(hit.entity).ok().and_then(|t| t.hitboxes) {
            let (o, d) = self.local_ray(hit.entity, self.eye, dir)?;
            return boxes
                .0
                .iter()
                .filter_map(|b| b.ray_span(o, d))
                .map(|(_, out)| out)
                .reduce(f32::max);
        }
        let from = hit.point + *dir * STEP;
        self.w
            .spatial
            .cast_ray_predicate(from, dir, 10.0, false, &SpatialQueryFilter::default(), &|e| {
                self.body(e) == hit.entity
            })
            .map(|h| hit.distance + STEP + h.distance)
    }

    /// Where a bullet that entered something solid `entry` meters from the
    /// eye comes out of it, if that's within `limit` meters: it steps from
    /// boundary to boundary while inside any solid collider (adjacent
    /// brushes count as one wall). None also when there is nothing to pass
    /// (the point just past the entry isn't inside a solid: open surfaces
    /// such as displacements).
    fn solid_exit(&self, dir: Dir3, entry: f32, limit: f32) -> Option<f32> {
        let filter = SpatialQueryFilter::from_excluded_entities([self.owner]).with_mask(SOLID_LAYERS);
        let mut t = entry;
        for _ in 0..64 {
            let p = self.eye + *dir * (t + STEP);
            let mut inside = Vec::new();
            self.w.spatial.point_intersections_callback(p, &filter, |e| {
                if !self.is_character(self.body(e)) {
                    inside.push(e);
                }
                true
            });
            if inside.is_empty() {
                return (t > entry).then_some(t);
            }
            let left = entry + limit - t - STEP;
            if left <= 0.0 {
                return None;
            }
            let hit = self
                .w
                .spatial
                .cast_ray_predicate(p, dir, left, false, &filter, &|e| inside.contains(&e))?;
            t += STEP + hit.distance;
        }
        None
    }

    /// The material class of the surface hit (`MapSurface::game_material`):
    /// a prop's own surface property, else the nearest world triangle's.
    fn material_class(&self, hit: &Hit) -> Option<char> {
        let name = match self.w.props.get(hit.collider).or_else(|_| self.w.props.get(hit.entity)) {
            Ok(s) => s.0.clone(),
            Err(_) => self.w.grid.as_ref()?.nearest(hit.point, SURFACE_REACH)?.to_string(),
        };
        let sounds = &self.w.bank.as_ref()?.0;
        sounds
            .surface(&name)
            .or_else(|| sounds.surface("default"))
            .map(|s| s.game_material)
    }

    /// Fire every pellet of one shot.
    pub fn fire(&mut self, scan: &Hitscan, effect: &DamageEffect, pen: Option<&Penetration>) {
        let mut total: Vec<(Entity, f32, Hitgroup, Vec3, Vec3)> = Vec::new();
        for pellet in 0..scan.pellets.max(1) {
            let mut rng = Rng::new(self.seed.wrapping_add(1 + pellet));
            let dir = match scan.spread {
                SpreadShape::Template(s) => {
                    let x = rng.range(-0.5, 0.5) + rng.range(-0.5, 0.5);
                    let y = rng.range(-0.5, 0.5) + rng.range(-0.5, 0.5);
                    spread_dir(self.aim, s, x, y)
                }
                SpreadShape::Disc { inaccuracy, spread } => {
                    let mut offset = Vec2::ZERO;
                    for r in [inaccuracy, spread] {
                        let angle = rng.range(0.0, std::f32::consts::TAU);
                        offset += Vec2::from_angle(angle) * rng.range(0.0, 1.0) * r;
                    }
                    spread_dir(self.aim, 1.0, offset.x, offset.y)
                }
            };
            // The bullet's damage so far (before hitgroups), where the
            // current segment starts and how far it may go, and what it
            // passed.
            let mut carried = effect.amount;
            let (mut start, mut end) = (0.0, scan.range);
            let mut skip: Vec<Entity> = Vec::new();
            let mut passed = 0;
            let mut budget = pen.map_or(0.0, |p| p.power);
            loop {
                let hit = self.trace(dir, start, end, &skip);
                let (from, to) = (
                    self.eye + *dir * start,
                    hit.as_ref().map_or(self.eye + *dir * end, |h| h.point),
                );
                let (entity, normal) = (hit.as_ref().map(|h| h.entity), hit.as_ref().map(|h| h.normal));
                let kind = if passed == 0 {
                    WeaponEventKind::Shot {
                        from,
                        to,
                        hit: entity,
                        normal,
                    }
                } else {
                    WeaponEventKind::ShotContinued {
                        from,
                        to,
                        hit: entity,
                        normal,
                    }
                };
                self.w.events.write(WeaponEvent {
                    owner: self.owner,
                    weapon: self.weapon,
                    kind,
                });
                let Some(hit) = hit else { break };
                self.push(hit.entity, hit.point, hit.dir * effect.impulse);
                // Falloff again at every hit, by the distance from the eye.
                carried *= falloff(effect.falloff, effect.falloff_step, hit.distance);
                if let Ok(t) = self.w.targets.get(hit.entity)
                    && (t.health.is_some() || t.damageable.is_some())
                {
                    // Plain objects (breakables) have no hitgroups.
                    let group = match (hit.group, t.health) {
                        (Some(g), _) => g,
                        (None, Some(_)) => t.aabb.map_or(Hitgroup::Generic, |b| hitgroup_at(b, hit.point)),
                        (None, None) => Hitgroup::Generic,
                    };
                    let amount = carried * effect.hitgroups.get(group);
                    match total.iter_mut().find(|t| t.0 == hit.entity) {
                        Some(t) => {
                            t.1 += amount;
                            t.2 = group;
                            t.3 = hit.point;
                        }
                        None => total.push((hit.entity, amount, group, hit.point, hit.dir)),
                    }
                }
                // Through it?
                let Some(pen) = pen else { break };
                if passed >= pen.objects || hit.distance >= pen.max_distance {
                    break;
                }
                let character = self.is_character(hit.entity);
                let (exit, cost, factor) = if character {
                    let pass = self.w.materials.character;
                    let Some(exit) = self.character_exit(&hit) else { break };
                    (exit, pass.cost, pass.damage)
                } else {
                    let m = self.w.materials.get(self.material_class(&hit));
                    if m.scale <= 0.0 {
                        break;
                    }
                    let Some(exit) = self.solid_exit(dir, hit.distance, budget * m.scale) else {
                        break;
                    };
                    (exit, (exit - hit.distance) / m.scale, m.damage)
                };
                if cost > budget {
                    break;
                }
                budget -= cost;
                carried *= factor;
                passed += 1;
                if character {
                    skip.push(hit.entity);
                    end = exit + (end - exit) * self.w.materials.character.range_scale;
                }
                start = exit.max(hit.distance) + STEP;
                if start >= end {
                    break;
                }
            }
        }
        for (target, amount, hitgroup, point, dir) in total {
            self.apply(
                target,
                amount,
                effect.armor_ratio,
                effect.quantum,
                hitgroup,
                point,
                dir,
                DamageKind::Bullet,
            );
        }
    }

    /// Deal `raw` damage: armour (if the weapon's ratio applies and it
    /// covers the hitgroup) takes its share, then both are truncated.
    #[allow(clippy::too_many_arguments)]
    fn apply(
        &mut self,
        target: Entity,
        raw: f32,
        armor_ratio: Option<f32>,
        quantum: f32,
        hitgroup: Hitgroup,
        point: Vec3,
        dir: Vec3,
        kind: DamageKind,
    ) {
        let mut amount = quantize(raw, quantum);
        if let Some(ratio) = armor_ratio
            && let Ok(mut armor) = self.w.armor.get_mut(target)
            && armor.covers(hitgroup)
        {
            let to_health = raw * ratio * 0.5;
            let to_armor = quantize((raw - to_health) * 0.5, quantum);
            amount = quantize(to_health, quantum);
            // Armour running out mid-hit isn't measured: clamped.
            armor.amount = (armor.amount - to_armor).max(0.0);
        }
        self.w.damage.write(Damage {
            target,
            attacker: Some(self.owner),
            amount,
            point,
            dir,
            hitgroup,
            kind,
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
        let mut hit = self.trace(dir, 0.0, swing.range, &[]);
        if hit.is_none()
            && let Some(half) = swing.hull
        {
            // Box swept to the range minus its corner distance, counting
            // only targets roughly in front.
            let reach = swing.hull_reach;
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
                        collider: h.entity,
                        point: h.point1,
                        dir: forward,
                        distance: h.distance,
                        normal: -h.normal1,
                        group: None,
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
                at: hit.as_ref().map(|h| (h.point, h.normal, h.entity)),
            },
        });
        let Some(hit) = hit else {
            if let Some(s) = &swing.sound_miss {
                self.w.play.write(PlaySound::at(s.clone(), self.eye));
            }
            return false;
        };
        let (alive, object, yaw, aabb, origin) = match self.w.targets.get(hit.entity) {
            Ok(t) => (
                t.health.is_some(),
                t.damageable.is_some(),
                t.intent.map(|i| i.yaw_rotation()),
                t.aabb.copied(),
                Some(t.transform.translation),
            ),
            Err(_) => (false, false, None, None, None),
        };
        let mut amount = swing.damage;
        if let (Some(back), Some(yaw), Some(origin)) = (swing.backstab, yaw, origin) {
            // Backstab: the target faces along the bearing from the
            // attacker to it (not the attacker's view; measured M11).
            let facing = (yaw * Vec3::NEG_Z).with_y(0.0).normalize_or_zero();
            let bearing = (origin - self.eye).with_y(0.0).normalize_or_zero();
            if facing.dot(bearing) >= BACKSTAB_COS {
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
        if alive || object {
            let group = match aabb.filter(|_| swing.hitgroups && alive) {
                Some(b) => hitgroup_at(&b, hit.point),
                None => Hitgroup::Generic,
            };
            self.apply(
                hit.entity,
                amount,
                swing.armor_ratio,
                swing.quantum,
                group,
                hit.point,
                hit.dir,
                DamageKind::Melee,
            );
        }
        true
    }
}

/// Backstab facing test: within 36.87° (CS:S measured: 36° backstab, 37°
/// front).
const BACKSTAB_COS: f32 = 0.8;

/// `amount` truncated to whole multiples of `quantum` (0: unchanged),
/// tolerant of float error just below a whole step.
pub fn quantize(amount: f32, quantum: f32) -> f32 {
    if quantum <= 0.0 {
        return amount;
    }
    (amount / quantum + 1e-4).floor() * quantum
}

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
