//! Bullet tracers for any game: a game asks for one with a `Tracer`
//! message (from the shot's start to its end, with the game's look); the
//! streak starts at the shooter's gun muzzle as last drawn (the view
//! model's for your own gun, re-projected as its muzzle flash is, else the
//! held world model's), falling back to the shot's start.

use std::collections::HashMap;

use bevy::prelude::*;

use super::particles::{Fade, Motion, Particle, ParticleGroup, ParticleMaterials, Particles, Ramp, Shape};

/// How a game's tracers look (meters, seconds).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TracerLook {
    /// Particle material name.
    pub material: &'static str,
    pub speed: f32,
    /// Streak length and core half-width ranges.
    pub length: (f32, f32),
    pub half_width: (f32, f32),
    /// Shots shorter than this draw nothing.
    pub min_distance: f32,
}

/// Draw a tracer for `owner`'s shot from `from` to `to`.
#[derive(Message, Clone, Debug)]
pub struct Tracer {
    pub owner: Entity,
    pub from: Vec3,
    pub to: Vec3,
    pub look: TracerLook,
}

/// Each shooter's last muzzle-flash point (world) and when (seconds).
#[derive(Resource, Default)]
pub struct MuzzleCache(pub HashMap<Entity, (Vec3, f64)>);

/// Tracers waiting a moment for this shot's muzzle flash.
#[derive(Resource, Default)]
pub(super) struct PendingTracers(Vec<(Tracer, f64)>);

/// How long a tracer waits for its shot's muzzle flash, seconds.
const WAIT: f64 = 0.05;

pub(super) fn draw_tracers(
    mut requests: MessageReader<Tracer>,
    mut pending: ResMut<PendingTracers>,
    muzzles: Res<MuzzleCache>,
    materials: Option<Res<ParticleMaterials>>,
    mut particles: ResMut<Particles>,
    time: Res<Time>,
    mut seed: Local<u64>,
) {
    let now = time.elapsed_secs_f64();
    pending.0.extend(requests.read().map(|t| (t.clone(), now)));
    let Some(materials) = materials else {
        pending.0.clear();
        return;
    };
    let mut rand = || {
        *seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        (*seed >> 40) as f32 / (1u64 << 24) as f32
    };
    let mut keep = Vec::new();
    for (t, asked) in pending.0.drain(..) {
        let fresh = muzzles.0.get(&t.owner).filter(|(_, when)| *when >= asked - 1e-3);
        let start = match fresh {
            Some((at, _)) => *at,
            None if now - asked < WAIT => {
                keep.push((t, asked));
                continue;
            }
            None => t.from,
        };
        let Some(material) = materials.0.find(t.look.material) else { continue };
        let to = t.to - start;
        let distance = to.length();
        if distance < t.look.min_distance {
            continue;
        }
        let lerp = |(a, b): (f32, f32), r: f32| a + (b - a) * r;
        let length = lerp(t.look.length, rand());
        let width = lerp(t.look.half_width, rand());
        let mut p = Particle::new(start, (distance + length) / t.look.speed, material, 0.0);
        p.fade = Fade::Ramp(Ramp::constant(1.0));
        p.shape = Shape::Streak {
            start,
            dir: to / distance,
            distance,
            length,
            width,
            speed: t.look.speed,
        };
        let mut group = ParticleGroup::new(Motion::default());
        group.capped = false;
        group.skip_first = false;
        group.particles.push(p);
        particles.add(group);
    }
    pending.0 = keep;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::map::particles::{MapParticles, ParticleMaterial};

    const LOOK: TracerLook = TracerLook {
        material: "spark",
        speed: 127.0,
        length: (1.6, 3.2),
        half_width: (0.02, 0.023),
        min_distance: 6.5,
    };

    fn app() -> App {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .init_resource::<Particles>()
            .init_resource::<MuzzleCache>()
            .init_resource::<PendingTracers>()
            .add_message::<Tracer>()
            .insert_resource(ParticleMaterials(MapParticles {
                materials: vec![ParticleMaterial {
                    name: "spark".into(),
                    ..default()
                }],
            }))
            .add_systems(Update, draw_tracers);
        app
    }

    fn shoot(app: &mut App, owner: Entity, to: Vec3) {
        app.world_mut().write_message(Tracer {
            owner,
            from: Vec3::ZERO,
            to,
            look: LOOK,
        });
    }

    #[test]
    fn tracers_start_at_the_muzzle_or_after_a_wait_at_the_shot() {
        let mut app = app();
        let owner = app.world_mut().spawn_empty().id();
        // A long shot with no muzzle flash yet: waits, then starts at the eye.
        shoot(&mut app, owner, Vec3::new(0.0, 0.0, -20.0));
        app.update();
        assert_eq!(app.world().resource::<Particles>().count(), 0, "waits for the flash");
        std::thread::sleep(std::time::Duration::from_millis(60));
        app.update();
        let streak = |app: &App| match app.world().resource::<Particles>().groups.last().unwrap().particles[0].shape {
            Shape::Streak { start, distance, .. } => (start, distance),
            _ => panic!("not a streak"),
        };
        assert_eq!(streak(&app).0, Vec3::ZERO);
        // With a fresh muzzle point it starts there at once.
        let now = app.world().resource::<Time>().elapsed_secs_f64();
        app.world_mut()
            .resource_mut::<MuzzleCache>()
            .0
            .insert(owner, (Vec3::new(0.3, -0.2, 0.0), now + 1.0));
        shoot(&mut app, owner, Vec3::new(0.0, 0.0, -20.0));
        app.update();
        assert_eq!(streak(&app).0, Vec3::new(0.3, -0.2, 0.0));
        // Short shots draw nothing.
        let before = app.world().resource::<Particles>().count();
        shoot(&mut app, owner, Vec3::new(0.0, 0.0, -3.0));
        app.update();
        assert_eq!(app.world().resource::<Particles>().count(), before);
    }
}
