//! CS:S's game menu and options look (Source's GameUI), read from the
//! install as a neutral `map::hud::GameUi`: the GameUI scheme
//! (`resource/SourceScheme.res` and the `SourceSchemeBase.res` it bases on,
//! under `platform/`), the menu's entries (`resource/GameMenu.res`), the
//! options pages (`resource/OptionsSub*.res`, the keyboard and video tabs'
//! Advanced dialogs `OptionsSubKeyboardAdvancedDlg.res` and
//! `OptionsSubVideoAdvancedDlg.res` too), Create Server's pages
//! (`resource/CreateMultiplayerGame{Server,Gameplay,Bot}Page.res`) and its
//! Game page's options (`cfg/settings.scr`), the keyboard page's actions
//! (`scripts/kb_act.lst`), strings from `gameui_english.txt`,
//! `valve_english.txt` and `cstrike_english.txt`, and the main menu's look: its background (`materials/console/
//! background01.vtf` and `background01_widescreen.vtf`, stretched over the
//! screen) and the game's title (`gameinfo.txt`'s `title` and `title2`) in
//! the client scheme's `ClientTitleFont` (`resource/clientscheme.res`, its
//! font file from `CustomFontFiles`); the loading dialog
//! (`resource/LoadingDialogNoBanner.res`) and the interface sounds
//! (`sound/ui/buttonrollover.wav`, `buttonclick.wav`,
//! `buttonclickrelease.wav`); the server browser (Find Servers): its
//! layouts under `servers/` (`DialogServerBrowser.res`,
//! `InternetGamesPage.res` and its `_Filters` version, `DialogAddServer.res`,
//! `DialogServerPassword.res`), words (`serverbrowser_english.txt`) and
//! column icons (`icon_password.tga`, `icon_bots.tga`). The install has no menu music: CS:S's
//! folders hold no `sound/music/` or startup track of its own (only
//! Half-Life 2's, under `hl2/`), so the menu is silent but for these.
//!
//! The scheme has platform conditionals (`[$WIN32]`, `[!$OSX]`,
//! `[$X360]`): entries are kept as the Windows PC game reads them.

use std::collections::HashMap;

use super::hud::Kv;
use crate::{
    map::hud::{GameUi, GameUiItem, KeyAction, ServerSetting, ServerSettingKind, UiImage, UiSound},
    mount::Mount,
};

/// Whether a platform condition (`$WIN32`, `!$OSX`, `$X360 || $PS3`)
/// holds for the Windows PC game.
pub(crate) fn condition_holds(cond: &str) -> bool {
    let c = cond.trim().trim_start_matches('[').trim_end_matches(']');
    c.split("||").any(|any| {
        any.split("&&").all(|term| {
            let term = term.trim();
            let (neg, name) = match term.strip_prefix('!') {
                Some(n) => (true, n),
                None => (false, term),
            };
            let name = name.trim().trim_start_matches('$').to_uppercase();
            let on = matches!(name.as_str(), "WIN32" | "WINDOWS");
            on != neg
        })
    })
}

/// KeyValues as the Windows PC game reads them: an entry followed by a
/// platform condition that doesn't hold there is left out.
pub(crate) fn parse_pc(text: &str) -> Kv {
    let t = super::surfaceprops::tokens(text);
    let cond = |t: &[String], i: usize| t.get(i).filter(|s| s.starts_with('[') && s.ends_with(']')).cloned();
    fn block(t: &[String], i: &mut usize, cond: &dyn Fn(&[String], usize) -> Option<String>) -> Kv {
        let mut items = Vec::new();
        while *i < t.len() && t[*i] != "}" {
            let key = t[*i].clone();
            *i += 1;
            let mut keep = true;
            // A condition between a block's name and its brace.
            if let Some(c) = cond(t, *i) {
                keep &= condition_holds(&c);
                *i += 1;
            }
            if *i >= t.len() {
                break;
            }
            let value = if t[*i] == "{" {
                *i += 1;
                let b = block(t, i, cond);
                *i += 1; // the closing brace
                b
            } else {
                let v = Kv::Value(t[*i].clone());
                *i += 1;
                v
            };
            if let Some(c) = cond(t, *i) {
                keep &= condition_holds(&c);
                *i += 1;
            }
            if keep {
                items.push((key, value));
            }
        }
        Kv::Block(items)
    }
    let mut i = 0;
    block(&t, &mut i, &cond)
}

/// A `.res` file's root block, read as the PC game does, with its `#base`
/// files merged in.
fn read_res_pc(read: &mut dyn FnMut(&str) -> Option<String>, path: &str) -> Option<Kv> {
    fn go(read: &mut dyn FnMut(&str) -> Option<String>, path: &str, depth: usize) -> Option<Kv> {
        let kv = parse_pc(&read(path)?);
        let mut root = None;
        let mut bases = Vec::new();
        for (k, v) in kv.items() {
            match v {
                Kv::Value(b) if k.eq_ignore_ascii_case("#base") => bases.push(b.clone()),
                Kv::Block(_) if root.is_none() => root = Some(v.clone()),
                _ => {}
            }
        }
        let mut root = root.unwrap_or(Kv::Block(Vec::new()));
        if depth < 4 {
            let dir = path.rsplit_once(['/', '\\']).map(|(d, _)| d);
            for b in bases {
                let base_path = match dir {
                    Some(d) => format!("{d}/{b}"),
                    None => b,
                };
                if let Some(base) = go(read, &base_path, depth + 1) {
                    root = super::vgui::merge(base, root);
                }
            }
        }
        Some(root)
    }
    go(read, path, 0)
}

fn rgba(text: &str) -> Option<[u8; 4]> {
    let v: Vec<u8> = text.split_whitespace().filter_map(|x| x.parse().ok()).collect();
    match v.as_slice() {
        [r, g, b] => Some([*r, *g, *b, 255]),
        [r, g, b, a] => Some([*r, *g, *b, *a]),
        _ => None,
    }
}

/// A scheme's colours (named, then base settings naming them or each
/// other) and its numeric base settings.
pub(crate) fn scheme_settings(scheme: &Kv) -> (HashMap<String, [u8; 4]>, HashMap<String, f32>) {
    let mut colors = HashMap::new();
    let mut numbers = HashMap::new();
    for (k, v) in scheme.get("Colors").map(Kv::items).unwrap_or_default() {
        if let Kv::Value(v) = v
            && let Some(c) = rgba(v)
        {
            colors.insert(k.clone(), c);
        }
    }
    let base = scheme.get("BaseSettings").map(Kv::items).unwrap_or_default();
    // Settings may name settings further down: a few passes.
    for _ in 0..3 {
        for (k, v) in base {
            let Kv::Value(v) = v else { continue };
            if let Some(c) = rgba(v).or_else(|| colors.get(v.trim()).copied()) {
                colors.insert(k.clone(), c);
            } else if let Ok(n) = v.trim().parse::<f32>() {
                numbers.insert(k.clone(), n);
            }
        }
    }
    (colors, numbers)
}

/// `#token` text from the strings (lower-case keys), else as written.
fn localise(text: &str, strings: &HashMap<String, String>) -> String {
    match text.strip_prefix('#') {
        Some(t) => strings.get(&t.to_lowercase()).cloned().unwrap_or_else(|| text.to_string()),
        None => text.to_string(),
    }
}

/// The game menu's entries (`GameMenu.res`), labels localised; an entry
/// with no label or command is a gap.
pub(crate) fn game_menu(root: &Kv, strings: &HashMap<String, String>) -> Vec<GameUiItem> {
    root.items()
        .iter()
        .filter(|(_, v)| matches!(v, Kv::Block(_)))
        .map(|(_, v)| GameUiItem {
            label: localise(v.str("label").unwrap_or_default(), strings),
            command: v.str("command").unwrap_or_default().to_string(),
            in_game_only: v.str("OnlyInGame").is_some_and(|x| x.trim() != "0"),
        })
        .collect()
}

/// The keyboard page's list (`kb_act.lst`): `"blank" "#Title"` starts a
/// section (`"blank" "===="` lines are rules), every other pair is a
/// console line and its description.
pub(crate) fn key_actions(text: &str, strings: &HashMap<String, String>) -> Vec<KeyAction> {
    let t = super::surfaceprops::tokens(text);
    t.chunks_exact(2)
        .filter_map(|pair| {
            let (key, value) = (&pair[0], &pair[1]);
            if key.eq_ignore_ascii_case("blank") {
                value
                    .starts_with('#')
                    .then(|| KeyAction::Section(localise(value, strings)))
            } else {
                Some(KeyAction::Action {
                    command: key.clone(),
                    label: localise(value, strings),
                })
            }
        })
        .collect()
}

/// The options pages' and Create Server's layout files, by page.
const OPTION_PAGES: [(&str, &str); 10] = [
    ("keyboard", "resource/optionssubkeyboard.res"),
    // The keyboard tab's Advanced dialog.
    ("keyboard_advanced", "resource/optionssubkeyboardadvanceddlg.res"),
    ("mouse", "resource/optionssubmouse.res"),
    ("audio", "resource/optionssubaudio.res"),
    ("video", "resource/optionssubvideo.res"),
    // The video tab's Advanced dialog.
    ("video_advanced", "resource/optionssubvideoadvanceddlg.res"),
    ("multiplayer", "resource/optionssubmultiplayer.res"),
    // Create Server's pages: Server, Game (its list from `SERVER_SCRIPTS`),
    // Bot.
    ("create_server", "resource/createmultiplayergameserverpage.res"),
    ("create_game", "resource/createmultiplayergamegameplaypage.res"),
    ("create_bot", "resource/createmultiplayergamebotpage.res"),
];

/// Create Server's Game page options: the install's own, else the
/// default one.
const SERVER_SCRIPTS: [&str; 2] = ["cfg/settings.scr", "cfg/settings_default.scr"];

/// The loading dialog's layout files, preferred first.
const LOADING_DIALOGS: [&str; 2] = ["resource/loadingdialognobanner.res", "resource/loadingdialog.res"];

/// The interface sounds: the waves VGUI buttons name in the install's
/// layouts (`sound_armed`, `sound_depressed`, `sound_released`; e.g.
/// `resource/ui/econ/messageboxdialog.res`).
/// And the freeze cam's (its picture taken).
const UI_SOUNDS: [(UiSound, &str); 4] = [
    (UiSound::Rollover, "sound/ui/buttonrollover.wav"),
    (UiSound::Click, "sound/ui/buttonclick.wav"),
    (UiSound::Release, "sound/ui/buttonclickrelease.wav"),
    (UiSound::FreezeCam, "sound/ui/freeze_cam.wav"),
];

/// The server browser's layouts (Find Servers; under `platform/`): name,
/// file.
const SERVER_LAYOUTS: [(&str, &str); 5] = [
    ("dialog", "servers/dialogserverbrowser.res"),
    ("page", "servers/internetgamespage.res"),
    ("page_filters", "servers/internetgamespage_filters.res"),
    ("add", "servers/dialogaddserver.res"),
    ("password", "servers/dialogserverpassword.res"),
];

/// The server browser's words and icons.
const SERVER_STRINGS: &str = "servers/serverbrowser_english.txt";
const SERVER_ICONS: [&str; 4] = ["password", "bots", "password_column", "bots_column"];

/// The main menu's backgrounds: 4:3, widescreen.
const BACKGROUNDS: [&str; 2] = [
    "materials/console/background01.vtf",
    "materials/console/background01_widescreen.vtf",
];

/// The game's title lines from `gameinfo.txt` (`title`, then `title2`).
pub(crate) fn game_title(gameinfo: &str) -> Vec<String> {
    let kv = parse_pc(gameinfo);
    let Some((_, root)) = kv.items().iter().find(|(_, v)| matches!(v, Kv::Block(_))) else {
        return Vec::new();
    };
    ["title", "title2"]
        .iter()
        .filter_map(|k| root.str(k))
        .map(str::trim)
        .filter(|t| !t.is_empty())
        .map(String::from)
        .collect()
}

/// The title font in a client scheme: `ClientTitleFont`'s family and
/// height (its first size), and the scheme's font files to find it in.
pub(crate) fn title_font(scheme: &Kv) -> Option<(String, f32, Vec<String>)> {
    let font = scheme.get("Fonts")?.get("ClientTitleFont")?;
    let (_, first) = font.items().iter().find(|(_, v)| matches!(v, Kv::Block(_)))?;
    let family = first.str("name")?.trim().to_string();
    let tall = first.str("tall")?.trim().parse().ok()?;
    let files = scheme
        .get("CustomFontFiles")
        .map(Kv::items)
        .unwrap_or_default()
        .iter()
        .filter_map(|(_, v)| match v {
            Kv::Value(path) => Some(path.clone()),
            Kv::Block(_) => v.str("font").map(String::from),
        })
        .collect();
    Some((family, tall, files))
}

/// A VTF's first frame as RGBA8.
fn decode_vtf(bytes: &[u8]) -> Option<UiImage> {
    let image = vtf::from_bytes(bytes)
        .ok()
        .and_then(|v| v.highres_image.decode(0).ok())
        .map(|i| i.to_rgba8())?;
    Some(UiImage {
        width: image.width(),
        height: image.height(),
        rgba8: image.into_raw(),
    })
}

/// A Targa picture as RGBA8.
fn decode_tga(bytes: &[u8]) -> Option<UiImage> {
    let image = image::load_from_memory_with_format(bytes, image::ImageFormat::Tga).ok()?.to_rgba8();
    Some(UiImage {
        width: image.width(),
        height: image.height(),
        rgba8: image.into_raw(),
    })
}

/// The main menu's look: backgrounds, title and title font.
fn main_menu_look(mount: &Mount, ui: &mut GameUi) {
    let [four_three, wide] = BACKGROUNDS.map(|p| mount.read(p).ok().and_then(|b| decode_vtf(&b)));
    ui.background = four_three;
    ui.background_wide = wide;
    if let Ok(info) = mount.read("gameinfo.txt") {
        ui.title = game_title(&String::from_utf8_lossy(&info));
    }
    let mut read = |p: &str| mount.read(p).ok().map(|b| super::radio::decode(&b));
    let Some((family, tall, files)) = read_res_pc(&mut read, "resource/clientscheme.res")
        .as_ref()
        .and_then(title_font)
    else {
        return;
    };
    for file in files {
        let path = file.replace('\\', "/").to_lowercase();
        let Ok(bytes) = mount.read(&path) else { continue };
        if super::hud::family_name(&bytes).is_some_and(|f| f.eq_ignore_ascii_case(&family)) {
            ui.title_font = Some((std::sync::Arc::new(bytes), tall));
            return;
        }
    }
}

/// The GameUI look from a mounted install (None without its scheme and
/// menu files).
pub fn load(mount: &Mount) -> Option<GameUi> {
    let mut read = |p: &str| mount.read(p).ok().map(|b| super::radio::decode(&b));
    let scheme = read_res_pc(&mut read, "resource/sourcescheme.res");
    let menu_root = read_res_pc(&mut read, "resource/gamemenu.res");
    if scheme.is_none() && menu_root.is_none() {
        return None;
    }
    let mut strings = HashMap::new();
    for file in [
        "resource/gameui_english.txt",
        "resource/valve_english.txt",
        "resource/cstrike_english.txt",
    ] {
        if let Some(text) = read(file) {
            for (k, v) in super::radio::localization(&text) {
                strings.entry(k).or_insert(v);
            }
        }
    }
    let scheme = scheme.unwrap_or(Kv::Block(Vec::new()));
    let (colors, numbers) = scheme_settings(&scheme);
    let mut ui = GameUi {
        fonts: super::vgui::fonts(&scheme),
        menu: menu_root.map(|m| game_menu(&m, &strings)).unwrap_or_default(),
        actions: read("scripts/kb_act.lst")
            .map(|t| key_actions(&t, &strings))
            .unwrap_or_default(),
        colors,
        numbers,
        ..Default::default()
    };
    for (page, file) in OPTION_PAGES {
        if let Some(root) = read_res_pc(&mut read, file) {
            let layout = super::vgui::layout(&root, &strings, &ui.colors, &mut |_| None);
            ui.options.insert(page.to_string(), layout);
        }
    }
    // The loading dialog: CS:S shows the one without the banner.
    ui.loading = LOADING_DIALOGS.iter().find_map(|file| {
        let root = read_res_pc(&mut read, file)?;
        Some(super::vgui::layout(&root, &strings, &ui.colors, &mut |_| None))
    });
    // The server browser: its own words first, then its layouts.
    if let Some(text) = read(SERVER_STRINGS) {
        for (k, v) in super::radio::localization(&text) {
            strings.entry(k).or_insert(v);
        }
    }
    for (name, file) in SERVER_LAYOUTS {
        if let Some(root) = read_res_pc(&mut read, file) {
            let layout = super::vgui::layout(&root, &strings, &ui.colors, &mut |_| None);
            ui.servers.insert(name.to_string(), layout);
        }
    }
    for name in SERVER_ICONS {
        let path = format!("servers/icon_{name}.tga");
        if let Some(pic) = mount.read(&path).ok().and_then(|b| decode_tga(&b)) {
            ui.server_icons.insert(name.to_string(), pic);
        }
    }
    ui.strings = strings;
    for (sound, file) in UI_SOUNDS {
        if let Some(clip) = mount.read(file).ok().and_then(|b| super::wav::decode(&b).ok()) {
            ui.sounds.insert(sound, clip);
        }
    }
    main_menu_look(mount, &mut ui);
    // Create Server's Game page: the server's own script first.
    ui.server_settings = SERVER_SCRIPTS
        .iter()
        .find_map(|file| read(file))
        .map(|text| server_settings(&text, &ui.strings))
        .unwrap_or_default();
    Some(ui)
}

/// The options of Create Server's Game page (`cfg/settings.scr`): each
/// cvar's label (localised), its control and default. The script's form:
/// `"cvar" { "#Label" { STRING | NUMBER min max | BOOL | LIST "label"
/// "value" ... } { "default" } }` (a max of -1: none).
pub(crate) fn server_settings(text: &str, strings: &HashMap<String, String>) -> Vec<ServerSetting> {
    let t = super::surfaceprops::tokens(text);
    let Some(open) = t.iter().position(|s| s == "{") else {
        return Vec::new();
    };
    let mut out = Vec::new();
    let mut i = open + 1;
    let get = |i: usize| t.get(i).map(String::as_str).unwrap_or("");
    while i < t.len() && get(i) != "}" {
        // "cvar" { "label" { type ... } { "default" } }
        let cvar = get(i).to_string();
        if get(i + 1) != "{" {
            break;
        }
        let label = localise(get(i + 2), strings);
        if get(i + 3) != "{" {
            break;
        }
        let mut j = i + 4;
        let kind_word = get(j).to_uppercase();
        j += 1;
        let mut args = Vec::new();
        while j < t.len() && get(j) != "}" {
            args.push(get(j).to_string());
            j += 1;
        }
        j += 1; // the type's closing brace
        let num = |s: Option<&String>| s.and_then(|v| v.parse::<f32>().ok()).filter(|v| *v >= 0.0);
        let kind = match kind_word.as_str() {
            "NUMBER" => ServerSettingKind::Number {
                min: num(args.first()),
                max: num(args.get(1)),
            },
            "BOOL" => ServerSettingKind::Bool,
            "LIST" => ServerSettingKind::List(args.chunks_exact(2).map(|p| (p[0].clone(), p[1].clone())).collect()),
            _ => ServerSettingKind::Text,
        };
        let mut default = String::new();
        if get(j) == "{" {
            default = if get(j + 1) == "}" { String::new() } else { get(j + 1).to_string() };
            while j < t.len() && get(j) != "}" {
                j += 1;
            }
            j += 1;
        }
        // The setting's own closing brace.
        while j < t.len() && get(j) != "}" {
            j += 1;
        }
        out.push(ServerSetting {
            cvar,
            label,
            kind,
            default,
        });
        i = j + 1;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn conditions_as_the_windows_pc_reads_them() {
        assert!(condition_holds("[$WIN32]"));
        assert!(condition_holds("[!$OSX]"));
        assert!(!condition_holds("[$X360]"));
        assert!(!condition_holds("[$POSIX]"));
        assert!(!condition_holds("[$LINUX]"));
        assert!(condition_holds("[$X360 || $WIN32]"));
        assert!(!condition_holds("[$WIN32 && $X360]"));
    }

    #[test]
    fn scheme_entries_for_other_platforms_are_dropped() {
        let kv = parse_pc(
            r#"Scheme {
                Colors { "White" "255 255 255 255" }
                BaseSettings {
                    Frame.BgColor "160 160 160 128" [$WIN32]
                    Frame.BgColor "80 80 80 192" [$X360]
                    Button.TextColor "White"
                    Frame.ClientInsetX 8
                }
                Fonts {
                    "UiBold" {
                        "1" [$WIN32] { "name" "Tahoma" [!$OSX] "name" "Verdana" [$OSX] "tall" "12" [!$LINUX] "tall" "15" [$LINUX] }
                        "1" [$X360] { "name" "Tahoma" "tall" "24" }
                    }
                }
            }"#,
        );
        let scheme = &kv.items()[0].1;
        let (colors, numbers) = scheme_settings(scheme);
        assert_eq!(colors["Frame.BgColor"], [160, 160, 160, 128]);
        assert_eq!(colors["Button.TextColor"], [255, 255, 255, 255]);
        assert_eq!(numbers["Frame.ClientInsetX"], 8.0);
        let fonts = super::super::vgui::fonts(scheme);
        assert_eq!(fonts["UiBold"].len(), 1);
        assert_eq!((fonts["UiBold"][0].family.as_str(), fonts["UiBold"][0].tall), ("Tahoma", 12.0));
    }

    #[test]
    fn kb_act_sections_and_actions() {
        let strings = HashMap::from([
            ("valve_movement_title".to_string(), "Movement".to_string()),
            ("valve_move_forward".to_string(), "Move forward".to_string()),
        ]);
        let list = key_actions(
            "\"blank\"\t\t\"==========\"\n\"blank\" \"#Valve_Movement_Title\"\n\"blank\" \"=====\"\n\
             \"+forward\"\t\"#Valve_Move_Forward\"\n\"impulse 100\" \"#Valve_Flashlight\"\n",
            &strings,
        );
        assert_eq!(
            list,
            [
                KeyAction::Section("Movement".into()),
                KeyAction::Action {
                    command: "+forward".into(),
                    label: "Move forward".into()
                },
                KeyAction::Action {
                    command: "impulse 100".into(),
                    label: "#Valve_Flashlight".into()
                },
            ]
        );
    }

    #[test]
    fn game_menu_entries_in_order() {
        let strings = HashMap::from([("gameui_gamemenu_quit".to_string(), "QUIT".to_string())]);
        let root = parse_pc(
            r##""GameMenu" {
                "1" { "label" "#GameUI_GameMenu_ResumeGame" "command" "ResumeGame" "OnlyInGame" "1" }
                "4" { "label" "" "command" "" }
                "11" { "label" "#GameUI_GameMenu_Quit" "command" "Quit" }
            }"##,
        );
        let items = game_menu(&root.items()[0].1, &strings);
        assert_eq!(items.len(), 3);
        assert!(items[0].in_game_only);
        assert_eq!(items[0].command, "ResumeGame");
        assert_eq!(items[1].label, "");
        assert_eq!((items[2].label.as_str(), items[2].command.as_str()), ("QUIT", "Quit"));
    }

    #[test]
    fn the_create_server_game_page_options() {
        let strings = HashMap::from([("valve_hostname".to_string(), "Server Name".to_string())]);
        let scr = r##"VERSION 1.0
            DESCRIPTION SERVER_OPTIONS
            {
                "hostname" { "#Valve_Hostname" { STRING } { "Counter-Strike Source" } }
                "maxplayers" { "#Valve_Max_Players" { NUMBER 1 32 } { "32" } }
                "mp_timelimit" { "Time" { NUMBER 0 -1 } { "20" } }
                // a comment
                "mp_footsteps" { "Steps" { BOOL } { "1" } }
                "mp_forcecamera" { "Camera" { LIST "#A" "0" "B" "1" } { "0" } }
            }"##;
        let o = server_settings(scr, &strings);
        assert_eq!(o.len(), 5);
        assert_eq!((o[0].cvar.as_str(), o[0].label.as_str()), ("hostname", "Server Name"));
        assert_eq!(o[0].kind, ServerSettingKind::Text);
        assert_eq!(o[0].default, "Counter-Strike Source");
        assert_eq!(o[1].kind, ServerSettingKind::Number { min: Some(1.0), max: Some(32.0) });
        assert_eq!(o[2].kind, ServerSettingKind::Number { min: Some(0.0), max: None }, "-1: no limit");
        assert_eq!(o[3].kind, ServerSettingKind::Bool);
        assert_eq!(
            o[4].kind,
            ServerSettingKind::List(vec![("#A".into(), "0".into()), ("B".into(), "1".into())]),
            "unknown tokens stay as written"
        );
        assert!(server_settings("nothing here", &strings).is_empty());
    }

    #[test]
    fn title_lines_from_gameinfo() {
        let info = "\"GameInfo\"\n{\n\tgame\t\"Some Game\"\n\ttitle\t\"SOME GAME'\"\n\ttitle2\t\"two\"\n\
                    \ttype multiplayer_only\n\tFileSystem { SteamAppId 1 }\n}\n";
        assert_eq!(game_title(info), ["SOME GAME'", "two"]);
        assert!(game_title("GameInfo { title \"\" }").is_empty());
        assert!(game_title("").is_empty());
    }

    #[test]
    fn title_font_and_its_files() {
        let kv = parse_pc(
            r#"Scheme {
                Fonts {
                    ClientTitleFont { "1" { "name" "Title Face" "tall" "60" "weight" "0" } }
                }
                CustomFontFiles { "1" "resource/a.ttf" "2" { "font" "resource/b.ttf" "name" "B" } }
            }"#,
        );
        let scheme = &kv.items()[0].1;
        let (family, tall, files) = title_font(scheme).unwrap();
        assert_eq!((family.as_str(), tall), ("Title Face", 60.0));
        assert_eq!(files, ["resource/a.ttf", "resource/b.ttf"]);
        assert!(title_font(&parse_pc("Scheme { Fonts { } }").items()[0].1).is_none());
    }

    #[test]
    fn base_files_merge() {
        let files = HashMap::from([
            ("resource/sourcescheme.res", "#base \"SourceSchemeBase.res\"".to_string()),
            (
                "resource/SourceSchemeBase.res",
                "Scheme { Colors { \"Orange\" \"255 155 0 255\" } }".to_string(),
            ),
        ]);
        let mut read = |p: &str| files.get(p).cloned();
        let kv = read_res_pc(&mut read, "resource/sourcescheme.res").unwrap();
        assert_eq!(kv.get("Colors").unwrap().str("Orange"), Some("255 155 0 255"));
    }
}
