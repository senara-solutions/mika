---
status: pending
priority: p2
issue_id: "848a"
tags: [code-review, agent-loop, timeout, mika-848-followup]
dependencies: []
---

# Deadline-check granularity: single-iteration tool burst can exceed 420s bound

## Problem Statement

The mika#848 fix checks the agent total deadline only at the top of each `run_loop` iteration (`crates/mika-agent/src/agent.rs:701`). Within a single iteration, the loop can:

1. Make one LLM call (capped at 120s by the provider's `reqwest` timeout).
2. Dispatch N tool calls sequentially via `process_tool_calls`. Each tool call is capped at `TOOL_TIMEOUT_SECS = 30`, but the LLM can emit many tool_use blocks in one response.

If the LLM emits 20 tool_use blocks in a single response (a legal but pathological shape), the resulting iteration runs for `120s + 20 × 30s = 720s` — well beyond the documented 420s worst-case bound in the plan and solution doc.

## Why it matters

- The 420s bound is documented as the operator-facing contract in `docs/solutions/runtime-errors/agent-deadline-graceful-exit-2026-04-27.md`.
- Reliability reviewer surfaced this in the mika#848 ce:review pass (rel-1, severity high, confidence 0.85).
- A misbehaving model that loves emitting tool calls can hold an agent slot far longer than the operator expects, blocking the per-customer SQLite mutex.

## Findings

- **Source:** mika#848 reliability review (rel-1)
- **Location:** `crates/mika-agent/src/agent.rs:701-722` (deadline check in `run_loop`); `crates/mika-agent/src/agent.rs` `process_tool_calls` (sequential tool dispatch)
- **Evidence:** Plan mika#848 documents `300s + 120s = 420s` as the worst case. Tool-burst case violates this.

## Proposed Solutions

### Option A: Add deadline check inside `process_tool_calls` (Recommended)

Thread `deadline: Instant` into `process_tool_calls`. After each tool dispatch, check the deadline. If exceeded, stop dispatching the remaining tools in the current step and return what's been processed.

**Tradeoff:** Partial-batch dispatch is a behavior change. The LLM expects either all tool_use blocks to be processed or none — partial completion may produce confusing tool_result patterns. May require also injecting a synthetic `[deadline reached, remaining tools skipped]` user message before the next iteration.

### Option B: Short-circuit when remaining budget < TOOL_TIMEOUT_SECS

Before entering `process_tool_calls`, check if `Instant::now() + TOOL_TIMEOUT_SECS > deadline`. If yes, skip the entire batch and return DeadlineExceeded immediately.

**Tradeoff:** Simpler than Option A but more conservative — kills some iterations that would have succeeded.

### Option C: Tighten the documented bound, accept the looser worst case

Change the plan/solution doc to say `300s + 120s + (max_tool_burst × 30s)` — empirically bounded by the model's tool-use shape. Document that a misbehaving model can blow past 420s.

**Tradeoff:** Honest documentation but no behavior change. May be acceptable if the tool-burst case is rare in practice.

## Recommendation

Option A. The granularity gap is a real correctness issue, but the implementation is non-trivial because partial-batch dispatch needs a follow-up turn. Worth a separate ticket with proper plan + acceptance criteria.

## Definition of Done

- [ ] `process_tool_calls` accepts `deadline: Instant` and checks it between tool dispatches.
- [ ] Test: 20-tool-burst response with deadline 30s, asserts no more than ~3 tools dispatch before DeadlineExceeded.
- [ ] Worst-case bound in solution doc updated.

## Notes

Defer-or-decline rationale (mika#848 PR scope): the architect-groomed plan claimed `420s` as the bound; the implementation honors the plan as-written. Tightening the bound is scope expansion that requires re-grooming.
