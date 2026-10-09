//! CS:S's team radio from the install: the three menus' titles and labels
//! (`RadioA`/`RadioB`/`RadioC` in `resource/cstrike_english.txt`), each
//! command's chat text (its `Cstrike_TitlesTXT_*` string) and sound entry
//! (`scripts/game_sounds_radio.txt`), and the chat line format
//! (`Game_radio`, `Game_radio_location`), and the players' text chat
//! formats (`Cstrike_Chat_*`). Only the pairing of console
//! names, menu slots, sound entries and text keys is ours; it follows the
//! menus' own order.

use std::collections::HashMap;

use super::material::MaterialLoader;
use crate::map::radio::{RadioCommand, RadioCommands, RadioMenu, SayFormats, TeamSay};

/// The sound a chat line plays (Source's chat HUD entry).
pub const CHAT_SOUND: &str = "HudChat.Message";

/// Console name, menu (0-based) and key, then (sound entry, text key)
/// variants. `fireinhole` is said on a grenade throw, from no menu.
type Entry = (
    &'static str,
    Option<(usize, usize)>,
    &'static [(&'static str, &'static str)],
);

const COMMANDS: &[Entry] = &[
    ("coverme", Some((0, 1)), &[("Radio.CoverMe", "Cover_me")]),
    (
        "takepoint",
        Some((0, 2)),
        &[("Radio.YouTakeThePoint", "You_take_the_point")],
    ),
    ("holdpos", Some((0, 3)), &[("Radio.HoldPosition", "Hold_this_position")]),
    ("regroup", Some((0, 4)), &[("Radio.Regroup", "Regroup_team")]),
    ("followme", Some((0, 5)), &[("Radio.FollowMe", "Follow_me")]),
    ("takingfire", Some((0, 6)), &[("Radio.TakingFire", "Taking_fire")]),
    ("go", Some((1, 1)), &[("Radio.GoGoGo", "Go_go_go")]),
    ("fallback", Some((1, 2)), &[("Radio.TeamFallBack", "Team_fall_back")]),
    (
        "sticktog",
        Some((1, 3)),
        &[("Radio.StickTogether", "Stick_together_team")],
    ),
    (
        "getinpos",
        Some((1, 4)),
        &[("Radio.GetInPosition", "Get_in_position_and_wait")],
    ),
    ("stormfront", Some((1, 5)), &[("Radio.StormFront", "Storm_the_front")]),
    ("report", Some((1, 6)), &[("Radio.ReportInTeam", "Report_in_team")]),
    (
        "roger",
        Some((2, 1)),
        &[("Radio.Affirmitive", "Affirmative"), ("Radio.Roger", "Roger_that")],
    ),
    ("enemyspot", Some((2, 2)), &[("Radio.EnemySpotted", "Enemy_spotted")]),
    ("needbackup", Some((2, 3)), &[("Radio.NeedBackup", "Need_backup")]),
    ("sectorclear", Some((2, 4)), &[("Radio.SectorClear", "Sector_clear")]),
    ("inposition", Some((2, 5)), &[("Radio.InPosition", "In_position")]),
    ("reportingin", Some((2, 6)), &[("Radio.ReportingIn", "Reporting_in")]),
    ("getout", Some((2, 7)), &[("Radio.GetOutOfThere", "Get_out_of_there")]),
    ("negative", Some((2, 8)), &[("Radio.Negative", "Negative")]),
    ("enemydown", Some((2, 9)), &[("Radio.EnemyDown", "Enemy_down")]),
    ("fireinhole", None, &[("Radio.FireInTheHole", "Fire_in_the_hole")]),
];

/// Every sound entry the radio uses (for the sound loader).
pub fn sound_entries() -> impl Iterator<Item = &'static str> {
    COMMANDS.iter().flat_map(|(_, _, v)| v.iter().map(|(s, _)| *s))
}

/// The radio, with the install's strings (or None without them).
pub fn load(materials: &MaterialLoader) -> Option<RadioCommands> {
    let bytes = materials.read("resource/cstrike_english.txt")?;
    Some(build(&localization(&decode(&bytes))))
}

/// Build the radio from localized strings (keys as in the file).
pub fn build(strings: &HashMap<String, String>) -> RadioCommands {
    let get = |k: &str| strings.get(&k.to_lowercase()).cloned();
    let commands = COMMANDS
        .iter()
        .map(|(command, menu, variants)| RadioCommand {
            command: command.to_string(),
            menu: *menu,
            variants: variants
                .iter()
                .map(|(sound, key)| {
                    let text = get(&format!("Cstrike_TitlesTXT_{key}")).unwrap_or_else(|| key.replace('_', " "));
                    (sound.to_string(), text)
                })
                .collect(),
        })
        .collect();
    let menus = ["RadioA", "RadioB", "RadioC"]
        .iter()
        .map(|k| get(k).map(|t| menu(&t)).unwrap_or_default())
        .collect();
    RadioCommands {
        commands,
        menus,
        format: get("Game_radio").unwrap_or_else(|| "\u{2}%s1 (RADIO): %s2".into()),
        format_location: get("Game_radio_location"),
        say: say(&get),
    }
}

/// The text chat formats (our team 1 terrorists, 2 CTs); the generic
/// look where a string is missing.
fn say(get: &dyn Fn(&str) -> Option<String>) -> SayFormats {
    let generic = SayFormats::default();
    let team = |side: &str| TeamSay {
        alive: get(&format!("Cstrike_Chat_{side}")).unwrap_or_default(),
        alive_place: get(&format!("Cstrike_Chat_{side}_Loc")),
        dead: get(&format!("Cstrike_Chat_{side}_Dead")).unwrap_or_default(),
    };
    let mut team = vec![(1, team("T")), (2, team("CT"))];
    team.retain(|(_, t)| !t.alive.is_empty() && !t.dead.is_empty());
    SayFormats {
        all: get("Cstrike_Chat_All").unwrap_or(generic.all),
        all_dead: get("Cstrike_Chat_AllDead").unwrap_or(generic.all_dead),
        all_spectator: get("Cstrike_Chat_AllSpec").unwrap_or(generic.all_spectator),
        team,
        team_spectator: get("Cstrike_Chat_Spec").unwrap_or(generic.team_spectator),
        sound: Some(CHAT_SOUND.into()),
        joins: [(1, "terrorist"), (2, "ct")]
            .into_iter()
            .filter_map(|(t, side)| Some((t, get(&format!("Cstrike_game_join_{side}"))?.trim_end().to_string())))
            .collect(),
        name_change: get("Cstrike_Name_Change")
            .map(|s| s.trim_end().to_string())
            .unwrap_or(generic.name_change),
    }
}

/// A menu text: its first line is the title, then `N. "Label"` lines.
fn menu(text: &str) -> RadioMenu {
    let mut lines = text.lines().map(str::trim).filter(|l| !l.is_empty());
    let title = lines.next().unwrap_or_default().to_string();
    let items = lines
        .filter_map(|l| {
            let (n, label) = l.split_once('.')?;
            let n: usize = n.trim().parse().ok()?;
            Some((n, label.trim().trim_matches('"').to_string()))
        })
        .filter(|(n, _)| *n != 0)
        .collect();
    RadioMenu { title, items }
}

/// Text from a localization file: UTF-16 (with a byte-order mark) or UTF-8.
pub fn decode(bytes: &[u8]) -> String {
    match bytes {
        [0xFF, 0xFE, rest @ ..] => {
            let units: Vec<u16> = rest.chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
            String::from_utf16_lossy(&units)
        }
        [0xFE, 0xFF, rest @ ..] => {
            let units: Vec<u16> = rest.chunks_exact(2).map(|c| u16::from_be_bytes([c[0], c[1]])).collect();
            String::from_utf16_lossy(&units)
        }
        _ => String::from_utf8_lossy(bytes.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(bytes)).into_owned(),
    }
}

/// A localization file's strings (lower-case keys; the first of a repeated
/// key wins, so platform variants after it are ignored). Quoted strings
/// may span lines and escape `\"`, `\n` and `\\`; `//` starts a comment
/// outside quotes; `[$X360]` conditions are dropped.
pub fn localization(text: &str) -> HashMap<String, String> {
    let mut tokens = Vec::new();
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '"' => {
                let mut s = String::new();
                while let Some(c) = chars.next() {
                    match c {
                        '"' => break,
                        '\\' => match chars.next() {
                            Some('n') => s.push('\n'),
                            Some('t') => s.push('\t'),
                            Some(o) => s.push(o),
                            None => {}
                        },
                        c => s.push(c),
                    }
                }
                tokens.push(Some(s));
            }
            '{' | '}' => tokens.push(None),
            '/' if chars.peek() == Some(&'/') => {
                for c in chars.by_ref() {
                    if c == '\n' {
                        break;
                    }
                }
            }
            c if c.is_whitespace() => {}
            c => {
                let mut s = c.to_string();
                while let Some(&n) = chars.peek() {
                    if n.is_whitespace() || n == '"' || n == '{' || n == '}' {
                        break;
                    }
                    s.push(n);
                    chars.next();
                }
                if !(s.starts_with("[$") || s.starts_with("[!$")) {
                    tokens.push(Some(s));
                }
            }
        }
    }
    // Key-value pairs: a string followed by a string (braces break pairs).
    let mut out = HashMap::new();
    let mut i = 0;
    while i + 1 < tokens.len() {
        if let (Some(k), Some(v)) = (&tokens[i], &tokens[i + 1]) {
            out.entry(k.to_lowercase()).or_insert_with(|| v.clone());
            i += 2;
        } else {
            i += 1;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "\"lang\"\n{\n\"Language\" \"English\"\n\"Tokens\"\n{\n\
        \"Cstrike_TitlesTXT_Cover_me\"\t\t\"Cover Me!\"\n\
        // a comment \"Cstrike_TitlesTXT_Cover_me\" \"no\"\n\
        \"Cstrike_TitlesTXT_Roger_that\" \"Roger that.\"\n\
        \"Cstrike_TitlesTXT_Affirmative\" \"Affirmative.\"\n\
        \"Game_radio\" \"\u{2}%s1 (RADIO): %s2\"\n\
        \"Cstrike_Chat_CT\" \"\u{1}(Counter-Terrorist) \u{3}%s1\u{1} :  %s2\"\n\
        \"Cstrike_Chat_CT_Dead\" \"\u{1}*DEAD*(Counter-Terrorist) \u{3}%s1\u{1} :  %s2\"\n\
        \"RadioC\"\n\"Radio Responses/Reports\n\n1. \\\"Affirmative/Roger\\\"\n2. \\\"Enemy Spotted\\\"\n\n0. Exit\n\"\n\
        \"Other\" \"x\" [$X360]\n\"Other\" \"y\"\n}\n}\n";

    #[test]
    fn localization_strings_with_escapes_and_conditions() {
        let s = localization(SAMPLE);
        assert_eq!(s["cstrike_titlestxt_cover_me"], "Cover Me!");
        assert_eq!(s["other"], "x");
        assert!(s["radioc"].contains("1. \"Affirmative/Roger\""));
        let utf16: Vec<u8> = [0xFF, 0xFE]
            .into_iter()
            .chain("\"a\" \"b\"".encode_utf16().flat_map(|u| u.to_le_bytes()))
            .collect();
        assert_eq!(localization(&decode(&utf16))["a"], "b");
    }

    #[test]
    fn commands_pair_menu_keys_sounds_and_text() {
        let r = build(&localization(SAMPLE));
        let roger = r.pick(2, 1).unwrap();
        assert_eq!(roger.command, "roger");
        assert_eq!(
            roger.variants,
            [
                ("Radio.Affirmitive".to_string(), "Affirmative.".to_string()),
                ("Radio.Roger".to_string(), "Roger that.".to_string())
            ]
        );
        assert_eq!(r.get("coverme").unwrap().variants[0].1, "Cover Me!");
        // Missing strings fall back to the key's words.
        assert_eq!(r.get("enemydown").unwrap().variants[0].1, "Enemy down");
        assert_eq!(r.menus[2].title, "Radio Responses/Reports");
        assert_eq!(r.menus[2].items[1], (2, "Enemy Spotted".to_string()));
        assert!(r.menus[0].items.is_empty());
        assert_eq!(r.format, "\u{2}%s1 (RADIO): %s2");
        // Text chat: the CTs' team formats; the rest generic.
        assert_eq!(r.say.team.len(), 1);
        assert_eq!(r.say.team[0].0, 2);
        assert!(r.say.team[0].1.dead.contains("*DEAD*(Counter-Terrorist)"));
        assert_eq!(r.say.all, "\u{2}%s1 :  %s2");
        assert_eq!(r.say.sound.as_deref(), Some(CHAT_SOUND));
        // Every menu has keys 1.. without gaps, and every command a sound.
        for m in 0..3 {
            let keys: Vec<usize> = r
                .commands
                .iter()
                .filter_map(|c| c.menu.filter(|x| x.0 == m))
                .map(|x| x.1)
                .collect();
            assert_eq!(keys, (1..=keys.len()).collect::<Vec<_>>());
        }
        assert_eq!(sound_entries().count(), 23);
        assert!(r.get("fireinhole").is_some_and(|c| c.menu.is_none()));
    }
}
