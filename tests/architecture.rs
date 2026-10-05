//! Structural tests: module layering and repository rules from
//! docs/ARCHITECTURE.md and CLAUDE.md. Failure messages say how to fix the
//! problem, because agents read them.

use std::{
    collections::BTreeSet,
    fs,
    path::{Path, PathBuf},
    process::Command,
};

/// Top-level module -> top-level modules it may use via `crate::`.
/// Keep in sync with the layer diagram in docs/ARCHITECTURE.md.
const ALLOWED: &[(&str, &[&str])] = &[
    ("core", &[]),
    ("console", &["core"]),
    ("mount", &[]),
    ("slots", &["core"]),
    ("character", &["core", "slots"]),
    ("movement", &["core", "slots", "character"]),
    ("greybox", &["core"]),
    ("map", &["core"]),
    // `lib` = crate-root items such as `SimPlugins`.
    (
        "harness",
        &[
            "lib",
            "core",
            "console",
            "slots",
            "character",
            "movement",
            "greybox",
            "map",
        ],
    ),
    (
        "client",
        &[
            "lib",
            "core",
            "console",
            "slots",
            "character",
            "movement",
            "greybox",
            "map",
            "mount",
            "games",
        ],
    ),
    // Game plugins: may use the shared layers, never another game.
    (
        "games",
        &["core", "console", "slots", "character", "movement", "mount", "map"],
    ),
    // Binaries use the library through `mashup::`, not `crate::`.
    ("bin", &[]),
];

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn rust_files(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            rust_files(&path, out);
        } else if path.extension().is_some_and(|e| e == "rs") {
            out.push(path);
        }
    }
}

/// The top-level module a file belongs to: `src/movement/noclip.rs` -> `movement`.
fn module_of(src: &Path, file: &Path) -> Option<String> {
    let rel = file.strip_prefix(src).ok()?;
    let first = rel.components().next()?.as_os_str().to_str()?;
    Some(first.trim_end_matches(".rs").to_string())
}

fn ident(s: &str) -> String {
    s.trim_start()
        .chars()
        .take_while(|c| c.is_alphanumeric() || *c == '_')
        .collect()
}

/// Top-level paths used after `crate::` in `code` (comments already
/// stripped). `crate::{a::X, b}` yields `a` and `b`; `crate::games::cs::X`
/// yields `games::cs`. Crate-root items (`crate::SimPlugins`) yield `lib`.
fn crate_refs(code: &str, modules: &BTreeSet<String>) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    let mut add = |path: &str| {
        let first = ident(path);
        if first.is_empty() {
            return;
        }
        if first == "games" {
            let rest = path.trim_start()["games".len()..].trim_start_matches("::");
            out.insert(format!("games::{}", ident(rest)));
        } else if modules.contains(&first) {
            out.insert(first);
        } else {
            out.insert("lib".to_string());
        }
    };
    let mut rest = code;
    while let Some(i) = rest.find("crate::") {
        rest = &rest[i + "crate::".len()..];
        if let Some(group) = rest.strip_prefix('{') {
            // Split the group at top-level commas.
            let (mut depth, mut item_start) = (0usize, 0usize);
            for (j, c) in group.char_indices() {
                match c {
                    '{' => depth += 1,
                    '}' if depth == 0 => {
                        add(&group[item_start..j]);
                        break;
                    }
                    '}' => depth -= 1,
                    ',' if depth == 0 => {
                        add(&group[item_start..j]);
                        item_start = j + 1;
                    }
                    _ => {}
                }
            }
        } else {
            add(rest);
        }
    }
    out
}

#[test]
fn module_layering() {
    let src = root().join("src");
    let mut files = Vec::new();
    rust_files(&src, &mut files);
    let modules: BTreeSet<String> = ALLOWED.iter().map(|(m, _)| m.to_string()).collect();
    let mut errors = Vec::new();

    for file in files {
        let Some(module) = module_of(&src, &file) else { continue };
        if module == "lib" || module == "main" {
            continue;
        }
        let Some((_, allowed)) = ALLOWED.iter().find(|(m, _)| *m == module) else {
            errors.push(format!(
                "{}: module `{module}` has no layering rule. Add it to ALLOWED in tests/architecture.rs \
                 and to the layer diagram in docs/ARCHITECTURE.md.",
                file.display()
            ));
            continue;
        };
        let text = fs::read_to_string(&file).unwrap();
        // Strip line comments and join, so grouped imports can span lines.
        let code = text
            .lines()
            .map(|l| l.split("//").next().unwrap_or(""))
            .collect::<Vec<_>>()
            .join(" ");
        for r in crate_refs(&code, &modules) {
            if let Some(game) = r.strip_prefix("games::") {
                let own = file
                    .strip_prefix(src.join("games"))
                    .ok()
                    .and_then(|p| p.components().next());
                let own = own.map(|c| c.as_os_str().to_string_lossy().trim_end_matches(".rs").to_string());
                if module != "client" && own.as_deref() != Some(game) && module != "lib" {
                    errors.push(format!(
                        "{}: uses game `{game}`. Games must not depend on each other, and shared layers must \
                         not depend on any game. Move the shared concept into `core` (rule of three) or add a \
                         bridge; see docs/ARCHITECTURE.md.",
                        file.display()
                    ));
                }
                continue;
            }
            if r == module || allowed.contains(&r.as_str()) {
                continue;
            }
            errors.push(format!(
                "{}: `{module}` uses `crate::{r}`, which its layer may not depend on (allowed: {allowed:?}). \
                 Move the shared piece down a layer or invert the dependency (e.g. register through `slots`); \
                 see docs/ARCHITECTURE.md.",
                file.display()
            ));
        }
    }
    assert!(errors.is_empty(), "\n{}\n", errors.join("\n"));
}

fn tracked_files() -> Vec<String> {
    let out = Command::new("git")
        .args(["ls-files", "-z"])
        .current_dir(root())
        .output()
        .expect("git ls-files");
    String::from_utf8_lossy(&out.stdout)
        .split('\0')
        .filter(|s| !s.is_empty())
        .map(String::from)
        .collect()
}

/// Other games' assets must never be committed (README, "Assets, code and legal").
#[test]
fn no_game_assets_tracked() {
    const FORBIDDEN: &[&str] = &[
        "vpk", "bsp", "vtf", "vmt", "mdl", "vvd", "vtx", "phy", "rez", "ltb", "dtx", "dat", "wav", "mp3", "ogg",
    ];
    const MAX_BYTES: u64 = 1024 * 1024;
    let mut errors = Vec::new();
    for f in tracked_files() {
        let ext = Path::new(&f)
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_lowercase();
        if FORBIDDEN.contains(&ext.as_str()) {
            errors.push(format!("{f}: game asset type .{ext} is tracked. Remove it with `git rm --cached`; assets load at runtime from the user's install."));
        }
        if let Ok(meta) = fs::metadata(root().join(&f))
            && meta.len() > MAX_BYTES
            && f != "Cargo.lock"
        {
            errors.push(format!(
                "{f}: {} bytes. Large files are probably assets or extracted data; keep them out of git.",
                meta.len()
            ));
        }
    }
    assert!(errors.is_empty(), "\n{}\n", errors.join("\n"));
}

/// Specs describe behavior; they must not carry source code
/// (specs/README.md, "Rules for spec writers").
#[test]
fn specs_contain_no_code_blocks() {
    let mut errors = Vec::new();
    for f in tracked_files() {
        if !f.starts_with("specs/") || !f.ends_with(".md") || f == "specs/README.md" {
            continue;
        }
        let text = fs::read_to_string(root().join(&f)).unwrap();
        for (i, line) in text.lines().enumerate() {
            let t = line.trim_start();
            if t.starts_with("```") && !t.trim_start_matches('`').trim().is_empty() && !t.contains("math") {
                errors.push(format!(
                    "{f}:{}: fenced code block `{t}`. Specs describe behavior as prose, tables and math; \
                     do not include source or pseudocode that mirrors it. Use ```math for formulas.",
                    i + 1
                ));
            }
        }
    }
    assert!(errors.is_empty(), "\n{}\n", errors.join("\n"));
}

/// Documents that CLAUDE.md points to must exist, so the map never rots.
#[test]
fn docs_map_links_resolve() {
    let text = fs::read_to_string(root().join("CLAUDE.md")).unwrap();
    let mut errors = Vec::new();
    for part in text.split("](").skip(1) {
        let target = part.split(')').next().unwrap_or("");
        if target.starts_with("http") || target.is_empty() {
            continue;
        }
        let path = target.split('#').next().unwrap();
        if !root().join(path).exists() {
            errors.push(format!(
                "CLAUDE.md links to {target}, which does not exist. Fix the link or restore the file."
            ));
        }
    }
    assert!(errors.is_empty(), "\n{}\n", errors.join("\n"));
}
