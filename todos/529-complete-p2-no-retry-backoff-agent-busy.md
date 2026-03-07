---
status: complete
priority: p2
issue_id: 529
tags: [code-review, performance, task-engine]
dependencies: [526]
---

# No Retry Backoff or Cap for "Agent Busy" Re-queuing

## Problem Statement

When task dispatch fails with "agent busy," the task is reset to pending with a fixed 10-second delay and no retry counter. If the agent stays busy, tasks cycle through pending → in_progress → pending every 10 seconds indefinitely, generating N unnecessary DB writes per cycle.

**Severity:** P2 — Write amplification under sustained load.

## Findings

- `crates/mika-agent/src/task_engine/engine.rs:435` — fixed 10s delay, no retry count
- `crates/mika-agent/src/server/handlers.rs:424` — same pattern

## Proposed Solutions

1. **Exponential backoff with max retries**
   - Store retry count in action_config or a column, backoff 10s → 20s → 40s → ..., fail after N retries
   - Effort: Medium
   - Risk: Low

2. **Let timeout_at handle it naturally**
   - Keep re-queuing, but check timeout_at before re-queue — if past deadline, let it expire
   - Effort: Small
   - Risk: Low

## Acceptance Criteria

- [ ] Retry delays increase over time (backoff)
- [ ] Tasks eventually fail/expire instead of retrying forever
