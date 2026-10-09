//! Development tool: load every map in mashup's content cache headless and
//! report what goes wrong (docs/plans/active/community-maps.md).
//!
//! Per map: load time, an error or panic, load warnings by kind, entity
//! classes nothing handles, what the map logic logs in its first seconds
//! (refused server commands, unhandled inputs), counts (triangles, brush
//! entities, props, spawns), and with `--shots` a screenshot from the
//! first spawn plus the `mashup_perf_log` frame time there. Writes
//! `report.md`, `report.csv` and `warnings.txt` to `--out`.

use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    panic::{AssertUnwindSafe, catch_unwind},
    path::{Path, PathBuf},
    process::{Command, ExitCode},
    time::Instant,
};

use mashup::{
    games::{self, cs_source},
    logic::{LogicWorld, NoCollision, classes::Class},
    map::MapData,
    mount::config::content_dir,
};

const USAGE: &str = "\
usage: mapsweep [options]
  --filter <text>    only maps whose name contains <text>
  --out <dir>        report folder (default: target/mapsweep)
  --secs <n>         seconds of map logic to run per map (default 10)
  --shots <exe>      also run <exe> (a mashup build, e.g. target/playtest/mashup) per map:
                     a 1280x720 screenshot from the first spawn and mashup_perf_log's
                     frame times there (needs a display)
  --frames <n>       frames per --shots run (default 600)";

/// Entity classes the game or the map loader handles outside the logic
/// layer (drawn, volumes, objectives, lighting and look, sound), or that
/// do nothing in a map at run time (hints for tools and the compiler).
const HANDLED_ELSEWHERE: &[&str] = &[
    "worldspawn",
    "info_player_terrorist",
    "info_player_counterterrorist",
    // CS:S spawns players only at the team spawns.
    "info_player_start",
    // Ladders come from the brushes' ladder contents.
    "info_ladder",
    "info_target",
    "info_teleport_destination",
    "info_node",
    "info_null",
    "info_landmark",
    "info_lighting",
    "info_overlay",
    "info_overlay_accessor",
    "infodecal",
    "info_map_parameters",
    "info_bomb_target",
    "info_hostage_rescue",
    "hostage_entity",
    "func_bomb_target",
    "func_buyzone",
    "func_hostage_rescue",
    "func_escapezone",
    "func_vip_safetyzone",
    "func_no_defuse",
    "func_illusionary",
    "func_wall",
    "func_detail",
    "func_lod",
    "func_viscluster",
    "func_vehicleclip",
    "func_clip_vphysics",
    "func_precipitation",
    "func_smokevolume",
    "func_ladderendpoint",
    "light_environment",
    "env_cubemap",
    "env_fog_controller",
    "env_tonemap_controller",
    "env_soundscape",
    "env_soundscape_triggerable",
    "env_soundscape_proxy",
    "sky_camera",
    "shadow_control",
    "water_lod_control",
    "keyframe_rope",
    "move_rope",
    "prop_static",
    "prop_detail",
    "env_sun",
];

struct Args {
    filter: Option<String>,
    out: PathBuf,
    secs: f32,
    shots: Option<PathBuf>,
    frames: u32,
}

fn parse() -> Result<Args, String> {
    let mut a = Args {
        filter: None,
        out: PathBuf::from("target/mapsweep"),
        secs: 10.0,
        shots: None,
        frames: 600,
    };
    let mut it = std::env::args().skip(1);
    while let Some(flag) = it.next() {
        let mut value = || it.next().ok_or(format!("{flag} needs a value"));
        match flag.as_str() {
            "--filter" => a.filter = Some(value()?.to_lowercase()),
            "--out" => a.out = PathBuf::from(value()?),
            "--secs" => a.secs = value()?.parse().map_err(|_| "--secs: not a number")?,
            "--shots" => a.shots = Some(PathBuf::from(value()?)),
            "--frames" => a.frames = value()?.parse().map_err(|_| "--frames: not a number")?,
            "--help" | "-h" => return Err(String::new()),
            other => return Err(format!("unknown option {other}")),
        }
    }
    Ok(a)
}

/// One map's results.
#[derive(Default)]
struct Row {
    name: String,
    status: String,
    load_s: f32,
    bsp_version: i32,
    warnings: Vec<String>,
    /// Kind -> count.
    kinds: BTreeMap<String, usize>,
    /// Classname -> count, for classes nothing handles.
    unsupported: BTreeMap<String, usize>,
    /// Classes the logic handles only partly (count).
    partial: BTreeMap<String, usize>,
    /// Logic log lines, kind -> count.
    logic: BTreeMap<String, usize>,
    logic_lines: Vec<String>,
    triangles: usize,
    brush_entities: usize,
    movers: usize,
    props: usize,
    spawns_t: usize,
    spawns_ct: usize,
    spawns_other: usize,
    shot: Option<PathBuf>,
    frame_ms: Option<String>,
}

/// A warning's kind, for counting across maps.
fn warning_kind(w: &str) -> String {
    let lower = w.to_lowercase();
    if lower.ends_with("decals found no surface to project onto") {
        return "decals without a surface".into();
    }
    if lower.ends_with("overlays produced no geometry") {
        return "overlays without geometry".into();
    }
    if lower.starts_with("sound missing") {
        return "sound missing".into();
    }
    if lower.contains("unknown variant") {
        return "material: unknown shader".into();
    }
    if lower.starts_with("sky ") {
        return "sky texture missing".into();
    }
    if lower.contains("not found") {
        let path = lower.split(':').next().unwrap_or("");
        let ext = path.rsplit('.').next().unwrap_or("");
        return match ext {
            "vmt" => "material missing".into(),
            "vtf" => "texture missing".into(),
            "mdl" | "vvd" | "vtx" | "phy" => "model missing".into(),
            "wav" | "mp3" => "sound missing".into(),
            _ => format!("missing .{ext}"),
        };
    }
    if lower.contains(".vtf:") {
        return "texture unreadable".into();
    }
    if lower.contains(".vmt:") {
        return "material unreadable".into();
    }
    if lower.contains(".mdl") {
        return "model unreadable".into();
    }
    if lower.starts_with("propdata") {
        return "gib list missing".into();
    }
    // First words, without numbers and paths.
    lower
        .split_whitespace()
        .filter(|t| !t.contains('/') && !t.chars().any(|c| c.is_ascii_digit()))
        .take(4)
        .collect::<Vec<_>>()
        .join(" ")
}

/// A logic log line's kind.
fn logic_kind(line: &str) -> String {
    let l = line.to_lowercase();
    if let Some(rest) = l.strip_prefix("point_servercommand: refused '") {
        let cmd = rest.split([' ', '\'']).next().unwrap_or("");
        return format!("servercommand refused: {cmd}");
    }
    if let Some(rest) = l.strip_prefix("point_clientcommand: refused '") {
        let cmd = rest.split([' ', '\'']).next().unwrap_or("");
        return format!("clientcommand refused: {cmd}");
    }
    if let Some((class, input)) = l.split_once(": unhandled input ") {
        return format!("unhandled input: {class}.{input}");
    }
    if l.starts_with("unhandled input: no entity") {
        return "input to a missing entity".into();
    }
    l.split_whitespace()
        .filter(|t| !t.chars().any(|c| c.is_ascii_digit()) && !t.contains('\''))
        .take(5)
        .collect::<Vec<_>>()
        .join(" ")
}

fn bsp_version(path: &Path) -> i32 {
    fs::read(path)
        .ok()
        .and_then(|b| b.get(4..8).map(|v| i32::from_le_bytes(v.try_into().unwrap())))
        .unwrap_or(0)
}

/// Sounds the map's ambient_generics name that didn't load.
fn missing_sounds(map: &MapData) -> Vec<String> {
    let (entries, raw) = cs_source::sound::ambient_messages(&map.entities);
    entries
        .into_iter()
        .chain(raw)
        .filter(|s| map.sounds.entry(s).is_none_or(|e| e.waves.is_empty()))
        .map(|s| format!("sound missing: {s}"))
        .collect()
}

fn sweep_map(name: &str, path: &Path, secs: f32) -> Row {
    let mut row = Row {
        name: name.to_string(),
        bsp_version: bsp_version(path),
        ..Row::default()
    };
    let t0 = Instant::now();
    let loaded = catch_unwind(|| games::load_map(&format!("{}:{name}", cs_source::GAME)));
    row.load_s = t0.elapsed().as_secs_f32();
    let map = match loaded {
        Ok(Ok(map)) => map,
        Ok(Err(e)) => {
            row.status = format!("load error: {e}");
            return row;
        }
        Err(p) => {
            row.status = format!("panic: {}", panic_text(&p));
            return row;
        }
    };
    row.status = "ok".into();
    row.warnings = map.warnings.clone();
    row.warnings.extend(missing_sounds(&map));
    // Decoded textures whose pixels don't match their size (drawing code
    // indexes by size).
    for t in &map.textures {
        if t.rgba8.len() != (t.width * t.height * 4) as usize {
            row.warnings.push(format!("texture size mismatch: {} {}x{} with {} bytes", t.name, t.width, t.height, t.rgba8.len()));
        }
    }
    for w in &row.warnings {
        *row.kinds.entry(warning_kind(w)).or_default() += 1;
    }
    row.triangles = map.meshes.iter().map(|m| m.indices.len() / 3).sum();
    row.props = map.props.len();
    row.brush_entities = map
        .entities
        .iter()
        .filter(|e| e.get("model").is_some_and(|m| m.starts_with('*')))
        .count();
    row.movers = map.entities.iter().filter(|e| e.mover).count();
    for (_, team) in &map.spawns {
        match team.map(|t| t.0) {
            Some(1) => row.spawns_t += 1,
            Some(2) => row.spawns_ct += 1,
            _ => row.spawns_other += 1,
        }
    }

    // The map logic: spawned, then run for a while with no players.
    let logic = catch_unwind(AssertUnwindSafe(|| {
        let mut w = LogicWorld::new(1.0 / 66.0);
        let ids = w.load_map(&map.entities);
        let mut none: Vec<String> = Vec::new();
        for id in ids {
            if let Some(e) = w.get(id) {
                if matches!(e.class, Class::None) {
                    none.push(e.classname.to_lowercase());
                }
            }
        }
        for _ in 0..(secs * 66.0) as usize {
            w.frame(&NoCollision);
        }
        (none, std::mem::take(&mut w.log))
    }));
    match logic {
        Ok((none, log)) => {
            for class in none {
                // Placed weapons: the weapon layer (`weapon::equip`).
                if HANDLED_ELSEWHERE.contains(&class.as_str()) || class.starts_with("weapon_") {
                    continue;
                }
                *row.unsupported.entry(class).or_default() += 1;
            }
            for line in log {
                *row.logic.entry(logic_kind(&line)).or_default() += 1;
                if row.logic_lines.len() < 200 {
                    row.logic_lines.push(line);
                }
            }
        }
        Err(p) => {
            row.status = format!("logic panic: {}", panic_text(&p));
        }
    }
    // Brush classes that load as static world brushes but should do more
    // (none left: physics brushes, wall toggles and conveyors have nodes).
    const PARTIAL: &[&str] = &[];
    for e in &map.entities {
        let c = e.classname().to_lowercase();
        if PARTIAL.contains(&c.as_str()) {
            *row.partial.entry(c).or_default() += 1;
        }
    }
    row
}

fn panic_text(p: &Box<dyn std::any::Any + Send>) -> String {
    p.downcast_ref::<String>()
        .cloned()
        .or_else(|| p.downcast_ref::<&str>().map(|s| s.to_string()))
        .unwrap_or_else(|| "(no message)".into())
}

/// Run a mashup build on the map: a screenshot from the first spawn and
/// the perf log's frame-time lines.
fn shoot(exe: &Path, name: &str, out: &Path, frames: u32) -> (Option<PathBuf>, Option<String>) {
    let shot = out.join("shots").join(format!("{name}.png"));
    let _ = fs::create_dir_all(shot.parent().unwrap());
    let result = Command::new(exe)
        .args([
            "--map",
            &format!("{}:{name}", cs_source::GAME),
            "--window",
            "1280x720",
            "--frames",
            &frames.to_string(),
            "--screenshot",
        ])
        .arg(&shot)
        .args(["+mashup_perf_log", "1", "+mat_vsync", "0"])
        .output();
    let Ok(output) = result else {
        return (None, None);
    };
    let text = String::from_utf8_lossy(&output.stdout).to_string() + &String::from_utf8_lossy(&output.stderr);
    let _ = fs::write(out.join("shots").join(format!("{name}.log")), &text);
    // The last perf line (frame times once things have settled).
    let perf = text
        .lines()
        .filter(|l| l.contains("mashup_perf:") && l.contains("frame"))
        .last()
        .map(|l| {
            // "frame 16.7 ms (p95 ..) | main world CPU .. | GPU .." as one cell.
            let at = l.find("frame").unwrap_or(0);
            l[at..].split(" | ").take(3).collect::<Vec<_>>().join("; ")
        });
    (shot.is_file().then_some(shot), perf)
}

fn main() -> ExitCode {
    let args = match parse() {
        Ok(a) => a,
        Err(e) => {
            if !e.is_empty() {
                eprintln!("{e}");
            }
            eprintln!("{USAGE}");
            return ExitCode::FAILURE;
        }
    };
    let Some(dir) = content_dir(cs_source::GAME).map(|d| d.join("maps")) else {
        eprintln!("no content folder");
        return ExitCode::FAILURE;
    };
    let mut maps: Vec<(String, PathBuf)> = fs::read_dir(&dir)
        .map(|r| {
            r.filter_map(|e| e.ok())
                .map(|e| e.path())
                .filter(|p| p.extension().is_some_and(|x| x.eq_ignore_ascii_case("bsp")))
                .filter_map(|p| Some((p.file_stem()?.to_str()?.to_string(), p)))
                .collect()
        })
        .unwrap_or_default();
    maps.retain(|(n, _)| args.filter.as_ref().is_none_or(|f| n.to_lowercase().contains(f)));
    maps.sort();
    // Panics are caught per map; keep their messages short.
    std::panic::set_hook(Box::new(|info| {
        eprintln!("  panic: {info}");
    }));
    let _ = fs::create_dir_all(&args.out);
    let mut rows = Vec::new();
    for (i, (name, path)) in maps.iter().enumerate() {
        eprintln!("[{}/{}] {name}", i + 1, maps.len());
        let mut row = sweep_map(name, path, args.secs);
        if let Some(exe) = &args.shots
            && row.status == "ok"
        {
            (row.shot, row.frame_ms) = shoot(exe, name, &args.out, args.frames);
        }
        eprintln!(
            "  {} in {:.1}s, {} warnings, {} unsupported classes",
            row.status,
            row.load_s,
            row.warnings.len(),
            row.unsupported.len()
        );
        rows.push(row);
    }
    if let Err(e) = write_reports(&args.out, &rows) {
        eprintln!("writing the report: {e}");
        return ExitCode::FAILURE;
    }
    eprintln!("report: {}", args.out.join("report.md").display());
    ExitCode::SUCCESS
}

fn write_reports(out: &Path, rows: &[Row]) -> std::io::Result<()> {
    // CSV: one line per map.
    let mut csv = String::from(
        "map,status,load_s,bsp_version,warnings,triangles,brush_entities,movers,props,spawns_t,spawns_ct,unsupported_classes,logic_lines,frame\n",
    );
    for r in rows {
        csv += &format!(
            "{},{},{:.2},{},{},{},{},{},{},{},{},{},{},{}\n",
            r.name,
            r.status.replace(',', ";"),
            r.load_s,
            r.bsp_version,
            r.warnings.len(),
            r.triangles,
            r.brush_entities,
            r.movers,
            r.props,
            r.spawns_t,
            r.spawns_ct,
            r.unsupported.values().sum::<usize>(),
            r.logic.values().sum::<usize>(),
            r.frame_ms.clone().unwrap_or_default().replace(',', ";"),
        );
    }
    fs::write(out.join("report.csv"), csv)?;

    // Every warning and logic line, per map.
    let mut raw = String::new();
    for r in rows {
        raw += &format!("== {} ({})\n", r.name, r.status);
        for w in &r.warnings {
            raw += &format!("  {w}\n");
        }
        for (c, n) in &r.unsupported {
            raw += &format!("  unsupported class {c} x{n}\n");
        }
        for l in &r.logic_lines {
            raw += &format!("  logic: {l}\n");
        }
    }
    fs::write(out.join("warnings.txt"), raw)?;

    // Markdown: problems ranked by the maps they affect, then a table.
    let mut md = String::from("# Map sweep\n\n");
    let ok = rows.iter().filter(|r| r.status == "ok").count();
    let clean = rows
        .iter()
        .filter(|r| r.status == "ok" && r.warnings.is_empty() && r.unsupported.is_empty() && r.logic.is_empty())
        .count();
    md += &format!(
        "{} maps; {ok} load ({clean} without warnings, unsupported classes or logic complaints); {} fail.\n\n",
        rows.len(),
        rows.len() - ok
    );
    let rank = |f: &dyn Fn(&Row) -> Vec<(String, usize)>| {
        let mut by: BTreeMap<String, (BTreeSet<String>, usize)> = BTreeMap::new();
        for r in rows {
            for (k, n) in f(r) {
                let e = by.entry(k).or_default();
                e.0.insert(r.name.clone());
                e.1 += n;
            }
        }
        let mut v: Vec<_> = by.into_iter().collect();
        v.sort_by(|a, b| b.1.0.len().cmp(&a.1.0.len()).then(b.1.1.cmp(&a.1.1)));
        v
    };
    let section = |md: &mut String, title: &str, v: Vec<(String, (BTreeSet<String>, usize))>| {
        *md += &format!("## {title}\n\n| Problem | Maps | Count | Examples |\n|---|---|---|---|\n");
        for (k, (maps, n)) in v {
            let ex: Vec<_> = maps.iter().take(4).cloned().collect();
            *md += &format!("| {k} | {} | {n} | {} |\n", maps.len(), ex.join(", "));
        }
        *md += "\n";
    };
    section(
        &mut md,
        "Failures",
        rank(&|r| {
            if r.status == "ok" {
                vec![]
            } else {
                vec![(r.status.chars().take(100).collect(), 1)]
            }
        }),
    );
    section(
        &mut md,
        "Load warnings by kind",
        rank(&|r| r.kinds.iter().map(|(k, n)| (k.clone(), *n)).collect()),
    );
    section(
        &mut md,
        "Entity classes nothing handles",
        rank(&|r| r.unsupported.iter().map(|(k, n)| (k.clone(), *n)).collect()),
    );
    section(
        &mut md,
        "Classes handled only partly (static brushes)",
        rank(&|r| r.partial.iter().map(|(k, n)| (k.clone(), *n)).collect()),
    );
    section(
        &mut md,
        "Map logic complaints (first seconds, no players)",
        rank(&|r| r.logic.iter().map(|(k, n)| (k.clone(), *n)).collect()),
    );
    md += "## Maps\n\n| Map | Status | Load s | BSP | Warnings | Tris (k) | Brush ents | Props | Spawns T/CT | Unsupported | Frame | Shot |\n|---|---|---|---|---|---|---|---|---|---|---|---|\n";
    for r in rows {
        md += &format!(
            "| {} | {} | {:.1} | {} | {} | {} | {} | {} | {}/{} | {} | {} | {} |\n",
            r.name,
            r.status.chars().take(60).collect::<String>(),
            r.load_s,
            r.bsp_version,
            r.warnings.len(),
            r.triangles / 1000,
            r.brush_entities,
            r.props,
            r.spawns_t,
            r.spawns_ct,
            r.unsupported.values().sum::<usize>(),
            r.frame_ms.clone().unwrap_or_default(),
            r.shot.as_ref().map(|p| p.display().to_string()).unwrap_or_default(),
        );
    }
    fs::write(out.join("report.md"), md)
}
