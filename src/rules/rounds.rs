//! Team rounds with money, the Counter-Strike way (`mashup_rounds 1`):
//! a freeze period, a timed round won by eliminating the other team (or
//! by the defenders when time runs out), a pause, then everyone back at
//! their spawns; survivors keep their weapons. Money: a start amount, kill
//! rewards, a win bonus and a loss bonus that grows with a losing streak;
//! buying is open during the freeze and the buy period.
//!
//! Objectives (`objectives`, specs/cs_source/objectives.md 7-8): on bomb
//! maps the bomb ends the round once planted (exploded: terrorists,
//! defused: counter-terrorists) and the defenders win on time; on hostage
//! maps rescuing every hostage wins for the counter-terrorists and the
//! terrorists win on time; each reason pays its own bonus.
//!
//! Defaults are CS:S's public server defaults (mp_freezetime,
//! mp_roundtime, mp_buytime, mp_startmoney) and its well-known reward
//! amounts; none measured here yet (docs/plans/active/rounds.md).

use bevy::prelude::*;

use super::{Dead, put_at_spawn};
use crate::{
    console::{ConsoleAppExt, resource_cvar},
    core::{Died, FreezeTime, Health, Intent, Team},
    objectives::{
        MapKind, MapObjectives, ObjectiveEvent, RoundOpen,
        bomb::{BombOutcome, BombState},
        hostages::{Hostage, HostageTally},
    },
    weapon::economy::{BuyWindow, Money},
};

/// The two sides: `Team(1)` attacks (terrorists), `Team(2)` defends
/// (counter-terrorists) and wins when time runs out.
pub const ATTACKERS: Team = Team(1);
pub const DEFENDERS: Team = Team(2);

#[derive(Resource, Clone, Debug, PartialEq)]
pub struct RoundSettings {
    /// 1: play rounds (instead of deathmatch respawns).
    pub enabled: u8,
    /// Seconds frozen at the start of a round (mp_freezetime).
    pub freeze_time: f32,
    /// Minutes a round lasts (mp_roundtime).
    pub round_time: f32,
    /// Minutes after the freeze that buying stays open (mp_buytime).
    pub buy_time: f32,
    /// mp_startmoney.
    pub start_money: u32,
    pub max_money: u32,
    /// Seconds between a round's end and the next one.
    pub end_delay: f32,
    pub win_bonus: u32,
    /// Loss bonus: base, step per consecutive loss, cap.
    pub loss_bonus: u32,
    pub loss_step: u32,
    pub loss_max: u32,
    pub kill_reward: u32,
    pub teamkill_penalty: u32,
    /// Objective wins (spec 8, *public*/*hyp.*): the bomb exploded or was
    /// defused; time ran out on a bomb map with no plant; all hostages
    /// rescued; time ran out on a hostage map.
    pub win_bomb: u32,
    pub win_target_saved: u32,
    pub win_rescued: u32,
    pub win_not_rescued: u32,
    /// Every terrorist, on top of the loss bonus, when the bomb was planted
    /// and they lost anyway.
    pub plant_loss_bonus: u32,
    pub planter_reward: u32,
    pub defuser_reward: u32,
    /// The defender who first leads a hostage this round; the one whose
    /// hostage is rescued; whoever kills one loses this.
    pub hostage_touch: u32,
    pub hostage_rescue: u32,
    pub hostage_kill_penalty: u32,
}

impl Default for RoundSettings {
    fn default() -> Self {
        Self {
            enabled: 0,
            freeze_time: 6.0,
            round_time: 5.0,
            buy_time: 1.5,
            start_money: 800,
            max_money: 16000,
            end_delay: 5.0,
            win_bonus: 3250,
            loss_bonus: 1400,
            loss_step: 500,
            loss_max: 3400,
            kill_reward: 300,
            teamkill_penalty: 3300,
            win_bomb: 3500,
            win_target_saved: 3250,
            win_rescued: 3500,
            win_not_rescued: 3250,
            plant_loss_bonus: 800,
            planter_reward: 300,
            defuser_reward: 0,
            hostage_touch: 150,
            hostage_rescue: 1000,
            hostage_kill_penalty: 1500,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Phase {
    /// Rounds are off.
    Off,
    /// Frozen until this time.
    Freeze { until: f64 },
    /// Playing; started at, ends at.
    Live { since: f64, ends: f64 },
    /// Over (winner, None: draw); the next round starts at `until`.
    Over { until: f64, winner: Option<Team> },
}

#[derive(Resource, Clone, Debug, PartialEq)]
pub struct RoundState {
    pub phase: Phase,
    /// Rounds started since rounds were turned on.
    pub number: u32,
    /// Rounds won by attackers and defenders.
    pub wins: [u32; 2],
    /// Consecutive losses of attackers and defenders.
    losses: [u32; 2],
    /// When `mp_restartgame` restarts the game (seconds since startup).
    pub restart_at: Option<f64>,
}

impl Default for RoundState {
    fn default() -> Self {
        Self {
            phase: Phase::Off,
            number: 0,
            wins: [0; 2],
            losses: [0; 2],
            restart_at: None,
        }
    }
}

impl RoundState {
    /// Seconds left on the round clock (the freeze while frozen), if any.
    pub fn clock(&self, now: f64) -> Option<f32> {
        match self.phase {
            Phase::Freeze { until } => Some((until - now).max(0.0) as f32),
            Phase::Live { ends, .. } => Some((ends - now).max(0.0) as f32),
            Phase::Over { .. } | Phase::Off => None,
        }
    }
}

/// A round ended: the winner (None: draw) and why.
#[derive(Message, Clone, Debug, PartialEq)]
pub struct RoundEnded {
    pub winner: Option<Team>,
    pub reason: RoundEndReason,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RoundEndReason {
    Eliminated,
    TimeRanOut,
    Draw,
    /// The planted bomb exploded.
    TargetBombed,
    BombDefused,
    /// Time ran out on a bomb map with no bomb planted.
    TargetSaved,
    HostagesRescued,
    /// Time ran out on a hostage map.
    HostagesNotRescued,
}

impl RoundEndReason {
    /// What the game says across the screen (CS:S's words; elimination
    /// and time name the winner).
    pub fn text(self, winner: Option<Team>) -> &'static str {
        match self {
            RoundEndReason::TargetBombed => "Target Successfully Bombed!",
            RoundEndReason::BombDefused => "The bomb has been defused.",
            RoundEndReason::TargetSaved => "Target has been saved!",
            RoundEndReason::HostagesRescued => "All hostages have been rescued!",
            RoundEndReason::HostagesNotRescued => "Hostages have not been rescued!",
            RoundEndReason::Draw => "Round Draw!",
            _ => match winner {
                Some(ATTACKERS) => "Terrorists Win!",
                Some(DEFENDERS) => "Counter-Terrorists Win!",
                _ => "Round Draw!",
            },
        }
    }
}

fn side(team: Team) -> Option<usize> {
    match team {
        ATTACKERS => Some(0),
        DEFENDERS => Some(1),
        _ => None,
    }
}

pub(super) fn plugin(app: &mut App) {
    app.init_resource::<RoundSettings>()
        .init_resource::<RoundState>()
        .add_message::<RoundEnded>();
    let cvars: [(&str, &str, fn(&mut RoundSettings) -> &mut f32); 3] = [
        ("mp_freezetime", "Seconds frozen at the start of each round.", |r| {
            &mut r.freeze_time
        }),
        ("mp_roundtime", "Minutes per round.", |r| &mut r.round_time),
        ("mp_buytime", "Minutes buying stays open after the freeze.", |r| {
            &mut r.buy_time
        }),
    ];
    for (name, help, field) in cvars {
        resource_cvar::<RoundSettings, f32>(app, name, help, field);
    }
    resource_cvar::<RoundSettings, u8>(
        app,
        "mashup_rounds",
        "1: Counter-Strike rounds with money (0: deathmatch respawns).",
        |r| &mut r.enabled,
    );
    resource_cvar::<RoundSettings, u32>(app, "mp_startmoney", "Money each player starts with.", |r| {
        &mut r.start_money
    });
    app.console_command(
        "mp_restartgame",
        "mp_restartgame <seconds>: restart the game that many seconds from now (scores, money and the round \
         start over, everyone respawns; in deathmatch everyone respawns). 0 does nothing.",
        |w, a| {
            let secs = a
                .first()
                .and_then(|v| v.parse::<f64>().ok())
                .ok_or("mp_restartgame <seconds>")?
                .clamp(0.0, 60.0);
            if secs == 0.0 {
                return Ok(None);
            }
            // The rules' clock (fixed ticks), which can run behind the
            // frame clock when ticks are slow to simulate.
            let now = w.resource::<Time<Fixed>>().elapsed_secs_f64();
            w.resource_mut::<RoundState>().restart_at = Some(now + secs);
            Ok(Some(format!("Game will restart in {secs} seconds")))
        },
    );
}

/// `mp_restartgame` when its time comes: a fresh game (rounds), or
/// everyone back at a spawn (deathmatch).
fn restart_when_due(world: &mut World, rounds: bool, now: f64) {
    if !world.resource::<RoundState>().restart_at.is_some_and(|t| now >= t) {
        return;
    }
    if rounds {
        // Phase::Off: the next step starts a game with the start money.
        *world.resource_mut::<RoundState>() = RoundState::default();
    } else {
        world.resource_mut::<RoundState>().restart_at = None;
        super::respawn_everyone(world);
    }
}

/// Advance the round: start, freeze, play, decide, pause, restart.
pub(super) fn run_rounds(world: &mut World) {
    let settings = world.resource::<RoundSettings>().clone();
    let now = world.resource::<Time>().elapsed_secs_f64();
    restart_when_due(world, settings.enabled != 0, now);
    let phase = world.resource::<RoundState>().phase;
    // The dead drop their weapon in rounds, not in deathmatch.
    let drops = crate::weapon::drop::DeathDrops(settings.enabled != 0);
    if world.get_resource::<crate::weapon::drop::DeathDrops>() != Some(&drops) {
        world.insert_resource(drops);
    }
    if settings.enabled == 0 {
        if phase != Phase::Off {
            *world.resource_mut::<RoundState>() = RoundState::default();
            world.insert_resource(BuyWindow::default());
            world.insert_resource(FreezeTime(false));
            world.insert_resource(RoundOpen(true));
        }
        return;
    }
    // Everyone has money while rounds run.
    let broke: Vec<Entity> = world
        .query_filtered::<Entity, (With<Intent>, With<Health>, Without<Money>, Without<Hostage>)>()
        .iter(world)
        .collect();
    for e in broke {
        world.entity_mut(e).insert(Money(settings.start_money));
    }
    match phase {
        Phase::Off => {
            // A fresh game.
            *world.resource_mut::<RoundState>() = RoundState::default();
            let all: Vec<Entity> = world
                .query_filtered::<Entity, (With<Intent>, With<Health>, Without<Hostage>)>()
                .iter(world)
                .collect();
            for e in all {
                world.entity_mut(e).insert(Money(settings.start_money));
            }
            start_round(world, &settings, now, true);
        }
        Phase::Freeze { until } if now >= until => {
            world.resource_mut::<RoundState>().phase = Phase::Live {
                since: now,
                ends: now + settings.round_time as f64 * 60.0,
            };
        }
        Phase::Live { ends, .. } => {
            if let Some((winner, reason)) = live_outcome(world, now >= ends) {
                end_round(world, &settings, now, winner, reason);
                world.write_message(RoundEnded { winner, reason });
            }
        }
        Phase::Over { until, .. } if now >= until => start_round(world, &settings, now, false),
        _ => {}
    }
    // Buying: during the freeze and the buy period after it.
    let window = match world.resource::<RoundState>().phase {
        Phase::Freeze { .. } => Ok(()),
        Phase::Live { since, .. } if now - since <= settings.buy_time as f64 * 60.0 => Ok(()),
        // CS:S's words.
        _ => Err(format!(
            "{:.0} seconds have passed.\nYou can't buy anything now.",
            settings.buy_time * 60.0
        )),
    };
    if world.resource::<BuyWindow>().0 != window {
        world.insert_resource(BuyWindow(window));
    }
    let frozen = matches!(world.resource::<RoundState>().phase, Phase::Freeze { .. });
    if world.get_resource::<FreezeTime>().is_none_or(|f| f.0 != frozen) {
        world.insert_resource(FreezeTime(frozen));
    }
    // Objectives (planting) only while the round is live.
    let open = matches!(world.resource::<RoundState>().phase, Phase::Live { .. });
    if world.get_resource::<RoundOpen>().is_none_or(|o| o.0 != open) {
        world.insert_resource(RoundOpen(open));
    }
}

/// Whether the live round ends now, and how (spec 7's table, in order).
fn live_outcome(world: &mut World, time_up: bool) -> Option<(Option<Team>, RoundEndReason)> {
    let kind = world.get_resource::<MapObjectives>().map_or(MapKind::Neither, |o| o.kind());
    let bomb = world.get_resource::<BombState>().cloned().unwrap_or_default();
    let planted = bomb.planted.is_some() || bomb.outcome.is_some();
    match bomb.outcome {
        Some(BombOutcome::Exploded) => return Some((Some(ATTACKERS), RoundEndReason::TargetBombed)),
        Some(BombOutcome::Defused) => return Some((Some(DEFENDERS), RoundEndReason::BombDefused)),
        None => {}
    }
    let (alive, present) = team_counts(world);
    if present[0] > 0 && present[1] > 0 {
        match (alive[0], alive[1]) {
            (0, 0) if !planted => return Some((None, RoundEndReason::Draw)),
            (_, 0) => return Some((Some(ATTACKERS), RoundEndReason::Eliminated)),
            (0, _) if !planted => return Some((Some(DEFENDERS), RoundEndReason::Eliminated)),
            _ => {}
        }
    } else if present[0] + present[1] > 0 && alive == [0, 0] && !planted {
        // Players on one side only (a lone player; minigame maps with one
        // team's spawns: races, obstacle courses): all of them dead is a
        // draw and the next round, not a wait for the clock. Our reading
        // of CS:S (objectives.md question 15).
        return Some((None, RoundEndReason::Draw));
    }
    if kind == MapKind::Hostage && world.get_resource::<HostageTally>().is_some_and(|t| t.all_rescued()) {
        return Some((Some(DEFENDERS), RoundEndReason::HostagesRescued));
    }
    // After a plant the clock no longer ends the round.
    if !time_up || planted {
        return None;
    }
    Some(match kind {
        MapKind::Bomb => (Some(DEFENDERS), RoundEndReason::TargetSaved),
        MapKind::Hostage => (Some(ATTACKERS), RoundEndReason::HostagesNotRescued),
        MapKind::Neither if present[1] > 0 => (Some(DEFENDERS), RoundEndReason::TimeRanOut),
        MapKind::Neither => (None, RoundEndReason::Draw),
    })
}

/// Living and present characters per side.
fn team_counts(world: &mut World) -> ([u32; 2], [u32; 2]) {
    let (mut alive, mut present) = ([0; 2], [0; 2]);
    for (team, health, dead) in world
        .query_filtered::<(&Team, &Health, Has<Dead>), (Without<Hostage>, Without<crate::core::Connecting>)>()
        .iter(world)
    {
        if let Some(s) = side(*team) {
            present[s] += 1;
            if !dead && health.current > 0.0 {
                alive[s] += 1;
            }
        }
    }
    (alive, present)
}

fn end_round(world: &mut World, s: &RoundSettings, now: f64, winner: Option<Team>, reason: RoundEndReason) {
    let planted = world
        .get_resource::<BombState>()
        .is_some_and(|b| b.planted.is_some() || b.outcome.is_some());
    let mut state = world.resource_mut::<RoundState>();
    state.phase = Phase::Over {
        until: now + s.end_delay as f64,
        winner,
    };
    // Bonuses: the winners' fixed one, the losers' growing with the streak
    // (a draw pays both the loss bonus).
    let mut bonus = [0u32; 2];
    for (i, team) in [ATTACKERS, DEFENDERS].into_iter().enumerate() {
        if winner == Some(team) {
            state.wins[i] += 1;
            state.losses[i] = 0;
            bonus[i] = match reason {
                RoundEndReason::TargetBombed | RoundEndReason::BombDefused => s.win_bomb,
                RoundEndReason::TargetSaved => s.win_target_saved,
                RoundEndReason::HostagesRescued => s.win_rescued,
                RoundEndReason::HostagesNotRescued => s.win_not_rescued,
                _ => s.win_bonus,
            };
        } else {
            bonus[i] = (s.loss_bonus + s.loss_step * state.losses[i]).min(s.loss_max);
            state.losses[i] += 1;
            // A planted bomb pays the terrorists even when they lose.
            if team == ATTACKERS && planted {
                bonus[i] += s.plant_loss_bonus;
            }
        }
    }
    let payees: Vec<(Entity, usize)> = world
        .query_filtered::<(Entity, &Team), Without<Hostage>>()
        .iter(world)
        .filter_map(|(e, t)| side(*t).map(|i| (e, i)))
        .collect();
    for (e, i) in payees {
        add_money(world, e, bonus[i] as i64, s.max_money);
    }
}

fn start_round(world: &mut World, s: &RoundSettings, now: f64, fresh: bool) {
    {
        let mut state = world.resource_mut::<RoundState>();
        state.number += 1;
        state.phase = Phase::Freeze {
            until: now + s.freeze_time as f64,
        };
    }
    // The map's entities as they spawned (logic and map layers).
    world.get_resource_or_init::<crate::core::RoundRestarts>().0 += 1;
    // Everyone back at their spawns: the dead (or everyone, at the start
    // of a game) with fresh weapons, survivors with theirs.
    let all: Vec<(Entity, bool)> = world
        .query_filtered::<(Entity, Has<Dead>), (
            With<Intent>,
            With<Health>,
            Without<Hostage>,
            Without<crate::core::Connecting>,
        )>()
        .iter(world)
        .collect();
    for (e, dead) in all {
        put_at_spawn(world, e, fresh || dead);
    }
    // Computer players shop at once.
    let bots: Vec<Entity> = world
        .query_filtered::<Entity, (
            With<Intent>,
            With<Health>,
            Without<crate::core::LocalPlayer>,
            Without<crate::core::RemotePlayer>,
            Without<Hostage>,
        )>()
        .iter(world)
        .collect();
    world.insert_resource(BuyWindow::default());
    for e in bots {
        crate::weapon::economy::autobuy(world, e);
    }
    // Loose weapons from the last round go.
    let loose: Vec<(Entity, Entity)> = world
        .query::<(Entity, &crate::weapon::drop::Loose)>()
        .iter(world)
        .map(|(e, l)| (e, l.weapon))
        .collect();
    for (e, w) in loose {
        world.despawn(e);
        world.despawn(w);
    }
    // The bomb to a terrorist, hostages back.
    crate::objectives::round_start(world);
}

fn add_money(world: &mut World, e: Entity, delta: i64, max: u32) {
    if let Some(mut m) = world.get_mut::<Money>(e) {
        m.0 = (m.0 as i64 + delta).clamp(0, max as i64) as u32;
    }
}

/// Kill rewards (and the team-kill penalty).
pub(super) fn kill_rewards(
    settings: Res<RoundSettings>,
    mut died: MessageReader<Died>,
    teams: Query<&Team>,
    hostages: Query<(), With<Hostage>>,
    mut money: Query<&mut Money>,
) {
    for d in died.read() {
        // Hostages cost money instead (`objective_money`).
        if settings.enabled == 0 || hostages.contains(d.entity) {
            continue;
        }
        let Some(killer) = d.attacker.filter(|a| *a != d.entity) else {
            continue;
        };
        let same = matches!((teams.get(killer), teams.get(d.entity)), (Ok(a), Ok(b)) if a == b);
        if let Ok(mut m) = money.get_mut(killer) {
            m.0 = if same {
                m.0.saturating_sub(settings.teamkill_penalty)
            } else {
                (m.0 + settings.kill_reward).min(settings.max_money)
            };
        }
    }
}

/// While frozen, nobody moves or shoots (they can still look around).
/// Bots think before the rules (`bot::BotPlugin`), so this holds them too.
pub(super) fn hold_frozen(frozen: Res<FreezeTime>, mut intents: Query<&mut Intent>) {
    if !frozen.0 {
        return;
    }
    for mut intent in &mut intents {
        let (yaw, pitch, select) = (intent.yaw, intent.pitch, intent.select);
        *intent = Intent {
            yaw,
            pitch,
            select,
            ..default()
        };
    }
}

/// Money for objectives as they happen (spec 8): planting, defusing,
/// leading and rescuing hostages, killing them.
pub(super) fn objective_money(
    settings: Res<RoundSettings>,
    mut events: MessageReader<ObjectiveEvent>,
    mut money: Query<&mut Money>,
) {
    for ev in events.read() {
        if settings.enabled == 0 {
            continue;
        }
        let (who, delta) = match ev {
            ObjectiveEvent::Planted { who, .. } => (Some(*who), settings.planter_reward as i64),
            ObjectiveEvent::Defused { who } => (Some(*who), settings.defuser_reward as i64),
            ObjectiveEvent::HostageFollows { leader, first: true, .. } => (Some(*leader), settings.hostage_touch as i64),
            ObjectiveEvent::HostageRescued { leader, .. } => (*leader, settings.hostage_rescue as i64),
            ObjectiveEvent::HostageKilled { attacker, .. } => (*attacker, -(settings.hostage_kill_penalty as i64)),
            _ => (None, 0),
        };
        if delta != 0
            && let Some(mut m) = who.and_then(|w| money.get_mut(w).ok())
        {
            m.0 = (m.0 as i64 + delta).clamp(0, settings.max_money as i64) as u32;
        }
    }
}
