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

### ⚠️ Superseded by #1089 — `send_message` removed from OR-shape

The OR-shape was narrowed by mika#1089 (2026-05-13). Post-#996 (auto-groom via `run_claude_pilot(dev-groom)`), the `send_message`-only path became obsolete — all legitimate completion paths call `run_claude_pilot`. The over-broad match was exploited by LLM fabrication: sonnet fabricated a `check_task` pre-flight with a stale task ID, got a failure, then called `send_message` with a hallucinated "slot occupied" excuse. The guard accepted `send_message` as valid completion, and the `send_message` hit `chat_id=0` (NoChannel) — triple silent failure.

The predicate now requires `run_claude_pilot` attempted:

```rust
fn ready_label_dispatch_satisfied(summaries: &[ToolCallSummary]) -> bool {
    summaries.iter().any(|s| s.name == "run_claude_pilot")
}
```

### Exhaustion handler

The exhaustion handler (fires when the guard retried but `run_claude_pilot` was not called) says "dispatch (run_claude_pilot) did not complete." The `error!` log and operator notification via `message_sender.send()` are unchanged.

## Why This Matters

Without grooming enforcement, every operator slip costs a wasted claude-pilot session and risks off-target commits. The `ready` label was designed as a consent signal but must also encode grooming completeness. The grooming marker (`> - **Plan:**`) is the canonical evidence that the architect pipeline ran — it is written by `dev-groom` as its terminal output.

The defense-in-depth layering follows the established pattern from `docs/solutions/architecture-patterns/engine-guards-vs-prompt-rules-for-agent-behavior-2026-04-19.md`: "Don't dispatch without grooming" is against-gradient behavior (the LLM's default is to dispatch when triggered), so engine-level enforcement is required, not prompt rules alone.

## When to Apply

- When adding new dispatch prerequisites to the ready-label flow: the guard now requires `run_claude_pilot` — any new path must ultimately call `run_claude_pilot`. Do NOT re-add `send_message` to the predicate (see #1089 for why). Update the correction message to present the new path, update the exhaustion handler text
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

**After (#907, superseded by #1089):**
```rust
// #907 OR-shape (SUPERSEDED by #1089 — send_message removed):
// fn ready_label_dispatch_satisfied(summaries: &[ToolCallSummary]) -> bool {
//     summaries.iter().any(|s| s.name == "run_claude_pilot" || s.name == "send_message")
// }

// #1089 — run_claude_pilot only (post-#996 auto-groom makes send_message obsolete):
fn ready_label_dispatch_satisfied(summaries: &[ToolCallSummary]) -> bool {
    summaries.iter().any(|s| s.name == "run_claude_pilot")
}
```

**Skill prompt grooming check (Step 3, updated by #996 and #1089):**
After fetching the issue body, scan for `> - **Plan:**`. If absent, auto-groom via `create_task` + `run_claude_pilot(dev-groom)`. If present, dispatch via `create_task` + `run_claude_pilot(dev-pilot)`. Both paths call `run_claude_pilot` and satisfy the guard.

## Related

- mika#841 — introduced the `ready` label as consent signal
- mika#846 — added the `webhook_ready_label_dispatch` engine guard
- mika#901 — the incident that exposed the missing grooming gate
- mika#907 — this fix (OR-shape introduction)
- mika#996 — auto-groom on dispatch (replaced send_message rejection with run_claude_pilot/dev-groom)
- mika#1089 — narrowed OR-shape to run_claude_pilot-only (fabrication defense)
- `docs/solutions/architecture-patterns/engine-guards-vs-prompt-rules-for-agent-behavior-2026-04-19.md` — against-gradient classification
- `docs/solutions/architecture-patterns/intent-precondition-registry-guard-generalization-2026-04-21.md` — registry pattern
- `docs/solutions/workflow-issues/ready-label-dispatch-handler-regression-2026-04-27.md` — prior regression in the same handler
