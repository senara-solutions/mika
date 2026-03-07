---
status: complete
priority: p1
issue_id: "555"
tags: [code-review, correctness, data-loss]
dependencies: []
---

# Claim-Before-Process Ordering Risks Permanent Callback Loss

## Problem Statement

In the TUI path, `mark_task_delivered()` is called atomically in `poll_callback_tasks()` BEFORE the callback result is sent to the agent worker for processing. If the subsequent agent run fails (Claude API error, timeout, etc.), the task is already in `delivered` status and will never be retried. The callback result is permanently lost.

Contrast with the server path (`dispatcher.rs`) which correctly calls `mark_task_delivered()` AFTER `run_silent_agent` succeeds.

The plan document (Phase 3) specified retry logic (3 attempts, then fail), but this was not implemented.

## Findings

- **Found by:** Security Sentinel, Performance Oracle, Agent-Native Reviewer (3/8 agents)
- **Location:** `crates/mika-cli/src/tui/app.rs:1086-1095` (marks delivered before sending to worker)
- **Contrast:** `crates/mika-agent/src/task_engine/dispatcher.rs:278-281` (marks delivered after success)

## Proposed Solutions

### Option A: Move mark_task_delivered to after successful agent processing (Recommended)
- In `chat.rs` CallbackResult handler, call `mark_task_delivered` after `run_agent` succeeds
- On failure, leave task as `completed` for next poll cycle to retry
- Add a retry counter (e.g., in task metadata) with max 3 retries before marking `failed`
- **Pros:** Prevents data loss, matches server path semantics
- **Cons:** Multiple TUI instances could double-process (but `AgentStatus::Idle` guard + ~5s window makes this unlikely)
- **Effort:** Medium
- **Risk:** Low

### Option B: Two-phase claiming
- Add a `claiming` intermediate status or a `claimed_at` timestamp
- Mark `claiming` in poll, `delivered` after success, back to `completed` on failure
- **Pros:** Prevents both data loss and double-processing
- **Cons:** More complex, adds another status
- **Effort:** Medium
- **Risk:** Medium (more states to manage)

## Recommended Action

Option A — move `mark_task_delivered` to after success. The multi-instance race is negligible in practice.

## Technical Details

- **Affected files:** `crates/mika-cli/src/tui/app.rs`, `crates/mika-cli/src/commands/chat.rs`

## Acceptance Criteria

- [ ] Failed agent runs do NOT permanently lose callback results
- [ ] Task stays `completed` on failure, retried on next poll
- [ ] After max retries, task marked `failed` with error message
- [ ] Server and CLI paths both mark `delivered` after success

## Work Log

| Date | Action | Learnings |
|------|--------|-----------|
| 2026-03-07 | Created from code review | 3/8 agents flagged |
| 2026-03-07 | Approved during triage | Move mark_task_delivered to after success, add retry counter |

## Resources

- Plan: `docs/plans/2026-03-07-feat-callback-tui-delivery-plan.md` (Phase 3 retry spec)
