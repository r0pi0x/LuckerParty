//! A network soak (docs/plans/active/multiplayer.md, "Soak"): a server
//! with four clients and bots filling to twelve plays rounds on de_dust2,
//! the greybox, a community map from the content cache (if there is one)
//! and de_dust2 again, over `NetSim`'s in-memory link with each client on
//! its own link (0/50/150/300 ms round trips, 0-5 % loss). Scripted
//! players walk, jump, duck, shoot at whoever they see, buy, throw
//! grenades, drop guns (and walk over others'), plant and defuse the bomb,
//! chat, call on the radio and change names; one leaves mid-game and
//! another joins. Checked all along:
//!
//! - nothing panics;
//! - what clients hear of others (`NetBody` snapshots) is what the server
//!   had at that tick;
//! - health, team, score, name, the round, and each client's own money,
//!   armour and weapons with their ammo are the server's once both sides
//!   have held them still for a while (settled);
//! - prediction errors per client stay rare (server teleports, which the
//!   test makes for planting and defusing, excused);
//! - entity counts per kind stay level from round to round and from one
//!   visit of de_dust2 to the next, on the server and on every client;
//! - memory (the process's resident size), bandwidth per client and the
//!   server's frame time stay level.
//!
//! `MASHUP_SOAK_MINUTES` sets the game time (default 3; 30 for the long
//! run), `MASHUP_SOAK_COMMUNITY` the community map. A summary table is
//! printed: `MASHUP_SOAK_MINUTES=30 cargo test --release --features dev
//! --test it heavy::net_soak -- --nocapture`. Skipped without a CS:S
//! install.

use std::{
    collections::{BTreeMap, HashMap},
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use avian3d::prelude::Position;
use bevy::{
    ecs::message::{MessageCursor, Messages},
    prelude::*,
};
use bevy_replicon::shared::server_entity_map::ServerEntityMap;
use mashup::{
    bot::Tactics,
    console::Console,
    core::{Connecting, Health, Intent, LocalPlayer, MovementState, SimClock, Team},
    games::{
        self,
        cs_source::{
            self,
            movement::{self as source, SourceMovementPlugin},
            weapons::CsWeaponsPlugin,
        },
    },
    greybox::GreyboxMapPlugin,
    harness::NetSim,
    map::{MapData, MapPlugin},
    mount::config::{LocalConfig, content_dir},
    net::{
        NetBody, NetCharacter, NetEvent, NetItem, NetProp, NetRound, NetScore, NetSettings, RadioRequest,
        client::Joined, interp::Snapshots, maps::MapFiles, memory::LinkConditions, predict::NetGraph,
    },
    objectives::bomb::BombState,
    rules::{
        Dead,
        rounds::{Phase, RoundState},
    },
    slots::Loadout,
    weapon::{Armor, Inventory, Magazine, Weapon, economy::Money},
};

const DUST2: &str = "cs_source:de_dust2";
const GREYBOX: &str = "greybox";
/// Steps a value must have held still on both sides to count as settled.
const SETTLE: u64 = 160;
/// Steps between agreement checks.
const CHECK_EVERY: u64 = 8;
/// Steps after a server teleport whose prediction errors are excused.
const EXCUSE: u64 = 200;

fn installed() -> bool {
    let ok = LocalConfig::load()
        .ok()
        .and_then(|c| c.game_path(cs_source::GAME))
        .is_some_and(|p| p.join("cstrike").is_dir());
    if !ok {
        eprintln!("skipping: no CS:S install configured");
    }
    ok
}

/// Maps loaded once and handed out as copies (five worlds load each).
#[derive(Resource, Clone, Default)]
struct MapCache(Arc<Mutex<HashMap<String, MapData>>>);

impl MapCache {
    fn get(&self, id: &str) -> MapData {
        let mut maps = self.0.lock().unwrap();
        maps.entry(id.to_string())
            .or_insert_with(|| games::load_map(id).unwrap_or_else(|e| panic!("{id}: {e}")))
            .clone()
    }
}

fn tick_of(id: &str) -> Duration {
    if id == GREYBOX {
        Duration::from_secs_f64(1.0 / mashup::DEFAULT_TICK_HZ)
    } else {
        Duration::from_secs_f64(cs_source::TICK_INTERVAL)
    }
}

/// What the game's client layer does with `NetEvent::LoadMap`, at once.
fn load_requested(world: &mut World, mut cursor: Local<MessageCursor<NetEvent>>) {
    let events: Vec<NetEvent> = cursor.read(world.resource::<Messages<NetEvent>>()).cloned().collect();
    for e in events {
        if let NetEvent::LoadMap { map, .. } = e {
            let data = (map != GREYBOX).then(|| world.resource::<MapCache>().get(&map));
            mashup::swap_map(world, &map, data, tick_of(&map));
        }
    }
}

/// Errors Bevy hands its error handler in any of the soak's worlds
/// (failed commands, systems returning errors): counted with the first
/// few kept, where the game would log a warning.
static ERRORS: Mutex<Vec<String>> = Mutex::new(Vec::new());
static ERROR_COUNT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

fn count_error(error: bevy::ecs::error::BevyError, ctx: bevy::ecs::error::ErrorContext) {
    ERROR_COUNT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let mut kept = ERRORS.lock().unwrap();
    if kept.len() < 20 {
        kept.push(format!("{} {:?}: {error}", ctx.kind(), ctx.name()));
    }
}

/// The community map: `MASHUP_SOAK_COMMUNITY`, else one of a few known
/// to load, else none.
fn community_map() -> Option<String> {
    let dir = content_dir(cs_source::GAME)?.join("maps");
    let has = |n: &str| dir.join(format!("{n}.bsp")).is_file();
    if let Ok(n) = std::env::var("MASHUP_SOAK_COMMUNITY") {
        return has(&n).then(|| format!("cs_source:{n}"));
    }
    ["gg_fy_tactic_fight", "gg_hex", "gg_mr_pillar_v1", "gg_future"]
        .into_iter()
        .find(|n| has(n))
        .map(|n| format!("cs_source:{n}"))
}

/// Resident memory of this process, MB (Linux; None elsewhere).
/// Resident plus swapped-out memory of this process, MB (Linux; None
/// elsewhere): resident alone drops when a busy machine swaps.
fn rss_mb() -> Option<f64> {
    let status = std::fs::read_to_string("/proc/self/status").ok()?;
    let kb = |key: &str| -> Option<f64> {
        status
            .lines()
            .find(|l| l.starts_with(key))?
            .split_whitespace()
            .nth(1)?
            .parse()
            .ok()
    };
    Some((kb("VmRSS:")? + kb("VmSwap:").unwrap_or(0.0)) / 1000.0)
}

struct Rng(u64);

impl Rng {
    fn next(&mut self) -> f64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        (self.0.wrapping_mul(0x2545_F491_4F6C_DD1D) >> 11) as f64 / (1u64 << 53) as f64
    }
    fn chance(&mut self, p: f64) -> bool {
        self.next() < p
    }
    fn pick<'a, T>(&mut self, of: &'a [T]) -> &'a T {
        &of[(self.next() * of.len() as f64) as usize % of.len()]
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Mode {
    Roam,
    Grenade { since: u64 },
    Plant { since: u64 },
    Defuse { since: u64, at: Vec3 },
}

/// A scripted player on one client.
struct Brain {
    id: u64,
    rng: Rng,
    mode: Mode,
    axis: Vec2,
    yaw: f32,
    next_turn: u64,
    crouch_until: u64,
    bought_round: Option<u32>,
    next_grenade: u64,
    next_drop: u64,
    next_say: u64,
    next_radio: u64,
    said: u32,
}

impl Brain {
    fn new(id: u64, now: u64) -> Self {
        Self {
            id,
            rng: Rng(0x9E37_79B9_7F4A_7C15 ^ id.wrapping_mul(0xD1B5_4A32_D192_ED03)),
            mode: Mode::Roam,
            axis: Vec2::ZERO,
            yaw: 0.0,
            next_turn: now,
            crouch_until: 0,
            bought_round: None,
            next_grenade: now + 600,
            next_drop: now + 1500,
            next_say: now + 300 + id * 200,
            next_radio: now + 500 + id * 150,
            said: 0,
        }
    }
}

/// Yaw and pitch looking from `eye` at `at` (the game's convention).
fn aim(eye: Vec3, at: Vec3) -> (f32, f32) {
    let d = at - eye;
    (
        (-d.x).atan2(-d.z).rem_euclid(std::f32::consts::TAU),
        d.y.atan2(d.xz().length()),
    )
}

/// What a client's world shows of its own player and others.
struct View {
    me: Entity,
    eye: Vec3,
    team: u8,
    alive: bool,
    enemies: Vec<Vec3>,
    round: Option<u32>,
    freeze: bool,
}

fn view(world: &mut World) -> Option<View> {
    let me = world.query_filtered::<Entity, With<LocalPlayer>>().iter(world).next()?;
    let team = world.get::<Team>(me)?.0;
    let alive = world.get::<Dead>(me).is_none() && world.get::<Health>(me).is_some_and(|h| h.current > 0.0);
    let eye = world.get::<Transform>(me)?.translation
        + world.get::<MovementState>(me).map_or(Vec3::Y * 0.6, |m| m.eye_offset);
    let enemies = world
        .query::<(Entity, &Team, &Health, &Transform, &NetCharacter)>()
        .iter(world)
        .filter(|(e, t, h, ..)| *e != me && t.0 != team && t.0 != 0 && h.current > 0.0)
        .map(|(_, _, _, tr, _)| tr.translation)
        .collect();
    let round = world.query::<&NetRound>().iter(world).next().map(|r| r.number);
    let freeze = matches!(world.resource::<RoundState>().phase, Phase::Freeze { .. });
    Some(View {
        me,
        eye,
        team,
        alive,
        enemies,
        round,
        freeze,
    })
}

/// One step of a scripted player: its intent, and console lines and radio
/// calls now and then.
fn drive(brain: &mut Brain, world: &mut World, step: u64) {
    let Some(v) = view(world) else { return };
    let b = brain;
    let mut lines: Vec<String> = Vec::new();
    // Buy once a freeze.
    if v.freeze && v.round.is_some() && b.bought_round != v.round {
        b.bought_round = v.round;
        let primaries: &[&str] = if v.team == 1 {
            &["ak47", "galil", "mp5navy", "m3"]
        } else {
            &["m4a1", "famas", "mp5navy", "p90"]
        };
        lines.push(format!("buy {}", b.rng.pick(primaries)));
        for extra in [
            "vesthelm",
            "hegrenade",
            "flashbang",
            "smokegrenade",
            "defuser",
            "deagle",
        ] {
            if b.rng.chance(0.5) {
                lines.push(format!("buy {extra}"));
            }
        }
    }
    if step >= b.next_say {
        b.next_say = step + 900 + (b.rng.next() * 1200.0) as u64;
        b.said += 1;
        let cmd = if b.rng.chance(0.3) { "say_team" } else { "say" };
        lines.push(format!("{cmd} \"soak line {} from {}\"", b.said, b.id));
    }
    if step >= b.next_radio {
        b.next_radio = step + 700 + (b.rng.next() * 900.0) as u64;
        let call = *b
            .rng
            .pick(&["go", "coverme", "enemyspot", "roger", "needbackup", "followme"]);
        world.write_message(RadioRequest { command: call.into() });
    }
    if v.alive && step >= b.next_drop && b.mode == Mode::Roam {
        b.next_drop = step + 1800 + (b.rng.next() * 1800.0) as u64;
        lines.push("drop".into());
    }
    if v.alive && step >= b.next_grenade && b.mode == Mode::Roam && !v.freeze {
        b.next_grenade = step + 900 + (b.rng.next() * 900.0) as u64;
        b.mode = Mode::Grenade { since: step };
    }
    if step >= b.next_turn {
        b.next_turn = step + 30 + (b.rng.next() * 120.0) as u64;
        let a = (b.rng.next() * std::f64::consts::TAU) as f32;
        b.axis = if b.rng.chance(0.1) {
            Vec2::ZERO
        } else {
            Vec2::new(a.cos(), a.sin())
        };
        b.yaw = (b.yaw + (b.rng.next() as f32 - 0.5) * 2.5).rem_euclid(std::f32::consts::TAU);
        if b.rng.chance(0.15) {
            b.crouch_until = step + 40;
        }
    }
    let nearest = v
        .enemies
        .iter()
        .copied()
        .filter(|p| p.distance(v.eye) < 35.0)
        .min_by(|a, c| a.distance(v.eye).total_cmp(&c.distance(v.eye)));
    let jump = b.rng.chance(0.01);
    let fire_roll = b.rng.next();
    let reload = b.rng.chance(0.003);
    let switch = b.rng.chance(0.004).then(|| (b.rng.next() * 3.0) as u8);
    let walk = b.rng.chance(0.2);
    let mode = b.mode;
    let mut done = false;
    {
        let mut i = world.get_mut::<Intent>(v.me).unwrap();
        i.select = None;
        i.use_key = false;
        i.reload = false;
        match mode {
            Mode::Roam => {
                i.move_axis = b.axis;
                i.jump = jump;
                i.crouch = step < b.crouch_until;
                i.walk = walk && i.walk;
                i.reload = reload;
                i.select = switch;
                if let Some(at) = nearest {
                    let (yaw, pitch) = aim(v.eye, at + Vec3::Y * 0.4);
                    i.yaw = yaw;
                    i.pitch = pitch;
                    i.fire = fire_roll < 0.5;
                } else {
                    i.yaw = b.yaw;
                    i.pitch = 0.0;
                    i.fire = fire_roll < 0.01;
                }
            }
            Mode::Grenade { since } => {
                let t = step - since;
                i.move_axis = Vec2::ZERO;
                i.select = (t < 2).then_some(3);
                i.pitch = -0.3;
                i.fire = (40..60).contains(&t);
                done = t > 100;
            }
            Mode::Plant { since } => {
                let t = step - since;
                i.move_axis = Vec2::ZERO;
                i.jump = false;
                i.crouch = false;
                i.select = (t < 2).then_some(4);
                i.pitch = 0.5;
                i.fire = t > 40;
                done = t > 400;
            }
            Mode::Defuse { since, at } => {
                let t = step - since;
                let (yaw, pitch) = aim(v.eye, at + Vec3::Y * 0.05);
                i.move_axis = Vec2::ZERO;
                i.jump = false;
                i.crouch = false;
                i.fire = false;
                i.yaw = yaw;
                i.pitch = pitch;
                i.use_key = t > 10;
                done = t > 800;
            }
        }
    }
    if done || (!v.alive && mode != Mode::Roam) {
        b.mode = Mode::Roam;
        if let Some(mut i) = world.get_mut::<Intent>(v.me) {
            i.fire = false;
            i.use_key = false;
        }
    }
    let mut console = world.resource_mut::<Console>();
    for l in lines {
        console.submit(l);
    }
}

/// A value the server and a client should agree on, as text.
type Key = (Entity, &'static str);

fn weapons_of(world: &World, e: Entity) -> Option<String> {
    let inv = world.get::<Inventory>(e)?;
    let mut w: Vec<String> = inv
        .weapons
        .iter()
        .filter_map(|x| {
            let id = world.get::<Weapon>(*x)?.id;
            let m = world.get::<Magazine>(*x);
            Some(format!(
                "{id}:{}/{}",
                m.map_or(0, |m| m.clip),
                m.map_or(0, |m| m.reserve)
            ))
        })
        .collect();
    w.sort();
    Some(w.join(","))
}

/// A character's values (server entity `key` names it). `own`: the
/// player's own (money, armour, weapons) too.
fn character_values(world: &World, e: Entity, key: Entity, own: bool, out: &mut HashMap<Key, String>) {
    if let Some(h) = world.get::<Health>(e) {
        out.insert((key, "health"), format!("{:.4}", h.current));
    }
    if let Some(t) = world.get::<Team>(e) {
        out.insert((key, "team"), t.0.to_string());
    }
    if let Some(s) = world.get::<NetScore>(e) {
        out.insert((key, "score"), format!("{}/{}", s.kills, s.deaths));
    }
    if let Some(c) = world.get::<NetCharacter>(e) {
        out.insert((key, "name"), c.name.clone());
    }
    if own {
        out.insert((key, "money"), format!("{:?}", world.get::<Money>(e).map(|m| m.0)));
        out.insert(
            (key, "armor"),
            format!("{:?}", world.get::<Armor>(e).map(|a| (a.amount as i32, a.helmet))),
        );
        if let Some(w) = weapons_of(world, e) {
            out.insert((key, "weapons"), w);
        }
    }
}

fn round_values(world: &mut World, out: &mut HashMap<Key, String>) {
    let phase = match world.resource::<RoundState>().phase {
        Phase::Off => "off",
        Phase::Freeze { .. } => "freeze",
        Phase::Live { .. } => "live",
        Phase::Over { .. } => "over",
    };
    out.insert((Entity::PLACEHOLDER, "phase"), phase.into());
    if let Some(r) = world.query::<&NetRound>().iter(world).next() {
        out.insert(
            (Entity::PLACEHOLDER, "round"),
            format!("#{} wins {:?} bomb {}", r.number, r.wins, r.bomb),
        );
    }
}

/// Values held since a step.
#[derive(Default)]
struct Stable(
    HashMap<Key, (String, u64)>,
    HashMap<Key, std::collections::VecDeque<(u64, String)>>,
);

impl Stable {
    fn update(&mut self, now: u64, values: HashMap<Key, String>) {
        self.0.retain(|k, _| values.contains_key(k));
        for (k, v) in values {
            match self.0.get_mut(&k) {
                Some(old) if old.0 == v => {}
                _ => {
                    let log = self.1.entry(k).or_default();
                    log.push_back((now, v.clone()));
                    if log.len() > 6 {
                        log.pop_front();
                    }
                    self.0.insert(k, (v, now));
                }
            }
        }
    }

    fn log(&self, k: &Key) -> String {
        self.1.get(k).map_or(String::new(), |l| format!("{l:?}"))
    }
    fn settled(&self, k: &Key, now: u64) -> Option<&str> {
        self.0
            .get(k)
            .filter(|(_, since)| now - since >= SETTLE)
            .map(|(v, _)| v.as_str())
    }
}

#[derive(Default, Clone)]
struct Counts {
    total: u32,
    characters: usize,
    weapons: usize,
    items: usize,
    props: usize,
    bodies: usize,
}

fn counts(world: &mut World) -> Counts {
    Counts {
        total: world.entities().count_spawned(),
        characters: world.query::<&NetCharacter>().iter(world).count(),
        weapons: world.query::<&Weapon>().iter(world).count(),
        items: world.query::<&NetItem>().iter(world).count(),
        props: world.query::<&NetProp>().iter(world).count(),
        bodies: world.query::<&NetBody>().iter(world).count(),
    }
}

/// Per client id: what it saw.
#[derive(Default)]
struct ClientStats {
    name: String,
    link: String,
    errors: u64,
    /// Errors the server's own doing explains: teleports (spawns, a
    /// jump over 1 m) and inventory-only changes (buys, deaths' drops).
    server_events: u64,
    /// Errors with another player within 1.5 m (pushing against others
    /// drawn in the past: Source has them too) or hit in the last second
    /// (damage's slowdown and blasts' pushes are the server's).
    contacts: u64,
    /// The server's count of ticks without our command, of commands too
    /// late, and the client's clock jumps (this connection).
    missed_late: (u32, u32, u32),
    /// Errors in a row now, and the longest run (what, when).
    streak: u64,
    worst_streak: (u64, String),
    excused: u64,
    checked: u64,
    worst_error: f32,
    last_graph_errors: u64,
    last_graph_checked: u64,
    bytes_in: Vec<(u64, u64)>,
    excuse_until: u64,
    mismatches: u64,
    compared: u64,
    body_compared: u64,
    body_off: u64,
    body_off_loading: u64,
    /// Errors of a dead player's position alone, below 0.01 mm.
    dead_drift: u64,
    body_worst: f32,
    ping: f64,
    /// Prediction errors by what differed and when (the round's phase,
    /// alive or dead).
    parts: BTreeMap<String, u64>,
}

struct Soak {
    sim: NetSim,
    maps: MapCache,
    brains: Vec<Brain>,
    stats: BTreeMap<u64, ClientStats>,
    links: Vec<LinkConditions>,
    step: u64,
    server_stable: Stable,
    client_stable: HashMap<u64, Stable>,
    /// The server's `NetBody` origin per tick and character.
    history: BTreeMap<u64, HashMap<Entity, [f32; 3]>>,
    /// Each client's own player's state by tick, as the server sent it
    /// and as the client predicted it (to say what an error was).
    server_states: HashMap<u64, BTreeMap<u64, Vec<u8>>>,
    client_preds: HashMap<u64, BTreeMap<u64, Vec<u8>>>,
    diagnosed: HashMap<String, u32>,
    trace: HashMap<(u64, Entity), f32>,
    /// Each client's character's health and when it last went down.
    hurt: HashMap<Entity, (f32, u64)>,
    level_changed_at: u64,
    /// Since when each client has heard each body off the server's.
    body_runs: HashMap<(u64, Entity), Option<(u64, bool)>>,
    shapes: HashMap<Entity, String>,
    examples: Vec<String>,
    /// Settled values a client had wrong: (client, field, whose) ->
    /// (first step, last step, server's, client's, checks).
    mismatch_runs: BTreeMap<(u64, &'static str, String), (u64, u64, String, String, u64)>,
    /// (map, visit, round, app, counts): sampled a second into each freeze.
    samples: Vec<(String, u32, u32, String, Counts)>,
    frame_ms: Vec<(u64, f64, u64)>,
    rss: Vec<(u64, String, f64)>,
    map: String,
    visits: HashMap<String, u32>,
    freeze_since: Option<u64>,
    sampled_round: Option<(String, u32, u32)>,
    planted_tries: u32,
    defuse_tries: u32,
    plant_round: Option<u32>,
    defuse_bomb: Option<Entity>,
    rounds_seen: u32,
    last_round: Option<u32>,
}

fn link(ms_rtt: u64, jitter: u64, loss: f64) -> LinkConditions {
    LinkConditions {
        latency: Duration::from_millis(ms_rtt / 2),
        jitter: Duration::from_millis(jitter),
        loss,
    }
}

impl Soak {
    fn server(&mut self) -> &mut World {
        self.sim.server.app.world_mut()
    }

    fn add_client(&mut self, conditions: LinkConditions) {
        let n = self.stats.len() + 1;
        let name = format!("Soaker {n}");
        let i = self
            .sim
            .add_client(|w| w.resource_mut::<NetSettings>().name = name.clone());
        self.sim.set_conditions(i, conditions);
        let id = self.sim.client_id(i);
        self.brains.push(Brain::new(id, self.step));
        self.links.push(conditions);
        self.stats.insert(
            id,
            ClientStats {
                name,
                link: format!(
                    "{} ms ±{} {:.0}%",
                    conditions.latency.as_millis() * 2,
                    conditions.jitter.as_millis(),
                    conditions.loss * 100.0
                ),
                ..default()
            },
        );
    }

    fn remove_client(&mut self, i: usize) {
        let id = self.sim.client_id(i);
        self.sim.remove_client(i);
        self.brains.remove(i);
        self.links.remove(i);
        self.client_stable.remove(&id);
    }

    fn joined(&self, i: usize) -> bool {
        self.sim.clients[i]
            .app
            .world()
            .get_resource::<Joined>()
            .is_some_and(|j| j.map == self.map)
    }

    /// One frame everywhere, with the scripted players and the checks.
    fn step(&mut self) {
        for i in 0..self.sim.clients.len() {
            if self.joined(i) {
                drive(&mut self.brains[i], self.sim.clients[i].app.world_mut(), self.step);
            }
        }
        let frame = Duration::from_secs_f64(1.0 / mashup::DEFAULT_TICK_HZ);
        self.sim.link.lock().unwrap().advance(frame);
        let before = self.sim.server.app.world().resource::<SimClock>().tick;
        let t = Instant::now();
        self.sim.server.app.update();
        let ms = t.elapsed().as_secs_f64() * 1e3;
        let ticks = self.sim.server.app.world().resource::<SimClock>().tick - before;
        self.frame_ms.push((self.step, ms, ticks));
        for i in 0..self.sim.clients.len() {
            let id = self.sim.client_id(i);
            let Some(e) = self.sim.character_of(i) else { continue };
            let Some(st) = self
                .sim
                .server
                .app
                .world()
                .get::<mashup::net::server::OwnStateOut>(e)
                .and_then(|o| o.state())
            else {
                continue;
            };
            let m = self.server_states.entry(id).or_default();
            m.insert(st.tick, st.state.clone());
            while m.len() > 300 {
                m.pop_first();
            }
        }
        for i in 0..self.sim.clients.len() {
            self.sim.clients[i].app.update();
            let id = self.sim.client_id(i);
            let preds = self.client_preds.entry(id).or_default();
            if let Some(h) = self.sim.clients[i]
                .app
                .world()
                .get_resource::<mashup::net::predict::PredictionHistory>()
            {
                for p in &h.0 {
                    preds.insert(p.tick, p.state.clone());
                }
            }
            while preds.len() > 300 {
                preds.pop_first();
            }
        }
        self.step += 1;
        self.record();
        if self.step % CHECK_EVERY == 0 {
            self.check();
        }
    }

    fn steps(&mut self, n: u64) {
        for _ in 0..n {
            self.step();
        }
    }

    fn record(&mut self) {
        let step = self.step;
        let tick = self.sim.server.app.world().resource::<SimClock>().tick;
        if std::env::var("MASHUP_SOAK_TRACE_HEALTH").is_ok() {
            // Every change of every character's health, server and
            // clients, per step (server entity names them).
            let w = self.sim.server.app.world_mut();
            let now: Vec<(Entity, f32, String)> = w
                .query::<(
                    Entity,
                    &Health,
                    &NetCharacter,
                    Has<mashup::net::NetHeld>,
                    Has<mashup::net::NetScore>,
                    Has<Team>,
                    Has<NetBody>,
                )>()
                .iter(w)
                .map(|(e, h, _, a, b, c, d)| (e, h.current, format!("{}{}{}{}", a as u8, b as u8, c as u8, d as u8)))
                .collect();
            for (e, h, shape) in now {
                let last = self.trace.entry((0, e)).or_insert(-1.0);
                if *last != h {
                    eprintln!("[health] step {step} tick {tick} server {e:?} {last} -> {h} shape {shape}");
                    *last = h;
                }
                let old = self.shapes.entry(e).or_default();
                if *old != shape {
                    eprintln!("[shape] step {step} tick {tick} server {e:?} {old} -> {shape}");
                    *old = shape;
                }
            }
            for i in 0..self.sim.clients.len() {
                let id = self.sim.client_id(i);
                let w = self.sim.clients[i].app.world_mut();
                let Some(map) = w.get_resource::<ServerEntityMap>() else {
                    continue;
                };
                let to_server: HashMap<Entity, Entity> = map.to_client().iter().map(|(s, c)| (*c, *s)).collect();
                let now: Vec<(Entity, f32, Option<u32>)> = w
                    .query::<(
                        Entity,
                        &Health,
                        &NetCharacter,
                        Option<&bevy_replicon::client::confirm_history::ConfirmHistory>,
                    )>()
                    .iter(w)
                    .filter_map(|(c, h, _, ch)| {
                        to_server
                            .get(&c)
                            .map(|s| (*s, h.current, ch.map(|ch| ch.last_tick().get())))
                    })
                    .collect();
                for (e, h, ct) in now {
                    let last = self.trace.entry((id, e)).or_insert(-1.0);
                    if *last != h {
                        eprintln!("[health] step {step} client {id} {e:?} {last} -> {h} (confirmed {ct:?})");
                        *last = h;
                    }
                }
            }
        }
        let w = self.server();
        let bodies: HashMap<Entity, [f32; 3]> = w
            .query::<(Entity, &NetBody)>()
            .iter(w)
            .map(|(e, b)| (e, b.origin))
            .collect();
        self.history.insert(tick, bodies);
        while self.history.len() > 400 {
            self.history.pop_first();
        }
        // Prediction errors and bandwidth per client.
        let bytes = self.sim.link.lock().unwrap().bytes_to.clone();
        for i in 0..self.sim.clients.len() {
            let id = self.sim.client_id(i);
            let g = self.sim.clients[i].app.world().get_resource::<NetGraph>().cloned();
            let context = {
                let w = self.sim.clients[i].app.world_mut();
                let phase = format!("{:?}", w.resource::<RoundState>().phase);
                let phase = phase.split([' ', '{']).next().unwrap_or("").to_string();
                let alive = w
                    .query_filtered::<(&Health, Has<Dead>), With<LocalPlayer>>()
                    .iter(w)
                    .next()
                    .map_or("none", |(h, d)| if !d && h.current > 0.0 { "alive" } else { "dead" });
                format!("{phase} {alive}")
            };
            // Another living player within reach of ours on the server: a
            // contact the client predicts against others drawn in the past.
            let near = self.sim.character_of(i).is_some_and(|me| {
                let w = self.sim.server.app.world_mut();
                let Some(at) = w.get::<Transform>(me).map(|t| t.translation) else {
                    return false;
                };
                // The dead count too: on a crowded spawn one stands on
                // another's box (the dead's are the server's to drop).
                w.query_filtered::<(Entity, &Transform), With<NetCharacter>>()
                    .iter(w)
                    .any(|(e, t)| e != me && t.translation.distance(at) < 1.5)
            });
            // Hit lately (health down in the last second on the server):
            // damage slows and blasts push, the server's to say.
            let hurt = self.sim.character_of(i).is_some_and(|me| {
                let h = self.sim.server.app.world().get::<Health>(me).map_or(0.0, |h| h.current);
                let last = self.hurt.entry(me).or_insert((h, 0));
                if h < last.0 {
                    last.1 = step;
                }
                last.0 = h;
                last.1 > 0 && step - last.1 < 64
            });
            let near = near || hurt;
            let context = match (near, hurt) {
                (_, true) => format!("{context} hit"),
                (true, false) => format!("{context} near"),
                _ => context,
            };
            let s = self.stats.get_mut(&id).unwrap();
            if let Some(g) = g {
                // A new connection's graph starts over.
                if g.errors < s.last_graph_errors || g.checked < s.last_graph_checked {
                    s.last_graph_errors = 0;
                    s.last_graph_checked = 0;
                }
                let d = g.errors - s.last_graph_errors;
                if step < s.excuse_until {
                    s.excused += d;
                } else {
                    let inventory_only = g
                        .last_differing
                        .iter()
                        .all(|n| n.ends_with("::Inventory") || n.ends_with("::MaxSpeed"));
                    if d > 0 && (g.last_error > 1.0 || inventory_only) {
                        s.server_events += d;
                    } else if d > 0 && near {
                        s.contacts += d;
                    } else {
                        s.errors += d;
                    }
                    // A held (dead) player's position off by less than the
                    // physics' tolerance, alone: the stale physics copy
                    // (`net::canonical_position`) put it back every tick.
                    if d > 0
                        && context.contains("dead")
                        && g.last_error < 1e-5
                        && g.last_differing.len() == 1
                        && g.last_differing[0].ends_with("::Transform")
                    {
                        s.dead_drift += d;
                    }
                    if d > 0 {
                        if g.last_error <= 1.0 {
                            s.worst_error = s.worst_error.max(g.last_error);
                        }
                        let bucket = format!("{} [{context}]", g.last_differing.join("+"));
                        *s.parts.entry(bucket.clone()).or_default() += d;
                        let n = self.diagnosed.entry(bucket.clone()).or_default();
                        if *n < 3 && std::env::var("MASHUP_SOAK_DIAGNOSE").is_ok() {
                            *n += 1;
                            let ours = self.client_preds.get(&id).and_then(|m| m.get(&g.last_error_tick));
                            let theirs = self.server_states.get(&id).and_then(|m| m.get(&g.last_error_tick));
                            if let (Some(a), Some(b)) = (ours, theirs) {
                                let flags = |w: &World, e: Option<Entity>| {
                                    e.map_or("-".to_string(), |e| {
                                        format!(
                                            "dead {} connecting {} nocollide {} pos {:?} slot {}",
                                            w.get::<Dead>(e).is_some(),
                                            w.get::<Connecting>(e).is_some(),
                                            w.get::<avian3d::prelude::ColliderDisabled>(e).is_some(),
                                            w.get::<Position>(e).map(|p| p.0),
                                            w.get::<mashup::slots::MovementSlot>(e).is_some(),
                                        )
                                    })
                                };
                                let local = {
                                    let w = self.sim.clients[i].app.world_mut();
                                    w.query_filtered::<Entity, With<LocalPlayer>>().iter(w).next()
                                };
                                let server_e = self.sim.character_of(i);
                                eprintln!(
                                    "[diagnose] client {id} tick {} {bucket}: {}\n    server {}\n    client {}",
                                    g.last_error_tick,
                                    describe(self.sim.clients[i].app.world(), a, b),
                                    flags(self.sim.server.app.world(), server_e),
                                    flags(self.sim.clients[i].app.world(), local)
                                );
                            }
                        }
                    }
                }
                // Errors in a row: a misprediction that a correction
                // doesn't cure (the next states wrong again) is a bug.
                let new_checked = g.checked - s.last_graph_checked;
                if new_checked > 0 {
                    if d == 0 {
                        s.streak = 0;
                    } else if g.last_error <= 1.0
                        && (!near
                            || g.last_differing
                                .iter()
                                .all(|n| n.ends_with("::Inventory") || n.ends_with("::MaxSpeed")))
                    {
                        s.streak += d;
                        if s.streak > s.worst_streak.0 {
                            s.worst_streak = (
                                s.streak,
                                format!("{} [{context}] step {step}", g.last_differing.join("+")),
                            );
                        }
                        if s.streak == 30 && std::env::var("MASHUP_SOAK_DIAGNOSE").is_ok() {
                            let ours = self.client_preds.get(&id).and_then(|m| m.get(&g.last_error_tick));
                            let theirs = self.server_states.get(&id).and_then(|m| m.get(&g.last_error_tick));
                            if let (Some(a), Some(b)) = (ours, theirs) {
                                let sv = self.sim.character_of(i).map(|e| {
                                    let w = self.sim.server.app.world();
                                    format!(
                                        "server: dead {} frozen {:?} intent axis {:?} buffer missed {:?} late {:?} queued {:?} applied {:?}",
                                        w.get::<Dead>(e).is_some(),
                                        w.get_resource::<mashup::core::FreezeTime>().map(|f| f.0),
                                        w.get::<Intent>(e).map(|i| i.move_axis),
                                        w.get::<mashup::net::server::CommandBuffer>(e).map(|b| b.missed),
                                        w.get::<mashup::net::server::CommandBuffer>(e).map(|b| b.late),
                                        w.get::<mashup::net::server::CommandBuffer>(e).map(|b| b.queued.keys().copied().collect::<Vec<_>>()),
                                        w.get::<mashup::net::server::CommandBuffer>(e).map(|b| b.applied),
                                    )
                                });
                                let cl = {
                                    let w = self.sim.clients[i].app.world_mut();
                                    let me = w.query_filtered::<Entity, With<LocalPlayer>>().iter(w).next();
                                    let hist: Vec<(u64, Vec2)> = w
                                        .resource::<mashup::net::predict::PredictionHistory>()
                                        .0
                                        .iter()
                                        .map(|p| (p.tick, p.intent.move_axis))
                                        .take(4)
                                        .collect();
                                    format!(
                                        "client: dead {} frozen {:?} clock {:?} history {hist:?}",
                                        me.is_some_and(|m| w.get::<Dead>(m).is_some()),
                                        w.get_resource::<mashup::core::FreezeTime>().map(|f| f.0),
                                        w.resource::<mashup::net::predict::CommandClock>().tick,
                                    )
                                };
                                eprintln!("[streak] {sv:?}\n         {cl}");
                                eprintln!(
                                    "[streak] client {id} tick {} {}: {}\n  ours   {a:02x?}\n  theirs {b:02x?}",
                                    g.last_error_tick,
                                    context,
                                    describe(self.sim.clients[i].app.world(), a, b)
                                );
                            }
                        }
                    }
                }
                s.checked += new_checked;
                s.last_graph_errors = g.errors;
                s.last_graph_checked = g.checked;
                s.ping = g.ping_ms;
                s.missed_late = (g.missed, g.late, g.clock_jumps);
            }
            if step % 64 == 0 {
                s.bytes_in.push((step, bytes.get(&id).copied().unwrap_or(0)));
            }
        }
        if step % (64 * 30) == 0 {
            if let Some(mb) = rss_mb() {
                let visit = self.visits.get(&self.map).copied().unwrap_or(0);
                self.rss.push((step, format!("{} #{visit}", self.map), mb));
            }
        }
        // Rounds: sample entity counts a second into each freeze.
        let phase = self.server().resource::<RoundState>().phase;
        let number = {
            let w = self.server();
            w.query::<&NetRound>().iter(w).next().map_or(0, |r| r.number)
        };
        if self.last_round != Some(number) {
            self.last_round = Some(number);
            self.rounds_seen += 1;
        }
        if matches!(phase, Phase::Freeze { .. }) {
            let since = *self.freeze_since.get_or_insert(step);
            let key = (self.map.clone(), self.visits[&self.map], number);
            if step - since == 64 && self.sampled_round.as_ref() != Some(&key) {
                self.sampled_round = Some(key.clone());
                let c = counts(self.sim.server.app.world_mut());
                self.samples.push((key.0.clone(), key.1, key.2, "server".into(), c));
                for i in 0..self.sim.clients.len() {
                    if !self.joined(i) {
                        continue;
                    }
                    let id = self.sim.client_id(i);
                    let c = counts(self.sim.clients[i].app.world_mut());
                    self.samples
                        .push((key.0.clone(), key.1, key.2, format!("client {id}"), c));
                }
            }
        } else {
            self.freeze_since = None;
        }
        self.objectives();
    }

    /// Planting and defusing by scripted players on maps with bomb sites:
    /// the server puts them there (excusing the correction).
    fn objectives(&mut self) {
        let step = self.step;
        let phase = self.server().resource::<RoundState>().phase;
        let Phase::Live { since, .. } = phase else { return };
        let now = self.server().resource::<SimClock>().now;
        if now - since < 3.0 {
            return;
        }
        let round = self.last_round;
        let sites: Vec<Vec3> = self
            .server()
            .get_resource::<Tactics>()
            .map(|t| t.sites.iter().map(|s| s.point).collect())
            .unwrap_or_default();
        if sites.is_empty() {
            return;
        }
        let bomb = self.server().resource::<BombState>().clone();
        if bomb.planted.is_none() && self.plant_round != round {
            for i in 0..self.sim.clients.len() {
                if !self.joined(i) || self.brains[i].mode != Mode::Roam {
                    continue;
                }
                let Some(e) = self.sim.character_of(i) else { continue };
                let w = self.sim.server.app.world();
                if w.get::<Dead>(e).is_some() {
                    continue;
                }
                let has_c4 = w.get::<Inventory>(e).is_some_and(|inv| {
                    inv.weapons
                        .iter()
                        .any(|x| w.get::<Weapon>(*x).is_some_and(|x| x.id.ends_with("weapon_c4")))
                });
                if !has_c4 {
                    continue;
                }
                self.plant_round = round;
                self.planted_tries += 1;
                let site = sites[self.planted_tries as usize % sites.len()];
                self.teleport(i, site + Vec3::Y * 0.95);
                self.brains[i].mode = Mode::Plant { since: step };
                break;
            }
        }
        if let (Some(b), None) = (bomb.planted, bomb.outcome) {
            if self.defuse_bomb == Some(b) {
                return;
            }
            let Some(at) = self.server().get::<Transform>(b).map(|t| t.translation) else {
                return;
            };
            for i in 0..self.sim.clients.len() {
                if !self.joined(i) || self.brains[i].mode != Mode::Roam {
                    continue;
                }
                let Some(e) = self.sim.character_of(i) else { continue };
                let w = self.sim.server.app.world();
                if w.get::<Dead>(e).is_some() || w.get::<Team>(e).map(|t| t.0) != Some(2) {
                    continue;
                }
                self.defuse_bomb = Some(b);
                self.defuse_tries += 1;
                let toward = sites
                    .iter()
                    .copied()
                    .min_by(|a, c| a.distance(at).total_cmp(&c.distance(at)))
                    .map(|s| (s - at).with_y(0.0))
                    .filter(|d| d.length() > 0.2)
                    .map_or(Vec3::X, |d| d.normalize());
                self.teleport(i, at + toward * 0.9 + Vec3::Y * 1.0);
                self.brains[i].mode = Mode::Defuse { since: step, at };
                break;
            }
        }
    }

    fn teleport(&mut self, i: usize, at: Vec3) {
        let id = self.sim.client_id(i);
        let e = self.sim.character_of(i).unwrap();
        let w = self.server();
        w.get_mut::<Transform>(e).unwrap().translation = at;
        if let Some(mut p) = w.get_mut::<Position>(e) {
            p.0 = at;
        }
        self.stats.get_mut(&id).unwrap().excuse_until = self.step + EXCUSE;
    }

    /// The server's and each client's values: settled ones must agree;
    /// others' bodies as heard must be the server's at their tick.
    fn check(&mut self) {
        let step = self.step;
        let mut values = HashMap::new();
        {
            let w = self.sim.server.app.world_mut();
            let owned: HashMap<Entity, bool> = w
                .query_filtered::<(Entity, &NetCharacter), Without<Connecting>>()
                .iter(w)
                .map(|(e, c)| (e, c.owner.is_some()))
                .collect();
            for (e, own) in owned {
                character_values(w, e, e, own, &mut values);
            }
            round_values(w, &mut values);
        }
        self.server_stable.update(step, values);
        for i in 0..self.sim.clients.len() {
            if !self.joined(i) {
                continue;
            }
            let id = self.sim.client_id(i);
            let mine = self.sim.character_of(i);
            let w = self.sim.clients[i].app.world_mut();
            let Some(map) = w.get_resource::<ServerEntityMap>() else {
                continue;
            };
            let to_server: HashMap<Entity, Entity> = map.to_client().iter().map(|(s, c)| (*c, *s)).collect();
            let mut values = HashMap::new();
            let chars: Vec<(Entity, Entity)> = w
                .query_filtered::<Entity, With<NetCharacter>>()
                .iter(w)
                .filter_map(|c| to_server.get(&c).map(|s| (c, *s)))
                .collect();
            for (c, s) in &chars {
                character_values(w, *c, *s, Some(*s) == mine, &mut values);
            }
            round_values(w, &mut values);
            let stable = self.client_stable.entry(id).or_default();
            stable.update(step, values);
            let stats = self.stats.get_mut(&id).unwrap();
            for (k, (sv, since)) in &self.server_stable.0 {
                if step - since < SETTLE {
                    continue;
                }
                // Money, armour and weapons are only the owner's to see.
                if matches!(k.1, "money" | "armor" | "weapons") && Some(k.0) != mine {
                    continue;
                }
                let Some(cv) = stable.settled(k, step) else { continue };
                stats.compared += 1;
                if cv != sv {
                    stats.mismatches += 1;
                    let who = {
                        let w = self.sim.server.app.world();
                        let c = w.get::<NetCharacter>(k.0);
                        format!(
                            "{:?} ({})",
                            k.0,
                            c.map_or("-".into(), |c| format!("{} owner {:?}", c.name, c.owner))
                        )
                    };
                    let client_e = w
                        .get_resource::<ServerEntityMap>()
                        .and_then(|m| m.to_client().get(&k.0).copied());
                    let extra = {
                        let cw = &*w;
                        let confirm = client_e
                            .and_then(|c| cw.get::<bevy_replicon::client::confirm_history::ConfirmHistory>(c))
                            .map(|h| h.last_tick());
                        let body = client_e
                            .and_then(|c| cw.get::<Snapshots<NetBody>>(c))
                            .and_then(|s| s.newest().map(|n| n.0));
                        let tick = self.sim.server.app.world().resource::<SimClock>().tick;
                        format!(
                            "client entity {client_e:?} confirmed {confirm:?} newest body tick {body:?} server tick {tick}"
                        )
                    };
                    let e = self.mismatch_runs.entry((id, k.1, who)).or_insert((
                        step,
                        step,
                        String::new(),
                        String::new(),
                        0,
                    ));
                    e.1 = step;
                    e.2 = format!("{sv} {}", self.server_stable.log(k));
                    e.3 = format!("{cv} {} [{extra}]", stable.log(k));
                    e.4 += 1;
                }
            }
            // Others' bodies as heard vs the server's at that tick.
            let heard: Vec<(Entity, u64, [f32; 3])> = w
                .query::<(Entity, &Snapshots<NetBody>)>()
                .iter(w)
                .filter_map(|(c, s)| s.newest().map(|(t, b)| (c, *t, b.origin)))
                .collect();
            for (c, tick, origin) in heard {
                let (Some(s), Some(at)) = (to_server.get(&c), self.history.get(&tick)) else {
                    continue;
                };
                let Some(server) = at.get(s) else { continue };
                let off = Vec3::from(*server).distance(Vec3::from(origin));
                stats.body_compared += 1;
                stats.body_worst = stats.body_worst.max(off);
                // A body off for longer than a value takes to settle
                // and be sent again (`resend_settled`) plus a round trip
                // is lost for good; shorter, a loss being repaired.
                let run = self.body_runs.entry((id, *s)).or_insert(None);
                if off <= 0.01 {
                    *run = None;
                } else if step < self.level_changed_at + 128 {
                    // While everyone loads the new map (reported apart).
                    stats.body_off_loading += 1;
                } else {
                    let (since, counted) = run.get_or_insert((step, false));
                    if !*counted && step - *since >= mashup::net::server::SETTLE_TICKS + 64 {
                        *counted = true;
                        stats.body_off += 1;
                        if self.examples.len() < 30 {
                            self.examples.push(format!(
                                "step {step} client {id}: body {s:?} at tick {tick} off by {off:.3} m since step {since}"
                            ));
                        }
                    }
                }
            }
        }
    }

    /// `changelevel id` as the dedicated server does it, then play on
    /// until every client is in.
    fn change_level(&mut self, id: &str) {
        eprintln!("[soak] step {}: changelevel {id}", self.step);
        self.level_changed_at = self.step;
        mashup::harness::begin_level_change(self.server(), id);
        self.steps(5);
        let data = (id != GREYBOX).then(|| self.maps.get(id));
        mashup::swap_map(self.server(), id, data, tick_of(id));
        self.enter(id);
        let ok = {
            let mut ok = false;
            for _ in 0..4000 {
                if (0..self.sim.clients.len()).all(|i| self.joined(i)) {
                    ok = true;
                    break;
                }
                self.step();
            }
            ok
        };
        assert!(ok, "every client followed to {id}");
        eprintln!("[soak] step {}: everyone on {id}", self.step);
    }

    fn enter(&mut self, id: &str) {
        self.map = id.to_string();
        *self.visits.entry(id.to_string()).or_default() += 1;
        self.plant_round = None;
        self.defuse_bomb = None;
    }

    fn rename(&mut self, i: usize, name: &str) {
        let id = self.sim.client_id(i);
        eprintln!("[soak] step {}: client {id} renames to {name}", self.step);
        self.sim.clients[i]
            .app
            .world_mut()
            .resource_mut::<Console>()
            .submit(format!("name \"{name}\""));
        self.stats.get_mut(&id).unwrap().name = name.to_string();
    }
}

#[test]
fn soak_four_clients_and_bots_over_map_changes() {
    if !installed() {
        return;
    }
    let minutes: f64 = std::env::var("MASHUP_SOAK_MINUTES")
        .ok()
        .and_then(|m| m.parse().ok())
        .unwrap_or(3.0);
    let total = (minutes * 60.0 * mashup::DEFAULT_TICK_HZ) as u64;
    let community = community_map();
    let maps = MapCache::default();
    let setup = {
        let maps = maps.clone();
        move |app: &mut App| {
            app.set_error_handler(count_error);
            app.add_plugins((
                GreyboxMapPlugin,
                MapPlugin::empty(),
                SourceMovementPlugin,
                CsWeaponsPlugin,
            ))
            .insert_resource(Loadout { movement: source::ID })
            .insert_resource(MapFiles {
                read: Arc::new(games::map_file_bytes),
                cache: None,
            })
            .insert_resource(maps.clone())
            .add_systems(Update, load_requested);
        }
    };
    let mut sim = NetSim::new(LinkConditions::default(), 97, 0, setup);
    {
        let w = sim.server.app.world_mut();
        let data = maps.get(DUST2);
        mashup::swap_map(w, DUST2, Some(data), tick_of(DUST2));
        w.resource_mut::<Console>().submit(
            "mashup_rounds 1; mp_freezetime 2; mp_roundtime 0.75; mp_c4timer 25; bot_join_after_player 0; \
             bot_quota_mode fill; bot_quota 12",
        );
    }
    let mut soak = Soak {
        sim,
        maps: maps.clone(),
        brains: Vec::new(),
        stats: BTreeMap::new(),
        links: Vec::new(),
        step: 0,
        server_stable: Stable::default(),
        client_stable: HashMap::new(),
        history: BTreeMap::new(),
        server_states: HashMap::new(),
        client_preds: HashMap::new(),
        diagnosed: HashMap::new(),
        trace: HashMap::new(),
        hurt: HashMap::new(),
        level_changed_at: 0,
        body_runs: HashMap::new(),
        shapes: HashMap::new(),
        examples: Vec::new(),
        mismatch_runs: BTreeMap::new(),
        samples: Vec::new(),
        frame_ms: Vec::new(),
        rss: Vec::new(),
        map: String::new(),
        visits: HashMap::new(),
        freeze_since: None,
        sampled_round: None,
        planted_tries: 0,
        defuse_tries: 0,
        plant_round: None,
        defuse_bomb: None,
        rounds_seen: 0,
        last_round: None,
    };
    soak.enter(DUST2);
    let links = [
        link(0, 0, 0.0),
        link(50, 5, 0.0),
        link(150, 15, 0.02),
        link(300, 30, 0.05),
    ];
    for l in links {
        soak.add_client(l);
    }
    let started = Instant::now();
    let ok = {
        let mut ok = false;
        for _ in 0..4000 {
            if (0..soak.sim.clients.len()).all(|i| soak.joined(i)) {
                ok = true;
                break;
            }
            soak.step();
        }
        ok
    };
    assert!(ok, "the clients joined");
    // Everyone in a fresh game.
    soak.server().resource_mut::<Console>().submit("mp_restartgame 1");

    let at = |f: f64| (total as f64 * f) as u64;
    let mut events: Vec<(u64, &str)> = vec![
        (at(0.10), "rename"),
        (at(0.15), "leave"),
        (at(0.20), "join"),
        (at(0.30), "greybox"),
        (at(0.40), "rename"),
        (at(0.50), "community"),
        (at(0.60), "rename"),
        (at(0.70), "dust2"),
        (at(0.85), "rename"),
    ];
    events.reverse();
    let mut renames = 0;
    let start = soak.step;
    while soak.step - start < total {
        let t = soak.step - start;
        while events.last().is_some_and(|(when, _)| *when <= t) {
            let (_, what) = events.pop().unwrap();
            match what {
                "rename" => {
                    renames += 1;
                    let i = renames % soak.sim.clients.len();
                    soak.rename(i, &format!("Renamed {renames} (soak)"));
                }
                "leave" => {
                    let id = soak.sim.client_id(3);
                    eprintln!("[soak] step {}: client {id} leaves", soak.step);
                    soak.remove_client(3);
                }
                "join" => {
                    eprintln!("[soak] step {}: a client joins", soak.step);
                    soak.add_client(link(150, 20, 0.03));
                }
                "greybox" => soak.change_level(GREYBOX),
                "community" => match community.clone() {
                    Some(m) => soak.change_level(&m),
                    None => eprintln!("[soak] no community map in the content cache: skipped"),
                },
                "dust2" => soak.change_level(DUST2),
                _ => unreachable!(),
            }
        }
        soak.step();
        if soak.step % (64 * 60) == 0 {
            eprintln!(
                "[soak] {:.1} game-min, {:.0} s real, map {}, rounds {}",
                (soak.step - start) as f64 / 64.0 / 60.0,
                started.elapsed().as_secs_f64(),
                soak.map,
                soak.rounds_seen
            );
        }
    }
    report(&mut soak, minutes, started.elapsed());
}

/// What differs between two predicted-state blobs (ours, the server's).
fn describe(world: &World, ours: &[u8], theirs: &[u8]) -> String {
    let reg = world.resource::<mashup::core::PredictedComponents>();
    let (Ok(a), Ok(b)) = (reg.parts(ours), reg.parts(theirs)) else {
        return "layout".into();
    };
    let mut out = Vec::new();
    for ((name, x), (_, y)) in a.into_iter().zip(b) {
        if x == y {
            continue;
        }
        let text = match (x, y) {
            (Some(x), Some(y)) if name.ends_with("::Transform") => {
                let (x, y): (Transform, Transform) =
                    (postcard::from_bytes(x).unwrap(), postcard::from_bytes(y).unwrap());
                format!(
                    "pos {:?} vs {:?} (d {:?}), rot {:?} vs {:?}",
                    x.translation,
                    y.translation,
                    y.translation - x.translation,
                    x.rotation,
                    y.rotation
                )
            }
            (Some(x), Some(y)) if name.ends_with("::Velocity") => {
                let (x, y): (mashup::core::Velocity, mashup::core::Velocity) =
                    (postcard::from_bytes(x).unwrap(), postcard::from_bytes(y).unwrap());
                format!("{:?} vs {:?}", x.0, y.0)
            }
            (Some(x), Some(y)) => {
                let first = x
                    .iter()
                    .zip(y)
                    .position(|(p, q)| p != q)
                    .unwrap_or(x.len().min(y.len()));
                format!("{} vs {} bytes, first difference at {first}", x.len(), y.len())
            }
            (x, y) => format!("present {} vs {}", x.is_some(), y.is_some()),
        };
        out.push(format!("{}: {text}", name.rsplit("::").next().unwrap()));
    }
    out.join("; ")
}

fn percentile(v: &mut [f64], p: f64) -> f64 {
    if v.is_empty() {
        return 0.0;
    }
    v.sort_by(f64::total_cmp);
    v[((v.len() - 1) as f64 * p).round() as usize]
}

fn report(soak: &mut Soak, minutes: f64, real: Duration) {
    println!(
        "\n== network soak: {minutes} game-minutes in {:.0} s, {} rounds, plants tried {}, defuses tried {} ==",
        real.as_secs_f64(),
        soak.rounds_seen,
        soak.planted_tries,
        soak.defuse_tries
    );
    let bomb = soak.server().resource::<BombState>().clone();
    let _ = bomb;
    println!(
        "\nclient | link | ping ms | checked | movement errors | err % | contacts, hits | server events | excused | worst m | KB/s mean | KB/s peak | settled compared | mismatches | bodies compared | bodies off"
    );
    let mut failures: Vec<String> = Vec::new();
    for (id, s) in &soak.stats {
        let rates: Vec<f64> = s
            .bytes_in
            .windows(2)
            .map(|w| (w[1].1 - w[0].1) as f64 / ((w[1].0 - w[0].0) as f64 / 64.0) / 1000.0)
            .collect();
        let mean = if s.bytes_in.len() >= 2 {
            let (a, b) = (s.bytes_in[0], s.bytes_in[s.bytes_in.len() - 1]);
            (b.1 - a.1) as f64 / ((b.0 - a.0) as f64 / 64.0) / 1000.0
        } else {
            0.0
        };
        let peak = rates.iter().copied().fold(0.0, f64::max);
        let rate = s.errors as f64 / s.checked.max(1) as f64 * 100.0;
        // The worst link (300 ms, 5 % loss) gets more room.
        let worst_link = s.link.starts_with("300");
        println!(
            "{id} {} | {} | {:.0} | {} | {} | {:.2} | {} | {} | {} | {:.3} | {:.1} | {:.1} | {} | {} | {} | {}",
            s.name,
            s.link,
            s.ping,
            s.checked,
            s.errors,
            rate,
            s.contacts,
            s.server_events,
            s.excused,
            s.worst_error,
            mean,
            peak,
            s.compared,
            s.mismatches,
            s.body_compared,
            s.body_off
        );
        if rate > if worst_link { 5.0 } else { 2.0 } {
            failures.push(format!("client {id}: {rate:.2} % of states mispredicted"));
        }
        // The plan's budget (multiplayer.md, "Soak"): 96 KB/s a client
        // with twelve characters (measured 71-84), 128 on the worst link
        // (loss makes replicon send values again until acknowledged).
        if mean > if worst_link { 128.0 } else { 96.0 } {
            failures.push(format!("client {id}: {mean:.1} KB/s in"));
        }
        if s.mismatches > 0 {
            failures.push(format!(
                "client {id}: {} settled values differ from the server's",
                s.mismatches
            ));
        }
        if s.body_off > 0 {
            failures.push(format!("client {id}: {} bodies heard off the server's", s.body_off));
        }
    }
    // Frame time per game-minute.
    println!("\nminute | server frames | mean ms | p95 ms | p99 ms | max ms | ms per tick");
    let mut minute_medians = Vec::new();
    for (m, chunk) in soak.frame_ms.chunks(64 * 60).enumerate() {
        let mut ms: Vec<f64> = chunk.iter().map(|c| c.1).collect();
        let ticks: u64 = chunk.iter().map(|c| c.2).sum();
        let sum: f64 = ms.iter().sum();
        let p95 = percentile(&mut ms, 0.95);
        // Full minutes' medians: wall times on a shared machine jump with
        // its load (p95 too much to compare).
        if chunk.len() == 64 * 60 {
            minute_medians.push(percentile(&mut ms, 0.5));
        }
        println!(
            "{m} | {} | {:.2} | {:.2} | {:.2} | {:.2} | {:.2}",
            ms.len(),
            sum / ms.len() as f64,
            p95,
            percentile(&mut ms, 0.99),
            ms.last().copied().unwrap_or(0.0),
            sum / ticks.max(1) as f64
        );
    }
    // Within a visit of a map (after its first minute), memory may not
    // climb: a leak grows with rounds, not with maps loaded.
    let mut by_visit: BTreeMap<String, Vec<f64>> = BTreeMap::new();
    for (_, visit, mb) in &soak.rss {
        by_visit.entry(visit.clone()).or_default().push(*mb);
    }
    for (visit, mb) in &by_visit {
        if mb.len() >= 6 {
            let (first, last) = (mb[2], mb[mb.len() - 1]);
            if last > first * 1.1 + 100.0 {
                failures.push(format!("memory on {visit} grew from {first:.0} to {last:.0} MB"));
            }
        }
    }
    println!(
        "\nmemory (process resident + swapped, MB): {:?}",
        soak.rss
            .iter()
            .map(|(s, m, mb)| format!("{:.1}min {m} {mb:.0}", *s as f64 / 3840.0))
            .collect::<Vec<_>>()
    );
    // Entity counts per round.
    println!("\nmap | visit | round | app | entities | characters | weapons | items | props | bodies");
    for (map, visit, round, app, c) in &soak.samples {
        println!(
            "{map} | {visit} | {round} | {app} | {} | {} | {} | {} | {} | {}",
            c.total, c.characters, c.weapons, c.items, c.props, c.bodies
        );
    }
    // Leaks: within a visit, after its first two rounds, no app's entity
    // count may climb; de_dust2's second visit no higher than its first.
    let mut by: BTreeMap<(String, u32, String), Vec<u32>> = BTreeMap::new();
    for (map, visit, _, app, c) in &soak.samples {
        by.entry((map.clone(), *visit, app.clone())).or_default().push(c.total);
    }
    for ((map, visit, app), totals) in &by {
        if totals.len() < 4 {
            continue;
        }
        let base = totals[1..3].iter().copied().max().unwrap();
        let last = *totals.last().unwrap();
        if last > base + 40 {
            failures.push(format!("{app} on {map} (visit {visit}): entities {totals:?}"));
        }
    }
    for app in by
        .keys()
        .filter(|k| k.0 == DUST2 && k.1 == 1)
        .map(|k| k.2.clone())
        .collect::<Vec<_>>()
    {
        let (Some(a), Some(b)) = (
            by.get(&(DUST2.into(), 1, app.clone())),
            by.get(&(DUST2.into(), 2, app.clone())),
        ) else {
            continue;
        };
        let (a, b) = (a.iter().copied().min().unwrap(), b.iter().copied().min().unwrap());
        if b > a + 40 {
            failures.push(format!(
                "{app}: de_dust2 entities {a} on the first visit, {b} on the second"
            ));
        }
    }
    if minute_medians.len() >= 3 {
        let first = minute_medians[1];
        let last = *minute_medians.last().unwrap();
        if last > first * 3.0 + 2.0 {
            failures.push(format!("server frame median grew from {first:.2} to {last:.2} ms"));
        }
    }
    {
        let w = soak.server();
        let own: Vec<usize> = w
            .query::<&mashup::net::server::OwnStateOut>()
            .iter(w)
            .filter_map(|o| o.state().map(|s| postcard::to_allocvec(s).unwrap().len()))
            .collect();
        let bodies: Vec<usize> = w
            .query::<&NetBody>()
            .iter(w)
            .map(|b| postcard::to_allocvec(b).unwrap().len())
            .collect();
        println!("\nsizes: OwnState {own:?} bytes, NetBody {bodies:?} bytes");
    }
    for (id, s) in &soak.stats {
        if s.dead_drift > 0 {
            failures.push(format!(
                "client {id}: {} errors of a dead player's position below 0.01 mm",
                s.dead_drift
            ));
        }
        if s.body_off_loading > 0 {
            println!(
                "client {id}: {} bodies off the server's within 2 s of a map change",
                s.body_off_loading
            );
        }
        println!(
            "client {id} longest run of errors: {} ({}); commands missed {}, late {}, clock jumps {}",
            s.worst_streak.0, s.worst_streak.1, s.missed_late.0, s.missed_late.1, s.missed_late.2
        );
        if s.worst_streak.0 > 40 {
            failures.push(format!(
                "client {id}: {} prediction errors in a row ({})",
                s.worst_streak.0, s.worst_streak.1
            ));
        }
    }
    for (id, s) in &soak.stats {
        println!("client {id} errors: {:?}", s.parts);
    }
    let errors = ERROR_COUNT.load(std::sync::atomic::Ordering::Relaxed);
    println!("\nBevy errors (failed commands, ...): {errors}");
    for e in ERRORS.lock().unwrap().iter() {
        println!("  {e}");
    }
    if errors > 0 {
        failures.push(format!("{errors} Bevy errors (commands on missing entities, ...)"));
    }
    for ((id, field, who), (a, b, sv, cv, n)) in &soak.mismatch_runs {
        println!("  client {id} {field} of {who}: steps {a}-{b} ({n} checks), server {sv:?}, client {cv:?}");
    }
    for e in &soak.examples {
        println!("  {e}");
    }
    assert!(failures.is_empty(), "soak failures:\n{}", failures.join("\n"));
}
