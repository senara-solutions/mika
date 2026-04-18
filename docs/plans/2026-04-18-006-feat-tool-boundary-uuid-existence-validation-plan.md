---
title: "feat: Tool-boundary UUID existence validation (extension of #531)"
type: feat
status: active
date: 2026-04-18
---

# feat: Tool-boundary UUID existence validation (extension of #531)

## Overview

Introduce a `validate_task_exists()` helper in `tools/mod.rs` that combines UUID format validation with DB existence checking in a single call. This replaces the current two-step pattern (format check + separate DB lookup with inconsistent error messages) across all task-accepting tools, producing structured JSON errors that help the LLM self-correct when it fabricates UUIDs.

## Problem Frame

Issue #595 revealed the root cause: mika-dev's LLM fabricated a `task_id` that was well-formed (passed `validate_uuid`) but didn't exist in the database. The tool handler accepted it, the exec script forwarded it to claude-pilot, and every subsequent relay callback failed with "task not found" for 23 minutes. While #531 closed the format-only gap, the existence gap remains — each tool implements its own DB lookup with inconsistent error messages and no structured JSON for the "not found" case.

## Requirements Trace

- R1. Format-valid but non-existent task_id returns a structured JSON error (`task_not_found`) instead of plain-text "not found"
- R2. Cross-agent task_id (exists but belongs to another agent) returns the same `task_not_found` error (no information disclosure)
- R3. Real task_id passes through and returns the `Task` struct for downstream business logic
- R4. DB errors fail closed (reject the call, not silent passthrough)
- R5. Existing `validate_work_item()` is refactored to use the new helper as its base layer
- R6. Unit tests cover: valid, invalid-format, valid-format-not-in-DB, cross-agent
- R7. No regression in existing tool behavior — only error message format changes

## Scope Boundaries

- Only tool `execute()` methods are in scope — not `cancel_task_and_kill` (shared infrastructure used by HTTP handlers and CLI, returns `anyhow::Result`, not `ToolOutput`)
- Dashboard endpoints using `get_task_unscoped()` are unaffected
- `cancel_reminder` inherits changes via delegation to `CancelTaskTool` — no separate work needed
- The "wrong agent" vs "not found" distinction is intentionally NOT provided — agent-scoped queries conflate them, and distinguishing would require `get_task_unscoped` which leaks cross-agent existence information

### Deferred to Separate Tasks

- `create_work_item`'s `parent_task_id` uses `get_task_depth()` which returns `Option<i64>`, not `Task`. Converting it to use the new helper would require a full `get_task` + reading `depth` from `Task`. Since `Task.depth` exists, this is viable but changes the query pattern. Defer to a follow-up if the team wants to unify this path too.

## Context & Research

### Relevant Code and Patterns

- `crates/mika-agent/src/tools/mod.rs` — `validate_uuid()` (line 244), `validate_work_item()` (line 276), `ToolOutput`, `ToolContext`
- `crates/mika-agent/src/tools/get_task.rs` — canonical simple tool pattern (format check + `db.get_task` + "not found")
- `crates/mika-agent/src/tools/complete_task.rs` — same pattern + business logic on `Task` fields
- `crates/mika-agent/src/tools/cancel_task.rs` — delegates to `cancel_task_and_kill` (out of scope for helper)
- `crates/mika-agent/src/tools/update_work_item_status.rs` — uses `db.get_task` + manual trigger_type check
- `crates/mika-agent/src/tools/check_work_item.rs` — uses `db.get_manual_task` (separate agent-scoped query)
- `crates/mika-agent/src/async_db.rs` — `AsyncDatabase` with `agent_id` field, `get_task()` is agent-scoped
- `crates/mika-agent/src/db.rs` — `Task` struct (line 133, has `depth` field), `get_task` (line 2989, `WHERE id = ?1 AND agent_id = ?2`)

### Institutional Learnings

- `docs/solutions/best-practices/uuid-validation-at-tool-boundary.md` — documents #531 pattern, structured JSON error format
- `docs/solutions/architecture-patterns/delegation-work-item-guard-enforcement.md` — `validate_work_item()` layering pattern
- `docs/solutions/architecture-patterns/dispatch-readiness-guard-long-running-status-validation.md` — fail-closed on DB errors, layered validation (format -> existence -> business rules)
- `docs/solutions/database-issues/team-task-child-wrong-agent-id.md` — agent_id scoping decision framework (tool access = scoped, tree traversal = unscoped)

## Key Technical Decisions

- **Single-query, conflated errors (R2):** `validate_task_exists` uses the existing agent-scoped `db.get_task()`. A cross-agent UUID and a non-existent UUID both return `task_not_found`. This avoids information disclosure and reuses the existing query path.
- **Return `Result<Task, ToolOutput>` (R3):** Matches the `validate_uuid` convention (`Result<Uuid, ToolOutput>`) and gives callers the `Task` struct directly, eliminating the redundant second DB query that most tools currently do.
- **Structured JSON errors (R1):** Error format matches `validate_uuid` pattern: `{"error": "task_not_found", "field": "<name>", "task_id": "<value>", "reason": "..."}`. The `task_id` field (instead of `received`) distinguishes it from format errors.
- **Refactor `validate_work_item` to layer on top (R5):** The existing `validate_work_item()` will call `validate_task_exists()` for format+existence, then layer its own trigger_type and status checks. Return type stays `Option<String>` to avoid a larger refactor of `delegate_task` and dispatch-readiness callers.
- **`check_work_item` keeps `get_manual_task` (performance):** `check_work_item` currently uses `db.get_manual_task()` which adds `AND trigger_type = 'manual'` to the query. Using `validate_task_exists` + post-check would do a full `get_task` then filter in Rust. Since the error messages are already good and the query is agent-scoped, adopting the helper here adds a structured error for "not found" but changes the query. Adopt it for consistency — the performance difference on single-row lookups is negligible.
- **`cancel_task` adopts helper for the format+existence check only:** `cancel_task` calls `cancel_task_and_kill` which does its own `get_task`. The tool's `execute()` currently does format validation then delegates. The helper can replace the format check + add a pre-existence check before delegation, but `cancel_task_and_kill` remains unchanged (infrastructure code).

## Open Questions

### Resolved During Planning

- **Should "wrong agent" be a separate error?** No. Agent-scoped queries conflate by design. Separating would require `get_task_unscoped` which leaks cross-agent existence.
- **Should `cancel_task_and_kill` use the helper?** No. It's shared infrastructure returning `anyhow::Result`, not a tool boundary. Layer violation.
- **Should `create_work_item`'s parent_task_id use the helper?** Deferred. It uses `get_task_depth` for depth calculation. Could be unified since `Task.depth` exists, but it changes the query pattern for no functional benefit in the parent validation path.

### Deferred to Implementation

- Exact method signature details (whether `field_name` should be generic or default to `"task_id"`)
- Whether `check_work_item` should switch from `get_manual_task` to `get_task` + post-filter (implementer's judgment on clarity vs query change)

## Implementation Units

- [x] **Unit 1: Add `validate_task_exists` helper**

**Goal:** Create the core helper function that combines UUID format validation with DB existence checking.

**Requirements:** R1, R2, R3, R4

**Dependencies:** None

**Files:**
- Modify: `crates/mika-agent/src/tools/mod.rs`
- Test: `crates/mika-agent/src/tools/mod.rs` (inline `#[cfg(test)]` module)

**Approach:**
- Add `validate_task_exists(db: &AsyncDatabase, field_name: &str, value: &str) -> Result<Task, ToolOutput>` as a `pub(crate) async fn` in `tools/mod.rs`
- Internally: call `validate_uuid(field_name, value)` first (format check), then `db.get_task(uuid_str)` (existence + agent scope)
- On `Ok(None)`: return structured JSON error `{"error": "task_not_found", "field": ..., "task_id": ..., "reason": "no task with this ID exists"}`
- On `Err(e)`: fail closed — return structured JSON error `{"error": "db_error", "field": ..., "reason": "..."}` (R4)
- On `Ok(Some(task))`: return `Ok(task)`

**Patterns to follow:**
- `validate_uuid()` in same file — same `Result<T, ToolOutput>` convention, same structured JSON pattern
- `validate_work_item()` — existing compound validator pattern

**Test scenarios:**
- Happy path: valid UUID that exists in DB for the calling agent -> returns `Ok(Task)`
- Error path: empty string -> returns `invalid_uuid` structured error (inherited from `validate_uuid`)
- Error path: malformed UUID ("not-a-uuid") -> returns `invalid_uuid` structured error
- Error path: well-formed UUID not in DB -> returns `task_not_found` structured error with correct field name and task_id
- Error path: UUID belonging to a different agent -> returns `task_not_found` (same as non-existent, no info disclosure)
- Edge case: verify error JSON is parseable and contains expected fields (`error`, `field`, `task_id`, `reason`)

**Verification:**
- `cargo test -p mika-agent -- validate_task_exists` passes
- Helper compiles and is importable from tool modules

- [x] **Unit 2: Adopt helper in simple task tools (get_task, complete_task)**

**Goal:** Replace the two-step format-check + DB-lookup pattern with the unified `validate_task_exists` call in the simplest tool handlers.

**Requirements:** R1, R3, R7

**Dependencies:** Unit 1

**Files:**
- Modify: `crates/mika-agent/src/tools/get_task.rs`
- Modify: `crates/mika-agent/src/tools/complete_task.rs`
- Test: `crates/mika-agent/src/tools/get_task.rs` (existing inline tests)
- Test: `crates/mika-agent/src/tools/complete_task.rs` (existing inline tests)

**Approach:**
- In each tool's `execute()`, replace the `validate_uuid` call + `db.get_task` match block with a single `validate_task_exists(ctx.db, "id", id)?` call that returns the `Task` directly
- The `?` operator with the `Result<Task, ToolOutput>` maps `Err(ToolOutput)` to `Ok(ToolOutput)` — need to handle via `match` or `.map_err()` since `execute` returns `Result<ToolOutput>`, not `Result<Task, ToolOutput>`. The pattern will be: `let task = match validate_task_exists(...).await { Ok(t) => t, Err(e) => return Ok(e) };`
- Business logic after the lookup stays unchanged

**Patterns to follow:**
- Existing `validate_uuid` usage pattern: `if let Err(e) = validate_uuid(...) { return Ok(e); }`

**Test scenarios:**
- Happy path: `get_task` with valid existing task_id -> same success output as before
- Happy path: `complete_task` with valid callback task_id + result -> task completed
- Error path: both tools with non-existent UUID -> now returns structured JSON `task_not_found` instead of plain text "not found"
- Regression: existing tests for empty id, invalid UUID, wrong trigger type (complete_task) all still pass

**Verification:**
- `cargo test -p mika-agent -- get_task` and `cargo test -p mika-agent -- complete_task` pass
- Error messages for "not found" now contain `task_not_found` JSON structure

- [x] **Unit 3: Adopt helper in work item tools (update_work_item_status, check_work_item, cancel_task)**

**Goal:** Extend adoption to the remaining tools that accept task UUID parameters.

**Requirements:** R1, R3, R7

**Dependencies:** Unit 1

**Files:**
- Modify: `crates/mika-agent/src/tools/update_work_item_status.rs`
- Modify: `crates/mika-agent/src/tools/check_work_item.rs`
- Modify: `crates/mika-agent/src/tools/cancel_task.rs`
- Test: `crates/mika-agent/src/tools/update_work_item_status.rs` (existing inline tests)
- Test: `crates/mika-agent/src/tools/check_work_item.rs` (existing inline tests)
- Test: `crates/mika-agent/src/tools/cancel_task.rs` (existing inline tests)

**Approach:**
- `update_work_item_status`: replace `validate_uuid("task_id", ...)` + `db.get_task(...)` with `validate_task_exists(ctx.db, "task_id", ...)`. Keep the `trigger_type == "manual"` post-check.
- `check_work_item`: replace `validate_uuid("task_id", ...)` + `db.get_manual_task(...)` with `validate_task_exists(ctx.db, "task_id", ...)`. Add `trigger_type == "manual"` post-check (currently done by the specialized query).
- `cancel_task`: replace `validate_uuid("id", ...)` with `validate_task_exists(ctx.db, "id", ...)` for the pre-check. The `cancel_task_and_kill` call that follows still does its own `get_task` internally — this adds an existence check before delegation, catching fabricated IDs before they reach the infrastructure layer.

**Patterns to follow:**
- Unit 2's adoption pattern
- `update_work_item_status` already has post-lookup trigger_type checking to preserve

**Test scenarios:**
- Happy path: `update_work_item_status` with valid manual task -> status updated
- Happy path: `check_work_item` with valid manual task -> enriched status returned
- Happy path: `cancel_task` with valid cancellable task -> task cancelled
- Error path: all three tools with non-existent UUID -> structured `task_not_found` JSON error
- Regression: `update_work_item_status` with non-manual task -> still gets "only manual tasks" error
- Regression: `check_work_item` with non-manual task -> still gets appropriate error
- Regression: `cancel_task` with non-cancellable status -> still gets appropriate error
- Edge case: `cancel_reminder` (delegates to `cancel_task`) inherits the new validation transitively

**Verification:**
- `cargo test -p mika-agent -- update_work_item_status` and `cargo test -p mika-agent -- check_work_item` and `cargo test -p mika-agent -- cancel_task` pass

- [x] **Unit 4: Refactor `validate_work_item` to layer on `validate_task_exists`**

**Goal:** Eliminate duplication in `validate_work_item()` by using the new helper as its base layer.

**Requirements:** R5

**Dependencies:** Unit 1

**Files:**
- Modify: `crates/mika-agent/src/tools/mod.rs`
- Test: `crates/mika-agent/src/tools/mod.rs` (inline tests for `validate_work_item`)

**Approach:**
- Refactor `validate_work_item()` to call `validate_task_exists(db, "work_item_id", work_item_id)` instead of separate `validate_uuid` + `db.get_task` calls
- Keep the empty-string check with its custom message ("You must create a work item first...")
- Keep the trigger_type=manual + active status checks on the returned `Task`
- Keep the return type as `Option<String>` — map `validate_task_exists` errors to `Some(error.content)`
- On DB error (which `validate_task_exists` now surfaces as structured error), map to `Some(error_message)`

**Patterns to follow:**
- Existing `validate_work_item()` structure and error messages

**Test scenarios:**
- Happy path: active manual work item -> `None` (valid)
- Error path: empty work_item_id -> `Some` with "create a work item first" message (unchanged)
- Error path: malformed UUID -> `Some` with `invalid_uuid` JSON
- Error path: non-existent UUID -> `Some` with `task_not_found` JSON (new structured error)
- Error path: non-manual task -> `Some` with "not an active work item" message (unchanged)
- Error path: completed/cancelled manual task -> `Some` with "not an active work item" message
- Integration: `delegate_task` still works end-to-end with `validate_work_item` changes

**Verification:**
- `cargo test -p mika-agent -- validate_work_item` passes
- `cargo test -p mika-agent -- delegate_task` passes (downstream consumer)

- [x] **Unit 5: Cross-agent test coverage and solution doc**

**Goal:** Add explicit cross-agent test cases and update the existing solution documentation.

**Requirements:** R2, R6

**Dependencies:** Units 1-4

**Files:**
- Modify: `crates/mika-agent/src/tools/mod.rs` (add cross-agent tests)
- Modify: `docs/solutions/best-practices/uuid-validation-at-tool-boundary.md`

**Approach:**
- Add test that creates a task with agent_id "agent-a", then attempts `validate_task_exists` with a db scoped to "agent-b" — should return `task_not_found`
- Add test verifying the error JSON does NOT reveal that the task exists for another agent (same error as non-existent)
- Update the existing solution doc to document the new existence validation layer, structured error format, and the layered validation chain: `validate_uuid` (format) -> `validate_task_exists` (format + existence) -> `validate_work_item` (format + existence + business rules)

**Patterns to follow:**
- `TestHarness` usage pattern in existing tool tests
- `AsyncDatabase::new_with_agent` for creating a differently-scoped DB in tests

**Test scenarios:**
- Error path: well-formed UUID created by agent-a, validated by agent-b -> `task_not_found` with no hint about other agent
- Error path: well-formed UUID not in DB at all, validated by agent-b -> identical error structure as cross-agent case
- Happy path: task created and validated by same agent -> `Ok(Task)`

**Verification:**
- All cross-agent tests pass
- `docs/solutions/best-practices/uuid-validation-at-tool-boundary.md` documents the three-layer validation chain
- Full test suite: `cargo test -p mika-agent` passes with no regressions

## System-Wide Impact

- **Error propagation:** Tool error messages change from plain text ("Task not found") to structured JSON (`{"error": "task_not_found", ...}`). LLMs consuming these errors get better self-correction signals. Human-readable message is embedded in the `reason` field.
- **Interaction graph:** `validate_task_exists` is called from tool `execute()` methods only. `cancel_task_and_kill`, dashboard endpoints, and team engine internals are unaffected.
- **State lifecycle risks:** None. The helper is read-only (DB lookup), not a mutation.
- **API surface parity:** No external API changes. Internal tool error format changes are additive (structured JSON is a superset of the information in plain text).
- **Unchanged invariants:** `cancel_task_and_kill` in `process_kill.rs` continues to use its own `db.get_task()` call. Dashboard `get_task_unscoped` paths are untouched. Team engine cross-agent task access patterns are unaffected.

## Risks & Dependencies

| Risk | Mitigation |
|------|------------|
| Existing tests assert on plain-text "not found" messages — will break | Update test assertions to match new structured JSON format. Systematic search for "not found" assertions in affected tool test modules. |
| Extra DB round-trip in `cancel_task` (helper does `get_task`, then `cancel_task_and_kill` does another) | Single-row indexed lookup on primary key — sub-millisecond. Acceptable for the safety benefit. |
| `check_work_item` switches from `get_manual_task` to `get_task` + post-filter | Both are single-row indexed lookups. The manual-filter moves from SQL to Rust. No performance concern. |

## Sources & References

- Related issues: #596 (this issue), #531 (format-only validation), #595 (incident that triggered this)
- Related code: `crates/mika-agent/src/tools/mod.rs` (validate_uuid, validate_work_item)
- Related docs: `docs/solutions/best-practices/uuid-validation-at-tool-boundary.md`
