---
module: self-dev
date: 2026-04-30
problem_type: workflow_issue
component: agent-loop
severity: high
tags:
  - ready-label
  - grooming
  - dispatch-safety
  - intent-guard
  - defense-in-depth
related_components:
  - skills/bundled/self-dev
  - skills/bundled/dev-groom
  - crates/mika-agent/src/agent.rs
---

# Ready-label dispatch requires grooming marker before run_claude_pilot

## Context

The `ready` label is the canonical positive-consent signal for autonomous dispatch (mika#841). The `webhook_ready_label_dispatch` intent-precondition guard (mika#846) enforces that `run_claude_pilot` is attempted on ready-label webhook turns. However, consent was the only gate — grooming was not enforced.

mika#901 exposed the gap: the operator applied `ready` to an ungroomed ticket (no `> - **Plan:**` callout in the issue body, no architect review). mika-dev launched claude-pilot, which edited 3 skill prompts without architect input and exited without a PR. Cost: ~$1.70, ~305 seconds, plus off-target commits.

## Guidance

### Two-layer defense-in-depth

**Layer 1 — Skill-prompt PRE-FLIGHT (primary gate):** The self-dev Ready-Label Dispatch handler checks for the `> - **Plan:**` callout in the fetched issue body (Step 3) before proceeding to `create_task` and `run_claude_pilot`. If absent, the agent calls `send_message` to notify the operator and stops the turn.

**Layer 2 — Engine guard OR-shape (structural backstop):** The `webhook_ready_label_dispatch` intent-precondition guard's `satisfied` predicate accepts EITHER `run_claude_pilot` attempt (dispatch path) OR `send_message` call (grooming-rejection path). The correction message presents both valid paths.

### The `send_message` match is intentionally over-broad

Any `send_message` call satisfies the guard — not just grooming-rejection notifications. This is acceptable because:

1. The prompt is the primary grooming gate; the engine guard is a backstop
2. The trigger is very specific (only `[GitHub] Issue labeled ready on` events)
3. Content-based discrimination on truncated `input_summary` is fragile
4. Same pattern as the `callback_terminal_action` guard's `send_message` matching

### Exhaustion handler covers the OR-shape

The exhaustion handler (fires when the guard retried but neither tool was called) now says "neither dispatch nor grooming-rejection notification completed" — accurate for both paths. The `error!` log and operator notification via `message_sender.send()` are unchanged.

## Why This Matters

Without grooming enforcement, every operator slip costs a wasted claude-pilot session and risks off-target commits. The `ready` label was designed as a consent signal but must also encode grooming completeness. The grooming marker (`> - **Plan:**`) is the canonical evidence that the architect pipeline ran — it is written by `dev-groom` as its terminal output.

The defense-in-depth layering follows the established pattern from `docs/solutions/architecture-patterns/engine-guards-vs-prompt-rules-for-agent-behavior-2026-04-19.md`: "Don't dispatch without grooming" is against-gradient behavior (the LLM's default is to dispatch when triggered), so engine-level enforcement is required, not prompt rules alone.

## When to Apply

- When adding new dispatch prerequisites to the ready-label flow: follow the OR-shape pattern — widen the `satisfied` predicate, update the correction message to present all valid paths, update the exhaustion handler text
- When modifying the grooming marker format: the self-dev skill prompt (Step 3) and the dev-groom output contract must stay in sync — there is no integration test coupling them
- When studying the `NoChannel` gap: `send_message` on GitHub webhook sessions (`chat_id=0`) returns `ToolOutput::success` but the operator never receives the notification — this is a pre-existing architectural gap affecting all notification-as-rejection paths

## Examples

**Before (consent-only gate):**
```rust
// Only run_claude_pilot satisfied the guard
fn ready_label_dispatch_satisfied(summaries: &[ToolCallSummary]) -> bool {
    summaries.iter().any(|s| s.name == "run_claude_pilot")
}
```

**After (consent + grooming gate):**
```rust
// Either dispatch or grooming-rejection notification satisfies the guard
fn ready_label_dispatch_satisfied(summaries: &[ToolCallSummary]) -> bool {
    summaries
        .iter()
        .any(|s| s.name == "run_claude_pilot" || s.name == "send_message")
}
```

**Skill prompt grooming check (new Step 3):**
After fetching the issue body, scan for `> - **Plan:**`. If absent, call `send_message` with rejection notification including the issue reference, reason, and recovery instruction (run `/mika-groom-ticket`, then re-add `ready`). Do NOT proceed to `create_task` or `run_claude_pilot`.

## Related

- mika#841 — introduced the `ready` label as consent signal
- mika#846 — added the `webhook_ready_label_dispatch` engine guard
- mika#901 — the incident that exposed the missing grooming gate
- mika#907 — this fix
- `docs/solutions/architecture-patterns/engine-guards-vs-prompt-rules-for-agent-behavior-2026-04-19.md` — against-gradient classification
- `docs/solutions/architecture-patterns/intent-precondition-registry-guard-generalization-2026-04-21.md` — registry pattern
- `docs/solutions/workflow-issues/ready-label-dispatch-handler-regression-2026-04-27.md` — prior regression in the same handler
