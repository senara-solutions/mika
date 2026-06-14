---
title: "fix: clear token separation — remove GH_TOKEN from .mika/.env, verify run_gh injection"
type: fix
status: completed
date: 2026-04-02
issue: "#380"
---

# fix: clear token separation — remove GH_TOKEN from .mika/.env, verify run_gh injection

## Overview

`GH_TOKEN` in `~/.mika/.env` causes `dotenvy` to inject it into Mika's process env, overriding the host's `gh auth` for ALL `gh` commands (including Claude Code inside claude-pilot). This collapses both identities into `mika-platform`, causing the self-approval block in the autonomous dev loop.

## Current State (Already Implemented)

Research confirms these are already correct:

- **`run_gh` injection** (`crates/mika-agent/src/skills/builtin_handlers.rs:826-833`): Correctly injects `MIKA_GITHUB_TOKEN` as `GH_TOKEN` on child Command before `scrub_mika_env_vars()`. `GH_TOKEN` survives the scrub because it lacks the `MIKA_` prefix. ✅
- **`.env.example`** (lines 76-79): Already warns against setting `GH_TOKEN` in `~/.mika/.env`. ✅
- **`docs/configuration.md`** (lines 153-171): Already documents the identity matrix. ✅
- **Solution doc**: `docs/solutions/integration-issues/run-gh-github-token-injection.md` documents the fix. ✅

## Remaining Work

### 1. Add startup warning + active removal of `GH_TOKEN` from process env

**File:** `crates/mika-common/src/dotenv.rs`

The `check_deprecated_env_vars()` function currently only checks for `MIKA_LLM_API_KEY`. Extend it to also detect `GH_TOKEN` in the `.env` file and actively remove it from the process environment.

**Why active removal, not just a warning:** In autonomous mode (claude-pilot), nobody reads startup logs. A warning alone leaves the identity collision active for the entire process lifetime. Active removal is safe because:
- `run_gh` explicitly sets `GH_TOKEN` from `ctx.github_token` — it never relies on the inherited env
- `git_ops` and other handlers set `GIT_TERMINAL_PROMPT=0` — they don't use `GH_TOKEN` for auth
- MCP child processes use `env_clear()` — `GH_TOKEN` never reaches them anyway

**Implementation approach:**
1. Before `load_dotenv()` runs, check if `GH_TOKEN` exists in the process env (from shell)
2. After `load_dotenv()` runs, if `GH_TOKEN` is now set and it WASN'T set before, it came from `.env`
3. Remove it from the process env with `std::env::remove_var("GH_TOKEN")`
4. Emit a `warn!` log: `"GH_TOKEN found in {path}/.env and removed from process environment. Use MIKA_GITHUB_TOKEN for agent GitHub operations. See docs/configuration.md for identity separation."`

Alternatively, a simpler approach that achieves the same safety: parse the `.env` file as text before loading, check for `GH_TOKEN=` lines, warn, and then after loading call `std::env::remove_var("GH_TOKEN")` only if it wasn't set in the shell pre-load.

**Signature change:** `check_deprecated_env_vars()` → `check_env_warnings(home_dir: &Path)` (takes the home dir to locate the `.env` file, covers both deprecated vars and misplaced vars).

**Call sites to update:**
- `crates/mika-cli/src/main.rs:33` (global home)
- `crates/mika-cli/src/main.rs:115` (agent-specific home)
- `crates/mika-agent/src/bin/mika-spirit.rs:7`

### 2. Add `GH_TOKEN` to exec handler env scrubbing

**File:** `crates/mika-agent/src/skills/executor.rs`

Add `GH_TOKEN` to the blocklist in both `scrub_mika_env_vars()` and `scrub_mika_env_vars_std()`. This is defense-in-depth — even if the startup removal fails or is bypassed, exec handler children (including long-running ones like claude-pilot via self-dev) will not inherit the contaminated `GH_TOKEN`.

This is safe because:
- `run_gh` explicitly sets `GH_TOKEN` AFTER calling `scrub_mika_env_vars()` — wait, no, it sets it BEFORE. But `scrub_mika_env_vars` only calls `cmd.env_remove()` for `MIKA_*` vars, so `GH_TOKEN` survives. If we add `GH_TOKEN` to the scrub, it would be removed.
- **Fix:** Change `run_gh` ordering to set `GH_TOKEN` AFTER `scrub_mika_env_vars()`. Then add `GH_TOKEN` to the scrub list. This way: scrub removes inherited `GH_TOKEN` → explicit `.env("GH_TOKEN", token)` re-adds the correct platform token.

**Updated `run_gh` flow:**
```
scrub_mika_env_vars(&mut cmd);  // removes MIKA_* AND GH_TOKEN
cmd.env("GH_TOKEN", token);     // re-adds with correct platform token
```

### 3. Verify and update documentation

**Files to verify (already correct per research):**
- `docs/configuration.md` — identity matrix ✅
- `.env.example` — warning against `GH_TOKEN` ✅
- `CLAUDE.md` — documents `GH_TOKEN` is not `MIKA_*`-prefixed ✅

**Update `CLAUDE.md`** env vars section: add note that `GH_TOKEN` is actively scrubbed from the process env if detected in `~/.mika/.env`.

## Acceptance Criteria

- [x] `check_env_warnings(home_dir)` detects `GH_TOKEN` in `.env` file and removes it from process env
- [x] Warning log emitted when `GH_TOKEN` found in `.env`
- [x] `scrub_mika_env_vars()` removes `GH_TOKEN` alongside `MIKA_*` vars (defense-in-depth)
- [x] `run_gh` sets `GH_TOKEN` AFTER `scrub_mika_env_vars()` (ordering fix)
- [x] All call sites of `check_deprecated_env_vars()` updated to pass `home_dir`
- [x] Existing tests pass (`cargo test`)
- [x] New unit test for `GH_TOKEN` detection in `.env` file parsing

## Out of Scope

- Per-agent `.env` files (`~/.mika/agents/<name>/.env`) — lower risk, follow-up if needed
- `mika doctor` integration — natural follow-up
- Renaming `scrub_mika_env_vars()` function — cosmetic, not worth the churn

## Sources

- Issue: #380
- Solution doc: `docs/solutions/integration-issues/run-gh-github-token-injection.md`
- Solution doc: `docs/solutions/architecture-patterns/dedicated-github-token-agent-operations.md`
- Solution doc: `docs/solutions/security-issues/env-var-leakage-exec-handler-child-processes.md`
