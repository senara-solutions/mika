---
problem_type: architecture-pattern
title: "Adding Log Format Selection to tracing_subscriber Init"
date: 2026-03-15
components:
  - crates/mika-common (logging.rs, config.rs)
  - crates/mika-agent (mika-server binary)
  - crates/mika-gateway (settings.rs, main.rs)
symptoms:
  - mika-server and mika-gateway always output JSON logs
  - No way to get human-readable logs during local development
root_cause: >
  logging::init() hardcoded .json().flatten_event(true) for the stdout layer.
  The CLI had init_pretty() with .pretty() but servers had no equivalent option.
keywords:
  - log format
  - tracing_subscriber
  - pretty logs
  - MIKA_LOG_FORMAT
  - type-level layer composition
---

## Problem

mika-server and mika-gateway always output JSON structured logs, which are
hard to read during local development. The CLI already had `init_pretty()`
for human-readable output, but there was no way to switch server/gateway
stdout between JSON and pretty format.

## Root Cause

`logging::init()` hardcoded `fmt::layer().json().flatten_event(true)` for
the stdout layer. No format parameter existed.

## Solution

Added a `LogFormat` enum (`Json`, `Pretty`) and a `log_format: LogFormat`
parameter to `logging::init()`. When `Pretty`, the stdout layer uses
`.pretty()` instead of `.json()`. File output always stays JSON.

### Key design decisions

1. **4-arm match in `init()`**: `tracing_subscriber`'s type-level layer
   composition means `.json()` and `.pretty()` produce different types.
   You cannot store them in a variable and branch — each subscriber chain
   must be a concrete type. The 4-arm match (`log_file` x `log_format`)
   is the idiomatic and unavoidable pattern. The existing `init_pretty()`
   already uses the same pattern for the same reason.

2. **String field, parse at call site**: `log_format` is stored as `String`
   on both `Settings` and `GatewaySettings`, matching the existing
   `log_level: String` convention. Parsing to `LogFormat` happens once at
   startup in each binary's `main()`. This is consistent with how the
   codebase handles all logging-related config.

3. **Case-sensitive `FromStr`**: Only lowercase `"json"` and `"pretty"` are
   accepted. Invalid values cause a clear startup error (fail-fast).

4. **CLI unaffected**: The CLI uses `init_pretty()` which is unchanged.
   `MIKA_LOG_FORMAT` only affects server/gateway binaries.

### Checklist for adding new env vars

Follow the checklist from `config-key-rename-across-layers.md`:

1. `Settings` struct field + serde default function
2. `ConfigKeyInfo` registry entry (key, backend, env_var, secret, description)
3. `get_effective_value()` match arm
4. Manual `Debug` impl for `Settings`
5. `GatewaySettings` struct field + serde default (if applicable)
6. Manual `Debug` impl for `GatewaySettings`
7. Test struct literals in both settings files
8. `.env.example`
9. `CLAUDE.md` Environment Variables section
10. `docs/configuration.md` (Settings Reference, server/gateway env tables)

## Prevention

- When adding format/mode switches to `tracing_subscriber` init functions,
  expect to multiply match arms by the number of new variants. This is a
  known constraint of the type system, not a design flaw.
- New env vars should follow the checklist above to avoid missing any of
  the ~10 touch points across the codebase.
