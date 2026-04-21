---
module: kg
date: 2026-04-22
problem_type: best_practice
component: database
severity: medium
tags:
  - async-db
  - mutex
  - domain-graph
  - startup
  - with-db
applies_when:
  - Building a startup-time data pipeline that reads from a std::sync::Mutex-guarded registry and writes to AsyncDatabase
  - Any code path that needs to enumerate data from a sync-locked source and then perform async DB writes
  - Implementing idempotent UPSERT patterns for projection tables populated from authoritative sources
---

# Two-Phase Async Rebuild Pattern for Sync Mutex + AsyncDatabase

## Context

When building the `DomainGraphBuilder` (#687), the startup sequence needed to:
1. Read skill/tool data from `SkillRegistry` (behind a `std::sync::Mutex` in `AgentState`)
2. Write entities and relationships to SQLite via `AsyncDatabase::with_db()` (async)

The naive approach — holding the `MutexGuard` while calling `builder.rebuild().await` — triggered `clippy::await_holding_lock` because the guard would be held across multiple await points (`with_db` sends work to a dedicated DB thread via an async channel).

## Guidance

Split the operation into two phases:

1. **Enumerate (sync)** — While holding the lock, call a method that reads all data and produces an owned `DesiredState` struct. No async, no await. The lock is released when the guard drops.

2. **Write (async)** — After the lock is released, pass the owned state to an async method that does the DB writes via `with_db`.

The API pattern uses `enumerate_to_pending()` which returns a `PendingRebuild` struct:

```rust
// Phase 1: sync enumeration while holding the lock
let pending = {
    let skill_reg = state.skills.lock().expect("skills lock poisoned");
    let builder = DomainGraphBuilder::new(&db, &skill_reg, tool_defs, mcp_ref, &agents);
    builder.enumerate_to_pending()
}; // lock released here

// Phase 2: async DB write, lock is no longer held
match pending.write(&db).await {
    Ok(stats) => info!("domain graph ready"),
    Err(e) => warn!("domain graph rebuild failed: {e}"),
}
```

The `PendingRebuild` struct owns all the data needed for the write phase:

```rust
pub struct PendingRebuild {
    desired: DesiredState,  // entities + edges + entity_keys
    trace_id: String,
}

impl PendingRebuild {
    pub async fn write(self, db: &AsyncDatabase) -> Result<RebuildStats> {
        write_desired_state(db, self.desired, &self.trace_id).await
    }
}
```

## Why This Matters

- `clippy::await_holding_lock` is a correctness lint, not a style lint. Holding a `std::sync::Mutex` across an await point can cause deadlocks if another task on the same tokio runtime needs the same lock.
- `AsyncDatabase::with_db()` requires `Send + 'static` closures, so you can't capture references to the `MutexGuard` anyway — the data must be owned.
- The pattern also works as a clean API: callers who don't need the two-phase split can use `builder.rebuild().await` directly (which calls both phases internally).

## When to Apply

- Any startup hook that reads from `AgentState.skills` (or other `std::sync::Mutex`-guarded fields) and writes to `AsyncDatabase`
- Future KG builders (lexical ingestion, subject extraction) if they need registry data
- Any projection/materialized-view pattern where the source is sync-locked and the sink is async

## Examples

The `DomainGraphBuilder` in `crates/mika-agent/src/kg/domain_builder.rs` is the canonical example. The server startup hook in `crates/mika-agent/src/server/mod.rs` (search for "Domain graph rebuild") shows the two-phase call site.

The idempotent write strategy inside `write_desired_state` is also reusable:
- **Entities**: `INSERT ... ON CONFLICT(entity_key) DO UPDATE` preserves rowids for FK stability
- **Relationships**: `DELETE WHERE type IN (domain types)` then `INSERT` — simpler than per-edge upsert since relationships have no downstream FK references
- **Prune**: `DELETE FROM kg_entities WHERE entity_type IN (domain types) AND entity_key NOT IN (desired)` — scoped to builder-owned types only
- All three operations in a single `unchecked_transaction()` for atomicity
