---
title: "self-dev close-out silently ends turn on task_not_found instead of recovering"
date: 2026-04-20
category: logic-errors
module: self-dev
problem_type: logic_error
component: tooling
symptoms:
  - "update_task_status returns task_not_found but agent ends the turn without retrying"
  - "child task left in_progress after PR merge, blocking milestone advancement"
  - "correct task ID visible in list_tasks output but agent does not use it"
root_cause: logic_error
resolution_type: documentation_update
severity: medium
tags:
  - self-dev
  - task-not-found
  - hallucinated-uuid
  - update-task-status
  - close-out
  - milestone
  - prompt-rule
---

# self-dev close-out silently ends turn on task_not_found instead of recovering

## Problem

When self-dev's Step 6 (close-out) calls `update_task_status` with a hallucinated task UUID, the tool returns a structured `task_not_found` error. The agent proceeds to call `list_tasks` and `search_memory` in subsequent steps — the correct ID is visible in the output — but the turn ends without retrying `update_task_status` with the recovered ID. The child task is left stuck in `in_progress` after its PR has already merged, blocking milestone advancement and poisoning downstream state.

## Symptoms

- `update_task_status` returns `{"error": "task_not_found", "field": "task_id", ...}` during Step 6 close-out
- Agent calls `list_tasks` in a subsequent step — correct task ID is visible in output
- Turn ends without a successful `update_task_status` call
- Child task remains `in_progress` despite its PR being merged
- Milestone stalls: heartbeat/nudge sees an `in_progress` task with no work pending

## What Didn't Work

- The agent's natural recovery behavior (calling `list_tasks` and `search_memory` after the error) was necessary but insufficient — it found the correct ID but didn't use it to retry because the prompt had no explicit instruction to do so.
- The hallucination pattern (correct 8-char UUID prefix, wrong suffix) is a generic LLM characteristic across providers (observed with kimi-k2.5 but not provider-specific). Engine-level auto-resolve was considered and rejected because the recovery needs agent context (which issue, which `reference_url`) to identify the correct task from `list_tasks` output.

## Solution

Added a mandatory recovery rule to `skills/bundled/self-dev/system_prompt.md` in Step 6, immediately after the `update_task_status` call instruction:

**Recovery rule structure:**
1. On `task_not_found` from `update_task_status`, call `list_tasks(status="in_progress")` and scan for a `reference_url` matching the current issue. Fall back to `list_tasks(status="pending")` if no match.
2. **GATE:** If exactly one task matches, retry `update_task_status` with the recovered `task_id` and the original status + metadata.
3. If zero matches: escalate to Vincent (do NOT silently end the turn).
4. If multiple matches: escalate with the candidate list.

The rule uses structural gate language and cites the incident by trace ID and date to anchor LLM behavior — the same pattern used in the existing Calibration Rules section.

**Eval harness test** (`crates/mika-agent/tests/eval/test_task_not_found_retry.rs`) verifies the tool call sequence: `update_task_status` (fail with `task_not_found`) -> `list_tasks` -> `update_task_status` (success with recovered ID). Includes a noise task to verify `reference_url` matching selectivity, and positive assertion that the retry call uses the correct `task_id`.

## Why This Works

The failure mode is a hallucinated UUID — the agent has the right information (which issue it's working on) but fabricates the task ID suffix after context compaction. The recovery path already exists naturally (the agent calls `list_tasks` after the error), but without an explicit prompt instruction to retry, the turn ends once the agent "acknowledges" the error. The rule converts the implicit recovery into a mandatory gate: do NOT end the turn until `update_task_status` succeeds or escalation fires.

This is a with-gradient behavior per the engine-guards-vs-prompt-rules decision framework — the agent already has the information and tends toward the correct action; it just needs explicit instruction to complete the loop. An engine guard would be appropriate only if this prompt rule fails repeatedly across multiple providers.

## Prevention

- **Structural gates over prose instructions:** When a tool failure must be recovered from (not just logged), use the explicit GATE pattern with numbered steps and a "do NOT end the turn" imperative. Prose like "you should try to recover" is too soft for LLMs under context pressure.
- **Incident citations anchor behavior:** Referencing a specific trace ID and date makes the rule harder for LLMs to generalize away or skip. The Calibration Rules section in self-dev uses this pattern throughout.
- **Pattern mirrors existing prompt conventions:** Step 2 already teaches `list_tasks` + `reference_url` matching. The recovery rule references the same pattern, making it consistent and reinforcing the behavior.

## Related Issues

- [mika#693](https://github.com/senara-solutions/mika/issues/693) — the issue tracking this fix
- `docs/solutions/best-practices/uuid-validation-at-tool-boundary.md` — three-layer UUID validation chain that produces the structured `task_not_found` error
- `docs/solutions/architecture-patterns/engine-guards-vs-prompt-rules-for-agent-behavior-2026-04-19.md` — decision framework for prompt rule vs engine guard
- `docs/solutions/architecture-patterns/phantom-retry-guard-active-dispatch-metadata-validation.md` — related guard that rejects retry-semantic metadata during active dispatches
