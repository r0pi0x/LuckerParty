//! CS:S's game menu and options look (Source's GameUI), read from the
//! install as a neutral `map::hud::GameUi`: the GameUI scheme
//! (`resource/SourceScheme.res` and the `SourceSchemeBase.res` it bases on,
//! under `platform/`), the menu's entries (`resource/GameMenu.res`), the
//! options pages (`resource/OptionsSub*.res`), the keyboard page's actions
//! (`scripts/kb_act.lst`), strings from `gameui_english.txt`,
//! `valve_english.txt` and `cstrike_english.txt`, and the new game
//! dialog's map thumbnails (`materials/vgui/maps/menu_thumb_<map>.vtf`).
//!
//! The scheme has platform conditionals (`[$WIN32]`, `[!$OSX]`,
//! `[$X360]`): entries are kept as the Windows PC game reads them.

use std::collections::HashMap;

use super::hud::Kv;
use crate::{
    map::hud::{GameUi, GameUiItem, KeyAction, UiImage},
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

/// The options pages' layout files, by page.
const OPTION_PAGES: [(&str, &str); 5] = [
    ("keyboard", "resource/optionssubkeyboard.res"),
    ("mouse", "resource/optionssubmouse.res"),
    ("audio", "resource/optionssubaudio.res"),
    ("video", "resource/optionssubvideo.res"),
    ("multiplayer", "resource/optionssubmultiplayer.res"),
];

const THUMB_PREFIX: &str = "materials/vgui/maps/menu_thumb_";

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
    ui.strings = strings;
    // Thumbnails: small pictures, decoded once.
    for (entry, _) in mount.entries() {
        let Some(map) = entry
            .path
            .strip_prefix(THUMB_PREFIX)
            .and_then(|p| p.strip_suffix(".vtf"))
        else {
            continue;
        };
        if ui.thumbnails.contains_key(map) {
            continue;
        }
        let Ok(bytes) = mount.read(&entry.path) else { continue };
        let Some(image) = vtf::from_bytes(&bytes)
            .ok()
            .and_then(|v| v.highres_image.decode(0).ok())
            .map(|i| i.to_rgba8())
        else {
            continue;
        };
        let (width, height) = (image.width(), image.height());
        ui.thumbnails.insert(map.to_string(), crop_padding(width, height, image.into_raw()));
    }
    Some(ui)
}

/// A picture without the plain rows padding its foot (the thumbnails
/// keep a 4:3 shot in a square texture).
pub(crate) fn crop_padding(width: u32, height: u32, mut rgba8: Vec<u8>) -> UiImage {
    let row = (width * 4) as usize;
    let plain = |r: &[u8]| r.chunks_exact(4).all(|p| p == &r[..4]);
    let mut rows = height as usize;
    while rows > 1 && rgba8.get((rows - 1) * row..rows * row).is_some_and(plain) {
        rows -= 1;
    }
    rgba8.truncate(rows * row);
    UiImage {
        width,
        height: rows as u32,
        rgba8,
    }
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
    fn thumbnails_lose_their_plain_foot() {
        // 2x4: two picture rows, two white ones.
        let px = [[1, 2, 3, 255], [4, 5, 6, 255], [7, 8, 9, 255], [1, 1, 1, 255], [255; 4], [255; 4], [255; 4], [255; 4]];
        let pic = crop_padding(2, 4, px.concat());
        assert_eq!((pic.width, pic.height, pic.rgba8.len()), (2, 2, 16));
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
