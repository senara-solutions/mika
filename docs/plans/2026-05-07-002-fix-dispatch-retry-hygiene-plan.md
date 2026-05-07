---
title: "fix: Dispatch retry hygiene — task status, action_config, placeholder UUIDs"
type: fix
status: active
date: 2026-05-07
---

# fix: Dispatch retry hygiene — task status, action_config, placeholder UUIDs

## Overview

Three dispatch-side telemetry gaps observed during milestone#19 execution cause audit queries to miss successful runs, require parent joins to reconstruct child dispatch context, and allow malformed UUIDs to pass through the dispatch validation gate.

## Problem Frame

Issue #958 bundles three related gaps in the self-dev dispatch pipeline:

1. **Parent task status stuck at `failed` after retry success.** The reaper (`reap_orphaned_parent_tasks`) marks parents `failed` after 600s if the callback delivered without a `pr_url`. A retry child then succeeds, and `try_extract_callback_metadata()` writes metadata (including `pr_url`) to the parent — but does not transition the parent's status. Since `failed` is terminal in the LLM-facing state machine (`VALID_TRANSITIONS`), the LLM's `update_task_status` call also cannot fix it. The parent stays `failed` with successful metadata.

2. **`long_running:run_claude_pilot` child tasks have empty `action_config`.** `execute_long_running()` creates the callback child with `action_config: "{}"` (hardcoded). The dispatch input (prompt, skill, task_id) is stored only in `input_context`. Audit queries on `action_config.input` return NULL for every child.

3. **Placeholder UUID at first dispatch.** The self-dev system prompt uses angle-bracket templates (`<task UUID from Step 2>`) that the LLM occasionally fails to substitute. `dispatch-lib.sh` validates UUID format but only warns — it does not reject malformed values. The dispatch proceeds with a non-UUID string, fails at `create_task` (which uses layer-1 `Uuid::parse_str`), and auto-recovers on retry.

## Requirements Trace

- R1. After a retry succeeds, the parent task's `status` reflects the outcome (not `failed`).
- R2. A `long_running:run_claude_pilot` row alone is enough to identify what was dispatched (prompt, skill, task_id in `action_config.input`).
- R3. No `placeholder-uuid-will-replac` strings appear in `tasks.action_config.input.task_id` for any new dispatch — malformed UUIDs are rejected at the handler boundary.

## Scope Boundaries

- Does not change the LLM-facing `VALID_TRANSITIONS` state machine — `failed` stays terminal for agent-driven transitions.
- Does not change handler crash recovery (#955 — sibling ticket).
- Does not change the stale callback watchdog (#959 — companion ticket).
- Does not backfill existing rows with empty `action_config` — forward-only.

## Context & Research

### Relevant Code and Patterns

- `crates/mika-agent/src/task_engine/dispatcher.rs` — `try_extract_callback_metadata()` (lines 1106-1165): engine-level metadata extraction that runs pre-agent. Best-effort, fire-and-forget. Already extracts `pr_url` from callback results.
- `crates/mika-agent/src/task_engine/engine.rs` — `reap_orphaned_parent_tasks()` (lines 586-682): transitions orphaned parents to `failed` via guarded `update_task_failed()`.
- `crates/mika-agent/src/skills/executor.rs` — `execute_long_running()` (lines 968-1014): creates callback child task with `action_config: "{}"` and `input_context: Some(serialized_input)`.
- `crates/mika-agent/src/db.rs` — `update_task_failed()` (line 4321): guarded UPDATE with `status NOT IN ('completed', 'failed', 'cancelled', 'expired', 'delivered')`.
- `crates/mika-agent/src/tools/update_task_status.rs` — `VALID_TRANSITIONS`: `failed` has no entry (terminal).
- `skills/bundled/_shared/dispatch-lib.sh` — UUID validation (lines 135-138): non-blocking warning only.

### Institutional Learnings

- `docs/solutions/logic-errors/parent-self-dev-task-leaks-in-progress-after-callback-delivers-2026-04-29.md` — same failure class; documents the reaper and its `update_task_failed` guard.
- `docs/solutions/architecture-patterns/engine-level-callback-metadata-extraction.md` — metadata persistence must happen at engine level (deterministic, pre-agent), not rely on agent step budget.
- `docs/solutions/architecture-patterns/phantom-retry-guard-active-dispatch-metadata-validation.md` — retry metadata writes are guarded against phantom retries when active callback children exist.
- `docs/solutions/logic-errors/terminal-state-metadata-rejection-race.md` — terminal-state metadata fallback: metadata writable, status not.
- `docs/solutions/architecture-patterns/dispatch-readiness-guard-long-running-status-validation.md` — `validate_dispatch_readiness()` only allows `pending` and `in_progress` for dispatch.

## Key Technical Decisions

- **Engine-level parent status promotion (not LLM-driven):** Add a new `promote_parent_on_retry_success()` function in `dispatcher.rs` that transitions the parent from `failed` → `completed` via a direct guarded DB method. This keeps the LLM-facing state machine unchanged and follows the engine-level callback metadata extraction pattern. The promotion is deterministic (keyed off `pr_url` presence in extracted metadata) and runs alongside `try_extract_callback_metadata()`.

- **New guarded DB method `promote_task_completed()`:** Symmetric to `update_task_failed()`. Only transitions from `failed` → `completed` (guarded WHERE clause). Returns `bool` for idempotency. Emits audit event with `tool_name='task_engine_retry_promoter'`.

- **Populate `action_config.input` from tool input:** Replace the hardcoded `"{}"` with a JSON object containing the dispatch input fields (prompt, skill, task_id). The `input_context` field continues to carry the full serialized input for backward compatibility.

- **Hard error for malformed UUIDs in dispatch-lib.sh:** Change the non-blocking warning to a hard error (`exit 1`). The engine's `validate_dispatch_readiness()` already validates UUIDs via `Uuid::parse_str()`, but that validation happens at `create_task` time — after the handler has already started. Rejecting early in the handler prevents wasted subprocess startup.

## Open Questions

### Resolved During Planning

- **Should `failed` → `completed` be added to `VALID_TRANSITIONS`?** No. The LLM should not be able to retry-promote parent tasks — that's a structural engine responsibility. The direct DB method keeps this out of the LLM's reach.
- **Should we backfill existing empty `action_config` rows?** No. Forward-only fix. Existing rows can still be reconstructed via parent join or `input_context`.
- **What fields go into `action_config.input`?** The user-provided tool input fields: `prompt`, `skill`, `task_id`, `branch` (when present). Mirrors the schema the system prompt documents.

### Deferred to Implementation

- Exact JSON structure for `action_config.input` — will match the tool input shape but may need field selection to avoid redundancy with `input_context`.

## Implementation Units

- [x] **Unit 1: Add `promote_task_completed()` DB method**

**Goal:** Add a guarded DB method that transitions a task from `failed` → `completed`, symmetric to `update_task_failed()`.

**Requirements:** R1

**Dependencies:** None

**Files:**
- Modify: `crates/mika-agent/src/db.rs`
- Modify: `crates/mika-agent/src/async_db.rs`
- Test: `crates/mika-agent/src/db.rs` (inline `#[cfg(test)] mod tests`)

**Approach:**
- Add `promote_task_completed(id, agent_id, reason)` to `Database` with a guarded `UPDATE tasks SET status = 'completed', result = ?1, completed_at = ... WHERE id = ?2 AND agent_id = ?3 AND status = 'failed'`. The WHERE clause ensures this only fires from `failed` state — no other source states.
- Add async wrapper in `AsyncDatabase`.
- Return `Result<bool>` — `true` if transition happened, `false` if task was not in `failed` state (race guard).

**Patterns to follow:**
- `update_task_failed()` in `db.rs:4321-4330` — same guarded UPDATE pattern with `status NOT IN (...)` guard, returning `Ok(rows > 0)`.

**Test scenarios:**
- Happy path: task in `failed` status → `promote_task_completed()` returns `true`, task is now `completed`
- Edge case: task in `completed` status → returns `false`, no change
- Edge case: task in `in_progress` status → returns `false`, no change (only promotes from `failed`)
- Edge case: task does not exist → returns `false`
- Edge case: wrong `agent_id` → returns `false`

**Verification:**
- `promote_task_completed()` transitions `failed` → `completed` and is a no-op for all other states.

---

- [x] **Unit 2: Add engine-level parent status promotion on retry success**

**Goal:** After `try_extract_callback_metadata()` writes successful metadata to a parent task, check if the parent was `failed` and promote it to `completed`.

**Requirements:** R1

**Dependencies:** Unit 1

**Files:**
- Modify: `crates/mika-agent/src/task_engine/dispatcher.rs`
- Test: `crates/mika-agent/tests/eval/` (integration test via EvalHarness, or inline unit test)

**Approach:**
- Add `try_promote_parent_on_retry_success()` in `dispatcher.rs`, called from `dispatch_resume_agent()` after `try_extract_callback_metadata()`.
- The function checks: (a) parent exists, (b) parent status is `failed`, (c) extracted metadata contains `pr_url` (success indicator). If all three hold, call `promote_task_completed()` and emit audit event with `tool_name='task_engine_retry_promoter'`.
- Log at INFO level: `"engine: promoted parent task from failed to completed (retry success)"` with `parent_task_id`, `callback_task_id`, `pr_url`.
- Best-effort, fire-and-forget (same pattern as `try_extract_callback_metadata`).

**Patterns to follow:**
- `try_extract_callback_metadata()` in `dispatcher.rs:1106-1165` — same best-effort pattern with early returns on missing data.
- Audit event emission pattern from `reap_orphaned_parent_tasks()` in `engine.rs:614-623`.

**Test scenarios:**
- Happy path: parent `failed` + callback result has `pr_url` → parent promoted to `completed`, audit event emitted
- Edge case: parent `in_progress` + callback has `pr_url` → no promotion (not `failed`, no action)
- Edge case: parent `failed` + callback has NO `pr_url` → no promotion (no success indicator)
- Edge case: parent already `completed` → `promote_task_completed()` returns `false`, no-op
- Integration: callback result with `"PR: https://github.com/..."` line → `extract_callback_fields()` parses `pr_url` → parent promoted

**Verification:**
- After a retry callback delivers with `pr_url`, the parent task's status is `completed` (not `failed`). Audit log shows `task_engine_retry_promoter` entry.

---

- [x] **Unit 3: Populate `action_config.input` on callback child tasks**

**Goal:** Replace the hardcoded `action_config: "{}"` with structured dispatch input so child tasks are self-describing.

**Requirements:** R2

**Dependencies:** None

**Files:**
- Modify: `crates/mika-agent/src/skills/executor.rs`
- Test: `crates/mika-agent/src/skills/executor.rs` (or integration test)

**Approach:**
- In `execute_long_running()`, build an `action_config` JSON object with `{"input": { ... }}` containing the relevant fields from the tool input: `prompt`, `skill`, `task_id`, and `branch` (when present).
- Use `serde_json::json!()` to build the object, extracting values from the existing `input: &Value` parameter.
- The `input_context` field continues to carry the full serialized input unchanged (backward compatibility).
- Cap the serialized `action_config` at a reasonable size (the same `input_context` data is already persisted without cap, so this is additive).

**Patterns to follow:**
- The `input_context` serialization pattern at `executor.rs:985-1007` — same source data, different destination field.
- Reminder tasks in `dispatcher.rs` use `action_config.text` — the pattern of structured `action_config` is established.

**Test scenarios:**
- Happy path: `run_claude_pilot` with prompt, skill, task_id → callback child task has `action_config.input.prompt`, `.skill`, `.task_id` populated
- Happy path: optional `branch` field present → included in `action_config.input`
- Edge case: missing optional fields (e.g., no `branch`) → field absent from `action_config.input`, not null
- Integration: query `json_extract(action_config, '$.input.prompt')` on created child task → returns the dispatch prompt

**Verification:**
- `SELECT json_extract(action_config, '$.input.prompt') FROM tasks WHERE label = 'long_running:run_claude_pilot'` returns non-NULL for new dispatches.

---

- [x] **Unit 4: Hard error for malformed UUIDs in dispatch-lib.sh**

**Goal:** Reject non-UUID `task_id` values at the handler boundary instead of warning.

**Requirements:** R3

**Dependencies:** None

**Files:**
- Modify: `skills/bundled/_shared/dispatch-lib.sh`
- Test: Manual verification (shell script — no Rust test harness)

**Approach:**
- Change lines 135-138 in `dispatch-lib.sh` from a warning to a hard error: replace the `echo "WARNING: ..."` with the structured error format and `exit 1`.
- Use the same `DISPATCH_VALIDATION_ERROR` prefix and JSON structure as the existing `missing_required_field` errors (lines 126-133) for consistency.
- Error message should include the malformed value and expected format.

**Patterns to follow:**
- Existing validation errors in `dispatch-lib.sh:126-133` — `DISPATCH_VALIDATION_ERROR: {JSON}` format with `exit 1`.

**Test scenarios:**
- Happy path: valid UUID → passes validation, dispatch proceeds
- Error path: `"placeholder-uuid-will-replac"` → hard error, exit 1, structured JSON error
- Error path: empty-ish but non-empty string → hard error
- Edge case: uppercase UUID → passes (the regex uses `grep -iE` for case-insensitive match)

**Verification:**
- Dispatching with a non-UUID `task_id` returns a structured error and does not spawn a subprocess.

## System-Wide Impact

- **Interaction graph:** `try_promote_parent_on_retry_success()` runs in `dispatch_resume_agent()` after `try_extract_callback_metadata()` — the same pre-agent execution path. The reaper (`reap_orphaned_parent_tasks`) and this promoter are symmetric: the reaper demotes to `failed`, the promoter promotes to `completed`. They cannot race because the reaper's query filters for `pr_url IS NULL` and the promoter fires only when `pr_url` is present.
- **Error propagation:** All new paths are best-effort, fire-and-forget — failures are logged but do not block callback dispatch.
- **State lifecycle risks:** The `promote_task_completed` guard (`WHERE status = 'failed'`) prevents double-promotion. The reaper's terminal-state guard (`status NOT IN (completed, failed, ...)`) prevents re-reaping a promoted task.
- **API surface parity:** Dashboard `TaskResponse` already surfaces `status` and `action_config` — no API changes needed. `action_config.input` becomes populated where it was previously empty.
- **Unchanged invariants:** `VALID_TRANSITIONS` is unchanged. The LLM still cannot transition from `failed`. Dispatch readiness guard (`pending`/`in_progress` only) is unchanged.

## Risks & Dependencies

| Risk | Mitigation |
|------|------------|
| Reaper and promoter race on the same parent task | Cannot race: reaper filters for `pr_url IS NULL`; promoter requires `pr_url` present. Guarded WHERE clauses on both sides provide defense-in-depth. |
| Existing `action_config: "{}"` consumers break | `action_config` is not read by the dispatcher for callbacks (uses `task.result`). Adding `input` is additive. |
| Hard UUID validation blocks legitimate dispatches | UUID format is well-defined (RFC 4122). The existing warning has never fired for valid UUIDs — only for LLM hallucinations. |

## Sources & References

- Related issue: #958
- Sibling: #955 (handler crash on missing skill arg)
- Companion: #959 (stale callback watchdog)
- Related code: `crates/mika-agent/src/task_engine/dispatcher.rs` (callback metadata extraction)
- Related code: `crates/mika-agent/src/task_engine/engine.rs` (orphaned parent reaper)
- Related code: `crates/mika-agent/src/skills/executor.rs` (long-running dispatch)
- Related code: `skills/bundled/_shared/dispatch-lib.sh` (shared dispatch library)
