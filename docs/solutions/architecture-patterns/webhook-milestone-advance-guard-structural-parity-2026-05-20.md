---
title: "Webhook milestone advance guard — structural parity with callback path"
module: agent-core, self-dev, server
date: 2026-05-20
problem_type: best_practice
component: tooling
severity: medium
tags: [engine-guard, milestone, webhook, intent-guard, against-gradient, structural-enforcement, pr-closed, deploy-hook]
applies_when:
  - Adding a new event-source entry point to the milestone/project advancement loop
  - A prompt-level "advance OR halt" obligation needs structural backstop enforcement
  - The same behavioral invariant applies across multiple event sources (callback, webhook, heartbeat) but each source has a different user message shape
---

# Webhook milestone advance guard — structural parity with callback path

## Context

mika#991 established the `callback_milestone_advance` inline guard: when a callback turn completes for a task whose parent is a milestone/project, the engine requires the LLM to either advance the queue (`run_claude_pilot`) or halt (`update_task_status` on parent with `blocked`/`completed`). Without the guard, the LLM's trained default is "acknowledge and close the turn" — the deliberation-stall pattern where the agent posts a confirmation question instead of acting.

mika#1208 extended this obligation to the webhook path (`pull_request.closed(merged:true)` events handled by `self-dev-webhook-qa` § Path A step 5.5) but shipped with prompt-only enforcement and an explicit `⚠ ENGINE GUARD PENDING mika#1218` warning. Per `docs/solutions/architecture-patterns/engine-guards-vs-prompt-rules-for-agent-behavior-2026-04-19.md`, milestone advancement is an against-gradient behavior class — prompt rules partially work but drift under cognitive load.

mika#1218 closes the parity gap by adding the structural engine guard for the webhook path.

## Guidance

### Two-layer architecture: server handler + inline guard

The webhook guard requires two cooperating layers because the agent loop (`run_loop`) doesn't have access to the DB context needed to determine whether a webhook turn is milestone-scoped:

**Layer 1 — Server-side marker injection** (`server::milestone_context_handler`):
Runs in the `handlers.rs::process_message_inner` chain AFTER `ci_failure_handler`. For `[GitHub] PR closed:` webhooks with `Merged: true`, correlates the PR URL to a task via `find_active_task_by_pr_url`, checks if the task has a milestone/project parent, and prepends `[milestone-parent: <parent_id>]\n` to the user message. The marker is the same format used by `run_silent_agent` for callbacks — the agent loop doesn't need to know the event source.

**Layer 2 — Inline guard in `run_loop`** (`agent.rs`):
Triggers on `contains(MILESTONE_PARENT_MARKER) && contains("[GitHub] PR closed:")`. Shares `extract_milestone_parent_id` and `MILESTONE_PARENT_MARKER` with the callback guard — single parser, two trigger predicates. Three satisfaction paths:
- **Path A (advance):** `run_claude_pilot` or `run_claude_pilot_groom`
- **Path B (halt/finish):** `update_task_status` targeting parent with `blocked`/`completed`
- **Path C (deploy hook):** `deploy_mika` + `send_message` (the 5.5.b deploy-hook ack path)

### Why inline, not in the `INTENT_GUARDS` const array

The `INTENT_GUARDS` registry has `trigger: fn(&str) -> bool` and `satisfied: fn(&[ToolCallSummary]) -> bool` — pure function pointers with no dynamic context channel. Both the callback and webhook guards need the `parent_task_id` extracted from the user message to distinguish parent-targeting `update_task_status` calls from child-targeting ones. This is the same constraint that forced `callback_milestone_advance` (#991) to be inline. A registry redesign to support dynamic context would be a substantially larger ticket.

### Trigger mutual exclusivity

The callback and webhook guards fire on mutually exclusive user message shapes:
- Callback: `starts_with("[callback:")` — injected by `run_silent_agent`
- Webhook: `contains("[GitHub] PR closed:")` — injected by the gateway

No single user message can satisfy both triggers. The `contains` form (vs `starts_with`) for both the marker and the webhook prefix provides resilience against handler-chain reordering that might prepend other enrichments.

### Handler chain ordering matters

The milestone-context handler runs AFTER `ci_failure_handler` in the `if req.channel == "github"` block. This is deliberate: CI-failure pre-digests that replace `req.text` via `Handled` would lose a prepended marker. Since CI failure events are disjoint from PR-closed events (they match `[GitHub] Check suite failure/timed_out`, not `[GitHub] PR closed:`), the handlers self-select on event type and don't interfere.

### Fail-open policy

The handler returns `Passthrough { enrichment: None }` on any DB error. The prompt-level "advance OR halt" obligation in `self-dev-webhook-qa` step 5.5 remains as the fallback enforcement layer. The engine guard is defense-in-depth, not the sole enforcement.

## Why This Matters

Without the webhook guard, the LLM can acknowledge a PR-merge webhook and close the turn without advancing the milestone queue. This leaves the milestone stalled until the `PostCallbackAdvance` backstop fires (if one exists) or the heartbeat detects the stall. The deliberation-stall pattern was observed 3+ times in the callback path before #991 added structural enforcement; the webhook path carries the same risk profile.

The guard also establishes per-event-source symmetry: callback turns (#991) and webhook turns (#1218) now have the same enforcement shape. A future unified `MilestoneAdvance` SilentTrigger (consolidating both event sources) can build on this symmetry.

## When to Apply

- When adding a new event-source entry point (e.g., API endpoint, scheduled trigger, manual CLI command) that reaches the milestone advancement decision point
- When porting the "advance OR halt" obligation to a new context where the LLM must make a queue-progress decision
- When the same behavioral invariant applies across multiple event sources but the user message shape differs per source — the two-layer pattern (server-side marker injection + inline guard with shared parser) is the reusable architecture

## Examples

### Before — prompt-only enforcement (mika#1208)

```markdown
# In self-dev-webhook-qa/system_prompt.md:
⚠ **ENGINE GUARD PENDING mika#1218** — this gate is prompt-prose-only
until mika#1218 lands. The LLM's trained default is "acknowledge and
close the turn" rather than "advance the queue."
```

Result: the obligation held ~80% of the time. Under cognitive load (long milestone chains, concurrent webhooks), the LLM occasionally posted a confirmation question instead of dispatching.

### After — structural guard (mika#1218)

The engine intercepts the EndTurn, detects the milestone-parent marker, checks satisfaction paths, and re-prompts with a corrective message if none are satisfied. Single-retry semantics: the guard fires once; if the retry also fails, EndTurn is accepted with a WARN log. The prompt-level obligation is preserved as the first-line defense; the guard is the structural backstop.

## Related

- mika#991 — `callback_milestone_advance` inline guard (direct precedent)
- mika#1208 — HOLD re-entry semantics (prompt-only fix this guard backstops)
- `docs/solutions/architecture-patterns/engine-guards-vs-prompt-rules-for-agent-behavior-2026-04-19.md` — the doctrine this guard follows
- `docs/solutions/architecture-patterns/intent-guard-predicate-sharing-2026-05-14.md` — shared predicate pattern between guard layers
