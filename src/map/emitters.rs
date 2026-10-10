//! Map effects drawn through the particle pool (`particles`), for any
//! game: sprite trails (Source's env_spritetrail, specs/source/
//! visual_entities.md 5), smoke stacks (env_smokestack,
//! specs/source/particles_and_smoke.md 3) and tesla arcs (point_tesla,
//! public entity docs). Each is an entity of its own
//! (`TrailEmitter`, `SmokeStackEmitter`) whose transform is where its map
//! entity is (the logic moves it when the entity follows a parent:
//! `FollowsEntity`); each frame it adds what it shows as a one-frame
//! particle group, between the pool's step and draw. Engine space:
//! meters, seconds.

use std::collections::VecDeque;

use bevy::prelude::*;

use super::EntityPart;
use super::particles::{Particle, ParticleGroup, ParticleRng, Particles, Ramp, Shape};

/// Points a trail keeps (trail_max_points).
pub const TRAIL_MAX_POINTS: usize = 256;
/// A new trail point needs the entity to have moved more than this (2
/// units), meters.
pub const TRAIL_MIN_STEP: f32 = 2.0 * 0.0254;
/// Seconds a smoke stack simulates and emits unseen after it appears.
pub const SMOKESTACK_FULL_SIM: f32 = 5.0;

/// A sprite trail (env_spritetrail).
#[derive(Clone, Debug, PartialEq)]
pub struct MapTrail {
    /// The entity (index into `MapData::entities`).
    pub entity: Option<usize>,
    /// Where it starts, engine space.
    pub position: Vec3,
    /// Index into `MapData::particles::materials`.
    pub material: usize,
    /// Seconds a point lives.
    pub life: f32,
    /// Full widths at the newest and oldest end, meters; no end width:
    /// the start width all along.
    pub start_width: f32,
    pub end_width: Option<f32>,
    /// rendercolor / 255 (sRGB) and renderamt / 255.
    pub color: Vec3,
    pub alpha: f32,
}

/// A smoke stack (env_smokestack). Directions are unit vectors in engine
/// space; lengths meters; speeds m/s.
#[derive(Clone, Debug, PartialEq)]
pub struct MapSmokeStack {
    pub entity: Option<usize>,
    pub position: Vec3,
    /// The entity's angles (the column rises along their up).
    pub rotation: Quat,
    pub material: usize,
    /// The ring puffs start on, and the sideways speed range.
    pub base_spread: f32,
    pub spread_speed: f32,
    /// Rising speed (along the entity's up).
    pub speed: f32,
    /// Half-widths at birth and at the end of life.
    pub start_size: f32,
    pub end_size: f32,
    /// Puffs per second.
    pub rate: f32,
    /// How far the puffs rise: their life is length / speed.
    pub length: f32,
    /// Acceleration (wind), m/s².
    pub wind: Vec3,
    /// Twist about the vertical axis through the origin, degrees per
    /// second (positive: clockwise seen from above).
    pub twist: f32,
    /// Roll speed up to this either way, degrees per second.
    pub roll: f32,
    /// Colour (sRGB, 0-1) and opacity scale (renderamt / 255).
    pub color: Vec3,
    pub alpha: f32,
    pub start_on: bool,
}

impl MapSmokeStack {
    /// A puff's life, seconds (none: the stack makes no puffs).
    pub fn life(&self) -> Option<f32> {
        (self.speed > 0.0 && self.length > 0.0).then(|| self.length / self.speed)
    }

    /// Half-width at life fraction `t`.
    pub fn size(&self, t: f32) -> f32 {
        self.start_size + (self.end_size - self.start_size) * t
    }

    /// Opacity (0-1) at life fraction `t`: ½ − ½cos 2πt, squared past the
    /// middle, times renderamt.
    pub fn opacity(&self, t: f32) -> f32 {
        let a = 0.5 - 0.5 * (std::f32::consts::TAU * t).cos();
        self.alpha * if t <= 0.5 { a } else { a * a }
    }
}

/// A point_tesla (public entity docs): arcs from its origin to surfaces
/// within its radius, each spark a random number of them.
#[derive(Clone, Debug, PartialEq)]
pub struct MapTesla {
    pub entity: Option<usize>,
    pub position: Vec3,
    pub material: usize,
    /// sRGB 0-1.
    pub color: Vec3,
    /// Meters.
    pub radius: f32,
    /// Arc widths (meters), lives (seconds) and counts, each a range.
    pub width: (f32, f32),
    pub life: (f32, f32),
    pub beams: (u32, u32),
}

/// One spark of a point_tesla (the logic times them).
#[derive(Message, Clone, Copy, Debug, PartialEq, Eq)]
pub struct TeslaSpark {
    /// The entity (index into `MapData::entities`).
    pub entity: usize,
}

struct Arc {
    to: Vec3,
    width: f32,
    age: f32,
    life: f32,
}

/// A point_tesla's live arcs.
#[derive(Component)]
pub struct TeslaEmitter {
    pub tesla: MapTesla,
    arcs: Vec<Arc>,
    rng: ParticleRng,
}

/// Segments an arc is drawn in (jagged, redrawn every frame).
pub const TESLA_SEGMENTS: usize = 8;

impl TeslaEmitter {
    pub fn new(tesla: MapTesla, seed: u64) -> Self {
        Self {
            tesla,
            arcs: Vec::new(),
            rng: ParticleRng::new(seed),
        }
    }

    pub fn count(&self) -> usize {
        self.arcs.len()
    }

    /// A spark from `from`: its arcs to where `trace` (a fraction along a
    /// segment, if it hits) finds a surface within the radius, four
    /// directions tried per arc; none in reach: the radius into the air.
    pub fn spark(&mut self, from: Vec3, trace: impl Fn(Vec3, Vec3) -> Option<f32>) {
        let t = self.tesla.clone();
        let n = self.rng.int(t.beams.0 as i32, t.beams.1.max(t.beams.0) as i32).max(0);
        for _ in 0..n {
            // Up to four tries for a surface; none in reach: into the air.
            let mut end = from;
            for _ in 0..4 {
                let dir = loop {
                    let v = self.rng.vec3(-1.0, 1.0);
                    if v.length_squared() > 1e-4 && v.length_squared() <= 1.0 {
                        break v.normalize();
                    }
                };
                let to = from + dir * t.radius;
                end = to;
                if let Some(f) = trace(from, to) {
                    end = from.lerp(to, f);
                    break;
                }
            }
            if end.distance_squared(from) > 1e-6 {
                self.arcs.push(Arc {
                    to: end,
                    width: self.rng.float(t.width.0, t.width.1.max(t.width.0)),
                    age: 0.0,
                    life: self.rng.float(t.life.0, t.life.1.max(t.life.0)).max(0.05),
                });
            }
        }
    }

    /// Age the arcs by `dt`; the dead ones go.
    pub fn step(&mut self, dt: f32) {
        for a in &mut self.arcs {
            a.age += dt;
        }
        self.arcs.retain(|a| a.age < a.life);
    }

    /// The arcs as jagged camera-facing strips (a new zigzag each frame),
    /// fading over their life.
    pub fn quads(&mut self, from: Vec3, eye: Vec3) -> Vec<Particle> {
        let mut out = Vec::new();
        let (material, color) = (self.tesla.material, self.tesla.color);
        let arcs: Vec<(Vec3, f32, f32)> = self
            .arcs
            .iter()
            .map(|a| (a.to, a.width, 1.0 - a.age / a.life))
            .collect();
        for (to, width, left) in arcs {
            let length = from.distance(to);
            let mut points = vec![from];
            for k in 1..TESLA_SEGMENTS {
                let along = from.lerp(to, k as f32 / TESLA_SEGMENTS as f32);
                points.push(along + self.rng.vec3(-1.0, 1.0) * length * 0.08);
            }
            points.push(to);
            for pair in points.windows(2) {
                let (a, b) = (pair[0], pair[1]);
                let mid = (a + b) * 0.5;
                let side = (b - a).cross(mid - eye).normalize_or_zero() * width * 0.5;
                let mut p = Particle::new(mid, 1.0, material, 0.0);
                p.color = color;
                p.alpha = left;
                p.shape = Shape::Quad {
                    corners: [a - side, b - side, b + side, a + side],
                    rect: [0.0, 0.0, 1.0, 1.0],
                };
                out.push(p);
            }
        }
        out
    }
}

/// Marks a drawn part whose transform the logic sets to its map entity's
/// pose each tick (it follows a parent).
#[derive(Component, Clone, Copy, Debug, Default)]
pub struct FollowsEntity;

/// A trail's recorded points: where, and when each dies (seconds of the
/// trail's own clock).
#[derive(Component, Debug)]
pub struct TrailEmitter {
    pub trail: MapTrail,
    points: VecDeque<(Vec3, f32)>,
    now: f32,
    last_record: f32,
}

impl TrailEmitter {
    pub fn new(trail: MapTrail) -> Self {
        Self {
            trail,
            points: VecDeque::new(),
            now: 0.0,
            last_record: f32::NEG_INFINITY,
        }
    }

    pub fn points(&self) -> usize {
        self.points.len()
    }

    /// Advance `dt` with the entity at `at`: a point when it moved far
    /// enough (at most one per life/256 s), dead points dropped.
    pub fn step(&mut self, dt: f32, at: Vec3) {
        self.now += dt;
        let life = self.trail.life.max(1e-3);
        if self.now - self.last_record >= life / TRAIL_MAX_POINTS as f32 {
            let moved = self
                .points
                .back()
                .is_none_or(|(p, _)| p.distance_squared(at) > TRAIL_MIN_STEP * TRAIL_MIN_STEP);
            if moved {
                self.points.push_back((at, self.now + life));
                self.last_record = self.now;
                if self.points.len() > TRAIL_MAX_POINTS {
                    self.points.pop_front();
                }
            }
        }
    }

    /// Life left (0-1) of a point that dies at `death`.
    fn left(&self, death: f32) -> f32 {
        ((death - self.now) / self.trail.life.max(1e-3)).clamp(0.0, 1.0)
    }

    /// Full width at life left `l`.
    pub fn width(&self, l: f32) -> f32 {
        match self.trail.end_width {
            Some(e) => (e + l * (self.trail.start_width - e)).max(0.0),
            None => self.trail.start_width.max(0.0),
        }
    }

    /// The strip from the oldest point to `at`, facing `eye`: one quad per
    /// pair of points (texture row 0 across the width, the trail's
    /// texture resolution being 0), then the dead points go.
    pub fn quads(&mut self, at: Vec3, eye: Vec3) -> Vec<Particle> {
        let mut out = Vec::new();
        let mut pts: Vec<(Vec3, f32)> = self.points.iter().map(|(p, d)| (*p, self.left(*d))).collect();
        pts.push((at, 1.0));
        for pair in pts.windows(2) {
            let ((a, la), (b, lb)) = (pair[0], pair[1]);
            if a.distance_squared(b) < 1e-10 {
                continue;
            }
            let mid = (a + b) * 0.5;
            let side = (b - a).cross(mid - eye).normalize_or_zero();
            let (wa, wb) = (side * self.width(la) * 0.5, side * self.width(lb) * 0.5);
            let mut p = Particle::new(mid, 1.0, self.trail.material, 0.0);
            p.color = self.trail.color;
            p.alpha = self.trail.alpha * (la + lb) * 0.5;
            p.shape = Shape::Quad {
                corners: [a - wa, b - wb, b + wb, a + wa],
                rect: [0.0, 0.0, 1.0, 0.0],
            };
            out.push(p);
        }
        let now = self.now;
        self.points.retain(|(_, d)| *d > now);
        out
    }
}

struct Puff {
    position: Vec3,
    velocity: Vec3,
    age: f32,
    roll: f32,
    roll_speed: f32,
}

/// A smoke stack's puffs.
#[derive(Component)]
pub struct SmokeStackEmitter {
    pub stack: MapSmokeStack,
    puffs: Vec<Puff>,
    /// Seconds until the next puff.
    next: f32,
    /// Seconds since it appeared.
    alive: f32,
    rng: ParticleRng,
}

impl SmokeStackEmitter {
    pub fn new(stack: MapSmokeStack, seed: u64) -> Self {
        Self {
            stack,
            puffs: Vec::new(),
            next: 0.0,
            alive: 0.0,
            rng: ParticleRng::new(seed),
        }
    }

    pub fn count(&self) -> usize {
        self.puffs.len()
    }

    /// One frame (3.3, 3.4) with the stack at `origin` facing `basis`
    /// (forward, right, up): while emitting and seen (or in its first 5
    /// s), puffs are born on the ring, then everything ages, twists,
    /// moves and rolls. Emitting but unseen after 5 s, it is frozen; not
    /// emitting, it always runs (the column dies out).
    pub fn step(&mut self, dt: f32, on: bool, seen: bool, origin: Vec3, basis: [Vec3; 3]) {
        self.alive += dt;
        let Some(life) = self.stack.life() else {
            self.puffs.clear();
            return;
        };
        let active = seen || self.alive <= SMOKESTACK_FULL_SIM;
        if on && !active {
            return;
        }
        let [f, r, u] = basis;
        if on && self.stack.rate > 0.0 {
            self.next -= dt;
            while self.next <= 0.0 {
                let s = &self.stack;
                let phi = self.rng.float(0.0, std::f32::consts::TAU);
                let position = origin + (r * phi.cos() + f * phi.sin()) * s.base_spread;
                let (a, b) = (
                    self.rng.float(-s.spread_speed, s.spread_speed),
                    self.rng.float(-s.spread_speed, s.spread_speed),
                );
                let velocity = r * a + f * b + u * s.speed;
                let roll_speed = self.rng.float(-s.roll, s.roll).to_radians();
                self.puffs.push(Puff {
                    position,
                    velocity,
                    age: 0.0,
                    roll: 0.0,
                    roll_speed,
                });
                self.next += 1.0 / self.stack.rate;
            }
        }
        let wind = self.stack.wind;
        let twist = (-self.stack.twist * dt).to_radians();
        // About the vertical through the origin: Source's turn about +Z by
        // an angle is the engine's about +Y by the same angle.
        let turn = Quat::from_rotation_y(twist);
        self.puffs.retain_mut(|p| {
            p.age += dt;
            if p.age >= life {
                return false;
            }
            if twist != 0.0 {
                let mut off = p.position - origin;
                let y = off.y;
                off.y = 0.0;
                off = turn * off;
                off.y = y;
                p.position = origin + off;
            }
            p.position += p.velocity * dt + wind * (0.5 * dt * dt);
            p.velocity += wind * dt;
            p.roll += p.roll_speed * dt;
            true
        });
    }

    /// The puffs as sprites (3.5): those with an opacity below 0.5/255
    /// aren't drawn.
    pub fn sprites(&self) -> Vec<Particle> {
        let Some(life) = self.stack.life() else {
            return Vec::new();
        };
        self.puffs
            .iter()
            .filter_map(|p| {
                let t = (p.age / life).clamp(0.0, 1.0);
                let alpha = self.stack.opacity(t);
                if alpha * 255.0 < 0.5 {
                    return None;
                }
                let mut s = Particle::new(p.position, 1.0, self.stack.material, self.stack.size(t));
                s.size = Ramp::constant(self.stack.size(t));
                s.color = self.stack.color;
                s.alpha = alpha;
                s.roll = p.roll;
                Some(s)
            })
            .collect()
    }
}

type EyeQuery<'w, 's> = Query<
    'w,
    's,
    &'static GlobalTransform,
    (
        With<Camera3d>,
        Without<super::monitor::ScreenCamera>,
        Without<super::SkyboxCamera>,
        Without<super::ViewModelCamera>,
        Without<super::water::WaterReflectionCamera>,
    ),
>;

fn frame_group(particles: Vec<Particle>) -> ParticleGroup {
    let mut g = ParticleGroup::new(default());
    g.particles = particles;
    g.capped = false;
    g.skip_first = true;
    g.kill_after = Some(0.0);
    g
}

/// Trails record where their entity is and show their strip (hidden ones,
/// out of the potentially visible set, record but aren't drawn).
pub(super) fn update_trails(
    time: Res<Time>,
    eyes: EyeQuery,
    mut trails: Query<(
        &mut TrailEmitter,
        &GlobalTransform,
        Option<&EntityPart>,
        Option<&Visibility>,
    )>,
    mut pool: ResMut<Particles>,
) {
    let dt = time.delta_secs().min(0.1);
    let eye = eyes.iter().next().map(|e| e.translation());
    for (mut t, at, part, visibility) in &mut trails {
        if part.is_some_and(|p| !p.exists) {
            continue;
        }
        let at = at.translation();
        t.step(dt, at);
        let Some(eye) = eye else { continue };
        if visibility == Some(&Visibility::Hidden) {
            continue;
        }
        let quads = t.quads(at, eye);
        if !quads.is_empty() {
            pool.add(frame_group(quads));
        }
    }
}

/// Teslas spark when the logic says (`TeslaSpark`), their arcs traced
/// against the world; arcs age and crackle.
pub(super) fn update_teslas(
    time: Res<Time>,
    eyes: EyeQuery,
    mut sparks: MessageReader<TeslaSpark>,
    mut teslas: Query<(&mut TeslaEmitter, &GlobalTransform, Option<&Visibility>)>,
    world: super::particles::WorldTracer,
    mut pool: ResMut<Particles>,
) {
    use super::particles::WorldTrace;
    let dt = time.delta_secs().min(0.1);
    let fired: Vec<usize> = sparks.read().map(|s| s.entity).collect();
    let eye = eyes.iter().next().map(|e| e.translation());
    for (mut t, at, visibility) in &mut teslas {
        let from = at.translation();
        if t.tesla.entity.is_some_and(|e| fired.contains(&e)) {
            t.spark(from, |a, b| world.trace(a, b).map(|(f, _)| f));
        }
        t.step(dt);
        let Some(eye) = eye else { continue };
        if visibility == Some(&Visibility::Hidden) || t.count() == 0 {
            continue;
        }
        let quads = t.quads(from, eye);
        if !quads.is_empty() {
            pool.add(frame_group(quads));
        }
    }
}

/// Smoke stacks emit and show their puffs.
pub(super) fn update_smokestacks(
    time: Res<Time>,
    mut stacks: Query<(
        &mut SmokeStackEmitter,
        &GlobalTransform,
        Option<&EntityPart>,
        Option<&Visibility>,
    )>,
    mut pool: ResMut<Particles>,
) {
    let dt = time.delta_secs().min(0.1);
    for (mut s, at, part, visibility) in &mut stacks {
        if part.is_some_and(|p| !p.exists) {
            s.puffs.clear();
            continue;
        }
        let on = part.map_or(s.stack.start_on, |p| p.on);
        let seen = visibility != Some(&Visibility::Hidden);
        let (_, rotation, origin) = at.to_scale_rotation_translation();
        // Source's forward (+X), right (-Y) and up (+Z) are engine X, Z
        // and Y turned by the entity's rotation
        // (`entities::rotation_to_engine`).
        let basis = [rotation * Vec3::X, rotation * Vec3::Z, rotation * Vec3::Y];
        s.step(dt, on, seen, origin, basis);
        if seen {
            let sprites = s.sprites();
            if !sprites.is_empty() {
                pool.add(frame_group(sprites));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const UNIT: f32 = 0.0254;

    fn trail(end: Option<f32>) -> TrailEmitter {
        TrailEmitter::new(MapTrail {
            entity: None,
            position: Vec3::ZERO,
            material: 0,
            life: 1.0,
            start_width: 8.0 * UNIT,
            end_width: end.map(|e| e * UNIT),
            color: Vec3::ONE,
            alpha: 1.0,
        })
    }

    #[test]
    fn trail_points_every_other_frame_at_100_units_a_second() {
        // Spec test: lifetime 1, 100 u/s at 60 fps: 1.67 units a frame is
        // not > 2, so a point every 2nd frame, ≈ 30 points after a second.
        let mut t = trail(Some(0.0));
        let step = 100.0 * UNIT / 60.0;
        for i in 0..60 {
            t.step(1.0 / 60.0, Vec3::X * step * i as f32);
            let _ = t.quads(Vec3::X * step * i as f32, Vec3::new(0.0, 10.0, 0.0));
        }
        assert!((29..=31).contains(&t.points()), "{}", t.points());
    }

    #[test]
    fn trail_width_and_alpha_taper() {
        let mut t = trail(Some(0.0));
        assert!((t.width(1.0) - 8.0 * UNIT).abs() < 1e-6);
        assert!((t.width(0.5) - 4.0 * UNIT).abs() < 1e-6);
        // No end width: constant.
        assert!((trail(None).width(0.2) - 8.0 * UNIT).abs() < 1e-6);
        // Two points: the newest quad is nearly opaque, an old one fades.
        t.step(0.0, Vec3::ZERO);
        t.step(0.5, Vec3::X);
        let q = t.quads(Vec3::X * 2.0, Vec3::new(0.0, 10.0, 0.0));
        assert_eq!(q.len(), 2);
        assert!(q[0].alpha < q[1].alpha, "{} {}", q[0].alpha, q[1].alpha);
        // The older point has half its life left: (0.5 + 1) / 2.
        assert!((q[0].alpha - 0.75).abs() < 1e-3, "{}", q[0].alpha);
        // Points die after their life (one more is made where it is now).
        t.step(2.0, Vec3::X * 2.0);
        let _ = t.quads(Vec3::X * 2.0, Vec3::Y);
        assert_eq!(t.points(), 1);
    }

    fn stack() -> MapSmokeStack {
        MapSmokeStack {
            entity: None,
            position: Vec3::ZERO,
            rotation: Quat::IDENTITY,
            material: 0,
            base_spread: 20.0 * UNIT,
            spread_speed: 15.0 * UNIT,
            speed: 30.0 * UNIT,
            start_size: 10.0 * UNIT,
            end_size: 15.0 * UNIT,
            rate: 80.0,
            length: 180.0 * UNIT,
            wind: Vec3::ZERO,
            twist: 0.0,
            roll: 0.0,
            color: Vec3::ONE,
            alpha: 1.0,
            start_on: true,
        }
    }

    #[test]
    fn smokestack_opacity_and_size() {
        let s = stack();
        assert_eq!(s.opacity(0.0), 0.0);
        assert!((s.opacity(0.5) - 1.0).abs() < 1e-6);
        // Past the middle the fade is squared: t = 0.75 gives 0.25.
        assert!((s.opacity(0.75) - 0.25).abs() < 1e-6);
        assert!((s.size(0.5) / UNIT - 12.5).abs() < 1e-4);
        assert!((s.life().unwrap() - 6.0).abs() < 1e-5);
    }

    #[test]
    fn smokestack_puffs_start_on_the_ring_and_rise() {
        let mut e = SmokeStackEmitter::new(stack(), 3);
        let basis = [Vec3::NEG_Z, Vec3::X, Vec3::Y];
        e.step(1.0 / 60.0, true, true, Vec3::ZERO, basis);
        assert!(e.count() >= 1);
        for p in &e.puffs {
            // On the ring (moved one frame), rising at Speed.
            let flat = Vec2::new(p.position.x, p.position.z).length();
            assert!((flat - 20.0 * UNIT).abs() < 0.5 * UNIT, "{flat}");
            assert!((p.velocity.y - 30.0 * UNIT).abs() < 1e-6);
        }
        // About Rate a second.
        for _ in 0..59 {
            e.step(1.0 / 60.0, true, true, Vec3::ZERO, basis);
        }
        assert!((79..=81).contains(&e.count()), "{}", e.count());
        // Unseen after 5 s while emitting: frozen.
        for _ in 0..300 {
            e.step(1.0 / 60.0, true, true, Vec3::ZERO, basis);
        }
        let n = e.count();
        let y = e.puffs[0].position.y;
        e.step(1.0 / 60.0, true, false, Vec3::ZERO, basis);
        assert_eq!((e.count(), e.puffs[0].position.y), (n, y));
        // Off: the column dies out even unseen.
        for _ in 0..400 {
            e.step(1.0 / 60.0, false, false, Vec3::ZERO, basis);
        }
        assert_eq!(e.count(), 0);
    }

    #[test]
    fn smokestack_twist_turns_clockwise_from_above() {
        let mut s = stack();
        s.twist = 90.0;
        s.spread_speed = 0.0;
        s.speed = 1e-3;
        s.length = 1.0;
        let mut e = SmokeStackEmitter::new(s, 1);
        e.puffs.push(Puff {
            position: Vec3::X,
            velocity: Vec3::ZERO,
            age: 0.0,
            roll: 0.0,
            roll_speed: 0.0,
        });
        e.step(1.0, false, true, Vec3::ZERO, [Vec3::NEG_Z, Vec3::X, Vec3::Y]);
        // Source +X (east) turning clockwise from above goes to Source -Y,
        // engine +Z.
        let p = e.puffs[0].position;
        assert!((p - Vec3::Z).length() < 1e-3, "{p}");
    }
}

#[cfg(test)]
mod tesla_tests {
    use super::*;

    #[test]
    fn a_spark_arcs_to_surfaces_and_the_arcs_die() {
        let mut t = TeslaEmitter::new(
            MapTesla {
                entity: Some(0),
                position: Vec3::ZERO,
                material: 0,
                color: Vec3::ONE,
                radius: 2.0,
                width: (0.1, 0.2),
                life: (0.3, 0.3),
                beams: (6, 8),
            },
            5,
        );
        // Nothing within reach: arcs into the air, the radius long.
        t.spark(Vec3::ZERO, |_, _| None);
        assert!((6..=8).contains(&t.count()), "{}", t.count());
        assert!(t.arcs.iter().all(|a| (a.to.length() - 2.0).abs() < 1e-4));
        t.step(1.0);
        // A wall everywhere at half the radius.
        t.spark(Vec3::ZERO, |_, _| Some(0.5));
        assert!((6..=8).contains(&t.count()), "{}", t.count());
        assert!(t.arcs.iter().all(|a| (a.to.length() - 1.0).abs() < 1e-4));
        let q = t.quads(Vec3::ZERO, Vec3::new(0.0, 0.0, 5.0));
        assert_eq!(q.len(), t.count() * TESLA_SEGMENTS);
        t.step(0.31);
        assert_eq!(t.count(), 0);
    }
}
