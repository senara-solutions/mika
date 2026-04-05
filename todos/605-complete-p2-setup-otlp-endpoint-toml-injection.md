---
status: complete
priority: p2
issue_id: 605
tags: [code-review, security, input-validation]
dependencies: []
---

# OTLP endpoint value not validated before TOML interpolation

## Problem Statement

The `set_config_toml_value` function in `setup.rs` interpolates user-provided values directly into TOML format strings without validation or escaping. A user-supplied OTLP endpoint containing double-quote or newline characters can break out of the TOML string and inject arbitrary key-value pairs into `config.toml`.

Example: entering `http://evil.com"\nmalicious_key = "payload` as the endpoint would produce malformed TOML with injected keys.

While the user is entering their own config (self-attack), this breaks config integrity and could cause confusing failures when `config-rs` parses the file later.

## Findings

- **Source:** security-sentinel + architecture-strategist agents
- **Location:** `crates/mika-cli/src/commands/setup.rs` — `set_config_toml_value()` and the OTLP endpoint interpolation
- **Evidence:** `format!("\"{endpoint}\"")` on line 71 does no escaping of special characters
- **Related:** The `toml` crate is already a workspace dependency and already in `mika-cli/Cargo.toml`

## Proposed Solutions

### Option A: Use `toml` crate for proper serialization (Recommended)
Replace line-based `set_config_toml_value` with proper TOML parsing:
```rust
fn set_config_toml_value(home_dir: &Path, key: &str, value: toml::Value) -> Result<()> {
    let config_path = home_dir.join("config.toml");
    let mut table: toml::Table = match std::fs::read_to_string(&config_path) {
        Ok(content) => content.parse().unwrap_or_default(),
        Err(_) => toml::Table::new(),
    };
    table.insert(key.to_string(), value);
    std::fs::write(&config_path, toml::to_string_pretty(&table)?)?;
    Ok(())
}
```
- Effort: Small
- Risk: Low — `toml` is already a dependency, handles all edge cases
- Pro: Also fixes `config_key_is_set` (use parsed table instead of line scanning)
- Con: May reformat existing comments in config.toml (toml crate discards comments)

### Option B: Validate/reject special characters
```rust
if endpoint.contains('"') || endpoint.contains('\n') || endpoint.contains('\r') {
    anyhow::bail!("OTLP endpoint contains invalid characters");
}
```
- Effort: Small
- Risk: Low
- Pro: Minimal change, preserves existing formatting
- Con: Does not fix the underlying fragility of line-based TOML editing

## Acceptance Criteria

- [x] OTLP endpoint values containing quotes/newlines are handled safely
- [x] `set_config_toml_value` produces valid TOML for all inputs
- [x] Existing config.toml tests pass
