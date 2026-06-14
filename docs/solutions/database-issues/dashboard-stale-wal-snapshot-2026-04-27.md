---
title: Dashboard reads stale SQLite WAL snapshot — sessions invisible until restart
date: 2026-04-27
category: database-issues
module: mika-agent/server
problem_type: database_issue
component: database
symptoms:
  - Dashboard API returns stale data — new sessions invisible until mika-spirit restart
  - DB has more sessions than dashboard shows (e.g. 5731 vs 5705)
  - Newest visible session is over an hour behind actual newest
  - WAL file grows to several MB without being checkpointed
root_cause: scope_issue
resolution_type: code_fix
severity: high
tags:
  - sqlite
  - wal
  - stale-snapshot
  - dashboard
  - transaction-leak
  - raii
  - checkpoint
---

# Dashboard reads stale SQLite WAL snapshot — sessions invisible until restart

## Problem

The dashboard API (`/api/v1/sessions`) returns stale data. Sessions created by other processes (e.g. `mika ask --agent mika-dev`) are invisible in the dashboard until mika-spirit is restarted. The root cause is the server's `AsyncDatabase` connection getting pinned to a stale WAL snapshot.

## Symptoms

- `sqlite3 ~/.mika/data/mika.db "SELECT COUNT(*) FROM sessions"` returns 5731
- Dashboard API `/api/v1/sessions?page=1&per_page=1` shows total 5705
- After `rc-service mika-spirit restart`, dashboard immediately shows all sessions
- WAL file at 3.9MB confirms un-checkpointed writes

## What Didn't Work

- **Restarting the server** — fixes the symptom temporarily but not the root cause. Sessions created after restart eventually become invisible again.
- **PRAGMA wal_autocheckpoint** alone — the global checkpoint threshold doesn't help when the connection is pinned to a stale snapshot by a stuck transaction.

## Solution

Two complementary fixes:

### Fix A — RAII Transactions (root cause)

Replace raw `BEGIN`/`BEGIN IMMEDIATE` + `execute_batch("COMMIT")` patterns with `rusqlite::Transaction` RAII. The `Transaction` type's `Drop` implementation runs `ROLLBACK` automatically if `commit()` is not called, preventing stuck transactions from pinning the WAL snapshot.

Three runtime callsites were load-bearing for this bug:

```rust
// Before (replace_with_summary — DEFERRED):
self.conn.execute_batch("BEGIN")?;
self.conn.execute("DELETE FROM messages ...", params![...])?;
self.conn.execute("INSERT INTO messages ...", params![...])?;
self.conn.execute_batch("COMMIT")?;

// After:
let tx = self.conn.transaction()?;  // DEFERRED — preserves existing semantics
tx.execute("DELETE FROM messages ...", params![...])?;
tx.execute("INSERT INTO messages ...", params![...])?;
tx.commit()?;
// On any `?` early-return: `tx` drops, automatic ROLLBACK fires.
```

For IMMEDIATE transactions (`set_skill_enabled`, `delete_skill_llm_override`):

```rust
let tx = self.conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
```

The `Transaction` refactor required changing `Database` methods from `&self` to `&mut self` (since `Connection::transaction()` requires `&mut`), cascading through `AsyncDatabase` (`DbClosure` now takes `&mut Database`) and all callers.

### Fix B — Periodic WAL checkpoint (defense-in-depth)

New `server::checkpoint` module spawns a tokio task that runs `PRAGMA wal_checkpoint(PASSIVE)` every 60 seconds on the dashboard DB connection, forcing snapshot refresh even if a future regression introduces a new stuck-transaction path.

```rust
// In run_server(), after dashboard_db construction:
checkpoint::spawn_dashboard_checkpoint_task(dashboard_db.clone());
```

## Why This Works

In SQLite WAL mode, each connection sees writes committed before its current read transaction started. If a connection holds a long-lived implicit read transaction (from a `BEGIN` that was never committed or rolled back after an error between `BEGIN` and `COMMIT`), all subsequent reads see the stale snapshot.

The `dashboard_db` is constructed via `default_agent.db.clone()` — they share the same underlying `Connection` (same OS thread, same `AsyncDatabaseInner` Arc). Any stuck transaction on the default agent's connection pins the dashboard's WAL snapshot too.

RAII `Transaction` eliminates the leak path (Drop auto-rolls back on error). The periodic checkpoint is the safety net: even if a leak occurs, the checkpoint forces the connection to see the latest WAL state.

## Prevention

- **Always use `rusqlite::Transaction` for multi-statement writes.** Never use raw `execute_batch("BEGIN")` + `execute_batch("COMMIT")` — the error path between them can skip the COMMIT and leave the transaction open.
- **Preserve locking semantics.** Use `conn.transaction()` for DEFERRED (default) and `conn.transaction_with_behavior(TransactionBehavior::Immediate)` for IMMEDIATE.
- **Monitor `checkpoint.complete` log events.** If `busy_pages` is consistently non-zero, investigate whether a connection is holding a long read transaction.

## Related Issues

- [mika#636](https://github.com/senara-solutions/mika/issues/636) — Original issue report
