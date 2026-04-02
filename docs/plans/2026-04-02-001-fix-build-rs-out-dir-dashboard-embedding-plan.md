---
title: "fix: use build.rs + OUT_DIR for dashboard asset embedding"
type: fix
status: completed
date: 2026-04-02
---

# fix: use build.rs + OUT_DIR for dashboard asset embedding

## Problem Statement

The dashboard assets live at `dashboard/dist/` (workspace root) but are embedded via `rust-embed` in `crates/mika-agent/` using `#[folder = "../../dashboard/dist/"]`. This cross-crate-boundary relative path fails `cargo package --verify` because packaging copies the crate to an isolated temp directory where the workspace structure doesn't exist.

Current workarounds include committed `dashboard/dist/` in git, CI placeholder `mkdir -p dashboard/dist && touch dashboard/dist/.gitkeep`, and disabling `cargo package --verify` in release-plz. These are fragile.

## Proposed Solution

Use `build.rs` + `OUT_DIR` to copy dashboard assets into the crate's build output directory, then point `rust-embed` at that location using the `interpolate-folder-path` feature (which supports `$OUT_DIR` expansion via `shellexpand`).

### Implementation Steps

#### 1. Enable `interpolate-folder-path` feature on `rust-embed`

**File:** `Cargo.toml` (workspace root, line ~123)

```rust
// Before:
rust-embed = "8"

// After:
rust-embed = { version = "8", features = ["interpolate-folder-path"] }
```

#### 2. Update `build.rs` to copy dashboard assets to `OUT_DIR`

**File:** `crates/mika-agent/build.rs`

Add a function after the existing docs-copy logic that:

1. Resolves source: `CARGO_MANIFEST_DIR/../../dashboard/dist/`
2. Resolves destination: `OUT_DIR/dashboard_dist/`
3. If source exists and is non-empty:
   - Recursively copies all files (filtering dotfiles like `.gitkeep`, `.DS_Store`)
   - Emits `cargo:rerun-if-changed` for the source directory AND each file within it
4. If source doesn't exist or is empty:
   - Creates an empty `OUT_DIR/dashboard_dist/` directory
   - Emits `cargo:warning=Dashboard dist not found`
   - Emits `cargo:rerun-if-changed` for the source path (so a future build picks up the directory when it appears)

This follows the **existing pattern** in the same `build.rs` for docs copying (`include_str!(concat!(env!("OUT_DIR"), ...))`) — established precedent.

Key: build.rs does NOT invoke npm/vite — it only copies already-built assets. The Rust and Node.js build systems remain decoupled (per documented architecture decision).

#### 3. Update the `#[folder]` attribute

**File:** `crates/mika-agent/src/server/embedded_dashboard.rs`

```rust
// Before:
#[derive(Embed)]
#[folder = "../../dashboard/dist/"]
struct DashboardAssets;

// After:
#[derive(Embed)]
#[folder = "$OUT_DIR/dashboard_dist/"]
#[allow_missing = true]
struct DashboardAssets;
```

- `$OUT_DIR` is expanded by `shellexpand::full()` at proc-macro time (Cargo sets `OUT_DIR` before proc macros run)
- `#[allow_missing = true]` is defense-in-depth: if `OUT_DIR/dashboard_dist/` somehow doesn't exist, the crate compiles with zero embedded files rather than failing

#### 4. Remove the old dashboard existence check from `build.rs`

The existing check for `../../dashboard/dist/index.html` (lines ~48-61) becomes redundant since the copy logic subsumes it. Replace with the new copy-or-create-empty logic.

#### 5. Clean up CI placeholder pattern

**Files:** `.github/workflows/ci.yml`, `.github/workflows/release-plz.yml`, `.github/workflows/release.yml`

The `mkdir -p dashboard/dist && touch dashboard/dist/.gitkeep` steps can be simplified to just `mkdir -p dashboard/dist` (no .gitkeep needed — build.rs creates `OUT_DIR/dashboard_dist/` regardless, and `#[allow_missing]` handles the edge case).

Alternatively, the entire placeholder step can be removed since build.rs handles the missing case gracefully. But keeping `mkdir -p dashboard/dist` is low-risk and documents intent.

#### 6. Re-enable `cargo package --verify` in release-plz

**File:** `.github/workflows/release-plz.yml`

If there's a `--no-verify` flag or skip-verification config, remove it so `cargo package --verify` runs.

## Technical Considerations

- **Debug mode:** `rust-embed` reads from filesystem at runtime in debug builds. With `$OUT_DIR`, the path is deep in `target/`. This is acceptable because devs use Vite dev server (`:5173`), not embedded dashboard, during development. `#[allow_missing]` prevents failures after `cargo clean`.
- **Build performance:** Copying ~5-20MB of dashboard assets adds minor I/O. Acceptable tradeoff. Symlinks would re-introduce the cross-boundary problem during `cargo package`.
- **`.gitkeep` false positive:** If CI creates `.gitkeep`, build.rs will filter dotfiles during copy to prevent `has_embedded_assets()` returning `true` incorrectly.
- **Committed `dashboard/dist/`:** Whether to remove it from git is a separate concern. This plan keeps the current git strategy unchanged.

## Acceptance Criteria

- [x] `cargo build` succeeds without `dashboard/dist/` existing (zero assets embedded)
- [x] `cargo build` with full `dashboard/dist/` embeds all assets correctly
- [ ] `cargo package --verify -p mika-agent` passes (the motivating case)
- [x] `cargo test` passes without `dashboard/dist/`
- [x] `cargo clippy` passes
- [x] Rebuilds trigger when `dashboard/dist/` contents change
- [x] Dashboard assets are correctly served at runtime (`/dashboard/*`)
- [x] `has_embedded_assets()` returns `false` when only placeholder files exist
- [ ] Docker build works end-to-end (3-stage)
- [ ] `make deploy` works end-to-end

## Files to Modify

| File | Change |
|------|--------|
| `Cargo.toml` | Add `interpolate-folder-path` feature to `rust-embed` |
| `crates/mika-agent/build.rs` | Add dashboard dist copy-to-OUT_DIR logic |
| `crates/mika-agent/src/server/embedded_dashboard.rs` | Change `#[folder]` to `$OUT_DIR`, add `#[allow_missing]` |
| `.github/workflows/ci.yml` | Simplify placeholder step |
| `.github/workflows/release-plz.yml` | Simplify placeholder, re-enable verify |
| `.github/workflows/release.yml` | Simplify placeholder step |

## Sources

- rust-embed v8 `interpolate-folder-path` feature: `shellexpand::full()` at proc-macro time
- Existing pattern: `crates/mika-agent/build.rs` docs copy to `OUT_DIR`
- Documented solution: `docs/solutions/architecture-patterns/embed-dashboard-spa-rust-embed.md`
- Documented solution: `docs/solutions/ci-cd/release-binary-build-failures-rust-embed.md`
- GitHub issue: #374
