//! The options' sound settings (CS:S's Audio tab): the master `volume`,
//! `snd_musicvolume` (music: `map::live_sound::is_music`) and
//! `snd_mute_losefocus` (silent while the window is in the background).
//! Bevy's `GlobalVolume` is what the mixer reads (`map::live_sound`,
//! `map::room`): the volume, or nothing while muted for focus.

use bevy::{
    audio::{GlobalVolume, Volume},
    prelude::*,
};

use crate::{
    console::{Console, ConsoleAppExt, resource_cvar},
    map::live_sound::SoundMix,
};

/// Sound volume a fresh config starts with (`volume`).
pub const DEFAULT_VOLUME: f32 = 0.5;

/// The player's sound settings.
#[derive(Resource, Clone, Copy, Debug, PartialEq)]
pub struct SoundOptions {
    /// `volume`, 0 to 1.
    pub volume: f32,
    /// `snd_mute_losefocus`: 1 silences the game while its window is in
    /// the background (CS:S's default).
    pub mute_losefocus: u8,
    /// `snd_surround_speakers`: 0 headphones, 2 speakers, 4, 5 (5.1), 7
    /// (7.1). Kept and saved; the mixer is stereo whatever it says (more
    /// speakers get the stereo mix; headphones and 2 speakers pan alike
    /// until a spec says how Source's differ).
    pub speakers: u8,
    /// `snd_pitchquality` (1) and `dsp_slow_cpu` (0): the Audio tab's
    /// sound quality, High (1 0), Medium (0 0), Low (0 1). Kept and
    /// saved; the mixer has one quality (its resampling isn't ours to
    /// choose).
    pub pitch_quality: u8,
    pub slow_cpu: u8,
    /// `closecaption` (0) and `cc_subtitles` (0): the Audio tab's
    /// captioning, No captions (0 0), Subtitles (1 1), Closed Captions
    /// (1 0). Kept and saved; captions aren't drawn yet
    /// (docs/plans/active/video-settings.md, "Audio").
    pub closecaption: u8,
    pub cc_subtitles: u8,
}

impl Default for SoundOptions {
    fn default() -> Self {
        Self {
            volume: DEFAULT_VOLUME,
            mute_losefocus: 1,
            speakers: 2,
            pitch_quality: 1,
            slow_cpu: 0,
            closecaption: 0,
            cc_subtitles: 0,
        }
    }
}

/// The Audio tab's spoken language (`mashup_spoken_language`, CS:S's
/// "Audio (spoken) language", a Steam setting there, not a cvar): the
/// voice files mashup reads are the install's English ones
/// (`hl2_sound_vo_english`), so English is the one choice.
#[derive(Resource, Clone, Debug, PartialEq)]
pub struct SpokenLanguage(pub String);

impl Default for SpokenLanguage {
    fn default() -> Self {
        Self("english".into())
    }
}

impl SoundOptions {
    /// The master volume heard, with the window focused or not.
    pub fn master(&self, focused: bool) -> f32 {
        if focused || self.mute_losefocus == 0 {
            self.volume.clamp(0.0, 1.0)
        } else {
            0.0
        }
    }
}

pub struct SoundOptionsPlugin;

impl Plugin for SoundOptionsPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<SoundOptions>()
            .init_resource::<SpokenLanguage>()
            .init_resource::<SoundMix>()
            .insert_resource(GlobalVolume::new(Volume::Linear(DEFAULT_VOLUME)))
            .add_systems(Update, apply_master);
        sound_cvars(app);
    }
}

fn sound_cvars(app: &mut App) {
    app.console_cvar(
        "volume",
        "Sound volume, 0 to 1.",
        &DEFAULT_VOLUME.to_string(),
        |w| w.get_resource::<SoundOptions>().map(|s| s.volume.to_string()),
        |w, v| {
            let v: f32 = v.trim().parse().map_err(|_| format!("bad value \"{v}\""))?;
            w.get_resource_mut::<SoundOptions>().ok_or("not available")?.volume = v.clamp(0.0, 1.0);
            Ok(())
        },
    );
    resource_cvar::<SoundOptions, u8>(
        app,
        "snd_mute_losefocus",
        "1: silence the game while its window is in the background.",
        |s| &mut s.mute_losefocus,
    );
    resource_cvar::<SoundMix, f32>(
        app,
        "snd_musicvolume",
        "Music volume, 0 to 1 (sounds under music/ and MP3s), times the master volume.",
        |m| &mut m.music,
    );
    resource_cvar::<SoundOptions, u8>(
        app,
        "snd_surround_speakers",
        "Speaker configuration: 0 headphones, 2 speakers, 4, 5 (5.1), 7 (7.1). Kept; the mixer is stereo.",
        |s| &mut s.speakers,
    );
    resource_cvar::<SoundOptions, u8>(
        app,
        "snd_pitchquality",
        "1: high quality pitch shifting (the Audio tab's High sound quality). Kept; the mixer has one quality.",
        |s| &mut s.pitch_quality,
    );
    resource_cvar::<SoundOptions, u8>(
        app,
        "dsp_slow_cpu",
        "1: cheaper room effects (the Audio tab's Low sound quality). Kept; the mixer has one quality.",
        |s| &mut s.slow_cpu,
    );
    resource_cvar::<SoundOptions, u8>(
        app,
        "closecaption",
        "1: captions for sounds that have them (with cc_subtitles 1, dialogue only). Kept; not drawn yet.",
        |s| &mut s.closecaption,
    );
    resource_cvar::<SoundOptions, u8>(
        app,
        "cc_subtitles",
        "1: captions are subtitles (dialogue only, no sound effects).",
        |s| &mut s.cc_subtitles,
    );
    resource_cvar::<SpokenLanguage, String>(
        app,
        "mashup_spoken_language",
        "The voice language (CS:S's Audio tab): english, the install's voice files.",
        |l| &mut l.0,
    );
    let mut console = app.world_mut().resource_mut::<Console>();
    for name in SOUND_CVARS {
        console.archive(name);
    }
}

/// The Audio tab's cvars, saved in config.cfg.
pub const SOUND_CVARS: [&str; 9] = [
    "volume",
    "snd_mute_losefocus",
    "snd_musicvolume",
    "snd_surround_speakers",
    "snd_pitchquality",
    "dsp_slow_cpu",
    "closecaption",
    "cc_subtitles",
    "mashup_spoken_language",
];

/// The mixer's master volume: the player's, or silence while the window
/// is in the background and `snd_mute_losefocus` is on. No window
/// (headless) counts as focused.
fn apply_master(options: Res<SoundOptions>, windows: Query<&Window>, global: Option<ResMut<GlobalVolume>>) {
    let Some(mut global) = global else { return };
    let focused = windows.iter().next().is_none_or(|w| w.focused);
    let want = options.master(focused);
    if (global.volume.to_linear() - want).abs() > 1e-6 {
        global.volume = Volume::Linear(want);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn app() -> App {
        let mut app = App::new();
        app.add_plugins(crate::console::ConsolePlugin).add_plugins(SoundOptionsPlugin);
        app
    }

    fn run(app: &mut App, line: &str) {
        app.world_mut().resource_mut::<Console>().submit(line);
        app.update();
    }

    fn master(app: &App) -> f32 {
        app.world().resource::<GlobalVolume>().volume.to_linear()
    }

    #[test]
    fn volume_is_clamped_and_archived() {
        let mut app = app();
        run(&mut app, "volume 0.25");
        assert!((master(&app) - 0.25).abs() < 1e-6);
        run(&mut app, "volume 3");
        assert_eq!(master(&app), 1.0);
        run(&mut app, "volume -1");
        assert_eq!(master(&app), 0.0);
        let console = app.world().resource::<Console>();
        for name in SOUND_CVARS {
            assert!(console.cvar(name).unwrap().archive, "{name}");
        }
        // The Audio tab's drop-downs start at High quality, no captions,
        // two speakers (ours: CS:S detects the system's), English.
        for (name, default) in [
            ("snd_pitchquality", "1"),
            ("dsp_slow_cpu", "0"),
            ("closecaption", "0"),
            ("cc_subtitles", "0"),
            ("snd_surround_speakers", "2"),
            ("mashup_spoken_language", "english"),
        ] {
            assert_eq!(console.cvar(name).unwrap().default, default, "{name}");
        }
        assert_eq!(console.cvar("snd_mute_losefocus").unwrap().default, "1", "CS:S's");
        assert_eq!(console.cvar("snd_musicvolume").unwrap().default, "1", "CS:S's");
    }

    #[test]
    fn the_game_goes_quiet_in_the_background() {
        let mut app = app();
        run(&mut app, "volume 0.8");
        let window = app.world_mut().spawn(Window::default()).id();
        app.update();
        assert!((master(&app) - 0.8).abs() < 1e-6, "focused: heard");
        app.world_mut().get_mut::<Window>(window).unwrap().focused = false;
        app.update();
        assert_eq!(master(&app), 0.0, "in the background: muted");
        // The volume itself is kept (and saved as it is).
        assert_eq!(app.world().resource::<SoundOptions>().volume, 0.8);
        run(&mut app, "snd_mute_losefocus 0");
        assert!((master(&app) - 0.8).abs() < 1e-6, "the option off: heard");
    }

    #[test]
    fn music_volume_scales_music_only() {
        let mut app = app();
        run(&mut app, "snd_musicvolume 0.25");
        let mix = *app.world().resource::<SoundMix>();
        assert_eq!(mix.gain("music/hl2_song1.wav"), 0.25);
        assert_eq!(mix.gain("#music/a.wav"), 0.25, "sound characters skipped");
        assert_eq!(mix.gain("sound/custom/theme.mp3"), 0.25, "MP3s are music");
        assert_eq!(mix.gain("weapons/ak47/ak47-1.wav"), 1.0);
        assert_eq!(mix.gain("Weapon_AK47.Single"), 1.0);
    }
}
