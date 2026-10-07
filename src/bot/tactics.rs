//! Team play for bots. CS:S's bot code isn't public, so all of this is
//! our design (docs/tech-debt.md), built on the nav mesh's data
//! (specs/cs_source/nav.md: places, hiding spots, potential visibility).
//!
//! Each map yields a scenario and its sites: bomb sites (attackers are
//! team 1, the terrorists) or hostage groups (attackers are team 2, the
//! counter-terrorists, who come for them). For each site we find where
//! routes from either spawn come onto it (approaches) and rank hold spots
//! near it: hiding spots in cover that see an approach from a distance.
//!
//! Each round (and when bots join or leave) every team gets a plan: the
//! attackers pick one site and walk there as a group behind a leader, who
//! waits for stragglers; once on it they take hold spots facing the other
//! team's approaches. Defenders split over the sites and hold spots
//! watching the attackers' approaches. A teammate's "enemy spotted" or
//! "need backup" sends the nearest free bots to them. Deaths mark their
//! area as dangerous to that team for a while (path cost).

use avian3d::prelude::*;
use bevy::prelude::*;

use super::{Bot, path};
use crate::{
    character::CAPSULE_HEIGHT,
    core::{Died, Health, Intent, Radio, RoundRestarts, SpawnPoint, Team},
    map::{
        MapEntities,
        nav::{NavMesh, spot},
    },
};

/// Path distance from a site's point within which its hold spots lie, m.
pub const SITE_RADIUS: f32 = 18.0;
/// Approaches are where routes cross into this path distance of a site.
const ENTRY_RADIUS: f32 = 16.0;
const MAX_APPROACHES: usize = 3;
/// A detour route only counts as another approach up to this × the
/// shortest route's length.
const DETOUR_LIMIT: f64 = 2.5;
/// Hold spots kept per site and side.
const HOLDS_PER_SITE: usize = 8;
/// Attackers count as on the site within this of its point, m.
pub const ARRIVE_RADIUS: f32 = 7.0;
/// Hold spots this close to an approach stand in its way, m.
const MIN_WATCH: f32 = 5.0;
/// Approaches farther than this aren't watched from a spot, m.
const MAX_WATCH: f32 = 40.0;
/// Hold spots keep this far apart, m.
const HOLD_SPACING: f32 = 4.0;
/// The group leader waits while a member is farther than this, m...
const GROUP_SPREAD: f32 = 12.0;
/// ...for at most this long, then not again for `WAIT_COOLDOWN`, s.
const WAIT_MAX: f64 = 5.0;
const WAIT_COOLDOWN: f64 = 10.0;
/// The attackers gather when the leader has this much route left to the
/// site, until everyone is within `STAGE_GROUP` of it (at least
/// `STAGE_MIN` seconds, for grenades) or `STAGE_MAX` seconds passed, m
/// and s.
pub const STAGE_ROUTE: f32 = 24.0;
const STAGE_GROUP: f32 = 7.0;
const STAGE_MIN: f64 = 2.0;
const STAGE_MAX: f64 = 8.0;
/// An "enemy spotted" from a defender at its site moves the nearest
/// defender from another site there, this many per round.
const ROTATE_MAX: usize = 1;
/// Radio calls bring teammates within this, m.
const ASSIST_RANGE: f32 = 60.0;
/// A link a bot got stuck on is avoided by every bot for this long per
/// report, and for the rest of the map after `FAILED_FOREVER` reports, s.
const FAILED_MEMORY: f64 = 60.0;
const FAILED_FOREVER: u32 = 3;
/// Danger halves in this many seconds.
const DANGER_HALF_LIFE: f32 = 30.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Scenario {
    /// No objectives: everybody roams.
    #[default]
    None,
    Bomb,
    Hostage,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Role {
    #[default]
    Roam,
    Attack,
    Defend,
}

/// A place to hold, and the approaches (feet positions) to watch from it,
/// nearest first.
#[derive(Clone, Debug, PartialEq)]
pub struct Hold {
    pub spot: Vec3,
    pub watch: Vec<Vec3>,
}

#[derive(Clone, Debug)]
pub struct Site {
    /// The nav place it lies in (BombsiteA), else "A", "B", ...
    pub name: String,
    /// Feet position of its middle.
    pub point: Vec3,
    pub area: usize,
    /// Where routes from the attackers' spawn [0] and the defenders'
    /// spawn [1] come onto it (feet).
    pub approaches: [Vec<Vec3>; 2],
    /// Ranked hold spots for defenders [0] (watching the attackers'
    /// approaches) and for attackers who took it [1].
    pub holds: [Vec<Hold>; 2],
}

/// A team's plan for the round.
#[derive(Clone, Debug, Default)]
pub struct TeamPlan {
    pub team: Team,
    pub role: Role,
    /// The attackers' site.
    pub site: Option<usize>,
    pub seed: u64,
    /// Bots on the team, in a stable order.
    pub members: Vec<Entity>,
    pub leader: Option<Entity>,
    pub leader_feet: Option<Vec3>,
    /// The leader's route left to walk, m.
    pub leader_left: f32,
    /// The leader waits for the group.
    pub leader_wait: bool,
    /// The group gathers before going onto the site (and throws its
    /// grenades at it), since when; then it went in.
    pub staging: bool,
    stage_since: Option<f64>,
    pub staged: bool,
    /// Defenders sent from another site this round.
    rotated: usize,
    wait_since: Option<f64>,
    no_wait_until: f64,
    /// Per area: recent deaths of this team there (decaying).
    pub danger: Vec<f32>,
}

/// What the bots know about the map and their teams.
#[derive(Resource, Default)]
pub struct Tactics {
    key: (usize, usize, usize, usize),
    pub scenario: Scenario,
    pub sites: Vec<Site>,
    pub attackers: Team,
    pub defenders: Team,
    /// Points worth roaming to (sites, hostages, rescue zones).
    pub objectives: Vec<Vec3>,
    /// Per area, 0 (covered) to 1 (open): `path::exposure`.
    pub exposure: Vec<f32>,
    pub teams: Vec<TeamPlan>,
    round: Option<u32>,
    /// Nav links bots got stuck on (`Bot::stuck_links`), shared by all
    /// bots: (link, until, times reported, the last report's expiry).
    failed: Vec<((usize, usize), f64, u32, f64)>,
    /// The links in `failed` still in force (path cost `stuck`).
    pub failed_links: Vec<(usize, usize)>,
}

impl Tactics {
    pub fn team(&self, team: Team) -> Option<&TeamPlan> {
        self.teams.iter().find(|t| t.team == team)
    }

    pub fn site_name(&self, site: usize) -> &str {
        self.sites.get(site).map_or("?", |s| s.name.as_str())
    }
}

/// The map's scenario and its objective points: bomb sites, or else
/// hostages (grouped within 8 m), and every objective for roaming.
pub fn scenario(map: &MapEntities) -> (Scenario, Vec<Vec3>, Vec<Vec3>) {
    let all = super::objectives(map);
    let bombs: Vec<Vec3> = super::objectives_of(map, &["func_bomb_target", "info_bomb_target"]);
    if !bombs.is_empty() {
        return (Scenario::Bomb, bombs, all);
    }
    let hostages = super::objectives_of(map, &["hostage_entity"]);
    if hostages.is_empty() {
        return (Scenario::None, Vec::new(), all);
    }
    let mut groups: Vec<(Vec3, usize)> = Vec::new();
    for h in hostages {
        match groups.iter_mut().find(|g| (g.0 / g.1 as f32).distance(h) < 8.0) {
            Some(g) => {
                g.0 += h;
                g.1 += 1;
            }
            None => groups.push((h, 1)),
        }
    }
    let points = groups.iter().map(|g| g.0 / g.1 as f32).collect();
    (Scenario::Hostage, points, all)
}

/// Which site each of `n` defenders holds (by member order): as even a
/// split as possible, the sites with one more chosen and the members
/// shuffled by `seed`.
pub fn split(n: usize, sites: usize, seed: u64) -> Vec<usize> {
    if sites == 0 {
        return vec![0; n];
    }
    let rot = (seed % sites as u64) as usize;
    let mut out: Vec<usize> = (0..n).map(|i| (i + rot) % sites).collect();
    // Fisher-Yates with the seed.
    for i in (1..n).rev() {
        let j = (path::hash01(seed, i) * (i + 1) as f64) as usize % (i + 1);
        out.swap(i, j);
    }
    out
}

/// A place a bot could hold: position, `spot` flags and path distance
/// from the site.
#[derive(Clone, Copy, Debug)]
pub struct Candidate {
    pub pos: Vec3,
    pub flags: u8,
    pub site_dist: f32,
}

/// Rank up to `count` hold spots among `cands`: in cover, near the site,
/// seeing approaches from `MIN_WATCH`..`MAX_WATCH` (`sees(spot,
/// approach)`), not standing in one, spread `HOLD_SPACING` apart, each
/// next one preferring approaches nobody watches yet. A spot watches the
/// approaches it sees (nearest first), else all of them, else `fallback`.
pub fn choose_holds(
    cands: &[Candidate],
    approaches: &[Vec3],
    count: usize,
    fallback: Vec3,
    sees: &mut dyn FnMut(Vec3, Vec3) -> bool,
) -> Vec<Hold> {
    // What each candidate sees: (approach, distance).
    let seen: Vec<Vec<(usize, f32)>> = cands
        .iter()
        .map(|c| {
            approaches
                .iter()
                .enumerate()
                .filter_map(|(i, a)| {
                    let d = c.pos.distance(*a);
                    (d <= MAX_WATCH && sees(c.pos, *a)).then_some((i, d))
                })
                .collect()
        })
        .collect();
    let mut watched = vec![0usize; approaches.len()];
    let mut taken: Vec<usize> = Vec::new();
    let mut out = Vec::new();
    while out.len() < count {
        let mut best: Option<(usize, f32)> = None;
        for (i, c) in cands.iter().enumerate() {
            if taken.iter().any(|&t| cands[t].pos.distance(c.pos) < HOLD_SPACING) {
                continue;
            }
            let mut score = -c.site_dist / SITE_RADIUS;
            if c.flags & spot::IN_COVER != 0 {
                score += 2.0;
            }
            if c.flags & spot::EXPOSED != 0 {
                score -= 1.0;
            }
            if approaches.iter().any(|a| a.distance(c.pos) < MIN_WATCH) {
                score -= 3.0;
            }
            let useful: Vec<&(usize, f32)> = seen[i].iter().filter(|(_, d)| *d >= MIN_WATCH).collect();
            if useful.is_empty() && !approaches.is_empty() {
                score -= 1.5;
            }
            for (a, _) in &useful {
                score += if watched[*a] == 0 { 2.0 } else { 0.5 };
            }
            if best.is_none_or(|b| score > b.1) {
                best = Some((i, score));
            }
        }
        let Some((i, _)) = best else { break };
        taken.push(i);
        let mut watch: Vec<(usize, f32)> = seen[i].iter().copied().filter(|(_, d)| *d >= MIN_WATCH).collect();
        if watch.is_empty() {
            watch = approaches
                .iter()
                .enumerate()
                .map(|(a, p)| (a, p.distance(cands[i].pos)))
                .collect();
        }
        watch.sort_by(|a, b| a.1.total_cmp(&b.1));
        for (a, _) in &watch {
            watched[*a] += 1;
        }
        let mut points: Vec<Vec3> = watch.iter().map(|(a, _)| approaches[*a]).collect();
        if points.is_empty() {
            points.push(fallback);
        }
        out.push(Hold {
            spot: cands[i].pos,
            watch: points,
        });
    }
    out
}

/// Where routes from area `from` come onto the site: up to
/// `MAX_APPROACHES` distinct entry points (feet), each found by
/// searching again with the areas already used made costly.
pub fn approaches(nav: &NavMesh, from: usize, site_area: usize) -> Vec<Vec3> {
    let mut entry = vec![false; nav.areas.len()];
    for (a, _) in nav.within(site_area, ENTRY_RADIUS) {
        entry[a] = true;
    }
    let mut used = vec![false; nav.areas.len()];
    let mut out: Vec<Vec3> = Vec::new();
    let mut shortest: Option<f64> = None;
    for _ in 0..MAX_APPROACHES * 2 {
        let Some((areas, _)) = nav.find_path_with(from, site_area, |a, b, v| {
            let c = nav.step_cost(a, b, v);
            if used[b] && !entry[b] { c * 6.0 } else { c }
        }) else {
            break;
        };
        let length: f64 = areas
            .windows(2)
            .map(|w| nav.step_cost(w[0].0, w[1].0, w[1].1.unwrap()))
            .sum();
        let first = *shortest.get_or_insert(length);
        if length > first * DETOUR_LIMIT {
            break;
        }
        for (a, _) in &areas {
            used[*a] = true;
        }
        let Some(i) = areas.iter().position(|(a, _)| entry[*a]) else {
            continue;
        };
        let point = if i == 0 {
            nav.areas[areas[0].0].center
        } else {
            let (prev, next) = (areas[i - 1].0, areas[i].0);
            nav.areas[next].closest_point(nav.areas[prev].center)
        };
        if out.iter().all(|q| q.distance(point) > 6.0) {
            out.push(point);
            if out.len() == MAX_APPROACHES {
                break;
            }
        }
    }
    out
}

/// Hold candidates around a site: hiding spots in areas within
/// `SITE_RADIUS`, plus area middles when there are few spots.
fn candidates(nav: &NavMesh, site_area: usize) -> Vec<Candidate> {
    use crate::map::nav::flags;
    let near = nav.within(site_area, SITE_RADIUS);
    let mut out: Vec<Candidate> = near
        .iter()
        .flat_map(|&(a, d)| {
            nav.areas[a].hiding.iter().map(move |s| Candidate {
                pos: s.pos,
                flags: s.flags,
                site_dist: d,
            })
        })
        .collect();
    if out.len() < 6 {
        out.extend(near.iter().filter_map(|&(a, d)| {
            let area = &nav.areas[a];
            let size = area.max - area.min;
            (!area.has(flags::CROUCH) && size.min_element() >= 0.8).then(|| Candidate {
                pos: area.closest_point(area.center),
                flags: 0,
                site_dist: d,
            })
        }));
    }
    out
}

fn mean(points: &[Vec3]) -> Option<Vec3> {
    (!points.is_empty()).then(|| points.iter().sum::<Vec3>() / points.len() as f32)
}

/// Work out the map's sites, approaches and hold spots.
fn build_map(
    tactics: &mut Tactics,
    nav: &NavMesh,
    map: Option<&MapEntities>,
    spawns: &[(Vec3, Option<Team>)],
    sees: &mut dyn FnMut(Vec3, Vec3) -> bool,
) {
    let (scenario, points, objectives) = map.map(scenario).unwrap_or_default();
    tactics.scenario = scenario;
    tactics.objectives = objectives;
    (tactics.attackers, tactics.defenders) = match scenario {
        Scenario::Hostage => (Team(2), Team(1)),
        _ => (Team(1), Team(2)),
    };
    tactics.exposure = path::exposure(nav);
    tactics.sites.clear();
    if nav.areas.is_empty() {
        return;
    }
    let spawn_of = |team: Team| {
        let own: Vec<Vec3> = spawns.iter().filter(|s| s.1 == Some(team)).map(|s| s.0).collect();
        mean(&own).and_then(|p| nav.nearest_area(p))
    };
    let from = [spawn_of(tactics.attackers), spawn_of(tactics.defenders)];
    for (i, p) in points.iter().enumerate() {
        let Some(area) = nav.area_at(*p + Vec3::Y * 0.5).or_else(|| nav.nearest_area(*p)) else {
            continue;
        };
        let name = nav.areas[area]
            .place
            .and_then(|pl| nav.places.get(pl).cloned())
            .filter(|n| !tactics.sites.iter().any(|s| &s.name == n))
            .unwrap_or_else(|| ((b'A' + i as u8) as char).to_string());
        let point = nav.areas[area].closest_point(*p);
        let approaches = from.map(|f| f.map(|f| approaches(nav, f, area)).unwrap_or_default());
        let cands = candidates(nav, area);
        let holds = [0, 1].map(|side| choose_holds(&cands, &approaches[side], HOLDS_PER_SITE, point, sees));
        tactics.sites.push(Site {
            name,
            point,
            area,
            approaches,
            holds,
        });
    }
    info!(
        "bots: {:?} with {} sites ({})",
        scenario,
        tactics.sites.len(),
        tactics
            .sites
            .iter()
            .map(|s| format!(
                "{}: {}+{} approaches, {}+{} holds",
                s.name,
                s.approaches[0].len(),
                s.approaches[1].len(),
                s.holds[0].len(),
                s.holds[1].len()
            ))
            .collect::<Vec<_>>()
            .join("; ")
    );
}

/// A bot's standing orders for the round.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Orders {
    pub role: Role,
    pub site: Option<usize>,
    pub hold: Option<Hold>,
    /// Place in the team's member order.
    pub rank: usize,
}

/// Each team's plan and every bot's orders.
fn plan_round(tactics: &mut Tactics, round: u32, members: &[(Team, Entity)]) -> Vec<(Entity, Orders)> {
    let mut teams: Vec<Team> = members.iter().map(|m| m.0).collect();
    teams.sort_by_key(|t| t.0);
    teams.dedup();
    let areas = tactics.exposure.len();
    let mut out = Vec::new();
    let old = std::mem::take(&mut tactics.teams);
    for team in teams {
        let list: Vec<Entity> = members.iter().filter(|m| m.0 == team).map(|m| m.1).collect();
        let seed = path::hash64(0x5EED ^ (round as u64) << 8, team.0 as usize);
        let role = match tactics.scenario {
            _ if tactics.sites.is_empty() => Role::Roam,
            _ if team == tactics.attackers => Role::Attack,
            _ if team == tactics.defenders => Role::Defend,
            _ => Role::Roam,
        };
        let sites = tactics.sites.len();
        let site = (role == Role::Attack).then(|| (seed % sites as u64) as usize);
        let defend = split(list.len(), sites, seed);
        let mut per_site = vec![0usize; sites];
        // A few rounds start further down the ranking, for variety.
        let offset = (seed >> 8) as usize % 3;
        for (rank, &e) in list.iter().enumerate() {
            let orders = match role {
                Role::Attack => Orders {
                    role,
                    site,
                    hold: None,
                    rank,
                },
                Role::Defend => {
                    let s = defend[rank];
                    let holds = &tactics.sites[s].holds[0];
                    let k = per_site[s];
                    per_site[s] += 1;
                    let hold = (!holds.is_empty()).then(|| {
                        let start = if holds.len() > k + offset { offset } else { 0 };
                        holds[(start + k) % holds.len()].clone()
                    });
                    Orders {
                        role,
                        site: Some(s),
                        hold,
                        rank,
                    }
                }
                Role::Roam => Orders { rank, ..default() },
            };
            out.push((e, orders));
        }
        let danger = old
            .iter()
            .find(|t| t.team == team)
            .map(|t| t.danger.clone())
            .filter(|d| d.len() == areas)
            .unwrap_or_else(|| vec![0.0; areas]);
        tactics.teams.push(TeamPlan {
            team,
            role,
            site,
            seed,
            members: list,
            danger,
            ..default()
        });
    }
    out
}

/// The hold an attacker takes once on its site (by rank).
pub fn attacker_hold(site: &Site, rank: usize) -> Option<Hold> {
    let holds = &site.holds[1];
    (!holds.is_empty()).then(|| holds[rank % holds.len()].clone())
}

#[allow(clippy::too_many_arguments, clippy::type_complexity)]
pub(super) fn update(
    mut tactics: ResMut<Tactics>,
    nav: Option<Res<NavMesh>>,
    entities: Option<Res<MapEntities>>,
    spawn_points: Query<(&Transform, &SpawnPoint)>,
    mut bots: Query<(Entity, &mut Bot, &Transform, &Team, &Health)>,
    chars: Query<(&Transform, &Team), With<Intent>>,
    restarts: Option<Res<RoundRestarts>>,
    mut died: MessageReader<Died>,
    mut radio: MessageReader<Radio>,
    spatial: SpatialQuery,
    time: Res<Time>,
) {
    let now = time.elapsed_secs_f64();
    let dt = time.delta_secs();
    let empty = NavMesh::default();
    let nav = nav.as_deref().unwrap_or(&empty);
    // The map: sites, approaches, holds.
    let spawns: Vec<(Vec3, Option<Team>)> = spawn_points
        .iter()
        .map(|(t, s)| (t.translation - Vec3::Y * CAPSULE_HEIGHT / 2.0, s.team))
        .collect();
    let key = (
        nav.areas.as_ptr() as usize,
        nav.areas.len(),
        entities
            .as_ref()
            .map_or(0, |m| std::sync::Arc::as_ptr(&m.entities) as usize),
        spawns.len(),
    );
    if key != tactics.key {
        tactics.key = key;
        let filter = SpatialQueryFilter::default().with_mask(crate::core::SOLID_LAYERS);
        let not_character = |e: Entity| !chars.contains(e);
        let mut sees = |from: Vec3, to: Vec3| {
            if let (Some(a), Some(b)) = (nav.area_at(from + Vec3::Y * 0.3), nav.area_at(to + Vec3::Y * 0.3))
                && nav.sees(a, b) == Some(false)
            {
                return false;
            }
            let (a, b) = (from + Vec3::Y * 1.6, to + Vec3::Y * 1.2);
            let d = b - a;
            let Ok(dir) = Dir3::new(d) else { return true };
            spatial
                .cast_ray_predicate(a, dir, d.length(), true, &filter, &not_character)
                .is_none_or(|h| h.distance >= d.length() - 0.3)
        };
        build_map(&mut tactics, nav, entities.as_deref(), &spawns, &mut sees);
        tactics.round = None;
        tactics.failed.clear();
    }

    // Links bots keep getting stuck on: avoided by all of them.
    for (_, bot, ..) in &bots {
        for &(link, until) in &bot.stuck_links {
            match tactics.failed.iter_mut().find(|f| f.0 == link) {
                Some(f) if f.3 == until => {}
                Some(f) => {
                    f.2 += 1;
                    f.3 = until;
                    f.1 = if f.2 >= FAILED_FOREVER {
                        f64::INFINITY
                    } else {
                        now + FAILED_MEMORY * f.2 as f64
                    };
                    info!("bots: nav link {link:?} failed {} times", f.2);
                }
                None => tactics.failed.push((link, now + FAILED_MEMORY, 1, until)),
            }
        }
    }
    tactics.failed.retain(|f| f.1 > now);
    tactics.failed_links = tactics.failed.iter().map(|f| f.0).collect();

    // Deaths make their area dangerous for the victim's team.
    let decay = 0.5f32.powf(dt / DANGER_HALF_LIFE);
    for t in &mut tactics.teams {
        for d in &mut t.danger {
            *d *= decay;
        }
    }
    for d in died.read() {
        let Ok((at, team)) = chars.get(d.entity) else { continue };
        let Some(area) = nav.area_at(at.translation - Vec3::Y * CAPSULE_HEIGHT / 2.0) else {
            continue;
        };
        if let Some(plan) = tactics.teams.iter_mut().find(|t| t.team == *team)
            && plan.danger.len() == nav.areas.len()
        {
            plan.danger[area] += 1.0;
            for &(n, _) in &nav.areas[area].links {
                plan.danger[n] += 0.5;
            }
        }
    }

    // A new round, or bots joined or left: new plans and orders.
    let round = restarts.map_or(0, |r| r.0);
    let mut members: Vec<(Team, Entity)> = bots.iter().map(|(e, _, _, t, _)| (*t, e)).collect();
    members.sort_by_key(|m| (m.0.0, m.1));
    let known: Vec<(Team, Entity)> = tactics
        .teams
        .iter()
        .flat_map(|t| t.members.iter().map(move |e| (t.team, *e)))
        .collect();
    let new_round = tactics.round != Some(round);
    if new_round || known != members {
        let orders = plan_round(&mut tactics, round, &members);
        tactics.round = Some(round);
        for (e, o) in orders {
            if let Ok((_, mut bot, ..)) = bots.get_mut(e) {
                let fresh = new_round || bot.orders != o;
                bot.orders = o;
                if fresh {
                    bot.new_life();
                }
            }
        }
    }

    // Group leaders: the bomb carrier, else the first living attacker,
    // waiting for stragglers, and gathering everyone before the site.
    let alive: Vec<(Entity, Vec3, f32, bool, bool)> = bots
        .iter()
        .filter(|b| b.4.current > 0.0)
        .map(|(e, b, t, ..)| {
            (
                e,
                t.translation - Vec3::Y * CAPSULE_HEIGHT / 2.0,
                b.route_left(),
                b.hold.is_none() && b.site == b.orders.site,
                b.carrying,
            )
        })
        .collect();
    for plan in &mut tactics.teams {
        if plan.role != Role::Attack {
            continue;
        }
        let group: Vec<&(Entity, Vec3, f32, bool, bool)> =
            alive.iter().filter(|a| a.3 && plan.members.contains(&a.0)).collect();
        let leader = group.iter().find(|a| a.4).or(group.first()).copied();
        plan.leader = leader.map(|l| l.0);
        plan.leader_feet = leader.map(|l| l.1);
        plan.leader_left = leader.map_or(0.0, |l| l.2);
        let spread = leader.map_or(0.0, |l| group.iter().map(|g| g.1.distance(l.1)).fold(0.0, f32::max));
        // Gather before going on: once, when the leader nears the site.
        plan.staging = false;
        if let Some(l) = leader
            && !plan.staged
            && group.len() > 1
            && l.2 > 0.0
            && l.2 < STAGE_ROUTE
        {
            let since = *plan.stage_since.get_or_insert(now);
            let gathered = group.iter().all(|g| g.1.distance(l.1) < STAGE_GROUP);
            if (gathered && now - since >= STAGE_MIN) || now - since >= STAGE_MAX {
                plan.staged = true;
                // In together, no more waiting on the way.
                plan.no_wait_until = f64::INFINITY;
            } else {
                plan.staging = true;
            }
        }
        let wait = plan.staging || (spread > GROUP_SPREAD && now >= plan.no_wait_until);
        match (wait, plan.wait_since) {
            (true, None) => plan.wait_since = Some(now),
            (true, Some(since)) if now - since > WAIT_MAX => {
                plan.wait_since = None;
                plan.no_wait_until = now + WAIT_COOLDOWN;
            }
            (false, _) => plan.wait_since = None,
            _ => {}
        }
        plan.leader_wait = plan.wait_since.is_some();
    }

    // Teammates' calls bring the nearest free bots; a defender seeing
    // enemies at its site brings some from the other sites.
    let calls: Vec<Radio> = radio.read().cloned().collect();
    for call in &calls {
        if call.command != "enemyspot" {
            continue;
        }
        let Ok((_, caller, t, team, _)) = bots.get(call.sender) else {
            continue;
        };
        let (team, at) = (*team, t.translation);
        let Some(site) = caller.site.filter(|_| caller.orders.role == Role::Defend) else {
            continue;
        };
        let Some(point) = tactics.sites.get(site).map(|s| s.point) else {
            continue;
        };
        if at.distance(point) > SITE_RADIUS * 1.5 {
            continue;
        }
        let others: Vec<(Entity, f32)> = bots
            .iter()
            .filter(|(_, b, _, tm, h)| {
                **tm == team
                    && h.current > 0.0
                    && b.orders.role == Role::Defend
                    && b.site.is_some_and(|s| s != site)
                    && b.target.is_none()
            })
            .map(|(e, _, t, ..)| (e, t.translation.distance(point)))
            .collect();
        let Some(plan) = tactics.teams.iter_mut().find(|p| p.team == team) else {
            continue;
        };
        if plan.rotated >= ROTATE_MAX {
            continue;
        }
        let Some(&(e, _)) = others.iter().min_by(|a, b| a.1.total_cmp(&b.1)) else {
            continue;
        };
        plan.rotated += 1;
        // A hold at the site nobody has.
        let taken: Vec<Vec3> = bots.iter().filter_map(|(_, b, ..)| b.hold.as_ref().map(|h| h.spot)).collect();
        let hold = tactics.sites[site].holds[0]
            .iter()
            .find(|h| taken.iter().all(|t| t.distance(h.spot) > 1.0))
            .cloned();
        if let Ok((_, mut bot, ..)) = bots.get_mut(e) {
            debug!("bots: defender {e} rotates to site {site}");
            bot.site = Some(site);
            bot.hold = hold;
            bot.hold_since = None;
            bot.hunting = false;
        }
    }
    for call in &calls {
        let wanted = match call.command.as_str() {
            "enemyspot" => 1,
            "needbackup" | "takingfire" => 2,
            _ => continue,
        };
        let Ok((at, team)) = chars.get(call.sender) else {
            continue;
        };
        let from = at.translation - Vec3::Y * CAPSULE_HEIGHT / 2.0;
        let mut free: Vec<(Entity, f32)> = bots
            .iter()
            .filter(|(e, b, t, tm, h)| {
                *e != call.sender
                    && *tm == team
                    && h.current > 0.0
                    && b.target.is_none()
                    && b.assist.is_none()
                    && (8.0..ASSIST_RANGE).contains(&t.translation.distance(from))
            })
            .map(|(e, _, t, ..)| (e, t.translation.distance(from)))
            .collect();
        free.sort_by(|a, b| a.1.total_cmp(&b.1));
        for (e, _) in free.into_iter().take(wanted) {
            if let Ok((_, mut bot, ..)) = bots.get_mut(e) {
                bot.assist = Some((from, now));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bot::path::tests::{area, link};
    use crate::map::nav::{HidingSpot, Side};

    #[test]
    fn defenders_split_over_the_sites() {
        for seed in 0..50u64 {
            let s = split(5, 2, seed.wrapping_mul(0xABCDEF));
            let a = s.iter().filter(|x| **x == 0).count();
            assert!(a == 2 || a == 3, "{s:?}");
            let s = split(4, 2, seed);
            assert_eq!(s.iter().filter(|x| **x == 0).count(), 2, "{s:?}");
            let s = split(1, 2, seed);
            assert!(s[0] < 2);
        }
        // Over rounds, both sites get the odd one out, and members move.
        let firsts: std::collections::BTreeSet<Vec<usize>> = (0..20).map(|r| split(3, 2, r * 77)).collect();
        assert!(firsts.len() > 1);
    }

    #[test]
    fn roles_follow_the_scenario() {
        let mut t = Tactics {
            scenario: Scenario::Bomb,
            attackers: Team(1),
            defenders: Team(2),
            exposure: vec![0.0; 4],
            ..default()
        };
        let hold = |x: f32| Hold {
            spot: Vec3::new(x, 0.0, 0.0),
            watch: vec![Vec3::ZERO],
        };
        for name in ["A", "B"] {
            t.sites.push(Site {
                name: name.into(),
                point: Vec3::ZERO,
                area: 0,
                approaches: default(),
                holds: [vec![hold(1.0), hold(2.0), hold(3.0), hold(4.0)], vec![hold(9.0)]],
            });
        }
        let e = |i: u32| Entity::from_raw_u32(i + 1).unwrap();
        let members: Vec<(Team, Entity)> = (0..4)
            .map(|i| (Team(1), e(i)))
            .chain((4..9).map(|i| (Team(2), e(i))))
            .collect();
        let orders = plan_round(&mut t, 3, &members);
        let attack: Vec<&Orders> = orders
            .iter()
            .filter(|o| o.1.role == Role::Attack)
            .map(|o| &o.1)
            .collect();
        assert_eq!(attack.len(), 4);
        // One site for the whole group, no hold until they get there.
        assert!(attack.iter().all(|o| o.site == attack[0].site && o.hold.is_none()));
        let defend: Vec<&Orders> = orders
            .iter()
            .filter(|o| o.1.role == Role::Defend)
            .map(|o| &o.1)
            .collect();
        assert_eq!(defend.len(), 5);
        for s in 0..2 {
            let here: Vec<&&Orders> = defend.iter().filter(|o| o.site == Some(s)).collect();
            assert!(here.len() >= 2, "site {s}: {}", here.len());
            // Different spots at one site.
            let mut spots: Vec<f32> = here.iter().map(|o| o.hold.as_ref().unwrap().spot.x).collect();
            spots.dedup();
            assert_eq!(spots.len(), here.len());
        }
        assert_eq!(t.team(Team(1)).unwrap().role, Role::Attack);
        // Over rounds the attackers go to both sites.
        let sites: std::collections::BTreeSet<Option<usize>> =
            (0..12).map(|r| plan_round(&mut t, r, &members)[0].1.site).collect();
        assert_eq!(sites.len(), 2, "{sites:?}");
        // Hostage maps swap sides.
        t.scenario = Scenario::Hostage;
        (t.attackers, t.defenders) = (Team(2), Team(1));
        let orders = plan_round(&mut t, 3, &members);
        assert_eq!(orders[0].1.role, Role::Defend);
        assert_eq!(orders[8].1.role, Role::Attack);
    }

    #[test]
    fn holds_prefer_cover_that_sees_an_approach_from_a_distance() {
        let approaches = [Vec3::new(0.0, 0.0, -20.0), Vec3::new(20.0, 0.0, 0.0)];
        let c = |x: f32, z: f32, flags: u8| Candidate {
            pos: Vec3::new(x, 0.0, z),
            flags,
            site_dist: Vec2::new(x, z).length(),
        };
        let cands = [
            c(0.0, -18.0, spot::IN_COVER), // in the first doorway
            c(1.0, 1.0, spot::EXPOSED),    // open middle, sees both
            c(-3.0, 2.0, spot::IN_COVER),  // covered, sees the first only
            c(3.0, 4.0, spot::IN_COVER),   // covered, sees the second only
            c(-3.5, 2.5, spot::IN_COVER),  // next to the third
            c(-6.0, 6.0, spot::IN_COVER),  // covered, sees nothing
        ];
        // Walls: the third sees only x <= 0 approaches, the fourth only
        // x > 0, the sixth nothing.
        let mut sees = |s: Vec3, a: Vec3| {
            if s.x == -6.0 {
                false
            } else if s.x < -2.0 {
                a.x <= 0.0
            } else if s.x > 2.0 {
                a.x > 0.0
            } else {
                true
            }
        };
        let holds = choose_holds(&cands, &approaches, 3, Vec3::ZERO, &mut sees);
        let spots: Vec<Vec3> = holds.iter().map(|h| h.spot).collect();
        // Both covered watchers first (one each), never the doorway or
        // the spot beside a taken one.
        assert_eq!(spots[..2].len(), 2);
        assert!(
            spots[..2].contains(&cands[2].pos) && spots[..2].contains(&cands[3].pos),
            "{spots:?}"
        );
        assert!(
            !spots.contains(&cands[0].pos) && !spots.contains(&cands[4].pos),
            "{spots:?}"
        );
        let h = holds.iter().find(|h| h.spot == cands[2].pos).unwrap();
        assert_eq!(h.watch, [approaches[0]]);
        // The blind spot watches everything (nearest first) if it's ever
        // picked; with no approaches at all, the fallback.
        let holds = choose_holds(&cands[5..], &approaches, 1, Vec3::ZERO, &mut sees);
        assert_eq!(holds[0].watch.len(), 2);
        let holds = choose_holds(&cands[5..], &[], 1, Vec3::ONE, &mut sees);
        assert_eq!(holds[0].watch, [Vec3::ONE]);
    }

    #[test]
    fn approaches_find_both_ways_onto_a_site() {
        // A ring of areas: spawn at 0, the site at 4 on the far side, two
        // ways round of equal length, plus the site's own neighbours.
        //   z  0..4 : [0][1][2]
        //   z  4..8 : [3]   [5]
        //   z 8..12 : [6][4][7]
        let mut m = NavMesh {
            areas: vec![
                area(1, 0.0, 0.0, 4.0, 4.0, 0.0, 0),
                area(2, 4.0, 0.0, 8.0, 4.0, 0.0, 0),
                area(3, 8.0, 0.0, 12.0, 4.0, 0.0, 0),
                area(4, 0.0, 4.0, 4.0, 8.0, 0.0, 0),
                area(5, 4.0, 8.0, 8.0, 12.0, 0.0, 0),
                area(6, 8.0, 4.0, 12.0, 8.0, 0.0, 0),
                area(7, 0.0, 8.0, 4.0, 12.0, 0.0, 0),
                area(8, 8.0, 8.0, 12.0, 12.0, 0.0, 0),
            ],
            ..default()
        };
        // Stretch everything ×4 so the entry radius (16 m) covers only
        // the site's neighbours.
        for a in &mut m.areas {
            a.min *= 4.0;
            a.max *= 4.0;
            a.center *= 4.0;
        }
        link(&mut m, 0, 1, Side::MaxX);
        link(&mut m, 1, 2, Side::MaxX);
        link(&mut m, 0, 3, Side::MaxZ);
        link(&mut m, 3, 6, Side::MaxZ);
        link(&mut m, 6, 4, Side::MaxX);
        link(&mut m, 2, 5, Side::MaxZ);
        link(&mut m, 5, 7, Side::MaxZ);
        link(&mut m, 7, 4, Side::MinX);
        let pos = m.areas[4].center;
        m.areas[4].hiding.push(HidingSpot {
            pos,
            flags: spot::IN_COVER,
        });
        let found = approaches(&m, 0, 4);
        assert_eq!(found.len(), 2, "{found:?}");
        // One from each side of the site.
        assert!(
            found.iter().any(|p| p.x < 16.0) && found.iter().any(|p| p.x > 32.0),
            "{found:?}"
        );
        let cands = candidates(&m, 4);
        assert!(cands.iter().any(|c| c.flags == spot::IN_COVER));
    }
}
