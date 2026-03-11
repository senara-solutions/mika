---
title: ConfigKeyRegistry — Unified CLI Config Management Across Backends
date: 2026-03-11
status: documented
category: architecture-patterns
tags: [configuration, cli, config-registry, multi-backend, validation]
modules:
  - mika-common (config.rs ConfigKeyRegistry, validation.rs)
  - mika-cli (commands/config.rs get/set/list, commands/doctor.rs)
severity: low
symptoms:
  - Users must manually edit config.toml, .env, or use TUI to change settings
  - No CLI parity for config read/write operations
  - No diagnostic command to validate installation health
---

# ConfigKeyRegistry — Unified CLI Config Management

## Problem

Mika's configuration spans four backends (config.toml files, .env secrets, SQLite database, runtime-computed), but users had no unified CLI interface to read or write config values. They had to know which file to edit for each setting. Additionally, there was no diagnostic command to validate installation health.

## Solution

### ConfigKeyRegistry Pattern

A static registry (`CONFIG_KEYS: &[ConfigKeyInfo]`) maps every known config key to its storage backend and metadata:

```rust
pub enum ConfigBackend { File, Env, Database, ReadOnly }

pub struct ConfigKeyInfo {
    pub key: &'static str,
    pub backend: ConfigBackend,
    pub env_var: Option<&'static str>,
    pub secret: bool,
    pub description: &'static str,
}
```

The `mika config set <key> [value]` command uses the registry to route writes to the correct backend:
- **File** → `toml_edit` for comment-preserving writes to `config.toml`
- **Env** → `dialoguer::Password` prompt, writes to `~/.mika/.env` (never accepts secrets as CLI args)
- **Database** → `async_db.set_customer_config()`
- **ReadOnly** → rejected with clear error

### Doctor Command Pattern

`mika doctor` validates installation health without full initialization. Key design decision: it **cannot** use `init_db_only_for_agent()` because that triggers migrations and bootstrapping. Instead, it manually resolves paths and checks each component independently. Each check catches its own errors — the command never panics or bails early.

## Key Decisions

1. **Registry is static, not dynamic** — All keys are known at compile time. No runtime registration needed since the config schema is fixed.

2. **Secret keys never accept CLI value args** — Even if provided, the value is ignored with a warning. Always prompts via `dialoguer::Password`. TTY guard bails with actionable message in non-interactive contexts.

3. **File writes use toml_edit, not toml::to_string_pretty** — Preserves user comments and formatting in config.toml.

4. **Doctor works on broken installations** — Uses `resolve_home_dir()` directly, checks each component independently, reports all failures rather than bailing on the first one.

5. **Shared validation module** — `crates/mika-common/src/validation.rs` contains validators reused by both doctor and config set commands (API key format, file key validation, path permissions, binary-in-PATH lookup).

## Files

- `crates/mika-common/src/config.rs` — `ConfigBackend`, `ConfigKeyInfo`, `CONFIG_KEYS`, `lookup_config_key()`, `get_effective_value()`
- `crates/mika-common/src/validation.rs` — `validate_api_key_format()`, `validate_file_key()`, `check_path_permissions()`, `check_binary_in_path()`
- `crates/mika-cli/src/commands/config.rs` — `run_get()`, `run_set()`, `run_list()`, `write_config_toml()`, `resolve_source()`
- `crates/mika-cli/src/commands/doctor.rs` — 11 independent health checks with colored/JSON output

## Gotchas

- `get_effective_value()` must have a branch for every File/ReadOnly key in `CONFIG_KEYS` — the coverage test in `config.rs` guards against forgetting one.
- Env var overrides are checked in `resolve_source()` before backend-specific checks — this matches the actual config cascade priority.
- API key validation must never include partial key content in error messages (security).
