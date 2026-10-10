//! Bot personalities from the game's bot profile file (Source's
//! `botprofile.db`, read from the install at run time): each profile's
//! name, the difficulties it plays at and its team, for naming bots as
//! the game does. The file's own comments describe its layout: a
//! `Default` block, `Template <name>` blocks, then one block per bot,
//! `<Template>[+<Template>...] <Name>`, each block `Key = Value` lines up
//! to `End`; a profile takes Default's values, then each template's in
//! order, then its own. Bots buy by a profile's `WeaponPreference` lines
//! (`bot::buy`): a block that lists any replaces the inherited list.

use bevy::prelude::*;

/// `BotProfile::difficulty` bits (the file's EASY, NORMAL, HARD, EXPERT;
/// `bot_difficulty` 0-3).
pub mod difficulty {
    pub const EASY: u8 = 1;
    pub const NORMAL: u8 = 1 << 1;
    pub const HARD: u8 = 1 << 2;
    pub const EXPERT: u8 = 1 << 3;

    /// The bit for `bot_difficulty`'s number (0 easy ... 3 expert).
    pub fn of_level(level: u8) -> u8 {
        1 << level.min(3)
    }

    /// The bit for a name (`easy`, `normal`, `hard`, `expert`).
    pub fn of_name(name: &str) -> Option<u8> {
        match name.to_ascii_lowercase().as_str() {
            "easy" => Some(EASY),
            "normal" => Some(NORMAL),
            "hard" => Some(HARD),
            "expert" => Some(EXPERT),
            _ => None,
        }
    }
}

/// One bot personality.
#[derive(Clone, Debug, PartialEq)]
pub struct BotProfile {
    pub name: String,
    /// `difficulty::*` bits it plays at.
    pub difficulty: u8,
    /// The team it joins (our numbers: 1 terrorists, 2 counter-terrorists);
    /// None: either.
    pub team: Option<u8>,
    /// Weapons it buys first, best first, as the file names them
    /// (lowercase; "none" possible).
    pub weapons: Vec<String>,
}

/// The game's bot profiles, in the file's order (a resource while a map
/// of that game is loaded).
#[derive(Resource, Clone, Debug, Default, PartialEq)]
pub struct BotProfiles(pub Vec<BotProfile>);

/// The attributes a block sets that naming uses.
#[derive(Clone, Default)]
struct Attrs {
    difficulty: Option<u8>,
    team: Option<Option<u8>>,
    weapons: Option<Vec<String>>,
}

impl Attrs {
    fn over(self, base: Attrs) -> Attrs {
        Attrs {
            difficulty: self.difficulty.or(base.difficulty),
            team: self.team.or(base.team),
            weapons: self.weapons.or(base.weapons),
        }
    }
}

fn difficulty_bits(value: &str) -> u8 {
    value
        .split('+')
        .filter_map(|d| difficulty::of_name(d.trim()))
        .fold(0, |a, b| a | b)
}

fn team_of(value: &str) -> Option<u8> {
    match value.trim().to_ascii_uppercase().as_str() {
        "T" | "TERRORIST" => Some(1),
        "CT" | "COUNTER-TERRORIST" => Some(2),
        _ => None,
    }
}

impl BotProfiles {
    /// Read a bot profile file (Source's layout; see the module docs).
    /// Lines it doesn't understand are skipped.
    pub fn parse(text: &str) -> Self {
        let mut default = Attrs::default();
        let mut templates: Vec<(String, Attrs)> = Vec::new();
        let mut out = Vec::new();
        // The block being read: what it is, and what it set.
        enum Block {
            Default,
            Template(String),
            Profile { name: String, bases: Vec<String> },
        }
        let mut block: Option<(Block, Attrs)> = None;
        for line in text.lines() {
            let line = line.split("//").next().unwrap_or("").trim();
            if line.is_empty() {
                continue;
            }
            if line.eq_ignore_ascii_case("End") {
                match block.take() {
                    Some((Block::Default, a)) => default = a,
                    Some((Block::Template(name), a)) => templates.push((name, a)),
                    Some((Block::Profile { name, bases }, own)) => {
                        let mut a = default.clone();
                        for b in &bases {
                            if let Some((_, t)) = templates.iter().find(|(n, _)| n.eq_ignore_ascii_case(b)) {
                                a = t.clone().over(a);
                            }
                        }
                        let a = own.over(a);
                        out.push(BotProfile {
                            name,
                            difficulty: a.difficulty.unwrap_or(difficulty::NORMAL),
                            team: a.team.flatten(),
                            weapons: a.weapons.unwrap_or_default(),
                        });
                    }
                    None => {}
                }
                continue;
            }
            if let Some((_, attrs)) = &mut block {
                if let Some((key, value)) = line.split_once('=') {
                    match key.trim().to_ascii_lowercase().as_str() {
                        "difficulty" => attrs.difficulty = Some(difficulty_bits(value)),
                        "team" => attrs.team = Some(team_of(value)),
                        "weaponpreference" => attrs
                            .weapons
                            .get_or_insert_with(Vec::new)
                            .push(value.trim().to_ascii_lowercase()),
                        _ => {}
                    }
                }
                continue;
            }
            let mut words = line.split_whitespace();
            let first = words.next().unwrap_or("");
            let rest: Vec<&str> = words.collect();
            let kind = if first.eq_ignore_ascii_case("Default") && rest.is_empty() {
                Block::Default
            } else if first.eq_ignore_ascii_case("Template") && !rest.is_empty() {
                Block::Template(rest.join(" "))
            } else if !rest.is_empty() {
                Block::Profile {
                    name: rest.join(" "),
                    bases: first.split('+').map(str::to_string).collect(),
                }
            } else {
                continue;
            };
            block = Some((kind, Attrs::default()));
        }
        Self(out)
    }

    /// The profile a new bot takes: one playing at `level`'s difficulty
    /// bit, for `team`, whose name no bot in `taken` has; else one of any
    /// difficulty not taken. `pick` chooses among them (the game picks at
    /// random; callers pass a seeded number so runs repeat).
    pub fn choose(&self, level: u8, team: u8, taken: &[String], pick: u64) -> Option<&BotProfile> {
        let free = |p: &&BotProfile| {
            p.team.is_none_or(|t| t == team) && !taken.iter().any(|n| n.eq_ignore_ascii_case(&p.name))
        };
        let at_level: Vec<&BotProfile> = self.0.iter().filter(free).filter(|p| p.difficulty & level != 0).collect();
        let pool = if at_level.is_empty() {
            self.0.iter().filter(free).collect()
        } else {
            at_level
        };
        (!pool.is_empty()).then(|| pool[(pick % pool.len() as u64) as usize])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A file laid out as the game's (made up here).
    const FILE: &str = "\
// comment
Default
\tSkill = 50
\tDifficulty = NORMAL
End

Template Rifle
\tWeaponPreference = m4a1   // a gun
End

Template Top
\tDifficulty = EXPERT
End

Template Mid
\tDifficulty = NORMAL+HARD
End

Top+Rifle Alpha
\tSkin = 1
End

Mid Bravo
End

Rifle Charlie
\tTeam = T
\tWeaponPreference = AWP
\tWeaponPreference = ak47
End

Top Delta
\tDifficulty = EASY
End
";

    #[test]
    fn profiles_take_their_templates_in_order() {
        let p = BotProfiles::parse(FILE);
        let got: Vec<(&str, u8, Option<u8>)> = p.0.iter().map(|p| (p.name.as_str(), p.difficulty, p.team)).collect();
        use difficulty::*;
        let weapons: Vec<&[String]> = p.0.iter().map(|p| &p.weapons[..]).collect();
        assert_eq!(weapons[0], ["m4a1"], "Rifle's");
        assert!(weapons[1].is_empty());
        assert_eq!(weapons[2], ["awp", "ak47"], "its own replace Rifle's");
        assert_eq!(
            got,
            [
                ("Alpha", EXPERT, None),
                ("Bravo", NORMAL | HARD, None),
                ("Charlie", NORMAL, Some(1)),
                ("Delta", EASY, None),
            ]
        );
    }

    #[test]
    fn bots_get_free_names_at_their_difficulty() {
        let p = BotProfiles::parse(FILE);
        let normal = difficulty::of_level(1);
        let names = |taken: &[&str], team: u8, pick: u64| {
            let taken: Vec<String> = taken.iter().map(|s| s.to_string()).collect();
            p.choose(normal, team, &taken, pick).map(|p| p.name.clone())
        };
        // Normal: Bravo and (terrorists only) Charlie.
        assert_eq!(names(&[], 1, 0), Some("Bravo".into()));
        assert_eq!(names(&[], 1, 1), Some("Charlie".into()));
        assert_eq!(names(&[], 2, 1), Some("Bravo".into()), "Charlie is a terrorist");
        // Taken at that difficulty: another one.
        assert_eq!(names(&["Bravo"], 2, 0), Some("Alpha".into()));
        assert_eq!(names(&["Alpha", "Bravo", "Charlie", "Delta"], 1, 0), None);
        assert_eq!(difficulty::of_name("Hard"), Some(difficulty::HARD));
    }
}
