---
title: "feat: Add mika doctor and config set/get/list commands"
type: feat
status: active
date: 2026-03-11
issues: ["#62", "#63"]
parent: "#60"
---

# feat: Add mika doctor and config set/get/list commands

## Overview

Two related CLI commands for onboarding and setup improvements:
1. **`mika doctor`** — diagnostic command that validates installation health (home dir, API keys, DB, config, dependencies)
2. **`mika config set/get/list`** — extend the existing config command with read/write operations across all config backends

Both are part of the onboarding improvements umbrella (#60).

## Problem Statement

Users have no way to diagnose installation issues. When `mika` fails to start, the error messages are scattered and context-dependent. A `doctor` command provides a single entry point for installation validation.

The existing `mika config` command is read-only and limited. Users must manually edit `config.toml`, `.env`, or use the TUI `/config set` to change settings. A CLI `config set/get/list` brings parity.

## Proposed Solution

### Part 1: `mika doctor`

New command at `crates/mika-cli/src/commands/doctor.rs` with shared validation helpers in `crates/mika-common/src/validation.rs`.

**Flags:**
- `--verify-api` — make a live Claude API call (`max_tokens: 1`) to verify credentials
- `--json` — output machine-readable JSON instead of colored text

**Checks (in order):**

| # | Check | Level | Critical? |
|---|-------|-------|-----------|
| 1 | Home directory exists with correct permissions (0700) | Dir | Yes |
| 2 | Agent directory exists with required files (identity.toml, soul.md) | Dir | Yes |
| 3 | API key set and format valid (sk-ant-* or sk-ant-oat*) + source (.env vs env var) | Auth | Yes |
| 4 | Database opens and schema version matches current (8) | DB | Yes |
| 5 | `.env` file permissions (0600 if exists) | Security | No (warn) |
| 6 | `config.toml` parses successfully | Config | Yes |
| 7 | OpenAI API key (optional embedding support) | Optional | No (warn) |
| 8 | Brave API key (optional web search) | Optional | No (warn) |
| 9 | `jq` binary in PATH (required by skill handlers) | Deps | No (warn) |
| 10 | MCP servers configured (count from mcp.json) | Info | No (info) |
| 11 | Skills installed (count from skills dir) | Info | No (info) |

**Key design decision:** `mika doctor` must work **without full initialization**. It cannot use `init_db_only_for_agent()` because that triggers migrations and bootstrapping. Instead, it manually resolves paths and checks each component independently. Each check catches its own errors and reports them — the command never panics or bails early.

**Output format (text):**
```
Mika Doctor — checking installation health...

  [OK]   Home directory (~/.mika/) — permissions 0700
  [OK]   Agent directory (mika) — identity.toml, soul.md present
  [OK]   API key — API key format valid (source: .env)
  [OK]   Database — schema v8, opens successfully
  [WARN] .env permissions — 0644 (expected 0600)
  [OK]   config.toml — parses successfully
  [WARN] OpenAI API key — not set (vector search disabled)
  [OK]   Brave API key — set (source: env var)
  [OK]   jq — found at /usr/bin/jq
  [OK]   MCP servers — 2 configured
  [OK]   Skills — 8 installed (5 built-in, 3 marketplace)

Summary: 8 OK, 2 warnings, 0 failures
```

**JSON schema (`--json`):**
```json
{
  "checks": [
    {
      "name": "home_directory",
      "status": "ok",
      "message": "permissions 0700",
      "details": "/home/user/.mika"
    }
  ],
  "summary": { "ok": 8, "warn": 2, "fail": 0 }
}
```

**Exit codes:** 0 if no critical failures, 1 if any critical check fails.

### Part 2: `mika config set/get/list`

Extend `ConfigCommand` enum with three new variants. Introduce `ConfigKeyRegistry` to map keys to their storage backends.

**ConfigKeyRegistry** (in `crates/mika-common/src/config.rs`):

```rust
pub enum ConfigBackend {
    File,      // config.toml via toml_edit
    Env,       // ~/.mika/.env via dotenv::set_env_var
    Database,  // customer_config table
    ReadOnly,  // computed at runtime, not writable
}

pub struct ConfigKeyInfo {
    pub key: &'static str,
    pub backend: ConfigBackend,
    pub env_var: Option<&'static str>,  // MIKA_* override name
    pub secret: bool,                    // redact in output, use Password prompt
    pub description: &'static str,
    pub default_display: Option<&'static str>,
}
```

**Key mapping:**

| Key | Backend | Env Override | Secret | Description |
|-----|---------|-------------|--------|-------------|
| `claude_model` | File | `MIKA_CLAUDE_MODEL` | No | Claude model ID |
| `claude_max_tokens` | File | `MIKA_CLAUDE_MAX_TOKENS` | No | Max response tokens |
| `log_level` | File | `MIKA_LOG_LEVEL` | No | Log level (trace/debug/info/warn/error) |
| `server_port` | File | `MIKA_SERVER_PORT` | No | HTTP server port |
| `embedding_model` | File | `MIKA_EMBEDDING_MODEL` | No | OpenAI embedding model |
| `embedding_dimensions` | File | `MIKA_EMBEDDING_DIMENSIONS` | No | Embedding vector dimensions |
| `anthropic_api_key` | Env | `MIKA_ANTHROPIC_API_KEY` | Yes | Anthropic API key |
| `openai_api_key` | Env | `MIKA_OPENAI_API_KEY` | Yes | OpenAI API key |
| `brave_api_key` | Env | `MIKA_BRAVE_API_KEY` | Yes | Brave Search API key |
| `internal_token` | Env | `MIKA_INTERNAL_TOKEN` | Yes | Server auth token |
| `timezone` | Database | — | No | User timezone |
| `thinking_level` | Database | — | No | Claude thinking level |
| `home_dir` | ReadOnly | `MIKA_HOME` | No | Mika home directory |
| `db_path` | ReadOnly | — | No | Database file path |

**Subcommands:**

- **`mika config get <key>`** — Show effective value (after cascade resolution). Secrets are redacted unless `--verbose`. With `--verbose`, show source layer.
- **`mika config set <key> [value]`** — Write to the key's backend. Secret keys ignore CLI value arg and prompt via `dialoguer::Password`. File keys use `toml_edit` for comment-preserving writes. DB keys use existing `set_customer_config`. Validates before writing. Warns if a higher-priority env var overrides the written value. Writes to per-agent config.toml (not global) since `--agent` selects context.
- **`mika config list`** — Show all keys with effective values. Secrets redacted. With `--verbose`, show source per key.

**Backward compatibility:** `mika config` (no subcommand) retains existing summary behavior. `Edit` and `Soul` subcommands are preserved.

**Non-TTY handling:** Secret keys require a terminal. Non-TTY context bails with: "Secret keys require an interactive terminal. Set MIKA_ANTHROPIC_API_KEY as an environment variable instead."

**Validation per file key:**
- `claude_model`: non-empty string
- `claude_max_tokens`: positive integer (1–131072)
- `log_level`: one of trace, debug, info, warn, error
- `server_port`: integer (1–65535)
- `embedding_model`: non-empty string
- `embedding_dimensions`: positive integer (1–4096)

DB key validation reuses existing `config_keys::validate_config_value()`.

## Technical Approach

### Implementation Phases

#### Phase 1: Shared Validation Module + ConfigKeyRegistry

**Files to create/modify:**

1. **`crates/mika-common/src/validation.rs`** (new) — shared validation helpers:
   - `validate_api_key_format(key: &str) -> Result<ApiKeyFormat>` — returns `ApiKey` or `OAuthToken` or error
   - `validate_api_key_live(key: &str) -> Result<()>` — minimal Claude API call (uses `ClaudeClient` with `max_tokens: 1`)
   - `check_home_permissions(path: &Path) -> Result<PermissionStatus>` — checks existence + mode
   - `check_binary_in_path(name: &str) -> Option<PathBuf>` — `which`-style lookup via `PATH`
   - `validate_file_key(key: &str, value: &str) -> Result<()>` — per-key validation for File backend keys

2. **`crates/mika-common/src/config.rs`** (modify) — add `ConfigKeyRegistry`:
   - `ConfigBackend` enum
   - `ConfigKeyInfo` struct
   - `ConfigKeyRegistry` with static `KEYS` array and lookup methods
   - `get_effective_value(key, settings, agent_config)` — resolves value through cascade
   - `get_value_source(key, home_dir, agent_home)` — identifies which layer provides the value

3. **`crates/mika-common/Cargo.toml`** (modify) — add `toml_edit` dependency

4. **`crates/mika-common/src/lib.rs`** (modify) — add `pub mod validation;`

**Tests:** Unit tests for each validation function, ConfigKeyRegistry lookup, value resolution.

#### Phase 2: `mika doctor` Command

**Files to create/modify:**

1. **`crates/mika-cli/src/commands/doctor.rs`** (new):
   - `DoctorArgs` struct with `--verify-api` and `--json` flags
   - `CheckResult` struct: `{ name, status: Ok|Warn|Fail, message, details }`
   - `run(args, agent_name)` function that:
     - Resolves `home_dir` and `agent_home` without `init_db_only_for_agent`
     - Runs each check independently, collecting `Vec<CheckResult>`
     - Formats output (colored text or JSON)
     - Returns exit code based on critical failures
   - Individual check functions: `check_home_dir()`, `check_agent_dir()`, `check_api_key()`, `check_database()`, `check_env_permissions()`, `check_config_toml()`, `check_optional_key()`, `check_jq()`, `check_mcp()`, `check_skills()`

2. **`crates/mika-cli/src/commands/mod.rs`** (modify) — add `pub mod doctor;`

3. **`crates/mika-cli/src/cli.rs`** (modify):
   - Add `Doctor(DoctorArgs)` variant to `Commands` enum
   - `DoctorArgs`: `--verify-api: bool`, `--json: bool`

4. **`crates/mika-cli/src/main.rs`** (modify) — add dispatch arm for `Commands::Doctor`

**Tests:** Unit tests for each check function using tempdir fixtures. Integration test for JSON output format.

#### Phase 3: `mika config set/get/list`

**Files to modify:**

1. **`crates/mika-cli/src/cli.rs`** (modify):
   - Add `Get`, `Set`, `List` variants to `ConfigCommand`:
     ```rust
     Get {
         key: String,
         #[arg(long)]
         verbose: bool,
     },
     Set {
         key: String,
         value: Option<String>,  // None for secrets (prompted)
     },
     List {
         #[arg(long)]
         verbose: bool,
     },
     ```

2. **`crates/mika-cli/src/commands/config.rs`** (modify) — extend `run()`:
   - `handle_get(key, verbose, ctx)` — lookup in registry, resolve effective value, display
   - `handle_set(key, value, ctx)` — lookup backend, validate, route to correct writer:
     - File → `toml_edit` read-modify-write with atomic rename
     - Env → `dotenv::set_env_var()` with `dialoguer::Password` prompt
     - Database → `async_db.set_customer_config()`
     - ReadOnly → bail with error
   - `handle_list(verbose, ctx)` — iterate all registry keys, resolve values, format table

3. **`crates/mika-cli/src/commands/config.rs`** — add `write_config_toml(path, key, value)` helper using `toml_edit`:
   - Parse existing file content as `toml_edit::DocumentMut`
   - Set/update the key
   - Atomic write (temp + rename)

**Tests:** Unit tests for get/set/list with tempdir + temp DB. Test secret redaction. Test validation rejection. Test env var override warning.

### Dependency: `toml_edit`

Add to `crates/mika-common/Cargo.toml`:
```toml
toml_edit = "0.22"
```

This is the standard crate for comment-preserving TOML manipulation. Already used widely in the Rust ecosystem (cargo itself uses it).

## System-Wide Impact

- **No DB schema changes** — uses existing `customer_config` table
- **No API changes** — CLI-only features
- **Config cascade unchanged** — `Settings::load_for_agent()` is read-only; new `config set` writes to individual backends
- **TUI `/config` handler unchanged** — continues to use `config_keys.rs` for DB keys; the new `ConfigKeyRegistry` is a superset that adds File/Env/ReadOnly keys
- **Existing `mika config` behavior preserved** — bare command still shows summary; `edit` and `soul` subcommands unchanged

## Acceptance Criteria

### mika doctor (#62)

- [ ] `mika doctor` runs all checks and shows colored [OK]/[WARN]/[FAIL] output
- [ ] `mika doctor --json` outputs valid JSON matching defined schema
- [ ] `mika doctor --verify-api` makes a live API call and reports result
- [ ] Works on uninitialized installation (reports failures, doesn't crash)
- [ ] Exit code 0 when all critical checks pass, 1 when any critical fails
- [ ] Respects `--agent <name>` flag for agent-specific checks
- [ ] Shared validation module in `mika-common/src/validation.rs`

### mika config set/get/list (#63)

- [ ] `mika config get <key>` shows effective value for any registered key
- [ ] `mika config set <key> <value>` writes to correct backend (File/Env/DB)
- [ ] `mika config set <secret_key>` prompts via dialoguer::Password (never accepts CLI arg)
- [ ] `mika config set <secret_key>` fails gracefully in non-TTY with actionable message
- [ ] `mika config list` shows all keys with values (secrets redacted)
- [ ] `--verbose` flag shows value source on get/list
- [ ] File writes use `toml_edit` (preserves comments)
- [ ] Secret writes use `dotenv::set_env_var` (atomic, 0600 perms)
- [ ] Validates values before writing; rejects invalid input with clear error
- [ ] Warns when env var override makes the written value ineffective
- [ ] Existing `mika config` (no subcommand), `edit`, `soul` behavior unchanged
- [ ] ReadOnly keys (`home_dir`, `db_path`) reject `set` with clear error

### Shared

- [ ] All new code has inline `#[cfg(test)] mod tests`
- [ ] `cargo test` passes
- [ ] `cargo clippy` clean
- [ ] `cargo fmt` clean

## Sources & References

### Internal References

- CLI arg definitions: `crates/mika-cli/src/cli.rs:268-280`
- Current config command: `crates/mika-cli/src/commands/config.rs:1-59`
- Command dispatch: `crates/mika-cli/src/main.rs:129-172`
- Init helpers: `crates/mika-cli/src/init.rs:29-93`
- Settings struct: `crates/mika-common/src/config.rs:6-90`
- Config cascade: `crates/mika-common/src/config.rs:136-199`
- API key auth: `crates/mika-common/src/claude.rs:11-49`
- Home dir helpers: `crates/mika-common/src/home.rs:6-77`
- DB config keys: `crates/mika-agent/src/config_keys.rs:7-54`
- Dotenv module: `crates/mika-common/src/dotenv.rs:1-87`
- Async DB config ops: `crates/mika-agent/src/async_db.rs:814-827`
- Setup command (dialoguer patterns): `crates/mika-cli/src/commands/setup.rs:361-433`
- TUI config handler: `crates/mika-cli/src/tui/commands/handlers.rs:313-376`

### Institutional Learnings

- `docs/solutions/security-issues/setup-wizard-secret-handling.md` — TTY guards, atomic writes, dialoguer::Password pattern
- `docs/solutions/architecture-patterns/simplified-config-4-source-model.md` — 4-source config cascade, dotenv module functions
- `docs/solutions/architecture-patterns/delegation-work-item-guard-enforcement.md` — three-layer defense philosophy (code guard > prompt)

### Related Issues

- #60 — Onboarding improvements (parent)
- #62 — mika doctor
- #63 — mika config set/get/list
