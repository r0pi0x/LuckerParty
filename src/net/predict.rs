//! A client's commands, clock and prediction (docs/plans/active/
//! multiplayer.md, slice 2; Source's model as the Valve Developer Wiki's
//! "Source Multiplayer Networking" and "Prediction" pages describe it).
//!
//! - **Commands bound to ticks.** Each fixed tick the client runs is a
//!   server tick (`CommandClock::tick`): its intent, made what the server
//!   will simulate (`NetCmd::normalize`), goes out as that tick's command,
//!   with the last `CMD_BACKUP` again (`UserCmds`). The server runs
//!   command N at its tick N (`server::apply_commands`).
//! - **Clock sync.** The server says how early the commands arrive
//!   (`OwnState::lead`); the client keeps that at `CommandClock::target`
//!   ticks (2, more with jitter) by running its fixed ticks a little faster
//!   or slower (`nudge_clock`), or jumps when far off (startup, a stall).
//!   So the client runs ahead of the server by RTT/2 plus the buffer.
//! - **Prediction.** The local player runs its commands at once through
//!   the normal tick (movement, the weapon frame: the `core::Predict`
//!   schedules; on a client only it has their components), and each tick's
//!   command and outcome are kept (`PredictionHistory`).
//! - **Reconciliation.** When the server's state after tick S arrives
//!   (`OwnState`), it's compared byte for byte with what was predicted for
//!   S. On a mismatch the server's state is put back and the commands
//!   after S run again (`core::predict`, `FirstTimePredicted` false: no
//!   sounds). The drawn eye eases the jump out over `cl_smoothtime`
//!   (`smooth_view`) unless it's a teleport.
//! - **Readout.** `NetGraph` (the perf overlay and the F2 Perf tab):
//!   ping, loss, lead, server buffer, missed commands, prediction errors.
//!   `cl_showerror 1` logs each error.

use std::{collections::VecDeque, time::Duration};

use bevy::{app::RunFixedMainLoopSystems, prelude::*};
use bevy_replicon::prelude::*;
use bevy_replicon_renet::RenetClient;

use super::{CMD_BACKUP, NetCmd, OwnState, UserCmds};
use crate::{
    console::resource_cvar,
    core::{FirstTimePredicted, Intent, LocalPlayer, NetRole, PredictedComponents, Seed, SimClock, SimSet},
    map::interp::{InterpSystems, Interpolated, RenderedView, SNAP_SPEED},
    slots::{MovementRegistry, MovementSlot, set_movement},
};

/// Ticks of history kept (about 4 s at 64 Hz): more than any round trip
/// worth predicting across.
const HISTORY: usize = 256;
/// Commands kept to resend.
const OUTGOING: usize = 32;
/// Lead error (ticks) past which the clock jumps instead of easing.
const JUMP_TICKS: f64 = 6.0;
/// Lead error (ticks) the clock lets be.
const DEADZONE: f64 = 0.4;
/// How much faster or slower the fixed ticks run per tick of lead error,
/// and at most.
const GAIN: f64 = 0.02;
const MAX_NUDGE: f64 = 0.08;

/// `cl_showerror`, `cl_smoothtime`.
#[derive(Resource, Clone, Debug)]
pub struct PredictSettings {
    /// 1: log each prediction error and clock jump.
    pub showerror: u8,
    /// Seconds the drawn eye takes to ease a correction out (Source's
    /// `cl_smoothtime`); 0 snaps.
    pub smoothtime: f32,
    /// 1: commands say when we drew others, so the server traces our
    /// shots against them there (Source's `cl_lagcompensation`).
    pub lagcompensation: u8,
}

impl Default for PredictSettings {
    fn default() -> Self {
        Self {
            showerror: 0,
            smoothtime: 0.1,
            lagcompensation: 1,
        }
    }
}

/// The client's command clock: the server tick it is predicting, and how
/// early its commands reach the server.
#[derive(Resource, Debug, Default, Clone)]
pub struct CommandClock {
    /// The tick being run (the last one); None until the server's first
    /// `OwnState` starts prediction.
    pub tick: Option<u64>,
    /// The server's time base: a tick, its time and the tick length.
    base: Option<(u64, Duration, Duration)>,
    /// The commands' lead at the server, ticks, smoothed; its mean
    /// deviation (jitter); samples since the clock last jumped.
    pub lead: f64,
    pub jitter: f64,
    pub samples: u32,
    /// Leads count only for commands from this tick on (sent after the
    /// last jump).
    settle_from: u64,
    /// How much faster (+) the ticks ran this frame.
    pub nudge: f64,
    pub jumps: u32,
}

impl CommandClock {
    /// The lead to keep, ticks: 2 (a tick of buffer), more with jitter.
    pub fn target(&self) -> f64 {
        (2.0 + 2.0 * self.jitter).min(16.0)
    }

    fn sample(&mut self, lead: f64) {
        if self.samples == 0 {
            self.lead = lead;
            self.jitter = 0.0;
        } else {
            let d = lead - self.lead;
            self.lead += 0.1 * d;
            self.jitter += 0.05 * (d.abs() - self.jitter);
        }
        self.samples += 1;
    }

    /// The server's tick length, once its first state has come.
    pub fn step(&self) -> Option<Duration> {
        self.base.map(|b| b.2)
    }

    /// The simulation clock the server has (had) at `tick`.
    pub fn clock_for(&self, tick: u64) -> Option<SimClock> {
        let (base_tick, base_time, step) = self.base?;
        let now = if tick >= base_tick {
            base_time + step * (tick - base_tick) as u32
        } else {
            base_time.saturating_sub(step * (base_tick - tick) as u32)
        };
        Some(SimClock {
            tick,
            now: now.as_secs_f64(),
            delta: step,
        })
    }
}

/// One predicted tick: the command as simulated (held, numbered), the
/// clock it ran at, and the outcome (`PredictedComponents::encode`).
#[derive(Clone, Debug)]
pub struct Predicted {
    pub tick: u64,
    pub intent: Intent,
    pub clock: SimClock,
    pub state: Vec<u8>,
    pub position: Vec3,
}

/// The ticks predicted since the server's last state, oldest first.
#[derive(Resource, Default, Debug)]
pub struct PredictionHistory(pub VecDeque<Predicted>);

/// Commands to send (newest last) and how many are new.
#[derive(Resource, Default)]
struct Outgoing {
    cmds: VecDeque<NetCmd>,
    fresh: usize,
}

/// The newest server state not yet compared, and the last one that was.
#[derive(Resource, Default)]
struct Pending {
    state: Option<OwnState>,
    done: u64,
}

/// The rules hold our player (dead, freeze time), as the server said.
#[derive(Resource, Default)]
struct Held(bool);

/// This tick's intent before the rules held it, put back after the tick
/// (the input owns `Intent`; a held tick mustn't erase it).
#[derive(Resource, Default)]
struct RawIntent(Option<Intent>);

/// The drawn eye's error being eased out: `from` at `since` (real s),
/// down to nothing over `cl_smoothtime`.
#[derive(Resource, Default, Debug, Clone, Copy)]
pub struct Smoothing {
    from: Vec3,
    since: f64,
}

impl Smoothing {
    /// When the latest correction started easing out (real s).
    pub fn since(&self) -> f64 {
        self.since
    }

    pub fn offset(&self, now: f64, smoothtime: f32) -> Vec3 {
        if smoothtime <= 0.0 {
            return Vec3::ZERO;
        }
        let left = 1.0 - ((now - self.since) / smoothtime as f64).clamp(0.0, 1.0);
        self.from * left as f32
    }
}

/// The client's network readout (Source's `net_graph`, plus prediction).
#[derive(Resource, Default, Debug, Clone)]
pub struct NetGraph {
    pub ping_ms: f64,
    pub loss: f64,
    pub in_kbs: f64,
    pub out_kbs: f64,
    pub lead: f64,
    pub target: f64,
    pub nudge: f64,
    pub clock_jumps: u32,
    /// From the server: our commands waiting there, ticks run without one,
    /// commands that came too late.
    pub buffered: u16,
    pub missed: u32,
    pub late: u32,
    /// Server states compared with the prediction, ones that differed
    /// (prediction errors), and restarts without a prediction to compare
    /// (joining, after a clock jump).
    pub checked: u64,
    pub errors: u64,
    pub resyncs: u64,
    /// Position error of the last and the worst prediction error, m.
    pub last_error: f32,
    pub worst_error: f32,
    /// Commands run again after corrections.
    pub replayed: u64,
    /// Errors in the last second (real time) and their worst, m.
    pub errors_per_sec: usize,
    pub worst_recent: f32,
    recent: VecDeque<(f64, f32)>,
    /// The drawn eye's remaining correction, m.
    pub smoothing: f32,
    /// Others drawn from snapshots (`interp`): how far in the past (ms),
    /// the time between server updates (ms), characters drawn, snapshots
    /// newer than the render time (per character), how far ahead of it
    /// the newest tick heard is (ms).
    pub interp_ms: f64,
    pub update_ms: f64,
    pub interp_drawn: usize,
    pub interp_ahead: f64,
    pub interp_newest_ms: f64,
    /// Frames drawn, frames with someone past the newest snapshot
    /// (extrapolated, then held), teleports drawn at once.
    pub interp_frames: u64,
    pub interp_extrapolated: u64,
    pub interp_held: u64,
    pub interp_snaps: u64,
    /// Moving brushes the client steps itself (`movers`), and ticks it
    /// carried its own player on one.
    pub movers: usize,
    pub carried: u64,
}

impl NetGraph {
    pub fn net_line(&self) -> String {
        format!(
            "net: ping {:.0} ms, loss {:.1}%, in {:.1} KB/s, out {:.1} KB/s",
            self.ping_ms,
            self.loss * 100.0,
            self.in_kbs,
            self.out_kbs
        )
    }

    pub fn clock_line(&self) -> String {
        format!(
            "cmds: lead {:.1} ticks (target {:.1}, {:+.0}% speed, {} jumps), server buffer {}, missed {}, late {}",
            self.lead,
            self.target,
            self.nudge * 100.0,
            self.clock_jumps,
            self.buffered,
            self.missed,
            self.late
        )
    }

    pub fn prediction_line(&self) -> String {
        format!(
            "prediction: {} errors/s (worst {:.3} m), {} of {} states wrong, last {:.3} m, {} replayed, easing {:.3} m",
            self.errors_per_sec,
            self.worst_recent,
            self.errors,
            self.checked,
            self.last_error,
            self.replayed,
            self.smoothing
        )
    }

    pub fn interp_line(&self) -> String {
        format!(
            "interp: {:.0} ms behind (updates every {:.1} ms), {} drawn, {:.1} snapshots ahead (newest +{:.0} ms), \
             extrapolated {} / held {} of {} frames, {} snaps; {} movers, carried {} ticks",
            self.interp_ms,
            self.update_ms,
            self.interp_drawn,
            self.interp_ahead,
            self.interp_newest_ms,
            self.interp_extrapolated,
            self.interp_held,
            self.interp_frames,
            self.interp_snaps,
            self.movers,
            self.carried
        )
    }

    pub fn lines(&self) -> [String; 4] {
        [
            self.net_line(),
            self.clock_line(),
            self.prediction_line(),
            self.interp_line(),
        ]
    }
}

pub(super) fn plugin(app: &mut App) {
    let client = || resource_equals(NetRole::Client);
    app.init_resource::<PredictSettings>()
        .init_resource::<CommandClock>()
        .init_resource::<PredictionHistory>()
        .init_resource::<Outgoing>()
        .init_resource::<Pending>()
        .init_resource::<Held>()
        .init_resource::<RawIntent>()
        .init_resource::<Smoothing>()
        .init_resource::<NetGraph>()
        .add_systems(
            PreUpdate,
            receive_own_states.after(ClientSystems::Receive).run_if(client()),
        )
        .add_systems(
            RunFixedMainLoop,
            nudge_clock
                .in_set(RunFixedMainLoopSystems::BeforeFixedMainLoop)
                .run_if(client()),
        )
        .add_systems(
            FixedFirst,
            start_command
                .after(crate::core::start_tick)
                .after(InterpSystems::Restore)
                .run_if(client()),
        )
        .add_systems(
            FixedUpdate,
            (
                capture_command.before(SimSet::Rules),
                hold_local.in_set(SimSet::Rules),
                record_command.after(SimSet::Commands).before(SimSet::Movement),
            )
                .run_if(client()),
        )
        .add_systems(FixedLast, record_state.run_if(client()))
        .add_systems(
            RunFixedMainLoop,
            smooth_view
                .in_set(RunFixedMainLoopSystems::AfterFixedMainLoop)
                .after(InterpSystems::Ease)
                .run_if(client()),
        )
        .add_systems(Update, update_graph.run_if(client()))
        .add_systems(PostUpdate, send_commands.before(ClientSystems::Send).run_if(client()));
    resource_cvar::<PredictSettings, u8>(
        app,
        "cl_showerror",
        "1: log each prediction error (the server's state of your player differed from what was predicted) and each clock jump.",
        |s| &mut s.showerror,
    );
    resource_cvar::<PredictSettings, u8>(
        app,
        "cl_lagcompensation",
        "1: the server traces your shots against others where you saw them (lag compensation); 0: where they are.",
        |s| &mut s.lagcompensation,
    );
    resource_cvar::<PredictSettings, f32>(
        app,
        "cl_smoothtime",
        "Seconds the view takes to ease out a prediction correction (0 snaps).",
        |s| &mut s.smoothtime,
    );
}

/// Back to not predicting (left the server).
pub(super) fn reset(world: &mut World) {
    // The tick length without the clock sync's nudge.
    if let Some((_, _, step)) = world.resource::<CommandClock>().base {
        world.resource_mut::<Time<Fixed>>().set_timestep(step);
    }
    world.insert_resource(CommandClock::default());
    world.insert_resource(PredictionHistory::default());
    world.insert_resource(Outgoing::default());
    world.insert_resource(Pending::default());
    world.insert_resource(Held::default());
    world.insert_resource(RawIntent::default());
    world.insert_resource(Smoothing::default());
    world.insert_resource(NetGraph::default());
}

/// Our own player, once it has a movement (from the server's state).
pub fn predicted_player(world: &mut World) -> Option<Entity> {
    world
        .query_filtered::<Entity, (With<LocalPlayer>, With<MovementSlot>)>()
        .iter(world)
        .next()
}

/// The server's states of our player: keep the newest, feed the clock
/// sync, mirror its movement implementation, seed and hold.
#[allow(clippy::too_many_arguments)]
fn receive_own_states(
    mut states: MessageReader<OwnState>,
    mut clock: ResMut<CommandClock>,
    mut pending: ResMut<Pending>,
    mut graph: ResMut<NetGraph>,
    mut held: ResMut<Held>,
    player: Option<Single<(Entity, Option<&MovementSlot>, Option<&Seed>), With<LocalPlayer>>>,
    registry: Option<Res<MovementRegistry>>,
    mut commands: Commands,
) {
    let mut newest: Option<OwnState> = None;
    for s in states.read() {
        if let Some(lead) = s.lead
            && clock.tick.is_some()
            && s.newest >= clock.settle_from
        {
            clock.sample(lead as f64);
        }
        if newest.as_ref().is_none_or(|n| s.tick > n.tick) {
            newest = Some(s.clone());
        }
    }
    let Some(s) = newest else { return };
    let current = pending
        .state
        .as_ref()
        .map_or(pending.done, |p| p.tick.max(pending.done));
    if s.tick <= current {
        return;
    }
    graph.buffered = s.buffered;
    graph.missed = s.missed;
    graph.late = s.late;
    held.0 = s.held;
    if let Some(p) = player {
        let (e, slot, seed) = *p;
        if slot.is_none_or(|m| m.0 != s.movement)
            && let Some(id) = registry.as_ref().and_then(|r| r.get(&s.movement)).map(|m| m.id)
        {
            commands.queue(set_movement(e, id));
        }
        if seed.is_none_or(|x| x.0 != s.seed) {
            commands.entity(e).insert(Seed(s.seed));
        }
    }
    pending.state = Some(s);
}

/// Keep the commands' lead at the target: tick a little faster or slower,
/// or jump when far off.
fn nudge_clock(
    mut clock: ResMut<CommandClock>,
    mut fixed: ResMut<Time<Fixed>>,
    mut history: ResMut<PredictionHistory>,
    mut outgoing: ResMut<Outgoing>,
    settings: Res<PredictSettings>,
) {
    let Some(tick) = clock.tick else { return };
    let err = if clock.samples == 0 {
        0.0
    } else {
        clock.lead - clock.target()
    };
    if err.abs() > JUMP_TICKS {
        let to = (tick as i64 - err.round() as i64).max(1) as u64;
        if settings.showerror > 0 {
            info!(
                "command clock jumps {:+} ticks (lead {:.1}, target {:.1})",
                to as i64 - tick as i64,
                clock.lead,
                clock.target()
            );
        }
        clock.tick = Some(to);
        clock.settle_from = to + 1;
        clock.samples = 0;
        clock.jumps += 1;
        clock.nudge = 0.0;
        // Predictions for ticks we no longer run: the next server state
        // starts over.
        history.0.clear();
        outgoing.cmds.clear();
        outgoing.fresh = 0;
        if let Some((_, _, step)) = clock.base {
            fixed.set_timestep(step);
        }
        return;
    }
    // Ahead (err > 0): longer ticks, so fewer of them per second. Only
    // the real time between ticks changes: the simulation's tick length
    // stays the server's (`SimClock`, set per command).
    let f = if err.abs() <= DEADZONE {
        0.0
    } else {
        (err * GAIN).clamp(-MAX_NUDGE, MAX_NUDGE)
    };
    if let Some((_, _, step)) = clock.base {
        let want = step.mul_f64(1.0 + f);
        if fixed.timestep() != want {
            fixed.set_timestep(want);
        }
    }
    clock.nudge = -f;
}

/// Start of a tick on a client: the next command tick and its clock; the
/// server's newest state compared with what we predicted for it.
fn start_command(world: &mut World) {
    let Some(player) = predicted_player(world) else {
        return;
    };
    let pending = world.resource_mut::<Pending>().state.take();
    if let Some(s) = &pending {
        world.resource_mut::<Pending>().done = s.tick;
        let step = Duration::from_nanos(s.tick_nanos);
        world.resource_mut::<CommandClock>().base = Some((s.tick, Duration::from_nanos(s.time_nanos), step));
    }
    let started = world.resource::<CommandClock>().tick;
    let tick = match started {
        Some(t) => t + 1,
        None => {
            // The first state: start at the server's tick plus the round
            // trip and the buffer; the lead feedback corrects the guess.
            let Some(s) = &pending else { return };
            let step = (s.tick_nanos as f64 * 1e-9).max(1e-4);
            let rtt = world.get_resource::<RenetClient>().map_or(0.0, |c| c.rtt());
            let target = world.resource::<CommandClock>().target();
            let tick = s.tick + (rtt / step).ceil() as u64 + target.round() as u64;
            world.resource_mut::<CommandClock>().settle_from = tick;
            tick
        }
    };
    world.resource_mut::<CommandClock>().tick = Some(tick);
    if let Some(s) = pending {
        reconcile(world, player, s);
    }
    if let Some(clock) = world.resource::<CommandClock>().clock_for(tick) {
        world.insert_resource(clock);
    }
}

/// Compare the server's state after tick S with ours for S; on a
/// mismatch take the server's and run the commands since again.
fn reconcile(world: &mut World, player: Entity, server: OwnState) {
    let index = world
        .resource::<PredictionHistory>()
        .0
        .iter()
        .position(|p| p.tick == server.tick);
    // What we predicted for S, when it differs: its position and the
    // components that differ.
    let mut error: Option<(Vec3, Vec<&'static str>)> = None;
    if let Some(i) = index {
        let ours = &world.resource::<PredictionHistory>().0[i];
        if ours.state == server.state {
            world.resource_mut::<NetGraph>().checked += 1;
            world.resource_mut::<PredictionHistory>().0.drain(..=i);
            return;
        }
        let names = world
            .resource::<PredictedComponents>()
            .differing(&ours.state, &server.state);
        error = Some((ours.position, names));
    } else if world
        .resource::<PredictionHistory>()
        .0
        .front()
        .is_some_and(|p| p.tick > server.tick)
    {
        // Older than anything we predicted (from before prediction started
        // or the clock jumped): newer states will tell.
        return;
    }
    let before = world
        .get::<Transform>(player)
        .map(|t| t.translation)
        .unwrap_or_default();
    let decoded =
        world.resource_scope(|world, registry: Mut<PredictedComponents>| registry.decode(world, player, &server.state));
    if let Err(e) = decoded {
        warn!("the server's state of our player didn't decode: {e}");
        return;
    }
    // What it stands on isn't in the blob (an entity): the mover by its
    // map index, so the replay carries it as the server did.
    super::movers::restore_ground(world, player, server.ground);
    let server_pos = world
        .get::<Transform>(player)
        .map(|t| t.translation)
        .unwrap_or_default();
    // The commands after S, again.
    let replay: Vec<Predicted> = {
        let mut h = world.resource_mut::<PredictionHistory>();
        match index {
            Some(i) => {
                let rest: Vec<Predicted> = h.0.drain(..).skip(i + 1).collect();
                rest
            }
            None => {
                // Nothing to compare with (joining, after a jump, or the
                // server ahead of us): its state, and what we predicted
                // since that is newer.
                let rest: Vec<Predicted> = h.0.drain(..).filter(|p| p.tick > server.tick).collect();
                rest
            }
        }
    };
    let live_intent = world.get::<Intent>(player).cloned();
    let live_clock = *world.resource::<SimClock>();
    world.insert_resource(FirstTimePredicted(false));
    let mut redone = VecDeque::with_capacity(replay.len());
    for mut p in replay {
        if let Some(mut i) = world.get_mut::<Intent>(player) {
            *i = p.intent.clone();
        }
        world.insert_resource(p.clock);
        super::movers::place(world);
        crate::core::predict(world);
        super::canonical_position(world, player);
        p.state = world.resource::<PredictedComponents>().encode(world, player);
        p.position = world
            .get::<Transform>(player)
            .map(|t| t.translation)
            .unwrap_or_default();
        redone.push_back(p);
    }
    world.insert_resource(FirstTimePredicted(true));
    world.insert_resource(live_clock);
    if let Some(i) = live_intent
        && let Some(mut intent) = world.get_mut::<Intent>(player)
    {
        *intent = i;
    }
    let replayed = redone.len();
    world.resource_mut::<PredictionHistory>().0 = redone;
    let after = world
        .get::<Transform>(player)
        .map(|t| t.translation)
        .unwrap_or_default();

    // The drawn eye: eased from where it was, unless that's a teleport.
    let jump = after - before;
    let now = world.resource::<Time<Real>>().elapsed_secs_f64();
    let step = world
        .resource::<CommandClock>()
        .base
        .map_or(1.0 / 64.0, |b| b.2.as_secs_f32());
    let smoothtime = world.resource::<PredictSettings>().smoothtime;
    let mut smoothing = *world.resource::<Smoothing>();
    let left = smoothing.offset(now, smoothtime) - jump;
    smoothing = if left.length() > SNAP_SPEED * step {
        Smoothing::default()
    } else {
        Smoothing { from: left, since: now }
    };
    world.insert_resource(smoothing);
    if let Some(mut interp) = world.get_mut::<Interpolated>(player) {
        interp.shift(jump);
    }

    let mut graph = world.resource_mut::<NetGraph>();
    graph.replayed += replayed as u64;
    match error {
        Some((predicted, names)) => {
            // How far off the prediction for S was.
            let dist = predicted.distance(server_pos);
            graph.checked += 1;
            graph.errors += 1;
            graph.last_error = dist;
            graph.worst_error = graph.worst_error.max(dist);
            graph.recent.push_back((now, dist));
            if world.resource::<PredictSettings>().showerror > 0 {
                info!(
                    "prediction error at tick {}: {:.4} m off ({}); replayed {replayed} commands, view moved {:.4} m",
                    server.tick,
                    dist,
                    names.join(", "),
                    jump.length()
                );
            }
        }
        None => {
            graph.resyncs += 1;
            if world.resource::<PredictSettings>().showerror > 0 {
                info!(
                    "prediction restarts from the server's tick {} (nothing predicted for it)",
                    server.tick
                );
            }
        }
    }
}

/// Before the rules: this tick's intent made what the server will
/// simulate, kept to send as the tick's command.
fn capture_command(
    clock: Res<CommandClock>,
    mut player: Option<Single<&mut Intent, (With<LocalPlayer>, With<MovementSlot>)>>,
    mut outgoing: ResMut<Outgoing>,
    mut raw: ResMut<RawIntent>,
    interp: Res<super::interp::InterpClock>,
    settings: Res<PredictSettings>,
) {
    let (Some(tick), Some(intent)) = (clock.tick, player.as_deref_mut()) else {
        return;
    };
    let mut cmd = NetCmd::normalize(tick, intent);
    // Others as drawn when this command was made (the last frame's
    // render time): our shots are traced against them there.
    if interp.running && settings.lagcompensation != 0 {
        cmd.view_tick = interp.render_tick;
    }
    raw.0 = Some(intent.clone());
    if outgoing.cmds.back().is_some_and(|c| c.tick >= tick) {
        outgoing.cmds.clear();
    }
    outgoing.cmds.push_back(cmd);
    outgoing.fresh += 1;
    while outgoing.cmds.len() > OUTGOING {
        outgoing.cmds.pop_front();
    }
}

/// The rules hold our player as the server's do (`rules::hold_the_dead`,
/// freeze time): nothing but the look.
fn hold_local(held: Res<Held>, mut player: Option<Single<&mut Intent, (With<LocalPlayer>, With<MovementSlot>)>>) {
    if !held.0 {
        return;
    }
    if let Some(intent) = player.as_deref_mut() {
        let (yaw, pitch, command) = (intent.yaw, intent.pitch, intent.command);
        **intent = Intent {
            yaw,
            pitch,
            command,
            ..default()
        };
    }
}

/// The command as simulated this tick (held, numbered), into the history.
pub(super) fn record_command(
    clock: Res<CommandClock>,
    sim: Res<SimClock>,
    player: Option<Single<&Intent, (With<LocalPlayer>, With<MovementSlot>)>>,
    mut history: ResMut<PredictionHistory>,
) {
    let (Some(tick), Some(intent)) = (clock.tick, player) else {
        return;
    };
    if history.0.back().is_some_and(|p| p.tick >= tick) {
        history.0.clear();
    }
    history.0.push_back(Predicted {
        tick,
        intent: intent.clone(),
        clock: *sim,
        state: Vec::new(),
        position: Vec3::ZERO,
    });
    while history.0.len() > HISTORY {
        history.0.pop_front();
    }
}

/// End of the tick: its outcome into the history; the input's intent back.
fn record_state(world: &mut World) {
    let Some(tick) = world.resource::<CommandClock>().tick else {
        return;
    };
    let Some(player) = predicted_player(world) else {
        return;
    };
    super::canonical_position(world, player);
    let state = world.resource::<PredictedComponents>().encode(world, player);
    let position = world
        .get::<Transform>(player)
        .map(|t| t.translation)
        .unwrap_or_default();
    if let Some(p) = world.resource_mut::<PredictionHistory>().0.back_mut()
        && p.tick == tick
    {
        p.state = state;
        p.position = position;
    }
    if let Some(raw) = world.resource_mut::<RawIntent>().0.take()
        && let Some(mut intent) = world.get_mut::<Intent>(player)
    {
        *intent = raw;
    }
}

/// The drawn eye carries what's left of the last corrections.
fn smooth_view(
    smoothing: Res<Smoothing>,
    settings: Res<PredictSettings>,
    time: Res<Time<Real>>,
    mut player: Option<Single<&mut RenderedView, With<LocalPlayer>>>,
    mut graph: ResMut<NetGraph>,
) {
    let offset = smoothing.offset(time.elapsed_secs_f64(), settings.smoothtime);
    graph.smoothing = offset.length();
    if offset == Vec3::ZERO {
        return;
    }
    if let Some(v) = player.as_deref_mut() {
        v.now.eye_offset += offset;
    }
}

/// New commands, with the last few again, to the server.
fn send_commands(mut outgoing: ResMut<Outgoing>, mut out: MessageWriter<UserCmds>) {
    if outgoing.fresh == 0 {
        return;
    }
    let n = (outgoing.fresh + CMD_BACKUP).min(outgoing.cmds.len());
    let start = outgoing.cmds.len() - n;
    let cmds: Vec<NetCmd> = outgoing.cmds.iter().skip(start).cloned().collect();
    outgoing.fresh = 0;
    out.write(UserCmds { cmds });
}

/// The readout's transport numbers and error rate.
fn update_graph(
    client: Option<Res<RenetClient>>,
    clock: Res<CommandClock>,
    time: Res<Time<Real>>,
    mut graph: ResMut<NetGraph>,
) {
    if let Some(c) = client {
        let info = c.network_info();
        graph.ping_ms = info.rtt * 1000.0;
        graph.loss = info.packet_loss;
        graph.in_kbs = info.bytes_received_per_second / 1000.0;
        graph.out_kbs = info.bytes_sent_per_second / 1000.0;
    }
    graph.lead = clock.lead;
    graph.target = clock.target();
    graph.nudge = clock.nudge;
    graph.clock_jumps = clock.jumps;
    let now = time.elapsed_secs_f64();
    while graph.recent.front().is_some_and(|(t, _)| now - t > 1.0) {
        graph.recent.pop_front();
    }
    graph.errors_per_sec = graph.recent.len();
    graph.worst_recent = graph.recent.iter().map(|(_, d)| *d).fold(0.0, f32::max);
}
