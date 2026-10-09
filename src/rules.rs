//! Match rules. For now one ruleset, deathmatch: the dead stop acting,
//! then respawn at a spawn point with full health and fresh starting
//! weapons; kills and deaths are counted.

pub mod rounds;

use avian3d::prelude::*;
use bevy::prelude::*;

use crate::{
    console::{ConsoleAppExt, resource_cvar},
    core::{Died, Health, Intent, SimSet, SpawnPoint, Team, Velocity},
    weapon::{Inventory, StartingWeapons, give},
};

pub struct DeathmatchPlugin;

impl Plugin for DeathmatchPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Deathmatch>().add_message::<Died>().add_systems(
            FixedUpdate,
            (
                (rounds::run_rounds, respawn, hold_the_dead, rounds::hold_frozen)
                    .chain()
                    .in_set(SimSet::Rules),
                (count_deaths, rounds::kill_rewards, rounds::objective_money, apply_score_changes).after(SimSet::Weapons),
            )
                // The rules are the server's.
                .run_if(crate::core::authoritative),
        );
        rounds::plugin(app);
        resource_cvar::<Deathmatch, f32>(app, "mp_respawn_delay", "Seconds before the dead respawn.", |d| {
            &mut d.respawn_delay
        });
        app.init_resource::<crate::core::FriendlyFire>();
        resource_cvar::<crate::core::FriendlyFire, u8>(app, "mp_friendlyfire", "1: teammates hurt each other.", |f| {
            &mut f.0
        });
        resource_cvar::<Deathmatch, u32>(
            app,
            "mp_limitteams",
            "In a network game, most players one team may have over the other (0: no limit).",
            |d| &mut d.limit_teams,
        );
        app.add_message::<TeamRequested>().console_command(
            "jointeam",
            "jointeam <2|3>: join the terrorists (2) or counter-terrorists (3) and respawn at their spawn.",
            |w, a| {
                let team = match a.first().map(String::as_str) {
                    Some("2") => Team(1),
                    Some("3") => Team(2),
                    _ => return Err("jointeam <2|3> (2: terrorists, 3: counter-terrorists)".into()),
                };
                // A network client asks the server (`net`).
                if w.get_resource::<crate::core::NetRole>() == Some(&crate::core::NetRole::Client) {
                    w.write_message(TeamRequested(team));
                    return Ok(None);
                }
                let player = w
                    .query_filtered::<Entity, With<crate::core::LocalPlayer>>()
                    .iter(w)
                    .next()
                    .ok_or("no local player")?;
                join_team(w, player, team).map(Some)
            },
        );
    }
}

/// A network client's `jointeam`: the team it asks the server for.
#[derive(Message, Clone, Copy, Debug, PartialEq)]
pub struct TeamRequested(pub Team);

/// Put `player` on `team`: it respawns at the new team's spawn (with
/// rounds, at the next round; no death counted). In a network game
/// (`NetRole::Server`) a team may not get more than `mp_limitteams`
/// players ahead of the other.
pub fn join_team(w: &mut World, player: Entity, team: Team) -> Result<String, String> {
    if w.get::<Team>(player) == Some(&team) {
        return Ok("already on that team".into());
    }
    let limit = w.resource::<Deathmatch>().limit_teams;
    if limit > 0 && w.get_resource::<crate::core::NetRole>() == Some(&crate::core::NetRole::Server) {
        let mut counts = [0u32; 3];
        for (e, t) in w
            .query_filtered::<(Entity, &Team), (With<Intent>, Without<crate::objectives::hostages::Hostage>)>()
            .iter(w)
        {
            if e != player && (1..=2).contains(&t.0) {
                counts[t.0 as usize] += 1;
            }
        }
        let other = if team.0 == 1 { 2 } else { 1 };
        if counts[team.0 as usize] + 1 > counts[other] + limit {
            return Err(format!(
                "Too many {} (mp_limitteams {limit}): join the other team.",
                if team.0 == 1 { "Terrorists" } else { "Counter-Terrorists" }
            ));
        }
    }
    // Switching teams: respawn at the new team's spawn now (no death
    // counted).
    w.entity_mut(player).insert((team, Dead { since: f64::MIN }));
    Ok(format!(
        "joined the {}",
        if team.0 == 1 {
            "terrorists"
        } else {
            "counter-terrorists"
        }
    ))
}

#[derive(Resource, Clone, Debug)]
pub struct Deathmatch {
    pub respawn_delay: f32,
    /// `mp_limitteams` (network games only).
    pub limit_teams: u32,
    /// Spawn points picked so far (round robin, so players spread out).
    next_spawn: usize,
}

impl Default for Deathmatch {
    fn default() -> Self {
        Self {
            respawn_delay: 2.0,
            // CS:S's default.
            limit_teams: 2,
            next_spawn: 0,
        }
    }
}

/// Kills and deaths.
#[derive(Component, Default, Clone, Copy, Debug, Reflect)]
#[reflect(Component)]
pub struct Score {
    pub kills: i32,
    pub deaths: u32,
}

/// Dead since this time (fixed-tick seconds).
#[derive(Component, Clone, Copy, Debug)]
pub struct Dead {
    pub since: f64,
}

/// Respawn every character on the next tick (after a map change), with
/// fresh weapons, at the spawn points.
/// A fresh game on a newly loaded map, as a map change is in CS:S: with
/// rounds, the next step starts a new game (everyone at a spawn, start
/// money); without, everyone respawns now. Without this the dead stayed
/// dead (spectating) until the old round's clock ran out on the new map.
pub fn new_game(world: &mut World) {
    if let Some(mut state) = world.get_resource_mut::<rounds::RoundState>() {
        *state = rounds::RoundState::default();
    }
    respawn_everyone(world);
}

pub fn respawn_everyone(world: &mut World) {
    let all: Vec<Entity> = world
        .query_filtered::<Entity, (With<Intent>, With<Health>, Without<crate::objectives::hostages::Hostage>)>()
        .iter(world)
        .collect();
    for e in all {
        world.entity_mut(e).insert(Dead { since: f64::MIN });
    }
}

/// Score changes from map logic (game_score; specs/source/
/// game_entities.md 4): a player's kills (a negative change without
/// "Allow Negative" stops at 0 and does nothing below it), or its team's
/// rounds won (our reading of "team score", spec open question 8; ours
/// can't go below 0).
fn apply_score_changes(
    mut changes: MessageReader<crate::core::ScoreChange>,
    mut scores: Query<&mut Score>,
    teams: Query<&Team>,
    rounds: Option<ResMut<rounds::RoundState>>,
    mut commands: Commands,
) {
    let mut rounds = rounds;
    for c in changes.read() {
        if c.team {
            let Some(r) = rounds.as_mut() else { continue };
            let Some(i) = teams.get(c.target).ok().and_then(|t| (t.0 as usize).checked_sub(1)).filter(|i| *i < 2) else {
                continue;
            };
            r.wins[i] = (r.wins[i] as i64 + c.points as i64).max(0) as u32;
            continue;
        }
        let kills = scores.get(c.target).map_or(0, |s| s.kills);
        let new = if c.points >= 0 || c.allow_negative {
            kills + c.points
        } else if kills < 0 {
            kills
        } else {
            (kills + c.points).max(0)
        };
        if let Ok(mut s) = scores.get_mut(c.target) {
            s.kills = new;
        } else if let Ok(mut e) = commands.get_entity(c.target) {
            e.insert(Score { kills: new, deaths: 0 });
        }
    }
}

fn count_deaths(mut died: MessageReader<Died>, mut scores: Query<&mut Score>, mut commands: Commands, time: Res<Time>) {
    let now = time.elapsed_secs_f64();
    for d in died.read() {
        if let Ok(mut s) = scores.get_mut(d.entity) {
            s.deaths += 1;
        } else if let Ok(mut e) = commands.get_entity(d.entity) {
            e.insert(Score { kills: 0, deaths: 1 });
        }
        if let Some(a) = d.attacker.filter(|a| *a != d.entity) {
            if let Ok(mut s) = scores.get_mut(a) {
                s.kills += 1;
            } else if let Ok(mut e) = commands.get_entity(a) {
                e.insert(Score { kills: 1, deaths: 0 });
            }
        }
        // The dead body stops being solid (a ragdoll takes its place), so
        // it neither blocks the living nor holds them up.
        if let Ok(mut e) = commands.get_entity(d.entity) {
            e.insert((Dead { since: now }, ColliderDisabled));
        }
    }
}

/// The dead neither move nor shoot (their intent is cleared each tick).
fn hold_the_dead(mut dead: Query<&mut Intent, With<Dead>>) {
    for mut intent in &mut dead {
        let (yaw, pitch) = (intent.yaw, intent.pitch);
        *intent = Intent {
            yaw,
            pitch,
            ..default()
        };
    }
}

fn respawn(world: &mut World) {
    // Rounds respawn everyone at the round's start instead.
    if world
        .get_resource::<rounds::RoundSettings>()
        .is_some_and(|r| r.enabled != 0)
    {
        return;
    }
    let now = world.resource::<Time>().elapsed_secs_f64();
    let delay = world.resource::<Deathmatch>().respawn_delay as f64;
    let ready: Vec<Entity> = world
        .query_filtered::<(Entity, &Dead), Without<crate::objectives::hostages::Hostage>>()
        .iter(world)
        .filter(|(_, d)| now - d.since >= delay)
        .map(|(e, _)| e)
        .collect();
    for e in ready {
        put_at_spawn(world, e, true);
    }
}

/// Bring a character back at one of its team's spawn points (round robin)
/// with full health and no velocity; with `fresh` weapons its old ones go
/// and it gets the starting weapons, otherwise it keeps what it carries.
pub(crate) fn put_at_spawn(world: &mut World, e: Entity, fresh: bool) {
    let spawns: Vec<(Transform, Option<Team>)> = world
        .query::<(&Transform, &SpawnPoint)>()
        .iter(world)
        .map(|(t, s)| (*t, s.team))
        .collect();
    let starting = world.resource::<StartingWeapons>().clone();
    {
        // The character's own team's spawns, else any.
        let team = world.get::<Team>(e).copied();
        let own: Vec<Transform> = spawns
            .iter()
            .filter(|(_, t)| t.is_some() && *t == team)
            .map(|(t, _)| *t)
            .collect();
        let pool: Vec<Transform> = if own.is_empty() {
            spawns.iter().map(|(t, _)| *t).collect()
        } else {
            own
        };
        let at = if pool.is_empty() {
            None
        } else {
            let mut dm = world.resource_mut::<Deathmatch>();
            let i = dm.next_spawn % pool.len();
            dm.next_spawn += 1;
            Some(pool[i])
        };
        // Fresh weapons: drop the old ones.
        if fresh {
            let old = world.get::<Inventory>(e).map(|i| i.weapons.clone()).unwrap_or_default();
            for w in old {
                world.despawn(w);
            }
        }
        let Ok(mut ent) = world.get_entity_mut(e) else { return };
        ent.remove::<(Dead, ColliderDisabled)>();
        if fresh {
            ent.insert(Inventory::default());
        }
        if let Some(mut h) = ent.get_mut::<Health>() {
            h.current = h.max;
        }
        if let Some(mut v) = ent.get_mut::<Velocity>() {
            v.0 = Vec3::ZERO;
        }
        if let Some(at) = at {
            // Spawn points are already lifted to a character's centre.
            if let Some(mut t) = ent.get_mut::<Transform>() {
                t.translation = at.translation;
            }
            if let Some(mut p) = ent.get_mut::<Position>() {
                p.0 = at.translation;
            }
            if let Some(mut i) = ent.get_mut::<Intent>() {
                let (yaw, _, _) = at.rotation.to_euler(EulerRot::YXZ);
                i.yaw = yaw;
                i.pitch = 0.0;
            }
        }
        // A map's spawn equipment (game_player_equip) replaces the kit,
        // and comes at every spawn (survivors of a round too).
        let equipment = world
            .get_resource::<crate::weapon::equip::SpawnEquipment>()
            .and_then(|s| s.0.clone());
        if let Some(items) = equipment {
            for (name, count) in items {
                crate::weapon::equip::give_item(world, e, &name, count);
            }
        } else if fresh {
            let team = world.get::<Team>(e).map(|t| t.0);
            // Rounds start from the team's own kit (you buy the rest);
            // deathmatch adds the extras everyone gets.
            let rounds = world
                .get_resource::<rounds::RoundSettings>()
                .is_some_and(|r| r.enabled != 0);
            let mut kit = starting.for_team(team);
            if rounds {
                kit.retain(|id| !starting.all.contains(id));
            }
            for id in kit {
                let weapon = give(world, e, id);
                // Rounds: you buy ammo, so the kit's reserve is the game's
                // starting amount.
                if rounds && let Some(w) = weapon {
                    crate::weapon::economy::starting_reserve(world, w);
                }
            }
        }
    }
}
