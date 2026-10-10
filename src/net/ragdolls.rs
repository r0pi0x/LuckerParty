//! Map-placed ragdolls in a network game (`map::placed_ragdoll`;
//! specs/source/physics_brushes.md 6.10): the server simulates them and
//! writes each one's part poses into its node's `NetRagdoll` after each
//! tick; a client's copy stops simulating (its bodies kinematic) and is
//! drawn between the snapshots around the render time, as physics props
//! are (`props`).

use avian3d::prelude::RigidBody;
use bevy::{app::RunFixedMainLoopSystems, prelude::*};

use super::{
    NetRagdoll,
    interp::{Around, InterpClock, Snapshots},
};
use crate::{
    core::NetRole,
    map::{
        PropIndex, Ragdoll,
        interp::{InterpSystems, NetDrawn, SNAP_SPEED},
        placed_ragdoll::PlacedRagdoll,
    },
};

/// The most parts a ragdoll sends (spec 6.10).
pub const MAX_PARTS: usize = 24;

pub(super) fn plugin(app: &mut App) {
    app.add_systems(FixedLast, write_ragdolls.run_if(resource_equals(NetRole::Server)))
        .add_systems(
            RunFixedMainLoop,
            draw_ragdolls
                .in_set(RunFixedMainLoopSystems::AfterFixedMainLoop)
                .after(InterpSystems::Ease)
                .run_if(resource_equals(NetRole::Client))
                .run_if(resource_exists::<super::client::Joined>),
        );
}

/// The server: each placed ragdoll's part poses into its node's
/// `NetRagdoll` (replicated from the first one on).
fn write_ragdolls(
    mut nodes: Query<(Entity, &PropIndex, &PlacedRagdoll, Option<&mut NetRagdoll>)>,
    sims: Query<&Ragdoll>,
    parts: Query<&Transform>,
    mut commands: Commands,
) {
    for (e, index, placed, have) in &mut nodes {
        let Some(r) = placed.sim.and_then(|s| sims.get(s).ok()) else {
            continue;
        };
        let now = NetRagdoll {
            index: index.0 as u32,
            parts: r
                .bodies
                .iter()
                .take(MAX_PARTS)
                .filter_map(|b| parts.get(*b).ok())
                .map(|t| {
                    let (p, q) = (t.translation, t.rotation);
                    [p.x, p.y, p.z, q.x, q.y, q.z, q.w]
                })
                .collect(),
        };
        match have {
            Some(mut have) => {
                have.set_if_neq(now);
            }
            None => {
                commands.entity(e).insert((bevy_replicon::prelude::Replicated, now));
            }
        }
    }
}

fn part(p: &[f32; 7]) -> (Vec3, Quat) {
    (
        Vec3::new(p[0], p[1], p[2]),
        Quat::from_xyzw(p[3], p[4], p[5], p[6]).normalize(),
    )
}

/// A client: its placed ragdolls' parts where the server had them at the
/// render time.
#[allow(clippy::type_complexity)]
fn draw_ragdolls(
    clock: Res<InterpClock>,
    command_clock: Res<super::predict::CommandClock>,
    fixed: Res<Time<Fixed>>,
    mut proxies: Query<&mut Snapshots<NetRagdoll>>,
    nodes: Query<(&PropIndex, &PlacedRagdoll), Without<Snapshots<NetRagdoll>>>,
    sims: Query<&Ragdoll>,
    mut bodies: Query<(&mut Transform, Option<&RigidBody>, Has<NetDrawn>), Without<PlacedRagdoll>>,
    mut commands: Commands,
) {
    if !clock.running || proxies.is_empty() {
        return;
    }
    let at = clock.render_tick;
    let step = super::interp::tick_step(&command_clock, &fixed) as f32;
    let mut by_index: std::collections::HashMap<u32, Vec<(Vec3, Quat)>> = Default::default();
    for mut buf in &mut proxies {
        buf.prune(at);
        let (index, poses) = match buf.around(at) {
            Around::Empty => continue,
            Around::Before((_, a)) | Around::After((_, a)) => (a.index, a.parts.iter().map(part).collect()),
            Around::Between((ta, a), (tb, b), f) => {
                let far = SNAP_SPEED * step * (tb - ta) as f32;
                let poses = a
                    .parts
                    .iter()
                    .zip(&b.parts)
                    .map(|(pa, pb)| {
                        let ((p0, q0), (p1, q1)) = (part(pa), part(pb));
                        if p0.distance(p1) > far {
                            (p0, q0)
                        } else {
                            (p0.lerp(p1, f), q0.slerp(q1, f))
                        }
                    })
                    .collect();
                (a.index, poses)
            }
        };
        by_index.insert(index, poses);
    }
    for (index, placed) in &nodes {
        let Some(poses) = by_index.get(&(index.0 as u32)) else {
            continue;
        };
        let Some(r) = placed.sim.and_then(|s| sims.get(s).ok()) else {
            continue;
        };
        for (b, (p, q)) in r.bodies.iter().zip(poses) {
            let Ok((mut t, body, drawn)) = bodies.get_mut(*b) else {
                continue;
            };
            if !drawn {
                // The server simulates it: here it only follows.
                commands.entity(*b).insert(NetDrawn);
                if body.is_some_and(|b| !b.is_kinematic()) {
                    commands.entity(*b).insert(RigidBody::Kinematic);
                }
            }
            if t.translation != *p || t.rotation != *q {
                t.translation = *p;
                t.rotation = *q;
            }
        }
    }
}
