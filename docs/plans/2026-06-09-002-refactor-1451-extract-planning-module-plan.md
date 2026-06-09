# Plan — refactor(mika#1259): extract planning/ into its own module dir

**Ticket:** mika#1451
**Parent:** mika#1259 (Layer 3 domain refactor)
**Operational responsibility:** Plan-doc invariants, dispatch-readiness predicates, agent-loop policy
**Foundation ref:** `docs/architecture/operational-partner-frame.md` §6 — cross-cutting reads; no exclusive writes

## Scope

Create `crates/mika-agent/src/planning/mod.rs` and relocate all dispatch-readiness logic, plan-doc invariant checks, webhook dispatch predicates, and dispatch-policy INTENT_GUARDS from their current locations. Pure module split — no behavior change.

### mika#1363 status check

mika#1363 (auto-pull groomed-not-ready) is **OPEN and unimplemented**. No `is_groomed()` or `auto_pull.rs` exists in the codebase. The coupling note from the decomposition plan (F2) does not apply — if #1363 ships later, its predicate lands directly in `planning/`.

## What moves

### 1. From `crates/mika-agent/src/skills/executor.rs` (~620 lines)

| Function | Lines | Purpose |
|---|---|---|
| `derive_dispatch_class()` | 772–777 | Maps skill name → `"implement"` / `"groom"` dispatch class |
| `extract_skill_from_input()` | 780–782 | Extracts skill name from tool input JSON |
| `check_grooming_markers()` | 803–817 | Plan-doc invariant: checks issue body for 3 canonical grooming signals |
| `record_dispatch_rejection()` | 824–832 | Fire-and-forget write of rejection reason to `tasks.result` |
| `validate_dispatch_readiness()` | 843–1317 | Seven-check dispatch-readiness gate (the primary function) |
| `check_task_has_open_pr()` | 1357–1449 | Open PR re-dispatch prevention |
| `build_open_pr_rejection()` | 1455+ | Builds structured rejection JSON for open-PR case |
| `extract_pr_url()` | 1755+ | Extracts PR URL from task metadata JSON |

**Tests that move with the functions** (from `executor.rs` `#[cfg(test)]` block):
- `test_derive_dispatch_class_values`
- `test_validate_dispatch_readiness_*` family
- `test_check_grooming_markers_*` family (if present)
- `test_extract_pr_url_*` family (if present)

### 2. From `crates/mika-agent/src/webhook_dispatch.rs` (entire module, 184 lines)

| Item | Lines | Purpose |
|---|---|---|
| `READY_LABEL_DISPATCH_MARKER` re-export | 11 | Re-export from mika-common |
| `is_unauthorized_webhook_dispatch()` | 30–47 | Webhook domain allowlist predicate |
| `is_ready_label_dispatch_marker()` | 50–52 | Ready-label detection helper |
| `mod tests` | 54–184 | Exhaustive prefix-surface test matrix |

The `webhook_dispatch.rs` file is entirely about dispatch gating — it belongs to `planning/` in its entirety. The module is deleted after absorption; `planning/` re-exports the public items.

### 3. From `crates/mika-agent/src/agent.rs` (~180 lines, dispatch-policy guards only)

| Item | Lines | Purpose |
|---|---|---|
| `IntentPrecondition` struct | 5772–5781 | Type definition for intent-guard entries |
| `INTENT_GUARDS` const array | 5787–5910 | Registry of 6 dispatch-policy intent guards |
| `CALLBACK_TERMINAL_ACTION_LABEL` | 5915 | Shared label constant |
| `CALLBACK_TERMINAL_ACTION_CORRECTION` | 5916–5923 | Shared correction message |
| `ready_label_dispatch_trigger()` | ~5929+ | Trigger fn for ready-label guard |
| `ready_label_dispatch_satisfied()` | ~5940+ | Satisfied fn for ready-label guard |
| `webhook_no_unauthorized_dispatch_trigger()` | ~5979+ | Trigger fn for unauthorized dispatch guard |
| `webhook_no_unauthorized_dispatch_satisfied()` | ~5988+ | Satisfied fn for unauthorized dispatch guard |
| `detect_resume_intent()` | ~6040+ | Trigger fn for resume-reconcile guard |
| `resume_reconcile_satisfied()` | ~6060+ | Satisfied fn for resume-reconcile guard |
| `callback_trigger_active()` | ~6070+ | Trigger fn for callback terminal action |
| `callback_terminal_action_satisfied()` | ~6080+ | Satisfied fn for callback terminal action |
| `deferred_dispatch_trigger()` | ~6090+ | Trigger fn for deferred dispatch |
| `deferred_dispatch_satisfied()` | ~6095+ | Satisfied fn for deferred dispatch |

**What does NOT move from `agent.rs`:**
- Inline guards for fabrication detection (`detect_fabricated_action_claim`, `detect_completion_claim`, `detect_milestone_close_claim_without_patch`, asserted-unavailability guard, assert-grounded guard) — these are `evidence/` territory per Foundation §6
- Callback milestone advance inline guard (#991) and webhook companion guard (#1218) — these are inline (not in `INTENT_GUARDS` array) and tightly coupled to the run_loop evaluation site. They stay in `agent.rs` for now; `agent_loop/` extraction (mika#1259-H) will address them
- The EndTurn chain evaluation code that iterates `INTENT_GUARDS` — stays in `agent.rs`; it imports `INTENT_GUARDS` from `planning/`
- Post-condition guards #3–#9 — these are evidence/grounding concerns, not planning

### 4. DB methods — NO relocation

The following `db.rs` / `async_db` methods are called by `validate_dispatch_readiness()` but are Database trait methods that cannot be extracted without restructuring the DB layer:
- `has_active_callback_tasks_excluding()`
- `has_non_deferred_active_callback_child()`
- `write_task_dispatch_rejection()`
- `update_task_dispatch_class()`
- `get_task()`

These stay in `db.rs`. The `planning/` module calls them through the existing `AsyncDatabase` reference.

## What does NOT move

| Item | Stays in | Reason |
|---|---|---|
| `execute_skill_tool()` + exec/http handlers | `skills/executor.rs` | Tool execution, not planning |
| Long-running dispatch (`execute_long_running`, `spawn_long_running_exec`, `register_deferred_callback`, `check_lineage_cycle`) | `skills/executor.rs` | Tool execution; consumes planning predicates but is not a planning function itself |
| Fabrication/grounding guards in `agent.rs` | `agent.rs` → future `evidence/` | Foundation §6 assigns to evidence/ |
| `IntentPrecondition` evaluation loop in `run_loop` | `agent.rs` → future `agent_loop/` | Evaluation site, not definition site |
| GitHub GraphQL helpers (`fetch_open_blockers`, `fetch_issue_labels`, `parse_phase_label`, etc.) | `github_graphql.rs`, `tools/check_task.rs` | Shared infrastructure; planning/ imports them |
| DB methods for dispatch readiness | `db.rs` | Database layer; planning/ calls them via `AsyncDatabase` |

## New module structure

```
crates/mika-agent/src/planning/
└── mod.rs          # All planning functions in a single file
```

Single-file module (no sub-files). The ~800 lines of relocated code don't warrant sub-module splitting. `mod.rs` gets a one-paragraph doc-comment per AC4:

```rust
//! Plan-doc invariants, dispatch-readiness predicates, and agent-loop policy.
//!
//! This module owns the cross-cutting read predicates that gate dispatch
//! and enforce dispatch policy in the agent loop. It has no exclusive
//! writes to any table — it reads task state, issue bodies, and webhook
//! markers to make go/no-go decisions.
```

## Visibility changes

| Item | Current visibility | New visibility |
|---|---|---|
| `derive_dispatch_class` | `pub(crate)` in `skills::executor` | `pub(crate)` in `planning` |
| `extract_skill_from_input` | `fn` (private) in `skills::executor` | `pub(crate)` in `planning` (needed by `executor.rs` for long-running dispatch) |
| `check_grooming_markers` | `pub` in `skills::executor` | `pub` in `planning` (used by bundled skill tests) |
| `record_dispatch_rejection` | `async fn` (private) in `skills::executor` | `pub(crate)` in `planning` (needed by `executor.rs` for deferred-dispatch rejection) |
| `validate_dispatch_readiness` | `async fn` (private) in `skills::executor` | `pub(crate)` in `planning` |
| `check_task_has_open_pr` | `async fn` (private) | `pub(crate)` in `planning` |
| `build_open_pr_rejection` | `fn` (private) | `pub(crate)` in `planning` |
| `extract_pr_url` | `fn` (private) | `pub(crate)` in `planning` |
| `IntentPrecondition` | `struct` (private) in `agent` | `pub(crate)` in `planning` |
| `INTENT_GUARDS` | `const` (private) in `agent` | `pub(crate)` in `planning` |
| Trigger/satisfied fns | `fn` (private) in `agent` | `pub(crate)` in `planning` |
| `is_unauthorized_webhook_dispatch` | `pub(crate)` in `webhook_dispatch` | `pub(crate)` in `planning` |
| `is_ready_label_dispatch_marker` | `pub(crate)` in `webhook_dispatch` | `pub(crate)` in `planning` |
| `READY_LABEL_DISPATCH_MARKER` | `pub(crate) use` in `webhook_dispatch` | `pub(crate) use` in `planning` |

## Import updates

### `skills/executor.rs`
- Remove: `derive_dispatch_class`, `extract_skill_from_input`, `check_grooming_markers`, `record_dispatch_rejection`, `validate_dispatch_readiness`, `check_task_has_open_pr`, `build_open_pr_rejection`, `extract_pr_url` function definitions
- Add: `use crate::planning::{validate_dispatch_readiness, derive_dispatch_class, extract_skill_from_input, record_dispatch_rejection, extract_pr_url};`
- The long-running dispatch code (`execute_long_running`) calls `validate_dispatch_readiness` and `derive_dispatch_class` — these become cross-module imports

### `agent.rs`
- Remove: `IntentPrecondition`, `INTENT_GUARDS`, all trigger/satisfied functions, `CALLBACK_TERMINAL_ACTION_LABEL`, `CALLBACK_TERMINAL_ACTION_CORRECTION`, `use crate::webhook_dispatch::{...}`
- Add: `use crate::planning::{IntentPrecondition, INTENT_GUARDS, CALLBACK_TERMINAL_ACTION_LABEL, CALLBACK_TERMINAL_ACTION_CORRECTION, is_unauthorized_webhook_dispatch, READY_LABEL_DISPATCH_MARKER};`
- The EndTurn chain at ~line 1610 (`for guard in INTENT_GUARDS`) continues to work via the import

### `lib.rs`
- Add: `pub(crate) mod planning;`
- Remove: `pub(crate) mod webhook_dispatch;`

### Other callers
- Grep for any other `use crate::webhook_dispatch::` — update to `use crate::planning::`
- `agent.rs` line 5927 uses `crate::webhook_dispatch::{READY_LABEL_DISPATCH_MARKER, is_unauthorized_webhook_dispatch}` — update to `crate::planning::`

## Implementation steps

1. **Create `crates/mika-agent/src/planning/mod.rs`** with doc-comment and required imports (`use crate::async_db::AsyncDatabase`, `use crate::db`, `use crate::github_graphql::*`, `use crate::tools::check_task::{GitHubRef, parse_github_ref}`, `use tracing::warn`, `use mika_common::github_event_format::READY_LABEL_DISPATCH_MARKER`, etc.)

2. **Move webhook dispatch predicates** from `webhook_dispatch.rs` into `planning/mod.rs`. Delete `webhook_dispatch.rs`.

3. **Move dispatch-readiness functions** from `skills/executor.rs` into `planning/mod.rs`. Leave stub imports in `executor.rs`.

4. **Move INTENT_GUARDS** from `agent.rs` into `planning/mod.rs`. Leave stub imports in `agent.rs`.

5. **Update `lib.rs`**: add `pub(crate) mod planning;`, remove `pub(crate) mod webhook_dispatch;`

6. **Update all import sites** in `agent.rs`, `skills/executor.rs`, and any other files that reference `webhook_dispatch`.

7. **Move tests**: relocate unit tests for moved functions into `planning/mod.rs`'s `#[cfg(test)] mod tests` block.

8. **Verify**: `cargo test -p mika-agent`, `cargo clippy -p mika-agent --tests --no-deps -- -D warnings`

## Risks

- **Low:** The functions being moved have clear boundaries (input → output, no mutable shared state). Cross-module imports are straightforward.
- **Medium:** `validate_dispatch_readiness` is 475 lines with many imports from `db`, `github_graphql`, `tools/check_task`. Need to ensure all type imports resolve correctly from the new module location.
- **Low:** The `INTENT_GUARDS` array uses function pointers — these work across modules without issue as long as the functions are accessible.

## Acceptance criteria verification

- [x] AC1: `planning/mod.rs` created — step 1
- [x] AC2: Logic moved from `agent.rs` + `skills/executor.rs` + `webhook_dispatch.rs` — steps 2–4
- [x] AC3: `lib.rs` declares `pub(crate) mod planning` — step 5
- [x] AC4: One-paragraph doc-comment naming operational responsibility — step 1
- [x] AC5: `cargo test -p mika-agent` passes — step 8
- [x] AC6: `cargo clippy` clean — step 8
- [x] AC7: No behavior change — pure relocation, verified by existing test suite

## Out of scope

- Moving fabrication/grounding guards (evidence/ territory, mika#1259-A)
- Moving long-running dispatch execution (tool_execution/ territory, mika#1259-B)
- Moving the EndTurn evaluation loop (agent_loop/ territory, mika#1259-H)
- Creating sub-files within planning/ (single-file module suffices at ~800 lines)
- New tests (existing coverage is the regression gate per parent AC3)
