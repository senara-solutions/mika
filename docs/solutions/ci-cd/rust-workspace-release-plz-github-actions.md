---
title: "Automated Release System for Rust Workspace with Cross-Platform Binaries"
date: 2026-03-01
category: ci-cd
tags:
  - release-automation
  - release-plz
  - github-actions
  - cross-compilation
  - rustls-migration
  - ci-cd
  - semantic-versioning
modules:
  - .github/workflows
  - crates/mika-common
  - crates/mika-agent
  - crates/mika-cli
  - crates/mika-gateway
  - Cargo.toml
  - release-plz.toml
  - install.sh
  - Dockerfile.agent
  - Dockerfile.gateway
severity: medium
resolved: true
resolution_time: "1 day"
superseded_by: release-please
---

# Automated Release System for Rust Workspace with Cross-Platform Binaries

> **Historical document.** This describes the release-plz setup (Stage 1, 2026-03-01 → 2026-04-03). The current release automation uses `googleapis/release-please-action` — see `release-please-config.json` at repo root. Retained for institutional memory per the chronic-drift compound doc.

## Problem Statement

The Mika project -- a Rust workspace with 4 crates (`mika-common`, `mika-agent`, `mika-cli`, `mika-gateway`) -- had no CI/CD pipeline. Users had to build from source or use `cargo install` (requiring a Rust toolchain plus C compiler for bundled SQLite). Six interconnected problems needed solving simultaneously:

1. No CI enforcement on PRs (formatting, lints, tests)
2. No automated versioning or publishing to crates.io
3. No pre-built binaries for end users
4. OpenSSL dynamic linkage made binaries non-portable across Linux distros
5. GitHub's `GITHUB_TOKEN` limitation: tags don't trigger downstream workflows
6. Toolchain/MSRV misalignment between Dockerfiles, `rust-toolchain.toml`, and `Cargo.toml`

## Solution

### Architecture: Three-Workflow Pipeline

```
Developer pushes conventional commits to main
    |
    v
ci.yml (fmt + clippy + test)
    |
    v  (in parallel)
release-plz.yml [release-pr job]
    |  creates/updates release PR with version bump + CHANGELOG
    v
Maintainer merges release PR
    |
    v
release-plz.yml [release job]
    |  creates git tag (v0.2.0) and GitHub Release using PAT
    |  (no crates.io publishing — all crates are publish = false)
    v
release.yml (triggered by v* tag)
    |  builds mika + mika-spirit binaries for 4 targets (with telemetry)
    |  uploads .tar.gz + .sha256 to GitHub Release
    v
End user: curl install.sh | sh
```

### Key Implementation Decisions

**1. TLS Portability: OpenSSL to rustls**

Switched the workspace-level `reqwest` dependency from default features (native-tls/OpenSSL) to `rustls-tls-native-roots`:

```toml
# Cargo.toml (workspace root)
reqwest = { version = "0.12", default-features = false, features = ["json", "rustls-tls-native-roots"] }
```

This eliminates OpenSSL dynamic linkage from all binaries. Verify with: `ldd target/release/mika` should show no `libssl` or `libcrypto`.

**2. PAT for Tag-Triggered Workflows**

GitHub's `GITHUB_TOKEN` tags don't trigger downstream workflows. Both release-plz jobs use `secrets.RELEASE_PLZ_TOKEN` (a PAT with `contents:write` + `pull_requests:write`). The PAT must be passed in two places: as `token:` for `actions/checkout` AND as `GITHUB_TOKEN` env var for the release-plz action.

**3. Cross-Compilation for aarch64-linux**

Uses `taiki-e/setup-cross-toolchain-action` (conditional on `matrix.target == 'aarch64-unknown-linux-gnu'`). Alternative `cross` was rejected due to Docker-in-Docker issues on GitHub Actions runners.

**4. Build Matrix**

Both `mika` CLI and `mika-spirit` HTTP server binaries are built for distribution (with `--features telemetry`). `mika-gateway` is Docker-only deployment. Release binaries use an empty `dashboard/dist/` placeholder — the embedded dashboard shows a "disabled" page; full dashboard embedding requires a Node.js build step (tracked as follow-up).

| Target | Runner | Notes |
|--------|--------|-------|
| `x86_64-unknown-linux-gnu` | `ubuntu-22.04` | Native build |
| `aarch64-unknown-linux-gnu` | `ubuntu-22.04` | Cross-compiled via taiki-e |
| `x86_64-apple-darwin` | `macos-13` | Intel Mac |
| `aarch64-apple-darwin` | `macos-14` | Apple Silicon |

**5. release-plz Configuration**

- Single aggregated changelog via `changelog_include` on `mika-common`
- `mika-gateway` excluded (`release = false`) -- Docker-only deployment
- `semver_check = false` -- not all crates expose stable public APIs
- Conventional commit parsing for categorized changelogs

**6. Supply-Chain Security**

All GitHub Actions pinned to full commit SHAs with version comments:

```yaml
- uses: actions/checkout@34e114876b0b11c390a56381ad16ebd13914f8d5  # v4
```

Fork safety via `if: github.repository_owner == 'senara-solutions'` on all release jobs.

## Gotchas and Pitfalls

### GITHUB_TOKEN Tags Do Not Trigger Downstream Workflows

**Symptom:** release-plz creates a `v*` tag, but `release.yml` never runs.
**Fix:** Use a PAT (`RELEASE_PLZ_TOKEN`) instead of `GITHUB_TOKEN` for all git operations in release-plz.

### OpenSSL Dynamic Linkage

**Symptom:** Binaries crash with `libssl.so.3: cannot open shared object file` on different Linux distros.
**Fix:** Use `rustls-tls-native-roots` feature. Verify with `ldd`.

### MSRV Drift

**Symptom:** Declared MSRV `1.85` but code requires newer APIs after clippy fixes.
**Fix:** Bumped MSRV to `1.91`. Keep MSRV honest -- it should reflect actual compilation requirements.

### Dockerfile Version Skew

**Symptom:** Dockerfiles used `rust:1.85-slim` but toolchain was pinned to 1.93.
**Fix:** Manually update both Dockerfiles when toolchain pin changes. Consider a CI check for consistency.

### Clippy Lint Evolution on Rust 1.93

New lints introduced: `io_other_error`, `collapsible_if` with let-chains, `vec_init_then_push`, `needless_borrow`, `print_literal`, `new_without_default`. Fixed across all 4 crates. Bulk fix with `cargo clippy --fix --allow-dirty` for mechanical changes (let-chains).

### Workspace Crate Publishing Order (Historical)

> **Note:** All crates are now `publish = false` — no crates.io publishing. This section is retained for historical context.

`mika-cli` depends on `mika-agent` depends on `mika-common`. release-plz handles dependency-ordered publishing natively and treats "already published" as success, making re-runs idempotent.

### Pipeline Artifacts Gate vs release-plz PRs

**Symptom:** The `pipeline-artifacts` CI job fails on every release-plz PR because it requires a plan doc (`docs/plans/*.md`) in the diff.
**Fix:** Add `!startsWith(github.head_ref, 'release-plz-')` to the job's `if` condition. Any CI job that enforces dev-workflow conventions must exclude automated bot branches.

### Concurrency Groups

Multiple rapid pushes to `main` can cause concurrent release-plz runs that conflict. Each job uses a `concurrency` group with `cancel-in-progress: false` to queue runs sequentially.

## Prevention Strategies

### Keep MSRV Honest
- Add a CI job that builds with the declared MSRV (`cargo-msrv verify`)
- Treat MSRV bumps as semver-minor changes

### Prevent OpenSSL Creep
- Add CI check: `cargo tree -i openssl-sys` should return no matches
- Audit transitive dependencies periodically

### Maintain Version Consistency
- Single source of truth: `rust-toolchain.toml`
- CI check comparing versions across Dockerfiles, CI YAML, and toolchain file
- Use Renovate/Dependabot for coordinated version bumps

### Monitor Release Pipeline
- Scheduled workflow to detect orphaned tags (tag exists, no release/binaries)
- `cargo publish --dry-run` as pre-release gate
- Dependabot for GitHub Actions SHA pin updates

## Required GitHub Repository Secrets

| Secret | Purpose |
|--------|---------|
| `RELEASE_PLZ_TOKEN` | PAT with `contents:write` + `pull_requests:write`. Required for tag-triggered downstream workflows. |

> **Note:** `CARGO_REGISTRY_TOKEN` was removed — all crates are `publish = false` (no crates.io publishing). The secret can be deleted from GitHub repository settings.

## Files Changed

| File | Change |
|------|--------|
| `.github/workflows/ci.yml` | New -- CI checks (fmt, clippy, test) |
| `.github/workflows/release-plz.yml` | New -- version management and crates.io publishing |
| `.github/workflows/release.yml` | New -- cross-platform binary builds |
| `release-plz.toml` | New -- release-plz configuration |
| `install.sh` | New -- installer script with SHA256 verification |
| `Cargo.toml` | Changed -- MSRV 1.85->1.91, reqwest native-tls->rustls |
| `Dockerfile.agent` | Changed -- rust:1.85->1.93, removed OpenSSL deps |
| `Dockerfile.gateway` | Changed -- rust:1.85->1.93, removed build deps |
| `crates/mika-cli/src/cli.rs` | Changed -- added `version` to clap command |
| Various source files | Changed -- clippy lint fixes for Rust 1.93 |

## Related

- [PR #41](https://github.com/senara-solutions/mika/pull/41) -- Implementation PR
- [Implementation Plan](../../plans/2026-03-01-feat-automated-release-system-plan.md)
- [Deployment Guide](../../deployment.md) -- Section 3c: CI/CD Pipeline
- [Gateway Monorepo Migration](../integration-issues/gateway-monorepo-migration.md) -- Prior Dockerfile work
- [release-plz documentation](https://release-plz.ieni.dev/docs)
- [taiki-e/upload-rust-binary-action](https://github.com/taiki-e/upload-rust-binary-action)
