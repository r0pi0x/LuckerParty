//! Bots and the round's objectives (`crate::objectives`), the simple
//! version: a terrorist carrying the bomb walks to the nearest bomb target
//! and, once inside with no enemy in sight, draws the bomb and holds fire
//! until it is planted; once it is, counter-terrorists walk to it and
//! hold +use on it while looking at it; terrorists guard it. A loose bomb
//! draws terrorists to it. Leading hostages, buying kits and anything
//! smarter are left for the bots' own tactics (docs/backlog.md).

use bevy::prelude::*;

use super::Bot;
use crate::{
    core::{Health, Intent, MovementState, Team},
    objectives::{
        MapObjectives, RoundOpen,
        bomb::{BombRules, C4, PlantedBomb, hull},
    },
    weapon::{Inventory, drop::Loose},
};

/// Defusers start this close to the bomb, m.
const DEFUSE_REACH: f32 = 1.2;

/// Where each bot's objective is.
#[allow(clippy::type_complexity)]
pub(super) fn goals(
    mut bots: Query<(
        &mut Bot,
        &Transform,
        Option<&MovementState>,
        &Team,
        &Health,
        Option<&Inventory>,
    )>,
    c4s: Query<(), With<C4>>,
    planted: Query<(&PlantedBomb, &Transform)>,
    loose: Query<(&Loose, &Transform)>,
    objectives: Res<MapObjectives>,
    rules: Res<BombRules>,
) {
    let bomb = planted.iter().find(|(b, _)| !b.defused).map(|(_, t)| t.translation);
    let lying = loose
        .iter()
        .find(|(l, _)| c4s.contains(l.weapon))
        .map(|(_, t)| t.translation);
    for (mut bot, t, state, team, health, inv) in &mut bots {
        if health.current <= 0.0 {
            bot.objective = None;
            continue;
        }
        let feet = t.translation.with_y(hull(t, state).0.y);
        let carrying = inv.is_some_and(|i| i.weapons.iter().any(|w| c4s.contains(*w)));
        bot.objective = if let Some(b) = bomb {
            Some(b)
        } else if carrying {
            objectives
                .bomb_targets
                .iter()
                .map(|z| z.floor_centre())
                .min_by(|a, b| a.distance(feet).total_cmp(&b.distance(feet)))
        } else if *team == rules.carrier_team {
            lying
        } else {
            None
        };
    }
}

/// Planting and defusing when there: overrides what `think` decided.
#[allow(clippy::type_complexity)]
pub(super) fn act(
    mut bots: Query<(
        &Bot,
        &mut Intent,
        &Transform,
        Option<&MovementState>,
        &Team,
        &Health,
        Option<&mut Inventory>,
    )>,
    c4s: Query<(), With<C4>>,
    planted: Query<(&PlantedBomb, &Transform)>,
    objectives: Res<MapObjectives>,
    rules: Res<BombRules>,
    open: Res<RoundOpen>,
) {
    let bomb = planted.iter().find(|(b, _)| !b.defused).map(|(_, t)| t.translation);
    for (bot, mut intent, t, state, team, health, inv) in &mut bots {
        if health.current <= 0.0 || bot.target.is_some() || !open.0 {
            continue;
        }
        let Some(mut inv) = inv else { continue };
        let (lo, hi) = hull(t, state);
        // Plant: in a target with the bomb.
        let c4 = inv.weapons.iter().copied().find(|w| c4s.contains(*w));
        if let Some(c4) = c4
            && objectives.bomb_target_at(lo, hi).is_some()
            && state.is_none_or(|s| s.on_ground)
        {
            if inv.active != Some(c4) {
                inv.wanted = Some(c4);
            }
            intent.move_axis = Vec2::ZERO;
            intent.jump = false;
            intent.fire = inv.active == Some(c4);
            continue;
        }
        // Defuse: beside the planted bomb, looking at it.
        if *team == rules.defuser_team
            && let Some(at) = bomb
        {
            let feet = t.translation.with_y(lo.y);
            if (at - feet).xz().length() < DEFUSE_REACH {
                let eye = t.translation + state.map_or(Vec3::Y * 0.6, |s| s.eye_offset);
                let d = at + Vec3::Y * 0.05 - eye;
                intent.yaw = (-d.x).atan2(-d.z);
                intent.pitch = d.y.atan2(d.xz().length());
                intent.move_axis = Vec2::ZERO;
                intent.jump = false;
                intent.fire = false;
                // Another defender at it already: told so, no harm.
                intent.use_key = true;
                continue;
            }
        }
        intent.use_key = false;
    }
}
