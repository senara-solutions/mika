# Plan: Fix long-running tools blocked from callback turns (mika#803)

**Ticket:** senara-solutions/mika#803
**Branch:** `fix/803/engine-long-running-tools-callback-turns`
**Type:** bug, p1-important, agent-core
**Authored:** 2026-04-27
**Architect review:** mika-arch session `05d1f78d-88b4-4dc2-8eb5-d77bc757e7aa` — Disposition: ITERATE on first pass; revisions below address all 9 findings.

## Problem

`run_claude_pilot` (and any tool with `long_running: true`) is rejected by the engine when called from a callback turn. The rejection site is `crates/mika-agent/src/skills/executor.rs:148-156`:

```
Tool '<name>' is declared long_running but cannot run in the current context
(callback turn, silent mode, or CLI test). Long-running tools require a
conversation-mode turn with an active task engine.
```

This contradicts the documented retry path in `mika/skills/bundled/self-dev/system_prompt.md:130`:

> 4. If retries remain: notify Vincent... Then call `run_claude_pilot` with the same `repo#number` and `task_id` (handler reuses existing worktree). Wait for callback and re-enter this entry point.

`create_task` is also blocked from callback turns (`crates/mika-agent/src/tools/create_task.rs:143`), so the agent cannot defer the retry via task creation either. Result: any pipeline-failure-class callback that warrants retry must escalate to Vincent for manual CLI dispatch. The autonomous dev loop is mechanically incomplete.

Production evidence: mika#798 on 2026-04-25 — `error_max_turns` after 5 commits, no PR; mika-dev attempted documented retry path at 10:57:23Z, blocked by the constraint, surfaced to Vincent ~30 min later.

## Decision

**Adopt Option B from the ticket: defer retry via a new task lifecycle state, scanned by the existing heartbeat loop.**

Why B over A (lift the long-running gate for callback turns):
- The long-running guard exists because callback turns share an engine context with the parent task; running another long-running tool inside that context would nest task lifecycles and break the invariant that `run_claude_pilot` owns a top-level task slot.
- Lifting the guard with a "same task_id" carve-out adds a special case to a constraint whose generality is the safety property. The generality matters more than the carve-out costs.
- B reuses the heartbeat loop that already exists (`SilentTrigger::Heartbeat`, `agent.rs:2194`) — no new background machinery.
- B keeps the surface tiny: one new tool, one new task status, one heartbeat handler arm, one skill prompt edit, one v28→v29 schema bump.

## Architecture

### New tool: `schedule_retry`

A non-long-running tool callable from callback turns. It writes retry intent and args to the parent task's metadata and transitions its status to `retry_pending`. The heartbeat tick scans for `retry_pending` tasks and re-dispatches them in a fresh conversation-mode silent turn.

**Tool signature** (`crates/mika-agent/src/tools/schedule_retry.rs`):
```
schedule_retry {
    task_id: String,                  // parent task whose run we want to retry
    tool_name: String,                // long-running tool to re-invoke (e.g. "run_claude_pilot")
    tool_args: serde_json::Value,     // args to pass on re-dispatch
    reason: String,                   // human-readable trigger ("error_max_turns", ...)
    delay_seconds: Option<u64>        // optional; default 0 (next heartbeat tick)
}
```

**Effect (transactional):**
1. Validate `task_id` exists, belongs to current agent, and current status is `in_progress`. Reject otherwise.
2. Read `pipeline_retry_count` from task `metadata` JSON (default 0).
3. **Cap-exhausted path:** if `pipeline_retry_count + 1 > max_retries` (cap = 2, current `max_retries`):
   - Flip task status to `failed` with reason in metadata.
   - Send notification to operator: "Retry cap exhausted on `<task_id>`: `<reason>`. Manual intervention required."
   - Return `{ scheduled: false, reason: "cap_exhausted", retry_count: <n>, max: <m> }`.
4. **Happy path:** increment `pipeline_retry_count`, write `retry_tool_name`, `retry_tool_args`, `retry_reason`, `retry_scheduled_at = now + delay_seconds` into metadata, flip status to `retry_pending`.
5. Return `{ scheduled: true, retry_count: <n>, max: <m>, scheduled_at: <iso8601> }`.

All status mutations and metadata writes happen inside a single `rusqlite::Transaction` (DEFERRED, matching the pattern in `replace_with_summary` per #636) so the agent never observes a partial state.

**Why a dedicated tool, not a `update_task_status` extension:**
- `update_task_status` is broadly available; adding "retry" semantics there couples policy (retry-count cap) to a generic state setter, and a prompt-injected `update_task_status {status: "retry_pending"}` would bypass the cap entirely.
- A dedicated tool keeps the retry invariants (cap, agent ownership, state machine) localized and discoverable in tool listing.
- **Defensive constraint (per architect Finding 2):** `update_task_status` is updated to explicitly reject `retry_pending` as a target status with a structured error pointing the agent at `schedule_retry`. `schedule_retry` is the **only** write path to `TaskStatus::RetryPending`.

### New task status: `retry_pending`

Add to the `status` CHECK constraint in `crates/mika-agent/src/db.rs:1125-1128`. Schema migration v28 → v29 (additive enum value; no data migration required).

**State machine (per architect Finding 8):**

| From | Valid To | Mechanism |
|------|----------|-----------|
| `in_progress` | `retry_pending` | `schedule_retry` (within cap) |
| `in_progress` | `failed` | `schedule_retry` (cap exhausted) |
| `retry_pending` | `in_progress` | Heartbeat handler (re-dispatch) |
| `retry_pending` | `cancelled` | Operator cancel (existing path) |
| `retry_pending` | `failed` | Defensive: heartbeat finds task at cap when scanning |

`update_task_status` rejects `retry_pending` as a target; only `schedule_retry` writes it. Terminal-state guards (`completed`, `cancelled`) remain unchanged — neither can transition into `retry_pending`.

### Heartbeat handler arm

The unified task engine already runs a 1-second tick loop with a periodic DB scan every 60 ticks (per `task_engine/` and the architecture docs). The scan gains a new pre-trigger step:

```sql
SELECT id, agent_id FROM tasks
WHERE status = 'retry_pending'
  AND agent_id = :self
  AND (json_extract(metadata, '$.retry_scheduled_at') IS NULL
       OR json_extract(metadata, '$.retry_scheduled_at') <= :now)
ORDER BY json_extract(metadata, '$.retry_scheduled_at') ASC
LIMIT 1;
```

**Per architect Finding 3:** at most ONE `retry_pending` task is dispatched per tick, ordered by `retry_scheduled_at ASC` (FIFO). If multiple tasks are due, additional ones wait for subsequent ticks. This prevents floods after long outages.

For the matched task, atomically flip status to `in_progress` using a `BEGIN IMMEDIATE` transaction wrapping the SELECT + UPDATE pair (rusqlite + SQLite ≥ 3.35 supports `RETURNING`):

```sql
BEGIN IMMEDIATE;
UPDATE tasks
   SET status = 'in_progress', updated_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now')
 WHERE id = :id AND status = 'retry_pending'
 RETURNING metadata;
COMMIT;
```

If the UPDATE returns zero rows (lost the race to a concurrent worker), skip and try next tick. If it returns one row, parse `retry_tool_name` + `retry_tool_args` from metadata and emit `SilentTrigger::RetryPending { task_id, tool_name, tool_args }`. The conversation-mode silent turn handles dispatch through the normal long-running execution path. **The long-running guard in `executor.rs` stays untouched** — the actual `run_claude_pilot` call happens in a conversation-mode turn that owns a top-level task slot, just like the original dispatch.

**Latency bound (per architect Finding 6):** with a 1s tick and 60-tick DB scan, max retry latency is ~60s + `delay_seconds`. Acceptable for autonomous-loop "transient pipeline failure" recovery. Documented in skill prompt and plan.

### Defensive integration with existing guards

- **Phantom retry guard (#579)** (`update_task_status`): already rejects retry-semantic metadata writes when an active callback child exists. `schedule_retry` runs after the callback child has completed (we're in the callback turn handling its result), so this guard does not fire. Verify in implementation.
- **Dispatch-readiness guard (#525)** (5 checks in `validate_dispatch_readiness`): the heartbeat-emitted retry runs through this guard normally. Per-turn dispatch counter resets per silent turn, so the cap of 1 dispatch/turn is honored. The task is in `in_progress` (just flipped), so check 1 passes. No active callback child should exist at this point (the prior one closed before `schedule_retry` was called) so check 2 passes.
- **Blocked-by GitHub check (#713)**: re-runs on each retry. If a blocking issue was added between attempts, retry is correctly blocked.

### Skill prompt update

`mika/skills/bundled/self-dev/system_prompt.md:130` retry path becomes (per architect Finding 9, with explicit negative constraint):

> 4. If retries remain: notify Vincent. Then call `schedule_retry { task_id: <current_task_id>, tool_name: "run_claude_pilot", tool_args: <same args as original dispatch>, reason: "<error class>" }`. The heartbeat will re-dispatch within ~60 seconds in a fresh turn.
> **Do NOT call `run_claude_pilot` directly from this callback turn — the engine will reject it. Use `schedule_retry` instead.**
> If `schedule_retry` returns `scheduled: false, reason: "cap_exhausted"`, send_message to Vincent naming the task and reason; the engine has already flipped the task to `failed`.

## Files

### New
- `crates/mika-agent/src/tools/schedule_retry.rs` — new tool implementation + inline tests.

### Modified
- `crates/mika-agent/src/db.rs` (around line 1125) — add `'retry_pending'` to the status CHECK constraint. Add v28→v29 migration entry.
- `crates/mika-agent/src/task_engine/types.rs` — add `RETRY_PENDING` constant alongside existing status string constants. Confirm any rust-side `TaskStatus` enum (if it exists) gets the new variant.
- `crates/mika-agent/src/agent.rs` — add `SilentTrigger::RetryPending { task_id, tool_name, tool_args }` variant; add heartbeat scan + dispatch handler.
- `crates/mika-agent/src/task_engine/engine.rs` — wire the periodic-scan retry pickup (this is where the 60-tick periodic scan lives per architecture docs).
- `crates/mika-agent/src/tools/mod.rs` — register `schedule_retry`.
- `crates/mika-agent/src/tools/update_task_status.rs` — reject `'retry_pending'` as target status with structured `{"error": "retry_pending_requires_schedule_retry", ...}` pointing at `schedule_retry`.
- `crates/mika-agent/src/skills/executor.rs` — **no functional change** to the long-running guard. (This is the **point** of Option B.)
- `mika/skills/bundled/self-dev/system_prompt.md` — update retry path on line ~130 with the new tool and negative constraint.
- `crates/mika-agent/CLAUDE.md` — document `schedule_retry` tool, `retry_pending` status, the v28→v29 migration entry, and the SilentTrigger::RetryPending variant in the appropriate sections (Tools, Schema Version, Silent Mode Agent Loop, Unified Task Engine).
- `crates/mika-agent/docs/architecture.md` — task lifecycle diagram update.

### Tests

`crates/mika-agent/src/tools/schedule_retry.rs` inline:
- Happy path: callback turn calls `schedule_retry`, task transitions to `retry_pending`, retry_count increments, metadata fields written.
- Cap enforcement: at `max_retries`, call returns `{scheduled: false, reason: "cap_exhausted"}` and task is flipped to `failed`. Notification sent (if message_sender available).
- Wrong agent: rejects with structured error.
- Bad task_id: rejects with `task_not_found`.
- Wrong status (e.g. `pending`, `completed`): rejects.
- Atomicity: simulated mid-transaction failure leaves task in original state (no partial metadata write).

`crates/mika-agent/src/task_engine/engine.rs` (or wherever heartbeat scan is tested):
- Heartbeat scan picks up oldest `retry_pending` task and emits exactly one `SilentTrigger::RetryPending` per tick even when multiple are due.
- Atomic flip: two concurrent ticks see `retry_pending`, only one wins the UPDATE (the other gets zero rows, skips).
- Re-dispatch transitions task `retry_pending → in_progress`.
- `delay_seconds` honored: task with future `retry_scheduled_at` not picked up.

`crates/mika-agent/src/tools/update_task_status.rs`:
- Rejects `retry_pending` target with the new structured error.

E2E (manual or scripted): mika-dev callback turn → `schedule_retry` → next heartbeat → `run_claude_pilot` re-dispatch → second callback completes the task. Reproduce mika#798 scenario by forcing low max-turns config.

## Migration / rollout

- **DB migration v28 → v29**: forward-only ALTER on the `status` CHECK constraint. SQLite doesn't support `ALTER TABLE ... DROP CONSTRAINT`, so this is a copy-table-and-rename migration (matches the pattern of prior schema changes). No data backfill — existing rows keep their current statuses.
- Skill prompt change ships in `mika/skills/bundled/self-dev/system_prompt.md`; takes effect on next agent skills update (`mika skills --agent mika-dev update`) — covered by `make deploy`.
- No env-var feature flag. The new state, tool, and trigger are additive; existing callbacks without `schedule_retry` calls behave exactly as today.

## Verification

- [ ] `cargo build` and `cargo test -p mika-agent` — all green.
- [ ] `cargo clippy --all-targets` — no new warnings.
- [ ] Schema migration applies cleanly on a fresh DB and on an existing v28 DB:
    - `mika-server` startup logs `Migrating database v28 → v29`.
    - `SELECT sql FROM sqlite_master WHERE name = 'tasks'` shows `retry_pending` in the CHECK.
- [ ] Manual repro of mika#798 scenario:
    1. Force a `run_claude_pilot` task to hit `error_max_turns` mid-run (low max-turns config).
    2. Confirm the callback turn calls `schedule_retry` per the updated skill prompt.
    3. Confirm heartbeat tick picks it up within ~60s and re-dispatches.
    4. Confirm second run completes (or hits cap and surfaces to Vincent).
- [ ] `SELECT status, json_extract(metadata, '$.pipeline_retry_count') FROM tasks WHERE id = <test_id>` shows `retry_pending` between attempts and `completed`/`failed` at terminal.
- [ ] Existing self-dev callbacks that don't need retry still complete normally (regression).
- [ ] `update_task_status {status: "retry_pending"}` returns the structured rejection error.
- [ ] Two heartbeat ticks against the same `retry_pending` task only re-dispatch once (atomicity check).

## Out of scope

- **Generalizing retry to other long-running tools beyond `run_claude_pilot`**: the architecture supports any tool with persisted args (the tool takes `tool_name` + `tool_args`), but only `run_claude_pilot`'s callback handler is updated to use it in this PR. Per architect Finding 7: architecture generalizes; scope doesn't.
- **Changing the long-running guard in `executor.rs`**: that guard remains intact — Option A is explicitly rejected.
- **Heartbeat-driven retry of non-callback failure modes** (e.g., a one-shot network blip during dispatch). Those still surface to the user.
- **Retry storms / exponential backoff beyond the existing `max_retries = 2` cap**. If we see retry abuse, follow up.
- **Stuck `retry_pending` task sweep** (heartbeat handler crashes after flipping to `running` but before dispatching). Mitigation in spirit by atomic flip; full sweep follow-up tracked under #802 lineage.

## Risks

1. **Heartbeat tick latency.** ~60s max + `delay_seconds`. Acceptable; documented in skill prompt so Vincent can set expectations.
2. **Race between `schedule_retry` and a concurrent heartbeat scan.** Mitigated by atomic `UPDATE ... WHERE status = 'retry_pending' RETURNING` inside `BEGIN IMMEDIATE` — the loser of the race gets zero rows and skips.
3. **Stuck `retry_pending` tasks (heartbeat handler crashes mid-flip).** Mitigation: atomic flip writes `updated_at`. A follow-up sweep marks `retry_pending` tasks idle for > T minutes as `failed`; tracked separately, not in this PR.
4. **Skill prompt drift.** If the skill prompt isn't updated in the same release, agents will keep calling `run_claude_pilot` and getting the existing rejection. Mitigation: ship skill change in same PR; CI builds the bundled skill into the binary so they ship atomically (`build.rs` discovers `skills/bundled/`).
5. **Metadata field name collisions.** `pipeline_retry_count` already exists in #579's phantom-retry-guard semantics. We reuse the same field — incrementing it is the correct semantic. Verify the field is read consistently across the two callsites.

## Companion / related

- #800 — per-agent extractor races on shared corpora; orthogonal but lives near this code path.
- #802 — graceful KG-task shutdown on SIGTERM; same "in-flight long-running task lifecycle" surface area, separate concern. The "stuck retry_pending sweep" follow-up belongs in this lineage.
- #743 — cancel_task SIGTERM on active long-running callback; this plan does not change cancel semantics. The `retry_pending → cancelled` transition is supported via the existing operator cancel path.
- Sibling: self-dev callback handler missing `error_max_turns` case (filed concurrently). This plan assumes that case lands; if not, `schedule_retry`'s callsite catches generic failure modes.

## Architect-review changelog

This plan was revised once after a first-pass review by mika-arch (session `05d1f78d-88b4-4dc2-8eb5-d77bc757e7aa`). Findings addressed:

- **F1 (no action)** — Option B confirmed correct.
- **F2** — Added defensive constraint: `update_task_status` explicitly rejects `retry_pending` target. `schedule_retry` is the sole write path.
- **F3** — Heartbeat scan emits exactly one `RetryPending` trigger per tick, ordered by `retry_scheduled_at ASC`.
- **F4 (conditional dispatch-blocker, resolved)** — Verified: callback task already persists args in `input_context` (`executor.rs:846`); for the parent task, `schedule_retry` writes `retry_tool_name` + `retry_tool_args` into existing `metadata` TEXT column. **Path C: no schema columns added for args; metadata JSON suffices.**
- **F5** — Verified: rusqlite + SQLite 3.35+ supports `RETURNING`. Atomic flip uses `BEGIN IMMEDIATE` matching `replace_with_summary` pattern (#636).
- **F6** — Verified: heartbeat tick = 1s, periodic DB scan every 60 ticks → ~60s max latency. Documented in plan and skill prompt.
- **F7** — Added explicit scope statement: architecture generalizes to any long-running tool with persisted args; PR scope is `run_claude_pilot`-only.
- **F8** — Added state machine table and cap-exhausted handler that flips to `failed` + sends notification.
- **F9** — Added negative constraint to skill prompt: "Do NOT call `run_claude_pilot` directly from this callback turn."
