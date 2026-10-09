//! Instructions-per-tick regression gate (docs/performance.md,
//! "Regression gate"): headless scenarios with fixed inputs, measured by
//! hardware counters (user-space instructions retired, summed over every
//! thread), compared with a stored baseline. Unlike frame times, the
//! counts barely move with the machine's load.
//!
//! Scenarios: `greybox` (always), `de_dust2` (rounds with 10 bots; needs a
//! CS:S install), `mg_lego_multigames_v2` (a community map, when it is in
//! the content cache). Each runs `--warmup` ticks, then measures `--ticks`
//! ticks one by one.

use std::{
    path::{Path, PathBuf},
    time::Instant,
};

use bevy::prelude::*;
use mashup::{
    console::Console,
    core::{Intent, Team},
    games::{
        self,
        cs_source::{self, TICK_INTERVAL, movement::SourceMovementPlugin, weapons::CsWeaponsPlugin},
    },
    greybox::{self, GreyboxMapPlugin},
    harness::Sim,
    map::MapPlugin,
    metrics::{AllThreads, Counts, Percentiles, ThreadCounters, hw_status, short},
    mount::config::{LocalConfig, content_dir},
};
use serde::{Deserialize, Serialize};

const USAGE: &str = "\
usage: perfgate [--scenario <name>]... [--ticks N] [--warmup N] [--tolerance PCT]
                [--baseline <file>] [--update] [--json <file>]
  Runs headless scenarios with fixed inputs and counts user-space
  instructions per tick (all threads) with hardware counters; fails when a
  scenario's total rises more than the tolerance above the baseline for
  this build profile. Skips (exit 0) where counters are unavailable.
  --scenario <name>  greybox, de_dust2, mg_lego_multigames_v2 (default: all
                     available)
  --ticks N          ticks measured per scenario (default 2000)
  --warmup N         ticks run first, not measured (default 300)
  --tolerance PCT    allowed rise in percent (default: the baseline's)
  --baseline <file>  default tools/perfgate/baseline.json
  --update           write this run's numbers as the baseline for this
                     build profile (after checking a change is wanted)
  --json <file>      also write this run's numbers there";

const SCENARIOS: [&str; 3] = ["greybox", "de_dust2", "mg_lego_multigames_v2"];

/// Default allowed rise (percent): see docs/performance.md, "Regression
/// gate", for the measured run-to-run spread it is chosen from.
const DEFAULT_TOLERANCE: f64 = 3.0;

#[derive(Serialize, Deserialize, Clone, Debug)]
struct Result {
    ticks: u64,
    /// Instructions per tick, all threads.
    instructions_total: u64,
    instructions_p50: f64,
    instructions_p99: f64,
    /// Instructions per tick on the main thread only.
    main_instructions_p50: f64,
    main_instructions_total: u64,
    /// CPU time per tick, all threads (ms), and wall time per tick: for
    /// reading, not gated (they follow the machine's load).
    cpu_ms_p50: f64,
    cpu_ms_p99: f64,
    wall_ms_p50: f64,
    wall_ms_p99: f64,
    cycles_total: u64,
    cache_misses_total: u64,
}

#[derive(Serialize, Deserialize, Default)]
struct Baselines {
    /// Allowed rise, percent, and per scenario where its own run-to-run
    /// spread needs more (greybox: ticks of under a million instructions,
    /// where the task pools' overhead weighs more).
    tolerance_pct: f64,
    #[serde(default)]
    scenario_tolerance_pct: std::collections::BTreeMap<String, f64>,
    /// By build profile (`playtest`, `debug`, ...), then scenario.
    profiles: std::collections::BTreeMap<String, Profile>,
}

#[derive(Serialize, Deserialize, Default, Clone)]
struct Profile {
    /// The commit the numbers were taken at, and the CPU.
    build: String,
    cpu: String,
    scenarios: std::collections::BTreeMap<String, Result>,
}

struct Args {
    scenarios: Vec<String>,
    ticks: u64,
    warmup: u64,
    tolerance: Option<f64>,
    baseline: PathBuf,
    update: bool,
    json: Option<PathBuf>,
}

fn parse_args() -> std::result::Result<Args, String> {
    let mut a = Args {
        scenarios: Vec::new(),
        ticks: 2000,
        warmup: 300,
        tolerance: None,
        baseline: Path::new(env!("CARGO_MANIFEST_DIR")).join("tools/perfgate/baseline.json"),
        update: false,
        json: None,
    };
    let mut it = std::env::args().skip(1);
    while let Some(arg) = it.next() {
        let mut value = || it.next().ok_or(format!("{arg} needs a value"));
        match arg.as_str() {
            "--scenario" => a.scenarios.push(value()?),
            "--ticks" => a.ticks = value()?.parse().map_err(|e| format!("--ticks: {e}"))?,
            "--warmup" => a.warmup = value()?.parse().map_err(|e| format!("--warmup: {e}"))?,
            "--tolerance" => a.tolerance = Some(value()?.parse().map_err(|e| format!("--tolerance: {e}"))?),
            "--baseline" => a.baseline = value()?.into(),
            "--json" => a.json = Some(value()?.into()),
            "--update" => a.update = true,
            "-h" | "--help" => return Err(String::new()),
            other => return Err(format!("unknown argument {other}")),
        }
    }
    for s in &a.scenarios {
        if !SCENARIOS.contains(&s.as_str()) {
            return Err(format!("unknown scenario {s}"));
        }
    }
    Ok(a)
}

fn main() {
    let args = match parse_args() {
        Ok(a) => a,
        Err(e) => {
            if !e.is_empty() {
                eprintln!("{e}");
            }
            eprintln!("{USAGE}");
            std::process::exit(2);
        }
    };
    // Counters first: without them there is nothing to gate.
    let mut probe = ThreadCounters::current();
    if probe.read().hw.is_none() {
        println!("perfgate: skipped, hardware counters {}", hw_status());
        return;
    }
    let profile = build_profile();
    let wanted: Vec<&str> = if args.scenarios.is_empty() {
        SCENARIOS.to_vec()
    } else {
        args.scenarios.iter().map(String::as_str).collect()
    };
    let mut results = std::collections::BTreeMap::new();
    for name in wanted {
        let Some(sim) = scenario(name) else {
            println!("{name}: skipped (map not installed or cached)");
            continue;
        };
        let r = measure(sim, args.warmup, args.ticks);
        println!(
            "{name}: {} ticks, instructions per tick p50 {} p99 {} total {} (main thread p50 {}); \
             cpu ms p50 {:.3} p99 {:.3}; wall ms p50 {:.3} p99 {:.3}",
            r.ticks,
            short(r.instructions_p50),
            short(r.instructions_p99),
            short(r.instructions_total as f64),
            short(r.main_instructions_p50),
            r.cpu_ms_p50,
            r.cpu_ms_p99,
            r.wall_ms_p50,
            r.wall_ms_p99
        );
        results.insert(name.to_string(), r);
    }
    let mut baselines: Baselines = std::fs::read_to_string(&args.baseline)
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or(Baselines {
            tolerance_pct: DEFAULT_TOLERANCE,
            ..default()
        });
    let this = Profile {
        build: env!("MASHUP_GIT").to_string(),
        cpu: cpu_model(),
        scenarios: results.clone(),
    };
    if let Some(path) = &args.json
        && let Err(e) = std::fs::write(path, serde_json::to_string_pretty(&this).unwrap())
    {
        eprintln!("{}: {e}", path.display());
    }
    if args.update {
        let entry = baselines.profiles.entry(profile.clone()).or_default();
        entry.build = this.build.clone();
        entry.cpu = this.cpu.clone();
        entry.scenarios.extend(results);
        if let Some(t) = args.tolerance {
            baselines.tolerance_pct = t;
        }
        if let Some(dir) = args.baseline.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        match std::fs::write(&args.baseline, serde_json::to_string_pretty(&baselines).unwrap() + "\n") {
            Ok(()) => println!("baseline for `{profile}` written to {}", args.baseline.display()),
            Err(e) => {
                eprintln!("{}: {e}", args.baseline.display());
                std::process::exit(1);
            }
        }
        return;
    }
    let Some(base) = baselines.profiles.get(&profile) else {
        println!(
            "no baseline for build profile `{profile}` in {}: nothing compared (--update writes one)",
            args.baseline.display()
        );
        return;
    };
    if base.cpu != this.cpu {
        println!("note: the baseline was taken on {:?}, this is {:?}", base.cpu, this.cpu);
    }
    let mut failed = Vec::new();
    for (name, r) in &this.scenarios {
        let Some(b) = base.scenarios.get(name) else {
            println!("{name}: no baseline");
            continue;
        };
        if b.ticks != r.ticks {
            println!(
                "{name}: baseline has {} ticks, this run {}: not compared",
                b.ticks, r.ticks
            );
            continue;
        }
        let tolerance = args
            .tolerance
            .or(baselines.scenario_tolerance_pct.get(name).copied())
            .unwrap_or(baselines.tolerance_pct);
        let change = (r.instructions_total as f64 / b.instructions_total.max(1) as f64 - 1.0) * 100.0;
        let verdict = if change > tolerance {
            failed.push(format!("{name} (tolerance {tolerance}%)"));
            "FAIL"
        } else if change < -tolerance {
            "faster (update the baseline if intended)"
        } else {
            "ok"
        };
        println!(
            "{name}: {} instructions vs baseline {} ({change:+.2}%, tolerance {tolerance}%): {verdict}",
            short(r.instructions_total as f64),
            short(b.instructions_total as f64)
        );
    }
    if !failed.is_empty() {
        eprintln!(
            "perfgate: {} rose by more than the tolerance in instructions: find the cause (tracesum, \
             mashup_hitch_ratio), or if the cost is wanted, `perfgate --update` (docs/performance.md)",
            failed.join(", ")
        );
        std::process::exit(1);
    }
}

/// The scenario's simulation, ready to tick; None when its map is missing.
fn scenario(name: &str) -> Option<Sim> {
    let mut sim = match name {
        "greybox" => {
            let mut sim = Sim::new(GreyboxMapPlugin);
            // Characters walking and turning on fixed patterns (`drive`).
            for (i, at) in greybox::SPAWNS.iter().enumerate() {
                let e = sim.spawn_character(*at, mashup::movement::placeholder::ID);
                sim.app.world_mut().entity_mut(e).insert(Driven(i as u32));
            }
            sim.app.add_systems(FixedPreUpdate, drive);
            sim
        }
        "de_dust2" => {
            if !installed() {
                return None;
            }
            let mut sim = cs_map("de_dust2")?;
            for team in [1, 2] {
                for _ in 0..5 {
                    mashup::bot::add_bot(sim.app.world_mut(), Team(team)).expect("bot");
                }
            }
            sim.app
                .world_mut()
                .resource_mut::<Console>()
                .submit("mp_freezetime 2; mp_roundtime 2; mashup_rounds 1");
            sim
        }
        "mg_lego_multigames_v2" => {
            let cached =
                content_dir(cs_source::GAME).is_some_and(|d| d.join("maps").join(format!("{name}.bsp")).is_file());
            if !installed() || !cached {
                return None;
            }
            cs_map(name)?
        }
        _ => return None,
    };
    single_threaded_schedules(&mut sim.app);
    Some(sim)
}

fn installed() -> bool {
    LocalConfig::load()
        .ok()
        .and_then(|c| c.game_path(cs_source::GAME))
        .is_some_and(|p| p.join("cstrike").is_dir())
}

fn cs_map(name: &str) -> Option<Sim> {
    let map = games::load_map(&format!("cs_source:{name}")).ok()?;
    let mut sim = Sim::new((MapPlugin::new(map), SourceMovementPlugin, CsWeaponsPlugin));
    sim.set_tick_interval(TICK_INTERVAL);
    sim.app.insert_resource(mashup::slots::Loadout {
        movement: cs_source::movement::ID,
    });
    Some(sim)
}

/// A greybox character's pattern number.
#[derive(Component)]
struct Driven(u32);

/// Walk, strafe, turn and jump on a schedule of the tick number: the same
/// every run.
fn drive(tick: Res<mashup::core::SimTick>, mut q: Query<(&Driven, &mut Intent)>) {
    for (d, mut i) in &mut q {
        let t = tick.0 + d.0 as u64 * 37;
        let phase = (t / 64) % 4;
        i.move_axis = [Vec2::Y, Vec2::X, -Vec2::Y, -Vec2::X][phase as usize];
        i.yaw = (t % 360) as f32 * 0.0174533;
        i.jump = t.is_multiple_of(97);
    }
}

/// As the game runs them (src/main.rs): every per-frame and fixed schedule
/// on one thread, so the counts don't depend on how work was handed out.
fn single_threaded_schedules(app: &mut App) {
    use bevy::ecs::{intern::Interned, schedule::ScheduleLabel, schedule::SingleThreadedExecutor};
    let labels: [Interned<dyn ScheduleLabel>; 10] = [
        First.intern(),
        PreUpdate.intern(),
        Update.intern(),
        PostUpdate.intern(),
        Last.intern(),
        FixedFirst.intern(),
        FixedPreUpdate.intern(),
        FixedUpdate.intern(),
        FixedPostUpdate.intern(),
        FixedLast.intern(),
    ];
    for label in labels {
        app.edit_schedule(label, |s| {
            s.set_executor(SingleThreadedExecutor::new());
        });
    }
}

/// Run `warmup` ticks, then measure `ticks` ticks one at a time (a `Sim`
/// update is one tick).
fn measure(mut sim: Sim, warmup: u64, ticks: u64) -> Result {
    sim.ticks(warmup);
    let mut all = AllThreads::default();
    let mut main = ThreadCounters::current();
    let (mut ins, mut main_ins, mut cpu, mut wall) = (Vec::new(), Vec::new(), Vec::new(), Vec::new());
    let mut total = Counts::default();
    let mut main_total = 0u64;
    all.read();
    for _ in 0..ticks {
        let (a0, m0, t0) = (all.read(), main.read(), Instant::now());
        sim.ticks(1);
        let wall_ms = t0.elapsed().as_secs_f64() * 1e3;
        let (a, m) = (all.read() - a0, main.read() - m0);
        let hw = a.hw.unwrap_or_default();
        let mhw = m.hw.unwrap_or_default();
        ins.push(hw.instructions as f64);
        main_ins.push(mhw.instructions as f64);
        cpu.push(a.cpu_ns as f64 / 1e6);
        wall.push(wall_ms);
        total = if total.hw.is_none() { a } else { total + a };
        main_total += mhw.instructions;
    }
    let p = |v: &[f64]| Percentiles::of(v).unwrap_or_default();
    let (pi, pm, pc, pw) = (p(&ins), p(&main_ins), p(&cpu), p(&wall));
    let hw = total.hw.unwrap_or_default();
    Result {
        ticks,
        instructions_total: hw.instructions,
        instructions_p50: pi.p50,
        instructions_p99: pi.p99,
        main_instructions_p50: pm.p50,
        main_instructions_total: main_total,
        cpu_ms_p50: pc.p50,
        cpu_ms_p99: pc.p99,
        wall_ms_p50: pw.p50,
        wall_ms_p99: pw.p99,
        cycles_total: hw.cycles,
        cache_misses_total: hw.cache_misses,
    }
}

/// Which build this binary is (`playtest`, `debug`, `release`).
fn build_profile() -> String {
    std::env::current_exe()
        .ok()
        .and_then(|e| e.parent()?.file_name().map(|n| n.to_string_lossy().into_owned()))
        .unwrap_or_default()
}

fn cpu_model() -> String {
    std::fs::read_to_string("/proc/cpuinfo")
        .ok()
        .and_then(|t| {
            t.lines()
                .find(|l| l.starts_with("model name"))
                .and_then(|l| l.split(':').nth(1))
                .map(|s| s.trim().to_string())
        })
        .unwrap_or_else(|| std::env::consts::ARCH.to_string())
}
