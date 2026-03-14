---
title: "Google Workspace CLI: Remove Token Injection, Use Native Keyring Auth"
date: 2026-03-14
category: integration-issues
tags:
  - google-workspace
  - authentication
  - security
  - config-removal
  - cli-integration
severity: high
component:
  - crates/mika-common/src/config.rs
  - crates/mika-agent/src/skills/builtin_handlers.rs
  - crates/mika-agent/templates/skills/shell-exec/handlers/run.sh
related_issues:
  - "#75"
---

# Google Workspace CLI: Remove Token Injection, Use Native Keyring Auth

## Problem

The `MIKA_GOOGLE_TOKEN` environment variable approach for authenticating the Google Workspace CLI (`gws`) was fundamentally broken:

1. **`gws auth export` outputs a status line before JSON** — the first line is informational, not valid token data
2. **dotenvy cannot parse multi-line JSON** — the `.env` parser expects single-line `KEY=VALUE` pairs
3. **Truncated token causes 401 errors** — the corrupted `.env` value produces authentication failures

Meanwhile, `gws` already has native keyring-based auth via `gws auth login` that works perfectly without any Mika intervention.

## Root Cause

Misapplied pattern: the `run_gh` handler reads `GH_TOKEN` from ambient environment (set by GitHub Actions). This pattern was copied for `gws`, but `gws` uses OS keyring storage by default — no environment variable needed.

## Solution

Three coordinated changes:

### 1. Remove `MIKA_GOOGLE_TOKEN` from all layers

Removed the `google_token` field from:
- `Settings` struct and `ConfigKeyInfo` registry (`config.rs`)
- `ToolContext`, `AgentParams`, `SilentAgentParams`, `TeamAgentParams` (`agent.rs`, `tools/mod.rs`)
- CLI commands (`ask.rs`, `chat.rs`)
- Server state and handlers (`server/mod.rs`, `server/handlers.rs`, `server/state.rs`)
- Task engine dispatcher (`task_engine/dispatcher.rs`)
- Team engine (`teams/engine.rs`)
- Test utilities (`test_utils.rs`)
- Documentation (`.env.example`, `CLAUDE.md`, `docs/configuration.md`)

### 2. Simplify `run_gws` handler

Before:
```rust
// Read token from env, validate, scrub GOOGLE_WORKSPACE_CLI_* vars, inject token
let token = std::env::var("MIKA_GOOGLE_TOKEN")?;
scrub_gws_env_vars(&mut cmd);
cmd.env("GOOGLE_WORKSPACE_CLI_TOKEN", &token);
```

After:
```rust
// Just scrub MIKA_* vars and let gws use native keyring auth
super::executor::scrub_mika_env_vars(&mut cmd);
```

Removed: `scrub_gws_env_vars` function, token reading/validation, `--token` from blocked flags.
Kept: `scrub_mika_env_vars` (defense-in-depth), subcommand allowlist, config flag blocking.

### 3. Block `gws` and `gh` from `run_shell`

Added to `shell-exec/handlers/run.sh`:
```bash
FIRST_WORD=$(printf '%s\n' "$COMMAND" | awk '{print $1}')
case "$FIRST_WORD" in
    gws)  echo "Error: Use the dedicated run_gws skill instead of run_shell for security." >&2; exit 1 ;;
    gh)   echo "Error: Use the dedicated run_gh skill instead of run_shell for security." >&2; exit 1 ;;
esac
```

This forces the agent to use `run_gws`/`run_gh` with their security controls (allowlists, flag blocking, env scrubbing).

## Prevention

### Prefer native CLI auth over env var token injection

When integrating an external CLI tool:
1. Check if it has native auth (keyring, browser-based, `tool auth login`)
2. Test the tool without any MIKA_* env vars — if it works, don't add one
3. If a token env var is needed (e.g., Docker/server mode), test `tool auth export` output format against dotenvy's single-line requirement first

### Config field removal checklist

Removing a config field touches ~13 layers. Use this ordered checklist:
1. Settings struct + ConfigKeyInfo registry + Debug impl (`config.rs`)
2. ToolContext (`tools/mod.rs`)
3. AgentParams variants (`agent.rs`)
4. CLI commands (`commands/*.rs`)
5. Server state + handlers (`server/`)
6. Task engine (`task_engine/`)
7. Team engine (`teams/`)
8. Test utilities (`test_utils.rs`)
9. Documentation (`.env.example`, `CLAUDE.md`, `docs/configuration.md`)

Verify with: `grep -r "field_name\|ENV_VAR_NAME" . --include="*.rs" --include="*.md" --include="*.sh"`

### Block CLI tools from `run_shell` when dedicated handlers exist

Every CLI tool with a dedicated skill handler should be blocked in `shell-exec/handlers/run.sh` to prevent bypassing security controls.

## Related

- [CLI blocked-flag equals bypass](../security-issues/cli-blocked-flag-equals-bypass.md) — fixed `--flag=value` bypass in same branch
- [GWS auth simplification brainstorm](../../brainstorms/2026-03-14-gws-auth-simplification-brainstorm.md)
- [Google Workspace skill plan](../../plans/2026-03-13-feat-google-workspace-skill-plan.md)
