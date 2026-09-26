---
ticket: mika#1436
branch: fix/1436/orphan-recurring-tasks-startup-removal
status: active
date: 2026-06-13
origin: https://github.com/senara-solutions/mika/issues/1436
execution: code
---

# Plan: orphan recurring task cleanup at startup (mika#1436)

## Problem frame

When an agent dir is deleted from `~/.mika/agents/` (e.g., mika-relay per mika#1188 Phase C), the agent's `recurring_tasks` rows (stored in the `tasks` table with `trigger_type='recurring'`) remain in `mika.db`. The startup code at `crates/mika-agent/src/server/mod.rs:1272-1320` only iterates `state.agents` to ADD missing recurring tasks; it has no companion code to REMOVE orphan tasks for agents no longer on disk.

Behavioral impact is cheap — orphan rows are inert (no `AgentState` means no task engine to dispatch them on the configured cron). But they create graveyard noise in `list_scheduled_tasks` and confuse audit tooling.

## Resolution of first-pass findings

**F1 (BLOCKING) — Commit to direction + add acceptance criteria.**

Plan commits to **Option 1: startup-time `expire_orphan_recurring_tasks(known_agents)`** — a single SQL operation at a well-defined lifecycle point, before per-agent recurring-task setup. Rationale:
- Simple: one query, one pass, runs once per startup.
- Complete: catches all orphans regardless of how the agent was removed (CLI delete, manual `rm -rf`, container image rebuild).
- Safe: runs before task engines spawn, no race with dispatch.

Options 2 and 4 from the body require agent-deletion infrastructure that doesn't exist (no CLI agent-delete command). Option 3 (status quo) is defensible but the fix is cheap enough.

**Acceptance criteria:**
- AC1: After agent dir removal + server restart, recurring tasks for the removed agent transition to status `'cancelled'` (preserves audit trail via the existing audit_events row).
- AC2: `list_scheduled_tasks` no longer shows orphan entries for the removed agent.
- AC3: Existing agents' recurring tasks are unaffected (idempotent on subsequent restarts).
- AC4: Operator log line on every orphan-sweep run, listing the count of cancelled tasks and the agent_ids involved (operator observability into when the sweep fires).

**F2 (sharpening) — Explicit scope boundary.**

Plan scopes cleanup to **`tasks` table rows with `trigger_type='recurring'` only** — forward-looking scheduled work for agents that no longer exist. Historical/audit data is explicitly out-of-scope:

| Surface | Treatment | Reason |
|---------|-----------|--------|
| `tasks` (trigger_type='recurring', active status) | **Cleanup target** | Forward-looking — schedules dispatch for a non-existent agent. Inert but noisy. |
| `sessions`, `messages`, `tool_calls`, `llm_calls` | **Out of scope** | Historical record of agent's prior work. Removing this destroys audit trail. |
| `audit_events` | **Out of scope** | Append-only audit log. |
| `kg_*` | **Out of scope** | Per-agent KG state can be cleaned via separate `mika kg purge` CLI (already exists per `CLAUDE.md`). |
| `tasks` (trigger_type='manual'/'callback', terminal status) | **Out of scope** | Historical task records. Removing breaks task-tree introspection. |

A broader agent-retirement sweep would be a separate ticket if/when an agent-delete CLI command lands.

## Sibling interaction with mika#1399

mika#1399 (also groomed this session) migrates `AppState.agents` to `DashMap` with lazy-insert. The orphan sweep here runs at startup, against the initial filesystem-walk set populated before any HTTP requests can trigger lazy-inserts. Sequencing:

1. Server startup → `state.agents` populated from filesystem walk (`mika_common::agent::list_agents()`).
2. **NEW: orphan-sweep runs** — cancels `recurring_tasks` rows for agents not in the populated set.
3. For-loop iterates `state.agents` → `ensure_recurring_task` for known agents.
4. Server accepts HTTP requests; #1399's lazy-insert may add post-boot agents to the map.

A lazy-inserted post-boot agent's recurring tasks are added by `ensure_recurring_task` when they're exercised — the orphan sweep won't have removed them (the agent didn't exist at startup; there's nothing to remove for it). No conflict.

## Scope boundaries

- One new function: `cancel_orphan_recurring_tasks(known_agent_ids: &[String]) -> Result<Vec<String>>` in `crates/mika-agent/src/db.rs`. Returns the list of cancelled task IDs for logging.
- One new call site in `server/mod.rs` between filesystem walk and the existing `state.agents.iter()` for-loop.
- Operator log line.
- **Out of scope:** broader audit-data cleanup; agent-delete CLI; KG cleanup (already covered by `mika kg purge`).

## Implementation Units

### U1 — DB function: `cancel_orphan_recurring_tasks`

**Goal:** Database method that cancels recurring tasks belonging to agents not in the provided set.

**Files:**
- Modify: `crates/mika-agent/src/db.rs` (alongside existing recurring-task helpers around line 1608)

**Approach:**

```rust
/// Cancels active recurring tasks belonging to agents no longer on disk (mika#1436).
///
/// Called once at startup after the filesystem walk populates the agent set.
/// Cancels (not deletes) so the audit trail via audit_events is preserved.
/// Returns the list of cancelled task IDs for operator observability.
pub async fn cancel_orphan_recurring_tasks(
    &self,
    known_agent_ids: &[String],
) -> Result<Vec<String>> {
    // SELECT first to return IDs for logging
    // UPDATE second to transition to 'cancelled'
    // Both in the same transaction
}
```

Active statuses: `pending`, `recurring_active`, `in_progress` (mirrors the `status NOT IN ('cancelled','failed','expired','delivered')` predicate used elsewhere in db.rs).

SQL:
```sql
UPDATE tasks
SET status = 'cancelled', updated_at = ?
WHERE trigger_type = 'recurring'
  AND status IN ('pending', 'recurring_active', 'in_progress')
  AND agent_id NOT IN (rarray-of-known-agent-ids);
```

For the `NOT IN` over a list of known agent IDs, use `rusqlite::vtab::array::Array` (rarray extension) or a string-joined parameterized query. Existing db.rs patterns at lines 1608/1915/2164 will guide the implementer.

**Test scenarios:**
- **Happy path — one orphan:** seed `tasks` table with two recurring tasks (agent_a + agent_b); call with known_agent_ids = ["agent_a"] → agent_b's task cancelled, agent_a's unchanged.
- **No orphans:** all known → no changes.
- **Multiple orphans for one agent:** mika-relay had `heartbeat` + `reflection` + `auto_pull_groomed` recurring tasks → all three cancelled.
- **Already-cancelled task:** orphan task with status `cancelled` → no change (idempotent, NOT IN active-status set means it's already terminal).
- **Empty known_agent_ids:** all active recurring tasks cancelled. Safety: should this case panic instead, in case `known_agent_ids` was empty due to a startup discovery bug? Implementer-choice: a warn-and-skip if `known_agent_ids.is_empty()` is the safer default.

**Verification:** unit tests in `db.rs::tests`; existing tests pass.

### U2 — Wire the orphan sweep into startup

**Goal:** `server::mod.rs` calls the new function between filesystem walk and recurring-task setup.

**Files:**
- Modify: `crates/mika-agent/src/server/mod.rs` (insert before line 1272's `for (name, agent_state) in state.agents.iter()`)

**Approach:**

```rust
// Cancel orphan recurring tasks for agents no longer on disk (mika#1436).
// Runs against the startup-time agent set; #1399's lazy-insert path runs later
// and adds tasks per ensure_recurring_task when the agent is first exercised.
let known_agent_ids: Vec<String> = state.agents.iter().map(|(k, _)| k.clone()).collect();
if !known_agent_ids.is_empty() {
    let dashboard_db = state.dashboard_db.clone();
    match dashboard_db.cancel_orphan_recurring_tasks(&known_agent_ids).await {
        Ok(cancelled_ids) if !cancelled_ids.is_empty() => {
            info!(
                count = cancelled_ids.len(),
                task_ids = ?cancelled_ids,
                "cancelled orphan recurring tasks for agents no longer on disk"
            );
        }
        Ok(_) => {}  // no orphans, no log noise
        Err(e) => warn!(error = %e, "orphan recurring task sweep failed"),
    }
} else {
    warn!("startup-time agent set is empty; skipping orphan recurring task sweep as safety measure");
}
```

The fail-open semantics (warn and continue) match existing dispatch-readiness fail-open patterns. The empty-set safety check protects against the edge case where filesystem discovery returns zero agents.

**Test scenarios:**
- **Smoke test:** create agent foo, restart server, verify foo's recurring tasks exist. Delete foo's dir, restart server, verify orphan sweep cancels foo's recurring tasks (visible in `list_scheduled_tasks`).
- **No-op on healthy startup:** all agents present → no orphans logged.
- **Empty known set:** safety log fires; no DB mutation.

**Verification:** integration smoke test in a test harness if available; manual smoke on a real mika-spirit.

### U3 — Docs note

**Goal:** Document the startup-sweep behavior.

**Files:**
- Modify: `crates/mika-agent/CLAUDE.md` § Unified Task Engine — add a note about the orphan sweep

**Approach:** One-line addition:

> **Orphan recurring task sweep (mika#1436):** At server startup, after the filesystem walk populates `state.agents`, the engine cancels any active recurring tasks (`trigger_type='recurring'`, status in `pending|recurring_active|in_progress`) whose `agent_id` is not in the on-disk set. Cancellation preserves the audit trail (vs deletion). Companion to #1399's lazy-insert: lazy-resolved post-boot agents add their recurring tasks via `ensure_recurring_task` on demand.

**Verification:** manual read.

## Dependencies / sequencing

- U1 → U2 (U2 calls the new function)
- U3 ships in same PR; last

## Patterns to follow (cross-cutting)

- `crates/mika-agent/src/db.rs:1608-1620` — existing recurring-task SQL pattern
- `crates/mika-agent/src/server/mod.rs:1272` — existing `state.agents.iter()` loop for per-agent setup
- Fail-open warn-and-continue pattern matches dispatch-readiness fail-open semantics elsewhere in the codebase

## Verification (top-level)

- `cargo test -p mika-agent db::tests` passes
- `cargo clippy --workspace` clean
- Manual smoke: delete an agent's dir, restart mika-spirit, confirm `list_scheduled_tasks` no longer shows orphan entries; log line visible

## Risk / known unknowns

- **Empty `known_agent_ids` safety.** If filesystem discovery has a bug and returns zero agents, calling the sweep with an empty list would cancel ALL active recurring tasks. The U2 empty-set guard rejects this case with a warn.
- **Race with concurrent agent creation.** None — the sweep runs before HTTP requests are accepted, so no other code path is creating/exercising agents at this moment.
- **Composition with #1399 (lazy-insert).** Documented in §Sibling interaction. No ordering conflict.

## Out-of-scope (explicit)

- Cleanup of historical audit data (`sessions`, `messages`, `tool_calls`, `llm_calls`, `audit_events`) — these are audit records, not orphans.
- KG cleanup — already covered by `mika kg purge --agent <name>`.
- Manual non-recurring task cleanup — `trigger_type='manual'` tasks are user-facing records.
- An agent-delete CLI command — a separate ticket; this fix is the cheapest interim measure.
