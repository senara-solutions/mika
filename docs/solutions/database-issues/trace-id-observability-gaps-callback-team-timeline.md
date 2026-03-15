---
title: "Fix trace_id/session_id linkage gaps breaking unified timeline observability"
category: database-issues
date: 2026-03-15
tags: [trace-id, session-id, unified-timeline, team-engine, schema-migration, observability]
components: [agent-core, team-engine, database]
severity: medium
root_cause_type: missing-data-correlation
github_issues: [160, 161, 162]
schema_version: 10
---

# Fix trace_id/session_id Linkage Gaps Breaking Unified Timeline Observability

## Problem

Three observability gaps broke trace_id correlation in the `unified_timeline` dashboard. In each case, trace_id data was either available but not propagated, not persisted across async boundaries, or an entire data source was excluded from the correlation VIEW.

### Symptoms

1. **Callback tasks appeared as orphans** — long-running skill callback tasks had `created_trace_id = NULL`, making them unlinkable to the originating agent turn.
2. **Resumed team runs broke trace correlation** — after suspend/resume, all events from the resumed run had a different trace_id than events before suspend.
3. **Team orchestration invisible to dashboard** — goals, assignments, critic feedback, and deliverables stored in `team_workspace` were absent from `unified_timeline` queries.

## Root Cause

1. **`executor.rs:528`** — `LongRunningContext` carried `trace_id`, but `NewTask` hardcoded `created_trace_id: None`.
2. **`engine.rs:195`** — `new_for_resume()` called `generate_trace_id()` unconditionally. The `team_runs` table had no `trace_id` column to persist it.
3. **`db.rs:27-46`** — `UNIFIED_TIMELINE_VIEW_SQL` only unioned `messages`, `audit_events`, and `tasks`. `team_workspace` was excluded.

## Solution

### Fix 1: Wire trace_id into long-running callback tasks

One-line change in `crates/mika-agent/src/skills/executor.rs`:

```rust
// Before
created_trace_id: None,

// After
created_trace_id: Some(ctx.trace_id.clone()),
```

### Fix 2: Persist and restore trace_id across team run suspend/resume

Schema migration v9→v10 adds `trace_id TEXT` column to `team_runs`. The migration uses `BEGIN IMMEDIATE` / `COMMIT` transaction wrapping with a `column_exists` idempotency guard:

```rust
fn migrate_v9_to_v10(&self) -> Result<()> {
    self.conn.execute_batch("BEGIN IMMEDIATE;")?;
    let result = (|| -> Result<()> {
        if !self.column_exists("team_runs", "trace_id")? {
            self.conn.execute_batch("ALTER TABLE team_runs ADD COLUMN trace_id TEXT;")?;
        }
        self.conn.execute_batch("DROP VIEW IF EXISTS unified_timeline;")?;
        self.conn.execute_batch(UNIFIED_TIMELINE_VIEW_SQL)?;
        self.conn.execute_batch(
            "CREATE INDEX IF NOT EXISTS idx_team_ws_trace ON team_workspace(trace_id)
                 WHERE trace_id IS NOT NULL;",
        )?;
        self.conn.execute("INSERT INTO schema_version (version) VALUES (10)", [])?;
        Ok(())
    })();
    match result {
        Ok(()) => { self.conn.execute_batch("COMMIT;")?; Ok(()) }
        Err(e) => { let _ = self.conn.execute_batch("ROLLBACK;"); Err(e) }
    }
}
```

`insert_team_run` now accepts and persists `trace_id: Option<&str>`. `new_for_resume` was changed from sync to `async` to load trace_id from DB, with explicit error handling:

```rust
let trace_id = match team_db.load_team_run_trace_id(&run.run_id).await {
    Ok(Some(tid)) => tid,
    Ok(None) => {
        debug!(run_id = %run.run_id, "no trace_id in team_run (pre-v10), generating fresh");
        mika_common::trace::generate_trace_id()
    }
    Err(e) => {
        warn!(error = %e, run_id = %run.run_id, "failed to load trace_id, generating fresh");
        mika_common::trace::generate_trace_id()
    }
};
```

### Fix 3: Add `team_workspace` to `unified_timeline` VIEW

New UNION ALL leg in `UNIFIED_TIMELINE_VIEW_SQL`:

```sql
UNION ALL
SELECT trace_id, 'team-' || run_id AS session_id, NULL AS agent_id,
    'team_workspace' AS event_type, entry_type AS event_subtype,
    CASE WHEN length(content) > 200 THEN substr(content, 1, 200) || '...'
         ELSE content END AS summary,
    created_at
FROM team_workspace
```

Column mapping: `agent_id=NULL` (team entries are cross-agent), `session_id` synthesized as `team-{run_id}` to match the team session naming convention.

### Code review fixes

- **Transaction wrapping** — `migrate_v9_to_v10` wrapped in `BEGIN IMMEDIATE/COMMIT/ROLLBACK` with `column_exists` idempotency guard (crash recovery).
- **Missing index** — Added `idx_team_ws_trace` partial index on `team_workspace(trace_id)` for consistency with `idx_msg_trace`, `idx_audit_trace`, `idx_tasks_trace`.
- **Wildcard match** — Split `_` catch-all in `new_for_resume` into explicit `Ok(None)` / `Err(e)` arms with appropriate logging.

## Files Changed

| File | Change |
|------|--------|
| `crates/mika-agent/src/skills/executor.rs` | 1-line fix: wire `created_trace_id` |
| `crates/mika-agent/src/db.rs` | Schema v10, VIEW SQL, `insert_team_run` signature, `load_team_run_trace_id`, clean-slate schema, tests |
| `crates/mika-agent/src/async_db.rs` | Async wrappers for new/updated DB methods |
| `crates/mika-agent/src/teams/engine.rs` | `new_for_resume` async + trace_id restore, `execute` passes trace_id |
| `crates/mika-agent/src/teams/mod.rs` | `.await` on `new_for_resume` |
| `crates/mika-agent/src/tools/get_team_history.rs` | Test signature updates |
| `crates/mika-agent/src/tools/get_team_status.rs` | Test signature updates |

## Prevention Checklist

When adding trace_id to new subsystems:

- [ ] Every `created_trace_id` field populated from context (never `None` when trace_id is available)
- [ ] Resume/restore paths reload persisted trace_id (never `generate_trace_id()` at resume sites)
- [ ] New tables with `trace_id` column added to `unified_timeline` VIEW
- [ ] Partial index `idx_{table}_trace ON {table}(trace_id) WHERE trace_id IS NOT NULL` created
- [ ] Schema migration transaction-wrapped with idempotency guards
- [ ] Clean-slate schema updated to match migration result

## Code Review Patterns

- **"Follow the None"** — When a field is set to `None`, check if a value is available in the enclosing context.
- **"Grep for fresh generation at resume sites"** — Search for `generate_trace_id()` calls; at resume/continuation sites, fresh generation is a bug.
- **"Symmetric coverage audit"** — When adding a table, check all parallel subsystems (VIEW membership, indexes, context propagation) for consistency.
- **"Transaction boundary check"** — Verify all DDL/DML + version bump are inside the same transaction.

## Related

- [trace-id-correlation-unified-observability](../architecture-patterns/trace-id-correlation-unified-observability.md) — Original trace_id architecture (schema v5)
- [sql-column-mismatch-trace-detail-view](sql-column-mismatch-trace-detail-view.md) — Prior trace_id column mismatch bug
- [team-graph-persistence-replacing-toml-history](team-graph-persistence-replacing-toml-history.md) — `team_workspace` table origin
- [team-task-child-wrong-agent-id](team-task-child-wrong-agent-id.md) — Related team engine agent_id scoping issue
- [callback-resume-agent-lifecycle](../architecture/callback-resume-agent-lifecycle.md) — Callback task lifecycle
- GitHub: [#160](https://github.com/senara-solutions/mika/issues/160) (P1), [#161](https://github.com/senara-solutions/mika/issues/161) (P2), [#162](https://github.com/senara-solutions/mika/issues/162) (P3)
