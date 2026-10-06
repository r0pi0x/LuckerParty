//! Team rounds with money, the Counter-Strike way (`mashup_rounds 1`):
//! a freeze period, a timed round won by eliminating the other team (or
//! by the defenders when time runs out), a pause, then everyone back at
//! their spawns; survivors keep their weapons. Money: a start amount, kill
//! rewards, a win bonus and a loss bonus that grows with a losing streak;
//! buying is open during the freeze and the buy period.
//!
//! Defaults are CS:S's public server defaults (mp_freezetime,
//! mp_roundtime, mp_buytime, mp_startmoney) and its well-known reward
//! amounts; none measured here yet (docs/plans/active/rounds.md).

use bevy::prelude::*;

use super::{Dead, put_at_spawn};
use crate::{
    console::resource_cvar,
    core::{Died, Health, Intent, Team},
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
}

impl Default for RoundState {
    fn default() -> Self {
        Self {
            phase: Phase::Off,
            number: 0,
            wins: [0; 2],
            losses: [0; 2],
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
        ("mp_freezetime", "Seconds frozen at the start of each round.", |r| &mut r.freeze_time),
        ("mp_roundtime", "Minutes per round.", |r| &mut r.round_time),
        ("mp_buytime", "Minutes buying stays open after the freeze.", |r| &mut r.buy_time),
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
}

/// Advance the round: start, freeze, play, decide, pause, restart.
pub(super) fn run_rounds(world: &mut World) {
    let settings = world.resource::<RoundSettings>().clone();
    let now = world.resource::<Time>().elapsed_secs_f64();
    let phase = world.resource::<RoundState>().phase;
    if settings.enabled == 0 {
        if phase != Phase::Off {
            *world.resource_mut::<RoundState>() = RoundState::default();
            world.insert_resource(BuyWindow::default());
        }
        return;
    }
    // Everyone has money while rounds run.
    let broke: Vec<Entity> = world
        .query_filtered::<Entity, (With<Intent>, With<Health>, Without<Money>)>()
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
                .query_filtered::<Entity, (With<Intent>, With<Health>)>()
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
            let (alive, present) = team_counts(world);
            let outcome = if present[0] > 0 && present[1] > 0 && (alive[0] == 0 || alive[1] == 0) {
                match (alive[0], alive[1]) {
                    (0, 0) => Some((None, RoundEndReason::Draw)),
                    (0, _) => Some((Some(DEFENDERS), RoundEndReason::Eliminated)),
                    _ => Some((Some(ATTACKERS), RoundEndReason::Eliminated)),
                }
            } else if now >= ends {
                Some(if present[1] > 0 {
                    (Some(DEFENDERS), RoundEndReason::TimeRanOut)
                } else {
                    (None, RoundEndReason::Draw)
                })
            } else {
                None
            };
            if let Some((winner, reason)) = outcome {
                end_round(world, &settings, now, winner);
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
        _ => Err(format!(
            "the {:.0} second buy period has expired",
            settings.buy_time * 60.0
        )),
    };
    if world.resource::<BuyWindow>().0 != window {
        world.insert_resource(BuyWindow(window));
    }
}

/// Living and present characters per side.
fn team_counts(world: &mut World) -> ([u32; 2], [u32; 2]) {
    let (mut alive, mut present) = ([0; 2], [0; 2]);
    for (team, health, dead) in world
        .query::<(&Team, &Health, Has<Dead>)>()
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

fn end_round(world: &mut World, s: &RoundSettings, now: f64, winner: Option<Team>) {
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
            bonus[i] = s.win_bonus;
        } else {
            bonus[i] = (s.loss_bonus + s.loss_step * state.losses[i]).min(s.loss_max);
            state.losses[i] += 1;
        }
    }
    let payees: Vec<(Entity, usize)> = world
        .query::<(Entity, &Team)>()
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
    // Everyone back at their spawns: the dead (or everyone, at the start
    // of a game) with fresh weapons, survivors with theirs.
    let all: Vec<(Entity, bool)> = world
        .query_filtered::<(Entity, Has<Dead>), (With<Intent>, With<Health>)>()
        .iter(world)
        .collect();
    for (e, dead) in all {
        put_at_spawn(world, e, fresh || dead);
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
    mut money: Query<&mut Money>,
) {
    for d in died.read() {
        if settings.enabled == 0 {
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
pub(super) fn hold_frozen(state: Res<RoundState>, mut intents: Query<&mut Intent>) {
    if !matches!(state.phase, Phase::Freeze { .. }) {
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
