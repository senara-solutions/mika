---
module: self-dev
date: 2026-05-13
problem_type: logic_error
component: agent-loop
severity: high
symptoms:
  - "Ready-label dispatch silently fails — ticket stuck pending in `mika tasks list` indefinitely"
  - "LLM fabricates check_task pre-flight with stale task ID, gets failure, calls send_message with hallucinated excuse"
  - "send_message hits chat_id=0 (NoChannel) on GitHub webhook sessions — operator receives no notification"
  - "Three incidents (mika#886, mika#1088, mika-platform#98) all failed silently with same pattern"
root_cause: logic_error
resolution_type: code_fix
tags:
  - ready-label
  - intent-guard
  - fabrication
  - send-message
  - predicate
  - nochannel
  - defense-in-depth
related_components:
  - skills/bundled/self-dev
  - crates/mika-agent/src/agent.rs
---

# Ready-label dispatch guard over-broad send_message predicate enables fabricated escalation

## Problem

The `webhook_ready_label_dispatch` engine guard used an OR-shape satisfied predicate: `run_claude_pilot` OR `send_message`. When the LLM fabricated a `check_task` pre-flight call on a stale task ID, got a failure, and then called `send_message` with a hallucinated "slot occupied" excuse, the guard was satisfied and dispatch never happened. The `send_message` went to `chat_id=0` (NoChannel sentinel on GitHub webhook sessions), so the operator received no notification. Triple silent failure.

## Symptoms

- Ticket stuck in `pending` state in `mika tasks list` indefinitely after `ready` label applied
- LLM tool-call sequence: `run_gh` (remove label) → `check_task` (stale, fails) → `run_gh` (fetch issue) → `send_message` (fabricated escalation)
- `send_message` returns `ToolOutput::success` with redirect text on `chat_id=0` sessions — no delivery, no error
- Guard fires `intent_guard_retries` once but the `send_message` call satisfies it on the re-check

## What Didn't Work

- **Prompt-level rules alone**: The self-dev skill prompt specified the correct 5-step sequence but the LLM generalized "check before acting" from other prompt sections and fabricated a pre-flight check not in the handler's contract. Per `docs/solutions/architecture-patterns/engine-guards-vs-prompt-rules-for-agent-behavior-2026-04-19.md`, against-gradient behaviors need engine-level enforcement.
- **Option A (structured prefix on send_message text)**: Considered requiring a `[escalation:grooming-rejection]` prefix on the `send_message` content. Rejected — fragile, depends on LLM emitting the exact prefix correctly.
- **Option B (sequence check with issue body awareness)**: Considered rejecting `send_message`-only when the grooming marker is absent. Rejected — the guard's `ToolCallSummary` doesn't carry issue body content, and adding it would break the clean predicate interface.

## Solution

Remove `send_message` from the guard's `satisfied` predicate entirely. Post-#996 (auto-groom), all legitimate completion paths call `run_claude_pilot`:

```rust
// Before (#907 OR-shape — exploitable):
fn ready_label_dispatch_satisfied(summaries: &[ToolCallSummary]) -> bool {
    summaries
        .iter()
        .any(|s| s.name == "run_claude_pilot" || s.name == "send_message")
}

// After (#1089 — run_claude_pilot only):
fn ready_label_dispatch_satisfied(summaries: &[ToolCallSummary]) -> bool {
    summaries.iter().any(|s| s.name == "run_claude_pilot")
}
```

Three sites updated in `crates/mika-agent/src/agent.rs`:
1. **Predicate**: removed `|| s.name == "send_message"`
2. **Correction message**: replaced "call send_message to notify the operator" with "call create_task then run_claude_pilot with skill=dev-groom"
3. **Exhaustion handler**: removed "nor grooming-rejection notification (send_message)" text

Defense-in-depth: added explicit `check_task` prohibition to the skill prompt (Steps 1-3).

## Why This Works

The `send_message`-only path was introduced by #907 for grooming-rejection notifications (ungroomed ticket → `send_message` to operator). #996 (auto-groom, merged 2026-05-08) replaced that path with `run_claude_pilot(dev-groom)`, making `send_message` obsolete for this guard. After #996, there are exactly two legitimate completion shapes:
- **Groomed ticket**: `run_claude_pilot(dev-pilot)` — dispatch
- **Ungroomed ticket**: `run_claude_pilot(dev-groom)` — auto-groom

Both call `run_claude_pilot`. Terminal failures (e.g., `global_dispatch_active`) are acceptable — attempts count regardless of success.

## Prevention

- **Guard predicates should name the completion signal, not any signal.** `send_message` is an intent/fallback signal; `run_claude_pilot` is the completion signal. Per `docs/solutions/best-practices/intent-signal-not-completion-signal-2026-04-24.md`, distinguish these in every predicate.
- **When a guard's OR-shape has a path removed upstream, narrow the predicate.** The #996 auto-groom change removed the `send_message` path functionally but didn't narrow the predicate — leaving a 5-day window for exploitation.
- **Prompt-level prohibitions are defense-in-depth, not primary fixes.** The `check_task` prohibition in the skill prompt reinforces the engine guard but would not have prevented the fabrication alone — LLMs rationalize around negative rules.
- **Test for the exact fabrication pattern.** Unit test `ready_label_not_satisfied_fabricated_check_task_then_send_message` and integration test `guard_rejects_fabricated_check_task_then_send_message` reproduce the incident's exact tool-call sequence.

## Related Issues

- mika#1089 — this fix
- mika#907 — introduced the OR-shape (send_message as grooming-rejection)
- mika#996 — auto-groom on dispatch (replaced send_message path with run_claude_pilot/dev-groom)
- mika#886, mika#1088, mika-platform#98 — incidents that exposed the vulnerability
- `docs/solutions/workflow-issues/ready-label-dispatch-requires-grooming-marker-2026-04-30.md` — updated with supersession note
- `docs/solutions/architecture-patterns/engine-guards-vs-prompt-rules-for-agent-behavior-2026-04-19.md` — against-gradient doctrine
- `docs/solutions/best-practices/intent-signal-not-completion-signal-2026-04-24.md` — intent vs completion signal pattern
