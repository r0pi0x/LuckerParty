//! The game's rules in a network game (docs/plans/active/multiplayer.md,
//! slice 5): everything here is the server's, and a client shows it.
//!
//! - **The round** (`NetRound`, one replicated entity): phase, its times,
//!   the score, whether buying is open, the bomb's outcome and the
//!   hostages' tally. A client writes them into the same resources single
//!   player has (`rules::rounds::RoundState`, `weapon::economy::BuyWindow`,
//!   `objectives::bomb::BombState`, `HostageTally`), times moved onto its
//!   own clock (`ServerTime`), so the HUD, the scoreboard and the
//!   spectator bars read them as they always did. A round's end comes as
//!   `RoundOver` (the banner and the announcer).
//! - **Scores** (`NetScore` on every character): kills, deaths, ping, a
//!   bot, the bomb and a defusal kit, for the scoreboard. A client's own
//!   money, armour, kit and arming come in its `OwnState`: money is shown
//!   only to its owner, as in CS:S.
//! - **Requests**: buying (`BuyRequest`) and joining a team
//!   (`TeamRequest`) are the server's to check (buy zone, buy time, money,
//!   team-only items, `mp_limitteams`); a refusal comes back as `Notice`,
//!   the game's hint.
//! - **Objectives**: the planted bomb (`NetBomb`: where, its timer, who
//!   defuses), hostages as characters (`NetHostage`), what happens as
//!   `ObjectiveNews` (the HUD's words) and the rules' sounds as
//!   `ServerSound` (`map::GameSound`: beeps, the plant, the defuse, the
//!   blast, hostages).

use bevy::prelude::*;
use bevy_replicon::prelude::*;

use super::{
    BuyRequest, NetBomb, NetCharacter, NetHostage, NetRound, NetScore, Notice, ObjectiveNews, OwnState, RoundOver,
    ServerSound, TeamRequest, bomb_outcome, interp::InterpClock, predict::CommandClock, round_phase, score_flags,
    server::Player,
};
use crate::{
    core::{LocalPlayer, NetRole, SimClock, Team},
    map::{
        BodyName, GameSound, PlaySound, PredictedSound,
        loose::{ShownItem, SitOnOrigin},
    },
    objectives::{
        ObjectiveEvent, RoundOpen,
        bomb::{BombOutcome, BombRules, BombState, C4, Defuse, PlantedBomb},
        hostages::{Hostage, HostagePenalty, HostageTally},
    },
    rules::{
        Score,
        rounds::{Phase, RoundEndReason, RoundEnded, RoundState},
    },
    weapon::{
        Armor, Inventory,
        economy::{BuyRequested, BuyWindow, DefuseKit, Money},
    },
};

pub(super) fn plugin(app: &mut App) {
    let server = || resource_equals(NetRole::Server);
    let client = || resource_equals(NetRole::Client);
    app.add_systems(
        FixedLast,
        (write_round, write_scores, write_bombs, write_hostages).run_if(server()),
    )
    .add_systems(
        PreUpdate,
        (receive_buys, receive_teams)
            .after(ServerSystems::Receive)
            .run_if(in_state(ServerState::Running)),
    )
    .add_systems(
        PostUpdate,
        (replicate_hostages, send_round_over, send_objectives, send_sounds)
            .before(ServerSystems::Send)
            .run_if(server()),
    )
    .add_systems(
        PreUpdate,
        (
            apply_round,
            apply_scores,
            apply_bombs,
            apply_hostages,
            apply_own,
            receive_round_over,
            receive_objectives,
            receive_sounds,
        )
            .chain()
            .after(ClientSystems::Receive)
            .run_if(client()),
    )
    .add_systems(
        PostUpdate,
        (send_buys, send_teams).before(ClientSystems::Send).run_if(client()),
    );
}

// --- Time

/// A server time (`SimClock::now`, s) as a server tick (fractional), by
/// the tick `clock` is at.
pub fn tick_of(clock: &SimClock, t: f64) -> f64 {
    clock.tick as f64 + (t - clock.now) / clock.delta.as_secs_f64().max(1e-9)
}

/// A client's view of the server's clock: the server tick it is at now
/// (as its updates arrive: the interpolation clock's render tick plus
/// the delay), the tick length, and this client's fixed clock now. A
/// server tick `t` is `fixed + (t - tick) × step` on the client's clock.
#[derive(Clone, Copy, Debug)]
pub struct ServerTime {
    pub tick: f64,
    pub step: f64,
    pub fixed: f64,
}

impl ServerTime {
    pub fn of(world: &World) -> Option<Self> {
        let interp = world.get_resource::<InterpClock>()?;
        if !interp.running {
            return None;
        }
        let fixed = world.resource::<Time<Fixed>>();
        let step = world
            .get_resource::<CommandClock>()
            .and_then(|c| c.step())
            .unwrap_or(fixed.timestep())
            .as_secs_f64()
            .max(1e-6);
        Some(Self {
            tick: interp.render_tick + interp.delay / step,
            step,
            fixed: fixed.elapsed_secs_f64(),
        })
    }

    /// Server tick `t` on this client's fixed clock (s).
    pub fn local(&self, t: f64) -> f64 {
        self.fixed + (t - self.tick) * self.step
    }
}

// --- Server

/// The round, the buy window, the bomb and the hostages into `NetRound`.
fn write_round(world: &mut World) {
    let clock = *world.resource::<SimClock>();
    let t = |s: f64| tick_of(&clock, s);
    let state = world.get_resource::<RoundState>().cloned().unwrap_or_default();
    let (phase, since, until, winner) = match state.phase {
        Phase::Off => (round_phase::OFF, 0.0, 0.0, None),
        Phase::Freeze { until } => (round_phase::FREEZE, 0.0, t(until), None),
        Phase::Live { since, ends } => (round_phase::LIVE, t(since), t(ends), None),
        Phase::Over { until, winner } => (round_phase::OVER, 0.0, t(until), winner.map(|w| w.0)),
    };
    let bomb = world.get_resource::<BombState>().cloned().unwrap_or_default();
    let tally = world.get_resource::<HostageTally>().copied().unwrap_or_default();
    let now = NetRound {
        phase,
        since,
        until,
        winner,
        number: state.number,
        wins: state.wins,
        buy_closed: world.get_resource::<BuyWindow>().and_then(|b| b.0.clone().err()),
        bomb: match (bomb.outcome, bomb.planted) {
            (Some(BombOutcome::Exploded), _) => bomb_outcome::EXPLODED,
            (Some(BombOutcome::Defused), _) => bomb_outcome::DEFUSED,
            (None, Some(_)) => bomb_outcome::PLANTED,
            (None, None) => bomb_outcome::NONE,
        },
        hostages: [tally.total, tally.rescued, tally.killed],
    };
    let have = world.query::<(Entity, &NetRound)>().iter(world).next().map(|(e, r)| (e, r.clone()));
    match have {
        Some((e, r)) => {
            if r != now {
                world.entity_mut(e).insert(now);
            }
        }
        None => {
            world.spawn((Name::new("Round"), Replicated, now));
        }
    }
}

/// Each character's scoreboard line. Ping is measured again every
/// `PING_TICKS` ticks (Source's scoreboard updates about once a second).
#[allow(clippy::type_complexity)]
fn write_scores(
    mut q: Query<
        (
            Entity,
            &NetCharacter,
            Option<&Score>,
            Option<&Inventory>,
            Has<DefuseKit>,
            Option<&mut NetScore>,
        ),
        Without<Hostage>,
    >,
    players: Query<(&Player, Option<&ConnectedClientStats>)>,
    bombs: Query<(), With<C4>>,
    clock: Res<SimClock>,
    mut commands: Commands,
) {
    const PING_TICKS: u64 = 64;
    let measure = clock.tick % PING_TICKS == 0;
    for (e, c, score, inv, kit, have) in &mut q {
        let s = score.copied().unwrap_or_default();
        let mut flags = 0;
        if c.owner.is_none() {
            flags |= score_flags::BOT;
        }
        if inv.is_some_and(|i| i.weapons.iter().any(|w| bombs.contains(*w))) {
            flags |= score_flags::BOMB;
        }
        if kit {
            flags |= score_flags::KIT;
        }
        let ping = if measure || have.is_none() {
            players
                .iter()
                .find(|(p, _)| p.character == e)
                .and_then(|(_, stats)| stats)
                .map_or(0, |s| (s.rtt * 1000.0).round().clamp(0.0, u16::MAX as f64) as u16)
        } else {
            have.as_ref().map_or(0, |h| h.ping)
        };
        let now = NetScore {
            kills: s.kills,
            deaths: s.deaths,
            ping,
            flags,
        };
        match have {
            Some(mut h) => {
                h.set_if_neq(now);
            }
            None => {
                commands.entity(e).insert(now);
            }
        }
    }
}

/// The planted bomb into `NetBomb` (replicated from the plant on).
#[allow(clippy::type_complexity)]
fn write_bombs(
    mut q: Query<(Entity, &PlantedBomb, &Transform, Option<&ShownItem>, Option<&mut NetBomb>)>,
    replicated: Query<(), With<Replicated>>,
    clock: Res<SimClock>,
    mut commands: Commands,
) {
    let t = |s: f64| tick_of(&clock, s);
    for (e, b, tf, model, have) in &mut q {
        let defuse = b.defuse.filter(|d| replicated.contains(d.who));
        let now = NetBomb {
            origin: tf.translation.to_array(),
            rotation: tf.rotation.to_array(),
            model: model.map(|m| m.0.clone()),
            site: b.site.map(|s| s as u32),
            planted: t(b.planted_at),
            explode: t(b.explode_at),
            timer: b.timer,
            defuser: defuse.map(|d| d.who),
            defuse_since: defuse.map_or(0.0, |d| t(d.started)),
            defuse_until: defuse.map_or(0.0, |d| t(d.ends)),
            defuse_kit: defuse.is_some_and(|d| d.kit),
            defused: b.defused,
        };
        match have {
            Some(mut h) => {
                h.set_if_neq(now);
            }
            None => {
                commands.entity(e).insert((Replicated, now));
            }
        }
    }
}

/// Whom each hostage follows.
fn write_hostages(mut q: Query<(&Hostage, &mut NetHostage)>, replicated: Query<(), With<Replicated>>) {
    for (h, mut n) in &mut q {
        let leader = h.leader.filter(|l| replicated.contains(*l));
        if n.leader != leader {
            n.leader = leader;
        }
    }
}

/// Hostages replicate as characters (`NetCharacter`, `NetBody`) with
/// `NetHostage`.
fn replicate_hostages(
    q: Query<(Entity, &Hostage, Option<&Name>, Option<&BodyName>), Without<Replicated>>,
    mut commands: Commands,
) {
    for (e, h, name, body) in &q {
        commands.entity(e).insert((
            Replicated,
            NetCharacter {
                owner: None,
                name: name.map_or_else(|| format!("Hostage {}", h.index + 1), |n| n.to_string()),
            },
            super::NetBody::default(),
            NetHostage {
                index: h.index as u32,
                model: body.map(|b| b.0.clone()),
                leader: None,
            },
        ));
    }
}

/// A client asked to buy something: the rules decide (`economy::buy`);
/// why not goes back as the game's hint.
fn receive_buys(mut requests: MessageReader<FromClient<BuyRequest>>, players: Query<&Player>, mut commands: Commands) {
    for r in requests.read() {
        let Some(client) = r.client_id.entity() else { continue };
        let Ok(p) = players.get(client) else { continue };
        let (character, what) = (p.character, r.message.what.chars().take(64).collect::<String>());
        commands.queue(move |w: &mut World| {
            if w.get_entity(character).is_err() {
                return;
            }
            if let Err(text) = crate::weapon::economy::buy(w, character, &what) {
                notify(w, client, text);
            }
        });
    }
}

/// A client asked for a team (`rules::join_team`).
fn receive_teams(mut requests: MessageReader<FromClient<TeamRequest>>, players: Query<&Player>, mut commands: Commands) {
    for r in requests.read() {
        let Some(client) = r.client_id.entity() else { continue };
        let Ok(p) = players.get(client) else { continue };
        let (character, team) = (p.character, r.message.team);
        commands.queue(move |w: &mut World| {
            if !(1..=2).contains(&team) || w.get_entity(character).is_err() {
                return;
            }
            if let Err(text) = crate::rules::join_team(w, character, Team(team)) {
                notify(w, client, text);
            }
        });
    }
}

/// The game's hint to one client.
pub(super) fn notify(world: &mut World, client: Entity, text: String) {
    world.write_message(ToClients {
        targets: SendTargets::Single(ClientId::Client(client)),
        message: Notice { text },
    });
}

/// Round ends, to every client.
fn send_round_over(mut ended: MessageReader<RoundEnded>, mut out: MessageWriter<ToClients<RoundOver>>) {
    for e in ended.read() {
        out.write(ToClients {
            targets: SendTargets::CLIENTS_ONLY,
            message: RoundOver {
                winner: e.winner.map(|t| t.0),
                reason: REASONS.iter().position(|r| *r == e.reason).unwrap_or(0) as u8,
            },
        });
    }
}

/// Objective events and hostage penalties, to every client (each client
/// shows what concerns it, as single player does for the local player).
fn send_objectives(
    mut events: MessageReader<ObjectiveEvent>,
    mut penalties: MessageReader<HostagePenalty>,
    replicated: Query<(), With<Replicated>>,
    mut out: MessageWriter<ToClients<ObjectiveNews>>,
) {
    let known = |e: Entity| replicated.contains(e).then_some(e);
    let news = events
        .read()
        .filter_map(|e| encode_objective(e, &known))
        .chain(penalties.read().filter_map(|p| {
            let (kind, who) = match p {
                HostagePenalty::Warning { who } => (PENALTY_WARNING, who),
                HostagePenalty::Removed { who } => (PENALTY_REMOVED, who),
            };
            Some(ObjectiveNews {
                kind,
                who: Some(known(*who)?),
                ..default()
            })
        }))
        .collect::<Vec<_>>();
    for message in news {
        out.write(ToClients {
            targets: SendTargets::CLIENTS_ONLY,
            message,
        });
    }
}

/// The rules' sounds, to every client; a sound of a player's predicted
/// action (`PredictedSound`) to every client but that player's, which
/// played it already.
fn send_sounds(
    mut sounds: MessageReader<GameSound>,
    mut predicted: MessageReader<PredictedSound>,
    replicated: Query<(), With<Replicated>>,
    players: Query<(Entity, &Player)>,
    mut out: MessageWriter<ToClients<ServerSound>>,
) {
    let all = sounds.read().map(|s| (&s.0, None));
    let own = predicted.read().map(|s| (&s.sound, Some(s.by)));
    for (s, by) in all.chain(own) {
        let client = by.and_then(|by| players.iter().find(|(_, p)| p.character == by).map(|(c, _)| c));
        out.write(ToClients {
            targets: client.map_or(SendTargets::CLIENTS_ONLY, |c| SendTargets::AllExcept(ClientId::Client(c))),
            message: ServerSound {
                entry: s.entry.clone(),
                at: s.at.map(|a| a.to_array()),
                volume: s.volume,
                pitch: s.pitch,
                source: s.source.filter(|e| replicated.contains(*e)),
                channel: s.channel,
            },
        });
    }
}

// --- Encoding

const REASONS: [RoundEndReason; 8] = [
    RoundEndReason::Eliminated,
    RoundEndReason::TimeRanOut,
    RoundEndReason::Draw,
    RoundEndReason::TargetBombed,
    RoundEndReason::BombDefused,
    RoundEndReason::TargetSaved,
    RoundEndReason::HostagesRescued,
    RoundEndReason::HostagesNotRescued,
];

/// Why planting or defusing was refused: the game's words
/// (`objectives::bomb`), by number.
const REFUSALS: [&str; 4] = [
    "C4 must be planted at a bomb site.",
    "You must be standing on the ground to plant the C4.",
    "The bomb is already being defused.",
    "You must be on the ground to defuse the bomb.",
];

const PENALTY_WARNING: u8 = 100;
const PENALTY_REMOVED: u8 = 101;

fn reason_number(r: &str) -> u8 {
    REFUSALS.iter().position(|x| *x == r).unwrap_or(0) as u8
}

/// An objective event as news, with only entities clients know.
fn encode_objective(e: &ObjectiveEvent, known: &dyn Fn(Entity) -> Option<Entity>) -> Option<ObjectiveNews> {
    use ObjectiveEvent as E;
    let n = |kind: u8, who: Entity| -> Option<ObjectiveNews> {
        Some(ObjectiveNews {
            kind,
            who: Some(known(who)?),
            ..default()
        })
    };
    Some(match e {
        E::GotBomb { who } => n(0, *who)?,
        E::DroppedBomb { who } => n(1, *who)?,
        E::PickedUpBomb { who } => n(2, *who)?,
        E::BeginPlant { who } => n(3, *who)?,
        E::AbortPlant { who, left_zone } => ObjectiveNews {
            flag: *left_zone,
            ..n(4, *who)?
        },
        E::PlantRefused { who, reason } => ObjectiveNews {
            reason: reason_number(reason),
            ..n(5, *who)?
        },
        E::Planted { who, at, site } => ObjectiveNews {
            kind: 6,
            who: known(*who),
            at: at.to_array(),
            site: site.map(|s| s as u32),
            ..default()
        },
        E::BeginDefuse { who, kit } => ObjectiveNews { flag: *kit, ..n(7, *who)? },
        E::AbortDefuse { who } => n(8, *who)?,
        E::DefuseRefused { who, reason } => ObjectiveNews {
            reason: reason_number(reason),
            ..n(9, *who)?
        },
        E::Defused { who } => ObjectiveNews {
            kind: 10,
            who: known(*who),
            ..default()
        },
        E::Exploded { at } => ObjectiveNews {
            kind: 11,
            at: at.to_array(),
            ..default()
        },
        E::Beep { at } => ObjectiveNews {
            kind: 12,
            at: at.to_array(),
            ..default()
        },
        E::PickedUpKit { who } => n(13, *who)?,
        E::HostageFollows { hostage, leader, first } => ObjectiveNews {
            other: Some(known(*leader)?),
            flag: *first,
            ..n(14, *hostage)?
        },
        E::HostageStops { hostage, leader } => ObjectiveNews {
            other: Some(known(*leader)?),
            ..n(15, *hostage)?
        },
        E::HostageRefused { who } => n(16, *who)?,
        E::HostageRescued { hostage, leader } => ObjectiveNews {
            other: leader.and_then(known),
            ..n(17, *hostage)?
        },
        E::HostageHurt {
            hostage,
            attacker,
            amount,
        } => ObjectiveNews {
            other: attacker.and_then(known),
            amount: *amount,
            ..n(18, *hostage)?
        },
        E::HostageKilled { hostage, attacker } => ObjectiveNews {
            other: attacker.and_then(known),
            ..n(19, *hostage)?
        },
    })
}

/// News back into the event single player has (an entity a client
/// couldn't map is `Entity::PLACEHOLDER`, never anyone's).
enum Decoded {
    Event(ObjectiveEvent),
    Penalty(HostagePenalty),
}

fn decode_objective(n: &ObjectiveNews) -> Option<Decoded> {
    use ObjectiveEvent as E;
    let who = n.who.unwrap_or(Entity::PLACEHOLDER);
    let at = Vec3::from_array(n.at);
    let reason = REFUSALS[(n.reason as usize).min(REFUSALS.len() - 1)];
    Some(Decoded::Event(match n.kind {
        0 => E::GotBomb { who },
        1 => E::DroppedBomb { who },
        2 => E::PickedUpBomb { who },
        3 => E::BeginPlant { who },
        4 => E::AbortPlant {
            who,
            left_zone: n.flag,
        },
        5 => E::PlantRefused { who, reason },
        6 => E::Planted {
            who,
            at,
            site: n.site.map(|s| s as usize),
        },
        7 => E::BeginDefuse { who, kit: n.flag },
        8 => E::AbortDefuse { who },
        9 => E::DefuseRefused { who, reason },
        10 => E::Defused { who },
        11 => E::Exploded { at },
        12 => E::Beep { at },
        13 => E::PickedUpKit { who },
        14 => E::HostageFollows {
            hostage: who,
            leader: n.other?,
            first: n.flag,
        },
        15 => E::HostageStops {
            hostage: who,
            leader: n.other?,
        },
        16 => E::HostageRefused { who },
        17 => E::HostageRescued {
            hostage: who,
            leader: n.other,
        },
        18 => E::HostageHurt {
            hostage: who,
            attacker: n.other,
            amount: n.amount,
        },
        19 => E::HostageKilled {
            hostage: who,
            attacker: n.other,
        },
        PENALTY_WARNING => return Some(Decoded::Penalty(HostagePenalty::Warning { who })),
        PENALTY_REMOVED => return Some(Decoded::Penalty(HostagePenalty::Removed { who })),
        _ => return None,
    }))
}

// --- Client

/// The server's round into the resources the HUD reads.
fn apply_round(world: &mut World) {
    let Some(r) = world.query::<&NetRound>().iter(world).next().cloned() else {
        return;
    };
    let Some(time) = ServerTime::of(world) else { return };
    let t = |tick: f64| time.local(tick);
    let phase = match r.phase {
        round_phase::FREEZE => Phase::Freeze { until: t(r.until) },
        round_phase::LIVE => Phase::Live {
            since: t(r.since),
            ends: t(r.until),
        },
        round_phase::OVER => Phase::Over {
            until: t(r.until),
            winner: r.winner.map(Team),
        },
        _ => Phase::Off,
    };
    {
        let mut state = world.get_resource_or_init::<RoundState>();
        state.phase = phase;
        if state.number != r.number {
            state.number = r.number;
        }
        if state.wins != r.wins {
            state.wins = r.wins;
        }
    }
    let window = BuyWindow(r.buy_closed.clone().map_or(Ok(()), Err));
    if world.get_resource::<BuyWindow>().is_none_or(|w| w.0 != window.0) {
        world.insert_resource(window);
    }
    let frozen = r.phase == round_phase::FREEZE;
    if world.get_resource::<crate::core::FreezeTime>().is_none_or(|f| f.0 != frozen) {
        world.insert_resource(crate::core::FreezeTime(frozen));
    }
    let open = r.phase == round_phase::LIVE || r.phase == round_phase::OFF;
    if world.get_resource::<RoundOpen>().is_none_or(|o| o.0 != open) {
        world.insert_resource(RoundOpen(open));
    }
    let planted = world.query_filtered::<Entity, With<NetBomb>>().iter(world).next();
    let bomb = BombState {
        planted: planted.filter(|_| r.bomb == bomb_outcome::PLANTED),
        planter: None,
        outcome: match r.bomb {
            bomb_outcome::EXPLODED => Some(BombOutcome::Exploded),
            bomb_outcome::DEFUSED => Some(BombOutcome::Defused),
            _ => None,
        },
    };
    if world.get_resource::<BombState>() != Some(&bomb) {
        world.insert_resource(bomb);
    }
    let tally = HostageTally {
        total: r.hostages[0],
        rescued: r.hostages[1],
        killed: r.hostages[2],
    };
    if world.get_resource::<HostageTally>() != Some(&tally) {
        world.insert_resource(tally);
    }
}

/// Scores and kits into `Score` and `DefuseKit` (the scoreboard and HUD
/// read them as in single player).
fn apply_scores(q: Query<(Entity, Ref<NetScore>, Option<&Score>, Has<DefuseKit>)>, mut commands: Commands) {
    for (e, s, score, kit) in &q {
        if !s.is_changed() && score.is_some() {
            continue;
        }
        if score.is_none_or(|x| x.kills != s.kills || x.deaths != s.deaths) {
            commands.entity(e).insert(Score {
                kills: s.kills,
                deaths: s.deaths,
            });
        }
        let has = s.flags & score_flags::KIT != 0;
        if has && !kit {
            commands.entity(e).insert(DefuseKit);
        } else if !has && kit {
            commands.entity(e).remove::<DefuseKit>();
        }
    }
}

/// The planted bomb: drawn with its model where the server put it, its
/// clock on ours.
fn apply_bombs(world: &mut World) {
    let Some(time) = ServerTime::of(world) else { return };
    let turn = world.get_resource::<BombRules>().map_or(Quat::IDENTITY, |r| r.planted_turn);
    let bombs: Vec<(Entity, NetBomb, bool)> = world
        .query::<(Entity, &NetBomb, Has<PlantedBomb>)>()
        .iter(world)
        .map(|(e, b, p)| (e, b.clone(), p))
        .collect();
    for (e, b, had) in bombs {
        let t = |tick: f64| time.local(tick);
        let defuse = b
            .defuser
            .filter(|d| *d != Entity::PLACEHOLDER && world.get_entity(*d).is_ok())
            .map(|who| Defuse {
                who,
                started: t(b.defuse_since),
                ends: t(b.defuse_until),
                kit: b.defuse_kit,
            });
        let planted = PlantedBomb {
            planter: None,
            site: b.site.map(|s| s as usize),
            planted_at: t(b.planted),
            explode_at: t(b.explode),
            timer: b.timer,
            next_beep: 0.0,
            defuse,
            defused: b.defused,
        };
        let pose =
            Transform::from_translation(Vec3::from_array(b.origin)).with_rotation(Quat::from_array(b.rotation).normalize());
        let mut ent = world.entity_mut(e);
        ent.insert(planted);
        if !had {
            ent.insert((pose, crate::map::interp::NetDrawn));
            if let Some(m) = &b.model {
                ent.insert((ShownItem(m.clone()), SitOnOrigin(turn)));
            }
        } else if ent.get::<Transform>().is_some_and(|t| t.translation != pose.translation) {
            ent.insert(pose);
        }
    }
}

/// Hostages: characters without a team, with their model and leader.
#[allow(clippy::type_complexity)]
fn apply_hostages(
    teams: Query<Entity, (With<NetHostage>, With<Team>)>,
    mut q: Query<(Entity, &NetHostage, Option<&mut Hostage>)>,
    mut commands: Commands,
) {
    for e in &teams {
        commands.entity(e).remove::<Team>();
    }
    for (e, n, h) in &mut q {
        let leader = n.leader.filter(|l| *l != Entity::PLACEHOLDER);
        match h {
            Some(mut h) => {
                if h.leader != leader {
                    h.leader = leader;
                }
            }
            None => {
                let mut ent = commands.entity(e);
                ent.insert(Hostage::shown(n.index as usize, leader));
                if let Some(m) = &n.model {
                    ent.insert(BodyName(m.clone()));
                }
            }
        }
    }
}

/// Our money and armour from the server's newest state (arming and
/// defusing are predicted: `objectives::bomb::arm`).
fn apply_own(
    mut states: MessageReader<OwnState>,
    player: Option<Single<(Entity, Option<&Money>, Option<&Armor>), With<LocalPlayer>>>,
    mut commands: Commands,
) {
    let Some(s) = states.read().max_by_key(|s| s.tick).cloned() else {
        return;
    };
    let Some((e, money, armor)) = player.map(|p| *p) else {
        return;
    };
    match s.money {
        Some(m) if money.map(|x| x.0) != Some(m) => {
            commands.entity(e).insert(Money(m));
        }
        None if money.is_some() => {
            commands.entity(e).remove::<Money>();
        }
        _ => {}
    }
    match s.armor {
        Some((amount, helmet)) if armor.is_none_or(|a| a.amount != amount || a.helmet != helmet) => {
            commands.entity(e).insert(Armor { amount, helmet });
        }
        None if armor.is_some() => {
            commands.entity(e).remove::<Armor>();
        }
        _ => {}
    }
}

/// The round's end: the banner and the announcer (`RoundEnded`).
fn receive_round_over(mut over: MessageReader<RoundOver>, mut ended: MessageWriter<RoundEnded>) {
    for o in over.read() {
        ended.write(RoundEnded {
            winner: o.winner.map(Team),
            reason: REASONS[(o.reason as usize).min(REASONS.len() - 1)],
        });
    }
}

/// Objective news back into events (the HUD's words, the bomb's LED).
fn receive_objectives(
    mut news: MessageReader<ObjectiveNews>,
    mut events: MessageWriter<ObjectiveEvent>,
    mut penalties: MessageWriter<HostagePenalty>,
) {
    for n in news.read() {
        match decode_objective(n) {
            Some(Decoded::Event(e)) => {
                events.write(e);
            }
            Some(Decoded::Penalty(p)) => {
                penalties.write(p);
            }
            None => {}
        }
    }
}

/// The rules' sounds the server played.
fn receive_sounds(mut sounds: MessageReader<ServerSound>, mut play: MessageWriter<PlaySound>) {
    for s in sounds.read() {
        play.write(PlaySound {
            entry: s.entry.clone(),
            at: s.at.map(Vec3::from_array),
            volume: s.volume,
            pitch: s.pitch,
            source: s.source.filter(|e| *e != Entity::PLACEHOLDER),
            channel: s.channel,
        });
    }
}

/// Our buys, to the server.
fn send_buys(mut asked: MessageReader<BuyRequested>, mut out: MessageWriter<BuyRequest>) {
    for a in asked.read() {
        out.write(BuyRequest { what: a.0.clone() });
    }
}

/// Our team changes, to the server.
fn send_teams(mut asked: MessageReader<crate::rules::TeamRequested>, mut out: MessageWriter<TeamRequest>) {
    for a in asked.read() {
        out.write(TeamRequest { team: a.0.0 });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn objective_news_round_trips() {
        let a = Entity::from_raw_u32(5).unwrap();
        let b = Entity::from_raw_u32(9).unwrap();
        let known = |e: Entity| Some(e);
        let events = [
            ObjectiveEvent::PlantRefused {
                who: a,
                reason: REFUSALS[1],
            },
            ObjectiveEvent::Planted {
                who: a,
                at: Vec3::new(1.0, 2.0, 3.0),
                site: Some(1),
            },
            ObjectiveEvent::BeginDefuse { who: b, kit: true },
            ObjectiveEvent::HostageFollows {
                hostage: a,
                leader: b,
                first: true,
            },
            ObjectiveEvent::HostageKilled {
                hostage: a,
                attacker: None,
            },
            ObjectiveEvent::Beep { at: Vec3::X },
        ];
        for e in events {
            let n = encode_objective(&e, &known).unwrap();
            match decode_objective(&n) {
                Some(Decoded::Event(back)) => assert_eq!(back, e),
                _ => panic!("{e:?} didn't come back"),
            }
        }
        // An entity clients don't know: the event isn't sent.
        let none = |_: Entity| None;
        assert!(encode_objective(&ObjectiveEvent::GotBomb { who: a }, &none).is_none());
    }

    #[test]
    fn server_times_become_ticks() {
        let clock = SimClock {
            tick: 100,
            now: 1.5,
            delta: std::time::Duration::from_secs_f64(0.015),
        };
        assert!((tick_of(&clock, 1.5 + 0.15) - 110.0).abs() < 1e-9);
        let t = ServerTime {
            tick: 105.0,
            step: 0.015,
            fixed: 20.0,
        };
        assert!((t.local(110.0) - 20.075).abs() < 1e-9);
    }
}
