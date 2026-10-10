//! Bots on maps without a navigation mesh (most minigame, surf, bhop and
//! kz maps ship none): CS:S would generate one; ours can't walk, so they
//! at least keep out of the way. Each walks a few steps off the spawn
//! points and teleport destinations it stands near, to a clear spot on
//! the ground beside them outside every teleport trigger, and waits
//! there. Fighting is left as it is (it shoots what it sees from where it
//! stands). The console says once per map that bots have no mesh there.
//! Our design (docs/tech-debt.md): no CS:S behaviour to match.

use avian3d::prelude::*;
use bevy::prelude::*;

use super::Bot;
use crate::{
    character::CAPSULE_HEIGHT,
    core::{Health, Intent, SpawnPoint},
    map::{MapEntities, nav::NavMesh},
};

const UNIT: f32 = 0.0254;
/// How far from a spawn or teleport destination a bot stands (a player's
/// box is 32 units wide: two of them and some room).
const CLEAR: f32 = 72.0 * UNIT;
/// How far it looks for a spot.
const RINGS: [f32; 6] = [80.0, 112.0, 144.0, 192.0, 256.0, 320.0];
const DIRECTIONS: usize = 16;
/// Close enough to its spot.
const ARRIVED: f32 = 10.0 * UNIT;
/// Steps up it may take and drops it accepts on the way.
const STEP: f32 = 18.0 * UNIT;
/// Gives up on a spot it hasn't neared for this long (seconds).
const GIVE_UP: f32 = 2.0;

/// A bot's spot to stand aside at.
#[derive(Component, Default, Debug)]
pub struct Aside {
    pub spot: Option<Vec3>,
    /// Where it was and when it last got closer.
    best: f32,
    since: f64,
    /// Feet last tick (a jump means a teleport or respawn: look again).
    last: Option<Vec3>,
    /// Spots given up on.
    failed: Vec<Vec3>,
}

/// Spawn points and teleport destinations (engine space, feet), and the
/// teleport triggers' bounds (min, max).
pub fn keep_clear(spawns: &[Vec3], map: Option<&MapEntities>) -> (Vec<Vec3>, Vec<(Vec3, Vec3)>) {
    use crate::map::entities::{entity_rotation, entity_to_engine};
    let mut spots: Vec<Vec3> = spawns.to_vec();
    let mut volumes = Vec::new();
    let Some(map) = map else { return (spots, volumes) };
    let targets: std::collections::HashSet<String> = map
        .entities
        .iter()
        .filter(|e| e.classname() == "trigger_teleport")
        .filter_map(|e| e.get("target").map(|t| t.trim().to_lowercase()))
        .collect();
    for e in map.entities.iter() {
        let class = e.classname();
        let named = e
            .get("targetname")
            .is_some_and(|n| targets.contains(&n.trim().to_lowercase()));
        if class == "info_teleport_destination" || (named && class != "trigger_teleport") {
            spots.push(entity_to_engine(e.origin(), map.scale));
        }
        if class == "trigger_teleport" {
            let rot = entity_rotation(e.angles());
            let points: Vec<Vec3> = e
                .hulls
                .iter()
                .flat_map(|h| h.points.iter())
                .map(|p| entity_to_engine(e.origin() + rot * *p, map.scale))
                .collect();
            if !points.is_empty() {
                let lo = points.iter().fold(Vec3::splat(f32::MAX), |a, p| a.min(*p));
                let hi = points.iter().fold(Vec3::splat(f32::MIN), |a, p| a.max(*p));
                volumes.push((lo, hi));
            }
        }
    }
    (spots, volumes)
}

/// Whether a bot standing with its feet at `p` is in the way: near a spot
/// to keep clear or inside a teleport trigger (with the box's reach).
pub fn in_the_way(p: Vec3, spots: &[Vec3], volumes: &[(Vec3, Vec3)]) -> bool {
    near(p, spots, volumes, 0.0)
}

/// `in_the_way` with `margin` (meters) more room around the bot.
fn near(p: Vec3, spots: &[Vec3], volumes: &[(Vec3, Vec3)], margin: f32) -> bool {
    let reach = Vec3::new(16.0 * UNIT + margin, 72.0 * UNIT, 16.0 * UNIT + margin);
    // Feet resting on a trigger's top face (bhop maps put teleports under
    // their floors) don't reach into it.
    let feet = p - reach.with_y(0.0) + Vec3::Y * 2.0 * UNIT;
    spots
        .iter()
        .any(|s| (s - p).with_y(0.0).length() < CLEAR + margin && (s.y - p.y).abs() < 72.0 * UNIT)
        || volumes
            .iter()
            .any(|(lo, hi)| (p + reach).cmpgt(*lo).all() && feet.cmplt(*hi).all())
}

#[allow(clippy::type_complexity, clippy::too_many_arguments)]
pub fn step_aside(
    mut commands: Commands,
    mut bots: Query<(Entity, &Bot, &mut Intent, &Transform, &Health, Option<&mut Aside>)>,
    characters: Query<Entity, With<Intent>>,
    spawns: Query<&Transform, With<SpawnPoint>>,
    nav: Option<Res<NavMesh>>,
    map: Option<Res<MapEntities>>,
    file: Option<Res<crate::map::MapFile>>,
    mut noted: Local<Option<String>>,
    mut console: Option<ResMut<crate::console::Console>>,
    spatial: SpatialQuery,
    time: Res<Time>,
) {
    // A loaded map without a mesh (maps built in code, as tests use, have
    // no file).
    if nav.as_deref().is_some_and(|n| !n.areas.is_empty()) || bots.is_empty() || map.is_none() {
        return;
    }
    let Some(name) = file.as_deref().map(|f| f.name.clone()) else {
        return;
    };
    if noted.as_deref() != Some(name.as_str()) {
        *noted = Some(name.clone());
        let line = format!(
            "Bots: {name} has no navigation mesh (maps/{name}.nav): bots can't move around this map. \
             They step off spawns and teleport destinations and wait."
        );
        info!("{line}");
        if let Some(c) = console.as_deref_mut() {
            c.info(line);
        }
    }
    let spawn_feet: Vec<Vec3> = spawns.iter().map(|t| t.translation).collect();
    let (spots, volumes) = keep_clear(&spawn_feet, map.as_deref());
    let filter = SpatialQueryFilter::default().with_mask(crate::core::SOLID_LAYERS);
    let not_character = |e: Entity| !characters.contains(e);
    let now = time.elapsed_secs_f64();
    let mut taken: Vec<Vec3> = bots.iter().filter_map(|b| b.5.and_then(|a| a.spot)).collect();
    for (me, bot, mut intent, t, health, aside) in &mut bots {
        let Some(mut aside) = aside else {
            commands.entity(me).insert(Aside::default());
            continue;
        };
        if health.current <= 0.0 {
            aside.spot = None;
            aside.last = None;
            continue;
        }
        let feet = t.translation - Vec3::Y * CAPSULE_HEIGHT / 2.0;
        if aside.last.is_some_and(|l| l.distance(feet) > 64.0 * UNIT) {
            // Teleported or respawned.
            aside.spot = None;
            aside.failed.clear();
        }
        aside.last = Some(feet);
        if aside.spot.is_none() && in_the_way(feet, &spots, &volumes) {
            let ray = |a: Vec3, b: Vec3| -> Option<f32> {
                let d = b - a;
                let dir = Dir3::new(d).ok()?;
                spatial
                    .cast_ray_predicate(a, dir, d.length(), true, &filter, &not_character)
                    .map(|h| h.distance)
            };
            let knee = Vec3::Y * STEP * 1.2;
            let spot = RINGS.iter().find_map(|r| {
                (0..DIRECTIONS).find_map(|k| {
                    let a = k as f32 / DIRECTIONS as f32 * std::f32::consts::TAU;
                    let p = feet + Vec3::new(a.cos(), 0.0, a.sin()) * r * UNIT;
                    // The way there is open at knee and head height, a
                    // body's width beyond.
                    let beyond = (p - feet).normalize_or_zero() * 20.0 * UNIT;
                    let open = [knee, Vec3::Y * 60.0 * UNIT]
                        .iter()
                        .all(|h| ray(feet + *h, p + beyond + *h).is_none());
                    // Ground under it, no drop.
                    let ground = ray(p + knee, p - Vec3::Y * STEP * 2.0).map(|d| p + knee - Vec3::Y * d);
                    let ground = ground?;
                    (open
                        // With room for where it stops (`ARRIVED`).
                        && !near(ground, &spots, &volumes, ARRIVED * 2.0)
                        && !taken.iter().any(|s| s.distance(ground) < CLEAR / 2.0)
                        && !aside.failed.iter().any(|s| s.distance(ground) < ARRIVED * 2.0))
                    .then_some(ground)
                })
            });
            if let Some(s) = spot {
                taken.push(s);
                aside.spot = Some(s);
                aside.best = f32::MAX;
                aside.since = now;
            }
        }
        let Some(spot) = aside.spot else { continue };
        let to = (spot - feet).with_y(0.0);
        if to.length() < ARRIVED {
            if bot.target.is_none() {
                intent.move_axis = Vec2::ZERO;
            }
            continue;
        }
        if to.length() < aside.best - 1.0 * UNIT {
            aside.best = to.length();
            aside.since = now;
        } else if now - aside.since > GIVE_UP as f64 {
            aside.failed.push(spot);
            aside.spot = None;
            continue;
        }
        // Walk there (fighting keeps its own keys).
        if bot.target.is_none() {
            let dir = to.normalize();
            intent.yaw = (-dir.x).atan2(-dir.z);
            intent.move_axis = Vec2::new(0.0, 1.0);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spawns_destinations_and_teleports_are_in_the_way() {
        let spots = [Vec3::ZERO];
        let volumes = [(Vec3::new(5.0, 0.0, 5.0), Vec3::new(6.0, 1.0, 6.0))];
        assert!(in_the_way(Vec3::new(0.5, 0.0, 0.0), &spots, &volumes));
        assert!(!in_the_way(Vec3::new(2.5, 0.0, 0.0), &spots, &volumes), "well off it");
        assert!(!in_the_way(Vec3::new(0.0, 3.0, 0.0), &spots, &volumes), "a floor above");
        assert!(in_the_way(Vec3::new(5.5, 0.2, 5.5), &spots, &volumes), "in a teleport");
        assert!(
            in_the_way(Vec3::new(4.9, 0.2, 5.5), &spots, &volumes),
            "its box reaches in"
        );
        assert!(
            !in_the_way(Vec3::new(5.5, 1.0, 5.5), &spots, &volumes),
            "standing on its top"
        );
    }
}
