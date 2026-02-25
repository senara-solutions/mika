---
title: "Fresh Install Agent Not Found Error"
date: 2026-02-25
category: logic-errors
severity: high
component: mika-common/agent, mika-cli/init
tags:
  - fresh-install
  - agent-lifecycle
  - initialization
  - chicken-and-egg
  - sentinel-mismatch
related_issues:
  - commit 0988fa8
---

# Fresh Install "Agent Not Found" Error

## Problem

On first run after bootstrapping, Mika fails with a false error:

```
✦ Mika initialized at /home/user/.mika/agents/main
Error: Agent 'main' not found. Create it with `mika agents create main`.
```

The agent directory exists with proper configuration files, yet the CLI refuses to recognize it.

## Root Cause

A sentinel file mismatch between what `bootstrap()` creates and what existence checks look for.

Three functions defined "agent exists" as "has `data/mika.db`":
- `agent_exists()` in `crates/mika-common/src/agent.rs`
- `list_agents()` in `crates/mika-common/src/agent.rs`
- `ensure_initialized_for_agent()` in `crates/mika-cli/src/init.rs`

But `bootstrap()` in `crates/mika-common/src/home.rs` creates directories and config files (`config.toml`, `soul.md`, etc.) — **not** `mika.db`. The database is created lazily by `Database::open()`, which runs *after* the existence check. This created a chicken-and-egg failure: the init check demanded the DB exist before it would open it.

**Failure chain:**
1. `bootstrap_fresh_install()` creates `~/.mika/agents/main/config.toml` (no `mika.db`)
2. `setup::run()` prints "Mika initialized" and returns
3. `chat::run()` calls `init_base_for_agent("main")`
4. `ensure_initialized_for_agent()` checks `data/mika.db` → doesn't exist → **BAIL**
5. `Database::open()` (which would CREATE `mika.db`) never runs

## Solution

Changed the agent existence sentinel from `data/mika.db` to `config.toml` — the file that `bootstrap()` actually creates.

### Before

```rust
// agent.rs
pub fn agent_exists(home_dir: &Path, name: &str) -> bool {
    agent_dir(home_dir, name).join("data").join("mika.db").exists()
}

// init.rs
if !agent_home.join("data").join("mika.db").exists() {
    anyhow::bail!("Agent '{agent_name}' not found...");
}
```

### After

```rust
// agent.rs
pub fn agent_exists(home_dir: &Path, name: &str) -> bool {
    agent_dir(home_dir, name).join("config.toml").exists()
}

// init.rs
if !agent_home.join("config.toml").exists() {
    anyhow::bail!("Agent '{agent_name}' not found...");
}
```

The same change was applied to the `list_agents()` filter. Legacy layout detection (`is_legacy_layout`, `is_initialized` for legacy path) correctly continues to check `data/mika.db` since pre-migration installs always have the DB at the root level.

### Files Changed

| File | Change |
|------|--------|
| `crates/mika-common/src/agent.rs` | `agent_exists()` + `list_agents()` sentinel |
| `crates/mika-cli/src/init.rs` | `ensure_initialized_for_agent()` check |
| `crates/mika-common/src/home.rs` | Stale comment update |
| `crates/mika-common/src/team.rs` | Test updates |

## Prevention

1. **Align existence checks with bootstrap outputs.** When a bootstrap function creates files A, B, C, the existence check should look for one of A, B, or C — not for file D that is created later by a different subsystem.

2. **Test full fresh-install flows.** Integration tests should cover: no home directory → setup → chat, verifying no errors occur in the gap between bootstrap and first DB access.

3. **Document lazy initialization assumptions.** If a resource (like `mika.db`) is created lazily, document that it should not be used as an existence sentinel.

4. **Centralize the existence definition.** The "agent exists" concept should be defined in one place (`agent_exists()`) and all callers should use it rather than implementing their own checks.

## Key Insight

"Exists" and "has been used" are different concepts. An agent exists as soon as it's bootstrapped (config files created). It has been used once its database is populated. Existence checks should use the former, not the latter.
