//! A game's radio (Counter-Strike's team radio): which commands exist,
//! the menus that list them, the sound entry and chat text each says, and
//! the chat line format; also the players' own text chat formats
//! (`SayFormats`). Games fill `RadioCommands` from their data; the
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
    /// Players' text chat (`say`, `say_team`).
    pub say: SayFormats,
}

/// The join line key (`SayFormats::joins`) of the spectators, who have no
/// team of their own (`core::Spectating`).
pub const SPECTATORS: u8 = 3;

/// Player text chat lines (`say`, `say_team`): `%s1` the sender, `%s2` the
/// text, `%s3` the sender's place, with Source's colour control characters
/// (as `RadioCommands::format`).
#[derive(Clone, Debug, PartialEq)]
pub struct SayFormats {
    pub all: String,
    pub all_dead: String,
    /// From a player on no team.
    pub all_spectator: String,
    /// Team chat by team number.
    pub team: Vec<(u8, TeamSay)>,
    /// Team chat from a player on no team.
    pub team_spectator: String,
    /// The sound entry a chat line plays.
    pub sound: Option<String>,
    /// "%s1 is joining the Terrorist force" by team number (`%s1` the
    /// player), the spectators under `SPECTATORS`.
    pub joins: Vec<(u8, String)>,
    /// "* %s1 changed name to %s2".
    pub name_change: String,
}

/// One team's chat formats: alive, alive at a known place, dead.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct TeamSay {
    pub alive: String,
    pub alive_place: Option<String>,
    pub dead: String,
}

impl Default for SayFormats {
    /// A generic look (CS:S's without team names).
    fn default() -> Self {
        Self {
            all: "\u{2}%s1 :  %s2".into(),
            all_dead: "\u{1}*DEAD* \u{3}%s1\u{1} :  %s2".into(),
            all_spectator: "\u{1}*SPEC* \u{3}%s1\u{1} :  %s2".into(),
            team: Vec::new(),
            team_spectator: "\u{1}(Spectator) \u{3}%s1\u{1} :  %s2".into(),
            sound: None,
            joins: Vec::new(),
            name_change: "* %s1 changed name to %s2".into(),
        }
    }
}

impl SayFormats {
    /// The chat line saying `old` is now called `new`.
    pub fn renamed(&self, old: &str, new: &str) -> Vec<(ChatColor, String)> {
        colour_runs(&fill(&self.name_change, &[old, new]), old)
    }

    /// The chat line saying `player` joins `team`, when the game has one.
    pub fn join(&self, player: &str, team: u8) -> Option<Vec<(ChatColor, String)>> {
        let (_, f) = self.joins.iter().find(|(t, _)| *t == team)?;
        Some(colour_runs(&fill(f, &[player]), player))
    }

    /// The line `sender` (of `team`, alive or not, at `place`) says to
    /// everyone or (`team_only`) to their team, as coloured runs.
    pub fn line(
        &self,
        sender: &str,
        text: &str,
        team: Option<u8>,
        alive: bool,
        team_only: bool,
        place: Option<&str>,
    ) -> Vec<(ChatColor, String)> {
        let own = team.and_then(|t| self.team.iter().find(|(n, _)| *n == t).map(|(_, f)| f));
        let generic_team = TeamSay {
            alive: "\u{1}(Team) \u{3}%s1\u{1} :  %s2".into(),
            alive_place: None,
            dead: "\u{1}*DEAD*(Team) \u{3}%s1\u{1} :  %s2".into(),
        };
        let format = match (team_only, team, alive) {
            (false, None, _) => &self.all_spectator,
            (false, _, true) => &self.all,
            (false, _, false) => &self.all_dead,
            (true, None, _) => &self.team_spectator,
            (true, Some(_), _) => {
                let f = own.unwrap_or(&generic_team);
                match (alive, place, &f.alive_place) {
                    (true, Some(p), Some(with_place)) => {
                        return colour_runs(&fill(with_place, &[sender, text, p]), sender);
                    }
                    (true, ..) => &f.alive,
                    (false, ..) => &f.dead,
                }
            }
        };
        colour_runs(&fill(format, &[sender, text]), sender)
    }
}

/// Whether a viewer reads a chat line: team chat only within the team,
/// and the living don't read the dead (CS:S).
pub fn sees_say(
    viewer_alive: bool,
    viewer_team: Option<u8>,
    sender_alive: bool,
    sender_team: Option<u8>,
    team_only: bool,
) -> bool {
    (!team_only || viewer_team == sender_team) && (sender_alive || !viewer_alive)
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
            say: SayFormats::default(),
        }
    }

    #[test]
    fn say_lines_like_the_game() {
        let mut f = SayFormats::default();
        f.team.push((
            2,
            TeamSay {
                alive: "\u{1}(Counter-Terrorist) \u{3}%s1\u{1} :  %s2".into(),
                alive_place: Some("\u{1}(Counter-Terrorist) \u{3}%s1\u{1} @ \u{4}%s3\u{1} :  %s2".into()),
                dead: "\u{1}*DEAD*(Counter-Terrorist) \u{3}%s1\u{1} :  %s2".into(),
            },
        ));
        let text = |runs: Vec<(ChatColor, String)>| runs.into_iter().map(|r| r.1).collect::<String>();
        assert_eq!(
            f.line("Player", "hi", Some(2), true, false, None),
            [
                (ChatColor::Team, "Player".to_string()),
                (ChatColor::Normal, " :  hi".to_string())
            ]
        );
        assert_eq!(text(f.line("Player", "hi", Some(2), false, false, None)), "*DEAD* Player :  hi");
        assert_eq!(
            text(f.line("Player", "go", Some(2), true, true, None)),
            "(Counter-Terrorist) Player :  go"
        );
        assert_eq!(
            f.line("Player", "go", Some(2), true, true, Some("BombsiteA"))[3],
            (ChatColor::Location, "BombsiteA".to_string())
        );
        assert_eq!(
            text(f.line("Player", "go", Some(2), false, true, Some("BombsiteA"))),
            "*DEAD*(Counter-Terrorist) Player :  go"
        );
        assert_eq!(text(f.line("Player", "x", Some(1), true, true, None)), "(Team) Player :  x");
        assert_eq!(text(f.line("Player", "x", None, true, false, None)), "*SPEC* Player :  x");
        // Who reads it.
        assert!(sees_say(true, Some(2), true, Some(1), false));
        assert!(!sees_say(true, Some(2), true, Some(1), true), "other team's team chat");
        assert!(!sees_say(true, Some(2), false, Some(2), false), "the dead to the living");
        assert!(sees_say(false, Some(1), false, Some(2), false), "the dead to the dead");
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
