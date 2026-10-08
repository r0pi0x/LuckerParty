//! Developer tools. F1: entity inspector. F3: collision shapes.
//! V: cycle the local player's Movement implementation.

use avian3d::prelude::*;
use bevy::{input::common_conditions::input_toggle_active, prelude::*, window::CursorOptions};
use bevy_inspector_egui::{bevy_egui::EguiPlugin, quick::WorldInspectorPlugin};

use crate::{
    client::input::release_cursor,
    core::{LocalPlayer, MovementState, Velocity},
    slots::{MovementRegistry, MovementSlot, set_movement},
};

pub struct DebugPlugin;

impl Plugin for DebugPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins((
            EguiPlugin::default(),
            WorldInspectorPlugin::default().run_if(input_toggle_active(false, KeyCode::F1)),
            PhysicsDebugPlugin,
        ))
        .init_resource::<DrawHitboxes>()
        .init_resource::<DrawPhys>()
        .add_systems(Startup, (spawn_hud, hide_physics_gizmos))
        .add_systems(Update, (keys, update_hud, draw_hitboxes, draw_phys));
        crate::console::resource_cvar::<DrawHitboxes, u8>(
            app,
            "mashup_drawhitboxes",
            "1: outline characters' hitboxes (head red, chest yellow, stomach green, arms blue, legs cyan).",
            |d| &mut d.0,
        );
        crate::console::resource_cvar::<DrawPhys, u8>(
            app,
            "mashup_drawphys",
            "1: outline physics props by state (green moving, blue asleep, grey still or frozen; \
             multiplayer props darker), players' physics shadows (white, yellow lines to the props \
             they touch) and list the props nearest you (mass, mode, state) on the debug HUD.",
            |d| &mut d.0,
        );
    }
}

#[derive(Resource, Default)]
struct DrawPhys(u8);

/// A physics prop's state for `mashup_drawphys`.
fn prop_state(rb: &RigidBody, sleeping: bool, asleep_since_spawn: bool) -> &'static str {
    if asleep_since_spawn {
        "asleep (since spawn)"
    } else if !rb.is_dynamic() {
        "still"
    } else if sleeping {
        "asleep"
    } else {
        "moving"
    }
}

/// `mashup_drawphys 1`: props' boxes by state, shadows and what they
/// touch.
#[allow(clippy::type_complexity)]
fn draw_phys(
    on: Res<DrawPhys>,
    props: Query<(
        &crate::map::PhysicsProp,
        &RigidBody,
        &ColliderAabb,
        Has<Sleeping>,
        Has<crate::map::prop_physics::StartAsleep>,
    )>,
    shadows: Query<(Entity, &crate::map::prop_physics::PhysicsShadow, &ColliderAabb)>,
    collisions: Collisions,
    mut gizmos: Gizmos,
) {
    if on.0 == 0 {
        return;
    }
    let cube = |aabb: &ColliderAabb| {
        Transform::from_translation((aabb.min + aabb.max) / 2.0).with_scale((aabb.max - aabb.min).max(Vec3::splat(0.01)))
    };
    for (p, rb, aabb, sleeping, asleep) in &props {
        let mut color = match prop_state(rb, sleeping, asleep) {
            "moving" => Color::srgb(0.2, 1.0, 0.2),
            "still" => Color::srgb(0.6, 0.6, 0.6),
            _ => Color::srgb(0.3, 0.5, 1.0),
        };
        if p.push != crate::map::PushAway::Collide {
            color = color.darker(0.3);
        }
        gizmos.cube(cube(aabb), color);
    }
    for (e, s, aabb) in &shadows {
        if !s.active {
            continue;
        }
        gizmos.cube(cube(aabb), Color::WHITE);
        let from = (aabb.min + aabb.max) / 2.0;
        for other in collisions.entities_colliding_with(e) {
            if let Ok((_, _, o, ..)) = props.get(other) {
                gizmos.line(from, (o.min + o.max) / 2.0, Color::srgb(1.0, 1.0, 0.2));
            }
        }
    }
}

#[derive(Resource, Default)]
struct DrawHitboxes(u8);

/// Outline every character's hitboxes where the trace sees them (its
/// collider's bottom, turned by the look yaw).
fn draw_hitboxes(
    on: Res<DrawHitboxes>,
    characters: Query<
        (
            &crate::core::Hitboxes,
            &GlobalTransform,
            &ColliderAabb,
            Option<&crate::core::Intent>,
        ),
        Without<LocalPlayer>,
    >,
    mut gizmos: Gizmos,
) {
    use crate::core::Hitgroup;
    if on.0 == 0 {
        return;
    }
    for (boxes, at, aabb, intent) in &characters {
        let feet = at.translation().with_y(aabb.min.y);
        let yaw = intent.map_or(Quat::IDENTITY, |i| i.yaw_rotation());
        for h in &boxes.0 {
            let color = match h.group {
                Hitgroup::Head => Color::srgb(1.0, 0.2, 0.2),
                Hitgroup::Chest => Color::srgb(1.0, 1.0, 0.2),
                Hitgroup::Stomach => Color::srgb(0.2, 1.0, 0.2),
                Hitgroup::LeftArm | Hitgroup::RightArm => Color::srgb(0.3, 0.5, 1.0),
                Hitgroup::LeftLeg | Hitgroup::RightLeg => Color::srgb(0.2, 1.0, 1.0),
                _ => Color::WHITE,
            };
            let t = Transform::from_translation(feet + yaw * h.center)
                .with_rotation(yaw * h.rotation)
                .with_scale(h.half * 2.0);
            gizmos.cube(t, color);
        }
    }
}

#[derive(Component)]
struct DebugHud;

fn hide_physics_gizmos(mut store: ResMut<GizmoConfigStore>) {
    store.config_mut::<PhysicsGizmos>().0.enabled = false;
}

fn keys(
    keys: Res<ButtonInput<KeyCode>>,
    mut commands: Commands,
    mut cursor: Single<&mut CursorOptions>,
    mut store: ResMut<GizmoConfigStore>,
    player: Single<(Entity, &MovementSlot), With<LocalPlayer>>,
    registry: Res<MovementRegistry>,
    console: Option<Res<super::console::ConsoleUi>>,
) {
    // Typing in the console isn't a shortcut.
    if console.is_some_and(|c| c.open) {
        return;
    }
    if keys.just_pressed(KeyCode::F1) {
        release_cursor(&mut cursor);
    }
    if keys.just_pressed(KeyCode::F3) {
        let config = store.config_mut::<PhysicsGizmos>().0;
        config.enabled = !config.enabled;
    }
    if keys.just_pressed(KeyCode::KeyV) {
        let (entity, slot) = *player;
        if let Some(next) = registry.next_after(slot.0) {
            commands.queue(set_movement(entity, next.id));
        }
    }
}

fn spawn_hud(mut commands: Commands) {
    commands.spawn((
        DebugHud,
        Text::default(),
        TextFont {
            font_size: FontSize::Px(14.0),
            ..default()
        },
        Node {
            position_type: PositionType::Absolute,
            // Below the radar and place name (top left, as in CS:S).
            top: percent(30.0),
            left: px(8.0),
            ..default()
        },
    ));
}

#[allow(clippy::type_complexity)]
fn update_hud(
    mut hud: Single<&mut Text, With<DebugHud>>,
    player: Single<(&MovementSlot, &Velocity, &MovementState, &GlobalTransform), With<LocalPlayer>>,
    phys: Res<DrawPhys>,
    props: Query<(
        &crate::map::PhysicsProp,
        &RigidBody,
        &ColliderAabb,
        Has<Sleeping>,
        Has<crate::map::prop_physics::StartAsleep>,
    )>,
) {
    let (slot, vel, state, at) = *player;
    let speed = Vec2::new(vel.x, vel.z).length();
    let mut text = format!(
        "movement: {}\nspeed: {speed:.2} m/s  vertical: {:.2}\nground: {}  crouch: {}  sprint: {}\n\n\
         click: capture mouse  esc: menu\nV: next movement  F1: inspector  F3: collision",
        slot.0, vel.y, state.on_ground, state.crouching, state.sprinting,
    );
    if phys.0 != 0 {
        // The physics props nearest the player.
        let me = at.translation();
        let mut near: Vec<_> = props
            .iter()
            .map(|(p, rb, aabb, sleeping, asleep)| {
                let centre = (aabb.min + aabb.max) / 2.0;
                (centre.distance(me), p, prop_state(rb, sleeping, asleep))
            })
            .collect();
        near.sort_by(|a, b| a.0.total_cmp(&b.0));
        text.push_str("\n\nnearest physics props:");
        for (d, p, s) in near.iter().take(6) {
            text.push_str(&format!("\n{d:5.1} m  {:7.1} kg  {:?}  {s}", p.mass, p.push));
        }
    }
    hud.0 = text;
}
