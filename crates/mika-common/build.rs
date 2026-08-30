//! Build-time provenance capture (mika#2066).
//!
//! This is the SINGLE place the git commit stamp is captured (AC3). It lives in
//! `mika-common` — the crate every binary links — because `option_env!` resolves
//! against the environment of the crate whose build script set the variable.
//! Capturing here, once, lets `mika`, `mika-gateway`, and `mika-spirit` all read
//! the same stamp through `mika_common::build_info` instead of recopying a
//! `build.rs` per crate.
//!
//! Falls back to `"unknown"` when `.git` is absent (Docker layers, source
//! tarballs) — never a build failure (AC3).

use std::process::Command;

fn main() {
    let git_hash = Command::new("git")
        .args(["rev-parse", "--short=8", "HEAD"])
        .output()
        .ok()
        .and_then(|o| o.status.success().then_some(o))
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "unknown".to_string());
    println!("cargo:rustc-env=GIT_HASH={git_hash}");

    // Rebuild the stamp when HEAD moves. The build script's CWD is this crate's
    // manifest dir, so resolve the real git dir instead of guessing a relative
    // `.git/` (the per-crate `build.rs` this replaces watched a path that never
    // existed under `crates/<crate>/.git`).
    if let Some(git_dir) = Command::new("git")
        .args(["rev-parse", "--absolute-git-dir"])
        .output()
        .ok()
        .and_then(|o| o.status.success().then_some(o))
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
    {
        println!("cargo::rerun-if-changed={git_dir}/HEAD");
        println!("cargo::rerun-if-changed={git_dir}/refs");
    }
}
