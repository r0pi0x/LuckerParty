//! Room DSP for any game (Source's soundscape "dsp" presets,
//! specs/cs_source/sounds.md "Loudness and panning" and "Soundscapes"):
//! the current soundscape names a room preset; world sounds send a copy of
//! what they play to one shared reverb bus, which adds the room's
//! reverberation and echo to the mix.
//!
//! Source's preset parameters are engine-side and unpublished, so
//! `PRESETS` is ours: one row per Source index (0-28, the names in the
//! soundscape manifest's comments), derived from the names (docs/tech-debt.md).
//!
//! The bus (`RoomBus`) is one long-lived Bevy audio source. Every playing
//! clip (`live_sound::LiveDecoder`) adds its send into a shared ring of
//! samples ahead of the bus's read position; Bevy's mixer pulls all
//! sources one sample at a time on the audio thread, so writers and the
//! reader stay within a few samples of each other. The bus runs a
//! Freeverb-style reverb (8 combs, 4 all-passes per ear), a predelay, a
//! low-pass on the send and one feedback echo, then the listener's hearing
//! (`hearing::Muffle`). Headless, nothing plays and the selection
//! (`RoomDsp::preset`) is still kept.

use std::sync::{
    Arc,
    atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering},
};

use bevy::{
    audio::{AudioPlayer, AudioSink, AudioSinkPlayback, ChannelCount, Decodable, PlaybackSettings, SampleRate, Source},
    prelude::*,
    reflect::TypePath,
};

use super::hearing::{Hearing, HearingMix, Muffle, low_pass_alpha};

/// One room preset (all values ours, unmeasured).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RoomPreset {
    pub name: &'static str,
    /// Reverberation time to -60 dB, seconds.
    pub decay: f32,
    /// Delay before the reverberation starts, ms.
    pub predelay: f32,
    /// High-frequency loss per reflection, 0 (bright) to 1 (dull).
    pub damping: f32,
    /// Low-pass on what enters the room, Hz.
    pub cutoff: f32,
    /// Reverberation level (Freeverb's wet, 1/3 is very wet).
    pub wet: f32,
    /// A distinct repeating echo: delay (ms, 0 none), feedback and level.
    pub echo: f32,
    pub echo_feedback: f32,
    pub echo_gain: f32,
}

const fn room(name: &'static str, decay: f32, predelay: f32, damping: f32, cutoff: f32, wet: f32) -> RoomPreset {
    RoomPreset {
        name,
        decay,
        predelay,
        damping,
        cutoff,
        wet,
        echo: 0.0,
        echo_feedback: 0.0,
        echo_gain: 0.0,
    }
}

const fn echo(r: RoomPreset, ms: f32, feedback: f32, gain: f32) -> RoomPreset {
    RoomPreset {
        echo: ms,
        echo_feedback: feedback,
        echo_gain: gain,
        ..r
    }
}

/// Room presets by Source's soundscape "dsp" index (names from CS:S's
/// soundscape manifest comments). Parameters are ours: decay grows with
/// the S/M/L size, metal and bright rooms damp little, water rooms are
/// dull, tunnels, big rooms and caverns add a repeating echo whose delay
/// grows with the size, the "weirdo" rooms a strong short one.
pub const PRESETS: [RoomPreset; 29] = [
    room("Normal (off)", 0.0, 0.0, 0.5, 8000.0, 0.0),
    room("Generic", 0.9, 8.0, 0.5, 6000.0, 0.10),
    echo(room("Metal S", 0.6, 4.0, 0.15, 9000.0, 0.16), 12.0, 0.35, 0.10),
    echo(room("Metal M", 1.0, 8.0, 0.15, 9000.0, 0.18), 20.0, 0.35, 0.10),
    echo(room("Metal L", 1.6, 14.0, 0.15, 9000.0, 0.20), 32.0, 0.35, 0.10),
    echo(room("Tunnel S", 1.0, 10.0, 0.35, 5000.0, 0.18), 45.0, 0.30, 0.14),
    echo(room("Tunnel M", 1.6, 18.0, 0.35, 5000.0, 0.20), 70.0, 0.35, 0.16),
    echo(room("Tunnel L", 2.4, 28.0, 0.35, 5000.0, 0.22), 100.0, 0.40, 0.18),
    room("Chamber S", 0.6, 5.0, 0.45, 6000.0, 0.16),
    room("Chamber M", 1.0, 10.0, 0.45, 6000.0, 0.18),
    room("Chamber L", 1.6, 18.0, 0.45, 6000.0, 0.20),
    room("Bright S", 0.5, 4.0, 0.05, 11000.0, 0.16),
    room("Bright M", 0.9, 8.0, 0.05, 11000.0, 0.18),
    room("Bright L", 1.4, 14.0, 0.05, 11000.0, 0.20),
    room("Water 1", 1.2, 10.0, 0.7, 900.0, 0.28),
    room("Water 2", 1.8, 15.0, 0.75, 700.0, 0.32),
    room("Water 3", 2.5, 20.0, 0.8, 500.0, 0.36),
    room("Concrete S", 0.7, 5.0, 0.3, 7000.0, 0.16),
    room("Concrete M", 1.1, 10.0, 0.3, 7000.0, 0.18),
    room("Concrete L", 1.7, 18.0, 0.3, 7000.0, 0.20),
    echo(room("Big 1", 1.8, 35.0, 0.4, 6000.0, 0.16), 150.0, 0.20, 0.12),
    echo(room("Big 2", 2.5, 50.0, 0.4, 6000.0, 0.18), 220.0, 0.25, 0.14),
    echo(room("Big 3", 3.2, 70.0, 0.4, 6000.0, 0.20), 300.0, 0.30, 0.16),
    echo(room("Cavern S", 2.2, 40.0, 0.3, 5000.0, 0.20), 180.0, 0.30, 0.15),
    echo(room("Cavern M", 3.0, 60.0, 0.3, 5000.0, 0.22), 260.0, 0.35, 0.18),
    echo(room("Cavern L", 4.0, 80.0, 0.3, 5000.0, 0.25), 350.0, 0.40, 0.20),
    echo(room("Weirdo 1", 1.0, 20.0, 0.2, 8000.0, 0.18), 60.0, 0.60, 0.22),
    echo(room("Weirdo 2", 1.5, 30.0, 0.2, 8000.0, 0.20), 110.0, 0.65, 0.24),
    echo(room("Weirdo 3", 2.0, 40.0, 0.2, 8000.0, 0.22), 170.0, 0.70, 0.26),
];

/// The preset for a soundscape's "dsp" index; an index past the table
/// plays as Generic (ours: Source's later engines add more rooms).
pub fn preset(index: u16) -> &'static RoomPreset {
    PRESETS.get(index as usize).unwrap_or(&PRESETS[1])
}

/// How much of a world sound goes to the room: the near level at the
/// listener, rising to all of it at `FAR_UNITS` (ours: farther sounds
/// reach the ear more through the room).
pub const NEAR_SEND: f32 = 0.6;
pub const FAR_UNITS: f32 = 1500.0;
/// Unspatialized world sounds (soundscape ambience, play-everywhere
/// ambient sounds) send this much.
pub const AMBIENT_SEND: f32 = 1.0;

/// The send of a sound `units` (Source units) from the listener.
pub fn distance_send(units: f32) -> f32 {
    NEAR_SEND + (1.0 - NEAR_SEND) * (units / FAR_UNITS).clamp(0.0, 1.0)
}

/// The room the listener is in and the bus that plays it.
#[derive(Resource)]
pub struct RoomDsp {
    /// Source preset index (`PRESETS`), set by the current soundscape.
    pub preset: u16,
    /// `dsp_off`: 1 turns the room off.
    pub off: u8,
    /// `dsp_volume`: scales the room's level.
    pub volume: f32,
    pub bus: Arc<RoomBus>,
    playing: Option<Entity>,
}

impl Default for RoomDsp {
    fn default() -> Self {
        Self {
            preset: 0,
            off: 0,
            volume: 1.0,
            bus: Arc::default(),
            playing: None,
        }
    }
}

impl RoomDsp {
    /// The preset that plays now (Normal while off).
    pub fn active(&self) -> &'static RoomPreset {
        if self.off != 0 { &PRESETS[0] } else { preset(self.preset) }
    }
}

/// Ring size (frames of the bus) and how far ahead of the bus's read
/// position the voices write.
const RING: usize = 8192;
const LEAD: u64 = 256;
/// The bus's own rate.
const BUS_RATE: u32 = 44100;

/// What the voices and the bus share.
#[derive(Debug)]
pub struct RoomBus {
    /// Frames the bus has played.
    pos: AtomicU64,
    /// The bus plays (voices send nothing otherwise).
    running: AtomicBool,
    /// Mono send, f32 bits, indexed by bus frame.
    ring: Box<[AtomicU32]>,
    /// The preset to play (index) and level (f32 bits).
    preset: AtomicU32,
    level: AtomicU32,
}

impl Default for RoomBus {
    fn default() -> Self {
        Self {
            pos: AtomicU64::new(0),
            running: AtomicBool::new(false),
            ring: (0..RING).map(|_| AtomicU32::new(0)).collect(),
            preset: AtomicU32::new(0),
            level: AtomicU32::new(1f32.to_bits()),
        }
    }
}

impl RoomBus {
    fn add(&self, frame: u64, v: f32) {
        let slot = &self.ring[(frame % RING as u64) as usize];
        let _ = slot.fetch_update(Ordering::Relaxed, Ordering::Relaxed, |b| {
            Some((f32::from_bits(b) + v).to_bits())
        });
    }

    fn set(&self, preset: u16, level: f32) {
        self.preset.store(preset as u32, Ordering::Relaxed);
        self.level.store(level.to_bits(), Ordering::Relaxed);
    }
}

/// A voice's send into the bus: averages its samples per bus frame and
/// holds them over the frames it skips (a voice slower than the bus).
#[derive(Debug)]
pub struct RoomSend {
    bus: Arc<RoomBus>,
    frame: Option<u64>,
    sum: f32,
    count: u32,
}

impl RoomSend {
    pub fn new(bus: Arc<RoomBus>) -> Self {
        Self {
            bus,
            frame: None,
            sum: 0.0,
            count: 0,
        }
    }

    /// One mono sample (already scaled by the send level).
    pub fn push(&mut self, v: f32) {
        if !self.bus.running.load(Ordering::Relaxed) {
            self.frame = None;
            return;
        }
        let now = self.bus.pos.load(Ordering::Relaxed);
        match self.frame {
            Some(f) if f == now => {}
            Some(f) if f < now && now - f < RING as u64 - LEAD => {
                let avg = self.sum / self.count.max(1) as f32;
                if avg != 0.0 {
                    for k in f..now {
                        self.bus.add(k + LEAD, avg);
                    }
                }
                self.frame = Some(now);
                self.sum = 0.0;
                self.count = 0;
            }
            _ => {
                self.frame = Some(now);
                self.sum = 0.0;
                self.count = 0;
            }
        }
        self.sum += v;
        self.count += 1;
    }
}

/// Freeverb's tunings at 44.1 kHz, and the right ear's offset.
const COMBS: [usize; 8] = [1116, 1188, 1277, 1356, 1422, 1491, 1557, 1617];
const ALLPASSES: [usize; 4] = [556, 441, 341, 225];
const SPREAD: usize = 23;
const INPUT_GAIN: f32 = 0.015;
const WET_SCALE: f32 = 3.0;
/// Parameters are re-read and smoothed every this many frames.
const BLOCK: u32 = 64;
const SMOOTHING: f32 = 0.05;
const MAX_PREDELAY_MS: f32 = 100.0;
const MAX_ECHO_MS: f32 = 400.0;

struct Comb {
    buf: Vec<f32>,
    i: usize,
    store: f32,
    feedback: f32,
}

impl Comb {
    fn new(len: usize) -> Self {
        Self {
            buf: vec![0.0; len],
            i: 0,
            store: 0.0,
            feedback: 0.0,
        }
    }
    #[inline]
    fn run(&mut self, x: f32, damping: f32) -> f32 {
        let out = self.buf[self.i];
        self.store = out * (1.0 - damping) + self.store * damping;
        self.buf[self.i] = x + self.store * self.feedback;
        self.i = (self.i + 1) % self.buf.len();
        out
    }
}

struct AllPass {
    buf: Vec<f32>,
    i: usize,
}

impl AllPass {
    #[inline]
    fn run(&mut self, x: f32) -> f32 {
        let b = self.buf[self.i];
        self.buf[self.i] = x + b * 0.5;
        self.i = (self.i + 1) % self.buf.len();
        b - x
    }
}

/// A delay line read `delay` samples behind its write position.
struct Delay {
    buf: Vec<f32>,
    i: usize,
}

impl Delay {
    fn new(len: usize) -> Self {
        Self {
            buf: vec![0.0; len.max(1)],
            i: 0,
        }
    }
    #[inline]
    fn read(&self, delay: usize) -> f32 {
        let n = self.buf.len();
        self.buf[(self.i + n - delay.min(n - 1)) % n]
    }
    #[inline]
    fn write(&mut self, x: f32) {
        self.buf[self.i] = x;
        self.i = (self.i + 1) % self.buf.len();
    }
}

/// Smoothed preset values the reverb runs with.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct Running {
    decay: f32,
    damping: f32,
    wet: f32,
    echo_feedback: f32,
    echo_gain: f32,
}

/// The room: mono in, stereo out (wet only).
pub struct Reverb {
    rate: f32,
    combs: [Vec<Comb>; 2],
    allpasses: [Vec<AllPass>; 2],
    predelay: Delay,
    echo: Delay,
    low: f32,
    alpha: f32,
    now: Running,
    target: &'static RoomPreset,
    level: f32,
    predelay_frames: usize,
    echo_frames: usize,
    counter: u32,
}

impl Reverb {
    pub fn new(rate: u32) -> Self {
        let scale = rate as f32 / 44100.0;
        let len = |n: usize| ((n as f32 * scale) as usize).max(1);
        let side = |extra: usize| -> (Vec<Comb>, Vec<AllPass>) {
            (
                COMBS.iter().map(|&n| Comb::new(len(n + extra))).collect(),
                ALLPASSES
                    .iter()
                    .map(|&n| AllPass {
                        buf: vec![0.0; len(n + extra)],
                        i: 0,
                    })
                    .collect(),
            )
        };
        let (cl, al) = side(0);
        let (cr, ar) = side(SPREAD);
        let ms = |m: f32| (m * rate as f32 / 1000.0) as usize + 1;
        let mut me = Self {
            rate: rate as f32,
            combs: [cl, cr],
            allpasses: [al, ar],
            predelay: Delay::new(ms(MAX_PREDELAY_MS)),
            echo: Delay::new(ms(MAX_ECHO_MS)),
            low: 0.0,
            alpha: 1.0,
            now: Running::default(),
            target: &PRESETS[0],
            level: 1.0,
            predelay_frames: 0,
            echo_frames: 0,
            counter: 0,
        };
        me.retune();
        me
    }

    /// The preset (and level) to move to.
    pub fn set(&mut self, preset: &'static RoomPreset, level: f32) {
        self.target = preset;
        self.level = level;
    }

    /// Move the running values toward the target and recompute the
    /// coefficients (every `BLOCK` frames).
    fn retune(&mut self) {
        let t = self.target;
        let goal = Running {
            decay: t.decay,
            damping: t.damping,
            wet: t.wet * self.level,
            echo_feedback: t.echo_feedback,
            echo_gain: t.echo_gain * self.level,
        };
        let step = |a: &mut f32, b: f32| *a += (b - *a) * SMOOTHING;
        step(&mut self.now.decay, goal.decay);
        step(&mut self.now.damping, goal.damping);
        step(&mut self.now.wet, goal.wet);
        step(&mut self.now.echo_feedback, goal.echo_feedback);
        step(&mut self.now.echo_gain, goal.echo_gain);
        for side in &mut self.combs {
            for c in side.iter_mut() {
                let seconds = c.buf.len() as f32 / self.rate;
                c.feedback = if self.now.decay > 1e-3 {
                    10f32.powf(-3.0 * seconds / self.now.decay).min(0.98)
                } else {
                    0.0
                };
            }
        }
        self.alpha = low_pass_alpha(t.cutoff, self.rate);
        let frames = |ms: f32| (ms * self.rate / 1000.0) as usize;
        self.predelay_frames = frames(t.predelay.min(MAX_PREDELAY_MS));
        self.echo_frames = frames(t.echo.min(MAX_ECHO_MS));
    }

    /// One frame: the room's (left, right).
    pub fn frame(&mut self, x: f32) -> (f32, f32) {
        if self.counter == 0 {
            self.retune();
        }
        self.counter = (self.counter + 1) % BLOCK;
        // Keep tails out of denormals.
        self.low += self.alpha * (x - self.low) + 1e-18;
        self.predelay.write(self.low);
        let input = self.predelay.read(self.predelay_frames) * INPUT_GAIN;
        let mut out = [0.0f32; 2];
        for (side, o) in out.iter_mut().enumerate() {
            let mut acc = 0.0;
            for c in self.combs[side].iter_mut() {
                acc += c.run(input, self.now.damping);
            }
            for a in self.allpasses[side].iter_mut() {
                acc = a.run(acc);
            }
            *o = acc * self.now.wet * WET_SCALE;
        }
        let echoed = if self.echo_frames > 0 {
            self.echo.read(self.echo_frames)
        } else {
            0.0
        };
        self.echo.write(self.low + echoed * self.now.echo_feedback);
        let e = echoed * self.now.echo_gain;
        (out[0] + e, out[1] + e)
    }
}

/// The bus as a Bevy audio source.
#[derive(Asset, TypePath)]
pub struct RoomClip {
    bus: Arc<RoomBus>,
    hearing: Arc<HearingMix>,
}

pub struct RoomDecoder {
    bus: Arc<RoomBus>,
    hearing: Arc<HearingMix>,
    reverb: Reverb,
    muffle: Muffle,
    right: Option<f32>,
}

impl Iterator for RoomDecoder {
    type Item = bevy::audio::Sample;

    fn next(&mut self) -> Option<Self::Item> {
        if let Some(r) = self.right.take() {
            return Some(r);
        }
        let pos = self.bus.pos.load(Ordering::Relaxed);
        let x = f32::from_bits(self.bus.ring[(pos % RING as u64) as usize].swap(0, Ordering::Relaxed));
        self.bus.pos.store(pos + 1, Ordering::Relaxed);
        if pos.is_multiple_of(BLOCK as u64) {
            let p = self.bus.preset.load(Ordering::Relaxed) as u16;
            let level = f32::from_bits(self.bus.level.load(Ordering::Relaxed));
            self.reverb.set(preset(p), level);
        }
        let (l, r) = self.reverb.frame(x);
        let (l, r) = self.muffle.frame(&self.hearing, BUS_RATE as f32, l, r);
        self.right = Some(r);
        Some(l)
    }
}

impl Drop for RoomDecoder {
    fn drop(&mut self) {
        self.bus.running.store(false, Ordering::Relaxed);
    }
}

impl Source for RoomDecoder {
    fn current_span_len(&self) -> Option<usize> {
        None
    }
    fn channels(&self) -> ChannelCount {
        ChannelCount::new(2).unwrap()
    }
    fn sample_rate(&self) -> SampleRate {
        SampleRate::new(BUS_RATE).unwrap()
    }
    fn total_duration(&self) -> Option<std::time::Duration> {
        None
    }
}

impl Decodable for RoomClip {
    type Decoder = RoomDecoder;

    fn decoder(&self) -> RoomDecoder {
        self.bus.running.store(true, Ordering::Relaxed);
        RoomDecoder {
            bus: self.bus.clone(),
            hearing: self.hearing.clone(),
            reverb: Reverb::new(BUS_RATE),
            muffle: Muffle::default(),
            right: None,
        }
    }
}

/// Registered by `sound::SoundPlugin`.
pub(super) struct RoomPlugin;

impl Plugin for RoomPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<RoomDsp>()
            .add_systems(PostUpdate, drive_bus.run_if(resource_exists::<Assets<RoomClip>>));
    }

    fn finish(&self, app: &mut App) {
        if app.is_plugin_added::<bevy::audio::AudioPlugin>() {
            bevy::audio::AddAudioSource::add_audio_source::<RoomClip>(app);
        }
    }
}

/// Start the bus once, hand it the room, follow the master volume.
fn drive_bus(
    mut room: ResMut<RoomDsp>,
    mut clips: ResMut<Assets<RoomClip>>,
    mut sinks: Query<&mut AudioSink>,
    hearing: Res<Hearing>,
    global: Option<Res<bevy::audio::GlobalVolume>>,
    mut commands: Commands,
) {
    let index = if room.off != 0 { 0 } else { room.preset };
    room.bus.set(index, room.volume.max(0.0));
    let master = global.map_or(1.0, |g| g.volume.to_linear());
    match room.playing {
        Some(e) if commands.get_entity(e).is_ok() => {
            if let Ok(mut sink) = sinks.get_mut(e)
                && (sink.volume().to_linear() - master).abs() > 1e-4
            {
                sink.set_volume(bevy::audio::Volume::Linear(master));
            }
        }
        _ => {
            let handle = clips.add(RoomClip {
                bus: room.bus.clone(),
                hearing: hearing.mix.clone(),
            });
            room.playing = Some(commands.spawn((AudioPlayer(handle), PlaybackSettings::DESPAWN)).id());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn presets_follow_sources_numbering() {
        assert_eq!(PRESETS[0].wet, 0.0);
        assert_eq!(preset(1).name, "Generic");
        assert_eq!(preset(6).name, "Tunnel M");
        assert_eq!(preset(15).name, "Water 2");
        assert_eq!(preset(28).name, "Weirdo 3");
        // Past the table: Generic.
        assert_eq!(preset(40).name, "Generic");
        // Sizes grow S -> M -> L.
        for base in [2, 5, 8, 11, 14, 17, 20, 23, 26] {
            assert!(PRESETS[base].decay < PRESETS[base + 1].decay);
            assert!(PRESETS[base + 1].decay < PRESETS[base + 2].decay);
        }
    }

    #[test]
    fn off_plays_normal() {
        let mut r = RoomDsp {
            preset: 6,
            ..default()
        };
        assert_eq!(r.active().name, "Tunnel M");
        r.off = 1;
        assert_eq!(r.active().name, "Normal (off)");
    }

    #[test]
    fn send_grows_with_distance() {
        assert!((distance_send(0.0) - NEAR_SEND).abs() < 1e-6);
        assert!(distance_send(500.0) > NEAR_SEND);
        assert!((distance_send(5000.0) - 1.0).abs() < 1e-6);
    }

    /// Energy of the room's answer to a click over `seconds`.
    fn tail(p: &'static RoomPreset, from: f32, to: f32) -> f32 {
        let mut r = Reverb::new(44100);
        r.set(p, 1.0);
        // Let the running values reach the preset.
        for _ in 0..44100 {
            r.frame(0.0);
        }
        let mut e = 0.0;
        for i in 0..(to * 44100.0) as usize {
            let (l, rr) = r.frame(if i == 0 { 1.0 } else { 0.0 });
            if i as f32 >= from * 44100.0 {
                e += l * l + rr * rr;
            }
        }
        e
    }

    #[test]
    fn rooms_ring_by_their_size_and_off_is_silent() {
        assert_eq!(tail(&PRESETS[0], 0.0, 1.0), 0.0);
        let small = tail(preset(17), 0.5, 1.5);
        let large = tail(preset(19), 0.5, 1.5);
        assert!(small > 0.0 && large > small * 4.0, "{small} {large}");
        // A tunnel's echo arrives after its delay.
        let early = tail(preset(7), 0.0, 0.09);
        let echoed = tail(preset(7), 0.09, 0.12);
        assert!(echoed > early, "{early} {echoed}");
    }

    #[test]
    fn voices_send_into_the_bus_ahead_of_it() {
        let bus = Arc::new(RoomBus::default());
        let mut send = RoomSend::new(bus.clone());
        // Not running: nothing is written.
        send.push(1.0);
        assert!(bus.ring.iter().all(|s| s.load(Ordering::Relaxed) == 0));
        bus.running.store(true, Ordering::Relaxed);
        // Two samples in bus frame 0 average; a voice at half the bus's
        // rate holds its value over the frame it skips.
        send.push(1.0);
        send.push(3.0);
        bus.pos.store(1, Ordering::Relaxed);
        send.push(5.0);
        bus.pos.store(3, Ordering::Relaxed);
        send.push(0.0);
        let at = |f: u64| f32::from_bits(bus.ring[(f % RING as u64) as usize].load(Ordering::Relaxed));
        assert_eq!(at(LEAD), 2.0);
        assert_eq!((at(LEAD + 1), at(LEAD + 2)), (5.0, 5.0));
        assert_eq!(at(LEAD + 3), 0.0);
    }
}
