//! Reference comparison against real Counter-Strike: Source.
//! See docs/OBSERVABILITY.md ("Comparing with the real game").
//!
//! Captures the same camera views in CS:S (driven over RCON, so it works
//! with the desktop locked) and in mashup, then reports per-view brightness,
//! saturation and sharpness and writes side-by-side images. Output goes to a
//! per-user folder, never the repository (it contains game imagery).

#[path = "shared/rcon.rs"]
mod rcon;
use rcon::Rcon;
use std::{
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
usage: refcmp [all|capture-ref|capture-ours|report|fit|skyconv] [--views <file>] [--only <name>] [--keep-running]
  all            capture both, then report (default)
  capture-ref    capture views in CS:S (Steam must be logged in on this machine)
  capture-ours   capture views in mashup
  report         compare existing captures
  fit            capture mashup's albedo and lighting debug views and fit how
                 CS:S combines texture and light, per pixel, against the reference
  skyconv        measure the engine's cubemap convention with an encoded debug
                 sky, then fit each face's texture and orientation to the reference
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
            "all" | "capture-ref" | "capture-ours" | "report" | "fit" | "skyconv" => args.command = a,
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
    if args.command == "skyconv" {
        file.view.retain(|v| v.name.starts_with("sky"));
        skyconv(&file, &ref_dir, &out)?;
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

    let mut rcon = match Rcon::connect(RCON_ADDR, RCON_PASSWORD) {
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
            wait_for(Duration::from_secs(240), "CS:S RCON", || {
                Rcon::connect(RCON_ADDR, RCON_PASSWORD).ok()
            })?
        }
    };
    // Wait until the map is loaded and our player is in the game. A reused
    // game may be sitting at the menu (e.g. after an idle kick): load the map.
    let map_line = format!("map     : {}", file.map);
    let status = rcon.exec("status").unwrap_or_default();
    if !status.contains(&map_line) {
        println!("loading {} in the running game...", file.map);
        let _ = rcon.exec(&format!("map {}", file.map));
        sleep(Duration::from_secs(5));
        rcon = wait_for(Duration::from_secs(120), "CS:S RCON after map change", || {
            Rcon::connect(RCON_ADDR, RCON_PASSWORD).ok()
        })?;
    }
    wait_for(Duration::from_secs(240), "the map to load", || {
        let status = rcon.exec("status").ok()?;
        (status.contains(&map_line) && status.contains(" active ")).then_some(())
    })?;
    sleep(Duration::from_secs(3));

    let cam = &file.camera;
    for cmd in [
        "sv_cheats 1",
        // Our spectator never moves; don't kick it for idling.
        "mp_autokick 0",
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
    capture_ours_env(file, out, extra, &[])
}

fn capture_ours_env(file: &ViewsFile, out: &Path, extra: &[&str], env: &[(&str, String)]) -> Result<(), String> {
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
        .with_file_name(if cfg!(windows) { "mashup.exe" } else { "mashup" });
    warn_if_stale(&exe);
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
        .envs(env.iter().map(|(k, v)| (*k, v.as_str())))
        .stdout(std::process::Stdio::null())
        // The game's warnings and errors are worth seeing.
        .stderr(std::process::Stdio::inherit())
        .env("RUST_LOG", std::env::var("RUST_LOG").unwrap_or_else(|_| "warn".into()))
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

/// Correlation of the two images' fine detail (Laplacian of luma): near 1
/// when texture relief lines up, near 0 when unrelated, negative when
/// inverted (e.g. normal maps lit from the wrong side).
fn detail_correlation(a: &RgbImage, b: &RgbImage) -> f32 {
    // Half resolution first: tolerant of sub-pixel differences between
    // renderers, still sensitive to texture relief.
    let half =
        |img: &RgbImage| imageops::resize(img, img.width() / 2, img.height() / 2, imageops::FilterType::Triangle);
    let (a, b) = (&half(a), &half(b));
    let lap = |img: &RgbImage| {
        let (w, h) = img.dimensions();
        let l = |x: u32, y: u32| {
            let p = img.get_pixel(x, y);
            0.2126 * p[0] as f32 + 0.7152 * p[1] as f32 + 0.0722 * p[2] as f32
        };
        let mut out = Vec::with_capacity((w * h) as usize);
        for y in 1..h - 1 {
            for x in 1..w - 1 {
                out.push(4.0 * l(x, y) - l(x - 1, y) - l(x + 1, y) - l(x, y - 1) - l(x, y + 1));
            }
        }
        out
    };
    let (la, lb) = (lap(a), lap(b));
    let n = la.len() as f32;
    let (ma, mb) = (la.iter().sum::<f32>() / n, lb.iter().sum::<f32>() / n);
    let (mut sab, mut saa, mut sbb) = (0.0f64, 0.0f64, 0.0f64);
    for (x, y) in la.iter().zip(&lb) {
        let (dx, dy) = ((x - ma) as f64, (y - mb) as f64);
        sab += dx * dy;
        saa += dx * dx;
        sbb += dy * dy;
    }
    (sab / (saa.sqrt() * sbb.sqrt()).max(1e-9)) as f32
}

fn report(file: &ViewsFile, ref_dir: &Path, ours_dir: &Path, out: &Path) -> Result<(), String> {
    println!(
        "\n{:<14} {:>12} {:>12} {:>14} {:>9} {:>7}   (ref -> ours)",
        "view", "luma", "saturation", "sharpness", "abs diff", "detail"
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
        let detail = detail_correlation(&r, &o);
        println!(
            "{:<14} {:>5.3}->{:<5.3} {:>5.3}->{:<5.3} {:>6.1}->{:<6.1} {:>9.3} {:>7.3}",
            v.name, rs.luma, os.luma, rs.saturation, os.saturation, rs.sharpness, os.sharpness, abs_diff, detail
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
        rows.push(serde_json::json!({"view": v.name, "ref": rs, "ours": os, "abs_diff": abs_diff, "detail_correlation": detail}));
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

// ----------------------------------------------------------------- skyconv

/// Per engine cube face: which direction component (0 x, 1 y, 2 z) and sign
/// gives the face image's u and v, as `(component / |major axis| + 1) / 2`.
type Convention = [((usize, f32), (usize, f32)); 6];

fn view_direction(v: &SourceView, x: u32, y: u32, w: u32, h: u32) -> bevy::math::Vec3 {
    use bevy::math::{EulerRot, Quat, Vec3};
    let tan = (74f32.to_radians() / 2.0).tan();
    let q = Quat::from_euler(
        EulerRot::YXZ,
        (v.angles[1] - 90.0).to_radians(),
        (-v.angles[0]).to_radians(),
        0.0,
    );
    let nx = (x as f32 + 0.5) / w as f32 * 2.0 - 1.0;
    let ny = 1.0 - (y as f32 + 0.5) / h as f32 * 2.0;
    (q * Vec3::new(nx * tan * w as f32 / h as f32, ny * tan, -1.0)).normalize()
}

fn major_face(d: bevy::math::Vec3) -> (usize, f32) {
    let a = d.abs();
    if a.x >= a.y && a.x >= a.z {
        (if d.x > 0.0 { 0 } else { 1 }, a.x)
    } else if a.y >= a.z {
        (if d.y > 0.0 { 2 } else { 3 }, a.y)
    } else {
        (if d.z > 0.0 { 4 } else { 5 }, a.z)
    }
}

fn skyconv(file: &ViewsFile, ref_dir: &Path, out: &Path) -> Result<(), String> {
    let mask_dir = out.join("ours_albedo");
    let debug_dir = out.join("skydebug");
    for d in [&mask_dir, &debug_dir] {
        std::fs::create_dir_all(d).map_err(|e| e.to_string())?;
    }
    capture_ours(file, &mask_dir, &["--debug-view", "albedo"])?;
    capture_ours_env(file, &debug_dir, &[], &[("MASHUP_SKY_DEBUG", "1".into())])?;
    let load = |p: PathBuf| {
        image::open(&p)
            .map(|i| i.to_rgb8())
            .map_err(|e| format!("{}: {e}", p.display()))
    };
    let is_sky = |m: &Rgb<u8>| m[0] > 250 && m[1] < 5 && m[2] > 250;

    // 1. Measure the convention: which cube layer each view direction
    //    samples, and how the layer's u and v follow the direction's
    //    components (over the major axis).
    let mut layer_for = [[0usize; 6]; 6]; // [direction major face][layer]
    let mut sums = vec![[[0.0f64; 6]; 2]; 6]; // [layer][u|v][axis*2 + sign]
    let mut counts = [0usize; 6];
    for v in &file.view {
        let mask = load(mask_dir.join(format!("{}.png", v.name)))?;
        let dbg = load(debug_dir.join(format!("{}.png", v.name)))?;
        let (w, h) = dbg.dimensions();
        for y in (0..h).step_by(3) {
            for x in (0..w).step_by(3) {
                if !is_sky(mask.get_pixel(x, y)) {
                    continue;
                }
                let p = dbg.get_pixel(x, y);
                let layer = ((p[2] as f32 - 20.0) / 40.0).round() as usize;
                if layer > 5 || (p[2] as i32 - (layer as i32 * 40 + 20)).abs() > 6 {
                    continue;
                }
                let dir = view_direction(v, x, y, w, h);
                let (major, m) = major_face(dir);
                layer_for[major][layer] += 1;
                let d = dir / m;
                let uv = [p[0] as f64 / 255.0 * 2.0 - 1.0, p[1] as f64 / 255.0 * 2.0 - 1.0];
                for (c, value) in uv.iter().enumerate() {
                    for axis in 0..3 {
                        for (si, sign) in [1.0f64, -1.0].iter().enumerate() {
                            sums[layer][c][axis * 2 + si] -= (value - sign * d[axis] as f64).abs();
                        }
                    }
                }
                counts[layer] += 1;
            }
        }
    }
    let names = ["+X", "-X", "+Y", "-Y", "+Z", "-Z"];
    let axis_name = ["x", "y", "z"];
    // direction major face -> layer it samples
    let mut layer_of_dir = [usize::MAX; 6];
    for d in 0..6 {
        if let Some((l, &c)) = layer_for[d].iter().enumerate().max_by_key(|(_, c)| **c)
            && c > 0
        {
            layer_of_dir[d] = l;
        }
    }
    let mut conv: Convention = [((0, 1.0), (0, 1.0)); 6];
    println!("measured cubemap convention:");
    for l in 0..6 {
        let dir = (0..6).find(|d| layer_of_dir[*d] == l);
        if counts[l] == 0 || dir.is_none() {
            println!("  layer {} ({}): not visible", l, names[l]);
            continue;
        }
        let pick = |c: usize| {
            let (i, _) = sums[l][c].iter().enumerate().max_by(|a, b| a.1.total_cmp(b.1)).unwrap();
            (i / 2, if i % 2 == 0 { 1.0f32 } else { -1.0 })
        };
        conv[l] = (pick(0), pick(1));
        let err = |c: usize| -sums[l][c].iter().cloned().fold(f64::MIN, f64::max) / counts[l] as f64;
        println!(
            "  layer {} ({}) is seen looking {}: u = {}{}  v = {}{}   (residual {:.3}, {:.3}; {} px)",
            l,
            names[l],
            names[dir.unwrap()],
            if conv[l].0.1 > 0.0 { "+" } else { "-" },
            axis_name[conv[l].0.0],
            if conv[l].1.1 > 0.0 { "+" } else { "-" },
            axis_name[conv[l].1.0],
            err(0),
            err(1),
            counts[l]
        );
    }

    // 2. With the convention known, fit each face's texture and orientation
    //    to the reference sky on the CPU.
    let map = mashup::games::load_map(&format!("cs_source:{}", file.map))?;
    let sky = map.sky.as_ref().ok_or("map has no sky")?;
    let suffix_textures: Vec<(&str, &mashup::map::MapTexture)> = {
        use mashup::games::cs_source::sky::FACES;
        FACES
            .iter()
            .map(|(sfx, face, _)| (*sfx, &map.textures[sky.faces[*face].0]))
            .collect()
    };
    let sample = |t: &mashup::map::MapTexture, orient: u8, u: f32, v: f32| -> [f32; 3] {
        let size = t.width;
        let (x, y) = (
            ((u * size as f32) as u32).min(size - 1),
            ((v * size as f32) as u32).min(size - 1),
        );
        // Same transform as the engine's sky_image.
        let (mut a, mut b) = (x, y);
        for _ in 0..orient % 4 {
            (a, b) = (b, size - 1 - a);
        }
        if orient >= 4 {
            a = size - 1 - a;
        }
        let i = ((b * t.width + a) * 4) as usize;
        [t.rgba8[i] as f32, t.rgba8[i + 1] as f32, t.rgba8[i + 2] as f32]
    };
    let mut err = vec![vec![0.0f64; 48]; 6];
    let mut n = [0usize; 6];
    for v in &file.view {
        let r = load(ref_dir.join(format!("{}.jpg", v.name)))?;
        let mask = load(mask_dir.join(format!("{}.png", v.name)))?;
        let (w, h) = r.dimensions();
        for y in (0..h).step_by(2) {
            for x in (0..w).step_by(2) {
                if !is_sky(mask.get_pixel(x, y)) {
                    continue;
                }
                let d = view_direction(v, x, y, w, h);
                let (major, m) = major_face(d);
                let face = layer_of_dir[major];
                if face > 5 {
                    continue;
                }
                let d = d / m;
                let ((ua, us), (va, vs)) = conv[face];
                let u = (us * d[ua] + 1.0) / 2.0;
                let vv = (vs * d[va] + 1.0) / 2.0;
                let rp = r.get_pixel(x, y);
                for (ci, (_, t)) in suffix_textures.iter().enumerate() {
                    for orient in 0..8u8 {
                        let c = sample(t, orient, u, vv);
                        err[face][ci * 8 + orient as usize] +=
                            (0..3).map(|k| (c[k] - rp[k] as f32).abs() as f64).sum::<f64>() / (3.0 * 255.0);
                    }
                }
                n[face] += 1;
            }
        }
    }
    println!("best texture/orientation per cube layer with the measured convention:");
    for f in 0..6 {
        if n[f] == 0 {
            println!("  {}: not visible", names[f]);
            continue;
        }
        let mut ranked: Vec<(usize, f64)> = err[f].iter().map(|e| e / n[f] as f64).enumerate().collect();
        ranked.sort_by(|a, b| a.1.total_cmp(&b.1));
        let show = |(i, e): (usize, f64)| format!("{}/{} {:.4}", suffix_textures[i / 8].0, i % 8, e);
        println!(
            "  {} ({:>6} px): {}   next {}, {}",
            names[f],
            n[f],
            show(ranked[0]),
            show(ranked[1]),
            show(ranked[2])
        );
    }
    Ok(())
}

/// Warn when the game binary is older than the sources: refcmp runs the
/// built game, and `cargo run --bin refcmp` doesn't rebuild it.
fn warn_if_stale(exe: &std::path::Path) {
    fn newest(dir: &std::path::Path) -> Option<std::time::SystemTime> {
        let mut best = None;
        for entry in std::fs::read_dir(dir).ok()?.flatten() {
            let path = entry.path();
            let t = if path.is_dir() {
                newest(&path)
            } else {
                entry.metadata().ok()?.modified().ok()
            };
            best = best.max(t);
        }
        best
    }
    let src = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let built = std::fs::metadata(exe).and_then(|m| m.modified()).ok();
    if let (Some(built), Some(changed)) = (built, newest(&src))
        && changed > built
    {
        eprintln!(
            "warning: {} is older than the sources; captures show the old build \
             (run `cargo build --features dev` first)",
            exe.display()
        );
    }
}
