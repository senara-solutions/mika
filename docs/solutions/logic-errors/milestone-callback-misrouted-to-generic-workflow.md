---
title: Milestone callback misrouted to Generic Workflow — missing prompt-level routing
date: 2026-04-17
category: logic-errors
module: skills-executor, self-dev
problem_type: logic_error
component: tooling
symptoms:
  - "Callback turn after claude-pilot child completion dispatches new work instead of resuming milestone loop"
  - "Orphan work items created outside the milestone tree during milestone execution"
  - "Milestone loop never advances past child #1"
root_cause: missing_workflow_step
resolution_type: documentation_update
severity: high
tags: [milestone, callback, routing, self-dev, prompt-engineering, parent-task-id, generic-workflow]
---

# Milestone callback misrouted to Generic Workflow — missing prompt-level routing

## Problem

During the first milestone dispatch (mika#6), claude-pilot completed child #1 (mika#582). The callback arrived as a `SilentTrigger::Callback` turn containing "mika#582" as an issue reference. mika-dev's LLM pattern-matched this to the Generic Workflow ("implement mika#582") instead of following the Callback Entry Point back to Step M4 (serial execution loop). The milestone loop never advanced to child #2.

Separately, all 3 child work items were created without `parent_task_id` because the agent follows JSON examples literally, and Step M3 used bullet-list format without an explicit JSON code block.

## Symptoms

- Callback turn created an orphan work item for "mika#582" outside the milestone tree
- Agent dispatched `run_claude_pilot` 4 times for the same child issue
- Milestone parent work item was never updated past child #1
- `check_work_item` on children showed no `parent_task_id` set

## What Didn't Work

- The engine's generic callback framing (mika#313) correctly surfaces `parent_task_id` in the callback context. But the self-dev prompt's Callback Entry Point had no instructions to detect milestone context and route accordingly — the engine provides the data, but the skill prompt must act on it.
- Step M3 described `parent_task_id` in the bullet-list parameters, but the agent consistently omitted it because it was not shown in a JSON example block. Prose instructions are weaker than JSON examples for tool call compliance.

## Solution

Two prompt changes to `skills/bundled/self-dev/system_prompt.md`:

### 1. Milestone/project context check in Callback Entry Point

Added a mandatory check block before the success/failure/pipeline-failure handlers:

1. Call `check_work_item(task_id)` on the callback's work item to get `parent_task_id`
2. If `parent_task_id` exists, call `check_work_item(parent_task_id)` on the parent
3. If parent's `type` is `'milestone'` or `'project'`, route to Step M4/P4 after completing the success/failure handling

Key detail: the routing to M4/P4 only happens when the child reaches a **terminal state** (completed, blocked, failed after retries exhausted). Non-terminal paths (e.g., pipeline-failure retry) follow their existing "wait for callback" flow — the next callback re-enters the context check.

Negative instructions prevent the LLM from misinterpreting the callback's issue reference as a new dispatch trigger.

### 2. JSON code blocks for create_work_item in Step M3 and Step P3

Replaced bullet-list format with explicit JSON examples showing all fields including `parent_task_id`:

```json
{
  "type": "issue",
  "parent_task_id": "<milestone_wi>",
  "label": "<repo>#<issue_number>",
  "reference_url": "https://github.com/senara-solutions/<repo>/issues/<issue_number>",
  "source": "self_dev"
}
```

Added consequence explanation: "without it, the child is an orphan and callback routing to Step M4 will fail."

## Why This Works

**Callback routing:** The engine deliberately uses generic callback framing (see `docs/solutions/architecture-patterns/generic-callback-framing-parent-task-id.md`) — it provides `parent_task_id` as data but does not encode workflow-specific routing. The skill prompt must explicitly detect milestone/project context and branch accordingly. Without this check, any callback containing an issue reference will be misidentified as a new dispatch trigger by the Generic Workflow's pattern matching.

**JSON examples:** LLMs treat JSON examples as the strongest signal for structured output compliance — stronger than prose instructions alone (see `docs/solutions/prompt-engineering/2026-04-10-harden-skill-review-prompt-enforcement.md`). An absent key in the example consistently produces an absent key in every generated call. The JSON block makes `parent_task_id` unmissable.

## Prevention

- **Every distinct entry point needs explicit routing instructions.** Fallthrough to a catch-all (Generic Workflow) is always a bug — same class as the webhook fallthrough incident (mika#583, see `docs/solutions/logic-errors/webhook-fallthrough-dispatches-unrelated-backlog-work.md`).
- **Tool call examples must show all required parameters in JSON format.** Bullet lists are ambiguous; agents skip parameters not visible in examples. This is Rule 4 in the self-dev calibration rules.
- **Terminal vs non-terminal routing must be explicit.** When a prompt says "return to Step X," specify whether that applies only after terminal outcomes or also mid-retry.

## Related Issues

- mika#609 — this fix
- mika#6 — milestone where the break occurred
- mika#583 — webhook fallthrough incident (same bug class)
- mika#313 — generic callback framing engine changes
- `docs/solutions/architecture-patterns/generic-callback-framing-parent-task-id.md` — engine-side framing
- `docs/solutions/logic-errors/webhook-fallthrough-dispatches-unrelated-backlog-work.md` — same bug class
- `docs/solutions/prompt-engineering/2026-04-10-harden-skill-review-prompt-enforcement.md` — JSON example compliance
