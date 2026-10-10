//! What bots want from the shop (`weapon::economy::BotBuying`, read by
//! `economy::autobuy` at the round's start): their profile's weapon
//! preferences from the install's `botprofile.db` (`map::bot_profiles`,
//! read at run time), else the game's weights; grenades by role; a
//! defusal kit for the defusing team. The buying rules themselves (eco,
//! full buy, armour first) are `economy`'s.

use bevy::prelude::*;

use super::{Bot, Role};
use crate::{
    core::Team,
    map::bot_profiles::BotProfiles,
    objectives::{MapObjectives, bomb::BombRules},
    weapon::{WeaponRegistry, economy::BotBuying},
};

/// Chance of buying each grenade (HE, flash, smoke) by role (ours: CS:S's
/// bot buying isn't public): attackers lean on flashes and smokes to take
/// a site, defenders on HEs to stop a rush; roamers take the old mix.
pub const GRENADES_ATTACK: [f32; 3] = [0.5, 0.7, 0.5];
pub const GRENADES_DEFEND: [f32; 3] = [0.6, 0.5, 0.3];
pub const GRENADES_ROAM: [f32; 3] = [0.6, 0.5, 0.3];

/// A profile's `WeaponPreference` name as a weapon ID's short name: the
/// file's aliases where they differ from the weapon's own (ours, from the
/// game's buy aliases).
pub fn preference_alias(name: &str) -> &str {
    match name {
        "mp5" => "mp5navy",
        "autoshotgun" => "xm1014",
        "shotgun" => "m3",
        "krieg550" => "sg550",
        "krieg552" => "sg552",
        "bullpup" => "aug",
        "d3au1" => "g3sg1",
        "defender" => "galil",
        "clarion" => "famas",
        "magnum" => "awp",
        "smg" => "mac10",
        "machinegun" => "m249",
        "fn57" => "fiveseven",
        "nighthawk" => "deagle",
        "km45" => "usp",
        "9x19mm" => "glock",
        "228compact" => "p228",
        "elites" => "elite",
        other => other,
    }
}

/// Resolve a profile's preferences to registered weapon IDs, in order
/// (unknown names and `none` skipped).
pub fn resolve(names: &[String], registry: &WeaponRegistry) -> Vec<&'static str> {
    names
        .iter()
        .filter(|n| !n.eq_ignore_ascii_case("none"))
        .filter_map(|n| {
            let short = preference_alias(&n.to_ascii_lowercase()).to_string();
            registry.find(&format!("weapon_{short}")).map(|d| d.id)
        })
        .collect()
}

/// Keep every bot's `BotBuying` in step with its profile and role.
#[allow(clippy::type_complexity)]
pub(super) fn wishes(
    mut commands: Commands,
    bots: Query<(Entity, &Bot, &Team, Option<&BotBuying>)>,
    profiles: Option<Res<BotProfiles>>,
    registry: Option<Res<WeaponRegistry>>,
    rules: Option<Res<BombRules>>,
    objectives: Option<Res<MapObjectives>>,
) {
    let bomb_map = objectives.is_some_and(|o| !o.bomb_targets.is_empty());
    let Some(registry) = registry else { return };
    for (e, bot, team, had) in &bots {
        let grenades = match bot.role() {
            Role::Attack => GRENADES_ATTACK,
            Role::Defend => GRENADES_DEFEND,
            Role::Roam => GRENADES_ROAM,
        };
        let kit = bomb_map
            && rules
                .as_ref()
                .is_some_and(|r| r.weapon.is_some() && r.defuser_team == *team);
        // (The profile never changes: its preferences are resolved once.)
        if had.is_some_and(|h| h.grenades == grenades && h.kit == kit) {
            continue;
        }
        let prefs = bot
            .profile
            .as_ref()
            .and_then(|name| profiles.as_ref()?.0.iter().find(|p| &p.name == name))
            .map(|p| resolve(&p.weapons, &registry))
            .unwrap_or_default();
        let want = BotBuying { prefs, grenades, kit };
        if had != Some(&want) {
            commands.entity(e).insert(want);
        }
    }
}
