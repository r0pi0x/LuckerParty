//! What bots say on the team radio (`core::Radio`): "Enemy spotted" when
//! an enemy first comes into sight, "Enemy down" after a kill, "Need
//! backup" when hurt badly. Each call has its own cooldown per bot, and a
//! bot stays quiet when a teammate said the same thing a moment ago. The
//! calls are CS:S bots' radio habits as seen in play; the numbers are
//! ours (docs/tech-debt.md).
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
    core::{Died, Health, Intent, Radio, SpawnPoint, Team},
};

/// Seconds before a bot repeats a call.
pub const COOLDOWN: f64 = 12.0;
/// Seconds a teammate's call keeps the rest of the team from repeating it.
pub const TEAM_QUIET: f64 = 4.0;
/// Health share at or below which a hurt bot asks for backup.
pub const BACKUP_HEALTH: f32 = 0.4;

/// A bot's radio memory: when each call may be said again, what it saw
/// last tick and its health then.
#[derive(Clone, Debug, Default)]
pub struct BotRadio {
    next: Vec<(&'static str, f64)>,
    had_target: bool,
    health: Option<f32>,
    /// An answer to a teammate's command, said at that time.
    pub(super) reply: Option<(&'static str, f64)>,
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

/// Recent calls per team, for `TEAM_QUIET`.
#[derive(Resource, Default)]
pub(super) struct TeamCalls(Vec<(Team, &'static str, f64)>);

#[allow(clippy::type_complexity)]
pub(super) fn speak(
    mut bots: Query<(Entity, &mut Bot, &Health, &Team)>,
    teams: Query<&Team>,
    mut died: MessageReader<Died>,
    mut radio: MessageWriter<Radio>,
    mut recent: ResMut<TeamCalls>,
    cfg: Res<super::BotConfig>,
    time: Res<Time>,
) {
    let now = time.elapsed_secs_f64();
    recent.0.retain(|(_, _, at)| now - at < TEAM_QUIET);
    let mut wanted: Vec<(Entity, &'static str)> = Vec::new();
    for d in died.read() {
        let Some(killer) = d.attacker.filter(|a| *a != d.entity) else {
            continue;
        };
        let enemies = match (teams.get(killer), teams.get(d.entity)) {
            (Ok(a), Ok(b)) => a != b,
            _ => true,
        };
        if enemies && bots.contains(killer) {
            wanted.push((killer, "enemydown"));
        }
    }
    for (e, mut bot, health, _) in &mut bots {
        let has_target = bot.target.is_some();
        if let Some(call) = observe(&mut bot.radio, has_target, health.current) {
            wanted.push((e, call));
        }
    }
    // Answers to teammates' commands, when due (no cooldown: every
    // command gets its answer).
    for (e, mut bot, health, _) in &mut bots {
        let Some((call, at)) = bot.radio.reply else { continue };
        if now < at {
            continue;
        }
        bot.radio.reply = None;
        if cfg.radio != 0 && health.current > 0.0 {
            radio.write(Radio {
                sender: e,
                command: call.into(),
            });
        }
    }
    if cfg.radio == 0 {
        return;
    }
    for (e, call) in wanted {
        let Ok((_, mut bot, health, team)) = bots.get_mut(e) else {
            continue;
        };
        if health.current <= 0.0 && call != "enemydown" {
            continue;
        }
        if recent.0.iter().any(|(t, c, _)| t == team && *c == call) || !bot.radio.ready(call, now) {
            continue;
        }
        recent.0.push((*team, call, now));
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
