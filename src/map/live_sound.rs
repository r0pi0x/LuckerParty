//! Long-lived sounds for any game (docs/plans/active/sound.md): started,
//! changed (volume, pitch) and stopped by their owner through
//! `SoundControl` messages, each under its own `SoundKey`. Map ambience
//! (Source's ambient_generic, from the logic layer) uses them.
//!
//! A clip with a loop point plays from its start, then loops from that
//! point forever; any other clip plays once and ends. A sound sits at a
//! point or follows an entity; its distance falloff and panning follow
//! the listener while it plays. The state (`LiveSounds`) is kept without
//! audio too, so headless tests read what plays.

use std::{
    collections::HashMap,
    sync::{
        Arc,
        atomic::{AtomicU32, Ordering},
    },
};

use bevy::{
    audio::{
        AddAudioSource, AudioPlayer, AudioSink, AudioSinkPlayback, ChannelCount, Decodable, PlaybackSettings,
        SampleRate, Source,
    },
    prelude::*,
    reflect::TypePath,
};

use super::hearing::{Hearing, HearingMix, Muffle};
use super::sound::{METERS_PER_UNIT, MapSoundClip, SoundBank, SoundLevel, SoundListener, distance_gain, pan};

/// Who a long-lived sound belongs to (chosen by its owner).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct SoundKey(pub u64);

/// Start, change or stop a long-lived sound.
#[derive(Message, Clone, Debug)]
pub enum SoundControl {
    /// Replaces whatever plays under the same key.
    Start(StartSound),
    /// New volume and/or pitch (percent) for a playing sound.
    Change {
        key: SoundKey,
        volume: Option<f32>,
        pitch: Option<f32>,
    },
    Stop(SoundKey),
    /// Every long-lived sound (map change, round restart).
    StopAll,
}

/// A long-lived sound to start.
#[derive(Clone, Debug)]
pub struct StartSound {
    pub key: SoundKey,
    /// A sound entry of the map's `SoundBank` (games add raw files as
    /// entries of their own).
    pub entry: String,
    /// Where (engine space); None plays it unspatialized.
    pub at: Option<Vec3>,
    /// Plays from this entity's position while it exists (else `at`).
    pub follow: Option<Entity>,
    /// Override the entry's volume, pitch (percent) and level.
    pub volume: Option<f32>,
    pub pitch: Option<f32>,
    pub level: Option<SoundLevel>,
}

/// A playing long-lived sound.
#[derive(Clone, Debug)]
pub struct LiveSound {
    pub entry: String,
    /// Index into the bank's clips.
    pub clip: usize,
    pub at: Option<Vec3>,
    pub follow: Option<Entity>,
    pub volume: f32,
    /// Percent of the recorded rate.
    pub pitch: f32,
    pub level: SoundLevel,
    /// The clip loops (it has a loop point).
    pub looping: bool,
    /// Seconds of the clip left to play at pitch 100 (one-shots).
    pub left: f32,
    audio: Option<(Entity, Arc<Gains>)>,
}

/// The long-lived sounds playing now.
#[derive(Resource, Default)]
pub struct LiveSounds {
    pub sounds: HashMap<SoundKey, LiveSound>,
}

/// Per-ear gains a playing clip reads (f32 bits), set each frame.
#[derive(Debug, Default)]
pub struct Gains {
    left: AtomicU32,
    right: AtomicU32,
}

impl Gains {
    pub fn new(left: f32, right: f32) -> Self {
        let g = Self::default();
        g.set(left, right);
        g
    }
    fn set(&self, left: f32, right: f32) {
        self.left.store(left.to_bits(), Ordering::Relaxed);
        self.right.store(right.to_bits(), Ordering::Relaxed);
    }
    fn get(&self) -> (f32, f32) {
        (
            f32::from_bits(self.left.load(Ordering::Relaxed)),
            f32::from_bits(self.right.load(Ordering::Relaxed)),
        )
    }
}

/// A clip played with live gains, looping from its loop point, through
/// the listener's hearing (`hearing::HearingMix`). Every sound plays as
/// one, so the hearing reaches them all.
#[derive(Asset, TypePath)]
pub struct LiveClip {
    clip: MapSoundClip,
    gains: Arc<Gains>,
    hearing: Arc<HearingMix>,
}

impl LiveClip {
    pub fn new(clip: MapSoundClip, gains: Arc<Gains>, hearing: Arc<HearingMix>) -> Self {
        Self { clip, gains, hearing }
    }
}

/// Stereo samples of a `LiveClip`.
pub struct LiveDecoder {
    samples: Arc<[i16]>,
    channels: usize,
    rate: SampleRate,
    loop_start: Option<usize>,
    frame: usize,
    right: Option<f32>,
    gains: Arc<Gains>,
    current: (f32, f32),
    hearing: Arc<HearingMix>,
    muffle: Muffle,
}

/// How fast the applied gains follow a change, per frame of audio
/// (about 5 ms at 44 kHz), so changes don't click.
const GAIN_SMOOTHING: f32 = 0.005;

impl Iterator for LiveDecoder {
    type Item = bevy::audio::Sample;

    fn next(&mut self) -> Option<Self::Item> {
        if let Some(r) = self.right.take() {
            return Some(r);
        }
        let frames = self.samples.len() / self.channels.max(1);
        if self.frame >= frames {
            match self.loop_start {
                Some(s) if s < frames => self.frame = s,
                _ => return None,
            }
        }
        let i = self.frame * self.channels;
        let (l, r) = if self.channels >= 2 {
            (self.samples[i], self.samples[i + 1])
        } else {
            (self.samples[i], self.samples[i])
        };
        self.frame += 1;
        let (tl, tr) = self.gains.get();
        self.current.0 += (tl - self.current.0) * GAIN_SMOOTHING;
        self.current.1 += (tr - self.current.1) * GAIN_SMOOTHING;
        let (l, r) = self.muffle.frame(
            &self.hearing,
            self.rate.get() as f32,
            l as f32 / 32768.0 * self.current.0,
            r as f32 / 32768.0 * self.current.1,
        );
        self.right = Some(r);
        Some(l)
    }
}

impl Source for LiveDecoder {
    fn current_span_len(&self) -> Option<usize> {
        None
    }
    fn channels(&self) -> ChannelCount {
        ChannelCount::new(2).unwrap()
    }
    fn sample_rate(&self) -> SampleRate {
        self.rate
    }
    fn total_duration(&self) -> Option<std::time::Duration> {
        None
    }
}

impl Decodable for LiveClip {
    type Decoder = LiveDecoder;

    fn decoder(&self) -> LiveDecoder {
        let start = self.gains.get();
        LiveDecoder {
            samples: self.clip.samples.clone(),
            channels: self.clip.channels as usize,
            rate: SampleRate::new(self.clip.rate.max(1)).unwrap(),
            loop_start: self.clip.loop_start,
            frame: 0,
            right: None,
            gains: self.gains.clone(),
            current: start,
            hearing: self.hearing.clone(),
            muffle: Muffle::default(),
        }
    }
}

/// Registered by `sound::SoundPlugin`.
pub(super) struct LiveSoundPlugin;

impl Plugin for LiveSoundPlugin {
    fn build(&self, app: &mut App) {
        app.add_message::<SoundControl>()
            .init_resource::<LiveSounds>()
            .add_systems(PostUpdate, (control, drive_audio.run_if(resource_exists::<Assets<LiveClip>>)).chain());
    }

    fn finish(&self, app: &mut App) {
        // Audio is added after the map's plugins (or not at all, headless).
        if app.is_plugin_added::<bevy::audio::AudioPlugin>() {
            app.add_audio_source::<LiveClip>();
        }
    }
}

/// Stop every long-lived sound (map change).
pub fn reset(world: &mut World) {
    let Some(mut live) = world.get_resource_mut::<LiveSounds>() else { return };
    let audio: Vec<Entity> = live.sounds.drain().filter_map(|(_, s)| s.audio.map(|a| a.0)).collect();
    for e in audio {
        if let Some(sink) = world.get::<AudioSink>(e) {
            sink.stop();
        }
        if let Ok(e) = world.get_entity_mut(e) {
            e.despawn();
        }
    }
}

fn stop(commands: &mut Commands, sinks: &Query<&AudioSink>, sound: LiveSound) {
    if let Some((e, _)) = sound.audio {
        if let Ok(sink) = sinks.get(e) {
            sink.stop();
        }
        commands.entity(e).try_despawn();
    }
}

/// Apply the owners' messages, follow entities, end finished one-shots.
#[allow(clippy::too_many_arguments)]
fn control(
    mut messages: MessageReader<SoundControl>,
    bank: Option<Res<SoundBank>>,
    mut live: ResMut<LiveSounds>,
    positions: Query<&GlobalTransform>,
    sinks: Query<&AudioSink>,
    time: Res<Time>,
    mut seed: Local<u64>,
    mut commands: Commands,
) {
    let mut unit = || {
        *seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        (*seed >> 40) as f32 / (1u64 << 24) as f32
    };
    for m in messages.read() {
        match m {
            SoundControl::Start(s) => {
                if let Some(old) = live.sounds.remove(&s.key) {
                    stop(&mut commands, &sinks, old);
                }
                let Some(bank) = &bank else { continue };
                let Some(entry) = bank.0.entry(&s.entry) else {
                    debug!("sound: no entry '{}'", s.entry);
                    continue;
                };
                if entry.waves.is_empty() {
                    continue;
                }
                let n = entry.waves.len();
                let clip_index = entry.waves[((unit() * n as f32) as usize).min(n - 1)];
                let clip = &bank.0.clips[clip_index];
                let volume = s.volume.unwrap_or_else(|| entry.volume.draw(unit())).clamp(0.0, 1.0);
                let pitch = s.pitch.unwrap_or_else(|| entry.pitch.draw(unit()).trunc());
                let level = s.level.unwrap_or(entry.level);
                let frames = clip.samples.len() / clip.channels.max(1) as usize;
                let looping = clip.loop_start.is_some_and(|l| l < frames);
                info!(
                    "sound: start '{}' volume {volume:.2} pitch {pitch} level {level:?}{}",
                    s.entry,
                    if looping { ", looping" } else { "" }
                );
                live.sounds.insert(
                    s.key,
                    LiveSound {
                        entry: s.entry.clone(),
                        clip: clip_index,
                        at: s.at,
                        follow: s.follow,
                        volume,
                        pitch,
                        level,
                        looping,
                        left: frames as f32 / clip.rate.max(1) as f32,
                        audio: None,
                    },
                );
            }
            SoundControl::Change { key, volume, pitch } => {
                if let Some(s) = live.sounds.get_mut(key) {
                    if let Some(v) = volume {
                        s.volume = v.clamp(0.0, 1.0);
                    }
                    if let Some(p) = pitch {
                        s.pitch = *p;
                    }
                }
            }
            SoundControl::Stop(key) => {
                if let Some(old) = live.sounds.remove(key) {
                    stop(&mut commands, &sinks, old);
                }
            }
            SoundControl::StopAll => {
                for (_, old) in live.sounds.drain() {
                    stop(&mut commands, &sinks, old);
                }
            }
        }
    }
    let dt = time.delta_secs();
    let ended: Vec<SoundKey> = live
        .sounds
        .iter_mut()
        .filter_map(|(k, s)| {
            if let Some(p) = s.follow.and_then(|f| positions.get(f).ok()) {
                s.at = Some(p.translation());
            }
            if !s.looping {
                s.left -= dt * s.pitch.clamp(1.0, 255.0) / 100.0;
            }
            (!s.looping && s.left <= 0.0).then_some(*k)
        })
        .collect();
    for k in ended {
        if let Some(old) = live.sounds.remove(&k) {
            stop(&mut commands, &sinks, old);
        }
    }
}

/// Start audio for new sounds; set gains (volume, distance, panning) and
/// speed (pitch) of the playing ones.
fn drive_audio(
    mut live: ResMut<LiveSounds>,
    bank: Option<Res<SoundBank>>,
    listener: Query<&GlobalTransform, With<SoundListener>>,
    mut sinks: Query<&mut AudioSink>,
    mut clips: ResMut<Assets<LiveClip>>,
    global: Option<Res<bevy::audio::GlobalVolume>>,
    hearing: Res<Hearing>,
    mut commands: Commands,
) {
    let Some(bank) = bank else { return };
    // Bevy applies the master volume when a sink starts; follow later
    // changes to it (the clip's own gains carry the rest).
    let master = global.map_or(1.0, |g| g.volume.to_linear());
    let ear = listener.iter().next();
    for s in live.sounds.values_mut() {
        let (left, right) = match (s.at, ear) {
            (Some(at), Some(ear)) if !matches!(s.level, SoundLevel::Db(l) if l <= 0.0) => {
                let to = at - ear.translation();
                let g = distance_gain(s.level, to.length() / METERS_PER_UNIT) * s.volume;
                let (pl, pr) = pan(to.normalize_or_zero(), ear.right().as_vec3());
                (g * pl, g * pr)
            }
            _ => (s.volume, s.volume),
        };
        let speed = s.pitch.clamp(1.0, 255.0) / 100.0;
        match &s.audio {
            Some((e, gains)) => {
                gains.set(left, right);
                if let Ok(mut sink) = sinks.get_mut(*e) {
                    if (sink.speed() - speed).abs() > 1e-4 {
                        sink.set_speed(speed);
                    }
                    if (sink.volume().to_linear() - master).abs() > 1e-4 {
                        sink.set_volume(bevy::audio::Volume::Linear(master));
                    }
                }
            }
            None => {
                let Some(clip) = bank.0.clips.get(s.clip) else { continue };
                let gains = Arc::new(Gains::new(left, right));
                let handle = clips.add(LiveClip::new(clip.clone(), gains.clone(), hearing.mix.clone()));
                let e = commands
                    .spawn((AudioPlayer(handle), PlaybackSettings::DESPAWN.with_speed(speed)))
                    .id();
                s.audio = Some((e, gains));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn clip(frames: usize, loop_start: Option<usize>) -> MapSoundClip {
        MapSoundClip {
            rate: 11025,
            channels: 1,
            samples: (0..frames as i16).map(|i| i * 100).collect::<Vec<_>>().into(),
            loop_start,
        }
    }

    fn decoder(c: MapSoundClip) -> LiveDecoder {
        LiveClip::new(c, Arc::new(Gains::new(1.0, 0.5)), Arc::default()).decoder()
    }

    #[test]
    fn one_shot_plays_once_in_stereo() {
        let out: Vec<f32> = decoder(clip(4, None)).collect();
        assert_eq!(out.len(), 8);
        assert!((out[2] - 100.0 / 32768.0).abs() < 1e-6);
        assert!((out[3] - 50.0 / 32768.0).abs() < 1e-6);
    }

    #[test]
    fn loop_plays_intro_then_repeats_from_the_loop_point() {
        let left: Vec<f32> = decoder(clip(4, Some(2))).step_by(2).take(8).collect();
        let frames: Vec<i32> = left.iter().map(|v| (v * 32768.0 / 100.0).round() as i32).collect();
        assert_eq!(frames, [0, 1, 2, 3, 2, 3, 2, 3]);
    }
}
