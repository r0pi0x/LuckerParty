//! Monitors (Source's func_monitor showing a point_camera's view, public
//! entity docs), for any game: the map's monitor screen texture
//! (`MapData::camera_texture`) is a render target that a camera of our
//! own draws into from the active camera's pose (`MonitorCamera`, which
//! the logic writes), only while one is active. What the camera sees is
//! culled for the player's own view (`vis`), as the rest of the map.

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
#[derive(Resource, Clone, Copy, Debug, Default, PartialEq)]
pub struct MonitorCamera(pub Option<(Vec3, Quat, f32)>);

/// Our camera drawing into `MonitorScreen`.
#[derive(Component)]
pub struct ScreenCamera;

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
fn drive_screen_camera(
    screen: Option<Res<MonitorScreen>>,
    want: Res<MonitorCamera>,
    mut cameras: Query<(Entity, &mut Camera, &mut Transform, &mut Projection), With<ScreenCamera>>,
    mut commands: Commands,
) {
    let Some(screen) = screen else {
        for (e, ..) in &cameras {
            commands.entity(e).despawn();
        }
        return;
    };
    let Some((at, rotation, fov)) = want.0 else {
        for (_, mut c, ..) in &mut cameras {
            if c.is_active {
                c.is_active = false;
            }
        }
        return;
    };
    let projection = Projection::Perspective(PerspectiveProjection {
        fov: fov.clamp(1.0, 170.0).to_radians(),
        aspect_ratio: 1.0,
        ..default()
    });
    match cameras.single_mut() {
        Ok((_, mut c, mut t, mut p)) => {
            if !c.is_active {
                c.is_active = true;
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
                    // Before the main view, which shows its picture.
                    order: -1,
                    clear_color: Color::BLACK.into(),
                    ..default()
                },
                RenderTarget::Image(screen.0.clone().into()),
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
