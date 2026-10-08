//! What bots say on the team radio (`core::Radio`): "Enemy spotted" when
//! an enemy first comes into sight, "Enemy down" after a kill, "Need
//! backup" when hurt badly, and on their own: the attackers' leader's
//! "Go go go" or "Stick together team" as the round opens and "Follow
//! me" when it leads the group onto the site, "Cover me" while planting
//! or defusing, "Sector clear" a while after a fight, "In position" when
//! holding, "Regroup team" when most of the team is down. Each call has
//! its own cooldown per bot (commands: once a round), a bot stays quiet
//! when a teammate said the same thing a moment ago, and a team's bots
//! start one call per `TEAM_GAP` at most. The calls are CS:S bots' radio
//! habits as seen in play; when and how often are ours
//! (docs/tech-debt.md). The install's radio script has the bomb only in
//! the round's announcements (`Event.BombPlanted`), no team call about
//! it, so bots don't call it.
//!
//! What bots do with teammates' radio commands (`obey`): one or two of
//! the nearest answer "Roger that" / "Affirmative" (or "Negative" when
//! busy) a moment later, and they take an `Order`: follow the caller
//! ("Follow me", "Cover me", "Stick together team"), stay near the
//! caller's spot ("Hold this position", "Get in position and wait"), go
//! to it ("Regroup team"), back off toward their spawn ("Team fall back",
//! "Get out of there"), or drop orders and press on ("Go go go", "Storm
//! the front", "You take the point"). A person's "Need backup" or
//! "Taking fire" gets an answer from a bot `tactics` sends. CS:S's bot
//! behaviour isn't in the specs: which call does what, ranges and times
//! are ours (docs/tech-debt.md).

use bevy::prelude::*;

use super::Bot;
use crate::{
    character::CAPSULE_HEIGHT,
    core::{Died, Health, Intent, Radio, RoundRestarts, SpawnPoint, Team},
    objectives::bomb::PlantedBomb,
};

/// Seconds before a bot repeats a call.
pub const COOLDOWN: f64 = 12.0;
/// Seconds a teammate's call keeps the rest of the team from repeating it.
pub const TEAM_QUIET: f64 = 4.0;
/// Seconds between two calls bots of one team start (answers to commands
/// aren't counted).
pub const TEAM_GAP: f64 = 3.0;
/// Health share at or below which a hurt bot asks for backup.
pub const BACKUP_HEALTH: f32 = 0.4;
/// What happened (enemy spotted or down, need backup) waits this long
/// for the team's `TEAM_GAP`, s.
pub const PENDING_FOR: f64 = 3.0;
/// The attackers' leader calls the round's start ("Go go go" or "Stick
/// together team") this long after the round opens, s; not after the
/// window's end.
pub const ROUND_CALL_AT: f64 = 1.0;
pub const ROUND_CALL_WINDOW: f64 = 10.0;
/// Share of round starts the leader says "Go go go" rather than "Stick
/// together team".
pub const GO_SHARE: f32 = 1.0 / 3.0;
/// "Sector clear": this long after the last enemy seen or heard
/// following a fight, s.
pub const CLEAR_AFTER: f64 = 5.0;
/// "Regroup team": once a team (of at least `REGROUP_TEAM` bots) is down
/// to this share of its bots, with someone left to regroup with.
pub const REGROUP_SHARE: f32 = 0.34;
pub const REGROUP_TEAM: usize = 3;
/// What bots say on their own (not answers): the cooldowns above apply.
pub const CALLS: &[&str] = &[
    "enemydown",
    "enemyspot",
    "needbackup",
    "coverme",
    "regroup",
    "go",
    "sticktog",
    "followme",
    "sectorclear",
    "inposition",
];

/// A bot's radio memory: when each call may be said again, what it saw
/// last tick and its health then, and the last thing it said.
#[derive(Clone, Debug, Default)]
pub struct BotRadio {
    next: Vec<(&'static str, f64)>,
    had_target: bool,
    health: Option<f32>,
    /// An answer to a teammate's command, said at that time.
    pub(super) reply: Option<(&'static str, f64)>,
    /// A call for something that happened, and when, waiting for the
    /// team's turn (`PENDING_FOR`).
    pending: Option<(&'static str, f64)>,
    /// Planting or defusing now (`objectives::act`).
    pub(super) at_objective: bool,
    /// Fought since its last "Sector clear".
    fought: bool,
    /// Calls said once a round, and which round that is.
    said: Vec<&'static str>,
    round: u32,
    /// The last call (answers too) and when.
    last: Option<(&'static str, f64)>,
}

impl BotRadio {
    /// Whether `call` is off cooldown at `now`; if so it is used up.
    pub fn ready(&mut self, call: &'static str, now: f64) -> bool {
        match self.next.iter_mut().find(|(c, _)| *c == call) {
            Some((_, at)) if now < *at => false,
            Some((_, at)) => {
                *at = now + COOLDOWN;
                true
            }
            None => {
                self.next.push((call, now + COOLDOWN));
                true
            }
        }
    }

    /// Keep `call` for when the team's turn comes, unless a more urgent
    /// one (earlier in `CALLS`) waits already.
    fn hold(&mut self, call: &'static str, now: f64) {
        let rank = |c: &str| CALLS.iter().position(|x| *x == c).unwrap_or(CALLS.len());
        if self.pending.is_none_or(|(c, _)| rank(call) <= rank(c)) {
            self.pending = Some((call, now));
        }
    }

    /// Whether `call` was said in `round` already.
    fn said_in(&mut self, call: &'static str, round: u32) -> bool {
        if self.round != round {
            self.round = round;
            self.said.clear();
        }
        self.said.contains(&call)
    }
}

/// The call (if any) for what changed since last tick: an enemy came into
/// sight, or health fell to the backup threshold or was hit below it.
pub fn observe(radio: &mut BotRadio, has_target: bool, health: f32) -> Option<&'static str> {
    let spotted = has_target && !radio.had_target;
    let hurt = radio
        .health
        .is_some_and(|before| health < before && health <= BACKUP_HEALTH && health > 0.0);
    radio.had_target = has_target;
    radio.health = Some(health);
    if spotted {
        Some("enemyspot")
    } else if hurt {
        Some("needbackup")
    } else {
        None
    }
}

/// Whether a call is said once a round by one bot of the team (else once
/// a round per bot, or on cooldown).
fn once_per_team(call: &str) -> bool {
    matches!(call, "go" | "sticktog" | "regroup" | "inposition")
}

fn once_per_bot(call: &str) -> bool {
    matches!(call, "followme" | "coverme")
}

/// The leader's round start call: "Go go go" for `GO_SHARE` of rounds,
/// else "Stick together team", from the team's plan seed and the round.
pub fn round_call(seed: u64, round: u32) -> &'static str {
    let r = super::path::hash64(seed ^ 0x60_60_60, round as usize);
    if ((r >> 40) as f32 / (1u64 << 24) as f32) < GO_SHARE {
        "go"
    } else {
        "sticktog"
    }
}

/// Recent calls per team, for `TEAM_QUIET` and `TEAM_GAP`; calls said
/// once a round per team; when the round opened.
#[derive(Resource, Default)]
pub(super) struct TeamCalls {
    recent: Vec<(Team, &'static str, f64)>,
    last: Vec<(Team, f64)>,
    once: Vec<(Team, &'static str, u32)>,
    /// The round and when it opened (planting allowed: after the freeze).
    opened: Option<(u32, f64)>,
}

impl TeamCalls {
    fn gap_ok(&self, team: Team, now: f64) -> bool {
        self.last.iter().all(|(t, at)| *t != team || now - at >= TEAM_GAP)
    }
}

/// A living bot: team, feet, whom it follows, number.
type Mate = (Entity, Team, Vec3, Option<Entity>, u32);

type SpeakBots<'w, 's> = Query<
    'w,
    's,
    (
        Entity,
        &'static mut Bot,
        &'static Health,
        &'static Team,
        &'static Transform,
    ),
>;

#[allow(clippy::too_many_arguments)]
pub(super) fn speak(
    mut bots: SpeakBots,
    teams: Query<&Team>,
    mut died: MessageReader<Died>,
    mut radio: MessageWriter<Radio>,
    mut calls: ResMut<TeamCalls>,
    tactics: Res<super::Tactics>,
    open: Option<Res<crate::objectives::RoundOpen>>,
    restarts: Option<Res<RoundRestarts>>,
    planted: Query<&PlantedBomb>,
    cfg: Res<super::BotConfig>,
    time: Res<Time>,
) {
    let now = time.elapsed_secs_f64();
    let round = restarts.map_or(0, |r| r.0);
    let open = open.is_none_or(|o| o.0);
    if !open {
        calls.opened = None;
    } else if calls.opened.map(|o| o.0) != Some(round) {
        calls.opened = Some((round, now));
    }
    let since_open = calls.opened.map(|(_, at)| now - at);
    calls.recent.retain(|(_, _, at)| now - at < TEAM_QUIET);
    calls.once.retain(|(.., r)| *r == round);
    let bomb_planted = planted.iter().any(|b| !b.defused);
    // (priority, bot number, bot, call): said in this order.
    let mut wanted: Vec<(usize, u32, Entity, &'static str)> = Vec::new();
    let mut want = |e: Entity, number: u32, call: &'static str| {
        let p = CALLS.iter().position(|c| *c == call).unwrap_or(CALLS.len());
        wanted.push((p, number, e, call));
    };
    for d in died.read() {
        let Some(killer) = d.attacker.filter(|a| *a != d.entity) else {
            continue;
        };
        let enemies = match (teams.get(killer), teams.get(d.entity)) {
            (Ok(a), Ok(b)) => a != b,
            _ => true,
        };
        if enemies && let Ok((_, mut bot, ..)) = bots.get_mut(killer) {
            bot.radio.hold("enemydown", now);
        }
    }
    let feet_of = |t: &Transform| t.translation - Vec3::Y * CAPSULE_HEIGHT / 2.0;
    // Living bots per team (position, following whom), and how many the
    // team has.
    let living: Vec<Mate> = bots
        .iter()
        .filter(|b| b.2.current > 0.0)
        .map(|(e, b, _, team, t)| {
            let leader = b.order.and_then(|o| match o.order {
                Order::Follow { leader, .. } => Some(leader),
                _ => None,
            });
            (e, *team, feet_of(t), leader, b.number)
        })
        .collect();
    let all: Vec<Team> = bots.iter().map(|b| *b.3).collect();
    let team_size = |team: Team| all.iter().filter(|t| **t == team).count();
    for (e, mut bot, health, &team, t) in &mut bots {
        let has_target = bot.target.is_some();
        let alive = health.current > 0.0;
        if let Some(call) = observe(&mut bot.radio, has_target, health.current) {
            bot.radio.hold(call, now);
        }
        match bot.radio.pending {
            Some((_, at)) if now - at > PENDING_FOR => bot.radio.pending = None,
            Some((call, _)) => want(e, bot.number, call),
            None => {}
        }
        if !alive {
            bot.radio.fought = false;
            continue;
        }
        if has_target {
            bot.radio.fought = true;
        }
        let feet = feet_of(t);
        let mates_near: Vec<&Mate> = living
            .iter()
            .filter(|m| m.0 != e && m.1 == team && m.2.distance(feet) <= ORDER_RANGE)
            .collect();
        let plan = tactics.team(team);
        let leads = plan.is_some_and(|p| p.role == super::Role::Attack && p.leader == Some(e));
        // Round start: the attackers' leader.
        if leads
            && since_open.is_some_and(|s| (ROUND_CALL_AT..ROUND_CALL_AT + ROUND_CALL_WINDOW).contains(&s))
            && !mates_near.is_empty()
            && let Some(p) = plan
        {
            want(e, bot.number, round_call(p.seed, round));
        }
        // Leading the group onto the site.
        if leads && plan.is_some_and(|p| p.staged) && !bomb_planted && mates_near.iter().any(|m| m.3 != Some(e)) {
            want(e, bot.number, "followme");
        }
        // Planting or defusing with a teammate near.
        if bot.radio.at_objective && !mates_near.is_empty() {
            want(e, bot.number, "coverme");
        }
        if !has_target && bot.radio.fought && now - bot.contact >= CLEAR_AFTER {
            want(e, bot.number, "sectorclear");
        }
        if bot.activity == super::Activity::Holding && bot.order.is_none() {
            want(e, bot.number, "inposition");
        }
        // Most of the team down: the lowest numbered of the rest calls
        // the others together (not with a bomb down).
        let mates: Vec<&Mate> = living.iter().filter(|m| m.1 == team).collect();
        let size = team_size(team);
        if size >= REGROUP_TEAM
            && mates.len() >= 2
            && mates.len() as f32 <= size as f32 * REGROUP_SHARE + 0.01
            && !bomb_planted
            && !has_target
            && mates.iter().map(|m| m.4).min() == Some(bot.number)
        {
            want(e, bot.number, "regroup");
        }
    }
    // Answers to teammates' commands, when due (no cooldown: every
    // command gets its answer).
    for (e, mut bot, health, ..) in &mut bots {
        let Some((call, at)) = bot.radio.reply else { continue };
        if now < at {
            continue;
        }
        bot.radio.reply = None;
        if cfg.radio != 0 && health.current > 0.0 {
            bot.radio.last = Some((call, now));
            radio.write(Radio {
                sender: e,
                command: call.into(),
            });
        }
    }
    if cfg.radio == 0 {
        return;
    }
    wanted.sort_by_key(|w| (w.0, w.1));
    for (_, _, e, call) in wanted {
        let Ok((_, mut bot, health, &team, _)) = bots.get_mut(e) else {
            continue;
        };
        if health.current <= 0.0 && call != "enemydown" {
            continue;
        }
        if !calls.gap_ok(team, now) || calls.recent.iter().any(|(t, c, _)| *t == team && *c == call) {
            continue;
        }
        if once_per_team(call) && calls.once.iter().any(|(t, c, _)| *t == team && *c == call) {
            continue;
        }
        if once_per_bot(call) && bot.radio.said_in(call, round) {
            continue;
        }
        if !bot.radio.ready(call, now) {
            continue;
        }
        if once_per_team(call) {
            calls.once.push((team, call, round));
        }
        if once_per_bot(call) {
            bot.radio.said.push(call);
        }
        if call == "sectorclear" {
            bot.radio.fought = false;
        }
        if bot.radio.pending.is_some_and(|(c, _)| c == call) {
            bot.radio.pending = None;
        }
        calls.recent.push((team, call, now));
        calls.last.retain(|(t, _)| *t != team);
        calls.last.push((team, now));
        bot.radio.last = Some((call, now));
        radio.write(Radio {
            sender: e,
            command: call.into(),
        });
    }
}

/// Seconds before the first answer to a command (where in the range comes
/// from the bot's seed), and between two bots answering.
pub const REPLY_DELAY: (f64, f64) = (0.5, 1.2);
pub const REPLY_STAGGER: f64 = 0.8;
/// Bots that answer one command.
pub const REPLIES: usize = 2;
/// Bots within this of the caller follow ("Follow me", "Cover me",
/// "Stick together team"), hold ("Hold this position") or get out ("Get
/// out of there"), m.
pub const ORDER_RANGE: f32 = 30.0;
/// Following: walks to the caller when farther than this, m; for this
/// long ("Cover me": `COVER_TIME`), s.
pub const FOLLOW_NEAR: f32 = 3.5;
pub const FOLLOW_TIME: f64 = 40.0;
pub const COVER_TIME: f64 = 20.0;
/// Holding: walks back to the spot when farther than this, m; for this
/// long, s.
pub const HOLD_NEAR: f32 = 4.0;
pub const HOLD_TIME: f64 = 30.0;
/// Regrouping or falling back ends this close to the spot, or after this
/// long, m and s.
pub const ARRIVE_NEAR: f32 = 4.0;
pub const REGROUP_TIME: f64 = 30.0;
pub const FALLBACK_TIME: f64 = 15.0;
/// "Go go go" and the like: no waiting for the group for this long, s.
pub const URGENT_TIME: f64 = 20.0;

/// A teammate's radio command a bot carries out (`Bot::order`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Order {
    /// Stay close to `leader` (`at`: where they stand, kept up to date).
    Follow { leader: Entity, at: Vec3 },
    /// Stay near a spot.
    Hold { at: Vec3 },
    /// Go where the caller was.
    Regroup { at: Vec3 },
    /// Back off toward the team's spawn.
    FallBack { to: Vec3 },
}

/// An order, who gave it, and until when it holds.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Ordered {
    pub order: Order,
    pub from: Entity,
    pub until: f64,
}

impl Ordered {
    /// What it does, given the caller's name (debug views).
    pub fn describe(&self, name: &str) -> String {
        match self.order {
            Order::Follow { .. } => format!("following {name}"),
            Order::Hold { .. } => format!("holding for {name}"),
            Order::Regroup { .. } => format!("regrouping on {name}"),
            Order::FallBack { .. } => format!("falling back ({name})"),
        }
    }

    /// Where the order takes it (feet).
    pub fn point(&self) -> Vec3 {
        match self.order {
            Order::Follow { at, .. } | Order::Hold { at } | Order::Regroup { at } => at,
            Order::FallBack { to } => to,
        }
    }
}

/// What a command asks of the bots that hear it.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Ask {
    /// The nearest free bot in range follows for this long.
    Follow(f64),
    /// Every free bot in range follows.
    StickTogether,
    /// Free bots in range hold at the caller's spot.
    Hold,
    /// Every free bot goes to the caller's spot.
    Regroup,
    /// Every free bot (only those in range: `true`) falls back.
    FallBack(bool),
    /// Every bot drops its order and presses on (storming: defenders too).
    Go(bool),
    /// The nearest bots answer this, nothing more.
    Answer(&'static str),
    /// A teammate `tactics` sent to help answers.
    Backup,
}

fn ask(command: &str) -> Option<Ask> {
    Some(match command {
        "followme" => Ask::Follow(FOLLOW_TIME),
        "coverme" => Ask::Follow(COVER_TIME),
        "sticktog" => Ask::StickTogether,
        "holdpos" | "getinpos" => Ask::Hold,
        "regroup" => Ask::Regroup,
        "fallback" => Ask::FallBack(false),
        "getout" => Ask::FallBack(true),
        "go" | "takepoint" => Ask::Go(false),
        "stormfront" => Ask::Go(true),
        "report" => Ask::Answer("reportingin"),
        "needbackup" | "takingfire" => Ask::Backup,
        _ => return None,
    })
}

/// When a bot (by its seed) answers a command heard at `now`, as the
/// `nth` to answer: deterministic, never from entity ids.
pub fn reply_at(seed: u64, now: f64, nth: usize) -> f64 {
    let r = super::path::hash64(seed ^ 0x5EED_4AD1_0C0D_E5A7, (now * 1000.0).round() as usize);
    let u = (r >> 11) as f64 / (1u64 << 53) as f64;
    now + REPLY_DELAY.0 + (REPLY_DELAY.1 - REPLY_DELAY.0) * u + nth as f64 * REPLY_STAGGER
}

impl Bot {
    /// The teammate's radio command it carries out.
    pub fn order(&self) -> Option<&Ordered> {
        self.order.as_ref()
    }

    /// Pressing on after "Go go go" (no waiting for the group).
    pub fn urgent(&self, now: f64) -> bool {
        now < self.urgent_until
    }

    /// The last thing it said on the radio (a command name), and when.
    pub fn last_call(&self) -> Option<(&'static str, f64)> {
        self.radio.last
    }

    /// The answer it is about to say on the radio.
    pub fn reply(&self) -> Option<&'static str> {
        self.radio.reply.map(|r| r.0)
    }

    /// Free to take an order: not fighting, planting, defusing or
    /// carrying the bomb, nor sent somewhere.
    fn free(&self) -> bool {
        self.target.is_none() && self.objective.is_none() && !self.carrying && self.move_to.is_none()
    }

    fn answer(&mut self, call: &'static str, now: f64, nth: usize) {
        if self.radio.reply.is_none() {
            self.radio.reply = Some((call, reply_at(self.seed, now, nth)));
        }
    }
}

/// Where its order takes a bot (`Some(None)`: stay where it is), if it
/// has one.
pub(super) fn order_goal(bot: &mut Bot, feet: Vec3) -> Option<Option<Vec3>> {
    let o = bot.order?;
    bot.activity = super::Activity::Obeying;
    let away = |at: Vec3, near: f32| (at - feet).xz().length() > near || (at.y - feet.y).abs() > 1.5;
    Some(match o.order {
        Order::Follow { at, .. } => away(at, FOLLOW_NEAR).then_some(at),
        Order::Hold { at } => away(at, HOLD_NEAR).then_some(at),
        Order::Regroup { at } => Some(at),
        Order::FallBack { to } => Some(to),
    })
}

type Bots<'w, 's> = Query<
    'w,
    's,
    (
        Entity,
        &'static mut Bot,
        &'static Transform,
        &'static Team,
        &'static Health,
    ),
>;

/// A free bot `e` takes `order` (from `from`, for `time` seconds) and,
/// while fewer than `REPLIES` have, answers "Roger that".
fn take(bots: &mut Bots, e: Entity, order: Order, from: Entity, time: f64, now: f64, answered: &mut usize) {
    let Ok((_, mut bot, ..)) = bots.get_mut(e) else { return };
    if !bot.free() {
        return;
    }
    bot.order = Some(Ordered {
        order,
        from,
        until: now + time,
    });
    bot.assist = None;
    if *answered < REPLIES {
        bot.answer("roger", now, *answered);
        *answered += 1;
    }
}

/// Teammates' radio commands become orders and answers; orders run out,
/// keep up with whom they follow and end on arrival.
#[allow(clippy::type_complexity)]
pub(super) fn obey(
    mut calls: MessageReader<Radio>,
    mut bots: Bots,
    chars: Query<(&Transform, &Team, &Health, Has<Bot>), With<Intent>>,
    spawns: Query<(&Transform, &SpawnPoint)>,
    time: Res<Time>,
) {
    let now = time.elapsed_secs_f64();
    let feet_of = |t: &Transform| t.translation - Vec3::Y * CAPSULE_HEIGHT / 2.0;
    for (_, mut bot, t, _, health) in &mut bots {
        let Some(mut o) = bot.order else { continue };
        let feet = feet_of(t);
        let done = match &mut o.order {
            _ if now > o.until || health.current <= 0.0 => true,
            Order::Follow { leader, at } => match chars.get(*leader) {
                Ok((lt, _, lh, _)) if lh.current > 0.0 => {
                    *at = feet_of(lt);
                    false
                }
                _ => true,
            },
            Order::Hold { .. } => false,
            Order::Regroup { at } | Order::FallBack { to: at } => (*at - feet).xz().length() < ARRIVE_NEAR,
        };
        if done {
            bot.order = None;
        } else if bot.order != Some(o) {
            bot.order = Some(o);
        }
    }
    for call in calls.read() {
        let Some(asked) = ask(&call.command) else { continue };
        let Ok((ct, &team, ch, from_bot)) = chars.get(call.sender) else {
            continue;
        };
        // Bots answer people's calls for backup; tactics sends help to
        // bots' calls without a word.
        if ch.current <= 0.0 || (asked == Ask::Backup && from_bot) {
            continue;
        }
        let from = feet_of(ct);
        let sender = call.sender;
        // Living teammate bots, nearest first (bot number breaks ties).
        let mut mates: Vec<(Entity, f32, u32)> = bots
            .iter()
            .filter(|(e, _, _, tm, h)| *e != sender && **tm == team && h.current > 0.0)
            .map(|(e, b, t, ..)| (e, feet_of(t).distance(from), b.number))
            .collect();
        mates.sort_by(|a, b| a.1.total_cmp(&b.1).then(a.2.cmp(&b.2)));
        let near: Vec<Entity> = mates.iter().filter(|m| m.1 <= ORDER_RANGE).map(|m| m.0).collect();
        let all: Vec<Entity> = mates.iter().map(|m| m.0).collect();
        let mut answered = 0;
        match asked {
            Ask::Follow(time) => {
                // The nearest free one follows; a busy one nearer says no.
                let mut said_no = false;
                for &e in &near {
                    if bots.get(e).is_ok_and(|b| b.1.free()) {
                        let order = Order::Follow {
                            leader: sender,
                            at: from,
                        };
                        take(&mut bots, e, order, sender, time, now, &mut answered);
                        break;
                    }
                    let Ok((_, mut bot, ..)) = bots.get_mut(e) else {
                        continue;
                    };
                    if !said_no {
                        bot.answer("negative", now, answered);
                        said_no = true;
                        answered += 1;
                    }
                }
            }
            Ask::StickTogether => {
                for &e in &near {
                    let order = Order::Follow {
                        leader: sender,
                        at: from,
                    };
                    take(&mut bots, e, order, sender, FOLLOW_TIME, now, &mut answered);
                }
            }
            Ask::Hold => {
                for &e in &near {
                    take(
                        &mut bots,
                        e,
                        Order::Hold { at: from },
                        sender,
                        HOLD_TIME,
                        now,
                        &mut answered,
                    );
                }
            }
            Ask::Regroup => {
                for &e in &all {
                    take(
                        &mut bots,
                        e,
                        Order::Regroup { at: from },
                        sender,
                        REGROUP_TIME,
                        now,
                        &mut answered,
                    );
                }
            }
            Ask::FallBack(only_near) => {
                let own: Vec<Vec3> = spawns
                    .iter()
                    .filter(|s| s.1.team == Some(team))
                    .map(|s| feet_of(s.0))
                    .collect();
                if own.is_empty() {
                    continue;
                }
                // The spawn point nearest the team's spawns' middle.
                let mid = own.iter().sum::<Vec3>() / own.len() as f32;
                let to = own
                    .iter()
                    .copied()
                    .min_by(|a, b| a.distance(mid).total_cmp(&b.distance(mid)))
                    .unwrap_or(mid);
                for &e in if only_near { &near } else { &all } {
                    take(
                        &mut bots,
                        e,
                        Order::FallBack { to },
                        sender,
                        FALLBACK_TIME,
                        now,
                        &mut answered,
                    );
                }
            }
            Ask::Go(storm) => {
                for &e in &all {
                    let Ok((_, mut bot, ..)) = bots.get_mut(e) else {
                        continue;
                    };
                    bot.order = None;
                    bot.assist = None;
                    bot.lead = None;
                    bot.urgent_until = now + URGENT_TIME;
                    if storm && bot.orders.role == super::Role::Defend && bot.objective.is_none() {
                        bot.hold = None;
                        bot.hunting = true;
                    }
                    if answered < REPLIES {
                        bot.answer("roger", now, answered);
                        answered += 1;
                    }
                }
            }
            Ask::Answer(what) => {
                for &e in all.iter().take(REPLIES) {
                    if let Ok((_, mut bot, ..)) = bots.get_mut(e) {
                        bot.answer(what, now, answered);
                        answered += 1;
                    }
                }
            }
            Ask::Backup => {
                // `tactics` sent the nearest free ones this tick.
                for &e in &all {
                    let Ok((_, mut bot, ..)) = bots.get_mut(e) else {
                        continue;
                    };
                    if bot.assist.is_some_and(|(_, when)| when == now) {
                        bot.answer("roger", now, 0);
                        break;
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn answers_come_late_and_one_after_another() {
        let a = reply_at(7, 10.0, 0);
        assert!((10.0 + REPLY_DELAY.0..=10.0 + REPLY_DELAY.1).contains(&a), "{a}");
        assert_eq!(a, reply_at(7, 10.0, 0), "same seed, same time");
        let b = reply_at(8, 10.0, 1);
        assert!(b >= 10.0 + REPLY_DELAY.0 + REPLY_STAGGER, "{b}");
    }

    #[test]
    fn which_calls_ask_what() {
        assert_eq!(ask("followme"), Some(Ask::Follow(FOLLOW_TIME)));
        assert_eq!(ask("holdpos"), Some(Ask::Hold));
        assert_eq!(ask("stormfront"), Some(Ask::Go(true)));
        assert_eq!(ask("roger"), None, "answers aren't answered");
        assert_eq!(ask("enemyspot"), None);
    }

    #[test]
    fn calls_wait_out_their_cooldown() {
        let mut r = BotRadio::default();
        assert!(r.ready("enemyspot", 0.0));
        assert!(!r.ready("enemyspot", COOLDOWN - 0.1));
        assert!(r.ready("enemydown", 1.0), "each call has its own");
        assert!(r.ready("enemyspot", COOLDOWN));
    }

    #[test]
    fn round_calls_come_from_the_seed() {
        let calls: Vec<&str> = (0..30).map(|r| round_call(42, r)).collect();
        assert_eq!(calls, (0..30).map(|r| round_call(42, r)).collect::<Vec<_>>());
        let go = calls.iter().filter(|c| **c == "go").count();
        assert!((3..20).contains(&go), "{go} of 30 rounds start with go");
        assert!(calls.iter().all(|c| CALLS.contains(c)));
    }

    #[test]
    fn a_team_starts_one_call_per_gap() {
        let mut t = TeamCalls::default();
        t.last.push((Team(1), 10.0));
        assert!(!t.gap_ok(Team(1), 10.0 + TEAM_GAP - 0.1));
        assert!(t.gap_ok(Team(2), 10.5), "per team");
        assert!(t.gap_ok(Team(1), 10.0 + TEAM_GAP));
    }

    #[test]
    fn commands_once_a_round() {
        let mut r = BotRadio::default();
        assert!(!r.said_in("coverme", 1));
        r.said.push("coverme");
        assert!(r.said_in("coverme", 1));
        assert!(!r.said_in("coverme", 2), "a new round");
    }

    #[test]
    fn spotting_and_getting_hurt() {
        let mut r = BotRadio::default();
        assert_eq!(observe(&mut r, false, 1.0), None);
        assert_eq!(observe(&mut r, true, 1.0), Some("enemyspot"));
        assert_eq!(observe(&mut r, true, 1.0), None, "same sighting");
        assert_eq!(observe(&mut r, true, 0.5), None, "still healthy");
        assert_eq!(observe(&mut r, true, 0.3), Some("needbackup"));
        assert_eq!(observe(&mut r, true, 0.3), None, "not hit again");
        assert_eq!(observe(&mut r, true, 0.0), None, "dead");
        assert_eq!(observe(&mut r, false, 1.0), None, "respawned");
    }
}
