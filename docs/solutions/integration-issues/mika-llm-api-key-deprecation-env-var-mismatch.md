---
title: "MIKA_LLM_API_KEY deprecation — env var mismatch between setup and config"
category: integration-issues
date: 2026-03-29
tags: [config, env-vars, deprecation, oauth, setup, multi-provider]
related_modules: [mika-common/dotenv, mika-common/claude, mika-cli/setup, mika-cli/doctor, mika-agent/server]
severity: high
---

# MIKA_LLM_API_KEY Deprecation — Env Var Mismatch Between Setup and Config

## Problem

After the per-provider LLM config was introduced (2026-03-22), `MIKA_LLM_API_KEY` became a dead env var — it had no corresponding field in the `Settings` struct, so config-rs never read it. However, `setup.rs`, `doctor.rs`, `claude.rs`, and `mika-server.rs` all still referenced it in user-facing code.

This created a split-brain:
- `mika setup` wrote `MIKA_LLM_API_KEY` to `~/.mika/.env`
- The config system read `MIKA_ANTHROPIC_API_KEY` from the environment
- Every user who ran `mika setup` had a broken `.env` file

The OAuth flow was the most painful case: `setup --mode oauth` persisted the subscription token hash against `MIKA_LLM_API_KEY`, but `ClaudeClient` received the key from `MIKA_ANTHROPIC_API_KEY` — hash mismatch, auth failure with the error "OAuth token resolution failed."

## Root Cause

The per-provider config migration (introducing `MIKA_ANTHROPIC_API_KEY`, `MIKA_OPENAI_API_KEY`, etc.) updated the config-rs `Settings` struct and provider resolution code, but did not update the user-facing code paths (setup wizard, doctor health check, error messages) that still hardcoded the old `MIKA_LLM_API_KEY` name. The old env var had no `Settings` field, so `config-rs` with `Environment::with_prefix("MIKA")` simply ignored it.

## Solution

1. **Replace all references** to `MIKA_LLM_API_KEY` with `MIKA_ANTHROPIC_API_KEY` in:
   - `setup.rs` (interactive wizard, non-interactive `--api-key`, OAuth flow, docker-compose)
   - `doctor.rs` (health check env var and `.env` file scan)
   - `claude.rs` (error messages)
   - `mika-server.rs` (startup error message)
   - `smoke.rs` (test env var)
   - Comments in `executor.rs` and `mcp/mod.rs`

2. **Add deprecation warning** in `mika-common/src/dotenv.rs`:
   ```rust
   pub fn check_deprecated_env_vars() {
       if std::env::var("MIKA_LLM_API_KEY").is_ok() {
           warn!(
               "MIKA_LLM_API_KEY is deprecated and ignored by the config system. \
                Rename it to MIKA_ANTHROPIC_API_KEY in your ~/.mika/.env file."
           );
       }
   }
   ```
   Called after `load_dotenv()` in both CLI and server entry points.

3. **Replace explicit `unset` list with wildcard scrub** in `shell-exec/handlers/run.sh`:
   ```sh
   for _mika_var in $(env | grep '^MIKA_' | cut -d= -f1); do unset "$_mika_var"; done
   ```
   This mirrors the Rust executor's `scrub_mika_env_vars()` wildcard approach and eliminates the maintenance burden of keeping a hardcoded env var list in sync.

## Prevention

- **When adding per-provider config fields**, grep for all user-facing references to the old env var name — setup wizards, health checks, error messages, tests, and docs.
- **Shell handler env scrubbing** should use wildcard patterns (`MIKA_*`) instead of explicit lists to prevent future gaps.
- **The `ConfigKeyInfo` registry** in `config.rs` is the source of truth for which env vars the config system reads — user-facing code should reference the same names.

## Related

- Issue: #317
- Superseded doc: `docs/solutions/architecture-patterns/unified-llm-api-key-consolidation.md`
- Per-provider config plan: `docs/plans/2026-03-22-001-feat-per-provider-llm-config-plan.md`
