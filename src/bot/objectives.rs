//! Bots and the round's objectives (`crate::objectives`), the simple
//! version: the terrorist carrying the bomb leads the attackers' group to
//! their site (`tactics`), and once near it walks to its bomb target and,
//! once inside with no enemy in sight, draws the bomb and holds fire until
//! it is planted; once it is, counter-terrorists gather near it (the
//! first ones there wait a few seconds for the rest), then walk to it and
//! hold +use on it while looking at it; terrorists hold spots around it. A loose
//! bomb draws the nearest terrorist to it. Leading hostages, buying kits
//! and anything smarter are left for the bots' own tactics
//! (docs/backlog.md).

use bevy::prelude::*;

use super::{Bot, Tactics};
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
/// The carrier leaves the group for the bomb target within this of the
/// team's site, m.
const PLANT_RADIUS: f32 = 15.0;
/// Defenders this close to a planted bomb wait for the others to come
/// within it too, for at most `RETAKE_WAIT` seconds after the plant, m.
const RETAKE_GATHER: f32 = 20.0;
const RETAKE_WAIT: f64 = 10.0;

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
    tactics: Res<Tactics>,
    time: Res<Time>,
) {
    let now = time.elapsed_secs_f64();
    let planted_bomb = planted.iter().find(|(b, _)| !b.defused);
    let bomb = planted_bomb.map(|(_, t)| t.translation);
    // Retake together: wait near the bomb while a defender is still far.
    let gathering = planted_bomb.is_some_and(|(b, t)| {
        now - b.planted_at < RETAKE_WAIT
            && bots.iter().any(|(_, bt, _, team, h, _)| {
                *team != rules.carrier_team
                    && h.current > 0.0
                    && bt.translation.distance(t.translation) > RETAKE_GATHER
            })
    });
    let lying = loose
        .iter()
        .find(|(l, _)| c4s.contains(l.weapon))
        .map(|(_, t)| t.translation);
    // The terrorist nearest a loose bomb goes for it.
    let fetcher = lying.and_then(|at| {
        bots.iter()
            .filter(|(_, _, _, team, h, _)| **team == rules.carrier_team && h.current > 0.0)
            .map(|(_, t, ..)| t.translation.distance(at))
            .min_by(f32::total_cmp)
    });
    // The site the attackers head for, and the target there.
    let site = tactics
        .team(rules.carrier_team)
        .and_then(|p| p.site)
        .and_then(|s| tactics.sites.get(s))
        .map(|s| s.point);
    let target_near = |p: Vec3| {
        objectives
            .bomb_targets
            .iter()
            .map(|z| z.floor_centre())
            .min_by(|a, b| a.distance(p).total_cmp(&b.distance(p)))
    };
    // After the plant, attackers hold the site the bomb is on.
    let bomb_site = bomb.and_then(|b| {
        (0..tactics.sites.len()).min_by(|x, y| {
            let d = |i: usize| tactics.sites[i].point.distance(b);
            d(*x).total_cmp(&d(*y))
        })
    });
    for (mut bot, t, state, team, health, inv) in &mut bots {
        if health.current <= 0.0 {
            bot.objective = None;
            bot.carrying = false;
            continue;
        }
        let feet = t.translation.with_y(hull(t, state).0.y);
        let carrying = inv.is_some_and(|i| i.weapons.iter().any(|w| c4s.contains(*w)));
        bot.carrying = carrying;
        let attacker = *team == rules.carrier_team;
        bot.objective = if let Some(b) = bomb {
            if attacker {
                if bomb_site.is_some() && bot.site != bomb_site {
                    bot.site = bomb_site;
                    bot.hold = None;
                    bot.hold_since = None;
                }
                None
            } else if gathering && feet.distance(b) < RETAKE_GATHER {
                Some(feet)
            } else {
                Some(b)
            }
        } else if carrying {
            // With the group until near the site, then to its target.
            match site {
                Some(p) if feet.distance(p) > PLANT_RADIUS => None,
                Some(p) => target_near(p),
                None => target_near(feet),
            }
        } else if attacker {
            lying.filter(|at| fetcher.is_some_and(|d| t.translation.distance(*at) <= d + 0.01))
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
