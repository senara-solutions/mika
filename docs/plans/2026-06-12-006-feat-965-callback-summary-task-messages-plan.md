---
title: "feat: append callback summary to task_messages for dispatch session continuity"
date: 2026-06-12
type: feat
origin: "mika#965"
depth: Lightweight
---

# feat: append callback summary to task_messages for dispatch session continuity

## Summary

After `try_extract_callback_metadata()` parses structured fields from a callback result (session_id, turns, cost_usd, duration_ms, pr_url), write a summary message directly to `task_messages` keyed to the scope-root task_id. This closes the narrative gap where the dispatching session's `rebuild_context()` merges `task_messages` but no callback outcome was ever written there — the dispatch session's next turn sees the callback result in its rebuilt context.

## Problem Frame

mika#974 shipped the `task_messages` parallel narrative table and the `rebuild_context()` merge path. The agent's context assembly now reads from `task_messages` when a `scope_task_id` resolves. However, the callback delivery path in `dispatcher.rs` writes extracted metadata only to `tasks.metadata` (via `try_extract_callback_metadata`, line 1375). No code path writes a human-readable summary to `task_messages`. The dispatch session's next context rebuild therefore has no record of what the callback produced — no PR URL, no cost, no outcome status.

This is a correctness gap independent of any concurrency framing: even in a fully sequential loop, "the callback summary should land in a way the originating dispatch session can read" is unmet.

## Requirements

- **R1.** After successful callback metadata extraction, a structured summary message is written to `task_messages` with the scope-root `task_id` as key.
- **R2.** The summary includes all fields extracted by `extract_callback_fields()`: `session_id`, `turns`, `cost_usd`, `duration_ms`, `pr_url` (when present).
- **R3.** The write is best-effort, fire-and-forget — matches the existing `try_extract_callback_metadata()` error discipline (warn on failure, never blocks callback dispatch).
- **R4.** The summary is written to `task_messages` only (not to `messages`) — it is engine-internal task narrative, not a conversation message.
- **R5.** Scope-root resolution reuses the existing `resolve_scope_root_task_id()` walker — no new hierarchy traversal logic.

## Key Technical Decisions

**KTD-1: task_messages-only write (not double-write).** The callback summary is engine-internal narrative for task continuity, not a conversation message. Writing to `messages` would pollute channel history. A new `insert_task_message()` method (standalone, non-transactional) on `Database` and `AsyncDatabase` exposes the existing `INSERT INTO task_messages` without the `messages` double-write. This method is `pub` so the dispatcher can call it directly.

**KTD-2: role = "system" for callback summaries.** The summary is engine-generated, not user or assistant content. Using `role = "system"` distinguishes it from agent-authored narrative in `rebuild_context()` output. Matches the session creation pattern in `dispatch_resume_agent()` which uses `"system"` channel type.

**KTD-3: Write site is immediately after metadata extraction.** The summary write fires at `dispatcher.rs` line ~1433, right after `try_extract_callback_metadata()` persists to `tasks.metadata`. This location guarantees the extracted fields are available and the parent task has been verified. It runs BEFORE the silent agent turn, so the callback turn's `rebuild_context()` already includes the summary.

**KTD-4: Idempotency via content shape.** No explicit dedup key — the write fires once per callback delivery (the `dispatch_resume_agent` path is single-entry per task). The `task_messages` table is append-only; a duplicate on restart retry is acceptable (the content is identical, and `rebuild_context()` dedup on `(session_id, role, content, created_at)` handles it).

## Scope Boundaries

### In scope
- New `insert_task_message` / `insert_task_message_async` methods
- New `try_write_callback_summary_to_task_messages()` function in dispatcher
- Tests for the new write path

### Deferred to Follow-Up Work
- PR-review write-back (mika-qa → mika-dev) — separate ticket if needed
- Scheduled task / cron firing write-back
- Cross-agent dispatch summaries
- Dashboard UI for task-narrative views

---

## Implementation Units

### U1. Add standalone `insert_task_message` to Database and AsyncDatabase

**Goal:** Expose a public, non-transactional `INSERT INTO task_messages` method that the dispatcher can call without the `messages` double-write.

**Requirements:** R4

**Dependencies:** None

**Files:**
- `crates/mika-agent/src/db.rs` — new `pub fn insert_task_message()`
- `crates/mika-agent/src/async_db.rs` — new `pub async fn insert_task_message()`
- `crates/mika-agent/src/db.rs` (test module) — unit test

**Approach:** Extract the SQL from the existing `insert_task_message_tx` but operate on `self.conn` directly instead of a caller-provided transaction. Signature: `(task_id, agent_id, session_id, role, content, metadata, trace_id) -> Result<i64>`. The async wrapper follows the standard `with_db` closure pattern already used by `save_message_with_task_context`.

**Patterns to follow:** `save_message_with_metadata` (non-transactional single insert), `load_task_messages` async wrapper.

**Test scenarios:**
- Insert a task message and verify it appears in `load_task_messages()` output with correct fields
- Insert with `None` metadata and trace_id — verify no crash and fields are NULL

### U2. Write callback summary to task_messages after metadata extraction

**Goal:** After `try_extract_callback_metadata()` succeeds, resolve the scope-root task_id and write a structured summary to `task_messages`.

**Requirements:** R1, R2, R3, R5

**Dependencies:** U1

**Files:**
- `crates/mika-agent/src/task_engine/dispatcher.rs` — new `try_write_callback_summary()` async fn, call site after line ~1408
- `crates/mika-agent/tests/eval/` or inline test module — integration test

**Approach:** New async function `try_write_callback_summary(db: &AsyncDatabase, task: &Task)` in `dispatcher.rs`. Steps:
1. Guard: `parent_task_id` must exist (same guard as `try_extract_callback_metadata`)
2. Parse result via `extract_callback_fields()` (reuse, not re-parse — or accept the extracted `serde_json::Value` as parameter to avoid double-parsing)
3. Resolve scope-root via `db.resolve_scope_root_task_id(parent_id)` — if `None`, return silently (no scope root = no task narrative to append to)
4. Format summary content as a human-readable string (e.g., `"Callback completed: session=<id>, turns=<n>, cost=$<usd>, duration=<ms>ms, PR: <url>"`)
5. Call `db.insert_task_message(scope_root_id, agent_id, callback_session_id, "system", summary_content, Some(extracted_json), trace_id)`
6. Log success/failure at info/warn level, matching `try_extract_callback_metadata` discipline

Call site: `dispatcher.rs` in `dispatch_resume_agent()`, immediately after `try_extract_callback_metadata(&self.db, task).await` (line ~408). The function is fire-and-forget — errors are logged, never propagated.

**Design consideration:** To avoid double-parsing the callback result, either (a) have `try_extract_callback_metadata` return the extracted `Value` so the caller can pass it to `try_write_callback_summary`, or (b) accept the minor cost of re-parsing (the result text is small, O(1KB)). Option (b) is simpler and matches the existing fire-and-forget pattern where each function is self-contained.

**Patterns to follow:** `try_extract_callback_metadata` (guard structure, fire-and-forget error handling, logging pattern), `try_promote_parent_on_retry_success` (sibling best-effort function).

**Test scenarios:**
- Callback with parent task and valid result — verify `task_messages` row written with scope-root task_id, role "system", and content containing PR URL
- Callback with no parent task — verify no write, no error
- Callback with parent but no scope root resolvable — verify no write, no error
- Callback with empty result — verify no write
- Verify the summary content includes all extracted fields (session_id, turns, cost_usd, duration_ms, pr_url) when present
- Verify the summary content gracefully handles missing optional fields (e.g., no pr_url on groom callbacks)

## Open Questions

None — the approach follows established precedent (`try_extract_callback_metadata` pattern) and uses shipped infrastructure (`task_messages`, `resolve_scope_root_task_id`).
