---
status: complete
priority: p2
issue_id: 532
tags: [code-review, implementation, teams]
dependencies: []
---

# Wire up next_phase Parameter in execute_from_phase

## Problem Statement

`execute_from_phase` accepts `_next_phase: &str` but unconditionally runs Review then Deliver. The full plumbing is already in place: `invoke_orchestrator` action serializes `next_phase` into `action_config`, `dispatch_invoke_orchestrator` reads it, `execute_from_phase` receives it. The parameter just needs to be used instead of ignored.

**Severity:** P2 — Incomplete implementation, resume always starts at Review regardless of what was requested.

## Findings

- `crates/mika-agent/src/teams/engine.rs:208` — `_next_phase: &str` (underscore prefix = unused)
- `crates/mika-agent/src/teams/mod.rs` — threads it through correctly
- `crates/mika-agent/src/task_engine/dispatcher.rs` — reads from action_config correctly
- `crates/mika-agent/src/teams/engine.rs` — stores in action_config correctly

## Proposed Solutions

1. **Wire up the parameter to control phase entry point**
   - Remove underscore prefix, match on phase name to determine starting point
   - E.g., `"review"` → run Review then Deliver; `"deliver"` → skip Review, run Deliver only
   - Pros: Completes the implementation, enables flexible resume points
   - Cons: None — plumbing already exists
   - Effort: Small
   - Risk: Low

## Acceptance Criteria

- [ ] `execute_from_phase` uses `next_phase` to determine which phase to start from
- [ ] `"review"` starts at Review → Deliver
- [ ] `"deliver"` starts at Deliver only
- [ ] Underscore prefix removed from parameter name
