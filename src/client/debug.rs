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
        .add_systems(Startup, (spawn_hud, hide_physics_gizmos))
        .add_systems(Update, (keys, update_hud));
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
            top: px(8.0),
            left: px(8.0),
            ..default()
        },
    ));
}

fn update_hud(
    mut hud: Single<&mut Text, With<DebugHud>>,
    player: Single<(&MovementSlot, &Velocity, &MovementState), With<LocalPlayer>>,
) {
    let (slot, vel, state) = *player;
    let speed = Vec2::new(vel.x, vel.z).length();
    hud.0 = format!(
        "movement: {}\nspeed: {speed:.2} m/s  vertical: {:.2}\nground: {}  crouch: {}  sprint: {}\n\n\
         click: capture mouse  esc: release\nV: next movement  F1: inspector  F3: collision",
        slot.0, vel.y, state.on_ground, state.crouching, state.sprinting,
    );
}
