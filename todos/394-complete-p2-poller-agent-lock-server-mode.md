---
status: complete
priority: p2
issue_id: "394"
tags: [code-review, architecture, concurrency]
dependencies: []
---

# Poller does not acquire agent_lock in server mode

## Problem Statement

The reminder poller's `check_and_fire_reminders()` calls `run_silent_agent()` without acquiring the per-agent `agent_lock` (`tokio::sync::Mutex`). In server mode, this means a reminder can fire concurrently with a user message or heartbeat, producing interleaved agent loops.

SQLite serializes DB writes via `AsyncDatabase`'s dedicated thread, and `run_silent_agent` uses unique session IDs, so data corruption is unlikely. However, concurrent Claude API calls waste money and interleaved writes to `conversations` or memory could produce confusing context.

## Findings

- The heartbeat handler (`handlers.rs`) correctly uses `try_lock` on `agent_lock` and skips if busy
- The message handler acquires `agent_lock` before running the agent loop
- The poller has no reference to `agent_lock` at all
- In CLI mode, the channel-based serialization on the worker task already prevents true concurrency
- Identified by: architecture-strategist, performance-oracle, agent-native-reviewer, security-sentinel

## Proposed Solutions

### Option A: Pass agent_lock to ReminderScheduler (IMPLEMENTED)
- Add `agent_lock: Arc<tokio::sync::Mutex<()>>` to `ReminderScheduler`
- Use `try_lock` in `check_and_fire_reminders()`, skip (defer to next cycle) if busy
- Pros: Matches existing heartbeat pattern, prevents concurrent agent loops
- Cons: Adds coupling between scheduler and server state
- Effort: Small
- Risk: Low

### Option B: Accept current behavior
- Document that reminders may run concurrently with user messages
- The isolation via unique session IDs provides sufficient safety
- Pros: No code change
- Cons: Concurrent API calls waste money; context may be confusing
- Effort: None
- Risk: Low (but not ideal)

## Technical Details

- Affected files: `crates/mika-agent/src/scheduler.rs`, `crates/mika-agent/src/server/mod.rs`, `crates/mika-cli/src/commands/chat.rs`
- Pattern: mirrors heartbeat handler's `try_lock` pattern in `handlers.rs`

## Acceptance Criteria

- [x] Poller uses `try_lock` before firing reminders in server mode
- [x] Reminder deferred to next poll cycle if agent is busy
- [x] No change to CLI mode behavior

## Work Log

- 2026-03-02: Created during code review of reminder poller implementation
- 2026-03-02: Implemented Option A — added `agent_lock: Option<Arc<tokio::sync::Mutex<()>>>` to `ReminderScheduler`. In `check_and_fire_reminders()`, `try_lock` is called once for the entire batch before firing any reminders; if the agent is busy, all reminders are deferred to the next 60s cycle. Server mode passes `Some(agent_lock.clone())` (same Arc used by AgentState), CLI passes `None`. All tests pass `cargo build` and `cargo clippy` clean.
