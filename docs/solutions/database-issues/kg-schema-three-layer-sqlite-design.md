---
module: mika-agent/db
tags: [kg, schema, migration, sqlite, knowledge-graph]
problem_type: design
date: 2026-04-21
---

# KG Schema: Three-Layer SQLite Design

## Problem

Agents lacked structured self-awareness. Skills, tools, agents, and problem types existed as unstructured prose that drifted from authoritative state. A knowledge graph requires a schema that supports three distinct layers (domain, lexical, subject) within the existing single-container SQLite database.

## Solution

Schema v25 adds 10 tables organized into three KG layers, plus provenance and tracking tables:

### Domain Layer (global, no agent_id)
- `kg_entities` — typed nodes with CHECK-enforced `entity_key = type || ':' || name`
- `kg_relationships` — directed edges with FK cascade on entity deletion

### Lexical Layer (per-agent)
- `kg_chunks` — chunk structural metadata, composes with `search_content` via `source_type='kg_chunk'`

### Subject Layer (per-agent)
- `kg_subject_entities` — LLM-extracted entities with confidence scores
- `kg_subject_resolutions` — subject-to-domain resolution edges
- `kg_subject_relationships` — subject-to-subject fact triples

### Provenance & Tracking
- `kg_chunk_subjects` — chunk-to-subject entity provenance
- `kg_chunk_subject_relationships` — chunk-to-subject relationship provenance
- `kg_extractions` — extraction completion tracking
- `kg_resolutions_log` — resolution outcome tracking

## Key Design Decisions

1. **Agent scoping is per-layer** (D1): Domain tables have no `agent_id` — skills and tools are defined once globally. Subject/lexical tables carry `agent_id` with CASCADE delete.

2. **Composed indexing** (D2): `kg_chunks` holds structural metadata; text and embeddings flow through the existing `search_content` + FTS5 + sqlite-vec pipeline. No parallel indexing infrastructure.

3. **INTEGER PK + UNIQUE entity_key** (D3): Join efficiency via integer rowid; human-readable `type:name` keys via a derived UNIQUE TEXT column with a CHECK constraint.

4. **No direct chunk→entity FK** (D9): Chunk-to-domain linkage goes through the subject→resolution pipeline (`kg_subject_entities` + `kg_subject_resolutions`), not a direct column.

## Testing Pattern: Schema Convergence

The migration forward-test harness validates that `migrate_v1()` (clean-slate) and `migrate_v24_to_v25()` (incremental) produce structurally identical schemas. The approach:

1. Create DB1 via `Database::open_in_memory()` (clean-slate v25)
2. Create DB2 by replaying non-KG DDL from a fresh DB onto a raw connection (simulating v24), then running `migrate_v24_to_v25()` incrementally
3. Snapshot both schemas via PRAGMA introspection (columns, indexes, FKs)
4. Assert structural equality table by table

This pattern catches drift between the two migration paths that text-level DDL comparison would miss due to cosmetic formatting differences.

## Files Changed

- `crates/mika-agent/src/db.rs` — `migrate_v24_to_v25()`, `migrate_v1()` update, 12 tests
- `crates/mika-agent/src/db/kg_schema.rs` — column constants, type enums, write contract docs
- `docs/architecture/kg-id-convention.md` — typed-prefix ID scheme
- `docs/adr/003-layer3-hybrid-vector-search.md` — KG composition note

## References

- Plan: `docs/plans/2026-04-21-003-feat-kg-sqlite-schema-plan.md`
- Issue: mika#686
- Milestone: mika milestone#14 (Knowledge Graph)
- Milestone retrospective: [`../workflow-issues/kg-milestone-14-autonomous-execution-retrospective-2026-04-22.md`](../workflow-issues/kg-milestone-14-autonomous-execution-retrospective-2026-04-22.md) — covers the schema amendments D9–D15 that folded back from #689/#690.
