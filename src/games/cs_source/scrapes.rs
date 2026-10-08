//! Scrapes: looping friction sounds of physics props, broken props' pieces
//! and ragdolls sliding on something (specs/cs_source/sounds.md 4,
//! "Scrapes"). The spec's rule runs on the physics engine's friction
//! energy, whose units and formula are engine-side (spec open question 8);
//! ours is a stand-in from avian's contacts (`friction_energy`).
//!
//! Each sliding body keeps at most one scrape, against the contact that
//! slides hardest; props share the server's 4 slots, pieces and ragdolls
//! the client's 8. A scrape is a long-lived sound (`map::live_sound`) that
//! follows its body, ramps to each new volume and pitch over 0.1 s and
//! stops 0.1 s after its last refresh.

use avian3d::prelude::*;
use bevy::prelude::*;

use crate::{
    core::{FirstTimePredicted, Intent, PropSurface},
    map::{
        PhysicsProp, SoundControl, SoundKey, StartSound,
        breakables::PieceBody,
        ragdoll::RagdollBody,
        sound::{MapSounds, MapSurface, SoundBank, SurfaceGrid},
    },
};

use super::impacts::{surface, surface_of};

const UNIT: f32 = 0.0254;
/// Reports below this energy are ignored.
pub const SCRAPE_MIN_ENERGY: f32 = 75.0;
/// Energy to amplitude.
pub const SCRAPE_ENERGY_SCALE: f32 = 1.0 / 15500.0;
/// Amplitude² must exceed this to do anything.
pub const SCRAPE_MIN_VOLUME: f32 = 1.0 / 128.0;
/// Script volume × amplitude² must exceed this to start a scrape.
pub const SCRAPE_START_VOLUME: f32 = 0.1;
/// Volume and pitch ramp to a new value over this, s.
pub const SCRAPE_RAMP: f32 = 0.1;
/// A scrape not refreshed for this long stops, s.
pub const SCRAPE_TIMEOUT: f64 = 0.1;
/// A server object's playing scrape is updated at most this often, s
/// (refreshes in between only keep it alive).
pub const SCRAPE_SERVER_UPDATE: f64 = 0.5;
/// Simultaneous scrapes of server objects (props) and client objects
/// (pieces, ragdolls).
pub const SCRAPE_SLOTS_SERVER: usize = 4;
pub const SCRAPE_SLOTS_CLIENT: usize = 8;
/// Our stand-in's time base: the friction work of one Source tick, s.
const SOURCE_TICK: f32 = 0.015;
/// Bodies whose fastest point moves slower than this can't scrape (u/s):
/// a cheap test before reading their contacts.
const MIN_SLIDE_SPEED: f32 = 5.0;
/// Scrape sound keys (`live_sound::SoundKey`): this bit, not the logic's
/// (63) or the soundscape's (62).
const SCRAPE_KEYS: u64 = 1 << 61;

/// Friction energy, our stand-in for Source's (spec open question 8): the
/// work friction does in one Source tick (0.015 s) at the contact, in
/// kg·in²/s² (Source's energy units): friction coefficient × normal force
/// (N) × sliding speed (m/s) × 0.015 s, over 0.0254² m² per in². A 30 kg
/// crate sliding at 100 u/s on concrete (friction 0.8) gives about 13 900,
/// near full volume; 2 kg debris at the same speed about 930, silent.
pub fn friction_energy(friction: f32, normal_force: f32, slide_speed: f32) -> f32 {
    friction.max(0.0) * normal_force.max(0.0) * slide_speed.max(0.0) * SOURCE_TICK / (UNIT * UNIT)
}

/// The scrape amplitude² (volume factor) for friction energy `e`, or None
/// when it does nothing (spec rule steps 1, 2 and 4).
pub fn scrape_volume(e: f32) -> Option<f32> {
    if e < SCRAPE_MIN_ENERGY {
        return None;
    }
    let a = e * SCRAPE_ENERGY_SCALE;
    let v = (a * a).clamp(0.0, 1.0);
    (v > SCRAPE_MIN_VOLUME).then_some(v)
}

/// The entry an object with surface `own` scrapes with on `other`: the
/// rough one, or the smooth one when it has one and the other surface is
/// smoother than its threshold (rule step 3).
pub fn scrape_entry<'a>(own: &'a MapSurface, other: &MapSurface) -> Option<&'a str> {
    match own.scrape_smooth.as_deref() {
        Some(smooth) if other.roughness < own.rough_threshold => Some(smooth),
        _ => own.scrape_rough.as_deref(),
    }
}

/// A scrape's target pitch at volume factor `v`: across the entry's pitch
/// range (rule step 7).
pub fn scrape_pitch(entry: &crate::map::MapSoundEntry, v: f32) -> f32 {
    entry.pitch.start + v * entry.pitch.range
}

/// A playing scrape.
#[derive(Clone, Debug)]
pub struct Scrape {
    pub body: Entity,
    pub key: SoundKey,
    pub entry: String,
    /// A client object's (piece, ragdoll): updated on every refresh.
    pub client: bool,
    pub refreshed: f64,
    pub updated: f64,
    pub volume: f32,
    pub pitch: f32,
    target: (f32, f32),
    step: (f32, f32),
}

/// The scrapes playing now.
#[derive(Resource, Default)]
pub struct Scrapes {
    pub playing: Vec<Scrape>,
    next_key: u64,
}

pub struct ScrapePlugin;

impl Plugin for ScrapePlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Scrapes>()
            .add_message::<SoundControl>()
            .add_systems(FixedPostUpdate, scrape.after(PhysicsSystems::Writeback));
    }
}

/// A body's velocity at a world point.
fn point_velocity(v: Vec3, w: Vec3, centre: Vec3, at: Vec3) -> Vec3 {
    v + w.cross(at - centre)
}

type BodyData<'a> = (
    Entity,
    &'a LinearVelocity,
    &'a AngularVelocity,
    &'a Position,
    &'a Rotation,
    &'a ComputedCenterOfMass,
    &'a PropSurface,
    Has<PhysicsProp>,
);

#[allow(clippy::too_many_arguments, clippy::type_complexity)]
fn scrape(
    sliders: Query<
        BodyData,
        (
            Or<(With<PhysicsProp>, With<PieceBody>, With<RagdollBody>)>,
            Without<Sleeping>,
            Without<RigidBodyDisabled>,
        ),
    >,
    others: Query<(
        &LinearVelocity,
        &AngularVelocity,
        &Position,
        &Rotation,
        &ComputedCenterOfMass,
    )>,
    statics: Query<&RigidBody>,
    collisions: Collisions,
    props: Query<&PropSurface>,
    characters: Query<(), With<Intent>>,
    bank: Option<Res<SoundBank>>,
    grid: Option<Res<SurfaceGrid>>,
    first: Res<FirstTimePredicted>,
    time: Res<Time>,
    mut scrapes: ResMut<Scrapes>,
    mut control: MessageWriter<SoundControl>,
) {
    let Some(bank) = bank else { return };
    let sounds = &bank.0;
    let now = time.elapsed_secs_f64();
    let dt = time.delta_secs();
    // A replayed command doesn't sound again (the physics doesn't run in
    // replays today; kept for when it does).
    if first.0 && dt > 0.0 {
        for (body, v, w, pos, rot, com, own, server) in &sliders {
            let centre = pos.0 + rot.0 * com.0;
            // Fastest a point of it could move (a generous radius): skip
            // the still ones before reading contacts.
            let fastest = (v.0.length() + w.0.length() * 2.0) / UNIT;
            if fastest < MIN_SLIDE_SPEED {
                continue;
            }
            // The contact sliding hardest: (energy, other's surface).
            let mut best: Option<(f32, String)> = None;
            for pair in collisions.collisions_with(body) {
                let (other, other_body) = if pair.collider1 == body {
                    (pair.collider2, pair.body2)
                } else {
                    (pair.collider1, pair.body1)
                };
                let other_motion = other_body.and_then(|b| others.get(b).ok());
                for m in &pair.manifolds {
                    let normal = m.normal;
                    for p in &m.points {
                        let mut rel = point_velocity(v.0, w.0, centre, p.point);
                        if let Some((ov, ow, op, or, oc)) = other_motion {
                            rel -= point_velocity(ov.0, ow.0, op.0 + or.0 * oc.0, p.point);
                        }
                        let slide = (rel - normal * rel.dot(normal)).length();
                        let force = p.normal_impulse / dt;
                        let e = friction_energy(m.friction, force, slide);
                        if best.as_ref().is_none_or(|b| e > b.0) {
                            let world = other_body
                                .and_then(|b| statics.get(b).ok())
                                .is_none_or(RigidBody::is_static)
                                && !props.contains(other);
                            let name =
                                surface_of((!world).then_some(other), p.point, &props, &characters, grid.as_deref());
                            best = Some((e, name));
                        }
                    }
                }
            }
            let Some((energy, other)) = best else { continue };
            report(
                &mut scrapes,
                sounds,
                body,
                &own.0,
                &other,
                energy,
                !server,
                now,
                &mut control,
            );
        }
    }
    // Stop the scrapes not refreshed lately; ramp the others.
    scrapes.playing.retain_mut(|s| {
        if now - s.refreshed > SCRAPE_TIMEOUT {
            control.write(SoundControl::Stop(s.key));
            return false;
        }
        let (volume, pitch) = (
            approach(s.volume, s.target.0, s.step.0 * dt),
            approach(s.pitch, s.target.1, s.step.1 * dt),
        );
        if volume != s.volume || pitch != s.pitch {
            s.volume = volume;
            s.pitch = pitch;
            control.write(SoundControl::Change {
                key: s.key,
                volume: Some(volume),
                pitch: Some(pitch),
            });
        }
        true
    });
}

fn approach(from: f32, to: f32, step: f32) -> f32 {
    from + (to - from).clamp(-step.abs(), step.abs())
}

/// One friction report for `body` (surface `own`) sliding on `other` with
/// energy `e`: the spec's rule.
#[allow(clippy::too_many_arguments)]
fn report(
    scrapes: &mut Scrapes,
    sounds: &MapSounds,
    body: Entity,
    own: &str,
    other: &str,
    e: f32,
    client: bool,
    now: f64,
    control: &mut MessageWriter<SoundControl>,
) {
    let (Some(own_s), Some(other_s)) = (surface(sounds, own), surface(sounds, other)) else {
        return;
    };
    // A playing server scrape updated less than 0.5 s ago is only kept
    // alive.
    if let Some(s) = scrapes.playing.iter_mut().find(|s| s.body == body)
        && !s.client
        && now - s.updated < SCRAPE_SERVER_UPDATE
    {
        s.refreshed = now;
        return;
    }
    if e < SCRAPE_MIN_ENERGY || own_s.game_material == 'X' || other_s.game_material == 'X' {
        return;
    }
    let Some(name) = scrape_entry(own_s, other_s) else {
        return;
    };
    let Some(v) = scrape_volume(e) else { return };
    let Some(entry) = sounds.entry(name) else { return };
    let script = entry.volume.start + entry.volume.range / 2.0;
    let target = (script * v, scrape_pitch(entry, v));
    if let Some(s) = scrapes.playing.iter_mut().find(|s| s.body == body) {
        s.target = target;
        s.step = (
            (target.0 - s.volume).abs() / SCRAPE_RAMP,
            (target.1 - s.pitch).abs() / SCRAPE_RAMP,
        );
        s.refreshed = now;
        s.updated = now;
        return;
    }
    // A free slot of its kind.
    let slots = if client {
        SCRAPE_SLOTS_CLIENT
    } else {
        SCRAPE_SLOTS_SERVER
    };
    if scrapes.playing.iter().filter(|s| s.client == client).count() >= slots {
        return;
    }
    if script * v <= SCRAPE_START_VOLUME {
        return;
    }
    scrapes.next_key += 1;
    let key = SoundKey(SCRAPE_KEYS | (scrapes.next_key & ((1 << 61) - 1)));
    let pitch = entry.pitch.start + entry.pitch.range / 2.0;
    control.write(SoundControl::Start(StartSound {
        key,
        entry: name.to_string(),
        follow: Some(body),
        volume: Some(script * v),
        pitch: Some(pitch),
        looping: true,
        ..default()
    }));
    scrapes.playing.push(Scrape {
        body,
        key,
        entry: name.to_string(),
        client,
        refreshed: now,
        updated: now,
        volume: script * v,
        pitch,
        target: (script * v, pitch),
        step: (0.0, 0.0),
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::map::sound::Interval;

    #[test]
    fn spec_thresholds() {
        // Spec test cases: E = 1000 does nothing (e² = 0.00416 < 1/128);
        // E = 7750 gives e² = 0.25.
        assert_eq!(scrape_volume(1000.0), None);
        assert!((scrape_volume(7750.0).unwrap() - 0.25).abs() < 1e-6);
        assert_eq!(scrape_volume(70.0), None);
        assert_eq!(scrape_volume(40000.0), Some(1.0));
        // Anything happens only above about 1370.
        assert_eq!(scrape_volume(1369.0), None);
        assert!(scrape_volume(1371.0).is_some());
    }

    #[test]
    fn stand_in_energy() {
        // 30 kg on a friction-0.8 floor (normal force m·g) sliding at
        // 100 u/s: near full volume; 2 kg debris: silent.
        let crate_ = friction_energy(0.8, 30.0 * 9.81, 100.0 * UNIT);
        assert!((13_000.0..15_000.0).contains(&crate_), "{crate_}");
        let debris = friction_energy(0.8, 2.0 * 9.81, 100.0 * UNIT);
        assert_eq!(scrape_volume(debris), None);
        // Not sliding, or no friction: nothing.
        assert_eq!(friction_energy(0.8, 300.0, 0.0), 0.0);
        assert_eq!(friction_energy(0.0, 300.0, 2.0), 0.0);
    }

    #[test]
    fn rough_or_smooth() {
        let crate_ = MapSurface {
            scrape_rough: Some("Wood_Crate.ScrapeRough".into()),
            scrape_smooth: Some("Wood_Crate.ScrapeSmooth".into()),
            rough_threshold: 0.5,
            ..default()
        };
        let glass = MapSurface {
            roughness: 0.1,
            ..default()
        };
        let concrete = MapSurface {
            roughness: 1.0,
            ..default()
        };
        assert_eq!(scrape_entry(&crate_, &glass), Some("Wood_Crate.ScrapeSmooth"));
        assert_eq!(scrape_entry(&crate_, &concrete), Some("Wood_Crate.ScrapeRough"));
        let rough_only = MapSurface {
            scrape_rough: Some("Default.ScrapeRough".into()),
            ..crate_.clone()
        };
        let rough_only = MapSurface {
            scrape_smooth: None,
            ..rough_only
        };
        assert_eq!(scrape_entry(&rough_only, &glass), Some("Default.ScrapeRough"));
    }

    #[test]
    fn pitch_follows_volume_across_the_range() {
        let entry = crate::map::MapSoundEntry {
            waves: vec![0],
            volume: Interval::fixed(0.5),
            pitch: Interval {
                start: 90.0,
                range: 20.0,
            },
            level: crate::map::SoundLevel::Db(75.0),
            channel: 4,
            dry: false,
        };
        assert_eq!(scrape_pitch(&entry, 0.0), 90.0);
        assert_eq!(scrape_pitch(&entry, 0.5), 100.0);
        assert_eq!(scrape_pitch(&entry, 1.0), 110.0);
    }
}
