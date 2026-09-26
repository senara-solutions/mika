# Groomed No-TBDs Gate: Clean Plan (READY)

## Plan Under Review: docs/plans/1244-session-cleanup.md

### Summary

Add an automatic session cleanup job that prunes sessions older than 30 days
from the SQLite database to prevent unbounded growth.

### Design

- **Trigger:** Periodic task via existing task engine, every 24 hours
- **Retention:** 30 days (hardcoded constant `SESSION_RETENTION_DAYS = 30`)
- **Scope:** Per-agent — each agent's cleanup runs against its own DB
- **Cascade:** Delete session rows first, then orphaned messages via FK cascade
- **Transaction:** Single DEFERRED transaction wrapping the DELETE + VACUUM

### Implementation

1. Add `cleanup_old_sessions(retention_days: u32)` method to `Database`
2. Register a recurring task in `ensure_recurring_task()` with 24h interval
3. Add `SilentTrigger::SessionCleanup` variant (reuses `safe_always_on_skills()`)
4. The silent agent turn calls `cleanup_old_sessions` directly (no LLM needed)

### Error Handling

- Transaction failure → log `session_cleanup_error` WARN, retry on next tick
- Empty result (no old sessions) → log `session_cleanup_noop` DEBUG, no-op

### Test Plan

- Unit: `cleanup_old_sessions` with seeded sessions at various ages
- Integration: verify FK cascade deletes orphaned messages
- Edge case: concurrent cleanup + compaction (both touch sessions table)

### Scope Boundaries

- No configurable retention period (hardcoded 30 days)
- No per-session exemptions
- No manual trigger (defer to future ticket)

### Files

- `crates/mika-agent/src/db.rs` — new `cleanup_old_sessions` method
- `crates/mika-agent/src/task_engine/mod.rs` — register recurring task
- `crates/mika-agent/src/agent.rs` — new `SilentTrigger` variant
