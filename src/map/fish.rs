//! func_fish_pool's fish (no spec; public entity docs: a pool of fish
//! models that swim around within "max_range" of its origin), for any
//! game: each fish swims toward a point picked at random within the range
//! and inside the map's water, turning toward where it goes, and picks
//! another when it gets there. Ours: no flocking and no fleeing from
//! players (tech-debt).

use bevy::prelude::*;

/// A fish's speed, m/s (ours: about 30 units/s).
pub const FISH_SPEED: f32 = 30.0 * 0.0254;
/// How fast a fish turns toward its heading, radians per second.
const TURN_RATE: f32 = 2.0;

/// One fish of a pool.
#[derive(Component, Clone, Debug)]
pub struct Fish {
    /// The pool's origin and range (engine space, meters).
    pub home: Vec3,
    pub range: f32,
    pub target: Option<Vec3>,
    pub seed: u64,
}

impl Fish {
    pub fn new(home: Vec3, range: f32, seed: u64) -> Self {
        Self {
            home,
            range,
            target: None,
            seed: seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1,
        }
    }

    fn random(&mut self) -> f32 {
        self.seed ^= self.seed << 13;
        self.seed ^= self.seed >> 7;
        self.seed ^= self.seed << 17;
        (self.seed >> 40) as f32 / (1u64 << 24) as f32
    }

    /// A new place to swim to: within the range (flatter than tall), in
    /// water when `wet` says so (a few tries, else the pool's origin).
    pub fn pick(&mut self, wet: &dyn Fn(Vec3) -> bool) -> Vec3 {
        for _ in 0..8 {
            let a = self.random() * std::f32::consts::TAU;
            let r = self.range * self.random().sqrt();
            let dy = (self.random() - 0.5) * self.range * 0.3;
            let p = self.home + Vec3::new(r * a.cos(), dy, r * a.sin());
            if wet(p) {
                return p;
            }
        }
        self.home
    }
}

/// The yaw (about engine +Y) that turns a model facing +X toward `dir`.
pub fn heading(dir: Vec3) -> Quat {
    Quat::from_rotation_y((-dir.z).atan2(dir.x))
}

/// Fish swim to their targets and pick new ones.
pub(super) fn swim(
    time: Res<Time>,
    water: Option<Res<crate::core::MapWater>>,
    mut fish: Query<(&mut Fish, &mut Transform)>,
) {
    let dt = time.delta_secs().min(0.1);
    let wet = |p: Vec3| {
        water
            .as_ref()
            .is_none_or(|w| w.0.iter().any(|v| v.brush.planes.iter().all(|(n, d)| n.dot(p) <= *d)))
    };
    for (mut f, mut t) in &mut fish {
        let target = match f.target {
            Some(target) if target.distance(t.translation) > FISH_SPEED * 0.5 => target,
            _ => {
                let p = f.pick(&wet);
                f.target = Some(p);
                p
            }
        };
        let dir = (target - t.translation).normalize_or_zero();
        if dir == Vec3::ZERO {
            continue;
        }
        let want = heading(dir);
        t.rotation = t.rotation.slerp(want, (TURN_RATE * dt).min(1.0));
        let ahead = t.rotation * Vec3::X;
        let next = t.translation + ahead * FISH_SPEED * dt;
        // Never out of the water.
        if wet(next) {
            t.translation = next;
        } else {
            f.target = None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fish_pick_wet_places_within_range_and_face_them() {
        let mut f = Fish::new(Vec3::ZERO, 2.0, 7);
        for _ in 0..50 {
            let p = f.pick(&|p: Vec3| p.x > 0.0);
            let flat = Vec2::new(p.x, p.z).length();
            assert!(flat <= 2.0 + 1e-4 && p.y.abs() <= 0.3 + 1e-4 && (p.x > 0.0 || p == Vec3::ZERO), "{p}");
        }
        // Facing +Z (Source -Y) turns a +X model by -90 degrees about Y.
        let q = heading(Vec3::Z);
        assert!((q * Vec3::X - Vec3::Z).length() < 1e-5);
    }
}
