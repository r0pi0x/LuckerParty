//! The player use search (spec 5, "Finding the bomb"; the same search the
//! logic layer runs for doors and buttons, here over things that are
//! entities of their own: a planted bomb, hostages). Engine space,
//! meters; targets are boxes.

use bevy::prelude::*;

use super::UNIT;

/// Reach: a found thing's distance must be below this (80 units).
pub const USE_RADIUS: f32 = 80.0 * UNIT;
/// The straight trace's length (1024 units).
pub const USE_TRACE_LONG: f32 = 1024.0 * UNIT;
/// The tilted probes: a ±16 unit box swept 72 units.
pub const USE_BOX_HALF: f32 = 16.0 * UNIT;
pub const USE_TRACE_BOX: f32 = 72.0 * UNIT;
/// Things found by the sphere must be within this cosine of the view.
pub const USE_CONE_DOT: f32 = 0.8;
/// The probes' tilt from the view: toward the feet by these tangents
/// (45°, 30°, 20°, 15°, 10°), then up (10°, 15°).
pub const USE_TANGENTS: [f32; 7] = [1.0, 0.5774, 0.3640, 0.2679, 0.1763, -0.1763, -0.2679];

/// Something usable: its entity and box.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct UseTarget {
    pub entity: Entity,
    pub lo: Vec3,
    pub hi: Vec3,
}

/// Who is looking: the eye, the view's forward and up vectors, and the
/// vertical range of their own hull (feet, head).
#[derive(Clone, Copy, Debug)]
pub struct User {
    pub eye: Vec3,
    pub forward: Vec3,
    pub up: Vec3,
    pub feet: f32,
    pub head: f32,
}

impl User {
    /// Horizontal distance from the eye to `h`, combined with how far `h`
    /// lies above or below the user's hull (spec 5.2).
    pub fn distance(&self, h: Vec3) -> f32 {
        let dy = if h.y < self.feet {
            self.feet - h.y
        } else if h.y > self.head {
            h.y - self.head
        } else {
            0.0
        };
        Vec2::new(h.x - self.eye.x, h.z - self.eye.z).extend(dy).length()
    }
}

/// Where a ray from `from` along `dir` (unit) first enters a box, as a
/// distance along it (0 if it starts inside).
pub fn ray_box(from: Vec3, dir: Vec3, lo: Vec3, hi: Vec3) -> Option<f32> {
    let (mut t0, mut t1) = (0.0f32, f32::MAX);
    for a in 0..3 {
        let (o, d, l, h) = (from[a], dir[a], lo[a], hi[a]);
        if d.abs() < 1e-9 {
            if o < l || o > h {
                return None;
            }
            continue;
        }
        let (mut a0, mut a1) = ((l - o) / d, (h - o) / d);
        if a0 > a1 {
            std::mem::swap(&mut a0, &mut a1);
        }
        t0 = t0.max(a0);
        t1 = t1.min(a1);
        if t0 > t1 {
            return None;
        }
    }
    Some(t0)
}

/// The use search: the thing straight ahead within reach, else the last
/// one a tilted probe touches within reach, else (and competing with the
/// probes' find) the one nearest the view line among those within reach
/// and the use cone. `clear(a, b)`: the world doesn't block the line.
pub fn find_use(user: &User, targets: &[UseTarget], clear: &dyn Fn(Vec3, Vec3) -> bool) -> Option<Entity> {
    let f = user.forward.normalize_or_zero();
    let eye = user.eye;
    // 1. The straight trace: the first target it enters.
    let straight = targets
        .iter()
        .filter_map(|t| ray_box(eye, f, t.lo, t.hi).map(|d| (t, d)))
        .filter(|(_, d)| *d <= USE_TRACE_LONG)
        .min_by(|a, b| a.1.total_cmp(&b.1));
    if let Some((t, d)) = straight {
        let hit = eye + f * d;
        if clear(eye, hit) && user.distance(hit) < USE_RADIUS {
            return Some(t.entity);
        }
    }
    // 2. The tilted box probes: the box's centre path against each target
    // grown by the box; reach is measured to the target's own nearest point.
    let mut candidate: Option<Entity> = None;
    for k in USE_TANGENTS {
        let dir = (f - user.up * k).normalize_or_zero();
        let grow = Vec3::splat(USE_BOX_HALF);
        let hit = targets
            .iter()
            .filter_map(|t| ray_box(eye, dir, t.lo - grow, t.hi + grow).map(|d| (t, d)))
            .filter(|(_, d)| *d <= USE_TRACE_BOX)
            .min_by(|a, b| a.1.total_cmp(&b.1));
        if let Some((t, d)) = hit {
            let centre = eye + dir * d;
            let near = centre.clamp(t.lo, t.hi);
            if clear(eye, centre) && user.distance(near) < USE_RADIUS {
                candidate = Some(t.entity);
            }
        }
    }
    // 3. The sphere and cone.
    let line = |p: Vec3| (p - eye).cross(f).length();
    let mut best = candidate.and_then(|c| {
        targets
            .iter()
            .find(|t| t.entity == c)
            .map(|t| (c, line(eye.clamp(t.lo, t.hi))))
    });
    for t in targets {
        let p = eye.clamp(t.lo, t.hi);
        if !in_cone(eye, f, p) {
            continue;
        }
        let score = line(p);
        if best.is_some_and(|(_, s)| s <= score) {
            continue;
        }
        if clear(eye, eye + (p - eye) * 0.99) {
            best = Some((t.entity, score));
        }
    }
    best.map(|(e, _)| e)
}

/// The sphere test alone: `p` (a target's nearest point) within reach of
/// the eye and inside the use cone.
pub fn in_cone(eye: Vec3, forward: Vec3, p: Vec3) -> bool {
    (p - eye).length() <= USE_RADIUS && (p - eye).normalize_or_zero().dot(forward) >= USE_CONE_DOT
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Source (x forward, y left, z up) units to engine meters.
    fn src(x: f32, y: f32, z: f32) -> Vec3 {
        Vec3::new(x, z, -y) * UNIT
    }

    fn user(forward: Vec3) -> User {
        User {
            eye: src(0.0, 0.0, 64.0),
            forward,
            up: Vec3::Y,
            feet: 0.0,
            head: 72.0 * UNIT,
        }
    }

    fn bomb_at(x: f32) -> UseTarget {
        UseTarget {
            entity: Entity::from_raw_u32(7).unwrap(),
            lo: src(x, -6.0, 0.0).min(src(x + 12.0, 6.0, 8.0)),
            hi: src(x, -6.0, 0.0).max(src(x + 12.0, 6.0, 8.0)),
        }
    }

    #[test]
    fn reach_counts_horizontal_distance_within_the_hull() {
        // O29: the hit point (70, 0, 1) is 70 units away horizontally and
        // inside the hull's height: found, though the eye is 94 away.
        let u = user(Vec3::X);
        assert!((u.distance(src(70.0, 0.0, 1.0)) - 70.0 * UNIT).abs() < 1e-5);
        let down = (src(70.0, 0.0, 1.0) - u.eye).normalize();
        let found = find_use(&user(down), &[bomb_at(70.0)], &|_, _| true);
        assert!(found.is_some());
        // O30: at 90 units, too far.
        let down = (src(90.0, 0.0, 1.0) - u.eye).normalize();
        assert_eq!(find_use(&user(down), &[bomb_at(90.0)], &|_, _| true), None);
    }

    #[test]
    fn the_sphere_needs_the_cone_but_a_probe_still_finds_it() {
        // O31: nearest point (40, 0, 0), view straight ahead: 0.53 < 0.8.
        let u = user(Vec3::X);
        assert!(!in_cone(u.eye, u.forward, src(40.0, 0.0, 0.0)));
        // The 45° probe reaches it.
        assert!(find_use(&u, &[bomb_at(40.0)], &|_, _| true).is_some());
        // A wall in the way blocks everything.
        assert_eq!(find_use(&u, &[bomb_at(40.0)], &|_, _| false), None);
    }
}
