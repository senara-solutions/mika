---
title: "Dashboard dist makes repo dirty after every deploy"
category: build-errors
date: 2026-04-06
tags: [vite, dashboard, rust-embed, gitignore, build-artifacts]
modules: [dashboard, mika-agent/build.rs, mika-agent/server/embedded_dashboard]
---

# Dashboard dist makes repo dirty after every deploy

## Problem

Every `make deploy` rebuilds the Vite dashboard, producing content-hashed filenames (e.g., `index-ChiOg_Zo.js`) that differ between builds even when source hasn't changed. Since `dashboard/dist/` was committed to git (for rust-embed to embed into the binary), the repo appeared dirty after every deploy — old hashed files showed as deleted, new ones as untracked.

## Root Cause

Vite's default `[name]-[hash]` output pattern produces non-deterministic filenames across `npm ci` reinstalls. The `dashboard/dist/` directory was tracked in git based on the assumption that rust-embed needed it at commit time, but `build.rs` actually copies it to `$OUT_DIR` at compile time — the git-tracked copy was unnecessary.

## Solution

Add `dashboard/dist/` to `.gitignore` and remove existing tracked files with `git rm -r --cached dashboard/dist/`.

This is safe because the entire build pipeline already handles the missing case:
- `build.rs` creates an empty `$OUT_DIR/dashboard_dist/` when source is missing
- `#[allow_missing = true]` on the rust-embed struct compiles without assets
- The Makefile `deploy` target runs `build-dashboard` before `cargo build`
- CI uses `mkdir -p dashboard/dist` as a placeholder
- `Dockerfile.agent` has a separate Node.js builder stage

## Prevention

When adding build artifacts that are embedded at compile time, prefer gitignoring them and relying on build-time generation rather than committing them. Check if the embedding mechanism (rust-embed, include_bytes, etc.) has a graceful fallback for missing files before deciding to track build output.

## Related

- `docs/solutions/build-errors/rust-embed-out-dir-crate-boundary.md` — the OUT_DIR pattern that made this fix safe
- `docs/solutions/ci-cd/release-binary-build-failures-rust-embed.md` — CI placeholder pattern
- `docs/solutions/architecture-patterns/embed-dashboard-spa-rust-embed.md` — dashboard embedding architecture
