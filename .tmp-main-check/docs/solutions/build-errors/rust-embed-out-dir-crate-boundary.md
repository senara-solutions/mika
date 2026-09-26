---
title: "rust-embed OUT_DIR pattern for cross-crate asset embedding"
date: 2026-04-02
category: build-errors
tags: [rust-embed, build-rs, OUT_DIR, cargo-package, dashboard, interpolate-folder-path]
modules:
  - crates/mika-agent/build.rs
  - crates/mika-agent/src/server/embedded_dashboard.rs
  - Cargo.toml
severity: medium
resolved: true
issue: "#374"
---

# rust-embed OUT_DIR Pattern for Cross-Crate Asset Embedding

## Problem

`rust-embed`'s `#[folder = "../../dashboard/dist/"]` uses a relative path from the crate root to reference assets at the workspace root. This violates Cargo's crate boundary expectations: `cargo package --verify` copies the crate to an isolated temp directory where `../../dashboard/dist/` doesn't exist, causing build failure.

Workarounds included committed `dashboard/dist/` in git, CI placeholder directories (`mkdir -p dashboard/dist && touch dashboard/dist/.gitkeep`), and disabling `cargo package --verify` in release-plz. All fragile.

## Root Cause

Cargo's packaging model expects all compile-time-referenced files to be within the crate directory or accessed through `OUT_DIR`. The `#[folder]` attribute in `rust-embed` resolves relative paths against `CARGO_MANIFEST_DIR` — a workspace-relative path like `../../dashboard/dist/` works during normal builds but breaks when the crate is built outside its workspace context.

## Solution

Use `build.rs` to copy dashboard assets into `OUT_DIR/dashboard_dist/`, then point `rust-embed` at that location using the `interpolate-folder-path` feature.

### 1. Enable env var expansion in rust-embed

```toml
# Cargo.toml (workspace)
rust-embed = { version = "8", features = ["interpolate-folder-path"] }
```

This pulls in `shellexpand` which resolves `$OUT_DIR` at proc-macro compile time.

### 2. Copy assets in build.rs

```rust
fn copy_dashboard_assets(manifest_dir: &str, out_dir: &str) {
    let dashboard_src = Path::new(manifest_dir).join("../../dashboard/dist");
    let dashboard_dst = Path::new(out_dir).join("dashboard_dist");

    println!("cargo:rerun-if-changed={}", dashboard_src.display());
    fs::create_dir_all(&dashboard_dst).unwrap();

    if !dashboard_src.exists() || !dashboard_src.is_dir() {
        println!("cargo:warning=Dashboard assets not found...");
        return;
    }

    copy_dir_recursive(&dashboard_src, &dashboard_dst);
}
```

Key details:
- **Dotfile filtering**: Skip `.gitkeep`, `.DS_Store` to prevent `has_embedded_assets()` false positives
- **Symlink skipping**: Use `entry.file_type().is_symlink()` guard to prevent embedding unintended files via supply chain compromise
- **Per-file `rerun-if-changed`**: Emit for each copied file, plus the source directory itself
- **Graceful missing**: Create empty `OUT_DIR/dashboard_dist/` when source absent — compilation succeeds with zero embedded files

### 3. Update the embed directive

```rust
#[derive(Embed)]
#[folder = "$OUT_DIR/dashboard_dist/"]
#[allow_missing = true]
struct DashboardAssets;
```

- `$OUT_DIR` expanded by `shellexpand::full()` at proc-macro time
- `#[allow_missing = true]` as defense-in-depth for the directory-not-created edge case

### 4. Simplify CI placeholders

```yaml
# Before:
run: mkdir -p dashboard/dist && touch dashboard/dist/.gitkeep

# After:
run: mkdir -p dashboard/dist
```

The `.gitkeep` is no longer needed since `#[allow_missing]` handles the empty case.

## Gotchas

- **`interpolate-folder-path` is required**: Without this feature, `$OUT_DIR` is treated as a literal string and compilation fails with a helpful hint about the feature flag.
- **`OUT_DIR` is available to proc macros**: Cargo sets `OUT_DIR` for the crate's compilation phase, not just for `build.rs`. This is under-documented but reliable.
- **Debug mode reads from filesystem**: In debug builds (without `debug-embed`), `rust-embed` reads files from the `#[folder]` path at runtime. With `$OUT_DIR`, this path is deep in `target/` and breaks after `cargo clean`. Acceptable since devs use the Vite dev server (`:5173`), not the embedded dashboard.
- **Existing pattern**: The same `build.rs` already copies docs into `OUT_DIR` for `include_str!()` — this extends that established pattern.

## Prevention

- For any cross-crate compile-time asset embedding, use the `build.rs` + `OUT_DIR` copy pattern instead of relative paths outside the crate boundary.
- Always add `#[allow_missing = true]` to `rust-embed` structs where the assets are optional — prevents build failures in CI/dev environments without the asset build step.
- Use `entry.file_type()` (not `path.is_dir()`) when recursively copying to avoid following symlinks.

## Related

- [Embed Dashboard SPA](../architecture-patterns/embed-dashboard-spa-rust-embed.md) — overall dashboard embedding architecture
- [Release Binary Build Failures](../ci-cd/release-binary-build-failures-rust-embed.md) — previous CI failures from missing dashboard/dist/
- [#374](https://github.com/senara-solutions/mika/issues/374) — tracking issue
