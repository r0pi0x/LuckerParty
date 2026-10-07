//! The Map slot's neutral data: what any game's map importer produces and
//! the engine spawns. Units are meters, Y up (README: mounts own each game's
//! units and axes; the engine sees only this).

use std::{collections::HashMap, sync::Arc};

use avian3d::prelude::*;
use bevy::{
    asset::RenderAssetUsages,
    image::{ImageAddressMode, ImageFilterMode, ImageSampler, ImageSamplerDescriptor},
    mesh::Indices,
    prelude::*,
    render::render_resource::{Extent3d, PrimitiveTopology, TextureDimension, TextureFormat},
};

use crate::core::{MovingSolid, SpawnPoint, Team};
// Collision-world types live in `core` (the greybox map uses them too).
pub use crate::core::{
    BrushTreeNode, MapBrush, MapBrushCollider, MapBrushTree, MapBrushes, MapTerrain, MapTerrainCollider, MapWater, MapWaterVolume, PropSurface,
};

pub mod anim;
pub mod decal;
pub mod entities;
pub mod fire;
pub mod fog;
pub use entities::{MapBrushEntity, MapEntities, MapEntity, MapHull};
pub mod breakables;
pub mod contact_filter;
pub use breakables::{BreakProp, BrushPanes, GlassShatter, MapBreak, MapBreakPiece, SpawnGibs};
mod dust;
pub mod hud;
pub mod hearing;
pub mod live_sound;
pub mod loose;
pub mod probe_lit;
pub mod radio;
pub mod nav;
pub mod particles;
pub mod tracer;
pub mod prop_material;
pub mod prop_physics;
pub mod ragdoll;
pub use ragdoll::{MapCollisionHooks, MapRagdoll, MapRagdollBody, MapRagdollJoint, Ragdoll, RagdollBody, RagdollShot};
pub mod rope_material;
pub mod shadows;
pub mod sound;
pub mod soundscape;
pub use live_sound::{LiveSounds, SoundControl, SoundKey, StartSound};
pub use sound::{MapSoundClip, MapSoundEntry, MapSounds, MapSurface, PlaySound, SoundLevel};
pub mod shells;
pub mod sprite_material;
pub mod surface_color;
pub mod view_model;
pub mod vis;
pub mod water;
pub mod world_material;
pub use view_model::{
    DynamicLight, EffectSettings, MapAttachment, MapViewModel, ViewAnimator, ViewModelAnchor, ViewModelCamera,
    ViewModelEvent, ViewModelEventKind, ViewModelOffset, ViewModelSettings, ViewModelSource, ViewModels,
};

use prop_material::{PropMaterial, PropParams};
use rope_material::{RopeMaterial, RopeParams};
use sprite_material::{SpriteMaterial, SpriteParams};
use world_material::{WorldMaterial, WorldParams};

/// Signs for normal maps' red and green channels in the world material.
/// Source's normal maps are DirectX-style (x along texture u, y along v,
/// image-down) and used unflipped (specs/cs_source/shaders.md). Overridable
/// with MASHUP_NORMAL_X_SIGN / MASHUP_NORMAL_G_SIGN for experiments.
const NORMAL_G_SIGN: f32 = 1.0;
const NORMAL_X_SIGN: f32 = 1.0;

/// Source LDR lightmaps (specs/cs_source/shaders.md): each linear value L is
/// rounded to 1/1024, clamped to 4095/1024, stored as the 8-bit sRGB texel
/// 0.5 L^(1/2.2), and multiplied back by 2^2.2 after the sampler's decode.
const SOURCE_LIGHTMAP_SCALE: f32 = 4.594_793;

pub fn source_ldr_texel(l: f32) -> u8 {
    let i = (l * 1024.0).round().clamp(0.0, 4095.0);
    (255.0 * 0.5 * (i / 1024.0).powf(1.0 / 2.2)).round() as u8
}

/// Source LDR bump pages (specs/cs_source/shaders.md, "Bump page encoding
/// at upload"): the three directional values of a luxel (linear) are
/// scaled so their mean is the flat value's encoded level, overflow above 1
/// is shared out to the other pages, and the result is stored linearly
/// (the sampler's sRGB decode x 2^2.2 still applies when reading).
pub fn source_ldr_bump_texels(flat: [f32; 3], pages: [[f32; 3]; 3]) -> [[u8; 3]; 3] {
    let mut q = [[0.0f32; 3]; 3];
    for c in 0..3 {
        let i = (flat[c] * 1024.0).round().clamp(0.0, 4095.0);
        let goal = 0.5 * (i / 1024.0).powf(1.0 / 2.2);
        let mean = (pages[0][c] + pages[1][c] + pages[2][c]) / 3.0;
        let s = if mean > 0.0 { goal / mean } else { 0.0 };
        for k in 0..3 {
            q[k][c] = pages[k][c] * s;
        }
    }
    let max = |v: [f32; 3]| v[0].max(v[1]).max(v[2]);
    // Order once by largest channel, descending; ties go to the last
    // matching permutation in this list.
    const ORDERS: [[usize; 3]; 6] = [[0, 1, 2], [0, 2, 1], [1, 0, 2], [1, 2, 0], [2, 0, 1], [2, 1, 0]];
    let m = q.map(max);
    let order = ORDERS
        .iter()
        .rev()
        .find(|o| m[o[0]] >= m[o[1]] && m[o[1]] >= m[o[2]])
        .copied()
        .unwrap_or([0, 1, 2]);
    for &k in &order {
        let top = max(q[k]);
        if top > 1.0 {
            let excess = q[k].map(|v| v * (top - 1.0) / top);
            for c in 0..3 {
                q[k][c] -= excess[c];
                for other in (0..3).filter(|&o| o != k) {
                    q[other][c] += excess[c] / 2.0;
                }
            }
        }
    }
    q.map(|v| {
        let top = max(v);
        let v = if top > 1.0 { v.map(|x| x / top) } else { v };
        v.map(|x| (255.0 * x.max(0.0)).round().clamp(0.0, 255.0) as u8)
    })
}

/// An RGBA8 texture in sRGB, top row first.
#[derive(Clone, Debug)]
pub struct MapTexture {
    /// Source path, e.g. `materials/de_dust/sitebwall01.vtf`.
    pub name: String,
    /// Color data (sRGB) or data such as normal maps (linear).
    pub srgb: bool,
    /// Smaller mip levels shipped with the texture (RGBA8, halving each
    /// time). Empty: generate them.
    pub mips: Vec<Vec<u8>>,
    pub width: u32,
    pub height: u32,
    pub rgba8: Vec<u8>,
}

/// How a surface's alpha is used.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub enum MapAlpha {
    #[default]
    Opaque,
    /// Cut out below this alpha.
    Mask(f32),
    Blend,
    /// Added to what's behind (Source `$additive`: light glows, beams).
    Add,
}

impl MapAlpha {
    /// The material shaders' `translucent` parameter: 0 opaque, 1 blend,
    /// 2 additive.
    pub fn shader_mode(self) -> f32 {
        match self {
            MapAlpha::Opaque | MapAlpha::Mask(_) => 0.0,
            MapAlpha::Blend => 1.0,
            MapAlpha::Add => 2.0,
        }
    }

    /// Blend state for the material shaders (they output alpha 0 when
    /// additive, so premultiplied blending adds).
    pub fn shader_alpha_mode(self) -> AlphaMode {
        match self {
            MapAlpha::Opaque | MapAlpha::Mask(_) => AlphaMode::Opaque,
            MapAlpha::Blend => AlphaMode::Blend,
            MapAlpha::Add => AlphaMode::Add,
        }
    }
}

/// Triangles sharing one material.
#[derive(Clone, Debug, Default)]
pub struct MapMesh {
    /// The source game's material name, e.g. `de_dust/sitebwall01`.
    pub material: String,
    /// Part of the 3D skybox (drawn by the sky camera, see `MapSkyCamera`).
    pub skybox: bool,
    pub positions: Vec<[f32; 3]>,
    pub normals: Vec<[f32; 3]>,
    /// Texture coordinates in texture repeats (1.0 = one texture width).
    pub uvs: Vec<[f32; 2]>,
    /// Skinned meshes: up to four bones per vertex (indices into the
    /// model's `MapBone`s) and their weights (summing to 1). Empty when
    /// the mesh isn't skinned.
    pub joints: Vec<[u16; 4]>,
    pub joint_weights: Vec<[f32; 4]>,
    /// Counter-clockwise triangles when seen from the front.
    pub indices: Vec<u32>,
    /// Flat color, used when there's no texture.
    pub color: [u8; 3],
    /// Index into `MapData::textures`.
    pub texture: Option<usize>,
    pub alpha: MapAlpha,
    pub double_sided: bool,
    /// Tangent-space normal map (index into `MapData::textures`, linear).
    pub normal_map: Option<usize>,
    /// Per-vertex coordinates into `MapData::lightmap` (0..1); empty when
    /// the map has no baked lighting.
    pub lightmap_uvs: Vec<[f32; 2]>,
    /// A second texture blended in per vertex (Source's WorldVertexTransition
    /// on displacements).
    pub blend: Option<MapBlend>,
    /// Per-vertex blend weight: 0 = first texture, 1 = second. Empty when
    /// there's no blend.
    pub blend_weights: Vec<f32>,
    /// A detail texture tiled over the base texture.
    pub detail: Option<MapDetail>,
    /// Drawn at the texture's own brightness, ignoring lighting (Source's
    /// UnlitGeneric).
    pub unlit: bool,
    /// Surface property name (footsteps, impacts), lower-case.
    pub surface: Option<String>,
    /// Reflection of a baked cubemap (Source `$envmap`).
    pub envmap: Option<MapEnvmap>,
    /// Part of a mover brush entity (index into `MapData::entities`):
    /// positions are relative to that entity's origin and unrotated
    /// (engine axes), drawn under its `MapBrushEntity` node.
    pub entity: Option<usize>,
    /// Colour multiplier (linear) on the texture (Source `$color`/`$color2`).
    pub tint: Option<[f32; 3]>,
    /// A water surface: index into `MapData::water_materials` (drawn by
    /// `water::WaterMaterial` instead).
    pub water: Option<usize>,
    /// A model mesh that belongs to one choice of a body part with
    /// several (part, choice): drawn only while the prop's body picks it
    /// (`MapModel::body_parts`).
    pub body: Option<(u16, u16)>,
}

/// How a model mesh looks under one skin family: the material fields of
/// `MapMesh` that a skin changes.
#[derive(Clone, Debug, Default)]
pub struct MapMeshLook {
    pub material: String,
    pub texture: Option<usize>,
    pub alpha: MapAlpha,
    pub double_sided: bool,
    pub unlit: bool,
    pub envmap: Option<MapEnvmap>,
    pub tint: Option<[f32; 3]>,
}

impl MapMeshLook {
    /// The mesh with this look.
    pub fn apply(&self, m: &MapMesh) -> MapMesh {
        MapMesh {
            material: self.material.clone(),
            texture: self.texture,
            alpha: self.alpha,
            double_sided: self.double_sided,
            unlit: self.unlit,
            envmap: self.envmap.clone(),
            tint: self.tint,
            ..m.clone()
        }
    }
}

/// A model's skeleton and what it can play (animated props).
#[derive(Clone, Debug)]
pub struct MapRig {
    /// Parents before children, in the source game's axes and units.
    pub bones: Vec<MapBone>,
    /// From the skeleton's space to the model's (engine axes, meters).
    pub root: Transform,
    pub animations: Arc<anim::AnimSet>,
}

/// A baked environment cubemap: six square RGBA8 sRGB faces in the
/// source game's face order and axes (Source: +X, -X, +Y, -Y, +Z, -Z in
/// its Z-up frame), sampled with directions in that frame.
#[derive(Clone, Debug)]
pub struct MapCubemap {
    pub name: String,
    pub size: u32,
    pub faces: [Vec<u8>; 6],
}

/// What scales a surface's reflection, per texel.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub enum EnvmapMask {
    #[default]
    None,
    /// The normal map's alpha.
    NormalAlpha,
    /// One minus the base texture's alpha (Source's inverted mask).
    BaseAlphaInverted,
    /// A mask texture's colour (index into `MapData::textures`).
    Texture(usize),
}

/// Reflection parameters (specs/cs_source/shaders.md, "$envmap"), after the
/// shader's fast-path rule has been applied.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MapEnvmap {
    /// Index into `MapData::cubemaps`; None: the cubemap sample nearest
    /// to each object using the material (`MapData::cubemap_samples`).
    pub cubemap: Option<usize>,
    pub mask: EnvmapMask,
    pub tint: [f32; 3],
    pub contrast: f32,
    pub saturation: f32,
    /// Fresnel R0 (1: no fresnel).
    pub fresnel: f32,
}

/// Source `$detail`: a texture tiled `scale` times per base texture repeat
/// and combined with the base color (specs/cs_source/shaders.md).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MapDetail {
    /// Index into `MapData::textures` (linear for mode 0, sRGB otherwise).
    pub texture: usize,
    pub scale: [f32; 2],
    pub factor: f32,
    /// 0: multiply by 2 x detail ("mod2x"); 1: add; 2: blend the detail
    /// over the base by its alpha; 3 and 4: WorldTwoTextureBlend (detail
    /// over base, and the 2x grime mask), which also light the surface
    /// their own way (specs/cs_source/shaders_two_texture_blend.md).
    pub mode: u8,
}

/// The second layer of a two-texture surface. Indices into
/// `MapData::textures`.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct MapBlend {
    pub texture: Option<usize>,
    pub normal_map: Option<usize>,
    /// Shapes the blend: green moves the transition point, red sets its
    /// softness (linear texture).
    pub mask: Option<usize>,
}

/// Baked lighting atlas: linear RGB, where 1.0 shows a texture at its own
/// brightness (above 1.0 is overbright).
#[derive(Clone, Debug, Default)]
pub struct MapLightmap {
    pub width: u32,
    pub height: u32,
    /// Row-major, top row first.
    pub rgb: Vec<[f32; 3]>,
    /// Directional lightmaps for normal-mapped surfaces (same layout), one
    /// per basis direction of radiosity normal mapping. Surfaces without
    /// them repeat `rgb`.
    pub bumped: Option<[Vec<[f32; 3]>; 3]>,
    /// Switchable light styles: what each adds where it lights, so they
    /// can be turned on and off (`LightStyles`). `rgb` and `bumped` hold
    /// the lighting at map start (styles with `on` included).
    pub styles: Vec<MapLightStyle>,
}

/// One switchable light style's share of the lightmap atlas.
#[derive(Clone, Debug, Default)]
pub struct MapLightStyle {
    pub style: u8,
    /// Lit at map start.
    pub on: bool,
    /// Atlas texels (row-major index) it lights, and the light it adds
    /// there (same units as `MapLightmap::rgb`).
    pub texels: Vec<u32>,
    pub rgb: Vec<[f32; 3]>,
    /// What it adds to each directional page, per texel.
    pub bumped: Option<[Vec<[f32; 3]>; 3]>,
}

impl MapLightmap {
    /// The atlas (flat and directional pages) with each style lit as
    /// `lit(style)` says.
    #[allow(clippy::type_complexity)]
    pub fn relit(&self, lit: &dyn Fn(u8) -> bool) -> (Vec<[f32; 3]>, Option<[Vec<[f32; 3]>; 3]>) {
        let mut rgb = self.rgb.clone();
        let mut bumped = self.bumped.clone();
        for s in &self.styles {
            let sign = match (s.on, lit(s.style)) {
                (false, true) => 1.0,
                (true, false) => -1.0,
                _ => continue,
            };
            let add = |dst: &mut [f32; 3], v: [f32; 3]| {
                for k in 0..3 {
                    dst[k] = (dst[k] + sign * v[k]).max(0.0);
                }
            };
            for (i, &t) in s.texels.iter().enumerate() {
                add(&mut rgb[t as usize], s.rgb[i]);
                if let (Some(pages), Some(src)) = (bumped.as_mut(), s.bumped.as_ref()) {
                    for k in 0..3 {
                        add(&mut pages[k][t as usize], src[k][i]);
                    }
                }
            }
        }
        (rgb, bumped)
    }
}

/// A reusable model (e.g. a window frame), in its own space: meters, Y up.
#[derive(Clone, Debug, Default)]
pub struct MapModel {
    pub meshes: Vec<MapMesh>,
    /// Collision box in model space (min, max), for box-solid props.
    pub bounds: (Vec3, Vec3),
    /// The model's own collision model (Source `.phy`), when it has one.
    pub collision: Option<MapCollision>,
    /// Surface property name (footsteps on the prop), lower-case.
    pub surfaceprop: Option<String>,
    /// Where the model is lit from, in model space (Source
    /// `$illumposition`), when the game gives one.
    pub illum: Option<Vec3>,
    /// Each skin family's look per mesh (same order as `meshes`), when
    /// the model has more than one; the meshes themselves show the skin
    /// they were loaded with.
    pub skins: Vec<Vec<MapMeshLook>>,
    /// Choices per body part (Source body groups); a prop's body number
    /// picks one per part (`body_choice`).
    pub body_parts: Vec<u16>,
    /// Skeleton and animations, for props that play sequences (meshes
    /// skinned to it).
    pub rig: Option<Arc<MapRig>>,
    /// What it breaks into, when it can break.
    pub breaks: Option<breakables::MapBreak>,
}

impl MapModel {
    /// Which choice of body part `part` a body number shows.
    pub fn body_choice(&self, body: i32, part: usize) -> u16 {
        body_choice(&self.body_parts, body, part)
    }
}

/// Which choice of body part `part` a body number shows, given each
/// part's choice count (Source: the body number in the mixed radix of
/// the parts' counts, the first part lowest).
pub fn body_choice(parts: &[u16], body: i32, part: usize) -> u16 {
    let base: i32 = parts[..part.min(parts.len())]
        .iter()
        .map(|n| (*n).max(1) as i32)
        .product();
    let n = parts.get(part).copied().unwrap_or(1).max(1) as i32;
    ((body.max(0) / base.max(1)) % n) as u16
}


/// A character body from the game: meshes in the character's local space
/// (feet at the origin, facing -Z) and its hitboxes in the same space.
#[derive(Clone, Debug)]
pub struct MapCharacterModel {
    /// The team it's for; None: anyone without a better match.
    pub team: Option<Team>,
    /// Set for a model only characters that name it use (`BodyName`, e.g.
    /// hostages); such models are never picked by team.
    pub name: Option<String>,
    pub model: MapModel,
    pub hitboxes: Vec<crate::core::Hitbox>,
    /// The skeleton the meshes are skinned to, parents before children.
    pub bones: Vec<MapBone>,
    /// From the skeleton's space (the source game's axes and units) to the
    /// character's local space.
    pub root: Transform,
    /// What the skeleton can play (None: it stays in the reference pose).
    pub animations: Option<Arc<anim::AnimSet>>,
    /// The hitboxes on their bones (same order as `hitboxes`), to follow
    /// the animated skeleton.
    pub boxes: Vec<BoneBox>,
    /// The body's physics when it dies (None: no ragdoll; the dead body
    /// is just hidden).
    pub ragdoll: Option<MapRagdoll>,
}

/// A hitbox in its bone's frame (the source game's axes and units).
#[derive(Clone, Debug)]
pub struct BoneBox {
    pub bone: usize,
    pub center: Vec3,
    pub half: Vec3,
    pub group: crate::core::Hitgroup,
}

/// A model characters hold (a weapon's world model), keyed by what
/// `Held` names; meshes in the frame of `bone` of the character skeleton
/// (its axes and units).
#[derive(Clone, Debug)]
pub struct MapHeldModel {
    pub key: String,
    pub model: MapModel,
    pub bone: String,
    /// Where it shoots from, in the bone's frame (muzzle flashes; +X
    /// forward).
    pub muzzle: Option<Transform>,
}

/// What a character holds (a `MapHeldModel` key), drawn in its body's
/// hand. Games set it.
#[derive(Component, Clone, Default, Debug, PartialEq)]
pub struct Held(pub Option<String>);

/// Which character model a character uses (index in `CharacterModels`).
#[derive(Component, Clone, Copy, PartialEq, Eq, Debug)]
pub struct BodyModel(pub usize);
/// One bone of a character skeleton, in its reference pose relative to
/// its parent (the source game's axes and units).
#[derive(Clone, Debug)]
pub struct MapBone {
    pub name: String,
    pub parent: Option<usize>,
    pub position: Vec3,
    pub rotation: Quat,
}

/// The loaded map's character models (see `MapCharacterModel`).
#[derive(Resource, Clone)]
pub struct CharacterModels(pub Arc<Vec<MapCharacterModel>>);

impl CharacterModels {
    /// The model for `team`: its own, else one without a team, else any.
    pub fn for_team(&self, team: Option<Team>) -> Option<(usize, &MapCharacterModel)> {
        let all = self.0.iter().enumerate().filter(|(_, m)| m.name.is_none());
        all.clone()
            .find(|(_, m)| team.is_some() && m.team == team)
            .or_else(|| all.clone().find(|(_, m)| m.team.is_none()))
            .or_else(|| all.clone().next())
    }

    /// The model a character uses: the one its `BodyName` names, else its
    /// team's.
    pub fn for_character(&self, team: Option<Team>, name: Option<&BodyName>) -> Option<(usize, &MapCharacterModel)> {
        match name {
            Some(n) => self
                .0
                .iter()
                .enumerate()
                .find(|(_, m)| m.name.as_deref().is_some_and(|x| x.eq_ignore_ascii_case(&n.0))),
            None => self.for_team(team),
        }
    }
}

/// A character drawn with a named model (`MapCharacterModel::name`)
/// instead of its team's: a hostage.
#[derive(Component, Clone, Debug, PartialEq)]
pub struct BodyName(pub String);

/// Meshes and materials of each character model (index as in
/// `CharacterModels`).
#[derive(Resource)]
struct CharacterBodies(Vec<BodyAssets>);

/// Meshes and materials of each held model, by key.
#[derive(Resource)]
#[allow(clippy::type_complexity)]
struct HeldAssets(HashMap<String, (String, Vec<(Handle<Mesh>, Handle<PropMaterial>)>, Option<Transform>)>);

struct BodyAssets {
    parts: Vec<(Handle<Mesh>, Handle<PropMaterial>)>,
    bindposes: Handle<bevy::mesh::skinning::SkinnedMeshInverseBindposes>,
    bones: Vec<MapBone>,
    root: Transform,
}

/// A joint of a character body's skeleton: its bone index.
#[derive(Component)]
pub struct BodyJoint(pub usize);

/// A character's drawn body: which model, at its feet.
#[derive(Component)]
pub struct CharacterBody {
    model: usize,
    /// Joint entities by bone index.
    joints: Vec<Entity>,
    /// The held model shown and its key.
    held: Option<(Entity, String)>,
}

/// Whether the local player's own body is drawn (third person). The
/// client sets it; the body is there either way, animated, and hidden
/// when this is false.
#[derive(Resource, Default, Clone, Copy, Debug, PartialEq, Eq)]
pub struct ShowLocalBody(pub bool);

/// Give characters their team's body (a child at the feet); the local
/// player's is hidden unless `ShowLocalBody`.
#[allow(clippy::type_complexity)]
fn attach_bodies(
    models: Option<Res<CharacterModels>>,
    bodies: Option<Res<CharacterBodies>>,
    characters: Query<
        (
            Entity,
            (Option<&Team>, Option<&BodyName>),
            &ColliderAabb,
            &GlobalTransform,
            Option<&Children>,
            Has<crate::core::LocalPlayer>,
        ),
        (With<crate::core::Intent>, Without<ragdoll::Ragdolled>),
    >,
    existing: Query<&CharacterBody>,
    show_local: Res<ShowLocalBody>,
    mut materials: Option<ResMut<Assets<PropMaterial>>>,
    mut commands: Commands,
) {
    let (Some(models), Some(bodies)) = (models, bodies) else {
        return;
    };
    for (e, (team, name), aabb, at, children, local) in &characters {
        let Some((index, _)) = models.for_character(team.copied(), name) else {
            continue;
        };
        let current = children
            .into_iter()
            .flatten()
            .find_map(|c| existing.get(*c).ok().map(|b| (*c, b.model)));
        match current {
            Some((_, m)) if m == index => continue,
            Some((old, _)) => commands.entity(old).despawn(),
            None => {}
        }
        let feet = aabb.min.y - at.translation().y;
        let assets = &bodies.0[index];
        let body = commands
            .spawn((
                Name::new("Body"),
                Transform::from_xyz(0.0, feet, 0.0),
                body_visibility(local, *show_local),
                ChildOf(e),
            ))
            .id();
        // The skeleton: a root in the source game's axes and units, then
        // each bone under its parent, in the reference pose.
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
        commands.entity(body).insert(CharacterBody {
            model: index,
            joints: joints.clone(),
            held: None,
        });
        // Its own materials, lit where it stands (probe_lit), at its centre.
        let mut own = Vec::new();
        for (mesh, material) in &assets.parts {
            let material = match materials.as_mut() {
                Some(m) => probe_lit::instance(m, material),
                None => material.clone(),
            };
            own.push(material.clone());
            commands.spawn((
                Mesh3d(mesh.clone()),
                MeshMaterial3d(material),
                bevy::mesh::skinning::SkinnedMesh {
                    inverse_bindposes: assets.bindposes.clone(),
                    joints: joints.clone(),
                },
                ChildOf(body),
            ));
        }
        commands
            .entity(body)
            .insert(probe_lit::ProbeLit::new(own, Vec3::Y * BODY_LIGHT_HEIGHT));
    }
}

/// Height of a body's lighting point above its feet, meters (about the
/// models' own illumination position, their middle).
const BODY_LIGHT_HEIGHT: f32 = 0.9;

fn body_visibility(local: bool, show_local: ShowLocalBody) -> Visibility {
    if local && !show_local.0 {
        Visibility::Hidden
    } else {
        Visibility::Inherited
    }
}

/// Show or hide the local player's body when `ShowLocalBody` changes.
fn show_local_body(
    show: Res<ShowLocalBody>,
    local: Query<&Children, With<crate::core::LocalPlayer>>,
    mut bodies: Query<&mut Visibility, With<CharacterBody>>,
) {
    let want = body_visibility(true, *show);
    for children in &local {
        let mut it = bodies.iter_many_mut(children);
        while let Some(mut vis) = it.fetch_next() {
            if *vis != want {
                *vis = want;
            }
        }
    }
}

/// Turn bodies with their character's yaw.
fn turn_bodies(
    characters: Query<(&crate::core::Intent, &Children, Option<&anim::Animator>)>,
    mut bodies: Query<&mut Transform, With<CharacterBody>>,
) {
    for (intent, children, animator) in &characters {
        let yaw = animator.and_then(|a| a.yaw).unwrap_or(intent.yaw);
        for c in children {
            if let Ok(mut t) = bodies.get_mut(*c) {
                t.rotation = Quat::from_rotation_y(yaw);
            }
        }
    }
}

/// Draw what each character holds in its body's hand.
fn attach_held(
    held: Option<Res<HeldAssets>>,
    bodies: Option<Res<CharacterBodies>>,
    characters: Query<(Entity, &Held, &Children)>,
    mut body_query: Query<&mut CharacterBody>,
    mut materials: Option<ResMut<Assets<PropMaterial>>>,
    mut commands: Commands,
) {
    let (Some(held), Some(bodies)) = (held, bodies) else {
        return;
    };
    for (character, want, children) in &characters {
        let Some(c) = children.iter().find(|c| body_query.contains(*c)) else {
            continue;
        };
        let mut body = body_query.get_mut(c).unwrap();
        if body.held.as_ref().map(|(_, k)| k.as_str()) == want.0.as_deref() {
            continue;
        }
        if let Some((e, _)) = body.held.take() {
            commands.entity(e).despawn();
        }
        let Some(key) = &want.0 else { continue };
        let Some((bone, parts, muzzle)) = held.0.get(key) else {
            continue;
        };
        let Some(joint) = bodies.0[body.model]
            .bones
            .iter()
            .position(|b| b.name.eq_ignore_ascii_case(bone))
            .and_then(|i| body.joints.get(i).copied())
        else {
            continue;
        };
        // Its own materials, lit where it is (probe_lit).
        let own: Vec<Handle<PropMaterial>> = parts
            .iter()
            .map(|(_, m)| match materials.as_mut() {
                Some(assets) => probe_lit::instance(assets, m),
                None => m.clone(),
            })
            .collect();
        let model = commands
            .spawn((
                Transform::default(),
                Visibility::Inherited,
                ChildOf(joint),
                probe_lit::ProbeLit::new(own.clone(), Vec3::ZERO),
            ))
            .with_children(|m| {
                for ((mesh, _), material) in parts.iter().zip(own) {
                    m.spawn((Mesh3d(mesh.clone()), MeshMaterial3d(material)));
                }
                if let Some(t) = muzzle {
                    m.spawn((*t, view_model::HeldMuzzle { owner: character }));
                }
            })
            .id();
        body.held = Some((model, key.clone()));
    }
}

/// Game systems that decide what bodies play run in this set, before
/// the joints are posed.
#[derive(SystemSet, Debug, Clone, PartialEq, Eq, Hash)]
pub struct DriveAnimation;

/// Pose each animated body's joints from its character's `Animator`.
fn pose_bodies(
    time: Res<Time>,
    characters: Query<(&anim::Animator, &Children)>,
    bodies: Query<&CharacterBody>,
    mut joints: Query<&mut Transform, With<BodyJoint>>,
) {
    let now = time.elapsed_secs_f64();
    for (animator, children) in &characters {
        let Some(body) = children.iter().find_map(|c| bodies.get(c).ok()) else {
            continue;
        };
        if animator.main.is_none() {
            continue;
        }
        let pose = animator.pose(now);
        for (joint, (q, p)) in body.joints.iter().zip(pose) {
            if let Ok(mut t) = joints.get_mut(*joint) {
                t.rotation = q;
                t.translation = p;
            }
        }
    }
}

/// Characters get their team's hitboxes from the loaded character models.
/// Characters also get the model's animations (an `Animator`, driven by
/// the game).
fn attach_hitboxes(
    models: Option<Res<CharacterModels>>,
    characters: Query<(Entity, Option<&Team>, Option<&BodyName>, Option<&BodyModel>), With<crate::core::Intent>>,
    mut commands: Commands,
) {
    let Some(models) = models else { return };
    for (e, team, name, current) in &characters {
        let Some((index, m)) = models.for_character(team.copied(), name) else {
            continue;
        };
        if current == Some(&BodyModel(index)) {
            continue;
        }
        let mut c = commands.entity(e);
        c.insert((BodyModel(index), crate::core::Hitboxes(m.hitboxes.clone())));
        match &m.animations {
            Some(set) => c.insert(anim::Animator::new(set.clone())),
            None => c.remove::<anim::Animator>(),
        };
    }
}

/// Hitboxes follow the animated skeleton (as the server places them).
fn pose_hitboxes(
    models: Option<Res<CharacterModels>>,
    mut characters: Query<(
        &anim::Animator,
        &BodyModel,
        &crate::core::Intent,
        &ragdoll::SkeletonPose,
        &mut crate::core::Hitboxes,
    )>,
) {
    let Some(models) = models else { return };
    for (animator, model, intent, pose, mut hitboxes) in &mut characters {
        let Some(m) = models.0.get(model.0) else { continue };
        if animator.main.is_none() || m.boxes.len() != hitboxes.0.len() {
            continue;
        }
        // Bones in the skeleton's space (this tick's pose), then to the
        // character's frame (which the trace turns by the look yaw; the
        // body turns by its own yaw).
        let Some(frame) = pose.frames.back() else { continue };
        let global = &frame.bones;
        let turn = Quat::from_rotation_y(animator.yaw.unwrap_or(intent.yaw) - intent.yaw) * m.root.rotation;
        let scale = m.root.scale.x;
        for (h, b) in hitboxes.0.iter_mut().zip(&m.boxes) {
            let Some((q, p)) = global.get(b.bone) else { continue };
            h.center = turn * (*p + *q * b.center) * scale;
            h.half = b.half * scale;
            h.rotation = turn * *q;
        }
    }
}

/// A model's collision: convex pieces in model space, and the physics
/// parameters that came with them.
#[derive(Clone, Debug, Default)]
pub struct MapCollision {
    pub pieces: Vec<MapConvex>,
    /// kg.
    pub mass: f32,
    pub damping: f32,
    pub rotdamping: f32,
    /// Multiplier on the inertia tensor.
    pub inertia: f32,
    /// Surface property name (friction, elasticity, sounds).
    pub surfaceprop: String,
}

/// A convex piece: its corners and outward planes (n . p <= d inside).
#[derive(Clone, Debug, Default)]
pub struct MapConvex {
    pub points: Vec<Vec3>,
    pub planes: Vec<(Vec3, f32)>,
}

/// How players collide with a prop.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum PropSolid {
    None,
    /// The model's bounds as a box, rotated with the prop.
    Box,
    /// The model's triangles.
    Mesh,
}

/// Lighting for something without a lightmap, sampled at one point: light
/// arriving from six axis directions (+X, -X, +Y, -Y, +Z, -Z) plus
/// directional lights (direction toward the light, color), all in lightmap
/// units. The same model Source uses for props and characters.
#[derive(Clone, Debug, Default)]
pub struct LightProbe {
    pub cube: [Vec3; 6],
    pub lights: Vec<(Vec3, Vec3)>,
}

impl LightProbe {
    /// Light on a surface with normal `n`.
    pub fn eval(&self, n: Vec3) -> Vec3 {
        let c = &self.cube;
        let pick = |v: f32, pos: Vec3, neg: Vec3| if v >= 0.0 { pos } else { neg };
        let ambient =
            pick(n.x, c[0], c[1]) * n.x * n.x + pick(n.y, c[2], c[3]) * n.y * n.y + pick(n.z, c[4], c[5]) * n.z * n.z;
        self.lights
            .iter()
            .fold(ambient, |acc, (dir, color)| acc + *color * n.dot(*dir).max(0.0))
    }
}

/// A placed model.
#[derive(Clone, Debug)]
pub struct MapProp {
    /// Index into `MapData::models`.
    pub model: usize,
    pub translation: Vec3,
    pub rotation: Quat,
    pub solid: PropSolid,
    /// Part of the 3D skybox.
    pub skybox: bool,
    /// Baked lighting; lit by the scene's lights when absent.
    pub lighting: Option<LightProbe>,
    /// Casts a dynamic shadow onto the world (Source: entity props, not
    /// static props, whose shadows are baked into lightmaps).
    pub casts_shadow: bool,
    /// Simulated as a rigid body, when set.
    pub physics: Option<MapPhysics>,
    /// Fade distances from the camera (start, gone), meters, when the prop
    /// fades out with distance: dithered out between them (`vis::fade_band`),
    /// hidden beyond the far one.
    pub fade: Option<(f32, f32)>,
    /// The mover entity it's attached to (index into `MapData::entities`):
    /// it rides that entity's node (de_nuke's door handles) and isn't solid.
    pub parent: Option<usize>,
    /// The entity it was placed by (index into `MapData::entities`); its
    /// node then carries `PropEntity`.
    pub entity: Option<usize>,
    /// The skin family its meshes show (`MapModel::skins`).
    pub skin: i32,
    /// Its body number (`MapModel::body_choice`).
    pub body: i32,
}

/// A physics prop's body (specs/cs_source/physics_props.md 3, 4).
#[derive(Clone, Debug)]
pub struct MapPhysics {
    /// kg.
    pub mass: f32,
    pub friction: f32,
    pub elasticity: f32,
    pub damping: f32,
    pub rotdamping: f32,
    pub push: PushAway,
    /// Held in place (Source "motion disabled") until something enables
    /// its motion: it stays put like a static prop until then.
    pub frozen: bool,
    /// Starts asleep (Source spawnflag 1): it doesn't move until woken
    /// (`prop_physics::PropAwakened` then tells).
    pub asleep: bool,
}

/// How a physics prop and players interact.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PushAway {
    /// Collides with players like a wall (pushed by their physics shadow).
    Collide,
    /// Players walk through it; it's shoved away from them, and pushes
    /// them back (multiplayer "solid" mode).
    Solid,
    /// Players walk through it; it's shoved away from them.
    NonSolid,
    /// Players walk through it and don't push it (multiplayer client-side
    /// props, with the default sv_pushaway_clientside 0).
    Ignore,
}

/// Dynamic prop shadows (Source `shadow_control`, specs/cs_source/shadows_sky.md):
/// one direction, colour and cast distance for the whole map.
#[derive(Clone, Debug)]
pub struct MapShadows {
    /// Unit vector from caster toward receiver, engine space.
    pub direction: Vec3,
    /// sRGB bytes.
    pub color: [u8; 3],
    /// How far past the caster a shadow reaches, meters.
    pub distance: f32,
}

/// Sound entries the announcer plays for rounds: a side's win, a draw,
/// and one picked at random when a round goes live.
#[derive(Resource, Clone, Debug, Default, PartialEq)]
pub struct RoundSounds {
    pub attackers_win: Option<String>,
    pub defenders_win: Option<String>,
    pub draw: Option<String>,
    pub start: Vec<String>,
}

/// Indices into `core::MapBrushes` of brushes that stop players but not
/// line traces such as a fire's (player clips, grates, ladder-only
/// volumes; `MapData::trace_skip`).
#[derive(Resource, Clone, Debug, Default)]
pub struct MapTraceSkip(pub Vec<usize>);

#[derive(Clone, Debug, Default)]
pub struct MapData {
    pub name: String,
    pub meshes: Vec<MapMesh>,
    pub textures: Vec<MapTexture>,
    /// Baked reflection cubemaps (`MapEnvmap::cubemap` indexes these).
    pub cubemaps: Vec<MapCubemap>,
    /// Where the map's cubemaps were baked: (position, index into
    /// `cubemaps`), for objects that take the nearest.
    pub cubemap_samples: Vec<(Vec3, usize)>,
    pub lightmap: Option<MapLightmap>,
    pub models: Vec<MapModel>,
    pub props: Vec<MapProp>,
    /// Collision triangles (surfaces with no solid volume, e.g. terrain).
    pub collision_positions: Vec<[f32; 3]>,
    pub collision_indices: Vec<[u32; 3]>,
    /// Solid convex volumes, each as its corner points: what stops shots
    /// and physics bodies (player-only clips are in `collision_brushes`
    /// only).
    pub collision_hulls: Vec<Vec<[f32; 3]>>,
    /// The same volumes as planes, for exact swept-box movement collision.
    pub collision_brushes: Vec<MapBrush>,
    /// Indices into `collision_brushes` of the ones line traces pass
    /// (player clips, grates, ladder-only volumes): `MapTraceSkip`.
    pub trace_skip: Vec<usize>,
    /// The BSP tree over `collision_brushes` (Source maps), for the order
    /// traces meet coincident faces in.
    pub brush_tree: Option<MapBrushTree>,
    /// Feet positions.
    pub spawns: Vec<(Vec3, Option<Team>)>,
    /// Each spawn's facing as an `Intent` yaw (radians, 0 = -Z), same order
    /// as `spawns`; missing entries face -Z.
    pub spawn_yaws: Vec<f32>,
    /// Things the importer couldn't load (missing materials etc.).
    pub warnings: Vec<String>,
    /// How the source game presents the map, so it can be reproduced.
    pub look: MapLook,
    pub sky: Option<MapSky>,
    pub sky_camera: Option<MapSkyCamera>,
    /// The world's fog (not the 3D skybox's, which `sky_camera` has).
    pub fog: Option<MapFog>,
    pub sprites: Vec<MapSprite>,
    pub dust: Vec<MapDust>,
    pub ropes: Vec<MapRope>,
    /// Which sky each part of the map can see (Source: BSP leaf flags). When
    /// set, the camera draws the sky only from places that see it, and
    /// clears to black inside solid.
    pub sky_vis: Option<MapSkyVis>,
    /// Precomputed visibility (clusters and potentially visible sets),
    /// when the game's maps have it: parts the camera can't see are hidden.
    pub visibility: Option<Arc<vis::MapVisibility>>,
    /// Water and slime volumes.
    pub water: Vec<MapWaterVolume>,
    /// The map's entities (keyvalues, brush volumes) for the logic layer,
    /// in the map's own order.
    pub entities: Vec<MapEntity>,
    /// Meters per entity-space unit (`entities` module docs); 0 when the
    /// map has no entities.
    pub entity_scale: f32,
    /// How water surfaces look (`MapMesh::water` indexes these).
    pub water_materials: Vec<water::MapWaterMaterial>,
    /// Dynamic prop shadows, when the game draws them.
    pub shadows: Option<MapShadows>,
    /// The announcer's round sounds (sound entries), when the game has them.
    pub round_sounds: RoundSounds,
    /// The team radio (commands, menus, chat format), when the game has one.
    pub radio: Option<radio::RadioCommands>,
    /// Gravity for physics bodies, m/s^2 (downward), when the game sets it.
    pub gravity: Option<f32>,
    /// The playable area (engine space, min and max), when the map has a 3D
    /// skybox outside it.
    pub playable: Option<(Vec3, Vec3)>,
    /// Sound entries, clips and surfaces the map uses.
    pub sounds: Arc<MapSounds>,
    /// The bots' navigation mesh, if the game ships one for the map.
    pub nav: Option<Arc<nav::NavMesh>>,
    /// How characters look and where they can be hit, per team.
    pub characters: Vec<MapCharacterModel>,
    /// Models characters can hold (weapons' world models).
    pub held: Vec<MapHeldModel>,
    /// Runtime decals (bullet holes, slashes) by group.
    pub decals: decal::MapDecals,
    /// The game's own HUD look, when it has one.
    pub hud: Option<Arc<hud::GameHud>>,
    /// A top-down picture of the map (radar), when the game has one.
    pub overview: Option<hud::MapOverview>,
    /// Materials for particle effects (impacts).
    pub particles: particles::MapParticles,
    /// What characters see of what they hold (weapons' view models).
    pub view_models: Vec<MapViewModel>,
    /// The light at any point, for moving models.
    pub light_field: Option<MapLightField>,
    /// What shots look like at the muzzle (flash sprites and light).
    pub muzzle_flash: Option<MapMuzzleFlash>,
    /// Spent shell types and how they fly.
    pub shells: Vec<shells::MapShell>,
    pub shell_physics: Option<shells::MapShellPhysics>,
    /// Gib lists for breaking brushes, and how gibs move.
    pub gibs: Vec<breakables::MapGibSet>,
    pub gib_physics: Option<breakables::MapGibPhysics>,
}

/// A muzzle flash: view-facing additive sprites strung out along the
/// muzzle's forward axis, shown for `life` seconds, and a brief point
/// light (`DynamicLight`) that lights the world, props and view models.
#[derive(Clone, Debug, Default)]
pub struct MapMuzzleFlash {
    /// Index into `MapData::textures`.
    pub texture: Option<usize>,
    /// Each sprite: distance ahead of the muzzle and full size (meters),
    /// before the random scale.
    pub sprites: Vec<(f32, f32)>,
    /// Random size factor range.
    pub scale: (f32, f32),
    /// sRGB colour multiplier of the sprites.
    pub color: [f32; 3],
    pub life: f32,
    pub light: Option<MapFlashLight>,
}

/// A muzzle flash's light: colour in lightmap units (1 = a fully lit
/// surface shows its texture), radius range (meters, picked at random) and
/// lifetime; its radius shrinks to 0 over the lifetime.
#[derive(Clone, Copy, Debug)]
pub struct MapFlashLight {
    pub color: Vec3,
    pub radius: (f32, f32),
    pub life: f32,
}

/// What the BSP leaf around a point can see of the sky.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LeafSky {
    /// Inside solid: the view clears to black.
    Solid,
    /// No sky visible: nothing is drawn behind the world.
    None,
    /// The 2D sky only.
    Sky2d,
    /// The 3D skybox (with the 2D sky behind it).
    Sky3d,
}

/// A BSP tree for point queries, in engine space (meters).
#[derive(Clone, Debug, Default)]
pub struct MapSkyVis {
    /// Plane normal and distance.
    pub planes: Vec<(Vec3, f32)>,
    /// Plane index and children (front, back); a negative child `c` is
    /// leaf `-c - 1`.
    pub nodes: Vec<(usize, [i32; 2])>,
    pub leaves: Vec<LeafSky>,
}

impl MapSkyVis {
    pub fn at(&self, p: Vec3) -> LeafSky {
        self.leaf(p)
            .and_then(|l| self.leaves.get(l).copied())
            .unwrap_or(LeafSky::None)
    }

    /// The index of the leaf containing `p`.
    pub fn leaf(&self, p: Vec3) -> Option<usize> {
        let mut node = 0i32;
        while node >= 0 {
            let &(plane, children) = self.nodes.get(node as usize)?;
            let (n, d) = self.planes[plane];
            node = if n.dot(p) - d >= 0.0 { children[0] } else { children[1] };
        }
        Some((-node - 1) as usize)
    }
}

/// The light arriving at any point (engine space), for models that move
/// (view models): the same probe static props bake, queried at run time.
#[derive(Clone)]
pub struct MapLightField(pub Arc<dyn Fn(Vec3) -> LightProbe + Send + Sync>);

impl std::fmt::Debug for MapLightField {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("MapLightField")
    }
}

/// The loaded map's light field, when the game provides one.
#[derive(Resource, Clone)]
pub struct LightField(pub MapLightField);

/// A dust mote volume (`func_dustmotes`): slow specks spawned inside a box,
/// fading in and out over their life and with distance
/// (specs/cs_source/sprites_dust.md). Engine space, meters, seconds.
#[derive(Clone, Debug)]
pub struct MapDust {
    pub min: Vec3,
    pub max: Vec3,
    /// Motes per second.
    pub rate: f32,
    /// Screen-constant size: half-width = size / 10000 x view depth.
    pub size: (f32, f32),
    /// Initial speed per axis, up to this (m/s).
    pub speed: f32,
    pub life: (f32, f32),
    /// Not drawn beyond this view depth; fade toward it (meters).
    pub fade_distance: f32,
    /// sRGB color and alpha (0-1).
    pub color: [f32; 4],
    /// Index into `MapData::textures`.
    pub texture: Option<usize>,
    /// The entity it comes from (index into `MapData::entities`), which
    /// the logic turns on and off (`EntityPart`).
    pub entity: Option<usize>,
    /// Spawning motes at map start.
    pub start_on: bool,
}

/// A camera-facing sprite (lamp glows): a quad parallel to the view plane,
/// drawn additively (specs/cs_source/sprites_dust.md).
#[derive(Clone, Debug)]
pub struct MapSprite {
    /// Engine space, meters.
    pub position: Vec3,
    /// Index into `MapData::textures`.
    pub texture: usize,
    /// Full width and height, meters.
    pub size: Vec2,
    /// Color x texture is added to the image: RGB scale and alpha weight.
    pub color: Vec4,
    /// A glow: drawn over everything, faded by how much of its occlusion
    /// proxy can be seen. Otherwise depth tested against the world.
    pub glow: bool,
    /// Glow occlusion proxy: half-diagonal of a square pulled this far
    /// toward the viewer, meters.
    pub proxy: f32,
    /// The entity it comes from (index into `MapData::entities`), which
    /// the logic shows and hides (`EntityPart`).
    pub entity: Option<usize>,
    /// Shown at map start.
    pub start_on: bool,
}

/// A drawn part of a map entity the logic switches (sprites, dust
/// volumes): `on` shows a sprite or lets dust spawn motes; `exists` false
/// (killed) removes it from view at once. The logic layer writes it.
#[derive(Component, Clone, Copy, Debug, PartialEq, Eq)]
pub struct EntityPart {
    /// Index into `MapData::entities`.
    pub entity: usize,
    pub on: bool,
    pub exists: bool,
}

/// How a prop entity looks, as the logic says (skin family, and the body
/// number SetBodyGroup set; `MapModel::skins`, `MapModel::body_choice`).
/// The logic layer writes it; the map applies it.
#[derive(Component, Clone, Debug, PartialEq, Eq)]
pub struct PropLook {
    pub skin: i32,
    pub body_group: Option<i32>,
}

/// What an animated prop plays, as the logic says: a sequence by name
/// with a serial (a new serial restarts it), the one played at spawn and
/// after a sequence ends, and the round (a new round starts over).
#[derive(Component, Clone, Debug, Default, PartialEq, Eq)]
pub struct PropSequence {
    pub play: Option<(String, u32)>,
    pub default: String,
    pub round: u32,
}

/// Switchable light styles that are off (Source styles 32+). The logic
/// layer writes it; the map relights the lightmaps when it changes.
#[derive(Resource, Clone, Debug, Default, PartialEq, Eq)]
pub struct LightStyles {
    /// Style numbers and whether each is lit; unlisted styles keep their
    /// state at map start.
    pub styles: Vec<(u8, bool)>,
}

/// Which soundscape volumes (by entity index, `SoundscapeZone::entity`)
/// the listener touches, as the logic layer's touch code says; None: the
/// map tests the zones' boxes itself.
#[derive(Resource, Clone, Debug, Default, PartialEq, Eq)]
pub struct SoundscapeTouches(pub Option<Vec<usize>>);

/// A rope or cable: a line of points drawn as a strip that always faces the
/// camera (the rope shader widens it per view). Source's Cable look:
/// texture x normal map's blue squared x per-point light.
#[derive(Clone, Debug, Default)]
pub struct MapRope {
    /// Engine space, meters.
    pub points: Vec<Vec3>,
    /// Texture V per point; U runs across the strip.
    pub v: Vec<f32>,
    /// Light per point, lightmap units (0-1).
    pub light: Vec<Vec3>,
    /// Full strip width, meters.
    pub width: f32,
    pub texture: Option<usize>,
    pub normal_map: Option<usize>,
    /// Texture and normal map of a translucent strip drawn behind the rope
    /// that keeps thin, distant ropes visible (fake anti-aliasing).
    pub back: Option<(Option<usize>, Option<usize>)>,
}

/// A 3D skybox: a miniature scene (meshes and props marked `skybox`) drawn
/// behind the world from `origin + eye / scale`, so it appears `scale` times
/// larger and moves with the player.
#[derive(Clone, Debug)]
pub struct MapSkyCamera {
    /// Engine space, meters.
    pub origin: Vec3,
    pub scale: f32,
    pub fog: Option<MapFog>,
}

/// Distance fog, distances in meters as seen in the world. Source's range
/// fog: f = clamp(min(max_density, (depth - start) / (end - start))), and
/// the color moves toward the fog color by f^2 (specs/cs_source/shaders.md).
#[derive(Clone, Debug)]
pub struct MapFog {
    /// sRGB color.
    pub color: [f32; 3],
    pub start: f32,
    pub end: f32,
    pub max_density: f32,
}

/// A sky cubemap: per engine face (+X, -X, +Y, -Y, +Z, -Z), an index into
/// `MapData::textures` and an orientation (0-3 clockwise quarter turns, 4-7
/// the same after mirroring horizontally).
#[derive(Clone, Debug)]
pub struct MapSky {
    pub faces: [(usize, u8); 6],
}

/// Presentation choices that differ between games, measured against the
/// original (see `refcmp`).
#[derive(Clone, Debug)]
pub struct MapLook {
    /// Multiplier on all baked light (lightmaps and prop probes).
    pub light_scale: f32,
    /// Blend between mip levels (trilinear) or snap to the nearest one.
    pub trilinear: bool,
    /// Anisotropic filtering level; 1 = off.
    pub anisotropy: u16,
    /// Apply a filmic tonemapper (off: plain clamped output, like LDR games).
    pub tonemapping: bool,
    /// Store lightmaps the way Source's LDR path does (8-bit, gamma, 2x
    /// overbright), with its banding, clamp and gamma-space filtering.
    pub source_ldr_lightmaps: bool,
    /// Sample lightmaps with a bicubic B-spline filter (4 bilinear taps)
    /// instead of bilinear.
    pub bicubic_lightmaps: bool,
}

impl Default for MapLook {
    fn default() -> Self {
        Self {
            light_scale: 1.0,
            trilinear: true,
            anisotropy: 8,
            tonemapping: true,
            source_ldr_lightmaps: false,
            bicubic_lightmaps: false,
        }
    }
}

impl MapData {
    pub fn triangle_count(&self) -> usize {
        self.meshes.iter().map(|m| m.indices.len() / 3).sum()
    }

    pub fn bounds(&self) -> (Vec3, Vec3) {
        let mut lo = Vec3::splat(f32::MAX);
        let mut hi = Vec3::splat(f32::MIN);
        for p in self.meshes.iter().flat_map(|m| &m.positions) {
            lo = lo.min(Vec3::from(*p));
            hi = hi.max(Vec3::from(*p));
        }
        (lo, hi)
    }
}

/// Spawns a loaded map: collision always, meshes only when rendering is
/// present (same pattern as the greybox, so maps load in headless tests).
pub struct MapPlugin {
    /// The map to spawn at startup; None starts without one (another map
    /// can be loaded later with `change_map`).
    pub data: Option<Arc<MapData>>,
    pub view: MapDebugView,
}

/// Debug renders, used to compare against reference screenshots.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub enum MapDebugView {
    #[default]
    Normal,
    /// White surfaces: only baked lighting (lightmaps, prop probes),
    /// multiplied by `scale` (e.g. 0.25 so overbright light fits in 0..1).
    Lighting { scale: f32 },
    /// Textures only, unlit.
    Albedo,
}

impl MapPlugin {
    pub fn new(data: MapData) -> Self {
        Self {
            data: Some(Arc::new(data)),
            view: MapDebugView::Normal,
        }
    }

    /// The map systems and materials without a map loaded yet.
    pub fn empty() -> Self {
        Self {
            data: None,
            view: MapDebugView::Normal,
        }
    }
}

#[derive(Resource)]
struct PendingMap(Arc<MapData>, MapDebugView);

/// Render layer for 3D skybox content.
pub const SKYBOX_LAYER: usize = 1;

/// Draws the 3D skybox; follows the main camera.
#[derive(Component)]
pub struct SkyboxCamera;

#[derive(Resource, Clone)]
struct SkyCameraInfo(MapSkyCamera);

/// The map's sky visibility, when it has one (see `MapSkyVis`).
#[derive(Resource)]
struct SkyVis(MapSkyVis);

/// Characters below this height (meters) have fallen out of the map.
#[derive(Resource, Clone, Copy, Debug)]
pub struct KillHeight(pub f32);

/// How thick terrain surfaces are as solids (`MapTerrain`), meters: 2 units.
const TERRAIN_THICKNESS: f32 = 2.0 * 0.0254;

/// How far below the map's lowest point (meters) falling ends.
const KILL_MARGIN: f32 = 3.0;

/// Falling out of the map kills; characters that can't die (god mode, no
/// health) go back to a spawn point instead.
#[allow(clippy::type_complexity)]
fn fall_out_of_map(
    kill: Option<Res<KillHeight>>,
    mut characters: Query<
        (
            Entity,
            &mut Transform,
            Option<&mut crate::core::Velocity>,
            Option<&crate::core::Health>,
            Has<crate::core::God>,
            Option<&Team>,
        ),
        With<crate::core::Intent>,
    >,
    spawns: Query<(&Transform, &crate::core::SpawnPoint), Without<crate::core::Intent>>,
    mut damage: MessageWriter<crate::core::Damage>,
) {
    let Some(kill) = kill else { return };
    for (e, mut at, velocity, health, god, team) in &mut characters {
        if at.translation.y >= kill.0 {
            continue;
        }
        match health {
            Some(h) if !god => {
                if h.current > 0.0 {
                    damage.write(crate::core::Damage {
                        force: bevy::math::Vec3::ZERO,
                        target: e,
                        attacker: None,
                        amount: h.current.max(1.0) * 1000.0,
                        point: at.translation,
                        dir: Vec3::NEG_Y,
                        hitgroup: crate::core::Hitgroup::Generic,
                        kind: crate::core::DamageKind::Generic,
                        weapon: None,
                    });
                }
            }
            _ => {
                let spawn = spawns
                    .iter()
                    .find(|(_, s)| s.team.is_some() && s.team == team.copied())
                    .or_else(|| spawns.iter().next());
                if let Some((s, _)) = spawn {
                    at.translation = s.translation;
                    if let Some(mut v) = velocity {
                        v.0 = Vec3::ZERO;
                    }
                }
            }
        }
    }
}

/// The map's playable area (see `MapData::playable`).
#[derive(Resource)]
struct PlayableArea((Vec3, Vec3));

/// The 3D skybox also exists where it was built, outside the playable area
/// (as in Source, where you can noclip to it). Drawn there only while the
/// camera is outside the playable area too: without the game's visibility
/// data, that keeps it from floating in view from inside the map.
#[allow(clippy::type_complexity)]
fn show_skybox_in_place(
    area: Option<Res<PlayableArea>>,
    cameras: Query<(&GlobalTransform, &Camera), (With<Camera3d>, Without<SkyboxCamera>, Without<ViewModelCamera>, Without<water::WaterReflectionCamera>)>,
    mut parts: Query<&mut bevy::camera::visibility::RenderLayers, Without<Camera>>,
    mut outside_before: Local<Option<bool>>,
) {
    let Some(area) = area else { return };
    let Some((eye, _)) = cameras.iter().find(|(_, c)| c.is_active) else {
        return;
    };
    let p = eye.translation();
    let (lo, hi) = area.0;
    let outside = p.cmplt(lo).any() || p.cmpgt(hi).any();
    if *outside_before == Some(outside) {
        return;
    }
    *outside_before = Some(outside);
    let sky = bevy::camera::visibility::RenderLayers::layer(SKYBOX_LAYER);
    let both = bevy::camera::visibility::RenderLayers::from_layers(&[0, SKYBOX_LAYER]);
    for mut layers in &mut parts {
        if layers.intersects(&sky) {
            *layers = if outside { both.clone() } else { sky.clone() };
        }
    }
}

/// Set when the map has a real 3D skybox (not just the 2D-sky camera).
#[derive(Resource)]
struct ActiveMapHas3dSky;

/// Holds nothing: the sky camera's layer when only the 2D sky shows.
const EMPTY_LAYER: usize = 31;

/// The map's sky cubemap, for cameras to show.
#[derive(Resource, Clone)]
pub struct MapSkybox(pub Handle<Image>);

/// The loaded map's presentation settings, for cameras to follow.
#[derive(Resource, Clone, Debug)]
pub struct ActiveMapLook(pub MapLook);

/// The loaded map's name (`MapData::name`), for reports; the client sets
/// it (from `--map` and the `map` command) so the simulation's entities
/// stay as they were (bot seeds come from entity ids).
#[derive(Resource, Clone, Debug, Default, PartialEq)]
pub struct LoadedMapName(pub String);

/// Marks every entity belonging to the loaded map.
#[derive(Component)]
pub struct MapPart;

impl Plugin for MapPlugin {
    fn build(&self, app: &mut App) {
        if let Some(data) = &self.data {
            app.insert_resource(PendingMap(data.clone(), self.view))
                .insert_resource(ActiveMapLook(data.look.clone()));
        }
        ragdoll::plugin(app);
        app.add_plugins(sound::SoundPlugin)
            .init_resource::<ShowLocalBody>()
            .init_resource::<vis::NoVis>()
            .init_resource::<vis::PortalsOpenAll>()
            .init_resource::<vis::VisStats>()
            .add_message::<decal::PlaceDecal>()
            .add_message::<ViewModelEvent>()
            .add_message::<SpawnGibs>()
            .add_message::<breakables::BreakProp>()
            .add_message::<prop_physics::PropAwakened>()
            .add_message::<GlassShatter>()
            .init_resource::<particles::Particles>()
            .configure_sets(Update, particles::ParticleSet::Step.before(particles::ParticleSet::Draw))
            .init_resource::<tracer::MuzzleCache>()
            .init_resource::<tracer::PendingTracers>()
            .add_message::<tracer::Tracer>()
            .insert_resource(GlobalAmbientLight {
                brightness: 600.0,
                // Baked lighting already includes the map's ambient light.
                affects_lightmapped_meshes: false,
                ..default()
            })
            .add_systems(Startup, spawn_map.run_if(resource_exists::<PendingMap>))
            .add_systems(
                FixedUpdate,
                (attach_hitboxes, ragdoll::record_poses, pose_hitboxes)
                    .chain()
                    .before(crate::core::SimSet::Movement),
            )
            .add_systems(FixedUpdate, fall_out_of_map.after(crate::core::SimSet::Movement))
            .add_systems(
                FixedUpdate,
                breakables::round_restart
                    .after(crate::core::SimSet::Rules)
                    .before(crate::core::SimSet::Movement),
            )
            .add_systems(FixedPostUpdate, breakables::update_panes)
            .add_systems(FixedPostUpdate, prop_physics::wake_on_contact.after(PhysicsSystems::Writeback))
            // What the logic switches: sprites and dust, lights, prop
            // skins, bodies and sequences.
            .add_systems(
                Update,
                (
                    switch_parts.before(glow_visibility).before(dust::update_dust),
                    relight,
                    apply_prop_looks,
                    animate_props,
                ),
            )
            .add_systems(Update, (breakables::break_props, breakables::spawn_gibs, breakables::fly_gibs).chain())
            // Before the physics step reads it (headless too).
            .add_systems(FixedUpdate, loose::attach_loose.before(crate::core::SimSet::Movement))
            .add_systems(
                Update,
                (
                    attach_sky,
                    glow_visibility,
                    dust::update_dust,
                    (
                        tracer::draw_tracers,
                        particles::step_particles.in_set(particles::ParticleSet::Step),
                        particles::draw_particles
                            .run_if(
                                resource_exists::<Assets<Mesh>>
                                    .and_then(resource_exists::<Assets<particles::ParticleDrawMaterial>>),
                            )
                            .in_set(particles::ParticleSet::Draw),
                    )
                        .chain(),
                    show_skybox_in_place,
                    decal::place_decals,
                    (
                        attach_bodies,
                        show_local_body,
                        turn_bodies,
                        pose_bodies.after(DriveAnimation),
                        attach_held,
                        probe_lit::relight,
                        loose::attach_shown,
                    )
                        .run_if(resource_exists::<CharacterBodies>),
                    view_model::draw_view_models
                        .after(DriveAnimation)
                        .run_if(resource_exists::<view_model::ViewModelAssets>),
                    (
                        view_model::muzzle_flashes,
                        view_model::size_flash_sprites,
                        view_model::age_effects,
                        shells::eject,
                        shells::fly,
                    )
                        .chain()
                        .after(DriveAnimation)
                        .after(view_model::draw_view_models),
                ),
            )
            .add_systems(
                PostUpdate,
                update_prop_shadows
                    .run_if(resource_exists::<Assets<Mesh>>.and_then(resource_exists::<Assets<Image>>))
                    .after(bevy::transform::TransformSystems::Propagate),
            )
            .add_systems(
                PostUpdate,
                // After propagation, so it sees this frame's (interpolated)
                // eye rather than the last physics tick's; before frusta.
                follow_sky_camera
                    .after(bevy::transform::TransformSystems::Propagate)
                    .before(bevy::camera::visibility::VisibilitySystems::UpdateFrusta),
            )
            .add_systems(
                PostUpdate,
                // From this frame's camera, before visibility propagates.
                vis::cull
                    .after(bevy::transform::TransformSystems::Propagate)
                    .before(bevy::camera::visibility::VisibilitySystems::VisibilityPropagate),
            );
    }
}

/// Bevy adds `lightmap * lightmap_exposure` as light, then applies the
/// camera's exposure. This cancels the default camera exposure (EV100 9.7)
/// so a lightmap value of 1.0 shows a texture at its own brightness.
const LIGHTMAP_EXPOSURE: f32 = 1.2 * 831.746_4; // 1.2 * 2^9.7

/// Height of the capsule center above the feet, so spawns start standing.
pub const SPAWN_LIFT: f32 = 1.0;

fn spawn_map(
    mut commands: Commands,
    pending: Res<PendingMap>,
    mut meshes: Option<ResMut<Assets<Mesh>>>,
    mut materials: Option<ResMut<Assets<StandardMaterial>>>,
    mut images: Option<ResMut<Assets<Image>>>,
    mut world_materials: Option<ResMut<Assets<WorldMaterial>>>,
    mut rope_materials: Option<ResMut<Assets<RopeMaterial>>>,
    mut sprite_materials: Option<ResMut<Assets<SpriteMaterial>>>,
    mut prop_materials: Option<ResMut<Assets<PropMaterial>>>,
    mut shadow_materials: Option<ResMut<Assets<shadows::ShadowMaterial>>>,
    mut water_materials: Option<ResMut<Assets<water::WaterMaterial>>>,
    mut bindposes: Option<ResMut<Assets<bevy::mesh::skinning::SkinnedMeshInverseBindposes>>>,
) {
    let data = &pending.0;
    let view = pending.1;
    info!(
        "spawning map: world material {}, lightmap {}",
        if world_materials.is_some() {
            "available"
        } else {
            "missing"
        },
        data.lightmap.is_some()
    );
    let root = commands
        .spawn((
            Name::new(format!("Map {}", data.name)),
            MapPart,
            Transform::default(),
            Visibility::default(),
        ))
        .id();

    if !data.collision_indices.is_empty() {
        commands.insert_resource(MapTerrain::from_triangles(
            data.collision_indices
                .iter()
                .map(|t| t.map(|i| Vec3::from(data.collision_positions[i as usize]))),
            TERRAIN_THICKNESS,
        ));
        let positions: Vec<Vec3> = data.collision_positions.iter().map(|p| Vec3::from(*p)).collect();
        commands.entity(root).with_child((
            Name::new("Map collision (surfaces)"),
            MapPart,
            MapTerrainCollider,
            RigidBody::Static,
            Collider::trimesh(positions.clone(), data.collision_indices.clone()),
            // Loose items slide on their own copy (below).
            CollisionLayers::new(LayerMask::DEFAULT, LayerMask::ALL & !crate::core::ITEM_LAYER),
            Transform::default(),
        ));
        if let Some(smooth) = smooth_terrain(positions, &data.collision_indices) {
            commands.entity(root).with_child((
                Name::new("Map collision (surfaces, for loose items)"),
                MapPart,
                MapTerrainCollider,
                RigidBody::Static,
                smooth,
                CollisionLayers::new(crate::core::ITEM_GROUND_LAYER, crate::core::ITEM_LAYER),
                Transform::default(),
            ));
        }
    }
    if !data.collision_hulls.is_empty() {
        let hulls: Vec<_> = data
            .collision_hulls
            .iter()
            .filter_map(|h| Collider::convex_hull(h.iter().map(|p| Vec3::from(*p)).collect()))
            .map(|c| (Vec3::ZERO, Quat::IDENTITY, c))
            .collect();
        commands.entity(root).with_child((
            Name::new("Map collision (solids)"),
            MapPart,
            MapBrushCollider,
            RigidBody::Static,
            Collider::compound(hulls),
            Transform::default(),
        ));
    }

    // Mover brush entities: a node each (placed by origin and angles), with
    // a kinematic collider; their meshes are spawned under it below.
    let entity_nodes: Vec<Option<Entity>> = data
        .entities
        .iter()
        .enumerate()
        .map(|(i, e)| {
            if !e.mover {
                return None;
            }
            let scale = data.entity_scale;
            let rotation = entities::entity_rotation(e.angles());
            let collider = entities::brush_collider(e, scale);
            let mut node = commands.spawn((
                Name::new(format!("Brush entity {i} ({})", e.classname())),
                MapPart,
                MapBrushEntity(i),
                Transform::from_translation(entities::entity_to_engine(e.origin(), scale))
                    .with_rotation(entities::rotation_to_engine(rotation)),
                Visibility::default(),
                ChildOf(root),
            ));
            if let Some(collider) = collider {
                node.insert((MapBrushCollider, RigidBody::Kinematic, collider, contact_filter::hooks()));
            }
            Some(node.id())
        })
        .collect();
    let parent_of = |m: &MapMesh| m.entity.and_then(|i| entity_nodes.get(i).copied().flatten()).unwrap_or(root);
    if !data.entities.is_empty() {
        commands.insert_resource(MapEntities {
            entities: Arc::new(data.entities.clone()),
            scale: data.entity_scale,
        });
    }
    commands.insert_resource(breakables::prop_breaks(data));

    // Prop models: one collider per model (shared by its placements), and
    // render handles when rendering exists.
    let model_colliders: Vec<Option<Collider>> = data.models.iter().map(model_collider).collect();
    let mut model_parts: Vec<Vec<(Handle<Mesh>, Handle<StandardMaterial>)>> = Vec::new();
    // Per model mesh: [in the world (fogged), in the 3D skybox].
    let mut lit_model_materials: Vec<Vec<[Handle<PropMaterial>; 2]>> = Vec::new();
    // Per model with several skins: each skin's materials per mesh.
    let mut skin_model_materials: Vec<Vec<Vec<[Handle<PropMaterial>; 2]>>> = Vec::new();
    // Cubemap handles, and prop materials with the nearest one, made as
    // props need them: (model, mesh, sky layer, cubemap).
    let mut cubemap_handles: Vec<Handle<Image>> = Vec::new();
    let mut texture_handles: Vec<Handle<Image>> = Vec::new();
    let mut rig_bindposes: HashMap<usize, Handle<bevy::mesh::skinning::SkinnedMeshInverseBindposes>> = HashMap::new();
    let mut envmap_variants: std::collections::HashMap<(usize, usize, bool, usize), Handle<PropMaterial>> =
        std::collections::HashMap::new();

    // Headless: dropped weapons and gibs still have their bodies, undrawn.
    if (meshes.is_none() || materials.is_none() || images.is_none() || bindposes.is_none())
        && let Some(root) = data.characters.first().map(|c| c.root)
    {
        let loose = data
            .held
            .iter()
            .map(|h| (h.key.clone(), loose::asset(h, root, Vec::new())))
            .collect();
        commands.insert_resource(loose::LooseAssets(loose));
    }
    if (meshes.is_none() || materials.is_none() || images.is_none() || prop_materials.is_none())
        && let Some(gibs) = breakables::build_assets(data, &[], view, None)
    {
        commands.insert_resource(gibs);
    }
    if let (Some(meshes), Some(materials), Some(images)) = (meshes.as_mut(), materials.as_mut(), images.as_mut()) {
        let textures: Vec<Handle<Image>> = data
            .textures
            .iter()
            .map(|t| images.add(to_image(t, &data.look)))
            .collect();
        let cubemaps: Vec<Handle<Image>> = data.cubemaps.iter().map(|c| images.add(cube_image(c))).collect();
        cubemap_handles = cubemaps.clone();
        texture_handles = textures.clone();
        // Character bodies, drawn by `attach_bodies`: skinned when the
        // model has a skeleton.
        if let Some(bindposes) = bindposes.as_mut() {
            // Bodies and what they hold are lit from the light field where
            // they are (probe_lit), as Source lights models.
            let Some(pm) = prop_materials.as_deref_mut() else {
                panic!("PropMaterial assets exist wherever StandardMaterial does");
            };
            let model_material = |m: &MapMesh| {
                let mut material = lit_prop_material(m, &textures, data, view, false);
                if !m.unlit && view != MapDebugView::Albedo && data.light_field.is_some() {
                    material.params.set_probe(&LightProbe::default(), 1.0);
                }
                material
            };
            let lighting_scale = if let MapDebugView::Lighting { scale } = view { scale } else { 1.0 };
            commands.insert_resource(probe_lit::ProbeLightScale(data.look.light_scale * lighting_scale));
            let bodies = data
                .characters
                .iter()
                .map(|c| body_assets(&c.model, &c.bones, c.root, meshes, pm, bindposes, &model_material))
                .collect();
            commands.insert_resource(CharacterBodies(bodies));
            let held = data
                .held
                .iter()
                .map(|h| {
                    let parts: Vec<(Handle<Mesh>, Handle<PropMaterial>)> = h
                        .model
                        .meshes
                        .iter()
                        .map(|m| (meshes.add(build_mesh(m, false)), pm.add(model_material(m))))
                        .collect();
                    (h.key.clone(), (h.bone.clone(), parts, h.muzzle))
                })
                .collect::<HashMap<_, _>>();
            // Loose (dropped) forms, sized by the first character's scale.
            if let Some(root) = data.characters.first().map(|c| c.root) {
                let loose = data
                    .held
                    .iter()
                    .filter_map(|h| {
                        let parts = held.get(&h.key)?.1.clone();
                        Some((h.key.clone(), loose::asset(h, root, parts)))
                    })
                    .collect();
                commands.insert_resource(loose::LooseAssets(loose));
            }
            commands.insert_resource(HeldAssets(held));
            if let Some(prop_materials) = prop_materials.as_mut() {
                commands.insert_resource(view_model::build_assets(
                    data,
                    &textures,
                    &cubemaps,
                    view,
                    meshes,
                    prop_materials,
                    bindposes,
                ));
            }
            if let Some(flash) =
                view_model::build_flash_assets(data, &textures, meshes, sprite_materials.as_deref_mut())
            {
                commands.insert_resource(flash);
            }
            if let Some(prop_materials) = prop_materials.as_mut()
                && let Some(shells) = shells::build_assets(data, &textures, view, meshes, prop_materials)
            {
                commands.insert_resource(shells);
            }
            if let Some(prop_materials) = prop_materials.as_mut()
                && let Some(gibs) = breakables::build_assets(data, &textures, view, Some((meshes, prop_materials)))
            {
                commands.insert_resource(gibs);
            }
        }
        let source_ldr = data.look.source_ldr_lightmaps;
        let built = data
            .lightmap
            .as_ref()
            .map(|l| LightmapLayers::build(&l.rgb, l.bumped.as_ref(), l.width, l.height, source_ldr));
        let lightmap = built.as_ref().map(|b| images.add(b.plain.clone()));
        let world_lightmap = built.as_ref().map(|b| images.add(b.world.clone()));
        let bumped_lightmaps: Option<[Handle<Image>; 3]> =
            built.and_then(|b| b.bumped).map(|pages| pages.map(|p| images.add(p)));
        // Switchable light styles relight these images (`relight`).
        if let Some(l) = data.lightmap.as_ref().filter(|l| !l.styles.is_empty()) {
            commands.insert_resource(SwitchableLightmaps {
                atlas: Arc::new(l.clone()),
                source_ldr,
                plain: lightmap.clone(),
                world: world_lightmap.clone(),
                bumped: bumped_lightmaps.clone(),
                applied: l.styles.iter().map(|s| s.on).collect(),
            });
        }
        let mut sky_handle = None;
        if let Some(sky) = &data.sky
            && view == MapDebugView::Normal
        {
            let handle = images.add(sky_image(sky, &data.textures));
            sky_handle = Some(handle.clone());
            commands.insert_resource(MapSkybox(handle));
        }
        if let Some(bounds) = data.playable {
            commands.insert_resource(PlayableArea(bounds));
        }

        if let Some(cam) = &data.sky_camera
            && view == MapDebugView::Normal
        {
            commands.insert_resource(SkyCameraInfo(cam.clone()));
            commands.insert_resource(ActiveMapHas3dSky);
        }
        if let Some(vis) = &data.sky_vis
            && view == MapDebugView::Normal
        {
            commands.insert_resource(SkyVis(vis.clone()));
            // The sky is drawn by a sky camera even without a 3D skybox, so
            // it can be switched off where the map can't see it.
            if data.sky_camera.is_none() {
                commands.insert_resource(SkyCameraInfo(MapSkyCamera {
                    origin: Vec3::ZERO,
                    scale: 1.0,
                    fog: None,
                }));
            }
        }
        // Water surfaces have their own material (map::water).
        let water_drawn = water_materials.is_some() && view == MapDebugView::Normal;
        if water_drawn && let Some(water_materials) = water_materials.as_mut() {
            water::spawn_surfaces(
                &mut commands,
                data,
                root,
                meshes,
                images,
                water_materials,
                &cubemaps,
                sky_handle.as_ref(),
            );
        }
        // World meshes in chunks with tight bounds, tagged with the
        // clusters they touch (see `vis`). Meshes of entities with their own
        // node (movers, breakables) stay whole and untagged.
        let visibility = data.visibility.as_deref().filter(|_| !merged_world());
        let tag = |e: &mut EntityCommands, clusters: Vec<u32>| {
            if visibility.is_some() && !clusters.is_empty() {
                e.insert(vis::VisClusters::new(clusters));
            }
        };
        let chunk_size = vis::chunk_size();
        for m in &data.meshes {
            if water_drawn && m.water.is_some() {
                continue;
            }
            let own_node = parent_of(m) != root;
            let mesh_vis = visibility.filter(|_| !own_node);
            // Chunks are placed at their centre (vertices relative to it,
            // small numbers), each a map part of its own.
            let chunks: Vec<(MapMesh, Vec<u32>, Vec3)> = vis::split_mesh(m, mesh_vis, chunk_size)
                .into_iter()
                .map(|(mut chunk, clusters)| {
                    let centre = if mesh_vis.is_some() { vis::recentre(&mut chunk) } else { Vec3::ZERO };
                    (chunk, clusters, centre)
                })
                .collect();
            let lit = lightmap.as_ref().filter(|_| m.lightmap_uvs.len() == m.positions.len());
            // Lightmapped world surfaces: Source-style texture x baked light.
            if let (Some(_), Some(lm), Some(world_materials)) = (lit, world_lightmap.as_ref(), world_materials.as_mut())
            {
                let [r, g, b] = m.color;
                // Calibration hook: MASHUP_NORMAL_G_SIGN overrides the sign;
                // 0 turns radiosity normal mapping off.
                let g_sign = std::env::var("MASHUP_NORMAL_G_SIGN")
                    .ok()
                    .and_then(|v| v.parse::<f32>().ok())
                    .unwrap_or(NORMAL_G_SIGN);
                let bumped = m.normal_map.is_some() && bumped_lightmaps.is_some() && g_sign != 0.0;
                // Two-texture blend: the weights ride in the vertex color's alpha.
                let blend = m.blend.unwrap_or_default();
                let blended = blend.texture.is_some() && m.blend_weights.len() == m.positions.len();
                let material = WorldMaterial {
                    params: WorldParams {
                        base_color: if m.texture.is_some() {
                            Vec4::ONE
                        } else {
                            Vec4::new(r as f32 / 255.0, g as f32 / 255.0, b as f32 / 255.0, 1.0)
                        },
                        light_scale: data.look.light_scale,
                        lightmap_scale: if source_ldr { SOURCE_LIGHTMAP_SCALE } else { 1.0 },
                        bicubic: if data.look.bicubic_lightmaps { 1.0 } else { 0.0 },
                        translucent: m.alpha.shader_mode(),
                        blend: if blended { 1.0 } else { 0.0 },
                        blend_masked: if blend.mask.is_some() { 1.0 } else { 0.0 },
                        blend_normal: if bumped && blend.normal_map.is_some() { 1.0 } else { 0.0 },
                        detail: m.detail.map_or(0.0, |d| d.mode as f32 + 1.0),
                        detail_factor: m.detail.map_or(0.0, |d| d.factor),
                        detail_scale: m.detail.map_or(Vec2::ONE, |d| Vec2::from_array(d.scale)),
                        fog_color: fog_color(data.fog.as_ref().filter(|_| view == MapDebugView::Normal && !m.skybox)),
                        fog_range: fog_range(data.fog.as_ref()),
                        bumped: if bumped { 1.0 } else { 0.0 },
                        normal_g_sign: g_sign,
                        normal_x_sign: std::env::var("MASHUP_NORMAL_X_SIGN")
                            .ok()
                            .and_then(|v| v.parse::<f32>().ok())
                            .unwrap_or(NORMAL_X_SIGN),
                        alpha_cutoff: if let MapAlpha::Mask(c) = m.alpha { c } else { 0.0 },
                        debug_view: match view {
                            MapDebugView::Normal => 0.0,
                            MapDebugView::Lighting { .. } => 1.0,
                            MapDebugView::Albedo => 2.0,
                        },
                        envmap: if m.envmap.is_some_and(|e| e.cubemap.is_some()) {
                            1.0
                        } else {
                            0.0
                        },
                        envmap_mask: match m.envmap.map(|e| e.mask) {
                            Some(EnvmapMask::NormalAlpha) if m.normal_map.is_some() => 1.0,
                            Some(EnvmapMask::BaseAlphaInverted) => 2.0,
                            Some(EnvmapMask::Texture(_)) => 3.0,
                            _ => 0.0,
                        },
                        has_normal: if m.normal_map.is_some() && g_sign != 0.0 {
                            1.0
                        } else {
                            0.0
                        },
                        envmap_contrast: m.envmap.map_or(0.0, |e| e.contrast),
                        envmap_saturation: m.envmap.map_or(1.0, |e| e.saturation),
                        envmap_fresnel: m.envmap.map_or(1.0, |e| e.fresnel),
                        envmap_tint: m.envmap.map_or(Vec4::ONE, |e| Vec3::from_array(e.tint).extend(1.0)),
                        ..default()
                    },
                    base: m.texture.map(|i| textures[i].clone()),
                    // Bound for radiosity bump lighting, and for reflections
                    // that follow the normal map.
                    normal: m
                        .normal_map
                        .filter(|_| bumped || (m.envmap.is_some() && g_sign != 0.0))
                        .map(|i| textures[i].clone()),
                    envmap: m.envmap.and_then(|e| e.cubemap).map(|c| cubemaps[c].clone()),
                    envmap_mask: match m.envmap.map(|e| e.mask) {
                        Some(EnvmapMask::Texture(i)) => Some(textures[i].clone()),
                        _ => None,
                    },
                    lightmap: Some(lm.clone()),
                    lightmap_b0: bumped_lightmaps.as_ref().map(|b| b[0].clone()),
                    lightmap_b1: bumped_lightmaps.as_ref().map(|b| b[1].clone()),
                    lightmap_b2: bumped_lightmaps.as_ref().map(|b| b[2].clone()),
                    base2: blend.texture.filter(|_| blended).map(|i| textures[i].clone()),
                    normal2: blend
                        .normal_map
                        .filter(|_| blended && bumped)
                        .map(|i| textures[i].clone()),
                    blend_mask: blend.mask.filter(|_| blended).map(|i| textures[i].clone()),
                    detail: m.detail.map(|d| textures[d.texture].clone()),
                    alpha_mode: m.alpha.shader_alpha_mode(),
                    double_sided: m.double_sided,
                    // Map overlays and infodecals (see WorldMaterial::decal).
                    decal: m.material.starts_with("decal:") && m.alpha != MapAlpha::Opaque,
                };
                let material = world_materials.add(material);
                for (chunk, clusters, centre) in chunks {
                    let mut e = commands.spawn((
                        Name::new(chunk.material.clone()),
                        MapPart,
                        world_layer_of(chunk.skybox),
                        Mesh3d(meshes.add({
                            let mut mesh = build_mesh(&chunk, true);
                            if blended {
                                let colors: Vec<[f32; 4]> =
                                    chunk.blend_weights.iter().map(|w| [1.0, 1.0, 1.0, *w]).collect();
                                mesh.insert_attribute(Mesh::ATTRIBUTE_COLOR, colors);
                            }
                            mesh
                        })),
                        MeshMaterial3d(material.clone()),
                        Transform::from_translation(centre),
                        ChildOf(parent_of(m)),
                    ));
                    tag(&mut e, clusters);
                }
                continue;
            }
            let material = materials.add(build_material(m, &textures, view, data.look.light_scale));
            for (chunk, clusters, centre) in chunks {
                let mut part = commands.spawn((
                    Name::new(chunk.material.clone()),
                    MapPart,
                    world_layer_of(chunk.skybox),
                    Mesh3d(meshes.add(build_mesh(&chunk, lit.is_some()))),
                    MeshMaterial3d(material.clone()),
                    Transform::from_translation(centre),
                    ChildOf(parent_of(m)),
                ));
                if let Some(image) = lit {
                    part.insert(bevy::pbr::Lightmap {
                        image: image.clone(),
                        uv_rect: Rect::new(0.0, 0.0, 1.0, 1.0),
                        bicubic_sampling: false,
                    });
                }
                tag(&mut part, clusters);
            }
        }
        model_parts = data
            .models
            .iter()
            .map(|model| {
                model
                    .meshes
                    .iter()
                    .map(|m| {
                        (
                            meshes.add(build_mesh(m, false)),
                            materials.add(build_material(m, &textures, view, data.look.light_scale)),
                        )
                    })
                    .collect()
            })
            .collect();
        if let Some(prop_materials) = prop_materials.as_mut() {
            lit_model_materials = data
                .models
                .iter()
                .map(|model| {
                    model
                        .meshes
                        .iter()
                        .map(|m| {
                            [false, true]
                                .map(|skybox| prop_materials.add(lit_prop_material(m, &textures, data, view, skybox)))
                        })
                        .collect()
                })
                .collect();
            skin_model_materials = data
                .models
                .iter()
                .map(|model| {
                    model
                        .skins
                        .iter()
                        .map(|looks| {
                            model
                                .meshes
                                .iter()
                                .zip(looks)
                                .map(|(m, look)| {
                                    let m = look.apply(m);
                                    [false, true].map(|skybox| {
                                        prop_materials.add(lit_prop_material(&m, &textures, data, view, skybox))
                                    })
                                })
                                .collect()
                        })
                        .collect()
                })
                .collect();
        }

        if view == MapDebugView::Normal {
            for (i, dust) in data.dust.iter().enumerate() {
                let mesh = meshes.add(dust::empty_mesh());
                let clusters = visibility.map(|v| vis::box_clusters(v, dust.min, dust.max)).unwrap_or_default();
                let mut e = commands.spawn((
                    Name::new(format!("Dust {i}")),
                    MapPart,
                    Mesh3d(mesh.clone()),
                    MeshMaterial3d(materials.add(StandardMaterial {
                        base_color_texture: dust.texture.map(|t| textures[t].clone()),
                        unlit: true,
                        alpha_mode: AlphaMode::Blend,
                        double_sided: true,
                        cull_mode: None,
                        ..default()
                    })),
                    dust::DustEmitter::new(dust.clone(), mesh, i as u64 + 1),
                    // The mesh moves with the motes; don't cull it by stale bounds.
                    bevy::camera::visibility::NoFrustumCulling,
                    bevy::light::NotShadowCaster,
                    Transform::default(),
                    ChildOf(root),
                ));
                tag(&mut e, clusters);
                if let Some(entity) = dust.entity {
                    e.insert(EntityPart {
                        entity,
                        on: dust.start_on,
                        exists: true,
                    });
                }
            }
        }
        if let Some(sprite_materials) = sprite_materials.as_mut()
            && view == MapDebugView::Normal
        {
            for (i, sprite) in data.sprites.iter().enumerate() {
                let t = &data.textures[sprite.texture];
                let mut e = commands.spawn((
                    Name::new(format!("Sprite {i}")),
                    MapPart,
                    Mesh3d(meshes.add(sprite_material::sprite_mesh(UVec2::new(t.width, t.height)))),
                    MeshMaterial3d(sprite_materials.add(SpriteMaterial {
                        params: SpriteParams {
                            color: sprite.color,
                            size: sprite.size,
                        },
                        texture: Some(textures[sprite.texture].clone()),
                        glow: sprite.glow,
                    })),
                    bevy::light::NotShadowCaster,
                    // The quad is spread in the vertex shader, so the mesh's
                    // own bounds are a point: culling would drop the sprite
                    // as soon as its centre left the view.
                    bevy::camera::visibility::NoFrustumCulling,
                    Transform::from_translation(sprite.position),
                    ChildOf(root),
                ));
                if let Some(v) = visibility {
                    let r = Vec3::splat(sprite.size.max_element() / 2.0);
                    tag(&mut e, vis::box_clusters(v, sprite.position - r, sprite.position + r));
                }
                if sprite.glow {
                    e.insert(GlowSprite {
                        color: sprite.color,
                        proxy: sprite.proxy,
                    });
                }
                if let Some(entity) = sprite.entity {
                    e.insert(EntityPart {
                        entity,
                        on: sprite.start_on,
                        exists: true,
                    });
                    if !sprite.start_on {
                        e.insert((vis::LogicHidden, Visibility::Hidden));
                    }
                }
            }
        }
        if let Some(rope_materials) = rope_materials.as_mut()
            && view == MapDebugView::Normal
        {
            for (i, rope) in data.ropes.iter().enumerate() {
                let mesh = meshes.add(rope_material::rope_mesh(rope));
                let clusters = match visibility {
                    Some(v) if !rope.points.is_empty() => {
                        let lo = rope.points.iter().fold(Vec3::MAX, |a, p| a.min(*p));
                        let hi = rope.points.iter().fold(Vec3::MIN, |a, p| a.max(*p));
                        let r = Vec3::splat(rope.width);
                        vis::box_clusters(v, lo - r, hi + r)
                    }
                    _ => Vec::new(),
                };
                let strips =
                    rope.back
                        .map(|(t, n)| (t, n, true))
                        .into_iter()
                        .chain([(rope.texture, rope.normal_map, false)]);
                for (texture, normal, back) in strips {
                    let mut e = commands.spawn((
                        Name::new(format!("Rope {i}{}", if back { " (back)" } else { "" })),
                        MapPart,
                        Mesh3d(mesh.clone()),
                        MeshMaterial3d(rope_materials.add(RopeMaterial {
                            params: RopeParams {
                                width: rope.width,
                                back: if back { 1.0 } else { 0.0 },
                                light_scale: data.look.light_scale,
                                has_normal_map: if normal.is_some() { 1.0 } else { 0.0 },
                                fog: fog::world_fog(data.fog.as_ref()),
                            },
                            base: texture.map(|t| textures[t].clone()),
                            normal: normal.map(|t| textures[t].clone()),
                            blend: back,
                        })),
                        bevy::light::NotShadowCaster,
                        Transform::default(),
                        ChildOf(root),
                    ));
                    tag(&mut e, clusters.clone());
                }
            }
        }
        if let (Some(settings), Some(shadow_materials)) = (&data.shadows, shadow_materials.as_mut())
            && view == MapDebugView::Normal
        {
            let built = shadows::build(&data, settings);
            info!("prop shadows: {} casters reach the world", built.meshes.len());
            let atlas = images.add(built.atlas.image());
            let atlas_handle = atlas.clone();
            let material = shadow_materials.add(shadows::ShadowMaterial {
                params: shadows::ShadowParams {
                    color: shadows::shadow_color(settings.color),
                    texel: Vec2::new(1.0 / built.atlas.width as f32, 1.0 / built.atlas.height as f32),
                    fog_color: fog_color(data.fog.as_ref()),
                    fog_range: fog_range(data.fog.as_ref()),
                },
                atlas,
            });
            let mut entities = std::collections::HashMap::new();
            for (prop, mesh) in built.meshes {
                let clusters = match (visibility, bevy::camera::primitives::MeshAabb::compute_aabb(&mesh)) {
                    // Physics props' shadows move with them: always drawn.
                    (Some(v), Some(aabb)) if data.props[prop].physics.is_none() => {
                        vis::box_clusters(v, Vec3::from(aabb.min()), Vec3::from(aabb.max()))
                    }
                    _ => Vec::new(),
                };
                let mut e = commands
                    .spawn((
                        Name::new(format!("Shadow of prop {prop}")),
                        MapPart,
                        PropShadow { prop },
                        Mesh3d(meshes.add(mesh)),
                        MeshMaterial3d(material.clone()),
                        bevy::light::NotShadowCaster,
                        Transform::default(),
                        ChildOf(root),
                    ));
                tag(&mut e, clusters);
                entities.insert(prop, e.id());
            }
            commands.insert_resource(ShadowState {
                data: data.clone(),
                settings: settings.clone(),
                receivers: built.receivers,
                atlas: built.atlas,
                atlas_image: atlas_handle,
                material,
                cells: built.cells.into_iter().map(|c| (c.prop, c)).collect(),
                entities,
                root,
            });
        }
    }

    // Exact brushes for movement that sweeps them (Source): the world's,
    // plus props that are (close to) convex, in world space.
    let mut brushes = data.collision_brushes.clone();
    // Comparison hook: MASHUP_NO_PROP_BRUSHES=1 sweeps props as meshes.
    let no_prop_brushes = std::env::var("MASHUP_NO_PROP_BRUSHES").is_ok_and(|v| v == "1");
    // Models without a collision model: their hull, when (nearly) convex.
    let model_hulls: Vec<Option<Vec<(Vec3, f32)>>> = data
        .models
        .iter()
        .map(|m| {
            m.collision
                .is_none()
                .then(|| convex_planes(m, CONVEX_SURFACE_SHARE))
                .flatten()
        })
        .collect();
    info!(
        "props: {} of {} models collide by their collision model, {} more as convex hulls",
        data.models.iter().filter(|m| m.collision.is_some()).count(),
        data.models.len(),
        model_hulls.iter().filter(|h| h.is_some()).count(),
    );

    for (i, prop) in data.props.iter().enumerate() {
        // Convex pieces as planes: the model's collision model (each piece
        // exactly), else its hull when (nearly) convex, else the bounds box.
        let model = &data.models[prop.model];
        let pieces: Vec<Vec<(Vec3, f32)>> = match (prop.solid, &model.collision) {
            (PropSolid::Mesh, Some(c)) => c.pieces.iter().map(|p| p.planes.clone()).collect(),
            (PropSolid::Mesh, None) => model_hulls[prop.model].iter().cloned().collect(),
            (PropSolid::Box, _) => {
                let (lo, hi) = model.bounds;
                ((hi - lo).min_element() > 0.0)
                    .then(|| MapBrush::from_box(lo, hi).planes)
                    .into_iter()
                    .collect()
            }
            (PropSolid::None, _) => Vec::new(),
        };
        // Bodies that move are swept through physics queries, not as brushes.
        // Frozen ones stay put like static props until enabled
        // (`prop_physics::FrozenBody`).
        let dynamic = prop.physics.as_ref().filter(|p| !prop.skybox && !p.frozen);
        let prop_brush = (prop.parent.is_none() && !prop.skybox && !no_prop_brushes && !pieces.is_empty() && dynamic.is_none()).then_some(());
        // A prop entity's brushes go on its node (`MovingSolid`, still), so
        // they stop blocking once the logic removes it; others join the map's.
        let placed_brushes: Vec<MapBrush> = if prop_brush.is_some() {
            pieces
                .iter()
                .map(|planes| place_brush(planes, prop.translation, prop.rotation, model.surfaceprop.clone()))
                .collect()
        } else {
            Vec::new()
        };
        let own_solid = (prop.entity.is_some() && !placed_brushes.is_empty()).then(|| MovingSolid {
            brushes: placed_brushes.clone(),
            velocity: Vec3::ZERO,
            solid: true,
        });
        if own_solid.is_none() {
            brushes.extend(placed_brushes);
        }
        let placed = Transform::from_translation(prop.translation).with_rotation(prop.rotation);
        // Riding a mover: under its node, placed relative to it.
        let rider = prop
            .parent
            .and_then(|p| Some((entity_nodes.get(p).copied().flatten()?, data.entities.get(p)?)))
            .map(|(node, ent)| {
                let at = Transform::from_translation(entities::entity_to_engine(ent.origin(), data.entity_scale))
                    .with_rotation(entities::rotation_to_engine(entities::entity_rotation(ent.angles())));
                (node, Transform::from_matrix(at.to_matrix().inverse() * placed.to_matrix()))
            });
        let mut e = commands.spawn((
            Name::new(format!("Prop {i}")),
            PropIndex(i),
            MapPart,
            rider.map_or(placed, |r| r.1),
            Visibility::default(),
            ChildOf(rider.map_or(root, |r| r.0)),
        ));
        if let Some(solid) = own_solid {
            e.insert(solid);
        }
        // Props that stay put are hidden where the camera can't see them,
        // and beyond their fade distance (animated ones move anywhere);
        // between the near and far fade distances their meshes dither out
        // (Bevy's visibility range: opaque passes, no sorting).
        let mut fade_range = None;
        if !prop.skybox && dynamic.is_none() && rider.is_none() && model.rig.is_none() && !merged_world() {
            let clusters = match data.visibility.as_deref() {
                Some(v) => {
                    let (lo, hi) = model.bounds;
                    let corners = (0..8).map(|k| {
                        let c = Vec3::new(
                            if k & 1 == 0 { lo.x } else { hi.x },
                            if k & 2 == 0 { lo.y } else { hi.y },
                            if k & 4 == 0 { lo.z } else { hi.z },
                        );
                        prop.translation + prop.rotation * c
                    });
                    let (min, max) = corners.fold((Vec3::MAX, Vec3::MIN), |(a, b), p| (a.min(p), b.max(p)));
                    vis::box_clusters(v, min, max)
                }
                None => Vec::new(),
            };
            if !clusters.is_empty() || prop.fade.is_some() {
                e.insert(vis::VisClusters::new(clusters));
            }
            if let Some((near, far)) = prop.fade {
                e.insert(vis::FadeDistance(far));
                fade_range = vis::fade_band(near, far);
            }
        }
        if let Some(index) = prop.entity {
            e.insert((PropEntity(index), PropHome(rider.map_or(placed, |r| r.1))));
        }
        let solid = if rider.is_some() { PropSolid::None } else { prop.solid };
        match (solid, &model_colliders[prop.model]) {
            (PropSolid::Mesh, Some(collider)) if dynamic.is_some() => {
                let p = dynamic.unwrap();
                e.insert((collider.clone(), prop_physics::dynamic_body(p, data.models[prop.model].bounds)));
                if p.asleep && prop.entity.is_some() {
                    e.insert((prop_physics::StartAsleep, RigidBody::Static));
                }
                if let Some(s) = &model.surfaceprop {
                    e.insert(PropSurface(s.clone()));
                }
            }
            (PropSolid::Mesh, Some(collider)) => {
                e.insert((RigidBody::Static, collider.clone(), MapPropCollider));
                // A frozen physics prop can be set moving later.
                if let Some(p) = prop.physics.as_ref().filter(|p| p.frozen && !prop.skybox && prop.entity.is_some()) {
                    e.insert(prop_physics::FrozenBody {
                        physics: p.clone(),
                        bounds: data.models[prop.model].bounds,
                    });
                }
                if let Some(s) = &model.surfaceprop {
                    e.insert(PropSurface(s.clone()));
                }
                if prop_brush.is_some() {
                    e.insert(MapBrushCollider);
                }
            }
            (PropSolid::Box, _) => {
                let (lo, hi) = data.models[prop.model].bounds;
                let size = hi - lo;
                if size.min_element() > 0.0 {
                    let child = (
                        Collider::cuboid(size.x, size.y, size.z),
                        Transform::from_translation((lo + hi) / 2.0),
                    );
                    e.insert(RigidBody::Static).with_children(|c| {
                        let mut ec = c.spawn((child, MapPropCollider));
                        if let Some(s) = &model.surfaceprop {
                            ec.insert(PropSurface(s.clone()));
                        }
                        if prop_brush.is_some() {
                            ec.insert(MapBrushCollider);
                        }
                    });
                }
            }
            _ => {}
        }
        let id = e.id();
        let probe_scale = data.look.light_scale
            * if let MapDebugView::Lighting { scale } = view {
                scale
            } else {
                1.0
            };
        let probe = prop.lighting.as_ref().filter(|_| view != MapDebugView::Albedo);
        let model = &data.models[prop.model];
        // An animated prop's skeleton: a root in the skeleton's space under
        // the node, then each bone under its parent, in the reference pose.
        let rig = model.rig.as_ref().filter(|_| meshes.is_some() && bindposes.is_some());
        let mut joints: Vec<Entity> = Vec::new();
        let mut inverse_bindposes = None;
        if let (Some(rig), Some(bindposes)) = (rig, bindposes.as_mut()) {
            let root = commands.spawn((rig.root, Visibility::Inherited, ChildOf(id))).id();
            for b in &rig.bones {
                let parent = b.parent.and_then(|p| joints.get(p).copied()).unwrap_or(root);
                joints.push(
                    commands
                        .spawn((
                            Transform::from_translation(b.position).with_rotation(b.rotation),
                            Visibility::Inherited,
                            ChildOf(parent),
                        ))
                        .id(),
                );
            }
            inverse_bindposes = Some(
                rig_bindposes
                    .entry(prop.model)
                    .or_insert_with(|| bindposes.add(rig_inverse_bindposes(rig)))
                    .clone(),
            );
        }
        // Body parts: only the meshes of the chosen choice are drawn.
        let body_shown = |m: &MapMesh| {
            m.body
                .is_none_or(|(part, choice)| model.body_choice(prop.body, part as usize) == choice)
        };
        let mut spawned_materials: Vec<Handle<PropMaterial>> = Vec::new();
        match (probe, meshes.as_mut(), lit_model_materials.get(prop.model)) {
            // Baked: own mesh copy with per-vertex light, unlit material
            // (texture x light, like the lightmapped world).
            (Some(probe), Some(meshes), Some(mats)) => {
                for (mesh_index, (m, material)) in model.meshes.iter().zip(mats).enumerate() {
                    let mut material = material[prop.skybox as usize].clone();
                    // `env_cubemap`: the cubemap baked nearest to the prop.
                    if let (Some(env), Some(prop_materials)) =
                        (m.envmap.filter(|e| e.cubemap.is_none()), prop_materials.as_mut())
                        && let Some(&(_, cube)) = data.cubemap_samples.iter().min_by(|a, b| {
                            a.0.distance(prop.translation)
                                .total_cmp(&b.0.distance(prop.translation))
                        })
                        && let Some(cube_handle) = cubemap_handles.get(cube)
                    {
                        let key = (prop.model, mesh_index, prop.skybox, cube);
                        material = envmap_variants
                            .entry(key)
                            .or_insert_with(|| {
                                let mut variant = prop_materials.get(&material).cloned().expect("prop material");
                                set_prop_envmap(&mut variant, &env, cube_handle.clone(), &texture_handles);
                                prop_materials.add(variant)
                            })
                            .clone();
                    }
                    spawned_materials.push(material.clone());
                    let layer = layer_of(prop.skybox);
                    let colors: Vec<[f32; 4]> = m
                        .normals
                        .iter()
                        .map(|n| {
                            if m.unlit {
                                return [1.0, 1.0, 1.0, 1.0];
                            }
                            let l = probe.eval(prop.rotation * Vec3::from(*n)) * probe_scale;
                            [l.x, l.y, l.z, 1.0]
                        })
                        .collect();
                    let mut mesh = build_mesh(m, false);
                    mesh.insert_attribute(Mesh::ATTRIBUTE_COLOR, colors);
                    let skinned = inverse_bindposes.is_some() && m.joints.len() == m.positions.len();
                    if skinned {
                        mesh.insert_attribute(
                            Mesh::ATTRIBUTE_JOINT_INDEX,
                            bevy::mesh::VertexAttributeValues::Uint16x4(m.joints.clone()),
                        );
                        mesh.insert_attribute(Mesh::ATTRIBUTE_JOINT_WEIGHT, m.joint_weights.clone());
                    }
                    let mut c = commands.spawn((
                        Mesh3d(meshes.add(mesh)),
                        MeshMaterial3d(material),
                        layer,
                        PropMeshSlot { mesh: mesh_index },
                        if body_shown(m) {
                            Visibility::Inherited
                        } else {
                            Visibility::Hidden
                        },
                        ChildOf(id),
                    ));
                    if let Some(range) = &fade_range {
                        c.insert(range.clone());
                    }
                    if let (true, Some(inverse)) = (skinned, inverse_bindposes.clone()) {
                        c.insert((
                            bevy::mesh::skinning::SkinnedMesh {
                                inverse_bindposes: inverse,
                                joints: joints.clone(),
                            },
                            // Bounds of the bind pose don't follow the bones.
                            bevy::camera::visibility::NoFrustumCulling,
                        ));
                    }
                }
            }
            _ => {
                if let Some(parts) = model_parts.get(prop.model) {
                    for (mesh_index, ((mesh, material), m)) in parts.iter().zip(&model.meshes).enumerate() {
                        let mut c = commands.spawn((
                            Mesh3d(mesh.clone()),
                            MeshMaterial3d(material.clone()),
                            layer_of(prop.skybox),
                            PropMeshSlot { mesh: mesh_index },
                            if body_shown(m) {
                                Visibility::Inherited
                            } else {
                                Visibility::Hidden
                            },
                            ChildOf(id),
                        ));
                        if let Some(range) = &fade_range {
                            c.insert(range.clone());
                        }
                    }
                }
            }
        }
        // What the logic may change: skin and body (entity props), sequences
        // (animated ones).
        if prop.entity.is_some() {
            let mut skins: Vec<Vec<Handle<PropMaterial>>> = skin_model_materials
                .get(prop.model)
                .map(|skins| {
                    skins
                        .iter()
                        .map(|mats| mats.iter().map(|m| m[prop.skybox as usize].clone()).collect())
                        .collect()
                })
                .unwrap_or_default();
            if let Some(own) = skins.get_mut(prop.skin.max(0) as usize)
                && own.len() == spawned_materials.len()
            {
                *own = spawned_materials;
            }
            commands.entity(id).insert(PropVariants {
                skins,
                body: prop.body,
                parts: model.body_parts.clone(),
                mesh_bodies: model.meshes.iter().map(|m| m.body).collect(),
                shown: (prop.skin, prop.body),
            });
            if let Some(rig) = rig.filter(|_| !joints.is_empty()) {
                commands.entity(id).insert(PropRig {
                    animator: anim::Animator::new(rig.animations.clone()),
                    joints,
                    seen: None,
                    default: None,
                });
            }
        }
    }

    if let Some(v) = data.visibility.as_ref().filter(|_| !merged_world()) {
        commands.insert_resource(vis::ActiveVisibility(v.clone()));
    }
    commands.insert_resource(decal::DecalSurfaces::new(&data));
    commands.insert_resource(surface_color::SurfaceColors::new(data));
    commands.insert_resource(particles::ParticleMaterials(data.particles.clone()));
    if !texture_handles.is_empty() && !data.particles.materials.is_empty() {
        commands.insert_resource(particles::ParticleAssets::new(
            data.particles.clone(),
            texture_handles.clone(),
        ));
    }
    if let Some(h) = &data.hud {
        let images = h
            .sprites
            .values()
            .filter_map(|s| Some((s.texture, texture_handles.get(s.texture)?.clone())))
            .collect();
        commands.insert_resource(hud::ActiveHud(h.clone(), images));
    }
    if let Some(o) = &data.overview
        && let Some(image) = texture_handles.get(o.texture)
    {
        commands.insert_resource(hud::ActiveOverview(o.clone(), image.clone()));
    }
    if !texture_handles.is_empty() && !data.decals.groups.is_empty() {
        commands.insert_resource(decal::DecalAssets::new(data.decals.clone(), texture_handles.clone()));
        commands.insert_resource(decal::ImpactDecals);
    }
    commands.insert_resource(sound::SoundBank(data.sounds.clone()));
    if !data.characters.is_empty() {
        commands.insert_resource(CharacterModels(Arc::new(data.characters.clone())));
    }
    if !data.view_models.is_empty() {
        commands.insert_resource(ViewModels(Arc::new(data.view_models.clone())));
    }
    if let Some(field) = &data.light_field {
        commands.insert_resource(LightField(field.clone()));
    }
    if let Some(nav) = &data.nav {
        commands.insert_resource((**nav).clone());
    }
    commands.insert_resource(sound::SurfaceGrid::new(&data));
    if !brushes.is_empty() {
        let floor = data
            .playable
            .map(|(lo, _)| lo.y)
            .unwrap_or_else(|| brushes.iter().map(|b| b.min.y).fold(f32::MAX, f32::min));
        commands.insert_resource(KillHeight(floor - KILL_MARGIN));
        commands.insert_resource(MapBrushes(brushes));
        commands.insert_resource(MapTraceSkip(data.trace_skip.clone()));
        if let Some(tree) = &data.brush_tree {
            commands.insert_resource(tree.clone());
        }
        if let Some(g) = data.gravity {
            commands.insert_resource(Gravity(Vec3::NEG_Y * g));
        }
        commands.insert_resource(MapWater(data.water.clone()));
        commands.insert_resource(fog::SceneFog(fog::world_fog(data.fog.as_ref())));
        commands.insert_resource(data.round_sounds.clone());
        if let Some(r) = &data.radio {
            commands.insert_resource(r.clone());
        }
    }

    commands.spawn((
        Name::new("Sun"),
        MapPart,
        DirectionalLight {
            illuminance: 9000.0,
            // Models are lit from the baked light field (probe_lit), so no
            // shadow maps (they cost several ms a frame); the sun only lights
            // the few plain materials left.
            affects_lightmapped_mesh_diffuse: false,
            ..default()
        },
        Transform::default().looking_at(Vec3::new(-0.4, -1.0, -0.3), Vec3::Y),
    ));
    for (i, (feet, team)) in data.spawns.iter().enumerate() {
        commands.spawn((
            Name::new(format!("Spawn {i}")),
            MapPart,
            SpawnPoint { team: *team },
            Transform::from_translation(*feet + Vec3::Y * SPAWN_LIFT)
                .with_rotation(Quat::from_rotation_y(data.spawn_yaws.get(i).copied().unwrap_or(0.0))),
        ));
    }
}

/// Comparison hook: MASHUP_MERGED_WORLD=1 spawns the world as one mesh per
/// material with no visibility culling (as before chunking), for A/B
/// measurements against `vis`.
fn merged_world() -> bool {
    std::env::var("MASHUP_MERGED_WORLD").is_ok_and(|v| v == "1")
}

/// Remove the loaded map: its entities, its resources and the sky on
/// cameras. Characters stay.
pub fn unload_map(world: &mut World) {
    let parts: Vec<Entity> = world.query_filtered::<Entity, With<MapPart>>().iter(world).collect();
    for e in parts {
        if let Ok(e) = world.get_entity_mut(e) {
            e.despawn();
        }
    }
    let cams: Vec<Entity> = world
        .query_filtered::<Entity, With<bevy::light::Skybox>>()
        .iter(world)
        .collect();
    for c in cams {
        world.entity_mut(c).remove::<bevy::light::Skybox>();
    }
    world.remove_resource::<PendingMap>();
    world.remove_resource::<MapSkybox>();
    world.remove_resource::<PlayableArea>();
    world.remove_resource::<SkyCameraInfo>();
    world.remove_resource::<ActiveMapHas3dSky>();
    world.remove_resource::<SkyVis>();
    world.remove_resource::<vis::ActiveVisibility>();
    world.remove_resource::<ShadowState>();
    world.remove_resource::<sound::SoundBank>();
    world.remove_resource::<CharacterModels>();
    world.remove_resource::<CharacterBodies>();
    world.remove_resource::<HeldAssets>();
    world.remove_resource::<decal::DecalSurfaces>();
    world.remove_resource::<decal::DecalAssets>();
    world.remove_resource::<decal::ImpactDecals>();
    world.remove_resource::<hud::ActiveHud>();
    world.remove_resource::<hud::ActiveOverview>();
    world.remove_resource::<surface_color::SurfaceColors>();
    world.remove_resource::<particles::ParticleAssets>();
    world.remove_resource::<particles::ParticleMaterials>();
    if let Some(mut p) = world.get_resource_mut::<particles::Particles>() {
        p.groups.clear();
    }
    view_model::unload(world);
    shells::unload(world);
    breakables::unload(world);
    world.remove_resource::<breakables::PropBreaks>();
    let bodies: Vec<Entity> = world
        .query_filtered::<Entity, With<CharacterBody>>()
        .iter(world)
        .collect();
    for b in bodies {
        world.entity_mut(b).despawn();
    }
    let animated: Vec<Entity> = world
        .query_filtered::<Entity, With<anim::Animator>>()
        .iter(world)
        .collect();
    for e in animated {
        world.entity_mut(e).remove::<anim::Animator>();
    }
    let modelled: Vec<Entity> = world.query_filtered::<Entity, With<BodyModel>>().iter(world).collect();
    for e in modelled {
        world.entity_mut(e).remove::<(BodyModel, crate::core::Hitboxes)>();
    }
    world.remove_resource::<nav::NavMesh>();
    world.remove_resource::<sound::SurfaceGrid>();
    world.remove_resource::<MapBrushes>();
    world.remove_resource::<MapBrushTree>();
    world.remove_resource::<KillHeight>();
    world.remove_resource::<MapWater>();
    world.remove_resource::<RoundSounds>();
    world.remove_resource::<radio::RadioCommands>();
    world.remove_resource::<MapEntities>();
    world.remove_resource::<water::MapWaterRender>();
    world.remove_resource::<water::WaterView>();
    world.remove_resource::<fog::SceneFog>();
    world.remove_resource::<MapTerrain>();
    world.insert_resource(Gravity::default());
    soundscape::reset(world);
    live_sound::reset(world);
}

/// Replace the loaded map with `data` (spawned now, as at startup).
pub fn change_map(world: &mut World, data: MapData, view: MapDebugView) {
    unload_map(world);
    let data = Arc::new(data);
    world.insert_resource(ActiveMapLook(data.look.clone()));
    world.insert_resource(PendingMap(data, view));
    if let Err(e) = world.run_system_cached(spawn_map) {
        error!("spawning the map: {e}");
    }
}

/// A repeating, mipmapped GPU image. Mipmaps are box-filtered here; source
/// files' own mip levels aren't used yet.
fn to_image(t: &MapTexture, look: &MapLook) -> Image {
    let mut data = t.rgba8.clone();
    let (mut w, mut h) = (t.width as usize, t.height as usize);
    let mut level = t.rgba8.clone();
    let mut levels = 1;
    // Prefer the texture's own mip levels (the original tools' filtering).
    let complete = (t.width.max(t.height) as f32).log2() as usize;
    if !t.mips.is_empty() && t.mips.len() == complete {
        for m in &t.mips {
            data.extend_from_slice(m);
        }
        levels += t.mips.len() as u32;
        w = 1;
        h = 1;
    }
    while w > 1 || h > 1 {
        let (nw, nh) = ((w / 2).max(1), (h / 2).max(1));
        let mut next = vec![0u8; nw * nh * 4];
        for y in 0..nh {
            for x in 0..nw {
                for c in 0..4 {
                    let mut sum = 0u32;
                    for (dx, dy) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
                        let sx = (x * 2 + dx).min(w - 1);
                        let sy = (y * 2 + dy).min(h - 1);
                        sum += level[(sy * w + sx) * 4 + c] as u32;
                    }
                    next[(y * nw + x) * 4 + c] = (sum / 4) as u8;
                }
            }
        }
        data.extend_from_slice(&next);
        level = next;
        (w, h) = (nw, nh);
        levels += 1;
    }
    let mut image = Image::new_uninit(
        Extent3d {
            width: t.width,
            height: t.height,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        if t.srgb {
            TextureFormat::Rgba8UnormSrgb
        } else {
            TextureFormat::Rgba8Unorm
        },
        RenderAssetUsages::RENDER_WORLD,
    );
    image.data = Some(data);
    image.texture_descriptor.mip_level_count = levels;
    image.sampler = ImageSampler::Descriptor(ImageSamplerDescriptor {
        address_mode_u: ImageAddressMode::Repeat,
        address_mode_v: ImageAddressMode::Repeat,
        mag_filter: ImageFilterMode::Linear,
        min_filter: ImageFilterMode::Linear,
        mipmap_filter: if look.trilinear {
            ImageFilterMode::Linear
        } else {
            ImageFilterMode::Nearest
        },
        anisotropy_clamp: look.anisotropy.max(1),
        ..default()
    });
    image
}

/// Source's parameter gamma-to-linear table (specs/cs_source/shaders.md
/// Quirks): rounded to 1/255; 0.95 and up become 1; above 1 unchanged.
fn gamma_to_linear(v: f32) -> f32 {
    if v > 1.0 {
        return v;
    }
    let v = (v * 255.0).round() / 255.0;
    if v >= 0.95 { 1.0 } else { v.powf(2.2) }
}

/// A baked cubemap as a cube texture (six sRGB layers, clamped, linear).
fn cube_image(c: &MapCubemap) -> Image {
    let data: Vec<u8> = c.faces.concat();
    let mut image = Image::new(
        Extent3d {
            width: c.size,
            height: c.size,
            depth_or_array_layers: 6,
        },
        TextureDimension::D2,
        data,
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::RENDER_WORLD,
    );
    image.texture_view_descriptor = Some(bevy::render::render_resource::TextureViewDescriptor {
        dimension: Some(bevy::render::render_resource::TextureViewDimension::Cube),
        ..default()
    });
    image.sampler = ImageSampler::Descriptor(ImageSamplerDescriptor {
        address_mode_u: ImageAddressMode::ClampToEdge,
        address_mode_v: ImageAddressMode::ClampToEdge,
        address_mode_w: ImageAddressMode::ClampToEdge,
        mag_filter: ImageFilterMode::Linear,
        min_filter: ImageFilterMode::Linear,
        ..default()
    });
    image
}

/// A lighting atlas layer as a filterable half-float texture, clamped.
/// A lighting atlas layer in Source's LDR encoding (see
/// `SOURCE_LIGHTMAP_SCALE`): 8-bit sRGB, decoded by the sampler.
fn source_lightmap_image(rgb: &[[f32; 3]], width: u32, height: u32) -> Image {
    let mut data = Vec::with_capacity(rgb.len() * 4);
    for [r, g, b] in rgb {
        data.extend([source_ldr_texel(*r), source_ldr_texel(*g), source_ldr_texel(*b), 255]);
    }
    srgb8_image(data, width, height)
}

/// RGBA8 sRGB texels as a linearly filtered lightmap layer.
fn srgb8_image(data: Vec<u8>, width: u32, height: u32) -> Image {
    let mut image = Image::new(
        Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        data,
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::RENDER_WORLD,
    );
    image.sampler = ImageSampler::linear();
    image
}

/// The lightmap atlas as images: the plain copy (Bevy's `Lightmap` on
/// StandardMaterial parts), the world material's copy (in the game's own
/// encoding when `source_ldr`) and its directional pages.
struct LightmapLayers {
    plain: Image,
    world: Image,
    bumped: Option<[Image; 3]>,
}

impl LightmapLayers {
    fn build(rgb: &[[f32; 3]], bumped: Option<&[Vec<[f32; 3]>; 3]>, width: u32, height: u32, source_ldr: bool) -> Self {
        let world_layer = |rgb: &[[f32; 3]]| {
            if source_ldr {
                source_lightmap_image(rgb, width, height)
            } else {
                lightmap_image(rgb, width, height)
            }
        };
        let pages = bumped.map(|b| {
            if source_ldr {
                // The game encodes the three pages together, against the flat one.
                let mut texels: [Vec<u8>; 3] = std::array::from_fn(|_| Vec::with_capacity(rgb.len() * 4));
                for (i, flat) in rgb.iter().enumerate() {
                    let pages = source_ldr_bump_texels(*flat, [b[0][i], b[1][i], b[2][i]]);
                    for (t, [r, g, b]) in texels.iter_mut().zip(pages) {
                        t.extend([r, g, b, 255]);
                    }
                }
                texels.map(|t| srgb8_image(t, width, height))
            } else {
                std::array::from_fn(|i| world_layer(&b[i]))
            }
        });
        Self {
            plain: lightmap_image(rgb, width, height),
            world: world_layer(rgb),
            bumped: pages,
        }
    }
}

/// The lightmap images of a map with switchable light styles, and the
/// styles they show.
#[derive(Resource)]
struct SwitchableLightmaps {
    atlas: Arc<MapLightmap>,
    source_ldr: bool,
    plain: Option<Handle<Image>>,
    world: Option<Handle<Image>>,
    bumped: Option<[Handle<Image>; 3]>,
    /// Whether each of the atlas's styles is lit in the images now.
    applied: Vec<bool>,
}

/// Rebuild the lightmap images when switchable light styles change
/// (`LightStyles`): the atlas at map start, plus or minus each style
/// switched since (Source lights' TurnOn/TurnOff).
fn relight(
    styles: Option<Res<LightStyles>>,
    maps: Option<ResMut<SwitchableLightmaps>>,
    images: Option<ResMut<Assets<Image>>>,
    world_materials: Option<ResMut<Assets<world_material::WorldMaterial>>>,
    standard: Option<ResMut<Assets<StandardMaterial>>>,
) {
    let (Some(styles), Some(mut maps), Some(mut images)) = (styles, maps, images) else {
        return;
    };
    if !styles.is_changed() {
        return;
    }
    let l = maps.atlas.clone();
    let lit = |style: u8| {
        styles.styles.iter().find(|(s, _)| *s == style).map_or_else(
            || l.styles.iter().find(|s| s.style == style).is_some_and(|s| s.on),
            |(_, on)| *on,
        )
    };
    let want: Vec<bool> = l.styles.iter().map(|s| lit(s.style)).collect();
    if maps.applied == want {
        return;
    }
    maps.applied = want;
    let (rgb, bumped) = l.relit(&lit);
    let built = LightmapLayers::build(&rgb, bumped.as_ref(), l.width, l.height, maps.source_ldr);
    let mut put = |handle: &Option<Handle<Image>>, image: Image| {
        if let Some(h) = handle {
            let _ = images.insert(h.id(), image);
        }
    };
    put(&maps.plain, built.plain);
    put(&maps.world, built.world);
    if let (Some(handles), Some(pages)) = (&maps.bumped, built.bumped) {
        for (h, p) in handles.iter().zip(pages) {
            let _ = images.insert(h.id(), p);
        }
    }
    // Materials bound to the old images are prepared again.
    if let Some(mut m) = world_materials {
        let ids: Vec<_> = m.ids().collect();
        for id in ids {
            let _ = m.get_mut(id);
        }
    }
    if let Some(mut m) = standard {
        let ids: Vec<_> = m.ids().collect();
        for id in ids {
            let _ = m.get_mut(id);
        }
    }
    info!("lightmaps relit: {:?}", styles.styles);
}

fn lightmap_image(rgb: &[[f32; 3]], width: u32, height: u32) -> Image {
    let mut data = Vec::with_capacity(rgb.len() * 8);
    for [r, g, b] in rgb {
        for c in [*r, *g, *b, 1.0] {
            data.extend_from_slice(&half::f16::from_f32(c).to_le_bytes());
        }
    }
    let mut image = Image::new(
        Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        data,
        TextureFormat::Rgba16Float,
        RenderAssetUsages::RENDER_WORLD,
    );
    image.sampler = ImageSampler::linear();
    image
}

/// Render assets of a skinned model: meshes with joint attributes (when
/// it has a skeleton), materials, and inverse bind poses from the bones'
/// reference pose under `root`.
fn body_assets(
    model: &MapModel,
    bones: &[MapBone],
    root: Transform,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<PropMaterial>,
    bindposes: &mut Assets<bevy::mesh::skinning::SkinnedMeshInverseBindposes>,
    material: &dyn Fn(&MapMesh) -> PropMaterial,
) -> BodyAssets {
    let parts = model
        .meshes
        .iter()
        .map(|m| {
            let mut mesh = build_mesh(m, false);
            if m.joints.len() == m.positions.len() && !bones.is_empty() {
                mesh.insert_attribute(
                    Mesh::ATTRIBUTE_JOINT_INDEX,
                    bevy::mesh::VertexAttributeValues::Uint16x4(m.joints.clone()),
                );
                mesh.insert_attribute(Mesh::ATTRIBUTE_JOINT_WEIGHT, m.joint_weights.clone());
            }
            (meshes.add(mesh), materials.add(material(m)))
        })
        .collect();
    // Each bone's reference pose in body space, inverted.
    let mut global: Vec<Mat4> = Vec::with_capacity(bones.len());
    for b in bones {
        let local = Mat4::from_rotation_translation(b.rotation, b.position);
        let parent = b
            .parent
            .and_then(|p| global.get(p).copied())
            .unwrap_or(root.to_matrix());
        global.push(parent * local);
    }
    let inverse: Vec<Mat4> = global.iter().map(|m| m.inverse()).collect();
    BodyAssets {
        parts,
        bindposes: bindposes.add(bevy::mesh::skinning::SkinnedMeshInverseBindposes::from(inverse)),
        bones: bones.to_vec(),
        root,
    }
}

fn build_mesh(m: &MapMesh, with_lightmap: bool) -> Mesh {
    let mut mesh = Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::default());
    mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, m.positions.clone());
    mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, m.normals.clone());
    mesh.insert_attribute(Mesh::ATTRIBUTE_UV_0, m.uvs.clone());
    if with_lightmap {
        mesh.insert_attribute(Mesh::ATTRIBUTE_UV_1, m.lightmap_uvs.clone());
    }
    mesh.insert_indices(Indices::U32(m.indices.clone()));
    mesh
}

fn build_material(m: &MapMesh, textures: &[Handle<Image>], view: MapDebugView, light_scale: f32) -> StandardMaterial {
    let [r, g, b] = m.color;
    let lighting_only = matches!(view, MapDebugView::Lighting { .. });
    let scale = if let MapDebugView::Lighting { scale } = view {
        scale
    } else {
        1.0
    };
    let tint = m.tint.map_or(LinearRgba::WHITE, |[r, g, b]| LinearRgba::rgb(r, g, b));
    StandardMaterial {
        base_color: if m.texture.is_some() || lighting_only {
            tint.into()
        } else {
            {
                let c = Color::srgb_u8(r, g, b).to_linear();
                LinearRgba::rgb(c.red * tint.red, c.green * tint.green, c.blue * tint.blue).into()
            }
        },
        base_color_texture: m.texture.filter(|_| !lighting_only).map(|i| textures[i].clone()),
        unlit: view == MapDebugView::Albedo,
        perceptual_roughness: 0.95,
        reflectance: 0.2,
        alpha_mode: match m.alpha {
            MapAlpha::Opaque => AlphaMode::Opaque,
            MapAlpha::Mask(cutoff) => AlphaMode::Mask(cutoff),
            MapAlpha::Blend => AlphaMode::Blend,
            MapAlpha::Add => AlphaMode::Add,
        },
        double_sided: m.double_sided,
        cull_mode: if m.double_sided {
            None
        } else {
            Some(bevy::render::render_resource::Face::Back)
        },
        lightmap_exposure: LIGHTMAP_EXPOSURE * scale * light_scale,
        ..default()
    }
}

/// The material of a prop mesh lit by a light probe (`PropMaterial`),
/// without its cubemap; `skybox`: in the 3D skybox (no fog there).
fn lit_prop_material(
    m: &MapMesh,
    textures: &[Handle<Image>],
    data: &MapData,
    view: MapDebugView,
    skybox: bool,
) -> PropMaterial {
    let [r, g, b] = m.color;
    let lighting_only = matches!(view, MapDebugView::Lighting { .. });
    let tint = m.tint.map_or(Vec4::ONE, |[r, g, b]| Vec4::new(r, g, b, 1.0));
    PropMaterial {
        params: PropParams {
            base_color: if m.texture.is_some() || lighting_only {
                tint
            } else {
                Color::srgb_u8(r, g, b).to_linear().to_vec4() * tint
            },
            alpha_cutoff: if let MapAlpha::Mask(c) = m.alpha { c } else { 0.0 },
            fog_color: fog_color(data.fog.as_ref().filter(|_| view == MapDebugView::Normal && !skybox)),
            fog_range: fog_range(data.fog.as_ref()),
            translucent: m.alpha.shader_mode(),
            dynamic: if m.unlit || view != MapDebugView::Normal {
                0.0
            } else {
                1.0
            },
            ..default()
        },
        base: m.texture.filter(|_| !lighting_only).map(|i| textures[i].clone()),
        envmap: None,
        envmap_mask: None,
        alpha_mode: m.alpha.shader_alpha_mode(),
        double_sided: m.double_sided,
        cull_front: false,
    }
}

/// Make `material` reflect `cube` as `env` says (Source `$envmap`).
fn set_prop_envmap(material: &mut PropMaterial, env: &MapEnvmap, cube: Handle<Image>, textures: &[Handle<Image>]) {
    material.envmap = Some(cube);
    material.params.envmap = 1.0;
    material.params.envmap_mask = match env.mask {
        EnvmapMask::BaseAlphaInverted => 2.0,
        EnvmapMask::Texture(_) => 3.0,
        _ => 0.0,
    };
    if let EnvmapMask::Texture(t) = env.mask {
        material.envmap_mask = textures.get(t).cloned();
    }
    material.params.envmap_contrast = env.contrast;
    material.params.envmap_saturation = env.saturation;
    material.params.envmap_tint = Vec3::from_array(env.tint.map(gamma_to_linear)).extend(1.0);
}

/// A simulated physics prop: how players interact with it, its mass (kg)
/// and its model-space bounds.
#[derive(Component, Debug, Clone, Copy)]
pub struct PhysicsProp {
    pub push: PushAway,
    pub mass: f32,
    pub bounds: (Vec3, Vec3),
}

/// A prop entity's index in `MapData::props`.
#[derive(Component, Debug, Clone, Copy)]
pub struct PropIndex(pub usize);

/// The map entity a prop node was placed by (index into
/// `MapData::entities`), so logic can find it (a sound playing from it).
#[derive(Component, Debug, Clone, Copy)]
pub struct PropEntity(pub usize);

/// Where an entity prop was placed (its spawn transform), so a round
/// restart can put a moved physics prop back.
#[derive(Component, Debug, Clone, Copy)]
pub struct PropHome(pub Transform);

/// What it takes to redraw prop shadows when props move.
#[derive(Resource)]
struct ShadowState {
    data: Arc<MapData>,
    settings: MapShadows,
    receivers: shadows::Receivers,
    atlas: shadows::Atlas,
    atlas_image: Handle<Image>,
    material: Handle<shadows::ShadowMaterial>,
    cells: std::collections::HashMap<usize, shadows::Cell>,
    entities: std::collections::HashMap<usize, Entity>,
    root: Entity,
}

/// Redraw the shadows of physics props that moved (silhouette and mesh);
/// a prop the logic hid (broken, killed) hides its shadow until it shows
/// again (a round restart).
#[allow(clippy::type_complexity)]
fn update_prop_shadows(
    mut commands: Commands,
    state: Option<ResMut<ShadowState>>,
    moved: Query<(&PropIndex, &Transform), (With<PhysicsProp>, Changed<Transform>)>,
    hidden: Query<&PropIndex, Added<vis::LogicHidden>>,
    mut shown: RemovedComponents<vis::LogicHidden>,
    props: Query<&PropIndex, Without<vis::LogicHidden>>,
    shadow_vis: Query<Option<&vis::VisClusters>, With<PropShadow>>,
    mut images: ResMut<Assets<Image>>,
    mut meshes: ResMut<Assets<Mesh>>,
) {
    let Some(mut state) = state else {
        shown.clear();
        return;
    };
    let state = &mut *state;
    for index in &hidden {
        if let Some(e) = state.entities.get(&index.0) {
            commands.entity(*e).insert((vis::LogicHidden, Visibility::Hidden));
        }
    }
    for node in shown.read() {
        let Ok(index) = props.get(node) else { continue };
        if let Some(e) = state.entities.get(&index.0).copied() {
            let on = shadow_vis.get(e).ok().flatten().is_none_or(|v| v.potentially_visible);
            commands
                .entity(e)
                .remove::<vis::LogicHidden>()
                .insert(if on { Visibility::Inherited } else { Visibility::Hidden });
        }
    }
    let mut atlas_dirty = false;
    for (index, t) in &moved {
        let Some(cell) = state.cells.get(&index.0).copied() else {
            continue;
        };
        let mesh = shadows::rebuild(
            &state.data,
            &state.settings,
            &state.receivers,
            &mut state.atlas,
            &cell,
            t.translation,
            t.rotation,
        );
        atlas_dirty = true;
        match (mesh, state.entities.get(&index.0).copied()) {
            (Some(mesh), Some(e)) => {
                commands.entity(e).insert(Mesh3d(meshes.add(mesh)));
            }
            (Some(mesh), None) => {
                let e = commands
                    .spawn((
                        Name::new(format!("Shadow of prop {}", index.0)),
                        MapPart,
                        PropShadow { prop: index.0 },
                        Mesh3d(meshes.add(mesh)),
                        MeshMaterial3d(state.material.clone()),
                        bevy::light::NotShadowCaster,
                        Transform::default(),
                        ChildOf(state.root),
                    ))
                    .id();
                state.entities.insert(index.0, e);
            }
            (None, Some(e)) => {
                commands.entity(e).despawn();
                state.entities.remove(&index.0);
            }
            (None, None) => {}
        }
    }
    if atlas_dirty && let Some(mut image) = images.get_mut(&state.atlas_image) {
        image.data = Some(state.atlas.bytes());
    }
}

/// A prop's dynamic shadow on the world (index into `MapData::props`).
#[derive(Component, Debug)]
pub struct PropShadow {
    pub prop: usize,
}

/// A prop model's mesh (index into `MapModel::meshes`) drawn under the
/// prop's node.
#[derive(Component, Clone, Copy, Debug)]
struct PropMeshSlot {
    mesh: usize,
}

/// What a prop entity can switch to: its materials per skin family
/// (empty with one family), its body layout, and what it shows (skin,
/// body number).
#[derive(Component)]
struct PropVariants {
    skins: Vec<Vec<Handle<PropMaterial>>>,
    /// The body number it spawned with.
    body: i32,
    parts: Vec<u16>,
    mesh_bodies: Vec<Option<(u16, u16)>>,
    shown: (i32, i32),
}

/// An animated prop: what it plays, its joints by bone, the logic's last
/// word and the sequence it falls back to.
#[derive(Component)]
struct PropRig {
    animator: anim::Animator,
    joints: Vec<Entity>,
    seen: Option<PropSequence>,
    default: Option<usize>,
}

/// Inverse bind poses of a rig's bones in the reference pose under its
/// root.
fn rig_inverse_bindposes(rig: &MapRig) -> bevy::mesh::skinning::SkinnedMeshInverseBindposes {
    let mut global: Vec<Mat4> = Vec::with_capacity(rig.bones.len());
    for b in &rig.bones {
        let local = Mat4::from_rotation_translation(b.rotation, b.position);
        let parent = b
            .parent
            .and_then(|p| global.get(p).copied())
            .unwrap_or(rig.root.to_matrix());
        global.push(parent * local);
    }
    let inverse: Vec<Mat4> = global.iter().map(|m| m.inverse()).collect();
    bevy::mesh::skinning::SkinnedMeshInverseBindposes::from(inverse)
}

/// Prop entities show the skin and body group the logic gives them
/// (`PropLook`). A skin the model doesn't have leaves it as it is.
fn apply_prop_looks(
    mut props: Query<(&PropLook, &mut PropVariants, &Children), Changed<PropLook>>,
    mut slots: Query<(
        &PropMeshSlot,
        Option<&mut MeshMaterial3d<PropMaterial>>,
        &mut Visibility,
    )>,
) {
    for (look, mut v, children) in &mut props {
        // SetBodyGroup sets the whole body number (specs/source/prop_damage.md 8).
        let body = look.body_group.unwrap_or(v.body);
        let skin = if (look.skin.max(0) as usize) < v.skins.len() {
            look.skin
        } else {
            v.shown.0
        };
        if (skin, body) == v.shown {
            continue;
        }
        v.shown = (skin, body);
        for child in children {
            let Ok((slot, material, mut visibility)) = slots.get_mut(*child) else {
                continue;
            };
            if let (Some(mut material), Some(want)) =
                (material, v.skins.get(skin as usize).and_then(|s| s.get(slot.mesh)))
                && material.0 != *want
            {
                material.0 = want.clone();
            }
            let shown = v
                .mesh_bodies
                .get(slot.mesh)
                .copied()
                .flatten()
                .is_none_or(|(part, choice)| body_choice(&v.parts, body, part as usize) == choice);
            visibility.set_if_neq(if shown {
                Visibility::Inherited
            } else {
                Visibility::Hidden
            });
        }
    }
}

/// Animated props play what the logic asks (`PropSequence`,
/// specs/source/prop_damage.md 9): a new serial restarts the named
/// sequence (sequence 0 when the model lacks it), switching at once; a
/// new round starts the default one (else the first); a non-looping
/// sequence that ends starts the default again, if there is one.
fn animate_props(
    time: Res<Time>,
    mut props: Query<(&mut PropRig, Option<&PropSequence>)>,
    mut joints: Query<&mut Transform>,
) {
    let (now, dt) = (time.elapsed_secs_f64(), time.delta_secs());
    for (mut rig, seq) in &mut props {
        let rig = &mut *rig;
        if let Some(seq) = seq
            && rig.seen.as_ref() != Some(seq)
        {
            let old = rig.seen.replace(seq.clone());
            rig.default = Some(seq.default.as_str())
                .filter(|d| !d.is_empty())
                .and_then(|d| rig.animator.set.sequence(d));
            let new_round = old.as_ref().is_none_or(|o| o.round != seq.round);
            let old_serial = old.as_ref().and_then(|o| o.play.as_ref()).map(|p| p.1);
            match &seq.play {
                Some((name, serial)) if new_round || old_serial != Some(*serial) => {
                    let s = rig.animator.set.sequence(name).unwrap_or_else(|| {
                        warn!("prop: no sequence {name}");
                        0
                    });
                    rig.animator.restart(s, now);
                    rig.animator.fading.clear();
                }
                // No default: its first sequence (the model's idle).
                None if new_round => {
                    rig.animator.restart(rig.default.unwrap_or(0), now);
                    rig.animator.fading.clear();
                }
                _ => {}
            }
        }
        if rig.animator.main.is_none() {
            continue;
        }
        rig.animator.advance(dt, now);
        // A non-looping default restarts too, forever.
        if rig.animator.finished()
            && let Some(d) = rig.default
        {
            rig.animator.restart(d, now);
            rig.animator.fading.clear();
        }
        let pose = rig.animator.pose(now);
        for (joint, (q, p)) in rig.joints.iter().zip(pose) {
            if let Ok(mut t) = joints.get_mut(*joint) {
                t.rotation = q;
                t.translation = p;
            }
        }
    }
}

/// Sprites and dust volumes follow their entity (`EntityPart`): a sprite
/// is drawn while on; a dust volume while it exists (off, it only stops
/// spawning motes).
#[allow(clippy::type_complexity)]
fn switch_parts(
    parts: Query<
        (
            Entity,
            &EntityPart,
            Option<&vis::VisClusters>,
            Has<vis::LogicHidden>,
            Has<dust::DustEmitter>,
        ),
        Changed<EntityPart>,
    >,
    mut commands: Commands,
) {
    for (e, part, clusters, hidden, dust) in &parts {
        let shown = part.exists && (dust || part.on);
        if shown != hidden {
            continue;
        }
        if shown {
            let mut c = commands.entity(e);
            c.remove::<vis::LogicHidden>();
            if clusters.is_none_or(|c| c.potentially_visible) {
                c.insert(Visibility::Inherited);
            }
        } else {
            commands.entity(e).insert((vis::LogicHidden, Visibility::Hidden));
        }
    }
}

/// A prop's physics collider (sprite glows see through static props, as
/// the game's line test ignores them).
#[derive(Component)]
pub struct MapPropCollider;

/// A glow sprite: drawn over everything, faded by how much of its
/// occlusion proxy is visible.
#[derive(Component)]
pub(crate) struct GlowSprite {
    color: Vec4,
    proxy: f32,
}

/// Glow visibility (specs/cs_source/sprites_dust.md, 5a): the game measures
/// the visible fraction of a view-facing square (half-diagonal = proxy
/// size) pulled the proxy size toward the eye, with occlusion queries. We
/// approximate it with lines to the square's centre and corners, and fade
/// the glow by the fraction that get through. Static props don't occlude.
#[allow(clippy::type_complexity)]
fn glow_visibility(
    query: SpatialQuery,
    cameras: Query<(&GlobalTransform, &Camera), (With<Camera3d>, Without<SkyboxCamera>, Without<ViewModelCamera>, Without<water::WaterReflectionCamera>)>,
    ignored: Query<Entity, With<crate::core::Intent>>,
    mut glows: Query<
        (
            &GlobalTransform,
            &GlowSprite,
            &MeshMaterial3d<SpriteMaterial>,
            &mut Visibility,
            Option<&vis::VisClusters>,
        ),
        Without<vis::LogicHidden>,
    >,
    materials: Option<ResMut<Assets<SpriteMaterial>>>,
) {
    let (Some((eye, camera)), Some(mut materials)) = (cameras.iter().next(), materials) else {
        return;
    };
    let filter = SpatialQueryFilter::from_excluded_entities(ignored.iter());
    let from = eye.translation();
    let (right, up) = (eye.right().as_vec3(), eye.up().as_vec3());
    for (glow, sprite, material, mut visibility, clusters) in &mut glows {
        // Not potentially visible: hidden, no occlusion tests.
        if clusters.is_some_and(|c| !c.potentially_visible) {
            visibility.set_if_neq(Visibility::Hidden);
            continue;
        }
        let p = glow.translation();
        let centre = p + (from - p).normalize_or_zero() * sprite.proxy;
        let r = sprite.proxy / std::f32::consts::SQRT_2;
        let points = [
            centre,
            centre + (right + up) * r,
            centre + (right - up) * r,
            centre - (right + up) * r,
            centre - (right - up) * r,
        ];
        let seen = points
            .iter()
            .filter(|to| {
                Dir3::new(**to - from).map_or(true, |dir| {
                    query.cast_ray(from, dir, from.distance(**to), true, &filter).is_none()
                })
            })
            .count() as f32
            / points.len() as f32;
        // Times the fraction of the proxy's screen rectangle inside the
        // viewport (0 if any corner is behind the eye), as the game's
        // occlusion queries are clipped to the screen.
        let seen = seen * clip_fraction(camera, eye, &points[1..]);
        visibility.set_if_neq(if seen > 0.0 {
            Visibility::Inherited
        } else {
            Visibility::Hidden
        });
        // Only touch the material when the colour changes: a mutable
        // borrow alone re-prepares it for the GPU.
        let color = (sprite.color.truncate() * seen).extend(sprite.color.w);
        if materials.get(&material.0).is_some_and(|m| m.params.color != color)
            && let Some(mut m) = materials.get_mut(&material.0)
        {
            m.params.color = color;
        }
    }
}

/// How much of the screen rectangle around `corners` lies in the viewport.
fn clip_fraction(camera: &Camera, eye: &GlobalTransform, corners: &[Vec3]) -> f32 {
    let mut lo = Vec2::splat(f32::MAX);
    let mut hi = Vec2::splat(f32::MIN);
    for c in corners {
        let Some(ndc) = camera.world_to_ndc(eye, *c) else {
            return 0.0;
        };
        if ndc.z < 0.0 || ndc.z > 1.0 {
            return 0.0;
        }
        lo = lo.min(ndc.truncate());
        hi = hi.max(ndc.truncate());
    }
    let area = (hi - lo).max(Vec2::splat(1e-6));
    let inside = (hi.min(Vec2::ONE) - lo.max(Vec2::NEG_ONE)).max(Vec2::ZERO);
    (inside.x * inside.y / (area.x * area.y)).clamp(0.0, 1.0)
}

/// Fog color for shaders: linear RGB, w = 1 when fog is on.
fn fog_color(fog: Option<&MapFog>) -> Vec4 {
    fog.map_or(Vec4::ZERO, |f| {
        let c = Color::srgb(f.color[0], f.color[1], f.color[2]).to_linear();
        Vec4::new(c.red, c.green, c.blue, 1.0)
    })
}

/// Fog start, end (meters) and max density for shaders.
fn fog_range(fog: Option<&MapFog>) -> Vec4 {
    fog.map_or(Vec4::new(0.0, 1.0, 0.0, 0.0), |f| {
        Vec4::new(f.start, f.end.max(f.start + 0.01), f.max_density, 0.0)
    })
}

/// Share of a prop's surface that must lie on its convex hull for the hull
/// to stand in for it in exact (brush) collision. Crates and boxes with
/// shallow panel insets pass; arches and other concave props don't.
const CONVEX_SURFACE_SHARE: f32 = 0.75;
const MAX_HULL_PLANES: usize = 48;
/// How close (meters) a triangle must be to a hull face to count as on it.
const ON_HULL: f32 = 0.01;

/// A model's convex hull as planes (model space), if at least `share` of
/// its surface area lies on the hull.
fn convex_planes(model: &MapModel, share: f32) -> Option<Vec<(Vec3, f32)>> {
    let points: Vec<Vec3> = model
        .meshes
        .iter()
        .flat_map(|m| m.positions.iter().map(|p| Vec3::from(*p)))
        .collect();
    if points.len() < 4 {
        return None;
    }
    let (verts, tris) = avian3d::parry::transformation::convex_hull(&points);
    let mut planes: Vec<(Vec3, f32)> = Vec::new();
    for t in &tris {
        let [a, b, c] = t.map(|i| verts[i as usize]);
        let n = (b - a).cross(c - a);
        if n.length_squared() < 1e-12 {
            continue;
        }
        let n = n.normalize();
        let d = n.dot(a);
        if !planes.iter().any(|(m, e)| m.dot(n) > 0.9999 && (e - d).abs() < 1e-4) {
            planes.push((n, d));
        }
    }
    // Rounded props (many faces) stay meshes: little to gain, slow to place.
    if !(4..=MAX_HULL_PLANES).contains(&planes.len()) {
        return None;
    }
    let (mut on, mut total) = (0.0f32, 0.0f32);
    for m in &model.meshes {
        for t in m.indices.as_chunks::<3>().0 {
            let [a, b, c] = t.map(|i| Vec3::from(m.positions[i as usize]));
            let area = (b - a).cross(c - a).length() / 2.0;
            total += area;
            if planes
                .iter()
                .any(|(n, d)| [a, b, c].iter().all(|p| (n.dot(*p) - d).abs() < ON_HULL))
            {
                on += area;
            }
        }
    }
    (total > 0.0 && on >= share * total).then_some(planes)
}

/// Model-space planes placed in the world, with the bounding box's planes
/// added as bevels (so box sweeps stop at corners like Source's brushes).
fn place_brush(planes: &[(Vec3, f32)], translation: Vec3, rotation: Quat, surface: Option<String>) -> MapBrush {
    let mut world: Vec<(Vec3, f32)> = planes
        .iter()
        .map(|(n, d)| {
            let n2 = rotation * *n;
            (n2, d + n2.dot(translation))
        })
        .collect();
    // Corners: intersections of plane triples that lie inside all planes.
    let mut corners = Vec::new();
    for i in 0..world.len() {
        for j in i + 1..world.len() {
            for k in j + 1..world.len() {
                let ((n1, d1), (n2, d2), (n3, d3)) = (world[i], world[j], world[k]);
                let denom = n1.dot(n2.cross(n3));
                if denom.abs() < 1e-6 {
                    continue;
                }
                let p = (n2.cross(n3) * d1 + n3.cross(n1) * d2 + n1.cross(n2) * d3) / denom;
                if world.iter().all(|(n, d)| n.dot(p) <= d + 1e-3) {
                    corners.push(p);
                }
            }
        }
    }
    let min = corners.iter().fold(Vec3::splat(f32::MAX), |a, c| a.min(*c));
    let max = corners.iter().fold(Vec3::splat(f32::MIN), |a, c| a.max(*c));
    for (n, d) in MapBrush::from_box(min, max).planes {
        if !world.iter().any(|(m, e)| m.dot(n) > 0.9999 && (e - d).abs() < 1e-4) {
            world.push((n, d));
        }
    }
    MapBrush {
        planes: world,
        min,
        max,
        ladder: false,
        surface,
    }
}

/// The collision triangles (displacements) as loose items see them: each
/// triangle comes with its own corners, so they are merged and the mesh's
/// internal edges fixed. On the plain mesh a small box sliding over it (a
/// shot weapon) catches on every edge between two triangles as on a step
/// and stops dead. Only loose items use it: physics props resting on it
/// can sink through (a de_dust2 oil drum did), so they keep the plain one.
fn smooth_terrain(positions: Vec<Vec3>, indices: &[[u32; 3]]) -> Option<Collider> {
    let flags = TrimeshFlags::FIX_INTERNAL_EDGES
        | TrimeshFlags::DELETE_DEGENERATE_TRIANGLES
        | TrimeshFlags::DELETE_DUPLICATE_TRIANGLES;
    Collider::try_trimesh_with_config(positions, indices.to_vec(), flags)
        .inspect_err(|e| warn!("no smooth terrain for loose items: {e:?}"))
        .ok()
}

/// A model's collider in model space: its collision model's convex pieces,
/// else all its triangles.
fn model_collider(model: &MapModel) -> Option<Collider> {
    if let Some(c) = &model.collision {
        let parts: Vec<(Vec3, Quat, Collider)> = c
            .pieces
            .iter()
            .filter_map(|p| Collider::convex_hull(p.points.clone()))
            .map(|hull| (Vec3::ZERO, Quat::IDENTITY, hull))
            .collect();
        if !parts.is_empty() {
            return Some(Collider::compound(parts));
        }
    }
    let mut positions = Vec::new();
    let mut indices = Vec::new();
    for m in &model.meshes {
        let base = positions.len() as u32;
        positions.extend(m.positions.iter().map(|p| Vec3::from(*p)));
        indices.extend(
            m.indices
                .as_chunks::<3>()
                .0
                .iter()
                .map(|t| [base + t[0], base + t[1], base + t[2]]),
        );
    }
    (!indices.is_empty()).then(|| Collider::trimesh(positions, indices))
}

/// Give every 3D camera the map's sky.
#[allow(clippy::type_complexity)]
fn attach_sky(
    mut commands: Commands,
    sky: Option<Res<MapSkybox>>,
    sky_camera: Option<Res<SkyCameraInfo>>,
    cameras: Query<
        (Entity, Has<SkyboxCamera>),
        (With<Camera3d>, Without<bevy::light::Skybox>, Without<ViewModelCamera>),
    >,
) {
    let Some(sky) = sky else { return };
    for (cam, is_sky_camera) in &cameras {
        // With a 3D skybox, the 2D sky is drawn behind it by the sky camera.
        if sky_camera.is_some() && !is_sky_camera {
            continue;
        }
        commands.entity(cam).insert(bevy::light::Skybox {
            image: Some(sky.0.clone()),
            // Show the texture at its own brightness (cancel camera exposure).
            brightness: LIGHTMAP_EXPOSURE,
            ..default()
        });
    }
}

/// Six faces into one cube texture. Faces narrower than tall or shorter
/// than wide (Source sides are often half height) are stretched to square.
fn sky_image(sky: &MapSky, textures: &[MapTexture]) -> Image {
    let size = sky
        .faces
        .iter()
        .map(|(t, _)| textures[*t].width.max(textures[*t].height))
        .max()
        .unwrap_or(1);
    let mut data = Vec::with_capacity((size * size * 4 * 6) as usize);
    // Calibration hook for `refcmp skyconv`: each pixel encodes its own
    // face (blue) and position (red = u, green = v).
    let debug = std::env::var_os("MASHUP_SKY_DEBUG").is_some();
    for (face, (tex, turns)) in sky.faces.into_iter().enumerate() {
        if debug {
            for y in 0..size {
                for x in 0..size {
                    data.extend_from_slice(&[
                        (x * 255 / size) as u8,
                        (y * 255 / size) as u8,
                        face as u8 * 40 + 20,
                        255,
                    ]);
                }
            }
            continue;
        }
        let t = &textures[tex];
        for y in 0..size {
            for x in 0..size {
                // Rotate clockwise by `turns` quarter turns: sample the source
                // pixel that lands at (x, y).
                let (mut u, mut v) = (x, y);
                for _ in 0..turns % 4 {
                    (u, v) = (v, size - 1 - u);
                }
                if turns >= 4 {
                    u = size - 1 - u;
                }
                // Skip each face's outermost texel row and column: sky
                // textures leave them as borders (dust2's side faces end in
                // a black row) that the game never shows, while a cube map
                // blends them in at every seam.
                let inner = |p: u32, n: u32| 1 + (p as u64 * n.saturating_sub(2) as u64 / size as u64) as u32;
                let (sx, sy) = (inner(u, t.width).min(t.width - 1), inner(v, t.height).min(t.height - 1));
                let i = ((sy * t.width + sx) * 4) as usize;
                data.extend_from_slice(&t.rgba8[i..i + 4]);
            }
        }
    }
    let mut image = Image::new(
        Extent3d {
            width: size,
            height: size,
            depth_or_array_layers: 6,
        },
        TextureDimension::D2,
        data,
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::RENDER_WORLD,
    );
    image.texture_view_descriptor = Some(bevy::render::render_resource::TextureViewDescriptor {
        dimension: Some(bevy::render::render_resource::TextureViewDimension::Cube),
        ..default()
    });
    image.sampler = ImageSampler::linear();
    image
}

/// World brushes are also drawn by water reflections (props and characters
/// only when the reflection shows entities).
fn world_layer_of(skybox: bool) -> bevy::camera::visibility::RenderLayers {
    if skybox { layer_of(true) } else { water::world_layers() }
}

fn layer_of(skybox: bool) -> bevy::camera::visibility::RenderLayers {
    bevy::camera::visibility::RenderLayers::layer(if skybox { SKYBOX_LAYER } else { 0 })
}

/// Keep a sky camera behind each frame's main camera: at
/// `origin + eye / scale`, same rotation, projection and render target. The
/// main camera then draws the world over it without clearing.
#[allow(clippy::type_complexity)]
fn follow_sky_camera(
    mut commands: Commands,
    info: Option<Res<SkyCameraInfo>>,
    vis: Option<Res<SkyVis>>,
    has_3d_sky: Option<Res<ActiveMapHas3dSky>>,
    water_view: Option<Res<water::WaterView>>,
    main: Query<
        (
            &GlobalTransform,
            &Projection,
            Option<&bevy::camera::RenderTarget>,
            Option<&bevy::core_pipeline::tonemapping::Tonemapping>,
            Entity,
            &Camera,
        ),
        (With<Camera3d>, Without<SkyboxCamera>, Without<ViewModelCamera>, Without<water::WaterReflectionCamera>),
    >,
    mut sky: Query<
        (
            Entity,
            &mut Transform,
            &mut GlobalTransform,
            &mut Projection,
            &mut Camera,
            &mut bevy::camera::visibility::RenderLayers,
            Option<&bevy::camera::RenderTarget>,
            Option<&bevy::core_pipeline::tonemapping::Tonemapping>,
        ),
        With<SkyboxCamera>,
    >,
) {
    let Some(info) = info else { return };
    let Some((main_tf, projection, target, tonemapping, main_entity, main_camera)) =
        main.iter().find(|(.., c)| c.is_active)
    else {
        return;
    };
    let eye = main_tf.translation();
    let translation = info.0.origin + eye / info.0.scale;
    let rotation = main_tf.rotation();
    // Source: the sky is drawn only from leaves that see it; inside solid
    // the view clears to black (specs/cs_source/shadows_sky.md). Leaves
    // that see no sky show the previous frame in the game; black here.
    let leaf = vis.as_ref().map_or(LeafSky::Sky3d, |v| v.0.at(eye));
    // Under water the view clears to the water's fog colour and draws no
    // sky (specs/cs_source/water.md section 9).
    let under = water_view.as_ref().and_then(|w| w.clear);
    let sky_on = matches!(leaf, LeafSky::Sky2d | LeafSky::Sky3d) && under.is_none();
    let layer = if leaf == LeafSky::Sky3d && has_3d_sky.is_some() {
        SKYBOX_LAYER
    } else {
        EMPTY_LAYER
    };
    let clear = match (sky_on, under) {
        (true, _) => ClearColorConfig::None,
        (false, Some([r, g, b])) => ClearColorConfig::Custom(Color::srgb(r, g, b)),
        (false, None) => ClearColorConfig::Custom(Color::BLACK),
    };
    let same = match (main_camera.clear_color, clear) {
        (ClearColorConfig::None, ClearColorConfig::None) => true,
        (ClearColorConfig::Custom(a), ClearColorConfig::Custom(b)) => a == b,
        _ => false,
    };
    if !same {
        commands.entity(main_entity).insert(Camera {
            clear_color: clear,
            ..main_camera.clone()
        });
    }
    match sky.single_mut() {
        Ok((entity, mut tf, mut global, mut proj, mut camera, mut layers, sky_target, sky_tonemapping)) => {
            if camera.is_active != sky_on {
                camera.is_active = sky_on;
            }
            if !layers.intersects(&bevy::camera::visibility::RenderLayers::layer(layer)) {
                *layers = bevy::camera::visibility::RenderLayers::layer(layer);
            }
            tf.translation = translation;
            tf.rotation = rotation;
            // Propagation has run: set the global transform too (no parent).
            *global = GlobalTransform::from(*tf);
            // Only real changes: a changed projection, target or tonemapping
            // makes Bevy redo the camera's setup.
            if let (Projection::Perspective(p), Projection::Perspective(main_p)) = (&*proj, projection)
                && (p.fov != main_p.fov || p.aspect_ratio != main_p.aspect_ratio)
                && let Projection::Perspective(p) = &mut *proj
            {
                p.fov = main_p.fov;
                p.aspect_ratio = main_p.aspect_ratio;
            }
            let mut e = commands.entity(entity);
            if let Some(t) = target
                && sky_target.and_then(|c| c.normalize(None)) != t.normalize(None)
            {
                e.insert(t.clone());
            }
            if let Some(t) = tonemapping
                && sky_tonemapping != Some(t)
            {
                e.insert(*t);
            }
        }
        Err(_) => {
            let mut e = commands.spawn((
                Name::new("Sky camera"),
                SkyboxCamera,
                MapPart,
                Camera3d::default(),
                Camera {
                    order: main_camera.order - 1,
                    ..default()
                },
                Projection::Perspective(PerspectiveProjection {
                    near: 0.01,
                    ..default()
                }),
                Transform::from_translation(translation).with_rotation(rotation),
                bevy::camera::visibility::RenderLayers::layer(layer),
            ));
            if let Some(fog) = &info.0.fog {
                let [r, g, b] = fog.color;
                e.insert(bevy::pbr::DistanceFog {
                    color: Color::srgb(r, g, b),
                    falloff: bevy::pbr::FogFalloff::Linear {
                        start: fog.start / info.0.scale,
                        end: fog.end / info.0.scale,
                    },
                    ..default()
                });
            }
        }
    }
}

#[cfg(test)]
mod switch_tests {
    use super::*;

    #[test]
    fn body_numbers_pick_one_choice_per_part() {
        // Parts with 1, 3 and 2 choices: body = c0 + 1 * c1 + 3 * c2.
        let parts = [1, 3, 2];
        assert_eq!(body_choice(&parts, 5, 1), 2);
        assert_eq!(body_choice(&parts, 5, 2), 1);
    }

    #[test]
    fn switched_parts_hide_and_show() {
        let mut app = App::new();
        app.add_systems(Update, switch_parts);
        let sprite = app
            .world_mut()
            .spawn((
                EntityPart {
                    entity: 3,
                    on: true,
                    exists: true,
                },
                Visibility::Inherited,
            ))
            .id();
        app.update();
        assert!(!app.world().entity(sprite).contains::<vis::LogicHidden>());
        app.world_mut().get_mut::<EntityPart>(sprite).unwrap().on = false;
        app.update();
        assert!(app.world().entity(sprite).contains::<vis::LogicHidden>());
        assert_eq!(app.world().get::<Visibility>(sprite), Some(&Visibility::Hidden));
        app.world_mut().get_mut::<EntityPart>(sprite).unwrap().on = true;
        app.update();
        assert!(!app.world().entity(sprite).contains::<vis::LogicHidden>());
        assert_eq!(app.world().get::<Visibility>(sprite), Some(&Visibility::Inherited));
    }

    #[test]
    fn light_styles_add_and_remove_their_share() {
        let l = MapLightmap {
            width: 2,
            height: 1,
            rgb: vec![[1.0; 3], [0.5; 3]],
            bumped: None,
            styles: vec![
                MapLightStyle {
                    style: 32,
                    on: true,
                    texels: vec![0],
                    rgb: vec![[0.75; 3]],
                    bumped: None,
                },
                MapLightStyle {
                    style: 33,
                    on: false,
                    texels: vec![1],
                    rgb: vec![[0.25; 3]],
                    bumped: None,
                },
            ],
        };
        assert_eq!(l.relit(&|s| s == 32).0, l.rgb);
        assert_eq!(l.relit(&|s| s == 33).0, vec![[0.25; 3], [0.75; 3]]);
    }
}
