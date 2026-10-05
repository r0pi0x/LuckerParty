//! Development tool: inspect and extract a game install's files.
//! See docs/OBSERVABILITY.md ("Exploring game files").

use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    process::ExitCode,
};

use mashup::{
    games::{combat_arms, cs_source},
    mount::config::{LocalConfig, default_dump_dir},
};

const USAGE: &str = "\
usage: dump <game> [options]
  games: cs_source, combat_arms
  (no option)          summary: file counts and sizes by extension
  --list               list every file with its size
  --filter <text>      only paths containing <text> (case-insensitive)
  --extract            write the (filtered) files to --out
  --out <dir>          extraction folder (default: per-user data dir; never inside the repo)
  --install <dir>      install folder (default: from mashup.local.toml)";

struct Args {
    game: String,
    list: bool,
    extract: bool,
    filter: Option<String>,
    out: Option<PathBuf>,
    install: Option<PathBuf>,
}

fn parse() -> Result<Args, String> {
    let mut it = std::env::args().skip(1);
    let game = it.next().ok_or("missing <game>")?;
    if game == "--help" || game == "-h" {
        return Err(String::new());
    }
    let mut a = Args {
        game,
        list: false,
        extract: false,
        filter: None,
        out: None,
        install: None,
    };
    while let Some(flag) = it.next() {
        let mut value = || it.next().ok_or(format!("{flag}: missing value"));
        match flag.as_str() {
            "--list" => a.list = true,
            "--extract" => a.extract = true,
            "--filter" => a.filter = Some(value()?.to_lowercase()),
            "--out" => a.out = Some(value()?.into()),
            "--install" => a.install = Some(value()?.into()),
            _ => return Err(format!("unknown option {flag}")),
        }
    }
    Ok(a)
}

fn main() -> ExitCode {
    let args = match parse() {
        Ok(a) => a,
        Err(e) => {
            if !e.is_empty() {
                eprintln!("{e}\n");
            }
            eprintln!("{USAGE}");
            return ExitCode::from(2);
        }
    };
    match run(&args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
    }
}

fn install_path(args: &Args) -> Result<PathBuf, String> {
    if let Some(p) = &args.install {
        return Ok(p.clone());
    }
    LocalConfig::load()?.game_path(&args.game).ok_or_else(|| {
        format!(
            "no install path for `{}`. Set [games.{}] path in mashup.local.toml \
             (copy mashup.local.example.toml) or pass --install.",
            args.game, args.game
        )
    })
}

fn run(args: &Args) -> Result<(), String> {
    let install = install_path(args)?;
    match args.game.as_str() {
        cs_source::GAME => dump_cs_source(args, &install),
        combat_arms::GAME => dump_combat_arms(args, &install),
        other => Err(format!("unknown game `{other}`; expected cs_source or combat_arms")),
    }
}

fn human(bytes: u64) -> String {
    match bytes {
        b if b >= 1 << 30 => format!("{:.1} GB", b as f64 / (1u64 << 30) as f64),
        b if b >= 1 << 20 => format!("{:.1} MB", b as f64 / (1u64 << 20) as f64),
        b if b >= 1 << 10 => format!("{:.1} KB", b as f64 / 1024.0),
        b => format!("{b} B"),
    }
}

fn dump_cs_source(args: &Args, install: &Path) -> Result<(), String> {
    let mount = cs_source::mount::open(install).map_err(|e| e.to_string())?;
    let layers: Vec<String> = mount.layers().map(|l| l.name()).collect();
    let entries: Vec<_> = mount
        .entries()
        .into_iter()
        .filter(|(e, _)| args.filter.as_ref().is_none_or(|f| e.path.contains(f.as_str())))
        .collect();

    if args.list {
        for (e, layer) in &entries {
            println!("{:>10}  {}  [{}]", e.size, e.path, layer);
        }
    }

    let mut by_ext: BTreeMap<String, (usize, u64)> = BTreeMap::new();
    for (e, _) in &entries {
        let ext = e.path.rsplit_once('.').map(|(_, x)| x).unwrap_or("(none)").to_string();
        let slot = by_ext.entry(ext).or_default();
        slot.0 += 1;
        slot.1 += e.size;
    }
    println!("search path:");
    for (i, l) in layers.iter().enumerate() {
        println!("  [{i}] {l}");
    }
    let total: u64 = entries.iter().map(|(e, _)| e.size).sum();
    println!("{} files, {}", entries.len(), human(total));
    let mut top: Vec<_> = by_ext.into_iter().collect();
    top.sort_by_key(|(_, (_, size))| std::cmp::Reverse(*size));
    for (ext, (count, size)) in top.iter().take(15) {
        println!("  {ext:>8}: {count:>6} files, {}", human(*size));
    }

    if args.extract {
        let out = out_dir(args)?;
        for (e, _) in &entries {
            let dest = out.join(&e.path);
            fs::create_dir_all(dest.parent().unwrap()).map_err(|err| format!("{}: {err}", dest.display()))?;
            let data = mount.read(&e.path).map_err(|err| format!("{}: {err}", e.path))?;
            fs::write(&dest, data).map_err(|err| format!("{}: {err}", dest.display()))?;
        }
        println!("extracted {} files to {}", entries.len(), out.display());
    }
    Ok(())
}

fn dump_combat_arms(args: &Args, install: &Path) -> Result<(), String> {
    let game_dir = install.join("Game");
    let mut rez: Vec<_> = fs::read_dir(&game_dir)
        .map_err(|e| format!("{}: {e}; is this a Combat Arms install?", game_dir.display()))?
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x.eq_ignore_ascii_case("rez")))
        .filter(|p| {
            let name = p.file_name().unwrap().to_string_lossy().to_lowercase();
            args.filter.as_ref().is_none_or(|f| name.contains(f.as_str()))
        })
        .collect();
    rez.sort();

    let (mut v1, mut v2, mut total) = (0, 0, 0u64);
    if args.list {
        println!("{:>10}  {:7}  {:16}  name", "size", "entropy", "title");
    }
    for path in &rez {
        let size = fs::metadata(path).map(|m| m.len()).unwrap_or(0);
        total += size;
        let header = combat_arms::rez::RezHeader::read(path).map_err(|e| e.to_string())?;
        let entropy = combat_arms::rez::sample_entropy(path, 1 << 20).map_err(|e| e.to_string())?;
        match header.key_scheme() {
            Some("V1") => v1 += 1,
            Some(_) => v2 += 1,
            None => {}
        }
        if args.list {
            let title: String = header.title.chars().take(16).collect();
            let name = path.file_name().unwrap().to_string_lossy();
            println!("{:>10}  {entropy:7.3}  {title:16}  {name}", human(size));
        }
    }
    println!(
        "{} .rez archives, {}: {v1} keyed V1, {v2} keyed V2, {} plain (entropy near 8.0 = encrypted or compressed)",
        rez.len(),
        human(total),
        rez.len() - v1 - v2
    );
    println!("contents: not readable yet; needs the rez_archive spec (see specs/README.md)");
    if args.extract {
        return Err("Combat Arms extraction needs the rez_archive spec first".into());
    }
    Ok(())
}

/// The extraction folder. Refuses anything inside this repository, so game
/// files can't end up in git by accident.
fn out_dir(args: &Args) -> Result<PathBuf, String> {
    let out = match &args.out {
        Some(p) => p.clone(),
        None => default_dump_dir(&args.game).ok_or("no per-user data folder; pass --out")?,
    };
    fs::create_dir_all(&out).map_err(|e| format!("{}: {e}", out.display()))?;
    let out = out.canonicalize().map_err(|e| e.to_string())?;
    let repo = Path::new(env!("CARGO_MANIFEST_DIR"))
        .canonicalize()
        .map_err(|e| e.to_string())?;
    if out.starts_with(&repo) {
        return Err(format!(
            "{} is inside the repository. Game files must never be stored in the repo; choose a folder outside it.",
            out.display()
        ));
    }
    Ok(out)
}
