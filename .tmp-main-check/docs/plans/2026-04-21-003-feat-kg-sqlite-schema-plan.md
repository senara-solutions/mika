---
title: "feat: KG SQLite schema — entities, relationships, chunks, subject tables"
type: feat
status: active
date: 2026-04-21
---

# KG SQLite schema — entities, relationships, chunks, subject tables

## Overview

Add the SQLite schema foundation for Mika's Knowledge Graph (milestone mika#14). This ticket (mika#686) is the root of six dependent tickets (#687 domain builder, #688 query tool, #689 lexical ingestion, #690 subject extraction, #691 entity resolution, #692 self-knowledge upgrade). No population logic and no query tool in this ticket — those are separate. This plan defines the tables, indexes, ID scheme, migration, and the ingestion write contract. Output is a migrated schema (v24 → v25), a documented ID convention, and a documented write helper contract that #687, #689, and #690 will consume.

## Problem Frame

Agents today store "what they know" as prose in core memory and structured facts (Layer 1/2 memory). The prose drifts — a `current_priorities` sentence written at time T says mika#677 is pending, but at time T+1 the PR has merged; the prose doesn't self-update. Every bug filed against mika-dev in the last 48 hours (#693 hallucinated UUID, #695 duplicate PR review, #696 webhook fabrication, this morning's fabricated cancellation rationale) is downstream of the same root cause: *LLM-state-in-text that drifts from authoritative state, retrieved via semantic search over the drifted text*.

The KG replaces that substrate. The agent queries a structured graph where nodes are typed entities (Skills, Tools, Agents, ProblemTypes) and edges are typed relationships (`PROVIDES`, `DEPENDS_ON`, `SOLVED_BY`, etc.). The graph has three layers (see Key Technical Decisions):

- **Domain graph** — deterministic, imported from skill manifests and the tool registry. One shared truth per container.
- **Lexical graph** — chunked documents (solution docs, compound docs) with embeddings, linked to domain entities. Per-agent scope (chunks come from docs the agent has ingested).
- **Subject graph** — LLM-extracted entities and fact triples from agent-authored prose. Per-agent scope.

This ticket lands only the schema — the foundation the other six tickets build on. Schema shape decisions are hardest to reverse, so they land alone and get scrutiny before population and query tickets start.

## Requirements Trace

- R1. Three SQLite tables for domain/lexical/subject layers, plus a subject→domain resolution edge table.
- R2. Reuse existing FTS5 + sqlite-vec infrastructure (`search_content` in `crates/mika-agent/src/db.rs`) — do not fork the hybrid search pipeline.
- R3. Deterministic IDs for domain entities (human-readable `skill:self-dev` style), efficient INTEGER PKs for joins.
- R4. Schema migration v24 → v25 with forward-test coverage (no precedent for migration forward-tests in the repo — build harness as part of this ticket).
- R5. ID convention document so #687, #690, #691 agree on how node identifiers are constructed.
- R6. Write-contract documentation — how `kg_chunks` composes with `search_content` via transactional double-write.

## Scope Boundaries

- Tables, indexes, migration, ID convention, write-contract docs, migration test harness.
- No population logic (deferred).
- No query tool (deferred).
- No self-knowledge upgrade (deferred).

### Deferred to Separate Tasks

- Domain graph import from skill manifests + tool registry: **mika#687**.
- Lexical graph ingestion (chunk + embed docs): **mika#689**.
- Subject graph extraction (LLM-based NER + fact triples): **mika#690**.
- Entity resolution (subject → domain): **mika#691**.
- KG query tool (`query_knowledge_graph`): **mika#688**.
- Self-knowledge upgrade: **mika#692**.

## Context & Research

### Relevant Code and Patterns

- **Migration pattern** — `crates/mika-agent/src/db.rs:675-781` (`fn migrate`). Linear chain of `migrate_vN_to_vN+1` calls. v22→v23 (`db.rs:2553`) and v23→v24 (`db.rs:2573`) are the canonical templates: `column_exists()` idempotent guards, `BEGIN IMMEDIATE`/`COMMIT` wrappers. **Virtual tables run outside transactions** (`db.rs:1227-1233`). New tables must also be added to `migrate_v1()` (`db.rs:783`) for clean-slate startup.
- **FTS5 + sqlite-vec pipeline** — `search_content` table (`db.rs:1049-1059`) + `fts_search` virtual table (external content, no triggers — explicit writes via `index_content()` at `db.rs:6107`) + `vec_search` virtual table (`USING vec0(embedding float[512])`). Hybrid search via RRF with K=60 at `db.rs:6286-6340`. OpenAI `text-embedding-3-small` with 512 dims via `crates/mika-common/src/embedding.rs`.
- **Typed-prefix ID precedent** — `audit_events.target_key` uses `<type>:<id>` format (`person:42`, `task:<uuid>`, `skill:<name>`). Not a PK today — used as a lookup key. We extend this convention to `kg_entities.entity_key` (UNIQUE, not PK).
- **JSON metadata convention** — `metadata TEXT` columns (stored as JSON string), queried via `json_extract(col, '$.path')`. Examples: `tasks.metadata` (`db.rs:903`), `sessions.metadata` (`db.rs:941`). No generated columns or JSON indexes in the codebase — KG follows the same precedent for `properties_json`.
- **Existing source_type dispatch pattern** — `tools/search_memory.rs:12` defines `const INDEXED_CATEGORIES: &[&str]`. Callers receive `source_type` back from `hybrid_search` and format results uniformly (`[{type}] {content}`). No `resolve_source()` helper exists; the pattern is "caller switches on source_type when source-specific fields are needed." KG follows this pattern.
- **Async DB plumbing** — `AsyncDatabase` serializes writes via dedicated OS thread + `sync_channel(512)` mpsc (`crates/mika-agent/src/async_db.rs`). All KG writes go through `AsyncDatabase` closures, not raw `Database::open()`.

### Institutional Learnings

- **ADR-003 (Layer 3 hybrid vector search)** — `docs/adr/003-layer3-hybrid-vector-search.md`. Sets conventions being reused: 512-dim embeddings, RRF k=60, FTS5 input sanitized by double-quoting, indexing is best-effort (never fails tool responses). Embedding dim is globally locked at 512 in this container.
- **Single-container DB consolidation** — `docs/solutions/database-issues/consolidate-per-agent-team-dbs-into-single-container-db.md`. Every agent-scoped table carries `agent_id TEXT NOT NULL REFERENCES agents(id) ON DELETE CASCADE`. One DB per container. Row-level isolation, not file-level.
- **Startup embedding backfill** — `docs/solutions/logic-errors/startup-backfill-skips-embedding-generation.md`. Backfill is idempotent by `embedding_json IS NULL`, batch size 100. Any new `source_type` inherits this path automatically.
- **trace_id as correlation key** — `docs/solutions/database-issues/trace-id-as-observability-join-key.md`. New mutation tables should include `trace_id TEXT` for cross-subsystem observability.
- **ISO 8601 TEXT timestamps** — `docs/solutions/database-issues/iso8601-timestamp-migration.md`. `TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now'))`. Never use `datetime('now')` (space separator breaks comparisons).
- **SELECT \* migration ban / column constants** — `docs/solutions/database-issues/sql-column-mismatch-trace-detail-view.md`. Define `KG_ENTITY_COLUMNS`, `KG_RELATIONSHIP_COLUMNS`, etc. constants. No `SELECT *` in migrations or row mappers.
- **Dual-write anti-pattern** — `docs/solutions/logic-errors/a2a-dual-write-duplicate-rows.md`. Designate one persistence owner per entity kind (domain ingestor owns `skill:*` nodes; subject ingestor owns subject entities; resolver owns resolution edges). Document explicitly.

### External References

None needed — this is a schema design using existing in-repo FTS5/sqlite-vec infrastructure. The DeepLearning.ai course background that informed ticket #686 does not require external reference material at the schema level.

## Key Technical Decisions

### D1. Agent scoping is per-layer, not uniform

Resolved during planning. Domain tables (`kg_entities`, `kg_relationships`) have **no `agent_id`**. Lexical tables (`kg_chunks`) and subject tables (`kg_subject_entities`, `kg_subject_resolutions`) have `agent_id TEXT NOT NULL REFERENCES agents(id) ON DELETE CASCADE`.

Rationale: the `agent_id` column is a statement about what varies. Skills and tools are defined once in shared code — per-agent divergence is a YAGNI violation. Subject/lexical content is an agent's own ingested prose — scoping belongs there. Copying `agent_id` into domain tables creates semantic noise (filter is a no-op, future readers wonder if per-agent domain divergence is real).

If per-agent skill enable/disable state is ever needed (already exists via `skill_overrides`), that is a separate state table keyed on `(agent_id, skill_id)`, not duplicated domain nodes.

### D2. Composed indexing, not parallel indexing

Resolved during planning. `kg_chunks` holds chunk **structural metadata** (entity link, seq_id, source doc, agent_id). The **indexable text** flows through the existing `search_content` + `fts_search` + `vec_search` pipeline via `index_content(source_type="kg_chunk", source_id=<kg_chunks.rowid>, text=<chunk_text>)`.

Rationale: `search_content` owns indexability (one shared funnel). `kg_chunks` owns chunk identity within the lexical graph (one kind of fact). Collapsing them into one table violates SRP — chunk-specific columns would be nullable for existing source types (semantic noise) or forced into JSON (loses query clarity). Parallel FTS/vec tables would fork the embedding pipeline, RRF fusion, and hybrid_search entry point — every consumer that searches across both Layer 2 memory and KG chunks would re-implement fusion at a higher layer. The composition wins on every axis that matters for this codebase.

The `kg_chunks` write path is transactional: insert into `kg_chunks`, then call `index_content()` within the same transaction. If indexing fails, the chunk row rolls back. No orphan rows, no orphan indexes.

### D3. Entity PK is INTEGER rowid; entity_key is a UNIQUE derived TEXT column

Resolved during planning. `kg_entities.id INTEGER PRIMARY KEY AUTOINCREMENT`, `kg_entities.entity_key TEXT UNIQUE NOT NULL`, with a CHECK constraint: `CHECK (entity_key = type || ':' || name)`.

Rationale: the PK's job is stable, efficient joins. Typed-prefix TEXT PKs (the ticket's original proposal) conflate "external identifier for humans and manifests" with "internal join key." That conflation hurts in three specific ways:

- **Rename pain**: `skill:self-dev` → `skill:self-development` cascades across every `kg_relationships` row and every cross-layer edge. INTEGER PK makes rename a single `entity_key` column update.
- **vec0 compatibility**: vec0 virtual tables key on INTEGER rowids. If entity-level embeddings ever become attractive (likely for #691 entity resolution), a TEXT PK forces a parallel mapping table.
- **Pattern consistency**: the rest of the codebase uses INTEGER rowid PKs. "Breaks from precedent for a real reason" is how drift starts; the real reason (encoding type in the PK) is solved just as well by a `type` column with an index.

Cross-layer edges (subject → domain FK, relationship from/to) use INTEGER rowid for joins. `entity_key` is the external-facing identifier for manifests, logs, and user/LLM queries.

### D4. Relationships are directed; no FTS5 or embedding on relationships

Resolved during planning. `kg_relationships` has `from_entity_id INTEGER` and `to_entity_id INTEGER` (not symmetric pair). Queries for inverse traversal use an index on `(to_entity_id, type)`. No FTS5 or embedding on relationship properties — YAGNI; if ever needed, a future schema version adds it.

### D5. Properties as JSON, not typed columns

Resolved during planning. `kg_entities.properties_json TEXT` and `kg_relationships.properties_json TEXT`. Queried via `json_extract(properties_json, '$.path')`. Matches the codebase convention for metadata columns.

Rationale: typed columns per node kind would require separate tables per type (one for Skills, one for Tools, etc.), breaking the uniform entity/relationship model that makes recursive-CTE traversal simple. JSON properties are read-mostly in hot paths (traversal filters on type + name, not on arbitrary properties), so the json_extract cost is acceptable. If a specific property becomes hot (e.g., `always_on` on skills), a future migration can add a generated column + index.

### D6. Trace_id on all per-agent mutation tables

Resolved during planning. `kg_chunks`, `kg_subject_entities`, and `kg_subject_resolutions` all carry `trace_id TEXT` columns. `kg_entities` and `kg_relationships` are populated by deterministic startup code (not agent turns), so no trace_id.

Rationale: the `trace-id-as-observability-join-key.md` institutional learning argues for putting trace_id on every mutation table precisely to avoid forcing cross-table joins through sessions/tasks during debugging. Columns are cheap and the precedent is load-bearing. Earlier draft tried to externalize trace_id for `kg_chunks` ("ingestor carries it"); that was wrong — chunk ingestion can happen mid-turn, mid-callback, or during startup backfill, each with different trace correlation. Putting trace_id on the row handles all cases uniformly.

### D7. kg_chunks has a UNIQUE constraint on `(agent_id, source_doc_path, seq_id)`

Resolved during planning. The UNIQUE constraint is part of the v25 schema. Re-ingesting the same chunk position from the same doc for the same agent is treated as an upsert, not a duplicate insert.

Rationale: the uniqueness question is a schema concern, not an ingestor concern. Deferring it to #689 is a category error — if #689 decides "yes, upsert by `(agent_id, source_doc_path, seq_id)`" it costs a v25→v26 migration because v25 already shipped without the constraint. Adding it defensively now is cheap and matches the likely ingestor behavior. If #689 instead wants "each ingestion attempt creates a new row," the UNIQUE constraint forces an explicit `INSERT OR REPLACE` pattern at the ingestor — still workable. The cost of getting this wrong later (v26 migration + backfill) is much higher than the cost of adding the constraint now.

### D9. kg_chunks has no direct entity_id column — linkage goes through subject→resolution

Resolved during #689 planning (2026-04-21). Original v25 sketch included `kg_chunks.entity_id INTEGER REFERENCES kg_entities(id) ON DELETE SET NULL` as a direct chunk→domain linkage. **Removed.** #689's plan established that `entity_id` has no writer at any point in the pipeline — #689 doesn't infer domain entities from chunks (no inference at lexical layer), and #690/#691 link via the subject→resolution pipeline (per-agent subject entities, per-agent resolutions pointing at global domain entities). A column with no writer is dead weight that would silently be `NULL` forever.

Rationale: the layers compose through the resolution pipeline, not through direct cross-layer columns. Chunks → subject entities → resolutions → domain entities is the canonical path. Any shortcut (a direct `kg_chunks.entity_id` column) would be a side-channel that drifts from the canonical path and produces inconsistent results (captures only one linkage per chunk when most chunks describe multiple domain entities).

Queries like "chunks about skill X" use the multi-hop JOIN through `kg_subject_entities` + `kg_subject_resolutions`. See #689's plan for the canonical query shape. All joins are on indexed columns; at agent-scoped cardinality, SQLite handles multi-join queries efficiently.

### D10. kg_chunks has a `source_doc_hash TEXT NOT NULL` column for idempotency

Resolved during #689 planning (2026-04-21). The v25 schema adds `source_doc_hash TEXT NOT NULL` to `kg_chunks` to support content-change detection during ingestion.

Hash is **required, not optional**. If nullable, there would be two code paths (hash present → compare; hash absent → re-ingest unconditionally) and the second path invites future "backward compat" silent fallbacks that defeat idempotency. NOT NULL forces the invariant: every chunk row has a source doc hash, period.

Hash is SHA-256 of the source doc content **after normalization** (LF line endings, BOM stripped, per-line trailing whitespace stripped, single trailing newline enforced). This prevents false-positive re-ingestion from cross-platform line-ending differences, BOM insertions, or other cosmetic changes that don't reflect semantic content changes. The normalization rules are documented in #689's plan as the canonical hashing contract.

This column lives in v25 directly (inlined before ship) rather than as a v25→v26 migration — cheaper to fold in now than to migrate later, and it's load-bearing for #689's idempotency contract.

The normalization rules are enforced at the application layer by the chunk ingestor (#689); there is no SQL-level CHECK validating hash format. The SQL constraint is only `NOT NULL` — the ingestor is trusted to compute the hash correctly. Code that bypasses the ingestor and inserts raw `kg_chunks` rows is responsible for computing the normalized SHA-256 hash itself.

### D8. Terminology: "KG layers" (not "memory layers") in docs

Resolved during planning. The existing "three-layer memory model" (core memory / structured facts / hybrid search) in `CLAUDE.md` is Layer 1/2/3 of agent memory. The KG has its own "three layers" (domain / lexical / subject). These are different concerns. Docs and comments must qualify — e.g., `KG domain layer`, not `domain layer` alone; `memory Layer 3`, not `hybrid layer` alone. Plan docs, module rustdoc, and ID convention doc all use `KG <layer_name>` explicitly to avoid reader confusion.

### D11. `kg_subject_relationships` table for subject-to-subject edges (fact triples)

Resolved during #690 planning (2026-04-21). Subject-to-subject edges (e.g., `ProblemType -[SOLVED_BY]-> SolutionPath`) are per-agent, LLM-extracted, prose-derived. They don't belong in `kg_relationships` (domain, no agent_id per D1). New table mirrors `kg_relationships` but with `agent_id`, `confidence`, and FKs into `kg_subject_entities`.

```sql
CREATE TABLE kg_subject_relationships (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    agent_id TEXT NOT NULL REFERENCES agents(id) ON DELETE CASCADE,
    from_entity_id INTEGER NOT NULL REFERENCES kg_subject_entities(id) ON DELETE CASCADE,
    to_entity_id INTEGER NOT NULL REFERENCES kg_subject_entities(id) ON DELETE CASCADE,
    type TEXT NOT NULL,
    confidence REAL NOT NULL CHECK (confidence >= 0.0 AND confidence <= 1.0),
    properties_json TEXT,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
    trace_id TEXT,
    UNIQUE (agent_id, from_entity_id, to_entity_id, type)
);
CREATE INDEX idx_kg_subj_rel_from ON kg_subject_relationships(agent_id, from_entity_id, type);
CREATE INDEX idx_kg_subj_rel_to ON kg_subject_relationships(agent_id, to_entity_id, type);
CREATE INDEX idx_kg_subj_rel_type ON kg_subject_relationships(agent_id, type);
```

Rationale: SRP at the schema level. `kg_relationships` holds deterministic domain edges; `kg_subject_relationships` holds LLM-extracted per-agent edges. Different sources of truth, different volatility, different scoping, different write contracts. Mixing them in one table conflates all of that and breaks the dual-write anti-pattern (two writers with different ownership). Alternatives rejected: extending `kg_relationships` with optional agent_id (breaks D1, FK targets differ), storing in `properties_json` (not traversable, O(N) reverse lookups).

### D12. `kg_chunk_subjects` join table for entity provenance (chunk → subject entity)

Resolved during #690 planning (2026-04-21). Many-to-many: one chunk yields multiple entities, one entity appears in multiple chunks. Provenance is a first-class relationship needing its own table with its own fields.

```sql
CREATE TABLE kg_chunk_subjects (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    agent_id TEXT NOT NULL REFERENCES agents(id) ON DELETE CASCADE,
    chunk_id INTEGER NOT NULL REFERENCES kg_chunks(id) ON DELETE CASCADE,
    subject_entity_id INTEGER NOT NULL REFERENCES kg_subject_entities(id) ON DELETE CASCADE,
    extraction_trace_id TEXT,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
    UNIQUE (agent_id, chunk_id, subject_entity_id)
);
CREATE INDEX idx_kg_cs_chunk ON kg_chunk_subjects(agent_id, chunk_id);
CREATE INDEX idx_kg_cs_entity ON kg_chunk_subjects(agent_id, subject_entity_id);
CREATE INDEX idx_kg_cs_trace ON kg_chunk_subjects(agent_id, extraction_trace_id);
```

Uses `extraction_trace_id` (not generic `trace_id`) to explicitly name the semantics: which extraction run produced this provenance record. No confidence field — provenance is a fact ("this entity was extracted from this chunk"), not a judgment.

### D13. `kg_chunk_subject_relationships` join table for relationship provenance

Resolved during #690 planning (2026-04-21). Symmetric to D12 but for relationships. Without this, relationship orphaning after doc edits can't distinguish "this relationship is asserted by a surviving doc" from "this relationship's asserting doc was changed."

```sql
CREATE TABLE kg_chunk_subject_relationships (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    agent_id TEXT NOT NULL REFERENCES agents(id) ON DELETE CASCADE,
    chunk_id INTEGER NOT NULL REFERENCES kg_chunks(id) ON DELETE CASCADE,
    subject_relationship_id INTEGER NOT NULL REFERENCES kg_subject_relationships(id) ON DELETE CASCADE,
    extraction_trace_id TEXT,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
    UNIQUE (agent_id, chunk_id, subject_relationship_id)
);
CREATE INDEX idx_kg_csr_chunk ON kg_chunk_subject_relationships(agent_id, chunk_id);
CREATE INDEX idx_kg_csr_rel ON kg_chunk_subject_relationships(agent_id, subject_relationship_id);
```

This completes the subject layer's provenance model.

### D14. `kg_extractions` tracking table for extraction completion state

Resolved during #690 review (2026-04-21). The subject extractor needs to know "which docs have been extracted" authoritatively. Without explicit tracking, zero-entity docs (valid outcome — some docs have no extractable content) look identical to "not yet extracted" docs, causing unnecessary re-extraction every startup.

```sql
CREATE TABLE kg_extractions (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    agent_id TEXT NOT NULL REFERENCES agents(id) ON DELETE CASCADE,
    source_doc_path TEXT NOT NULL,
    extraction_model TEXT NOT NULL,
    entities_extracted INTEGER NOT NULL DEFAULT 0,
    relationships_extracted INTEGER NOT NULL DEFAULT 0,
    extraction_trace_id TEXT,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
    UNIQUE (agent_id, source_doc_path)
);
CREATE INDEX idx_kg_extractions_agent ON kg_extractions(agent_id);
```

Additional benefits beyond fixing the zero-entity bug: model-version invalidation (`DELETE FROM kg_extractions WHERE extraction_model != ?`), structured extraction observability, and explicit "has this doc been extracted?" query.

### D15. `confidence` column on `kg_subject_entities`

Resolved during #690 review (2026-04-21). Extraction-time confidence should be a queryable, constraint-checked column on `kg_subject_entities`, symmetric with `kg_subject_relationships.confidence`. Downstream consumers (#688 query tool, #692 self-knowledge) will filter by confidence threshold — that requires an indexable column, not JSON extraction via `properties_json`.

```sql
-- Amendment to kg_subject_entities:
-- ADD: confidence REAL NOT NULL CHECK (confidence >= 0.0 AND confidence <= 1.0)
```

### D16. `kg_resolutions_log` tracking table for resolution state

Resolved during #691 planning (2026-04-21). Dedicated tracking table for entity resolution attempts, separate from `kg_extractions` (different keys, metadata, invalidation triggers). Records outcome per (agent_id, subject_entity_id) — authoritative "has this been attempted?" and "what was the result?"

```sql
CREATE TABLE kg_resolutions_log (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    agent_id TEXT NOT NULL REFERENCES agents(id) ON DELETE CASCADE,
    subject_entity_id INTEGER NOT NULL REFERENCES kg_subject_entities(id) ON DELETE CASCADE,
    outcome TEXT NOT NULL CHECK (outcome IN (
        'matched_exact', 'matched_llm', 'no_match', 'skipped_discovered_type', 'skipped_no_llm', 'error'
    )),
    resolution_trace_id TEXT NOT NULL,
    source_extraction_trace_id TEXT,
    model TEXT,
    duration_ms INTEGER,
    resolved_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
    UNIQUE (agent_id, subject_entity_id)
);
CREATE INDEX idx_kg_res_log_pending ON kg_resolutions_log(agent_id, outcome);
```

Includes `source_extraction_trace_id` for detecting staleness when re-extraction regenerates an entity. See #691's plan D4 for the pending query and four staleness triggers.

`UNIQUE (agent_id, subject_entity_id)` records latest outcome only; history of previous resolution attempts is not preserved. If re-resolution audit history becomes a requirement, a separate `kg_resolutions_history` table is the natural extension — do not relax this UNIQUE to accommodate history.

The recurring pattern across D11-D16: processing state for each KG pipeline stage (extraction, resolution) belongs in its own tracking table with structured metadata. This convention should be added to `kg-implementation-conventions.md`.

## Open Questions

### Resolved During Planning

- D1: Agent scoping — per-layer.
- D2: Indexing strategy — composition with `search_content`.
- D3: Entity PK — INTEGER rowid with UNIQUE derived entity_key.
- D4: Relationships — directed, no FTS5/embedding.
- D5: Properties — JSON, not typed columns.
- D6: trace_id on per-agent mutation tables.
- D7: kg_chunks UNIQUE constraint on `(agent_id, source_doc_path, seq_id)`.
- D8: Terminology — use `KG <layer>` explicitly.
- D9: kg_chunks has no entity_id column — linkage via subject→resolution (from #689 planning).
- D10: kg_chunks.source_doc_hash TEXT NOT NULL for idempotency (from #689 planning).
- D11: kg_subject_relationships table — subject-to-subject edges (from #690 planning).
- D12: kg_chunk_subjects join table — entity provenance (from #690 planning).
- D13: kg_chunk_subject_relationships join table — relationship provenance (from #690 planning).
- D14: kg_extractions table — extraction tracking (from #690 review).
- D15: kg_subject_entities.confidence column (from #690 review).
- D16: kg_resolutions_log table — resolution tracking (from #691 planning).
- kg_subject_entities uniqueness — `UNIQUE (agent_id, entity_key)` in schema sketch.

### Deferred to Implementation

- Exact index list — the baseline indexes below are directionally right; final count may grow or shrink based on the query shapes emerging in #687–#692. Add partial indexes on JSON properties only if query profiling shows a hot path.
- ProblemType seed list — the initial set of `problem_type:<slug>` nodes (e.g., `problem_type:fabrication`, `problem_type:state_drift`) is a #687 concern, not this ticket's.

## Output Structure

No new directories — all changes land in existing files:

```
crates/mika-agent/src/
├── db.rs                          # migrate_v24_to_v25, table constants, column lists
├── db/
│   └── kg_schema.rs               # NEW: schema constants (column lists, type enums)
└── tests/
    └── migrations/                # NEW: v24→v25 forward test harness
        ├── mod.rs
        └── v24_to_v25.rs

docs/
├── architecture/
│   └── kg-id-convention.md        # NEW: typed-prefix ID scheme + format rules
└── plans/
    └── 2026-04-21-003-feat-kg-sqlite-schema-plan.md  # this file
```

## High-Level Technical Design

> *This illustrates the intended schema and write pipeline, and is directional guidance for review, not implementation specification. The implementing agent should treat it as context, not DDL to reproduce verbatim.*

### Schema sketch

```sql
-- Domain layer (global, no agent_id)
CREATE TABLE kg_entities (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    entity_key TEXT NOT NULL UNIQUE,
    type TEXT NOT NULL,
    name TEXT NOT NULL,
    properties_json TEXT,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
    updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
    CHECK (entity_key = type || ':' || name)
);
CREATE INDEX idx_kg_entities_type ON kg_entities(type);

CREATE TABLE kg_relationships (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    from_entity_id INTEGER NOT NULL REFERENCES kg_entities(id) ON DELETE CASCADE,
    to_entity_id INTEGER NOT NULL REFERENCES kg_entities(id) ON DELETE CASCADE,
    type TEXT NOT NULL,
    properties_json TEXT,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now'))
);
CREATE INDEX idx_kg_rel_from ON kg_relationships(from_entity_id, type);
CREATE INDEX idx_kg_rel_to ON kg_relationships(to_entity_id, type);

-- Lexical layer (per-agent)
CREATE TABLE kg_chunks (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    agent_id TEXT NOT NULL REFERENCES agents(id) ON DELETE CASCADE,
    seq_id INTEGER NOT NULL,
    source_doc_path TEXT NOT NULL,
    source_doc_hash TEXT NOT NULL,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
    trace_id TEXT,
    UNIQUE (agent_id, source_doc_path, seq_id)
    -- text + embedding live in search_content via source_type='kg_chunk', source_id=kg_chunks.id
    -- chunk → domain entity linkage goes through kg_subject_entities → kg_subject_resolutions (see D9, D10)
);
CREATE INDEX idx_kg_chunks_agent_doc ON kg_chunks(agent_id, source_doc_path);

-- Subject layer (per-agent)
CREATE TABLE kg_subject_entities (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    agent_id TEXT NOT NULL REFERENCES agents(id) ON DELETE CASCADE,
    entity_key TEXT NOT NULL,
    type TEXT NOT NULL,
    name TEXT NOT NULL,
    confidence REAL NOT NULL CHECK (confidence >= 0.0 AND confidence <= 1.0),  -- D15
    properties_json TEXT,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
    trace_id TEXT,
    CHECK (entity_key = type || ':' || name),
    UNIQUE (agent_id, entity_key)
);
CREATE INDEX idx_kg_subj_entities_agent_type ON kg_subject_entities(agent_id, type);

CREATE TABLE kg_subject_resolutions (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    agent_id TEXT NOT NULL REFERENCES agents(id) ON DELETE CASCADE,
    subject_entity_id INTEGER NOT NULL REFERENCES kg_subject_entities(id) ON DELETE CASCADE,
    domain_entity_id INTEGER NOT NULL REFERENCES kg_entities(id) ON DELETE CASCADE,
    confidence REAL NOT NULL,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
    trace_id TEXT,
    UNIQUE (agent_id, subject_entity_id, domain_entity_id),
    CHECK (confidence >= 0.0 AND confidence <= 1.0)
);
CREATE INDEX idx_kg_resolutions_agent_subj ON kg_subject_resolutions(agent_id, subject_entity_id);
CREATE INDEX idx_kg_resolutions_agent_dom ON kg_subject_resolutions(agent_id, domain_entity_id);

-- Subject-to-subject edges / fact triples (per-agent, from #690 planning — D11)
CREATE TABLE kg_subject_relationships (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    agent_id TEXT NOT NULL REFERENCES agents(id) ON DELETE CASCADE,
    from_entity_id INTEGER NOT NULL REFERENCES kg_subject_entities(id) ON DELETE CASCADE,
    to_entity_id INTEGER NOT NULL REFERENCES kg_subject_entities(id) ON DELETE CASCADE,
    type TEXT NOT NULL,
    confidence REAL NOT NULL CHECK (confidence >= 0.0 AND confidence <= 1.0),
    properties_json TEXT,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
    trace_id TEXT,
    UNIQUE (agent_id, from_entity_id, to_entity_id, type)
);
CREATE INDEX idx_kg_subj_rel_from ON kg_subject_relationships(agent_id, from_entity_id, type);
CREATE INDEX idx_kg_subj_rel_to ON kg_subject_relationships(agent_id, to_entity_id, type);
CREATE INDEX idx_kg_subj_rel_type ON kg_subject_relationships(agent_id, type);

-- Entity provenance: chunk → subject entity (many-to-many, from #690 planning — D12)
CREATE TABLE kg_chunk_subjects (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    agent_id TEXT NOT NULL REFERENCES agents(id) ON DELETE CASCADE,
    chunk_id INTEGER NOT NULL REFERENCES kg_chunks(id) ON DELETE CASCADE,
    subject_entity_id INTEGER NOT NULL REFERENCES kg_subject_entities(id) ON DELETE CASCADE,
    extraction_trace_id TEXT,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
    UNIQUE (agent_id, chunk_id, subject_entity_id)
);
CREATE INDEX idx_kg_cs_chunk ON kg_chunk_subjects(agent_id, chunk_id);
CREATE INDEX idx_kg_cs_entity ON kg_chunk_subjects(agent_id, subject_entity_id);
CREATE INDEX idx_kg_cs_trace ON kg_chunk_subjects(agent_id, extraction_trace_id);

-- Relationship provenance: chunk → subject relationship (many-to-many, from #690 planning — D13)
CREATE TABLE kg_chunk_subject_relationships (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    agent_id TEXT NOT NULL REFERENCES agents(id) ON DELETE CASCADE,
    chunk_id INTEGER NOT NULL REFERENCES kg_chunks(id) ON DELETE CASCADE,
    subject_relationship_id INTEGER NOT NULL REFERENCES kg_subject_relationships(id) ON DELETE CASCADE,
    extraction_trace_id TEXT,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
    UNIQUE (agent_id, chunk_id, subject_relationship_id)
);
CREATE INDEX idx_kg_csr_chunk ON kg_chunk_subject_relationships(agent_id, chunk_id);
CREATE INDEX idx_kg_csr_rel ON kg_chunk_subject_relationships(agent_id, subject_relationship_id);

-- Extraction tracking (from #690 review — D14)
CREATE TABLE kg_extractions (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    agent_id TEXT NOT NULL REFERENCES agents(id) ON DELETE CASCADE,
    source_doc_path TEXT NOT NULL,
    extraction_model TEXT NOT NULL,
    entities_extracted INTEGER NOT NULL DEFAULT 0,
    relationships_extracted INTEGER NOT NULL DEFAULT 0,
    extraction_trace_id TEXT,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
    UNIQUE (agent_id, source_doc_path)
);
CREATE INDEX idx_kg_extractions_agent ON kg_extractions(agent_id);

-- Resolution tracking (from #691 planning — D16)
CREATE TABLE kg_resolutions_log (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    agent_id TEXT NOT NULL REFERENCES agents(id) ON DELETE CASCADE,
    subject_entity_id INTEGER NOT NULL REFERENCES kg_subject_entities(id) ON DELETE CASCADE,
    outcome TEXT NOT NULL CHECK (outcome IN (
        'matched_exact', 'matched_llm', 'no_match', 'skipped_discovered_type', 'skipped_no_llm', 'error'
    )),
    resolution_trace_id TEXT NOT NULL,
    source_extraction_trace_id TEXT,
    model TEXT,
    duration_ms INTEGER,
    resolved_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
    UNIQUE (agent_id, subject_entity_id)
);
CREATE INDEX idx_kg_res_log_pending ON kg_resolutions_log(agent_id, outcome);
```

Note: `kg_subject_entities.confidence` (D15) is inlined in the CREATE TABLE above.

### Chunk write pipeline

```
ingest_kg_chunk(agent_id, entity_id, seq, doc_path, text)
    BEGIN TRANSACTION
        INSERT INTO kg_chunks(agent_id, entity_id, seq_id, source_doc_path)
            → rowid = R
        index_content(
            agent_id,
            source_type = "kg_chunk",
            source_id = R,
            content = text
        )
            → INSERT INTO search_content + INSERT INTO fts_search
            → embedding queued for backfill (existing path)
    COMMIT
```

### Traversal example (directional — recursive CTE from #688 territory)

```sql
-- "What tools does skill:self-dev provide?" — simple one-hop
SELECT e2.entity_key, e2.name, e2.properties_json
FROM kg_entities e1
JOIN kg_relationships r ON r.from_entity_id = e1.id
JOIN kg_entities e2 ON e2.id = r.to_entity_id
WHERE e1.entity_key = 'skill:self-dev' AND r.type = 'PROVIDES';

-- "Transitive dependencies of skill:self-dev" — recursive CTE
WITH RECURSIVE deps(id) AS (
    SELECT to_entity_id FROM kg_relationships
      WHERE from_entity_id = (SELECT id FROM kg_entities WHERE entity_key = 'skill:self-dev')
        AND type = 'DEPENDS_ON'
    UNION
    SELECT r.to_entity_id FROM kg_relationships r
      JOIN deps d ON r.from_entity_id = d.id
      WHERE r.type = 'DEPENDS_ON'
)
SELECT e.entity_key FROM kg_entities e JOIN deps d ON e.id = d.id;
```

## Implementation Units

- [ ] **Unit 1: Schema constants module**

**Goal:** Centralize KG table names, column lists, type enums, and ID convention constants in one module before any migration or tool code imports them. Matches the `const FOO_COLUMNS` pattern from the SELECT * migration ban learning.

**Requirements:** R1, R3, R5.

**Dependencies:** None.

**Files:**
- Create: `crates/mika-agent/src/db/kg_schema.rs`
- Test: inline `#[cfg(test)]` in the same module

**Approach:**
- Define `KG_ENTITY_COLUMNS`, `KG_RELATIONSHIP_COLUMNS`, `KG_CHUNK_COLUMNS`, `KG_SUBJECT_ENTITY_COLUMNS`, `KG_SUBJECT_RESOLUTION_COLUMNS` constants (comma-separated column lists, no `SELECT *`).
- Define `KG_DOMAIN_ENTITY_TYPES: &[&str] = &["skill", "tool", "agent", "problem_type"]` — seed set; #687 can extend. **This constant is the single source of truth for the reserved type list; `docs/architecture/kg-id-convention.md` (Unit 5) is derived from it.** Add a rustdoc comment on the constant stating this and pointing at the doc path, so any future type addition updates both.
- Define `KG_CHUNK_SOURCE_TYPE: &str = "kg_chunk"` — the `source_type` discriminator used with `search_content`.
- Define `fn format_entity_key(kind: &str, name: &str) -> String` helper that returns `format!("{}:{}", kind, name)` and enforces the same format used by the CHECK constraint. Callers use this helper so the PK derivation rule has a single home.

**Patterns to follow:**
- `const VALID_TASK_TYPES: &[&str]` in `db.rs` — enum-as-string-constant precedent.
- `SESSION_MESSAGE_COLUMNS` style column-list constants from `sql-column-mismatch-trace-detail-view.md`.

**Test scenarios:**
- Happy path: `format_entity_key("skill", "self-dev")` returns `"skill:self-dev"`.
- Edge case: `format_entity_key` with empty name — document behavior (empty `name` is not a valid entity; callers must validate upstream, but the helper itself shouldn't silently produce `"skill:"`). Decision: the helper does no validation; callers responsible. Assert the concatenation works for the empty case and leave semantic validation to Unit 2's constraints.

**Verification:**
- Module compiles cleanly.
- Constants match the migration SQL in Unit 2 exactly (column lists, types, source_type string).

---

- [ ] **Unit 2: v24 → v25 migration (ships paired with Unit 3)**

**Goal:** Add the five KG tables and their indexes to the schema via a new `migrate_v24_to_v25` function. Also add them to `migrate_v1` so fresh installs get the same state. Bump `CURRENT_SCHEMA_VERSION` to 25.

> **Pairing note:** Unit 2 and Unit 3 (forward-test harness) are effectively one deliverable. Unit 3's `v1_and_incremental_converge` test is the only mechanism that catches drift between the `migrate_v1` clean-slate path and the `migrate_v24_to_v25` incremental path. Do NOT land Unit 2 alone and defer Unit 3 — an implementer who does that has no way to know Unit 2 is correct.

**Requirements:** R1, R4.

**Dependencies:** Unit 1.

**Files:**
- Modify: `crates/mika-agent/src/db.rs`

**Approach:**
- Add `fn migrate_v24_to_v25(tx: &Transaction)` following the v22→v23 / v23→v24 template pattern. Wrap in `BEGIN IMMEDIATE` / `COMMIT` (note: no virtual tables in this migration, so no out-of-transaction statements needed).
- Each `CREATE TABLE IF NOT EXISTS` call uses the constants from Unit 1 for column lists.
- Create indexes listed in the High-Level Technical Design section.
- After table creation, append `INSERT INTO schema_version (version) VALUES (25)`.
- Mirror the CREATE TABLE + CREATE INDEX statements into `migrate_v1` so fresh DB installs reach v25 directly.
- Bump `const CURRENT_SCHEMA_VERSION: i64 = 25` at `db.rs:25`.
- Confirm the automatic backup path at `db.rs:619-637` fires correctly for this migration (should work unchanged — it's version-agnostic).

**Patterns to follow:**
- `migrate_v23_to_v24` at `db.rs:2573` (ALTER TABLE pattern — KG uses CREATE TABLE but the transaction wrapping is the same).
- `migrate_v21_to_v22` at (search `db.rs`) for a prior CREATE TABLE migration, if one exists; otherwise the v1 clean-slate path is the nearest template.

**Test scenarios:**
- Happy path: migration succeeds on a v24 DB; `schema_version` row with version=25 appears; all ten new KG tables exist with correct columns.
- Edge case: migration is idempotent — re-running does not error (relies on `IF NOT EXISTS` + the `version < N` guard in `migrate()`).
- Integration: fresh `open_in_memory()` reaches v25 via `migrate_v1` and has the same schema as a v24 DB that ran `migrate_v24_to_v25`.
- Error path: CHECK constraint on `kg_entities` rejects a row where `entity_key != type || ':' || name`.
- Error path: FK cascade on `kg_chunks` deletes rows when the agent is deleted.
- Error path: FK cascade on `kg_relationships` deletes rows when either endpoint entity is deleted.

**Verification:**
- `db.rs` compiles; `cargo test -p mika-agent` passes.
- `test_schema_version_is_current` (`db.rs:8989`) still passes with the bumped CURRENT_SCHEMA_VERSION.
- All ten new KG tables visible via `.schema` in a SQLite shell.

---

- [ ] **Unit 3: Migration forward-test harness**

**Goal:** Build a reusable harness for "start at v24, migrate to v25, assert post-state." The codebase has no existing migration forward-test infrastructure; this unit creates the pattern and applies it to v24→v25.

**Requirements:** R4.

**Dependencies:** Unit 2.

**Files:**
- Create: `crates/mika-agent/src/tests/migrations/mod.rs`
- Create: `crates/mika-agent/src/tests/migrations/v24_to_v25.rs`
- Modify: `crates/mika-agent/src/lib.rs` (add `#[cfg(test)] mod tests;` or similar wiring — follow existing test-module layout)

**Approach:**
- Helper function `fn seed_v24_schema(conn: &Connection)` — executes the raw SQL for the v24 schema. The simplest extraction is to capture the CREATE TABLE statements that `migrate_v1` produces *through* v24 (i.e., every table that existed pre-v25). This helper is the new piece — define it explicitly so future migrations can reuse it.
- Helper function `fn snapshot_schema(conn: &Connection) -> SchemaSnapshot` — builds a structural fingerprint of every table and index via PRAGMA introspection:
  - For each table in `sqlite_master` where `type='table'`: capture `PRAGMA table_info(<name>)` (column name, declared type, NOT NULL, default value, primary key flag) and `PRAGMA foreign_key_list(<name>)` (referenced table, from/to columns, on_delete action).
  - For each index: capture `PRAGMA index_list(<table>)` + `PRAGMA index_info(<index>)` (unique flag, indexed columns in order).
  - Normalize into a deterministic struct (sorted column lists, sorted table lists) so equality comparison is stable across formatting.
- **Rationale for PRAGMA over `sqlite_master` text comparison:** `sqlite_master.sql` stores the literal CREATE TABLE DDL including whitespace and column ordering exactly as written. `migrate_v1` and `migrate_v24_to_v25` write the same schema in different source locations and will differ by cosmetic whitespace even when semantically identical. Byte-level text comparison produces brittle false-negatives. PRAGMA introspection tests what actually matters — columns, types, constraints, indexes — and is robust to formatting drift.
- Test `v24_to_v25_migration_adds_tables`: seed v24, run `migrate_v24_to_v25`, snapshot schema, assert all ten new KG tables exist with expected columns, indexes, FK actions, CHECK constraints.
- Test `v24_to_v25_migration_is_idempotent`: seed v24, run migration twice, assert no error and schema_version row count stays at 1 for version 25.
- Test `v1_and_incremental_converge`: open two in-memory DBs — one via `migrate_v1()` (reaches v25 directly), one by seeding v24 then running `migrate_v24_to_v25`. Snapshot both schemas via `snapshot_schema()` and assert structural equality. Any drift between the two paths fails the test with a diff showing which column/index/constraint differs.

**Patterns to follow:**
- `fn db() -> Database { Database::open_in_memory().unwrap() }` at `db.rs:7241` — in-memory DB test helper.
- `test_fts_search_agent_isolation` at `db.rs:7854` — structure of a write-and-assert DB test.

**Test scenarios:**
- Happy path: `v24_to_v25_migration_adds_tables` passes on a fresh v24 seed.
- Idempotency: re-running the migration produces no error and does not duplicate `schema_version` rows.
- Convergence: `migrate_v1` and `seed_v24 + migrate_v24_to_v25` produce equivalent schemas.
- Integration: the `test_schema_version_is_current` assertion at `db.rs:8989` still passes after CURRENT_SCHEMA_VERSION bump.

**Verification:**
- `cargo test -p mika-agent migrations` runs the three new tests.
- All three tests pass on both clean and re-run invocations.

---

- [ ] **Unit 4: KG chunk write contract documentation**

**Goal:** Document the `kg_chunks` → `search_content` composed write contract so that #689 (lexical ingestion) has an unambiguous reference. This is documentation-only — the actual helper function lands in #689.

**Requirements:** R2, R6.

**Dependencies:** Units 1 and 2.

**Files:**
- Create: section in `crates/mika-agent/src/db/kg_schema.rs` (module-level rustdoc `//!` comment) covering the write contract.
- Modify: `docs/adr/003-layer3-hybrid-vector-search.md` — append a short "KG composition" subsection noting that `kg_chunk` is a registered `source_type` using the same pipeline.

**Approach:**
- Rustdoc at the top of `kg_schema.rs` describes: the four-step transactional write (BEGIN, INSERT kg_chunks, INSERT search_content via `index_content`, COMMIT), the rollback semantics on embedding-call failure, the idempotency rule (the UNIQUE constraint on `(agent_id, source_doc_path, seq_id)` is in the v25 schema per D7; ingestors use `INSERT OR REPLACE` or explicit UPSERT against it).
- ADR-003 update is one paragraph: "KG chunks plug into this pipeline as `source_type='kg_chunk'`. The kg_chunks table (schema v25) holds structural metadata; text and embeddings flow through `search_content` and the existing backfill."

**Patterns to follow:**
- Module rustdoc style: look at `crates/mika-agent/src/skills/context.rs` top-of-file docstring for example of "contract doc for a pattern this module enforces."

**Test scenarios:**
Test expectation: none — this unit is documentation. The behavioral guarantees it describes are tested as part of Unit 2 (CHECK/FK constraints) and will be tested in #689 (transactional write).

**Verification:**
- Documentation compiles (rustdoc passes).
- ADR-003 renders cleanly in Markdown.
- A reviewer reading only the new docs can determine: which tables to write, in what order, within what transaction, with what source_type value.

---

- [ ] **Unit 5: KG ID convention document**

**Goal:** Publish the typed-prefix ID scheme so #687, #690, #691 agree on how node identifiers are constructed. Precedent exists in `audit_events.target_key` but has never been formalized.

**Requirements:** R3, R5.

**Dependencies:** Unit 1 (for the constants this doc references).

**Files:**
- Create: `docs/architecture/kg-id-convention.md`

**Approach:**
- Format: `<type>:<name>`, lowercase-kebab-case for `<name>`.
- Reserved types (v25 initial set): `skill`, `tool`, `agent`, `problem_type`.
- Rules:
  - `<type>` must be in `KG_DOMAIN_ENTITY_TYPES` (from `kg_schema.rs`).
  - `<name>` must be non-empty, lowercase, `[a-z0-9_-]+`. No colons in `<name>` (would confuse parsing).
  - Deterministic derivation: `skill:<skill.toml name>`, `tool:<registered tool name>`, `agent:<agent name from config>`, `problem_type:<slug>`.
- Subject-layer entities: per-agent scope, same format. The LLM extractor chooses the type (may be a domain type if the mention resolves, or a subject-only type like `failure_mode`, `solution_path`).
- Cross-reference: link to `kg_schema.rs::format_entity_key` as the canonical helper.

**Patterns to follow:**
- `docs/adr/003-layer3-hybrid-vector-search.md` — tone and structure of an architectural decision doc.

**Test scenarios:**
Test expectation: none — this unit is documentation. ID format compliance is enforced by the CHECK constraint in Unit 2 + validation helpers in Unit 1.

**Verification:**
- Doc renders cleanly.
- #687 / #690 planners can reference it as the source of truth for ID construction.

## System-Wide Impact

- **Interaction graph:** Schema-only change. No callbacks, middleware, or observers touched. The `search_content` / `fts_search` / `vec_search` pipeline sees a new `source_type` value but no code changes in this ticket — the pipeline is already polymorphic on source_type.
- **Error propagation:** FK cascade on `kg_chunks.agent_id` and `kg_subject_entities.agent_id` ensures agent deletion cleans up KG state. CHECK constraint on `kg_entities.entity_key` rejects malformed rows at write time (no silent drift).
- **State lifecycle risks:** The transactional write contract (Unit 4) must be respected by #689 to avoid `kg_chunks` rows without corresponding `search_content` entries (or vice versa). Documented but not enforced structurally in this ticket.
- **API surface parity:** None — no tools or endpoints change in this ticket.
- **Integration coverage:** Migration forward-test harness (Unit 3) exercises the end-to-end migration path. Transactional write contract is tested in #689.
- **Unchanged invariants:** `search_content` / `fts_search` / `vec_search` shape and behavior are unchanged. Layer 2 memory tables (people, commitments, preferences, events) are unchanged. `hybrid_search` at `db.rs:6286` is unchanged. Existing `source_type` values (person, commitment, preference, event) continue to work exactly as today.

## Risks & Dependencies

| Risk | Mitigation |
|------|------------|
| Migration silently corrupts existing data | Auto-backup at `db.rs:619-637` runs before every migration. Forward-test harness in Unit 3 validates migration shape. `schema_version` is append-only, so re-running is safe. |
| #687 / #689 / #690 discover the schema shape is wrong once population begins | Ship schema alone, get it reviewed, then start population tickets. If a schema change is needed later, it's a v25 → v26 migration — not a re-run of v24 → v25. |
| Virtual table creation semantics differ from regular tables | Migration does not create virtual tables — KG chunks reuse existing `fts_search` / `vec_search`. No new virtual tables in v25. |
| CHECK constraint on `entity_key` rejects valid data | Constraint derives from `type` and `name` deterministically. Unit 1's `format_entity_key` helper is the single writer. Tests in Unit 2 confirm the constraint matches the helper. |
| Per-agent subject entity key collision with a domain entity key | `kg_subject_entities` is a separate table — no PK collision possible. Queries that union domain + subject must disambiguate by source table, not by key alone. Documented in Unit 5 ID convention. |
| JSON property queries become a hot path before indexes exist | Deferred — no JSON indexing today. If profiling shows a hot path post-#687, add generated columns + indexes in a future migration. |

## Documentation / Operational Notes

- ID convention doc (`docs/architecture/kg-id-convention.md`) is load-bearing for #687, #690, #691. Land it with this ticket.
- ADR-003 gets a short update noting KG composition (Unit 4).
- No rollout considerations — schema changes are container-local, apply on next mika-spirit startup via the migration chain.
- Monitoring: `audit_events` already captures schema-version transitions; the v24 → v25 migration fires one event. No new metrics needed in this ticket.

## Sources & References

- **Origin ticket:** [mika#686](https://github.com/senara-solutions/mika/issues/686)
- **Milestone:** [mika milestone#14 "Knowledge Graph"](https://github.com/senara-solutions/mika/milestone/14)
- **Dependent tickets:** #687 (domain builder), #688 (query tool), #689 (lexical ingestion), #690 (subject extraction), #691 (entity resolution), #692 (self-knowledge upgrade)
- **Related code:**
  - `crates/mika-agent/src/db.rs` — migration chain, existing FTS5/vec infrastructure, `hybrid_search`, `index_content`
  - `crates/mika-agent/src/async_db.rs` — async DB plumbing
  - `crates/mika-agent/src/tools/search_memory.rs` — existing source_type dispatch pattern (`INDEXED_CATEGORIES`)
  - `crates/mika-common/src/embedding.rs` — OpenAI 512-dim embedding client
- **ADR:** `docs/adr/003-layer3-hybrid-vector-search.md`
- **Institutional learnings:**
  - `docs/solutions/database-issues/consolidate-per-agent-team-dbs-into-single-container-db.md`
  - `docs/solutions/database-issues/iso8601-timestamp-migration.md`
  - `docs/solutions/database-issues/trace-id-as-observability-join-key.md`
  - `docs/solutions/database-issues/sql-column-mismatch-trace-detail-view.md`
  - `docs/solutions/logic-errors/startup-backfill-skips-embedding-generation.md`
  - `docs/solutions/logic-errors/a2a-dual-write-duplicate-rows.md`
  - `docs/solutions/database-issues/team-graph-persistence-replacing-toml-history.md` — prior "graph-in-SQLite" precedent (parent_id tree, single-parent; KG supersedes with true directed edges)
