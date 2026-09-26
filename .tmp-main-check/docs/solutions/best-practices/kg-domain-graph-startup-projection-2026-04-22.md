---
title: "KG domain graph as startup projection with sole-writer contract"
date: 2026-04-22
category: best-practices
module: kg
problem_type: best_practice
component: database
severity: medium
applies_when:
  - Building a graph or index layer that projects data from authoritative registries
  - Populating KG entities/relationships from skill manifests, tool registries, or configs
  - Designing idempotent startup-time population of derived data
tags:
  - knowledge-graph
  - domain-graph
  - startup-projection
  - sole-writer
  - upsert-idempotency
  - sqlite
---

# KG domain graph as startup projection with sole-writer contract

## Context

The Knowledge Graph domain layer (#687) needed to populate `kg_entities` and `kg_relationships` from four authoritative sources at server startup: `SkillRegistry`, `ToolRegistry`, `McpManager`, and agent configs. The design challenge was keeping the projection idempotent, preserving entity rowids across rebuilds (so FK references from the subject layer survive), and preventing dual-write bugs.

## Guidance

**Use a sole-writer pattern for each entity key namespace.** The domain graph builder is the exclusive writer of `skill:*`, `tool:*`, `agent:*`, and `problem_type:*` entity keys. No other code path writes these namespaces. This prevents the dual-write anti-pattern documented in `docs/solutions/logic-errors/a2a-dual-write-duplicate-rows.md`.

**UPSERT entities, DELETE+INSERT relationships.** Entities use `INSERT ... ON CONFLICT(entity_key) DO UPDATE` to preserve the `id` (INTEGER PRIMARY KEY AUTOINCREMENT) rowid. This is critical because `kg_subject_resolutions.domain_entity_id` FK-references `kg_entities.id`. If you accidentally DELETE+INSERT instead of UPSERT, the rowid changes and all FK references break silently. Relationships have no downstream FK dependents, so DELETE-all-then-INSERT per rebuild is simpler and safe.

**Collect existing keys before UPSERT for accurate add/update counts.** You cannot distinguish insert-vs-update from `changes()` alone with SQLite UPSERT — it always returns 1. Snapshot existing entity keys via `SELECT entity_key FROM kg_entities WHERE type IN (...)` before UPSERTing, then compare against the snapshot to count adds vs updates.

**Scope DELETE operations by entity type, not just relationship type.** The stale-entity pruning DELETE uses a type filter (`WHERE type IN ('skill', 'tool', 'agent', 'problem_type')`) sourced from `KG_DOMAIN_ENTITY_TYPES` in `kg_schema.rs`. This is a sole-writer enforcement at the SQL level — a future fifth entity type added by another subsystem won't be silently pruned.

**Fail open on rebuild errors.** Rebuild failures log `warn!` and the server continues to boot. KG queries return stale or empty results until the next successful rebuild. This matches the "indexing is best-effort" policy.

## Why This Matters

Rowid stability is the subtle invariant. If entity rowids drift across rebuilds, every `kg_subject_resolutions.domain_entity_id` reference becomes orphaned. The UPSERT-by-entity_key strategy prevents this, and the `rebuild_preserves_resolution_entity_links` test guards against regression.

The sole-writer contract prevents the exact stale-prose failure class the KG is intended to eliminate — just projected onto graph edges instead of text. Without it, two writers can silently diverge and neither knows the graph is inconsistent.

## When to Apply

- Building any graph or index layer that projects from authoritative sources
- Any startup-time population of derived/cached data in SQLite
- When downstream FK references must survive idempotent rebuilds
- When multiple subsystems may write to the same table in the future

## Examples

Entity UPSERT preserving rowid:
```sql
INSERT INTO kg_entities (entity_key, type, name, properties_json, created_at, updated_at)
VALUES (?1, ?2, ?3, ?4, ?5, ?5)
ON CONFLICT(entity_key) DO UPDATE SET
  name = excluded.name,
  properties_json = excluded.properties_json,
  updated_at = ?5
```

Type-scoped stale entity pruning:
```sql
DELETE FROM kg_entities
WHERE type IN ('skill', 'tool', 'agent', 'problem_type')
  AND entity_key NOT IN (... desired keys ...)
```

## Related

- [`../workflow-issues/kg-milestone-14-autonomous-execution-retrospective-2026-04-22.md`](../workflow-issues/kg-milestone-14-autonomous-execution-retrospective-2026-04-22.md) — KG milestone retrospective (Socratic planning, dispatcher-side failures, follow-up tickets).
- `docs/solutions/logic-errors/a2a-dual-write-duplicate-rows.md` — dual-write anti-pattern
- `docs/solutions/database-issues/sql-column-mismatch-trace-detail-view.md` — column constants
- `docs/solutions/database-issues/trace-id-as-observability-join-key.md` — trace_id pattern
- `docs/architecture/kg-implementation-conventions.md` — cross-cutting KG conventions
- `docs/architecture/kg-id-convention.md` — entity key format
- [mika#687](https://github.com/senara-solutions/mika/issues/687)
