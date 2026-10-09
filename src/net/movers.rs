//! Moving brushes in a network game (docs/plans/active/multiplayer.md,
//! slice 3): doors, platforms, trains, rotating and parented brushes.
//!
//! - **Server.** After each tick, every mover node's motion state goes
//!   into its `NetMover` (`write_movers`): the logic's pusher (where it
//!   is, its velocity and angular velocity, its local time, when its move
//!   ends and where), and whether it is shown, solid or gone. Only changes
//!   are sent.
//! - **Client.** A client runs its own player ahead of the server (the
//!   tick it predicts, `predict`), and the server ran that tick with the
//!   movers where they were then. So each predicted tick (and each
//!   replayed one) the client places every mover node at that tick
//!   (`place`): from the newest snapshot at or before it, stepped forward
//!   the way the logic steps a pusher (`Stepper`, the same arithmetic in
//!   the same order, so the result is the server's bit for bit while the
//!   mover keeps doing what it did); then pushes its own player with
//!   those steps as the logic does before movement
//!   (`logic::carry_player`: riders carried, players in the way shoved).
//!   Its movement then sweeps the movers' brushes where the server's did.
//!   What the client can't know ahead (a door starting to open, a train
//!   turning at a corner, a mover blocked by someone else, reversing at
//!   the end of its wait) shows up as a prediction error when the
//!   server's state arrives, corrected and eased like any other.
//! - Movers are drawn at the predicted tick too (eased between ticks as
//!   in single player, `map::interp`), so the client's own player stands
//!   on what it is drawn on; others riding them are drawn in the past
//!   (`interp`), as in Source.

use bevy::prelude::*;

use super::{NetMover, interp::Snapshots, mover_flags, predict::NetGraph};
use crate::{
    core::{MovementState, MovingSolid, NetRole, SimClock, SimSet},
    logic::{CarriedMover, Logic, brush_to_engine, carry_player, classes::Class, movers::pusher, mover_brushes_at},
    map::{
        MapBrushEntity, MapEntities,
        entities::{entity_rotation, entity_to_engine, rotation_to_engine},
    },
};

/// Ticks a client steps a mover past its newest snapshot, at most.
const MAX_STEPS: u64 = 512;

pub(super) fn plugin(app: &mut App) {
    app.add_systems(FixedLast, write_movers.run_if(resource_equals(NetRole::Server)))
        .add_systems(
            FixedUpdate,
            place
                .after(super::predict::record_command)
                .after(SimSet::Commands)
                .before(SimSet::Movement)
                .run_if(resource_equals(NetRole::Client)),
        );
}

/// The server: each mover node's state into its `NetMover` (replicated
/// from the first one on); a node whose entity is gone says so.
fn write_movers(world: &mut World) {
    let Some(logic) = world.get_resource::<Logic>() else {
        return;
    };
    let mut out: Vec<(Entity, NetMover)> = Vec::new();
    let mut live: Vec<Entity> = Vec::new();
    for (id, node) in &logic.nodes {
        let Some(e) = logic.world.get(*id) else { continue };
        let Some(p) = pusher(&e.class) else { continue };
        let Some(index) = world.get::<MapBrushEntity>(*node).map(|n| n.0 as u32) else {
            continue;
        };
        let mut flags = 0;
        for (on, bit) in [
            (p.visible, mover_flags::VISIBLE),
            (p.solid, mover_flags::SOLID),
            (p.inexact, mover_flags::INEXACT),
            (matches!(e.class, Class::Rotating(_)), mover_flags::SPIN),
            (matches!(e.class, Class::PropDoor(_)), mover_flags::PHYSICS_SOLID),
            (p.unblockable, mover_flags::UNBLOCKABLE),
            (logic.world.shootable(*id), mover_flags::SHOOTABLE),
            (
                matches!(e.class, Class::Attached(_) | Class::Breakable(_)),
                mover_flags::ATTACHED,
            ),
        ] {
            if on {
                flags |= bit;
            }
        }
        live.push(*node);
        out.push((
            *node,
            NetMover {
                index,
                origin: p.origin.to_array(),
                angles: p.angles.to_array(),
                velocity: p.velocity.to_array(),
                avelocity: p.avelocity.to_array(),
                ltime: p.ltime,
                move_done: p.move_done,
                goal_origin: p.goal_origin.map(|g| g.to_array()),
                goal_angles: p.goal_angles.map(|g| g.to_array()),
                flags,
            },
        ));
    }
    // Gone (killed, broken) until a round restart.
    let gone: Vec<(Entity, NetMover)> = world
        .query::<(Entity, &NetMover)>()
        .iter(world)
        .filter(|(e, m)| !live.contains(e) && m.flags & mover_flags::GONE == 0)
        .map(|(e, m)| {
            (
                e,
                NetMover {
                    flags: mover_flags::GONE,
                    velocity: [0.0; 3],
                    avelocity: [0.0; 3],
                    move_done: None,
                    ..m.clone()
                },
            )
        })
        .collect();
    for (node, m) in out.into_iter().chain(gone) {
        let mut e = world.entity_mut(node);
        match e.get_mut::<NetMover>() {
            Some(mut have) => {
                have.set_if_neq(m);
            }
            None => {
                e.insert((bevy_replicon::prelude::Replicated, m));
            }
        }
    }
}

/// A pusher stepped as the logic steps it (`LogicWorld::step_movers` and
/// `settle`), without the logic: what it does once a move or wait ends
/// is the server's to say.
#[derive(Clone, Debug, PartialEq)]
pub struct Stepper {
    pub origin: Vec3,
    pub angles: Vec3,
    pub velocity: Vec3,
    pub avelocity: Vec3,
    pub ltime: f64,
    pub move_done: Option<f64>,
    pub goal_origin: Option<Vec3>,
    pub goal_angles: Option<Vec3>,
    pub flags: u16,
}

impl Stepper {
    pub fn of(m: &NetMover) -> Self {
        Self {
            origin: Vec3::from_array(m.origin),
            angles: Vec3::from_array(m.angles),
            velocity: Vec3::from_array(m.velocity),
            avelocity: Vec3::from_array(m.avelocity),
            ltime: m.ltime,
            move_done: m.move_done,
            goal_origin: m.goal_origin.map(Vec3::from_array),
            goal_angles: m.goal_angles.map(Vec3::from_array),
            flags: m.flags,
        }
    }

    fn has(&self, bit: u16) -> bool {
        self.flags & bit != 0
    }

    /// One tick of `dt` s: how far it moved and turned.
    pub fn step(&mut self, dt: f64) -> (Vec3, Vec3) {
        let moving = self.velocity != Vec3::ZERO || self.avelocity != Vec3::ZERO;
        if self.has(mover_flags::ATTACHED | mover_flags::GONE) || (!moving && self.move_done.is_none()) {
            return (Vec3::ZERO, Vec3::ZERO);
        }
        let step = match self.move_done {
            Some(done) if !self.has(mover_flags::INEXACT) => (done - self.ltime).clamp(0.0, dt),
            _ => dt,
        };
        let d = self.velocity * step as f32;
        let da = self.avelocity * step as f32;
        self.origin += d;
        self.angles += da;
        if self.has(mover_flags::SPIN) {
            self.angles = Vec3::new(
                self.angles.x.rem_euclid(360.0),
                self.angles.y.rem_euclid(360.0),
                self.angles.z.rem_euclid(360.0),
            );
        }
        self.ltime += step;
        if let Some(done) = self.move_done
            && self.ltime >= done - 1e-6
        {
            if let Some(g) = self.goal_origin.take() {
                self.origin = g;
                self.velocity = Vec3::ZERO;
            }
            if let Some(g) = self.goal_angles.take() {
                self.angles = g;
                self.avelocity = Vec3::ZERO;
            }
            self.move_done = None;
        }
        (d, da)
    }
}

/// A mover at a predicted tick: before and after its step, and the step.
struct Placed {
    node: Entity,
    index: u32,
    before: Stepper,
    after: Stepper,
    d: Vec3,
    da: Vec3,
}

/// A client's movers at the tick it runs (`SimClock::tick`): stepped
/// from their snapshots, its own player carried as the server's logic
/// does, then the nodes placed (transform, brushes for movement, shown,
/// solid). Runs before movement in each predicted tick and each replayed
/// one (`predict::reconcile`).
pub fn place(world: &mut World) {
    if world.get_resource::<NetRole>() != Some(&NetRole::Client)
        || world.resource::<super::predict::CommandClock>().tick.is_none()
    {
        return;
    }
    let Some(entities) = world.get_resource::<MapEntities>().cloned() else {
        return;
    };
    let clock = *world.resource::<SimClock>();
    let tick = clock.tick;
    // The logic's tick length (an f32 there), as it steps.
    let dt = clock.delta.as_secs_f32() as f64;
    let nodes: std::collections::HashMap<u32, Entity> = world
        .query::<(Entity, &MapBrushEntity)>()
        .iter(world)
        .map(|(e, n)| (n.0 as u32, e))
        .collect();
    let mut placed: Vec<Placed> = Vec::new();
    for buf in world.query::<&Snapshots<NetMover>>().iter(world) {
        let Some((base_tick, base)) = buf.at_or_before(tick.saturating_sub(1)) else {
            continue;
        };
        let Some(node) = nodes.get(&base.index).copied() else {
            continue;
        };
        let mut s = Stepper::of(base);
        let mut t = *base_tick;
        while t + 1 < tick && tick - t <= MAX_STEPS {
            s.step(dt);
            t += 1;
        }
        let before = s.clone();
        let (d, da) = if t < tick { s.step(dt) } else { (Vec3::ZERO, Vec3::ZERO) };
        // A snapshot for this very tick is the server's word for where it
        // ends up (the step's push stays as stepped).
        let after = match buf.at_or_before(tick) {
            Some((t, m)) if *t == tick => Stepper::of(m),
            _ => s,
        };
        placed.push(Placed {
            node,
            index: base.index,
            before,
            after,
            d,
            da,
        });
    }
    placed.sort_by_key(|p| p.index);
    let scale = entities.scale;
    // Our own player, carried and pushed.
    if let Some(player) = super::predict::predicted_player(world) {
        let carried: Vec<CarriedMover> = placed
            .iter()
            .filter_map(|p| {
                let hulls = &entities.entities.get(p.index as usize)?.hulls;
                Some(CarriedMover {
                    node: p.node,
                    index: p.index,
                    hulls,
                    origin: p.before.origin,
                    angles: p.before.angles,
                    d: p.d,
                    da: p.da,
                    solid: p.before.has(mover_flags::SOLID) && !p.before.has(mover_flags::GONE),
                    physics_solid: p.before.has(mover_flags::PHYSICS_SOLID),
                    unblockable: p.before.has(mover_flags::UNBLOCKABLE),
                })
            })
            .collect();
        if carry_player(world, player, scale, &carried) {
            world.resource_mut::<NetGraph>().carried += 1;
        }
    }
    world.resource_mut::<NetGraph>().movers = placed.len();
    for p in &placed {
        let Some(hulls) = entities.entities.get(p.index as usize).map(|e| &e.hulls) else {
            continue;
        };
        let s = &p.after;
        let gone = s.has(mover_flags::GONE);
        let visible = s.has(mover_flags::VISIBLE) && !gone;
        let solid = s.has(mover_flags::SOLID) && !gone;
        let shootable = s.has(mover_flags::SHOOTABLE) && !gone;
        let transform = Transform::from_translation(entity_to_engine(s.origin, scale))
            .with_rotation(rotation_to_engine(entity_rotation(s.angles)));
        let velocity = entity_to_engine(s.velocity, scale);
        let Ok(mut e) = world.get_entity_mut(p.node) else {
            continue;
        };
        let moved = e.get::<Transform>().is_none_or(|t| *t != transform);
        if moved && let Some(mut t) = e.get_mut::<Transform>() {
            *t = transform;
        }
        crate::logic::set_node_shown(&mut e, visible);
        let same = !moved
            && e.get::<MovingSolid>()
                .is_some_and(|m| m.solid == solid && m.velocity == velocity);
        if !same {
            let brushes = if solid {
                mover_brushes_at(hulls, s.origin, s.angles)
                    .iter()
                    .map(|b| brush_to_engine(b, scale))
                    .collect()
            } else {
                Vec::new()
            };
            e.insert(MovingSolid {
                brushes,
                velocity,
                solid,
            });
        }
        if shootable == e.contains::<avian3d::prelude::ColliderDisabled>() {
            if shootable {
                e.remove::<avian3d::prelude::ColliderDisabled>();
            } else {
                e.insert(avian3d::prelude::ColliderDisabled);
            }
        }
    }
    // Snapshots the replays can no longer need.
    let keep = tick.saturating_sub(super::interp::MAX_SAMPLES as u64) as f64;
    for mut buf in world.query::<&mut Snapshots<NetMover>>().iter_mut(world) {
        buf.prune(keep);
    }
}

/// After a correction: the player stands on the server's mover again
/// (`OwnState::ground`, a map entity index).
pub fn restore_ground(world: &mut World, player: Entity, ground: Option<u32>) {
    let node = ground.and_then(|i| {
        world
            .query::<(Entity, &MapBrushEntity)>()
            .iter(world)
            .find(|(_, n)| n.0 as u32 == i)
            .map(|(e, _)| e)
    });
    if let Some(mut s) = world.get_mut::<MovementState>(player)
        && s.ground != node
    {
        s.ground = node;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_stepper_moves_as_the_logic_and_stops_at_its_goal() {
        // 100 units at 100 units/s from x = 0, 0.015 s ticks.
        let m = NetMover {
            index: 0,
            origin: [0.0; 3],
            velocity: [100.0, 0.0, 0.0],
            ltime: 0.0,
            move_done: Some(1.0),
            goal_origin: Some([100.0, 0.0, 0.0]),
            flags: mover_flags::SOLID | mover_flags::VISIBLE,
            ..default()
        };
        let mut s = Stepper::of(&m);
        let dt = 0.015f32 as f64;
        let mut x = 0.0f32;
        for _ in 0..66 {
            let (d, _) = s.step(dt);
            x += 100.0 * dt as f32;
            assert_eq!(d.x, 100.0 * dt as f32);
            assert_eq!(s.origin.x, x);
        }
        // The last partial step ends exactly at the goal and stops.
        let (d, _) = s.step(dt);
        assert!(d.x > 0.0 && d.x < 1.5, "{}", d.x);
        assert_eq!(s.origin.x, 100.0);
        assert_eq!(s.velocity, Vec3::ZERO);
        assert_eq!(s.step(dt), (Vec3::ZERO, Vec3::ZERO));
    }
}
