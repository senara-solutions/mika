---
title: "chore: Extract mika-gateway to mika-cloud private repo"
type: refactor
status: active
date: 2026-02-28
---

# Extract mika-gateway to mika-cloud

## Overview

Split the mika monorepo by extracting the gateway crate into a separate private repo (`mika-cloud`). The gateway crate and Dockerfile have already been copied to `/home/samidarko/workspace/senara-solutions/mika-cloud/`.

## Acceptance Criteria

### mika-cloud (new private repo)

- [x] Workspace `Cargo.toml` references `mika-common` via git from the public mika repo
- [ ] `mika-gateway/Cargo.toml` resolves `workspace.package` fields correctly (inline if needed)
- [ ] Dockerfile updated for new directory structure (no `crates/` prefix, no mika-agent references)
- [ ] `cargo build` compiles successfully
- [ ] Initial commit pushed

### mika (public repo)

- [ ] `crates/mika-gateway/` removed from workspace members
- [ ] `crates/mika-gateway/` directory deleted
- [ ] `Dockerfile.gateway` deleted
- [ ] `cargo build` compiles successfully
- [ ] Commit pushed

## Technical Details

### Key Issues to Fix

1. **Dockerfile in mika-cloud** still references `crates/` paths from the monorepo layout. Needs updating to match the new flat structure (`mika-gateway/` not `crates/mika-gateway/`). Also references `mika-common` and `mika-agent` dummy source creation which are no longer present locally — `mika-common` comes from git dep now.

2. **mika-gateway/Cargo.toml** uses `workspace = true` references which are already properly resolved by the workspace `Cargo.toml` — no changes needed.

3. **mika public Cargo.toml** uses `members = ["crates/*"]` glob which will automatically exclude `mika-gateway` once the directory is deleted. No explicit member removal needed.

4. **Cargo.lock**: mika-cloud needs to generate its own lockfile via `cargo build`. The public mika repo lockfile will be regenerated without gateway deps.

## Implementation Steps

1. Fix mika-cloud Dockerfile for new directory structure
2. Build mika-cloud (`cargo build`)
3. Commit and push mika-cloud
4. Delete `crates/mika-gateway/` and `Dockerfile.gateway` from mika public repo
5. Build mika public repo (`cargo build`)
6. Commit and push mika public repo
