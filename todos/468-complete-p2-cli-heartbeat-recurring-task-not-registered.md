---
status: pending
priority: p2
issue_id: "468"
tags: [code-review, correctness, task-engine, cli]
dependencies: []
---

# 468 · CLI does not register heartbeat recurring task at startup

## Problem Statement

The server registers both `heartbeat` and `reflection` recurring tasks at
startup. The CLI (`spawn_agent_worker`) only registers `reflection`. CLI
users who leave the app running do not receive heartbeat proactive check-ins.
The asymmetry is undocumented and likely unintentional — the heartbeat
pre-filter (active hours, rate limit) is already designed to be safe in both
modes.

## Findings

- **Location:** `crates/mika-cli/src/commands/chat.rs:91–97`
- The comment says "Set up task engine for background tasks (reminders, reflection)" — heartbeat conspicuously absent
- Server path (in `server/mod.rs`) registers both via `ensure_recurring_task` for heartbeat and reflection

## Proposed Solutions

### Option A — Register heartbeat in CLI spawn_agent_worker (recommended)
Add the `ensure_recurring_task` call for `"heartbeat"` / `HEARTBEAT_CRON` immediately after the existing reflection registration.

**Effort:** Trivial | **Risk:** Low

### Option B — Document the deliberate omission
If the intent is to not run heartbeat in CLI mode, add an explicit comment explaining why.
**Pros:** No behavior change.
**Cons:** Asymmetric UX remains.

## Recommended Action

Option A — register heartbeat in CLI mode to match server behavior.

## Technical Details

- **Affected files:** `crates/mika-cli/src/commands/chat.rs`

## Acceptance Criteria

- [ ] `ensure_recurring_task("heartbeat", HEARTBEAT_CRON, ...)` called in `spawn_agent_worker`
- [ ] Or explicit comment documenting deliberate omission with rationale

## Work Log

- 2026-03-06: Identified by architecture review agent (ARCH-6)
