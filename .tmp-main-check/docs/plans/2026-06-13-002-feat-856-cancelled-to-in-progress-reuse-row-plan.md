---
ticket: mika#856
branch: feat/856/cancelled-to-in-progress-reuse-row
status: active
date: 2026-06-13
origin: https://github.com/senara-solutions/mika/issues/856
execution: code
---

# Plan: allow `cancelled → in_progress` reusing existing task row (mika#856)

## Problem frame

Reverting a cancelled task back to `in_progress` (e.g. on mika#803) currently materializes a brand-new task row instead of reusing the original. One user-level "cancel and retry" produces: original task → cancelled record → status revert → new task. This creates graveyard noise — one extra info row for nothing.

## Resolution of first-pass findings

**F1 (BLOCKING) — Four-surface terminal-state invariant.** The architect named four surfaces that encode `cancelled` as terminal. Plan addresses each:

| Surface | Location | Resolution |
|---------|----------|------------|
| 1. Tool-layer state machine | `crates/mika-agent/src/tools/update_task_status.rs:25` `VALID_TRANSITIONS` constant | **Update.** Change `("cancelled", &[])` to `("cancelled", &["in_progress"])`. |
| 2. Tool description + agent-facing CLAUDE.md | `update_task_status.rs:60-68` tool description; root `CLAUDE.md` and `crates/mika-agent/CLAUDE.md` § Task Tracking → "Status transition state machine" line | **Update.** Replace "Completed and cancelled are terminal" with "Completed is terminal. Cancelled can return to in_progress (reuses the row)." All agents read the tool description on every turn — the contract must surface there. |
| 3. A2A `TaskStateMachine` | `crates/mika-a2a/src/state_machine.rs` | **Keep asymmetric — A2A `Canceled` remains terminal.** A2A is a cross-agent wire protocol with its own spec; allowing `Canceled → Working` would be an A2A spec amendment. Out of scope for this ticket. The plan documents the asymmetry: internal `tasks` table allows `cancelled → in_progress`; A2A protocol does not. Reopen as a follow-up if/when an A2A consumer needs un-cancel. |
| 4. `validate_task()` and active-status callers | `crates/mika-agent/src/tools/mod.rs:402-413` | **No change needed.** `validate_task()` checks `pending \| in_progress \| blocked` as "active." This is forward-looking based on current status: after a successful revert, the task IS in_progress, so it passes. Pre-revert, it remained cancelled (non-active). The check naturally handles the transition. Verified by reading the function body. |

The plan also greps for the literal string "Completed and cancelled are terminal" across the repo as part of U5 (docs sweep) to catch any other surface that documents the invariant.

**F2 (sharpening) — `cancelled_at` column handling.** Empirically verified: there is **NO `cancelled_at` column on the `tasks` table**. `grep -rn cancelled_at crates/` finds only `cancelled_reason` (a JSON metadata key, not a column). The `tasks` table schema (`db.rs:1304-1349`) has `created_at`, `updated_at`, `fired_at`, `completed_at` — no separate cancelled timestamp. The architect's Q3 concern was based on a speculative column that doesn't exist.

**Result:** the revert just updates the `status` column to `in_progress` and `updated_at` to now. No migration, no novel-state-combination, no `cancelled_at IS NOT NULL` consumer search needed (the column doesn't exist for callers to depend on). `cancelled_reason` in metadata is informational only — if the agent wants to preserve it for audit, it stays; if the operator wants it cleared, the implementer can choose. The plan recommends leaving `cancelled_reason` in metadata so a future query can see "this task was once cancelled" without scaffolding a transition log.

## Scope boundaries

- Update `VALID_TRANSITIONS` to allow `cancelled → in_progress` only.
- Update `update_task_status` tool description.
- Update `CLAUDE.md` documentation in two places (root + crates/mika-agent).
- Grep + sweep for any other "cancelled is terminal" assertion in docs.
- A2A protocol stays asymmetric (documented as a deliberate choice).
- **Out of scope:** `cancelled → blocked` or `cancelled → pending` (only `in_progress` makes sense per the issue body's wording: "cancel and retry"); A2A `TaskStateMachine` change (separate ticket if needed); deletion of `cancelled_reason` metadata on revert; a formal transition log table.

## Implementation Units

### U1 — Update `VALID_TRANSITIONS`

**Goal:** Allow `cancelled → in_progress`.

**Files:**
- Modify: `crates/mika-agent/src/tools/update_task_status.rs:17-26`

**Approach:**

```rust
const VALID_TRANSITIONS: &[(&str, &[&str])] = &[
    (
        "pending",
        &["in_progress", "blocked", "completed", "cancelled"],
    ),
    ("in_progress", &["blocked", "completed", "cancelled"]),
    ("blocked", &["in_progress", "completed", "cancelled"]),
    ("completed", &[]),
    ("cancelled", &["in_progress"]),
];
```

Constraint: only `cancelled → in_progress` is added. `cancelled → blocked` / `cancelled → pending` / `cancelled → completed` remain disallowed because the issue body specifically frames this as "cancel and retry" (un-cancel back to active work, not a re-plan).

**Test scenarios:**
- **Happy path:** `cancelled` task, call `update_task_status(status="in_progress")` → succeeds, row updates in place (same `task_id`).
- **Disallowed transitions:** `cancelled → blocked`, `cancelled → pending`, `cancelled → completed` → all fail with the existing `is_valid_transition` rejection message listing only `in_progress` as the allowed target.
- **Status remains `cancelled` on rejected transition:** failed transition does NOT mutate the row.
- **Metadata-only writes still work on cancelled tasks:** existing behavior preserved (passing only `metadata`, no `status`, succeeds — terminal-state-metadata path at `update_task_status` ignores status field).

**Verification:** unit tests in `update_task_status.rs::tests` covering each scenario; existing tests continue to pass.

### U2 — Update tool description

**Goal:** The `update_task_status` tool description reflects the new contract.

**Files:**
- Modify: `crates/mika-agent/src/tools/update_task_status.rs:60-68` (the `description` field of `ToolDefinition`)

**Approach:** Change the existing text:

```
Transitions are validated: pending can go to any status; in_progress can go to
blocked/completed/cancelled; blocked can go to in_progress/completed/cancelled.
Completed and cancelled are terminal — status cannot be changed, but metadata
can still be attached by passing the metadata field (the status field is ignored
in that case and the call succeeds).
```

to:

```
Transitions are validated: pending can go to any status; in_progress can go to
blocked/completed/cancelled; blocked can go to in_progress/completed/cancelled.
Cancelled can return to in_progress (cancel-and-retry — reuses the same task
row, mika#856). Completed is terminal. For terminal completed tasks, status
cannot be changed, but metadata can still be attached by passing the metadata
field (the status field is ignored in that case and the call succeeds).
```

Both the JSON-schema enum (`["pending", "in_progress", "blocked", "completed", "cancelled"]`) and the description's behavior section are aligned. The metadata-on-terminal carve-out narrows to `completed` only (it was correctly redundant for cancelled, since cancelled tasks can now transition out — but metadata writes on cancelled before the agent decides to revert still work via the same path).

**Test scenarios:** N/A — pure string change. Manual review confirms accuracy. The test that asserts the `enum` shape of the schema remains unchanged.

### U3 — Update root `CLAUDE.md` and crate `CLAUDE.md`

**Goal:** Documentation matches the code contract.

**Files:**
- Modify: `crates/mika-agent/CLAUDE.md` § Task Tracking → "Status transition state machine" line
- Modify: root `CLAUDE.md` if it carries the same statement

**Approach:** Change the `crates/mika-agent/CLAUDE.md` line:

> **Status transition state machine:** `pending` -> any; `in_progress` -> blocked/completed/cancelled; `blocked` -> in_progress/completed/cancelled. Terminal states (`completed`, `cancelled`) cannot transition to a new status, but metadata can still be written (#617) — the tool applies metadata and returns success without changing status.

to:

> **Status transition state machine:** `pending` -> any; `in_progress` -> blocked/completed/cancelled; `blocked` -> in_progress/completed/cancelled; `cancelled` -> in_progress (cancel-and-retry path, mika#856). `completed` is terminal — status cannot transition, but metadata can still be written (#617). Cancelled tasks can be reverted to in_progress, reusing the original row instead of creating a new task; while cancelled, metadata writes (e.g. via `cancelled_reason`) continue to work.

**Verification:** manual read. Grep for any other instance of `"Completed and cancelled are terminal"` in the repo:

```bash
grep -rn "Completed and cancelled are terminal\|cancelled.*terminal" crates/ docs/ CLAUDE.md
```

Update each hit consistently. Expected hits: this CLAUDE.md, the tool description (U2), and possibly KG-extracted prose in `docs/solutions/`. KG docs are historical and should not be edited; only the active CLAUDE.md and tool description should change.

### U4 — Verify `validate_task()` and active-status callers

**Goal:** Confirm no downstream code path needs adjustment.

**Files:**
- Read-only verification: `crates/mika-agent/src/tools/mod.rs:387-414` (`validate_task`)
- Read-only verification: `validate_dispatch_readiness()` in `crates/mika-agent/src/skills/executor.rs` (checks `pending|in_progress` for dispatch eligibility)

**Approach:** No code change — this unit is a verification step documented as a checklist:

- ✅ `validate_task()` checks current status (`pending|in_progress|blocked`); a reverted task is `in_progress`, so it passes the check naturally.
- ✅ `validate_dispatch_readiness()` (check 1) checks `pending|in_progress`; a reverted task is `in_progress`, so dispatch resumes naturally on the re-activated task.
- ✅ `cancel_task()` cascade behavior: when a parent is cancelled and the agent later reverts it, only the parent row reverts — callback children remain cancelled (their cascade was already correct at cancel time). The agent must re-dispatch to spawn new callbacks. Confirmed via reading `cancel_task` and the cascade-to-children logic.
- ✅ `audit_events` row is written by the existing `update_task_status` audit-logging path on every transition — the revert produces an audit row with `from='cancelled' to='in_progress'`. No new audit code needed.

**Test scenarios:**
- **Revert + re-dispatch:** cancelled task reverts to in_progress, agent calls `run_claude_pilot` again; the dispatch-readiness guard passes; a new callback child is spawned (the old cancelled children are not resurrected).
- **Audit trail visible:** `list_audit_events` or DB query on `audit_events` shows the cancellation, then the revert, in order.

**Verification:** integration smoke test post-implementation; existing eval-harness tests covering the cancel/revert path (if any).

### U5 — Documentation sweep

**Goal:** No "cancelled is terminal" statement remains in active documentation.

**Files:**
- Grep target: `crates/`, root `CLAUDE.md`, `docs/`

**Approach:** Run the grep above; update every active-source hit. KG docs (already-extracted historical solutions) are not edited.

**Verification:** post-implementation `grep -rn "cancelled.*terminal\|Cancelled.*terminal" crates/ CLAUDE.md` returns no hits in source code (only in docs/solutions/ as historical record).

## Dependencies / sequencing

- U1 → U2 → U3 are independent line-edits; can ship in any order within the same PR
- U4 is a verification step that gates the PR; if any downstream caller is found to need adjustment, that becomes U6
- U5 is the final sweep; can be done last

## Patterns to follow (cross-cutting)

- `crates/mika-agent/src/tools/update_task_status.rs:29-44` — existing `is_valid_transition` / `allowed_transitions` helpers, unchanged
- `crates/mika-agent/src/tools/mod.rs:387-414` — `validate_task` semantics, unchanged
- Audit logging via existing `audit_events` path on every transition (no new code)

## Verification (top-level)

- `cargo test -p mika-agent tools::update_task_status::tests` — existing tests pass + new tests for the U1 scenarios
- `cargo clippy --workspace` clean
- `cargo fmt --all -- --check` clean
- `grep -rn "Completed and cancelled are terminal" crates/ CLAUDE.md` returns 0 hits in source code
- Manual smoke: cancel a task, revert it via `update_task_status(status="in_progress")`, dispatch resumes on the same row

## Risk / known unknowns

- **A2A protocol asymmetry surface.** A2A clients (cross-agent tracking) cannot un-cancel; internal tasks can. If an A2A client ever observes a previously-cancelled task that is now active, it must reconcile via state queries — the same way it handles task status drift today. This is documented in the plan; a future A2A spec change would address it formally.
- **`cancelled_reason` metadata after revert.** The plan recommends leaving it in place (informational record of "this task was once cancelled"). If the operator wants it cleared, that's a separate UX call and a trivial follow-up.
- **Dashboard / dev-runs display.** If the dashboard renders "Cancelled at <time>" using `updated_at`-when-status-was-cancelled, post-revert the column will show the revert time. The dashboard query should source the cancel timestamp from `audit_events` (which preserves the cancel time as a separate event) — not from `updated_at`. The plan does NOT touch dashboard code (out of scope; existing dashboard behavior continues to work even if imprecise on cancel-then-revert tasks, which were impossible before this change).

## Out-of-scope (explicit)

- A2A `TaskStateMachine` change (separate concern; A2A protocol stays asymmetric).
- Migration to introduce a separate `cancelled_at` column (no column exists today; YAGNI).
- Formal transition log table (`task_status_history`). Audit events already cover this; a structured history table is overkill for a p3 UX cleanup.
- Dashboard UI for the cancel-then-revert visualization (separate concern; dashboard reads from audit_events if it wants the precise cancel timestamp).
- `cancelled → blocked|pending|completed` transitions (issue body specifically frames this as "cancel and retry"; un-cancel back to in_progress is the only motivated path).
