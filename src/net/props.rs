//! Physics props in a network game (docs/plans/active/multiplayer.md,
//! slice 3): the server simulates them; a client draws them from
//! snapshots at the render time, as it draws other players (`interp`).
//!
//! - **Server.** After each tick, each physics prop's pose, velocity and
//!   whether it is there (shown, solid) go into its node's `NetProp`
//!   (`write_props`); a prop at rest sends nothing.
//! - **Client.** Its own copy of the prop (the node with the same
//!   `map::PropIndex`) stops being simulated (a kinematic body, so its
//!   own player still bumps into it) and is drawn each frame between the
//!   two snapshots around the render time (position along a line, rotation
//!   the short way), snapped on a jump (a round restart puts props back).
//!   Pushing one is the server's to simulate: the client sees the result
//!   after the delay (Source the same).

use avian3d::prelude::{ColliderDisabled, LinearVelocity, RigidBody};
use bevy::{app::RunFixedMainLoopSystems, prelude::*};

use super::{
    NetProp,
    interp::{Around, InterpClock, Snapshots},
    prop_flags,
};
use crate::{
    core::NetRole,
    map::{
        PhysicsProp, PropIndex,
        interp::{InterpSystems, NetDrawn, SNAP_SPEED},
        prop_physics::FrozenBody,
        vis::{LogicHidden, VisClusters},
    },
};

pub(super) fn plugin(app: &mut App) {
    app.add_systems(FixedLast, write_props.run_if(resource_equals(NetRole::Server)))
        .add_systems(
            RunFixedMainLoop,
            draw_props
                .in_set(RunFixedMainLoopSystems::AfterFixedMainLoop)
                .after(InterpSystems::Ease)
                .after(super::interp::draw_others)
                .run_if(resource_equals(NetRole::Client)),
        );
}

/// The server: each physics prop's state into its node's `NetProp`
/// (replicated from the first one on).
#[allow(clippy::type_complexity)]
fn write_props(
    mut q: Query<
        (
            Entity,
            &PropIndex,
            &Transform,
            Option<&LinearVelocity>,
            Has<LogicHidden>,
            Has<ColliderDisabled>,
            Option<&mut NetProp>,
        ),
        Or<(With<PhysicsProp>, With<FrozenBody>)>,
    >,
    mut commands: Commands,
) {
    for (e, index, t, v, hidden, no_collision, have) in &mut q {
        let mut flags = 0;
        if !hidden {
            flags |= prop_flags::VISIBLE;
        }
        if !no_collision {
            flags |= prop_flags::SOLID;
        }
        let now = NetProp {
            index: index.0 as u32,
            origin: t.translation.to_array(),
            rotation: t.rotation.to_array(),
            velocity: v.map_or([0.0; 3], |v| v.0.to_array()),
            flags,
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

/// A client: its props where the server had them at the render time.
#[allow(clippy::type_complexity)]
fn draw_props(
    clock: Res<InterpClock>,
    command_clock: Res<super::predict::CommandClock>,
    fixed: Res<Time<Fixed>>,
    mut proxies: Query<&mut Snapshots<NetProp>>,
    mut props: Query<
        (
            Entity,
            &PropIndex,
            &mut Transform,
            Option<&RigidBody>,
            Has<NetDrawn>,
            Has<ColliderDisabled>,
            Option<&VisClusters>,
            Option<&Visibility>,
        ),
        Or<(With<PhysicsProp>, With<FrozenBody>)>,
    >,
    mut commands: Commands,
) {
    if !clock.running || proxies.is_empty() {
        return;
    }
    let at = clock.render_tick;
    let step = super::interp::tick_step(&command_clock, &fixed) as f32;
    let mut by_index: std::collections::HashMap<u32, (Transform, u16)> = std::collections::HashMap::new();
    for mut buf in &mut proxies {
        buf.prune(at);
        let pose = match buf.around(at) {
            Around::Empty => continue,
            Around::Before((_, p)) | Around::After((_, p)) => (pose_of(p), p.index, p.flags),
            Around::Between((ta, a), (tb, b), f) => {
                let (pa, pb) = (pose_of(a), pose_of(b));
                let far = SNAP_SPEED * step * (tb - ta) as f32;
                let t = if pa.translation.distance(pb.translation) > far {
                    pa
                } else {
                    Transform {
                        translation: pa.translation.lerp(pb.translation, f),
                        rotation: pa.rotation.slerp(pb.rotation, f),
                        scale: pa.scale,
                    }
                };
                (t, a.index, if f >= 1.0 { b.flags } else { a.flags })
            }
        };
        by_index.insert(pose.1, (pose.0, pose.2));
    }
    for (e, index, mut t, body, drawn, no_collision, vis, visibility) in &mut props {
        let Some((pose, flags)) = by_index.get(&(index.0 as u32)) else {
            continue;
        };
        if !drawn {
            // The server simulates it: here it only follows.
            commands.entity(e).insert(NetDrawn);
            if body.is_some_and(|b| b.is_dynamic()) {
                commands.entity(e).insert(RigidBody::Kinematic);
            }
        }
        let pose = Transform {
            scale: t.scale,
            ..*pose
        };
        if *t != pose {
            *t = pose;
        }
        let solid = flags & prop_flags::SOLID != 0;
        if solid == no_collision {
            if solid {
                commands.entity(e).remove::<ColliderDisabled>();
            } else {
                commands.entity(e).insert(ColliderDisabled);
            }
        }
        let shown = flags & prop_flags::VISIBLE != 0;
        let culled = vis.is_some_and(|v| !v.potentially_visible);
        let want = if !shown || culled { Visibility::Hidden } else { Visibility::Inherited };
        if visibility != Some(&want) {
            commands.entity(e).insert(want);
            if shown {
                commands.entity(e).remove::<LogicHidden>();
            } else {
                commands.entity(e).insert(LogicHidden);
            }
        }
    }
}

fn pose_of(p: &NetProp) -> Transform {
    Transform {
        translation: Vec3::from_array(p.origin),
        rotation: Quat::from_array(p.rotation).normalize(),
        scale: Vec3::ONE,
    }
}
