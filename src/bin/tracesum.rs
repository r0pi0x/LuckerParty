//! Development tool: summarize a Chrome trace written by a
//! `--features profile` build (Bevy's `trace_chrome`; `TRACE_CHROME=<file>`
//! names the file). Prints, per top-level span (the main world's `frame`,
//! the render world's, ...), its count and average time, and the spans
//! with the most time per frame, total and self (without their children).
//! See docs/performance.md ("Measuring").

use std::{
    collections::HashMap,
    io::{BufRead, BufReader},
    process::ExitCode,
};

const USAGE: &str = "\
usage: tracesum <trace.json> [options]
  --top <n>        spans to list (default 40)
  --skip <n>       ignore the first <n> frames (loading; default 120)
  --filter <text>  only spans whose name contains <text>
  --by-total       sort by total time instead of self time";

#[derive(Default, Clone)]
struct Stat {
    count: u64,
    total_us: f64,
    self_us: f64,
}

struct Open {
    name: String,
    start: f64,
    child_us: f64,
    root: String,
}

fn main() -> ExitCode {
    let mut args = std::env::args().skip(1);
    let (mut path, mut top, mut skip, mut filter, mut by_total) = (None, 40usize, 120u64, None, false);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--top" => top = args.next().and_then(|v| v.parse().ok()).unwrap_or(top),
            "--skip" => skip = args.next().and_then(|v| v.parse().ok()).unwrap_or(skip),
            "--filter" => filter = args.next(),
            "--by-total" => by_total = true,
            "-h" | "--help" => {
                println!("{USAGE}");
                return ExitCode::SUCCESS;
            }
            _ if path.is_none() => path = Some(a),
            _ => {
                eprintln!("{USAGE}");
                return ExitCode::from(2);
            }
        }
    }
    let Some(path) = path else {
        eprintln!("{USAGE}");
        return ExitCode::from(2);
    };
    let file = match std::fs::File::open(&path) {
        Ok(f) => f,
        Err(e) => {
            eprintln!("{path}: {e}");
            return ExitCode::FAILURE;
        }
    };
    // tracing-chrome writes one event per line ("{...},"), so a trace cut
    // short (a killed run) still reads.
    let mut stacks: HashMap<u64, Vec<Open>> = HashMap::new();
    let mut threads: HashMap<u64, String> = HashMap::new();
    let mut stats: HashMap<(String, String), Stat> = HashMap::new();
    let mut roots: HashMap<String, Stat> = HashMap::new();
    let mut frames = 0u64;
    for line in BufReader::new(file).lines() {
        let Ok(line) = line else { break };
        let l = line
            .trim()
            .trim_start_matches('[')
            .trim_end_matches(']')
            .trim_end_matches(',');
        if l.is_empty() {
            continue;
        }
        let Ok(e) = serde_json::from_str::<serde_json::Value>(l) else {
            continue;
        };
        let ph = e["ph"].as_str().unwrap_or("");
        let tid = e["tid"].as_u64().unwrap_or(0);
        let name = e["name"].as_str().unwrap_or("").to_string();
        if ph == "M" {
            threads.insert(tid, e["args"]["name"].as_str().unwrap_or("").to_string());
            continue;
        }
        let ts = e["ts"].as_f64().unwrap_or(0.0);
        let stack = stacks.entry(tid).or_default();
        match ph {
            "B" => {
                let root = stack.first().map_or_else(|| name.clone(), |o| o.root.clone());
                stack.push(Open {
                    name,
                    start: ts,
                    child_us: 0.0,
                    root,
                });
            }
            "E" => {
                let Some(open) = stack.pop() else { continue };
                let dur = ts - open.start;
                if let Some(parent) = stack.last_mut() {
                    parent.child_us += dur;
                }
                // One main-world update per frame.
                if open.name.starts_with("main app") {
                    frames += 1;
                }
                if frames < skip {
                    continue;
                }
                if stack.is_empty() || open.name.starts_with("main app") || open.name.starts_with("sub app") {
                    let r = roots.entry(open.name.clone()).or_default();
                    r.count += 1;
                    r.total_us += dur;
                }
                let thread = threads.get(&tid).map_or("?", |t| t.as_str());
                let group = if thread.starts_with("Compute") || thread.starts_with("Async") || thread.starts_with("IO")
                {
                    "workers".to_string()
                } else {
                    format!("thread {thread}, in {}", open.root)
                };
                let s = stats.entry((group, open.name)).or_default();
                s.count += 1;
                s.total_us += dur;
                s.self_us += dur - open.child_us;
            }
            _ => {}
        }
    }
    let counted = frames.saturating_sub(skip).max(1) as f64;
    println!(
        "{frames} frames in the trace, {} after skipping {skip}",
        frames.saturating_sub(skip)
    );
    println!(
        "\ntop-level spans and the worlds' updates (main app: the main world; sub app RenderApp: the render world, pipelined beside it):"
    );
    let mut r: Vec<_> = roots.into_iter().collect();
    r.sort_by(|a, b| b.1.total_us.total_cmp(&a.1.total_us));
    for (name, s) in r.iter().take(12) {
        println!(
            "  {:>8.3} ms avg  {:>8} x  {:>9.3} ms/frame  {}",
            s.total_us / s.count as f64 / 1e3,
            s.count,
            s.total_us / counted / 1e3,
            name
        );
    }
    let mut rows: Vec<_> = stats
        .into_iter()
        .filter(|((_, n), _)| filter.as_ref().is_none_or(|f| n.contains(f.as_str())))
        .collect();
    rows.sort_by(|a, b| {
        let key = |s: &Stat| if by_total { s.total_us } else { s.self_us };
        key(&b.1).total_cmp(&key(&a.1))
    });
    println!(
        "\nspans by {} time, ms per frame (thread and top-level span; systems run on the workers):",
        if by_total { "total" } else { "self" }
    );
    println!("  {:>8} {:>8} {:>8}  name", "self", "total", "calls");
    for ((root, name), s) in rows.iter().take(top) {
        println!(
            "  {:>8.3} {:>8.3} {:>8.1}  {name}  [{root}]",
            s.self_us / counted / 1e3,
            s.total_us / counted / 1e3,
            s.count as f64 / counted
        );
    }
    ExitCode::SUCCESS
}
