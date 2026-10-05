//! Reference comparison against real Counter-Strike: Source.
//! See docs/OBSERVABILITY.md ("Comparing with the real game").
//!
//! Captures the same camera views in CS:S (driven over RCON, so it works
//! with the desktop locked) and in mashup, then reports per-view brightness,
//! saturation and sharpness and writes side-by-side images. Output goes to a
//! per-user folder, never the repository (it contains game imagery).

use std::{
    io::{Read, Write},
    net::TcpStream,
    path::{Path, PathBuf},
    process::{Command, ExitCode},
    thread::sleep,
    time::{Duration, Instant},
};

use image::{GenericImageView, Rgb, RgbImage, imageops};
use mashup::{
    games::cs_source::bsp::to_engine,
    mount::config::{LocalConfig, default_dump_dir},
};
use serde::Deserialize;

const USAGE: &str = "\
usage: refcmp [all|capture-ref|capture-ours|report|fit] [--views <file>] [--only <name>] [--keep-running]
  all            capture both, then report (default)
  capture-ref    capture views in CS:S (Steam must be logged in on this machine)
  capture-ours   capture views in mashup
  report         compare existing captures
  fit            capture mashup's albedo and lighting debug views and fit how
                 CS:S combines texture and light, per pixel, against the reference
  --views <file> views file (default: tools/refcmp/de_dust2.toml)
  --only <name>  only views whose name contains <name>
  --keep-running leave CS:S running after capturing (faster next time)";

const RCON_PASSWORD: &str = "mashup-refcmp";
const RCON_ADDR: &str = "127.0.0.1:27015";

#[derive(Deserialize)]
struct ViewsFile {
    map: String,
    camera: String,
    view: Vec<SourceView>,
}

#[derive(Deserialize, Clone)]
struct SourceView {
    name: String,
    position: [f32; 3],
    /// [pitch, yaw], Source degrees.
    angles: [f32; 2],
}

struct Args {
    command: String,
    views: PathBuf,
    only: Option<String>,
    keep_running: bool,
}

fn main() -> ExitCode {
    let mut args = Args {
        command: "all".into(),
        views: Path::new(env!("CARGO_MANIFEST_DIR")).join("tools/refcmp/de_dust2.toml"),
        only: None,
        keep_running: false,
    };
    let mut it = std::env::args().skip(1);
    while let Some(a) = it.next() {
        match a.as_str() {
            "all" | "capture-ref" | "capture-ours" | "report" | "fit" => args.command = a,
            "--views" => args.views = it.next().map(PathBuf::from).unwrap_or_default(),
            "--only" => args.only = it.next(),
            "--keep-running" => args.keep_running = true,
            _ => {
                eprintln!("{USAGE}");
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
    let text = std::fs::read_to_string(&args.views).map_err(|e| format!("{}: {e}", args.views.display()))?;
    let mut file: ViewsFile = toml::from_str(&text).map_err(|e| format!("{}: {e}", args.views.display()))?;
    if let Some(only) = &args.only {
        file.view.retain(|v| v.name.contains(only.as_str()));
    }
    let out = default_dump_dir("refcmp")
        .ok_or("no per-user data folder")?
        .join(&file.map);
    let (ref_dir, ours_dir, report_dir) = (out.join("ref"), out.join("ours"), out.join("report"));
    for d in [&ref_dir, &ours_dir, &report_dir] {
        std::fs::create_dir_all(d).map_err(|e| format!("{}: {e}", d.display()))?;
    }
    let all = args.command == "all";
    if all || args.command == "capture-ref" {
        capture_ref(&file, &ref_dir, args.keep_running)?;
    }
    if all || args.command == "capture-ours" {
        capture_ours(&file, &ours_dir, &[])?;
    }
    if args.command == "fit" {
        let (albedo, lighting) = (out.join("ours_albedo"), out.join("ours_lighting"));
        for d in [&albedo, &lighting] {
            std::fs::create_dir_all(d).map_err(|e| e.to_string())?;
        }
        capture_ours(&file, &albedo, &["--debug-view", "albedo"])?;
        capture_ours(&file, &lighting, &["--debug-view", "lighting"])?;
        fit(&file, &ref_dir, &albedo, &lighting)?;
    }
    if all || args.command == "report" {
        report(&file, &ref_dir, &ours_dir, &report_dir)?;
    }
    Ok(())
}

// ---------------------------------------------------------------- CS:S side

/// Source RCON (Valve's documented TCP protocol).
struct Rcon {
    stream: TcpStream,
    next_id: i32,
}

impl Rcon {
    fn connect() -> std::io::Result<Self> {
        let stream = TcpStream::connect_timeout(&RCON_ADDR.parse().unwrap(), Duration::from_secs(2))?;
        stream.set_read_timeout(Some(Duration::from_secs(10)))?;
        let mut r = Self { stream, next_id: 1 };
        r.send(3, RCON_PASSWORD)?;
        loop {
            let (id, kind, _) = r.recv()?;
            if kind == 2 {
                if id == -1 {
                    return Err(std::io::Error::other("RCON authentication failed"));
                }
                return Ok(r);
            }
        }
    }

    fn send(&mut self, kind: i32, body: &str) -> std::io::Result<i32> {
        let id = self.next_id;
        self.next_id += 1;
        let mut packet = Vec::new();
        packet.extend((10 + body.len() as i32).to_le_bytes());
        packet.extend(id.to_le_bytes());
        packet.extend(kind.to_le_bytes());
        packet.extend(body.as_bytes());
        packet.extend([0, 0]);
        self.stream.write_all(&packet)?;
        Ok(id)
    }

    fn recv(&mut self) -> std::io::Result<(i32, i32, String)> {
        let mut len = [0u8; 4];
        self.stream.read_exact(&mut len)?;
        let mut data = vec![0u8; i32::from_le_bytes(len).max(10) as usize];
        self.stream.read_exact(&mut data)?;
        let id = i32::from_le_bytes(data[0..4].try_into().unwrap());
        let kind = i32::from_le_bytes(data[4..8].try_into().unwrap());
        Ok((id, kind, String::from_utf8_lossy(&data[8..data.len() - 2]).into_owned()))
    }

    /// Run a command and return its output (an empty marker packet after it
    /// tells us the response is complete).
    fn exec(&mut self, cmd: &str) -> std::io::Result<String> {
        self.send(2, cmd)?;
        let marker = self.send(0, "")?;
        let mut out = String::new();
        loop {
            let (id, _, body) = self.recv()?;
            if id == marker {
                return Ok(out);
            }
            out.push_str(&body);
        }
    }
}

fn steam_env(cmd: &mut Command) -> &mut Command {
    // The game runs on the desktop session even when this tool runs from a
    // terminal or agent shell.
    for (k, default) in [
        ("WAYLAND_DISPLAY", "wayland-1"),
        ("XDG_RUNTIME_DIR", "/run/user/1000"),
        ("DISPLAY", ":0"),
    ] {
        if std::env::var_os(k).is_none() {
            cmd.env(k, default);
        }
    }
    cmd
}

fn capture_ref(file: &ViewsFile, out: &Path, keep_running: bool) -> Result<(), String> {
    let install = LocalConfig::load()?
        .game_path("cs_source")
        .ok_or("no [games.cs_source] path in mashup.local.toml")?;
    let screenshots = install.join("cstrike/screenshots");

    let mut rcon = match Rcon::connect() {
        Ok(r) => {
            println!("CS:S already running; reusing it");
            r
        }
        Err(_) => {
            println!("launching CS:S on {} via Steam...", file.map);
            steam_env(&mut Command::new("steam"))
                .args([
                    "-applaunch",
                    "240",
                    "-novid",
                    "-windowed",
                    "-noborder",
                    "-w",
                    "1280",
                    "-h",
                    "720",
                ])
                .args([
                    "-condebug",
                    "-usercon",
                    "+ip",
                    "127.0.0.1",
                    "+rcon_password",
                    RCON_PASSWORD,
                ])
                .args(["+map", &file.map])
                .spawn()
                .map_err(|e| format!("starting steam: {e} (is the Steam client installed and logged in?)"))?;
            wait_for(Duration::from_secs(240), "CS:S RCON", || Rcon::connect().ok())?
        }
    };
    // Wait until the map is loaded and our player is in the game.
    let map_line = format!("map     : {}", file.map);
    wait_for(Duration::from_secs(240), "the map to load", || {
        let status = rcon.exec("status").ok()?;
        (status.contains(&map_line) && status.contains(" active ")).then_some(())
    })?;
    sleep(Duration::from_secs(3));

    let cam = &file.camera;
    for cmd in [
        "sv_cheats 1",
        "hidepanel info",
        "hidepanel team",
        "hidepanel specgui",
        "cl_drawhud 0",
        "r_drawviewmodel 0",
        "net_graph 0",
        &format!("ent_fire {cam} addoutput \"target \""),
        // Hold forever; don't start at or follow the player.
        &format!("ent_fire {cam} addoutput \"spawnflags 8\""),
    ] {
        rcon.exec(cmd).map_err(|e| format!("rcon {cmd}: {e}"))?;
    }

    for v in &file.view {
        let [x, y, z] = v.position;
        let [pitch, yaw] = v.angles;
        rcon.exec(&format!("ent_fire {cam} addoutput \"origin {x} {y} {z}\""))
            .map_err(|e| e.to_string())?;
        rcon.exec(&format!("ent_fire {cam} addoutput \"angles {pitch} {yaw} 0\""))
            .map_err(|e| e.to_string())?;
        rcon.exec(&format!("ent_fire {cam} enable"))
            .map_err(|e| e.to_string())?;
        sleep(Duration::from_millis(1500));
        let shot = screenshots.join(format!("refcmp_{}.jpg", v.name));
        let _ = std::fs::remove_file(&shot);
        rcon.exec(&format!("jpeg refcmp_{} 95", v.name))
            .map_err(|e| e.to_string())?;
        wait_for(Duration::from_secs(20), "the screenshot", || {
            shot.is_file().then_some(())
        })?;
        sleep(Duration::from_millis(300));
        let dest = out.join(format!("{}.jpg", v.name));
        std::fs::rename(&shot, &dest).map_err(|e| format!("{}: {e}", dest.display()))?;
        println!("  ref  {}", v.name);
    }
    if !keep_running {
        let _ = rcon.exec("quit");
    }
    Ok(())
}

fn wait_for<T>(limit: Duration, what: &str, mut f: impl FnMut() -> Option<T>) -> Result<T, String> {
    let start = Instant::now();
    loop {
        if let Some(t) = f() {
            return Ok(t);
        }
        if start.elapsed() > limit {
            return Err(format!("timed out waiting for {what}"));
        }
        sleep(Duration::from_millis(1000));
    }
}

// ------------------------------------------------------------- mashup side

fn capture_ours(file: &ViewsFile, out: &Path, extra: &[&str]) -> Result<(), String> {
    // Source eye position and [pitch, yaw] to engine space and our angles:
    // our yaw 0 looks down -Z (Source +Y), so yaw = source yaw - 90; our
    // positive pitch looks up.
    let views: Vec<serde_json::Value> = file
        .view
        .iter()
        .map(|v| {
            let [x, y, z] = v.position;
            let p = to_engine(vbsp::Vector { x, y, z });
            serde_json::json!({
                "name": v.name,
                "position": [p.x, p.y, p.z],
                "yaw": v.angles[1] - 90.0,
                "pitch": -v.angles[0],
            })
        })
        .collect();
    let views_path = out.join("views.json");
    std::fs::write(&views_path, serde_json::to_string_pretty(&views).unwrap()).map_err(|e| e.to_string())?;

    let exe = std::env::current_exe()
        .map_err(|e| e.to_string())?
        .with_file_name("mashup");
    println!("capturing {} views in mashup...", views.len());
    let status = steam_env(&mut Command::new(&exe))
        .args([
            "--map",
            &format!("cs_source:{}", file.map),
            "--movement",
            "mashup:noclip",
        ])
        .arg("--views")
        .arg(&views_path)
        .arg("--capture-dir")
        .arg(out)
        .args(extra)
        .status()
        .map_err(|e| format!("{}: {e} (build it first: cargo build --features dev)", exe.display()))?;
    if !status.success() {
        return Err(format!("mashup exited with {status}"));
    }
    Ok(())
}

// ------------------------------------------------------------------ report

#[derive(Default, serde::Serialize)]
struct Stats {
    /// Mean sRGB value per channel, 0..1.
    rgb: [f32; 3],
    luma: f32,
    /// Mean HSV saturation.
    saturation: f32,
    /// Mean absolute Laplacian of luma (0..255 scale): higher = crisper.
    sharpness: f32,
}

fn stats(img: &RgbImage) -> Stats {
    let (w, h) = img.dimensions();
    let n = (w * h) as f32;
    let mut s = Stats::default();
    let luma: Vec<f32> = img
        .pixels()
        .map(|Rgb([r, g, b])| {
            let (r, g, b) = (*r as f32 / 255.0, *g as f32 / 255.0, *b as f32 / 255.0);
            s.rgb[0] += r / n;
            s.rgb[1] += g / n;
            s.rgb[2] += b / n;
            let (max, min) = (r.max(g).max(b), r.min(g).min(b));
            s.saturation += if max > 0.0 { (max - min) / max } else { 0.0 } / n;
            0.2126 * r + 0.7152 * g + 0.0722 * b
        })
        .collect();
    s.luma = luma.iter().sum::<f32>() / n;
    let at = |x: u32, y: u32| luma[(y * w + x) as usize] * 255.0;
    let mut lap = 0.0;
    for y in 1..h - 1 {
        for x in 1..w - 1 {
            lap += (4.0 * at(x, y) - at(x - 1, y) - at(x + 1, y) - at(x, y - 1) - at(x, y + 1)).abs();
        }
    }
    s.sharpness = lap / ((w - 2) * (h - 2)) as f32;
    s
}

fn report(file: &ViewsFile, ref_dir: &Path, ours_dir: &Path, out: &Path) -> Result<(), String> {
    println!(
        "\n{:<14} {:>12} {:>12} {:>14} {:>9}   (ref -> ours)",
        "view", "luma", "saturation", "sharpness", "abs diff"
    );
    let mut rows = Vec::new();
    for v in &file.view {
        let (rp, op) = (
            ref_dir.join(format!("{}.jpg", v.name)),
            ours_dir.join(format!("{}.png", v.name)),
        );
        let (Ok(r), Ok(o)) = (image::open(&rp), image::open(&op)) else {
            println!("{:<14} missing capture(s)", v.name);
            continue;
        };
        let o = if o.dimensions() != r.dimensions() {
            o.resize_exact(r.width(), r.height(), imageops::FilterType::Triangle)
        } else {
            o
        };
        let (r, o) = (r.to_rgb8(), o.to_rgb8());
        let (rs, os) = (stats(&r), stats(&o));
        let mut diff = RgbImage::new(r.width(), r.height());
        let mut total = 0.0;
        for (d, (a, b)) in diff.pixels_mut().zip(r.pixels().zip(o.pixels())) {
            let px: [u8; 3] = std::array::from_fn(|c| (a[c] as i32 - b[c] as i32).unsigned_abs() as u8);
            total += px.iter().map(|v| *v as f32).sum::<f32>() / (3.0 * 255.0);
            *d = Rgb(px.map(|v| v.saturating_mul(4)));
        }
        let abs_diff = total / (r.width() * r.height()) as f32;
        println!(
            "{:<14} {:>5.3}->{:<5.3} {:>5.3}->{:<5.3} {:>6.1}->{:<6.1} {:>9.3}",
            v.name, rs.luma, os.luma, rs.saturation, os.saturation, rs.sharpness, os.sharpness, abs_diff
        );
        // Side by side: reference | ours | difference x4, at half size.
        let (hw, hh) = (r.width() / 2, r.height() / 2);
        let mut sheet = RgbImage::new(hw * 3, hh);
        for (i, img) in [&r, &o, &diff].into_iter().enumerate() {
            let small = imageops::resize(img, hw, hh, imageops::FilterType::Triangle);
            imageops::replace(&mut sheet, &small, (i as u32 * hw) as i64, 0);
        }
        let path = out.join(format!("{}.png", v.name));
        sheet.save(&path).map_err(|e| format!("{}: {e}", path.display()))?;
        rows.push(serde_json::json!({"view": v.name, "ref": rs, "ours": os, "abs_diff": abs_diff}));
    }
    let metrics = out.join("metrics.json");
    std::fs::write(&metrics, serde_json::to_string_pretty(&rows).unwrap()).map_err(|e| e.to_string())?;
    println!(
        "\nside-by-side images (reference | ours | difference x4) and metrics.json in {}",
        out.display()
    );
    Ok(())
}

// --------------------------------------------------------------------- fit

fn srgb_to_linear(c: f32) -> f32 {
    if c <= 0.04045 {
        c / 12.92
    } else {
        ((c + 0.055) / 1.055).powf(2.4)
    }
}

fn linear_to_srgb(c: f32) -> f32 {
    let c = c.clamp(0.0, 1.0);
    if c <= 0.0031308 {
        c * 12.92
    } else {
        1.055 * c.powf(1.0 / 2.4) - 0.055
    }
}

/// Per-pixel samples: (reference sRGB, albedo sRGB, linear light) per channel.
type Sample = ([f32; 3], [f32; 3], [f32; 3]);

fn fit(file: &ViewsFile, ref_dir: &Path, albedo_dir: &Path, light_dir: &Path) -> Result<(), String> {
    const LIGHT_SCALE: f32 = 0.25; // matches --debug-view lighting
    let mut samples: Vec<Sample> = Vec::new();
    for v in &file.view {
        let load = |p: PathBuf| {
            image::open(&p)
                .map(|i| i.to_rgb8())
                .map_err(|e| format!("{}: {e}", p.display()))
        };
        let r = load(ref_dir.join(format!("{}.jpg", v.name)))?;
        let a = load(albedo_dir.join(format!("{}.png", v.name)))?;
        let l = load(light_dir.join(format!("{}.png", v.name)))?;
        for ((rp, ap), lp) in r.pixels().zip(a.pixels()).zip(l.pixels()) {
            let magenta = |p: &Rgb<u8>| p[0] > 250 && p[1] < 5 && p[2] > 250;
            if magenta(ap) || magenta(lp) || rp.0.iter().any(|c| !(6..=249).contains(c)) {
                continue;
            }
            let f = |p: &Rgb<u8>| p.0.map(|c| c as f32 / 255.0);
            let (rg, ag) = (f(rp), f(ap));
            let light = f(lp).map(|c| srgb_to_linear(c) / LIGHT_SCALE);
            if ag.iter().any(|c| *c < 0.03) || light.iter().any(|c| *c < 0.02 || *c > 3.9) {
                continue;
            }
            samples.push((rg, ag, light));
        }
    }
    if samples.len() < 1000 {
        return Err(format!("only {} usable pixels", samples.len()));
    }
    let err = |model: &dyn Fn(f32, f32) -> f32| -> f32 {
        let mut e = 0.0;
        for (r, a, l) in &samples {
            for c in 0..3 {
                e += (model(a[c], l[c]).clamp(0.0, 1.0) - r[c]).abs();
            }
        }
        e / (samples.len() * 3) as f32
    };
    let median = |mut v: Vec<f32>| {
        v.sort_by(|a, b| a.total_cmp(b));
        v[v.len() / 2]
    };

    // Linear (our current model, without tonemapping), best exposure k.
    let k_lin = median(
        samples
            .iter()
            .flat_map(|(r, a, l)| (0..3).map(move |c| srgb_to_linear(r[c]) / (srgb_to_linear(a[c]) * l[c])))
            .collect(),
    );
    let lin = |a: f32, l: f32| linear_to_srgb(k_lin * srgb_to_linear(a) * l);
    // Source LDR as remembered: gamma-space, 2x overbright, light gamma 1/2.2.
    let source_ldr = |a: f32, l: f32| a * 2.0 * (l * 0.5).powf(1.0 / 2.2);
    // General gamma-space: log(r/a) = log k + p log l, least squares.
    let (mut sx, mut sy, mut sxx, mut sxy, mut n) = (0.0f64, 0.0f64, 0.0f64, 0.0f64, 0.0f64);
    for (r, a, l) in &samples {
        for c in 0..3 {
            let (x, y) = ((l[c] as f64).ln(), ((r[c] / a[c]) as f64).ln());
            sx += x;
            sy += y;
            sxx += x * x;
            sxy += x * y;
            n += 1.0;
        }
    }
    let p = ((n * sxy - sx * sy) / (n * sxx - sx * sx)) as f32;
    let k = (((sy - p as f64 * sx) / n) as f32).exp();
    let general = |a: f32, l: f32| a * k * l.powf(p);

    println!(
        "fit over {} pixels from {} views (mean abs error, sRGB 0..1):",
        samples.len(),
        file.view.len()
    );
    println!("  linear, k={k_lin:.3}                        {:.4}", err(&lin));
    println!("  Source LDR: a*2*(l/2)^(1/2.2)            {:.4}", err(&source_ldr));
    println!("  gamma-space fit: a*{k:.3}*l^{p:.3}           {:.4}", err(&general));
    Ok(())
}
