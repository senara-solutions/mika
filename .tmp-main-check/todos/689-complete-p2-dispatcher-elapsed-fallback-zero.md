---
status: pending
priority: p2
issue_id: "689"
tags: [code-review, correctness, task-engine]
dependencies: []
---

## Problem Statement

In `dispatcher.rs`, when `timestamp::parse()` fails on the last user message timestamp, the `elapsed` value defaults to `0`. This means the system treats a parse failure as "user messaged just now," which silently suppresses both reflection and heartbeat dispatching.

The old code returned `i64` directly from SQLite, so parse failure wasn't possible. The new code introduces a failure mode that defaults in the wrong direction.

## Findings

Found by: code-simplicity-reviewer, pattern-recognition-specialist

**Two locations in `dispatcher.rs`:**
1. ~line 560: reflection dispatch — `elapsed = 0` on parse failure → always defers reflection
2. ~line 699: heartbeat dispatch — `elapsed = 0` on parse failure → always suppresses heartbeat

## Proposed Solutions

### Option A: Default to large value + log warning (Recommended)

```rust
let elapsed = if let Ok(last_dt) = crate::timestamp::parse(&last_ts) {
    chrono::Utc::now().signed_duration_since(last_dt).num_seconds()
} else {
    warn!(timestamp = %last_ts, "failed to parse last_user_message_time, treating as stale");
    i64::MAX
};
```

- **Pros:** Fails open (allows scheduled work to proceed), observable via logs
- **Cons:** None meaningful
- **Effort:** Small
- **Risk:** Low

### Option B: Propagate error

- **Pros:** Strict correctness
- **Cons:** Changes control flow, may need broader refactoring
- **Effort:** Medium
- **Risk:** Could cause unnecessary failures

## Recommended Action

Option A — simple fix with correct fail-direction.

## Technical Details

- **Affected files:** `crates/mika-agent/src/task_engine/dispatcher.rs`

## Acceptance Criteria

- [ ] Parse failures log a warning
- [ ] Parse failures default to a large elapsed value (not 0)

## Work Log

- 2026-03-18: Identified during code review of timestamp migration changeset
