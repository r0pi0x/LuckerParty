//! Debug overlays drawn with gizmos: the nav mesh around you
//! (`mashup_drawnav 1`: areas coloured by place, links between them) and
//! what each bot is doing (`mashup_drawbots 1`: its target, where it last
//! saw or heard an enemy, the route it walks and where it roams, its
//! planned grenade arc, target and burst point, a flash it looks away
//! from) and
//! ragdolls (`mashup_ragdoll_debug 1`: each body's bounds and axes, each
//! joint from its parent body's anchor to the child body).

use bevy::prelude::*;

use crate::{
    bot::Bot,
    console::resource_cvar,
    core::{Intent, LocalPlayer},
    map::{
        nav::NavMesh,
        ragdoll::{RagdollBody, RagdollJoint, RagdollSettings},
    },
    weapon::grenade::GrenadeKind,
};

pub struct DebugViewsPlugin;

impl Plugin for DebugViewsPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<DebugViews>()
            .add_systems(Update, (draw_nav, draw_bots, draw_ragdolls));
        resource_cvar::<DebugViews, u8>(
            app,
            "mashup_ragdoll_debug",
            "1: draw ragdoll bodies (bounds, axes) and joints (parent anchor to child).",
            |d| &mut d.ragdolls,
        );
        resource_cvar::<RagdollSettings, u8>(
            app,
            "cl_ragdoll_physics_enable",
            "Enable/disable ragdoll physics (0: the dead just vanish).",
            |s| &mut s.enabled,
        );
        resource_cvar::<RagdollSettings, f32>(
            app,
            "ragdoll_sleepaftertime",
            "After this many seconds of being basically stationary, the ragdoll will go to sleep.",
            |s| &mut s.sleep_after,
        );
        resource_cvar::<DebugViews, u8>(
            app,
            "mashup_drawnav",
            "1: outline nav areas near you (coloured by place) and their links; 2: all of them.",
            |d| &mut d.nav,
        );
        resource_cvar::<DebugViews, u8>(
            app,
            "mashup_drawbots",
            "1: each bot's target, last known enemy position, route, roaming goal, grenade arc and flash it avoids.",
            |d| &mut d.bots,
        );
    }
}

#[derive(Resource, Default)]
struct DebugViews {
    nav: u8,
    bots: u8,
    ragdolls: u8,
}

/// Areas within this of you are drawn at `mashup_drawnav 1`, m.
const NAV_RADIUS: f32 = 25.0;
/// Lift off the floor, m.
const LIFT: f32 = 0.05;

fn place_color(place: Option<usize>) -> Color {
    match place {
        // A stable hue per place.
        Some(p) => Color::hsl((p as f32 * 67.0) % 360.0, 0.8, 0.55),
        None => Color::srgb(0.6, 0.6, 0.6),
    }
}

fn draw_nav(
    views: Res<DebugViews>,
    nav: Option<Res<NavMesh>>,
    me: Option<Single<&GlobalTransform, With<LocalPlayer>>>,
    mut gizmos: Gizmos,
) {
    let (Some(nav), true) = (nav, views.nav > 0) else { return };
    let here = me.map(|t| t.translation());
    for a in &nav.areas {
        if views.nav == 1 && here.is_some_and(|h| h.distance(a.center) > NAV_RADIUS) {
            continue;
        }
        let [h0, h1, h2, h3] = a.heights;
        let corners = [
            Vec3::new(a.min.x, h0 + LIFT, a.min.y),
            Vec3::new(a.max.x, h1 + LIFT, a.min.y),
            Vec3::new(a.max.x, h2 + LIFT, a.max.y),
            Vec3::new(a.min.x, h3 + LIFT, a.max.y),
        ];
        let color = place_color(a.place);
        gizmos.linestrip(corners.iter().copied().chain(std::iter::once(corners[0])), color);
        let from = a.center.with_y(a.height_at(a.center.x, a.center.z) + LIFT);
        for (to, _) in &a.links {
            let b = &nav.areas[*to];
            let end = b.center.with_y(b.height_at(b.center.x, b.center.z) + LIFT);
            // Half the way: two-way links meet in the middle.
            gizmos.line(from, from.lerp(end, 0.5), color.with_alpha(0.5));
        }
    }
}

fn draw_bots(
    views: Res<DebugViews>,
    bots: Query<(&Bot, &GlobalTransform, &Intent)>,
    targets: Query<&GlobalTransform>,
    mut gizmos: Gizmos,
) {
    if views.bots == 0 {
        return;
    }
    for (bot, at, intent) in &bots {
        let eye = at.translation() + Vec3::Y * 0.6;
        // Where it looks.
        gizmos.line(eye, eye + intent.look_rotation() * Vec3::NEG_Z * 1.5, Color::WHITE);
        if let Some(t) = bot.target.and_then(|t| targets.get(t).ok()) {
            gizmos.line(eye, t.translation(), Color::srgb(1.0, 0.2, 0.2));
        }
        if let Some((lead, _)) = bot.lead {
            gizmos.sphere(Isometry3d::from_translation(lead + Vec3::Y * 0.3), 0.3, Color::srgb(1.0, 0.6, 0.0));
            gizmos.line(eye, lead + Vec3::Y * 0.3, Color::srgba(1.0, 0.6, 0.0, 0.4));
        }
        let (route, next) = bot.route();
        let ahead = route.iter().skip(next).map(|p| *p + Vec3::Y * 0.1);
        gizmos.linestrip(std::iter::once(at.translation()).chain(ahead), Color::srgb(0.3, 0.9, 1.0));
        if let Some(goal) = bot.roam_goal() {
            gizmos.sphere(
                Isometry3d::from_translation(goal + Vec3::Y * 0.3),
                0.25,
                Color::srgb(0.6, 0.4, 1.0),
            );
        }
        if let Some(plan) = bot.grenade_plan() {
            let color = match plan.kind {
                GrenadeKind::Blast => Color::srgb(1.0, 0.25, 0.1),
                GrenadeKind::Flash => Color::srgb(1.0, 1.0, 0.6),
                GrenadeKind::Smoke => Color::srgb(0.7, 0.7, 0.7),
            };
            gizmos.linestrip(plan.points.iter().copied(), color);
            // The target (a cross on the floor) and where it should go off.
            let t = plan.target + Vec3::Y * LIFT;
            gizmos.line(t - Vec3::X * 0.4, t + Vec3::X * 0.4, Color::srgb(0.2, 1.0, 0.3));
            gizmos.line(t - Vec3::Z * 0.4, t + Vec3::Z * 0.4, Color::srgb(0.2, 1.0, 0.3));
            gizmos.sphere(Isometry3d::from_translation(plan.pop), 0.2, color);
            if bot.throwing() {
                gizmos.line(eye, plan.start, color);
            }
        }
        if let Some(from) = bot.averting() {
            gizmos.line(eye, from, Color::srgba(1.0, 1.0, 1.0, 0.3));
            gizmos.sphere(Isometry3d::from_translation(from), 0.15, Color::WHITE);
        }
    }
}

fn draw_ragdolls(
    views: Res<DebugViews>,
    bodies: Query<(&GlobalTransform, &avian3d::prelude::ColliderAabb), With<RagdollBody>>,
    joints: Query<&RagdollJoint>,
    mut gizmos: Gizmos,
) {
    if views.ragdolls == 0 {
        return;
    }
    for (t, aabb) in &bodies {
        let centre = (aabb.min + aabb.max) / 2.0;
        gizmos.cube(
            Transform::from_translation(centre).with_scale(aabb.max - aabb.min),
            Color::srgb(0.3, 0.9, 0.4),
        );
        gizmos.axes(*t, 0.1);
    }
    for j in &joints {
        let (Ok((p, _)), Ok((c, _))) = (bodies.get(j.parent), bodies.get(j.child)) else {
            continue;
        };
        let anchor = p.transform_point(j.anchor);
        gizmos.line(p.translation(), anchor, Color::srgb(1.0, 0.9, 0.2));
        // Red when the child drifted from its anchor.
        let drift = anchor.distance(c.translation());
        let color = if drift > 0.02 { Color::srgb(1.0, 0.1, 0.1) } else { Color::WHITE };
        gizmos.sphere(Isometry3d::from_translation(anchor), 0.015, color);
    }
}
