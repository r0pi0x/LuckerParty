//! Bots and the round's objectives (`crate::objectives`), the simple
//! version: the terrorist carrying the bomb leads the attackers' group to
//! their site (`tactics`), and once near it walks to its bomb target and,
//! once inside with no enemy in sight, draws the bomb and holds fire until
//! it is planted. After the plant (`Retake`) the defenders fall back to a
//! rally point, gather and go in together, one walking to the bomb and
//! holding +use on it while looking at it, the others covering it; the
//! terrorists guard the bomb from cover spots that see it. A loose bomb
//! draws the nearest terrorist to it. Leading hostages is left for later
//! (docs/backlog.md).

use avian3d::prelude::*;
use bevy::prelude::*;

use super::{Activity, Bot, Hold, Tactics};
use crate::{
    core::{Health, Intent, MovementState, Team},
    map::nav::NavMesh,
    objectives::{
        MapObjectives, RoundOpen,
        bomb::{BombRules, C4, PlantedBomb, hull},
    },
    weapon::{Inventory, drop::Loose, economy::DefuseKit},
};

/// Defusers start this close to the bomb, m.
const DEFUSE_REACH: f32 = 1.2;
/// The carrier leaves the group for the bomb target within this of the
/// team's site, m.
const PLANT_RADIUS: f32 = 15.0;

/// Defenders count as gathered at the rally point within this, m.
pub const GATHER_NEAR: f32 = 6.0;
/// A guard or cover spot counts as reached within this (flat), m.
pub const GUARD_NEAR: f32 = 1.5;
/// The retake goes in at the latest this long after the plant, s...
pub const RETAKE_WAIT_MAX: f64 = 25.0;
/// ...or once the bomb's time left is down to the walk in (at
/// `RETAKE_SPEED` over `RETAKE_DETOUR` × the straight distance), the
/// defuse and this margin, s and m/s.
pub const RETAKE_MARGIN: f32 = 4.0;
pub const RETAKE_SPEED: f32 = 6.0;
pub const RETAKE_DETOUR: f32 = 1.4;
/// Gathered, it still waits this long for the last ones to settle, s.
pub const RETAKE_SETTLE: f64 = 1.0;
/// A defender this close to the bomb with no enemy seen or heard for
/// `ON_IT_QUIET` seconds defuses at once rather than falling back, m.
pub const ON_IT: f32 = 4.0;
pub const ON_IT_QUIET: f64 = 3.0;
/// Guard and cover spots: hiding spots that see the bomb, at least this
/// far from it, m.
pub const GUARD_MIN: f32 = 3.0;
/// The defuser is the nearest defender, one with a kit counted this much
/// nearer, m.
pub const KIT_LEAD: f32 = 10.0;
/// The rally point lies this far along the way (by the mesh) from the
/// bomb toward the defenders, m.
pub const RALLY_BACK: f32 = 15.0;
/// Defenders farther than this from the bomb when it is planted choose
/// which way that is (all of them, if none is), m.
pub const RALLY_AWAY: f32 = 20.0;

/// What the bots make of a planted bomb (our design; CS:S's bot code isn't
/// public): the defenders fall back to a rally point outside the site
/// (where their own routes come onto it, nearest to them), gather there,
/// and go in together once gathered, when the clock says so, or after
/// `RETAKE_WAIT_MAX`; then one of them (with a kit, nearest) defuses and
/// the others take cover spots around the bomb watching the attackers'
/// ways in and hiding places. The planting team guards the bomb from
/// cover spots that see it.
#[derive(Resource, Clone, Debug, Default)]
pub struct Retake {
    /// The plant this is for (its time), and the site it is on.
    pub planted_at: Option<f64>,
    pub site: Option<usize>,
    pub rally: Vec3,
    /// Since when the defenders gather, and whether they went in (when).
    pub since: f64,
    pub went: Option<f64>,
    /// Hold spots around the bomb: the planting team's (watching the
    /// defenders' ways in and the bomb), the defenders' (watching the
    /// attackers' ways in and the guard spots).
    pub guards: Vec<Hold>,
    pub covers: Vec<Hold>,
    /// The defender who defuses.
    pub defuser: Option<Entity>,
}

/// Where each bot's objective is.
#[allow(clippy::type_complexity, clippy::too_many_arguments)]
pub(super) fn goals(
    mut bots: Query<(
        Entity,
        &mut Bot,
        &Transform,
        Option<&MovementState>,
        &Team,
        &Health,
        Option<&Inventory>,
        Has<DefuseKit>,
    )>,
    chars: Query<(), With<Intent>>,
    c4s: Query<(), With<C4>>,
    planted: Query<(&PlantedBomb, &Transform)>,
    loose: Query<(&Loose, &Transform)>,
    objectives: Res<MapObjectives>,
    rules: Res<BombRules>,
    tactics: Res<Tactics>,
    mut retake: ResMut<Retake>,
    nav: Option<Res<NavMesh>>,
    spatial: SpatialQuery,
    time: Res<Time>,
) {
    let now = time.elapsed_secs_f64();
    let planted_bomb = planted.iter().find(|(b, _)| !b.defused);
    let bomb = planted_bomb.map(|(_, t)| t.translation);
    let lying = loose
        .iter()
        .find(|(l, _)| c4s.contains(l.weapon))
        .map(|(_, t)| t.translation);
    // The terrorist nearest a loose bomb goes for it.
    let fetcher = lying.and_then(|at| {
        bots.iter()
            .filter(|b| *b.4 == rules.carrier_team && b.5.current > 0.0)
            .map(|b| b.2.translation.distance(at))
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
    let feet_of = |t: &Transform, state: Option<&MovementState>| t.translation.with_y(hull(t, state).0.y);
    match planted_bomb {
        Some((b, bt)) if retake.planted_at != Some(b.planted_at) => {
            let defenders: Vec<Vec3> = bots
                .iter()
                .filter(|x| *x.4 == rules.defuser_team && x.5.current > 0.0)
                .map(|x| feet_of(x.2, x.3))
                .collect();
            // Those away from the bomb set the side to fall back to (the
            // ones on the site fight there or die).
            let away: Vec<Vec3> = defenders
                .iter()
                .copied()
                .filter(|d| d.distance(bt.translation) > RALLY_AWAY)
                .collect();
            let defenders = if away.is_empty() { defenders } else { away };
            let empty = NavMesh::default();
            let nav = nav.as_deref().unwrap_or(&empty);
            let filter = SpatialQueryFilter::default().with_mask(crate::core::SOLID_LAYERS);
            let not_character = |e: Entity| !chars.contains(e);
            let mut sees = |from: Vec3, to: Vec3| {
                let (a, b) = (from + Vec3::Y * 1.6, to + Vec3::Y * 1.0);
                let d = b - a;
                let Ok(dir) = Dir3::new(d) else { return true };
                spatial
                    .cast_ray_predicate(a, dir, d.length(), true, &filter, &not_character)
                    .is_none_or(|h| h.distance >= d.length() - 0.3)
            };
            *retake = plan_retake(
                nav,
                &tactics,
                bomb_site,
                bt.translation,
                &defenders,
                b.planted_at,
                now,
                &mut sees,
            );
            // The defender nearest the rally point calls the others
            // together (`radio`: once a round per team).
            let rally = retake.rally;
            if let Some((_, mut bot, ..)) = bots
                .iter_mut()
                .filter(|x| *x.4 == rules.defuser_team && x.5.current > 0.0)
                .min_by(|x, y| x.2.translation.distance(rally).total_cmp(&y.2.translation.distance(rally)))
            {
                bot.radio.hold("regroup", now);
            }
        }
        Some(_) => {}
        None => *retake = Retake::default(),
    }
    // The retake: gathered, the clock, or waited long enough.
    if let Some((b, bt)) = planted_bomb
        && retake.went.is_none()
    {
        let at = bt.translation;
        let waiting: Vec<f32> = bots
            .iter()
            .filter(|x| *x.4 == rules.defuser_team && x.5.current > 0.0)
            .map(|x| (feet_of(x.2, x.3) - retake.rally).length())
            .collect();
        let kit = bots
            .iter()
            .any(|x| *x.4 == rules.defuser_team && x.5.current > 0.0 && x.7);
        let defuse = if kit { rules.defuse_time_kit } else { rules.defuse_time };
        let need = retake.rally.distance(at) * RETAKE_DETOUR / RETAKE_SPEED + defuse + RETAKE_MARGIN;
        let left = (b.explode_at - now) as f32;
        let gathered = waiting.iter().all(|d| *d < GATHER_NEAR) && now - retake.since >= RETAKE_SETTLE;
        // One standing at it with nobody about: it defuses at once, the
        // others cover it.
        let on_it = bots
            .iter()
            .filter(|x| {
                *x.4 == rules.defuser_team
                    && x.5.current > 0.0
                    && feet_of(x.2, x.3).distance(at) < ON_IT
                    && now - x.1.contact > ON_IT_QUIET
            })
            .min_by(|x, y| x.2.translation.distance(at).total_cmp(&y.2.translation.distance(at)))
            .map(|x| x.0);
        if on_it.is_some() {
            retake.defuser = on_it;
        }
        if on_it.is_some() || gathered || waiting.len() <= 1 || left <= need || now - retake.since >= RETAKE_WAIT_MAX {
            debug!(
                "bots: retake goes in ({} defenders, {left:.1} s left, gathered {gathered})",
                waiting.len()
            );
            retake.went = Some(now);
        }
    }
    // Who defuses: one with a kit, else the nearest (kept while alive).
    if let Some(at) = bomb
        && retake.went.is_some()
    {
        let alive = |e: Entity| bots.get(e).is_ok_and(|x| x.5.current > 0.0);
        if retake.defuser.is_none_or(|d| !alive(d)) {
            retake.defuser = bots
                .iter()
                .filter(|x| *x.4 == rules.defuser_team && x.5.current > 0.0)
                .map(|x| (x.0, x.2.translation.distance(at) - if x.7 { KIT_LEAD } else { 0.0 }))
                .min_by(|x, y| x.1.total_cmp(&y.1))
                .map(|x| x.0);
        }
    }
    // Rank in each side (bot number order, the dead too, so spots stay
    // put as bots die), for spots.
    let mut ranks: Vec<(Team, u32, Entity)> = bots.iter().map(|x| (*x.4, x.1.number, x.0)).collect();
    ranks.sort_by_key(|r| (r.0.0, r.1));
    let rank = |e: Entity, team: Team| {
        ranks
            .iter()
            .filter(|r| r.0 == team && Some(r.2) != retake.defuser)
            .position(|r| r.2 == e)
            .unwrap_or(0)
    };
    for (e, mut bot, t, state, team, health, inv, _) in &mut bots {
        if health.current <= 0.0 {
            bot.objective = None;
            bot.carrying = false;
            bot.guard = None;
            continue;
        }
        let feet = feet_of(t, state);
        let carrying = inv.is_some_and(|i| i.weapons.iter().any(|w| c4s.contains(*w)));
        bot.carrying = carrying;
        let attacker = *team == rules.carrier_team;
        bot.guard = None;
        bot.objective = if let Some(b) = bomb {
            if attacker {
                if retake.guards.is_empty() {
                    if bomb_site.is_some() && bot.site != bomb_site {
                        bot.site = bomb_site;
                        bot.hold = None;
                        bot.hold_since = None;
                    }
                } else {
                    let g = &retake.guards[(rank(e, *team) + bot.guard_skip) % retake.guards.len()];
                    bot.guard = Some((g.clone(), Activity::Covering));
                }
                None
            } else if retake.went.is_none() {
                // Gathering: watching toward the bomb and the other ways in.
                let mut watch = vec![b];
                if let Some(s) = retake.site.and_then(|s| tactics.sites.get(s)) {
                    watch.extend(s.approaches[1].iter().copied().filter(|a| a.distance(retake.rally) > 3.0));
                }
                let spot = Hold {
                    spot: retake.rally,
                    watch,
                };
                bot.guard = Some((spot, Activity::Retaking));
                None
            } else if retake.defuser == Some(e) || retake.covers.is_empty() {
                Some(b)
            } else {
                let c = &retake.covers[(rank(e, *team) + bot.guard_skip) % retake.covers.len()];
                bot.guard = Some((c.clone(), Activity::Covering));
                None
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

/// The retake for a bomb planted at `bomb` (on `site`, if known) with the
/// defenders at `defenders` (feet): the rally point, the guard and cover
/// spots (`sees(from, to)`: a clear line, feet to feet).
#[allow(clippy::too_many_arguments)]
pub fn plan_retake(
    nav: &NavMesh,
    tactics: &Tactics,
    site: Option<usize>,
    bomb: Vec3,
    defenders: &[Vec3],
    planted_at: f64,
    now: f64,
    sees: &mut dyn FnMut(Vec3, Vec3) -> bool,
) -> Retake {
    let s = site.and_then(|i| tactics.sites.get(i));
    let mean = (!defenders.is_empty()).then(|| defenders.iter().sum::<Vec3>() / defenders.len() as f32);
    // Back along the way from the bomb toward the defenders (by the
    // mesh): outside the site, on their side of it.
    let back = |from: Vec3| -> Option<Vec3> {
        let (a, b) = (nav.nearest_area(bomb)?, nav.nearest_area(from)?);
        let (areas, _) = nav.find_path(a, b)?;
        let mut walked = 0.0;
        let mut at = bomb;
        for (k, _) in areas.iter().skip(1) {
            let next = nav.areas[*k].center;
            walked += at.distance(next);
            at = next;
            if walked >= RALLY_BACK {
                break;
            }
        }
        Some(at)
    };
    let rally = mean
        .and_then(back)
        .or_else(|| {
            // Else where the defenders' own routes come onto the site.
            s.and_then(|s| s.approaches[1].iter().copied().max_by(|a, b| a.distance(bomb).total_cmp(&b.distance(bomb))))
        })
        .unwrap_or(bomb);
    let mut guards = Vec::new();
    let mut covers = Vec::new();
    if let Some(area) = nav.area_at(bomb + Vec3::Y * 0.3).or_else(|| nav.nearest_area(bomb)) {
        let all = super::tactics::candidates(nav, area);
        let seeing: Vec<super::tactics::Candidate> = all
            .iter()
            .copied()
            .filter(|c| c.pos.distance(bomb) >= GUARD_MIN && sees(c.pos, bomb))
            .collect();
        let cands = if seeing.len() >= 2 { seeing } else { all };
        let mut watch: Vec<Vec3> = s.map(|s| s.approaches[1].clone()).unwrap_or_default();
        watch.push(bomb);
        guards = super::tactics::choose_holds(&cands, &watch, 5, bomb, sees);
        let mut watch: Vec<Vec3> = s.map(|s| s.approaches[0].clone()).unwrap_or_default();
        watch.extend(guards.iter().take(3).map(|g| g.spot));
        // Covers on the near side of the bomb, coming from the rally point
        // (not past the one defusing).
        let near: Vec<super::tactics::Candidate> = cands
            .iter()
            .copied()
            .filter(|c| c.pos.distance(rally) <= bomb.distance(rally) + 1.0)
            .collect();
        let cands = if near.len() >= 2 { near } else { cands };
        covers = super::tactics::choose_holds(&cands, &watch, 5, bomb, sees);
    }
    Retake {
        planted_at: Some(planted_at),
        site,
        rally,
        since: now,
        went: None,
        guards,
        covers,
        defuser: None,
    }
}

/// Planting and defusing when there: overrides what `think` decided.
#[allow(clippy::type_complexity)]
pub(super) fn act(
    mut bots: Query<(
        &mut Bot,
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
    for (mut bot, mut intent, t, state, team, health, inv) in &mut bots {
        bot.radio.at_objective = false;
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
            bot.radio.at_objective = true;
            continue;
        }
        // Defuse: beside the planted bomb, looking at it (not one covering
        // or falling back that passes it).
        if *team == rules.defuser_team
            && bot.guard.is_none()
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
                bot.radio.at_objective = true;
                continue;
            }
        }
        intent.use_key = false;
    }
}
