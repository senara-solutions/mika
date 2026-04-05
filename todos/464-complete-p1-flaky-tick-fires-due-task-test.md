---
status: complete
priority: p1
issue_id: "464"
tags: [code-review, testing, task-engine, flaky]
dependencies: []
---

# 464 · `test_tick_fires_due_task` is flaky — fixed 100ms sleep is non-deterministic

## Problem Statement

`test_tick_fires_due_task` calls `engine.tick().await`, then sleeps 100ms
and asserts the task status is `"in_progress"` or `"completed"`. Under CI
load or debug builds, the spawned `tokio::spawn` task may not finish within
100ms, causing a flaky assertion. The wide acceptance mask also hides real
failures: if dispatch sets `status = 'failed'`, the `_ => {}` arm passes
silently.

## Findings

- **Location:** `crates/mika-agent/src/task_engine/engine.rs:473–495`
- `fire_task` spawns via `tokio::spawn` — completion is not awaited
- Fixed `tokio::time::sleep(Duration::from_millis(100))` is the only synchronization
- The assertion accepts both `"in_progress"` and `"completed"` — a failed dispatch is undetected

## Proposed Solutions

### Option A — Poll with timeout (recommended)
Replace the fixed sleep with a polling loop:
```rust
let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
loop {
    let status = /* query DB */;
    if status == "completed" { break; }
    if tokio::time::Instant::now() > deadline { panic!("timed out waiting for task"); }
    tokio::time::sleep(Duration::from_millis(10)).await;
}
```

**Pros:** Fast in practice, deterministic on completion, fails clearly.
**Effort:** Small | **Risk:** Low

### Option B — Inject a mock dispatcher with oneshot channel
The dispatcher signals completion via a channel; the test awaits it.
**Pros:** Zero sleep, fully deterministic.
**Cons:** Requires dispatcher to be mockable (trait + impl swap).
**Effort:** Medium | **Risk:** Low

## Recommended Action

Option A immediately. Option B as a follow-up if the dispatcher grows.

## Technical Details

- **Affected files:** `crates/mika-agent/src/task_engine/engine.rs` (test section)

## Acceptance Criteria

- [ ] Fixed sleep replaced with poll-with-timeout or channel-based sync
- [ ] Assertion checks for `"completed"` specifically (not `"in_progress"`)
- [ ] Test reliably passes 100× in a loop without flakes

## Work Log

- 2026-03-06: Identified by test coverage review agent (TEST-4)
