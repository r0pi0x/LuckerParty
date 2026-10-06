//! Match rules. For now one ruleset, deathmatch: the dead stop acting,
//! then respawn at a spawn point with full health and fresh starting
//! weapons; kills and deaths are counted.

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
                (respawn, hold_the_dead).chain().before(SimSet::Movement),
                count_deaths.after(SimSet::Weapons),
            ),
        );
        resource_cvar::<Deathmatch, f32>(app, "mp_respawn_delay", "Seconds before the dead respawn.", |d| {
            &mut d.respawn_delay
        });
        app.init_resource::<crate::core::FriendlyFire>();
        resource_cvar::<crate::core::FriendlyFire, u8>(app, "mp_friendlyfire", "1: teammates hurt each other.", |f| {
            &mut f.0
        });
        app.console_command(
            "jointeam",
            "jointeam <2|3>: join the terrorists (2) or counter-terrorists (3) and respawn at their spawn.",
            |w, a| {
                let team = match a.first().map(String::as_str) {
                    Some("2") => Team(1),
                    Some("3") => Team(2),
                    _ => return Err("jointeam <2|3> (2: terrorists, 3: counter-terrorists)".into()),
                };
                let player = w
                    .query_filtered::<Entity, With<crate::core::LocalPlayer>>()
                    .iter(w)
                    .next()
                    .ok_or("no local player")?;
                let mut e = w.entity_mut(player);
                if e.get::<Team>() == Some(&team) {
                    return Ok(Some("already on that team".into()));
                }
                // Switching teams: respawn at the new team's spawn now (no
                // death counted).
                e.insert((team, Dead { since: f64::MIN }));
                Ok(Some(format!(
                    "joined the {}",
                    if team.0 == 1 {
                        "terrorists"
                    } else {
                        "counter-terrorists"
                    }
                )))
            },
        );
    }
}

#[derive(Resource, Clone, Debug)]
pub struct Deathmatch {
    pub respawn_delay: f32,
    /// Spawn points picked so far (round robin, so players spread out).
    next_spawn: usize,
}

impl Default for Deathmatch {
    fn default() -> Self {
        Self {
            respawn_delay: 2.0,
            next_spawn: 0,
        }
    }
}

/// Kills and deaths.
#[derive(Component, Default, Clone, Copy, Debug, Reflect)]
#[reflect(Component)]
pub struct Score {
    pub kills: u32,
    pub deaths: u32,
}

/// Dead since this time (fixed-tick seconds).
#[derive(Component, Clone, Copy, Debug)]
pub struct Dead {
    pub since: f64,
}

/// Respawn every character on the next tick (after a map change), with
/// fresh weapons, at the spawn points.
pub fn respawn_everyone(world: &mut World) {
    let all: Vec<Entity> = world
        .query_filtered::<Entity, (With<Intent>, With<Health>)>()
        .iter(world)
        .collect();
    for e in all {
        world.entity_mut(e).insert(Dead { since: f64::MIN });
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
        if let Ok(mut e) = commands.get_entity(d.entity) {
            e.insert(Dead { since: now });
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
    let now = world.resource::<Time>().elapsed_secs_f64();
    let delay = world.resource::<Deathmatch>().respawn_delay as f64;
    let ready: Vec<Entity> = world
        .query::<(Entity, &Dead)>()
        .iter(world)
        .filter(|(_, d)| now - d.since >= delay)
        .map(|(e, _)| e)
        .collect();
    if ready.is_empty() {
        return;
    }
    let spawns: Vec<(Transform, Option<Team>)> = world
        .query::<(&Transform, &SpawnPoint)>()
        .iter(world)
        .map(|(t, s)| (*t, s.team))
        .collect();
    let starting = world.resource::<StartingWeapons>().clone();
    for e in ready {
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
        let old = world.get::<Inventory>(e).map(|i| i.weapons.clone()).unwrap_or_default();
        for w in old {
            world.despawn(w);
        }
        let Ok(mut ent) = world.get_entity_mut(e) else { continue };
        ent.remove::<Dead>();
        ent.insert(Inventory::default());
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
        let team = world.get::<Team>(e).map(|t| t.0);
        for id in starting.for_team(team) {
            give(world, e, id);
        }
    }
}
