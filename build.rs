//! Runs before the program is compiled, to tell it which commit of its code it's built from
//! (`HARMONIA_COMMIT`, like "73bf7c9", or "73bf7c9 with uncommitted changes"). Every layout says
//! it, since the same seed only makes the same layout with the same code. Outside a git checkout
//! (or without git) it's "unknown".

use std::process::Command;

fn main() {
    let git = |args: &[&str]| {
        let out = Command::new("git").args(args).output().ok().filter(|out| out.status.success())?;
        Some(String::from_utf8_lossy(&out.stdout).trim().to_string())
    };
    let commit = match git(&["rev-parse", "--short", "HEAD"]) {
        Some(hash) => match git(&["status", "--porcelain", "--untracked-files=no"]) {
            Some(changes) if !changes.is_empty() => format!("{hash} with uncommitted changes"),
            _ => hash,
        },
        None => "unknown".to_string(),
    };
    println!("cargo:rustc-env=HARMONIA_COMMIT={commit}");
    // Ask again after a commit, a checkout, or an edit to anything built in (the code, the
    // footprint library, the package).
    // Only paths that exist: cargo reruns this on every build for one that doesn't (a git
    // worktree has no .git folder, a fresh clone no packed-refs).
    let watched = [".git/HEAD", ".git/index", ".git/refs", ".git/packed-refs", "src", "footprints.json", "Cargo.toml"];
    for path in watched.into_iter().filter(|p| std::path::Path::new(p).exists()) {
        println!("cargo:rerun-if-changed={path}");
    }
}
