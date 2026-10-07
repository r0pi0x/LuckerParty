//! Bots' path cost on the navigation mesh. CS:S's own bot cost isn't in
//! the SDK (specs/cs_source/nav.md open question 2), so this is ours, in
//! the style the spec shows for later Valve bots: the stock cost (centre
//! distance, ×20 more into crouch areas, ×5 more into jump areas), plus
//! AVOID areas, climbs and big drops, danger where teammates died, open
//! ground seen from many places, and per-team and per-bot noise so routes
//! vary between rounds and teammates spread over parallel areas instead
//! of walking in single file (docs/tech-debt.md).

use crate::map::nav::{NavMesh, Via, flags};

const UNIT: f64 = 0.0254;
/// Stock penalties (× distance), spec "Stock path cost".
const CROUCH_PENALTY: f64 = 20.0;
const JUMP_PENALTY: f64 = 5.0;
/// Ours: entering an AVOID area, climbing more than a step, dropping
/// more than CS:S's fatal drop (200 units).
const AVOID_PENALTY: f64 = 9.0;
const CLIMB_PENALTY: f64 = 1.0;
const DEATH_DROP: f64 = 200.0 * UNIT;
const DROP_PENALTY: f64 = 20.0;
const STEP_HEIGHT: f64 = 18.0 * UNIT;
/// Danger (deaths nearby, decaying) costs this × distance per unit,
/// capped at `DANGER_CAP`.
pub const DANGER_WEIGHT: f64 = 3.0;
const DANGER_CAP: f32 = 3.0;
/// Exposure (0..1) costs up to this × distance.
pub const EXPOSURE_WEIGHT: f64 = 0.6;
/// Route noise: the team's (changes per round) and the bot's own, up to
/// these × distance.
pub const TEAM_NOISE: f64 = 0.8;
pub const BOT_NOISE: f64 = 0.25;

/// What a bot's path cost depends on besides the mesh.
#[derive(Clone, Copy, Default)]
pub struct CostParams<'a> {
    /// Per area: recent deaths of the bot's team there (decaying).
    pub danger: Option<&'a [f32]>,
    /// Per area: how exposed it is, 0 (covered) to 1 (open).
    pub exposure: Option<&'a [f32]>,
    pub team_seed: u64,
    pub bot_seed: u64,
    /// Links (from area, to area) this bot got stuck on lately: costly
    /// for a while, so it repaths around them.
    pub stuck: &'a [(usize, usize)],
}

/// A link a bot got stuck on costs this much more (× distance, plus a
/// fixed part in meters so short links count too).
pub const STUCK_PENALTY: f64 = 30.0;
pub const STUCK_EXTRA: f64 = 40.0;

/// A stable pseudo-random u64 from a seed and a number (splitmix64).
pub fn hash64(seed: u64, n: usize) -> u64 {
    let mut x = seed ^ (n as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15);
    x ^= x >> 30;
    x = x.wrapping_mul(0xBF58_476D_1CE4_E5B9);
    x ^= x >> 27;
    x = x.wrapping_mul(0x94D0_49BB_1331_11EB);
    x ^ (x >> 31)
}

/// A stable pseudo-random 0..1 from a seed and an area.
pub fn hash01(seed: u64, area: usize) -> f64 {
    (hash64(seed, area) >> 11) as f64 / (1u64 << 53) as f64
}

/// Cost of entering `to` from `from` by `via` (never below the distance,
/// so the search's straight-line heuristic stays a lower bound).
pub fn step_cost(nav: &NavMesh, from: usize, to: usize, via: Via, p: &CostParams) -> f64 {
    let dist = nav.step_length(from, to, via);
    let (a, b) = (&nav.areas[from], &nav.areas[to]);
    let mut cost = dist;
    if b.has(flags::CROUCH) {
        cost += CROUCH_PENALTY * dist;
    }
    if b.has(flags::JUMP) {
        cost += JUMP_PENALTY * dist;
    }
    if b.has(flags::AVOID) {
        cost += AVOID_PENALTY * dist;
    }
    if let Via::Walk(_) = via {
        // Height change across the shared edge: each side's nearest point
        // to the other's centre.
        let rise = (b.closest_point(a.center).y - a.closest_point(b.center).y) as f64;
        if rise > STEP_HEIGHT {
            cost += CLIMB_PENALTY * dist;
        } else if -rise > DEATH_DROP {
            cost += DROP_PENALTY * dist;
        }
    }
    if let Some(d) = p.danger.and_then(|d| d.get(to)) {
        cost += DANGER_WEIGHT * d.min(DANGER_CAP) as f64 * dist;
    }
    if let Some(e) = p.exposure.and_then(|e| e.get(to)) {
        cost += EXPOSURE_WEIGHT * e.clamp(0.0, 1.0) as f64 * dist;
    }
    if p.stuck.contains(&(from, to)) {
        cost += STUCK_PENALTY * dist + STUCK_EXTRA;
    }
    cost += dist * (TEAM_NOISE * hash01(p.team_seed, to) + BOT_NOISE * hash01(p.bot_seed, to));
    cost
}

/// How exposed each area is, 0..1: with visibility data, the share of
/// the mesh it can see (scaled so the 90th percentile is 1); without it,
/// from its hiding spots (all exposed: 0.7, some in cover: 0, none: 0.35).
pub fn exposure(nav: &NavMesh) -> Vec<f32> {
    use crate::map::nav::spot;
    if nav.areas.iter().any(|a| !a.visible.is_empty()) {
        let counts: Vec<f32> = nav.areas.iter().map(|a| a.visible.len() as f32).collect();
        let mut sorted = counts.clone();
        sorted.sort_by(f32::total_cmp);
        let p90 = sorted[(sorted.len() * 9 / 10).min(sorted.len() - 1)].max(1.0);
        counts.iter().map(|c| (c / p90).min(1.0)).collect()
    } else {
        nav.areas
            .iter()
            .map(|a| {
                if a.hiding.is_empty() {
                    0.35
                } else if a.hiding.iter().any(|s| s.flags & spot::IN_COVER != 0) {
                    0.0
                } else {
                    0.7
                }
            })
            .collect()
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use bevy::prelude::*;

    use super::*;
    use crate::map::nav::{NavArea, Side};

    /// A flat area spanning x0..x1, z0..z1 at height y.
    pub fn area(id: u32, x0: f32, z0: f32, x1: f32, z1: f32, y: f32, flags: u32) -> NavArea {
        NavArea {
            id,
            flags,
            min: Vec2::new(x0, z0),
            max: Vec2::new(x1, z1),
            heights: [y; 4],
            center: Vec3::new((x0 + x1) / 2.0, y, (z0 + z1) / 2.0),
            links: Vec::new(),
            place: None,
            hiding: Vec::new(),
            visible: Vec::new(),
            encounters: Vec::new(),
        }
    }

    pub fn link(m: &mut NavMesh, a: usize, b: usize, side: Side) {
        m.areas[a].links.push((b, Via::Walk(side)));
        m.areas[b].links.push((a, Via::Walk(side.opposite())));
    }

    /// Start (0) and goal (3): a short way through area 1 (its flags
    /// given) and a detour twice as long through areas 4, 2 and 5:
    ///   z 0..4:  [0][1][3]   (x 0..12)
    ///   z 4..8:  [4][2][5]
    pub fn two_ways(short_flags: u32) -> NavMesh {
        let mut m = NavMesh {
            areas: vec![
                area(1, 0.0, 0.0, 4.0, 4.0, 0.0, 0),
                area(2, 4.0, 0.0, 8.0, 4.0, 0.0, short_flags),
                area(3, 4.0, 4.0, 8.0, 8.0, 0.0, 0),
                area(4, 8.0, 0.0, 12.0, 4.0, 0.0, 0),
                area(5, 0.0, 4.0, 4.0, 8.0, 0.0, 0),
                area(6, 8.0, 4.0, 12.0, 8.0, 0.0, 0),
            ],
            ..Default::default()
        };
        link(&mut m, 0, 1, Side::MaxX);
        link(&mut m, 1, 3, Side::MaxX);
        link(&mut m, 0, 4, Side::MaxZ);
        link(&mut m, 4, 2, Side::MaxX);
        link(&mut m, 2, 5, Side::MaxX);
        link(&mut m, 5, 3, Side::MinZ);
        m
    }

    fn path(m: &NavMesh, p: &CostParams) -> Vec<usize> {
        m.find_path_with(0, 3, |a, b, v| step_cost(m, a, b, v, p))
            .unwrap()
            .0
            .iter()
            .map(|x| x.0)
            .collect()
    }

    #[test]
    fn plain_ground_takes_the_short_way() {
        let m = two_ways(0);
        assert_eq!(path(&m, &CostParams::default()), [0, 1, 3]);
    }

    #[test]
    fn crouch_and_avoid_areas_are_walked_around() {
        for f in [flags::CROUCH, flags::AVOID, flags::JUMP] {
            let m = two_ways(f);
            assert_eq!(path(&m, &CostParams::default()), [0, 4, 2, 5, 3], "flag {f:#x}");
        }
    }

    #[test]
    fn danger_and_exposure_push_routes_aside() {
        let m = two_ways(0);
        let mut danger = vec![0.0; 6];
        danger[1] = 1.0;
        let p = CostParams {
            danger: Some(&danger),
            ..default()
        };
        assert_eq!(path(&m, &p), [0, 4, 2, 5, 3], "a teammate died there");
        let mut open = vec![0.0; 6];
        open[1] = 1.0;
        let p = CostParams {
            exposure: Some(&open),
            ..default()
        };
        // Exposure alone is milder: the detour is twice as long.
        assert_eq!(path(&m, &p), [0, 1, 3]);
    }

    #[test]
    fn noise_varies_routes_between_seeds_but_never_undercuts_distance() {
        // Two equally long ways, over area 1 or area 2.
        let mut m = NavMesh {
            areas: vec![
                area(1, 0.0, 0.0, 4.0, 8.0, 0.0, 0),
                area(2, 4.0, 0.0, 8.0, 4.0, 0.0, 0),
                area(3, 4.0, 4.0, 8.0, 8.0, 0.0, 0),
                area(4, 8.0, 0.0, 12.0, 8.0, 0.0, 0),
            ],
            ..Default::default()
        };
        link(&mut m, 0, 1, Side::MaxX);
        link(&mut m, 0, 2, Side::MaxX);
        link(&mut m, 1, 3, Side::MaxX);
        link(&mut m, 2, 3, Side::MaxX);
        let mut seen = std::collections::BTreeMap::new();
        for seed in 0..64u64 {
            let p = CostParams {
                team_seed: seed.wrapping_mul(0x1234_5678_9ABC),
                bot_seed: seed,
                ..default()
            };
            for a in 0..m.areas.len() {
                for &(b, via) in &m.areas[a].links {
                    assert!(step_cost(&m, a, b, via, &p) >= m.step_length(a, b, via));
                }
            }
            let way = m.find_path_with(0, 3, |a, b, v| step_cost(&m, a, b, v, &p)).unwrap().0[1].0;
            *seen.entry(way).or_insert(0) += 1;
        }
        assert!(seen.len() == 2 && seen.values().all(|n| *n > 16), "{seen:?}");
        // A detour twice as long is still rarely taken.
        let m = two_ways(0);
        let short = (0..64u64)
            .filter(|s| {
                let p = CostParams {
                    team_seed: s.wrapping_mul(0x1234_5678_9ABC),
                    bot_seed: *s,
                    ..default()
                };
                path(&m, &p) == [0, 1, 3]
            })
            .count();
        assert!(short >= 60, "{short} of 64");
    }

    #[test]
    fn a_link_it_got_stuck_on_is_walked_around() {
        let m = two_ways(0);
        assert_eq!(path(&m, &CostParams::default()), [0, 1, 3]);
        let stuck = [(1, 3)];
        let p = CostParams {
            stuck: &stuck,
            ..default()
        };
        assert_eq!(path(&m, &p), [0, 4, 2, 5, 3]);
        // Only that way round: the reverse link costs as before.
        let p0 = CostParams::default();
        let back = step_cost(&m, 3, 1, Via::Walk(Side::MinX), &p);
        assert_eq!(back, step_cost(&m, 3, 1, Via::Walk(Side::MinX), &p0));
    }

    #[test]
    fn hash_is_spread() {
        let mean: f64 = (0..1000).map(|i| hash01(7, i)).sum::<f64>() / 1000.0;
        assert!((mean - 0.5).abs() < 0.05, "{mean}");
    }

    #[test]
    fn climbs_cost_more() {
        let mut m = NavMesh {
            areas: vec![area(1, 0.0, 0.0, 4.0, 4.0, 0.0, 0), area(2, 4.0, 0.0, 8.0, 4.0, 1.0, 0)],
            ..Default::default()
        };
        link(&mut m, 0, 1, Side::MaxX);
        let p = CostParams::default();
        let up = step_cost(&m, 0, 1, Via::Walk(Side::MaxX), &p);
        let down = step_cost(&m, 1, 0, Via::Walk(Side::MinX), &p);
        assert!(up > down * 1.5, "up {up} down {down}");
    }
}
