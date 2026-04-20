---
module: mika-agent::agent
date: 2026-04-20
problem_type: best_practice
component: assistant
severity: high
tags:
  - agent-loop
  - post-condition-guard
  - webhook
  - fabrication
  - endturn
  - self-dev
applies_when:
  - Adding new post-condition guards to the EndTurn chain
  - Debugging webhook fabrication (agent narrates instead of acting)
  - Understanding the guard ordering and skip_remaining_guards interactions
---

# Webhook Zero-Tools Guard: Structural Fabrication Prevention (#696)

## Context

When the agent receives a webhook event (message starting with `[GitHub]` prefix, injected by the gateway's `format_event_text()`), the expected behavior is to process the event by calling tools (e.g., `update_task_status`, `check_task`, `send_message`). However, under cognitive load or model drift, the LLM sometimes produces a text-only response that narrates about the webhook without actually processing it — a silent fabrication that leaves downstream state unupdated.

The self-dev system prompt (line 99) already contained a text rule: "You MUST make at least one tool call before your turn ends." This rule was violated in production (session `049e853b`, 2026-04-20) where the agent produced a Rust tutorial instead of handling a PR approval webhook. The duplicate second webhook was handled correctly, proving the agent *can* do it — the first was pure hallucination.

This is a textbook case of an against-gradient behavioral invariant (see `docs/solutions/architecture-patterns/engine-guards-vs-prompt-rules-for-agent-behavior-2026-04-19.md`): the LLM's turn-closure reflex fights the "must call tools" rule. Engine-level enforcement is the correct layer.

## Guidance

### Implementation Pattern

The guard follows the established EndTurn post-condition pattern (same as guards #1–#5):

1. **State variable:** `webhook_zero_tools_retry_done: bool` — initialized `false` before the loop
2. **Condition:** `!skip_remaining_guards && EndTurn && !retry_done && user_input_text.starts_with("[GitHub]") && !all_tool_summaries.iter().any(|s| s.success)`
3. **Action:** Set retry flag, push assistant response, push corrective user message, `continue`
4. **Position:** Guard #6 — after fabricated-action-claim (#5), before persistence-eval (#7)

### Key Design Decisions

- **`all_tool_summaries.iter().any(|s| s.success)` not `tools_called.is_empty()`:** Distinguishes "tried tools but they failed" (agent attempted processing — let through) from "never called any tool" (pure fabrication — reject). This prevents punishing the agent for external failures.
- **Respects `skip_remaining_guards`:** When the PR review early-accept (#3b) fires, this guard is skipped — consistent with guards #4–#7. If the agent successfully posted a PR review, forced continuation would risk duplicate actions.
- **Single retry:** Like all guards, fires at most once. If the agent still doesn't call tools after the re-prompt, the turn ends normally with the text saved. This prevents infinite loops.
- **No regex needed:** The `[GitHub]` prefix is a literal string injected by the gateway — a simple `starts_with()` check is deterministic and fast. No `LazyLock<Regex>` required.

### Guard Ordering Rationale

Position #6 (between fabricated-action-claim and persistence-eval) because:
- Must be after text-tool-call (#1) and prose-tool-call (#2) detectors — those might convert text to actual tool calls on retry, which would satisfy the webhook guard's condition
- Must be after fabricated-action-claim (#5) — that guard catches URL-containing fabrications with zero tools, which is a subset of this guard's condition. Letting #5 fire first produces a more specific corrective message when applicable
- Before persistence-eval (#7) because webhook processing is more urgent than knowledge persistence

## Why This Matters

Webhook fabrication is a **silent failure mode**. The agent appears to have responded (text is generated and saved), but no state was updated. In the observed incident, a PR approval event was effectively dropped — if the duplicate hadn't fired, the task would have stalled indefinitely. This guard converts a silent failure into a retry opportunity, making fabrication observable (via the `warn!` log) and self-correcting (via the re-prompt).

## Examples

**Guard fires:**
```
User: [GitHub] PR review (approved) on senara-solutions/mika#694 by reviewer...
Agent (text-only): "The PR has been approved. Everything looks good."
→ Guard rejects: zero successful tool calls on webhook turn
→ Re-prompt: "[Your response was rejected because you received a GitHub webhook event...]"
Agent (retry): calls check_task, then responds with actual status
```

**Guard skips (tool called):**
```
User: [GitHub] PR review (approved) on senara-solutions/mika#694 by reviewer...
Agent: calls update_task_status → "Updated task status for PR approval."
→ Guard does not fire: all_tool_summaries has a successful entry
```

## Related

- `docs/solutions/architecture-patterns/engine-guards-vs-prompt-rules-for-agent-behavior-2026-04-19.md` — foundational principle
- `docs/solutions/architecture-patterns/fabricated-action-claim-guard.md` — sibling guard pattern (#5)
- `docs/solutions/prompt-engineering/2026-04-12-tighten-webhook-qa-pass-entry-point.md` — the prompt-level fix this guard supersedes
- `docs/solutions/prompt-enforcement-structural-guards.md` — early-accept pattern and guard interaction docs
- Issue: #696
