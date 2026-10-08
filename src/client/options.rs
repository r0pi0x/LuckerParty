//! The options dialog's settings (`game_menu`'s Options page): which
//! cvar each control sets, on which tab, how its value steps and shows,
//! and the video cvars (`mat_vsync`, `mashup_fullscreen`,
//! `mashup_resolution`) that set the window.

use bevy::{
    prelude::*,
    window::{MonitorSelection, PresentMode, PrimaryMonitor, PrimaryWindow, VideoModeSelection, WindowMode},
};

use crate::console::{Console, resource_cvar};

/// The options dialog's tabs, in CS:S's order.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Tab {
    #[default]
    Keyboard,
    Mouse,
    Audio,
    Video,
    Multiplayer,
}

/// Every tab, with its label's token and our label.
pub const TABS: [(Tab, &str, &str); 5] = [
    (Tab::Keyboard, "#GameUI_Keyboard", "Keyboard"),
    (Tab::Mouse, "#GameUI_Mouse", "Mouse"),
    (Tab::Audio, "#GameUI_Audio", "Audio"),
    (Tab::Video, "#GameUI_Video", "Video"),
    (Tab::Multiplayer, "#GameUI_Multiplayer", "Multiplayer"),
];

impl Tab {
    pub fn index(self) -> usize {
        TABS.iter().position(|(t, ..)| *t == self).unwrap_or(0)
    }

    /// The tab `dir` steps away (wrapping).
    pub fn step(self, dir: i32) -> Tab {
        TABS[(self.index() as i32 + dir).rem_euclid(TABS.len() as i32) as usize].0
    }

    /// The game's layout page for it (`GameUi::options`).
    pub fn page(self) -> &'static str {
        match self {
            Tab::Keyboard => "keyboard",
            Tab::Mouse => "mouse",
            Tab::Audio => "audio",
            Tab::Video => "video",
            Tab::Multiplayer => "multiplayer",
        }
    }
}

/// How a setting's value changes.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum SettingKind {
    /// A slider: steps of `step` between `min` and `max`, shown with
    /// `decimals`.
    Range {
        min: f32,
        max: f32,
        step: f32,
        decimals: usize,
    },
    /// Fixed values (as the cvar takes them) with their labels (a `#token`
    /// for the game's text, then ours).
    Choice(&'static [(&'static str, &'static str)]),
    /// A check box: 0 or 1.
    Toggle,
    /// A check box that's on while the cvar is negative (`m_pitch`:
    /// reverse mouse); toggling negates it.
    Negate,
    /// The window size: the monitor's modes (`GameMenu::resolutions`).
    Resolution,
}

/// A setting on the options page: one cvar (archived, so config.cfg keeps
/// it).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Setting {
    pub tab: Tab,
    pub cvar: &'static str,
    /// The game's text for it (`#GameUI_...`), when it has one.
    pub token: Option<&'static str>,
    pub label: &'static str,
    pub kind: SettingKind,
}

const HDR_LEVELS: &[(&str, &str)] = &[("0", "None"), ("1", "Bloom"), ("2", "Full")];

pub const SETTINGS: &[Setting] = &[
    Setting {
        tab: Tab::Mouse,
        cvar: "sensitivity",
        token: Some("#GameUI_MouseSensitivity"),
        label: "Mouse sensitivity",
        kind: SettingKind::Range {
            min: 0.1,
            max: 20.0,
            step: 0.1,
            decimals: 1,
        },
    },
    Setting {
        tab: Tab::Mouse,
        cvar: "m_pitch",
        token: Some("#GameUI_ReverseMouse"),
        label: "Reverse mouse",
        kind: SettingKind::Negate,
    },
    Setting {
        tab: Tab::Mouse,
        cvar: "zoom_sensitivity_ratio",
        token: None,
        label: "Zoom sensitivity ratio",
        kind: SettingKind::Range {
            min: 0.1,
            max: 4.0,
            step: 0.1,
            decimals: 1,
        },
    },
    Setting {
        tab: Tab::Audio,
        cvar: "volume",
        token: Some("#GameUI_SoundEffectVolume"),
        label: "Game volume",
        kind: SettingKind::Range {
            min: 0.0,
            max: 1.0,
            step: 0.05,
            decimals: 2,
        },
    },
    Setting {
        tab: Tab::Audio,
        cvar: "dsp_volume",
        token: None,
        label: "Room reverb level",
        kind: SettingKind::Range {
            min: 0.0,
            max: 2.0,
            step: 0.1,
            decimals: 1,
        },
    },
    Setting {
        tab: Tab::Video,
        cvar: "mashup_resolution",
        token: Some("#GameUI_Resolution"),
        label: "Resolution",
        kind: SettingKind::Resolution,
    },
    Setting {
        tab: Tab::Video,
        cvar: "mashup_fullscreen",
        token: Some("#GameUI_DisplayMode"),
        label: "Display mode",
        kind: SettingKind::Choice(&[
            ("0", "#GameUI_Windowed|Windowed"),
            ("1", "#GameUI_Fullscreen|Full screen"),
            ("2", "Borderless full screen"),
        ]),
    },
    Setting {
        tab: Tab::Video,
        cvar: "mat_vsync",
        token: Some("#GameUI_Wait_For_VSync"),
        label: "Wait for vertical sync",
        kind: SettingKind::Toggle,
    },
    Setting {
        tab: Tab::Video,
        cvar: "mat_hdr_level",
        token: Some("#GameUI_HDR"),
        label: "High dynamic range (next map)",
        kind: SettingKind::Choice(HDR_LEVELS),
    },
    Setting {
        tab: Tab::Video,
        cvar: "cl_showfps",
        token: None,
        label: "Show FPS",
        kind: SettingKind::Choice(&[("0", "Off"), ("1", "On"), ("2", "Detailed")]),
    },
    Setting {
        tab: Tab::Multiplayer,
        cvar: "cl_crosshaircolor",
        token: None,
        label: "Crosshair colour",
        kind: SettingKind::Choice(&[
            ("0", "Green"),
            ("1", "Red"),
            ("2", "Blue"),
            ("3", "Yellow"),
            ("4", "Cyan"),
        ]),
    },
    Setting {
        tab: Tab::Multiplayer,
        cvar: "cl_crosshairscale",
        token: Some("#GameUI_CrosshairSize"),
        label: "Size",
        kind: SettingKind::Choice(&[("0", "Auto"), ("1200", "Small"), ("768", "Medium"), ("600", "Large")]),
    },
    Setting {
        tab: Tab::Multiplayer,
        cvar: "cl_crosshairusealpha",
        token: Some("#GameUI_CrosshairBlend"),
        label: "Translucent",
        kind: SettingKind::Toggle,
    },
    Setting {
        tab: Tab::Multiplayer,
        cvar: "cl_crosshairalpha",
        token: None,
        label: "Opacity",
        kind: SettingKind::Range {
            min: 0.0,
            max: 255.0,
            step: 5.0,
            decimals: 0,
        },
    },
    Setting {
        tab: Tab::Multiplayer,
        cvar: "cl_dynamiccrosshair",
        token: Some("#GameUI_CrosshairDynamic"),
        label: "Dynamic",
        kind: SettingKind::Toggle,
    },
    Setting {
        tab: Tab::Multiplayer,
        cvar: "cl_righthand",
        token: None,
        label: "Weapon hand",
        kind: SettingKind::Choice(&[("0", "Left"), ("1", "Right")]),
    },
    Setting {
        tab: Tab::Multiplayer,
        cvar: "viewmodel_fov",
        token: None,
        label: "View model FOV",
        kind: SettingKind::Range {
            min: 40.0,
            max: 90.0,
            step: 1.0,
            decimals: 0,
        },
    },
];

/// A choice label (`#Token|ours`): the game's text when it has the token,
/// else ours.
pub fn choice_label(label: &str, text: &dyn Fn(&str) -> Option<String>) -> String {
    match label.split_once('|') {
        Some((token, ours)) => text(token).unwrap_or_else(|| ours.to_string()),
        None => label.to_string(),
    }
}

impl Setting {
    /// The value `dir` steps from `current` (wrapping through choices,
    /// clamped in ranges, flipping check boxes); `choices` are the window
    /// sizes for `Resolution`.
    pub fn step(&self, current: &str, dir: i32, choices: &[String]) -> String {
        match self.kind {
            SettingKind::Range {
                min,
                max,
                step,
                decimals,
            } => {
                let v = current.trim().parse::<f32>().unwrap_or(min);
                // Onto the step grid, so 1.23 + 0.1 reads 1.3.
                let n = ((v - min) / step).round() + dir as f32;
                let v = (min + n * step).clamp(min, max);
                format!("{v:.decimals$}")
            }
            SettingKind::Choice(list) => {
                let i = list.iter().position(|(v, _)| *v == current.trim()).unwrap_or(0) as i32;
                let n = list.len() as i32;
                list[(i + dir).rem_euclid(n) as usize].0.to_string()
            }
            SettingKind::Toggle => if is_on(current) { "0" } else { "1" }.to_string(),
            SettingKind::Negate => {
                let v = current.trim().parse::<f32>().unwrap_or(0.022);
                format!("{}", -v)
            }
            SettingKind::Resolution => {
                if choices.is_empty() {
                    return current.to_string();
                }
                let i = choices.iter().position(|c| c == current.trim());
                let n = choices.len() as i32;
                let next = match i {
                    Some(i) => (i as i32 + dir).clamp(0, n - 1),
                    None if dir < 0 => n - 1,
                    None => 0,
                };
                choices[next as usize].clone()
            }
        }
    }

    /// The value as shown (`text` finds the game's strings).
    pub fn show(&self, value: &str, text: &dyn Fn(&str) -> Option<String>) -> String {
        match self.kind {
            SettingKind::Range { decimals, .. } => value
                .trim()
                .parse::<f32>()
                .map_or(value.to_string(), |v| format!("{v:.decimals$}")),
            SettingKind::Choice(list) => list
                .iter()
                .find(|(v, _)| *v == value.trim())
                .map_or(value.to_string(), |(_, l)| choice_label(l, text)),
            SettingKind::Toggle | SettingKind::Negate => {
                if self.checked(value) { "On" } else { "Off" }.to_string()
            }
            SettingKind::Resolution => {
                if value.trim().is_empty() {
                    "As started".to_string()
                } else {
                    value.trim().to_string()
                }
            }
        }
    }

    /// A check box's state.
    pub fn checked(&self, value: &str) -> bool {
        match self.kind {
            SettingKind::Negate => value.trim().parse::<f32>().is_ok_and(|v| v < 0.0),
            _ => is_on(value),
        }
    }

    /// A slider's position, 0 to 1.
    pub fn fraction(&self, value: &str) -> Option<f32> {
        match self.kind {
            SettingKind::Range { min, max, .. } => {
                let v = value.trim().parse::<f32>().ok()?;
                Some(((v - min) / (max - min)).clamp(0.0, 1.0))
            }
            _ => None,
        }
    }

    /// The value at a slider position (0 to 1), on the step grid.
    pub fn at_fraction(&self, f: f32) -> Option<String> {
        match self.kind {
            SettingKind::Range {
                min,
                max,
                step,
                decimals,
            } => {
                let v = min + ((f.clamp(0.0, 1.0) * (max - min)) / step).round() * step;
                Some(format!("{:.decimals$}", v.clamp(min, max)))
            }
            _ => None,
        }
    }
}

fn is_on(value: &str) -> bool {
    value.trim().parse::<f32>().is_ok_and(|v| v != 0.0)
}

// ---------------------------------------------------------------------------
// Video.

pub struct VideoPlugin;

impl Plugin for VideoPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<VideoSettings>()
            .add_systems(Update, apply_video);
        resource_cvar::<VideoSettings, u8>(
            app,
            "mat_vsync",
            "1: wait for vertical sync (no tearing, frames capped at the refresh rate).",
            |v| &mut v.vsync,
        );
        resource_cvar::<VideoSettings, u8>(
            app,
            "mashup_fullscreen",
            "Display mode: 0 windowed, 1 full screen, 2 borderless full screen.",
            |v| &mut v.fullscreen,
        );
        resource_cvar::<VideoSettings, String>(
            app,
            "mashup_resolution",
            "Window size in pixels, e.g. 1920x1080 (empty: as started).",
            |v| &mut v.resolution,
        );
        let mut console = app.world_mut().resource_mut::<Console>();
        for name in ["mat_vsync", "mashup_fullscreen", "mashup_resolution"] {
            console.archive(name);
        }
    }
}

/// The window's mode, size and vsync, as the options set them.
#[derive(Resource, Clone, Debug, PartialEq)]
pub struct VideoSettings {
    pub vsync: u8,
    pub fullscreen: u8,
    pub resolution: String,
}

impl Default for VideoSettings {
    fn default() -> Self {
        Self {
            vsync: 1,
            fullscreen: 0,
            resolution: String::new(),
        }
    }
}

/// `1920x1080` -> (1920, 1080).
pub fn parse_resolution(s: &str) -> Option<UVec2> {
    let (w, h) = s.trim().split_once(['x', 'X'])?;
    let size = UVec2::new(w.trim().parse().ok()?, h.trim().parse().ok()?);
    (size.x >= 320 && size.y >= 240).then_some(size)
}

/// The window sizes to offer: the primary monitor's modes, largest
/// first, else common ones.
pub fn resolutions(monitors: &[(UVec2, Vec<UVec2>)]) -> Vec<String> {
    let mut sizes: Vec<UVec2> = monitors.iter().flat_map(|(_, modes)| modes.iter().copied()).collect();
    if sizes.is_empty() {
        sizes = [
            (800, 600),
            (1024, 768),
            (1280, 720),
            (1280, 1024),
            (1366, 768),
            (1600, 900),
            (1920, 1080),
            (2560, 1440),
        ]
        .map(|(w, h)| UVec2::new(w, h))
        .to_vec();
        if let Some((size, _)) = monitors.first() {
            sizes.retain(|s| s.x <= size.x && s.y <= size.y);
            sizes.push(*size);
        }
    }
    sizes.retain(|s| s.x >= 640 && s.y >= 480);
    sizes.sort_by_key(|s| (std::cmp::Reverse(s.x), std::cmp::Reverse(s.y)));
    sizes.dedup();
    sizes.iter().map(|s| format!("{}x{}", s.x, s.y)).collect()
}

/// The primary monitor's size and modes (for `resolutions`).
pub fn monitor_modes(monitors: &Query<&bevy::window::Monitor, With<PrimaryMonitor>>) -> Vec<(UVec2, Vec<UVec2>)> {
    monitors
        .iter()
        .map(|m| {
            (
                UVec2::new(m.physical_width, m.physical_height),
                m.video_modes.iter().map(|v| v.physical_size).collect(),
            )
        })
        .collect()
}

/// Set the window as the video cvars say, when they change. Runs given a
/// window size (`--window`) or automated (screenshots, captures) leave the
/// window alone.
fn apply_video(
    settings: Res<VideoSettings>,
    args: Option<Res<super::ClientArgs>>,
    mut window: Query<&mut Window, With<PrimaryWindow>>,
    monitors: Query<&bevy::window::Monitor, With<PrimaryMonitor>>,
) {
    if !settings.is_changed() {
        return;
    }
    if args.is_some_and(|a| a.0.window.is_some() || a.0.screenshot.is_some() || a.0.views.is_some()) {
        return;
    }
    let Ok(mut window) = window.single_mut() else { return };
    let present = if settings.vsync != 0 {
        PresentMode::AutoVsync
    } else {
        PresentMode::AutoNoVsync
    };
    if window.present_mode != present {
        window.present_mode = present;
    }
    let size = parse_resolution(&settings.resolution);
    let mode = match settings.fullscreen {
        1 => {
            // The monitor's mode of that size, else its current one.
            let modes = monitors.iter().flat_map(|m| m.video_modes.iter()).filter(|v| Some(v.physical_size) == size);
            let best = modes.max_by_key(|v| (v.refresh_rate_millihertz, v.bit_depth)).copied();
            WindowMode::Fullscreen(
                MonitorSelection::Current,
                best.map_or(VideoModeSelection::Current, VideoModeSelection::Specific),
            )
        }
        2 => WindowMode::BorderlessFullscreen(MonitorSelection::Current),
        _ => WindowMode::Windowed,
    };
    if window.mode != mode {
        window.mode = mode;
    }
    if settings.fullscreen != 2
        && let Some(size) = size
        && window.resolution.physical_size() != size
    {
        window.resolution.set_physical_resolution(size.x, size.y);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn none(_: &str) -> Option<String> {
        None
    }

    fn setting(cvar: &str) -> Setting {
        *SETTINGS.iter().find(|s| s.cvar == cvar).unwrap()
    }

    #[test]
    fn steps_snap_to_the_grid() {
        let s = setting("sensitivity");
        assert_eq!(s.step("1.23", 1, &[]), "1.3");
        assert_eq!(s.step("0.1", -1, &[]), "0.1");
        assert_eq!(s.step("bad", 1, &[]), "0.2");
        let v = setting("volume");
        assert_eq!(v.step("0.5", 1, &[]), "0.55");
        assert_eq!(v.show("0.5", &none), "0.50");
        assert_eq!(v.fraction("0.25"), Some(0.25));
        assert_eq!(v.at_fraction(0.33).as_deref(), Some("0.35"));
    }

    #[test]
    fn check_boxes_and_reverse_mouse() {
        let invert = setting("m_pitch");
        assert!(!invert.checked("0.022"));
        assert_eq!(invert.step("0.022", 1, &[]), "-0.022");
        assert!(invert.checked("-0.022"));
        assert_eq!(invert.step("-0.022", -1, &[]), "0.022");
        let vsync = setting("mat_vsync");
        assert_eq!(vsync.step("1", 1, &[]), "0");
        assert_eq!(vsync.step("0", -1, &[]), "1");
    }

    #[test]
    fn choices_use_the_games_words_when_it_has_them() {
        let mode = setting("mashup_fullscreen");
        assert_eq!(mode.show("0", &none), "Windowed");
        let game = |t: &str| (t == "#GameUI_Windowed").then(|| "WINDOWED".to_string());
        assert_eq!(mode.show("0", &game), "WINDOWED");
        assert_eq!(mode.show("2", &game), "Borderless full screen");
    }

    #[test]
    fn resolutions_from_the_monitor_or_common_sizes() {
        let r = setting("mashup_resolution");
        let sizes = resolutions(&[(UVec2::new(1920, 1080), vec![UVec2::new(1280, 720), UVec2::new(1920, 1080)])]);
        assert_eq!(sizes, ["1920x1080", "1280x720"]);
        assert_eq!(r.step("", 1, &sizes), "1920x1080");
        assert_eq!(r.step("1920x1080", 1, &sizes), "1280x720");
        assert_eq!(r.step("1280x720", 1, &sizes), "1280x720", "stops at the end");
        let common = resolutions(&[(UVec2::new(1600, 900), vec![])]);
        assert_eq!(common.first().map(String::as_str), Some("1600x900"));
        assert!(!common.contains(&"1920x1080".to_string()));
        assert_eq!(parse_resolution("1280x720"), Some(UVec2::new(1280, 720)));
        assert_eq!(parse_resolution("tiny"), None);
    }

    #[test]
    fn every_setting_is_on_a_tab_other_than_the_keyboards() {
        assert!(SETTINGS.iter().all(|s| s.tab != Tab::Keyboard));
        for (tab, ..) in TABS.iter().skip(1) {
            assert!(SETTINGS.iter().any(|s| s.tab == *tab), "{tab:?} has settings");
        }
        assert_eq!(Tab::Keyboard.step(-1), Tab::Multiplayer);
    }
}
