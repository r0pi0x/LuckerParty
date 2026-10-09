//! Soundscape playback (specs/cs_source/sounds.md 6, "Soundscapes"): pick
//! the soundscape for the listener, fade its loops in over 3 s while the
//! previous one's fade out, play its random one-shots and set the room
//! DSP preset, room level and player preset it names (`room::RoomDsp`).
//! Emitters the logic disabled (`SoundscapeSwitches`) are not picked; one
//! becoming current fires its OnPlay output.
//!
//! Selection and the loops run headless too: the loops are long-lived
//! sounds (`live_sound`, keys under `live_sound::SOUNDSCAPE_KEYS`), which
//! play their intro, loop from their loop point and pan live. The random
//! one-shots play only with audio.

use std::sync::Arc;

use bevy::{
    audio::{AudioPlayer, PlaybackSettings},
    prelude::*,
};

use super::{
    hearing::Hearing,
    live_sound::{Gains, LiveClip, SOUNDSCAPE_KEYS, SoundControl, SoundKey, StartSound},
    room::{AMBIENT_SEND, PRESETS, RoomDsp, distance_send},
    sound::{METERS_PER_UNIT, MapSoundClip, ScapePosition, SoundBank, SoundLevel, SoundListener, distance_gain, pan},
    vis::ActiveVisibility,
};

/// Seconds for a loop's volume to move by 1.0 (soundscape_fadetime).
const FADE_TIME: f32 = 3.0;
/// Random sounds with "position random" play this far from the eye, units.
const RANDOM_DISTANCE: f32 = 36.0;
/// A new positional loop starts at this volume (soundscape_loop_start_vol).
const LOOP_START_VOLUME: f32 = 0.05;

pub struct SoundscapePlugin;

impl Plugin for SoundscapePlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<ScapeState>()
            .add_message::<super::entities::FireEntityOutput>()
            .add_systems(
            PostUpdate,
            (
                select,
                fade_loops,
                play_randoms.run_if(resource_exists::<Assets<LiveClip>>),
            )
                .chain(),
        );
    }
}

/// Which soundscape plays and the state of its sounds.
#[derive(Resource, Default)]
pub struct ScapeState {
    /// (soundscape, what selected it) now playing.
    pub current: Option<(usize, Source)>,
    /// The current soundscape's name (debug readouts).
    pub name: Option<String>,
    /// The sound mixer it names (shown, not played).
    pub mixer: Option<String>,
    /// Zones the listener is inside, most recently entered first.
    inside: Vec<usize>,
    generation: u32,
    randoms: Vec<(usize, f64)>,
    positions: Vec<Option<Vec3>>,
    loops: Vec<LoopState>,
    next_key: u64,
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
#[derive(Clone, Debug)]
struct LoopState {
    key: SoundKey,
    clip: usize,
    pitch: f32,
    at: Option<Vec3>,
    volume: f32,
    target: f32,
}

/// Forget the selection and the room (map change; `live_sound::reset`
/// stops the loops).
pub fn reset(world: &mut World) {
    world.insert_resource(ScapeState::default());
    if let Some(mut room) = world.get_resource_mut::<RoomDsp>() {
        room.preset = 0;
        room.scape_volume = None;
        room.player = None;
    }
}

/// Source's env_soundscape rule (sounds.md "Selection on the server"):
/// the current soundscape stays current even out of range or sight; a
/// candidate that `qualifies` (in range and visible) takes over when the
/// current one doesn't qualify, or when it is closer. Candidates are
/// visited in list order, each against the one chosen so far.
pub fn choose_emitter(
    current: Option<usize>,
    count: usize,
    qualifies: impl Fn(usize) -> bool,
    distance: impl Fn(usize) -> f32,
) -> Option<usize> {
    let mut chosen = current;
    let mut in_range = current.is_some_and(&qualifies);
    for i in 0..count {
        if Some(i) == chosen || !qualifies(i) {
            continue;
        }
        if !in_range || chosen.is_some_and(|c| distance(i) < distance(c)) {
            chosen = Some(i);
            in_range = true;
        }
    }
    chosen
}

/// Whether emitter `e` may be picked: the logic's switches, else its
/// StartDisabled key.
pub fn emitter_enabled(e: &super::sound::SoundscapeEmitter, switches: Option<&super::SoundscapeSwitches>) -> bool {
    match (switches.and_then(|s| s.0.as_ref()), e.entity) {
        (Some(on), Some(entity)) => on.contains(&entity),
        _ => !e.start_disabled,
    }
}

#[allow(clippy::too_many_arguments)]
fn select(
    bank: Option<Res<SoundBank>>,
    touches: Option<Res<super::SoundscapeTouches>>,
    switches: Option<Res<super::SoundscapeSwitches>>,
    vis: Option<Res<ActiveVisibility>>,
    listener: Query<&GlobalTransform, With<SoundListener>>,
    local: Query<Entity, With<crate::core::LocalPlayer>>,
    mut state: ResMut<ScapeState>,
    mut room: ResMut<RoomDsp>,
    mut control: MessageWriter<SoundControl>,
    mut outputs: MessageWriter<super::entities::FireEntityOutput>,
    time: Res<Time>,
) {
    let (Some(bank), Some(ear)) = (bank, listener.iter().next()) else {
        return;
    };
    let sounds = &bank.0;
    let ear = ear.translation();
    // Zones: entering one makes it current; leaving all keeps the last.
    // The logic's touch code says which triggers the listener touches
    // (their exact volumes, enabled ones); without it, their boxes.
    let touched = touches.as_ref().and_then(|t| t.0.as_ref());
    let containing: Vec<usize> = sounds
        .soundscape_zones
        .iter()
        .enumerate()
        .filter(|(_, z)| match (touched, z.entity) {
            (Some(list), Some(e)) => list.contains(&e),
            _ => ear.cmpge(z.min).all() && ear.cmple(z.max).all(),
        })
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
        // Emitters: in range (radius, or unlimited) and visible (the
        // emitter's cluster potentially visible from the ear's, and a
        // clear line through the world's solid leaves).
        let vis = vis.as_ref().map(|v| &*v.0);
        let ear_cluster = vis.and_then(|v| v.cluster_at(ear));
        let emitters = &sounds.soundscape_emitters;
        let enabled = |i: usize| emitter_enabled(&emitters[i], switches.as_deref());
        let qualifies = |i: usize| {
            let e = &emitters[i];
            if !enabled(i) || !e.radius.is_none_or(|r| e.at.distance(ear) < r) {
                return false;
            }
            let Some(v) = vis else { return true };
            let pvs = match (ear_cluster, v.cluster_at(e.at)) {
                (Some(a), Some(b)) => v.sees(a, b),
                _ => true,
            };
            pvs && v.segment_clear(e.at, ear)
        };
        // A disabled current one is dropped from the choice; it keeps
        // playing until another qualifies.
        let current = match state.current {
            Some((_, Source::Emitter(i))) if enabled(i) => Some(i),
            _ => None,
        };
        match choose_emitter(current, emitters.len(), qualifies, |i| emitters[i].at.distance(ear)) {
            Some(i) => Some(Source::Emitter(i)),
            None => state.current.map(|c| c.1),
        }
    };
    let Some(source) = wanted else { return };
    if state.current.is_some_and(|c| c.1 == source) {
        return;
    }
    // An env_soundscape becoming current fires OnPlay (the local player
    // as activator).
    if let Source::Emitter(i) = source
        && let Some(map_index) = sounds.soundscape_emitters[i].entity
    {
        outputs.write(super::entities::FireEntityOutput {
            map_index,
            output: "OnPlay".into(),
            activator: local.iter().next(),
        });
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
    let def = &sounds.soundscapes[scape];
    if let Some(dsp) = def.dsp {
        room.preset = dsp;
    }
    // dsp_volume and the mixer revert when absent; dsp_player stays.
    room.scape_volume = def.dsp_volume;
    if def.dsp_player.is_some() {
        room.player = def.dsp_player;
    }
    info!(
        "soundscape: {} (dsp {})",
        def.name,
        def.dsp
            .map_or("unchanged".to_string(), |d| format!("{d}, {}", super::room::preset(d).name))
    );
    state.current = Some((scape, source));
    state.name = Some(def.name.clone());
    state.mixer = def.mixer.clone();
    state.generation += 1;
    state.positions = positions.clone();
    let now = time.elapsed_secs_f64();

    // Old loops fade out unless the new soundscape reuses them.
    for l in state.loops.iter_mut() {
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
        let reuse = state.loops.iter_mut().find(|o| {
            o.clip == l.clip
                && o.pitch == pitch
                && match (o.at, at) {
                    (None, None) => true,
                    (Some(a), Some(b)) => a.distance(b) < 0.1 * METERS_PER_UNIT,
                    _ => false,
                }
        });
        if let Some(o) = reuse {
            o.target = volume;
            continue;
        }
        state.next_key += 1;
        let key = SoundKey(SOUNDSCAPE_KEYS | state.next_key);
        let start = if at.is_some() { LOOP_START_VOLUME } else { 0.0 };
        control.write(SoundControl::Start(StartSound {
            key,
            entry: format!("{} loop", def.name),
            at,
            volume: Some(start),
            pitch: Some(pitch),
            level: Some(SoundLevel::Db(l.level)),
            clip: Some(l.clip),
            looping: true,
            dry: l.dry,
            ..default()
        }));
        state.loops.push(LoopState {
            key,
            clip: l.clip,
            pitch,
            at,
            volume: start,
            target: volume,
        });
    }
    state.randoms.clear();
    for (i, r) in def.randoms.iter().enumerate() {
        let wait = 0.5 * r.time.draw(state.unit()) as f64;
        state.randoms.push((i, now + wait));
    }
}

/// Move each loop's volume toward its target; stop the faded ones.
fn fade_loops(mut state: ResMut<ScapeState>, mut control: MessageWriter<SoundControl>, time: Res<Time>) {
    let step = time.delta_secs() / FADE_TIME;
    state.loops.retain_mut(|l| {
        let before = l.volume;
        l.volume += (l.target - l.volume).clamp(-step, step);
        if l.target == 0.0 && l.volume <= 0.0 {
            control.write(SoundControl::Stop(l.key));
            return false;
        }
        if l.volume != before {
            control.write(SoundControl::Change {
                key: l.key,
                volume: Some(l.volume),
                pitch: None,
            });
        }
        true
    });
}

#[allow(clippy::too_many_arguments)]
fn play_randoms(
    bank: Option<Res<SoundBank>>,
    listener: Query<&GlobalTransform, With<SoundListener>>,
    mut state: ResMut<ScapeState>,
    mut sources: ResMut<Assets<LiveClip>>,
    hearing: Res<Hearing>,
    room: Res<RoomDsp>,
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
        let (left, right, send) = match at {
            Some(at) => {
                let to = at - ear.translation();
                let units = to.length() / METERS_PER_UNIT;
                let g = distance_gain(SoundLevel::Db(level), units) * volume;
                let (pl, pr) = pan(to.normalize_or_zero(), ear.right().as_vec3());
                (g * pl, g * pr, distance_send(units))
            }
            None => (volume, volume, AMBIENT_SEND),
        };
        if left.max(right) < 1e-3 {
            continue;
        }
        let once = MapSoundClip {
            loop_start: None,
            ..bank.0.clips[r.clips[pick]].clone()
        };
        let gains = Gains::new(left, right).with_send(if r.dry { 0.0 } else { send });
        let handle = sources.add(LiveClip::new(once, Arc::new(gains), hearing.mix.clone()).with_room(&room));
        commands.spawn((AudioPlayer(handle), PlaybackSettings::DESPAWN.with_speed(pitch / 100.0)));
    }
}

/// The room readout: "<soundscape>  dsp <n> <preset>", then the room
/// level, player preset and mixer a soundscape set.
pub fn readout(state: &ScapeState, room: &RoomDsp) -> String {
    let p = room.active();
    let index = if room.off != 0 { 0 } else { room.preset };
    let mut out = format!(
        "soundscape: {}  dsp {index} {}{}",
        state.name.as_deref().unwrap_or("none"),
        p.name,
        if (index as usize) >= PRESETS.len() { " (unknown index)" } else { "" }
    );
    if let Some(v) = room.scape_volume {
        out += &format!("  dsp_volume {v}");
    }
    if let Some(n) = room.player {
        out += &format!("  dsp_player {n} (not played)");
    }
    if let Some(m) = &state.mixer {
        out += &format!("  mixer {m} (not played)");
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn current_emitter_stays_until_another_qualifies() {
        let dist = [5.0, 3.0, 8.0];
        let d = |i: usize| dist[i];
        // Nothing current: the first qualifying one, then closer ones.
        assert_eq!(choose_emitter(None, 3, |_| true, d), Some(1));
        assert_eq!(choose_emitter(None, 3, |i| i != 1, d), Some(0));
        // The current one out of range or sight stays current while
        // nothing else qualifies.
        assert_eq!(choose_emitter(Some(2), 3, |_| false, d), Some(2));
        // ...and is replaced by any qualifying one, even a farther one.
        assert_eq!(choose_emitter(Some(1), 3, |i| i == 2, d), Some(2));
        // A qualifying current one gives way only to a closer one.
        assert_eq!(choose_emitter(Some(0), 3, |i| i != 1, d), Some(0));
        assert_eq!(choose_emitter(Some(2), 3, |_| true, d), Some(1));
        assert_eq!(choose_emitter(None, 0, |_| true, d), None);
    }
}
