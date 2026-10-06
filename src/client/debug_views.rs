//! Debug overlays drawn with gizmos: the nav mesh around you
//! (`mashup_drawnav 1`: areas coloured by place, links between them) and
//! what each bot is doing (`mashup_drawbots 1`: its target, where it last
//! saw or heard an enemy, the route it walks and where it roams).

use bevy::prelude::*;

use crate::{
    bot::Bot,
    console::resource_cvar,
    core::{Intent, LocalPlayer},
    map::nav::NavMesh,
};

pub struct DebugViewsPlugin;

impl Plugin for DebugViewsPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<DebugViews>()
            .add_systems(Update, (draw_nav, draw_bots));
        resource_cvar::<DebugViews, u8>(
            app,
            "mashup_drawnav",
            "1: outline nav areas near you (coloured by place) and their links; 2: all of them.",
            |d| &mut d.nav,
        );
        resource_cvar::<DebugViews, u8>(
            app,
            "mashup_drawbots",
            "1: each bot's target, last known enemy position, route and roaming goal.",
            |d| &mut d.bots,
        );
    }
}

#[derive(Resource, Default)]
struct DebugViews {
    nav: u8,
    bots: u8,
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
            gizmos.sphere(Isometry3d::from_translation(goal + Vec3::Y * 0.3), 0.25, Color::srgb(0.6, 0.4, 1.0));
        }
    }
}
