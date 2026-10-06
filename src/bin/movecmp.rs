//! Dev tool: compare mashup's Source movement with real CS:S, tick for tick.
//!
//! Each scenario is a start state plus per-tick inputs (buttons, move
//! amounts, view angles). The real game plays them on a bot through the
//! tools/css_probe SourceMod plugin on a local CS:S dedicated server and
//! logs the bot after every tick; mashup plays the same inputs from the
//! same state (the game's tick 0) on the same map. The report lists, per
//! scenario, the largest position and velocity differences and the first
//! tick where they diverge. See docs/OBSERVABILITY.md.
//!
//! `movecmp fuzz` plays seeded random inputs (keys, jump, duck, turning and
//! pitch in bursts) from ladder spots on de_nuke and water spots on
//! de_aztec, and fails when a run's position drifts past a tolerance or the
//! water level or ladder state disagree.
//!
//! The server comes from `[games.cs_source_server]` in mashup.local.toml.

#[path = "shared/rcon.rs"]
mod rcon;

use std::{
    f32::consts::FRAC_PI_2,
    path::Path,
    process::{Command, ExitCode, Stdio},
    sync::Arc,
    thread::sleep,
    time::{Duration, Instant},
};

use bevy::prelude::*;
use mashup::{
    core::Velocity,
    games::{
        self,
        cs_source::{
            TICK_INTERVAL,
            movement::{self, SourceMovement, SourceMovementConfig, SourceMovementPlugin, to_engine, to_source},
        },
    },
    harness::Sim,
    map::{MapData, MapDebugView, MapPlugin},
    mount::config::{LocalConfig, default_dump_dir},
};
use rcon::Rcon;

const RCON_ADDR: &str = "127.0.0.1:27030";
const RCON_PASSWORD: &str = "mashup-probe";
const MAP: &str = "de_dust2";

// Source input buttons.
const IN_JUMP: u32 = 1 << 1;
const IN_DUCK: u32 = 1 << 2;
const IN_FORWARD: u32 = 1 << 3;
const IN_BACK: u32 = 1 << 4;
const IN_MOVELEFT: u32 = 1 << 9;
const IN_MOVERIGHT: u32 = 1 << 10;
/// The walk key (+speed).
const IN_SPEED: u32 = 1 << 17;
/// What a held move key sends (cl_forwardspeed / cl_sidespeed).
const KEY: f32 = 400.0;

#[derive(Clone, Copy, Debug)]
struct Input {
    buttons: u32,
    forward: f32,
    side: f32,
    pitch: f32,
    yaw: f32,
}

impl Input {
    fn idle(yaw: f32) -> Self {
        Self {
            buttons: 0,
            forward: 0.0,
            side: 0.0,
            pitch: 0.0,
            yaw,
        }
    }
    fn forward(mut self) -> Self {
        self.buttons |= IN_FORWARD;
        self.forward = KEY;
        self
    }
    fn back(mut self) -> Self {
        self.buttons |= IN_BACK;
        self.forward = -KEY;
        self
    }
    fn look(mut self, pitch: f32) -> Self {
        self.pitch = pitch;
        self
    }
    /// Strafe right (+1) or left (-1).
    fn strafe(mut self, side: f32) -> Self {
        self.buttons |= if side > 0.0 { IN_MOVERIGHT } else { IN_MOVELEFT };
        self.side = KEY * side.signum();
        self
    }
    fn jump(mut self) -> Self {
        self.buttons |= IN_JUMP;
        self
    }
    fn duck(mut self) -> Self {
        self.buttons |= IN_DUCK;
        self
    }
    fn walk(mut self) -> Self {
        self.buttons |= IN_SPEED;
        self
    }
}

struct Scenario {
    name: String,
    map: String,
    /// Feet position (Source units) and yaw to start from.
    start: Vec3,
    yaw: f32,
    inputs: Vec<Input>,
}

/// An open patch of CT spawn on de_dust2, facing south.
const CT_SPAWN: Vec3 = Vec3::new(448.0, 2464.0, -60.0);

fn scenarios() -> Vec<Scenario> {
    let yaw = 270.0;
    let settle = || vec![Input::idle(yaw); 20];
    let mut out = Vec::new();

    let mut walk = settle();
    walk.extend(vec![Input::idle(yaw).forward(); 60]);
    walk.extend(vec![Input::idle(yaw); 50]);
    out.push(("walk_stop", walk));

    let mut jump = settle();
    jump.push(Input::idle(yaw).jump());
    jump.extend(vec![Input::idle(yaw); 60]);
    out.push(("jump", jump));

    // Walking (+speed) from rest, then walking ducked.
    let mut walk = settle();
    walk.extend(vec![Input::idle(yaw).walk().forward(); 60]);
    walk.extend(vec![Input::idle(yaw); 30]);
    walk.extend(vec![Input::idle(yaw).walk().duck().forward(); 40]);
    out.push(("walk", walk));

    let mut duck = settle();
    duck.extend(vec![Input::idle(yaw).duck(); 40]);
    duck.extend(vec![Input::idle(yaw).duck().forward(); 30]);
    duck.extend(vec![Input::idle(yaw); 30]);
    out.push(("duck_walk", duck));

    // Run, jump, then strafe right while turning right 3 degrees a tick.
    let mut strafe = settle();
    strafe.extend(vec![Input::idle(yaw).forward(); 40]);
    strafe.push(Input::idle(yaw).forward().jump());
    for k in 1..=50 {
        strafe.push(Input::idle(yaw - 3.0 * k as f32).strafe(1.0));
    }
    out.push(("strafe_jump", strafe));

    // Diagnostics for running jumps: forward held but jumping from rest,
    // and running then releasing forward on the jump tick.
    let mut fwd_jump = settle();
    fwd_jump.push(Input::idle(yaw).forward().jump());
    fwd_jump.extend(vec![Input::idle(yaw).forward(); 40]);
    out.push(("forward_jump_from_rest", fwd_jump));

    let mut release_jump = settle();
    release_jump.extend(vec![Input::idle(yaw).forward(); 40]);
    release_jump.push(Input::idle(yaw).jump());
    release_jump.extend(vec![Input::idle(yaw); 40]);
    out.push(("run_release_jump", release_jump));

    // Scroll-wheel bhop: jump pressed every other tick (a fresh press each
    // time), strafing and turning, switching sides every 22 ticks.
    let mut bhop = settle();
    bhop.extend(vec![Input::idle(yaw).forward(); 40]);
    let mut view = yaw;
    for k in 0..180 {
        let side = if (k / 22) % 2 == 0 { 1.0 } else { -1.0 };
        view -= 2.5 * side;
        let mut i = Input::idle(view).strafe(side);
        if k % 2 == 0 {
            i = i.jump();
        }
        bhop.push(i);
    }
    out.push(("bhop_scroll", bhop));

    let mut all: Vec<Scenario> = out
        .into_iter()
        .map(|(name, inputs)| Scenario {
            name: name.to_string(),
            map: MAP.to_string(),
            start: CT_SPAWN,
            yaw,
            inputs,
        })
        .collect();
    // The T spawn runway (open and flat westward): run, jump, land, then
    // keep running or let go. Shows CS:S's jump stamina (landing slowdown).
    let t_spawn = Vec3::new(-1024.0, -784.0, 140.0);
    let w = 180.0;
    let rest = || vec![Input::idle(w); 200];
    let mut hold = rest();
    hold.extend(vec![Input::idle(w).forward(); 45]);
    hold.push(Input::idle(w).forward().jump());
    hold.extend(vec![Input::idle(w).forward(); 100]);
    let mut release = rest();
    release.extend(vec![Input::idle(w).forward(); 45]);
    release.push(Input::idle(w).forward().jump());
    release.extend(vec![Input::idle(w).forward(); 46]);
    release.extend(vec![Input::idle(w); 50]);
    let mut hops = rest();
    for _ in 0..4 {
        hops.push(Input::idle(w).jump());
        hops.extend(vec![Input::idle(w); 49]);
    }
    for (name, inputs) in [
        ("tspawn_run_jump_hold", hold),
        ("tspawn_run_jump_release", release),
        ("tspawn_repeat_jumps", hops),
    ] {
        all.push(Scenario {
            name: name.to_string(),
            map: MAP.to_string(),
            start: t_spawn,
            yaw: w,
            inputs,
        });
    }
    // A standing jump where the running jumps take off.
    let mut here = settle();
    here.push(Input::idle(yaw).jump());
    here.extend(vec![Input::idle(yaw); 40]);
    all.push(Scenario {
        name: "jump_at_2346".to_string(),
        map: MAP.to_string(),
        start: Vec3::new(443.6, 2345.9, -110.0),
        yaw,
        inputs: here,
    });
    all
}

/// One tick of the trace: feet position and velocity in Source units.
#[derive(Clone, Copy, Debug, Default)]
struct State {
    pos: Vec3,
    vel: Vec3,
    ground: bool,
    ducked: bool,
    /// Water level 0-3.
    water: u8,
    ladder: bool,
    /// The buttons the game's movement actually ran with this tick (CS:S
    /// bots add their own, e.g. a duck on every jump).
    buttons: Option<u32>,
}

struct Args {
    only: Option<String>,
    keep_running: bool,
    fuzz: Option<Fuzz>,
}

struct Fuzz {
    seed: u64,
    runs: usize,
    ticks: usize,
    /// Fail when positions differ by more than this (units).
    tolerance: f32,
}

fn main() -> ExitCode {
    let mut args = Args {
        only: None,
        keep_running: false,
        fuzz: None,
    };
    let mut it = std::env::args().skip(1);
    let num = |v: Option<String>, what: &str| -> Result<f64, String> {
        v.and_then(|v| v.parse().ok()).ok_or(format!("{what} needs a number"))
    };
    while let Some(a) = it.next() {
        let fuzz = args.fuzz.get_or_insert_with(|| Fuzz {
            seed: 1,
            runs: 20,
            ticks: 200,
            tolerance: 2.0,
        });
        let parsed = match a.as_str() {
            "fuzz" => Ok(()),
            "--seed" => num(it.next(), "--seed").map(|v| fuzz.seed = v as u64),
            "--runs" => num(it.next(), "--runs").map(|v| fuzz.runs = v as usize),
            "--ticks" => num(it.next(), "--ticks").map(|v| fuzz.ticks = v as usize),
            "--tolerance" => num(it.next(), "--tolerance").map(|v| fuzz.tolerance = v as f32),
            "--only" => {
                args.only = it.next();
                Ok(())
            }
            "--keep-running" => {
                args.keep_running = true;
                Ok(())
            }
            "-h" | "--help" => {
                println!(
                    "usage: movecmp [--only <name>] [--keep-running]\n       \
                     movecmp fuzz [--seed N] [--runs N] [--ticks N] [--tolerance UNITS] [--only <map or kind>]\n  \
                     scenarios: {}\n  fuzz: random inputs at de_nuke ladders and de_aztec water",
                    scenarios()
                        .iter()
                        .map(|s| s.name.as_str())
                        .collect::<Vec<_>>()
                        .join(", ")
                );
                return ExitCode::SUCCESS;
            }
            other => Err(format!("unknown argument {other}")),
        };
        if let Err(e) = parsed {
            eprintln!("{e}");
            return ExitCode::from(2);
        }
    }
    // Only `fuzz` (or a fuzz option) turns on fuzzing.
    if !std::env::args().any(|a| a == "fuzz") {
        args.fuzz = None;
    }
    match run(&args) {
        Ok(true) => ExitCode::SUCCESS,
        Ok(false) => ExitCode::FAILURE,
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
    }
}

/// Runs the scenarios; false when a fuzz run failed.
fn run(args: &Args) -> Result<bool, String> {
    let config = LocalConfig::load()?;
    let server = config
        .game_path("cs_source_server")
        .ok_or("no [games.cs_source_server] path in mashup.local.toml (see mashup.local.example.toml)")?;
    let out_dir = default_dump_dir("movecmp").ok_or("no data dir")?;
    std::fs::create_dir_all(&out_dir).map_err(|e| e.to_string())?;

    let started = ensure_server(&server, &out_dir)?;
    let mut maps: std::collections::HashMap<String, Arc<MapData>> = Default::default();
    let mut load = |name: &str| -> Result<Arc<MapData>, String> {
        if let Some(m) = maps.get(name) {
            return Ok(m.clone());
        }
        let m = Arc::new(games::load_map(&format!("cs_source:{name}"))?);
        maps.insert(name.to_string(), m.clone());
        Ok(m)
    };
    let list = match &args.fuzz {
        Some(f) => {
            let mut all = Vec::new();
            for map in ["de_nuke", "de_aztec"] {
                if args
                    .only
                    .as_deref()
                    .is_some_and(|o| !map.contains(o) && !"ladder water".contains(o))
                {
                    continue;
                }
                all.extend(fuzz_scenarios(map, &load(map)?, f, args.only.as_deref()));
            }
            all
        }
        None => scenarios()
            .into_iter()
            .filter(|s| args.only.as_deref().is_none_or(|o| s.name.contains(o)))
            .collect(),
    };

    println!(
        "\nscenario                  ticks  max pos diff  max vel diff  first tick > 0.1 u (or water/ladder mismatch)"
    );
    let mut failed = 0;
    for s in &list {
        let map = load(&s.map)?;
        ensure_map(&s.map)?;
        let theirs = run_css(&server, s)?;
        let ours = run_ours(&map, s, &theirs);
        let (max_pos, mismatched) = report(s, &theirs, &ours, &out_dir)?;
        if let Some(f) = &args.fuzz
            && (max_pos > f.tolerance || mismatched > 0)
        {
            failed += 1;
        }
    }
    println!("\nper-tick CSVs in {}", out_dir.display());
    if let Some(f) = &args.fuzz {
        println!(
            "fuzz (seed {}): {} of {} runs within {} units, water level and ladder state agreeing",
            f.seed,
            list.len() - failed,
            list.len(),
            f.tolerance
        );
    }
    if started && !args.keep_running {
        stop_server();
    }
    Ok(failed == 0)
}

/// Switch the probe server to `map` if it's on another one.
fn ensure_map(map: &str) -> Result<(), String> {
    let on = |r: &mut Rcon| -> bool {
        r.exec("status")
            .map(|s| {
                s.lines()
                    .any(|l| l.starts_with("map") && l.split_whitespace().any(|w| w == map))
            })
            .unwrap_or(false)
    };
    let mut r = Rcon::connect(RCON_ADDR, RCON_PASSWORD).map_err(|e| e.to_string())?;
    if on(&mut r) {
        return Ok(());
    }
    println!("probe server: changelevel {map}");
    let _ = r.exec(&format!("changelevel {map}"));
    let start = Instant::now();
    loop {
        sleep(Duration::from_secs(2));
        if let Ok(mut r) = Rcon::connect(RCON_ADDR, RCON_PASSWORD)
            && on(&mut r)
            && check_ready(&mut r).is_ok()
        {
            // Let the bot spawn.
            sleep(Duration::from_secs(3));
            return Ok(());
        }
        if start.elapsed() > Duration::from_secs(90) {
            return Err(format!("the probe server didn't switch to {map} in 90 s"));
        }
    }
}

// ------------------------------------------------------------------- fuzz

/// xorshift64*: deterministic per seed.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }
    fn unit(&mut self) -> f32 {
        (self.next() >> 40) as f32 / (1u64 << 24) as f32
    }
    fn range(&mut self, lo: f32, hi: f32) -> f32 {
        lo + (hi - lo) * self.unit()
    }
    fn pick<T: Copy>(&mut self, items: &[T]) -> T {
        items[(self.next() % items.len() as u64) as usize]
    }
}

/// Places to start from: in front of each ladder (facing it), and in each
/// water volume deep enough to swim (waist and head deep), checked in our
/// sim to be free (and, for ladders, climbable with forward held).
fn fuzz_starts(map: &Arc<MapData>) -> Vec<(&'static str, Vec3, f32)> {
    let mut out = Vec::new();
    // Settle for 40 ticks, then hold forward for 20: a start is usable when
    // the player isn't stuck (forward moves it) and didn't fall far.
    let settle = |feet: Vec3, yaw: f32| -> Option<(Vec3, State)> {
        let s = Scenario {
            name: String::new(),
            map: String::new(),
            start: feet,
            yaw,
            inputs: (0..60)
                .map(|t| {
                    let i = Input::idle(yaw);
                    if t >= 40 { i.forward() } else { i }
                })
                .collect(),
        };
        let start = State { pos: feet, ..default() };
        let trace = run_ours(map, &s, &[start]);
        let (at, end) = (trace[40], *trace.last().unwrap());
        // Start on the ground (or in water), not mid-fall.
        let resting = at.ground || at.water >= 2;
        let moved = at.pos.distance(end.pos) > 8.0;
        let fell = feet.z - at.pos.z > 80.0;
        (moved && !fell && resting).then_some((at.pos, end))
    };
    for b in map.collision_brushes.iter().filter(|b| b.ladder) {
        let (lo, hi) = (
            to_source(b.min).min(to_source(b.max)),
            to_source(b.min).max(to_source(b.max)),
        );
        let centre = (lo + hi) / 2.0;
        for (d, yaw) in [
            (Vec3::X, 180.0),
            (Vec3::NEG_X, 0.0),
            (Vec3::Y, 270.0),
            (Vec3::NEG_Y, 90.0),
        ] {
            let reach = (hi - lo).dot(d.abs()) / 2.0 + 17.0;
            let feet = Vec3::new(centre.x, centre.y, lo.z + 64.0) + d * reach;
            if let Some((at, end)) = settle(feet, yaw)
                && end.ladder
            {
                out.push(("ladder", at, yaw));
                break;
            }
        }
    }
    for w in &map.water {
        let (lo, hi) = (
            to_source(w.brush.min).min(to_source(w.brush.max)),
            to_source(w.brush.min).max(to_source(w.brush.max)),
        );
        if hi.z - lo.z < 80.0 || (hi - lo).x < 64.0 || (hi - lo).y < 64.0 {
            continue;
        }
        let c = (lo + hi) / 2.0;
        for depth in [50.0, 90.0] {
            let feet = Vec3::new(c.x, c.y, hi.z - depth);
            if feet.z > lo.z
                && let Some((at, end)) = settle(feet, 0.0)
                && end.water >= 2
            {
                out.push(("water", at, 0.0));
            }
        }
    }
    out
}

fn fuzz_scenarios(map_name: &str, map: &Arc<MapData>, f: &Fuzz, only: Option<&str>) -> Vec<Scenario> {
    let starts: Vec<_> = fuzz_starts(map)
        .into_iter()
        .filter(|(kind, ..)| only.is_none_or(|o| map_name.contains(o) || kind.contains(o)))
        .collect();
    println!("{map_name}: {} fuzz starts", starts.len());
    if starts.is_empty() {
        return Vec::new();
    }
    // First, per ladder: walk straight at it, looking up a little, and climb.
    let mut out: Vec<Scenario> = starts
        .iter()
        .filter(|(kind, ..)| *kind == "ladder")
        .enumerate()
        .map(|(i, (_, start, yaw))| {
            let mut inputs = vec![Input::idle(*yaw); 10];
            inputs.extend(vec![Input::idle(*yaw).look(-30.0).forward(); 90]);
            Scenario {
                name: format!("climb_{map_name}_{i}"),
                map: map_name.to_string(),
                start: *start,
                yaw: *yaw,
                inputs,
            }
        })
        .collect();
    let mut rng = Rng(f.seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1);
    out.extend((0..f.runs).map(|run| {
        let (kind, start, yaw0) = starts[run % starts.len()];
        let (mut yaw, mut pitch) = (yaw0, 0.0f32);
        let mut inputs = vec![Input::idle(yaw); 10];
        while inputs.len() < f.ticks + 10 {
            let len = rng.range(5.0, 40.0) as usize;
            let fwd = rng.pick(&[-1i8, 0, 1, 1]);
            let side = rng.pick(&[-1i8, 0, 0, 1]);
            let jump = rng.unit() < 0.3;
            let duck = rng.unit() < 0.15;
            let walk = rng.unit() < 0.2;
            let turn = rng.range(-3.0, 3.0);
            let target_pitch = rng.range(-70.0, 70.0);
            for _ in 0..len {
                yaw = (yaw + turn).rem_euclid(360.0);
                pitch += (target_pitch - pitch).clamp(-4.0, 4.0);
                let mut i = Input::idle(yaw).look(pitch);
                i = match fwd {
                    1 => i.forward(),
                    -1 => i.back(),
                    _ => i,
                };
                if side != 0 {
                    i = i.strafe(side as f32);
                }
                if jump {
                    i = i.jump();
                }
                if duck {
                    i = i.duck();
                }
                if walk {
                    i = i.walk();
                }
                inputs.push(i);
            }
        }
        inputs.truncate(f.ticks + 10);
        Scenario {
            name: format!("fuzz_{map_name}_{kind}_{}_{run}", f.seed),
            map: map_name.to_string(),
            start,
            yaw: yaw0,
            inputs,
        }
    }));
    out
}

// ---------------------------------------------------------------- CS:S side

/// Connect to the probe server, starting it if needed. Returns whether we
/// started it.
fn ensure_server(server: &Path, out_dir: &Path) -> Result<bool, String> {
    if let Ok(mut r) = Rcon::connect(RCON_ADDR, RCON_PASSWORD) {
        check_ready(&mut r)?;
        return Ok(false);
    }
    println!("starting the CS:S probe server ({})", server.display());
    let log = std::fs::File::create(out_dir.join("srcds.log")).map_err(|e| e.to_string())?;
    Command::new("./srcds_run")
        .current_dir(server)
        .args([
            "-norestart",
            "-game",
            "cstrike",
            "-console",
            "+ip",
            "127.0.0.1",
            "-port",
            "27030",
            "-insecure",
            "+sv_lan",
            "1",
            "+maxplayers",
            "4",
            "+map",
            MAP,
            "+exec",
            "mashup_probe.cfg",
        ])
        .stdin(Stdio::null())
        .stdout(log.try_clone().map_err(|e| e.to_string())?)
        .stderr(log)
        .spawn()
        .map_err(|e| format!("starting srcds_run: {e}"))?;
    let start = Instant::now();
    loop {
        sleep(Duration::from_secs(2));
        if let Ok(mut r) = Rcon::connect(RCON_ADDR, RCON_PASSWORD)
            && check_ready(&mut r).is_ok()
        {
            return Ok(true);
        }
        if start.elapsed() > Duration::from_secs(90) {
            return Err("the probe server didn't come up in 90 s (see srcds.log)".into());
        }
    }
}

fn check_ready(r: &mut Rcon) -> Result<(), String> {
    let plugins = r.exec("sm plugins list").map_err(|e| e.to_string())?;
    if !plugins.contains("mashup probe") {
        return Err("the server has no mashup_probe plugin (tools/css_probe/README.md)".into());
    }
    let status = r.exec("status").map_err(|e| e.to_string())?;
    if !status.contains(" 1 bots") && !status.contains("BOT") {
        return Err("no bot on the probe server yet".into());
    }
    Ok(())
}

fn stop_server() {
    for name in ["srcds_run", "srcds_linux"] {
        let _ = Command::new("pkill").args(["-x", name]).status();
    }
}

fn run_css(server: &Path, s: &Scenario) -> Result<Vec<State>, String> {
    let dir = server.join("cstrike").join("mashup");
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let (input, output) = (dir.join(format!("{}.in", s.name)), dir.join(format!("{}.out", s.name)));
    let mut text = format!("start {} {} {} 0 {} 0 0 0\n", s.start.x, s.start.y, s.start.z, s.yaw);
    for i in &s.inputs {
        text += &format!("{} {} {} {} {}\n", i.buttons, i.forward, i.side, i.pitch, i.yaw);
    }
    std::fs::write(&input, text).map_err(|e| e.to_string())?;
    let _ = std::fs::remove_file(&output);
    let mut r = Rcon::connect(RCON_ADDR, RCON_PASSWORD).map_err(|e| e.to_string())?;
    r.exec(&format!("mashup_run mashup/{0}.in mashup/{0}.out", s.name))
        .map_err(|e| e.to_string())?;
    let want = s.inputs.len() + 1;
    let start = Instant::now();
    loop {
        sleep(Duration::from_millis(200));
        let rows: Vec<State> = std::fs::read_to_string(&output)
            .unwrap_or_default()
            .lines()
            .filter_map(parse_row)
            .collect();
        if rows.len() >= want {
            return Ok(rows);
        }
        if start.elapsed() > Duration::from_secs(30) {
            return Err(format!("{}: CS:S logged {} of {want} ticks", s.name, rows.len()));
        }
    }
}

fn parse_row(line: &str) -> Option<State> {
    let f: Vec<f32> = line.split_whitespace().filter_map(|v| v.parse().ok()).collect();
    (f.len() >= 9).then(|| State {
        pos: Vec3::new(f[1], f[2], f[3]),
        vel: Vec3::new(f[4], f[5], f[6]),
        ground: f[7] != 0.0,
        ducked: f[8] != 0.0,
        // Older plugin builds don't log these.
        water: f.get(12).map_or(0, |v| *v as u8),
        ladder: f.get(13).is_some_and(|v| *v != 0.0),
        buttons: f.get(11).map(|v| *v as u32),
    })
}

// ------------------------------------------------------------- mashup side

/// The transform is the standing box's centre, 36 units above the feet.
const ORIGIN_ABOVE_FEET: f32 = 36.0;

/// Play a scenario in mashup. With the game's trace, jump and duck come
/// from the buttons its movement actually ran with, so the bot's own
/// additions are played too.
fn run_ours(map: &Arc<MapData>, s: &Scenario, theirs: &[State]) -> Vec<State> {
    let start = &theirs[0];
    let mut sim = Sim::new((
        MapPlugin {
            data: Some(map.clone()),
            view: MapDebugView::Normal,
        },
        SourceMovementPlugin,
    ));
    sim.set_tick_interval(TICK_INTERVAL);
    sim.app.insert_resource(SourceMovementConfig::default());
    let p = sim.spawn_character(to_engine(start.pos + Vec3::Z * ORIGIN_ABOVE_FEET), movement::ID);
    {
        let world = sim.app.world_mut();
        world.get_mut::<Velocity>(p).unwrap().0 = to_engine(start.vel);
        let mut me = world.get_mut::<SourceMovement>(p).unwrap();
        me.on_ground = start.ground;
        me.ducked = start.ducked;
    }
    let state = |sim: &Sim| {
        let me = sim.app.world().get::<SourceMovement>(p).unwrap();
        State {
            pos: to_source(sim.position(p)) - Vec3::Z * ORIGIN_ABOVE_FEET,
            vel: to_source(sim.velocity(p)),
            ground: me.on_ground,
            ducked: me.ducked,
            water: me.water_level,
            ladder: me.ladder.is_some(),
            buttons: None,
        }
    };
    let mut out = vec![state(&sim)];
    for (t, i) in s.inputs.iter().enumerate() {
        let buttons = theirs.get(t + 1).and_then(|x| x.buttons).unwrap_or(i.buttons);
        {
            let mut intent = sim.intent(p);
            intent.move_axis = Vec2::new(i.side / KEY, i.forward / KEY);
            intent.yaw = i.yaw.to_radians() - FRAC_PI_2;
            intent.pitch = -i.pitch.to_radians();
            intent.jump = buttons & IN_JUMP != 0;
            // CS:S bots duck-jump on their own, inside the game rather than
            // through their buttons (a human doesn't): hold duck while the
            // bot is ducked in the air without the key. Jumps then differ
            // only by the bot's instant duck on the jump tick (8.5 units up).
            let bot_ducked = theirs
                .get(t + 1)
                .is_some_and(|x| x.ducked && !x.ground && x.buttons.is_some());
            intent.crouch = buttons & IN_DUCK != 0 || bot_ducked;
            intent.walk = buttons & IN_SPEED != 0;
        }
        sim.ticks(1);
        out.push(state(&sim));
    }
    out
}

// ------------------------------------------------------------------ report

/// Compare one scenario's traces; writes a CSV and prints a line. Returns
/// the largest position difference and how many ticks the water level or
/// ladder state disagreed.
fn report(s: &Scenario, theirs: &[State], ours: &[State], out_dir: &Path) -> Result<(f32, usize), String> {
    let n = theirs.len().min(ours.len());
    let (mut max_pos, mut max_vel) = (0.0f32, 0.0f32);
    let mut first: Option<usize> = None;
    let mut mismatched = 0;
    let mut csv = String::from(
        "tick,css_x,css_y,css_z,css_vx,css_vy,css_vz,css_ground,css_ducked,css_water,css_ladder,\
         ours_x,ours_y,ours_z,ours_vx,ours_vy,ours_vz,ours_ground,ours_ducked,ours_water,ours_ladder\n",
    );
    let row = |x: &State| {
        format!(
            "{},{},{},{},{},{},{},{},{},{}",
            x.pos.x,
            x.pos.y,
            x.pos.z,
            x.vel.x,
            x.vel.y,
            x.vel.z,
            x.ground as u8,
            x.ducked as u8,
            x.water,
            x.ladder as u8
        )
    };
    for t in 0..n {
        let (a, b) = (theirs[t], ours[t]);
        let dp = a.pos.distance(b.pos);
        max_pos = max_pos.max(dp);
        max_vel = max_vel.max(a.vel.distance(b.vel));
        // Tick 0 is the placement, before either side has categorized it.
        let state_differs = t > 0 && (a.water != b.water || a.ladder != b.ladder);
        mismatched += state_differs as usize;
        if first.is_none() && (dp > 0.1 || a.ground != b.ground || a.ducked != b.ducked || state_differs) {
            first = Some(t);
        }
        csv += &format!("{t},{},{}\n", row(&a), row(&b));
    }
    std::fs::write(out_dir.join(format!("{}.csv", s.name)), csv).map_err(|e| e.to_string())?;
    let show = |x: &State| {
        format!(
            "{:.2?} v {:.1?} g{} d{} w{} l{}",
            x.pos.to_array(),
            x.vel.to_array(),
            x.ground as u8,
            x.ducked as u8,
            x.water,
            x.ladder as u8
        )
    };
    let first_text = match first {
        None => "none".to_string(),
        Some(t) => format!("{t}: css {} | ours {}", show(&theirs[t]), show(&ours[t])),
    };
    println!(
        "{:<25} {:>6} {:>13.3} {:>13.3}  {first_text}",
        s.name, n, max_pos, max_vel
    );
    Ok((max_pos, mismatched))
}
