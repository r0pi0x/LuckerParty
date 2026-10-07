//! Bots: characters whose intent comes from a brain instead of a keyboard
//! (README, "Characters and control"), so they use the same movement and
//! weapons as players. The brain turns toward the nearest enemy it can see
//! at a limited turn rate, strafes, and fires once it has seen them for a
//! reaction time and is on target. With nobody in sight it walks the map's
//! navigation mesh to where it last saw or heard an enemy (sounds within
//! their falloff range: shots carry far, footsteps less), else roams to
//! random places on the mesh.

use std::sync::Arc;

use avian3d::prelude::*;
use bevy::prelude::*;

use crate::{
    character::CAPSULE_HEIGHT,
    character::spawn_character,
    console::{Command, Console, resource_cvar},
    core::{Health, Intent, LocalPlayer, MovementState, SimSet, SpawnPoint, Team},
    map::nav::{NavMesh, STEP_HEIGHT, flags},
    slots::{Loadout, MovementSlot},
};

pub struct BotPlugin;

impl Plugin for BotPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<BotConfig>()
            .add_systems(FixedUpdate, (hear, think).chain().before(SimSet::Movement));
        resource_cvar::<BotConfig, u8>(app, "bot_stop", "1: bots stand still.", |c| &mut c.stop);
        resource_cvar::<BotConfig, u8>(app, "bot_dont_shoot", "1: bots never fire.", |c| &mut c.dont_shoot);
        resource_cvar::<BotConfig, f32>(
            app,
            "bot_reaction",
            "Seconds a bot needs to see you before firing.",
            |c| &mut c.reaction,
        );
        resource_cvar::<BotConfig, f32>(app, "bot_aim_error", "Bot aim wobble, degrees.", |c| &mut c.aim_error);
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
    /// Aim wobble, degrees (re-rolled every `AIM_REROLL` seconds).
    pub aim_error: f32,
}

impl Default for BotConfig {
    fn default() -> Self {
        Self {
            stop: 0,
            dont_shoot: 0,
            reaction: 0.35,
            turn_rate: 360.0,
            aim_error: 2.5,
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
    /// Points to walk through (engine space) and the next one's index.
    route: Vec<Vec3>,
    next: usize,
    /// Seconds until the route is recomputed.
    repath: f32,
    /// Where the bot was at the last progress check, and when that was.
    progress: (Vec3, f32),
    /// Current aim offset (yaw, pitch radians) and seconds until re-rolled.
    wobble: (Vec2, f32),
    /// Where an enemy was last seen or heard (feet), and when.
    pub lead: Option<(Vec3, f64)>,
    /// Where the bot is roaming to (feet), with nothing better to do.
    roam: Option<Vec3>,
}

impl Bot {
    /// The route being walked (engine space) and the next point's index.
    pub fn route(&self) -> (&[Vec3], usize) {
        (&self.route, self.next)
    }

    /// Where the bot is roaming to, with nothing better to do.
    pub fn roam_goal(&self) -> Option<Vec3> {
        self.roam
    }
}

/// Share of roaming goals picked at the map's objectives (bomb sites,
/// hostages) rather than anywhere, so teams meet where CS:S bots do.
const OBJECTIVE_SHARE: f32 = 0.7;

/// The map's objective points (engine space, feet height of the volume's
/// bottom or the entity): bomb sites (`func_bomb_target`), hostage rescue
/// zones and hostages.
pub fn objectives(map: &crate::map::MapEntities) -> Vec<Vec3> {
    use crate::map::entities::{entity_rotation, entity_to_engine};
    map.entities
        .iter()
        .filter_map(|e| {
            let class = e.classname();
            match class {
                "func_bomb_target" | "func_hostage_rescue" => {
                    let points: Vec<Vec3> = e.hulls.iter().flat_map(|h| h.points.iter().copied()).collect();
                    if points.is_empty() {
                        return None;
                    }
                    let lo = points.iter().fold(Vec3::splat(f32::MAX), |a, p| a.min(*p));
                    let hi = points.iter().fold(Vec3::splat(f32::MIN), |a, p| a.max(*p));
                    let local = Vec3::new((lo.x + hi.x) / 2.0, (lo.y + hi.y) / 2.0, lo.z);
                    Some(entity_to_engine(e.origin() + entity_rotation(e.angles()) * local, map.scale))
                }
                "hostage_entity" | "info_bomb_target" => Some(entity_to_engine(e.origin(), map.scale)),
                _ => None,
            }
        })
        .collect()
}

/// Seconds a bot keeps chasing what it saw or heard.
const MEMORY: f64 = 15.0;
/// A remembered or roaming goal counts as reached within this, m.
const ARRIVED: f32 = 2.0;
/// A sound is heard when its distance gain at the bot is above this.
const HEARING_GAIN: f32 = 0.05;

/// Bots hear enemies' sounds (shots, footsteps, impacts they cause).
fn hear(
    mut sounds: MessageReader<crate::map::PlaySound>,
    bank: Option<Res<crate::map::sound::SoundBank>>,
    mut bots: Query<(&mut Bot, &Transform, &Team, &Health)>,
    teams: Query<(&Team, &Transform)>,
    time: Res<Time>,
) {
    let now = time.elapsed_secs_f64();
    for s in sounds.read() {
        let (Some(source), Some(at)) = (s.source, s.at) else {
            continue;
        };
        let Ok((source_team, source_at)) = teams.get(source) else {
            continue;
        };
        let level = bank
            .as_ref()
            .and_then(|b| b.0.entry(&s.entry).map(|e| e.level))
            .unwrap_or(crate::map::SoundLevel::Db(75.0));
        let feet = source_at.translation - Vec3::Y * CAPSULE_HEIGHT / 2.0;
        for (mut bot, t, team, health) in &mut bots {
            if team == source_team || health.current <= 0.0 {
                continue;
            }
            let units = t.translation.distance(at) / 0.0254;
            if crate::map::sound::distance_gain(level, units) >= HEARING_GAIN {
                bot.lead = Some((feet, now));
            }
        }
    }
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
    let yaw = at.rotation.to_euler(EulerRot::YXZ).0;
    let mut commands = world.commands();
    // Characters aren't rotated; the spawn's facing becomes the look yaw.
    let e = spawn_character(
        &mut commands,
        Transform::from_translation(at.translation),
        team,
        movement,
    );
    commands.entity(e).insert((
        Name::new(format!("Bot {}", bots + 1)),
        Bot {
            rng: 0x2545_F491_4F6C_DD1D ^ (e.to_bits().wrapping_mul(0x9E37_79B9_7F4A_7C15)),
            ..default()
        },
        Intent { yaw, ..default() },
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
/// Bots can't see while a flash leaves the screen whiter than this.
const BLIND_ALPHA: f32 = 0.5;
/// Seconds between new aim wobbles.
const AIM_REROLL: f32 = 0.4;

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
        Option<&crate::core::Blinded>,
    )>,
    others: Query<(Entity, &Transform, &Team, &Health), With<Intent>>,
    smoke: Query<&crate::core::SightBlocker>,
    spatial: SpatialQuery,
    nav: Option<Res<NavMesh>>,
    cfg: Res<BotConfig>,
    time: Res<Time>,
    entities: Option<Res<crate::map::MapEntities>>,
    mut goals: Local<Option<(usize, Vec<Vec3>)>>,
) {
    let dt = time.delta_secs();
    // The map's objectives, worked out once per map.
    let key = entities.as_ref().map_or(0, |m| std::sync::Arc::as_ptr(&m.entities) as usize);
    if goals.as_ref().is_none_or(|(k, _)| *k != key) {
        *goals = Some((key, entities.as_deref().map(objectives).unwrap_or_default()));
    }
    let objective_points: &[Vec3] = goals.as_ref().map_or(&[], |(_, g)| g);
    let now = time.elapsed_secs_f64();
    for (me, mut bot, mut intent, t, state, team, health, blinded) in &mut bots {
        if health.current <= 0.0 {
            bot.target = None;
            continue;
        }
        let eye = t.translation + state.eye_offset;
        // A flashed bot sees nothing until the white is mostly gone.
        let blind = blinded.is_some_and(|b| b.alpha_at(now) > BLIND_ALPHA);
        // The nearest living enemy in sight.
        let mut best: Option<(Entity, Vec3, f32)> = None;
        for (e, ot, oteam, oh) in &others {
            if blind || e == me || oteam == team || oh.current <= 0.0 {
                continue;
            }
            let aim = ot.translation + Vec3::Y * AIM_HEIGHT;
            let to = aim - eye;
            let dist = to.length();
            if best.is_some_and(|b| b.2 <= dist) {
                continue;
            }
            let Ok(dir) = Dir3::new(to) else { continue };
            let filter = SpatialQueryFilter::from_excluded_entities([me]).with_mask(crate::core::SOLID_LAYERS);
            let visible = spatial
                .cast_ray(eye, dir, dist, true, &filter)
                .is_none_or(|h| h.entity == e || h.distance >= dist - 0.05);
            // Nor through smoke (specs/cs_source/grenades.md Q16).
            if visible && !smoke.iter().any(|s| s.blocks(eye, aim)) {
                best = Some((e, aim, dist));
            }
        }

        intent.fire = false;
        match best {
            Some((e, aim, _)) => {
                // Remember where they were (feet) for after they hide.
                bot.lead = Some((
                    aim - Vec3::Y * (AIM_HEIGHT + CAPSULE_HEIGHT / 2.0),
                    time.elapsed_secs_f64(),
                ));
                if bot.target == Some(e) {
                    bot.seen += dt;
                } else {
                    bot.target = Some(e);
                    bot.seen = 0.0;
                }
                bot.wobble.1 -= dt;
                if bot.wobble.1 <= 0.0 {
                    let (a, r) = (bot.rand() * std::f32::consts::TAU, bot.rand().sqrt());
                    bot.wobble = (Vec2::from_angle(a) * r * cfg.aim_error.to_radians(), AIM_REROLL);
                }
                let to = aim - eye;
                let want_yaw = (-to.x).atan2(-to.z) + bot.wobble.0.x;
                let want_pitch = to.y.atan2(to.xz().length()) + bot.wobble.0.y;
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
        intent.jump = false;
        intent.crouch = false;
        if bot.target.is_some() || cfg.stop != 0 {
            bot.route.clear();
            continue;
        }
        let Some(nav) = nav.as_deref() else { continue };
        let feet = t.translation - Vec3::Y * CAPSULE_HEIGHT / 2.0;
        let now = time.elapsed_secs_f64();
        // Chase what was seen or heard; once there (or it's stale), roam.
        if bot
            .lead
            .is_some_and(|(at, when)| now - when > MEMORY || at.distance(feet) < ARRIVED)
        {
            bot.lead = None;
        }
        if bot.roam.is_some_and(|at| at.distance(feet) < ARRIVED) {
            bot.roam = None;
        }
        let goal = match bot.lead {
            Some((at, _)) => at,
            None => {
                if bot.roam.is_none() && !nav.areas.is_empty() {
                    let at_objective = !objective_points.is_empty() && bot.rand() < OBJECTIVE_SHARE;
                    bot.roam = Some(if at_objective {
                        let k = (bot.rand() * objective_points.len() as f32) as usize % objective_points.len();
                        objective_points[k]
                    } else {
                        let i = (bot.rand() * nav.areas.len() as f32) as usize % nav.areas.len();
                        nav.areas[i].center
                    });
                    bot.repath = 0.0;
                }
                match bot.roam {
                    Some(at) => at,
                    None => continue,
                }
            }
        };
        walk_route(&mut bot, &mut intent, nav, feet, goal, &cfg, dt);
    }
}

/// A route point counts as reached within this distance (2D), m.
const REACHED: f32 = 25.0 * 0.0254;
/// Seconds between route recomputations while walking.
const REPATH_SECONDS: f32 = 1.0;
/// Without this much progress in `STUCK_SECONDS`, repath (and jump).
const STUCK_DISTANCE: f32 = 0.5;
const STUCK_SECONDS: f32 = 1.5;

/// Walk the navigation mesh toward `goal` (feet positions).
fn walk_route(bot: &mut Bot, intent: &mut Intent, nav: &NavMesh, feet: Vec3, goal: Vec3, cfg: &BotConfig, dt: f32) {
    bot.repath -= dt;
    bot.progress.1 += dt;
    let mut stuck = false;
    if bot.progress.1 >= STUCK_SECONDS {
        stuck = feet.distance(bot.progress.0) < STUCK_DISTANCE;
        bot.progress = (feet, 0.0);
    }
    if bot.repath <= 0.0 || bot.next >= bot.route.len() || stuck {
        bot.repath = REPATH_SECONDS;
        bot.route = nav.route(feet, goal).unwrap_or_default();
        bot.next = 0;
    }
    while bot.next < bot.route.len() && (bot.route[bot.next] - feet).xz().length() < REACHED {
        bot.next += 1;
    }
    let Some(&point) = bot.route.get(bot.next) else { return };
    let to = point - feet;
    let want_yaw = (-to.x).atan2(-to.z);
    let step = cfg.turn_rate.to_radians() * dt;
    intent.yaw = wrap(intent.yaw + wrap(want_yaw - intent.yaw).clamp(-step, step));
    intent.pitch -= intent.pitch.clamp(-step, step);
    // Walk forward once roughly facing the point.
    let off = wrap(want_yaw - intent.yaw).abs();
    intent.move_axis = Vec2::new(0.0, if off < 1.0 { 1.0 } else { 0.0 });
    // Jump up ledges higher than a step, and when stuck.
    intent.jump = stuck || (to.y > STEP_HEIGHT && to.xz().length() < 1.5);
    intent.crouch = nav.area_at(feet).is_some_and(|a| nav.areas[a].has(flags::CROUCH));
}

fn wrap(a: f32) -> f32 {
    (a + std::f32::consts::PI).rem_euclid(std::f32::consts::TAU) - std::f32::consts::PI
}
