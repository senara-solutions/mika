---
title: "Reminders never fire at scheduled time"
category: runtime-errors
component: crates/mika-agent/src/scheduler.rs
tags: [reminders, scheduler, background-poller, silent-agent, tokio]
date_identified: 2026-03-02
date_resolved: 2026-03-02
severity: high
affected_modes: [cli-chat, server]
---

# Reminders Never Fire at Scheduled Time

## Problem

Reminders created via the `create_reminder` tool were stored in SQLite but **never fired at their scheduled `fire_at` time**. They only fired as "past-due recovery" on the next CLI/server startup — if the user happened to restart after the time passed.

### Symptoms

- User creates a reminder: "remind me in 2 minutes to test"
- Reminder is stored in the `reminders` table with correct `fire_at` timestamp
- 2+ minutes pass — nothing happens
- User restarts `mika` → reminder fires as "past-due recovery"
- `mika reminders` shows status "pending" indefinitely for future reminders

### Root Cause

`create_reminder` did a DB INSERT and nothing else. There was no background task polling for due reminders. The scheduler's `recover()` method fired past-due reminders on startup, and the doc comments explicitly said "timer scheduling is Phase 2." Phase 2 was never implemented.

```rust
// Before: scheduler.rs doc comment
/// - Future reminders: log count (timer scheduling is Phase 2)
```

## Solution

Added a background reminder poller via `tokio::spawn` that polls the DB every 60 seconds for past-due reminders and fires them via `run_silent_agent`. This works in both CLI chat mode and server mode.

### Key Design Decisions

1. **Polling over per-reminder timers**: Simpler, no need to coordinate timer lifecycle when reminders are created/cancelled. A 60-second max delay is acceptable for reminders.

2. **`MissedTickBehavior::Skip`**: Prevents burst-firing if a slow reminder causes the poller to miss ticks.

3. **First tick skipped**: `recover()` already handles startup, so the first poll at t=0 is redundant.

4. **Per-cycle cap (`MAX_POLL_REMINDERS = 10`)**: Bounds worst-case execution time. Excess reminders deferred to next cycle. Without this, 20 simultaneous reminders could block the poller for 100+ minutes (5-minute agent timeout per reminder).

5. **Agent lock in server mode**: The poller acquires `agent_lock` via `try_lock()` before firing any reminders. If the agent is busy (user message or heartbeat), all reminders are deferred to the next 60s cycle. CLI mode passes `None` for the lock (serialization handled by channel).

6. **Poller sequenced after recovery in server mode**: The poller starts only after `recover()` completes, preventing a race where both could fire the same reminder concurrently.

### Files Changed

| File | Change |
|------|--------|
| `crates/mika-agent/src/scheduler.rs` | Added `agent_lock` field, `check_and_fire_reminders()`, `spawn_poller()`, test |
| `crates/mika-cli/src/commands/chat.rs` | `AgentWorker` gets `poller_handle`, scheduler becomes `Arc`, abort on exit/switch |
| `crates/mika-agent/src/server/mod.rs` | Share `agent_lock` Arc, sequence poller after recovery |

### Implementation Details

**New `spawn_poller()` method** on `ReminderScheduler`:

```rust
pub fn spawn_poller(self: Arc<Self>) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_secs(60));
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        interval.tick().await; // skip first tick
        loop {
            interval.tick().await;
            self.check_and_fire_reminders().await;
        }
    })
}
```

**Agent lock guard** in `check_and_fire_reminders()`:

```rust
let _guard = if let Some(ref lock) = self.agent_lock {
    match lock.try_lock() {
        Ok(guard) => Some(guard),
        Err(_) => {
            debug!(count, "agent busy, deferring reminder firing to next cycle");
            return;
        }
    }
} else {
    None
};
```

### Review Findings Fixed

During code review (7 parallel agents), three additional bugs were caught and fixed:

1. **skills_dirty flag theft**: The poller initially cleared the `skills_dirty` flag without reloading the SkillRegistry, stealing the signal from server handlers and CLI worker. Fix: removed the no-op block entirely.

2. **No per-cycle cap**: Unlike `recover()` which capped at 5, the poller had no limit. Fix: added `MAX_POLL_REMINDERS = 10`.

3. **Recovery/poller race**: In server mode, `recover()` and the poller ran concurrently, potentially double-firing the same reminder. Fix: poller starts only after recovery completes.

4. **No agent_lock**: Poller could run concurrently with user messages in server mode. Fix: added `agent_lock: Option<Arc<tokio::sync::Mutex<()>>>` to `ReminderScheduler`, uses `try_lock` to defer when busy.

## Verification

1. `cargo test` — all tests pass (including new `test_check_and_fire_reminders_no_due`)
2. `cargo clippy` — no warnings
3. Manual test: start `mika` chat → create a 2-minute reminder → wait → reminder fires within 60s of due time

## Lessons Learned

- **Phase 2 comments are tech debt markers**: Treat "Phase 2" doc comments as bugs that need tracking. The comment existed since the initial implementation but was never tracked as a task.
- **Review catches real bugs in new code**: The 7 review agents found 3 genuine bugs (flag theft, no cap, race condition) that would have shipped without review.
- **`try_lock` for optional serialization**: Using `Option<Arc<Mutex>>` with `try_lock` lets the same code path work in both serialized (server) and unserialized (CLI) modes without branching the architecture.
