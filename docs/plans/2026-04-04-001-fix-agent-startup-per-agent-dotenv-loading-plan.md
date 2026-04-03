---
title: "fix: agent startup must load per-agent .env before constructing Settings"
type: fix
status: completed
date: 2026-04-04
---

# fix: agent startup must load per-agent .env before constructing Settings

## Overview

Per-agent `.env` files (`~/.mika/agents/<name>/.env`) are never loaded in CLI agent mode or server mode. Only `token` and `credential-helper` subcommands (fixed in PR #428) load per-agent `.env`. Since PR #426 moved GitHub App credentials to per-agent `.env`, tools using `Settings::agent_github_token()` fall back to the host's `gh auth` instead of the agent's App token. This caused mika-qa's PR review on #429 to appear as `samidarko` instead of `mika-qa-bot[bot]`.

## Root Cause

Two startup paths skip per-agent `.env`:

1. **CLI agent mode** (`crates/mika-cli/src/main.rs:149-154`): Loads global `.env` at line 152, never loads per-agent `.env`. The `agent_home` is resolved later at line 158.
2. **Server mode** (`crates/mika-agent/src/bin/mika-server.rs:6`): Loads global `.env` once. `init_agent()` in `server/mod.rs:288` calls `Settings::load_for_agent()` which reads process env vars — but per-agent `.env` was never loaded into them.

## Fix

Two distinct fixes for two different execution models:

### 1. CLI mode (single agent per process)

Move dotenv loading after `agent_home` resolution. Load per-agent `.env` first (dotenvy won't override), then global:

#### `crates/mika-cli/src/main.rs` (lines 149-154)

```rust
// Before (broken):
let global_home = home::resolve_home_dir().ok();
if let Some(ref h) = global_home {
    mika_common::dotenv::load_dotenv(h);
    mika_common::dotenv::check_env_warnings(h);
}
// ... agent_home resolved later at line 158 ...

// After (fixed):
let global_home = home::resolve_home_dir().ok();
let agent_home = global_home
    .as_ref()
    .map(|h| home::resolve_agent_home(h, &agent_name));
// Load per-agent .env first (dotenvy won't override), then global as fallback
if let Some(ref ah) = agent_home {
    mika_common::dotenv::load_dotenv(ah);
}
if let Some(ref h) = global_home {
    mika_common::dotenv::load_dotenv(h);
    mika_common::dotenv::check_env_warnings(h);
}
```

Remove the duplicate `agent_home` resolution at the old line 158-160.

### 2. Server mode (multiple agents per process)

Process env vars are global — can't use `load_dotenv` per agent (first agent wins, others ignored). Instead, parse per-agent `.env` without mutating env, and inject as a config-rs source.

#### `crates/mika-common/src/dotenv.rs` — new helper

```rust
/// Parse a `.env` file and return key-value pairs without modifying process env.
/// Keys are returned as-is (with MIKA_ prefix intact) for config-rs Environment source compat.
pub fn parse_dotenv(home_dir: &Path) -> HashMap<String, String> {
    let env_path = home_dir.join(".env");
    let mut map = HashMap::new();
    match dotenvy::from_path_iter(&env_path) {
        Ok(iter) => {
            for item in iter.flatten() {
                map.insert(item.0, item.1);
            }
        }
        Err(_) => {} // file not found or parse error — same as load_dotenv
    }
    map
}
```

#### `crates/mika-common/src/config.rs` — `Settings::load_for_agent`

Add per-agent dotenv as a config source between file sources and process env:

```rust
pub fn load_for_agent(global_home: &Path, agent_home: &Path) -> anyhow::Result<Self> {
    let global_config = global_home.join("config.toml");
    let agent_config = agent_home.join("config.toml");

    let mut builder = Config::builder().add_source(File::from(global_config).required(false));

    if global_home != agent_home {
        builder = builder.add_source(File::from(agent_config).required(false));
    }

    // Per-agent .env: parse without mutating process env, inject as config source.
    // Priority: config files < per-agent .env < process env vars
    if global_home != agent_home {
        let dotenv_vars = crate::dotenv::parse_dotenv(agent_home);
        for (key, value) in dotenv_vars {
            if let Some(config_key) = key.strip_prefix("MIKA_") {
                builder = builder.set_override(config_key.to_lowercase(), value)?;
            }
        }
    }

    // Process env vars (highest priority for CLI; for server, per-agent
    // .env overrides via set_override above take precedence — this is correct
    // because server-mode process env has only global vars, not per-agent ones)
    let settings: Settings = builder
        .add_source(Environment::with_prefix("MIKA")...)
        .build()?
        .try_deserialize()?;
    // ...
}
```

**Important:** `set_override` has highest priority in config-rs (above `add_source`). This is correct for server mode because process env vars only contain global values — per-agent `.env` should override them. For CLI mode, per-agent `.env` is already loaded into process env (via `load_dotenv`), so `set_override` from parsing the same file is a no-op (same values).

Wait — `set_override` being highest priority means per-agent `.env` would override shell-set env vars in server mode. That's wrong. Shell env vars should always win.

**Revised approach:** Instead of `set_override`, add as a custom config source between files and env:

```rust
// Per-agent .env as config source (below process env in priority)
if global_home != agent_home {
    let dotenv_vars = crate::dotenv::parse_dotenv(agent_home);
    let dotenv_map: HashMap<String, config::Value> = dotenv_vars
        .into_iter()
        .filter_map(|(k, v)| {
            k.strip_prefix("MIKA_")
                .map(|stripped| (stripped.to_lowercase(), config::Value::new(None, v)))
        })
        .collect();
    builder = builder.add_source(dotenv_map);
}
```

This inserts per-agent `.env` values as a source between config files and process env — process env always wins.

### Server init_agent: use per-agent settings for github_token

Currently `run_server()` passes `settings.agent_github_token()` (global) to all agents. After the fix, `init_agent` should use its own `agent_settings.agent_github_token()`:

#### `crates/mika-agent/src/server/mod.rs`

Remove `github_token` parameter from `init_agent` — derive it from `agent_settings` inside:

```rust
async fn init_agent(
    agent_name: &str,
    agent_home: &Path,
    global_home: &Path,
    // ... remove github_token param ...
) -> Result<AgentState> {
    let agent_settings = Settings::load_for_agent(global_home, agent_home)?;
    let github_token = agent_settings.agent_github_token().map(String::from);
    // ...
}
```

## Acceptance Criteria

- [x] `mika chat` / `mika ask` loads per-agent `.env` before global `.env`
- [x] `mika-server` agents each get their own `.env` values via Settings
- [x] Per-agent GitHub App credentials resolve correctly in server mode
- [x] Shell env vars still override `.env` values
- [x] `GH_TOKEN` scrubbing (`check_env_warnings`) still runs
- [x] Multi-agent server: each agent gets its OWN per-agent `.env` values
- [x] Legacy single-agent layout still works (no per-agent `.env` = no-op)
- [x] `cargo test` passes
- [x] `cargo clippy` clean

## Sources

- Issue: #430
- Prior fix (token/credential-helper only): PR #428, plan `2026-04-03-007`
- Per-agent `.env` introduced: PR #426
- Config cascade: `docs/solutions/architecture-patterns/simplified-config-4-source-model.md`
- GH_TOKEN defense: `docs/solutions/security-issues/gh-token-identity-collision-dotenv-leak.md`
- dotenv module: `crates/mika-common/src/dotenv.rs`
- CLI entry: `crates/mika-cli/src/main.rs:149-154`
- Server entry: `crates/mika-agent/src/bin/mika-server.rs:6`
- Agent init: `crates/mika-agent/src/server/mod.rs:288`
- Settings loader: `crates/mika-common/src/config.rs:955`
