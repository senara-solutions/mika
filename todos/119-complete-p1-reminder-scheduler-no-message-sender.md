---
status: complete
priority: p1
issue_id: "119"
tags: [code-review, architecture, agent-native]
dependencies: []
---

# ReminderScheduler Has No MessageSender — Recovered Reminders Can't Reach Users

## Problem Statement

In `server/mod.rs:69`, the `ReminderScheduler` is constructed with `message_sender: None`. When `scheduler.recover()` fires past-due reminders on startup, the silent agent loop has no way to send messages to the user via the gateway. Recovered reminders will execute but their output is silently lost.

## Findings

- **Source:** agent-native-reviewer (CRITICAL-2), code-simplicity-reviewer
- **Location:** `crates/mika-agent/src/server/mod.rs:69` — `message_sender: None`
- **Evidence:** `ReminderScheduler { ..., message_sender: None }` — comment says "Wired in PR 3 with GatewayMessageSender" but PR 3 IS this PR and it's still None
- **Impact:** Past-due reminders fire on startup but user never receives the message — silent data loss from user's perspective

## Proposed Solutions

### Option 1: Wire GatewayMessageSender into ReminderScheduler at construction
- **Pros**: Reminders can immediately reach users, simple fix
- **Cons**: Requires creating a GatewayMessageSender before scheduler (needs gateway_url and internal_token)
- **Effort**: Small
- **Risk**: Low

### Option 2: Pass message_sender factory/closure to scheduler
- **Pros**: Lazy construction, scheduler doesn't need to know about gateway details
- **Cons**: More complex, over-engineered for current needs
- **Effort**: Medium
- **Risk**: Low

## Recommended Action

Option 1 — create a `GatewayMessageSender` and wrap in `Arc<dyn MessageSender>` before constructing the scheduler. The gateway_url and internal_token are already validated at that point in `run_server`.

## Technical Details

- **Affected Files**: `crates/mika-agent/src/server/mod.rs`
- **Related Components**: ReminderScheduler, GatewayMessageSender, silent agent loop
- **Database Changes**: None

## Acceptance Criteria

- [ ] ReminderScheduler constructed with a real MessageSender in server mode
- [ ] Recovered reminders can send messages to users via gateway
- [ ] Stale "Wired in PR 3" comment removed
- [ ] Existing tests pass

## Work Log

### 2026-02-24 - Identified during PR #5 review
**By:** agent-native-reviewer, code-simplicity-reviewer
**Actions:** Flagged as P1 — recovered reminders silently fail to deliver

## Resources

- PR #5: Phase 2 Container HTTP Server
- Related: `crates/mika-agent/src/scheduler.rs` — `ReminderScheduler::recover()`
