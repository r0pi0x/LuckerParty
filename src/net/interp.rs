//! Others drawn in the past from snapshots (docs/plans/active/
//! multiplayer.md, slice 3; Source's entity interpolation as the Valve
//! Developer Wiki's "Source Multiplayer Networking" page describes it).
//!
//! - **Snapshots by server tick.** Every replicated value a client
//!   receives for a `NetBody` (characters), `NetMover` (moving brushes,
//!   `movers`) or `NetProp` (physics props, `props`) also goes into the
//!   entity's `Snapshots`, keyed by the server tick it is from
//!   (replicon's message tick, which the server keeps equal to its
//!   `SimClock::tick`: `server::lock_replication_tick`). The server sends
//!   a mutate message every tick even when nothing changed
//!   (`track_mutate_messages`), so a tick heard without a value for an
//!   entity is a tick it held still at (`Snapshots::hold`).
//! - **The render clock.** When ticks arrive (`MutateTickReceived`) gives
//!   the server's clock as seen here (`InterpClock`: an average of tick
//!   time less arrival time, eased in at most 5 % of real time so it never
//!   steps). Others are drawn at that time less the interpolation delay:
//!   `max(cl_interp, cl_interp_ratio × update interval)` (0.1 s, 2). The
//!   client's own player runs ahead (predicted, `predict`); others lag
//!   behind by the delay plus the latency, as in Source.
//! - **Drawing.** Each frame, between the two snapshots around the render
//!   time (linear: positions, velocity, eye height; look the short way
//!   round), into the same `Transform`, `RenderedView`, `MovementState`,
//!   `Velocity` and look (`Intent`) that bodies, cameras and animation
//!   read for any character (`draw_others`). Such characters are
//!   `map::interp::NetDrawn`: the tick blend leaves them alone. A gap
//!   (nothing newer than the render time) extrapolates along the last
//!   velocity for at most `cl_extrapolate_amount` (0.25 s), then holds; a
//!   jump between two snapshots no body makes in that time
//!   (`map::interp::SNAP_SPEED`: teleports) or a death or respawn between
//!   them is drawn at once at the newer one's tick.
//! - **Readout.** `NetGraph`'s `interp:` line (perf overlay, F2 Perf tab).

use std::collections::VecDeque;

use bevy::{app::RunFixedMainLoopSystems, ecs::component::Mutable, prelude::*};
use bevy_replicon::{
    bytes::Bytes,
    client::server_mutate_ticks::MutateTickReceived,
    prelude::*,
    shared::replication::{
        deferred_entity::DeferredEntity,
        registry::ctx::{RemoveCtx, WriteCtx},
    },
};

use super::{NetBody, NetMover, NetProp, body_flags, predict::NetGraph};
use crate::{
    console::resource_cvar,
    core::{Intent, LocalPlayer, MovementState, NetRole, Velocity},
    map::interp::{EyeView, InterpSystems, NetDrawn, RenderedView, SNAP_SPEED},
    rules::Dead,
};

/// Snapshots kept per entity (2 s at 64 Hz): more than any delay.
pub const MAX_SAMPLES: usize = 128;
/// How fast the render clock may run faster or slower than real time
/// while it catches up with the measured server clock.
const SLEW: f64 = 0.05;
/// Weight of each heard tick in the server clock and update interval
/// averages.
const CLOCK_GAIN: f64 = 0.05;
/// Off by more than this (s), the render clock jumps (joining, a stall).
const CLOCK_JUMP: f64 = 0.25;

/// A replicated value by the server tick it is from, oldest first (the
/// values received, and copies at ticks it was heard unchanged at).
#[derive(Component, Debug, Clone)]
pub struct Snapshots<T: Send + Sync + 'static> {
    samples: VecDeque<(u64, T)>,
    /// The newer tick of the last teleport counted (`NetGraph::snaps`).
    snapped: u64,
}

impl<T: Clone + Send + Sync + 'static> Snapshots<T> {
    pub fn new(tick: u64, value: T) -> Self {
        Self {
            samples: VecDeque::from([(tick, value)]),
            snapped: 0,
        }
    }

    pub fn samples(&self) -> &VecDeque<(u64, T)> {
        &self.samples
    }

    pub fn newest(&self) -> Option<&(u64, T)> {
        self.samples.back()
    }

    /// The value received for `tick` (replacing one for the same tick),
    /// in tick order.
    pub fn push(&mut self, tick: u64, value: T) {
        let i = self.samples.partition_point(|(t, _)| *t < tick);
        if self.samples.get(i).is_some_and(|(t, _)| *t == tick) {
            self.samples[i].1 = value;
        } else {
            self.samples.insert(i, (tick, value));
        }
        while self.samples.len() > MAX_SAMPLES {
            self.samples.pop_front();
        }
    }

    /// The server sent nothing for this entity at `tick`: it held still.
    /// Only after the newest value (an older tick heard late says nothing
    /// sure: a value for it may have been dropped as out of date).
    pub fn hold(&mut self, tick: u64) {
        if let Some((t, v)) = self.samples.back()
            && *t < tick
        {
            let v = v.clone();
            self.push(tick, v);
        }
    }

    /// The newest value at or before `tick`, else the oldest.
    pub fn at_or_before(&self, tick: u64) -> Option<&(u64, T)> {
        let i = self.samples.partition_point(|(t, _)| *t <= tick);
        if i == 0 { self.samples.front() } else { self.samples.get(i - 1) }
    }

    /// The two values around render tick `at`.
    pub fn around(&self, at: f64) -> Around<'_, T> {
        let i = self.samples.partition_point(|(t, _)| (*t as f64) <= at);
        match (i.checked_sub(1).and_then(|j| self.samples.get(j)), self.samples.get(i)) {
            (Some(a), Some(b)) => {
                let f = (at - a.0 as f64) / (b.0 - a.0) as f64;
                Around::Between(a, b, f.clamp(0.0, 1.0) as f32)
            }
            (Some(a), None) => Around::After(a),
            (None, Some(b)) => Around::Before(b),
            (None, None) => Around::Empty,
        }
    }

    /// Forget values older than the last one at or before tick `at`.
    pub fn prune(&mut self, at: f64) {
        while self.samples.len() > 2 && self.samples.get(1).is_some_and(|(t, _)| (*t as f64) <= at) {
            self.samples.pop_front();
        }
    }

    /// How many values are newer than tick `at`.
    pub fn ahead_of(&self, at: f64) -> usize {
        self.samples.len() - self.samples.partition_point(|(t, _)| (*t as f64) <= at)
    }
}

/// Where a render time falls in a snapshot buffer.
#[derive(Debug)]
pub enum Around<'a, T> {
    Empty,
    /// Before the oldest value.
    Before(&'a (u64, T)),
    /// Between two values, this far from the first (0..1).
    Between(&'a (u64, T), &'a (u64, T), f32),
    /// After the newest (a gap: extrapolate or hold).
    After(&'a (u64, T)),
}

/// Replicon's write for a component a client buffers (`NetPlugin`):
/// the value goes into the entity's `Snapshots` by its message tick, and
/// is the component's (newest) value as usual.
pub fn write_snapshot<C: Component<Mutability = Mutable> + Clone>(
    ctx: &mut WriteCtx,
    rule_fns: &RuleFns<C>,
    entity: &mut DeferredEntity,
    message: &mut Bytes,
) -> Result<()> {
    let value: C = rule_fns.deserialize(ctx, message)?;
    let tick = ctx.message_tick.get() as u64;
    if let Some(mut s) = entity.get_mut::<Snapshots<C>>() {
        s.push(tick, value.clone());
    } else {
        entity.insert(Snapshots::new(tick, value.clone()));
    }
    if let Some(mut c) = entity.get_mut::<C>() {
        *c = value;
    } else {
        entity.insert(value);
    }
    Ok(())
}

/// The component and its snapshots go together.
pub fn remove_snapshots<C: Component>(_ctx: &mut RemoveCtx, entity: &mut DeferredEntity) {
    entity.remove::<C>();
    entity.remove::<Snapshots<C>>();
}

/// `cl_interp`, `cl_interp_ratio`, `cl_extrapolate`,
/// `cl_extrapolate_amount` (Source's names and defaults).
#[derive(Resource, Clone, Debug)]
pub struct InterpSettings {
    /// Seconds others are drawn in the past, at least.
    pub interp: f32,
    /// Update intervals others are drawn in the past, at least (2: one
    /// lost update still leaves two to draw between).
    pub interp_ratio: f32,
    /// 1: past the newest snapshot, carry on along its velocity.
    pub extrapolate: u8,
    /// For at most this long (s), then hold.
    pub extrapolate_amount: f32,
}

impl Default for InterpSettings {
    fn default() -> Self {
        Self {
            interp: 0.1,
            interp_ratio: 2.0,
            extrapolate: 1,
            extrapolate_amount: 0.25,
        }
    }
}

/// The client's view of the server's clock, from when its ticks arrive,
/// and the render time others are drawn at.
#[derive(Resource, Debug, Default, Clone)]
pub struct InterpClock {
    /// Server time less real time (s): the average of what arrivals say,
    /// and the value in use (eased toward it).
    target: Option<f64>,
    offset: f64,
    /// The newest tick heard and the average time between heard ticks (s).
    pub newest: u64,
    pub interval: f64,
    /// Real time of the last frame drawn.
    last: Option<f64>,
    /// The server tick drawn this frame (fractional), and the delay (s).
    pub render_tick: f64,
    pub delay: f64,
    /// Whether `render_tick` is set (ticks heard).
    pub running: bool,
    pub jumps: u32,
}

impl InterpClock {
    /// A tick heard at real time `now` (s), ticks `step` s long.
    pub fn hear(&mut self, tick: u64, now: f64, step: f64) {
        let sample = tick as f64 * step - now;
        match self.target {
            Some(t) if (sample - t).abs() <= CLOCK_JUMP => {
                self.target = Some(t + CLOCK_GAIN * (sample - t));
            }
            _ => {
                if self.target.is_some() {
                    self.jumps += 1;
                }
                self.target = Some(sample);
                self.offset = sample;
            }
        }
        if tick > self.newest {
            if self.newest > 0 && self.interval > 0.0 {
                let gap = (tick - self.newest) as f64 * step;
                self.interval += CLOCK_GAIN * (gap - self.interval);
            } else {
                self.interval = step;
            }
            self.newest = tick;
        }
    }

    /// This frame's render tick at real time `now`.
    pub fn advance(&mut self, now: f64, step: f64, settings: &InterpSettings) {
        let Some(target) = self.target else { return };
        let dt = self.last.map_or(0.0, |l| (now - l).max(0.0));
        self.last = Some(now);
        let room = SLEW * dt;
        self.offset += (target - self.offset).clamp(-room, room);
        self.delay = (settings.interp as f64).max(settings.interp_ratio as f64 * self.interval);
        self.render_tick = (now + self.offset - self.delay) / step;
        self.running = true;
    }
}

pub(super) fn plugin(app: &mut App) {
    let client = || resource_equals(NetRole::Client);
    app.init_resource::<InterpSettings>()
        .init_resource::<InterpClock>()
        .add_systems(
            PreUpdate,
            hear_ticks.after(ClientSystems::Receive).run_if(client()),
        )
        .add_systems(
            RunFixedMainLoop,
            draw_others
                .in_set(RunFixedMainLoopSystems::AfterFixedMainLoop)
                .after(InterpSystems::Ease)
                .run_if(client()),
        );
    resource_cvar::<InterpSettings, f32>(
        app,
        "cl_interp",
        "Seconds other players and objects are drawn in the past, at least (between two snapshots from the \
         server); also at least cl_interp_ratio update intervals.",
        |s| &mut s.interp,
    );
    resource_cvar::<InterpSettings, f32>(
        app,
        "cl_interp_ratio",
        "Update intervals others are drawn in the past, at least (2: one lost update still leaves two \
         snapshots to draw between).",
        |s| &mut s.interp_ratio,
    );
    resource_cvar::<InterpSettings, u8>(
        app,
        "cl_extrapolate",
        "1: when no snapshot is newer than the render time, others carry on along their velocity \
         (cl_extrapolate_amount), then hold; 0: hold at once.",
        |s| &mut s.extrapolate,
    );
    resource_cvar::<InterpSettings, f32>(
        app,
        "cl_extrapolate_amount",
        "Seconds others may be extrapolated past the newest snapshot.",
        |s| &mut s.extrapolate_amount,
    );
}

/// Back to not drawing from snapshots (left the server).
pub(super) fn reset(world: &mut World) {
    world.insert_resource(InterpClock::default());
}

/// The length of a server tick, s.
pub fn tick_step(world_clock: &super::predict::CommandClock, fixed: &Time<Fixed>) -> f64 {
    world_clock
        .step()
        .unwrap_or_else(|| fixed.timestep())
        .as_secs_f64()
        .max(1e-4)
}

/// Ticks the server finished sending: the clock hears them, and every
/// buffered entity without a value for one held still at it.
fn hear_ticks(
    mut heard: MessageReader<MutateTickReceived>,
    time: Res<Time<Real>>,
    command_clock: Res<super::predict::CommandClock>,
    fixed: Res<Time<Fixed>>,
    mut clock: ResMut<InterpClock>,
    mut bodies: Query<&mut Snapshots<NetBody>>,
    mut movers: Query<&mut Snapshots<NetMover>>,
    mut props: Query<&mut Snapshots<NetProp>>,
    mut items: Query<&mut Snapshots<super::NetItem>>,
) {
    let step = tick_step(&command_clock, &fixed);
    let now = time.elapsed_secs_f64();
    for h in heard.read() {
        let tick = h.tick.get() as u64;
        clock.hear(tick, now, step);
        for mut b in &mut bodies {
            b.hold(tick);
        }
        for mut m in &mut movers {
            m.hold(tick);
        }
        for mut p in &mut props {
            p.hold(tick);
        }
        for mut i in &mut items {
            i.hold(tick);
        }
    }
}

/// A character's drawn state between two snapshots.
#[derive(Clone, Copy, Debug)]
struct Drawn {
    origin: Vec3,
    velocity: Vec3,
    yaw: f32,
    pitch: f32,
    eye: Vec3,
    flags: u8,
}

impl Drawn {
    fn of(b: &NetBody) -> Self {
        Self {
            origin: Vec3::from_array(b.origin),
            velocity: Vec3::from_array(b.velocity),
            yaw: b.yaw,
            pitch: b.pitch,
            eye: Vec3::from_array(b.eye),
            flags: b.flags,
        }
    }

    fn lerp(a: &Self, b: &Self, f: f32) -> Self {
        let view = EyeView::lerp(
            &EyeView {
                yaw: a.yaw,
                pitch: a.pitch,
                ..default()
            },
            &EyeView {
                yaw: b.yaw,
                pitch: b.pitch,
                ..default()
            },
            f,
        );
        Self {
            origin: a.origin.lerp(b.origin, f),
            velocity: a.velocity.lerp(b.velocity, f),
            yaw: view.yaw.rem_euclid(std::f32::consts::TAU),
            pitch: view.pitch,
            eye: a.eye.lerp(b.eye, f),
            // Flags (on ground, crouched, ladder, dead) switch at the
            // newer snapshot's tick.
            flags: if f >= 1.0 { b.flags } else { a.flags },
        }
    }
}

/// Others' characters at the render time: position, velocity, look, eye
/// height and flags, for bodies, cameras and animation.
#[allow(clippy::type_complexity)]
pub(super) fn draw_others(
    time: Res<Time<Real>>,
    settings: Res<InterpSettings>,
    command_clock: Res<super::predict::CommandClock>,
    fixed: Res<Time<Fixed>>,
    mut clock: ResMut<InterpClock>,
    mut graph: ResMut<NetGraph>,
    mut q: Query<
        (
            Entity,
            &mut Snapshots<NetBody>,
            &mut Transform,
            &mut Velocity,
            &mut MovementState,
            &mut Intent,
            Option<&mut RenderedView>,
            Has<Dead>,
        ),
        (With<NetDrawn>, Without<LocalPlayer>),
    >,
    mut commands: Commands,
) {
    let step = tick_step(&command_clock, &fixed);
    let now = time.elapsed_secs_f64();
    clock.advance(now, step, &settings);
    if !clock.running {
        return;
    }
    let at = clock.render_tick;
    let mut ahead = 0usize;
    let mut drawn = 0usize;
    let mut extrapolated = false;
    let mut held = false;
    for (e, mut buf, mut t, mut v, mut s, mut intent, view, dead) in &mut q {
        buf.prune(at);
        ahead += buf.ahead_of(at);
        drawn += 1;
        let body = match buf.around(at) {
            Around::Empty => continue,
            Around::Before((_, b)) => Drawn::of(b),
            Around::Between((ta, a), (tb, b), f) => {
                let (a, b) = (Drawn::of(a), Drawn::of(b));
                let far = SNAP_SPEED * step as f32 * (tb - ta) as f32;
                let teleport = a.origin.distance(b.origin) > far
                    || (a.flags & body_flags::DEAD) != (b.flags & body_flags::DEAD);
                if teleport {
                    let tb = *tb;
                    if buf.snapped != tb {
                        buf.snapped = tb;
                        graph.interp_snaps += 1;
                    }
                    a
                } else {
                    Drawn::lerp(&a, &b, f)
                }
            }
            Around::After((ta, a)) => {
                let mut a = Drawn::of(a);
                let gap = ((at - *ta as f64) * step) as f32;
                if settings.extrapolate != 0 && a.flags & body_flags::DEAD == 0 {
                    let amount = settings.extrapolate_amount.max(0.0);
                    if gap <= amount {
                        extrapolated = true;
                    } else {
                        held = true;
                    }
                    a.origin += a.velocity * gap.min(amount);
                } else {
                    held = true;
                }
                a
            }
        };
        let dead_now = body.flags & body_flags::DEAD != 0;
        if dead_now != dead {
            if dead_now {
                commands.entity(e).insert(Dead {
                    since: fixed.elapsed_secs_f64(),
                });
            } else {
                commands.entity(e).remove::<Dead>();
            }
        }
        if t.translation != body.origin {
            t.translation = body.origin;
        }
        if v.0 != body.velocity {
            v.0 = body.velocity;
        }
        let on = |bit| body.flags & bit != 0;
        let (ground, crouch, ladder) = (
            on(body_flags::ON_GROUND),
            on(body_flags::CROUCHING),
            on(body_flags::ON_LADDER),
        );
        if s.on_ground != ground || s.crouching != crouch || s.on_ladder != ladder || s.eye_offset != body.eye {
            s.on_ground = ground;
            s.crouching = crouch;
            s.on_ladder = ladder;
            s.eye_offset = body.eye;
        }
        if intent.yaw != body.yaw || intent.pitch != body.pitch || intent.crouch != crouch {
            intent.yaw = body.yaw;
            intent.pitch = body.pitch;
            intent.crouch = crouch;
        }
        if let Some(mut view) = view {
            let now_view = EyeView {
                eye_offset: body.eye,
                view_roll: 0.0,
                yaw: body.yaw,
                pitch: body.pitch,
                punch: Vec2::ZERO,
            };
            if view.now != now_view {
                view.now = now_view;
            }
        }
    }
    graph.interp_ms = clock.delay * 1000.0;
    graph.update_ms = clock.interval * 1000.0;
    graph.interp_drawn = drawn;
    graph.interp_ahead = if drawn > 0 { ahead as f64 / drawn as f64 } else { 0.0 };
    graph.interp_newest_ms = (clock.newest as f64 - at) * step * 1000.0;
    if extrapolated {
        graph.interp_extrapolated += 1;
    }
    if held {
        graph.interp_held += 1;
    }
    graph.interp_frames += 1;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn buffers_keep_tick_order_and_hold_only_after_the_newest() {
        let mut s = Snapshots::new(10, 1.0f32);
        s.push(12, 3.0);
        s.push(11, 2.0);
        s.push(12, 4.0);
        assert_eq!(s.samples().iter().map(|(t, _)| *t).collect::<Vec<_>>(), [10, 11, 12]);
        assert_eq!(s.newest(), Some(&(12, 4.0)));
        s.hold(11);
        assert_eq!(s.samples().len(), 3);
        s.hold(14);
        assert_eq!(s.newest(), Some(&(14, 4.0)));
        match s.around(13.0) {
            Around::Between((12, a), (14, b), f) => assert!(*a == 4.0 && *b == 4.0 && (f - 0.5).abs() < 1e-6),
            other => panic!("{other:?}"),
        }
        assert!(matches!(s.around(9.0), Around::Before((10, _))));
        assert!(matches!(s.around(15.0), Around::After((14, _))));
        assert_eq!(s.at_or_before(13).map(|x| x.0), Some(12));
        assert_eq!(s.at_or_before(3).map(|x| x.0), Some(10));
        s.prune(12.5);
        assert_eq!(s.samples().iter().map(|(t, _)| *t).collect::<Vec<_>>(), [12, 14]);
        assert_eq!(s.ahead_of(12.5), 1);
    }

    #[test]
    fn the_render_clock_eases_and_never_steps_back() {
        let settings = InterpSettings::default();
        let step = 1.0 / 64.0;
        let mut c = InterpClock::default();
        // Ticks arrive 50 ms after they happen, give or take 20 ms.
        let mut last = f64::MIN;
        for k in 0..2000u64 {
            let now = k as f64 * step / 4.0;
            if k % 4 == 0 {
                let tick = k / 4 + 100;
                let jitter = ((k * 7919) % 41) as f64 / 1000.0 - 0.02;
                c.hear(tick, now + 0.05 + jitter, step);
            }
            c.advance(now, step, &settings);
            assert!(c.render_tick > last, "frame {k}: {} after {last}", c.render_tick);
            last = c.render_tick;
        }
        // Settled: drawn the delay plus the latency behind the newest.
        let behind = (c.newest as f64 - c.render_tick) * step;
        assert!((behind - 0.15).abs() < 0.03, "{behind}");
        assert!((c.delay - 0.1).abs() < 1e-6);
    }
}
