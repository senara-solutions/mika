---
title: "feat: Add MIKA_LOG_FORMAT option for human-readable logs"
type: feat
status: completed
date: 2026-03-15
---

# feat: Add MIKA_LOG_FORMAT option for human-readable logs

## Overview

mika-server and mika-gateway always output JSON logs via `logging::init()`. The CLI already has `init_pretty()` for human-readable output, but there's no way to get pretty logs from server/gateway — which is inconvenient during local development. This adds a `MIKA_LOG_FORMAT` env var to switch between `json` (default) and `pretty`.

## Proposed Solution

Add a `LogFormat` enum to `logging.rs` and a `log_format` parameter to `init()`. When `Pretty`, use `.pretty()` for the stdout layer instead of `.json()`. File output always stays JSON.

## Acceptance Criteria

- [x] `LogFormat` enum (`Json`, `Pretty`) in `crates/mika-common/src/logging.rs`
- [x] `logging::init()` accepts `log_format: LogFormat` parameter
- [x] Stdout uses `.pretty()` when `LogFormat::Pretty`, `.json()` when `LogFormat::Json`
- [x] File output always uses `.json()` regardless of format setting
- [x] `MIKA_LOG_FORMAT` registered in `CONFIG_KEYS` (`ConfigBackend::File`, env_var `MIKA_LOG_FORMAT`)
- [x] `log_format` field on `Settings` struct (default `"json"`)
- [x] `get_effective_value()` match arm for `log_format`
- [x] Manual `Debug` impl updated for `Settings`
- [x] `log_format` field on `GatewaySettings` (default `"json"`)
- [x] Manual `Debug` impl updated for `GatewaySettings`
- [x] Test struct literals updated for both settings structs
- [x] mika-server passes `log_format` to `logging::init()`
- [x] mika-gateway passes `log_format` to `logging::init()`
- [x] `.env.example` updated with `MIKA_LOG_FORMAT`
- [x] `CLAUDE.md` Environment Variables section updated
- [x] Invalid values cause a clear startup error (fail-fast)
- [x] CLI is unaffected — `init_pretty()` is not modified
- [x] All existing tests pass

## Technical Considerations

### LogFormat enum design

```rust
// crates/mika-common/src/logging.rs
#[derive(Debug, Clone, PartialEq, Default)]
pub enum LogFormat {
    #[default]
    Json,
    Pretty,
}
```

Parse from string at call sites (not serde on the enum itself — Settings stores it as `String` like `log_level`). Use a `FromStr` or helper method with case-insensitive matching and clear error on invalid values.

### Signature change to `init()`

```rust
// Before
pub fn init<OL>(default_level: &str, log_file: Option<&Path>, otel_layer: Option<OL>)

// After
pub fn init<OL>(default_level: &str, log_file: Option<&Path>, log_format: LogFormat, otel_layer: Option<OL>)
```

Two callers to update: `mika-server.rs:19`, `mika-gateway/main.rs:28`.

### Case sensitivity

Accept lowercase only via the parse method. `MIKA_LOG_FORMAT=Pretty` → error with message: `"MIKA_LOG_FORMAT must be 'json' or 'pretty', got 'Pretty'"`.

### CLI scope

The CLI ignores `MIKA_LOG_FORMAT`. It uses `init_pretty()` which is unchanged. The field exists on `Settings` but is only consumed by `mika-server`. ConfigKeyInfo description should clarify: "Controls stdout log format for mika-server and mika-gateway (json or pretty). CLI always uses pretty."

### Checklist from config-key-rename doc

Per `docs/solutions/architecture-patterns/config-key-rename-across-layers.md`, new env vars touch:

1. `Settings` struct field + serde default — `crates/mika-common/src/config.rs`
2. `ConfigKeyInfo` registry entry — `crates/mika-common/src/config.rs`
3. `get_effective_value()` match arm — `crates/mika-common/src/config.rs`
4. Manual `Debug` impl for `Settings` — `crates/mika-common/src/config.rs`
5. `GatewaySettings` struct field + serde default — `crates/mika-gateway/src/settings.rs`
6. Manual `Debug` impl for `GatewaySettings` — `crates/mika-gateway/src/settings.rs`
7. Test struct literals — both settings files
8. `LogFormat` enum + init() signature — `crates/mika-common/src/logging.rs`
9. mika-server call site — `crates/mika-agent/src/bin/mika-server.rs`
10. mika-gateway call site — `crates/mika-gateway/src/main.rs`
11. `.env.example`
12. `CLAUDE.md` Environment Variables section

## Sources

- Existing plan: `~/.claude/plans/sunny-stirring-wadler.md`
- Config rename checklist: `docs/solutions/architecture-patterns/config-key-rename-across-layers.md`
- Observability architecture: `docs/solutions/architecture/observability-otel-tui-dashboard.md`
- ConfigKeyRegistry pattern: `docs/solutions/architecture-patterns/config-key-registry-cli-management.md`
