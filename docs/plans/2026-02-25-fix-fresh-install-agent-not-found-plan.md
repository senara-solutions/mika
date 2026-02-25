---
title: "Fix: Fresh Install Agent Not Found Error"
type: fix
status: completed
date: 2026-02-25
---

# Fix: Fresh Install Agent Not Found Error

## Overview

On first run (`mika`), after `setup` bootstraps the default agent, the chat command immediately fails with:
```
✦ Mika initialized at /home/user/.mika/agents/main
Error: Agent 'main' not found. Create it with `mika agents create main`.
```

## Problem Statement

`bootstrap()` creates directories + config files but does NOT create `mika.db`. Three functions define "agent exists" as "has `data/mika.db`", but `mika.db` is only created lazily by `Database::open()`. This creates a chicken-and-egg problem: the init check demands the DB exists before it will open it.

**Flow on fresh install:**
1. `bootstrap_fresh_install()` creates `~/.mika/agents/main/data/`, `config.toml`, `soul.md`, etc. (NO `mika.db`)
2. `setup::run()` prints "Mika initialized" and returns
3. `chat::run()` calls `init_base_for_agent("main")`
4. `ensure_initialized_for_agent()` checks for `data/mika.db` → doesn't exist → **BAIL**
5. `Database::open()` (which would CREATE `mika.db`) never runs

## Proposed Solution

Change the definition of "agent exists" from "has `data/mika.db`" to "has been bootstrapped" (has `config.toml`). This aligns with what `bootstrap()` actually creates.

### Files to change:

1. **`crates/mika-common/src/agent.rs`**
   - `agent_exists()` (line 44): Check for `config.toml` instead of `data/mika.db`
   - `list_agents()` (line 64): Filter by `config.toml` instead of `data/mika.db`

2. **`crates/mika-cli/src/init.rs`**
   - `ensure_initialized_for_agent()` (line 102): Check for `config.toml` instead of `data/mika.db`

3. **Update tests** in both files to match the new existence check.

## Acceptance Criteria

- [x] `mika` works on a completely fresh install (no `~/.mika/` directory)
- [x] `mika` works after `mika setup` (bootstrapped but never chatted)
- [x] `mika agents list` shows bootstrapped agents even before first chat
- [x] Existing installations with `mika.db` continue to work
- [x] All tests pass
