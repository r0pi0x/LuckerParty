//! Placed weapons' anchor nodes in a network game (`map::entities::
//! MapAnchor`: what is parented to a placed weapon rides it). The server
//! writes each anchor's pose into its node's `NetAnchor` after each tick;
//! a client draws its own anchor node (the same index) between the
//! snapshots around the render time, as it draws physics props, so the
//! props parented to an item follow whoever carries it.

use bevy::{app::RunFixedMainLoopSystems, prelude::*};

use super::{
    NetAnchor,
    interp::{Around, InterpClock, Snapshots},
};
use crate::{
    core::NetRole,
    map::{entities::MapAnchor, interp::InterpSystems},
};

pub(super) fn plugin(app: &mut App) {
    app.add_systems(FixedLast, write_anchors.run_if(resource_equals(NetRole::Server)))
        .add_systems(
            RunFixedMainLoop,
            draw_anchors
                .in_set(RunFixedMainLoopSystems::AfterFixedMainLoop)
                .after(InterpSystems::Ease)
                .run_if(resource_equals(NetRole::Client)),
        );
}

/// The server: each anchor node's pose into its `NetAnchor` (replicated
/// from the first one on).
fn write_anchors(mut q: Query<(Entity, &MapAnchor, &Transform, Option<&mut NetAnchor>)>, mut commands: Commands) {
    for (e, anchor, t, have) in &mut q {
        let now = NetAnchor {
            index: anchor.0 as u32,
            origin: t.translation.to_array(),
            rotation: t.rotation.to_array(),
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

/// A client: its anchor nodes where the server had them at the render
/// time.
fn draw_anchors(
    clock: Res<InterpClock>,
    mut proxies: Query<&mut Snapshots<NetAnchor>>,
    mut nodes: Query<(&MapAnchor, &mut Transform), Without<Snapshots<NetAnchor>>>,
) {
    if !clock.running || proxies.is_empty() {
        return;
    }
    let at = clock.render_tick;
    for mut buf in &mut proxies {
        buf.prune(at);
        let (index, pose) = match buf.around(at) {
            Around::Empty => continue,
            Around::Before((_, a)) | Around::After((_, a)) => (a.index, pose_of(a)),
            Around::Between((_, a), (_, b), f) => {
                let (pa, pb) = (pose_of(a), pose_of(b));
                (
                    a.index,
                    Transform {
                        translation: pa.translation.lerp(pb.translation, f),
                        rotation: pa.rotation.slerp(pb.rotation, f),
                        scale: Vec3::ONE,
                    },
                )
            }
        };
        for (anchor, mut t) in &mut nodes {
            if anchor.0 as u32 == index && *t != pose {
                *t = pose;
            }
        }
    }
}

fn pose_of(a: &NetAnchor) -> Transform {
    Transform {
        translation: Vec3::from_array(a.origin),
        rotation: Quat::from_array(a.rotation).normalize(),
        scale: Vec3::ONE,
    }
}
