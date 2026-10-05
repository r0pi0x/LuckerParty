//! Bots: characters whose intent comes from a brain instead of a keyboard
//! (README, "Characters and control"), so they use the same movement and
//! weapons as players. This first brain has no navigation: it strafes in
//! place, turns toward the nearest enemy it can see at a limited turn rate
//! and fires once it has seen them for a reaction time and is on target.

use std::sync::Arc;

use avian3d::prelude::*;
use bevy::prelude::*;

use crate::{
    character::spawn_character,
    console::{Command, Console, resource_cvar},
    core::{Health, Intent, LocalPlayer, MovementState, SimSet, SpawnPoint, Team},
    slots::{Loadout, MovementSlot},
};

pub struct BotPlugin;

impl Plugin for BotPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<BotConfig>()
            .add_systems(FixedUpdate, think.before(SimSet::Movement));
        resource_cvar::<BotConfig, u8>(app, "bot_stop", "1: bots stand still.", |c| &mut c.stop);
        resource_cvar::<BotConfig, u8>(app, "bot_dont_shoot", "1: bots never fire.", |c| &mut c.dont_shoot);
        resource_cvar::<BotConfig, f32>(
            app,
            "bot_reaction",
            "Seconds a bot needs to see you before firing.",
            |c| &mut c.reaction,
        );
        resource_cvar::<BotConfig, f32>(app, "bot_turn_rate", "Bot turn speed, degrees per second.", |c| {
            &mut c.turn_rate
        });
        app.init_resource::<Console>();
        let mut console = app.world_mut().resource_mut::<Console>();
        console.add_command(Command {
            name: "bot_add".into(),
            help: "bot_add [team]: add a bot (team 1 by default; players are team 0).".into(),
            run: Arc::new(|w, a| {
                let team = match a.first() {
                    Some(t) => t.parse().map_err(|_| format!("bad team {t}"))?,
                    None => 1,
                };
                let e = add_bot(w, Team(team)).ok_or("no spawn point")?;
                Ok(Some(format!("added bot {e}")))
            }),
            complete: None,
        });
        console.add_command(Command {
            name: "bot_kick".into(),
            help: "Remove every bot.".into(),
            run: Arc::new(|w, _| {
                let bots: Vec<Entity> = w.query_filtered::<Entity, With<Bot>>().iter(w).collect();
                let n = bots.len();
                for b in bots {
                    w.entity_mut(b).despawn();
                }
                Ok(Some(format!("kicked {n} bots")))
            }),
            complete: None,
        });
    }
}

#[derive(Resource, Clone, Debug)]
pub struct BotConfig {
    pub stop: u8,
    pub dont_shoot: u8,
    pub reaction: f32,
    pub turn_rate: f32,
}

impl Default for BotConfig {
    fn default() -> Self {
        Self {
            stop: 0,
            dont_shoot: 0,
            reaction: 0.35,
            turn_rate: 360.0,
        }
    }
}

/// A bot's brain state.
#[derive(Component, Clone, Debug, Default)]
pub struct Bot {
    pub target: Option<Entity>,
    /// Seconds the current target has been in sight.
    pub seen: f32,
    strafe: f32,
    strafe_left: f32,
    rng: u64,
}

/// Spawn a bot on `team` at a spawn point (the team's, if the map has
/// them), using the local player's movement or the loadout's.
pub fn add_bot(world: &mut World, team: Team) -> Option<Entity> {
    let spawns: Vec<(Transform, Option<Team>)> = world
        .query::<(&Transform, &SpawnPoint)>()
        .iter(world)
        .map(|(t, s)| (*t, s.team))
        .collect();
    let bots = world.query_filtered::<(), With<Bot>>().iter(world).count();
    let own: Vec<&Transform> = spawns.iter().filter(|s| s.1 == Some(team)).map(|s| &s.0).collect();
    let pool: Vec<&Transform> = if own.is_empty() {
        spawns.iter().map(|s| &s.0).collect()
    } else {
        own
    };
    // Skip the first spawn, where the local player starts.
    let at = **pool.get((bots + 1) % pool.len().max(1)).or(pool.first())?;
    let movement = world
        .query_filtered::<&MovementSlot, With<LocalPlayer>>()
        .iter(world)
        .next()
        .map(|m| m.0)
        .or_else(|| world.get_resource::<Loadout>().map(|l| l.movement))?;
    let mut commands = world.commands();
    let e = spawn_character(&mut commands, at, team, movement);
    commands.entity(e).insert((
        Name::new(format!("Bot {}", bots + 1)),
        Bot {
            rng: 0x2545_F491_4F6C_DD1D ^ (e.to_bits().wrapping_mul(0x9E37_79B9_7F4A_7C15)),
            ..default()
        },
    ));
    world.flush();
    Some(e)
}

impl Bot {
    fn rand(&mut self) -> f32 {
        // xorshift64*
        self.rng ^= self.rng >> 12;
        self.rng ^= self.rng << 25;
        self.rng ^= self.rng >> 27;
        (self.rng.wrapping_mul(0x2545_F491_4F6C_DD1D) >> 40) as f32 / (1u64 << 24) as f32
    }
}

/// Where to aim on a character: chest height above its origin (the
/// origin is the capsule's centre).
const AIM_HEIGHT: f32 = 0.35;
/// Fire when the aim is within this many degrees of the target.
const FIRE_CONE_DEG: f32 = 3.0;

#[allow(clippy::type_complexity)]
fn think(
    mut bots: Query<(
        Entity,
        &mut Bot,
        &mut Intent,
        &Transform,
        &MovementState,
        &Team,
        &Health,
    )>,
    others: Query<(Entity, &Transform, &Team, &Health), With<Intent>>,
    spatial: SpatialQuery,
    cfg: Res<BotConfig>,
    time: Res<Time>,
) {
    let dt = time.delta_secs();
    for (me, mut bot, mut intent, t, state, team, health) in &mut bots {
        if health.current <= 0.0 {
            bot.target = None;
            continue;
        }
        let eye = t.translation + state.eye_offset;
        // The nearest living enemy in sight.
        let mut best: Option<(Entity, Vec3, f32)> = None;
        for (e, ot, oteam, oh) in &others {
            if e == me || oteam == team || oh.current <= 0.0 {
                continue;
            }
            let aim = ot.translation + Vec3::Y * AIM_HEIGHT;
            let to = aim - eye;
            let dist = to.length();
            if best.is_some_and(|b| b.2 <= dist) {
                continue;
            }
            let Ok(dir) = Dir3::new(to) else { continue };
            let filter = SpatialQueryFilter::from_excluded_entities([me]);
            let visible = spatial
                .cast_ray(eye, dir, dist, true, &filter)
                .is_none_or(|h| h.entity == e || h.distance >= dist - 0.05);
            if visible {
                best = Some((e, aim, dist));
            }
        }

        intent.fire = false;
        match best {
            Some((e, aim, _)) => {
                if bot.target == Some(e) {
                    bot.seen += dt;
                } else {
                    bot.target = Some(e);
                    bot.seen = 0.0;
                }
                let to = aim - eye;
                let want_yaw = (-to.x).atan2(-to.z);
                let want_pitch = to.y.atan2(to.xz().length());
                let step = cfg.turn_rate.to_radians() * dt;
                let dyaw = wrap(want_yaw - intent.yaw);
                intent.yaw = wrap(intent.yaw + dyaw.clamp(-step, step));
                intent.pitch += (want_pitch - intent.pitch).clamp(-step, step);
                let off = wrap(want_yaw - intent.yaw).abs().max((want_pitch - intent.pitch).abs());
                intent.fire = cfg.dont_shoot == 0 && bot.seen >= cfg.reaction && off.to_degrees() < FIRE_CONE_DEG;
            }
            None => {
                bot.target = None;
                bot.seen = 0.0;
            }
        }

        // Strafe in place, switching direction every 0.4-1.2 s.
        bot.strafe_left -= dt;
        if bot.strafe_left <= 0.0 {
            bot.strafe_left = 0.4 + 0.8 * bot.rand();
            let r = bot.rand();
            bot.strafe = if r < 0.4 {
                -1.0
            } else if r < 0.8 {
                1.0
            } else {
                0.0
            };
        }
        intent.move_axis = if cfg.stop == 0 && bot.target.is_some() {
            Vec2::new(bot.strafe, 0.0)
        } else {
            Vec2::ZERO
        };
    }
}

fn wrap(a: f32) -> f32 {
    (a + std::f32::consts::PI).rem_euclid(std::f32::consts::TAU) - std::f32::consts::PI
}
