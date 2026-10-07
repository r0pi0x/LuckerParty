//! First-person view models (any game): the model a character sees of its
//! own weapon, with hands, drawn at the eye over the world. Games load
//! `MapViewModel`s with the map, drive each character's `ViewAnimator`
//! (which sequence plays) and `ViewModelOffset` (bob and sway), and send
//! `ViewModelEvent`s (muzzle flashes); this module draws the view model of
//! the character whose camera carries `ViewModelAnchor`.
//!
//! The model is drawn by its own camera on `VIEW_MODEL_LAYER`, after the
//! world camera and with a fresh depth buffer, so it never clips into walls,
//! and with its own field of view (`ViewModelSettings`). It is mirrored
//! across the eye's vertical plane when its handedness differs from the
//! player's, and lit per pixel by the map's light at its lighting origin
//! (`LightField`) plus the scene's point lights (`DynamicLight`).

use std::{collections::HashMap, sync::Arc};

use avian3d::prelude::{SpatialQuery, SpatialQueryFilter};
use bevy::{
    camera::visibility::RenderLayers,
    light::{NotShadowCaster, NotShadowReceiver},
    prelude::*,
};

use super::{
    BodyJoint, LightField, LightProbe, MapBone, MapData, MapDebugView, MapEnvmap, MapFlashLight, MapModel,
    MapMuzzleFlash, anim,
    prop_material::PropMaterial,
    sprite_material::{SpriteMaterial, SpriteParams},
};

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
    /// Built holding the weapon in the right hand (Source weapon scripts'
    /// `BuiltRightHanded`).
    pub right_handed: bool,
    /// May be mirrored to match the player's hand (`AllowFlipping`).
    pub allow_flipping: bool,
    /// Named points on bones (muzzle, ejection port).
    pub attachments: Vec<MapAttachment>,
    /// Where it is lit from, in the skeleton's space (Source: the model's
    /// illumination centre).
    pub light_origin: Vec3,
}

/// A point on a bone: `local` is in the bone's space (the skeleton's axes
/// and units).
#[derive(Clone, Debug)]
pub struct MapAttachment {
    pub name: String,
    pub bone: usize,
    pub local: Transform,
}

/// The loaded map's view models.
#[derive(Resource, Clone, Default)]
pub struct ViewModels(pub Arc<Vec<MapViewModel>>);

impl ViewModels {
    pub fn get(&self, key: &str) -> Option<&MapViewModel> {
        self.0.iter().find(|v| v.key == key)
    }
}

/// How view models are drawn (Source `viewmodel_fov`, `default_fov`,
/// `cl_righthand`, `r_drawviewmodel`). Games set the values and console
/// variables.
#[derive(Resource, Clone, Copy, Debug, PartialEq)]
pub struct ViewModelSettings {
    /// Horizontal field of view at 4:3, degrees.
    pub fov: f32,
    /// The player's unzoomed horizontal field of view at 4:3, degrees; the
    /// view model zooms by as many degrees as the player's view does.
    pub default_fov: f32,
    /// 1: the weapon in the right hand.
    pub right_hand: u8,
    /// 0: no view model.
    pub draw: u8,
}

impl Default for ViewModelSettings {
    fn default() -> Self {
        Self {
            fov: 54.0,
            default_fov: 90.0,
            right_hand: 1,
            draw: 1,
        }
    }
}

/// Which weapon effects are shown (Source `muzzleflash_light`,
/// `cl_ejectbrass`). Games set the values and console variables.
#[derive(Resource, Clone, Copy, Debug, PartialEq)]
pub struct EffectSettings {
    /// 1: muzzle flashes light their surroundings.
    pub muzzle_light: u8,
    /// 1: weapons eject shells.
    pub eject_brass: u8,
}

impl Default for EffectSettings {
    fn default() -> Self {
        Self {
            muzzle_light: 1,
            eject_brass: 1,
        }
    }
}

/// Where a character's view model sits relative to its eye (camera space:
/// X right, Y up, -Z forward; meters), e.g. walking bob and sway. Games
/// set it; it is mirrored with the model.
#[derive(Component, Clone, Copy, Debug, Default, PartialEq)]
pub struct ViewModelOffset {
    pub translation: Vec3,
    pub rotation: Quat,
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
    /// Not drawn for now, though still animated (e.g. behind a scope).
    pub hidden: bool,
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

/// Effects a character's view-model animation asks for. Games send them
/// from animation events; the map shows them at the view model's
/// attachment for the first-person view, else at the weapon held by the
/// character's body.
#[derive(Message, Clone, Debug, PartialEq)]
pub struct ViewModelEvent {
    pub owner: Entity,
    pub kind: ViewModelEventKind,
}

#[derive(Clone, Debug, PartialEq)]
pub enum ViewModelEventKind {
    /// A muzzle flash at the view model's attachment (index).
    MuzzleFlash { attachment: usize },
    /// A shell ejected from the attachment (index): the shell type's name
    /// and speed (game units per second).
    EjectBrass {
        attachment: usize,
        shell: String,
        speed: f32,
    },
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
    mirrored: bool,
    joints: Vec<Entity>,
    materials: Vec<Handle<PropMaterial>>,
    /// Where the probe was last sampled (world), and the cubemap used.
    lit_at: Option<Vec3>,
    cubemap: Option<usize>,
}

/// One drawable part of a view model: its mesh and its material drawn
/// normally and mirrored (front faces culled).
pub(super) struct ViewModelPart {
    mesh: Handle<Mesh>,
    materials: [Handle<PropMaterial>; 2],
    envmap: Option<MapEnvmap>,
}

pub(super) struct ViewModelAsset {
    parts: Vec<ViewModelPart>,
    bindposes: Handle<bevy::mesh::skinning::SkinnedMeshInverseBindposes>,
    bones: Vec<MapBone>,
    root: Transform,
}

/// Meshes, materials and bind poses of each view model, by key, and what
/// lights them.
#[derive(Resource)]
pub(super) struct ViewModelAssets {
    models: HashMap<String, ViewModelAsset>,
    /// Cubemap samples (position, image) for `env_cubemap` reflections.
    cubemaps: Vec<(Vec3, Handle<Image>)>,
    /// All cubemaps, for materials that name theirs.
    cubemap_images: Vec<Handle<Image>>,
    textures: Vec<Handle<Image>>,
    light_scale: f32,
}

/// Build the view models' render assets.
#[allow(clippy::too_many_arguments)]
pub(super) fn build_assets(
    data: &MapData,
    textures: &[Handle<Image>],
    cubemaps: &[Handle<Image>],
    view: MapDebugView,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<PropMaterial>,
    bindposes: &mut Assets<bevy::mesh::skinning::SkinnedMeshInverseBindposes>,
) -> ViewModelAssets {
    let models = data
        .view_models
        .iter()
        .map(|v| {
            let parts = v
                .model
                .meshes
                .iter()
                .map(|m| {
                    let mut mesh = super::build_mesh(m, false);
                    if m.joints.len() == m.positions.len() && !v.bones.is_empty() {
                        mesh.insert_attribute(
                            Mesh::ATTRIBUTE_JOINT_INDEX,
                            bevy::mesh::VertexAttributeValues::Uint16x4(m.joints.clone()),
                        );
                        mesh.insert_attribute(Mesh::ATTRIBUTE_JOINT_WEIGHT, m.joint_weights.clone());
                    }
                    let mut material = super::lit_prop_material(m, textures, data, view, false);
                    // Without a light field it shows its texture as is.
                    if !m.unlit && view != MapDebugView::Albedo && data.light_field.is_some() {
                        material.params.set_probe(&LightProbe::default(), 1.0);
                    }
                    let mut mirrored = material.clone();
                    mirrored.cull_front = true;
                    ViewModelPart {
                        mesh: meshes.add(mesh),
                        materials: [materials.add(material), materials.add(mirrored)],
                        // `PropMaterial` has no normal map, so a mask in its
                        // alpha (the hands') can't apply: no reflection
                        // rather than an unmasked one.
                        envmap: m
                            .envmap
                            .filter(|e| view == MapDebugView::Normal && e.mask != super::EnvmapMask::NormalAlpha),
                    }
                })
                .collect();
            // Each bone's reference pose in the eye's space, inverted.
            let mut global: Vec<Mat4> = Vec::with_capacity(v.bones.len());
            for b in &v.bones {
                let local = Mat4::from_rotation_translation(b.rotation, b.position);
                let parent = b
                    .parent
                    .and_then(|p| global.get(p).copied())
                    .unwrap_or(v.root.to_matrix());
                global.push(parent * local);
            }
            let inverse: Vec<Mat4> = global.iter().map(|m| m.inverse()).collect();
            (
                v.key.clone(),
                ViewModelAsset {
                    parts,
                    bindposes: bindposes.add(bevy::mesh::skinning::SkinnedMeshInverseBindposes::from(inverse)),
                    bones: v.bones.clone(),
                    root: v.root,
                },
            )
        })
        .collect();
    let lighting_scale = if let MapDebugView::Lighting { scale } = view {
        scale
    } else {
        1.0
    };
    ViewModelAssets {
        models,
        cubemaps: data
            .cubemap_samples
            .iter()
            .filter_map(|(p, i)| Some((*p, cubemaps.get(*i)?.clone())))
            .collect(),
        cubemap_images: cubemaps.to_vec(),
        textures: textures.to_vec(),
        light_scale: data.look.light_scale * lighting_scale,
    }
}

/// Whether a view model is drawn mirrored (Source: `AllowFlipping` and
/// `BuiltRightHanded` != `cl_righthand`).
pub fn mirrored(right_handed: bool, allow_flipping: bool, right_hand: bool) -> bool {
    allow_flipping && right_handed != right_hand
}

/// A horizontal field of view given for a 4:3 screen, for a screen of
/// `aspect` (width / height). Degrees.
pub fn aspect_fov(fov_43: f32, aspect: f32) -> f32 {
    2.0 * ((fov_43.to_radians() / 2.0).tan() * aspect / (4.0 / 3.0))
        .atan()
        .to_degrees()
}

/// The vertical field of view of a horizontal 4:3 one (the same on every
/// screen). Degrees.
pub fn vertical_fov(fov_43: f32) -> f32 {
    2.0 * ((fov_43.to_radians() / 2.0).tan() * 0.75).atan().to_degrees()
}

/// The view model's horizontal 4:3 field of view: zoomed by as many
/// degrees as the player's (`player_fov`, 4:3) is below `default_fov`.
pub fn view_model_fov(settings: &ViewModelSettings, player_fov: f32) -> f32 {
    (settings.fov - (settings.default_fov - player_fov)).clamp(0.1, 179.9)
}

/// A point seen in the view-model pass (camera space), moved so that it
/// appears at the same place on screen in the world pass: right and up
/// scaled by tan(world fov / 2) / tan(view-model fov / 2), any matching
/// pair of fields of view (both vertical or both horizontal), in degrees.
pub fn reproject(p: Vec3, world_fov: f32, view_model_fov: f32) -> Vec3 {
    let k = (world_fov.to_radians() / 2.0).tan() / (view_model_fov.to_radians() / 2.0).tan().max(1e-6);
    Vec3::new(p.x * k, p.y * k, p.z)
}

/// The view model's placement under its camera: the offset, mirrored
/// across the camera's vertical plane (X) when `mirror` (the mirror acts
/// on the final model, so it mirrors the offset too).
pub fn placement(offset: &ViewModelOffset, mirror: bool) -> Transform {
    if !mirror {
        return Transform::from_translation(offset.translation).with_rotation(offset.rotation);
    }
    let (t, q) = (offset.translation, offset.rotation);
    Transform {
        translation: Vec3::new(-t.x, t.y, t.z),
        rotation: Quat::from_xyzw(q.x, -q.y, -q.z, q.w),
        scale: Vec3::new(-1.0, 1.0, 1.0),
    }
}

/// An attachment of `model` posed by `pose` (local bone poses), placed by
/// `placement`: in camera space.
pub fn attachment_transform(
    model: &MapViewModel,
    pose: &[anim::BonePose],
    attachment: usize,
    placement: &Transform,
) -> Option<Mat4> {
    let a = model.attachments.get(attachment)?;
    let mut global: Vec<Mat4> = Vec::with_capacity(model.bones.len());
    for (i, b) in model.bones.iter().enumerate() {
        let (q, p) = pose.get(i).copied().unwrap_or((b.rotation, b.position));
        let local = Mat4::from_rotation_translation(q, p);
        global.push(match b.parent.and_then(|p| global.get(p)) {
            Some(parent) => *parent * local,
            None => local,
        });
    }
    Some(placement.to_matrix() * model.root.to_matrix() * *global.get(a.bone)? * a.local.to_matrix())
}

/// The camera's horizontal 4:3 field of view (degrees) from its vertical
/// one.
fn camera_fov_43(projection: &Projection) -> Option<f32> {
    let Projection::Perspective(p) = projection else {
        return None;
    };
    Some(2.0 * ((p.fov / 2.0).tan() / 0.75).atan().to_degrees())
}

/// Keep a view-model camera under each anchor, matching the anchor
/// camera's target, order and tonemapping, and the shown view model under
/// it; then place, pose and light it.
#[allow(clippy::type_complexity, clippy::too_many_arguments)]
pub(super) fn draw_view_models(
    time: Res<Time>,
    assets: Res<ViewModelAssets>,
    settings: Option<Res<ViewModelSettings>>,
    models: Option<Res<ViewModels>>,
    light_field: Option<Res<LightField>>,
    anchors: Query<
        (
            Entity,
            &ChildOf,
            &Camera,
            &Projection,
            &GlobalTransform,
            Option<&Children>,
            Option<&bevy::core_pipeline::tonemapping::Tonemapping>,
            Option<&bevy::camera::RenderTarget>,
        ),
        (With<ViewModelAnchor>, Without<ViewModelCamera>),
    >,
    owners: Query<(&ViewAnimator, Option<&ViewModelOffset>)>,
    third_person: Option<Res<super::ShowLocalBody>>,
    mut cameras: Query<
        (
            Entity,
            &mut Camera,
            &mut Projection,
            Option<&Children>,
            Option<&bevy::camera::RenderTarget>,
        ),
        With<ViewModelCamera>,
    >,
    mut bodies: Query<(&mut ViewModelBody, &mut Transform), Without<BodyJoint>>,
    mut joints: Query<&mut Transform, With<BodyJoint>>,
    mut materials: Option<ResMut<Assets<PropMaterial>>>,
    mut commands: Commands,
) {
    let now = time.elapsed_secs_f64();
    let settings = settings.map(|s| *s).unwrap_or_default();
    for (anchor, parent, anchor_camera, anchor_projection, eye, children, tonemapping, anchor_target) in &anchors {
        let state = owners.get(parent.parent()).ok();
        // In third person the own body holds the weapon; no view model.
        let hidden = third_person.as_ref().is_some_and(|t| t.0)
            || settings.draw == 0
            || state.is_some_and(|(s, _)| s.hidden);
        let shown = state
            .filter(|_| !hidden)
            .and_then(|(s, _)| s.key.as_deref())
            .and_then(|k| Some((k, assets.models.get(k)?, models.as_ref()?.get(k)?)));
        let camera = children.into_iter().flatten().find(|c| cameras.contains(**c)).copied();
        let Some((camera, mut cam, mut projection, cam_children, target)) =
            camera.and_then(|c| cameras.get_mut(c).ok())
        else {
            let mut e = commands.spawn((
                Name::new("View model camera"),
                ViewModelCamera,
                Camera3d::default(),
                Camera {
                    order: anchor_camera.order + 1,
                    clear_color: ClearColorConfig::None,
                    ..default()
                },
                Projection::Perspective(PerspectiveProjection::default()),
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
        // Off-screen captures (`--views`) retarget the anchor camera; the
        // view model must draw into the same image.
        if let Some(t) = anchor_target
            && target.and_then(|c| c.normalize(None)) != t.normalize(None)
        {
            commands.entity(camera).insert(t.clone());
        }
        // Field of view (spec view_models.md 5): the view model's, zoomed
        // with the player's, constant vertically. Near plane: Source's is 1
        // game unit, but that cuts into the knife's hands here (open
        // question in docs/plans/active/view-models.md), so 1 cm.
        if let (Some(player), Projection::Perspective(p)) = (camera_fov_43(anchor_projection), &mut *projection) {
            let fov = vertical_fov(view_model_fov(&settings, player)).to_radians();
            let near = 0.01;
            if (p.fov - fov).abs() > 1e-5 || p.near != near {
                p.fov = fov;
                p.near = near;
            }
        }
        let body = cam_children
            .into_iter()
            .flatten()
            .find(|c| bodies.contains(**c))
            .copied();
        let mirror =
            shown.is_some_and(|(_, _, m)| mirrored(m.right_handed, m.allow_flipping, settings.right_hand != 0));
        let current = body
            .and_then(|b| bodies.get(b).ok())
            .map(|(b, _)| (b.key.clone(), b.mirrored));
        if current != shown.map(|(k, _, _)| (k.to_string(), mirror)) {
            if let Some(e) = body {
                commands.entity(e).try_despawn();
            }
            if let Some((key, a, _)) = shown {
                spawn_body(&mut commands, camera, key, a, mirror);
            }
            continue;
        }
        let (Some(body), Some((_, asset, model))) = (body, shown) else {
            continue;
        };
        let Ok((mut body, mut placed)) = bodies.get_mut(body) else {
            continue;
        };
        let offset = state.and_then(|(_, o)| o.copied()).unwrap_or_default();
        *placed = placement(&offset, mirror);
        // Pose the shown model.
        if let Some(animator) = state
            .and_then(|(s, _)| s.animator.as_ref())
            .filter(|a| a.main.is_some())
        {
            for (joint, (q, p)) in body.joints.iter().zip(animator.pose(now)) {
                if let Ok(mut t) = joints.get_mut(*joint) {
                    t.rotation = q;
                    t.translation = p;
                }
            }
        }
        // Light it from one point near the eye (spec view_models.md 9): its
        // lighting origin, placed with the model but not mirrored.
        let Some(materials) = materials.as_mut() else { continue };
        let origin = eye.transform_point(
            Transform::from_translation(offset.translation)
                .with_rotation(offset.rotation)
                .transform_point(asset.root.transform_point(model.light_origin)),
        );
        let moved = body
            .lit_at
            .is_none_or(|at| at.distance(origin) > 0.5 * asset.root.scale.x);
        if let (true, Some(field)) = (moved, light_field.as_ref()) {
            let probe = (field.0.0)(origin);
            for m in &body.materials {
                if let Some(mut material) = materials.get_mut(m)
                    && material.params.probe > 0.5
                {
                    material.params.set_probe(&probe, assets.light_scale);
                }
            }
            body.lit_at = Some(origin);
        }
        // Reflections: the cubemap nearest the eye.
        let nearest = assets
            .cubemaps
            .iter()
            .enumerate()
            .min_by(|a, b| a.1.0.distance(origin).total_cmp(&b.1.0.distance(origin)))
            .map(|(i, _)| i);
        if nearest != body.cubemap
            && let Some(i) = nearest
        {
            for (part, m) in asset.parts.iter().zip(&body.materials) {
                if let (Some(env), Some(mut material)) = (part.envmap, materials.get_mut(m)) {
                    let cube = env
                        .cubemap
                        .and_then(|c| assets.cubemap_images.get(c).cloned())
                        .unwrap_or_else(|| assets.cubemaps[i].1.clone());
                    super::set_prop_envmap(&mut material, &env, cube, &assets.textures);
                }
            }
            body.cubemap = nearest;
        }
    }
}

fn spawn_body(commands: &mut Commands, camera: Entity, key: &str, asset: &ViewModelAsset, mirror: bool) {
    let layer = RenderLayers::layer(VIEW_MODEL_LAYER);
    let body = commands
        .spawn((
            Name::new(format!("View model {key}")),
            placement(&ViewModelOffset::default(), mirror),
            Visibility::Inherited,
            ChildOf(camera),
        ))
        .id();
    let root = commands.spawn((asset.root, Visibility::Inherited, ChildOf(body))).id();
    let mut joints: Vec<Entity> = Vec::with_capacity(asset.bones.len());
    for (i, b) in asset.bones.iter().enumerate() {
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
    let mut materials = Vec::with_capacity(asset.parts.len());
    for part in &asset.parts {
        // Mirroring turns triangles inside out: the mirrored material culls
        // front faces.
        let material = part.materials[mirror as usize].clone();
        materials.push(material.clone());
        commands.spawn((
            Mesh3d(part.mesh.clone()),
            MeshMaterial3d(material),
            bevy::mesh::skinning::SkinnedMesh {
                inverse_bindposes: asset.bindposes.clone(),
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
        mirrored: mirror,
        joints,
        materials,
        lit_at: None,
        cubemap: None,
    });
}

/// A brief point light (muzzle flash) that lights the world, props and
/// view models (see `dynamic_light` in world.wgsl and prop.wgsl): its
/// radius shrinks to 0 over its life.
#[derive(Component, Clone, Copy, Debug)]
pub struct DynamicLight {
    pub radius: f32,
    pub life: f32,
    pub age: f32,
}

/// A flash sprite, removed after `life` seconds.
#[derive(Component)]
pub(super) struct FlashSprite {
    life: f32,
    age: f32,
}

/// Where a character's held weapon shoots from (its world model's muzzle
/// attachment): a child of the held model.
#[derive(Component)]
pub(super) struct HeldMuzzle {
    pub(super) owner: Entity,
}

/// What muzzle flashes look like, ready to draw.
#[derive(Resource)]
pub(super) struct FlashAssets {
    data: MapMuzzleFlash,
    mesh: Handle<Mesh>,
    material: Option<Handle<SpriteMaterial>>,
}

pub(super) fn build_flash_assets(
    data: &MapData,
    textures: &[Handle<Image>],
    meshes: &mut Assets<Mesh>,
    sprites: Option<&mut Assets<SpriteMaterial>>,
) -> Option<FlashAssets> {
    let flash = data.muzzle_flash.clone()?;
    let size = flash
        .texture
        .and_then(|t| data.textures.get(t))
        .map_or(UVec2::ONE, |t| UVec2::new(t.width, t.height));
    let [r, g, b] = flash.color;
    let material = sprites
        .zip(flash.texture.and_then(|t| textures.get(t)))
        .map(|(sprites, t)| {
            sprites.add(SpriteMaterial {
                params: SpriteParams {
                    color: Vec4::new(r, g, b, 1.0),
                    size: Vec2::ONE,
                },
                texture: Some(t.clone()),
                glow: false,
            })
        });
    Some(FlashAssets {
        mesh: meshes.add(super::sprite_material::sprite_mesh(size)),
        material,
        data: flash,
    })
}

/// A small deterministic random source for effect variation.
#[derive(Resource, Default)]
pub(super) struct FlashDice(u32);

impl FlashDice {
    fn roll(&mut self) -> f32 {
        let mut x = self.0.wrapping_add(0x9e37_79b9).max(1);
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        self.0 = x;
        (x >> 8) as f32 / (1u32 << 24) as f32
    }

    fn range(&mut self, (lo, hi): (f32, f32)) -> f32 {
        lo + (hi - lo) * self.roll()
    }
}

/// Show muzzle flashes: at the view model's (re-projected) attachment for
/// the first-person view (spec view_models.md 5, "Attachment positions for
/// effects"), else at the held weapon's muzzle; a sprite plume and a
/// point light.
#[allow(clippy::type_complexity, clippy::too_many_arguments)]
pub(super) fn muzzle_flashes(
    mut events: MessageReader<ViewModelEvent>,
    flash: Option<Res<FlashAssets>>,
    models: Option<Res<ViewModels>>,
    time: Res<Time>,
    anchors: Query<(&ChildOf, &GlobalTransform, &Projection, &Children), With<ViewModelAnchor>>,
    vm_cameras: Query<&Projection, (With<ViewModelCamera>, Without<ViewModelAnchor>)>,
    bodies: Query<&ViewModelBody>,
    owners: Query<(&ViewAnimator, Option<&ViewModelOffset>)>,
    cam_children: Query<&Children, With<ViewModelCamera>>,
    muzzles: Query<(&HeldMuzzle, &GlobalTransform)>,
    characters: Query<Entity, With<crate::core::Intent>>,
    spatial: SpatialQuery,
    mut pool: LightPool,
    mut dice: Local<FlashDice>,
    effects: Option<Res<EffectSettings>>,
    mut commands: Commands,
) {
    let Some(flash) = flash else {
        events.clear();
        return;
    };
    // Lights ready before the first shot (see `spawn_flash_light`).
    if pool.is_empty() && flash.data.light.is_some() {
        for _ in 0..4 {
            commands.spawn((
                Name::new("Muzzle flash light"),
                PointLight {
                    color: Color::BLACK,
                    intensity: 4.0 * std::f32::consts::PI,
                    range: 1e-4,
                    radius: 0.0,
                    shadow_maps_enabled: false,
                    ..default()
                },
                DynamicLight {
                    radius: 0.0,
                    life: 0.0,
                    age: 0.0,
                },
                RenderLayers::from_layers(&[0, VIEW_MODEL_LAYER]),
                Transform::default(),
            ));
        }
    }
    let effects = effects.map(|e| *e).unwrap_or_default();
    let now = time.elapsed_secs_f64();
    for ev in events.read() {
        let ViewModelEventKind::MuzzleFlash { attachment } = ev.kind else {
            continue;
        };
        // First person: the owner's anchor has its view model drawn.
        let first_person = anchors.iter().find_map(|(parent, eye, projection, children)| {
            if parent.parent() != ev.owner {
                return None;
            }
            let vm_camera = children.iter().find(|c| vm_cameras.contains(*c))?;
            let body = cam_children
                .get(vm_camera)
                .ok()?
                .iter()
                .find_map(|c| bodies.get(c).ok())?;
            let (view, offset) = owners.get(ev.owner).ok()?;
            let model = models.as_ref()?.get(&body.key)?;
            let animator = view.animator.as_ref()?;
            let pose = animator.pose(now);
            let placed = placement(&offset.copied().unwrap_or_default(), body.mirrored);
            let m = attachment_transform(model, &pose, attachment, &placed)?;
            let (Projection::Perspective(world), Ok(Projection::Perspective(vm))) =
                (projection, vm_cameras.get(vm_camera))
            else {
                return None;
            };
            let (wf, vf) = (world.fov.to_degrees(), vm.fov.to_degrees());
            let muzzle = m.transform_point3(Vec3::ZERO);
            let at = reproject(muzzle, wf, vf);
            // The plume goes along the model's forward (the skeleton's +X),
            // not the attachment's own axes (on the AK they point sideways).
            let forward = (placed.to_matrix() * model.root.to_matrix()).transform_vector3(Vec3::X);
            let ahead = reproject(muzzle + forward, wf, vf);
            Some((
                eye.transform_point(at),
                (eye.transform_point(ahead) - eye.transform_point(at)).normalize_or_zero(),
            ))
        });
        let world_model = || {
            muzzles
                .iter()
                .find(|(m, _)| m.owner == ev.owner)
                .map(|(_, g)| (g.translation(), g.rotation() * Vec3::X))
        };
        let Some((at, forward)) = first_person.or_else(world_model) else {
            continue;
        };
        // Tracers start here (map::tracer).
        let owner = ev.owner;
        commands.queue(move |w: &mut World| {
            w.resource_mut::<super::tracer::MuzzleCache>().0.insert(owner, (at, now));
        });
        let data = &flash.data;
        if let Some(material) = &flash.material {
            let scale = dice.range(data.scale);
            for (ahead, size) in &data.sprites {
                commands.spawn((
                    Name::new("Muzzle flash"),
                    Mesh3d(flash.mesh.clone()),
                    MeshMaterial3d(material.clone()),
                    Transform::from_translation(at + forward * *ahead * scale),
                    FlashSprite {
                        life: data.life,
                        age: 0.0,
                    },
                    FlashSize(*size * scale),
                    NotShadowCaster,
                    bevy::camera::visibility::NoFrustumCulling,
                ));
            }
        }
        if let Some(light) = data.light.filter(|_| effects.muzzle_light != 0) {
            spawn_flash_light(
                &mut commands,
                &mut pool,
                &spatial,
                &characters,
                at,
                forward,
                light,
                &mut dice,
            );
        }
    }
}

/// A flash sprite's full size (meters); its material is shared, so the
/// size is applied per entity by `size_flash_sprites`.
#[derive(Component, Clone, Copy)]
pub(super) struct FlashSize(f32);

/// Most muzzle-flash lights at once; more flashes reuse the oldest.
const MAX_FLASH_LIGHTS: usize = 8;

type LightPool<'w, 's> = Query<
    'w,
    's,
    (
        Entity,
        &'static mut DynamicLight,
        &'static mut PointLight,
        &'static mut Transform,
    ),
>;

#[allow(clippy::too_many_arguments)]
fn spawn_flash_light(
    commands: &mut Commands,
    pool: &mut LightPool,
    spatial: &SpatialQuery,
    characters: &Query<Entity, With<crate::core::Intent>>,
    at: Vec3,
    forward: Vec3,
    light: MapFlashLight,
    dice: &mut FlashDice,
) {
    // Keep the light off a wall the muzzle touches or pokes into: look
    // along the barrel from behind and stop `CLEARANCE` short of what is
    // hit, so the wall is lit from the front rather than grazed.
    const BACK: f32 = 0.6;
    const CLEARANCE: f32 = 0.2;
    let from = at - forward * BACK;
    let mut position = at;
    if let Ok(dir) = Dir3::new(forward) {
        let filter = SpatialQueryFilter::from_excluded_entities(characters.iter());
        if let Some(hit) = spatial.cast_ray(from, dir, BACK + CLEARANCE, true, &filter) {
            position = from + forward * (hit.distance - CLEARANCE).clamp(0.0, BACK);
        }
    }
    let radius = dice.range(light.radius);
    let color = Color::LinearRgba(LinearRgba::rgb(light.color.x, light.color.y, light.color.z));
    let state = DynamicLight {
        radius,
        life: light.life,
        age: 0.0,
    };
    // Reuse a light that has gone out (or the oldest): new light entities
    // only reach the light clusters a frame or two after they appear,
    // longer than a flash lasts.
    let count = pool.iter().count();
    let reuse = pool
        .iter()
        .max_by(|a, b| (a.1.age / a.1.life.max(1e-6)).total_cmp(&(b.1.age / b.1.life.max(1e-6))))
        .filter(|(_, d, _, _)| d.age >= d.life || count >= MAX_FLASH_LIGHTS)
        .map(|(e, ..)| e);
    if let Some((_, mut d, mut point, mut t)) = reuse.and_then(|e| pool.get_mut(e).ok()) {
        *d = state;
        point.color = color;
        point.range = radius;
        t.translation = position;
        return;
    }
    commands.spawn((
        Name::new("Muzzle flash light"),
        PointLight {
            // The shaders read colour x intensity / (4 pi) as lightmap
            // units (see `dynamic_light` in world.wgsl).
            color,
            intensity: 4.0 * std::f32::consts::PI,
            range: radius,
            radius: 0.0,
            shadow_maps_enabled: false,
            ..default()
        },
        state,
        RenderLayers::from_layers(&[0, VIEW_MODEL_LAYER]),
        Transform::from_translation(position),
    ));
}

/// Shrink dynamic lights' radius to 0 over their life, then turn them off
/// (kept for reuse); remove flash sprites after their life.
pub(super) fn age_effects(
    time: Res<Time>,
    mut lights: Query<(&mut DynamicLight, &mut PointLight)>,
    mut sprites: Query<(Entity, &mut FlashSprite)>,
    mut commands: Commands,
) {
    let dt = time.delta_secs();
    for (mut d, mut light) in &mut lights {
        if d.age >= d.life {
            continue;
        }
        d.age += dt;
        if d.age >= d.life {
            light.color = Color::BLACK;
            light.range = 1e-4;
        } else {
            light.range = (d.radius * (1.0 - d.age / d.life)).max(1e-4);
        }
    }
    for (e, mut s) in &mut sprites {
        // Shown for at least one frame.
        if s.age >= s.life {
            commands.entity(e).try_despawn();
        }
        s.age += dt;
    }
}

/// Flash sprites share one material; give each its own size by a material
/// per size bucket.
pub(super) fn size_flash_sprites(
    flash: Option<Res<FlashAssets>>,
    mut sprites: Query<(&FlashSize, &mut MeshMaterial3d<SpriteMaterial>), Added<FlashSize>>,
    mut materials: Option<ResMut<Assets<SpriteMaterial>>>,
    mut cache: Local<HashMap<u32, Handle<SpriteMaterial>>>,
) {
    let (Some(flash), Some(materials)) = (flash, materials.as_mut()) else {
        return;
    };
    let Some(base) = flash.material.as_ref().and_then(|m| materials.get(m)).cloned() else {
        return;
    };
    for (size, mut material) in &mut sprites {
        // Millimetre buckets.
        let key = (size.0 * 1000.0).round() as u32;
        let handle = cache
            .entry(key)
            .or_insert_with(|| {
                let mut m = base.clone();
                m.params.size = Vec2::splat(key as f32 / 1000.0);
                materials.add(m)
            })
            .clone();
        material.0 = handle;
    }
}

/// Remove drawn view models (map unload); the cameras stay.
pub(super) fn unload(world: &mut World) {
    world.remove_resource::<ViewModels>();
    world.remove_resource::<ViewModelAssets>();
    world.remove_resource::<FlashAssets>();
    world.remove_resource::<LightField>();
    let bodies: Vec<Entity> = world
        .query_filtered::<Entity, Or<(With<ViewModelBody>, With<FlashSprite>, With<DynamicLight>)>>()
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
