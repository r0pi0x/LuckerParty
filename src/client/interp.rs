//! Drawing between simulation ticks (`map::interp`): the plugin, the
//! `cl_interpolate` switch, the view punch in the eased eye, grenades in
//! flight, and the local player's per-frame look in its eye.

use bevy::prelude::*;

use crate::{
    console::resource_cvar,
    core::{Intent, LocalPlayer},
    map::interp::{InterpPlugin, InterpSystems, Interpolated, Interpolation, RenderedView},
    weapon::{ViewPunch, grenade::Projectile},
};

pub struct ClientInterpPlugin;

impl Plugin for ClientInterpPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(InterpPlugin)
            .register_required_components::<Projectile, Interpolated>()
            .add_systems(FixedLast, sample_punch.in_set(InterpSystems::Extend))
            .add_systems(
                Update,
                local_look
                    .after(super::input::write_local_intent)
                    .before(crate::map::DriveAnimation),
            );
        resource_cvar::<Interpolation, u8>(
            app,
            "cl_interpolate",
            "1: draw what moves at the simulation's tick (players, eyes, props, doors) between its last two \
             ticks, one tick behind; 0: the latest tick as it is (steps at the tick rate).",
            |i| &mut i.enabled,
        );
    }
}

/// The punch goes into the tick's eye sample, so it eases too.
fn sample_punch(mut q: Query<(&mut RenderedView, Option<&ViewPunch>)>) {
    for (mut v, punch) in &mut q {
        let p = punch.map_or(Vec2::ZERO, |p| p.0);
        if v.latest_mut().punch != p {
            v.latest_mut().punch = p;
        }
    }
}

/// The local player's look is this frame's mouse, never eased.
fn local_look(mut q: Query<(&Intent, &mut RenderedView), With<LocalPlayer>>) {
    for (intent, mut v) in &mut q {
        if v.now.yaw != intent.yaw || v.now.pitch != intent.pitch {
            v.now.yaw = intent.yaw;
            v.now.pitch = intent.pitch;
        }
    }
}
