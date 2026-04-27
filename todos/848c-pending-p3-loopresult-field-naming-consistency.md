---
status: pending
priority: p3
issue_id: "848c"
tags: [code-review, agent-loop, maintainability, mika-848-followup]
dependencies: []
---

# `LoopResult` variant field naming inconsistency

## Problem Statement

The new `LoopResult` enum (`crates/mika-agent/src/agent.rs:517-548`) uses inconsistent field names for the same data across variants:

- `Done` and `MaxStepsExceeded` use `tool_call_summaries` and `usage`.
- `DeadlineExceeded` renames the same data to `partial_summaries` and `last_usage`.

Both fields hold identical types (`Vec<ToolCallSummary>` and `Option<LlmUsage>`). The "partial" semantics are already encoded by the variant name itself.

## Why it matters

- mika#848 ce:review (M2, severity medium, confidence 0.92).
- Match arms across the three outer functions need prefix-tracking per variant — adds friction to readability.
- A future "extract shared destructuring" refactor would need to rename one set or the other.

## Findings

- **Source:** mika#848 maintainability review (M2)
- **Location:** `crates/mika-agent/src/agent.rs:517-548`

## Proposed Solution

Rename `DeadlineExceeded`'s `partial_summaries` → `tool_call_summaries` and `last_usage` → `usage`. Update the three consumer match arms in `run_agent_inner`, `run_silent_inner`, `run_team_agent_inner_impl`.

## Definition of Done

- [ ] All three `LoopResult` variants use consistent field names for shared types.
- [ ] Consumer match arms updated to match.
- [ ] `cargo test -p mika-agent` passes.

## Notes

Cosmetic. Deferred from mika#848 PR scope to keep the fix focused.
