//! Everything that needs a window: local input, the local player's camera,
//! debug tools and agent-facing tools (screenshots, remote inspection).
//! Simulation code must never depend on this module.

pub mod binds;
pub mod buy_menu;
pub mod capture;
pub mod chat;
pub mod console;
pub mod debug;
pub mod debug_ui;
pub mod debug_views;
pub mod effects;
pub mod fonts;
pub mod game_hud;
pub mod game_menu;
pub mod hdr;
pub mod hud_sprites;
pub mod hud;
pub mod input;
pub mod objectives_hud;
pub mod options;
pub mod perf;
pub mod radar;
pub mod radio;
pub mod game_text;
pub mod scoreboard;
pub mod senses;
pub mod spectate;
pub mod team_menu;
pub mod vgui;
pub mod view;
pub mod weapon_select;
pub mod window_icon;

use std::path::PathBuf;

use bevy::prelude::*;

use crate::{
    character::spawn_character,
    core::{Intent, LocalPlayer, MovementState, SpawnPoint, Team},
    slots::{Loadout, MovementRegistry},
};

/// Command-line options. Run with `--help` for the list.
#[derive(Clone, Default, Debug)]
pub struct Args {
    /// Save a screenshot here after `frames` frames, then exit.
    pub screenshot: Option<PathBuf>,
    pub frames: Option<u32>,
    /// Movement implementation ID for the local player (overrides the loadout).
    pub movement: Option<String>,
    /// Spawn position in meters, overriding the map's spawn points.
    pub spawn: Option<Vec3>,
    /// Initial look direction: yaw and pitch in degrees.
    pub look: Option<Vec2>,
    /// Map ID such as `cs_source:de_dust2`, or `greybox`. Without it the
    /// greybox is loaded behind the main menu (see `starts_in_game`).
    pub map: Option<String>,
    /// Debug view: white surfaces, only baked lighting.
    pub lightmap_only: bool,
    /// Debug render for comparisons: `lighting` (x0.25) or `albedo`, with no
    /// tonemapping and a magenta background.
    pub debug_view: Option<String>,
    /// Capture these views (JSON, see `capture::View`), one PNG each.
    pub views: Option<PathBuf>,
    pub capture_dir: Option<PathBuf>,
    /// With `views`: time each view instead of capturing it.
    pub bench: bool,
    /// Console variables to set (`name=value`), in order, after any
    /// `exec` files.
    pub cvars: Vec<(String, String)>,
    /// Source-style config files (`name value` per line) to apply.
    pub exec: Vec<PathBuf>,
    /// Console commands from `+name args...` (Source style), run at startup.
    pub console: Vec<String>,
    /// Start with the console open.
    pub console_open: bool,
    /// Window size in physical pixels (screenshots at a known size).
    pub window: Option<UVec2>,
}

const USAGE: &str = "\
usage: mashup [options]
  --screenshot <file.png>   save a screenshot after --frames frames, then exit
  --frames <n>              frames to run before the screenshot or exit (default 60)
  --movement <id>           movement implementation for the local player
  --spawn <x,y,z>           spawn position in meters
  --look <yaw,pitch>        initial look angles in degrees (yaw 0 = -Z)
  --map <game:name>         load a game's map, e.g. cs_source:de_dust2, or greybox (mashup's
                            test map). Without it the game starts at the main menu, unless
                            --spawn, --look, --movement or --views is given (then: greybox)
  --cvar <name=value>       set a movement console variable, e.g. sv_airaccelerate=150
                            (repeatable; cs_source: sv_accelerate, sv_airaccelerate,
                            sv_friction, sv_stopspeed, sv_gravity, sv_maxspeed,
                            sv_stepsize, sv_maxvelocity, sv_bounce,
                            sv_enablebunnyhopping, sv_autobunnyhopping, cl_forwardspeed)
  --exec <file.cfg>         apply a Source-style config file (\"name value\" lines)
  --lightmap-only           debug view: white surfaces, only baked lighting
  --debug-view <kind>       lighting (x0.25) | albedo, untonemapped, magenta background
  --views <file.json>       capture each view (name, position, yaw, pitch; engine space)
                            off-screen at 1280x720, then exit (use with --movement mashup:noclip)
  --capture-dir <dir>       where --views writes <name>.png (default: current directory)
  --bench                   with --views: time frames at each view (no vsync) and print
                            avg/p95/max frame ms and drawn meshes instead of capturing
  --console                 start with the console open (~ toggles it)
  --window <WxH>            window size in pixels, e.g. 1920x1080 (screenshots at a known size;
                            a tiling window manager may still resize it)
  +<command> [args...]      run a console command at startup, Source style
                            (e.g. +sv_airaccelerate 150 +cl_showpos 1 +bind f noclip;
                            ++attack holds an action, e.g. to fire in a --screenshot run)";

impl Args {
    /// Whether the run starts playing (a map given, or the player placed
    /// or its movement or views chosen: the greybox then) rather than at
    /// the main menu.
    pub fn starts_in_game(&self) -> bool {
        self.map.is_some() || self.spawn.is_some() || self.look.is_some() || self.movement.is_some() || self.views.is_some()
    }

    /// The game map to load at startup: `--map` unless it names the
    /// greybox (`greybox` or `mashup:greybox`), which is there anyway.
    pub fn game_map(&self) -> Option<&str> {
        self.map
            .as_deref()
            .filter(|m| !matches!(m.to_lowercase().as_str(), "greybox" | "mashup:greybox"))
    }

    pub fn parse() -> Self {
        Self::parse_from(std::env::args().skip(1)).unwrap_or_else(|e| {
            eprintln!("{e}\n\n{USAGE}");
            std::process::exit(2);
        })
    }

    pub fn parse_from(args: impl IntoIterator<Item = String>) -> Result<Self, String> {
        fn floats<const N: usize>(flag: &str, v: &str) -> Result<[f32; N], String> {
            let parts: Vec<f32> = v
                .split(',')
                .map(|p| p.trim().parse::<f32>())
                .collect::<Result<_, _>>()
                .map_err(|_| format!("{flag}: expected {N} comma-separated numbers, got {v:?}"))?;
            parts
                .try_into()
                .map_err(|_| format!("{flag}: expected {N} numbers, got {v:?}"))
        }

        let mut out = Self::default();
        let mut it = args.into_iter().peekable();
        while let Some(flag) = it.next() {
            // `+command args...`: words up to the next option or +command.
            if let Some(name) = flag.strip_prefix('+').filter(|n| !n.is_empty()) {
                let mut line = vec![crate::console::quote(name)];
                while let Some(next) = it.peek() {
                    if next.starts_with('+') && next.len() > 1 || next.starts_with("--") {
                        break;
                    }
                    line.push(crate::console::quote(&it.next().unwrap()));
                }
                out.console.push(line.join(" "));
                continue;
            }
            if flag == "--console" {
                out.console_open = true;
                continue;
            }
            if flag == "--help" || flag == "-h" {
                println!("{USAGE}");
                std::process::exit(0);
            }
            if flag == "--bench" {
                out.bench = true;
                continue;
            }
            if flag == "--lightmap-only" {
                out.lightmap_only = true;
                continue;
            }
            let value = it.next().ok_or_else(|| format!("{flag}: missing value"))?;
            match flag.as_str() {
                "--screenshot" => out.screenshot = Some(value.into()),
                "--frames" => out.frames = Some(value.parse().map_err(|_| format!("--frames: not a number: {value}"))?),
                "--movement" => out.movement = Some(value),
                "--map" => out.map = Some(value),
                "--views" => out.views = Some(value.into()),
                "--debug-view" => out.debug_view = Some(value),
                "--capture-dir" => out.capture_dir = Some(value.into()),
                "--spawn" => out.spawn = Some(Vec3::from_array(floats::<3>(&flag, &value)?)),
                "--look" => out.look = Some(Vec2::from_array(floats::<2>(&flag, &value)?)),
                "--cvar" => {
                    let (name, v) = value
                        .split_once('=')
                        .ok_or_else(|| format!("--cvar: expected name=value, got {value}"))?;
                    out.cvars.push((name.to_string(), v.to_string()));
                }
                "--exec" => out.exec.push(value.into()),
                "--window" => {
                    let size = value
                        .split_once(['x', 'X'])
                        .and_then(|(w, h)| Some(UVec2::new(w.trim().parse().ok()?, h.trim().parse().ok()?)))
                        .ok_or_else(|| format!("--window: expected WxH, got {value}"))?;
                    out.window = Some(size);
                }
                _ => return Err(format!("unknown option {flag}")),
            }
        }
        Ok(out)
    }
}

#[derive(Resource, Clone)]
pub struct ClientArgs(pub Args);

pub struct ClientPlugin {
    pub args: Args,
}

impl Plugin for ClientPlugin {
    fn build(&self, app: &mut App) {
        if let Some(map) = &self.args.map {
            app.insert_resource(crate::map::LoadedMapName(map.clone()));
        }
        app.insert_resource(ClientArgs(self.args.clone()))
            .add_plugins((
                fonts::FontsPlugin,
                input::LocalInputPlugin,
                debug::DebugPlugin,
                debug_views::DebugViewsPlugin,
                capture::CapturePlugin,
                perf::PerfPlugin,
                console::ConsoleUiPlugin,
                hud::HudPlugin,
                game_hud::GameHudPlugin,
                hud_sprites::HudSpritesPlugin,
                scoreboard::ScoreboardPlugin,
                game_text::GameTextPlugin,
                radar::RadarPlugin,
                effects::ShotEffectsPlugin,
                view::ViewPlugin,
            ))
            .add_plugins((
                buy_menu::BuyMenuPlugin,
                weapon_select::WeaponSelectPlugin,
                team_menu::TeamMenuPlugin,
                vgui::VguiPlugin,
                chat::ChatPlugin,
                radio::RadioPlugin,
                objectives_hud::ObjectivesHudPlugin,
                game_menu::GameMenuPlugin,
                senses::SensesPlugin,
                spectate::SpectatePlugin,
                hdr::HdrPlugin,
                options::VideoPlugin,
                window_icon::WindowIconPlugin,
            ))
            .add_systems(PostStartup, spawn_local_player)
            .add_systems(Update, (follow_eye, zoom_camera).after(spectate::SpectateSet));

        // Bevy Remote Protocol: query and edit the live ECS over HTTP
        // (JSON-RPC on localhost:15702). See docs/OBSERVABILITY.md.
        #[cfg(feature = "dev")]
        app.add_plugins((
            bevy::remote::RemotePlugin::default().with_method_main("mashup/console", console::remote_exec),
            bevy::remote::http::RemoteHttpPlugin::default(),
        ));
    }
}

#[derive(Component)]
pub struct FirstPersonCamera;

fn spawn_local_player(
    mut commands: Commands,
    spawns: Query<&Transform, With<SpawnPoint>>,
    loadout: Res<Loadout>,
    registry: Res<MovementRegistry>,
    args: Res<ClientArgs>,
) {
    let args = &args.0;
    let at = match args.spawn {
        Some(pos) => Transform::from_translation(pos),
        None => spawns.iter().next().copied().unwrap_or_default(),
    };
    // Face the way the spawn point does unless --look says otherwise. The
    // character itself is never rotated: the look angles turn the camera.
    let spawn_yaw = at.rotation.to_euler(EulerRot::YXZ).0.to_degrees();
    let at = Transform::from_translation(at.translation);
    let movement = match &args.movement {
        Some(id) => match registry.get(id) {
            Some(m) => m.id,
            None => {
                let known: Vec<_> = registry.0.iter().map(|m| m.id).collect();
                error!("--movement {id}: unknown. Registered: {known:?}");
                loadout.movement
            }
        },
        None => loadout.movement,
    };

    // Counter-terrorists by default (CS:S would ask; `jointeam 2` switches
    // to the terrorists).
    let player = spawn_character(&mut commands, at, Team(2), movement);
    let look = args.look.unwrap_or(Vec2::new(spawn_yaw, 0.0));
    commands
        .entity(player)
        .insert((
            LocalPlayer,
            Intent {
                yaw: look.x.to_radians(),
                pitch: look.y.to_radians(),
                ..default()
            },
        ))
        .with_child((
            FirstPersonCamera,
            // The local player's weapon is drawn here (map::view_model).
            crate::map::ViewModelAnchor,
            crate::map::sound::SoundListener,
            Camera3d::default(),
            Projection::Perspective(PerspectiveProjection {
                // Source's fov 90 (horizontal at 4:3), kept vertically:
                // 2 atan(3/4) = 73.74 degrees.
                fov: 2.0 * 0.75f32.atan(),
                ..default()
            }),
        ));
    // Follow the map's presentation (e.g. no tonemapping for Source LDR).
    commands.queue(|world: &mut World| {
        let Some(look) = world.get_resource::<crate::map::ActiveMapLook>().cloned() else {
            return;
        };
        if look.0.tonemapping {
            return;
        }
        let mut q = world.query_filtered::<Entity, With<FirstPersonCamera>>();
        let cams: Vec<Entity> = q.iter(world).collect();
        for cam in cams {
            world
                .entity_mut(cam)
                .insert(bevy::core_pipeline::tonemapping::Tonemapping::None);
        }
    });
    if args.debug_view.is_some() {
        // Raw values for analysis: no tonemapping, an unmistakable background.
        commands.queue(move |world: &mut World| {
            let mut q = world.query_filtered::<Entity, With<FirstPersonCamera>>();
            let cams: Vec<Entity> = q.iter(world).collect();
            for cam in cams {
                world.entity_mut(cam).insert((
                    bevy::core_pipeline::tonemapping::Tonemapping::None,
                    Camera {
                        clear_color: ClearColorConfig::Custom(Color::srgb(1.0, 0.0, 1.0)),
                        ..default()
                    },
                ));
            }
        });
    }
}

/// The first-person camera's field of view: Source's 90 (horizontal at
/// 4:3), or the zoom the local player looks through (`weapon::Zoomed`).
/// Kept vertically, so it is the same on every screen shape; the view
/// model and sky cameras follow it.
fn zoom_camera(
    player: Option<Single<Option<&crate::weapon::Zoomed>, With<LocalPlayer>>>,
    spectating: Res<spectate::SpecView>,
    zoomed: Query<&crate::weapon::Zoomed>,
    mut cameras: Query<&mut Projection, With<FirstPersonCamera>>,
) {
    // Spectating in first person: the target's zoom; otherwise none.
    let zoom = match spectate::target_zoom(&spectating, &zoomed) {
        Some(z) => z,
        None if spectating.pose.is_some() => None,
        None => player.and_then(|z| z.map(|z| z.fov)),
    };
    let fov_43 = zoom.unwrap_or(90.0);
    let fov = crate::map::view_model::vertical_fov(fov_43).to_radians();
    for mut projection in &mut cameras {
        if let Projection::Perspective(p) = projection.as_ref()
            && (p.fov - fov).abs() > 1e-6
            && let Projection::Perspective(p) = projection.as_mut()
        {
            p.fov = fov;
        }
    }
}

/// Place the camera at the movement implementation's eye position (or
/// behind it in third person) and aim it along the look angles.
#[allow(clippy::type_complexity)]
fn follow_eye(
    players: Query<
        (
            &Transform,
            &Intent,
            &MovementState,
            &Children,
            Option<&crate::weapon::ViewPunch>,
        ),
        (With<LocalPlayer>, Without<FirstPersonCamera>),
    >,
    mut cameras: Query<&mut Transform, With<FirstPersonCamera>>,
    shake: Res<senses::ViewShake>,
    mode: Res<view::CameraMode>,
    free: Res<input::FreeLook>,
    mut freecam: ResMut<view::FreeCam>,
    spatial: avian3d::prelude::SpatialQuery,
    characters: Query<Entity, With<Intent>>,
    watch: Res<view::Watch>,
    spectating: Res<spectate::SpecView>,
    others: Query<(&Name, &Transform, &Intent, &MovementState), (Without<LocalPlayer>, Without<FirstPersonCamera>)>,
) {
    // The watched character, if any: a chase camera behind it (world space).
    let watched = (watch.0 > 0)
        .then(|| format!("Bot {}", watch.0))
        .and_then(|name| others.iter().find(|(n, ..)| n.as_str() == name))
        .map(|(_, t, i, s)| {
            let chase = view::CameraMode {
                third_person: true,
                ..*mode
            };
            let look = view::camera_look(&chase, Quat::from_euler(EulerRot::YXZ, i.yaw, i.pitch, 0.0));
            let offset = view::camera_offset(&chase, t.translation, s.eye_offset, look, &spatial, &characters);
            (t.translation + offset, look)
        });
    for (at, intent, state, children, punch) in &players {
        // Recoil kicks the view (pitch up, yaw left).
        let p = punch.map_or(Vec2::ZERO, |p| p.0);
        // A hard landing rolls it (Source's roll turns it clockwise).
        let look = Quat::from_euler(
            EulerRot::YXZ,
            intent.yaw + p.y + free.yaw,
            intent.pitch + p.x + free.pitch,
            -state.view_roll,
        );
        let look = view::camera_look(&mode, look);
        let offset = view::camera_offset(&mode, at.translation, state.eye_offset, look, &spatial, &characters);
        // Detached: starts where the camera is; placed in the world (the
        // camera is the player's child, so undo the player's transform).
        // Spectating while dead comes first; then the debug cameras.
        let (offset, look) = if let Some((p, q)) = spectating.pose.or(watched) {
            let inv = at.compute_affine().inverse();
            (inv.transform_point3(p), at.rotation.inverse() * q)
        } else if freecam.mode == 0 {
            freecam.bypass_change_detection().at = None;
            (offset, look)
        } else {
            if freecam.at.is_none() {
                let (yaw, pitch, _) = look.to_euler(EulerRot::YXZ);
                freecam.at = Some((*at * offset, yaw, pitch));
            }
            let (p, q) = freecam.rotation().unwrap();
            let inv = at.compute_affine().inverse();
            (inv.transform_point3(p), at.rotation.inverse() * q)
        };
        let mut cams = cameras.iter_many_mut(children);
        // Blasts shake the eye's view (not a detached camera's).
        let (offset, look) = if watched.is_none() && freecam.mode == 0 && spectating.pose.is_none() {
            (offset + shake.offset, look * Quat::from_rotation_z(shake.roll))
        } else {
            (offset, look)
        };
        while let Some(mut cam) = cams.fetch_next() {
            cam.translation = offset;
            cam.rotation = look;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(line: &str) -> Args {
        Args::parse_from(line.split_whitespace().map(String::from)).unwrap()
    }

    #[test]
    fn the_main_menu_unless_the_run_starts_playing() {
        assert!(!args("").starts_in_game());
        assert!(!args("--screenshot a.png --window 1280x720 +menu options").starts_in_game());
        assert!(args("--map cs_source:de_dust2").starts_in_game());
        assert!(args("--spawn 0,1,12").starts_in_game());
        assert!(args("--movement mashup:noclip").starts_in_game());
        // The greybox is a map name; no game map to load for it.
        let a = args("--map greybox");
        assert!(a.starts_in_game() && a.game_map().is_none());
        assert_eq!(args("--map cs_source:de_nuke").game_map(), Some("cs_source:de_nuke"));
    }
}
