---
title: "AsyncDatabase Wrapper: Bridging sync rusqlite to async tokio Runtime"
category: architecture
problem_type: async_integration
component: mika-agent
modules: [async_db, db, agent, tools, scheduler, compaction, cli]
tags: [rust, async, sqlite, tokio, mpsc, closure-dispatch, phase0]
date_solved: 2026-02-24
commit: 38a843b
severity: medium
---

# AsyncDatabase Wrapper: Bridging sync rusqlite to async tokio Runtime

## Problem

Mika's agent core uses rusqlite for per-customer SQLite storage. rusqlite's `Connection` is `!Send` and all operations are synchronous. The agent loop, tools, scheduler, and compaction all run on tokio's async runtime. Phase 2 (HTTP server) requires all database access to be non-blocking so that SQLite I/O doesn't stall the tokio event loop under concurrent load.

### Symptoms

- Direct `Database` calls on the tokio runtime would block async tasks
- `rusqlite::Connection` is `!Send`, preventing `spawn_blocking` with a shared connection
- ~35 database methods needed async wrappers without changing the proven sync `Database` implementation
- Tool futures use `#[async_trait(?Send)]`, adding constraints on how the wrapper could be designed

### Root Cause

rusqlite wraps SQLite's C library, which is inherently synchronous and single-threaded per connection. The `Connection` type is `!Send` by design. Tokio's `spawn_blocking` requires `Send` closures, so you can't simply wrap each call — you need a dedicated thread that owns the connection.

## Investigation

### Approaches Considered

1. **`Arc<Mutex<Database>>` + `spawn_blocking`** — Simple but creates a new blocking task per call, mutex contention under load, and doesn't guarantee connection affinity.

2. **Closure-based dispatch via dedicated OS thread** (chosen) — Single thread owns the `Database`, callers send `Box<dyn FnOnce(&Database) + Send>` via `std::sync::mpsc`, results return via `tokio::sync::oneshot`. Zero mutex contention, guaranteed single-writer, cheap `Clone`.

3. **Enum-based dispatch** — Define a 35-variant enum for all operations. Type-safe but massive boilerplate and painful to extend.

4. **r2d2 connection pool** — Overkill for single-customer containers. SQLite WAL mode + single writer makes pooling counterproductive.

### Why Closure Dispatch Won

- **No enum boilerplate**: Adding a new DB method = 5 lines (clone args, send closure, await result)
- **Connection affinity**: Single thread owns the connection — no lock contention
- **Send+Sync wrapper**: `mpsc::Sender` is `Send+Sync`, so `AsyncDatabase` is clone-able and sharable across tasks
- **Compatible with `?Send` futures**: Tool closures capture `&AsyncDatabase` (which is `Send`), while the actual `Database` ref never crosses thread boundaries

## Solution

### Architecture

```
┌─────────────┐     mpsc::channel      ┌──────────────────┐
│ AsyncDatabase│ ──── Box<FnOnce> ────▶ │ Dedicated OS     │
│ (Clone+Send) │                        │ Thread           │
│              │ ◀── oneshot reply ──── │ owns Database    │
└─────────────┘                         └──────────────────┘
```

### Core Implementation (`crates/mika-agent/src/async_db.rs`)

```rust
type DbClosure = Box<dyn FnOnce(&Database) + Send>;

#[derive(Clone)]
pub struct AsyncDatabase {
    sender: mpsc::Sender<DbClosure>,
}

impl AsyncDatabase {
    pub fn new(db: Database) -> Self {
        let (tx, rx) = mpsc::channel::<DbClosure>();
        std::thread::spawn(move || {
            while let Ok(f) = rx.recv() {
                f(&db);
            }
        });
        Self { sender: tx }
    }

    async fn with_db<T: Send + 'static>(
        &self,
        f: impl FnOnce(&Database) -> Result<T> + Send + 'static,
    ) -> Result<T> {
        let (tx, rx) = oneshot::channel();
        self.sender
            .send(Box::new(move |db| {
                let _ = tx.send(f(db));
            }))
            .map_err(|_| anyhow!("database thread has stopped"))?;
        rx.await.map_err(|_| anyhow!("database thread dropped reply"))?
    }

    // Each public method clones args to owned, calls with_db
    pub async fn save_message(&self, role: &str, content: &str, channel_type: &str) -> Result<i64> {
        let (r, c, ct) = (role.to_owned(), content.to_owned(), channel_type.to_owned());
        self.with_db(move |db| db.save_message(&r, &c, &ct)).await
    }
    // ... ~35 methods following same pattern
}
```

### Key Pattern: String Cloning at Boundary

Every `&str` parameter is cloned to `String` before capture in the `'static + Send` closure:

```rust
pub async fn store_fact(&self, category: &str, key: &str, value: &str, context: &str) -> Result<i64> {
    let (c, k, v, ctx) = (category.to_owned(), key.to_owned(), value.to_owned(), context.to_owned());
    self.with_db(move |db| db.store_fact(&c, &k, &v, &ctx)).await
}
```

### Integration Changes (19 files, 965 insertions, 231 deletions)

| File | Change |
|------|--------|
| `lib.rs` | Added `pub mod async_db;` |
| `tools/mod.rs` | `ToolContext.db: &'a Database` → `&'a AsyncDatabase` |
| `agent.rs` | `AgentParams.db` + `SilentAgentParams.db` → `&'a AsyncDatabase`, `.await` on ~15 db calls |
| `compaction.rs` | `.await` on ~5 db calls |
| `scheduler.rs` | `ReminderScheduler.db` → `&'a AsyncDatabase`, `.await` on ~8 db calls |
| `cli.rs` | Wrap: `let async_db = AsyncDatabase::new(db);`, async slash commands |
| `test_utils.rs` | Added `test_async_db()` → `AsyncDatabase`, updated `test_ctx` |
| All 8 tool files | Added `.await` to each `ctx.db.*` call |

### Bundled Fixes (Same Commit)

This commit also resolved 10 of 12 v3 code review findings:

- **#102**: `cancel_reminder` rejects `id <= 0` (was `id == 0`)
- **#100**: `search_memory` includes `context` field in event results
- **#105**: System prompt mentions `search_memory` tool
- **#108**: Core memory wrapped in `<core-memory>` XML tags, commitments in `<commitments>` tags
- **#103**: Compaction uses `chrono::NaiveDateTime` parsing (was fragile string slicing)
- **#104**: Batch `DELETE FROM memory_events WHERE created_at < ?1` (was per-event loop)
- **#106**: `INSERT ... ON CONFLICT(year, month) DO UPDATE` for summaries
- **#099**: SELECT + INSERT + DELETE wrapped in single `unchecked_transaction`
- **#101**: `compact_old_memory_events` returns `usize`; VACUUM only when `deleted > 0`
- **#098**: 15 new compaction tests (comprehensive coverage)

## Prevention

### 1. Async Wrapper Parity Testing

Every public method on `Database` that is exposed through `AsyncDatabase` should have a round-trip test:

```rust
#[tokio::test]
async fn test_async_save_and_load() {
    let db = test_async_db();
    let id = db.save_message("user", "hello", "cli").await.unwrap();
    let msgs = db.get_recent_messages(10).await.unwrap();
    assert_eq!(msgs.len(), 1);
}
```

### 2. Return Type Validation

When adding new async wrappers, match the return type exactly to the sync method. A mismatch (e.g., `Result<()>` vs `Result<i64>`) will compile but silently discard data.

### 3. XML Delimiter Discipline

Prompt sections using XML tags (`<core-memory>`, `<commitments>`) must use consistent open/close tags. Add assertion tests for tag presence in prompt output.

### 4. Compaction Safety

The refactored compaction function wraps all operations in a single transaction. Future modifications must maintain this — never separate the SELECT, INSERT, and DELETE into independent transactions.

## Related

- [Phase 0 Implementation Plan](../../docs/plans/2026-02-24-feat-phase0-resolve-v3-findings-async-database-plan.md)
- [Code Review Workflow Solution](../code-review-workflow/multi-agent-code-review-with-compound-engineering.md)
- [v1+v2 Review Findings Resolution](../refactoring/resolving-code-review-findings-across-codebase.md)
- Todo: `todos/111-pending-p2-vacuum-blocks-db-thread.md` — VACUUM stalls DB thread
- Todo: `todos/112-pending-p2-db-thread-panic-resilience.md` — catch_unwind needed
- Todo: `todos/116-pending-p3-no-graceful-db-thread-shutdown.md` — shutdown() method for Phase 2

## Test Cases

- `test_async_save_and_load` — round-trip message through wrapper
- `test_async_concurrent_reads` — multiple concurrent tasks sharing cloned handle
- `test_async_clone_shares_connection` — clones route to same background thread
- `test_async_open_helper` — `AsyncDatabase::open()` convenience method
- All 132 existing tests pass (15 new compaction tests + 4 async tests added)
