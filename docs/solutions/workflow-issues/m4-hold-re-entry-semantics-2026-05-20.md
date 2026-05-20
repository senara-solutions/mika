---
module: self-dev, agent-core
tags: [milestone-cascade, webhook, hold-state, engine-guards-vs-prompts]
problem_type: workflow_issue
category: workflow-issues
applies_when:
  - introducing a new completion path that advances a queue but cannot trivially reach the source handler's queue-management logic
  - adding a state (like HOLD) that blocks an iteration loop and requires a different event source to resume it
  - prompt-only enforcement of against-gradient LLM behavior that the engine should eventually guard structurally
---

# M4 HOLD Re-Entry Semantics (mika#1208)

## Problem

mika#789 introduced a HOLD state to M4 step 2.5 when `pr_merge_with_gate` returns `auto_merge_enabled`: the child task stays `in_progress`, and the loop should not advance until the PR actually merges. However, #789 did not define what happens when M4 re-enters against a child already in HOLD. The prompt claimed "re-enter M4 step 3 for this child" on webhook arrival, but no code path or prompt instruction backed that claim.

The merge webhook handler (`self-dev-webhook-qa` Path A) marked the child `completed` and stopped — it never advanced the milestone. This left M4 stuck: the serial execution loop had no mechanism to resume after a HOLD.

**Incident context:** mika#727 — KG milestone #14, PR #726 had auto-merge enabled but CI failed; next ticket #689 was dispatched against missing code. This is the exact failure class HOLD was introduced to prevent, but without re-entry semantics, HOLD could cause a different failure: permanent stall.

## Design Decision: Option (a) — HOLD Ends the Turn

**Chosen:** Option (a) — HOLD is a turn boundary. The webhook handler owns resume.

When `pr_merge_with_gate` returns `auto_merge_enabled`:
1. M4 step 2.5 persists the HOLD note via `update_task_status` (or `update_task_metadata` as fallback)
2. The turn ends immediately — no looping, no dispatching
3. The `pull_request.closed(merged: true)` webhook handler (Path A step 5.5) is responsible for verifying merge, advancing the milestone queue, and dispatching the next child

**Rejected:** Option (b) — iterate-over-non-HOLD pattern. This would let M4 dispatch the next child while the prior child's PR is unmerged, re-introducing the parallelism class mika#727 exposed. The "no-op iteration until webhook" approach also had no bound on how long it loops (stuck PRs, GH outages) and would need a HOLD-timeout escape — more new control flow for less safety.

## Key Design Elements

### Idempotent HOLD Re-Entry

If a `PostCallbackAdvance` backstop fires while the child is still HOLD (webhook hasn't arrived yet), the turn is a no-op:
- No re-dispatch
- No status change
- Agent surfaces "HOLD child not yet merged" notification
- Parent milestone blocked for operator review

This is correct — a `PostCallbackAdvance` asking "why did you not advance?" deserves the honest answer "because the child is still HOLD." Slow merges with CI failures or GitHub outages are exactly when operator attention is needed.

### Webhook Milestone Advance (Step 5.5)

The webhook handler Path A gained a new step 5.5 that mirrors the callback handler's "advance OR halt" contract:
- **5.5.a:** Verify PR actually merged via `run_gh pr view`
- **5.5.b:** Check deploy-hook labels → call `deploy_mika` if present
- **5.5.c:** Find and dispatch next pending child, or surface operator notification if last child

### Prompt-Only Enforcement (Fragility Acknowledgment)

The "advance OR halt" obligation in the webhook handler is prompt-prose-only until mika#1218 lands a `webhook_milestone_advance` INTENT_GUARD. This is the same against-gradient-behavior class as `callback_milestone_advance` (mika#991): the LLM's trained default is "acknowledge and close the turn" rather than "advance the queue."

Both prompt diffs carry explicit `ENGINE GUARD PENDING mika#1218` warnings. mika#1218's AC3 removes them when the engine guard lands.

## Cross-Link

This is the next data point after `docs/solutions/architecture-patterns/engine-guards-vs-prompt-rules-for-agent-behavior-2026-04-19.md` in the gradient from prompt-only to engine-enforced behavior. The pattern: prompt-only enforcement ships first for fast iteration, coupled-follow-up files the engine guard, the follow-up's AC removes the warning prose.

## Related

- mika#789 — parent ticket introducing HOLD state
- mika#1218 — coupled follow-up for `webhook_milestone_advance` INTENT_GUARD
- mika#991 — `callback_milestone_advance` guard (the mirror pattern this plan transplants)
- mika#727 — incident motivating the HOLD state
