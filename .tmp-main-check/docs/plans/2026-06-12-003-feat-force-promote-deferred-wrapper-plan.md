---
title: "feat: operator + agent verb to force-promote next deferred wrapper"
date: 2026-06-12
type: feat
issue: 1453
origin: "mika#1453 (inline ACs, rev 2 — F1/F2/F3 resolved, GROOMED)"
depth: Standard
---

## Summary

Add two symmetric surfaces for manually force-promoting the next pending `:deferred` dispatch wrapper for a given dispatch class: a CLI verb (`mika tasks promote-deferred <class>`) and an agent tool (`promote_deferred_callback`). The CLI supports a `--override` flag for cancel-then-promote when the slot is busy; the agent tool is fail-closed only (no override). Both emit structured audit events and share the existing `has_any_active_callback_for_class()` slot predicate.

---

## Problem Frame

DB primitives for deferred wrapper promotion exist (`promote_next_deferred_callback`, `promote_next_deferred_callback_for_class`) but are dispatcher-internal. Operators currently must cancel the slot-occupying callback indirectly and wait for the periodic backstop to promote on the next tick. No agent-side tool exists for programmatic promotion. This is the W2 parity gap from mika#1176.

---

## Requirements

- **R1 (AC1):** CLI verb `mika tasks promote-deferred <class>` that fails with a clear message and non-zero exit when the slot is occupied, unless `--override` is set.
- **R2 (AC2):** Agent tool `promote_deferred_callback` with fail-closed semantics only — no override path. Returns structured error when slot busy, instructing the agent to surface to operator.
- **R3 (AC3):** Slot-availability check reuses existing `has_any_active_callback_for_class()` — no predicate duplication.
- **R4 (AC4):** Three audit event types: `deferred_dispatch_force_promote_succeeded`, `deferred_dispatch_force_promote_rejected_slot_busy`, `deferred_dispatch_force_promote_override`.
- **R5 (AC5a):** Regression test — 2 pending wrappers + 1 in-flight real callback; force-promote without override → rejection with audit event, no state mutation.
- **R6 (AC5b):** Regression test — same setup; force-promote with override → cancel-then-promote with audit trail.
- **R7 (AC6):** `crates/mika-agent/CLAUDE.md` updated with force-promote verb in the deferred-dispatch lifecycle paragraph.

---

## Key Technical Decisions

**KTD-1: Shared force-promote function in `db.rs`.**
Both CLI and agent tool call the same new `force_promote_deferred_for_class(agent_id, class) -> Result<ForcePromoteResult>` method on `Database`. This method calls `has_any_active_callback_for_class()` then conditionally `promote_next_deferred_callback_for_class()`. The result enum carries `Promoted { task_id }`, `RejectedSlotBusy { blocking_label }`, or `NoPendingWrapper`. This prevents predicate duplication (R3) and keeps the slot-check + promote atomic within the same DB connection.

**KTD-2: Override is CLI-only cancel-then-promote.**
The `--override` flag on the CLI verb cancels the slot-occupying callback via the existing `cancel_task_and_kill()` path, then retries force-promote. The agent tool has no override path per AC2 — the agent must surface to operator via `send_message` if promotion is needed under busy conditions. This preserves the per-class-single-slot invariant (mika#1163).

**KTD-3: Override identifies the blocker via a new DB query.**
When force-promote returns `RejectedSlotBusy`, the CLI override path needs the task ID of the slot-occupying callback. A new `find_active_callback_for_class(agent_id, class) -> Option<Task>` query (same predicate as `has_any_active_callback_for_class` but returns the row) provides this. Used only by the override path.

**KTD-4: Audit events use the existing `log_audit_event` shape.**
Tool name: `"force_promote_deferred"`. Target key: `"dispatch_class:<class>"`. Before/after values carry the state change. This mirrors the existing `deferred_dispatch_promoted` event shape from mika#1172.

---

## Scope Boundaries

### In scope

- New `Database::force_promote_deferred_for_class()` method with `ForcePromoteResult` enum
- New `Database::find_active_callback_for_class()` query for the override path
- Async wrappers in `AsyncDatabase`
- CLI `PromoteDeferred` variant in `TaskCommand` enum
- Agent tool `promote_deferred_callback` in `tools/`
- Audit events for all three outcomes
- Regression tests for deadlock-safe invariant
- CLAUDE.md update

### Deferred to Follow-Up Work

- HTTP endpoint for force-promote (dashboard surface — separate ticket if needed)
- Auto-promote policy tuning
- Broader operator-override audit-trail design

---

## Implementation Units

### U1. DB methods and result type

**Goal:** Add `force_promote_deferred_for_class()` and `find_active_callback_for_class()` to `Database`, plus async wrappers.

**Requirements:** R3, R4

**Dependencies:** None

**Files:**
- `crates/mika-agent/src/db.rs` — new methods after `promote_next_deferred_callback_for_class`
- `crates/mika-agent/src/async_db.rs` — async wrappers
- `crates/mika-agent/src/db.rs` (or a new `crates/mika-agent/src/task_engine/force_promote.rs`) — `ForcePromoteResult` enum

**Approach:**
- `ForcePromoteResult` enum with three variants: `Promoted { task_id: String }`, `RejectedSlotBusy { blocking_label: String }`, `NoPendingWrapper`.
- `force_promote_deferred_for_class(&self, agent_id, dispatch_class) -> Result<ForcePromoteResult>`: calls `has_any_active_callback_for_class()` first; if busy returns `RejectedSlotBusy` with the blocker's label (from a SELECT); if free calls `promote_next_deferred_callback_for_class()` and returns `Promoted` or `NoPendingWrapper`.
- `find_active_callback_for_class(&self, agent_id, dispatch_class) -> Result<Option<String>>`: returns the task ID of the slot-occupying non-deferred callback. Same SQL predicate as `has_any_active_callback_for_class` but `SELECT id LIMIT 1` instead of `SELECT COUNT(*)`. Paired-predicate comment referencing the sibling.
- Async wrappers follow the existing `with_db` pattern.
- Place `ForcePromoteResult` in `db.rs` near the existing promote methods (it's a DB-layer result type).

**Patterns to follow:**
- `promote_next_deferred_callback_for_class` at `db.rs:6158` for SQL shape
- `has_any_active_callback_for_class` at `db.rs:6220` for predicate shape
- Async wrapper pattern at `async_db.rs:636`

**Test scenarios:**
- Slot free + 1 pending wrapper → `Promoted` with correct task_id
- Slot free + 0 pending wrappers → `NoPendingWrapper`
- Slot busy (1 real callback) + 1 pending wrapper → `RejectedSlotBusy` with blocker label
- Slot busy + 0 pending wrappers → `RejectedSlotBusy` (slot check fires before wrapper check)
- `find_active_callback_for_class` returns `Some(id)` when real callback exists, `None` when only deferred wrappers exist
- Pre-v34 NULL dispatch_class rows treated as `'implement'` via COALESCE

**Verification:** `cargo test -p mika-agent` passes with new unit tests covering all variants.

---

### U2. Agent tool — `promote_deferred_callback`

**Goal:** Add the agent-side tool with fail-closed semantics and audit event emission.

**Requirements:** R2, R3, R4

**Dependencies:** U1

**Files:**
- `crates/mika-agent/src/tools/promote_deferred_callback.rs` — new tool module
- `crates/mika-agent/src/tools/mod.rs` — register in `default_tools()`, add `mod` declaration

**Approach:**
- Input schema: `{ "dispatch_class": "implement" | "groom" }`. Required field, validated against known classes.
- Calls `ctx.db.force_promote_deferred_for_class(dispatch_class)`.
- On `Promoted`: emits `deferred_dispatch_force_promote_succeeded` audit event, returns success with promoted task ID.
- On `RejectedSlotBusy`: emits `deferred_dispatch_force_promote_rejected_slot_busy` audit event, returns `ToolOutput::error` with structured JSON explaining the rejection and instructing the agent to surface to operator via `send_message`.
- On `NoPendingWrapper`: returns `ToolOutput::error` with message "No pending deferred wrapper for class '<class>'".
- Registered in `default_tools()` alongside `cancel_task` (all agents, not gated behind multi-agent check — deferred dispatch is an engine-level concern).

**Patterns to follow:**
- `cancel_task.rs` for tool structure, audit event emission, and `validate_task_exists` pattern
- `ToolOutput::success()` / `ToolOutput::error()` for return shapes
- Audit event format: `tool_name = "force_promote_deferred"`, `target_key = "dispatch_class:<class>"`

**Test scenarios:**
- Happy path: slot free, pending wrapper → success response with task ID, audit event emitted
- Slot busy → error response with structured rejection, audit event emitted
- No pending wrapper → error response, no audit event
- Invalid dispatch class → validation error
- Missing dispatch_class field → validation error

**Verification:** `cargo test -p mika-agent` passes. Tool appears in `default_tools()` registry.

---

### U3. CLI verb — `mika tasks promote-deferred`

**Goal:** Add the CLI subcommand with `--override` flag.

**Requirements:** R1, R4

**Dependencies:** U1

**Files:**
- `crates/mika-cli/src/cli.rs` — add `PromoteDeferred` variant to `TaskCommand` enum
- `crates/mika-cli/src/commands/tasks.rs` — handler implementation

**Approach:**
- `TaskCommand::PromoteDeferred { class: String, r#override: bool }` — `class` is a positional arg, `--override` is a flag.
- Handler flow:
  1. Call `db.force_promote_deferred_for_class(&class)`.
  2. On `Promoted`: print success message with task ID. Emit `deferred_dispatch_force_promote_succeeded` audit event.
  3. On `RejectedSlotBusy`:
     - Without `--override`: print rejection message with blocker label, exit non-zero.
     - With `--override`:
       a. Call `db.find_active_callback_for_class(&class)` to get blocker task ID.
       b. Call `cancel_task_and_kill(db, &blocker_id)` — reuses existing cancel path.
       c. Emit `deferred_dispatch_force_promote_override` audit event.
       d. Retry `db.force_promote_deferred_for_class(&class)`.
       e. On `Promoted`: print success. On other: print error and exit non-zero.
  4. On `NoPendingWrapper`: print "No pending deferred wrapper" message, exit non-zero.
- Audit events emitted via `db.log_audit_event()` with `session_id = "cli"` (no session context in CLI).

**Patterns to follow:**
- `TaskCommand::Cancel` handler in `tasks.rs` for cancel_task_and_kill usage
- CLI exit conventions: `std::process::exit(1)` for non-zero exits, or `anyhow::bail!`

**Test scenarios:**
- Slot free + pending wrapper → success message printed
- Slot busy without override → rejection message, non-zero exit
- Slot busy with override → cancel + promote, success message
- No pending wrapper → informational message, non-zero exit
- Invalid class name → validation error

**Verification:** `cargo test -p mika-cli` passes. `cargo build --bin mika` compiles. Manual test: `mika tasks promote-deferred implement`.

---

### U4. Regression tests — deadlock-safe invariant

**Goal:** Prove that force-promote with two pending wrappers + one in-flight callback preserves the per-class-single-slot invariant.

**Requirements:** R5, R6

**Dependencies:** U1

**Files:**
- `crates/mika-agent/src/db.rs` — inline `#[cfg(test)] mod tests` section, or `crates/mika-agent/tests/` integration test

**Approach:**
- Test setup (shared across AC5a and AC5b):
  - Create 2 pending deferred callbacks (label `long_running:run_claude_pilot:deferred`, `dispatch_class = 'implement'`).
  - Create 1 in-flight real callback (`status = 'in_progress'`, non-deferred label, `dispatch_class = 'implement'`).
- **AC5a test:** Call `force_promote_deferred_for_class(agent_id, "implement")`.
  - Assert result is `RejectedSlotBusy`.
  - Assert both pending wrappers are still `pending` (no state mutation).
  - Assert audit event `deferred_dispatch_force_promote_rejected_slot_busy` was emitted (query `audit_events`).
- **AC5b test:** Simulate override path:
  1. Call `force_promote_deferred_for_class` → `RejectedSlotBusy`.
  2. Call `find_active_callback_for_class` → get blocker ID.
  3. Cancel the blocker via `db.cancel_task(blocker_id)`.
  4. Call `force_promote_deferred_for_class` again → `Promoted`.
  5. Assert only one of the two wrappers was promoted (the older one).
  6. Assert `has_any_active_callback_for_class` is now `false` (no real callback active — only the remaining pending wrapper).
  7. Assert audit events: both `deferred_dispatch_force_promote_override` and `deferred_dispatch_force_promote_succeeded`.

**Patterns to follow:**
- `cancel_task.rs` tests for `TestHarness` usage and `NewTask` setup
- `engine.rs` tests for deferred dispatch wrapper setup patterns

**Test scenarios:**
- AC5a: reject with audit when slot occupied, no state mutation
- AC5b: cancel-then-promote with audit trail, per-class invariant holds (≤1 active real callback)

**Verification:** `cargo test -p mika-agent` passes with all new tests green.

---

### U5. CLAUDE.md documentation update

**Goal:** Update deferred-dispatch lifecycle paragraph in `crates/mika-agent/CLAUDE.md`.

**Requirements:** R7

**Dependencies:** U1, U2, U3

**Files:**
- `crates/mika-agent/CLAUDE.md` — deferred dispatch lifecycle paragraph

**Approach:**
- Add a paragraph after the existing promotion paths section describing:
  - (3) Force-promote — `promote_deferred_callback` agent tool (fail-closed) and `mika tasks promote-deferred <class>` CLI verb (with `--override` for cancel-then-promote). Three audit event types.
  - Reference mika#1453.

**Test expectation:** none — documentation only.

**Verification:** Content accurately reflects the implementation.

---

## Open Questions

None — the issue body ACs (rev 2) resolved all design questions (F1: agent tool is fail-closed only; F2: AC5a uses RejectedSlotBusy assertion not task count; F3: override is cancel-then-promote).

---

## Sources & Research

- mika#1453 issue body (GROOMED, rev 2 — F1/F2/F3 resolved)
- mika#1163 deadlock fix — slot-availability predicate parity is the highest-priority structural concern
- mika#1172 — sibling where W1/W3/W4/W5/R9 shipped
- `docs/solutions/logic-errors/deferred-dispatch-promotion-deadlock-2026-05-10.md` — three-defect + fourth-defect deadlock history
- `docs/solutions/architecture-patterns/asymmetric-perimeter-predicate-drift.md` — predicate parity pattern
- `docs/solutions/best-practices/per-class-dispatch-slot-2026-05-11.md` — dispatch_class semantics
- `docs/solutions/best-practices/class-dimension-audit-2026-05-17.md` — six-step audit checklist for class-dimension changes
