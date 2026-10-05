//! Soundscape playback (specs/cs_source/sounds.md 6, "Soundscapes"): pick
//! the soundscape for the listener, fade its loops in over 3 s while the
//! previous one's fade out, and play its random one-shots. Runs only with
//! audio.

use bevy::{
    audio::{AudioPlayer, AudioSink, AudioSinkPlayback, AudioSource, PlaybackSettings, Volume},
    prelude::*,
};

use super::sound::{
    METERS_PER_UNIT, MapSoundClip, ScapePosition, SoundBank, SoundLevel, SoundListener, distance_gain, pan, stereo_wav,
};

/// Seconds for a loop's volume to move by 1.0 (soundscape_fadetime).
const FADE_TIME: f32 = 3.0;
/// Random sounds with "position random" play this far from the eye, units.
const RANDOM_DISTANCE: f32 = 36.0;

pub struct SoundscapePlugin;

impl Plugin for SoundscapePlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<ScapeState>().add_systems(
            PostUpdate,
            (select, fade_loops, play_randoms)
                .chain()
                .run_if(resource_exists::<Assets<AudioSource>>),
        );
    }
}

/// Which soundscape plays and the state of its random sounds.
#[derive(Resource, Default)]
pub struct ScapeState {
    /// (soundscape, positions) now playing, and what selected it.
    pub current: Option<(usize, Source)>,
    /// Zones the listener is inside, most recently entered first.
    inside: Vec<usize>,
    generation: u32,
    randoms: Vec<(usize, f64)>,
    positions: Vec<Option<Vec3>>,
    seed: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Source {
    Zone(usize),
    Emitter(usize),
}

impl ScapeState {
    fn unit(&mut self) -> f32 {
        self.seed = self
            .seed
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        (self.seed >> 40) as f32 / (1u64 << 24) as f32
    }
}

/// A playing soundscape loop.
#[derive(Component)]
struct ScapeLoop {
    clip: usize,
    pitch: f32,
    at: Option<Vec3>,
    level: f32,
    volume: f32,
    target: f32,
}

fn select(
    bank: Option<Res<SoundBank>>,
    listener: Query<&GlobalTransform, With<SoundListener>>,
    mut state: ResMut<ScapeState>,
    mut loops: Query<(Entity, &mut ScapeLoop)>,
    mut sources: ResMut<Assets<AudioSource>>,
    mut commands: Commands,
    time: Res<Time>,
) {
    let (Some(bank), Some(ear)) = (bank, listener.iter().next()) else {
        return;
    };
    let sounds = &bank.0;
    let ear = ear.translation();
    // Zones: entering one makes it current; leaving all keeps the last.
    let containing: Vec<usize> = sounds
        .soundscape_zones
        .iter()
        .enumerate()
        .filter(|(_, z)| ear.cmpge(z.min).all() && ear.cmple(z.max).all())
        .map(|(i, _)| i)
        .collect();
    state.inside.retain(|i| containing.contains(i));
    for i in containing {
        if !state.inside.contains(&i) {
            state.inside.insert(0, i);
        }
    }
    let wanted = if let Some(&z) = state.inside.first() {
        Some(Source::Zone(z))
    } else {
        // Emitters: the current one stays until a closer one in range
        // appears (visibility is not checked yet).
        let in_range = |i: usize| {
            let e = &sounds.soundscape_emitters[i];
            e.radius.is_none_or(|r| e.at.distance(ear) < r)
        };
        let dist = |i: usize| sounds.soundscape_emitters[i].at.distance(ear);
        let current = match state.current {
            Some((_, Source::Emitter(i))) if in_range(i) => Some(i),
            _ => None,
        };
        let best = (0..sounds.soundscape_emitters.len())
            .filter(|&i| in_range(i))
            .min_by(|&a, &b| dist(a).total_cmp(&dist(b)));
        match (current, best) {
            (Some(c), Some(b)) if dist(b) < dist(c) => Some(Source::Emitter(b)),
            (Some(c), _) => Some(Source::Emitter(c)),
            (None, Some(b)) => Some(Source::Emitter(b)),
            (None, None) => state.current.map(|c| c.1),
        }
    };
    let Some(source) = wanted else { return };
    if state.current.is_some_and(|c| c.1 == source) {
        return;
    }
    let (scape, positions) = match source {
        Source::Zone(i) => (
            sounds.soundscape_zones[i].scape,
            sounds.soundscape_zones[i].positions.clone(),
        ),
        Source::Emitter(i) => (
            sounds.soundscape_emitters[i].scape,
            sounds.soundscape_emitters[i].positions.clone(),
        ),
    };
    // Same soundscape from another zone: nothing changes but positions.
    if state.current.is_some_and(|c| c.0 == scape) && state.positions == positions {
        state.current = Some((scape, source));
        return;
    }
    info!("soundscape: {}", sounds.soundscapes[scape].name);
    state.current = Some((scape, source));
    state.generation += 1;
    state.positions = positions.clone();
    let now = time.elapsed_secs_f64();
    let def = &sounds.soundscapes[scape];

    // Old loops fade out unless the new soundscape reuses them.
    let mut old: Vec<(Entity, Mut<ScapeLoop>)> = loops.iter_mut().collect();
    for (_, l) in old.iter_mut() {
        l.target = 0.0;
    }
    for l in &def.loops {
        let at = match l.position {
            None => None,
            Some(n) => match positions.get(n).copied().flatten() {
                Some(p) => Some(p),
                None => continue,
            },
        };
        let volume = l.volume.draw(state.unit());
        let pitch = l.pitch.draw(state.unit());
        if volume <= 0.0 {
            continue;
        }
        let reuse = old.iter_mut().find(|(_, o)| {
            o.clip == l.clip
                && o.pitch == pitch
                && match (o.at, at) {
                    (None, None) => true,
                    (Some(a), Some(b)) => a.distance(b) < 0.1 * METERS_PER_UNIT,
                    _ => false,
                }
        });
        if let Some((_, o)) = reuse {
            o.target = volume;
            continue;
        }
        let clip = looped(&sounds.clips[l.clip]);
        let handle = sources.add(AudioSource {
            bytes: stereo_wav(&clip, 1.0, 1.0).into(),
        });
        let start = if at.is_some() { 0.05 } else { 0.0 };
        commands.spawn((
            AudioPlayer(handle),
            PlaybackSettings::LOOP
                .with_volume(Volume::Linear(0.0))
                .with_speed(pitch / 100.0),
            ScapeLoop {
                clip: l.clip,
                pitch,
                at,
                level: l.level,
                volume: start,
                target: volume,
            },
        ));
    }
    state.randoms.clear();
    for (i, r) in def.randoms.iter().enumerate() {
        let wait = 0.5 * r.time.draw(state.unit()) as f64;
        state.randoms.push((i, now + wait));
    }
}

/// The part of a clip that loops: from its loop point to the end.
fn looped(clip: &MapSoundClip) -> MapSoundClip {
    let Some(start) = clip.loop_start else {
        return clip.clone();
    };
    let from = (start * clip.channels as usize).min(clip.samples.len());
    MapSoundClip {
        samples: clip.samples[from..].into(),
        loop_start: None,
        ..clip.clone()
    }
}

fn fade_loops(
    mut loops: Query<(Entity, &mut ScapeLoop, Option<&mut AudioSink>)>,
    listener: Query<&GlobalTransform, With<SoundListener>>,
    time: Res<Time>,
    mut commands: Commands,
) {
    let ear = listener.iter().next().map(|l| l.translation());
    let step = time.delta_secs() / FADE_TIME;
    for (e, mut l, sink) in &mut loops {
        l.volume += (l.target - l.volume).clamp(-step, step);
        if l.target == 0.0 && l.volume <= 0.0 {
            commands.entity(e).despawn();
            continue;
        }
        let gain = match (l.at, ear) {
            (Some(at), Some(ear)) => distance_gain(SoundLevel::Db(l.level), at.distance(ear) / METERS_PER_UNIT),
            _ => 1.0,
        };
        if let Some(mut sink) = sink {
            sink.set_volume(Volume::Linear(l.volume * gain));
        }
    }
}

fn play_randoms(
    bank: Option<Res<SoundBank>>,
    listener: Query<&GlobalTransform, With<SoundListener>>,
    mut state: ResMut<ScapeState>,
    mut sources: ResMut<Assets<AudioSource>>,
    mut commands: Commands,
    time: Res<Time>,
) {
    let (Some(bank), Some(ear)) = (bank, listener.iter().next()) else {
        return;
    };
    let Some((scape, _)) = state.current else { return };
    let now = time.elapsed_secs_f64();
    let def = &bank.0.soundscapes[scape];
    for k in 0..state.randoms.len() {
        let (i, due) = state.randoms[k];
        if now < due {
            continue;
        }
        let r = &def.randoms[i];
        let wait = r.time.draw(state.unit()) as f64;
        state.randoms[k].1 = now + wait.max(0.1);
        if r.clips.is_empty() {
            continue;
        }
        let pick = ((state.unit() * r.clips.len() as f32) as usize).min(r.clips.len() - 1);
        let volume = r.volume.draw(state.unit()).clamp(0.0, 1.0);
        let pitch = r.pitch.draw(state.unit()).trunc().clamp(1.0, 255.0);
        let level = r.level.draw(state.unit()).trunc();
        let at = match r.position {
            ScapePosition::Ambient => None,
            ScapePosition::Index(n) => match state.positions.get(n).copied().flatten() {
                Some(p) => Some(p),
                None => continue,
            },
            ScapePosition::Random => {
                let theta = (state.unit() * 2.0 - 1.0) * std::f32::consts::PI;
                let dir = ear.right() * theta.cos() + ear.forward() * theta.sin();
                Some(ear.translation() + dir * RANDOM_DISTANCE * METERS_PER_UNIT)
            }
        };
        let (left, right) = match at {
            Some(at) => {
                let to = at - ear.translation();
                let g = distance_gain(SoundLevel::Db(level), to.length() / METERS_PER_UNIT) * volume;
                let (pl, pr) = pan(to.normalize_or_zero(), ear.right().as_vec3());
                (g * pl, g * pr)
            }
            None => (volume, volume),
        };
        if left.max(right) < 1e-3 {
            continue;
        }
        let handle = sources.add(AudioSource {
            bytes: stereo_wav(&bank.0.clips[r.clips[pick]], left, right).into(),
        });
        commands.spawn((AudioPlayer(handle), PlaybackSettings::DESPAWN.with_speed(pitch / 100.0)));
    }
}
