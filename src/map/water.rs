//! Water surfaces for any game (Source's Water shader is the model,
//! specs/cs_source/water.md): refraction of what lies below (the main view's
//! opaque colour and depth, fogged by the water's height fog), an optional
//! planar reflection rendered by a mirrored camera, a cheap cubemap
//! reflection blended in by distance, range fog under water and the bottom
//! material seen from below. Games fill `MapWaterMaterial`; everything here
//! is game-independent. Units: meters, Y up.

use bevy::{
    asset::{RenderAssetUsages, embedded_asset},
    camera::{RenderTarget, visibility::RenderLayers},
    prelude::*,
    reflect::TypePath,
    render::render_resource::{AsBindGroup, Extent3d, TextureDimension, TextureFormat},
    shader::ShaderRef,
};

/// Render layer of water surfaces: only main views draw them (never the
/// reflection camera). Main cameras get it while the map has water.
pub const WATER_LAYER: usize = 28;
/// Render layer of world brushes as the reflection sees them (Source's
/// "reflect world": no models). Entities stay on layer 0.
pub const REFLECT_LAYER: usize = 27;

/// How a water surface looks (Source Water material parameters after the
/// game's defaults and quirks are applied).
#[derive(Clone, Debug, PartialEq)]
pub struct MapWaterMaterial {
    /// The game's material name.
    pub name: String,
    /// The normal map's frames (indices into `MapData::textures`, linear),
    /// all the same size, shown at `frame_rate` per second (hard switch).
    pub normal_frames: Vec<usize>,
    pub frame_rate: f32,
    /// The first normal layer's scroll: texture repeats per second (the
    /// translation wraps to 0..1).
    pub bump_scroll: Vec2,
    /// Three-layer normals when `scroll1.x` is not zero (spec section 4).
    pub scroll1: Vec2,
    pub scroll2: Vec2,
    /// Shows what is below (refraction), distorted by `refract_amount`
    /// (screen uv per unit of normal xy).
    pub refract: bool,
    pub refract_amount: f32,
    /// Planar reflection (a mirrored camera), distorted by `reflect_amount`.
    pub reflect: bool,
    /// The reflection also shows models (props, characters), not just the
    /// world.
    pub reflect_entities: bool,
    pub reflect_amount: f32,
    /// Gamma 0..1.
    pub reflect_tint: [f32; 3],
    /// Water fog colour, gamma 0..1.
    pub fog_color: [f32; 3],
    /// Water fog range, meters.
    pub fog_start: f32,
    pub fog_end: f32,
    /// A top surface (fog from depth below); false: seen from below.
    pub above_water: bool,
    /// Never refraction or reflection: only the cubemap, opaque.
    pub force_cheap: bool,
    /// Planar reflection allowed whatever `WaterSettings::force_expensive`
    /// says (Source's `$forceexpensive`, on unless the material turns it
    /// off).
    pub force_expensive: bool,
    /// Cubemap reflection (the cheap pass), when the surface has one.
    pub envmap: Option<WaterEnvmap>,
    /// The cheap pass's distance ramp, meters: refraction near, cubemap
    /// from `cheap_end` on.
    pub cheap_start: f32,
    pub cheap_end: f32,
    /// None: Fresnel weights the cubemap; Some(f): a fixed weight.
    pub fixed_reflect_weight: Option<f32>,
    /// What the surface looks like from below (the game puts it on the
    /// downward faces).
    pub bottom: Option<Box<MapWaterMaterial>>,
    /// A full-screen warp drawn while the eye is in this water, after the
    /// view models.
    pub underwater_overlay: Option<MapScreenWarp>,
}

/// A full-screen refraction of the finished frame (Source's Refract shader
/// as an overlay): the screen sampled at an offset from a scrolling normal
/// map, tinted.
#[derive(Clone, Debug, PartialEq)]
pub struct MapScreenWarp {
    /// The game's material name.
    pub name: String,
    /// The normal map's frames (indices into `MapData::textures`, linear),
    /// shown at `frame_rate` per second.
    pub normal_frames: Vec<usize>,
    pub frame_rate: f32,
    /// Screen uv offset per unit of normal xy.
    pub refract_amount: f32,
    /// Multiplies the refracted colour (gamma 0..1).
    pub tint: [f32; 3],
    /// Normal-map repeats across the screen (u, v), and its scroll in
    /// repeats per second.
    pub scale: Vec2,
    pub scroll: Vec2,
}

impl Default for MapWaterMaterial {
    fn default() -> Self {
        Self {
            name: String::new(),
            normal_frames: Vec::new(),
            frame_rate: 0.0,
            bump_scroll: Vec2::ZERO,
            scroll1: Vec2::ZERO,
            scroll2: Vec2::ZERO,
            refract: true,
            refract_amount: 0.0,
            reflect: false,
            reflect_entities: false,
            reflect_amount: 0.8,
            reflect_tint: [1.0; 3],
            fog_color: [1.0, 0.0, 0.0],
            fog_start: 0.0,
            fog_end: 0.0,
            above_water: true,
            force_cheap: false,
            force_expensive: true,
            envmap: None,
            cheap_start: 0.0,
            cheap_end: 0.0,
            fixed_reflect_weight: None,
            bottom: None,
            underwater_overlay: None,
        }
    }
}

/// Which cubemap the cheap pass reflects.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum WaterEnvmap {
    /// Index into `MapData::cubemaps`.
    Baked(usize),
    /// The cubemap sample nearest to the surface.
    Nearest,
}

// --- The spec's maths, for the shader's reference and for tests. ---

/// Linear from a material colour channel (pure 2.2 power).
pub fn gamma_to_linear(c: f32) -> f32 {
    c.max(0.0).powf(2.2)
}

/// Fresnel (spec section 6.4): (1 - clamp(V.N))^5.
pub fn fresnel(v_dot_n: f32) -> f32 {
    (1.0 - v_dot_n.clamp(0.0, 1.0)).powi(5)
}

/// Shore fade: no reflection where the refraction's fog alpha is below
/// 0.05, all of it from 0.10.
pub fn shore_fade(a: f32) -> f32 {
    ((a - 0.05) * 20.0).clamp(0.0, 1.0)
}

/// The cheap pass's distance ramp (spec section 7).
pub fn cheap_ramp(d: f32, start: f32, end: f32) -> f32 {
    let range = end - start;
    if range <= 0.0 {
        return if d >= end { 1.0 } else { 0.0 };
    }
    (d / range - start / range).clamp(0.0, 1.0)
}

/// The water's height fog on a point below the surface (spec section 8):
/// `fog_plane` is the surface plus the 2-unit fudge, `depth` the point's
/// view depth, `k` one over the fog range. Same units throughout.
pub fn height_fog(eye_height: f32, point_height: f32, fog_plane: f32, depth: f32, k: f32) -> f32 {
    let below = fog_plane - point_height;
    let path = eye_height - point_height;
    let h = if path > 0.0 {
        (below / path).clamp(0.0, 1.0)
    } else {
        1.0
    };
    (h * depth * k).clamp(0.0, 1.0)
}

/// The scroll proxy's translation at time `t` (wraps into 0..1).
pub fn scroll_translation(per_second: Vec2, t: f32) -> Vec2 {
    let v = per_second * t;
    v - v.floor()
}

/// The animated normal map's frame at time `t`.
pub fn normal_frame(t: f32, rate: f32, count: usize) -> usize {
    if count == 0 {
        return 0;
    }
    ((rate * t).floor().max(0.0) as usize) % count
}

/// The second and third normal layers' coordinates (spec section 4).
pub fn layer_uvs(uv: Vec2, t: f32, scroll1: Vec2, scroll2: Vec2) -> (Vec2, Vec2) {
    (
        Vec2::new(0.1 * (uv.x + uv.y), 0.1 * (uv.y - uv.x)) + t * scroll1,
        Vec2::new(0.45 * uv.y, 0.45 * uv.x) + t * scroll2,
    )
}

/// The reflection camera (spec section 3): the eye mirrored across the
/// water plane at `height`, upright (forward mirrored, up kept up, so pitch
/// and roll flip). Its image is the mirror image upside down; the shader
/// samples it with v = (1 + y_ndc) / 2.
pub fn reflection_transform(eye: &GlobalTransform, height: f32) -> Transform {
    let mirror = |v: Vec3| Vec3::new(v.x, -v.y, v.z);
    let p = eye.translation();
    let right = mirror(*eye.right());
    let up = -mirror(*eye.up());
    let back = mirror(*eye.back());
    Transform {
        translation: Vec3::new(p.x, 2.0 * height - p.y, p.z),
        rotation: Quat::from_mat3(&Mat3::from_cols(right, up, back)).normalize(),
        scale: Vec3::ONE,
    }
}

/// What a water surface renders this frame (spec section 1).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WaterMode {
    pub reflect: bool,
    pub refract: bool,
}

/// The water console settings (Source's cvars; games register them).
#[derive(Resource, Clone, Copy, Debug, PartialEq)]
pub struct WaterSettings {
    /// `r_waterforceexpensive`: planar reflection even for materials that
    /// turn `$forceexpensive` off.
    pub force_expensive: u8,
    /// `r_waterforcereflectentities`: the reflection shows models too.
    pub reflect_entities: u8,
    /// `r_WaterDrawReflection`, `r_WaterDrawRefraction`: 0 turns the view
    /// off.
    pub draw_reflection: u8,
    pub draw_refraction: u8,
    /// `mat_drawwater`: 0 hides water surfaces.
    pub draw_water: u8,
}

impl Default for WaterSettings {
    /// CS:S's at DX9 ("Reflect world").
    fn default() -> Self {
        Self {
            force_expensive: 1,
            reflect_entities: 0,
            draw_reflection: 1,
            draw_refraction: 1,
            draw_water: 1,
        }
    }
}

/// The mode for a surface `distance` from the eye, at default settings.
pub fn water_mode(m: &MapWaterMaterial, distance: f32) -> WaterMode {
    surface_mode(m, &WaterSettings::default(), distance)
}

/// The mode for a surface `distance` from the eye (spec section 1).
pub fn surface_mode(m: &MapWaterMaterial, settings: &WaterSettings, distance: f32) -> WaterMode {
    let none = WaterMode {
        reflect: false,
        refract: false,
    };
    if m.force_cheap {
        return none;
    }
    let expensive = settings.force_expensive != 0 || m.force_expensive;
    let reflect = expensive && settings.draw_reflection != 0 && m.reflect;
    if distance >= m.cheap_end && !reflect {
        return none;
    }
    WaterMode {
        reflect,
        refract: settings.draw_refraction != 0 && m.refract,
    }
}

/// Whether the reflection shows models (props, characters) too.
pub fn reflects_entities(m: &MapWaterMaterial, settings: &WaterSettings) -> bool {
    settings.reflect_entities != 0 || m.reflect_entities
}

/// Margin of the near-plane-crosses-water test (Source's 7 units),
/// meters.
pub const INTERSECT_FUDGE: f32 = 7.0 * 0.0254;

/// The camera's near-plane rectangle, world space.
pub fn near_plane_corners(eye: &GlobalTransform, projection: &PerspectiveProjection) -> [Vec3; 4] {
    let half_h = projection.near * (projection.fov / 2.0).tan();
    let half_w = half_h * projection.aspect_ratio;
    let centre = eye.translation() + eye.forward() * projection.near;
    let (r, u) = (eye.right() * half_w, eye.up() * half_h);
    [centre - r - u, centre - r + u, centre + r + u, centre + r - u]
}

/// Whether the near plane crosses the water surface at `height` of a
/// volume spanning `min`..`max` (spec section 3, "Intersection view"):
/// the rectangle comes within the fudge of the surface from above and
/// below, and its box (grown by the fudge) touches the volume.
pub fn near_plane_crosses(corners: &[Vec3; 4], height: f32, min: Vec3, max: Vec3) -> bool {
    let lo = corners.iter().fold(Vec3::MAX, |a, c| a.min(*c));
    let hi = corners.iter().fold(Vec3::MIN, |a, c| a.max(*c));
    let f = INTERSECT_FUDGE;
    hi.y >= height - f
        && lo.y <= height + f
        && hi.x + f >= min.x
        && lo.x - f <= max.x
        && hi.z + f >= min.z
        && lo.z - f <= max.z
        && lo.y - f <= max.y
        && hi.y + f >= min.y
}

// --- Rendering ---

#[derive(Clone, Copy, Debug, Default, bevy::render::render_resource::ShaderType)]
pub struct WaterParams {
    /// Water fog: linear rgb, and gamma rgb (the cheap pass's colour space).
    pub fog_linear: Vec4,
    pub fog_gamma: Vec4,
    /// Reflection tint: linear (pass 1) and raw (pass 2).
    pub reflect_tint_linear: Vec4,
    pub reflect_tint_raw: Vec4,
    /// The scene's range fog on the surface: linear colour (w = 1 when on),
    /// gamma colour, and start, end (meters), max density.
    pub scene_fog_linear: Vec4,
    pub scene_fog_gamma: Vec4,
    pub scene_fog_range: Vec4,
    /// First layer scroll (xy), frame rate (z), frame count (w).
    pub bump: Vec4,
    /// Three-layer scrolls: scroll1 (xy), scroll2 (zw).
    pub scrolls: Vec4,
    pub refract_amount: f32,
    pub reflect_amount: f32,
    pub fog_start: f32,
    pub fog_end: f32,
    pub cheap_start: f32,
    pub cheap_end: f32,
    /// 1: top surface; 0: seen from below.
    pub above_water: f32,
    pub refract: f32,
    pub reflect: f32,
    /// 1 when the cubemap is blended over (the cheap pass).
    pub cheap_pass: f32,
    pub force_cheap: f32,
    /// Fresnel replacement for the cheap pass; negative: use Fresnel.
    pub fixed_weight: f32,
    /// 1 while the surface is beyond its cheap distance (no refraction this
    /// frame in the original: the fog alpha counts as 1).
    pub cheap_mode: f32,
    /// The 2-unit clip/fog fudge, meters.
    pub fudge: f32,
    /// 1: `envmap` is the sky (Bevy skybox axes), not a baked cubemap.
    pub sky_env: f32,
    /// The height (meters) of the surface whose refraction reads what lies
    /// below already fogged by the water's height fog (the near plane
    /// crosses it); far below the map when none.
    pub prefog_plane: f32,
    /// 1: the refraction texture holds only its clear here (the fog
    /// colour, fully fogged): water in the 3D skybox.
    pub refract_fog_only: f32,
}

#[derive(Asset, TypePath, AsBindGroup, Debug, Clone)]
pub struct WaterMaterial {
    #[uniform(0)]
    pub params: WaterParams,
    /// The normal map's frames as array layers (linear).
    #[texture(1, dimension = "2d_array")]
    #[sampler(2)]
    pub normal: Option<Handle<Image>>,
    /// The planar reflection (the reflection camera's target).
    #[texture(3)]
    #[sampler(4)]
    pub reflection: Option<Handle<Image>>,
    /// Cube texture sampled with Source-frame (Z-up) directions.
    #[texture(5, dimension = "cube")]
    #[sampler(6)]
    pub envmap: Option<Handle<Image>>,
}

impl Material for WaterMaterial {
    fn fragment_shader() -> ShaderRef {
        "embedded://mashup/map/water.wgsl".into()
    }

    /// Opaque, in the transmissive phase: the surface reads the opaque
    /// scene behind it (refraction) and writes its own colour.
    fn alpha_mode(&self) -> AlphaMode {
        AlphaMode::Opaque
    }

    fn reads_view_transmission_texture(&self) -> bool {
        true
    }

    fn enable_prepass() -> bool {
        false
    }

    fn enable_shadows() -> bool {
        false
    }
}

/// Registers the water material. Needs rendering.
pub struct WaterMaterialPlugin;

impl Plugin for WaterMaterialPlugin {
    fn build(&self, app: &mut App) {
        embedded_asset!(app, "water.wgsl");
        embedded_asset!(app, "screen_warp.wgsl");
        app.add_plugins((
            MaterialPlugin::<WaterMaterial>::default(),
            MaterialPlugin::<ScreenWarpMaterial>::default(),
        ))
        .init_resource::<WaterSettings>()
        .add_systems(Update, (prepare_main_cameras, spawn_overlays))
        .add_systems(
            PostUpdate,
            // After propagation (this frame's eye), before cameras
            // compute their projections and frusta.
            (update_water, underwater_fog, show_overlays)
                .chain()
                .after(bevy::transform::TransformSystems::Propagate)
                .before(bevy::camera::CameraUpdateSystems),
        );
    }
}

/// A spawned water surface (one per water material).
#[derive(Component, Clone, Debug)]
pub struct WaterSurface {
    /// Index into the map's water materials.
    pub material: usize,
    /// The surface's bounds (meters).
    pub min: Vec3,
    pub max: Vec3,
    /// The surface seen from below (the bottom material).
    pub below: bool,
    /// Part of the 3D skybox (drawn by the sky camera, cubemap only).
    pub skybox: bool,
}

/// The reflection camera.
#[derive(Component)]
pub struct WaterReflectionCamera;

/// The loaded map's water: materials and the shared reflection target.
#[derive(Resource, Clone)]
pub struct MapWaterRender {
    pub materials: Vec<MapWaterMaterial>,
    pub reflection: Option<Handle<Image>>,
    /// The scene fog's linear colour and range as the surfaces were built
    /// (restored on leaving the water).
    pub scene_fog: (Vec4, Vec4, Vec4),
    /// Screen warps still to spawn: the top material whose water shows
    /// it, the warp and its normal-map frames.
    pub overlays: Vec<(usize, MapScreenWarp, Handle<Image>)>,
}

/// The reflection's render target size (spec: 1024 x 1024, drawn at the
/// main view's aspect and stretched back by the screen-space lookup).
pub const REFLECTION_SIZE: u32 = 1024;

/// A render target for the reflection camera.
pub fn reflection_image() -> Image {
    let mut image = Image::new_target_texture(REFLECTION_SIZE, REFLECTION_SIZE, TextureFormat::Rgba8UnormSrgb, None);
    image.asset_usage = RenderAssetUsages::RENDER_WORLD;
    image.sampler = bevy::image::ImageSampler::linear();
    image
}

/// Normal-map frames (each already a full Image with mips) as one array
/// texture.
pub fn frames_image(frames: &[Image]) -> Option<Image> {
    let first = frames.first()?;
    let size = first.texture_descriptor.size;
    let levels = first.texture_descriptor.mip_level_count;
    let mut data = Vec::new();
    for f in frames {
        if f.texture_descriptor.size != size || f.texture_descriptor.mip_level_count != levels {
            return None;
        }
        data.extend_from_slice(f.data.as_ref()?);
    }
    let mut image = Image::new_uninit(
        Extent3d {
            width: size.width,
            height: size.height,
            depth_or_array_layers: frames.len() as u32,
        },
        TextureDimension::D2,
        first.texture_descriptor.format,
        RenderAssetUsages::RENDER_WORLD,
    );
    image.data = Some(data);
    image.texture_descriptor.mip_level_count = levels;
    image.sampler = first.sampler.clone();
    image.texture_view_descriptor = Some(bevy::render::render_resource::TextureViewDescriptor {
        dimension: Some(bevy::render::render_resource::TextureViewDimension::D2Array),
        ..default()
    });
    Some(image)
}

/// The layers a world brush mesh is drawn on (main views and the
/// reflection).
pub fn world_layers() -> RenderLayers {
    RenderLayers::from_layers(&[0, REFLECT_LAYER])
}

/// A spawned reflection camera's starting components.
pub fn reflection_camera(target: Handle<Image>, order: isize, entities: bool) -> impl Bundle {
    (
        Name::new("Water reflection camera"),
        WaterReflectionCamera,
        Camera3d::default(),
        Camera { order, ..default() },
        RenderTarget::Image(target.into()),
        Projection::Perspective(PerspectiveProjection::default()),
        Transform::default(),
        if entities {
            world_layers()
        } else {
            RenderLayers::layer(REFLECT_LAYER)
        },
    )
}

/// A projection that keeps the aspect ratio it is given (the main view's)
/// whatever the render target's shape: the reflection is drawn at the main
/// view's aspect into a square texture and stretched back on lookup.
#[derive(Clone, Debug, Default)]
pub struct ReflectionProjection(pub PerspectiveProjection);

impl bevy::camera::CameraProjection for ReflectionProjection {
    fn get_clip_from_view(&self) -> Mat4 {
        self.0.get_clip_from_view()
    }

    fn get_clip_from_view_for_sub(&self, sub_view: &bevy::camera::SubCameraView) -> Mat4 {
        self.0.get_clip_from_view_for_sub(sub_view)
    }

    fn update(&mut self, _width: f32, _height: f32) {}

    fn far(&self) -> f32 {
        self.0.far()
    }

    fn get_frustum_corners(&self, z_near: f32, z_far: f32) -> [bevy::math::Vec3A; 8] {
        self.0.get_frustum_corners(z_near, z_far)
    }
}

/// Uniforms for one water material.
fn params(m: &MapWaterMaterial, frames: usize, fog: Option<&super::MapFog>) -> WaterParams {
    let lin = |c: [f32; 3]| Vec4::new(gamma_to_linear(c[0]), gamma_to_linear(c[1]), gamma_to_linear(c[2]), 1.0);
    let raw = |c: [f32; 3]| Vec4::new(c[0], c[1], c[2], 1.0);
    let flag = |b: bool| if b { 1.0 } else { 0.0 };
    WaterParams {
        fog_linear: lin(m.fog_color),
        fog_gamma: raw(m.fog_color),
        reflect_tint_linear: lin(m.reflect_tint),
        reflect_tint_raw: raw(m.reflect_tint),
        scene_fog_linear: super::fog_color(fog),
        scene_fog_gamma: fog.map_or(Vec4::ZERO, |f| raw(f.color)),
        scene_fog_range: super::fog_range(fog),
        bump: Vec4::new(m.bump_scroll.x, m.bump_scroll.y, m.frame_rate, frames.max(1) as f32),
        scrolls: Vec4::new(m.scroll1.x, m.scroll1.y, m.scroll2.x, m.scroll2.y),
        refract_amount: m.refract_amount,
        reflect_amount: m.reflect_amount,
        fog_start: m.fog_start,
        fog_end: m.fog_end,
        cheap_start: m.cheap_start,
        cheap_end: m.cheap_end,
        above_water: flag(m.above_water),
        refract: flag(m.refract && !m.force_cheap),
        reflect: flag(m.reflect && !m.force_cheap),
        // Pass 2 only without a planar reflection, with a cubemap.
        cheap_pass: flag(!m.reflect && m.envmap.is_some()),
        force_cheap: flag(m.force_cheap),
        fixed_weight: m.fixed_reflect_weight.unwrap_or(-1.0),
        cheap_mode: 0.0,
        fudge: 2.0 * 0.0254,
        sky_env: 0.0,
        prefog_plane: NO_PREFOG,
        refract_fog_only: 0.0,
    }
}

/// Spawns the map's water surfaces (tops, and the downward faces drawn
/// with their bottom material) and, when one reflects, the reflection camera.
#[allow(clippy::too_many_arguments)]
pub(super) fn spawn_surfaces(
    commands: &mut Commands,
    data: &super::MapData,
    root: Entity,
    meshes: &mut Assets<Mesh>,
    images: &mut Assets<Image>,
    materials: &mut Assets<WaterMaterial>,
    cubemaps: &[Handle<Image>],
    sky: Option<&Handle<Image>>,
) {
    if data.water_materials.is_empty() {
        return;
    }
    let reflects = data.water_materials.iter().any(|m| m.reflect && !m.force_cheap);
    let reflection = reflects.then(|| images.add(reflection_image()));
    let mut normal_images: std::collections::HashMap<Vec<usize>, Handle<Image>> = Default::default();
    for m in data.meshes.iter() {
        let Some(index) = m.water else { continue };
        let top = &data.water_materials[index];
        let (mut min, mut max) = (Vec3::splat(f32::MAX), Vec3::splat(f32::MIN));
        for p in &m.positions {
            min = min.min(Vec3::from(*p));
            max = max.max(Vec3::from(*p));
        }
        let centre = (min + max) / 2.0;
        info!(
            "water {}: {min:.2}..{max:.2} m, refract {}, reflect {}, cubemap {}",
            top.name,
            top.refract,
            top.reflect,
            top.envmap.is_some()
        );
        let nearest = data
            .cubemap_samples
            .iter()
            .min_by(|a, b| a.0.distance_squared(centre).total_cmp(&b.0.distance_squared(centre)))
            .map(|(_, c)| *c);
        // The map has the downward faces itself (with the bottom material,
        // which the game sets per surface: `MapWaterMaterial::bottom`).
        // Only the horizontal faces: the water brushes' side faces (strips
        // at the volume's edges, de_port's wall at the world's edge) are
        // not drawn as water.
        let mut flat = m.clone();
        let facing = |i: u32| {
            let y = m.normals[i as usize][1];
            if top.above_water { y > 0.7 } else { y < -0.7 }
        };
        flat.indices = m
            .indices
            .chunks(3)
            .filter(|t| t.len() == 3 && facing(t[0]))
            .flatten()
            .copied()
            .collect();
        if flat.indices.is_empty() {
            continue;
        }
        {
            let (w, mesh, below) = (top, &flat, !top.above_water);
            let envmap = match w.envmap {
                Some(WaterEnvmap::Baked(c)) => Some(c),
                Some(WaterEnvmap::Nearest) => nearest,
                None => None,
            };
            let mut p = params(w, w.normal_frames.len(), data.fog.as_ref());
            if envmap.is_none() {
                p.cheap_pass = 0.0;
            }
            let mut envmap = envmap.map(|c| cubemaps[c].clone());
            if m.skybox {
                // In the 3D skybox (scaled up, far away, no depth prepass):
                // a reflection only, as water beyond its cheap distance; of
                // the sky, since no baked cubemap was taken out there.
                if let Some(sky) = sky {
                    envmap = Some(sky.clone());
                    p.sky_env = 1.0;
                }
                p.scene_fog_linear = Vec4::ZERO;
                p.scene_fog_gamma = Vec4::ZERO;
                p.refract = 0.0;
                p.reflect = 0.0;
                p.cheap_mode = 1.0;
                p.cheap_start = 0.0;
                p.cheap_end = 0.0;
                if reflection.is_some() && w.reflect && !w.force_cheap && !below {
                    // A planar reflection: the main view's (the sky
                    // camera draws the same screen, so its screen-space
                    // lookup lines up), over the refraction texture's
                    // clear (no under-water geometry out there): the fog
                    // colour, fully fogged. As the original's skybox water
                    // samples the same render targets, and so it matches
                    // the map's own water where they meet.
                    p.reflect = 1.0;
                    p.refract = 1.0;
                    p.refract_fog_only = 1.0;
                    p.cheap_pass = 0.0;
                } else if envmap.is_some() && !below {
                    // Opaque: the fog colour, the reflection over it by Fresnel.
                    p.cheap_pass = 1.0;
                    p.force_cheap = 1.0;
                }
            }
            let normal = if w.normal_frames.is_empty() {
                None
            } else if let Some(h) = normal_images.get(&w.normal_frames) {
                Some(h.clone())
            } else {
                let layers: Vec<Image> = w
                    .normal_frames
                    .iter()
                    .map(|&i| super::to_image(&data.textures[i], &data.look))
                    .collect();
                let h = frames_image(&layers).map(|i| images.add(i));
                if let Some(h) = &h {
                    normal_images.insert(w.normal_frames.clone(), h.clone());
                }
                h
            };
            // In chunks, each hidden where the camera can't see it, as the
            // world's faces are (`vis::split_mesh`): one mesh holds a
            // material's surfaces in every room, and other rooms' water
            // showed through the sky.
            let vis = data.visibility.as_deref().filter(|_| !super::merged_world());
            for (chunk, clusters) in super::vis::split_mesh(mesh, vis, super::vis::chunk_size()) {
                let (min, max) = chunk
                    .positions
                    .iter()
                    .fold((Vec3::MAX, Vec3::MIN), |(a, b), p| (a.min(Vec3::from(*p)), b.max(Vec3::from(*p))));
                let mut e = commands.spawn((
                    Name::new(if below {
                        format!("{} (below)", w.name)
                    } else {
                        w.name.clone()
                    }),
                    super::MapPart,
                    WaterSurface {
                        material: index,
                        min,
                        max,
                        below,
                        skybox: m.skybox,
                    },
                    if m.skybox {
                        RenderLayers::layer(super::SKYBOX_LAYER)
                    } else {
                        RenderLayers::layer(WATER_LAYER)
                    },
                    Mesh3d(meshes.add(super::build_mesh(&chunk, false))),
                    MeshMaterial3d(materials.add(WaterMaterial {
                        params: p,
                        normal: normal.clone(),
                        reflection: reflection.clone().filter(|_| w.reflect),
                        envmap: envmap.clone(),
                    })),
                    bevy::light::NotShadowCaster,
                    Transform::default(),
                    ChildOf(root),
                ));
                if !clusters.is_empty() {
                    e.insert(super::vis::VisClusters::new(clusters));
                }
            }
        }
    }
    let entities = data
        .water_materials
        .iter()
        .any(|m| m.reflect && m.reflect_entities && !m.force_cheap);
    if let Some(target) = &reflection {
        commands.spawn((reflection_camera(target.clone(), -10, entities), super::MapPart));
    }
    // Under-water screen warps, for the top materials that have one.
    let mut overlays = Vec::new();
    for (i, m) in data.water_materials.iter().enumerate().filter(|(_, m)| m.above_water) {
        let Some(warp) = overlay_for(m) else { continue };
        let layers: Vec<Image> = warp
            .normal_frames
            .iter()
            .map(|&t| super::to_image(&data.textures[t], &data.look))
            .collect();
        if let Some(image) = frames_image(&layers) {
            info!(
                "water {}: under-water overlay {} (refract {}, {} frames)",
                m.name,
                warp.name,
                warp.refract_amount,
                layers.len()
            );
            overlays.push((i, warp.clone(), images.add(image)));
        }
    }
    commands.insert_resource(MapWaterRender {
        materials: data.water_materials.clone(),
        overlays,
        reflection,
        scene_fog: (
            super::fog_color(data.fog.as_ref()),
            data.fog
                .as_ref()
                .map_or(Vec4::ZERO, |f| Vec4::new(f.color[0], f.color[1], f.color[2], 1.0)),
            super::fog_range(data.fog.as_ref()),
        ),
    });
    commands.insert_resource(WaterView::default());
}

/// Where the eye is relative to the map's water this frame.
#[derive(Resource, Clone, Debug, Default, PartialEq)]
pub struct WaterView {
    /// The water material whose volume holds the eye (index into the map's
    /// water materials).
    pub under: Option<usize>,
    /// Its fog colour (gamma), what an under-water view clears to.
    pub clear: Option<[f32; 3]>,
    /// Its surface height (meters).
    pub plane: Option<f32>,
    /// With the eye above water, the surface the near plane crosses (spec
    /// section 3, "Intersection view"): its material and height. What lies
    /// below it is drawn with the water's height fog.
    pub intersect: Option<(usize, f32)>,
}

impl WaterView {
    /// The water fog that views use now: (material, surface height, mode
    /// `fog::WATER_FOG_RANGE` or `fog::WATER_FOG_HEIGHT`).
    pub fn fog(&self) -> Option<(usize, f32, f32)> {
        match (self.under, self.plane, self.intersect) {
            (Some(i), Some(plane), _) => Some((i, plane, super::fog::WATER_FOG_RANGE)),
            (None, _, Some((i, plane))) => Some((i, plane, super::fog::WATER_FOG_HEIGHT)),
            _ => None,
        }
    }
}

/// The water fog uniforms for `WaterView::fog`: linear colour (w = mode),
/// and start, end, max density 1, fog plane (the surface plus the 2-unit
/// fudge), meters.
pub fn water_fog_uniforms(m: &MapWaterMaterial, plane: f32, mode: f32) -> (Vec4, Vec4) {
    let lin = m.fog_color.map(gamma_to_linear);
    (
        Vec4::new(lin[0], lin[1], lin[2], mode),
        Vec4::new(
            m.fog_start,
            m.fog_end.max(m.fog_start + 0.01),
            1.0,
            plane + 2.0 * 0.0254,
        ),
    )
}

/// The screen warp drawn while the eye is in water of material `m`: its
/// own `underwater_overlay`, else its bottom material's (de_port's sea
/// names it on the material seen from below; spec open question 7).
pub fn overlay_for(m: &MapWaterMaterial) -> Option<&MapScreenWarp> {
    m.underwater_overlay
        .as_ref()
        .or_else(|| m.bottom.as_ref().and_then(|b| b.underwater_overlay.as_ref()))
}

/// Distance from `p` to a box.
fn box_distance(p: Vec3, min: Vec3, max: Vec3) -> f32 {
    (min - p).max(p - max).max(Vec3::ZERO).length()
}

/// The top surface's material of a water volume, and the volume's surface
/// height. One material's mesh can span surfaces at several heights, so
/// the height comes from the volume.
fn volume_surface(v: &crate::core::MapWaterVolume, surfaces: &[WaterSurface]) -> Option<(usize, f32)> {
    let (vmin, vmax) = (v.brush.min, v.brush.max);
    surfaces
        .iter()
        .filter(|s| !s.below)
        .find(|s| {
            s.min.x <= vmax.x + 0.05
                && s.max.x >= vmin.x - 0.05
                && s.min.z <= vmax.z + 0.05
                && s.max.z >= vmin.z - 0.05
                && s.min.y <= vmax.y + 0.05
                && s.max.y >= vmax.y - 0.05
        })
        .map(|s| (s.material, vmax.y))
}

/// The water material whose volume holds `eye`, and its surface height.
fn under_water(eye: Vec3, volumes: &[crate::core::MapWaterVolume], surfaces: &[WaterSurface]) -> Option<(usize, f32)> {
    volumes
        .iter()
        .filter(|v| !v.slime && v.brush.planes.iter().all(|(n, d)| n.dot(eye) <= *d))
        .find_map(|v| volume_surface(v, surfaces))
}

/// The surface the near plane crosses, with the eye out of the water.
fn crossed_surface(
    corners: &[Vec3; 4],
    volumes: &[crate::core::MapWaterVolume],
    surfaces: &[WaterSurface],
) -> Option<(usize, f32)> {
    volumes.iter().filter(|v| !v.slime).find_map(|v| {
        let (m, h) = volume_surface(v, surfaces)?;
        near_plane_crosses(corners, h, v.brush.min, v.brush.max).then_some((m, h))
    })
}

/// The reflecting surface the reflection camera mirrors: the nearest
/// water volume below the eye whose surface reflects and that the camera
/// can see (spec: one water volume's views per frame, for the volume the
/// camera can see; `visible` by volume index). Its height and material.
fn reflection_plane(
    eye: Vec3,
    volumes: &[crate::core::MapWaterVolume],
    surfaces: &[WaterSurface],
    reflects: &dyn Fn(usize) -> bool,
    visible: &dyn Fn(usize) -> bool,
) -> Option<(f32, usize, usize)> {
    volumes
        .iter()
        .enumerate()
        .filter(|(i, v)| !v.slime && v.brush.max.y < eye.y && visible(*i))
        .filter_map(|(i, v)| {
            let (m, h) = volume_surface(v, surfaces)?;
            reflects(m).then(|| (box_distance(eye, v.brush.min, v.brush.max), h, m, i))
        })
        .min_by(|a, b| a.0.total_cmp(&b.0))
        .map(|(_, h, m, i)| (h, m, i))
}

/// On the reflection camera: the clusters around its water surface (the
/// reflection shows what is potentially visible from them; `vis::cull`).
#[derive(Component, Clone, PartialEq)]
pub struct ReflectionClusters(pub Vec<u32>);

pub(crate) type MainCameraFilter = (
    With<Camera3d>,
    Without<super::SkyboxCamera>,
    Without<super::ViewModelCamera>,
    Without<WaterReflectionCamera>,
);

/// Main views draw water surfaces, with a depth prepass (the refraction's
/// fog reads it).
#[allow(clippy::type_complexity)]
pub fn prepare_main_cameras(
    mut commands: Commands,
    render: Option<Res<MapWaterRender>>,
    main: Query<
        (
            Entity,
            Option<&RenderLayers>,
            Has<bevy::core_pipeline::prepass::DepthPrepass>,
        ),
        MainCameraFilter,
    >,
) {
    if render.is_none() {
        return;
    }
    let water = RenderLayers::layer(WATER_LAYER);
    for (entity, layers, prepass) in &main {
        if !layers.is_some_and(|l| l.intersects(&water)) {
            let base = layers.cloned().unwrap_or_default();
            commands.entity(entity).insert(base.union(&water));
        }
        if !prepass {
            commands
                .entity(entity)
                .insert(bevy::core_pipeline::prepass::DepthPrepass);
        }
    }
}

/// Per frame: whether the eye is under water or the near plane crosses a
/// surface, each surface's mode (the water cvars, its cheap distance), and
/// the reflection camera mirrored across the nearest reflecting surface
/// below the eye.
#[allow(clippy::type_complexity, clippy::too_many_arguments)]
pub fn update_water(
    mut commands: Commands,
    render: Option<Res<MapWaterRender>>,
    volumes: Option<Res<crate::core::MapWater>>,
    sky: Option<Res<super::MapSkybox>>,
    settings: Option<Res<WaterSettings>>,
    main: Query<
        (
            &GlobalTransform,
            &Projection,
            &Camera,
            Option<&bevy::core_pipeline::tonemapping::Tonemapping>,
            Option<&bevy::camera::primitives::Frustum>,
        ),
        MainCameraFilter,
    >,
    vis: (Option<Res<super::vis::ActiveVisibility>>, Option<Res<super::vis::NoVis>>),
    (mut volume_clusters, mut volume_key, mut last_surface): (
        Local<Option<Vec<Vec<u32>>>>,
        Local<Option<(usize, usize)>>,
        Local<Option<ReflectionClusters>>,
    ),
    mut reflection: Query<
        (
            Entity,
            &mut Transform,
            &mut GlobalTransform,
            &mut Projection,
            &mut Camera,
            Has<bevy::light::Skybox>,
            Option<&bevy::core_pipeline::tonemapping::Tonemapping>,
            &mut RenderLayers,
        ),
        With<WaterReflectionCamera>,
    >,
    mut surfaces: Query<(
        &WaterSurface,
        &MeshMaterial3d<WaterMaterial>,
        &mut Visibility,
        Option<&super::vis::VisClusters>,
    )>,
    mut materials: ResMut<Assets<WaterMaterial>>,
    view: Option<ResMut<WaterView>>,
) {
    let (Some(render), Some(mut view)) = (render, view) else {
        return;
    };
    let settings = settings.map(|s| *s).unwrap_or_default();
    let Some((eye_tf, projection, main_camera, main_tonemapping, frustum)) =
        main.iter().find(|(_, _, c, _, _)| c.is_active)
    else {
        return;
    };
    let eye = eye_tf.translation();
    let list: Vec<WaterSurface> = surfaces
        .iter()
        .filter(|(s, ..)| !s.skybox)
        .map(|(s, ..)| s.clone())
        .collect();
    let none: &[crate::core::MapWaterVolume] = &[];
    let volumes = volumes.as_ref().map_or(none, |v| &v.0);
    let under = under_water(eye, volumes, &list);
    let intersect = match (under, projection) {
        (None, Projection::Perspective(p)) => crossed_surface(&near_plane_corners(eye_tf, p), volumes, &list),
        _ => None,
    };
    let state = WaterView {
        under: under.map(|u| u.0),
        clear: under.map(|u| render.materials[u.0].fog_color),
        plane: under.map(|u| u.1),
        intersect,
    };
    if *view != state {
        *view = state;
    }
    let flag = |b: bool| if b { 1.0 } else { 0.0 };
    let draw = settings.draw_water != 0;
    for (s, handle, mut visibility, clusters) in &mut surfaces {
        // Drawn while `mat_drawwater` and the map's visibility allow.
        let shown = if draw && clusters.is_none_or(|c| c.potentially_visible) {
            Visibility::Inherited
        } else {
            Visibility::Hidden
        };
        if *visibility != shown {
            *visibility = shown;
        }
        if s.skybox {
            continue;
        }
        // The surface's passes (spec section 1): reflection and refraction
        // by the settings; beyond the cheap distance, without a planar
        // reflection, the original renders no refraction (cheap mode).
        let m = &render.materials[s.material];
        let near = surface_mode(m, &settings, 0.0);
        let far = surface_mode(m, &settings, box_distance(eye, s.min, s.max));
        let Some(w) = materials.get(&handle.0) else { continue };
        let want = (
            flag(near.refract),
            flag(near.reflect),
            flag(!near.reflect && w.envmap.is_some()),
            flag(!far.refract),
        );
        let p = &w.params;
        if (p.refract, p.reflect, p.cheap_pass, p.cheap_mode) != want
            && let Some(mut w) = materials.get_mut(&handle.0)
        {
            (
                w.params.refract,
                w.params.reflect,
                w.params.cheap_pass,
                w.params.cheap_mode,
            ) = want;
        }
    }
    // The reflection: the nearest reflecting top surface below the eye
    // that the camera can see: in its view frustum and potentially visible
    // from its cluster (each volume's clusters found once). Rendering a
    // reflection of water out of sight would cost a second view of the
    // scene, and its mirrored eye (often in solid) would turn visibility
    // culling off.
    let reflects = |m: usize| surface_mode(&render.materials[m], &settings, 0.0).reflect;
    let (map_vis, novis) = vis;
    let pvs = map_vis.filter(|_| novis.is_none_or(|n| n.0 == 0));
    // Found again for another map (its visibility is another allocation).
    let map_key = pvs.as_ref().map(|v| (std::sync::Arc::as_ptr(&v.0) as usize, volumes.len()));
    if let Some(v) = &pvs
        && *volume_key != map_key
    {
        *volume_key = map_key;
        *volume_clusters = Some(
            volumes
                .iter()
                // Up to just above the surface: the leaves a reflected
                // ray leaves the surface into.
                .map(|w| super::vis::box_clusters(&v.0, w.brush.min, w.brush.max + Vec3::Y * 0.1))
                .collect(),
        );
    }
    let eye_cluster = pvs.as_ref().and_then(|v| v.0.cluster_at(eye));
    let visible = |i: usize| {
        let w = &volumes[i];
        let in_view = frustum.is_none_or(|f| {
            f.intersects_obb_identity(&bevy::camera::primitives::Aabb::from_min_max(w.brush.min, w.brush.max))
        });
        let in_pvs = match (&pvs, eye_cluster, volume_clusters.as_ref()) {
            (Some(v), Some(c), Some(clusters)) => clusters.get(i).is_none_or(|cl| v.0.sees_any(c, cl)),
            _ => true,
        };
        in_view && in_pvs
    };
    let plane = reflection_plane(eye, volumes, &list, &reflects, &visible);
    let Ok((entity, mut tf, mut global, mut proj, mut camera, has_sky, tonemapping, mut layers)) =
        reflection.single_mut()
    else {
        return;
    };
    let active = plane.is_some() && under.is_none() && draw;
    if camera.is_active != active {
        camera.is_active = active;
    }
    let order = main_camera.order - 10;
    if camera.order != order {
        camera.order = order;
    }
    // What the reflection may show, for visibility culling.
    let surface = plane
        .filter(|_| active)
        .and_then(|(_, _, i)| volume_clusters.as_ref()?.get(i).cloned())
        .map(ReflectionClusters);
    if *last_surface != surface {
        match &surface {
            Some(s) => commands.entity(entity).insert(s.clone()),
            None => commands.entity(entity).remove::<ReflectionClusters>(),
        };
        *last_surface = surface;
    }
    let entities = plane.is_some_and(|(_, m, _)| reflects_entities(&render.materials[m], &settings));
    let want_layers = if entities {
        world_layers()
    } else {
        RenderLayers::layer(REFLECT_LAYER)
    };
    if *layers != want_layers {
        *layers = want_layers;
    }
    if !has_sky && let Some(sky) = &sky {
        commands.entity(entity).insert(bevy::light::Skybox {
            image: Some(sky.0.clone()),
            brightness: super::LIGHTMAP_EXPOSURE,
            ..default()
        });
    }
    if let Some(t) = main_tonemapping
        && tonemapping != Some(t)
    {
        commands.entity(entity).insert(*t);
    }
    let (Some((height, _, _)), Projection::Perspective(main_p)) = (plane, projection) else {
        return;
    };
    *tf = reflection_transform(eye_tf, height);
    *global = GlobalTransform::from(*tf);
    // Draw only what is above the surface minus the fudge (view space:
    // visible where c.xyz . p + c.w > 0).
    let clip = height - 2.0 * 0.0254;
    let n = tf.rotation.inverse() * Vec3::Y;
    let w = tf.translation.y - clip;
    let mut p = main_p.clone();
    if w < 0.0 {
        p.near_clip_plane = n.extend(w);
    }
    *proj = Projection::custom(ReflectionProjection(p));
}

/// No pre-fogged surface (`WaterParams::prefog_plane`).
const NO_PREFOG: f32 = -1.0e9;

/// The water's fog on everything drawn below its surface (spec sections 8
/// and 9), when the eye enters or leaves a water volume or the near plane
/// starts or stops crossing a surface:
/// - Under water: range fog with the water's own fog (`$fogcolor`,
///   `$fogstart`, `$fogend` of the top material: open question 7) below the
///   surface, while the world above keeps its fog (the original sees it
///   through the bottom material's refraction view, drawn with world fog).
///   The surface seen from below takes it too.
/// - Eye just above, near plane crossing (the intersection view): the
///   water's height fog below the surface, so what is seen directly under
///   the waterline matches what is seen through the surface; that
///   surface's refraction then reads it already fogged.
///
/// Sets the world and prop materials (not the 3D skybox's), the water
/// materials and `fog::SceneFog` (ropes, decals, particles).
#[allow(clippy::type_complexity, clippy::too_many_arguments)]
pub fn underwater_fog(
    view: Option<Res<WaterView>>,
    render: Option<Res<MapWaterRender>>,
    world: Option<ResMut<Assets<super::WorldMaterial>>>,
    props: Option<ResMut<Assets<super::PropMaterial>>>,
    mut water: ResMut<Assets<WaterMaterial>>,
    scene: Option<ResMut<super::fog::SceneFog>>,
    surfaces: Query<(&WaterSurface, &MeshMaterial3d<WaterMaterial>)>,
    sky_world: Query<(&MeshMaterial3d<super::WorldMaterial>, &RenderLayers)>,
    sky_props: Query<(&MeshMaterial3d<super::PropMaterial>, &RenderLayers)>,
    mut last: Local<Option<Option<(usize, f32, f32)>>>,
) {
    let (Some(view), Some(render)) = (view, render) else {
        *last = None;
        return;
    };
    let state = view.fog();
    if *last == Some(state) || (last.is_none() && state.is_none()) {
        *last = Some(state);
        return;
    }
    *last = Some(state);
    let fog = state.map(|(i, plane, mode)| {
        let m = &render.materials[i];
        let (color, range) = water_fog_uniforms(m, plane, mode);
        (
            color,
            Vec4::new(m.fog_color[0], m.fog_color[1], m.fog_color[2], 1.0),
            range,
            mode,
            plane,
        )
    });
    let (color, range) = fog.map_or((Vec4::ZERO, Vec4::ZERO), |(c, _, r, _, _)| (c, r));
    let sky = RenderLayers::layer(super::SKYBOX_LAYER);
    let sky_world: std::collections::HashSet<_> = sky_world
        .iter()
        .filter(|(_, l)| l.intersects(&sky))
        .map(|(m, _)| m.0.id())
        .collect();
    let sky_props: std::collections::HashSet<_> = sky_props
        .iter()
        .filter(|(_, l)| l.intersects(&sky))
        .map(|(m, _)| m.0.id())
        .collect();
    if let Some(mut world) = world {
        let ids: Vec<_> = world.ids().filter(|id| !sky_world.contains(id)).collect();
        for id in ids {
            if let Some(mut m) = world.get_mut(id) {
                m.params.water_fog_color = color;
                m.params.water_fog_range = range;
            }
        }
    }
    if let Some(mut props) = props {
        let ids: Vec<_> = props.ids().filter(|id| !sky_props.contains(id)).collect();
        for id in ids {
            if let Some(mut m) = props.get_mut(id) {
                m.params.water_fog_color = color;
                m.params.water_fog_range = range;
            }
        }
    }
    if let Some(mut scene) = scene {
        scene.0.water_color = color;
        scene.0.water_range = range;
    }
    // The surface itself: seen from below, it is in the under-water view;
    // crossed by the near plane, it reads what lies below already fogged.
    let (lin, gamma, range, prefog) = match fog {
        Some((c, g, r, mode, _)) if mode == super::fog::WATER_FOG_RANGE => {
            (c.truncate().extend(1.0), g, r.truncate().extend(0.0), NO_PREFOG)
        }
        Some((_, _, _, _, plane)) => (render.scene_fog.0, render.scene_fog.1, render.scene_fog.2, plane),
        None => (render.scene_fog.0, render.scene_fog.1, render.scene_fog.2, NO_PREFOG),
    };
    for (_, handle) in surfaces.iter().filter(|(s, _)| !s.skybox) {
        if let Some(mut m) = water.get_mut(&handle.0) {
            m.params.scene_fog_linear = lin;
            m.params.scene_fog_gamma = gamma;
            m.params.scene_fog_range = range;
            m.params.prefog_plane = prefog;
        }
    }
}

/// Uniforms of a screen warp (screen_warp.wgsl).
#[derive(Clone, Copy, Debug, Default, bevy::render::render_resource::ShaderType)]
pub struct ScreenWarpParams {
    /// Linear tint.
    pub tint: Vec4,
    /// Normal-map repeats across the screen (xy) and scroll per second
    /// (zw).
    pub scale_scroll: Vec4,
    pub refract_amount: f32,
    pub frame_rate: f32,
    pub frame_count: f32,
    pub _pad: f32,
}

/// The under-water screen warp: drawn by the view-model camera in its
/// transmissive phase, so it refracts the finished frame (world and view
/// models) after the view models, as the original draws it.
#[derive(Asset, TypePath, AsBindGroup, Debug, Clone)]
pub struct ScreenWarpMaterial {
    #[uniform(0)]
    pub params: ScreenWarpParams,
    #[texture(1, dimension = "2d_array")]
    #[sampler(2)]
    pub normal: Option<Handle<Image>>,
}

impl Material for ScreenWarpMaterial {
    fn vertex_shader() -> ShaderRef {
        "embedded://mashup/map/screen_warp.wgsl".into()
    }

    fn fragment_shader() -> ShaderRef {
        "embedded://mashup/map/screen_warp.wgsl".into()
    }

    fn alpha_mode(&self) -> AlphaMode {
        AlphaMode::Opaque
    }

    fn reads_view_transmission_texture(&self) -> bool {
        true
    }

    fn enable_prepass() -> bool {
        false
    }

    fn enable_shadows() -> bool {
        false
    }

    /// Full screen, over everything: no culling, no depth test or write.
    fn specialize(
        _pipeline: &bevy::pbr::MaterialPipeline,
        descriptor: &mut bevy::render::render_resource::RenderPipelineDescriptor,
        _layout: &bevy::mesh::MeshVertexBufferLayoutRef,
        _key: bevy::pbr::MaterialPipelineKey<Self>,
    ) -> Result<(), bevy::render::render_resource::SpecializedMeshPipelineError> {
        descriptor.primitive.cull_mode = None;
        if let Some(depth) = descriptor.depth_stencil.as_mut() {
            depth.depth_write_enabled = Some(false);
            depth.depth_compare = Some(bevy::render::render_resource::CompareFunction::Always);
        }
        Ok(())
    }
}

/// The uniforms of a screen warp.
pub fn screen_warp_params(w: &MapScreenWarp) -> ScreenWarpParams {
    ScreenWarpParams {
        tint: Vec4::new(
            gamma_to_linear(w.tint[0]),
            gamma_to_linear(w.tint[1]),
            gamma_to_linear(w.tint[2]),
            1.0,
        ),
        scale_scroll: Vec4::new(w.scale.x, w.scale.y, w.scroll.x, w.scroll.y),
        refract_amount: w.refract_amount,
        frame_rate: w.frame_rate,
        frame_count: w.normal_frames.len().max(1) as f32,
        _pad: 0.0,
    }
}

/// A screen warp shown while the eye is in water of this material (index
/// into the map's water materials).
#[derive(Component, Clone, Copy, Debug)]
pub struct UnderwaterOverlay {
    pub material: usize,
}

/// One triangle covering the screen: positions in clip space, uv 0..1
/// across the screen (v down).
pub fn fullscreen_triangle() -> Mesh {
    let mut mesh = Mesh::new(
        bevy::mesh::PrimitiveTopology::TriangleList,
        RenderAssetUsages::RENDER_WORLD,
    );
    mesh.insert_attribute(
        Mesh::ATTRIBUTE_POSITION,
        vec![[-1.0f32, -1.0, 0.0], [3.0, -1.0, 0.0], [-1.0, 3.0, 0.0]],
    );
    mesh.insert_attribute(Mesh::ATTRIBUTE_UV_0, vec![[0.0f32, 1.0], [2.0, 1.0], [0.0, -1.0]]);
    mesh
}

/// Spawn the map's under-water screen warps (hidden), drawn by the view
/// model camera.
pub fn spawn_overlays(
    mut commands: Commands,
    render: Option<ResMut<MapWaterRender>>,
    meshes: Option<ResMut<Assets<Mesh>>>,
    materials: Option<ResMut<Assets<ScreenWarpMaterial>>>,
) {
    let (Some(mut render), Some(mut meshes), Some(mut materials)) = (render, meshes, materials) else {
        return;
    };
    if render.overlays.is_empty() {
        return;
    }
    let mesh = meshes.add(fullscreen_triangle());
    for (material, warp, normal) in std::mem::take(&mut render.overlays) {
        commands.spawn((
            Name::new(format!("Under-water overlay {}", warp.name)),
            super::MapPart,
            UnderwaterOverlay { material },
            Mesh3d(mesh.clone()),
            MeshMaterial3d(materials.add(ScreenWarpMaterial {
                params: screen_warp_params(&warp),
                normal: Some(normal),
            })),
            RenderLayers::layer(super::view_model::VIEW_MODEL_LAYER),
            bevy::camera::visibility::NoFrustumCulling,
            bevy::light::NotShadowCaster,
            Transform::default(),
            Visibility::Hidden,
        ));
    }
}

/// Show the warp of the water the eye is in (spec section 9).
pub fn show_overlays(view: Option<Res<WaterView>>, mut overlays: Query<(&UnderwaterOverlay, &mut Visibility)>) {
    let shown = view.as_deref().and_then(|v| v.under);
    for (o, mut visibility) in &mut overlays {
        let want = if shown == Some(o.material) {
            Visibility::Visible
        } else {
            Visibility::Hidden
        };
        if *visibility != want {
            *visibility = want;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: f32, b: f32, eps: f32) -> bool {
        (a - b).abs() < eps
    }

    /// The reflection mirrors the nearest reflecting volume below the eye
    /// that the camera can see; water out of sight gets no reflection view.
    #[test]
    fn reflection_only_for_water_in_sight() {
        let volume = |x: f32, top: f32| crate::core::MapWaterVolume {
            brush: crate::core::MapBrush::from_box(Vec3::new(x, top - 2.0, 0.0), Vec3::new(x + 4.0, top, 4.0)),
            slime: false,
        };
        let surface = |x: f32, top: f32| WaterSurface {
            material: 0,
            min: Vec3::new(x, top, 0.0),
            max: Vec3::new(x + 4.0, top, 4.0),
            below: false,
            skybox: false,
        };
        let volumes = [volume(0.0, 1.0), volume(20.0, -3.0)];
        let surfaces = [surface(0.0, 1.0), surface(20.0, -3.0)];
        let eye = Vec3::new(2.0, 3.0, 2.0);
        let all = |_: usize| true;
        let plane = reflection_plane(eye, &volumes, &surfaces, &all, &all);
        assert_eq!(plane, Some((1.0, 0, 0)), "the nearest");
        let second = |i: usize| i == 1;
        assert_eq!(reflection_plane(eye, &volumes, &surfaces, &all, &second), Some((-3.0, 0, 1)));
        assert_eq!(reflection_plane(eye, &volumes, &surfaces, &all, &|_| false), None);
    }

    #[test]
    fn fresnel_and_shore_fade() {
        assert_eq!(fresnel(1.0), 0.0);
        assert!(close(fresnel(0.5), 0.03125, 1e-6));
        assert!(close(fresnel(85f32.to_radians().cos()), 0.63384, 1e-4));
        assert!(close(shore_fade(0.08), 0.6, 1e-5));
        assert_eq!(shore_fade(0.04), 0.0);
    }

    #[test]
    fn cheap_ramp_cases() {
        // de_aztec: 500 / 2000 units.
        let alpha = |d: f32, f: f32| (f + cheap_ramp(d, 500.0, 2000.0)).clamp(0.0, 1.0);
        assert!(close(alpha(1250.0, 0.03125), 0.53125, 1e-6));
        assert!(close(alpha(400.0, 0.03125), 0.03125, 1e-6));
        assert_eq!(alpha(2500.0, 0.03125), 1.0);
        // cs_militia: 512 / 960.
        assert!(close(cheap_ramp(736.0, 512.0, 960.0), 0.5, 1e-6));
    }

    #[test]
    fn height_fog_cases() {
        let k = 1.0 / 45.0;
        assert_eq!(height_fog(100.0, -50.0, 0.0, 150.0, k), 1.0);
        assert!(close(height_fog(100.0, -5.0, 0.0, 105.0, k), 0.1111, 1e-4));
    }

    #[test]
    fn refraction_fog_and_combine() {
        // de_aztec: P = 0.2 grey, a = 0.55 -> weight 0.5 toward the fog.
        let fog = [0.15f32, 0.1, 0.0].map(gamma_to_linear);
        assert!(close(fog[0], 0.015400, 1e-5) && close(fog[1], 0.0063096, 1e-6));
        let w = (0.55f32 - 0.05).clamp(0.0, 1.0);
        let p: Vec<f32> = fog.iter().map(|f| 0.2 + (f - 0.2) * w).collect();
        assert!(close(p[0], 0.10770, 1e-4) && close(p[1], 0.10316, 1e-4) && close(p[2], 0.1, 1e-6));
        // Pass 1: lerp(P, R, F).
        let c = Vec3::splat(0.1).lerp(Vec3::new(0.5, 0.6, 0.7), 0.25);
        assert!(c.abs_diff_eq(Vec3::new(0.2, 0.225, 0.25), 1e-6));
    }

    #[test]
    fn distortion_and_screen_mapping() {
        // a0 0.5 x $refractamount 0.2, N.xy (0.1, -0.05), base (0.4, 0.6).
        let uv = Vec2::new(0.4, 0.6) + 0.5 * 0.2 * Vec2::new(0.1, -0.05);
        assert!(uv.abs_diff_eq(Vec2::new(0.41, 0.595), 1e-6));
        // NDC (0.5, 0.5): reflection (0.75, 0.75), refraction (0.75, 0.25).
        let ndc = Vec2::new(0.5, 0.5);
        assert_eq!((Vec2::ONE + ndc) / 2.0, Vec2::new(0.75, 0.75));
        assert_eq!(
            Vec2::new((1.0 + ndc.x) / 2.0, (1.0 - ndc.y) / 2.0),
            Vec2::new(0.75, 0.25)
        );
    }

    #[test]
    fn animation() {
        assert_eq!(normal_frame(2.0, 16.0, 29), 3);
        assert_eq!(normal_frame(1.8125, 16.0, 29), 0);
        let a = 45f32.to_radians();
        let t = scroll_translation(0.01 * Vec2::new(a.cos(), a.sin()), 100.0);
        assert!(t.abs_diff_eq(Vec2::splat(0.70711), 1e-4));
        let a = 25f32.to_radians();
        let t = scroll_translation(0.02 * Vec2::new(a.cos(), a.sin()), 100.0);
        assert!(t.abs_diff_eq(Vec2::new(0.81262, 0.84524), 1e-4));
        let (uv1, uv2) = layer_uvs(Vec2::ONE, 10.0, Vec2::new(0.01, 0.01), Vec2::new(-0.025, 0.025));
        assert!(uv1.abs_diff_eq(Vec2::new(0.3, 0.1), 1e-6));
        assert!(uv2.abs_diff_eq(Vec2::new(0.2, 0.7), 1e-6));
    }

    #[test]
    fn three_layer_flat_bias() {
        let texel = 128.0 / 255.0;
        let s = 0.33 * 3.0 * texel;
        let n = 2.0 * s - 1.0;
        assert!(close(n, -0.00612, 1e-4));
        assert!(close(2.0 * 0.33 * 3.0 - 1.0, 0.98, 1e-6));
        assert!(close(0.33 * 3.0, 0.99, 1e-6));
    }

    #[test]
    fn reflection_camera_mirrors_pitch_and_roll() {
        // Engine frame: Y up, looking along -Z at yaw 0. Pitch 30 degrees
        // down, roll 5, eye 100 above a plane at 0.
        let rotation = Quat::from_euler(EulerRot::YXZ, 0.0, -30f32.to_radians(), 5f32.to_radians());
        let eye = GlobalTransform::from(Transform::from_xyz(0.0, 100.0, 0.0).with_rotation(rotation));
        let r = reflection_transform(&eye, 0.0);
        assert!(r.translation.abs_diff_eq(Vec3::new(0.0, -100.0, 0.0), 1e-4));
        let (yaw, pitch, roll) = r.rotation.to_euler(EulerRot::YXZ);
        assert!(close(yaw, 0.0, 1e-5));
        assert!(close(pitch, 30f32.to_radians(), 1e-5));
        assert!(close(roll, -5f32.to_radians(), 1e-5));
        // de_aztec: eye at -300 units over water at -455 -> -610.
        let eye = GlobalTransform::from(Transform::from_xyz(0.0, -300.0, 0.0));
        assert!(close(reflection_transform(&eye, -455.0).translation.y, -610.0, 1e-3));
    }

    #[test]
    fn modes() {
        let aztec = MapWaterMaterial {
            cheap_start: 500.0,
            cheap_end: 2000.0,
            envmap: Some(WaterEnvmap::Nearest),
            ..default()
        };
        assert_eq!(
            water_mode(&aztec, 1500.0),
            WaterMode {
                reflect: false,
                refract: true
            }
        );
        assert_eq!(
            water_mode(&aztec, 2100.0),
            WaterMode {
                reflect: false,
                refract: false
            }
        );
        let chateau = MapWaterMaterial {
            reflect: true,
            reflect_entities: true,
            cheap_start: 1000.0,
            cheap_end: 2000.0,
            ..default()
        };
        assert_eq!(
            water_mode(&chateau, 5000.0),
            WaterMode {
                reflect: true,
                refract: true
            }
        );
        let cheap = MapWaterMaterial {
            force_cheap: true,
            ..chateau
        };
        assert!(!water_mode(&cheap, 0.0).refract);
    }

    #[test]
    fn cvars_change_modes() {
        let port = MapWaterMaterial {
            reflect: true,
            cheap_end: 5000.0,
            ..default()
        };
        let on = WaterSettings::default();
        assert_eq!(
            surface_mode(&port, &on, 0.0),
            WaterMode {
                reflect: true,
                refract: true
            }
        );
        // r_WaterDrawReflection 0: refraction only, and cheap far away.
        let no_reflection = WaterSettings {
            draw_reflection: 0,
            ..on
        };
        assert!(!surface_mode(&port, &no_reflection, 0.0).reflect);
        assert!(!surface_mode(&port, &no_reflection, 6000.0).refract);
        // r_WaterDrawRefraction 0.
        let no_refraction = WaterSettings {
            draw_refraction: 0,
            ..on
        };
        assert!(!surface_mode(&port, &no_refraction, 0.0).refract);
        // $forceexpensive 0: reflection only with r_waterforceexpensive 1.
        let cheap_material = MapWaterMaterial {
            force_expensive: false,
            ..port.clone()
        };
        assert!(surface_mode(&cheap_material, &on, 0.0).reflect);
        let not_forced = WaterSettings {
            force_expensive: 0,
            ..on
        };
        assert!(!surface_mode(&cheap_material, &not_forced, 0.0).reflect);
        assert!(surface_mode(&port, &not_forced, 0.0).reflect);
        // Entities: the material or r_waterforcereflectentities.
        assert!(!reflects_entities(&port, &on));
        assert!(reflects_entities(
            &port,
            &WaterSettings {
                reflect_entities: 1,
                ..on
            }
        ));
    }

    #[test]
    fn near_plane_crossing() {
        let u = 0.0254;
        let eye = |y: f32, pitch: f32| {
            GlobalTransform::from(
                Transform::from_xyz(0.0, y, 0.0).with_rotation(Quat::from_rotation_x(pitch.to_radians())),
            )
        };
        let p = PerspectiveProjection {
            near: 0.1,
            fov: 2.0 * 0.75f32.atan(),
            aspect_ratio: 4.0 / 3.0,
            ..default()
        };
        let (min, max) = (Vec3::new(-10.0, -5.0, -10.0), Vec3::new(10.0, 0.0, 10.0));
        let crosses = |y: f32, pitch: f32| near_plane_crosses(&near_plane_corners(&eye(y, pitch), &p), 0.0, min, max);
        // Eye at the surface, a few units above or below: crosses.
        assert!(crosses(0.0, 0.0));
        assert!(crosses(4.0 * u, 0.0));
        assert!(crosses(-4.0 * u, -30.0));
        // Well above (the near rectangle is ~3 units tall, plus 7): not.
        assert!(!crosses(20.0 * u, 0.0));
        assert!(!crosses(-20.0 * u, 0.0));
        // Beside the volume: not.
        let far = GlobalTransform::from(Transform::from_xyz(50.0, 0.0, 0.0));
        assert!(!near_plane_crosses(&near_plane_corners(&far, &p), 0.0, min, max));
    }

    #[test]
    fn water_view_fog_and_overlay() {
        let warp = MapScreenWarp {
            name: "effects/water_warp01".into(),
            normal_frames: vec![0],
            frame_rate: 0.0,
            refract_amount: 0.05,
            tint: [1.0; 3],
            scale: Vec2::ONE,
            scroll: Vec2::ZERO,
        };
        // de_port: the top material has none, its bottom material has it.
        let port = MapWaterMaterial {
            fog_color: [21.0 / 255.0, 48.0 / 255.0, 52.0 / 255.0],
            fog_start: 100.0 * 0.0254,
            fog_end: 500.0 * 0.0254,
            bottom: Some(Box::new(MapWaterMaterial {
                above_water: false,
                underwater_overlay: Some(warp.clone()),
                ..default()
            })),
            ..default()
        };
        assert_eq!(overlay_for(&port), Some(&warp));
        assert_eq!(overlay_for(&MapWaterMaterial::default()), None);
        // Under water: range fog; at the surface from above: height fog.
        let under = WaterView {
            under: Some(0),
            clear: Some(port.fog_color),
            plane: Some(1.0),
            intersect: None,
        };
        assert_eq!(under.fog(), Some((0, 1.0, crate::map::fog::WATER_FOG_RANGE)));
        let at = WaterView {
            intersect: Some((0, 1.0)),
            ..default()
        };
        assert_eq!(at.fog(), Some((0, 1.0, crate::map::fog::WATER_FOG_HEIGHT)));
        assert_eq!(WaterView::default().fog(), None);
        let (c, r) = water_fog_uniforms(&port, 1.0, crate::map::fog::WATER_FOG_HEIGHT);
        assert_eq!(c.w, 2.0);
        assert!((c.x - gamma_to_linear(21.0 / 255.0)).abs() < 1e-6);
        assert!((r.y - 500.0 * 0.0254).abs() < 1e-6 && (r.w - (1.0 + 2.0 * 0.0254)).abs() < 1e-6);
        // The warp's uniforms: linear tint, frames.
        let p = screen_warp_params(&warp);
        assert_eq!((p.refract_amount, p.frame_count), (0.05, 1.0));
    }
}
