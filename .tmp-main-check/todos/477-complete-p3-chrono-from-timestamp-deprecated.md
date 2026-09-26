---
status: complete
priority: p3
issue_id: "477"
tags: [code-review, quality, dependencies]
dependencies: []
---

# 477 · `chrono::DateTime::from_timestamp` is deprecated in `cron.rs`

## Problem Statement

`cron.rs` uses `chrono::DateTime::from_timestamp(after_unix, 0)` which was
deprecated in chrono 0.4.27. This will produce a deprecation warning in
future chrono versions and the alternative `timestamp_opt` provides better
error handling for out-of-range timestamps.

## Findings

- **Location:** `crates/mika-agent/src/task_engine/cron.rs:14`

## Proposed Solutions

### Option A — Use `timestamp_opt` (recommended)
```rust
let after_dt = chrono::Utc.timestamp_opt(after_unix, 0)
    .single()
    .ok_or_else(|| anyhow!("invalid unix timestamp: {}", after_unix))?;
```

**Effort:** Trivial | **Risk:** Low

## Acceptance Criteria

- [ ] `from_timestamp` replaced with `timestamp_opt(...).single()`
- [ ] `cargo clippy` produces no deprecation warning for this line

## Work Log

- 2026-03-06: Identified by code quality review agent (QUAL-10)
