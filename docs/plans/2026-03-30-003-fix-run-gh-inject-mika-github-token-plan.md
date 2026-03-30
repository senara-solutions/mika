---
title: "fix: run_gh should inject MIKA_GITHUB_TOKEN as GH_TOKEN"
type: fix
status: active
date: 2026-03-30
issue: "#346"
---

# fix: run_gh should inject MIKA_GITHUB_TOKEN as GH_TOKEN

## Overview

The `run_gh` builtin handler does not inject `MIKA_GITHUB_TOKEN` as `GH_TOKEN` into the child process. This causes all agent `gh` CLI operations (QA reviews, issue management, PR comments) to run under the host's `gh auth` identity (`samidarko`) instead of the platform identity (`mika-platform`). This breaks the intended identity split and causes GitHub to block self-approval when claude-pilot creates PRs as `samidarko` and mika-qa tries to review as `samidarko`.

## Root Cause

The intended identity split:

| Layer | Token source | Identity | Responsibility |
|-------|-------------|----------|----------------|
| Host `gh auth` | `~/.config/gh/hosts.yml` | `samidarko` | Claude Code / claude-pilot: PR creation, git push |
| Mika platform | `MIKA_GITHUB_TOKEN` | `mika-platform` | Agent operations: QA reviews, PR comments, issue management |

`run_gh` in `builtin_handlers.rs:813` takes `_ctx: &ToolContext<'_>` (unused) and never sets `GH_TOKEN`. After `scrub_mika_env_vars()`, the child process falls back to `gh auth` from the host.

## Fix

### 1. Inject `ctx.github_token` as `GH_TOKEN` in `run_gh`

**File:** `crates/mika-agent/src/skills/builtin_handlers.rs:813`

- Rename `_ctx` to `ctx` in the function signature
- Before `scrub_mika_env_vars()`, inject the token:

```rust
// builtin_handlers.rs — run_gh (~line 825)
async fn run_gh(input: &serde_json::Value, ctx: &ToolContext<'_>) -> ToolOutput {
    // ... validation ...

    let mut cmd = tokio::process::Command::new("gh");
    cmd.args(&gh_args.args);

    if let Some(ref repo) = gh_args.repo {
        cmd.arg("--repo").arg(repo);
    }

    // Inject platform GitHub token for agent identity separation
    if let Some(token) = ctx.github_token {
        cmd.env("GH_TOKEN", token);
    }

    cmd.env("GH_PROMPT_DISABLED", "1");
    super::executor::scrub_mika_env_vars(&mut cmd);
    // GH_TOKEN is not MIKA_*-prefixed, so it survives scrubbing

    spawn_and_collect(cmd, "gh", "Is the GitHub CLI installed?").await
}
```

### 2. Update `.env.example`

**File:** `.env.example:77-80`

Update the `GH_TOKEN` comment to clarify it should NOT be set in `~/.mika/.env`:

```
# GH_TOKEN — DO NOT set in ~/.mika/.env. This var is for host-level gh CLI
# identity (e.g., Claude Code / claude-pilot sessions). Mika agent operations
# use MIKA_GITHUB_TOKEN instead, which run_gh injects as GH_TOKEN automatically.
# Setting GH_TOKEN in .mika/.env collapses both identities into one.
```

### 3. Update CLAUDE.md env var docs

**File:** `CLAUDE.md` — `GH_TOKEN` entry in Environment Variables section

Update to clarify the identity separation:

```
- `GH_TOKEN` — GitHub PAT for `gh` CLI in Claude Code sessions spawned via claude-pilot.
  Not `MIKA_*`-prefixed so it survives env scrubbing. Without this, `gh` falls back to
  the host user's personal `~/.config/gh/hosts.yml`. Do NOT set in `~/.mika/.env` — agent
  `run_gh` operations inject `MIKA_GITHUB_TOKEN` as `GH_TOKEN` automatically for platform
  identity separation.
```

### 4. Update `docs/configuration.md`

**File:** `docs/configuration.md:157` (and the crate-local copy at `crates/mika-agent/docs/configuration.md`)

Update `GH_TOKEN` docs to match the new behavior. Run `scripts/sync-agent-docs.sh` after editing `docs/configuration.md`.

## Acceptance Criteria

- [x] `run_gh` injects `ctx.github_token` as `GH_TOKEN` into child process
- [x] `_ctx` renamed to `ctx` in `run_gh` signature
- [x] `.env.example` updated: `GH_TOKEN` entry clarifies not to set in `~/.mika/.env`
- [x] `CLAUDE.md` env var docs for `GH_TOKEN` updated
- [x] `docs/configuration.md` updated for `GH_TOKEN` behavior
- [ ] `cargo test` passes (pre-existing dashboard build error blocks full test suite)
- [ ] `cargo clippy` passes (pre-existing dashboard build error)

## Sources

- GitHub issue: #346
- Current `run_gh`: `crates/mika-agent/src/skills/builtin_handlers.rs:813-830`
- Pattern reference: `crates/mika-agent/src/server/dashboard_dev_runs.rs:226` (`.env("GH_TOKEN", &github_token)`)
- Env scrubbing: `crates/mika-agent/src/skills/executor.rs` (`scrub_mika_env_vars`)
