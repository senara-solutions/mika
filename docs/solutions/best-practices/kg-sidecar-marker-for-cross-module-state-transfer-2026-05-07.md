---
module: kg/entity_resolver
date: 2026-05-07
problem_type: best_practice
component: tooling
severity: low
tags:
  - kg
  - entity-resolver
  - domain-builder
  - sidecar-table
  - observability
  - cross-module-state
applies_when:
  - A module deletes rows that another module needs to detect were previously present
  - Cross-module state transfer must survive process crashes
  - An in-memory tracker would require threading Arc<Mutex<T>> through unrelated code paths
---

# Sidecar Marker Table for Cross-Module State Transfer in KG

## Context

Issue #960 introduced domain-graph rebuild invalidation: when `domain_builder::rebuild()` adds new entities of a type, it DELETEs `kg_resolutions_log` rows with `outcome='no_match'` for subjects of that type. This makes those entities re-enter the pending pool for the next resolver tick.

Issue #961 needed the resolver to distinguish "reattempted after invalidation" entities from "never resolved" entities — both appear identical in the pending query (`r.id IS NULL`). The domain builder and resolver are separate modules with no shared runtime state; the domain builder runs once at startup, the resolver runs per-agent at startup and every 30 minutes.

## Guidance

Use a lightweight sidecar table as a durable marker when one module needs to communicate state to another module that runs later, and the original data was deleted rather than soft-flagged.

The pattern:
1. **Writer module** INSERTs marker rows in the same transaction that DELETEs the original data
2. **Reader module** LEFT JOINs the marker table in its existing query to detect the state
3. **Reader module** DELETEs marker rows after processing, in the same `with_db` closure that writes the new state

Key design decisions for `kg_invalidated_no_match`:
- **No FK constraints** — the table is ephemeral; entities may be deleted independently
- **`INSERT OR IGNORE`** — safe for crash-restart-crash-restart (duplicate markers are no-ops)
- **Cleanup in `write_log()` not `apply_result()`** — ensures cleanup from ALL code paths (startup, compound-hook, periodic tick), not just the `resolve_pending` path
- **Composite PK `(subject_entity_id, agent_id)`** — matches `kg_resolutions_log`'s per-agent granularity for multi-agent shared-corpus deployments

## Why This Matters

Three alternatives were considered and rejected:
- **In-memory `HashSet`**: Would require `Arc<Mutex<HashSet<i64>>>` threaded from `domain_builder` through `AgentState` to `SubjectEntityResolver`. Doesn't survive crashes. Adds coupling between modules that currently share no state.
- **Soft-delete on `kg_resolutions_log`** (add `invalidated_at` column): Requires expanding the CHECK constraint (table rebuild migration) and modifying the pending query's WHERE clause. More invasive than a separate table.
- **Marker column on `kg_subject_entities`**: Violates the subject extractor's sole-writer contract for that table.

The sidecar table costs one CREATE TABLE (additive migration, no data conversion), two helper functions in `kg_schema.rs`, and a LEFT JOIN on a table that typically has 0-100 rows.

## When to Apply

This pattern is appropriate when:
- Module A deletes data that Module B needs to detect was previously present
- The modules run in different execution contexts (different startup phases, different tick intervals)
- The state transfer must survive process crashes between the write and read
- The marker data is small and ephemeral (cleaned up after first read)

It is NOT appropriate when:
- The modules share runtime state already (use a field on the shared struct instead)
- The original data can be soft-deleted instead (add a status/flag column)
- The marker would accumulate unboundedly (add a TTL or periodic sweep)

## Examples

Helper functions in `kg_schema.rs` keep the SQL isolated:

```rust
// Writer side (domain_builder, inside transaction):
kg_schema::record_invalidated_no_match(&conn, agent_id, &subject_entity_ids)?;

// Reader side (entity_resolver, in pending query):
// LEFT JOIN kg_invalidated_no_match inv
//     ON inv.subject_entity_id = e.id AND inv.agent_id = ?1
// ... (inv.subject_entity_id IS NOT NULL) AS was_invalidated

// Cleanup side (entity_resolver, in write_log after UPSERT):
kg_schema::clear_invalidated_no_match(&conn, agent_id, subject_entity_id)?;
```
