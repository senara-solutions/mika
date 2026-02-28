---
title: "Monorepo split: extract mika-gateway to mika-cloud"
date: 2026-02-28
category: refactoring
tags:
  - monorepo
  - gateway
  - cargo-workspace
  - git-dependency
severity: low
component: workspace
---

# Monorepo Split: Extract mika-gateway to mika-cloud

## Problem Statement

The mika monorepo contained both the public agent/CLI crates and the private gateway crate in a single workspace. The gateway (Telegram webhook router, Postgres customer registry) needs to be in a private repo for deployment security, while the agent and CLI remain public. Splitting the gateway into a separate `mika-cloud` repo required resolving Cargo workspace dependencies across repo boundaries.

## Findings

### Workspace Structure Before

```
mika/ (single repo)
  Cargo.toml          # workspace: members = ["crates/*"]
  crates/
    mika-common/      # shared lib (public)
    mika-agent/       # agent container (public)
    mika-cli/         # TUI CLI (public)
    mika-gateway/     # Telegram gateway (should be private)
  Dockerfile.agent
  Dockerfile.gateway
```

### Key Challenges

1. **Shared dependency: mika-common.** The gateway imports `mika-common` for config, logging, and Claude API types. After splitting, it needs to reference `mika-common` via git dependency instead of a local path.

2. **workspace.package inheritance.** The gateway's `Cargo.toml` uses `version.workspace = true`, `edition.workspace = true`, etc. These must be resolvable from the new workspace root.

3. **Dockerfile references.** The gateway Dockerfile references `crates/mika-common/`, `crates/mika-agent/`, and `crates/mika-gateway/` from the monorepo layout. These paths don't exist in the new repo.

4. **Cargo git auth.** The `mika-common` git dependency uses SSH (`ssh://git@github.com/...`), and Cargo's built-in libgit2 can't authenticate via SSH keys on disk (only ssh-agent). Requires `net.git-fetch-with-cli = true` in `~/.cargo/config.toml`.

5. **Unused workspace dependencies.** After removing the gateway, `sqlx` (Postgres driver) was only used by the gateway. Must be cleaned up from the workspace `Cargo.toml`.

## Solution

### mika-cloud workspace setup

```toml
# mika-cloud/Cargo.toml
[workspace]
members = ["mika-gateway"]
resolver = "2"

[workspace.package]
version = "0.1.0"
edition = "2024"
license = "MIT"

[workspace.dependencies]
# ... all gateway deps duplicated from mika workspace ...
mika-common = { git = "ssh://git@github.com/senara-solutions/mika" }
```

The `mika-gateway/Cargo.toml` remains unchanged — `workspace = true` references resolve against the new workspace root.

### Dockerfile update

The gateway Dockerfile was simplified: removed all `crates/` prefix paths, removed `mika-common` and `mika-agent` dummy source creation (they come from git dep now), and changed `COPY crates/` to `COPY mika-gateway/`.

### Cargo git authentication

```toml
# ~/.cargo/config.toml
[net]
git-fetch-with-cli = true
```

This tells Cargo to delegate git fetches to the `git` CLI, which can use SSH keys from `~/.ssh/` directly. Without this, Cargo's libgit2 only works with ssh-agent, which may not be running.

### Public repo cleanup

- Removed `crates/mika-gateway/` directory
- Removed `Dockerfile.gateway`
- Removed `sqlx` from workspace dependencies (gateway-only dep)
- `members = ["crates/*"]` glob automatically excludes the deleted directory

## Prevention: Monorepo Split Checklist

When extracting a crate from a Cargo workspace into a separate repo:

- [ ] **Duplicate workspace dependencies** needed by the extracted crate into the new workspace Cargo.toml
- [ ] **Switch shared crate references** from `path = "..."` to `git = "ssh://..."` (or HTTPS with token)
- [ ] **Set up `net.git-fetch-with-cli`** in `~/.cargo/config.toml` if using SSH keys
- [ ] **Update Dockerfile paths** to match the new directory layout (no more `crates/` prefix)
- [ ] **Remove git-dep-only crates** (like `mika-common` dummy sources) from Dockerfile
- [ ] **Clean up unused workspace deps** from the source repo (e.g., `sqlx` was gateway-only)
- [ ] **Update all documentation** (CLAUDE.md, README, deployment docs) to reference the new repo
- [ ] **Run `cargo build` and `cargo test`** in both repos to verify

## Gotchas

1. **HTTPS vs SSH git URLs.** Cargo git deps with HTTPS URLs (`https://github.com/...`) require credential helpers. SSH URLs (`ssh://git@github.com/...`) are more reliable when SSH keys are configured.
2. **`net.git-fetch-with-cli` is per-machine.** Each developer/CI machine needs this in `~/.cargo/config.toml`. Consider adding it to CI pipeline setup.
3. **Git dep resolves to HEAD.** Without a `branch`, `tag`, or `rev` specifier, `mika-common = { git = "..." }` resolves to the default branch HEAD. Pin to a tag for production stability.
4. **Workspace members glob.** `members = ["crates/*"]` auto-adjusts when directories are added/removed. No explicit member list changes needed.

## Technical Details

- **Source repo:** `github.com/senara-solutions/mika` (public)
- **Target repo:** `github.com/senara-solutions/mika-cloud` (private)
- **Shared dependency:** `mika-common` referenced via git

## Related Documentation

- [Deployment guide](../../deployment.md) — Gateway build instructions updated
- [Extraction plan](../../plans/2026-02-28-chore-extract-gateway-to-mika-cloud-plan.md)

## Work Log

- 2026-02-28: Extracted gateway to mika-cloud, resolved git dep auth, updated docs, both repos build and tests pass.
