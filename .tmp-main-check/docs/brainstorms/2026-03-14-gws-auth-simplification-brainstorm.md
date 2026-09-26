# Brainstorm: Google Workspace Auth Simplification

**Date:** 2026-03-14
**Issue:** Follow-up to #75 (Google Workspace skill)
**Status:** Ready for planning

## What We're Building

Three changes to simplify Google Workspace integration:

1. **Remove `MIKA_GOOGLE_TOKEN`** — the `gws` CLI already has its own auth via `gws auth login` (keyring-based). No Mika-managed token needed for CLI usage.
2. **`run_gws` uses native gws auth** — just run `gws` without injecting any token env var. The keyring handles everything including token refresh.
3. **Block `gws` and `gh` from `run_shell`** — force the agent to use the dedicated `run_gws` and `run_gh` skills (which have security controls: subcommand allowlists, flag blocklists, env scrubbing).

## Why This Approach

The current `MIKA_GOOGLE_TOKEN` approach is broken and unnecessary:
- `gws auth export` outputs a status line before JSON, which corrupts the `.env` value
- `dotenvy` can't handle multi-line JSON in `.env`
- `gws auth login` already stores encrypted credentials + keyring tokens — just use that
- When Mika used `run_shell` with `gws gmail +triage`, it worked perfectly without any env vars

The token env var was designed for Docker/server deployments where keyring isn't available. But for CLI usage (the current phase), it's unnecessary complexity.

## Key Decisions

### 1. Remove `MIKA_GOOGLE_TOKEN` entirely
- Remove from `Settings` struct, `ToolContext`, all param structs, `ConfigKeyInfo` registry
- Remove from `.env.example`, `CLAUDE.md`, docs
- Undo the `google_token` threading from the earlier PR (revert the `brave_api_key`-mirroring pattern)

### 2. `run_gws` becomes auth-agnostic
- Still scrub `MIKA_*` env vars (defense-in-depth)
- Do NOT scrub `GOOGLE_WORKSPACE_CLI_*` — let gws use its native keyring/config
- No token validation, no error on missing token
- If gws returns an auth error, the system prompt already tells the agent to suggest `gws auth login`

### 3. Block `gws` and `gh` from `run_shell`
- Add command validation in the shell-exec handler script (`run.sh`)
- If the command starts with `gws` or `gh`, return an error directing the agent to use `run_gws` or `run_gh` instead
- This prevents the agent from bypassing the security controls (allowlists, flag blocking, env scrubbing) in the dedicated handlers

### 4. Docker/server auth (future)
When Docker/server deployments need Google Workspace, re-introduce auth as `MIKA_GOOGLE_CREDENTIALS_FILE` (path to credentials JSON). This is a future concern — not needed now.

## Resolved Questions

- **Why not multiple env vars?** Unnecessary — keyring auth works without any env vars
- **Why not keep MIKA_GOOGLE_TOKEN as optional?** Adds complexity for zero benefit in CLI mode. YAGNI.
- **Where to block gws/gh?** In the shell handler script — simplest enforcement point
- **What about the `google_token` threading we just added?** Revert it. It was solving the wrong problem.
