---
title: "Release Binary Build Failures from Missing rust-embed Directory"
date: 2026-04-01
category: ci-cd
tags:
  - rust-embed
  - github-actions
  - release-pipeline
  - dashboard
  - ci-cd
modules:
  - .github/workflows/release.yml
  - .github/workflows/release-plz.yml
  - release-plz.toml
  - crates/mika-agent/src/server/embedded_dashboard.rs
severity: medium
resolved: true
resolution_time: "1 hour"
---

# Release Binary Build Failures from Missing rust-embed Directory

## Problem Statement

All release binary builds failed since v0.1.6. The `release.yml` workflow did not create the `dashboard/dist/` directory that `rust-embed` requires at compile time. The `mika` CLI binary transitively depends on `mika-agent` (which uses `#[derive(Embed)] #[folder = "../../dashboard/dist/"]`), so even building just the CLI failed.

Additionally, only the `mika` CLI was published to GitHub Releases — the `mika-server` HTTP server binary was missing. Release names showed internal crate names (e.g., `mika-common-v0.3.1`) instead of clean version names.

## Root Cause

Three independent issues in the release pipeline:

1. **Missing directory:** `release.yml` lacked the `mkdir -p dashboard/dist && touch dashboard/dist/.gitkeep` step that `ci.yml` and `release-plz.yml` already had. The `rust-embed` derive macro fails at compile time if the referenced directory does not exist.

2. **Missing binary:** `release.yml` only had one `taiki-e/upload-rust-binary-action` step for the `mika` binary. The `mika-server` binary (a `[[bin]]` in the `mika-agent` crate) was not uploaded.

3. **Confusing names:** `release-plz.toml` lacked `git_release_name`, so the GitHub Release was named after the crate (`mika-common-v0.3.1`) rather than a clean version (`v0.3.1`).

## Solution

### 1. Add dashboard dist placeholder to release.yml

```yaml
- name: Create dashboard dist placeholder
  run: mkdir -p dashboard/dist && touch dashboard/dist/.gitkeep
```

This must appear before any Rust compilation step. The placeholder means zero dashboard assets are embedded (graceful degradation — `/dashboard/` shows a "disabled" page).

### 2. Add mika-server binary upload

```yaml
- name: Build and upload mika-server
  uses: taiki-e/upload-rust-binary-action@<sha>  # v1
  with:
    bin: mika-server
    features: telemetry
    target: ${{ matrix.target }}
    archive: mika-server-$tag-$target
    checksum: sha256
    token: ${{ secrets.GITHUB_TOKEN }}
```

Two separate upload steps produce separate archives — users can download just the CLI or just the server.

### 3. Fix release naming and disable crates.io

In `release-plz.toml`:
```toml
[workspace]
git_release_name = "v{{ version }}"

[[package]]
name = "mika-common"
publish = false
```

Belt-and-suspenders: also add `publish = false` to each crate's `Cargo.toml` to prevent accidental `cargo publish`.

## Gotchas

- **Transitive dependency:** Even if you only build the `mika` CLI, it depends on `mika-agent` which triggers the `rust-embed` derive macro. Any workflow that compiles any binary in this workspace needs the `dashboard/dist/` directory.
- **Empty vs missing:** `rust-embed` with an empty directory embeds zero files (graceful). A missing directory fails compilation. The placeholder trick (`touch .gitkeep`) ensures the directory exists.
- **Two upload steps, not one:** `taiki-e/upload-rust-binary-action` with `bin: mika,mika-server` would create a single combined archive. Separate steps create separate archives, which is correct for independent binaries.
- **Features parameter:** `features: telemetry` in the upload action maps to `--features telemetry` in cargo. Both `mika-cli` and `mika-agent` declare this feature and propagate it to `mika-common`.

## Prevention

- When adding new compile-time asset embedding (via `rust-embed`, `include_dir`, etc.), ensure ALL CI workflows that run `cargo build` create the required directories.
- Keep a checklist of workflows that need the placeholder: `ci.yml`, `release-plz.yml`, `release.yml`.
- The `release-plz.yml` dashboard placeholder already served as the pattern — the fix was copying it to `release.yml`.

## Related

- [Automated Release System](rust-workspace-release-plz-github-actions.md) — Full release pipeline documentation
- [Embed Dashboard SPA](../architecture-patterns/embed-dashboard-spa-rust-embed.md) — rust-embed integration details
- [#372](https://github.com/senara-solutions/mika/issues/372) — Tracking issue
