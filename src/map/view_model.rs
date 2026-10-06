//! First-person view models (any game): the model a character sees of its
//! own weapon, with hands, drawn at the eye over the world. Games load
//! `MapViewModel`s with the map and drive each character's `ViewAnimator`
//! (which sequence plays); this module draws the one of the character
//! whose camera carries `ViewModelAnchor`.
//!
//! The model is drawn by its own camera on `VIEW_MODEL_LAYER`, after the
//! world camera and with a fresh depth buffer, so it never clips into walls,
//! and with its own field of view.

use std::{collections::HashMap, sync::Arc};

use bevy::{
    camera::visibility::RenderLayers,
    light::{NotShadowCaster, NotShadowReceiver},
    prelude::*,
};

use super::{BodyAssets, BodyJoint, MapBone, MapModel, anim};

/// Render layer of view models and the light that lights them.
pub const VIEW_MODEL_LAYER: usize = 2;

/// A weapon's view model, keyed like `MapHeldModel` (the weapon ID).
#[derive(Clone, Debug)]
pub struct MapViewModel {
    pub key: String,
    /// Meshes in the eye's space (camera axes: -Z forward, Y up; meters),
    /// skinned to `bones` in their reference pose.
    pub model: MapModel,
    pub bones: Vec<MapBone>,
    /// From the skeleton's space (the source game's axes and units) to the
    /// eye's.
    pub root: Transform,
    /// What it can play (None: it stays in the reference pose).
    pub animations: Option<Arc<anim::AnimSet>>,
    /// Vertical field of view it is drawn with (radians).
    pub fov: f32,
    /// Drawn mirrored left to right (Source's `cl_righthand` flips
    /// left-handed models).
    pub mirror: bool,
}

/// The loaded map's view models.
#[derive(Resource, Clone, Default)]
pub struct ViewModels(pub Arc<Vec<MapViewModel>>);

impl ViewModels {
    pub fn get(&self, key: &str) -> Option<&MapViewModel> {
        self.0.iter().find(|v| v.key == key)
    }
}

/// What a character's view model shows and plays. Games set the key and
/// drive the animator (simulation side, so it works headless); the drawing
/// follows it.
#[derive(Component, Clone, Debug, Default)]
pub struct ViewAnimator {
    /// The `MapViewModel` shown (None: nothing, e.g. dead).
    pub key: Option<String>,
    /// Plays on the view model's `AnimSet`.
    pub animator: Option<anim::Animator>,
}

impl ViewAnimator {
    /// Show `key`'s view model (a fresh animator on its sequences); no
    /// change if it is already shown.
    pub fn show(&mut self, key: Option<&str>, models: Option<&ViewModels>) -> bool {
        if self.key.as_deref() == key {
            return false;
        }
        self.key = key.map(str::to_string);
        self.animator = key
            .and_then(|k| models?.get(k)?.animations.clone())
            .map(anim::Animator::new);
        true
    }

    /// The sequence playing, by name.
    pub fn sequence(&self) -> Option<&str> {
        let a = self.animator.as_ref()?;
        Some(a.set.sequences.get(a.main?)?.name.as_str())
    }

    /// The activity of the sequence playing.
    pub fn activity(&self) -> Option<&str> {
        let a = self.animator.as_ref()?;
        Some(a.set.sequences.get(a.main?)?.activity.as_str())
    }
}

/// Put on a camera that is a child of a character (the client's
/// first-person camera): that character's view model is drawn at it.
#[derive(Component, Default)]
pub struct ViewModelAnchor;

/// The camera that draws view models (a child of the anchor).
#[derive(Component)]
pub struct ViewModelCamera;

/// The drawn view model under a `ViewModelCamera`.
#[derive(Component)]
pub(super) struct ViewModelBody {
    key: String,
    joints: Vec<Entity>,
}

/// Meshes, materials and bind poses of each view model, by key.
#[derive(Resource)]
pub(super) struct ViewModelAssets(pub(super) HashMap<String, (BodyAssets, f32, bool)>);

/// Keep a view-model camera under each anchor, matching the anchor
/// camera's target, order and tonemapping, and the shown view model under
/// it; then pose its joints from the character's `ViewAnimator`.
#[allow(clippy::type_complexity)]
pub(super) fn draw_view_models(
    time: Res<Time>,
    assets: Res<ViewModelAssets>,
    anchors: Query<
        (
            Entity,
            &ChildOf,
            &Camera,
            Option<&Children>,
            Option<&bevy::core_pipeline::tonemapping::Tonemapping>,
        ),
        (With<ViewModelAnchor>, Without<ViewModelCamera>),
    >,
    owners: Query<&ViewAnimator>,
    third_person: Option<Res<super::ShowLocalBody>>,
    mut cameras: Query<(Entity, &mut Camera, &mut Projection, Option<&Children>), With<ViewModelCamera>>,
    bodies: Query<&ViewModelBody>,
    mut joints: Query<&mut Transform, With<BodyJoint>>,
    mut commands: Commands,
) {
    let now = time.elapsed_secs_f64();
    for (anchor, parent, anchor_camera, children, tonemapping) in &anchors {
        let state = owners.get(parent.parent()).ok();
        // In third person the own body holds the weapon; no view model.
        let hidden = third_person.as_ref().is_some_and(|t| t.0);
        let shown = state
            .filter(|_| !hidden)
            .and_then(|s| s.key.as_deref())
            .and_then(|k| assets.0.get(k).map(|a| (k, a)));
        let camera = children.into_iter().flatten().find(|c| cameras.contains(**c)).copied();
        let Some((camera, mut cam, mut projection, cam_children)) = camera.and_then(|c| cameras.get_mut(c).ok()) else {
            let mut e = commands.spawn((
                Name::new("View model camera"),
                ViewModelCamera,
                Camera3d::default(),
                Camera {
                    order: anchor_camera.order + 1,
                    clear_color: ClearColorConfig::None,
                    ..default()
                },
                Projection::Perspective(PerspectiveProjection {
                    near: 0.01,
                    ..default()
                }),
                RenderLayers::layer(VIEW_MODEL_LAYER),
                Transform::default(),
                Visibility::default(),
                ChildOf(anchor),
            ));
            if let Some(t) = tonemapping {
                e.insert(*t);
            }
            continue;
        };
        if cam.order != anchor_camera.order + 1 || cam.is_active != anchor_camera.is_active {
            cam.order = anchor_camera.order + 1;
            cam.is_active = anchor_camera.is_active;
        }
        if let Some(t) = tonemapping {
            commands.entity(camera).insert(*t);
        }
        let body = cam_children
            .into_iter()
            .flatten()
            .find_map(|c| bodies.get(*c).ok().map(|b| (*c, b)));
        let current = body.as_ref().map(|(_, b)| b.key.as_str());
        if current != shown.map(|(k, _)| k) {
            if let Some((e, _)) = body {
                commands.entity(e).despawn();
            }
            if let Some((key, (a, fov, mirror))) = shown {
                if let Projection::Perspective(p) = &mut *projection {
                    p.fov = *fov;
                }
                spawn_body(&mut commands, camera, key, a, *mirror);
            }
            continue;
        }
        // Pose the shown model.
        let (Some((_, body)), Some(animator)) = (body, state.and_then(|s| s.animator.as_ref())) else {
            continue;
        };
        if animator.main.is_none() {
            continue;
        }
        for (joint, (q, p)) in body.joints.iter().zip(animator.pose(now)) {
            if let Ok(mut t) = joints.get_mut(*joint) {
                t.rotation = q;
                t.translation = p;
            }
        }
    }
}

fn spawn_body(commands: &mut Commands, camera: Entity, key: &str, assets: &BodyAssets, mirror: bool) {
    let layer = RenderLayers::layer(VIEW_MODEL_LAYER);
    let body = commands
        .spawn((
            Name::new(format!("View model {key}")),
            // Mirrored across the eye's left-right axis (its materials
            // cull the other face, see `ViewModelAssets`).
            Transform::from_scale(Vec3::new(if mirror { -1.0 } else { 1.0 }, 1.0, 1.0)),
            Visibility::Inherited,
            ChildOf(camera),
        ))
        .id();
    let root = commands.spawn((assets.root, Visibility::Inherited, ChildOf(body))).id();
    let mut joints: Vec<Entity> = Vec::with_capacity(assets.bones.len());
    for (i, b) in assets.bones.iter().enumerate() {
        let parent = b.parent.and_then(|p| joints.get(p).copied()).unwrap_or(root);
        joints.push(
            commands
                .spawn((
                    BodyJoint(i),
                    Transform::from_translation(b.position).with_rotation(b.rotation),
                    Visibility::Inherited,
                    ChildOf(parent),
                ))
                .id(),
        );
    }
    for (mesh, material) in &assets.parts {
        commands.spawn((
            Mesh3d(mesh.clone()),
            MeshMaterial3d(material.clone()),
            bevy::mesh::skinning::SkinnedMesh {
                inverse_bindposes: assets.bindposes.clone(),
                joints: joints.clone(),
            },
            layer.clone(),
            NotShadowCaster,
            NotShadowReceiver,
            // Its bounds are the reference pose's, not the animated one's.
            bevy::camera::visibility::NoFrustumCulling,
            ChildOf(body),
        ));
    }
    commands.entity(body).insert(ViewModelBody {
        key: key.to_string(),
        joints,
    });
}

/// Remove drawn view models (map unload); the cameras stay.
pub(super) fn unload(world: &mut World) {
    world.remove_resource::<ViewModels>();
    world.remove_resource::<ViewModelAssets>();
    let bodies: Vec<Entity> = world
        .query_filtered::<Entity, With<ViewModelBody>>()
        .iter(world)
        .collect();
    for b in bodies {
        world.entity_mut(b).despawn();
    }
    let states: Vec<Entity> = world
        .query_filtered::<Entity, With<ViewAnimator>>()
        .iter(world)
        .collect();
    for e in states {
        world.entity_mut(e).insert(ViewAnimator::default());
    }
}
