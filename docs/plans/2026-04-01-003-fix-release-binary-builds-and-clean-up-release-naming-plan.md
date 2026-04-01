---
title: "Fix release binary builds and clean up release naming"
type: fix
status: completed
date: 2026-04-01
issue: "#372"
---

# Fix release binary builds and clean up release naming

## Overview

The GitHub Releases page is broken: all binary builds fail since v0.1.6, release names show internal crate names instead of the product name, `mika-server` is not published, and crates.io publishing is unnecessary. This plan fixes all four problems.

## Problem Statement

1. **Binary builds fail** — `release.yml` doesn't create `dashboard/dist/` placeholder, so `rust-embed` compilation fails (the `mika` CLI depends on `mika-agent` which uses `rust-embed` for `../../dashboard/dist/`)
2. **Confusing release names** — release-plz names releases after `mika-common` (the only crate with `git_release_enable = true`), producing names like `mika-common-v0.3.1`
3. **Missing `mika-server` binary** — only the `mika` CLI is uploaded to GitHub Releases
4. **Unnecessary crates.io publishing** — distribution is via GitHub Releases only; no external consumers depend on these crates

## Proposed Solution

Four targeted changes across CI/CD configuration files and Cargo manifests.

### Change 1: `.github/workflows/release.yml` — fix build + add mika-server + telemetry

**File:** `.github/workflows/release.yml`

Add dashboard dist placeholder step before the build (same as `ci.yml` line 31 and `release-plz.yml` lines 27/50):

```yaml
      - name: Create dashboard dist placeholder
        run: mkdir -p dashboard/dist && touch dashboard/dist/.gitkeep
```

Add `features: telemetry` to the existing mika upload step (matching `make deploy` behavior):

```yaml
      - name: Build and upload mika
        uses: taiki-e/upload-rust-binary-action@f391289bcff6a7f36b6301c0a74199657bbb4561  # v1
        with:
          bin: mika
          features: telemetry
          target: ${{ matrix.target }}
          archive: mika-$tag-$target
          checksum: sha256
          token: ${{ secrets.GITHUB_TOKEN }}
```

Add a second upload step for `mika-server` (separate archive):

```yaml
      - name: Build and upload mika-server
        uses: taiki-e/upload-rust-binary-action@f391289bcff6a7f36b6301c0a74199657bbb4561  # v1
        with:
          bin: mika-server
          features: telemetry
          target: ${{ matrix.target }}
          archive: mika-server-$tag-$target
          checksum: sha256
          token: ${{ secrets.GITHUB_TOKEN }}
```

**Note:** `mika-server` is a `[[bin]]` in `mika-agent` crate. The dashboard dist placeholder means zero assets are embedded (graceful degradation — the `/dashboard/` endpoint shows a "disabled" page). A full Node.js dashboard build in CI is a follow-up concern.

**Note:** `mika-gateway` is intentionally excluded — it's deployed via Docker images to Kubernetes, not as a standalone binary. Add a comment in the workflow to document this decision.

**Toolchain fix:** The current workflow uses `dtolnay/rust-toolchain@...` pinned to stable. Per institutional learnings (`docs/solutions/ci-cd/ci-rust-toolchain-version-mismatch.md`), repos with `rust-toolchain.toml` should use `rustup show` instead. However, `release.yml` uses `dtolnay/rust-toolchain` with explicit `targets:` for cross-compilation — changing this requires ensuring the cross-compilation targets are installed via `rustup target add`. This is a separate concern and should NOT be mixed into this PR to keep it focused.

### Change 2: `release-plz.toml` — stop crates.io publishing, fix release naming

**File:** `release-plz.toml`

Add `git_release_name` to the `[workspace]` section:

```toml
[workspace]
git_release_enable = true
git_tag_enable = true
git_tag_name = "v{{ version }}"
git_release_name = "v{{ version }}"
semver_check = false
pr_labels = ["release"]
```

Set `publish = false` on all four publishable crates:

```toml
[[package]]
name = "mika-common"
publish = false
# ... rest unchanged

[[package]]
name = "mika-agent"
publish = false
# ... rest unchanged

[[package]]
name = "mika-cli"
publish = false
# ... rest unchanged

[[package]]
name = "mika-a2a"
publish = false
# ... rest unchanged
```

### Change 3: `.github/workflows/release-plz.yml` — remove cargo registry token

**File:** `.github/workflows/release-plz.yml`

Remove the `CARGO_REGISTRY_TOKEN` line from the `release-plz-release` job:

```yaml
      - uses: release-plz/action@f708778669256143d984cce4b23592637532e040  # v0.5
        with:
          command: release
        env:
          GITHUB_TOKEN: ${{ secrets.RELEASE_PLZ_TOKEN }}
          # CARGO_REGISTRY_TOKEN removed — no crates.io publishing
```

**Manual follow-up:** Delete the `CARGO_REGISTRY_TOKEN` secret from GitHub repository settings after this PR merges.

### Change 4: All `crates/*/Cargo.toml` — add `publish = false`

Belt-and-suspenders with the release-plz config. Prevents accidental `cargo publish` from any context.

| File | Change |
|------|--------|
| `crates/mika-common/Cargo.toml` | Add `publish = false` |
| `crates/mika-agent/Cargo.toml` | Add `publish = false` |
| `crates/mika-cli/Cargo.toml` | Add `publish = false` |
| `crates/mika-a2a/Cargo.toml` | Add `publish = false` |
| `crates/mika-gateway/Cargo.toml` | Already has `publish = false` — no change |

## Acceptance Criteria

- [x] `release.yml` creates `dashboard/dist/` placeholder before Rust compilation
- [x] `release.yml` builds and uploads `mika` binary with telemetry feature
- [x] `release.yml` builds and uploads `mika-server` binary with telemetry feature (separate archive)
- [x] `release-plz.toml` has `publish = false` on all four crates (mika-common, mika-agent, mika-cli, mika-a2a)
- [x] `release-plz.toml` has `git_release_name = "v{{ version }}"` for clean release naming
- [x] `release-plz.yml` no longer references `CARGO_REGISTRY_TOKEN`
- [x] All four crate `Cargo.toml` files have `publish = false`
- [x] `mika-gateway` remains excluded from release binaries (comment in workflow explains why)
- [x] `cargo clippy` passes
- [x] `cargo test` passes

## Out of Scope (Follow-ups)

- **Dashboard embedding in release binaries** — requires Node.js build step in `release.yml`; current PR uses empty placeholder (zero assets, graceful degradation)
- **`install.sh` update for mika-server** — installer currently only handles `mika` CLI; track separately
- **Yanking existing crates.io versions** — manual action after PR merges (`cargo yank` for mika-common 0.1.3/0.1.4, mika-agent, mika-cli, mika-a2a)
- **Toolchain consistency in `release.yml`** — switching from `dtolnay/rust-toolchain` to `rustup show` (per `docs/solutions/ci-cd/ci-rust-toolchain-version-mismatch.md`); separate concern
- **Delete `CARGO_REGISTRY_TOKEN` secret from GitHub repo settings** — manual action

## Sources

- Issue: [#372](https://github.com/senara-solutions/mika/issues/372)
- Institutional learning: `docs/solutions/ci-cd/rust-workspace-release-plz-github-actions.md`
- Institutional learning: `docs/solutions/architecture-patterns/embed-dashboard-spa-rust-embed.md`
- Current files: `.github/workflows/release.yml`, `.github/workflows/release-plz.yml`, `release-plz.toml`
