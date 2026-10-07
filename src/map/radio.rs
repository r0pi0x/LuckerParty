//! A game's radio (Counter-Strike's team radio): which commands exist,
//! the menus that list them, the sound entry and chat text each says, and
//! the chat line format. Games fill `RadioCommands` from their data; the
//! client opens the menus, registers the commands and plays what
//! `core::Radio` messages say to the players who hear them.

use bevy::prelude::*;

/// One thing the radio can say. Several variants are picked from at
/// random (CS:S's "Affirmative" / "Roger that"); each pairs a sound entry
/// with its text.
#[derive(Clone, Debug, PartialEq)]
pub struct RadioCommand {
    /// The console name (`coverme`); also what `core::Radio` carries.
    pub command: String,
    /// Menu (0-based) and item (1-based number key), when a menu lists it.
    pub menu: Option<(usize, usize)>,
    /// Sound entry and chat text.
    pub variants: Vec<(String, String)>,
}

/// A radio menu: its title and item lines as the game shows them.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct RadioMenu {
    pub title: String,
    /// (number key, label).
    pub items: Vec<(usize, String)>,
}

/// One coloured run of a chat line.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChatColor {
    Normal,
    Team,
    Location,
}

/// The loaded map's radio, when the game has one.
#[derive(Resource, Clone, Debug, Default, PartialEq)]
pub struct RadioCommands {
    pub commands: Vec<RadioCommand>,
    pub menus: Vec<RadioMenu>,
    /// The chat line, `%s1` the sender, `%s2` the text, with Source's
    /// colour control characters (0x01 normal, 0x02 team colour up to the
    /// end of the name, 0x03 team colour, 0x04 location colour).
    pub format: String,
    /// The same with the sender's place: `%s1` sender, `%s2` place, `%s3`
    /// text.
    pub format_location: Option<String>,
}

impl RadioCommands {
    pub fn get(&self, command: &str) -> Option<&RadioCommand> {
        self.commands.iter().find(|c| c.command.eq_ignore_ascii_case(command))
    }

    /// The command behind number key `n` of menu `menu` (0-based).
    pub fn pick(&self, menu: usize, n: usize) -> Option<&RadioCommand> {
        self.commands.iter().find(|c| c.menu == Some((menu, n)))
    }

    /// Every sound entry any command plays.
    pub fn sound_entries(&self) -> impl Iterator<Item = &str> {
        self.commands
            .iter()
            .flat_map(|c| c.variants.iter().map(|(s, _)| s.as_str()))
    }

    /// The chat line for `sender` saying `text` (from `place`, when known),
    /// as coloured runs.
    pub fn line(&self, sender: &str, text: &str, place: Option<&str>) -> Vec<(ChatColor, String)> {
        let filled = match (place, &self.format_location) {
            (Some(p), Some(f)) => fill(f, &[sender, p, text]),
            _ => fill(&self.format, &[sender, text]),
        };
        colour_runs(&filled, sender)
    }
}

/// Replace `%s1`, `%s2`... with `args`.
fn fill(format: &str, args: &[&str]) -> String {
    let mut out = format.to_string();
    // Highest first, so %s1 doesn't eat the start of %s10.
    for (i, a) in args.iter().enumerate().rev() {
        out = out.replace(&format!("%s{}", i + 1), a);
    }
    out
}

/// Split a line at Source's chat colour control characters. A leading
/// 0x02 colours up to the end of `name` in the team colour.
pub fn colour_runs(line: &str, name: &str) -> Vec<(ChatColor, String)> {
    let mut runs: Vec<(ChatColor, String)> = Vec::new();
    let mut push = |c: ChatColor, s: &str| {
        if !s.is_empty() {
            runs.push((c, s.to_string()));
        }
    };
    if let Some(rest) = line.strip_prefix('\u{2}') {
        let split = rest.find(name).map_or(0, |i| i + name.len());
        push(ChatColor::Team, &rest[..split]);
        push(ChatColor::Normal, &rest[split..]);
        return runs;
    }
    let mut colour = ChatColor::Normal;
    let mut start = 0;
    for (i, ch) in line.char_indices() {
        let next = match ch {
            '\u{1}' => ChatColor::Normal,
            '\u{3}' => ChatColor::Team,
            '\u{4}' => ChatColor::Location,
            _ => continue,
        };
        push(colour, &line[start..i]);
        colour = next;
        start = i + ch.len_utf8();
    }
    push(colour, &line[start..]);
    runs
}

/// Whether the local player hears a radio call: their own, or a living
/// teammate's (a team of None hears nobody else).
pub fn hears(
    local: Entity,
    local_team: Option<u8>,
    sender: Entity,
    sender_team: Option<u8>,
    sender_alive: bool,
) -> bool {
    if sender == local {
        return sender_alive;
    }
    sender_alive && local_team.is_some() && local_team == sender_team
}

#[cfg(test)]
mod tests {
    use super::*;

    fn radio() -> RadioCommands {
        RadioCommands {
            commands: vec![
                RadioCommand {
                    command: "coverme".into(),
                    menu: Some((0, 1)),
                    variants: vec![("Radio.CoverMe".into(), "Cover Me!".into())],
                },
                RadioCommand {
                    command: "roger".into(),
                    menu: Some((2, 1)),
                    variants: vec![
                        ("Radio.Affirmitive".into(), "Affirmative.".into()),
                        ("Radio.Roger".into(), "Roger that.".into()),
                    ],
                },
            ],
            menus: Vec::new(),
            format: "\u{2}%s1 (RADIO): %s2".into(),
            format_location: Some("\u{3}%s1\u{1} @ \u{4}%s2\u{1} (RADIO): %s3".into()),
        }
    }

    #[test]
    fn menu_keys_find_commands_and_sounds() {
        let r = radio();
        assert_eq!(r.pick(0, 1).map(|c| c.command.as_str()), Some("coverme"));
        assert_eq!(r.pick(2, 1).map(|c| c.variants.len()), Some(2));
        assert!(r.pick(0, 2).is_none());
        assert_eq!(r.get("COVERME").unwrap().variants[0].0, "Radio.CoverMe");
        assert_eq!(
            r.sound_entries().collect::<Vec<_>>(),
            ["Radio.CoverMe", "Radio.Affirmitive", "Radio.Roger"]
        );
    }

    #[test]
    fn lines_are_coloured_like_the_game() {
        let r = radio();
        assert_eq!(
            r.line("Bot 1", "Cover Me!", None),
            [
                (ChatColor::Team, "Bot 1".to_string()),
                (ChatColor::Normal, " (RADIO): Cover Me!".to_string())
            ]
        );
        assert_eq!(
            r.line("Bot 1", "Enemy spotted.", Some("BombsiteA")),
            [
                (ChatColor::Team, "Bot 1".to_string()),
                (ChatColor::Normal, " @ ".to_string()),
                (ChatColor::Location, "BombsiteA".to_string()),
                (ChatColor::Normal, " (RADIO): Enemy spotted.".to_string())
            ]
        );
        assert_eq!(colour_runs("plain", "x"), [(ChatColor::Normal, "plain".to_string())]);
    }

    #[test]
    fn only_living_teammates_and_yourself_are_heard() {
        let (me, mate) = (Entity::from_raw_u32(1).unwrap(), Entity::from_raw_u32(2).unwrap());
        assert!(hears(me, Some(2), me, Some(2), true));
        assert!(hears(me, Some(2), mate, Some(2), true));
        assert!(!hears(me, Some(2), mate, Some(1), true), "enemy");
        assert!(!hears(me, Some(2), mate, Some(2), false), "dead sender");
        assert!(!hears(me, None, mate, None, true), "no team");
        assert!(!hears(me, Some(2), me, Some(2), false), "dead self");
    }
}
