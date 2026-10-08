//! Impact sounds (specs/cs_source/sounds.md 4): bullet impacts by the hit
//! surface, and physics impacts by both touching surfaces and the impact
//! speed. Scrapes are not done yet (the physics engine's friction energy
//! that drives them is engine-side; spec open question 8).

use std::collections::HashMap;

use avian3d::prelude::*;
use bevy::prelude::*;

use crate::{
    core::{Intent, MapWater},
    map::{
        PhysicsProp, PlaySound, PropSurface,
        breakables::PieceBody,
        decal::{DecalGroup, PlaceDecal},
        sound::{MapSounds, MapSurface, SoundBank, SurfaceGrid},
    },
    weapon::{WeaponEvent, WeaponEventKind},
};

const UNIT: f32 = 0.0254;
/// Impacts below this speed are silent, u/s.
const IMPACT_MIN_SPEED: f32 = 70.0;
/// Volume reaches 1 at this speed, u/s.
const IMPACT_FULL_SPEED: f32 = 320.0;
/// A pair's contacts closer together than this make one sound, s.
const IMPACT_MIN_DT: f64 = 0.05;
/// The same for client-side physics objects (a broken prop's pieces): a
/// sound needs strictly more than this since the pair's last contact.
const IMPACT_MIN_DT_CLIENT: f64 = 0.1;
/// After this many queued impacts in a frame, new ones merge into the last.
const IMPACT_MERGE_AFTER: usize = 4;
/// One shot's pellets skip an identical impact sound within this, units.
const IMPACT_GROUP_RADIUS: f32 = 300.0;
/// How far from a contact or hit point a world triangle may be, m.
const SURFACE_REACH: f32 = 0.3;

pub struct ImpactSoundsPlugin;

impl Plugin for ImpactSoundsPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<LastContact>()
            .add_message::<PlaySound>()
            .add_message::<WeaponEvent>()
            .add_message::<PlaceDecal>()
            .add_systems(
                FixedUpdate,
                (bullet_impacts, impact_decals).after(crate::core::SimSet::Weapons),
            )
            .add_systems(FixedPostUpdate, physics_impacts.after(PhysicsSystems::Writeback));
    }
}

/// Physics impact volume for an impact speed in u/s: (s/320)², at most 1.
pub fn impact_volume(speed: f32) -> f32 {
    (speed / IMPACT_FULL_SPEED).powi(2).min(1.0)
}

/// The entry an object with surface `own` plays when hitting `hit` at
/// `speed` u/s: the hard impact, or the soft one for soft targets or slow
/// impacts. None when the surface has no hard impact.
pub fn impact_entry<'a>(own: &'a MapSurface, hit: &MapSurface, speed: f32) -> Option<&'a str> {
    let hard = own.impact_hard.as_deref()?;
    let soft = own.impact_soft.as_deref();
    let use_soft = hit.hardness < own.hard_threshold || (own.hard_min_velocity > 0.0 && speed < own.hard_min_velocity);
    Some(match (use_soft, soft) {
        (true, Some(s)) => s,
        _ => hard,
    })
}

pub(super) fn surface<'a>(sounds: &'a MapSounds, name: &str) -> Option<&'a MapSurface> {
    sounds.surface(name).or_else(|| sounds.surface("default"))
}

/// Entry volume as the script sets it (middle of its range).
fn script_volume(sounds: &MapSounds, entry: &str) -> f32 {
    sounds
        .entry(entry)
        .map_or(1.0, |e| e.volume.start + e.volume.range / 2.0)
}

/// Surface name of what a trace or contact touched.
pub(super) fn surface_of(
    entity: Option<Entity>,
    at: Vec3,
    props: &Query<&PropSurface>,
    characters: &Query<(), With<Intent>>,
    grid: Option<&SurfaceGrid>,
) -> String {
    if let Some(e) = entity {
        if let Ok(s) = props.get(e) {
            return s.0.clone();
        }
        if characters.contains(e) {
            return "player".into();
        }
    }
    grid.and_then(|g| g.nearest(at, SURFACE_REACH))
        .unwrap_or("default")
        .to_string()
}

fn bullet_impacts(
    mut events: MessageReader<WeaponEvent>,
    bank: Option<Res<SoundBank>>,
    grid: Option<Res<SurfaceGrid>>,
    props: Query<&PropSurface>,
    characters: Query<(), With<Intent>>,
    water: Option<Res<MapWater>>,
    mut play: MessageWriter<PlaySound>,
) {
    let Some(bank) = bank else {
        events.clear();
        return;
    };
    // Played this tick per shooter: (entry, position), for pellet grouping.
    let mut played: Vec<(Entity, String, Vec3)> = Vec::new();
    for e in events.read() {
        // Every surface a bullet reaches, also after passing something.
        let (WeaponEventKind::Shot { from, to, hit, .. } | WeaponEventKind::ShotContinued { from, to, hit, .. }) =
            &e.kind
        else {
            continue;
        };
        if hit.is_none() || in_water(*from, *to, water.as_deref()) {
            continue;
        }
        let name = surface_of(*hit, *to, &props, &characters, grid.as_deref());
        let Some(entry) = surface(&bank.0, &name).and_then(|s| s.bullet_impact.clone()) else {
            continue;
        };
        if played
            .iter()
            .any(|(o, n, p)| *o == e.owner && *n == entry && p.distance(*to) < IMPACT_GROUP_RADIUS * UNIT)
        {
            continue;
        }
        played.push((e.owner, entry.clone(), *to));
        play.write(PlaySound::at(entry, *to));
    }
}

/// A shot ending in water makes no impact (a splash instead, or nothing
/// under water; specs/cs_source/impact_effects.md section 11).
fn in_water(from: Vec3, to: Vec3, water: Option<&MapWater>) -> bool {
    water.is_some_and(|w| super::impact_effects::water_shot(from, to, &w.0) != super::impact_effects::WaterShot::Dry)
}

/// The knife's mark on walls. Not in the public data: a guess from the
/// decal script's slash group, to check against the game.
const SLASH_DECAL: &str = "ManhackCut";

/// Decals where shots and knife swings hit the world: the hit surface's
/// impact decal ("TranslationData" by its game material), the knife's
/// slash, on the world or the prop that was hit. Characters take none
/// yet.
fn impact_decals(
    mut events: MessageReader<WeaponEvent>,
    bank: Option<Res<SoundBank>>,
    grid: Option<Res<SurfaceGrid>>,
    props: Query<&PropSurface>,
    characters: Query<(), With<Intent>>,
    water: Option<Res<MapWater>>,
    mut decals: MessageWriter<PlaceDecal>,
) {
    // Characters take blood, not these (not done yet).
    let marked = |e: Entity| !characters.contains(e);
    for e in events.read() {
        match &e.kind {
            // Every surface a bullet enters, also after passing something
            // (M13 logs an impact per surface reached; exits are not
            // logged, so they get no mark).
            WeaponEventKind::Shot {
                from,
                to,
                hit: Some(hit),
                normal: Some(normal),
            }
            | WeaponEventKind::ShotContinued {
                from,
                to,
                hit: Some(hit),
                normal: Some(normal),
            } if marked(*hit) && !in_water(*from, *to, water.as_deref()) => {
                let name = surface_of(Some(*hit), *to, &props, &characters, grid.as_deref());
                let Some(material) = bank
                    .as_ref()
                    .and_then(|b| surface(&b.0, &name))
                    .map(|s| s.game_material)
                else {
                    continue;
                };
                decals.write(PlaceDecal {
                    target: Some(*hit),
                    group: DecalGroup::Material(material),
                    point: *to,
                    normal: *normal,
                    dir: (*to - *from).normalize_or_zero(),
                    spin: false,
                });
            }
            // A swing into water only splashes (impact_effects).
            WeaponEventKind::Swing {
                at: Some((point, normal, hit)),
                line,
                ..
            } if marked(*hit) && super::impact_effects::knife_splash(*line, water.as_deref()).is_none() => {
                decals.write(PlaceDecal {
                    target: Some(*hit),
                    group: DecalGroup::Named(SLASH_DECAL.into()),
                    point: *point,
                    normal: *normal,
                    dir: -*normal,
                    spin: false,
                });
            }
            _ => {}
        }
    }
}

/// When each colliding pair last touched (fixed-tick seconds).
#[derive(Resource, Default)]
struct LastContact(HashMap<(Entity, Entity), f64>);

/// A queued impact sound for this physics frame.
struct Queued {
    own: String,
    hit: String,
    at: Vec3,
    volume: f32,
    /// The largest of the merged impacts' volumes and speeds.
    loudest: f32,
    speed: f32,
}

#[allow(clippy::too_many_arguments)]
fn physics_impacts(
    mut started: MessageReader<CollisionStart>,
    collisions: Collisions,
    bodies: Query<(&RigidBody, Option<&PhysicsProp>, Has<PieceBody>)>,
    props: Query<&PropSurface>,
    characters: Query<(), With<Intent>>,
    bank: Option<Res<SoundBank>>,
    grid: Option<Res<SurfaceGrid>>,
    mut last: ResMut<LastContact>,
    mut play: MessageWriter<PlaySound>,
    time: Res<Time>,
) {
    let Some(bank) = bank else {
        started.clear();
        return;
    };
    let sounds = &bank.0;
    let now = time.elapsed_secs_f64();
    let mut queue: Vec<Queued> = Vec::new();
    for start in started.read() {
        let Some(pair) = collisions.get(start.collider1, start.collider2) else {
            continue;
        };
        let key = (pair.collider1.min(pair.collider2), pair.collider1.max(pair.collider2));
        let previous = last.0.insert(key, now);
        let piece = |b: Option<Entity>| b.and_then(|b| bodies.get(b).ok()).is_some_and(|(.., p)| p);
        let client = piece(pair.body1) || piece(pair.body2);
        if previous.is_some_and(|t| {
            if client {
                now - t <= IMPACT_MIN_DT_CLIENT
            } else {
                now - t < IMPACT_MIN_DT
            }
        }) {
            continue;
        }
        // Approach speed before the solver, in u/s.
        let Some((speed, at)) = pair
            .manifolds
            .iter()
            .flat_map(|m| m.points.iter())
            .map(|p| (-p.normal_speed / UNIT, p.point))
            .max_by(|a, b| a.0.total_cmp(&b.0))
        else {
            continue;
        };
        if speed < IMPACT_MIN_SPEED {
            continue;
        }
        let simulated = |b: Option<Entity>| {
            b.and_then(|b| bodies.get(b).ok())
                .is_some_and(|(rb, prop, piece)| rb.is_dynamic() && (prop.is_some() || piece))
        };
        let sides = [
            (pair.collider1, pair.body1, pair.collider2, pair.body2),
            (pair.collider2, pair.body2, pair.collider1, pair.body1),
        ];
        if !sides.iter().any(|s| simulated(s.1)) {
            continue;
        }
        let names = sides.map(|(c, ..)| {
            let world = bodies.get(c).is_ok_and(|(rb, ..)| rb.is_static()) && !props.contains(c);
            surface_of((!world).then_some(c), at, &props, &characters, grid.as_deref())
        });
        // Silent materials (game material X).
        if names
            .iter()
            .any(|n| surface(sounds, n).is_some_and(|s| s.game_material == 'X'))
        {
            continue;
        }
        for (i, side) in sides.iter().enumerate() {
            if !simulated(side.1) {
                continue;
            }
            let (own, hit) = (names[i].clone(), names[1 - i].clone());
            let v = impact_volume(speed);
            // Merge into an entry for the same surface (any entry once the
            // queue is long), searching from the end.
            let full = queue.len() > IMPACT_MERGE_AFTER;
            match queue.iter_mut().rev().find(|q| full || q.own == own) {
                Some(q) => {
                    if v > q.loudest {
                        q.at = at;
                        q.hit = hit;
                        q.loudest = v;
                    }
                    q.volume += v;
                    q.speed = q.speed.max(speed + 0.0001);
                }
                None => queue.push(Queued {
                    own,
                    hit,
                    at,
                    volume: v,
                    loudest: v,
                    speed: speed + 0.0001,
                }),
            }
        }
    }
    // Forget pairs that haven't touched for a while.
    if last.0.len() > 4096 {
        last.0.retain(|_, t| now - *t < 1.0);
    }
    for q in queue.iter().rev() {
        let (Some(own), Some(hit)) = (surface(sounds, &q.own), surface(sounds, &q.hit)) else {
            continue;
        };
        let Some(entry) = impact_entry(own, hit, q.speed) else {
            continue;
        };
        if sounds.entry(entry).is_none() {
            // A failed lookup stops the rest of the queue (spec quirk).
            break;
        }
        play.write(PlaySound {
            entry: entry.to_string(),
            at: Some(q.at),
            volume: Some(script_volume(sounds, entry) * q.volume.min(1.0)),
            source: None,
            channel: None,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn impact_volumes() {
        // Spec: 70 -> 0.0478515625, 160 -> 0.25, 320+ -> 1.
        assert!((impact_volume(70.0) - 0.047_851_563).abs() < 1e-7);
        assert!((impact_volume(160.0) - 0.25).abs() < 1e-6);
        assert_eq!(impact_volume(500.0), 1.0);
    }

    #[test]
    fn soft_against_soft_targets() {
        let crate_ = MapSurface {
            impact_hard: Some("Wood_Crate.ImpactHard".into()),
            impact_soft: Some("Wood_Crate.ImpactSoft".into()),
            hard_threshold: 0.5,
            ..default()
        };
        let concrete = MapSurface {
            hardness: 1.0,
            ..default()
        };
        let dirt = MapSurface {
            hardness: 0.25,
            ..default()
        };
        assert_eq!(impact_entry(&crate_, &concrete, 200.0), Some("Wood_Crate.ImpactHard"));
        assert_eq!(impact_entry(&crate_, &dirt, 200.0), Some("Wood_Crate.ImpactSoft"));
        let slow = MapSurface {
            hard_min_velocity: 300.0,
            ..crate_.clone()
        };
        assert_eq!(impact_entry(&slow, &concrete, 200.0), Some("Wood_Crate.ImpactSoft"));
        let no_hard = MapSurface::default();
        assert_eq!(impact_entry(&no_hard, &concrete, 200.0), None);
    }
}
