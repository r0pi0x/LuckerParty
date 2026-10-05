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
const IN_MOVELEFT: u32 = 1 << 9;
const IN_MOVERIGHT: u32 = 1 << 10;
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
}

struct Scenario {
    name: &'static str,
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

    out.into_iter()
        .map(|(name, inputs)| Scenario {
            name,
            start: CT_SPAWN,
            yaw,
            inputs,
        })
        .collect()
}

/// One tick of the trace: feet position and velocity in Source units.
#[derive(Clone, Copy, Debug, Default)]
struct State {
    pos: Vec3,
    vel: Vec3,
    ground: bool,
    ducked: bool,
}

struct Args {
    only: Option<String>,
    keep_running: bool,
}

fn main() -> ExitCode {
    let mut args = Args {
        only: None,
        keep_running: false,
    };
    let mut it = std::env::args().skip(1);
    while let Some(a) = it.next() {
        match a.as_str() {
            "--only" => args.only = it.next(),
            "--keep-running" => args.keep_running = true,
            "-h" | "--help" => {
                println!(
                    "usage: movecmp [--only <name>] [--keep-running]\n  scenarios: {}",
                    scenarios().iter().map(|s| s.name).collect::<Vec<_>>().join(", ")
                );
                return ExitCode::SUCCESS;
            }
            other => {
                eprintln!("unknown argument {other}");
                return ExitCode::from(2);
            }
        }
    }
    match run(&args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
    }
}

fn run(args: &Args) -> Result<(), String> {
    let config = LocalConfig::load()?;
    let server = config
        .game_path("cs_source_server")
        .ok_or("no [games.cs_source_server] path in mashup.local.toml (see mashup.local.example.toml)")?;
    let out_dir = default_dump_dir("movecmp").ok_or("no data dir")?;
    std::fs::create_dir_all(&out_dir).map_err(|e| e.to_string())?;

    let started = ensure_server(&server, &out_dir)?;
    let map = Arc::new(games::load_map(&format!("cs_source:{MAP}"))?);

    println!("\nscenario        ticks  max pos diff  max vel diff  first tick > 0.1 u");
    for s in scenarios() {
        if args.only.as_deref().is_some_and(|o| !s.name.contains(o)) {
            continue;
        }
        let theirs = run_css(&server, &s)?;
        let ours = run_ours(&map, &s, &theirs[0]);
        report(&s, &theirs, &ours, &out_dir)?;
    }
    println!("\nper-tick CSVs in {}", out_dir.display());
    if started && !args.keep_running {
        stop_server();
    }
    Ok(())
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
    })
}

// ------------------------------------------------------------- mashup side

/// The transform is the standing box's centre, 36 units above the feet.
const ORIGIN_ABOVE_FEET: f32 = 36.0;

fn run_ours(map: &Arc<MapData>, s: &Scenario, start: &State) -> Vec<State> {
    let mut sim = Sim::new((
        MapPlugin {
            data: map.clone(),
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
        }
    };
    let mut out = vec![state(&sim)];
    for i in &s.inputs {
        {
            let mut intent = sim.intent(p);
            intent.move_axis = Vec2::new(i.side / KEY, i.forward / KEY);
            intent.yaw = i.yaw.to_radians() - FRAC_PI_2;
            intent.pitch = -i.pitch.to_radians();
            intent.jump = i.buttons & IN_JUMP != 0;
            intent.crouch = i.buttons & IN_DUCK != 0;
        }
        sim.ticks(1);
        out.push(state(&sim));
    }
    out
}

// ------------------------------------------------------------------ report

fn report(s: &Scenario, theirs: &[State], ours: &[State], out_dir: &Path) -> Result<(), String> {
    let n = theirs.len().min(ours.len());
    let (mut max_pos, mut max_vel) = (0.0f32, 0.0f32);
    let mut first: Option<usize> = None;
    let mut csv = String::from(
        "tick,css_x,css_y,css_z,css_vx,css_vy,css_vz,css_ground,css_ducked,ours_x,ours_y,ours_z,ours_vx,ours_vy,ours_vz,ours_ground,ours_ducked\n",
    );
    for t in 0..n {
        let (a, b) = (theirs[t], ours[t]);
        let dp = a.pos.distance(b.pos);
        max_pos = max_pos.max(dp);
        max_vel = max_vel.max(a.vel.distance(b.vel));
        if first.is_none() && (dp > 0.1 || a.ground != b.ground || a.ducked != b.ducked) {
            first = Some(t);
        }
        csv += &format!(
            "{t},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{}\n",
            a.pos.x,
            a.pos.y,
            a.pos.z,
            a.vel.x,
            a.vel.y,
            a.vel.z,
            a.ground as u8,
            a.ducked as u8,
            b.pos.x,
            b.pos.y,
            b.pos.z,
            b.vel.x,
            b.vel.y,
            b.vel.z,
            b.ground as u8,
            b.ducked as u8
        );
    }
    std::fs::write(out_dir.join(format!("{}.csv", s.name)), csv).map_err(|e| e.to_string())?;
    let first_text = match first {
        None => "none".to_string(),
        Some(t) => {
            let (a, b) = (theirs[t], ours[t]);
            format!(
                "{t}: css {:.2?} v {:.1?} g{} d{} | ours {:.2?} v {:.1?} g{} d{}",
                a.pos.to_array(),
                a.vel.to_array(),
                a.ground as u8,
                a.ducked as u8,
                b.pos.to_array(),
                b.vel.to_array(),
                b.ground as u8,
                b.ducked as u8
            )
        }
    };
    println!(
        "{:<14} {:>6} {:>13.3} {:>13.3}  {first_text}",
        s.name, n, max_pos, max_vel
    );
    Ok(())
}
