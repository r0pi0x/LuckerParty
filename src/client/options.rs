//! The options dialog's settings (`game_menu`'s Options page): which
//! cvar each control sets, on which tab, how its value steps and shows,
//! and the video cvars (`mat_vsync`, `mashup_fullscreen`,
//! `mashup_resolution`) that set the window, and `mat_antialias` (MSAA).

use bevy::{
    prelude::*,
    window::{MonitorSelection, PresentMode, PrimaryMonitor, PrimaryWindow, VideoModeSelection, WindowMode},
};

use crate::console::{Console, ConsoleAppExt, resource_cvar};

/// The options dialog's tabs, in CS:S's order.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Tab {
    #[default]
    Keyboard,
    Mouse,
    Audio,
    Video,
    Voice,
    Multiplayer,
}

/// Every tab, with its label's token and our label.
pub const TABS: [(Tab, &str, &str); 6] = [
    (Tab::Keyboard, "#GameUI_Keyboard", "Keyboard"),
    (Tab::Mouse, "#GameUI_Mouse", "Mouse"),
    (Tab::Audio, "#GameUI_Audio", "Audio"),
    (Tab::Video, "#GameUI_Video", "Video"),
    (Tab::Voice, "#GameUI_Voice", "Voice"),
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
            Tab::Voice => "voice",
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
    /// A check box that's on while the cvar isn't 0 and sets it to this
    /// value when ticked (`m_customaccel`: 3).
    ToggleTo(&'static str),
    /// A check box that's on while the cvar is negative (`m_pitch`:
    /// reverse mouse); toggling negates it.
    Negate,
    /// The window size: the monitor's modes (`GameMenu::resolutions`).
    Resolution,
}

/// Where a setting is shown: an options tab, one of the options' own
/// dialogs, or ours (settings CS:S's options don't have, kept out of its
/// dialogs: the main menu's "Lucker Party Options").
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Place {
    Options(Tab),
    /// The keyboard tab's Advanced dialog.
    KeyboardAdvanced,
    /// The video tab's Advanced dialog.
    VideoAdvanced,
    /// The video tab's brightness dialog (Adjust brightness levels...).
    Gamma,
    Extras,
}

/// A setting on the options page: one cvar (archived, so config.cfg keeps
/// it).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Setting {
    pub place: Place,
    pub cvar: &'static str,
    /// The control of the game's layout that shows it (its `fieldName` in
    /// `OptionsSub*.res`); None: placed in a column (ours, or without the
    /// install's layouts).
    pub field: Option<&'static str>,
    /// The game's text for it (`#GameUI_...`), when it has one.
    pub token: Option<&'static str>,
    pub label: &'static str,
    pub kind: SettingKind,
}

/// A check box setting shown in a combo box (the video Advanced dialog's
/// Wait for vertical sync).
pub const ON_OFF: &[(&str, &str)] = &[("0", "#GameUI_Disabled|Disabled"), ("1", "#GameUI_Enabled|Enabled")];

const HDR_LEVELS: &[(&str, &str)] = &[
    ("0", "#GameUI_hdr_level0|None"),
    ("1", "#GameUI_hdr_level1|Bloom"),
    ("2", "#GameUI_hdr_level2|Full"),
];

const fn setting(
    place: Place,
    cvar: &'static str,
    field: Option<&'static str>,
    token: Option<&'static str>,
    label: &'static str,
    kind: SettingKind,
) -> Setting {
    Setting {
        place,
        cvar,
        field,
        token,
        label,
        kind,
    }
}

const fn range(min: f32, max: f32, step: f32, decimals: usize) -> SettingKind {
    SettingKind::Range {
        min,
        max,
        step,
        decimals,
    }
}

use Place::{Extras, Gamma, KeyboardAdvanced, Options, VideoAdvanced};

/// Every setting, in each place's order (the game's layouts place those
/// with a `field`; a setting whose cvar mashup lacks shows greyed).
/// Defaults are CS:S's (each cvar's own registration); slider ranges are
/// the layout's (`minvalue`, `maxvalue`) where it gives them, else ours
/// (CS:S sets them in code: docs/plans/active/ui-parity.md lists those
/// a reference capture should confirm).
pub const SETTINGS: &[Setting] = &[
    // Mouse (`OptionsSubMouse.res`): the slider and the text entry beside
    // it (`VALUE_ENTRIES`) both show the sensitivity.
    setting(Options(Tab::Mouse), "m_pitch", Some("ReverseMouse"), Some("#GameUI_ReverseMouse"), "Reverse mouse", SettingKind::Negate),
    setting(Options(Tab::Mouse), "m_filter", Some("MouseFilter"), Some("#GameUI_MouseFilter"), "Mouse filter", SettingKind::Toggle),
    setting(Options(Tab::Mouse), "m_rawinput", Some("MouseRaw"), Some("#GameUI_MouseRaw"), "Raw mouse input", SettingKind::Toggle),
    setting(
        Options(Tab::Mouse),
        "m_customaccel",
        Some("MouseAccelerationCheckbox"),
        Some("#GameUI_MouseAcceleration"),
        "Mouse acceleration",
        SettingKind::ToggleTo("3"),
    ),
    setting(Options(Tab::Mouse), "sensitivity", Some("Slider"), Some("#GameUI_MouseSensitivity"), "Mouse sensitivity", range(0.1, 20.0, 0.1, 1)),
    setting(
        Options(Tab::Mouse),
        "m_customaccel_exponent",
        Some("MouseAccelerationSlider"),
        None,
        "Mouse acceleration amount",
        range(1.0, 1.4, 0.01, 2),
    ),
    // Audio (`OptionsSubAudio.res`).
    setting(Options(Tab::Audio), "volume", Some("SFXSlider"), Some("#GameUI_SoundEffectVolume"), "Game volume", range(0.0, 1.0, 0.05, 2)),
    setting(Options(Tab::Audio), "snd_musicvolume", Some("MusicSlider"), Some("#GameUI_MusicVolume"), "Music volume", range(0.0, 1.0, 0.05, 2)),
    setting(
        Options(Tab::Audio),
        "snd_mute_losefocus",
        Some("snd_mute_losefocus"),
        Some("#GameUI_SndMuteLoseFocus"),
        "Mute sound when the game loses focus",
        SettingKind::Toggle,
    ),
    // Its drop-downs, in CS:S's words; the values CS:S's code sets (the
    // sound quality and captioning each set two cvars). Kept and saved;
    // what each changes in mashup: `client::audio::SoundOptions`.
    setting(
        Options(Tab::Audio),
        "snd_surround_speakers",
        Some("SpeakerSetup"),
        Some("#GameUI_SpeakerConfiguration"),
        "Speaker configuration",
        SettingKind::Choice(&[
            ("0", "#GameUI_Headphones|Headphones"),
            ("2", "#GameUI_2Speakers|2 Speakers"),
            ("4", "#GameUI_4Speakers|4 Speakers"),
            ("5", "#GameUI_5Speakers|5.1 Speakers"),
            ("7", "#GameUI_7Speakers|7.1 Speakers"),
        ]),
    ),
    setting(
        Options(Tab::Audio),
        "snd_pitchquality dsp_slow_cpu",
        Some("SoundQuality"),
        Some("#GameUI_SoundQuality"),
        "Sound quality",
        SettingKind::Choice(&[("0 1", "#GameUI_Low|Low"), ("0 0", "#GameUI_Medium|Medium"), ("1 0", "#GameUI_High|High")]),
    ),
    setting(
        Options(Tab::Audio),
        "closecaption cc_subtitles",
        Some("CloseCaptionCheck"),
        Some("#GameUI_Captioning"),
        "Captioning",
        SettingKind::Choice(&[
            ("0 0", "#GameUI_NoClosedCaptions|No captions"),
            ("1 1", "#GameUI_Subtitles|Subtitles (dialog only)"),
            ("1 0", "#GameUI_SubtitlesAndSoundEffects|Closed Captions"),
        ]),
    ),
    setting(
        Options(Tab::Audio),
        "mashup_spoken_language",
        Some("AudioSpokenLanguage"),
        Some("#GAMEUI_AudioSpokenLanguage"),
        "Audio (spoken) language",
        SettingKind::Choice(&[("english", "#GameUI_Language_English|English")]),
    ),
    // Video (`OptionsSubVideo.res`).
    setting(Options(Tab::Video), "mashup_resolution", Some("Resolution"), Some("#GameUI_Resolution"), "Resolution", SettingKind::Resolution),
    setting(
        Options(Tab::Video),
        "mashup_fullscreen",
        Some("DisplayModeCombo"),
        Some("#GameUI_DisplayMode"),
        "Display mode",
        SettingKind::Choice(&[
            ("1", "#GameUI_Fullscreen|Fullscreen"),
            ("0", "#GameUI_Windowed|Windowed"),
            ("2", "Borderless fullscreen"),
        ]),
    ),
    // Its Advanced dialog (`OptionsSubVideoAdvancedDlg.res`).
    setting(
        VideoAdvanced,
        "mat_antialias",
        Some("AntialiasingMode"),
        Some("#GameUI_Antialiasing_Mode"),
        "Antialiasing mode",
        SettingKind::Choice(&[("0", "#GameUI_None|None"), ("2", "2x MSAA"), ("4", "4x MSAA")]),
    ),
    setting(
        VideoAdvanced,
        "r_waterforceexpensive r_waterforcereflectentities",
        Some("WaterDetail"),
        Some("#GameUI_Water_Detail"),
        "Water detail",
        SettingKind::Choice(&[
            ("0 0", "#gameui_noreflections|Simple reflections"),
            ("1 0", "#gameui_reflectonlyworld|Reflect world"),
            ("1 1", "#gameui_reflectall|Reflect all"),
        ]),
    ),
    // Shadow detail: Low blob shadows, Medium render-to-texture, High as
    // Medium (no flashlights in CS:S); rebuilt at once
    // (`map::shadows::apply_shadow_detail`).
    setting(
        VideoAdvanced,
        "r_shadowrendertotexture r_flashlightdepthtexture",
        Some("ShadowDetail"),
        Some("#GameUI_Shadow_Detail"),
        "Shadow detail",
        SettingKind::Choice(&[
            ("0 0", "#GameUI_Low|Low"),
            ("1 0", "#GameUI_Medium|Medium"),
            ("1 1", "#GameUI_High|High"),
        ]),
    ),
    // Texture detail and filtering: each map texture takes them as it
    // loads (`TextureSettings`; from the next map on).
    setting(
        VideoAdvanced,
        "mat_picmip",
        Some("TextureDetail"),
        Some("#GameUI_Texture_Detail"),
        "Texture detail (next map)",
        SettingKind::Choice(&[
            ("2", "#GameUI_Low|Low"),
            ("1", "#GameUI_Medium|Medium"),
            ("0", "#GameUI_High|High"),
            ("-1", "#GameUI_Ultra|Very High"),
        ]),
    ),
    setting(
        VideoAdvanced,
        "mat_trilinear mat_forceaniso",
        Some("FilteringMode"),
        Some("#GameUI_Filtering_Mode"),
        "Filtering mode (next map)",
        SettingKind::Choice(&[
            ("0 1", "#GameUI_Bilinear|Bilinear"),
            ("1 1", "#GameUI_Trilinear|Trilinear"),
            ("0 2", "#GameUI_Anisotropic2X|Anisotropic 2X"),
            ("0 4", "#GameUI_Anisotropic4X|Anisotropic 4X"),
            ("0 8", "#GameUI_Anisotropic8X|Anisotropic 8X"),
            ("0 16", "#GameUI_Anisotropic16X|Anisotropic 16X"),
        ]),
    ),
    setting(VideoAdvanced, "mat_vsync", Some("VSync"), Some("#GameUI_Wait_For_VSync"), "Wait for vertical sync", SettingKind::Toggle),
    setting(VideoAdvanced, "mat_hdr_level", Some("HDR"), Some("#GameUI_HDR"), "High dynamic range (next map)", SettingKind::Choice(HDR_LEVELS)),
    setting(VideoAdvanced, "fov_desired", Some("FovSlider"), Some("#GameUI_FOV"), "Field of view", range(75.0, 90.0, 1.0, 0)),
    // The video tab's brightness dialog (`OptionsSubVideoGammaDlg.res`):
    // the slider (LIGHT at its left) and the entry beside it
    // (`VALUE_ENTRIES`). `client::gamma` applies it to the whole screen.
    setting(Gamma, "mat_monitorgamma", Some("Gamma"), Some("#GameUI_Gamma"), "Gamma", range(1.6, 2.6, 0.05, 2)),
    // Voice (`OptionsSubVoice.res`): mashup has no voice chat.
    setting(Options(Tab::Voice), "voice_modenable", Some("voice_modenable"), Some("#GameUI_EnableVoice"), "Enable voice", SettingKind::Toggle),
    setting(Options(Tab::Voice), "voice_scale", Some("VoiceReceive"), Some("#GameUI_VoiceReceiveVolume"), "Voice receive volume", range(0.0, 1.0, 0.05, 2)),
    // Multiplayer (`OptionsSubMultiplayer.res`).
    setting(
        Options(Tab::Multiplayer),
        "cl_crosshaircolor",
        Some("CrosshairColorComboBox"),
        None,
        "Crosshair colour",
        SettingKind::Choice(&[
            ("0", "#Cstrike_Crosshair_Green|Green"),
            ("1", "#Cstrike_Crosshair_Red|Red"),
            ("2", "#Cstrike_Crosshair_Blue|Blue"),
            ("3", "#Cstrike_Crosshair_Yellow|Yellow"),
            ("4", "#Cstrike_Crosshair_LtBlue|Cyan"),
            ("5", "#Cstrike_Crosshair_Custom|Custom"),
        ]),
    ),
    setting(Options(Tab::Multiplayer), "cl_crosshairsize", Some("Size Slider"), Some("#GameUI_CrosshairSize"), "Size", range(0.0, 10.0, 0.1, 1)),
    setting(
        Options(Tab::Multiplayer),
        "cl_crosshairthickness",
        Some("Thickness Slider"),
        Some("#GameUI_CrosshairThickness"),
        "Thickness",
        range(0.0, 3.0, 0.1, 1),
    ),
    setting(Options(Tab::Multiplayer), "cl_crosshairalpha", Some("Alpha Slider"), None, "Opacity", range(0.0, 255.0, 5.0, 0)),
    setting(Options(Tab::Multiplayer), "cl_crosshaircolor_r", Some("Red Color Slider"), Some("#GameUI_CrosshairRed"), "Red", range(0.0, 255.0, 5.0, 0)),
    setting(Options(Tab::Multiplayer), "cl_crosshaircolor_g", Some("Green Color Slider"), Some("#GameUI_CrosshairGreen"), "Green", range(0.0, 255.0, 5.0, 0)),
    setting(Options(Tab::Multiplayer), "cl_crosshaircolor_b", Some("Blue Color Slider"), Some("#GameUI_CrosshairBlue"), "Blue", range(0.0, 255.0, 5.0, 0)),
    setting(
        Options(Tab::Multiplayer),
        "cl_crosshairusealpha",
        Some("CrosshairTranslucencyCheckbox"),
        Some("#GameUI_CrosshairBlend"),
        "Translucent",
        SettingKind::Toggle,
    ),
    setting(
        Options(Tab::Multiplayer),
        "cl_dynamiccrosshair",
        Some("CrosshairDynamicCheckbox"),
        Some("#GameUI_CrosshairDynamic"),
        "Dynamic",
        SettingKind::Toggle,
    ),
    setting(Options(Tab::Multiplayer), "cl_crosshairdot", Some("CrosshairDotCheckbox"), Some("#GameUI_CrosshairDot"), "Dot", SettingKind::Toggle),
    setting(
        Options(Tab::Multiplayer),
        "cl_radar_locked",
        Some("LockRadarRotationCheckbox"),
        Some("#Cstrike_RadarLocked"),
        "Lock radar rotation",
        SettingKind::Toggle,
    ),
    setting(
        Options(Tab::Multiplayer),
        "cl_downloadfilter",
        Some("DownloadFilterCheck"),
        None,
        "Custom content from servers",
        SettingKind::Choice(&[
            ("all", "#GameUI_DownloadFilter_ALL|Allow all custom files from server"),
            ("nosounds", "#GameUI_DownloadFilter_NoSounds|Do not download custom sounds"),
            ("mapsonly", "#GameUI_DownloadFilter_MapsOnly|Only allow map files"),
            ("none", "#GameUI_DownloadFilter_None|Do not download any custom files"),
        ]),
    ),
    // The keyboard tab's Advanced dialog (`OptionsSubKeyboardAdvancedDlg.res`).
    setting(KeyboardAdvanced, "hud_fastswitch", Some("FastSwitchCheck"), Some("#GameUI_FastSwitchCheck"), "Fast weapon switch", SettingKind::Toggle),
    setting(
        KeyboardAdvanced,
        "con_enable",
        Some("ConsoleCheck"),
        Some("#GameUI_DeveloperConsoleCheck"),
        "Enable developer console",
        SettingKind::Toggle,
    ),
    // Ours: what CS:S's options don't have. (Weapon hand, `cl_righthand`,
    // is CS:S's Multiplayer > Advanced.)
    setting(Extras, "zoom_sensitivity_ratio", None, None, "Zoom sensitivity ratio", range(0.1, 4.0, 0.1, 1)),
    setting(Extras, "dsp_volume", None, None, "Room reverb level", range(0.0, 2.0, 0.1, 1)),
    setting(
        Extras,
        "cl_crosshairscale",
        None,
        None,
        "Crosshair scale",
        SettingKind::Choice(&[("0", "Auto"), ("1200", "Small"), ("768", "Medium"), ("600", "Large")]),
    ),
    setting(Extras, "viewmodel_fov", None, None, "View model FOV", range(40.0, 90.0, 1.0, 0)),
    setting(Extras, "cl_showfps", None, None, "Show FPS", SettingKind::Choice(&[("0", "Off"), ("1", "On"), ("2", "Detailed")])),
];

/// Text entries beside a slider that show its value (`fieldName` of the
/// entry, the slider's cvar): VGUI's options pair them.
pub const VALUE_ENTRIES: [(&str, &str); 3] = [
    ("SensitivityLabel", "sensitivity"),
    ("MouseAccelerationLabel", "m_customaccel_exponent"),
    ("GammaEntry", "mat_monitorgamma"),
];

/// A setting's index in `SETTINGS` by its cvar (one of its cvars, for a
/// control setting several).
pub fn setting_index(cvar: &str) -> Option<usize> {
    SETTINGS.iter().position(|s| s.cvars().any(|c| c == cvar))
}

/// A choice label (`#Token|ours`): the game's text when it has the token,
/// else ours.
pub fn choice_label(label: &str, text: &dyn Fn(&str) -> Option<String>) -> String {
    match label.split_once('|') {
        Some((token, ours)) => text(token).unwrap_or_else(|| ours.to_string()),
        None => label.to_string(),
    }
}

impl Setting {
    /// The cvars it sets: one, or several for a control CS:S's code maps
    /// onto more than one (`cvar` lists them with spaces between; its
    /// value is theirs in that order, with spaces between).
    pub fn cvars(&self) -> impl Iterator<Item = &'static str> {
        self.cvar.split_whitespace()
    }

    /// Its value from its cvars' (`get`); None if mashup lacks any of them.
    pub fn read(&self, get: impl Fn(&str) -> Option<String>) -> Option<String> {
        let values: Option<Vec<String>> = self.cvars().map(|c| get(c).map(|v| v.trim().to_string())).collect();
        values.map(|v| v.join(" "))
    }

    /// The console lines setting it to `value` (one per cvar).
    pub fn lines(&self, value: &str) -> Vec<String> {
        let cvars: Vec<&str> = self.cvars().collect();
        if cvars.len() == 1 {
            return vec![format!("{} {}", cvars[0], crate::console::quote(value))];
        }
        cvars
            .iter()
            .zip(value.split_whitespace())
            .map(|(c, v)| format!("{c} {}", crate::console::quote(v)))
            .collect()
    }

    /// The value `dir` steps from `current` (wrapping through choices,
    /// clamped in ranges, flipping check boxes); `choices` are the window
    /// sizes for `Resolution`.
    pub fn step(&self, current: &str, dir: i32, choices: &[String]) -> String {
        self.step_with(current, dir, choices, true)
    }

    /// As `step`; `wrap` false: choices stop at their ends (a combo box
    /// stepped with the arrow keys).
    pub fn step_with(&self, current: &str, dir: i32, choices: &[String], wrap: bool) -> String {
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
                let next = if wrap { (i + dir).rem_euclid(n) } else { (i + dir).clamp(0, n - 1) };
                list[next as usize].0.to_string()
            }
            SettingKind::Toggle => if is_on(current) { "0" } else { "1" }.to_string(),
            SettingKind::ToggleTo(on) => if is_on(current) { "0" } else { on }.to_string(),
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
            SettingKind::Toggle | SettingKind::ToggleTo(_) | SettingKind::Negate => {
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

    /// A combo box's entries: values and their words (a check box shown as
    /// a combo: Disabled, Enabled; window sizes from `choices`). Empty for
    /// sliders and check boxes.
    pub fn entries(&self, choices: &[String], text: &dyn Fn(&str) -> Option<String>) -> Vec<(String, String)> {
        match self.kind {
            SettingKind::Choice(list) => list.iter().map(|(v, l)| (v.to_string(), choice_label(l, text))).collect(),
            SettingKind::Toggle => ON_OFF.iter().map(|(v, l)| (v.to_string(), choice_label(l, text))).collect(),
            SettingKind::Resolution => choices.iter().map(|c| (c.clone(), c.clone())).collect(),
            SettingKind::Range { .. } | SettingKind::Negate | SettingKind::ToggleTo(_) => Vec::new(),
        }
    }

    /// Which of `entries` a value is.
    pub fn entry_index(&self, value: &str, choices: &[String]) -> Option<usize> {
        let none = |_: &str| None;
        let v = value.trim();
        self.entries(choices, &none).iter().position(|(e, _)| match self.kind {
            SettingKind::Toggle => (e == "1") == is_on(v),
            _ => e == v,
        })
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
            .init_resource::<PlayerFov>()
            .add_systems(Update, apply_video)
            .add_systems(PostUpdate, apply_msaa);
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
        resource_cvar::<VideoSettings, u8>(
            app,
            "mat_antialias",
            "Multisample antialiasing: 0 off, 2 or 4 samples per pixel (4 by default). Off saves GPU time \
             at high resolutions (each sample is drawn and resolved for every camera).",
            |v| &mut v.antialias,
        );
        app.console_cvar(
            "fov_desired",
            "The player's field of view, 75 to 90 degrees (horizontal at 4:3); zoomed scopes keep theirs.",
            "90",
            |w| w.get_resource::<PlayerFov>().map(|f| f.0.to_string()),
            |w, v| {
                let v: f32 = v.trim().parse().map_err(|_| format!("bad value \"{v}\""))?;
                w.get_resource_mut::<PlayerFov>().ok_or("not available")?.0 = v.clamp(FOV_RANGE.0, FOV_RANGE.1);
                Ok(())
            },
        );
        app.init_resource::<crate::map::shadows::ShadowSettings>();
        resource_cvar::<crate::map::shadows::ShadowSettings, u8>(
            app,
            "r_shadows",
            "0: no dynamic shadows (props' and characters').",
            |s| &mut s.shadows,
        );
        resource_cvar::<crate::map::shadows::ShadowSettings, u8>(
            app,
            "r_shadowrendertotexture",
            "1: props cast their silhouette (render-to-texture shadows); 0: blob shadows, a round patch straight \
             down (shadow detail Low). Applies at once.",
            |s| &mut s.render_to_texture,
        );
        resource_cvar::<crate::map::shadows::ShadowSettings, u8>(
            app,
            "r_flashlightdepthtexture",
            "1: flashlights cast depth shadows (shadow detail High; CS:S has no flashlights: as Medium).",
            |s| &mut s.flashlight_depth,
        );
        app.init_resource::<TextureSettings>()
            .add_systems(
                PostUpdate,
                apply_texture_settings
                    .after(bevy::asset::AssetEventSystems)
                    .run_if(resource_exists::<Assets<Image>>),
            );
        resource_cvar::<TextureSettings, i32>(
            app,
            "mat_picmip",
            "Texture detail: 2 low, 1 medium, 0 high, -1 very high (as high: no larger textures). Each level skips \
             a texture's largest mip. Applies from the next map load.",
            |t| &mut t.picmip,
        );
        resource_cvar::<TextureSettings, u8>(
            app,
            "mat_trilinear",
            "1: blend between mip levels (trilinear filtering). Applies from the next map load.",
            |t| &mut t.trilinear,
        );
        resource_cvar::<TextureSettings, u16>(
            app,
            "mat_forceaniso",
            "Anisotropic filtering: 1 off, 2, 4, 8 or 16 samples. Applies from the next map load.",
            |t| &mut t.aniso,
        );
        let mut console = app.world_mut().resource_mut::<Console>();
        for name in [
            "mat_vsync",
            "mashup_fullscreen",
            "mashup_resolution",
            "mat_antialias",
            "fov_desired",
            "mat_picmip",
            "mat_trilinear",
            "mat_forceaniso",
            "r_shadows",
            "r_shadowrendertotexture",
            "r_flashlightdepthtexture",
        ] {
            console.archive(name);
        }
    }
}

/// Video > Advanced's texture detail and filtering (`mat_picmip`,
/// `mat_trilinear`, `mat_forceaniso`; CS:S's defaults: high, bilinear).
/// Each map texture takes them as it loads (`apply_texture_settings`):
/// the textures live only on the GPU once drawn, so a change shows from
/// the next map load (CS:S's applies at once).
#[derive(Resource, Clone, Copy, Debug, PartialEq)]
pub struct TextureSettings {
    pub picmip: i32,
    pub trilinear: u8,
    pub aniso: u16,
}

impl Default for TextureSettings {
    fn default() -> Self {
        Self {
            picmip: 0,
            trilinear: 0,
            aniso: 1,
        }
    }
}

impl TextureSettings {
    /// Set a map texture's sampler: repeating, mipmapped and filtered
    /// (the world's, models', water's; not the HUD's or UI's pictures,
    /// which clamp, or pixel-art ones, which don't filter). Returns
    /// whether it is one.
    pub fn apply(&self, image: &mut Image) -> bool {
        use bevy::image::{ImageAddressMode, ImageFilterMode, ImageSampler};
        let levels = image.texture_descriptor.mip_level_count;
        let ImageSampler::Descriptor(d) = &mut image.sampler else {
            return false;
        };
        if levels < 2
            || d.address_mode_u != ImageAddressMode::Repeat
            || d.mag_filter != ImageFilterMode::Linear
            || d.min_filter != ImageFilterMode::Linear
        {
            return false;
        }
        let aniso = self.aniso.clamp(1, 16);
        // The GPU filters anisotropically only with every filter linear.
        d.mipmap_filter = if self.trilinear != 0 || aniso > 1 {
            ImageFilterMode::Linear
        } else {
            ImageFilterMode::Nearest
        };
        d.anisotropy_clamp = aniso;
        // Low and medium skip the largest one or two levels (never the
        // last).
        d.lod_min_clamp = self.picmip.clamp(0, levels as i32 - 1) as f32;
        true
    }
}

/// Each texture added this frame takes `TextureSettings` before it goes
/// to the GPU (the render world takes it at the end of the frame).
fn apply_texture_settings(
    settings: Res<TextureSettings>,
    mut events: MessageReader<AssetEvent<Image>>,
    mut images: ResMut<Assets<Image>>,
) {
    for event in events.read() {
        if let AssetEvent::Added { id } = event
            && let Some(image) = images.get_mut_untracked(*id)
        {
            settings.apply(image);
        }
    }
}

/// `fov_desired`: the player's unzoomed field of view, degrees
/// horizontal at 4:3 (CS:S's 90; the video Advanced dialog's slider takes
/// 75 to 90). The world camera uses it unzoomed (`client::zoom_camera`);
/// the view model zooms with it (`map::view_model::view_model_fov`); a
/// scope's zoom is its own.
#[derive(Resource, Clone, Copy, Debug, PartialEq)]
pub struct PlayerFov(pub f32);

impl Default for PlayerFov {
    fn default() -> Self {
        Self(90.0)
    }
}

/// `fov_desired`'s bounds (CS:S's slider's, `OptionsSubVideoAdvancedDlg.res`).
pub const FOV_RANGE: (f32, f32) = (75.0, 90.0);

/// The window's mode, size and vsync, as the options set them.
#[derive(Resource, Clone, Debug, PartialEq)]
pub struct VideoSettings {
    pub vsync: u8,
    pub fullscreen: u8,
    pub resolution: String,
    /// MSAA samples (`mat_antialias`).
    pub antialias: u8,
}

impl Default for VideoSettings {
    fn default() -> Self {
        Self {
            vsync: 1,
            fullscreen: 0,
            resolution: String::new(),
            antialias: 4,
        }
    }
}

impl VideoSettings {
    /// `mat_antialias` as Bevy's MSAA setting: 0 or 1 off, 2, anything
    /// higher 4 (8 isn't supported everywhere).
    pub fn msaa(&self) -> Msaa {
        match self.antialias {
            0 | 1 => Msaa::Off,
            2 | 3 => Msaa::Sample2,
            _ => Msaa::Sample4,
        }
    }
}

/// Every camera (the main view, sky, view model, water reflection and the
/// HUD's: the ones drawing into one target should agree, or each switch
/// costs a full-screen copy) takes `mat_antialias`.
fn apply_msaa(settings: Res<VideoSettings>, cameras: Query<(Entity, Option<&Msaa>), With<Camera>>, mut commands: Commands) {
    let want = settings.msaa();
    for (e, msaa) in &cameras {
        if msaa != Some(&want) {
            commands.entity(e).insert(want);
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
    fn antialias_levels() {
        let msaa = |antialias| VideoSettings { antialias, ..default() }.msaa();
        assert_eq!(VideoSettings::default().msaa(), Msaa::Sample4, "4x by default");
        assert_eq!([msaa(0), msaa(1), msaa(2), msaa(4), msaa(8)], [Msaa::Off, Msaa::Off, Msaa::Sample2, Msaa::Sample4, Msaa::Sample4]);
        let s = setting("mat_antialias");
        assert_eq!(s.step("4", 1, &[]), "0", "wraps like the other choices");
    }

    /// Texture detail and filtering set a map texture's sampler; pictures
    /// that clamp (HUD, UI) or have no mips are left alone.
    #[test]
    fn texture_settings_set_map_samplers() {
        use bevy::image::{ImageAddressMode, ImageFilterMode, ImageSampler, ImageSamplerDescriptor};
        let texture = |address, levels| {
            let mut image = Image::default();
            image.texture_descriptor.mip_level_count = levels;
            image.sampler = ImageSampler::Descriptor(ImageSamplerDescriptor {
                address_mode_u: address,
                address_mode_v: address,
                mipmap_filter: ImageFilterMode::Linear,
                ..ImageSamplerDescriptor::linear()
            });
            image
        };
        let sampler = |image: &Image| match &image.sampler {
            ImageSampler::Descriptor(d) => (d.mipmap_filter, d.anisotropy_clamp, d.lod_min_clamp),
            _ => panic!(),
        };
        // CS:S's defaults: bilinear (nearest mip), no anisotropy, full size.
        let mut map = texture(ImageAddressMode::Repeat, 8);
        assert!(TextureSettings::default().apply(&mut map));
        assert_eq!(sampler(&map), (ImageFilterMode::Nearest, 1, 0.0));
        // Low detail, anisotropic 8x (which needs the mip filter linear).
        let low = TextureSettings {
            picmip: 2,
            trilinear: 0,
            aniso: 8,
        };
        assert!(low.apply(&mut map));
        assert_eq!(sampler(&map), (ImageFilterMode::Linear, 8, 2.0));
        // Never past the last mip.
        let mut small = texture(ImageAddressMode::Repeat, 2);
        TextureSettings { picmip: 2, ..default() }.apply(&mut small);
        assert_eq!(sampler(&small).2, 1.0);
        // A HUD picture (clamped) and one without mips: untouched.
        let mut hud = texture(ImageAddressMode::ClampToEdge, 8);
        assert!(!low.apply(&mut hud));
        assert_eq!(sampler(&hud), (ImageFilterMode::Linear, 1, 0.0));
        assert!(!low.apply(&mut texture(ImageAddressMode::Repeat, 1)));
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
        assert_eq!(mode.show("2", &game), "Borderless fullscreen");
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
    fn every_tab_has_settings_and_the_games_layout_places_them() {
        // The keyboard tab's own are its Advanced dialog's.
        assert!(SETTINGS.iter().all(|s| s.place != Place::Options(Tab::Keyboard)));
        for (tab, ..) in TABS.iter().skip(1) {
            assert!(SETTINGS.iter().any(|s| s.place == Place::Options(*tab)), "{tab:?} has settings");
        }
        // Each of the game's dialogs names its controls; ours have none.
        for s in SETTINGS {
            assert_eq!(s.field.is_none(), s.place == Place::Extras, "{}", s.cvar);
        }
        assert_eq!(Tab::Keyboard.step(-1), Tab::Multiplayer);
    }

    #[test]
    fn a_combo_box_lists_a_settings_values() {
        let vsync = setting("mat_vsync");
        let e = vsync.entries(&[], &none);
        assert_eq!(e, [("0".to_string(), "Disabled".to_string()), ("1".into(), "Enabled".into())]);
        assert_eq!(vsync.entry_index("1", &[]), Some(1));
        let r = setting("mashup_resolution");
        let sizes = ["1920x1080".to_string(), "1280x720".into()];
        assert_eq!(r.entries(&sizes, &none).len(), 2);
        assert_eq!(r.entry_index("1280x720", &sizes), Some(1));
        assert_eq!(r.entry_index("", &sizes), None, "as started: none picked");
        // Stepped as a combo box: no wrapping.
        let aa = setting("mat_antialias");
        assert_eq!(aa.step_with("4", 1, &[], false), "4");
        assert!(setting("sensitivity").entries(&[], &none).is_empty());
    }
    #[test]
    fn a_control_can_set_several_cvars() {
        let water = setting("r_waterforceexpensive r_waterforcereflectentities");
        assert_eq!(setting_index("r_waterforcereflectentities"), setting_index("r_waterforceexpensive"));
        let get = |c: &str| match c {
            "r_waterforceexpensive" => Some("1".to_string()),
            "r_waterforcereflectentities" => Some(" 0".to_string()),
            _ => None,
        };
        assert_eq!(water.read(get).as_deref(), Some("1 0"));
        assert_eq!(water.show("1 0", &none), "Reflect world");
        assert_eq!(water.read(|_| None), None, "greyed without its cvars");
        assert_eq!(water.lines("1 1"), ["r_waterforceexpensive 1", "r_waterforcereflectentities 1"]);
        assert_eq!(setting("volume").lines("0.5"), ["volume 0.5"]);
        // The mouse acceleration check box ticks to 3.
        let accel = setting("m_customaccel");
        assert_eq!(accel.step("0", 1, &[]), "3");
        assert!(accel.checked("3") && accel.checked("1"));
        assert_eq!(accel.step("3", 1, &[]), "0");
    }

    /// The cvars of the options' controls added for parity, with CS:S's
    /// defaults and a changed value.
    const PARITY_CVARS: [(&str, &str, &str); 25] = [
        ("fov_desired", "90", "80"),
        ("cl_crosshairsize", "5", "3"),
        ("cl_crosshairthickness", "0.5", "1.5"),
        ("cl_crosshairdot", "0", "1"),
        ("cl_crosshaircolor_r", "50", "200"),
        ("cl_crosshaircolor_g", "250", "10"),
        ("cl_crosshaircolor_b", "50", "90"),
        ("snd_musicvolume", "1", "0.3"),
        ("snd_mute_losefocus", "1", "0"),
        ("m_filter", "0", "1"),
        ("m_customaccel", "0", "3"),
        ("m_customaccel_exponent", "1.05", "1.2"),
        ("cl_radar_locked", "0", "1"),
        ("cl_downloadfilter", "all", "none"),
        ("mp_decals", "200", "50"),
        ("cl_c4progressbar", "1", "0"),
        ("volume", "0.5", "0.75"),
        ("snd_surround_speakers", "2", "0"),
        ("snd_pitchquality", "1", "0"),
        ("dsp_slow_cpu", "0", "1"),
        ("closecaption", "0", "1"),
        ("cc_subtitles", "0", "1"),
        ("mat_picmip", "0", "1"),
        ("mat_trilinear", "0", "1"),
        ("mat_forceaniso", "1", "8"),
    ];

    /// An app with every options cvar's owner registered (no window).
    fn options_app() -> App {
        let mut app = App::new();
        app.add_plugins(crate::console::ConsolePlugin)
            .add_plugins((VideoPlugin, crate::client::audio::SoundOptionsPlugin))
            .init_resource::<crate::client::hud::CrosshairColor>()
            .init_resource::<crate::client::input::MouseSettings>();
        crate::client::hud::crosshair_cvars(&mut app);
        crate::client::input::mouse_cvars(&mut app);
        crate::client::radar::radar_cvars(&mut app);
        crate::client::perf::decal_cvars(&mut app);
        crate::client::objectives_hud::c4_bar_cvar(&mut app);
        crate::net::maps::download_filter_cvar(&mut app);
        app
    }

    fn value(app: &mut App, name: &str) -> String {
        let cvar = app.world().resource::<Console>().cvar(name).cloned().unwrap_or_else(|| panic!("no {name}"));
        (cvar.get)(app.world_mut()).unwrap()
    }

    #[test]
    fn the_options_cvars_start_as_css_and_persist_in_the_config() {
        let mut app = options_app();
        for (name, default, _) in PARITY_CVARS {
            let c = app.world().resource::<Console>().cvar(name).cloned().unwrap();
            assert!(c.archive, "{name} is saved");
            assert_eq!(c.default, default, "{name}'s default");
            assert_eq!(value(&mut app, name), default, "{name} starts at it");
            app.world_mut().resource_mut::<Console>().submit(format!("{name} {}", PARITY_CVARS.iter().find(|c| c.0 == name).unwrap().2));
        }
        app.update();
        for (name, _, changed) in PARITY_CVARS {
            assert_eq!(value(&mut app, name), changed, "{name} set");
        }
        let text = crate::console::config_text(app.world_mut());
        // A fresh start runs the config: every value comes back.
        let mut fresh = options_app();
        for line in text.lines() {
            fresh.world_mut().resource_mut::<Console>().submit(line);
        }
        fresh.update();
        for (name, _, changed) in PARITY_CVARS {
            assert!(text.contains(&format!("{name} ")), "{name} in config.cfg:\n{text}");
            assert_eq!(value(&mut fresh, name), changed, "{name} after a restart");
        }
    }

    #[test]
    fn fov_desired_sets_the_cameras_field_of_view() {
        use crate::{client::FirstPersonCamera, core::LocalPlayer, map::view_model::vertical_fov};
        let mut app = options_app();
        app.init_resource::<crate::client::spectate::SpecView>()
            .add_systems(Update, crate::client::zoom_camera);
        let camera = app
            .world_mut()
            .spawn((FirstPersonCamera, Projection::Perspective(PerspectiveProjection::default())))
            .id();
        let player = app.world_mut().spawn(LocalPlayer).id();
        let fov = |app: &App| match app.world().get::<Projection>(camera) {
            Some(Projection::Perspective(p)) => p.fov.to_degrees(),
            _ => f32::NAN,
        };
        app.update();
        assert!((fov(&app) - vertical_fov(90.0)).abs() < 1e-3, "CS:S's 90 by default");
        app.world_mut().resource_mut::<Console>().submit("fov_desired 75");
        app.update();
        app.update();
        assert!((fov(&app) - vertical_fov(75.0)).abs() < 1e-3, "{}", fov(&app));
        // Out of the slider's range: clamped to it.
        app.world_mut().resource_mut::<Console>().submit("fov_desired 120");
        app.update();
        app.update();
        assert!((fov(&app) - vertical_fov(90.0)).abs() < 1e-3);
        // A scope's zoom is its own, whatever fov_desired is.
        app.world_mut().resource_mut::<Console>().submit("fov_desired 80");
        app.world_mut().entity_mut(player).insert(crate::weapon::Zoomed { fov: 40.0, scope: true });
        app.update();
        app.update();
        assert!((fov(&app) - vertical_fov(40.0)).abs() < 1e-3);
        // The view model zooms with the player's view, as with a scope.
        let vm = crate::map::view_model::ViewModelSettings::default();
        assert_eq!(crate::map::view_model::view_model_fov(&vm, 80.0), vm.fov - 10.0);
    }
}
