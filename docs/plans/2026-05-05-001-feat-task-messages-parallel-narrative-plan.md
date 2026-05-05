# Plan: `task_messages` Parallel Narrative Table for Cross-Session Task Continuity

**Ticket:** mika#974
**Date:** 2026-05-05
**Status:** GROOMED
**Architect grooming session:** `6e13e3e1-a7dc-4500-ae17-1b13f96c3488` (fresh session, distinct from the prior brief peer-review at `01963864-7c63-4242-a1ff-718941618f8a`)
**Decision provenance:** discovery brief at `mika/docs/brainstorms/2026-05-05-session-continuity-across-scope-types-brainstorm.md`

## Context

mika's compaction primitive deletes messages agent-scoped (`db.rs:5845`: `DELETE FROM messages WHERE agent_id = ?1 AND role != 'summary' AND id <= ?2`), at ~98.5% deletion rate empirically. Task narrative cannot survive across the dispatch/callback boundary without a structural fix. Per the discovery brief, three options were evaluated and **Option C — parallel non-compacted `task_messages` table** was selected on platform-direction grounds (more scope dimensions coming; storage trajectory shows unbounded structured surfaces are already the deployed pattern via `tool_calls`/`llm_calls`).

The architect grooming pass selected **(a) single SQLite transaction** as the consistency-recovery shape for the double-write contract (rationale below).

## Consistency-recovery rule: (a) single SQLite transaction

**Rationale for (a) over (b) and (c):**

SQLite serializes all writes through a single WAL-mode writer. The double-write is `INSERT INTO messages` followed by `INSERT INTO task_messages` in the same transaction — both rows share the same WAL frame. On crash between the two INSERTs, the transaction rolls back and neither row is visible. There is no partial-failure state to recover from.

- **(b) idempotent retry** adds eventual-consistency machinery (retry queue, deduplication on replay) for a failure mode that the SQLite WAL model already eliminates at the transaction boundary. Complexity without benefit.
- **(c) reconciliation worker** introduces a background job that scans `messages` for `task_id IS NOT NULL` and backfills `task_messages`. This requires the `messages.task_id` column (converges to Option A/B shape) and a persistent worker — invasive, contradicts the clean separation C provides.

Transaction overhead: a single SQLite transaction for a double-write is sub-microsecond additional latency. The codebase already wraps multi-row operations in transactions (`replace_with_summary`, `try_extract_callback_metadata`). This is consistent precedent.

**Consistency rule:** `INSERT INTO messages` and `INSERT INTO task_messages` execute inside a single `conn.transaction()` block. If either fails, both roll back. No compensating logic elsewhere.

## Phase 0 — Pre-coding pins (required before any implementation)

Before touching `executor.rs` or `db.rs`, verify exact line ranges from the production source. These are the load-bearing read targets:

1. **`db.rs:5845`** — `replace_with_summary` DELETE clause. Confirm `WHERE agent_id = ?1 AND role != 'summary' AND id <= ?2`. Verify no undocumented `task_id` column already exists on `messages`.
2. **`db.rs`** — current schema version constant (search `SCHEMA_VERSION` or equivalent). Confirm the version number that becomes v_current+1 (current is v30 per `crates/mika-agent/CLAUDE.md`).
3. **`db.rs:5714`** — `INSERT INTO messages` column list. This is the site that gets the transaction wrapper.
4. **`agent.rs::run_loop`** — message-save call sites at lines 1798, 1892, 2247, 2303, 2224, 2279, 2313. Confirm which carry a `task_id`-carrying context today and which don't.
5. **`dispatcher.rs::dispatch_resume_agent`** — locate the `messages` INSERT site (or the `db.*` call that writes callback-turn messages). Confirm `task_id` is accessible from the `Task` struct at that call site.
6. **`agent.rs`** — `load_recent_messages` call site (context assembly). Confirm the function signature — this is the call site that gains a `task_id: Option<&str>` parameter for task-mode rebuild.

Pinning these line ranges before code lands prevents the "claim about deployed surface that turns out to be wrong" failure mode that has bitten the project before.

## Implementation Units

### Unit 1 — Schema migration: `task_messages` table (v30 → v31)

**File:** `crates/mika-agent/src/db.rs` (confirm exact location of migration runner in Phase 0)

```sql
CREATE TABLE IF NOT EXISTS task_messages (
    id         INTEGER PRIMARY KEY AUTOINCREMENT,
    task_id    TEXT NOT NULL,
    agent_id   TEXT NOT NULL,
    session_id TEXT NOT NULL,
    role       TEXT NOT NULL,
    content    TEXT NOT NULL,
    metadata   TEXT,
    trace_id   TEXT,
    created_at TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_task_messages_task_created
    ON task_messages (task_id, created_at);

CREATE INDEX IF NOT EXISTS idx_task_messages_agent_created
    ON task_messages (agent_id, created_at);
```

Schema version bump: increment whatever constant guards the migration runner. Migration is additive — no existing table altered, no data touched. Safe to run on live DB.

**What does NOT change:** `messages` table schema is untouched. No `task_id` column added to `messages` — that would be Option A/B shape, not C. Compaction's DELETE clause is untouched.

### Unit 2 — DB layer: `insert_task_message_tx` and `insert_message_with_task_context`

**File:** `crates/mika-agent/src/db.rs`

Two new functions:

```rust
/// Insert a single row into task_messages.
/// Called inside a transaction opened by the caller.
pub fn insert_task_message_tx(
    tx: &Transaction,
    task_id: &str,
    agent_id: &str,
    session_id: &str,
    role: &str,
    content: &str,
    metadata: Option<&str>,
    trace_id: Option<&str>,
    created_at: &str,
) -> Result<i64>

/// Double-write: insert into messages (existing path) AND task_messages,
/// wrapped in a single transaction. task_id=None → messages-only (existing behavior).
pub fn insert_message_with_task_context(
    &mut self,
    /* existing messages fields */,
    task_id: Option<&str>,
) -> Result<i64>
```

When `task_id` is `Some`: open transaction, call existing `messages` INSERT, call `insert_task_message_tx`, commit. When `task_id` is `None`: call existing `messages` INSERT directly — no transaction overhead, no behavioral change.

**Constraint:** All existing callers must compile with `task_id: None`. Use `Option<&str>` to avoid silent breakage at call sites.

### Unit 3 — Scope-root resolution helper: `resolve_scope_root_task_id`

**File:** `crates/mika-agent/src/agent.rs` or `crates/mika-agent/src/db.rs`

```rust
/// Walk parent_task_id chain to the nearest scope root
/// (type IN ('issue', 'milestone', 'project')).
/// Returns None if no scope ancestor exists.
pub fn resolve_scope_root_task_id(
    db: &Db,
    task_id: &str,
) -> Result<Option<String>>
```

Single resolution site — all write paths call this helper. Cached for the duration of a turn (no persistent state).

**Edge cases:**
- Task not found → `None`.
- `type` not in scope set → walk up until scope root found or chain exhausted.
- Circular chain → depth limit of `SCOPE_ROOT_WALK_DEPTH_LIMIT = 20` hops; return `None` + emit `warn!(scope_root_walk_depth_limit_exceeded)`.
- `parent_task_id` points to non-existent task → `None`, no panic.

**Implementation note on `SCOPE_ROOT_WALK_DEPTH_LIMIT = 20`:** task hierarchies in the deployed model never exceed N=3 today (project → milestone → issue). 20 gives 6× headroom against pathological chains while still bounding worst-case walk cost. Land as a named `const` with a comment explaining the rationale — magic number 20 inline would obscure the intent.

### Unit 4 — Write-site instrumentation (four paths)

**4a. `agent.rs::run_loop`** — Resolve `scope_task_id: Option<String>` once at turn start from `AgentParams`'s current task context. Cache as turn-local. All `insert_message` calls at lines 1798, 1892, 2247, 2303 become `insert_message_with_task_context(..., scope_task_id.as_deref())`. Deadline-fallback paths (2224, 2279, 2313) inherit same turn-local value.

**4b. `dispatcher.rs::dispatch_resume_agent`** — `Task` struct is in scope. Walk `task.parent_task_id` to resolve `scope_task_id`. Callback-turn message-save call passes `scope_task_id.as_deref()`.

**4c. Webhook handler path** — Handlers that complete an issue→task lookup pass the resolved `task_id` to `insert_message_with_task_context`. Handlers without a successful issue→task mapping pass `None` → `messages` only. **This is the untagged-row fallback in practice.**

**4d. `send_message` outbound** — Inherits `scope_task_id` from current turn context (same source as 4a). No separate resolution.

**What does NOT change:** `messages.internal` semantics are untouched. No conflation of `internal=1` with task-context tagging. The discovery brief's reviewer note (two orthogonal concerns on one boolean) is resolved structurally by C — `task_messages` carries task-context marker; `internal` remains agent-to-agent visibility only.

### Unit 5 — Read-side: `rebuild_context` with task-mode

**File:** `crates/mika-agent/src/agent.rs`

```rust
pub fn rebuild_context(
    db: &Db,
    session_id: &str,
    task_id: Option<&str>,
    limit: usize,
) -> Result<Vec<Message>> {
    match task_id {
        Some(tid) => db.load_task_messages(tid),        // full history, no limit
        None      => db.load_recent_messages(session_id, limit, false), // existing
    }
}
```

**Hybrid-mode** (operator queries a task from a channel): merge both results sorted by `created_at`. Dedup on `(session_id, role, content, created_at)` — not on `id` (different ID spaces across tables).

**Constraint:** `replace_with_summary` reads only from `messages` — untouched. Compaction's LLM summary covers channel narrative; task narrative preserved verbatim in `task_messages`. These are permanently separate read surfaces.

## Schema migration approach

Single additive migration: v30 → v31 (confirm exact number in Phase 0). `CREATE TABLE IF NOT EXISTS` + `CREATE INDEX IF NOT EXISTS` — idempotent, no data transformation, safe on live DB. Follow exact pattern of most recent prior migration in the codebase.

## Test plan

### Unit tests

1. **`test_double_write_tagged_event`** — Insert with `task_id = Some("root-task-123")`. Assert row in both `messages` and `task_messages`. Assert transaction atomicity: mid-transaction abort → zero rows in both tables.

2. **`test_single_write_untagged_event`** — Insert with `task_id = None`. Assert row in `messages`. Assert `task_messages` has zero rows.

3. **`test_scope_root_walk`** — Build `tasks` tree: project → milestone → issue. Call `resolve_scope_root_task_id` from issue. Assert returns project-level `task_id`. Test depth-limit guard: 21-hop chain → `None` + `warn!` emitted.

4. **`test_malformed_parent_chain`** — `parent_task_id` points to non-existent task. Assert returns `None`, no panic, no error propagation.

### Integration tests (acceptance criteria from ticket body)

5. **Happy-path replay test** — Dispatch a milestone with N children. Walk callbacks via `load_task_messages(scope_root_id)`. Assert messages from dispatch session (turn 1), child callback (turn 2), and subsequent dispatch (turn 3) are all present in `task_messages` sorted by `created_at`. Assert "advance to item N+1" intent from dispatch session is visible when calling `rebuild_context(..., Some(scope_root_id))` from a callback session.

6. **Mixed-tagging acceptance test (universal-fallback exercise)** — Interleave tagged dispatch turns and untagged events (simulated generic webhook, cron heartbeat). Assert:
   - Task-mode rebuild surfaces only tagged events.
   - Channel-mode rebuild from same session surfaces both tagged and untagged rows from `messages`.
   - `task_messages` row count equals exactly the count of tagged insertions — no false entries from untagged events.
   - **Key assertion:** fire `replace_with_summary` manually at threshold. Assert `task_messages` rows survive untouched. Assert tagged `messages` rows are deleted (compaction has no `task_id` column visibility — correct). Assert task-mode rebuild still returns full narrative from `task_messages` post-compaction. **This is the structural guarantee the ticket delivers.**

## Out-of-scope confirmation

- No `messages.task_id` column — Option A/B territory.
- No compaction changes — `replace_with_summary` untouched.
- No cross-agent task threading.
- mika#965 downstream: once #974 lands, #965 scope narrows to "callback writes participate in `task_messages` double-write via Unit 4b." No `internal=1` extension needed.

## Disposition: READY

Architect grooming pass complete. Plan suitable for `/ce:work` or direct dev-pilot dispatch via `ready` label.
