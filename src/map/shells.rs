//! Spent shells thrown from a weapon's ejection port (any game; CS:S:
//! specs/cs_source/view_models.md 8): a small model flying under gravity,
//! bouncing off the world with a sound now and then, coming to rest on
//! floors and fading out. Started by `ViewModelEventKind::EjectBrass` for
//! the first-person view model. Until its first bounce it is drawn in the
//! view-model pass, where the view model's field of view places it next to
//! the gun; after that in the world.

use std::collections::HashMap;

use avian3d::prelude::{SpatialQuery, SpatialQueryFilter};
use bevy::{camera::visibility::RenderLayers, prelude::*};

use super::{
    LightField, MapData, MapDebugView, MapModel, ViewModels,
    prop_material::PropMaterial,
    sound::{PlaySound, SoundBank},
    view_model::{
        EffectSettings, ViewAnimator, ViewModelAnchor, ViewModelCamera, ViewModelEvent, ViewModelEventKind,
        ViewModelOffset, VIEW_MODEL_LAYER,
    },
};

/// A shell type: its model (engine axes, meters; +X along the shell) and
/// the sound entry it makes when it bounces.
#[derive(Clone, Debug)]
pub struct MapShell {
    /// The type's name as weapon events give it, lower case (`762nato`).
    pub key: String,
    pub model: MapModel,
    pub bounce: Option<String>,
}

/// How shells move (meters, seconds, radians). Games fill it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MapShellPhysics {
    /// Downward acceleration while flying.
    pub gravity: f32,
    /// Velocity kept per bounce.
    pub damping: f32,
    /// A hit on a surface whose normal's up part is above this, moving
    /// down slower than `rest_steps` frames of gravity, stops the shell.
    pub rest_normal: f32,
    pub rest_steps: f32,
    /// Angles kept per bounce (spin is not damped).
    pub angle_damping: f32,
    /// The event's speed (game units) is scaled to meters by `unit` and by
    /// a random factor in `speed_factor`; random speed up to `up_jitter`
    /// along the port's up and `right_jitter` along its right is added.
    pub unit: f32,
    pub speed_factor: (f32, f32),
    pub up_jitter: f32,
    pub right_jitter: f32,
    /// Pitch and yaw spin up to this, either way (rad/s).
    pub spin: f32,
    /// Seconds before fading, and the fade.
    pub life: f32,
    pub fade: f32,
    /// Chance of a bounce sound per hit, and the vertical impact speed at
    /// which it plays at full volume.
    pub sound_chance: f32,
    pub sound_full_speed: f32,
}

/// A flying (or resting) shell.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ShellState {
    pub position: Vec3,
    pub velocity: Vec3,
    pub rotation: Quat,
    /// Pitch and yaw rates (rad/s).
    pub spin: Vec2,
    pub age: f32,
    pub resting: bool,
    /// Bounced at least once (then drawn in the world).
    pub bounced: bool,
}

/// What happened to a shell this step.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ShellHit {
    None,
    /// It bounced; the vertical speed at impact.
    Bounced(f32),
    /// It came to rest; the vertical speed at impact.
    Rested(f32),
}

/// One step of `dt` seconds: move, spin and fall, or react to `hit` (the
/// first surface between the old and new position: point and normal).
/// Spec view_models.md 8, "Each frame".
pub fn shell_step(s: &mut ShellState, dt: f32, hit: Option<(Vec3, Vec3)>, physics: &MapShellPhysics) -> ShellHit {
    s.age += dt;
    if s.resting {
        return ShellHit::None;
    }
    let spin = s.spin * dt;
    s.rotation = Quat::from_rotation_y(spin.y) * s.rotation * Quat::from_rotation_z(spin.x);
    let Some((point, normal)) = hit else {
        s.position += s.velocity * dt;
        s.velocity.y -= physics.gravity * dt;
        return ShellHit::None;
    };
    s.position = point;
    s.bounced = true;
    let vz = s.velocity.y;
    let step = physics.gravity * dt;
    if normal.y > physics.rest_normal && vz <= 0.0 && vz >= -physics.rest_steps * step {
        s.velocity = Vec3::ZERO;
        s.spin = Vec2::ZERO;
        s.resting = true;
        // Lie flat: keep only the heading.
        let forward = s.rotation * Vec3::X;
        s.rotation = Quat::from_rotation_y(forward.z.atan2(forward.x) * -1.0);
        return ShellHit::Rested(vz);
    }
    s.velocity = (s.velocity - 2.0 * s.velocity.dot(normal) * normal) * physics.damping;
    s.rotation = Quat::IDENTITY.slerp(s.rotation, physics.angle_damping);
    ShellHit::Bounced(vz)
}

/// A shell's opacity at `age` (1, then fading to 0 over `fade`); None
/// once it is gone.
pub fn shell_alpha(age: f32, physics: &MapShellPhysics) -> Option<f32> {
    if age < physics.life {
        return Some(1.0);
    }
    let a = 1.0 - (age - physics.life) / physics.fade.max(1e-6);
    (a > 0.0).then_some(a)
}

/// The volume factor of a bounce at vertical impact speed `vz`.
pub fn bounce_volume(vz: f32, physics: &MapShellPhysics) -> f32 {
    (vz.abs() / physics.sound_full_speed.max(1e-6)).min(1.0)
}

/// Shell models ready to draw, and how they move.
#[derive(Resource)]
pub(super) struct ShellAssets {
    models: HashMap<String, (Vec<(Handle<Mesh>, Handle<PropMaterial>)>, Option<String>)>,
    physics: MapShellPhysics,
    light_scale: f32,
}

pub(super) fn build_assets(
    data: &MapData,
    textures: &[Handle<Image>],
    view: MapDebugView,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<PropMaterial>,
) -> Option<ShellAssets> {
    let physics = data.shell_physics?;
    let models = data
        .shells
        .iter()
        .map(|s| {
            let parts = s
                .model
                .meshes
                .iter()
                .map(|m| {
                    let mut material = super::lit_prop_material(m, textures, data, view, false);
                    material.params.dynamic = 0.0;
                    (meshes.add(super::build_mesh(m, false)), materials.add(material))
                })
                .collect();
            (s.key.to_lowercase(), (parts, s.bounce.clone()))
        })
        .collect();
    Some(ShellAssets {
        models,
        physics,
        light_scale: data.look.light_scale,
    })
}

/// A drawn shell.
#[derive(Component)]
pub(super) struct Shell {
    state: ShellState,
    bounce: Option<String>,
    materials: Vec<Handle<PropMaterial>>,
    in_world: bool,
}

/// The view-model camera's re-projection: (eye, world fov, view-model fov)
/// in degrees, both vertical.
fn projection_of(
    anchor: (&GlobalTransform, &Projection, &Children),
    vm_cameras: &Query<&Projection, (With<ViewModelCamera>, Without<ViewModelAnchor>)>,
) -> Option<(GlobalTransform, f32, f32)> {
    let (eye, projection, children) = anchor;
    let vm = children.iter().find_map(|c| vm_cameras.get(c).ok())?;
    let (Projection::Perspective(w), Projection::Perspective(v)) = (projection, vm) else {
        return None;
    };
    Some((*eye, w.fov.to_degrees(), v.fov.to_degrees()))
}

/// Throw shells for the first-person view model's brass events.
#[allow(clippy::type_complexity, clippy::too_many_arguments)]
pub(super) fn eject(
    mut events: MessageReader<ViewModelEvent>,
    assets: Option<Res<ShellAssets>>,
    effects: Option<Res<EffectSettings>>,
    settings: Option<Res<super::ViewModelSettings>>,
    models: Option<Res<ViewModels>>,
    light_field: Option<Res<LightField>>,
    time: Res<Time>,
    anchors: Query<(&ChildOf, &GlobalTransform, &Projection, &Children), With<ViewModelAnchor>>,
    vm_cameras: Query<&Projection, (With<ViewModelCamera>, Without<ViewModelAnchor>)>,
    owners: Query<(&ViewAnimator, Option<&ViewModelOffset>, Option<&crate::core::Velocity>)>,
    third_person: Option<Res<super::ShowLocalBody>>,
    mut materials: Option<ResMut<Assets<PropMaterial>>>,
    mut dice: Local<u32>,
    mut commands: Commands,
) {
    let Some(assets) = assets else {
        events.clear();
        return;
    };
    if effects.is_some_and(|e| e.eject_brass == 0) || third_person.is_some_and(|t| t.0) {
        events.clear();
        return;
    }
    let settings = settings.map(|s| *s).unwrap_or_default();
    let now = time.elapsed_secs_f64();
    let mut roll = |lo: f32, hi: f32| {
        let mut x = dice.wrapping_add(0x9e37_79b9).max(1);
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        *dice = x;
        lo + (hi - lo) * ((x >> 8) as f32 / (1u32 << 24) as f32)
    };
    let p = assets.physics;
    for ev in events.read() {
        let ViewModelEventKind::EjectBrass {
            attachment,
            ref shell,
            speed,
        } = ev.kind
        else {
            continue;
        };
        let Some((parts, bounce)) = assets.models.get(&shell.to_lowercase()) else { continue };
        let Some((_, eye, projection, children)) = anchors.iter().find(|(c, ..)| c.parent() == ev.owner) else {
            continue;
        };
        let Some((eye, world_fov, vm_fov)) = projection_of((eye, projection, children), &vm_cameras) else {
            continue;
        };
        let Ok((view, offset, velocity)) = owners.get(ev.owner) else { continue };
        let (Some(key), Some(animator)) = (view.key.as_deref(), view.animator.as_ref()) else { continue };
        let Some(model) = models.as_ref().and_then(|m| m.get(key)) else { continue };
        let mirror =
            super::view_model::mirrored(model.right_handed, model.allow_flipping, settings.right_hand != 0);
        let placed = super::view_model::placement(&offset.copied().unwrap_or_default(), mirror);
        let Some(m) = super::view_model::attachment_transform(model, &animator.pose(now), attachment, &placed)
        else {
            continue;
        };
        // The port's position and axes (skeleton: X forward, Y left, Z
        // up), re-projected into the world view.
        let at = |v: Vec3| eye.transform_point(super::view_model::reproject(m.transform_point3(v), world_fov, vm_fov));
        let origin = at(Vec3::ZERO);
        let axis = |v: Vec3| (at(v) - origin).normalize_or_zero();
        let (forward, right, up) = (axis(Vec3::X), axis(Vec3::NEG_Y), axis(Vec3::Z));
        let velocity = forward * speed * p.unit * roll(p.speed_factor.0, p.speed_factor.1)
            + up * roll(-p.up_jitter, p.up_jitter)
            + right * roll(-p.right_jitter, p.right_jitter)
            + velocity.map_or(Vec3::ZERO, |v| v.0);
        // Its own materials, lit where it starts, so it can fade alone.
        let mut own = Vec::with_capacity(parts.len());
        if let Some(materials) = materials.as_mut() {
            let probe = light_field.as_ref().map(|f| (f.0.0)(origin));
            for (_, m) in parts {
                let Some(mut material) = materials.get(m).cloned() else { continue };
                if let Some(probe) = &probe {
                    material.params.set_probe(probe, assets.light_scale);
                }
                own.push(materials.add(material));
            }
        }
        let state = ShellState {
            position: origin,
            velocity,
            // The shooter's eye angles; the model's +X along the view.
            rotation: eye.rotation() * Quat::from_rotation_y(std::f32::consts::FRAC_PI_2),
            spin: Vec2::new(roll(-p.spin, p.spin), roll(-p.spin, p.spin)),
            age: 0.0,
            resting: false,
            bounced: false,
        };
        commands
            .spawn((
                Name::new("Shell"),
                Shell {
                    state,
                    bounce: bounce.clone(),
                    materials: own.clone(),
                    in_world: false,
                },
                Transform::from_translation(origin).with_rotation(state.rotation),
                Visibility::default(),
                RenderLayers::layer(VIEW_MODEL_LAYER),
            ))
            .with_children(|s| {
                for ((mesh, _), material) in parts.iter().zip(own) {
                    s.spawn((
                        Mesh3d(mesh.clone()),
                        MeshMaterial3d(material),
                        RenderLayers::layer(VIEW_MODEL_LAYER),
                        bevy::light::NotShadowCaster,
                    ));
                }
            });
    }
}

/// Move shells, bounce them off the world, place them for drawing and fade
/// them out.
#[allow(clippy::type_complexity, clippy::too_many_arguments)]
pub(super) fn fly(
    time: Res<Time>,
    assets: Option<Res<ShellAssets>>,
    sounds: Option<Res<SoundBank>>,
    spatial: SpatialQuery,
    characters: Query<Entity, With<crate::core::Intent>>,
    anchors: Query<(&GlobalTransform, &Projection, &Children), With<ViewModelAnchor>>,
    vm_cameras: Query<&Projection, (With<ViewModelCamera>, Without<ViewModelAnchor>)>,
    mut shells: Query<(Entity, &mut Shell, &mut Transform, &Children)>,
    mut layers: Query<&mut RenderLayers, Without<Shell>>,
    mut materials: Option<ResMut<Assets<PropMaterial>>>,
    mut play: MessageWriter<PlaySound>,
    mut dice: Local<u32>,
    mut commands: Commands,
) {
    let Some(assets) = assets else { return };
    let p = assets.physics;
    let dt = time.delta_secs();
    let filter = SpatialQueryFilter::from_excluded_entities(characters.iter());
    let projection = anchors.iter().next().and_then(|a| projection_of(a, &vm_cameras));
    for (e, mut shell, mut transform, children) in &mut shells {
        let s = &mut shell.state;
        let travel = s.velocity * dt;
        let hit = if s.resting || dt <= 0.0 {
            None
        } else {
            Dir3::new(travel).ok().and_then(|dir| {
                spatial
                    .cast_ray(s.position, dir, travel.length(), true, &filter)
                    .map(|h| (s.position + dir * h.distance + h.normal * 0.002, h.normal))
            })
        };
        let result = shell_step(s, dt, hit, &p);
        if let ShellHit::Bounced(vz) | ShellHit::Rested(vz) = result {
            *dice = dice.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            let chance = (*dice >> 8) as f32 / (1u32 << 24) as f32;
            if chance < p.sound_chance
                && let (Some(name), Some(sounds)) = (shell.bounce.as_deref(), sounds.as_ref())
                && let Some(entry) = sounds.0.entry(name)
            {
                let mut sound = PlaySound::at(name, shell.state.position);
                sound.volume = Some(entry.volume.draw(0.5) * bounce_volume(vz, &p));
                play.write(sound);
            }
        }
        let s = shell.state;
        let Some(alpha) = shell_alpha(s.age, &p) else {
            commands.entity(e).try_despawn();
            continue;
        };
        if alpha < 1.0
            && let Some(materials) = materials.as_mut()
        {
            for m in &shell.materials {
                if let Some(mut material) = materials.get_mut(m) {
                    material.alpha_mode = AlphaMode::Blend;
                    material.params.translucent = 1.0;
                    material.params.base_color.w = alpha;
                }
            }
        }
        // Before its first bounce it is drawn in the view-model pass, at
        // the point that pass shows where the world point is.
        let mut position = s.position;
        if !s.bounced
            && let Some((eye, world_fov, vm_fov)) = projection
        {
            let camera = eye.affine().inverse().transform_point3(s.position);
            let back = super::view_model::reproject(camera, vm_fov, world_fov);
            position = eye.transform_point(back);
        }
        transform.translation = position;
        transform.rotation = s.rotation;
        if s.bounced && !shell.in_world {
            shell.in_world = true;
            commands.entity(e).insert(RenderLayers::layer(0));
            for c in children {
                if let Ok(mut l) = layers.get_mut(*c) {
                    *l = RenderLayers::layer(0);
                }
            }
        }
    }
}

/// Remove shells (map unload).
pub(super) fn unload(world: &mut World) {
    world.remove_resource::<ShellAssets>();
    let shells: Vec<Entity> = world.query_filtered::<Entity, With<Shell>>().iter(world).collect();
    for s in shells {
        world.entity_mut(s).despawn();
    }
}
