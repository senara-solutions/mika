---
title: Configuration System Simplification — 6-Layer to 4-Source Cascade with dotenvy Secrets Management
date: 2026-03-10
status: documented
category: architecture-patterns
tags: [configuration, secrets-management, dotenv, config-simplification, multi-agent]
modules:
  - mika-common (dotenv module, config.rs)
  - mika-cli (setup command, main.rs)
  - mika-agent (server wiring)
  - mika-gateway (CWD-based dotenv)
severity: medium
symptoms:
  - Complex 6-layer config cascade with unclear precedence
  - No file-based secrets management (API keys only via shell env vars)
  - Redundant bundled defaults in config/default.toml duplicating serde defaults
  - Poor onboarding UX — no guided API key setup
---

# Simplified Config: 4-Source Model with dotenvy

## Problem

Mika's configuration had a 6-layer cascade that was hard to reason about:

1. `config/default.toml` (bundled) — **100% redundant** with serde `#[serde(default)]` functions
2. `config/local.toml` (dev) — rarely used, shell env vars suffice
3. `~/.mika/config.toml` (global)
4. `~/.mika/agents/X/config.toml` (per-agent)
5. `~/.mika/.env` (proposed but didn't exist)
6. `MIKA_*` env vars (shell)

Secrets had no proper home besides shell profiles (visible in history) or manual env var
setup. The `mika setup` command prompted for an API key but had nowhere to store it.

## Root Cause

Two independent design gaps:

1. **Default duplication:** `config/default.toml` contained values identical to the serde
   default functions in `config.rs`, creating a maintenance burden with no benefit.
2. **Missing secrets layer:** No file-based secrets management meant API keys lived only
   in shell env vars or were typed repeatedly.

## Solution: 4-Source Model

Replaced the 6-layer cascade with a clean 4-source model:

| Priority | Source | Purpose |
|----------|--------|---------|
| 1 (lowest) | Rust `#[serde(default)]` | Compiled-in defaults |
| 2 | TOML config files | `~/.mika/config.toml` + optional per-agent |
| 3 | `~/.mika/.env` | Secrets (API keys, tokens) via dotenvy |
| 4 (highest) | `MIKA_*` env vars | Shell overrides, always win |

**Key invariant:** Shell env vars always win. `.env` never overrides them (dotenvy's
`from_path()` behavior). This prevents accidents and supports deployment overrides.

### New `dotenv.rs` Module

Three public functions in `crates/mika-common/src/dotenv.rs`:

```rust
/// Load `.env` into process environment (does NOT override existing vars).
pub fn load_dotenv(home_dir: &Path) { ... }

/// Read a single key without loading into process env.
/// Uses dotenvy::from_path_iter for parser consistency.
pub fn get_env_var(home_dir: &Path, key: &str) -> Option<String> { ... }

/// Write/update a key atomically. Creates file if missing. 0600 perms on Unix.
pub fn set_env_var(home_dir: &Path, key: &str, value: &str) -> Result<()> { ... }
```

`set_env_var` uses atomic writes (temp file + chmod + rename) to prevent TOCTOU races
and ensure secrets are never world-readable.

### Simplified Config Loader

Removed two lines from `Settings::load_for_agent()`:

```rust
// REMOVED:
.add_source(File::with_name("config/default").required(false))
.add_source(File::with_name("config/local").required(false))
```

The builder now starts directly with the global TOML config.

### Entry Point Wiring

`load_dotenv()` is called **before** `Settings::load()` in all entry points:

- **CLI** (`mika-cli/src/main.rs`): Both team-mode and normal paths
- **Server** (`mika-agent/src/bin/mika-server.rs`): After home_dir resolution
- **Gateway** (`mika-gateway/src/main.rs`): CWD-based `dotenvy::dotenv()` (no home dir)

### Setup Command

`mika setup` now writes the API key to `~/.mika/.env`:

```rust
if std::env::var("MIKA_ANTHROPIC_API_KEY").ok().filter(|v| !v.is_empty()).is_none()
    && mika_common::dotenv::get_env_var(&home_dir, "MIKA_ANTHROPIC_API_KEY").is_none()
{
    // Prompt and write to .env
    mika_common::dotenv::set_env_var(&home_dir, "MIKA_ANTHROPIC_API_KEY", key)?;
}
```

## Issues Found During Code Review

### 1. Parser Divergence (`get_env_var`)

**Problem:** Initial implementation used a custom line-by-line parser that didn't handle
quoted values, `export` prefix, or inline comments — diverging from dotenvy's behavior.

**Fix:** Replaced with `dotenvy::from_path_iter()` wrapper (5 lines vs 20):
```rust
pub fn get_env_var(home_dir: &Path, key: &str) -> Option<String> {
    let env_path = home_dir.join(".env");
    dotenvy::from_path_iter(&env_path).ok()?.find_map(|r| {
        let (k, v) = r.ok()?;
        (k == key).then_some(v)
    })
}
```

**Lesson:** Never hand-roll parsers for well-specified formats when a maintained crate
offers iterators. Check for low-level APIs before writing custom code.

### 2. Temp File Naming Bug

**Problem:** `env_path.with_extension("env.tmp")` on `.env` produces `.env.env.tmp`
because `with_extension` replaces everything after the last `.` in the stem.

**Fix:** Use `with_file_name(".env.tmp")` — sets sibling filename in same directory.

**Lesson:** `Path::with_extension` is a footgun on dotfiles. Prefer `with_file_name` for
sibling temp files. Always write a test asserting the exact output path.

### 3. Wrong Return Type

**Problem:** `set_env_var` returned `std::io::Result` instead of `anyhow::Result`,
violating the project convention and losing path context in errors.

**Fix:** Changed to `anyhow::Result` with `.with_context()` on all I/O operations.

**Lesson:** Enforce return type conventions via `clippy.toml` `disallowed-types` or
code review checklist.

## Prevention Strategies

1. **Parser divergence:** Establish a project rule — never hand-roll parsers for standard
   formats. Document the authoritative crate per format in CLAUDE.md.

2. **Path construction bugs:** Treat `Path::with_extension` as hazardous on dotfiles.
   Always test exact output paths. Consider a `sibling_path()` helper.

3. **Redundant defaults:** Write a test that deserializes empty TOML into Settings and
   asserts all fields match expected defaults. If it passes, `default.toml` is redundant.

4. **Atomic file writes:** Always write temp → chmod → rename. Set permissions before
   rename to prevent race window. Temp file must be in the same directory (same filesystem).

5. **Secret echo suppression:** Use `rpassword::read_password()` for interactive secret
   input (todo #604). Never print back secrets — confirm with derived signals
   (e.g., "API key set (starts with `sk-ant-...`)").

## Testing Checklist for Config/Dotenv Code

- Round-trip: write keys → read back → assert equality (including quotes, spaces, Unicode)
- Layer precedence: set same key at every layer → assert highest priority wins
- Missing file graceful degradation: no panic, defaults used
- Temp file path: assert exact constructed path (especially for dotfiles)
- Secret redaction: Debug output and stdout contain no raw keys
- Malformed input: garbage `.env` returns Err, not panic
- File permissions: assert 0600 on Unix after write
- Idempotency: set same key twice → exactly one occurrence in file

## Files Changed

| File | Change |
|------|--------|
| `crates/mika-common/src/dotenv.rs` | **NEW** — load/read/write `.env` |
| `crates/mika-common/src/config.rs` | Removed 2 source lines, updated cascade docs |
| `crates/mika-common/src/lib.rs` | Added `pub mod dotenv` |
| `crates/mika-common/Cargo.toml` | Added dotenvy dependency |
| `crates/mika-cli/src/main.rs` | Wired `load_dotenv()` in both paths |
| `crates/mika-cli/src/commands/setup.rs` | API key prompt writes to `.env` |
| `crates/mika-agent/src/bin/mika-server.rs` | Wired `load_dotenv()` |
| `crates/mika-gateway/src/main.rs` | CWD-based `dotenvy::dotenv()` |
| `Dockerfile.agent` | Removed `config/default.toml` COPY |
| `config/default.toml` | **DELETED** |
| `.gitignore`, `.dockerignore` | Removed `config/local.*` entries |
| `docs/configuration.md` | Updated cascade table, added `.env` section |
| `CLAUDE.md` | Updated Stack, removed `config/` from directory structure |

## Related Documentation

- [Configuration Reference](../../configuration.md) — full settings with 4-source cascade
- [Architecture Overview](../../architecture.md) — crate responsibilities
- [Env Var Leakage Prevention](../security-issues/env-var-leakage-exec-handler-child-processes.md) — defense-in-depth for MIKA_* vars
- [OTLP Endpoint Configuration](../integration-issues/otlp-endpoint-path-requirement.md) — telemetry env vars
- Todo #604: API key echo suppression (pending, P3)

## Verification

- **1,084 tests pass** (786 agent + 143 common + 103 CLI + 52 gateway)
- **8 new dotenv tests** covering all operations and edge cases
- **6 existing config tests** continue to pass unchanged
- **Clippy clean**, **cargo fmt clean**
- `cargo build -p mika-agent` succeeds (build.rs picks up doc changes)
