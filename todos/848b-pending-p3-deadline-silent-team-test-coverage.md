---
status: pending
priority: p3
issue_id: "848b"
tags: [code-review, agent-loop, testing, mika-848-followup]
dependencies: []
---

# Deadline-exceeded coverage gaps: silent + team mode entry points have no tests

## Problem Statement

The mika#848 fix exposes three test-only entry points: `run_agent_with_deadline`, `run_silent_agent_with_deadline`, and `run_team_agent_with_deadline`. Only the conversation-mode entry point is covered by `tests/eval/test_deadline_in_flight_llm_call.rs`.

The fix's three-mode symmetry means the silent and team paths share the same shape:
- Prelude deadline gate.
- `LoopResult::DeadlineExceeded` arm.
- Continuation-skip gate.

Each path has its own mode-specific side effect (`record_reflection_run("failed", 0, "Timed out")` for silent reflection, `update_task_completed(task_id, fallback)` for team) — none currently asserted by a test.

## Why it matters

- mika#848 ce:review surfaced this as testing-002 (silent-mode-deadline-untested, severity high) and testing-003 (team-mode-deadline-untested, severity high).
- A future regression in either mode would not fail any test.
- The compiler's `LoopResult` exhaustiveness check guarantees the variants are *handled*, not that they're handled *correctly*.

## Findings

- **Source:** mika#848 testing review (testing-002, testing-003)
- **Location:** `crates/mika-agent/src/agent.rs` (silent at ~2700-2950, team at ~3070-3300); test file at `tests/eval/test_deadline_in_flight_llm_call.rs`

## Proposed Solutions

Add two eval scenarios:

1. **Silent reflection deadline test** — drive `run_silent_agent_with_deadline` with `SilentTrigger::Reflection` and a slow tool-call response; assert (a) `llm_calls` row persisted, (b) `reflection_runs` row with status='failed' and summary='Timed out'.
2. **Team-mode deadline test** — drive `run_team_agent_with_deadline` with a slow tool-call response and a `child_task_id`; assert (a) `llm_calls` row persisted, (b) `tasks` row updated to completed with fallback string, (c) return value is `Some("Agent timed out while processing team task.")`.

## Definition of Done

- [ ] `tests/eval/test_deadline_silent_mode.rs` covers silent + reflection.
- [ ] `tests/eval/test_deadline_team_mode.rs` covers team + child_task.
- [ ] Both registered in `tests/eval.rs`.

## Notes

Defer-or-decline rationale (mika#848 PR scope): the conversation-mode test exercises the shared `run_loop` deadline check + `attempt_continuation_turn` save_row helper that all three modes use. Behavioral parity is thus structurally enforced at the shared-helper level. Mode-specific side-effect tests are additional defense-in-depth but not strictly load-bearing for the fix's primary contract.
