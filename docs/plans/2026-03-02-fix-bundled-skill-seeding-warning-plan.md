---
title: "fix: Improve bundled skill seeding disabled warning"
type: fix
status: completed
date: 2026-03-02
---

# fix: Improve bundled skill seeding disabled warning

## Overview

The `tracing::warn!("bundled skill seeding disabled by config")` message fires on every agent startup when `MIKA_DISABLE_BUNDLED_SKILLS=true` is set. While the config wiring is correct (default `false`, env var and TOML support, proper tests), the warning message is not actionable — it doesn't tell the user what config key controls it or how to re-enable seeding.

## Problem Statement

The warning at `crates/mika-agent/src/startup.rs:33` was intentionally set to `warn` level (per completed todo #334) because disabling bundled skill seeding is security-relevant — it prevents handler script security patches from deploying. However, the message provides no guidance on how to resolve it, making it noisy without being helpful.

The config system is already correctly wired:
- `Settings.disable_bundled_skills` (default: `false`) at `crates/mika-common/src/config.rs:64-66`
- CLI passes `settings.disable_bundled_skills` at `crates/mika-cli/src/init.rs:62`
- Server passes `settings.disable_bundled_skills` at `crates/mika-agent/src/server/mod.rs:193,212`
- Agent creation hardcodes `false` (intentional) at `crates/mika-cli/src/commands/agents.rs:57`
- Test coverage exists for both disabled/enabled paths at `crates/mika-agent/src/startup.rs:47-64`
- Env var override test at `crates/mika-common/src/config.rs:365-375`

## Proposed Solution

Improve the warning message to be actionable while keeping `warn` level for security visibility.

## Acceptance Criteria

- [x] Warning message includes the config key name (`disable_bundled_skills`) and env var (`MIKA_DISABLE_BUNDLED_SKILLS`)
- [x] Warning message includes the security context (prevents handler script updates)
- [x] Log level stays at `warn` (security-relevant per todo #334)
- [x] Existing tests continue to pass
- [x] `cargo clippy` passes

## MVP

### `crates/mika-agent/src/startup.rs:33`

Change:
```rust
tracing::warn!("bundled skill seeding disabled by config");
```

To:
```rust
tracing::warn!(
    "bundled skill seeding disabled by config \
     (MIKA_DISABLE_BUNDLED_SKILLS=true) — handler script security updates will not be applied; \
     set to false or remove to re-enable"
);
```

## References

- Todo #334 (completed): `todos/334-complete-p2-log-level-too-quiet-for-disabled-seeding.md`
- Todo #338 (completed): `todos/338-complete-p3-document-production-warning-in-config.md`
- Config field: `crates/mika-common/src/config.rs:64-66`
- Call sites: `crates/mika-cli/src/init.rs:62`, `crates/mika-agent/src/server/mod.rs:193,212`
