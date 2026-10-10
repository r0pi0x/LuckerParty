//! ambient_generic (specs/cs_source/sounds.md 6, "ambient_generic"): a
//! sound entry or raw wave played from the entity (or another entity),
//! started at map load unless it starts silent, toggled by inputs, with
//! pitch and volume ramps (spin up/down, fade in/out) and an LFO stepped
//! at 5 Hz. The logic only keeps the entity's state and asks for sounds
//! through `Effect::AmbientStart`/`AmbientChange`/`AmbientStop`; the host
//! plays them. Volumes and pitches are integers (percent), ramps fixed
//! point ×256, as in the spec.

use super::classes::Class;
use super::value::Value;
use super::world::{Effect, EntId, LogicEntity, LogicWorld};

/// Seconds between ramp/LFO steps (ambient_update_rate 5 Hz).
pub const RAMP_STEP: f32 = 0.2;
/// The first step after a start, toggle or fade input.
pub const RAMP_FIRST: f32 = 0.1;
/// Radius -> sound level: the level `REF_LEVEL` at `REF_DIST` units.
pub const REF_DIST: f32 = 36.0;
pub const REF_LEVEL: f32 = 40.0;

pub const FLAG_EVERYWHERE: u32 = 1;
pub const FLAG_START_SILENT: u32 = 16;
pub const FLAG_NOT_LOOPING: u32 = 32;

/// The sound level for an audible radius (units): trunc(40 +
/// 20·log10(r/36)); 0 (no falloff) for no radius or "play everywhere".
pub fn radius_level(radius: f32, everywhere: bool) -> f32 {
    if radius > 0.0 && !everywhere {
        (REF_LEVEL + 20.0 * (radius / REF_DIST).log10()).trunc()
    } else {
        0.0
    }
}

/// Legacy spin/fade speed 1–100 -> ramp rate per step (×256).
fn legacy_rate(s: i32) -> i32 {
    if s <= 0 { 0 } else { (101 - s.min(100)) * 64 }
}

/// Seconds for a full 0 -> 100 volume fade -> rate per step (×256); 0 s
/// fades in one step.
fn seconds_rate(t: f32) -> i32 {
    let t = t.clamp(0.0, 100.0);
    if t <= 0.0 {
        return 100 << 8;
    }
    (25600.0 / (5.0 * t)).floor() as i32
}

/// LFO shapes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Lfo {
    #[default]
    Off,
    Square,
    Triangle,
    Random,
}

/// The modulation state (the spec's "dynamic pitch/volume").
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Modulation {
    pub pitch_run: i32,
    pub pitch_start: i32,
    pub spinup: i32,
    pub spindown: i32,
    pub pitch: i32,
    pub pitch_frac: i32,
    /// Pitch ramp in progress: +spinup, −spindown, 0 none.
    pub spin: i32,
    pub vol_run: i32,
    pub vol_start: i32,
    pub fadein: i32,
    pub fadeout: i32,
    pub vol: i32,
    pub vol_frac: i32,
    /// Volume ramp in progress: +fadein, −fadeout, 0 none.
    pub fade: i32,
    pub lfo: Lfo,
    /// Signed: flips at either end.
    pub lfo_rate: i32,
    pub lfo_frac: i32,
    pub lfo_mult: i32,
    pub lfo_mod_pitch: i32,
    pub lfo_mod_vol: i32,
    pub cspinup: i32,
    pub cspin_count: i32,
}

impl Modulation {
    /// From the entity's keys, as at a start.
    fn new(e: &LogicEntity) -> Self {
        let i = |k: &str| e.kv_i(k);
        let fade = |secs: &str, legacy: &str| {
            let t = e.kv_f(secs);
            if t > 0.0 { seconds_rate(t) } else { legacy_rate(i(legacy)) }
        };
        let mut m = Modulation {
            pitch_run: i("pitch").clamp(0, 255),
            pitch_start: i("pitchstart").clamp(0, 255),
            spinup: legacy_rate(i("spinup")),
            spindown: legacy_rate(i("spindown")),
            vol_run: ((e.kv_f("health") * 10.0) as i32).clamp(0, 100),
            vol_start: (i("volstart") * 10).clamp(0, 100),
            fadein: fade("fadeinsecs", "fadein"),
            fadeout: fade("fadeoutsecs", "fadeout"),
            lfo: match i("lfotype") {
                1 => Lfo::Square,
                2 => Lfo::Triangle,
                3 => Lfo::Random,
                t if t > 4 => Lfo::Triangle,
                _ => Lfo::Off,
            },
            lfo_rate: i("lforate").clamp(0, 1000) * 256,
            lfo_mod_pitch: i("lfomodpitch").clamp(0, 100),
            lfo_mod_vol: i("lfomodvol").clamp(0, 100),
            cspinup: i("cspinup").clamp(0, 100),
            ..Default::default()
        };
        // On start.
        if m.fadein > 0 {
            m.vol = m.vol_start;
            m.fade = m.fadein;
        } else {
            m.vol = m.vol_run;
        }
        if m.spinup > 0 {
            m.pitch = m.pitch_start;
            m.spin = m.spinup;
        } else {
            m.pitch = m.pitch_run;
        }
        if m.pitch == 0 {
            m.pitch = 100;
        }
        if m.cspinup > 0 {
            m.cspin_count = 1;
            m.pitch_run = (m.pitch_start + (255 - m.pitch_start) / m.cspinup).min(255);
        }
        let pitch_mod = m.spinup > 0 || m.spindown > 0 || (m.lfo != Lfo::Off && m.lfo_mod_pitch > 0);
        if pitch_mod && m.pitch == 100 {
            m.pitch = 101;
        }
        m.pitch_frac = m.pitch << 8;
        m.vol_frac = m.vol << 8;
        m
    }

    fn lfo_on(&self) -> bool {
        self.lfo != Lfo::Off && self.lfo_rate != 0 && (self.lfo_mod_pitch > 0 || self.lfo_mod_vol > 0)
    }

    fn ramping(&self) -> bool {
        self.spin != 0 || self.fade != 0 || self.lfo_on()
    }
}

/// What a ramp step did.
#[derive(Debug, PartialEq, Eq)]
enum Step {
    Same,
    Changed,
    /// Spun down or faded out: stop the sound.
    Stop,
}

/// One 0.2 s step of the ramps and the LFO. `random` gives 0..256 for
/// the random LFO.
fn step(m: &mut Modulation, mut random: impl FnMut() -> i32) -> Step {
    let mut changed = false;
    if m.spin != 0 {
        m.pitch_frac += m.spin;
        m.pitch = m.pitch_frac >> 8;
        if m.spin > 0 && m.pitch > m.pitch_run {
            m.pitch = m.pitch_run;
            m.pitch_frac = m.pitch << 8;
            m.spin = 0;
        } else if m.spin < 0 && m.pitch < m.pitch_start {
            m.spin = 0;
            m.fade = 0;
            return Step::Stop;
        }
        m.pitch = m.pitch.clamp(1, 255);
        changed = true;
    }
    if m.fade != 0 {
        m.vol_frac += m.fade;
        m.vol = m.vol_frac >> 8;
        if m.fade > 0 && m.vol > m.vol_run {
            m.vol = m.vol_run;
            m.vol_frac = m.vol << 8;
            m.fade = 0;
        } else if m.fade < 0 && m.vol < m.vol_start {
            m.vol = m.vol_start;
            m.spin = 0;
            m.fade = 0;
            return Step::Stop;
        }
        m.vol = m.vol.clamp(1, 100);
        changed = true;
    }
    if m.lfo_on() {
        m.lfo_frac += m.lfo_rate;
        let mut pos = m.lfo_frac >> 8;
        if pos >= 255 {
            pos = 255;
            m.lfo_frac = 255 << 8;
            m.lfo_rate = -m.lfo_rate.abs();
            if m.lfo == Lfo::Random {
                m.lfo_mult = random();
            }
        } else if pos <= 0 {
            pos = 0;
            m.lfo_frac = 0;
            m.lfo_rate = m.lfo_rate.abs();
        }
        match m.lfo {
            Lfo::Square => m.lfo_mult = if pos < 128 { 255 } else { 0 },
            Lfo::Triangle => m.lfo_mult = pos,
            _ => {}
        }
        if m.lfo_mod_pitch > 0 {
            m.pitch = (m.pitch + (m.lfo_mult - 128) * m.lfo_mod_pitch / 100).clamp(1, 255);
            changed = true;
        }
        if m.lfo_mod_vol > 0 {
            m.vol = (m.vol + (m.lfo_mult - 128) * m.lfo_mod_vol / 100).clamp(0, 100);
            changed = true;
        }
    }
    if changed { Step::Changed } else { Step::Same }
}

/// An ambient_generic.
#[derive(Clone, Debug, Default)]
pub struct Ambient {
    /// Sound entry or raw wave ("…/x.wav").
    pub message: String,
    /// A raw wave file, not a script entry.
    pub raw: bool,
    /// Sound level from the radius (0: heard everywhere).
    pub level: f32,
    /// The not-looping flag is clear.
    pub looping: bool,
    /// Play from this entity (SourceEntityName).
    pub source: Option<String>,
    /// On (looping sounds only).
    pub active: bool,
    /// A sound was started and not stopped since.
    pub playing: bool,
    pub m: Modulation,
}

/// The class state at spawn; None removes the entity (no message).
pub(super) fn spawn(w: &mut LogicWorld, id: EntId) -> Option<Ambient> {
    let e = w.get(id)?;
    let message = e.kv("message").unwrap_or("").trim().to_string();
    let preset = e.kv_i("preset") != 0;
    let looping = !e.has_flag(FLAG_NOT_LOOPING);
    let lower = message.to_ascii_lowercase();
    let ambient = Ambient {
        raw: lower.contains(".wav") || lower.contains(".mp3"),
        level: radius_level(e.kv_f("radius"), e.has_flag(FLAG_EVERYWHERE)),
        looping,
        source: e.kv("SourceEntityName").map(str::trim).filter(|s| !s.is_empty()).map(String::from),
        active: looping && !e.has_flag(FLAG_START_SILENT),
        playing: false,
        m: Modulation::new(e),
        message,
    };
    if ambient.message.is_empty() {
        w.kill(id);
        return None;
    }
    if ambient.message.starts_with('!') {
        w.log.push(format!("ambient_generic: sentence '{}' is not supported (silent)", ambient.message));
    }
    if preset {
        // The 27 GoldSrc preset rows aren't in our spec (sounds.md Open
        // question 10): the entity's own keys play.
        w.note(
            "ambient_generic presets not applied (sounds.md Q10: the table isn't specified; its keys play)",
            ambient.message.clone(),
        );
    }
    Some(ambient)
}

/// AddOutput message: the sound it plays from now on (the next
/// PlaySound; a looping one already playing keeps its sound until then).
pub(super) fn set_message(w: &mut LogicWorld, id: EntId) -> bool {
    let Some(message) = w.get(id).and_then(|e| e.kv("message")).map(|m| m.trim().to_string()) else {
        return false;
    };
    let raw = {
        let lower = message.to_ascii_lowercase();
        lower.contains(".wav") || lower.contains(".mp3")
    };
    if let Some(Class::Ambient(a)) = w.get_mut(id).map(|e| &mut e.class) {
        a.message = message;
        a.raw = raw;
    }
    true
}

fn get(w: &LogicWorld, id: EntId) -> Option<Ambient> {
    match w.get(id).map(|e| &e.class) {
        Some(Class::Ambient(a)) => Some((**a).clone()),
        _ => None,
    }
}

fn put(w: &mut LogicWorld, id: EntId, a: Ambient) {
    if let Some(e) = w.get_mut(id) {
        e.class = Class::Ambient(Box::new(a));
    }
}

/// Map start (and each round's): an active sound starts with the
/// entity's volume and pitch, if it has any volume (or fades in).
pub(super) fn activate(w: &mut LogicWorld, id: EntId) {
    let Some(a) = get(w, id) else { return };
    if !a.active || (a.m.vol <= 0 && a.m.fade <= 0) {
        return;
    }
    start(w, id, a, true);
}

/// Start the sound: from the source entity if it names one. Raw waves
/// always take the entity's volume, pitch and level; script entries only
/// at map start, otherwise their own (spec "Quirks"), and always their
/// own level (open question 11).
fn start(w: &mut LogicWorld, id: EntId, mut a: Ambient, map_start: bool) {
    if a.message.starts_with('!') {
        return;
    }
    let Some(origin) = w.get(id).map(|e| e.origin) else { return };
    let source = a.source.as_deref().and_then(|s| w.find(s));
    if a.source.is_some() && source.is_none() {
        w.log.push(format!("ambient_generic: no source entity '{}'", a.source.as_deref().unwrap()));
    }
    let own = a.raw || map_start;
    w.effects.push(Effect::AmbientStart {
        id,
        entry: a.message.clone(),
        at: origin,
        source,
        volume: own.then_some(a.m.vol as f32 / 100.0),
        pitch: own.then_some(a.m.pitch as f32),
        level: a.raw.then_some(a.level),
    });
    a.playing = true;
    let ramp = a.m.ramping();
    put(w, id, a);
    if ramp {
        w.think_in(id, RAMP_FIRST);
    }
}

fn stop(w: &mut LogicWorld, id: EntId, a: &mut Ambient) {
    if a.playing {
        w.effects.push(Effect::AmbientStop { id });
        a.playing = false;
    }
}

fn change(w: &mut LogicWorld, id: EntId, m: &Modulation, pitch_norm_bump: bool) {
    let pitch = if pitch_norm_bump && m.pitch == 100 { 101 } else { m.pitch };
    w.effects.push(Effect::AmbientChange {
        id,
        volume: Some(m.vol as f32 / 100.0),
        pitch: Some(pitch as f32),
    });
}

fn toggle(w: &mut LogicWorld, id: EntId) {
    let Some(mut a) = get(w, id) else { return };
    let Some(fresh) = w.get(id).map(Modulation::new) else { return };
    if !a.looping || !a.active {
        // On: a one-shot restarts.
        if !a.looping {
            stop(w, id, &mut a);
        } else {
            a.active = true;
        }
        a.m = fresh;
        start(w, id, a, false);
        return;
    }
    // Off.
    if a.m.cspinup > 0 {
        if a.m.cspin_count <= a.m.cspinup {
            a.m.cspin_count += 1;
            let step = (255 - a.m.pitch_start) / a.m.cspinup;
            a.m.pitch_run = (a.m.pitch_run + step).min(255);
            if a.m.spinup > 0 {
                a.m.spin = a.m.spinup;
                put(w, id, a);
                w.think_in(id, RAMP_FIRST);
            } else {
                a.m.pitch = a.m.pitch_run;
                a.m.pitch_frac = a.m.pitch << 8;
                change(w, id, &a.m, false);
                put(w, id, a);
            }
        }
        return;
    }
    a.active = false;
    if a.m.spindown > 0 || a.m.fadeout > 0 {
        if a.m.spindown > 0 {
            a.m.spin = -a.m.spindown;
        }
        if a.m.fadeout > 0 {
            a.m.fade = -a.m.fadeout;
        }
        put(w, id, a);
        w.think_in(id, RAMP_FIRST);
    } else {
        stop(w, id, &mut a);
        put(w, id, a);
    }
}

/// A ramp/LFO step.
pub(super) fn think(w: &mut LogicWorld, id: EntId) {
    let Some(mut a) = get(w, id) else { return };
    let result = step(&mut a.m, || (w.random() * 256.0) as i32);
    match result {
        Step::Stop => {
            stop(w, id, &mut a);
            put(w, id, a);
            return;
        }
        Step::Changed if a.playing => change(w, id, &a.m, true),
        _ => {}
    }
    let again = a.m.ramping() && (a.playing || a.m.spin != 0 || a.m.fade != 0);
    put(w, id, a);
    if again {
        w.think_in(id, RAMP_STEP);
    }
}

/// Inputs; false when not one of ambient_generic's.
pub(super) fn input(w: &mut LogicWorld, id: EntId, input: &str, value: &Value) -> bool {
    let Some(mut a) = get(w, id) else { return false };
    match input {
        "playsound" => {
            if !a.active {
                stop(w, id, &mut a);
                put(w, id, a);
                toggle(w, id);
            }
        }
        "stopsound" => {
            if a.active {
                toggle(w, id);
            }
        }
        "togglesound" => toggle(w, id),
        "pitch" => {
            let Some(x) = w.need_float(value, input) else { return true };
            a.m.pitch = (x.round() as i32).clamp(0, 255);
            a.m.pitch_frac = a.m.pitch << 8;
            if a.playing {
                change(w, id, &a.m, false);
            }
            put(w, id, a);
        }
        "volume" => {
            let Some(x) = w.need_float(value, input) else { return true };
            a.m.vol = ((10.0 * x).round() as i32).clamp(0, 100);
            a.m.vol_frac = a.m.vol << 8;
            if a.playing {
                change(w, id, &a.m, false);
            }
            put(w, id, a);
        }
        "fadein" | "fadeout" => {
            let Some(t) = w.need_float(value, input) else { return true };
            let rate = seconds_rate(t);
            if input == "fadein" {
                a.m.fadein = rate;
                a.m.fade = rate;
            } else {
                a.m.fadeout = rate;
                a.m.fade = -rate;
            }
            put(w, id, a);
            w.think_in(id, RAMP_FIRST);
        }
        _ => return false,
    }
    true
}

/// The entity is being removed: its sound stops.
pub(super) fn removed(effects: &mut Vec<Effect>, id: EntId, a: &Ambient) {
    if a.playing {
        effects.push(Effect::AmbientStop { id });
    }
}

#[cfg(test)]
#[path = "ambient_tests.rs"]
mod tests;
