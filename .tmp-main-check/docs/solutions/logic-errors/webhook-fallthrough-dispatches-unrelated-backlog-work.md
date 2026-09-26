---
module: skills-executor
date: 2026-04-15
problem_type: logic_error
component: tooling
severity: high
symptoms:
  - "mika-dev dispatches claude-pilot on unrelated issues when receiving a GitHub webhook"
  - "list_tasks and create_task called during webhook turns that should be informational"
  - "Parallel claude-pilot sessions launched for different repos in the same turn"
  - "Stale pending tasks left behind from abandoned dispatch attempts"
root_cause: missing_workflow_step
resolution_type: code_fix
tags:
  - webhook
  - dispatch-guard
  - self-dev
  - scope-control
  - defense-in-depth
  - claude-pilot
  - long-running
---

# Webhook Fallthrough Dispatches Unrelated Backlog Work

## Problem

When mika-dev received a GitHub webhook (e.g., `pull_request_review.submitted`) that didn't keyword-match a specific webhook handler skill (`self-dev-webhook-qa`, `self-dev-webhook-ci`), the `self-dev` skill's always-on generic workflow took over. The generic workflow's Steps 1-3 instructed the agent to: understand the issue, track the task via `list_tasks`, and immediately launch `run_claude_pilot`. This caused the agent to scan the backlog and dispatch claude-pilot on completely unrelated tickets.

## Symptoms

- `pull_request_review.submitted` webhook for PR #38 triggered backlog scanning
- Agent called `list_tasks` and found unrelated issues #571, #572
- Agent created tasks and dispatched `run_claude_pilot` for both unrelated issues in the same turn
- Two parallel claude-pilot sessions launched against different repos simultaneously
- Previous dispatch waves left orphaned `pending` tasks that never transitioned

## What Didn't Work

- **Prompt-only SCOPE RULE** on the Callback Entry Point prevented backlog scanning during callbacks, but webhook turns had no equivalent guardrail
- **Skill decomposition** (`self-dev-webhook-qa`, `self-dev-webhook-ci`) handled specific event types but left a gap for unmatched events that fell through to the always-on `self-dev` prompt
- **Per-work-item dispatch guard** (`validate_dispatch_readiness`) prevented double-dispatch on the same task but allowed dispatches to different tasks in the same turn
- The dangling reference at line 141 of `self-dev/system_prompt.md` pointed to a "Webhook Entry Point" section that had been decomposed into the separate `self-dev-webhook-qa` skill but was never updated

## Solution

Defense-in-depth fix with three layers: prompt-level scope control, engine-level global dispatch guard, and per-turn dispatch cap.

### 1. Prompt: Webhook Fallthrough Section

Added a `### Webhook Fallthrough (no keyword-matched handler)` section to `mika-skills/self-dev/system_prompt.md` with a SCOPE RULE that explicitly prohibits `list_tasks` backlog scans, `create_task`, and `run_claude_pilot` on webhook turns where no specific handler skill activated. Also added Calibration Rule 9 encoding this incident.

### 2. Engine: Global Active-Dispatch Guard

Added a third check to `validate_dispatch_readiness()` in `executor.rs`: query `has_active_callback_tasks_excluding(task_id, agent_id)` to detect if ANY other task already has an active callback child. Rejects with `global_dispatch_active` error, scoped to the requesting agent's `agent_id` to avoid cross-agent false positives in team/delegate scenarios.

### 3. Engine: Per-Turn Dispatch Counter

Added `dispatch_count: AtomicU32` to `LongRunningContext`, initialized to 0 per agent turn. The counter is checked before dispatch (rejects with `dispatch_limit_exceeded` if > 0) and incremented right before subprocess spawn — after all validation and task creation succeed. This placement avoids leaving the counter stuck at 1 if `create_task` or path validation fails.

### 4. Health: Stale Pending Detection

Added `stale_pending` anomaly type to `get_task_health_summary()`: flags manual tasks in `pending` status for >24 hours with no callback child, surfacing items that were created but never dispatched.

## Why This Works

The root cause was a structural gap in the self-dev skill prompt: no webhook-specific entry point existed for unmatched webhook events, so they fell through to the generic orchestrator workflow which always scans the backlog and dispatches.

The prompt fix (Layer 1) addresses the immediate gap. But per institutional learnings from the #522 incident and the dispatch-readiness guard pattern, "tool boundaries are the only reliable enforcement — soft advisory strings from tools are ignored by LLMs under recovery load." The engine guards (Layers 2-3) provide structural backstops that cannot be bypassed by prompt non-adherence:

- The global guard ensures at most one active dispatch across all tasks per agent
- The per-turn counter ensures at most one dispatch per conversation turn
- Both return structured JSON errors that explain why dispatch was rejected, so the LLM can understand and comply

## Prevention

1. **When decomposing a skill into sub-skills**: Always add a fallthrough handler to the parent always-on skill that explicitly tells the agent what to do when no sub-skill keyword-matches. The absence of instructions is not equivalent to "don't act."

2. **When adding prompt-level SCOPE RULES**: Apply them to ALL entry points, not just the ones that currently exist. New entry points (webhooks, events, triggers) need their own scope rules.

3. **When relying on prompt adherence for resource-costly actions**: Add a structural engine guard as a backstop. Prompt instructions are the first line of defense; tool boundaries are the last.

4. **When adding per-turn limits**: Place the counter increment at the latest possible point (right before the irreversible action) to avoid counter stuck-at-1 bugs when intermediate validation steps fail.

## Related

- [mika#583](https://github.com/senara-solutions/mika/issues/583) — the issue this fix addresses
- `docs/solutions/agent-quality/2026-04-11-mika-dev-verdict-misclassification-pr-522.md` — prior incident establishing the "tool boundaries over prompt instructions" pattern
- `docs/solutions/architecture-patterns/dispatch-readiness-guard-long-running-status-validation.md` — the dispatch-readiness guard pattern this fix extends
- `docs/solutions/workflow-patterns/2026-04-10-self-dev-skill-decomposition.md` — the skill decomposition that created the gap
