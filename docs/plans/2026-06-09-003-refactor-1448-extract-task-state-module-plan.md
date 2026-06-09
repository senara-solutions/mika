---
title: "refactor: Extract task_state/ into its own module directory"
status: active
origin: "mika#1448 (sub-issue of mika#1259 — Layer 3 domain refactor)"
created: 2026-06-09
---

## Summary

Extract task lifecycle logic — status transition rules, the LLM-facing `UpdateTaskStatusTool`, and status-transition DB write methods — into a dedicated `task_state/` module under `crates/mika-agent/src/`. This is a pure code-relocation refactor per Foundation doc §6's domain boundary for task_state: "Task lifecycle: created → in_progress → blocked → done. Status transition rules."

The extraction moves ~1,750 lines from two primary sources (`tools/update_task_status.rs` and `db.rs`) into two new locations (`task_state/` top-level module and `db/task_state.rs` sub-module), following the existing `db/operational.rs` and `db/kg_schema.rs` convention for DB method grouping.

---

## Problem Frame

`crates/mika-agent/src/db.rs` is 17,650 lines and `tools/` hosts a 1,520-line file (`update_task_status.rs`) that is really a state machine module wearing a tool skin. The decomposition plan (mika#1259) identifies task_state/ as a leaf module with no hard dependencies on other #1259 modules, making it safe to extract independently.

The extraction creates a clear architectural seam: **task_state/** owns the rules (what transitions are valid, validation predicates), while **task_engine/** owns the execution (scheduling, dispatching, process liveness). Today these concerns are interleaved across db.rs and tools/.

---

## Requirements

- **R1.** Create `crates/mika-agent/src/task_state/mod.rs` with one-paragraph doc-comment naming the operational responsibility (parent AC4).
- **R2.** Move task lifecycle logic from `tools/update_task_status.rs` and status-transition DB methods from `db.rs` into the new module structure (parent AC3 — identical logic, no behavior change).
- **R3.** Declare the new module in `lib.rs` (parent AC1 — adopts Foundation §6 boundary).
- **R4.** `cargo test -p mika-agent` passes unchanged (parent AC2).
- **R5.** `cargo clippy -p mika-agent --tests --no-deps -- -D warnings` clean (ticket AC).

---

## Key Technical Decisions

**KTD1. Two extraction targets, not one.** The state machine rules and LLM tool move to the top-level `task_state/` module. The DB write methods move to `db/task_state.rs` (an `impl Database` block in a sub-module of db, following the `db/operational.rs` and `db/kg_schema.rs` pattern). Rationale: `impl Database` methods need `&self` access to `self.conn` — pulling them out of the `db` module entirely would require passing the connection explicitly, which is a behavioral change and violates AC3.

**KTD2. `task_metadata.rs` stays where it is.** `merge_metadata()` is a shared utility consumed by both `task_state/update_tool.rs` and `task_engine/dispatcher.rs`. It is not exclusively task-state logic. Moving it would create a false ownership signal. The existing `crate::task_metadata` path continues to work for both consumers.

**KTD3. `tools/mod.rs` re-exports the tool from its new home.** `UpdateTaskStatusTool` moves to `task_state/update_tool.rs` but `tools/mod.rs` continues to reference it for tool registration in `default_tools()`. The import path changes from `super::update_task_status::UpdateTaskStatusTool` to `crate::task_state::UpdateTaskStatusTool`. The `pub mod update_task_status;` declaration in `tools/mod.rs` is removed.

**KTD4. Transition functions become `pub`.** `is_valid_transition()`, `allowed_transitions()`, `VALID_STATUSES`, `VALID_TRANSITIONS`, and `has_retry_semantic_keys()` are currently `fn` (private to tools/update_task_status.rs). They become `pub` in `task_state/transitions.rs` so that future modules (e.g., `commitments/` from #1259-F) can use them without duplicating the state machine.

---

## Scope Boundaries

### In Scope

- Creating the `task_state/` module directory with `mod.rs` and `transitions.rs`
- Moving `tools/update_task_status.rs` → `task_state/update_tool.rs`
- Moving status-transition DB methods → `db/task_state.rs`
- Updating all import paths across the crate
- Removing the now-empty `tools/update_task_status.rs`

### Out of Scope

- Other #1259 sub-issue modules (evidence/, commitments/, etc.)
- Restructuring `task_engine/` — it continues to call DB methods via `db.update_task_status()` etc.
- Moving query methods (list_manual_tasks, get_tasks_by_status) — those are read-side, closer to the future dashboard_queries/ module
- Changing any function signatures, return types, or behavior
- Adding new tests — existing coverage is the regression gate

### Deferred to Follow-Up Work

- Cross-module interface improvements (e.g., task_engine calling task_state transition validators before DB writes) — belongs in a future interface-tightening ticket after all #1259 modules exist.

---

## Implementation Units

### U1. Create `task_state/` module with transition rules

**Goal:** Establish the module directory and extract the state machine constants and validation functions.

**Requirements:** R1, R3

**Dependencies:** None — leaf unit.

**Files:**
- Create `crates/mika-agent/src/task_state/mod.rs`
- Create `crates/mika-agent/src/task_state/transitions.rs`
- Modify `crates/mika-agent/src/lib.rs`

**Approach:**
- Create `task_state/mod.rs` with the required one-paragraph doc-comment: "Task lifecycle state machine: status constants, transition validation rules, and the LLM-facing `UpdateTaskStatusTool`. Owns the domain logic for task status transitions (created → in_progress → blocked → done)."
- `mod.rs` declares `pub mod transitions;` and `pub mod update_tool;` (update_tool added in U2), and re-exports key public items from transitions.
- `transitions.rs` receives from `tools/update_task_status.rs`:
  - `VALID_STATUSES` (line 8) — made `pub`
  - `VALID_TRANSITIONS` (line 17) — made `pub`
  - `is_valid_transition()` (line 29) — made `pub`
  - `allowed_transitions()` (line 38) — made `pub`
  - `MAX_METADATA_LEN` (line 47) — made `pub`
  - `has_retry_semantic_keys()` (line 273) — made `pub`
- Add `pub mod task_state;` to `lib.rs`.

**Patterns to follow:** The existing `operational/` module structure (`operational/mod.rs` + `operational/types.rs`).

**Test scenarios:**
- `cargo test -p mika-agent` compiles — confirms no broken imports from the new module declaration.

**Verification:** `lib.rs` declares the module, `task_state/mod.rs` has the doc-comment, `transitions.rs` exports all six items.

---

### U2. Move `UpdateTaskStatusTool` to `task_state/update_tool.rs`

**Goal:** Relocate the tool implementation and all its tests from `tools/` to `task_state/`.

**Requirements:** R2, R4, R5

**Dependencies:** U1 (transitions.rs must exist for imports)

**Files:**
- Create `crates/mika-agent/src/task_state/update_tool.rs`
- Delete `crates/mika-agent/src/tools/update_task_status.rs`
- Modify `crates/mika-agent/src/tools/mod.rs`
- Modify `crates/mika-agent/src/task_state/mod.rs`

**Approach:**
- Move the entire contents of `tools/update_task_status.rs` to `task_state/update_tool.rs`.
- Replace the local constants/functions (VALID_STATUSES, VALID_TRANSITIONS, is_valid_transition, allowed_transitions, MAX_METADATA_LEN, has_retry_semantic_keys) with imports from `super::transitions`.
- Keep `merge_and_persist_metadata()` as a private helper within `update_tool.rs` — it's tool-specific glue that calls `crate::task_metadata::merge_metadata`.
- Update `use` statements: `super::{MAX_INPUT_LEN, Tool, ToolContext, ToolOutput}` becomes `crate::tools::{MAX_INPUT_LEN, Tool, ToolContext, ToolOutput}`.
- In `tools/mod.rs`: remove `pub mod update_task_status;`, change the tool registration in `default_tools()` from `update_task_status::UpdateTaskStatusTool` to `crate::task_state::UpdateTaskStatusTool`.
- In `task_state/mod.rs`: add `pub mod update_tool;` and `pub use update_tool::UpdateTaskStatusTool;`.
- Delete the now-empty `tools/update_task_status.rs`.

**Patterns to follow:** The existing tool-in-module pattern — tools are structs that implement the `Tool` trait; their physical location doesn't affect registration as long as `default_tools()` can reach them.

**Test scenarios:**
- All ~30 existing tests in the `#[cfg(test)] mod tests` block compile and pass from their new location. These tests use `TestHarness` fixtures and exercise the full state machine (transitions, metadata, retry guards). No new tests needed — they ARE the regression gate.
- `cargo clippy -p mika-agent --tests --no-deps -- -D warnings` clean — no dead imports or unused code warnings from the move.

**Verification:** `tools/update_task_status.rs` no longer exists, `tools/mod.rs` no longer declares it, all 30 tool tests pass from `task_state/update_tool.rs`.

---

### U3. Move status-transition DB methods to `db/task_state.rs`

**Goal:** Group the task status-transition write methods into a dedicated DB sub-module, reducing db.rs line count by ~200 lines.

**Requirements:** R2, R4, R5

**Dependencies:** U1 (conceptually; no code dependency — this unit only touches db/)

**Files:**
- Create `crates/mika-agent/src/db/task_state.rs`
- Modify `crates/mika-agent/src/db.rs`

**Approach:**
- Create `db/task_state.rs` containing an `impl Database` block with these methods cut from db.rs:
  - `update_task_status()` (line 4855)
  - `update_task_execution_trace_id()` (line 4863)
  - `claim_and_fire_task()` (line 4877)
  - `update_task_completed()` (line 4888)
  - `update_task_failed()` (line 4903)
  - `update_task_dispatch_class()` (line 4920)
  - `write_task_dispatch_rejection()` (line 4942)
  - `promote_task_completed()` (line 4957)
  - `update_task_next_fire_at()` (line 4968)
  - `update_task_rescheduled()` (line 4978)
  - `cancel_task()` (line 4986)
  - `update_manual_task_status()` (line 5011)
- Add `pub mod task_state;` to the top of `db.rs` (alongside existing `pub mod kg_schema;` and `pub mod operational;`).
- The new file needs `use anyhow::Result;` and `use rusqlite::{params, OptionalExtension};` — match the exact imports each method needs.
- Import `super::Database;` to write the `impl Database` block.
- Preserve all doc-comments verbatim — they contain issue references and behavioral contracts.

**Patterns to follow:** `db/operational.rs` and `db/kg_schema.rs` — both define `impl Database` blocks in separate files under `db/`.

**Test scenarios:**
- All existing tests that call these DB methods (task engine tests, tool tests from U2, integration tests) compile and pass. The methods are still on `Database` with identical signatures — callers see no change.
- `cancel_task()` cascade behavior (callback children cancellation, mika#1011) continues to work — the method body includes a second SQL statement that cascades; this must move atomically with the method.

**Verification:** `db.rs` declares `pub mod task_state;`, the 12 methods are no longer in `db.rs`, `cargo test -p mika-agent` passes, `cargo clippy` clean.

---

### U4. Final import audit and cleanup

**Goal:** Ensure no stale imports, unused modules, or broken cross-references remain.

**Requirements:** R4, R5

**Dependencies:** U1, U2, U3

**Files:**
- Potentially modify any file in `crates/mika-agent/src/` that imports from the moved locations

**Approach:**
- Run `cargo clippy -p mika-agent --tests --no-deps -- -D warnings` and fix any warnings.
- Grep for `tools::update_task_status` across the crate to find any remaining references — they should all be gone after U2.
- Grep for references to moved DB method line numbers in doc-comments (these are informational, not functional, but worth updating if found in the same crate).
- Verify `tools/mod.rs` no longer has a dead `pub mod update_task_status;` line.
- Run `cargo test -p mika-agent` as the final gate.

**Test scenarios:**
- `cargo test -p mika-agent` — full test suite (~3,463 tests) passes.
- `cargo clippy -p mika-agent --tests --no-deps -- -D warnings` — zero warnings.
- `cargo build` — full workspace builds clean.

**Verification:** Green CI-equivalent locally. No behavioral changes — pure module relocation confirmed by identical test results.

---

## Open Questions

None — this is a well-bounded mechanical refactor with clear source and target locations. The decomposition plan and Foundation doc §6 define the boundary; the existing `db/operational.rs` pattern validates the DB extraction approach.

---

## Sources & Research

- **Foundation doc:** `docs/architecture/operational-partner-frame.md` §6 — defines task_state/ operational responsibility
- **Decomposition plan:** `docs/plans/2026-06-08-001-meta-1259-decomposition-plan.md` (commit 7a24fac8) — sequencing, LoC estimates, dependency graph
- **Existing patterns:** `db/operational.rs`, `db/kg_schema.rs` — DB sub-module convention; `operational/` — top-level module with mod.rs + sub-files
- **Parent ticket:** mika#1259 — Layer 3 domain refactor acceptance criteria
