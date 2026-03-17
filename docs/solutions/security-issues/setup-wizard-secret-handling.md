---
title: "Rewrite mika setup wizard: masked input, proper TOML serialization, atomic writes, TTY guard"
date: 2026-03-10
status: documented
category: security-issues
tags:
  - cli
  - setup
  - secret-handling
  - dialoguer
  - toml-serialization
  - atomic-writes
  - tty-detection
  - config
  - dotenv
module: mika-cli
severity: high
resolution_time: "4-6 hours"
related_issues:
  - 604
  - 605
  - 606
  - 607
  - 608
---

# Setup Wizard Secret Handling and Config Safety

## Problem Symptom

The `mika setup` CLI command had multiple security and robustness issues:

1. **Plaintext secret echo** — API key prompt used raw `stdin().read_line()`, displaying secrets on screen during input (shoulder surfing, screen recording risk)
2. **Single-secret only** — Only prompted for Anthropic API key; Brave API key, telemetry credentials, and internal token required manual configuration
3. **TOML injection** — Line-based string interpolation for `config.toml` editing allowed user input containing quotes/newlines to inject arbitrary keys
4. **No TTY guard** — `dialoguer` prompts would fail opaquely in non-interactive contexts (CI/CD, subprocesses)
5. **Non-atomic writes** — `config.toml` used plain `fs::write`, risking corruption on interrupted writes; permissions reset to umask defaults
6. **Not re-runnable** — Early return when already initialized prevented adding missing configuration

## Root Cause

The original `setup.rs` was a minimal ~30-line implementation using raw `stdin().read_line()` and string interpolation for config file editing. It was written as a quick bootstrap without considering security, robustness, or extensibility. The `dialoguer` and `toml` crates were already in the dependency tree but weren't being used.

## Solution

### 1. Echo suppression for secrets

Replaced `stdin().read_line()` with `dialoguer::Password` for all secret inputs:

```rust
fn prompt_optional_secret(home_dir: &Path, env_key: &str, prompt: &str) -> Result<bool> {
    let value = Password::new()
        .with_prompt(prompt)
        .allow_empty_password(true)
        .interact()?;
    let value = value.trim();
    if value.is_empty() {
        return Ok(false);
    }
    mika_common::dotenv::set_env_var(home_dir, env_key, value)?;
    Ok(true)
}
```

### 2. Multi-secret wizard

Extended to prompt for: Anthropic API key, Brave Search API key (optional), telemetry configuration (Confirm + Input + Password for auth header), and auto-generated `MIKA_INTERNAL_TOKEN` (64-char hex via `rand::fill` + `hex::encode`).

### 3. Proper TOML serialization

Replaced line-based `set_config_toml_value` with structured `toml::Table` parse/serialize:

```rust
fn set_config_toml_value(home_dir: &Path, key: &str, value: toml::Value) -> Result<()> {
    let config_path = home_dir.join("config.toml");
    let mut table: toml::Table = match std::fs::read_to_string(&config_path) {
        Ok(content) => content.parse().unwrap_or_default(),
        Err(_) => toml::Table::new(),
    };
    table.insert(key.to_string(), value);
    let content = toml::to_string_pretty(&table)?;
    // atomic write follows...
}
```

The `toml` crate was already a dependency — zero cost to use it properly. This eliminates injection risks entirely.

### 4. TTY guard

Added `std::io::stdin().is_terminal()` check. Non-TTY + first run = bail with clear error. Non-TTY + already initialized = skip prompts (only auto-generate token if missing).

```rust
let interactive = std::io::stdin().is_terminal();
if !interactive && !already_initialized {
    bail!(
        "mika setup requires an interactive terminal for first-time configuration. \
         Pre-set MIKA_LLM_API_KEY and other env vars, or run `mika setup` in a terminal."
    );
}
```

### 5. Atomic writes with 0600 permissions

`config.toml` writes now use temp-file, chmod 0600, rename — matching the existing `.env` atomic write pattern in `dotenv::set_env_var`:

```rust
let tmp_path = config_path.with_file_name("config.toml.tmp");
std::fs::write(&tmp_path, &content)?;
#[cfg(unix)]
{
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&tmp_path, std::fs::Permissions::from_mode(0o600))?;
}
std::fs::rename(&tmp_path, &config_path)?;
```

### 6. Re-runnable wizard

Removed early return when already initialized. Each prompt checks `secret_is_set()` (env var OR .env file) or `config_key_is_set()` (TOML parse) before prompting. Already-configured values are silently skipped.

### 7. Helper extraction

`prompt_optional_secret(home_dir, env_key, prompt) -> Result<bool>` deduplicates 3 identical Password prompt blocks.

## Key Insights

- **Use what you already have.** Both `dialoguer` (for password prompts) and `toml` (for structured serialization) were already in the dependency tree. The original code used raw stdin and string manipulation when proper tools were one import away.

- **Secrets and config belong in different files.** Routing secrets to `~/.mika/.env` (loaded by dotenvy, 0600 permissions) and non-secret configuration to `~/.mika/config.toml` (structured TOML, atomic writes) creates a clean separation.

- **Atomic writes are cheap insurance.** The temp-file, chmod, rename pattern adds ~5 lines of code but eliminates an entire class of corruption bugs. When the project already has this pattern (in `dotenv::set_env_var`), consistency is free.

- **Idempotency enables trust.** Making `mika setup` re-runnable (skipping already-configured values) means users can safely run it after adding features that require new secrets. The per-value check-before-prompt pattern is the key enabler.

- **TTY detection is a gate, not a feature.** A single `is_terminal()` check cleanly separates interactive and non-interactive paths, preventing hangs in CI while allowing safe operations like token auto-generation.

## Prevention Strategies

### Never hand-roll parsers for standard formats

When modifying TOML, JSON, or YAML files, always parse into a typed structure, modify in-memory, and serialize back. Never use line-based string manipulation. **Review gate:** if you see `file.lines().map(...)` or `content.replace(old, new)` applied to a structured config file, reject it.

### Always use echo suppression for secrets

Any function that reads a secret must use `dialoguer::Password` or `rpassword`. **Review gate:** grep for `read_line` in interactive flows — if the variable name contains "key", "token", "secret", or "password", it is a bug.

### TTY guard at the top of interactive commands

Any CLI subcommand that prompts for user input must check `std::io::stdin().is_terminal()` before entering interactive mode. The `skills.rs` installer already follows this pattern.

### Consistent atomic write pattern

Use the same temp-file-then-rename pattern for all config/data files. Consider extracting a shared `atomic_write_file(path, contents)` helper in `mika-common` if more callers emerge.

### Setup commands must be re-runnable

Never early-return just because a config file exists. Check each value individually and skip only what's already configured.

## Testing Checklist

- [ ] API key input is not echoed to terminal (visual check)
- [ ] TOML injection: provide a value containing `"\n[malicious]\nfoo = "bar` — verify config.toml parses correctly with no extra keys
- [ ] Non-TTY rejection: `echo "" | mika setup` exits non-zero with clear message
- [ ] Re-run preserves existing config: run setup twice, verify unchanged values survive
- [ ] File permissions: `.env` and `config.toml` are both `0600` on Unix
- [ ] Token generation: `MIKA_INTERNAL_TOKEN` is exactly 64 hex chars
- [ ] dotenvy round-trip: values written to `.env` load correctly via `dotenvy::from_path()`

## Related Documentation

- [Simplified Config 4-Source Model](../architecture-patterns/simplified-config-4-source-model.md) — Parent solution doc covering the config cascade refactor and dotenv module
- [Env Var Leakage in Exec Handlers](env-var-leakage-exec-handler-child-processes.md) — Defense-in-depth for MIKA_* env var scrubbing from child processes
- [OTLP Endpoint Path Requirement](../integration-issues/otlp-endpoint-path-requirement.md) — OTLP endpoint configuration requirements
- [Configuration Reference](../../configuration.md) — Full configuration reference with the 4-source cascade
- [Getting Started](../../getting-started.md) — Setup wizard documentation for end users

### Resolved Todos

| Todo | Priority | Finding |
|------|----------|---------|
| #604 | P3 | API key echo suppression |
| #605 | P2 | TOML injection via OTLP endpoint |
| #606 | P3 | Extract prompt helper to reduce repetition |
| #607 | P3 | Add TTY guard for non-interactive contexts |
| #608 | P3 | Atomic writes for config.toml |
