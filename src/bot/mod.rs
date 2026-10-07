//! Bots: characters whose intent comes from a brain instead of a keyboard
//! (README, "Characters and control"), so they use the same movement and
//! weapons as players. The brain turns toward the nearest enemy it can see
//! at a limited turn rate, strafes, and fires once it has seen them for a
//! reaction time and is on target. With nobody in sight it follows its
//! team's plan (`tactics`): attackers walk to a site as a group and hold
//! it, defenders hold spots at the sites watching the approaches; it goes
//! after enemies it saw or heard (sounds within their falloff range: shots
//! carry far, footsteps less) when its role allows, and to teammates who
//! call for help. It walks the navigation mesh with its own path cost
//! (`path`) and checks corners as it goes (`look`). Now and then it throws
//! a grenade it carries at a remembered enemy or at the site it walks to
//! (`grenades`), and it looks away from flashes about to go off.

mod grenades;
mod look;
pub mod objectives;
pub mod path;
pub mod radio;
pub mod tactics;

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
    weapon::{Inventory, grenade::Projectile},
};

pub use grenades::{GrenadePlan, follow, plan_throw};
pub use tactics::{Hold, Role, Tactics};

pub struct BotPlugin;

impl Plugin for BotPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<BotConfig>()
            .init_resource::<radio::TeamCalls>()
            .init_resource::<Tactics>()
            .add_message::<crate::core::Radio>();
        app.add_systems(FixedUpdate, radio::speak.after(crate::core::apply_damage));
        app.add_systems(
            FixedUpdate,
            (hear, tactics::update, objectives::goals, think, objectives::act)
                .chain()
                .before(SimSet::Movement)
                .before(crate::weapon::SelectWeapons),
        );
        resource_cvar::<BotConfig, u8>(app, "bot_stop", "1: bots stand still.", |c| &mut c.stop);
        resource_cvar::<BotConfig, u8>(app, "bot_dont_shoot", "1: bots never fire.", |c| &mut c.dont_shoot);
        resource_cvar::<BotConfig, f32>(
            app,
            "bot_reaction",
            "Seconds a bot needs to see you before firing.",
            |c| &mut c.reaction,
        );
        resource_cvar::<BotConfig, u8>(
            app,
            "bot_grenades",
            "Bots throw grenades: 0 never, 1 now and then, 2 whenever they can.",
            |c| &mut c.grenades,
        );
        resource_cvar::<BotConfig, u8>(
            app,
            "bot_radio",
            "1: bots use the team radio (enemy spotted, enemy down, need backup).",
            |c| &mut c.radio,
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
            name: "bot_give".into(),
            help: "bot_give <weapon>: give every bot a weapon (e.g. weapon_hegrenade); bots keep their gun in hand."
                .into(),
            run: Arc::new(|w, a| {
                let name = a.first().ok_or("usage: bot_give <weapon>")?.to_lowercase();
                let wanted = if name.contains(':') || name.starts_with("weapon_") {
                    name
                } else {
                    format!("weapon_{name}")
                };
                let id = w
                    .resource::<crate::weapon::WeaponRegistry>()
                    .find(&wanted)
                    .map(|d| d.id)
                    .ok_or_else(|| format!("no weapon {wanted}"))?;
                let bots: Vec<Entity> = w.query_filtered::<Entity, With<Bot>>().iter(w).collect();
                for &b in &bots {
                    crate::weapon::give(w, b, id);
                }
                Ok(Some(format!("gave {id} to {} bots", bots.len())))
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
        console.add_command(Command {
            name: "bot_goto".into(),
            help: "bot_goto <x> <y> <z>: send every bot to a feet position (CS:S units, as getpos less the eye height); \
                   bot_goto alone lets them go back to their orders."
                .into(),
            run: Arc::new(|w, a| {
                let at = match &a[..] {
                    [] => None,
                    [x, y, z] => {
                        let n = |s: &String| s.parse::<f32>().map_err(|_| format!("bad number \"{s}\""));
                        Some(Vec3::new(n(x)?, n(z)?, -n(y)?) * 0.0254)
                    }
                    _ => return Err("bot_goto [<x> <y> <z>]".into()),
                };
                let mut q = w.query::<&mut Bot>();
                let mut n = 0;
                for mut b in q.iter_mut(w) {
                    b.send_to(at);
                    n += 1;
                }
                Ok(Some(format!("sent {n} bots")))
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
    /// Grenades: 0 never, 1 now and then, 2 whenever a throw is possible.
    pub grenades: u8,
    /// 1: bots use the team radio (`radio`).
    pub radio: u8,
}

impl Default for BotConfig {
    fn default() -> Self {
        Self {
            stop: 0,
            dont_shoot: 0,
            reaction: 0.35,
            turn_rate: 360.0,
            aim_error: 2.5,
            grenades: 1,
            radio: 1,
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
    /// Points to walk through (engine space), the area each leads into,
    /// and the next one's index.
    route: Vec<Vec3>,
    route_areas: Vec<usize>,
    next: usize,
    /// Seconds until the route is recomputed.
    repath: f32,
    /// Where the bot was at the last progress check, and when that was.
    progress: (Vec3, f32),
    /// Seconds it has been trying to walk without moving.
    blocked: f32,
    /// Jump was pressed last tick (it must be let go between jumps).
    jump_held: bool,
    /// The route point it walks to, and since when.
    toward: (Vec3, f64),
    /// Progress checks in a row that found it stuck, and the links it
    /// got stuck on lately (from area, to area), costly until then.
    stuck_count: u32,
    stuck_links: Vec<((usize, usize), f64)>,
    /// When it last jumped.
    jumped: f64,
    /// Current aim offset (yaw, pitch radians) and seconds until re-rolled.
    wobble: (Vec2, f32),
    /// Where an enemy was last seen or heard (feet), and when.
    pub lead: Option<(Vec3, f64)>,
    /// Where the bot is roaming to (feet), with nothing better to do.
    roam: Option<Vec3>,
    /// The roaming goal is one of the map's objectives.
    roam_objective: bool,
    /// A grenade throw in progress, and the latest plan (shown until).
    toss: Option<grenades::Toss>,
    plan: Option<(GrenadePlan, f64)>,
    /// No throw before this; next look for a chance to throw.
    next_toss: f64,
    next_toss_check: f64,
    /// Looking away from a flash; flashes already noticed.
    avert: Option<grenades::Avert>,
    watched: Vec<Entity>,
    /// What it said on the radio lately.
    radio: radio::BotRadio,
    /// Route noise seed (fixed per bot).
    seed: u64,
    /// The round's orders from `tactics`, and what it does with them in
    /// this life: the site it heads for or holds, its hold spot, when it
    /// got there, how long it holds without contact before moving on,
    /// and whether it gave up holding to hunt.
    orders: tactics::Orders,
    site: Option<usize>,
    hold: Option<Hold>,
    hold_since: Option<f64>,
    patience: f64,
    hunting: bool,
    /// A teammate's call it answers: where they were, and when.
    assist: Option<(Vec3, f64)>,
    /// When it last saw or heard an enemy.
    contact: f64,
    /// Nav area it stands in, the one before, and where it came in.
    area: Option<usize>,
    prev_area: Option<usize>,
    entered: Vec3,
    corners: look::Corners,
    /// Which watched approach it looks at, until when.
    watch: (usize, f64),
    /// Where it is going and where it looks, and what it is doing (for
    /// debug views).
    goal: Option<Vec3>,
    look_at: Option<Vec3>,
    activity: Activity,
    dead: bool,
    /// Where its objective is (feet; a bomb target to plant at, the
    /// planted bomb to defuse or guard): walked to before anything else
    /// (`objectives`).
    pub objective: Option<Vec3>,
    /// Carries the bomb (`objectives`): the attackers' group follows it.
    pub carrying: bool,
    /// Sent somewhere from outside (`bot_goto`, tests): walked to before
    /// anything else but a fight.
    pub move_to: Option<Vec3>,
}

/// What a bot is doing, for debug views.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Activity {
    #[default]
    Idle,
    Dead,
    Fighting,
    Throwing,
    Averting,
    /// Going where it last saw or heard an enemy.
    Chasing,
    /// Going to a teammate who called.
    Assisting,
    ToSite,
    Following,
    /// Waiting for the group (leader) or for the leader (ahead of it).
    Waiting,
    ToHold,
    Holding,
    Hunting,
    Roaming,
    /// Going where it was sent (`Bot::move_to`).
    Sent,
}

impl Bot {
    /// The route being walked (engine space) and the next point's index.
    pub fn route(&self) -> (&[Vec3], usize) {
        (&self.route, self.next)
    }

    /// Where the bot is going (feet), whatever the reason.
    pub fn goal(&self) -> Option<Vec3> {
        self.goal
    }

    /// The point it looks at while no enemy is in sight.
    pub fn look_target(&self) -> Option<Vec3> {
        self.look_at
    }

    /// The hiding spot it is checking now.
    pub fn checking(&self) -> Option<Vec3> {
        self.corners.current()
    }

    pub fn role(&self) -> Role {
        self.orders.role
    }

    /// The site it heads for or holds (index into `Tactics::sites`).
    pub fn site(&self) -> Option<usize> {
        self.site
    }

    pub fn hold(&self) -> Option<&Hold> {
        self.hold.as_ref()
    }

    pub fn activity(&self) -> Activity {
        self.activity
    }

    /// Who it is answering, if a teammate called.
    pub fn assisting(&self) -> Option<Vec3> {
        self.assist.map(|a| a.0)
    }

    /// Send it somewhere (feet), before anything but a fight; None lets
    /// it go back to its orders. The route is worked out afresh.
    pub fn send_to(&mut self, at: Option<Vec3>) {
        self.move_to = at;
        self.route.clear();
        self.route_areas.clear();
        self.next = 0;
        self.repath = 0.0;
    }

    /// Route left to walk, m.
    pub fn route_left(&self) -> f32 {
        let rest = self.route.get(self.next..).unwrap_or(&[]);
        rest.windows(2).map(|w| w[0].distance(w[1])).sum()
    }

    /// Start a life (a round, a respawn, new orders): forget the last
    /// one and take up the orders.
    fn new_life(&mut self) {
        self.route.clear();
        self.route_areas.clear();
        self.next = 0;
        self.lead = None;
        self.assist = None;
        self.roam = None;
        self.target = None;
        self.toss = None;
        self.plan = None;
        self.avert = None;
        self.site = self.orders.site;
        self.hold = self.orders.hold.clone();
        self.hold_since = None;
        self.hunting = false;
        self.corners = default();
        self.area = None;
        self.prev_area = None;
        self.stuck_count = 0;
        self.stuck_links.clear();
        let (lo, hi) = match self.orders.role {
            Role::Attack => ATTACK_PATIENCE,
            _ => DEFEND_PATIENCE,
        };
        self.patience = lo + (hi - lo) * self.rand() as f64;
    }

    /// The grenade throw planned or carried out lately.
    pub fn grenade_plan(&self) -> Option<&GrenadePlan> {
        self.plan.as_ref().map(|(p, _)| p)
    }

    /// Throwing a grenade now.
    pub fn throwing(&self) -> bool {
        self.toss.is_some()
    }

    /// Looking away from a flash about to go off: from where.
    pub fn averting(&self) -> Option<Vec3> {
        self.avert.map(|a| a.from)
    }
}

/// Share of roaming goals picked at the map's objectives (bomb sites,
/// hostages) rather than anywhere, so teams meet where CS:S bots do.
const OBJECTIVE_SHARE: f32 = 0.7;

/// The map's objective points (engine space, feet height of the volume's
/// bottom or the entity): bomb sites (`func_bomb_target`), hostage rescue
/// zones and hostages.
pub fn objectives(map: &crate::map::MapEntities) -> Vec<Vec3> {
    objectives_of(
        map,
        &[
            "func_bomb_target",
            "func_hostage_rescue",
            "hostage_entity",
            "info_bomb_target",
        ],
    )
}

/// `objectives` of the given classes only.
pub fn objectives_of(map: &crate::map::MapEntities, classes: &[&str]) -> Vec<Vec3> {
    use crate::map::entities::{entity_rotation, entity_to_engine};
    map.entities
        .iter()
        .filter_map(|e| {
            let class = e.classname();
            if !classes.contains(&class) {
                return None;
            }
            match class {
                "func_bomb_target" | "func_hostage_rescue" => {
                    let points: Vec<Vec3> = e.hulls.iter().flat_map(|h| h.points.iter().copied()).collect();
                    if points.is_empty() {
                        return None;
                    }
                    let lo = points.iter().fold(Vec3::splat(f32::MAX), |a, p| a.min(*p));
                    let hi = points.iter().fold(Vec3::splat(f32::MIN), |a, p| a.max(*p));
                    let local = Vec3::new((lo.x + hi.x) / 2.0, (lo.y + hi.y) / 2.0, lo.z);
                    Some(entity_to_engine(
                        e.origin() + entity_rotation(e.angles()) * local,
                        map.scale,
                    ))
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
            seed: e.to_bits().wrapping_mul(0xD1B5_4A32_D192_ED03),
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

#[allow(clippy::type_complexity, clippy::too_many_arguments)]
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
        Option<&mut Inventory>,
        &crate::core::Velocity,
    )>,
    others: Query<(Entity, &Transform, &Team, &Health), With<Intent>>,
    teamless: Query<(&Transform, &Health), (With<Intent>, Without<Team>)>,
    arms: grenades::Arms,
    triggers: Query<&crate::weapon::Trigger>,
    projectiles: Query<(Entity, &Transform, &Projectile)>,
    smoke: Query<&crate::core::SightBlocker>,
    breakable: Query<(), With<crate::core::Damageable>>,
    spatial: SpatialQuery,
    nav: Option<Res<NavMesh>>,
    cfg: Res<BotConfig>,
    time: Res<Time>,
    tactics: Res<Tactics>,
) {
    let dt = time.delta_secs();
    let now = time.elapsed_secs_f64();
    // World traces for flash sight (a line) and grenade arcs (the
    // grenade's box swept, half size given): characters aren't in the way.
    let no_nav = NavMesh::default();
    let filter = SpatialQueryFilter::default().with_mask(crate::core::SOLID_LAYERS);
    let not_character = |e: Entity| !others.contains(e);
    let world_trace = |a: Vec3, b: Vec3| -> Option<(Vec3, Vec3)> {
        let d = b - a;
        let dir = Dir3::new(d).ok()?;
        spatial
            .cast_ray_predicate(a, dir, d.length(), true, &filter, &not_character)
            .map(|h| (a + *dir * h.distance, h.normal))
    };
    let box_trace = |a: Vec3, b: Vec3, half: f32| -> Option<(Vec3, Vec3)> {
        let d = b - a;
        let dir = Dir3::new(d).ok()?;
        let config = ShapeCastConfig {
            max_distance: d.length(),
            ignore_origin_penetration: true,
            ..ShapeCastConfig::DEFAULT
        };
        let shape = Collider::cuboid(half * 2.0, half * 2.0, half * 2.0);
        spatial
            .cast_shape_predicate(&shape, a, Quat::IDENTITY, dir, &config, &filter, &not_character)
            .map(|h| (a + *dir * h.distance, h.normal1.normalize_or_zero()))
    };
    for (me, mut bot, mut intent, t, state, team, health, blinded, mut inv, velocity) in &mut bots {
        if health.current <= 0.0 {
            bot.target = None;
            bot.toss = None;
            bot.avert = None;
            bot.dead = true;
            bot.activity = Activity::Dead;
            continue;
        }
        if bot.dead {
            // Respawned.
            bot.dead = false;
            bot.new_life();
        }
        let eye = t.translation + state.eye_offset;
        // A flashed bot sees nothing until the white is mostly gone, and
        // aims worse while it fades.
        let white = blinded.map_or(0.0, |b| b.alpha_at(now));
        let blind = white > BLIND_ALPHA;
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

        // Flashes in flight it sees: now and then look away until they pop.
        bot.watched.retain(|g| projectiles.contains(*g));
        if !blind {
            for (g, gt, p) in &projectiles {
                let flash = matches!(p.effect, crate::weapon::grenade::GrenadeEffect::Flash(_));
                let at = gt.translation;
                if !flash
                    || p.thrower == Some(me)
                    || bot.watched.contains(&g)
                    || at.distance(eye) > grenades::FLASH_WATCH
                    || world_trace(eye, at).is_some()
                    || smoke.iter().any(|s| s.blocks(eye, at))
                {
                    continue;
                }
                bot.watched.push(g);
                if bot.rand() < grenades::FLASH_REACT {
                    let left = (p.fuse - p.ticks as f32 * dt).max(0.0);
                    bot.avert = Some(grenades::Avert {
                        from: at,
                        until: now + left as f64 + 0.3,
                        grenade: Some(g),
                    });
                }
            }
        }
        if let Some(a) = bot.avert.as_mut() {
            match a.grenade.map(|g| projectiles.get(g)) {
                Some(Ok((_, gt, _))) => a.from = gt.translation,
                // Gone: it popped.
                Some(Err(_)) => a.until = now,
                None => {}
            }
            if now >= a.until {
                bot.avert = None;
            }
        }
        if bot.toss.is_none() && bot.plan.as_ref().is_some_and(|(_, until)| now > *until) {
            bot.plan = None;
        }
        // An enemy in sight before the pin is out: forget the throw.
        if best.is_some()
            && let Some(toss) = bot.toss.as_ref()
            && toss.pulled.is_none()
        {
            let previous = toss.previous;
            bot.toss = None;
            if let Some(inv) = inv.as_deref_mut() {
                inv.wanted = previous
                    .filter(|p| inv.weapons.contains(p))
                    .or_else(|| grenades::best_weapon(inv, &arms));
            }
        }

        // Semi-automatics fire once per press: let go every other tick.
        let pressed = intent.fire;
        let semi = inv
            .as_deref()
            .and_then(|i| i.active)
            .and_then(|w| triggers.get(w).ok())
            .is_some_and(|t| !t.automatic);
        intent.fire = false;
        match best {
            Some((e, aim, _)) => {
                bot.contact = now;
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
                    let error = cfg.aim_error * (1.0 + FLASHED_AIM * white);
                    bot.wobble = (Vec2::from_angle(a) * r * error.to_radians(), AIM_REROLL);
                }
                let to = aim - eye;
                let want_yaw = (-to.x).atan2(-to.z) + bot.wobble.0.x;
                let want_pitch = to.y.atan2(to.xz().length()) + bot.wobble.0.y;
                let step = cfg.turn_rate.to_radians() * dt;
                let dyaw = wrap(want_yaw - intent.yaw);
                intent.yaw = wrap(intent.yaw + dyaw.clamp(-step, step));
                intent.pitch += (want_pitch - intent.pitch).clamp(-step, step);
                let off = wrap(want_yaw - intent.yaw).abs().max((want_pitch - intent.pitch).abs());
                intent.fire = cfg.dont_shoot == 0
                    && bot.seen >= cfg.reaction
                    && off.to_degrees() < FIRE_CONE_DEG
                    && !(semi && pressed);
            }
            None => {
                bot.target = None;
                bot.seen = 0.0;
            }
        }

        // A throw in progress takes over.
        if bot.toss.is_some() {
            bot.activity = Activity::Throwing;
            match inv.as_deref_mut() {
                Some(inv) => {
                    grenades::step_toss(&mut bot, &mut intent, inv, &arms, &cfg, now, dt);
                }
                None => bot.toss = None,
            }
            continue;
        }
        // Not throwing: a grenade in hand goes back for a gun.
        if let Some(inv) = inv.as_deref_mut()
            && inv
                .active
                .is_some_and(|a| arms.get(a).is_ok_and(|(_, t)| t.is_some_and(|t| !t.primed())))
            && inv.wanted.is_none()
            && let Some(gun) = grenades::best_weapon(inv, &arms)
        {
            inv.wanted = Some(gun);
            intent.fire = false;
        }
        // Looking away from a flash: turn the back to it and wait.
        if let Some(a) = bot.avert {
            let away = eye - a.from;
            let want_yaw = (-away.x).atan2(-away.z);
            let step = cfg.turn_rate.to_radians() * dt;
            intent.yaw = wrap(intent.yaw + wrap(want_yaw - intent.yaw).clamp(-step, step));
            intent.pitch -= intent.pitch.clamp(-step, step);
            intent.fire = false;
            intent.move_axis = Vec2::ZERO;
            intent.jump = false;
            bot.activity = Activity::Averting;
            continue;
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
        if bot.target.is_some() {
            bot.route.clear();
            bot.activity = Activity::Fighting;
            continue;
        }
        let feet = t.translation - Vec3::Y * CAPSULE_HEIGHT / 2.0;
        // What was seen or heard goes stale, or is where it went.
        if bot
            .lead
            .is_some_and(|(at, when)| now - when > MEMORY || at.distance(feet) < ARRIVED)
        {
            bot.lead = None;
        }
        if let Some((_, when)) = bot.lead {
            bot.contact = bot.contact.max(when);
        }
        if bot
            .assist
            .is_some_and(|(at, when)| now - when > ASSIST_TIME || at.distance(feet) < ARRIVED * 2.0)
        {
            bot.assist = None;
        }
        if bot.roam.is_some_and(|at| at.distance(feet) < ARRIVED) {
            bot.roam = None;
        }
        if cfg.stop != 0 {
            bot.route.clear();
            bot.goal = None;
            bot.activity = Activity::Idle;
            continue;
        }
        // Without a mesh nothing walks, but throws still happen.
        let nav = nav.as_deref().unwrap_or(&no_nav);
        if let Some(here) = nav.area_at(feet + Vec3::Y * 0.1)
            && bot.area != Some(here)
        {
            bot.prev_area = bot.area;
            bot.area = Some(here);
            bot.entered = feet;
        }
        let plan = tactics.team(*team);
        let mates: Vec<Vec3> = others
            .iter()
            .filter(|(e, _, oteam, oh)| *e != me && *oteam == team && oh.current > 0.0)
            .map(|(_, ot, ..)| ot.translation - Vec3::Y * CAPSULE_HEIGHT / 2.0)
            .collect();
        let goal = choose_goal(&mut bot, me, feet, now, &tactics, plan, nav, &mates);
        bot.goal = goal;
        // Now and then a grenade at the remembered enemy or the site.
        if let Some(inv) = inv.as_deref()
            && cfg.grenades > 0
            && now >= bot.next_toss_check
        {
            bot.next_toss_check = now + grenades::TOSS_CHECK;
            if now >= bot.next_toss {
                // Gathering before the site: flashes and smokes onto where
                // the group comes onto it.
                let staging = plan.is_some_and(|p| p.staging && p.site == bot.site);
                let site = bot.site.and_then(|s| tactics.sites.get(s));
                let objective = match bot.activity {
                    _ if staging => site.and_then(|s| {
                        s.approaches[0]
                            .iter()
                            .copied()
                            .min_by(|a, b| a.distance(feet).total_cmp(&b.distance(feet)))
                            .or(Some(s.point))
                    }),
                    Activity::ToSite | Activity::Following | Activity::Waiting => site.map(|s| s.point),
                    Activity::Roaming if bot.roam_objective => bot.roam,
                    _ => None,
                };
                consider_throw(
                    &mut bot, inv, &arms, &cfg, eye, feet, objective, staging, now, dt, &box_trace,
                );
            }
        }
        if bot.toss.is_some() {
            intent.move_axis = Vec2::ZERO;
            continue;
        }
        let params = path::CostParams {
            danger: plan.map(|p| p.danger.as_slice()).filter(|d| d.len() == nav.areas.len()),
            exposure: Some(tactics.exposure.as_slice()).filter(|e| e.len() == nav.areas.len()),
            team_seed: plan.map_or(0, |p| p.seed),
            bot_seed: bot.seed,
            stuck: &tactics.failed_links,
        };
        let step = match goal {
            Some(goal) => walk_route(&mut bot, nav, feet, goal, &params, dt, now, state.on_ladder, state.on_ground),
            // Waiting keeps its route (for comparing with the leader's).
            None if bot.activity == Activity::Waiting => None,
            None => {
                bot.route.clear();
                bot.route_areas.clear();
                None
            }
        };
        // Look at what matters (`choose_look`), turning a little slower
        // than in a fight; walk wherever the route goes, whatever the
        // view.
        let looking = intent.look_rotation() * Vec3::NEG_Z;
        let los = |a: Vec3, b: Vec3| world_trace(a, b).is_none() && !smoke.iter().any(|s| s.blocks(a, b));
        let mut look = choose_look(&mut bot, nav, eye, feet, step.map(|s| s.dir), looking, now, &los);
        let mut pitch_limit = LOOK_PITCH;
        if let Some(step) = step {
            if let Some(r) = step.rungs {
                // A ladder: square to it. Up: looking up, pushing in.
                // Down: level (the movement grabs a ladder along the
                // view), pushing in to grab it, then backing off it.
                let up = step.climb.is_some_and(|c| c.y > feet.y);
                look = eye + r.face + Vec3::Y * if up { 6.0 } else { 0.0 };
                pitch_limit = LADDER_PITCH;
            } else if let Some(rung) = step.climb {
                // A climb or drop with no ladder: face it, looking up (or
                // down) the way to go.
                look = rung + Vec3::Y * (rung.y - feet.y).signum() * 2.0;
                pitch_limit = LADDER_PITCH;
            } else if bot.blocked > USE_AFTER {
                // Stuck: face the way (a door to open, a ledge to jump).
                look = eye + step.dir * 2.0;
            }
        }
        bot.look_at = Some(look);
        let to = look - eye;
        let want_yaw = (-to.x).atan2(-to.z);
        let want_pitch = to.y.atan2(to.xz().length()).clamp(-pitch_limit, pitch_limit);
        let turn = cfg.turn_rate.to_radians() * SCAN_TURN * dt;
        intent.yaw = wrap(intent.yaw + wrap(want_yaw - intent.yaw).clamp(-turn, turn));
        intent.pitch += (want_pitch - intent.pitch).clamp(-turn, turn);
        if let Some(step) = step {
            // Keep off teammates and other teamless characters
            // (hostages) close by (they block each other in doorways
            // otherwise).
            let mut dir = step.dir;
            let near = others
                .iter()
                .filter(|(e, _, oteam, oh)| *e != me && *oteam == team && oh.current > 0.0)
                .map(|(_, ot, ..)| ot)
                .chain(teamless.iter().filter(|(_, h)| h.current > 0.0).map(|(t, _)| t));
            for ot in near {
                let away = (t.translation - ot.translation).with_y(0.0);
                let d = away.length();
                if d < PERSONAL_SPACE && (ot.translation.y - t.translation.y).abs() < 1.5 {
                    let push = if d > 1e-3 { away / d } else { Vec3::X };
                    dir += push * (1.0 - d / PERSONAL_SPACE) * SEPARATION;
                }
            }
            let dir = dir.with_y(0.0).normalize_or(step.dir);
            let rot = intent.yaw_rotation();
            let (fwd, right) = (rot * Vec3::NEG_Z, rot * Vec3::X);
            let axis = Vec2::new(dir.dot(right), dir.dot(fwd));
            // Full speed whichever way (the larger key fully pressed); on
            // a ladder, straight at it.
            intent.move_axis = match step.rungs {
                // On the ladder: straight on, stepping sideways back to
                // its middle line.
                Some(r) if state.on_ladder => {
                    let perp = Vec3::Y.cross(r.normal);
                    let off = (feet - r.foot).dot(perp);
                    let side = if off.abs() > LADDER_CENTRE {
                        -(perp * off.signum()).dot(right).signum()
                    } else {
                        0.0
                    };
                    let up = step.climb.is_some_and(|c| c.y > feet.y);
                    Vec2::new(side, if up { 1.0 } else { -1.0 })
                }
                _ if step.climb.is_some() && step.rungs.is_none() => Vec2::Y,
                _ => axis / axis.abs().max_element().max(1e-3),
            };
            // Walking into something: press use now and then (doors),
            // jump it every so often (ledges).
            if velocity.0.xz().length() < BLOCKED_SPEED {
                bot.blocked += dt;
            } else {
                bot.blocked = 0.0;
            }
            let b = bot.blocked;
            intent.use_key = b > USE_AFTER && (b / USE_PERIOD) as u32 % 2 == 0;
            let blocked = b > BLOCKED_JUMP && (b - BLOCKED_JUMP) % JUMP_PERIOD < dt * 1.5;
            // Jump is a fresh press each time (held, it jumps once), and
            // never on a ladder (that lets go of it).
            let want_jump = ((step.jump && step.climb.is_none()) || blocked) && !state.on_ladder;
            intent.jump = want_jump && !bot.jump_held;
            bot.jump_held = intent.jump;
            if intent.jump && state.on_ground {
                bot.jumped = now;
            }
            // Duck in the air after a jump (a duck-jump clears higher
            // ledges, as CS:S bots do), and under a low ceiling ahead
            // (vents).
            let airborne = !state.on_ground && now - bot.jumped > 0.05 && now - bot.jumped < DUCK_JUMP;
            let ahead = feet + step.dir * LOW_AHEAD;
            let low = |from: Vec3| {
                // Not from inside something (a stair ahead).
                spatial
                    .cast_ray_predicate(
                        from + Vec3::Y * LOW_FROM,
                        Dir3::Y,
                        LOW_CEILING - LOW_FROM,
                        false,
                        &filter,
                        &not_character,
                    )
                    .is_some_and(|h| h.distance > 0.02)
            };
            let low_ceiling = !state.on_ladder && (low(feet) || low(ahead));
            intent.crouch = step.crouch || airborne || low_ceiling;
            // Something breakable in the way (a vent grille, a window):
            // shoot it.
            // How long it has been walking to this route point.
            if let Some(target) = step.target
                && bot.toward.0.distance(target) > 0.3
            {
                bot.toward = (target, now);
            }
            let slow = now - bot.toward.1 > BREAK_SLOW;
            if (b > BREAK_AFTER || slow)
                && cfg.dont_shoot == 0
                && let Some(target) = step.target
            {
                // Along the way at a few heights, and at the route point.
                let ahead = feet + step.dir * 0.6;
                let aims = [
                    ahead + Vec3::Y * 0.1,
                    ahead + Vec3::Y * 0.5,
                    ahead + Vec3::Y * 1.0,
                    ahead + Vec3::Y * 1.5,
                    target + Vec3::Y * 0.3,
                ];
                let filter = SpatialQueryFilter::from_excluded_entities([me]).with_mask(crate::core::SOLID_LAYERS);
                let hit = aims.into_iter().find_map(|aim| {
                    let dir = Dir3::new(aim - eye).ok()?;
                    spatial
                        .cast_ray(eye, dir, BREAK_REACH, true, &filter)
                        .filter(|h| breakable.contains(h.entity))
                        .map(|h| eye + *dir * h.distance)
                });
                if let Some(at) = hit {
                    let d = at - eye;
                    intent.yaw = (-d.x).atan2(-d.z);
                    intent.pitch = d.y.atan2(d.xz().length());
                    intent.fire = !(semi && pressed);
                    bot.look_at = Some(at);
                }
            }
        } else {
            bot.blocked = 0.0;
            intent.use_key = false;
        }
    }
}

/// Seconds a bot answers a teammate's call.
const ASSIST_TIME: f64 = 20.0;
/// Teammates closer than this push a walking bot aside, this hard, m.
const PERSONAL_SPACE: f32 = 1.2;
const SEPARATION: f32 = 1.5;
/// Seconds after a jump it ducks while in the air.
const DUCK_JUMP: f64 = 0.8;
/// A ceiling lower than this above the feet, here or this far ahead,
/// makes a walking bot crouch (traced from `LOW_FROM` up), m.
const LOW_CEILING: f32 = 73.0 * 0.0254;
const LOW_AHEAD: f32 = 0.6;
const LOW_FROM: f32 = 0.5;
/// Blocked this long: shoot a breakable within this in the way, s and m.
const BREAK_AFTER: f32 = 0.5;
/// Walking to one route point this long also looks for one, s.
const BREAK_SLOW: f64 = 2.0;
const BREAK_REACH: f32 = 2.5;
/// Slower than this while walking for this long: jump, m/s and s.
const BLOCKED_SPEED: f32 = 0.5;
const BLOCKED_JUMP: f32 = 0.4;
const JUMP_PERIOD: f32 = 0.8;
/// Blocked this long: face the way and press use every other
/// `USE_PERIOD` (doors open on a press), s.
const USE_AFTER: f32 = 0.25;
const USE_PERIOD: f32 = 0.15;
/// A route point at least this much above or below, this close (flat),
/// is up or down a ladder, m.
const LADDER_RISE: f32 = 1.2;
const LADDER_REACH: f32 = 1.5;
/// Going up a ladder starts from in front of it: the direction to its
/// foot within this (cosine) of straight at it; else to the point this
/// far out in front, m (spec: 0.9, 2 × half hull).
const LADDER_LINED_UP: f32 = -0.9;
const LADDER_MOUNT: f32 = 32.0 * 0.0254;
/// Going down, it backs out over the ladder's top until this far beyond
/// it, then presses in (falling past it, it catches it), m.
const LADDER_OVER: f32 = 0.45;
/// Pitch limit looking up or down a ladder (radians).
const LADDER_PITCH: f32 = 1.4;
/// On a ladder further than this from its middle line: step sideways, m.
const LADDER_CENTRE: f32 = 0.1;
/// Seconds without contact a bot holds before moving on: attackers on a
/// site they took go to the next one, defenders go hunting.
const ATTACK_PATIENCE: (f64, f64) = (20.0, 35.0);
const DEFEND_PATIENCE: (f64, f64) = (45.0, 80.0);
/// Attackers chase what they saw or heard within this; holders within
/// this of their spot, m.
const CHASE_ATTACK: f32 = 25.0;
const CHASE_HOLD: f32 = 15.0;
/// A follower farther than this from the leader goes to the leader; one
/// with this much less route left than the leader, and this far from it,
/// waits, m.
const FOLLOW_FAR: f32 = 10.0;
const AHEAD: f32 = 4.0;
const AHEAD_DIST: f32 = 5.0;
/// Gathering before the site, followers come this close to the leader, m.
const STAGE_CLOSE: f32 = 3.5;
/// At its hold spot within this, m.
const HOLD_REACHED: f32 = 0.75;
/// Within this of its spot, a bot holds where it is when a teammate
/// stands within `HOLD_TAKEN` of the spot or it has been blocked for
/// `HOLD_BLOCKED` seconds (bots shoulder each other otherwise), m.
const HOLD_SHARE: f32 = 2.5;
const HOLD_TAKEN: f32 = 1.0;
const HOLD_BLOCKED: f32 = 1.0;
/// Looks at a remembered enemy within this for this long, m and s.
const LEAD_LOOK: (f32, f64) = (40.0, 8.0);
/// Looks this far ahead along its route with nothing else to look at, m.
const LOOK_AHEAD: f32 = 6.0;
/// The view stays within this of the walking direction (radians).
const LOOK_SPREAD: f32 = 2.0;
/// Pitch limit when not fighting (radians), and the turn rate's share.
const LOOK_PITCH: f32 = 0.45;
const SCAN_TURN: f32 = 0.6;
/// Eye height looked at on watched approaches and the route, m.
const WATCH_LIFT: f32 = 1.3;

/// Where a bot goes with nobody in sight, by its orders (None: stay),
/// setting its `activity`.
#[allow(clippy::too_many_arguments)]
fn choose_goal(
    bot: &mut Bot,
    me: Entity,
    feet: Vec3,
    now: f64,
    tactics: &Tactics,
    plan: Option<&tactics::TeamPlan>,
    nav: &NavMesh,
    mates: &[Vec3],
) -> Option<Vec3> {
    if let Some(at) = bot.move_to {
        bot.activity = Activity::Sent;
        return ((at - feet).xz().length() > ARRIVED * 0.5 || (at.y - feet.y).abs() > 1.0).then_some(at);
    }
    // The bomb to plant or defuse comes first (`objectives`).
    if let Some(at) = bot.objective {
        bot.activity = Activity::ToSite;
        return Some(at);
    }
    let role = bot.orders.role;
    let site = bot.site.and_then(|s| tactics.sites.get(s));
    // A remembered enemy, if the role lets it go there.
    if let Some((at, _)) = bot.lead {
        let chase = match (role, &bot.hold) {
            _ if bot.hunting || site.is_none() => true,
            (Role::Roam, _) => true,
            (_, Some(h)) => at.distance(h.spot) < CHASE_HOLD,
            (Role::Attack, None) => at.distance(feet) < CHASE_ATTACK,
            (Role::Defend, None) => true,
        };
        if chase {
            bot.activity = Activity::Chasing;
            return Some(at);
        }
    }
    if let Some((at, _)) = bot.assist {
        bot.activity = Activity::Assisting;
        return Some(at);
    }
    let Some(site) = site.filter(|_| !bot.hunting && role != Role::Roam) else {
        // Roam: mostly to the objectives.
        if bot.roam.is_none() && !nav.areas.is_empty() {
            let objectives = &tactics.objectives;
            let at_objective = !objectives.is_empty() && bot.rand() < OBJECTIVE_SHARE;
            bot.roam = Some(if at_objective {
                let k = (bot.rand() * objectives.len() as f32) as usize % objectives.len();
                objectives[k]
            } else {
                let i = (bot.rand() * nav.areas.len() as f32) as usize % nav.areas.len();
                nav.areas[i].center
            });
            bot.roam_objective = at_objective;
            bot.repath = 0.0;
        }
        bot.activity = if bot.hunting {
            Activity::Hunting
        } else {
            Activity::Roaming
        };
        return bot.roam;
    };
    if bot.hold.is_none() {
        if role == Role::Attack && feet.distance(site.point) >= tactics::ARRIVE_RADIUS {
            // On the way, as a group.
            if let Some(plan) = plan.filter(|p| p.site == bot.site && p.leader.is_some()) {
                if plan.leader == Some(me) {
                    if plan.leader_wait {
                        bot.activity = Activity::Waiting;
                        return None;
                    }
                } else if let Some(lf) = plan.leader_feet {
                    let d = lf.distance(feet);
                    if plan.staging {
                        // Gathering: close to the leader, then wait.
                        if d > STAGE_CLOSE {
                            bot.activity = Activity::Following;
                            return Some(lf);
                        }
                        bot.activity = Activity::Waiting;
                        return None;
                    }
                    if d > FOLLOW_FAR {
                        bot.activity = Activity::Following;
                        return Some(lf);
                    }
                    if d > AHEAD_DIST && !bot.route.is_empty() && bot.route_left() + AHEAD < plan.leader_left {
                        bot.activity = Activity::Waiting;
                        return None;
                    }
                }
            }
            bot.activity = Activity::ToSite;
            return Some(site.point);
        }
        // There (or a defender with no spot): hold somewhere on it.
        let rank = bot.orders.rank;
        bot.hold = tactics::attacker_hold(site, rank)
            .filter(|_| role == Role::Attack)
            .or(Some(Hold {
                spot: site.point,
                watch: site.approaches[0].clone(),
            }));
        bot.hold_since = None;
    }
    let spot = bot.hold.as_ref().map(|h| h.spot)?;
    let reach = if bot.hold_since.is_some() {
        HOLD_REACHED * 3.0
    } else {
        HOLD_REACHED
    };
    if (spot - feet).xz().length() > reach || (spot.y - feet.y).abs() > 1.5 {
        // Nearly there, but a teammate stands on the spot or keeps it
        // from being reached: hold from here instead.
        let close = (spot - feet).xz().length() < HOLD_SHARE && (spot.y - feet.y).abs() < 1.0;
        let taken = mates.iter().any(|m| (*m - spot).xz().length() < HOLD_TAKEN);
        if close && (taken || bot.blocked > HOLD_BLOCKED) {
            if let Some(h) = bot.hold.as_mut() {
                h.spot = feet;
            }
        } else {
            bot.activity = Activity::ToHold;
            return Some(spot);
        }
    }
    let since = *bot.hold_since.get_or_insert(now);
    if now - since.max(bot.contact) > bot.patience {
        // Nothing happens here: move on.
        bot.hold = None;
        bot.hold_since = None;
        if role == Role::Attack && !tactics.sites.is_empty() {
            bot.site = bot.site.map(|s| (s + 1) % tactics.sites.len());
        } else {
            bot.hunting = true;
        }
        bot.contact = now;
    }
    bot.activity = Activity::Holding;
    None
}

/// Where a bot looks with nobody in sight: a remembered enemy nearby;
/// holding, the approaches it watches (switching now and then); else a
/// hiding spot coming into view (`look::corner`), else a few meters
/// ahead along its route, kept within `LOOK_SPREAD` of the walking
/// direction.
#[allow(clippy::too_many_arguments)]
fn choose_look(
    bot: &mut Bot,
    nav: &NavMesh,
    eye: Vec3,
    feet: Vec3,
    walk: Option<Vec3>,
    looking: Vec3,
    now: f64,
    los: &dyn Fn(Vec3, Vec3) -> bool,
) -> Vec3 {
    let ahead = |bot: &Bot| -> Option<Vec3> {
        let mut left = LOOK_AHEAD;
        let mut at = feet;
        for p in bot.route.iter().skip(bot.next) {
            let d = at.distance(*p);
            if d >= left {
                return Some(at + (*p - at) * (left / d) + Vec3::Y * WATCH_LIFT);
            }
            left -= d;
            at = *p;
        }
        (at != feet).then_some(at + Vec3::Y * WATCH_LIFT)
    };
    let within_spread = |p: Vec3| {
        walk.is_none_or(|w| {
            let d = (p - eye).with_y(0.0).normalize_or_zero();
            d == Vec3::ZERO || d.angle_between(w) <= LOOK_SPREAD
        })
    };
    if let Some((at, when)) = bot.lead
        && now - when < LEAD_LOOK.1
        && at.distance(feet) < LEAD_LOOK.0
    {
        let p = at + Vec3::Y * WATCH_LIFT;
        if within_spread(p) {
            return p;
        }
    }
    if bot.activity == Activity::Holding
        && let Some(n) = bot.hold.as_ref().map(|h| h.watch.len())
        && n > 0
    {
        if now >= bot.watch.1 {
            // Mostly the nearest approach.
            let i = if bot.rand() < 0.6 {
                0
            } else {
                (bot.rand() * n as f32) as usize % n
            };
            bot.watch = (i, now + 2.5 + 3.0 * bot.rand() as f64);
        }
        let h = bot.hold.as_ref().unwrap();
        return h.watch[bot.watch.0 % n] + Vec3::Y * WATCH_LIFT;
    }
    if let Some(spot) = look::corner(bot, nav, eye, feet, walk, looking, now, los) {
        let p = spot + Vec3::Y * look::SPOT_LIFT;
        if within_spread(p) {
            return p;
        }
    }
    if let Some(p) = ahead(bot) {
        return p;
    }
    match walk {
        Some(w) => eye + w * LOOK_AHEAD,
        None => eye + looking * LOOK_AHEAD,
    }
}

/// Aim error grows by this factor times the flash's whiteness (below the
/// blind threshold).
const FLASHED_AIM: f32 = 6.0;

/// Maybe start a grenade throw: at the remembered enemy (an HE or a
/// flash), else at the objective being walked to (a smoke or a flash),
/// if an arc ends close enough to it.
#[allow(clippy::too_many_arguments)]
fn consider_throw(
    bot: &mut Bot,
    inv: &Inventory,
    arms: &grenades::Arms,
    cfg: &BotConfig,
    eye: Vec3,
    feet: Vec3,
    objective: Option<Vec3>,
    staging: bool,
    now: f64,
    dt: f32,
    sweep: &dyn Fn(Vec3, Vec3, f32) -> Option<(Vec3, Vec3)>,
) {
    use crate::weapon::grenade::GrenadeKind;
    let held = grenades::held(inv, arms);
    if held.is_empty() {
        return;
    }
    let always = cfg.grenades >= 2;
    let in_range = |at: Vec3| {
        let d = (at - feet).xz().length();
        d >= grenades::TOSS_RANGE.0 && d <= grenades::TOSS_RANGE.1
    };
    let (target, kinds) = match (bot.lead, objective) {
        (Some((at, _)), _) if in_range(at) => {
            if !(always || bot.rand() < grenades::TOSS_CHANCE_LEAD) {
                return;
            }
            // Mostly an HE when there's a choice.
            if bot.rand() < 0.65 {
                (at, [GrenadeKind::Blast, GrenadeKind::Flash])
            } else {
                (at, [GrenadeKind::Flash, GrenadeKind::Blast])
            }
        }
        (_, Some(at)) if staging && in_range(at) => {
            if !(always || bot.rand() < grenades::TOSS_CHANCE_STAGING) {
                return;
            }
            (at, [GrenadeKind::Flash, GrenadeKind::Smoke])
        }
        (None, Some(at)) if in_range(at) => {
            if !(always || bot.rand() < grenades::TOSS_CHANCE_OBJECTIVE) {
                return;
            }
            (at, [GrenadeKind::Smoke, GrenadeKind::Flash])
        }
        _ => return,
    };
    let Some((weapon, t)) = kinds
        .iter()
        .find_map(|k| held.iter().find(|(_, t)| t.effect.kind() == *k))
    else {
        return;
    };
    let kind = t.effect.kind();
    let plan = grenades::plan_throw(
        kind,
        &t.throw,
        &t.flight,
        grenades::fuse_time(t.fuse, t.flight.check_interval, dt),
        kind == GrenadeKind::Smoke,
        eye,
        target + Vec3::Y * grenades::aim_lift(kind),
        &mut |a, b| sweep(a, b, t.flight.half),
    );
    match plan {
        Some(plan) if plan.error <= grenades::tolerance(kind) => {
            let (lo, hi) = grenades::TOSS_COOLDOWN;
            bot.next_toss = now + lo + (hi - lo) * bot.rand() as f64;
            bot.plan = Some((plan.clone(), now + grenades::PLAN_SHOWN));
            bot.toss = Some(grenades::Toss {
                weapon: *weapon,
                previous: inv.active,
                plan,
                started: now,
                pulled: None,
                released: false,
            });
        }
        // No good arc from here: look again in a while.
        _ => bot.next_toss = now + 2.0,
    }
}

/// A route point counts as reached within this distance (2D), m.
const REACHED: f32 = 25.0 * 0.0254;
/// Seconds between route recomputations while walking.
const REPATH_SECONDS: f32 = 1.0;
/// Without this much progress in `STUCK_SECONDS`, repath (and jump).
const STUCK_DISTANCE: f32 = 0.5;
const STUCK_SECONDS: f32 = 1.5;
/// That many times in a row: the link it is on costs more for
/// `STUCK_MEMORY` seconds (`path::STUCK_PENALTY`).
const STUCK_MARK: u32 = 2;
const STUCK_MEMORY: f64 = 25.0;

/// The nav link a bot is trying to cross: from the area it stands in to
/// the next different area its route leads into.
fn stuck_link(bot: &Bot, nav: &NavMesh, feet: Vec3) -> Option<(usize, usize)> {
    let from = nav.area_at(feet + Vec3::Y * 0.1).or(bot.area)?;
    let to = bot.route_areas.iter().skip(bot.next).find(|a| **a != from)?;
    nav.areas[from].links.iter().any(|l| l.0 == *to).then_some((from, *to))
}

/// Crossing points on edges wider than this lean aside, up to this share
/// of the way to an end, per bot and edge (spreads a group out), m.
const LEAN_WIDTH: f32 = 2.0;
const LEAN: f32 = 0.45;

/// One tick's walking: direction (flat, unit), jump and crouch.
#[derive(Clone, Copy, Debug)]
struct Step {
    dir: Vec3,
    /// The route point walked to.
    target: Option<Vec3>,
    jump: bool,
    crouch: bool,
    /// The point up or down a ladder it climbs to.
    climb: Option<Vec3>,
    /// The mesh's ladder it climbs.
    rungs: Option<Rungs>,
}

/// A ladder being climbed: its foot (bottom going up, top going down),
/// flat outward normal, and the way to face.
#[derive(Clone, Copy, Debug)]
struct Rungs {
    foot: Vec3,
    normal: Vec3,
    face: Vec3,
}

/// Walk the navigation mesh toward `goal` (feet positions) by the bot's
/// path cost: the direction to the next route point.
#[allow(clippy::too_many_arguments)]
fn walk_route(
    bot: &mut Bot,
    nav: &NavMesh,
    feet: Vec3,
    goal: Vec3,
    params: &path::CostParams,
    dt: f32,
    now: f64,
    on_ladder: bool,
    on_ground: bool,
) -> Option<Step> {
    // Mid-ladder or in the air the route stays (a new one would start
    // from the area below or above, back at a ladder's foot or top),
    // unless it is stuck there.
    if on_ground && !on_ladder {
        bot.repath -= dt;
    }
    bot.progress.1 += dt;
    let mut stuck = false;
    if bot.progress.1 >= STUCK_SECONDS {
        stuck = feet.distance(bot.progress.0) < STUCK_DISTANCE;
        bot.progress = (feet, 0.0);
        if stuck {
            bot.stuck_count += 1;
            if bot.stuck_count >= STUCK_MARK
                && let Some(link) = stuck_link(bot, nav, feet)
            {
                // Still stuck after a repath and a jump: avoid the link
                // it keeps failing for a while.
                debug!("bot stuck at {feet:.1} on link {link:?}: walking around it");
                bot.stuck_links.retain(|l| l.0 != link);
                bot.stuck_links.push((link, now + STUCK_MEMORY));
                bot.stuck_count = 0;
            }
        } else {
            bot.stuck_count = 0;
        }
    }
    bot.stuck_links.retain(|l| l.1 > now);
    if bot.repath <= 0.0 || bot.next >= bot.route.len() || stuck {
        bot.repath = REPATH_SECONDS;
        let seed = bot.seed;
        let links: Vec<(usize, usize)> = bot
            .stuck_links
            .iter()
            .map(|l| l.0)
            .chain(params.stuck.iter().copied())
            .collect();
        let params = &path::CostParams { stuck: &links, ..*params };
        let found = nav
            .nearest_area(feet)
            .zip(nav.nearest_area(goal))
            .and_then(|(a, b)| nav.find_path_with(a, b, |from, to, via| path::step_cost(nav, from, to, via, params)));
        let points = found.map(|(areas, _)| {
            nav.points_along(&areas, feet, goal, |from, to, width| {
                if width < LEAN_WIDTH {
                    0.0
                } else {
                    (path::hash01(seed, from * 7919 + to) as f32 * 2.0 - 1.0) * LEAN
                }
            })
        });
        (bot.route, bot.route_areas) = points.unwrap_or_default().into_iter().unzip();
        bot.next = 0;
        if on_ladder {
            // Already on it: skip the point at its foot or top.
            while bot.next + 1 < bot.route.len() {
                let (a, b) = (bot.route[bot.next], bot.route[bot.next + 1]);
                let mount = (b.y - a.y).abs() > LADDER_RISE && (b - a).xz().length() < LADDER_REACH;
                if mount && (a - feet).xz().length() < LADDER_REACH {
                    bot.next += 1;
                } else {
                    break;
                }
            }
        }
    }
    while bot.next < bot.route.len()
        && (bot.route[bot.next] - feet).xz().length() < REACHED
        && (bot.route[bot.next].y - feet.y).abs() < LADDER_RISE
    {
        bot.next += 1;
    }
    let point = *bot.route.get(bot.next)?;
    let to = point - feet;
    // Up or down a ladder: both ends near one of the mesh's ladders.
    let near_ladder = |p: Vec3| {
        nav.ladders.iter().any(|l| {
            (p.xz() - l.bottom.xz()).length() < LADDER_REACH
                && p.y > l.bottom.y.min(l.top.y) - 1.0
                && p.y < l.bottom.y.max(l.top.y) + 1.0
        })
    };
    let ladder = to.y.abs() > STEP_HEIGHT
        && to.xz().length() < LADDER_REACH
        && (to.y.abs() > LADDER_RISE || (near_ladder(point) && near_ladder(feet)));
    let mut step = Step {
        target: Some(point),
        climb: ladder.then_some(point),
        dir: to.with_y(0.0).normalize_or_zero(),
        // Jump up ledges higher than a step, and when stuck.
        jump: stuck || (to.y > STEP_HEIGHT && to.xz().length() < 1.5),
        crouch: nav.area_at(feet).is_some_and(|a| nav.areas[a].has(flags::CROUCH)),
        rungs: None,
    };
    // One of the mesh's ladders (the route goes to its top or foot): get
    // on it from the front, square to it (spec "Path following", ladder
    // handling), and stay on its middle.
    let up = to.y > 0.0;
    let mesh_ladder = nav
        .ladders
        .iter()
        .find(|l| if up { l.top } else { l.bottom }.distance(point) < 0.05);
    if ladder && let Some(l) = mesh_ladder {
        let n = l.normal.with_y(0.0).normalize_or_zero();
        let foot = if up { l.bottom } else { l.top };
        let to_foot = (foot - feet).with_y(0.0);
        if up && !on_ladder {
            let lined_up = to_foot.length() < 0.1 || to_foot.normalize().dot(n) < LADDER_LINED_UP;
            if !lined_up {
                // Round to the front first.
                let mount = l.bottom + n * LADDER_MOUNT;
                step.climb = None;
                step.jump = false;
                step.target = Some(mount);
                step.dir = (mount - feet).with_y(0.0).normalize_or(-n);
                return Some(step);
            }
        }
        // Always facing it. Up: walk at its foot. Down: back out over
        // its top, and once past it (falling) press in to catch it.
        step.dir = if up {
            if to_foot.length() > 0.1 { to_foot.normalize() } else { -n }
        } else {
            let beyond = (feet - foot).dot(n);
            let side = (feet - foot).with_y(0.0) - n * beyond;
            let back = if beyond > LADDER_OVER || !on_ground { -n } else { n };
            // Square to its middle on the way.
            (back - side * 2.0).normalize_or(back)
        };
        step.jump = false;
        step.rungs = Some(Rungs {
            foot,
            normal: n,
            face: -n,
        });
    }
    Some(step)
}

fn wrap(a: f32) -> f32 {
    (a + std::f32::consts::PI).rem_euclid(std::f32::consts::TAU) - std::f32::consts::PI
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bot::path::tests::two_ways;

    /// A bot that keeps failing to cross a link walks around it, and
    /// goes back to the short way once the memory runs out.
    #[test]
    fn stuck_bot_repaths_around_the_link() {
        let nav = two_ways(0);
        let mut bot = Bot::default();
        let params = path::CostParams::default();
        // Standing in area 1 (x 4..8, z 0..4), going to area 3's middle.
        let feet = Vec3::new(7.0, 0.0, 2.0);
        let goal = nav.areas[3].center;
        let dt = 1.0 / 60.0;
        let mut now = 0.0;
        walk_route(&mut bot, &nav, feet, goal, &params, dt, now, false, true);
        assert!(
            bot.route_areas.contains(&3) && !bot.route_areas.contains(&5),
            "{:?}",
            bot.route_areas
        );
        // Not moving at all (a wall the mesh doesn't know about).
        let mut jumped = false;
        while now < 3.0 * STUCK_SECONDS as f64 + 0.1 {
            now += dt as f64;
            let step = walk_route(&mut bot, &nav, feet, goal, &params, dt, now, false, true).unwrap();
            jumped |= step.jump;
        }
        assert!(jumped, "a stuck bot jumps first");
        assert_eq!(bot.stuck_links.iter().map(|l| l.0).collect::<Vec<_>>(), [(1, 3)]);
        // The new route goes round through areas 2 and 5.
        assert!(
            bot.route_areas.contains(&5) && bot.route_areas.contains(&2),
            "{:?}",
            bot.route_areas
        );
        // Later it is forgotten.
        now += STUCK_MEMORY + 1.0;
        bot.repath = 0.0;
        bot.progress = (Vec3::splat(100.0), 0.0);
        walk_route(&mut bot, &nav, feet, goal, &params, dt, now, false, true);
        assert!(bot.stuck_links.is_empty());
        assert!(!bot.route_areas.contains(&5), "{:?}", bot.route_areas);
    }
}
