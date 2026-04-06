---
title: "fix: Gitignore dashboard/dist to prevent dirty repo after deploy"
type: fix
status: completed
date: 2026-04-06
---

# fix: Gitignore dashboard/dist to prevent dirty repo after deploy

## Overview

Every `make deploy` rebuilds the Vite dashboard, producing content-hashed filenames (`index-ChiOg_Zo.js`, `index-UyiYNgTW.css`) that differ between builds even when source hasn't changed. Since `dashboard/dist/` is committed to git, the repo appears dirty after every deploy — old hashed files show as deleted, new ones as untracked.

## Problem Statement

The root cause chain:
1. `make deploy` calls `build-dashboard` which runs `npm ci` + `vite build`
2. Vite's default `[name]-[hash]` output pattern produces new filenames each build
3. `dashboard/dist/` is tracked in git (for rust-embed to embed into the binary)
4. New filenames make `git status` show deletions + untracked files

## Proposed Solution

**Gitignore `dashboard/dist/` and remove it from git tracking.** This is safe because the entire build pipeline already handles the missing case:

- `build.rs` creates an empty `$OUT_DIR/dashboard_dist/` when source is missing
- `#[allow_missing = true]` on the rust-embed struct compiles without assets
- The Makefile `deploy` target runs `build-dashboard` before `cargo build`
- CI already uses `mkdir -p dashboard/dist` as a placeholder
- `Dockerfile.agent` has a separate Node.js builder stage
- The server gracefully shows "not built" / "disabled" pages when assets are absent

## Acceptance Criteria

- [x] `dashboard/dist/` added to `.gitignore`
- [x] Existing tracked files removed from git index (`git rm -r --cached`)
- [x] Stale `.gitignore` comment updated
- [x] `make deploy` produces a clean `git status`
- [x] `cargo build` alone (without dashboard build) still compiles
- [x] Phantom `deploy-dashboard` reference in CLAUDE.md fixed

## MVP

### .gitignore

Replace the comment on line 22 and add the gitignore entry:

```gitignore
# dashboard/dist/ is a build artifact — embedded into mika-server via rust-embed at compile time.
# Run `make deploy` or `npm run build --prefix dashboard` to produce it.
dashboard/dist/
```

### Git index cleanup

```bash
git rm -r --cached dashboard/dist/
```

### CLAUDE.md

Remove or fix the phantom `make deploy-dashboard` command reference — `make deploy` already includes the dashboard build step.

## Sources

- Existing solution: `docs/solutions/build-errors/rust-embed-out-dir-crate-boundary.md` — confirms `#[allow_missing = true]` handles absent dist
- Existing solution: `docs/solutions/ci-cd/release-binary-build-failures-rust-embed.md` — confirms CI placeholders are belt-and-suspenders
- Existing solution: `docs/solutions/architecture-patterns/embed-dashboard-spa-rust-embed.md` — documents the embedding architecture
- `crates/mika-agent/build.rs:48-85` — copies dashboard/dist to OUT_DIR, handles missing case
- `crates/mika-agent/src/server/embedded_dashboard.rs:17-20` — rust-embed struct with `#[allow_missing = true]`
