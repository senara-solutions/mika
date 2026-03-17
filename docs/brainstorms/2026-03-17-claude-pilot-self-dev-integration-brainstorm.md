# Brainstorm: Claude-Pilot Self-Dev Integration Fix

**Date:** 2026-03-17
**Status:** Ready for implementation

## What We're Building

Fix the broken integration between mika-dev's self-dev workflow and claude-pilot. The old tmux relay is being replaced by claude-pilot (Claude Agent SDK wrapper), but three issues prevent it from working:

1. `--task-id` CLI flag collision between relay calls and callback completion
2. Missing PilotResponse JSON format instructions in self-dev prompt
3. Missing parent_task_id linkage between callback tasks and work items

## Why This Approach

### Root Cause 1: `--task-id` semantics collision

`transport.ts:27` appends `--task-id` to every `mika ask` relay call. But `ask.rs:91-146` treats `--task-id` as "complete this callback task and exit" — no agent run, no stdout. First relay completes the task (empty stdout → TransportError), all subsequent relays fail with "status completed/delivered". Every permission request falls through to auto-deny.

**Fix:** Remove `--task-id` from claude-pilot's relay args. The task_id is already in the PilotEvent JSON payload on stdin for correlation. `run.sh` still uses `--task-id` for the one-time final callback.

### Root Cause 2: Response format gap

Claude-pilot expects PilotResponse JSON on stdout: `{"action": "allow"}`, `{"action": "deny", "message": "..."}`, or `{"action": "answer", "answers": {"Q": "A"}}`. Mika-dev has no instructions about this format. Even with `--format json`, mika ask outputs `{"role":"assistant","content":"..."}` — wrong schema.

**Fix:** Update self-dev system_prompt.md with:
- Full PilotEvent input format documentation
- PilotResponse output format with examples for each action type
- The three-tier decision framework (migrated from claude-tmux-relay) adapted for JSON responses instead of tmux keystrokes
- Clear instruction that the agent must respond with ONLY the JSON payload, no commentary

### Root Cause 3: Missing parent_task_id linkage

The long-running executor creates callback tasks without setting `parent_task_id` to the work item. The relationship exists implicitly through session context but isn't in the task tree.

**Fix:** Modify the executor to set `parent_task_id` from the `work_item_id` field in the tool input.

### Bonus: Log filename correlation

`run.sh` uses `__mika_task_id` (callback UUID) for claude-pilot's `--task-id`, so logs land at `/var/log/claude-pilot/{callback-id}.log`. Self-dev expects them at `/var/log/claude-pilot/{work-item-id}.log`.

**Fix:** `run.sh` reads the user's `task_id` field from input JSON and passes it to `claude-pilot --task-id` for log naming. Uses `__mika_task_id` only for the final `mika ask` callback.

## Key Decisions

1. **Remove `--task-id` from relay args** — simplest fix, task_id stays in JSON payload
2. **System prompt instructions for response format** — no code changes to mika CLI needed, mika-dev outputs raw JSON via `--format text`
3. **Full decision framework in self-dev** — migrate from claude-tmux-relay, adapted for JSON responses instead of tmux keystrokes. claude-tmux-relay is being retired.
4. **Link callback tasks to work items** via parent_task_id in the executor
5. **Use work_item_id for log filenames** — run.sh separates log ID from callback ID

## Open Questions

None — all decisions resolved during brainstorm.
