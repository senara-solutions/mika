---
status: complete
priority: p3
issue_id: "395"
tags: [code-review, architecture, lifecycle]
dependencies: []
---

# Server poller JoinHandle not tracked for graceful shutdown

## Problem Statement

In server mode, the poller's `JoinHandle` is dropped immediately after spawning (`spawn_poller()` return value is unused). This means there is no way to gracefully abort the poller when the server receives SIGTERM. The CLI chat mode correctly stores and aborts the poller handle.

The poller task continues until the tokio runtime drops, which happens shortly after `axum::serve` returns. If the poller is mid-Claude-API-call, the request will be abruptly cancelled. This is generally safe but not ideal.

## Findings

- CLI chat mode stores `poller_handle` in `AgentWorker` and aborts it on quit (line 421 of chat.rs)
- Server mode drops the handle (line 274 of server/mod.rs)
- The comment "handle dropped, task lives on" is accurate but doesn't address shutdown
- Identified by: agent-native-reviewer, architecture-strategist

## Proposed Solutions

### Option A: Store and abort poller handles on shutdown
- Collect poller handles into a `Vec<JoinHandle<()>>`
- Store in `AppState` or a local variable
- Abort all handles before `axum::serve` returns
- Pros: Matches CLI pattern, clean shutdown
- Cons: Small complexity increase
- Effort: Small
- Risk: Low

### Option B: Accept current behavior (SELECTED)
- The runtime drop aborts tasks automatically
- No data loss risk (reminders are idempotent via `mark_reminder_delivered`)
- Pros: No code change, already safe
- Cons: Not fully graceful
- Effort: None
- Risk: None

## Technical Details

- Affected files: `crates/mika-agent/src/server/mod.rs`
- The poller is now spawned inside the recovery task, so the handle is created inside a `tokio::spawn` closure

## Recommended Action

Accepted as-is (Option B). The current behavior is safe and no code changes are needed.

The tokio runtime drops after `axum::serve` with `with_graceful_shutdown` returns,
which automatically aborts all spawned tasks including the poller. Reminders are
idempotent (`mark_reminder_delivered` is a no-op if already delivered), so an
abrupt cancellation mid-flight causes no data corruption or duplicate delivery.
This matches the standard Axum server pattern where background tasks are cancelled
on runtime drop.

## Acceptance Criteria

- [x] ~~Server mode tracks poller handles~~ Not needed: runtime drop handles cancellation
- [x] ~~Handles are aborted during graceful shutdown~~ Automatic via tokio runtime drop
- [x] No behavior change to the poller itself

## Work Log

- 2026-03-02: Created during code review of reminder poller implementation
- 2026-03-02: Resolved as Option B (accept current behavior). The tokio runtime drop after `axum::serve` returns automatically aborts all spawned tasks, including the poller. Reminders are idempotent via `mark_reminder_delivered`, so abrupt cancellation is safe with no data loss. This follows standard Axum patterns where background tasks are cancelled on runtime drop. No code changes required.
