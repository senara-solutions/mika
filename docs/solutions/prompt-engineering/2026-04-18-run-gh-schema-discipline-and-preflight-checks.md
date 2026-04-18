---
title: "run_gh schema discipline + create_work_item pre-flight checks"
category: prompt-engineering
date: 2026-04-18
module: self-dev, qa-review, self-dev-webhook-ci, self-dev-webhook-qa, self-dev-iterate
tags: [run_gh, tool-input-schema, idempotency, work-items, milestone, project]
problem_type: tool_call_input_shape
severity: high
---

# run_gh Schema Discipline + create_work_item Pre-Flight Checks

## Problem

Two distinct tool-call input shape bugs caused autonomous session failures:

1. **`run_gh` `--repo` in `command` array** — The agent serialized `--repo senara-solutions/mika` as a flag inside the `command` array instead of using the separate `repo` parameter. The wrapper rejected the call. On retry, the agent dropped `--repo` entirely (instead of relocating it), silently querying the wrong repo and concluding the milestone didn't exist.

2. **`create_work_item` missing fields / duplicates** — The agent truncated the 5-field JSON to just `{label, type}`, dropping `parent_task_id` (orphaning children from the milestone tree), `reference_url` (disabling dedup), and `source`. Separately, without a pre-flight dedup check, the agent created the same work item twice.

## Root Cause

Prompt examples used CLI shorthand (`run_gh issue list --milestone <n> --repo ...`) that the LLM treated as literal input format. The `create_work_item` instruction said "all fields" — a soft constraint the LLM frequently truncated, especially after context compaction.

## Solution

### run_gh discipline (6 files)

Added explicit `run_gh` schema discipline to all skill prompts that can trigger `run_gh` calls:

- **self-dev** Rule 4: Full schema documentation with two-input explanation, relocation rule ("move `--repo` out, don't drop it"), allowed subcommand list
- **qa-review**, **qa-review-build-callback**: Constraint bullet
- **self-dev-webhook-ci**, **self-dev-webhook-qa**: Rule 7
- **self-dev-iterate**: Rule 2

Key rule: `run_gh` takes TWO SEPARATE INPUTS — `"command"` (array) and `"repo"` (string). `--repo` is a sibling parameter, never inside the array. If the wrapper rejects, relocate `--repo`, don't drop it.

### Step M2: JSON tool-input form

Replaced CLI shorthand with explicit JSON form:
```json
run_gh({
  "command": ["issue", "list", "--milestone", "<n>", "--state", "open", ...],
  "repo": "senara-solutions/<repo>"
})
```

### Steps M3/P3: Pre-flight + "EXACTLY 5 fields"

- Added `list_work_items` pre-flight check by `reference_url` before every `create_work_item` call
- Changed "all fields" to "EXACTLY these 5 fields — copy the JSON block as-is"
- Added explicit warning covering all failure modes (orphaned children, disabled dedup, incomplete form)

## Prevention

- **Use JSON examples, not CLI shorthand** in prompts. LLMs treat JSON code blocks as the strongest signal for structured output compliance.
- **Enumerate field counts explicitly** ("EXACTLY 5 fields") rather than using soft quantifiers ("all fields").
- **Add pre-flight dedup checks** for any create operation that could run after context compaction.
- **Propagate tool-input discipline** to all skill prompts that can invoke the tool, not just the primary orchestrator.

## Related Issues

- mika#640 — This fix
- mika#639 — Complementary reconcile PR (workflow enforcement, not input shape)
- Session `4cbc6de7-...` — run_gh --repo-in-array incident
- Session `749345f9-...` — create_work_item duplicate + missing fields incident
- `docs/solutions/integration-issues/run-gh-string-to-array-coercion.md` — Related Rust-side coercion fix
- `docs/solutions/logic-errors/milestone-callback-misrouted-to-generic-workflow.md` — Related callback routing fix
