//! Player-facing view settings and debug views: the third-person camera
//! (`thirdperson` / `firstperson`, `cam_idealdist`), the master `volume`,
//! and health bars over characters (`mashup_healthbars`).

use std::collections::HashMap;

use avian3d::prelude::*;
use bevy::{
    audio::{GlobalVolume, Volume},
    prelude::*,
};

use super::FirstPersonCamera;
use crate::{
    console::{Console, ConsoleAppExt, resource_cvar},
    core::{Health, Intent, LocalPlayer},
    map::ShowLocalBody,
    rules::Dead,
};

/// Source units to meters (cam_idealdist is in CS:S units).
const METERS_PER_UNIT: f32 = 0.0254;
/// Radius of the sphere swept back from the eye to place the camera, so
/// it stops short of walls (CS:S sweeps a 28-unit box).
const CAMERA_RADIUS: f32 = 0.2;

/// Sound volume a fresh config starts with (`volume`).
pub const DEFAULT_VOLUME: f32 = 0.5;

/// First or third person, and how far behind the eye the third-person
/// camera sits.
#[derive(Resource, Clone, Copy, Debug, PartialEq)]
pub struct CameraMode {
    pub third_person: bool,
    /// CS:S units (cam_idealdist).
    pub ideal_dist: f32,
    /// Degrees the third-person camera orbits around the player
    /// (cam_idealyaw; 180 looks at your front).
    pub ideal_yaw: f32,
    /// CS:S units the third-person camera's pivot sits above the eye
    /// (`THIRD_PERSON_PIVOT_UP`; the spectator's chase camera has none).
    pub pivot_up: f32,
}

/// How far above the eye CS:S's third-person camera turns about, units:
/// none. CS:S with `thirdperson; cam_idealdist 150` at `setang 0` puts its
/// camera (`getpos`) at the eye's height, 150 units straight back
/// (de_dust2 CT spawn, 2026-10-09). An earlier capture that seemed to need
/// a raise was taken while CS:S's camera was still easing from a steeper
/// view (its angles lag the view's unless `cam_snapto 1`; we don't lag).
pub const THIRD_PERSON_PIVOT_UP: f32 = 0.0;

impl Default for CameraMode {
    fn default() -> Self {
        Self {
            third_person: false,
            ideal_dist: 150.0,
            ideal_yaw: 0.0,
            pivot_up: THIRD_PERSON_PIVOT_UP,
        }
    }
}

/// A camera detached from the local player (`mashup_freecam`), to watch
/// your own body (or bots) from outside: 1 flies it (WASD, mouse, space
/// and ctrl for up and down, shift for speed) while the player stands
/// still; 2 leaves it where it is and gives the controls back to the
/// player. 0 puts it back on the eye.
#[derive(Resource, Clone, Copy, Debug, Default, PartialEq)]
pub struct FreeCam {
    pub mode: u8,
    /// World position and look (yaw, pitch, radians) while detached; taken
    /// from the camera when it detaches.
    pub at: Option<(Vec3, f32, f32)>,
}

impl FreeCam {
    /// Fly: `input` is (right, forward, up) in -1..1.
    pub fn fly(&mut self, input: Vec3, turn: Vec2, speed: f32, dt: f32) {
        let Some((p, yaw, pitch)) = &mut self.at else { return };
        *yaw = (*yaw + turn.x).rem_euclid(std::f32::consts::TAU);
        *pitch = (*pitch + turn.y).clamp(-89f32.to_radians(), 89f32.to_radians());
        let look = Quat::from_euler(EulerRot::YXZ, *yaw, *pitch, 0.0);
        *p += (look * Vec3::X * input.x + look * Vec3::NEG_Z * input.y + Vec3::Y * input.z) * speed * dt;
    }

    pub fn rotation(&self) -> Option<(Vec3, Quat)> {
        self.at
            .map(|(p, yaw, pitch)| (p, Quat::from_euler(EulerRot::YXZ, yaw, pitch, 0.0)))
    }
}

/// Watch another character from behind (`mashup_watch <n>`: bot n, 0
/// off), at `cam_idealdist` and `cam_idealyaw`, like a spectator's chase
/// camera; your own player keeps its controls.
#[derive(Resource, Clone, Copy, Debug, Default, PartialEq)]
pub struct Watch(pub u32);

#[derive(Resource, Default)]
struct HealthBars(u8);

pub struct ViewPlugin;

impl Plugin for ViewPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<CameraMode>()
            .insert_resource(GlobalVolume::new(Volume::Linear(DEFAULT_VOLUME)))
            .init_resource::<FreeCam>()
            .init_resource::<Watch>()
            .init_resource::<HealthBars>()
            .init_resource::<ShowLocalBody>()
            .add_systems(Update, (show_local_body, draw_health_bars.after(super::follow_eye)));
        view_console(app);
    }
}

fn view_console(app: &mut App) {
    app.console_command(
        "thirdperson",
        "Camera behind the player (cam_idealdist), showing your own body.",
        |w, _| {
            w.resource_mut::<CameraMode>().third_person = true;
            Ok(None)
        },
    )
    .console_command("firstperson", "Camera back at the eyes.", |w, _| {
        w.resource_mut::<CameraMode>().third_person = false;
        Ok(None)
    })
    .console_cvar(
        "volume",
        "Sound volume, 0 to 1.",
        "0.5",
        |w| {
            Some(
                w.get_resource::<GlobalVolume>()
                    .map_or(1.0, |g| g.volume.to_linear())
                    .to_string(),
            )
        },
        |w, v| {
            let v: f32 = v.trim().parse().map_err(|_| format!("bad value \"{v}\""))?;
            w.insert_resource(GlobalVolume::new(Volume::Linear(v.clamp(0.0, 1.0))));
            Ok(())
        },
    );
    resource_cvar::<CameraMode, f32>(
        app,
        "cam_idealdist",
        "Third-person camera distance behind the eye, in CS:S units.",
        |m| &mut m.ideal_dist,
    );
    resource_cvar::<Watch, u32>(
        app,
        "mashup_watch",
        "Chase camera on bot n (\"Bot n\"), behind it at cam_idealdist/cam_idealyaw; 0 back to you.",
        |w| &mut w.0,
    );
    resource_cvar::<FreeCam, u8>(
        app,
        "mashup_freecam",
        "Detached camera: 1 fly it (player stands still), 2 keep it there and control the player, 0 back to the eye.",
        |f| &mut f.mode,
    );
    resource_cvar::<CameraMode, f32>(
        app,
        "cam_idealyaw",
        "Third-person camera orbit around the player, in degrees (180: from the front).",
        |m| &mut m.ideal_yaw,
    );
    resource_cvar::<HealthBars, u8>(
        app,
        "mashup_healthbars",
        "1: health bars above other living characters.",
        |h| &mut h.0,
    );
    app.world_mut().resource_mut::<Console>().archive("volume");
}

/// Where the third-person camera sits relative to the eye: `ideal` meters
/// back along the view, or at `hit` meters if something is in the way.
pub fn third_person_offset(look: Quat, ideal: f32, hit: Option<f32>) -> Vec3 {
    let d = hit.map_or(ideal, |h| h.min(ideal)).max(0.0);
    look * Vec3::Z * d
}

/// The camera's view direction: the look, turned by `cam_idealyaw` in
/// third person.
pub fn camera_look(mode: &CameraMode, look: Quat) -> Quat {
    if mode.third_person {
        Quat::from_rotation_y(mode.ideal_yaw.to_radians()) * look
    } else {
        look
    }
}

/// The camera's offset from the character's origin: the eye in first
/// person; in third person, behind it (along `look`, from `camera_look`),
/// pulled in by a sweep against everything but characters.
/// Clearance kept between the eye and a ceiling just above it: Bevy's
/// near plane (0.1 m) plus a little, so the near plane never cuts into it.
const CEILING_CLEARANCE: f32 = 0.12;

/// The eye, lowered if a ceiling sits closer above it than
/// `CEILING_CLEARANCE`. The measured CS:S box (62 units, eye at 64) fits a
/// 64-unit duct like de_nuke's vents standing, with the eye on the ceiling
/// plane: the near plane then cut into the ceiling and the map showed
/// through it.
fn below_ceiling(
    origin: Vec3,
    eye: Vec3,
    spatial: &SpatialQuery,
    characters: impl IntoIterator<Item = Entity>,
) -> Vec3 {
    let filter = SpatialQueryFilter::from_excluded_entities(characters).with_mask(crate::core::SOLID_LAYERS);
    let reach = eye.y + CEILING_CLEARANCE;
    if reach <= 0.0 {
        return eye;
    }
    let hit = spatial.cast_ray(origin, Dir3::Y, reach, true, &filter).map(|h| h.distance);
    eye.with_y(eye_under_ceiling(eye.y, hit))
}

/// The eye's height above the origin with a ceiling `hit` meters above it.
fn eye_under_ceiling(eye: f32, hit: Option<f32>) -> f32 {
    match hit {
        Some(d) if d < eye + CEILING_CLEARANCE => (d - CEILING_CLEARANCE).max(0.0).min(eye),
        _ => eye,
    }
}

pub fn camera_offset(
    mode: &CameraMode,
    origin: Vec3,
    eye: Vec3,
    look: Quat,
    spatial: &SpatialQuery,
    characters: impl IntoIterator<Item = Entity>,
) -> Vec3 {
    if !mode.third_person {
        return below_ceiling(origin, eye, spatial, characters);
    }
    let ideal = mode.ideal_dist.max(0.0) * METERS_PER_UNIT;
    let Ok(dir) = Dir3::new(look * Vec3::Z) else {
        return eye;
    };
    let filter = SpatialQueryFilter::from_excluded_entities(characters).with_mask(crate::core::SOLID_LAYERS);
    let sweep = |from: Vec3, dir: Dir3, max_distance: f32| {
        let config = ShapeCastConfig {
            max_distance,
            ignore_origin_penetration: true,
            ..default()
        };
        spatial
            .cast_shape(&Collider::sphere(CAMERA_RADIUS), from, Quat::IDENTITY, dir, &config, &filter)
            .map(|h| h.distance)
    };
    // The pivot: up from the eye, short of a ceiling.
    let up = mode.pivot_up.max(0.0) * METERS_PER_UNIT;
    let pivot = eye + Vec3::Y * up.min(sweep(origin + eye, Dir3::Y, up).unwrap_or(up));
    let hit = sweep(origin + pivot, dir, ideal);
    pivot + third_person_offset(look, ideal, hit)
}

/// Draw the local player's body in third person while alive.
fn show_local_body(
    mode: Res<CameraMode>,
    free: Res<FreeCam>,
    watch: Res<Watch>,
    local: Option<Single<(Has<Dead>, Has<crate::core::MapView>), With<LocalPlayer>>>,
    mut show: ResMut<ShowLocalBody>,
) {
    let (dead, camera) = local.map_or((false, false), |l| *l);
    // Viewing through a map camera (point_viewcontrol), the body stands
    // where it was and no view model is drawn.
    show.set_if_neq(ShowLocalBody(
        (mode.third_person || free.mode != 0 || watch.0 != 0 || camera) && !dead,
    ));
}

#[derive(Component)]
struct HealthBar;

const BAR_WIDTH: f32 = 40.0;
const BAR_HEIGHT: f32 = 7.0;
/// Meters above the character's origin (its centre).
const BAR_ABOVE: f32 = crate::character::CAPSULE_HEIGHT / 2.0 + 0.25;

/// `mashup_healthbars 1`: a bar over each living character other than the
/// local player, green when full, red when nearly dead.
#[allow(clippy::type_complexity)]
fn draw_health_bars(
    on: Res<HealthBars>,
    player: Option<Single<&Transform, With<LocalPlayer>>>,
    camera: Query<(&Camera, &Transform), With<FirstPersonCamera>>,
    characters: Query<(Entity, &Transform, &Health, Has<Dead>), (With<Intent>, Without<LocalPlayer>)>,
    mut nodes: Query<(&mut Node, &mut Visibility), With<HealthBar>>,
    mut fills: Query<(&mut Node, &mut BackgroundColor), Without<HealthBar>>,
    mut bars: Local<HashMap<Entity, (Entity, Entity)>>,
    mut commands: Commands,
) {
    if on.0 == 0 {
        for (_, (root, _)) in bars.drain() {
            commands.entity(root).despawn();
        }
        return;
    }
    // The camera as it will render this frame (its transform was just set).
    let view = match (player, camera.iter().next()) {
        (Some(p), Some((c, t))) => Some((c, GlobalTransform::from(p.mul_transform(*t)))),
        _ => None,
    };
    bars.retain(|owner, (root, _)| {
        let alive = characters.contains(*owner);
        if !alive {
            commands.entity(*root).despawn();
        }
        alive
    });
    for (e, t, health, dead) in &characters {
        let Some(&(root, fill)) = bars.get(&e) else {
            let root = commands
                .spawn((
                    HealthBar,
                    Node {
                        position_type: PositionType::Absolute,
                        width: px(BAR_WIDTH),
                        height: px(BAR_HEIGHT),
                        border: UiRect::all(px(1.0)),
                        ..default()
                    },
                    BackgroundColor(Color::srgba(0.0, 0.0, 0.0, 0.7)),
                    Visibility::Hidden,
                ))
                .id();
            let fill = commands
                .spawn((
                    Node {
                        height: percent(100.0),
                        ..default()
                    },
                    BackgroundColor(Color::NONE),
                    ChildOf(root),
                ))
                .id();
            bars.insert(e, (root, fill));
            continue;
        };
        let Ok((mut node, mut vis)) = nodes.get_mut(root) else {
            continue;
        };
        let at = t.translation + Vec3::Y * BAR_ABOVE;
        let screen = view
            .as_ref()
            .filter(|_| !dead && health.current > 0.0)
            .and_then(|(c, g)| c.world_to_viewport(g, at).ok());
        let Some(p) = screen else {
            vis.set_if_neq(Visibility::Hidden);
            continue;
        };
        vis.set_if_neq(Visibility::Visible);
        node.left = px(p.x - BAR_WIDTH / 2.0);
        node.top = px(p.y - BAR_HEIGHT);
        if let Ok((mut fill_node, mut color)) = fills.get_mut(fill) {
            let f = (health.current / health.max.max(1e-6)).clamp(0.0, 1.0);
            fill_node.width = percent(f * 100.0);
            color.0 = Color::srgb(1.0 - f, f, 0.1);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn free_camera_flies_along_its_view() {
        let mut f = FreeCam {
            mode: 1,
            at: Some((Vec3::ZERO, 0.0, 0.0)),
        };
        // Yaw 0 faces -Z: forward for one second at 2 m/s.
        f.fly(Vec3::new(0.0, 1.0, 0.0), Vec2::ZERO, 2.0, 1.0);
        assert!(f.at.unwrap().0.distance(Vec3::new(0.0, 0.0, -2.0)) < 1e-5);
        // Up is world up whatever the pitch.
        f.fly(Vec3::Z, Vec2::new(0.0, 1.0), 1.0, 1.0);
        assert!((f.at.unwrap().0.y - 1.0).abs() < 1e-5);
        assert!((f.at.unwrap().2 - 1.0).abs() < 1e-6);
    }

    fn app() -> App {
        let mut app = App::new();
        app.add_plugins(crate::console::ConsolePlugin)
            .init_resource::<CameraMode>()
            .init_resource::<HealthBars>();
        view_console(&mut app);
        app
    }

    fn run(app: &mut App, line: &str) {
        app.world_mut().resource_mut::<Console>().submit(line);
        app.update();
    }

    #[test]
    fn thirdperson_and_firstperson_toggle_the_camera() {
        let mut app = app();
        assert!(!app.world().resource::<CameraMode>().third_person);
        run(&mut app, "thirdperson");
        assert!(app.world().resource::<CameraMode>().third_person);
        run(&mut app, "cam_idealdist 100; firstperson");
        let mode = *app.world().resource::<CameraMode>();
        assert!(!mode.third_person);
        assert_eq!(mode.ideal_dist, 100.0);
        run(&mut app, "cam_idealyaw 180");
        assert_eq!(app.world().resource::<CameraMode>().ideal_yaw, 180.0);
    }

    #[test]
    fn volume_is_clamped_and_archived() {
        let mut app = app();
        run(&mut app, "volume 0.25");
        let v = app.world().resource::<GlobalVolume>().volume.to_linear();
        assert!((v - 0.25).abs() < 1e-6, "{v}");
        run(&mut app, "volume 3");
        assert_eq!(app.world().resource::<GlobalVolume>().volume.to_linear(), 1.0);
        run(&mut app, "volume -1");
        assert_eq!(app.world().resource::<GlobalVolume>().volume.to_linear(), 0.0);
        assert!(app.world().resource::<Console>().cvar("volume").unwrap().archive);
    }

    #[test]
    fn the_eye_stays_below_a_low_ceiling() {
        let u = METERS_PER_UNIT;
        // A 64-unit duct: origin 31 units up (box centre), eye 33 above it,
        // ceiling 33 above it: the eye drops to keep the near plane clear.
        let eye = eye_under_ceiling(33.0 * u, Some(33.0 * u));
        assert!((eye - (33.0 * u - CEILING_CLEARANCE)).abs() < 1e-6, "{eye}");
        // Open sky or a high ceiling: unchanged.
        assert_eq!(eye_under_ceiling(33.0 * u, None), 33.0 * u);
        assert_eq!(eye_under_ceiling(33.0 * u, Some(2.0)), 33.0 * u);
    }

    #[test]
    fn third_person_camera_sits_behind_and_stops_at_walls() {
        // Looking down -Z (yaw 0): behind is +Z.
        let look = Quat::IDENTITY;
        let ideal = 150.0 * METERS_PER_UNIT;
        let free = third_person_offset(look, ideal, None);
        assert!((free - Vec3::Z * ideal).length() < 1e-5, "{free}");
        // A wall 1 m back pulls the camera in to it.
        assert!((third_person_offset(look, ideal, Some(1.0)) - Vec3::Z).length() < 1e-5);
        // Looking down 45 degrees, the camera is behind and above.
        let down = Quat::from_euler(EulerRot::YXZ, 0.0, -45f32.to_radians(), 0.0);
        let o = third_person_offset(down, ideal, None);
        assert!(o.y > 0.0 && o.z > 0.0, "{o}");
    }
}
