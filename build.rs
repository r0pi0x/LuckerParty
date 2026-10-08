//! Stamps the build with its git commit (`MASHUP_GIT`), for bug reports.
//!
//! Reruns only when this checkout's commit changes: when HEAD or the
//! current branch's ref moves. Paths come from `git rev-parse --git-path`,
//! so they are right in worktrees too (there `.git` is a file, and a
//! missing rerun path would rerun this script, and relink every target,
//! on every build). The index isn't watched (every `git add` would
//! relink everything), so a `-dirty` mark is as of the last commit change.
use std::{path::Path, process::Command};

fn git(args: &[&str]) -> Option<String> {
    Command::new("git")
        .args(args)
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
}

fn main() {
    let hash = git(&["describe", "--always", "--dirty"]).unwrap_or_else(|| "unknown".into());
    println!("cargo:rustc-env=MASHUP_GIT={hash}");
    // Without git: rerun only when this script changes.
    println!("cargo:rerun-if-changed=build.rs");
    let Some(head) = git(&["rev-parse", "--git-path", "HEAD"]) else { return };
    println!("cargo:rerun-if-changed={head}");
    // The branch's own ref, or packed-refs when it is packed (a detached
    // HEAD changes HEAD itself).
    if let Some(branch) = git(&["symbolic-ref", "-q", "HEAD"]) {
        for name in [branch.as_str(), "packed-refs"] {
            if let Some(path) = git(&["rev-parse", "--git-path", name])
                && Path::new(&path).exists()
            {
                println!("cargo:rerun-if-changed={path}");
                break;
            }
        }
    }
}
