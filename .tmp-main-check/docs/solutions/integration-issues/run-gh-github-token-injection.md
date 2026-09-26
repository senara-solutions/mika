---
title: "run_gh missing MIKA_GITHUB_TOKEN injection as GH_TOKEN"
category: integration-issues
date: 2026-03-30
tags: [github, identity, token, run_gh, builtin-handler, env-vars]
issue: "#346"
---

# run_gh missing MIKA_GITHUB_TOKEN injection as GH_TOKEN

## Problem

The `run_gh` builtin handler did not inject `MIKA_GITHUB_TOKEN` as `GH_TOKEN` into child
processes. All agent `gh` CLI operations (QA reviews, PR comments, issue management) ran
under the host's `gh auth` identity instead of the platform identity. This broke the
intended identity split and caused GitHub to block self-approval when claude-pilot created
PRs as the same identity that mika-qa used for reviews.

**Symptom:** GitHub blocks PR self-approval because both PR author (claude-pilot) and
reviewer (mika-qa via `run_gh`) resolve to the same GitHub user.

## Root Cause

`run_gh` in `builtin_handlers.rs` took `_ctx: &ToolContext<'_>` (unused context) and never
set `GH_TOKEN` on the child process. After `scrub_mika_env_vars()` removed all `MIKA_*`
env vars, the `gh` CLI fell back to the host's `~/.config/gh/hosts.yml` auth — the same
identity used by claude-pilot for PR creation.

The intended identity split:

| Layer | Identity | Purpose |
|-------|----------|---------|
| Host `gh auth` | Developer account | Claude Code / claude-pilot: PR creation, git push |
| `MIKA_GITHUB_TOKEN` | Platform account | Agent operations: QA reviews, PR comments, issues |

A previous workaround of setting `GH_TOKEN` in `~/.mika/.env` collapsed both identities
into one, making the problem worse.

## Solution

Three-line fix in `crates/mika-agent/src/skills/builtin_handlers.rs`:

```rust
// 1. Rename _ctx to ctx in the function signature
async fn run_gh(input: &serde_json::Value, ctx: &ToolContext<'_>) -> ToolOutput {
    // ...
    // 2. Before scrub_mika_env_vars(), inject the platform token
    if let Some(token) = ctx.github_token {
        cmd.env("GH_TOKEN", token);
    }
    // GH_TOKEN is not MIKA_*-prefixed, so it survives scrub_mika_env_vars()
}
```

Key ordering: inject BEFORE `scrub_mika_env_vars()` — `GH_TOKEN` is not `MIKA_*`-prefixed
so it survives the scrub. This follows the same pattern as `dashboard_dev_runs.rs:226`.

Documentation was updated across `.env.example`, `CLAUDE.md`, `docs/configuration.md`, and
the github skill prompt to clarify that `GH_TOKEN` should NOT be set in `~/.mika/.env`.

## Prevention

- When adding new builtin handlers that call external CLIs, check whether the CLI needs
  identity-specific tokens from `ToolContext`. The `_ctx` pattern (unused context) is a
  code smell — if the handler interacts with authenticated services, it likely needs `ctx`.
- The `gws-token-auth-removal.md` solution in this directory documents a related anti-pattern
  where token injection was misapplied. Compare both to understand when token injection is
  appropriate (gh: yes, gws: no — uses OS keyring).

## Related

- [gws-token-auth-removal.md](gws-token-auth-removal.md) — Related anti-pattern where token
  injection was incorrectly applied to `gws`
- `crates/mika-agent/src/server/dashboard_dev_runs.rs:226` — Existing pattern for GH_TOKEN injection
