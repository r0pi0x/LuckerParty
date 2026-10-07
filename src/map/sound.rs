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
    pub fn draw(&self, unit: f32) -> f32 {
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
    pub bullet_impact: Option<String>,
    pub impact_soft: Option<String>,
    pub impact_hard: Option<String>,
    /// How hard it sounds when hit (audiohardnessfactor).
    pub hardness: f32,
    /// Hitting something softer than this plays the soft impact.
    pub hard_threshold: f32,
    /// Impacts slower than this play the soft sound (0: no rule), u/s.
    pub hard_min_velocity: f32,
}

/// Sound data the map carries.
#[derive(Clone, Debug, Default)]
pub struct MapSounds {
    /// Lower-case entry names.
    pub entries: HashMap<String, MapSoundEntry>,
    pub clips: Vec<MapSoundClip>,
    /// Lower-case surface names.
    pub surfaces: HashMap<String, MapSurface>,
    /// Ambience: named soundscapes and where each one applies.
    pub soundscapes: Vec<Soundscape>,
    pub soundscape_zones: Vec<SoundscapeZone>,
    pub soundscape_emitters: Vec<SoundscapeEmitter>,
}

/// A map's background ambience (Source soundscapes): loops that fade in
/// while it applies, and one-shots at random intervals.
#[derive(Clone, Debug, Default)]
pub struct Soundscape {
    pub name: String,
    pub loops: Vec<ScapeLoop>,
    pub randoms: Vec<ScapeRandom>,
}

#[derive(Clone, Debug)]
pub struct ScapeLoop {
    /// Index into `clips`.
    pub clip: usize,
    pub volume: Interval,
    /// Percent.
    pub pitch: Interval,
    /// Sound level in dB (0: no falloff).
    pub level: f32,
    /// Index into the applying zone's or emitter's positions; None plays
    /// everywhere, unspatialized.
    pub position: Option<usize>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScapePosition {
    Ambient,
    Index(usize),
    /// Near the listener in a random direction.
    Random,
}

#[derive(Clone, Debug)]
pub struct ScapeRandom {
    pub clips: Vec<usize>,
    /// Seconds between plays.
    pub time: Interval,
    pub volume: Interval,
    pub pitch: Interval,
    pub level: Interval,
    pub position: ScapePosition,
}

/// A box that selects a soundscape for a listener inside it (Source
/// trigger_soundscape); the most recently entered one wins.
#[derive(Clone, Debug)]
pub struct SoundscapeZone {
    pub min: Vec3,
    pub max: Vec3,
    pub scape: usize,
    pub positions: Vec<Option<Vec3>>,
}

/// A point that selects a soundscape within its radius (env_soundscape).
#[derive(Clone, Debug)]
pub struct SoundscapeEmitter {
    pub at: Vec3,
    /// Meters; None: unlimited.
    pub radius: Option<f32>,
    pub scape: usize,
    pub positions: Vec<Option<Vec3>>,
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

    /// Unspatialized (announcer, interface).
    pub fn ui(entry: impl Into<String>) -> Self {
        Self {
            at: None,
            ..Self::at(entry, Vec3::ZERO)
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

pub(super) const METERS_PER_UNIT: f32 = 0.0254;
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
            .add_systems(PostUpdate, play_sounds.run_if(resource_exists::<Assets<AudioSource>>))
            .add_plugins((super::soundscape::SoundscapePlugin, super::live_sound::LiveSoundPlugin));
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

/// World triangles tagged with their surface, on a horizontal grid, to
/// find the surface under a point (footsteps).
#[derive(Resource, Default)]
pub struct SurfaceGrid {
    names: Vec<String>,
    /// Triangle corners and surface index.
    tris: Vec<([Vec3; 3], u16)>,
    grid: HashMap<(i32, i32), Vec<u32>>,
}

impl SurfaceGrid {
    const CELL: f32 = 2.0;

    pub fn new(data: &super::MapData) -> Self {
        let mut me = Self::default();
        let mut index: HashMap<String, u16> = HashMap::new();
        for m in data
            .meshes
            .iter()
            .filter(|m| !m.skybox && m.entity.is_none() && !m.material.starts_with("decal:"))
        {
            let Some(surface) = &m.surface else { continue };
            let id = *index.entry(surface.clone()).or_insert_with(|| {
                me.names.push(surface.clone());
                (me.names.len() - 1) as u16
            });
            for t in m.indices.as_chunks::<3>().0 {
                let tri = t.map(|i| Vec3::from(m.positions[i as usize]));
                let i = me.tris.len() as u32;
                let lo = tri[0].min(tri[1]).min(tri[2]);
                let hi = tri[0].max(tri[1]).max(tri[2]);
                for gx in (lo.x / Self::CELL).floor() as i32..=(hi.x / Self::CELL).floor() as i32 {
                    for gz in (lo.z / Self::CELL).floor() as i32..=(hi.z / Self::CELL).floor() as i32 {
                        me.grid.entry((gx, gz)).or_default().push(i);
                    }
                }
                me.tris.push((tri, id));
            }
        }
        me
    }

    /// The surface of the highest upward-facing triangle under `p` (engine
    /// space), at most `reach` meters below it.
    pub fn below(&self, p: Vec3, reach: f32) -> Option<&str> {
        let key = ((p.x / Self::CELL).floor() as i32, (p.z / Self::CELL).floor() as i32);
        let mut best: Option<(f32, u16)> = None;
        for &i in self.grid.get(&key)? {
            let ([a, b, c], id) = self.tris[i as usize];
            let n = (b - a).cross(c - a);
            if n.y <= 1e-6 {
                continue;
            }
            // Barycentric in the horizontal plane.
            let (a2, b2, c2, q) = (a.xz(), b.xz(), c.xz(), p.xz());
            let area = (b2 - a2).perp_dot(c2 - a2);
            if area.abs() < 1e-9 {
                continue;
            }
            let w1 = (q - a2).perp_dot(c2 - a2) / area;
            let w2 = (b2 - a2).perp_dot(q - a2) / area;
            if w1 < 0.0 || w2 < 0.0 || w1 + w2 > 1.0 {
                continue;
            }
            let y = a.y + (b.y - a.y) * w1 + (c.y - a.y) * w2;
            if y <= p.y + 0.01 && p.y - y <= reach && best.is_none_or(|(by, _)| y > by) {
                best = Some((y, id));
            }
        }
        best.map(|(_, id)| self.names[id as usize].as_str())
    }

    /// The surface of the triangle nearest to `p` within `reach` meters
    /// (bullet impacts on walls, contacts).
    pub fn nearest(&self, p: Vec3, reach: f32) -> Option<&str> {
        let lo = ((p.xz() - reach) / Self::CELL).floor().as_ivec2();
        let hi = ((p.xz() + reach) / Self::CELL).floor().as_ivec2();
        let mut best: Option<(f32, u16)> = None;
        for gx in lo.x..=hi.x {
            for gz in lo.y..=hi.y {
                for &i in self.grid.get(&(gx, gz)).into_iter().flatten() {
                    let (tri, id) = self.tris[i as usize];
                    let d = closest_on_triangle(p, tri).distance_squared(p);
                    if d <= reach * reach && best.is_none_or(|(bd, _)| d < bd) {
                        best = Some((d, id));
                    }
                }
            }
        }
        best.map(|(_, id)| self.names[id as usize].as_str())
    }
}

/// The point of triangle `t` closest to `p`.
fn closest_on_triangle(p: Vec3, [a, b, c]: [Vec3; 3]) -> Vec3 {
    let (ab, ac, ap) = (b - a, c - a, p - a);
    let (d1, d2) = (ab.dot(ap), ac.dot(ap));
    if d1 <= 0.0 && d2 <= 0.0 {
        return a;
    }
    let bp = p - b;
    let (d3, d4) = (ab.dot(bp), ac.dot(bp));
    if d3 >= 0.0 && d4 <= d3 {
        return b;
    }
    let vc = d1 * d4 - d3 * d2;
    if vc <= 0.0 && d1 >= 0.0 && d3 <= 0.0 {
        return a + ab * (d1 / (d1 - d3));
    }
    let cp = p - c;
    let (d5, d6) = (ab.dot(cp), ac.dot(cp));
    if d6 >= 0.0 && d5 <= d6 {
        return c;
    }
    let vb = d5 * d2 - d1 * d6;
    if vb <= 0.0 && d2 >= 0.0 && d6 <= 0.0 {
        return a + ac * (d2 / (d2 - d6));
    }
    let va = d3 * d6 - d5 * d4;
    if va <= 0.0 && d4 - d3 >= 0.0 && d5 - d6 >= 0.0 {
        return b + (c - b) * ((d4 - d3) / ((d4 - d3) + (d5 - d6)));
    }
    let denom = 1.0 / (va + vb + vc);
    a + ab * (vb * denom) + ac * (vc * denom)
}
