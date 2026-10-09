//! Bots playing rounds on real maps (`bot::tactics`), headless. Skipped
//! without a CS:S install.
//!
//! `dust2_bot_round_stats` (ignored) plays whole rounds bot against bot
//! and prints winners, kills, time to first contact and where bots got
//! stuck (no progress for 4 s while walking a route, summed up by spot;
//! `tests/it/heavy/bot_nav.rs` has the tools to look at one):
//! `MASHUP_BOT_MAP=de_nuke MASHUP_BOT_ROUNDS=8 cargo test --features dev
//! --test it bot_rounds:: -- --ignored --nocapture`.

use bevy::prelude::*;
use mashup::{
    bot::{Activity, Bot, BotConfig, Role, Tactics, tactics::Scenario},
    console::Console,
    core::{Health, Team},
    games::{
        self,
        cs_source::{
            self, TICK_INTERVAL,
            movement::{self, SourceMovementPlugin},
            weapons::CsWeaponsPlugin,
        },
    },
    harness::Sim,
    map::MapPlugin,
    mount::config::LocalConfig,
    rules::{
        Score,
        rounds::{Phase, RoundState},
    },
};

fn installed() -> bool {
    let ok = LocalConfig::load()
        .ok()
        .and_then(|c| c.game_path(cs_source::GAME))
        .is_some_and(|p| p.join("cstrike").is_dir());
    if !ok {
        eprintln!("skipping: no CS:S install configured");
    }
    ok
}

/// A map with `t` terrorist and `ct` counter-terrorist bots.
fn sim(map: &str, t: usize, ct: usize) -> (Sim, Vec<Entity>) {
    let map = games::load_map(&format!("cs_source:{map}")).expect("load map");
    let mut sim = Sim::new((MapPlugin::new(map), SourceMovementPlugin, CsWeaponsPlugin));
    sim.set_tick_interval(TICK_INTERVAL);
    sim.app
        .insert_resource(mashup::slots::Loadout { movement: movement::ID });
    let mut bots = Vec::new();
    for _ in 0..t {
        bots.push(mashup::bot::add_bot(sim.app.world_mut(), Team(1)).expect("bot"));
    }
    for _ in 0..ct {
        bots.push(mashup::bot::add_bot(sim.app.world_mut(), Team(2)).expect("bot"));
    }
    sim.ticks(2);
    (sim, bots)
}

fn feet(sim: &Sim, e: Entity) -> Vec3 {
    sim.position(e) - Vec3::Y * 0.9
}

/// One line per bot: team, activity, place, position (`MASHUP_BOT_TRACE`).
fn trace_bots(sim: &Sim, bots: &[Entity], time: f64) {
    let nav = sim.app.world().get_resource::<mashup::map::nav::NavMesh>();
    for &b in bots {
        let w = sim.app.world();
        let (Some(bot), Some(team)) = (w.get::<Bot>(b), w.get::<Team>(b)) else {
            continue;
        };
        let at = feet(sim, b);
        let place = nav
            .and_then(|n| {
                n.area_at(at + Vec3::Y * 0.1)
                    .and_then(|a| n.areas[a].place)
                    .map(|p| n.places[p].clone())
            })
            .unwrap_or_default();
        eprintln!(
            "{time:6.2} {b} t{} {:?} {:?} {place} {at:.1} v {:.1} left {:.1} hp {:.2} next {:.1?} look {:.1?} yaw {:.2} axis {:.2}",
            team.0,
            bot.role(),
            bot.activity(),
            sim.velocity(b).xz().length(),
            bot.route_left(),
            w.get::<Health>(b).map_or(0.0, |h| h.current),
            bot.route().0.get(bot.route().1),
            bot.look_target(),
            w.get::<mashup::core::Intent>(b).map_or(0.0, |i| i.yaw),
            w.get::<mashup::core::Intent>(b).map_or(Vec2::ZERO, |i| i.move_axis),
        );
    }
}

/// dust2: the terrorists pick a bomb site and get there as a group; the
/// counter-terrorists split over both sites and hold spots there.
#[test]
fn dust2_attackers_take_a_site_and_defenders_hold_both() {
    if !installed() {
        return;
    }
    // Each side on its own (they would fight otherwise).
    let (mut sim, bots) = sim("de_dust2", 4, 0);
    sim.app.world_mut().resource_mut::<BotConfig>().grenades = 0;
    let ts = &bots[..];
    let (site, points) = {
        let t = sim.app.world().resource::<Tactics>();
        assert_eq!(t.scenario, Scenario::Bomb);
        assert_eq!(t.sites.len(), 2);
        let mut names: Vec<&str> = t.sites.iter().map(|s| s.name.as_str()).collect();
        names.sort();
        assert_eq!(names, ["BombsiteA", "BombsiteB"]);
        for s in &t.sites {
            assert!(!s.approaches[0].is_empty() && !s.approaches[1].is_empty(), "{}", s.name);
            assert!(s.holds[0].len() >= 3, "{}: {} holds", s.name, s.holds[0].len());
        }
        let plan = t.team(Team(1)).unwrap();
        assert_eq!(plan.role, Role::Attack);
        let site = plan.site.unwrap();
        (site, t.sites.iter().map(|s| s.point).collect::<Vec<_>>())
    };
    for &b in ts {
        let bot = sim.app.world().get::<Bot>(b).unwrap();
        assert_eq!((bot.role(), bot.site()), (Role::Attack, Some(site)));
    }
    let trace = std::env::var("MASHUP_BOT_TRACE").is_ok();
    let mut arrived = vec![None; ts.len()];
    let mut spread_max = 0.0f32;
    let step = 0.25;
    let mut time = 0.0;
    while time < 45.0 {
        sim.seconds(step);
        time += step;
        for (i, &b) in ts.iter().enumerate() {
            if arrived[i].is_none() && feet(&sim, b).distance(points[site]) < mashup::bot::tactics::ARRIVE_RADIUS + 3.0
            {
                arrived[i] = Some(time);
            }
        }
        if trace && (time < 10.0 || (time * 4.0) as u32 % 8 == 0) {
            trace_bots(&sim, &bots, time);
        }
        // A group: nobody runs off far ahead (while on the way).
        if arrived.iter().all(|a| a.is_none()) && time > 3.0 {
            let p: Vec<Vec3> = ts.iter().map(|b| feet(&sim, *b)).collect();
            let spread = p
                .iter()
                .flat_map(|a| p.iter().map(move |b| a.distance(*b)))
                .fold(0.0, f32::max);
            spread_max = spread_max.max(spread);
        }
    }
    eprintln!("attackers at site {site}: arrived {arrived:?}, largest spread on the way {spread_max:.1} m");
    let n = arrived.iter().filter(|a| a.is_some()).count();
    assert!(n >= 3, "only {n} of 4 attackers reached the site: {arrived:?}");
    assert!(spread_max < 40.0, "the group spread {spread_max:.1} m");

    // Defenders: near a site each, both sites held, mostly at their spots.
    let (mut sim, cts) = self::sim("de_dust2", 0, 4);
    sim.app.world_mut().resource_mut::<BotConfig>().grenades = 0;
    sim.seconds(25.0);
    let mut per_site = [0; 2];
    let mut holding = 0;
    for &b in &cts {
        let at = feet(&sim, b);
        let bot = sim.app.world().get::<Bot>(b).unwrap();
        let s = bot.site().expect("a defender's site");
        let d = at.distance(points[s]);
        eprintln!("defender {b}: site {s}, {d:.1} m away, {:?}", bot.activity());
        assert!(
            d < mashup::bot::tactics::SITE_RADIUS + 4.0,
            "defender {b} {d:.1} m from its site"
        );
        per_site[s] += 1;
        // Two defenders shouldering each other short of neighbouring
        // spots hold where they are (`HOLD_SHARE`), so this holds
        // whatever the entity ids (route noise seeds) are.
        if bot.activity() == Activity::Holding || bot.target.is_some() {
            holding += 1;
        }
    }
    assert!(per_site.iter().all(|n| *n >= 1), "sites held: {per_site:?}");
    assert!(holding >= 3, "{holding} of 4 defenders holding");
}

/// Playtest: bots walked off in the freeze time while the player was
/// held. Nobody moves until it ends; then they set off.
#[test]
fn dust2_bots_hold_still_in_the_freeze_time() {
    if !installed() {
        return;
    }
    let (mut sim, bots) = sim("de_dust2", 2, 2);
    sim.app.world_mut().resource_mut::<BotConfig>().grenades = 0;
    sim.app
        .world_mut()
        .resource_mut::<Console>()
        .submit("mp_freezetime 3; mp_roundtime 2; mashup_rounds 1");
    sim.ticks(3);
    assert!(matches!(
        sim.app.world().resource::<RoundState>().phase,
        Phase::Freeze { .. }
    ));
    // Settled onto the floor at the spawns.
    sim.seconds(0.5);
    let start: Vec<Vec3> = bots.iter().map(|&b| feet(&sim, b)).collect();
    sim.seconds(2.0);
    for (&b, at) in bots.iter().zip(&start) {
        let moved = (feet(&sim, b) - *at).with_y(0.0).length();
        assert!(moved < 0.01, "{b} moved {moved} m in the freeze time");
    }
    sim.seconds(3.0);
    assert!(matches!(sim.app.world().resource::<RoundState>().phase, Phase::Live { .. }));
    let walked = bots
        .iter()
        .zip(&start)
        .filter(|(b, at)| (feet(&sim, **b) - **at).with_y(0.0).length() > 1.0)
        .count();
    assert!(walked >= 3, "{walked} of 4 bots set off after the freeze");
}

/// Whole rounds, bots against bots; prints what happened.
#[test]
#[ignore]
fn dust2_bot_round_stats() {
    if !installed() {
        return;
    }
    let map = std::env::var("MASHUP_BOT_MAP").unwrap_or_else(|_| "de_dust2".into());
    let rounds: u32 = std::env::var("MASHUP_BOT_ROUNDS")
        .ok()
        .and_then(|r| r.parse().ok())
        .unwrap_or(6);
    let per_team: usize = std::env::var("MASHUP_BOT_TEAM")
        .ok()
        .and_then(|r| r.parse().ok())
        .unwrap_or(5);
    let (mut sim, bots) = sim(&map, per_team, per_team);
    sim.app
        .world_mut()
        .resource_mut::<Console>()
        .submit("mp_freezetime 2; mp_roundtime 2; mashup_rounds 1");
    sim.ticks(3);
    let mut stats: Vec<String> = Vec::new();
    let (mut contact, mut live_at, mut last_kills) = (None::<f64>, None::<f64>, 0i32);
    let mut wins = [0u32; 3];
    let mut contacts = Vec::new();
    let mut lengths = Vec::new();
    let mut kills_total = 0;
    let mut played = 0;
    let mut sites: Vec<String> = Vec::new();
    let step = 0.1;
    let trace = std::env::var("MASHUP_BOT_TRACE").is_ok();
    let mut t = 0.0f64;
    let limit = rounds as f64 * 150.0;
    let mut stuck = StuckWatch::default();
    while played < rounds && t < limit {
        sim.seconds(step);
        t += step;
        if matches!(sim.app.world().resource::<RoundState>().phase, Phase::Live { .. }) {
            stuck.check(&sim, &bots, t);
        }
        if trace && (t * 10.0).round() as u64 % 40 == 0 {
            eprintln!("-- round {}", played + 1);
            trace_bots(&sim, &bots, t);
        }
        let phase = sim.app.world().resource::<RoundState>().phase;
        match phase {
            Phase::Live { since, .. } => {
                if live_at.is_none() {
                    live_at = Some(since);
                    contact = None;
                    let tac = sim.app.world().resource::<Tactics>();
                    sites.push(
                        tac.team(tac.attackers)
                            .and_then(|p| p.site)
                            .map_or("-".into(), |s| tac.site_name(s).to_string()),
                    );
                }
                if contact.is_none()
                    && bots
                        .iter()
                        .any(|b| sim.app.world().get::<Bot>(*b).is_some_and(|x| x.target.is_some()))
                {
                    contact = Some(sim.app.world().resource::<Time>().elapsed_secs_f64() - since);
                    if trace {
                        for b in &bots {
                            if let Some(t) = sim.app.world().get::<Bot>(*b).and_then(|x| x.target) {
                                eprintln!("contact: {b} at {:.1} sees {t} at {:.1}", feet(&sim, *b), feet(&sim, t));
                            }
                        }
                    }
                }
            }
            Phase::Over { winner, .. } => {
                if let Some(since) = live_at.take() {
                    played += 1;
                    let now = sim.app.world().resource::<Time>().elapsed_secs_f64();
                    let w = sim.app.world_mut();
                    let kills: i32 = w.query::<&Score>().iter(w).map(|s| s.kills).sum();
                    let alive: Vec<(u8, bool)> = bots
                        .iter()
                        .map(|b| (w.get::<Team>(*b).unwrap().0, w.get::<Health>(*b).unwrap().current > 0.0))
                        .collect();
                    let alive_t = alive.iter().filter(|a| a.0 == 1 && a.1).count();
                    let alive_ct = alive.iter().filter(|a| a.0 == 2 && a.1).count();
                    let k = kills - last_kills;
                    last_kills = kills;
                    kills_total += k;
                    wins[winner.map_or(0, |t| t.0 as usize)] += 1;
                    if let Some(c) = contact {
                        contacts.push(c);
                    }
                    lengths.push(now - since);
                    stats.push(format!(
                        "round {played}: attackers to {}, winner {:?}, {:.1} s, contact {}, kills {k}, alive T {alive_t} CT {alive_ct}",
                        sites.last().unwrap(),
                        winner.map(|t| if t.0 == 1 { "T" } else { "CT" }),
                        now - since,
                        contact.map_or("none".into(), |c| format!("{c:.1} s")),
                    ));
                    eprintln!("{}", stats.last().unwrap());
                }
            }
            _ => {}
        }
    }
    let mean = |v: &[f64]| {
        if v.is_empty() {
            f64::NAN
        } else {
            v.iter().sum::<f64>() / v.len() as f64
        }
    };
    eprintln!(
        "{map}: {played} rounds, T {} CT {} draw {}, kills {kills_total}, mean round {:.1} s, mean time to contact {:.1} s ({} rounds with contact)",
        wins[1],
        wins[2],
        wins[0],
        mean(&lengths),
        mean(&contacts),
        contacts.len()
    );
    stuck.report(&map);
    assert!(played > 0, "no round finished");
}

/// Bots trying to walk somewhere without getting anywhere: each episode
/// (no more than `STUCK_MOVE` m in `STUCK_TIME` s while walking a route)
/// is printed, and the spots are summed up at the end.
#[derive(Default)]
struct StuckWatch {
    /// Per bot: recent (time, feet) samples, and whether it is in an
    /// episode now.
    samples: std::collections::HashMap<Entity, (Vec<(f64, Vec3)>, bool)>,
    /// Spots: where, place, how often, what it was walking toward.
    spots: Vec<(Vec3, String, u32, Vec3)>,
}

const STUCK_TIME: f64 = 4.0;
const STUCK_MOVE: f32 = 0.75;

impl StuckWatch {
    fn check(&mut self, sim: &Sim, bots: &[Entity], t: f64) {
        let w = sim.app.world();
        let nav = w.get_resource::<mashup::map::nav::NavMesh>();
        for &b in bots {
            let (Some(bot), Some(h)) = (w.get::<Bot>(b), w.get::<Health>(b)) else {
                continue;
            };
            let walking = matches!(
                bot.activity(),
                Activity::ToSite
                    | Activity::Following
                    | Activity::ToHold
                    | Activity::Chasing
                    | Activity::Assisting
                    | Activity::Hunting
                    | Activity::Roaming
            ) && bot.route_left() > 1.0;
            let entry = self.samples.entry(b).or_default();
            if h.current <= 0.0 || !walking {
                entry.0.clear();
                entry.1 = false;
                continue;
            }
            let at = feet(sim, b);
            entry.0.push((t, at));
            entry.0.retain(|s| t - s.0 <= STUCK_TIME + 0.05);
            if t - entry.0[0].0 < STUCK_TIME - 0.05 {
                continue;
            }
            let moved = entry.0.iter().map(|s| s.1.distance(at)).fold(0.0, f32::max);
            if moved > STUCK_MOVE {
                entry.1 = false;
                continue;
            }
            if entry.1 {
                continue;
            }
            entry.1 = true;
            let place = nav
                .and_then(|n| {
                    n.area_at(at + Vec3::Y * 0.1)
                        .and_then(|a| n.areas[a].place)
                        .map(|p| n.places[p].clone())
                })
                .unwrap_or_default();
            let next = bot.route().0.get(bot.route().1).copied().unwrap_or(at);
            eprintln!(
                "stuck: {t:.1} {b} {:?} {place} at {at:.2} next {next:.2} goal {:.1?}",
                bot.activity(),
                bot.goal()
            );
            match self.spots.iter_mut().find(|s| s.0.distance(at) < 3.0) {
                Some(s) => s.2 += 1,
                None => self.spots.push((at, place, 1, next)),
            }
        }
    }

    fn report(&mut self, map: &str) {
        self.spots.sort_by(|a, b| b.2.cmp(&a.2));
        let total: u32 = self.spots.iter().map(|s| s.2).sum();
        eprintln!("{map}: {total} stuck episodes at {} spots", self.spots.len());
        for (at, place, n, next) in &self.spots {
            eprintln!("  {n:3} x {place} at {at:.1} toward {next:.1}");
        }
    }
}
