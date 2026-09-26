# Groomed With Placeholder Path: Plan With Unresolved Path (ITERATE)

## Plan Under Review: docs/plans/1244-config-migration.md

### Summary

Migrate agent configuration from flat `.env` files to structured `config.toml`
for better organization and validation.

### Design

- **Format:** TOML with typed sections (`[llm]`, `[server]`, `[memory]`)
- **Parser:** `config-rs` with `MIKA_` env prefix (existing pattern)
- **Location:** `<path>/config.toml` — the exact path within the agent home directory
  will be decided at implementation time
- **Fallback:** If `config.toml` is missing, fall back to existing `.env` loading

### Implementation

1. Define `MikaConfig` struct with serde derives
2. Add `load_config()` that tries TOML first, `.env` fallback
3. Update `Settings::new()` to use `load_config()`
4. Add migration helper that reads `.env` and writes `config.toml`

### Error Handling

- Parse error in TOML → log error, fall back to `.env`
- Both missing → startup panic (existing behavior)

### Test Plan

- Unit: TOML parsing, `.env` fallback, migration helper
- Integration: startup with TOML-only, `.env`-only, and both present

### Files

- `crates/mika-common/src/config.rs` — new `load_config()` and `MikaConfig`
- `<path>/migration.rs` — migration helper (exact module location TBD)
- `crates/mika-common/src/settings.rs` — update `Settings::new()`
