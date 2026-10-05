//! Sounds for any game (docs/plans/active/sound.md): the map carries named
//! sound entries and decoded clips; game code writes `PlaySound`; playback
//! resolves the entry (random wave, drawn volume, pitch and level), applies
//! the distance model and left/right panning toward the listener, and
//! plays it, pitch as playback speed. A new sound on the same (source
//! entity, channel) replaces the one playing there.

use std::{collections::HashMap, sync::Arc};

use bevy::{
    audio::{AudioPlayer, AudioSink, AudioSinkPlayback, AudioSource, PlaybackSettings, Volume},
    prelude::*,
};

/// Decoded audio: interleaved 16-bit samples.
#[derive(Clone, Debug)]
pub struct MapSoundClip {
    pub rate: u32,
    pub channels: u16,
    pub samples: Arc<[i16]>,
    /// Loop from this frame to the end, when the file says so.
    pub loop_start: Option<usize>,
}

/// How a sound falls off with distance.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum SoundLevel {
    /// Source sound level (0..255 "dB"); 0 is heard everywhere.
    Db(f32),
    /// Linear falloff, silent beyond 1000 / attenuation units (GoldSrc
    /// style, Source's "CompatibilityAttenuation").
    Attenuation(f32),
}

/// A value drawn uniformly from start..start + range.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Interval {
    pub start: f32,
    pub range: f32,
}

impl Interval {
    pub const fn fixed(v: f32) -> Self {
        Self { start: v, range: 0.0 }
    }
    fn draw(&self, unit: f32) -> f32 {
        self.start + self.range * unit
    }
}

/// A named sound (Source: a sound script entry).
#[derive(Clone, Debug)]
pub struct MapSoundEntry {
    /// Indices into `MapData::sound_clips`; one is picked at random.
    pub waves: Vec<usize>,
    pub volume: Interval,
    /// Percent of the recorded rate.
    pub pitch: Interval,
    pub level: SoundLevel,
    pub channel: u8,
}

/// Ground properties that sounds use (Source surface properties).
#[derive(Clone, Debug, Default)]
pub struct MapSurface {
    pub step_left: Option<String>,
    pub step_right: Option<String>,
    /// Source game material letter ('C' concrete, 'D' dirt, ...).
    pub game_material: char,
}

/// Sound data the map carries.
#[derive(Clone, Debug, Default)]
pub struct MapSounds {
    /// Lower-case entry names.
    pub entries: HashMap<String, MapSoundEntry>,
    pub clips: Vec<MapSoundClip>,
    /// Lower-case surface names.
    pub surfaces: HashMap<String, MapSurface>,
}

impl MapSounds {
    pub fn entry(&self, name: &str) -> Option<&MapSoundEntry> {
        self.entries.get(&name.to_lowercase())
    }
    pub fn surface(&self, name: &str) -> Option<&MapSurface> {
        self.surfaces.get(&name.to_lowercase())
    }
}

/// Play a named sound.
#[derive(Message, Clone, Debug)]
pub struct PlaySound {
    pub entry: String,
    /// Where (engine space); None plays it unspatialized.
    pub at: Option<Vec3>,
    /// Replaces the entry's volume (footsteps compute their own).
    pub volume: Option<f32>,
    /// Who makes it; with `channel`, a new sound replaces the old one.
    pub source: Option<Entity>,
    pub channel: Option<u8>,
}

impl PlaySound {
    pub fn at(entry: impl Into<String>, at: Vec3) -> Self {
        Self {
            entry: entry.into(),
            at: Some(at),
            volume: None,
            source: None,
            channel: None,
        }
    }
}

/// The loaded map's sounds.
#[derive(Resource, Clone)]
pub struct SoundBank(pub Arc<MapSounds>);

/// Who's listening (the camera the player sees through).
#[derive(Component)]
pub struct SoundListener;

/// A playing sound: who made it on which channel.
#[derive(Component)]
struct Playing {
    source: Option<Entity>,
    channel: Option<u8>,
}

const METERS_PER_UNIT: f32 = 0.0254;
/// Distances closer than this don't get louder (units).
const MIN_DISTANCE: f32 = 36.0;

/// Gain at `units` from the source (spec open question 1: hypotheses H1
/// for sound levels, H2 for attenuation entries, until measured).
pub fn distance_gain(level: SoundLevel, units: f32) -> f32 {
    match level {
        SoundLevel::Db(l) if l <= 0.0 => 1.0,
        SoundLevel::Db(l) => (10f32.powf((l - 60.0) / 20.0) * MIN_DISTANCE / units.max(MIN_DISTANCE)).min(1.0),
        SoundLevel::Attenuation(a) if a <= 0.0 => 1.0,
        SoundLevel::Attenuation(a) => (1.0 - units * a / 1000.0).clamp(0.0, 1.0),
    }
}

/// Per-ear gains for a source in `dir` (unit, listener to source) given the
/// listener's right vector: full in the near ear, fading in the far one.
pub fn pan(dir: Vec3, right: Vec3) -> (f32, f32) {
    let d = dir.dot(right).clamp(-1.0, 1.0);
    ((1.0 - d).min(1.0), (1.0 + d).min(1.0))
}

pub struct SoundPlugin;

impl Plugin for SoundPlugin {
    fn build(&self, app: &mut App) {
        app.add_message::<PlaySound>()
            .add_systems(PostUpdate, play_sounds.run_if(resource_exists::<Assets<AudioSource>>));
    }
}

#[allow(clippy::too_many_arguments)]
fn play_sounds(
    mut commands: Commands,
    mut messages: MessageReader<PlaySound>,
    bank: Option<Res<SoundBank>>,
    listeners: Query<&GlobalTransform, With<SoundListener>>,
    playing: Query<(Entity, &Playing, Option<&AudioSink>)>,
    mut sources: ResMut<Assets<AudioSource>>,
    mut seed: Local<u64>,
) {
    let Some(bank) = bank else {
        messages.clear();
        return;
    };
    let listener = listeners.iter().next().copied();
    let mut unit = || {
        *seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        (*seed >> 40) as f32 / (1u64 << 24) as f32
    };
    for m in messages.read() {
        let Some(entry) = bank.0.entry(&m.entry) else {
            continue;
        };
        if entry.waves.is_empty() {
            continue;
        }
        let wave = entry.waves[((unit() * entry.waves.len() as f32) as usize).min(entry.waves.len() - 1)];
        let clip = &bank.0.clips[wave];
        let volume = m.volume.unwrap_or_else(|| entry.volume.draw(unit())).clamp(0.0, 1.0);
        let pitch = entry.pitch.draw(unit()).round().clamp(1.0, 255.0);
        let level = match entry.level {
            SoundLevel::Db(l) => SoundLevel::Db(l.round()),
            a => a,
        };
        let (left, right) = match (m.at, listener) {
            (Some(at), Some(l)) => {
                let to = at - l.translation();
                let g = distance_gain(level, to.length() / METERS_PER_UNIT) * volume;
                let (pl, pr) = pan(to.normalize_or_zero(), l.right().as_vec3());
                (g * pl, g * pr)
            }
            _ => (volume, volume),
        };
        if left.max(right) < 1e-3 {
            continue;
        }
        // Replace whatever this source plays on this channel.
        if let (Some(source), Some(channel)) = (m.source, m.channel) {
            for (e, p, sink) in &playing {
                if p.source == Some(source) && p.channel == Some(channel) {
                    if let Some(sink) = sink {
                        sink.stop();
                    }
                    commands.entity(e).despawn();
                }
            }
        }
        let bytes = stereo_wav(clip, left, right);
        let handle = sources.add(AudioSource { bytes: bytes.into() });
        commands.spawn((
            AudioPlayer(handle),
            PlaybackSettings::DESPAWN
                .with_volume(Volume::Linear(1.0))
                .with_speed(pitch / 100.0),
            Playing {
                source: m.source,
                channel: m.channel,
            },
        ));
    }
}

/// A clip as a 16-bit stereo PCM WAV, each channel scaled (for panning and
/// distance), for an audio backend that reads WAV.
pub fn stereo_wav(clip: &MapSoundClip, left: f32, right: f32) -> Vec<u8> {
    let frames = clip.samples.len() / clip.channels as usize;
    let mut out = Vec::with_capacity(44 + frames * 4);
    let data_len = (frames * 4) as u32;
    out.extend(b"RIFF");
    out.extend((36 + data_len).to_le_bytes());
    out.extend(b"WAVEfmt ");
    out.extend(16u32.to_le_bytes());
    out.extend(1u16.to_le_bytes());
    out.extend(2u16.to_le_bytes());
    out.extend(clip.rate.to_le_bytes());
    out.extend((clip.rate * 4).to_le_bytes());
    out.extend(4u16.to_le_bytes());
    out.extend(16u16.to_le_bytes());
    out.extend(b"data");
    out.extend(data_len.to_le_bytes());
    let scale = |s: i16, g: f32| ((s as f32 * g).clamp(-32768.0, 32767.0) as i16).to_le_bytes();
    for f in 0..frames {
        let (l, r) = if clip.channels >= 2 {
            (clip.samples[f * 2], clip.samples[f * 2 + 1])
        } else {
            (clip.samples[f], clip.samples[f])
        };
        out.extend(scale(l, left));
        out.extend(scale(r, right));
    }
    out
}
