//! Monitors (Source's func_monitor showing a point_camera's view, public
//! entity docs), for any game: the map's monitor screen texture
//! (`MapData::camera_texture`) is a render target that a camera of our
//! own draws into from the active camera's pose (`MonitorCamera`, which
//! the logic writes), only while one is active. What the camera sees is
//! culled for the player's own view (`vis`), as the rest of the map. The
//! sky shows behind it as in the main view: a sky camera of its own
//! (`ScreenSkyCamera`) draws the 2D sky and the 3D skybox from the
//! monitor camera's place scaled into it, first. Its fog is the
//! point_camera's own (`MonitorFog`), not the map's: the screen camera
//! carries it as a `DistanceFog` that the map's shaders read as the
//! view's fog in place of their own (`fog.wgsl`, `view_fog`).

use bevy::{
    camera::RenderTarget,
    image::{ImageAddressMode, ImageSampler, ImageSamplerDescriptor},
    prelude::*,
    render::render_resource::TextureFormat,
};

/// The monitor screen's size, pixels.
pub const SCREEN_SIZE: u32 = 512;

/// The render target monitors show.
#[derive(Resource, Clone, Debug)]
pub struct MonitorScreen(pub Handle<Image>);

/// Where the active monitor camera is (engine space, looking down -Z) and
/// its vertical field of view (degrees); None: no monitor shows anything.
/// With it, its fog (None: none).
#[derive(Resource, Clone, Copy, Debug, Default, PartialEq)]
pub struct MonitorCamera(pub Option<(Vec3, Quat, f32)>, pub Option<MonitorFog>);

/// A point_camera's own fog (public entity docs): on or off, colour (sRGB
/// 0-1), start and end (meters here; the logic gives entity units) and max
/// density.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct MonitorFog {
    pub enabled: bool,
    pub color: [f32; 3],
    pub start: f32,
    pub end: f32,
    pub max_density: f32,
}

/// `DistanceFog::directional_light_exponent` marking a view's fog as
/// Source fog for the map's shaders (`fog.wgsl`): its colour alpha 1 when
/// on, its linear start and end in meters, its max density in the
/// directional light colour's red (alpha 0, so Bevy's own scattering stays
/// off).
pub const SOURCE_VIEW_FOG: f32 = -7.0;

impl MonitorFog {
    /// The fog as the screen camera's `DistanceFog` (`SOURCE_VIEW_FOG`),
    /// distances scaled to meters by `scale`.
    pub fn distance_fog(&self, scale: f32) -> bevy::pbr::DistanceFog {
        let [r, g, b] = self.color;
        bevy::pbr::DistanceFog {
            color: Color::srgba(r, g, b, if self.enabled { 1.0 } else { 0.0 }),
            directional_light_color: Color::linear_rgba(self.max_density, 0.0, 0.0, 0.0),
            directional_light_exponent: SOURCE_VIEW_FOG,
            falloff: bevy::pbr::FogFalloff::Linear {
                start: self.start * scale,
                end: self.end * scale,
            },
        }
    }
}

/// Our camera drawing into `MonitorScreen`.
#[derive(Component)]
pub struct ScreenCamera;

/// The sky behind the monitor camera's view (as `SkyboxCamera` is for the
/// main view). It is a `ScreenCamera` too, so what leaves monitor cameras
/// out (the main view's systems) leaves it out.
#[derive(Component)]
pub struct ScreenSkyCamera;

/// A render target image for the screen.
pub fn screen_image(images: &mut Assets<Image>) -> Handle<Image> {
    let mut image = Image::new_target_texture(
        SCREEN_SIZE,
        SCREEN_SIZE,
        TextureFormat::Rgba8Unorm,
        Some(TextureFormat::Rgba8UnormSrgb),
    );
    // Monitor faces' texture coordinates are whole repeats away from 0
    // (59..60): the picture repeats, as the game's textures do.
    image.sampler = ImageSampler::Descriptor(ImageSamplerDescriptor {
        address_mode_u: ImageAddressMode::Repeat,
        address_mode_v: ImageAddressMode::Repeat,
        ..ImageSamplerDescriptor::linear()
    });
    images.add(image)
}

/// Keep the screen camera where the active point_camera is, on only while
/// one is.
#[allow(clippy::too_many_arguments, clippy::type_complexity)]
fn drive_screen_camera(
    screen: Option<Res<MonitorScreen>>,
    want: Res<MonitorCamera>,
    mut cameras: Query<(Entity, &mut Camera, &mut Transform, &mut Projection), (With<ScreenCamera>, Without<ScreenSkyCamera>)>,
    mut skies: Query<(Entity, &mut Camera, &mut Transform, &mut Projection), With<ScreenSkyCamera>>,
    sky_info: Option<Res<super::SkyCameraInfo>>,
    skybox: Option<Res<super::MapSkybox>>,
    mut commands: Commands,
) {
    let Some(screen) = screen else {
        for (e, ..) in cameras.iter().chain(skies.iter()) {
            commands.entity(e).despawn();
        }
        return;
    };
    // The camera's fog, else none (not the map's).
    let fog = want.1.unwrap_or_default().distance_fog(1.0);
    let Some((at, rotation, fov)) = want.0 else {
        for (_, mut c, ..) in cameras.iter_mut().chain(skies.iter_mut()) {
            if c.is_active {
                c.is_active = false;
            }
        }
        return;
    };
    // The sky camera: the 2D sky, and the 3D skybox from the monitor
    // camera's place scaled into it.
    let has_sky = skybox.is_some() || sky_info.is_some();
    if has_sky {
        let (sky_at, layer) = match &sky_info {
            Some(i) => (i.0.origin + at / i.0.scale, super::SKYBOX_LAYER),
            None => (at, super::EMPTY_LAYER),
        };
        let sky_projection = Projection::Perspective(PerspectiveProjection {
            fov: fov.clamp(1.0, 170.0).to_radians(),
            aspect_ratio: 1.0,
            near: 0.01,
            ..default()
        });
        match skies.single_mut() {
            Ok((_, mut c, mut t, mut p)) => {
                if !c.is_active {
                    c.is_active = true;
                }
                t.set_if_neq(Transform::from_translation(sky_at).with_rotation(rotation));
                if let (Projection::Perspective(now), Projection::Perspective(want)) = (&*p, &sky_projection)
                    && (now.fov - want.fov).abs() > 1e-4
                {
                    *p = sky_projection;
                }
            }
            Err(_) => {
                let mut e = commands.spawn((
                    Name::new("Monitor sky camera"),
                    ScreenSkyCamera,
                    ScreenCamera,
                    super::MapPart,
                    Camera3d::default(),
                    Camera {
                        order: -2,
                        clear_color: Color::BLACK.into(),
                        ..default()
                    },
                    RenderTarget::Image(screen.0.clone().into()),
                    sky_projection,
                    Transform::from_translation(sky_at).with_rotation(rotation),
                    bevy::camera::visibility::RenderLayers::layer(layer),
                ));
                if let Some(sky) = &skybox {
                    e.insert(bevy::light::Skybox {
                        image: Some(sky.0.clone()),
                        brightness: super::LIGHTMAP_EXPOSURE,
                        ..default()
                    });
                }
                if let Some(fog) = sky_info.as_ref().and_then(|i| i.0.fog.as_ref().map(|f| (f.clone(), i.0.scale))) {
                    let ([r, g, b], scale) = (fog.0.color, fog.1);
                    e.insert(bevy::pbr::DistanceFog {
                        color: Color::srgb(r, g, b),
                        falloff: bevy::pbr::FogFalloff::Linear {
                            start: fog.0.start / scale,
                            end: fog.0.end / scale,
                        },
                        ..default()
                    });
                }
            }
        }
    }
    let projection = Projection::Perspective(PerspectiveProjection {
        fov: fov.clamp(1.0, 170.0).to_radians(),
        aspect_ratio: 1.0,
        ..default()
    });
    match cameras.single_mut() {
        Ok((e, mut c, mut t, mut p)) => {
            if !c.is_active {
                c.is_active = true;
            }
            if want.is_changed() {
                commands.entity(e).insert(fog);
            }
            t.set_if_neq(Transform::from_translation(at).with_rotation(rotation));
            let fov_now = match &*p {
                Projection::Perspective(pp) => pp.fov,
                _ => -1.0,
            };
            if (fov_now - fov.clamp(1.0, 170.0).to_radians()).abs() > 1e-4 {
                *p = projection;
            }
        }
        Err(_) => {
            commands.spawn((
                Name::new("Monitor camera"),
                ScreenCamera,
                super::MapPart,
                Camera3d::default(),
                Camera {
                    // Before the main view, which shows its picture; after
                    // its sky camera, whose picture it draws over.
                    order: -1,
                    clear_color: if has_sky {
                        ClearColorConfig::None
                    } else {
                        Color::BLACK.into()
                    },
                    ..default()
                },
                RenderTarget::Image(screen.0.clone().into()),
                fog,
                projection,
                Transform::from_translation(at).with_rotation(rotation),
            ));
        }
    }
}

pub struct MonitorPlugin;

impl Plugin for MonitorPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<MonitorCamera>()
            .add_systems(Update, drive_screen_camera.run_if(resource_exists::<Assets<Image>>));
    }
}
