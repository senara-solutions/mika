---
title: Dispatch retry hygiene — parent status promotion, action_config, UUID validation
date: 2026-05-07
category: logic-errors
module: task-engine
problem_type: logic_error
component: tooling
symptoms:
  - Parent task status stuck at 'failed' after retry child succeeds with pr_url
  - Audit queries miss successful runs when filtering by status='completed'
  - long_running:run_claude_pilot child tasks have empty action_config.input
  - Placeholder UUID literal 'placeholder-uuid-will-replac' in task_id field
root_cause: logic_error
resolution_type: code_fix
severity: medium
tags:
  - task-engine
  - dispatch
  - retry
  - callback
  - reaper
  - action-config
  - uuid-validation
  - self-dev
---

# Dispatch retry hygiene — parent status promotion, action_config, UUID validation

## Problem

Three dispatch-side telemetry gaps observed during milestone#19 execution caused audit queries to miss successful runs, required parent joins to reconstruct child dispatch context, and allowed malformed UUIDs to pass through the dispatch validation gate.

## Symptoms

- Parent task has `status=failed` but metadata contains `claude_pilot.pr_url`, `cost`, `turns` from a successful retry child
- `SELECT json_extract(action_config, '$.input.prompt') FROM tasks WHERE label = 'long_running:run_claude_pilot'` returns NULL for all children
- First dispatch attempt had `task_id: "placeholder-uuid-will-replac"` (literal string), failed with `error: invalid_uuid`

## What Didn't Work

- The LLM-facing `VALID_TRANSITIONS` state machine has no entry for `failed` as a source state — it's terminal. The LLM's `update_task_status` call cannot transition from `failed` to `completed`.
- The orphaned-parent reaper (`reap_orphaned_parent_tasks`) correctly marks parents `failed` when callback delivers without `pr_url` after 600s grace. But when a retry child subsequently succeeds, no mechanism existed to promote the parent back to `completed`.
- `dispatch-lib.sh` validated UUID format but only warned — the malformed value proceeded through to `create_task` which rejected it via `Uuid::parse_str`, wasting a subprocess startup.

## Solution

Three engine-level fixes, all following the existing best-effort fire-and-forget pattern:

**1. Engine-level parent status promotion (`try_promote_parent_on_retry_success`)**

Added `promote_task_completed()` DB method — symmetric to `update_task_failed()`, with a guarded `WHERE status = 'failed'` clause that only transitions from `failed`. Called from `dispatch_resume_agent()` after `try_extract_callback_metadata()` when the extracted metadata contains `pr_url`.

Key guards:
- Parent must be `trigger_type='manual'`, `status='failed'`, `source='self_dev'` (mirrors reaper scope)
- Callback result must contain `pr_url` (success indicator)
- Emits audit event with `tool_name='task_engine_retry_promoter'`

**2. Populate `action_config.input` on callback child tasks**

Replaced hardcoded `action_config: "{}"` with structured JSON containing dispatch input fields (`prompt`, `skill`, `task_id`, `branch`). `input_context` continues to carry the full serialized input for backward compatibility.

**3. Hard error for malformed UUIDs in `dispatch-lib.sh`**

Changed the non-blocking warning to a `DISPATCH_VALIDATION_ERROR` with `exit 1`. Value is sanitized (backslashes and quotes escaped, truncated to 200 chars) before embedding in the JSON error output.

## Why This Works

The reaper and promoter are symmetric and cannot race: the reaper filters for `pr_url IS NULL` and the promoter requires `pr_url` present. The `WHERE status = 'failed'` guard prevents double-promotion. The `source='self_dev'` guard (added during review) ensures the promoter's scope matches the reaper's — non-self-dev manual tasks are unaffected.

The `action_config.input` population is additive — no existing code reads `action_config` for callback tasks (the dispatcher reads from `task.result`). The new field enables audit queries without parent joins.

The UUID hard error catches LLM hallucinations (unsubstituted template placeholders like `<task UUID from Step 2>`) at the handler boundary before subprocess startup, rather than letting them fail at `create_task` time.

## Prevention

- **Engine-level state management over LLM-driven transitions** — when a state transition is deterministic (keyed off specific metadata fields), implement it at the engine level rather than relying on the LLM's step budget. The LLM may exhaust its 20-step limit before calling `update_task_status`.
- **Guarded DB methods for symmetric state operations** — when the engine has a demote path (`update_task_failed`), consider whether a promote path (`promote_task_completed`) is needed for recovery scenarios.
- **Mirror scope guards** — when a promoter is symmetric to a reaper, ensure both have the same scope guards (`source='self_dev'`, `trigger_type='manual'`). Review finding caught the missing `source` guard.
- **Make dispatch validation hard errors** — non-blocking warnings for invalid input allow wasted work. Fail fast at the handler boundary with structured error output.

## Related Issues

- [#958](https://github.com/senara-solutions/mika/issues/958) — this fix
- [#955](https://github.com/senara-solutions/mika/issues/955) — handler crash on missing skill arg (sibling)
- [#959](https://github.com/senara-solutions/mika/issues/959) — stale callback watchdog (companion)
- [#871](https://github.com/senara-solutions/mika/issues/871) — orphaned parent reaper
- `docs/solutions/logic-errors/parent-self-dev-task-leaks-in-progress-after-callback-delivers-2026-04-29.md` — related failure class (parent stuck in `in_progress`)
- `docs/solutions/architecture-patterns/engine-level-callback-metadata-extraction.md` — pattern followed
