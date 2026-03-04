---
status: complete
priority: p1
issue_id: "451"
tags: [code-review, testing, telemetry]
dependencies: []
---

# Environment-Dependent OTel Test Failure

## Problem Statement

`test_build_otel_layer_disabled` in `crates/mika-common/src/telemetry.rs` calls `Settings::load(tmp.path())` which reads `MIKA_TELEMETRY_ENABLED` from the process environment via config-rs's `Environment::with_prefix("MIKA")`. If this variable is set in the developer's shell or CI, the test fails because `telemetry_enabled` becomes `true` instead of the expected `false`.

## Findings

- **Source**: Architecture strategist agent
- **Location**: `crates/mika-common/src/telemetry.rs:133-142`
- **Evidence**: `Settings::load()` merges env vars → test depends on clean env

## Proposed Solutions

### Option A: Clear env var before test (Recommended)
Use `unsafe { std::env::remove_var("MIKA_TELEMETRY_ENABLED") }` in a `clean_env()` helper, consistent with existing config tests that use `#[serial]` + env cleanup.
- **Pros**: Minimal change, follows existing pattern in `config.rs` tests
- **Cons**: Requires `unsafe` block (edition 2024), needs `#[serial]` for test isolation
- **Effort**: Small

### Option B: Construct Settings directly
Build a `Settings` struct with explicit `telemetry_enabled: false` instead of loading from disk/env.
- **Pros**: No env dependency, no unsafe
- **Cons**: Must construct full Settings struct or add a test helper
- **Effort**: Small

## Acceptance Criteria

- [ ] `cargo test --features telemetry` passes with `MIKA_TELEMETRY_ENABLED=true` in env
- [ ] Test does not depend on external environment state
