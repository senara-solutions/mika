---
title: "feat: subject graph extraction — LLM-based NER + fact triples from compound docs"
type: feat
status: active
date: 2026-04-21
---

# Subject graph extraction — LLM-based NER + fact triples from compound docs

## Overview

Populate the subject graph layer of Mika's Knowledge Graph (milestone mika#14, ticket mika#690). For each agent, extract named entities and fact triples from previously-ingested documents (#689's `kg_chunks`) using per-document LLM calls. Produce `kg_subject_entities`, `kg_subject_relationships`, `kg_chunk_subjects`, and `kg_chunk_subject_relationships` rows. Per-agent scope (per #686 D1). No entity resolution in this ticket — linking subject entities to domain entities is #691's concern.

This is the first KG ticket that introduces **non-interactive LLM calls** (per C2 in the conventions doc). All LLM-related conventions from C2 apply: model selection via `MIKA_KG_EXTRACTION_MODEL`, four-category retry taxonomy (C2.2), log-and-skip preserving lexical state (C2.3), `llm_calls` observability rows (C2.4).

## Problem Frame

#689 landed the lexical layer — chunked docs, indexed, searchable via FTS5 and vector. But chunks are unstructured text: "the fabrication bug was caused by state drift in core memory" is a sentence, not a traversable graph edge. An agent asking "what problems are caused by state drift?" must scan chunks by keyword, read the prose, and infer relationships — exactly the LLM-in-the-loop failure mode the KG replaces.

The subject graph makes those implicit relationships explicit. Extraction produces `problem_type:fabrication -[CAUSED_BY]-> problem_type:state_drift -[SOLVED_BY]-> solution_path:pre_persistence_validation` as first-class graph edges, traversable by CTE, queryable by #688's query tool, and surfaceable by #692's self-knowledge upgrade.

## Requirements Trace

- R1. Per-doc LLM extraction producing subject entities and fact triples from previously-ingested chunks.
- R2. Startup extraction runs asynchronously in the background after server readiness (does not block health check).
- R3. Compound-hook extraction runs synchronously inline (~1 doc, <2s).
- R4. Constrained extraction: only approved entity types and approved relationship types.
- R5. Per-chunk provenance for both entities and relationships via join tables.
- R6. Re-extraction on doc change: three-phase capture → reingest → reconcile transactionally.
- R7. Observability per C2.4 (llm_calls rows) and C3.3 (per-doc audit_events, progress logging).

## Scope Boundaries

- Entity extraction, relationship extraction, chunk provenance, re-extraction lifecycle.
- No entity resolution (subject → domain): **mika#691**.
- No query tool: **mika#688**.
- No self-knowledge upgrade: **mika#692**.
- No LLM prompt design in this plan — exact prompt wording deferred to implementation (empirical). This plan specifies the **output schema** the prompt must produce.

### Deferred to Separate Tasks

- Entity resolution (fuzzy matching + LLM disambiguation of subject → domain): **mika#691**.
- KG query tool (`query_knowledge_graph`): **mika#688**.
- Self-knowledge upgrade: **mika#692**.

## Context & Research

### Cross-cutting conventions

This plan cites `docs/architecture/kg-implementation-conventions.md` as the authoritative source for cross-cutting decisions. Sections that apply to #690:

- **C1.1 (async-embedding contract):** Subject entities with text content (e.g., descriptions) that need embedding follow the same async pattern — write rows synchronously, embeddings generated later by backfill.
- **C2 (non-interactive LLM call policy):** All of C2 applies. #690 is the primary consumer of `MIKA_KG_EXTRACTION_MODEL` (C2.1). Retry taxonomy (C2.2), log-and-skip (C2.3), and `llm_calls` observability (C2.4) are all mandatory.
- **C3.3 (observability — subject extraction):** Per-document `audit_events` with `tool_name: "extract_subject_entities"`, progress logging for background runs.

### #686, #687, #689 dependencies

- **#686 schema:** `kg_subject_entities` shape (UNIQUE(agent_id, entity_key), trace_id per D6, CHECK constraint per D3). Schema amendments D11 (`kg_subject_relationships`), D12 (`kg_chunk_subjects`), D13 (`kg_chunk_subject_relationships`) — surfaced during #690 planning and folded back.
- **#687 dependency:** Domain rebuild completes before extraction starts. Extraction does not read `kg_entities` directly — that's #691's concern. But the startup ordering (domain → lexical → subject) is preserved.
- **#689 dependency:** Chunks must exist before extraction runs. Per-doc extraction reads the full doc from disk (not chunk text from DB) but uses chunk boundary info for provenance.

### Relevant Code and Patterns

- **LLM provider infrastructure:** `crates/mika-common/src/llm/mod.rs:305` — `create_provider(spec, max_tokens)` takes a `ModelSpec` and returns `Arc<dyn LlmProvider>`. #690 creates a dedicated provider instance from `MIKA_KG_EXTRACTION_MODEL` env var (per C2.1).
- **LlmProvider trait:** `send_message(&self, request: &LlmRequest) -> Result<LlmResponse, LlmError>`. Request includes system prompt, messages, optional tool definitions. #690 uses a single user message with the doc text + extraction instructions, no tools — structured JSON output extracted from the response text.
- **llm_calls table:** `crates/mika-agent/src/db.rs` — existing `store_llm_call` pattern. #690 writes one row per extraction call (per C2.4).
- **Existing provider kinds:** 11 providers via `ProviderKind` enum. `MIKA_KG_EXTRACTION_MODEL` resolves through the same `Settings` → `ModelSpec` → `create_provider` pipeline.
- **Async spawn pattern:** `tokio::spawn` for background tasks. Server startup uses `tokio::spawn` for embedding backfill already — extraction follows the same pattern.
- **kg_chunks schema:** `UNIQUE(agent_id, source_doc_path, seq_id)`, `source_doc_hash TEXT NOT NULL`. Chunks are keyed by doc path + sequence.

### Institutional Learnings

- `docs/solutions/logic-errors/a2a-dual-write-duplicate-rows.md` — Sole-writer designation: #690's `SubjectExtractor` is the sole writer of subject-typed `entity_key`s in `kg_subject_entities`, all rows in `kg_subject_relationships`, all rows in `kg_chunk_subjects`, and all rows in `kg_chunk_subject_relationships`. No other code path writes these.
- `docs/solutions/database-issues/trace-id-as-observability-join-key.md` — One `trace_id` per extraction invocation (per-agent, per-run). Stamped on all rows written in that invocation. Separate `extraction_trace_id` on provenance join tables (per Vincent's refinement in D12).
- `docs/solutions/database-issues/sql-column-mismatch-trace-detail-view.md` — Column constants for new tables in `kg_schema.rs`.

## Key Technical Decisions

### D1. Per-doc extraction with chunk boundary markers

Resolved during planning. The LLM sees the **full document text** per extraction call, not individual chunks. Chunk boundary markers (`[CHUNK 0]`, `[CHUNK 1]`, ...) are inserted between chunks in the prompt so the LLM can attribute extractions to source chunks in its output.

Rationale: extraction needs cross-chunk context to produce coherent fact triples. A relationship like `problem_type:fabrication -[CAUSED_BY]-> problem_type:state_drift` may span chunks (problem stated in chunk 1, cause identified in chunk 3, solution in chunk 5). Per-chunk extraction structurally cannot produce cross-chunk relationships — it treats each chunk in isolation, losing discourse structure, coreference, and logical flow. Per-doc sees the whole document and produces the complete relationship set.

The cost delta is secondary to the quality delta. Per-doc produces fact triples that per-chunk structurally cannot produce, and those triples are the core value of the subject layer for #692's self-knowledge queries.

**Large-doc escape hatch:** For docs exceeding 30K tokens (unlikely for `docs/solutions/` at 2-10K tokens, but possible for future content), fall back to section-level extraction where sections are identified by `##` heading structure. Flag as a known boundary condition — implement only when the trigger case exists.

### D2. Async background extraction at startup, sync on compound hook

Resolved during planning. Two execution contexts:

**Startup:** After server readiness (domain rebuild + chunk ingestion complete), spawn one background extraction task per agent. Tasks run concurrently (per-agent parallelism), internally serial per doc. Server is ready for queries immediately; subject entities appear progressively as extraction completes. Matches C1.1's bounded-staleness pattern — agents see FTS5/vec results immediately, subject graph fills in asynchronously.

**Compound hook:** `/ce:compound` writes a doc → #689 `ingest_document` (sync) → #690 `extract_document` (sync, 1 call, <2s). The authoring agent gets immediate subject-graph queryability for the doc they just wrote. Failure follows C2.3 log-and-skip — extraction failure does not fail the compound write.

**`create_agent`:** After the tool writes the new agent, spawn a background extraction task for that agent's pending docs. Same code path as startup, single-agent scope.

Concurrency model: per-agent parallelism bounds peak LLM concurrency to N (agent count). Rate-limit handling per C2.2 is per-agent-local. If the provider rate-limits, individual agents' extractors back off independently.

### D3. Constrained extraction — approved types and relationship types only

Resolved during planning. Extraction is constrained to a fixed set of entity types and relationship types. The LLM prompt specifies the allowed types; the output validator rejects any extraction with an unapproved type.

**Approved entity types:**

| Type prefix | Source | Description |
|-------------|--------|-------------|
| `skill:` | Well-known (domain graph) | References to skills by name |
| `tool:` | Well-known (domain graph) | References to tools by name |
| `agent:` | Well-known (domain graph) | References to agents by name |
| `problem_type:` | Both (domain seed + discovered) | Bug categories, failure modes |
| `solution_path:` | Discovered | Named solution strategies |
| `failure_mode:` | Discovered | Specific failure patterns |
| `pattern:` | Discovered | Recurring architectural/workflow patterns |

Well-known types (skill, tool, agent) produce subject entities here; #691 resolves them to domain entities. Discovered types (solution_path, failure_mode, pattern) live purely in the subject graph — they have no domain counterpart unless a future ticket adds domain seeds for them.

**Approved relationship types:**

| Relationship | From type(s) | To type(s) |
|-------------|-------------|------------|
| `SOLVED_BY` | problem_type | solution_path |
| `USES` | solution_path | skill |
| `CALLS` | solution_path | tool |
| `INDICATES` | failure_mode | problem_type |
| `PREVENTS` | pattern | problem_type |
| `CAUSED_BY` | problem_type | problem_type |
| `MENTIONS` | any | agent |

The type constraint is structural (validated in code), not prompt-only. The prompt instructs the LLM to produce only approved types; the validator rejects non-compliant output at the JSON parsing layer. Per `feedback_prompt_enforcement_fragile.md`: structural guards, not prompt-text rules.

### D4. Output schema — what the LLM must produce

Resolved during planning. The extraction prompt produces a JSON object with three arrays. Exact prompt wording deferred to implementation (empirical); this plan specifies the **output contract**.

```json
{
  "entities": [
    {
      "type": "problem_type",
      "name": "fabrication",
      "description": "LLM generates content not grounded in tool results",
      "chunk_indices": [0, 2, 5],
      "confidence": 0.9
    }
  ],
  "relationships": [
    {
      "from_type": "problem_type",
      "from_name": "fabrication",
      "to_type": "problem_type",
      "to_name": "state_drift",
      "type": "CAUSED_BY",
      "chunk_indices": [2, 3],
      "confidence": 0.85
    }
  ]
}
```

**Key properties:**

- `chunk_indices` on both entities and relationships. These map to the `[CHUNK N]` markers in the prompt and drive `kg_chunk_subjects` / `kg_chunk_subject_relationships` provenance rows.
- `confidence` on both entities and relationships. Extraction-time confidence from the LLM. Stored on `kg_subject_entities.properties_json` (entities) and `kg_subject_relationships.confidence` (relationships). Downstream consumers (#688, #692) can filter by threshold.
- `type` + `name` on entities compose to `entity_key` via the `type || ':' || name` convention (per #686 D3).
- Entity `description` is optional free text stored in `properties_json`.

**Validation rules (structural, not prompt-only):**

1. Top-level must be an object with `entities` and `relationships` arrays.
2. Every entity must have `type` in the approved list, non-empty `name`, and non-empty `chunk_indices`.
3. Every relationship must have `type` in the approved list, valid `from_type`/`from_name` and `to_type`/`to_name` that reference entities in the same response, and non-empty `chunk_indices`.
4. Relationship type constraints are enforced (e.g., `SOLVED_BY` must go from `problem_type` to `solution_path`).
5. Malformed JSON or schema violations trigger the C2.2 semantic-failure retry (one retry with prompt reinforcement, then log-and-skip).

### D5. Delete-and-re-extract on doc change (three-phase reconciliation)

Resolved during planning. When #689 re-ingests a changed doc (per #689 D4/D6), subject extraction must reconcile. The naive "delete then re-extract" exposes intermediate partial state and can spuriously delete entities shared across docs.

**Three-phase flow:**

**Phase 1 — Capture (before #689 re-ingestion):**
Read the set of subject entities and relationships previously linked to this doc's chunks:
```sql
SELECT DISTINCT cs.subject_entity_id
FROM kg_chunk_subjects cs
JOIN kg_chunks c ON cs.chunk_id = c.id
WHERE c.agent_id = ? AND c.source_doc_path = ?
```
Store as `previous_entity_ids` and `previous_relationship_ids`. This set scopes the orphan sweep in Phase 3.

**Phase 2 — Reingest (#689's existing flow):**
#689 deletes old chunks, writes new chunks, within its own transaction. Chunk deletion CASCADE-deletes `kg_chunk_subjects` and `kg_chunk_subject_relationships` rows for those chunks.

**Phase 3 — Re-extract and reconcile (single transaction):**
1. Run per-doc extraction on the new doc text (LLM call, outside transaction).
2. `BEGIN TRANSACTION`.
3. UPSERT new entities: `INSERT INTO kg_subject_entities ... ON CONFLICT(agent_id, entity_key) DO UPDATE SET properties_json = ..., updated_at = ...`. Preserves `id` so existing relationships and resolutions survive.
4. INSERT new `kg_chunk_subjects` provenance rows (new chunks → entities).
5. UPSERT new relationships: `INSERT INTO kg_subject_relationships ... ON CONFLICT(agent_id, from_entity_id, to_entity_id, type) DO UPDATE SET confidence = ..., properties_json = ...`.
6. INSERT new `kg_chunk_subject_relationships` provenance rows (new chunks → relationships).
7. Scoped orphan sweep — delete entities from `previous_entity_ids` that now have zero provenance:
   ```sql
   DELETE FROM kg_subject_entities
   WHERE id IN (<previous_entity_ids>)
     AND id NOT IN (SELECT subject_entity_id FROM kg_chunk_subjects WHERE agent_id = ?)
   ```
   CASCADE handles `kg_subject_relationships`, `kg_subject_resolutions`, `kg_chunk_subjects`.
8. Scoped orphan sweep for relationships from `previous_relationship_ids` with zero provenance.
9. `COMMIT`.

**Correctness properties:**
- Entities shared across docs survive (they still have provenance from other docs' chunks).
- Entities unique to the edited doc that the new text still mentions survive (UPSERT in step 3 + new provenance in step 4).
- Entities unique to the edited doc that the new text no longer mentions are correctly deleted (zero provenance after step 4 → caught by step 7).
- No observable intermediate state — steps 2-9 are in one transaction.

### D6. Single extractor, multiple invokers

Resolved during planning. `SubjectExtractor.extract_document(doc_path, agent_id, previous_state: Option<PreviousState>)` is the sole extraction entry point. All invokers call it:

- **Startup background:** `extract_pending(agent_id)` enumerates docs needing extraction (see D7), calls `extract_document` for each.
- **Compound hook:** calls `extract_document` with `previous_state: None` (fresh doc, no orphan sweep).
- **`create_agent`:** spawns `extract_pending(new_agent_id)` as background task.
- **Re-extraction on doc change:** calls `extract_document` with `previous_state: Some(...)` for the three-phase flow.

The extractor is invocation-context-agnostic — no assumptions about being on the main runtime, no coupling to server state. This keeps the migration path to a dedicated worker (Option 3 from planning) open if scale demands it.

### D7. Pending-doc query drives extraction (idempotent)

Resolved during planning. The extractor determines what needs extraction by querying data shape, not tracking explicit state:

```sql
-- Docs with chunks but no subject provenance
SELECT DISTINCT c.source_doc_path
FROM kg_chunks c
WHERE c.agent_id = ?
  AND NOT EXISTS (
    SELECT 1 FROM kg_chunk_subjects cs
    WHERE cs.chunk_id = c.id
  )
```

This query drives startup extraction and handles all failure modes:
- Server crash mid-extraction: incomplete docs have chunks but no provenance → pending on next startup.
- Provider down at startup: retries exhaust → docs remain pending → picked up on next startup.
- New docs from compound hook: if hook extraction fails (C2.3 log-and-skip), doc stays pending → picked up at next startup.

No explicit state enum, no `kg_extractions` tracking table. The data shape is the state. Matches C1.1's embedding backfill pattern (idempotent by `embedding_json IS NULL`).

**Periodic retry (deferred):** A background task re-running the pending query every N hours would catch "provider was flaky at startup, recovered later." Deferred to implementation — build without it, add if "stale extraction due to startup-time provider failure" becomes an observed pattern.

### D8. Schema amendments to #686 — D11, D12, D13

Surfaced during #690 planning. All three are new tables for subject-layer cross-table relationships. They follow the pattern identified across the milestone: *every first-class KG table has relationships to other first-class tables that need their own tables*.

**D11. `kg_subject_relationships`** — subject-to-subject edges (fact triples).

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

**D12. `kg_chunk_subjects`** — entity provenance (chunk → subject entity, many-to-many).

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

**D13. `kg_chunk_subject_relationships`** — relationship provenance (chunk → subject relationship, many-to-many).

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

These three tables complete the subject layer's provenance model. The convention to add to `kg-implementation-conventions.md`: "Every first-class KG table has relationships to other first-class tables; those relationships need their own tables with their own provenance." D11-D13 should be folded into #686's v25 schema if still pre-ship, or batched as v25→v26 if v25 has shipped.

### D9. Observability — per-doc audit_events + progress logging

Per C3.3. Each per-doc extraction emits one `audit_events` row:

- `tool_name`: `extract_subject_entities`
- `target_key`: `kg_subject:<source_doc_path>`
- `before_value`/`after_value`: summary counts (e.g., `{"entities_extracted": 12, "relationships": 8}`)

Background extraction progress logging:

```
INFO trace_id=<id> event=subject_extraction_start agent_id=<a> pending_docs=200
INFO trace_id=<id> event=subject_extraction_progress agent_id=<a> completed=50 remaining=150 duration_ms=240000
INFO trace_id=<id> event=subject_extraction_complete agent_id=<a> completed=200 failed=3 duration_ms=720000
```

Every LLM call emits an `llm_calls` row per C2.4.

## Open Questions

### Resolved During Planning

- Extraction granularity — per-doc with chunk boundary markers (see D1).
- Execution model — async background at startup, sync on compound hook (see D2).
- Entity and relationship types — constrained to approved lists (see D3).
- Output schema — structured JSON with chunk-index annotations (see D4).
- Re-extraction lifecycle — three-phase capture → reingest → reconcile (see D5).
- Single extractor, multiple invokers (see D6).
- Pending-doc detection — idempotent by data shape (see D7).
- Schema amendments — D11/D12/D13 folded back into #686 (see D8).
- Subject-to-subject relationship storage — `kg_subject_relationships` table, not `kg_relationships` extension (D11).
- Chunk-subject provenance — `kg_chunk_subjects` join table with `extraction_trace_id` (D12).
- Relationship-chunk provenance — `kg_chunk_subject_relationships` join table (D13, surfaced during re-extraction design).

### Deferred to Implementation

- Exact LLM prompt wording for extraction (empirical — iterate during implementation).
- Extraction model default (`MIKA_KG_EXTRACTION_MODEL` default value — haiku-4.5 or deepseek-v3, to be determined by cost/quality testing).
- Progress logging interval (every N docs or every M seconds — tune during implementation).
- Whether `create_agent` auto-triggers extraction immediately or waits for next startup (implement the cleaner version if straightforward, defer if not).
- Large-doc section-level fallback threshold (suggested 30K tokens — implement only when trigger case exists).
- Periodic retry for stale-at-startup extraction (deferred until failure mode is observed).

## Output Structure

```
crates/mika-agent/src/
├── db.rs                            # ADD: kg_schema amendments (D11/D12/D13 table constants)
├── db/kg_schema.rs                  # ADD: column constants for new tables
└── kg/
    ├── mod.rs                       # MODIFY: add `pub mod subject_extractor;`
    ├── domain_builder.rs            # (from #687 — unchanged)
    ├── chunker.rs                   # (from #689 — unchanged)
    ├── lexical_ingestor.rs          # (from #689 — MODIFY: hook extraction after ingest)
    └── subject_extractor.rs         # NEW: per-doc extraction, output validation, DB writes

crates/mika-agent/src/server/
└── mod.rs                           # MODIFY: spawn background extraction after startup

crates/mika-agent/tests/
└── kg/
    └── subject_extractor.rs         # NEW: extraction integration tests

docs/plans/
└── 2026-04-21-006-feat-subject-graph-extraction-plan.md   # this file
```

## High-Level Technical Design

> *Directional guidance for review, not implementation specification.*

```rust
// crates/mika-agent/src/kg/subject_extractor.rs

/// Sole writer of kg_subject_entities, kg_subject_relationships,
/// kg_chunk_subjects, and kg_chunk_subject_relationships.
///
/// Invariants:
/// - All extraction goes through extract_document().
/// - Output JSON validated structurally, not just by prompt.
/// - Re-extraction uses three-phase reconciliation (D5).
/// - All writes carry trace_id for the extraction invocation.
pub struct SubjectExtractor {
    db: AsyncDatabase,
    llm: Arc<dyn LlmProvider>,  // from MIKA_KG_EXTRACTION_MODEL
    trace_id: String,
}

/// Previous provenance state, captured before re-ingestion.
/// None for fresh extraction (first-time or compound hook).
pub struct PreviousState {
    entity_ids: Vec<i64>,
    relationship_ids: Vec<i64>,
}

impl SubjectExtractor {
    /// Core extraction entry point — invocation-context-agnostic.
    pub async fn extract_document(
        &self,
        agent_id: &str,
        doc_path: &str,
        previous_state: Option<PreviousState>,
    ) -> Result<ExtractionStats> {
        // 1. Read full doc from disk.
        // 2. Read chunk boundaries from kg_chunks for this (agent_id, doc_path).
        // 3. Build prompt with chunk boundary markers.
        // 4. LLM call (with C2.2 retry on failure).
        // 5. Parse + validate output JSON (D4 schema).
        // 6. Write entities, relationships, provenance in single transaction (D5 phase 3).
        // 7. If previous_state is Some, run scoped orphan sweep.
        // 8. Emit audit_events row.
    }

    /// Enumerate and extract all pending docs for an agent.
    pub async fn extract_pending(&self, agent_id: &str) -> Result<BatchStats> {
        // Query: docs with chunks but no kg_chunk_subjects provenance (D7).
        // For each: extract_document(agent_id, doc_path, None).
        // Log progress per D9.
    }
}
```

### Extraction prompt shape (directional, not final)

```
System: You are a knowledge graph extraction agent. Extract named entities
and fact triples from the following document. Return ONLY valid JSON
matching the specified schema.

[Approved entity types: skill, tool, agent, problem_type, solution_path,
 failure_mode, pattern]
[Approved relationship types: SOLVED_BY, USES, CALLS, INDICATES, PREVENTS,
 CAUSED_BY, MENTIONS]

User: [CHUNK 0] <chunk 0 text> [CHUNK 1] <chunk 1 text> ... [CHUNK N] <chunk N text>

Expected output: { "entities": [...], "relationships": [...] }
```

## Implementation Units

- [ ] **Unit 1: Schema amendments (D11/D12/D13)**

**Goal:** Add `kg_subject_relationships`, `kg_chunk_subjects`, `kg_chunk_subject_relationships` tables to the KG schema migration. Column constants in `kg_schema.rs`.

**Requirements:** D8 (D11/D12/D13).

**Dependencies:** #686 schema (v25 base).

**Files:** `crates/mika-agent/src/db.rs` (migration), `crates/mika-agent/src/db/kg_schema.rs` (constants).

**Approach:** If #686 is still pre-ship, fold into v25 directly. If shipped, v25→v26 migration following the same pattern as v24→v25. Add to `migrate_v1()` for clean-slate convergence.

**Test scenarios:**
- Forward-test: v25→v26 migration applies cleanly on a DB with existing kg_entities/kg_chunks data.
- Clean-slate: fresh DB has all three new tables.
- FK CASCADE: deleting a kg_subject_entity CASCADE-deletes its kg_chunk_subjects and kg_subject_relationships rows.
- UNIQUE constraints prevent duplicate provenance/relationship rows.

**Verification:** `cargo test -p mika-agent -- migrations`

---

- [ ] **Unit 2: LLM provider for extraction model**

**Goal:** Create a dedicated `LlmProvider` instance from `MIKA_KG_EXTRACTION_MODEL` (with `MIKA_KG_INGESTION_MODEL` fallback per C2.1).

**Requirements:** C2.1.

**Dependencies:** `mika-common` LLM infrastructure.

**Files:** `crates/mika-agent/src/kg/subject_extractor.rs` (provider creation), `crates/mika-common/src/config.rs` (env var resolution).

**Approach:** Parse `MIKA_KG_EXTRACTION_MODEL` as `provider/model` string (e.g., `anthropic/claude-haiku-4-5-20251001`). Resolve API key from the provider's existing env var. Fall back to `MIKA_KG_INGESTION_MODEL`, then to a hardcoded default. Create via `create_provider()`.

**Test scenarios:**
- Explicit model set → uses that model.
- Fallback chain: extraction unset → ingestion → default.
- Missing API key for configured provider → clear error at startup, not a silent failure mid-extraction.

**Verification:** Unit tests with mock provider.

---

- [ ] **Unit 3: Output schema validation**

**Goal:** Parse and validate extraction LLM output against D4's schema. Reject malformed JSON, unapproved types, and relationship type constraint violations.

**Requirements:** D3, D4.

**Dependencies:** None (pure data validation).

**Files:** `crates/mika-agent/src/kg/subject_extractor.rs` (validation module).

**Approach:** Deserialize into typed structs (`ExtractionOutput`, `ExtractedEntity`, `ExtractedRelationship`). Validate entity types against `APPROVED_ENTITY_TYPES` const, relationship types against `APPROVED_RELATIONSHIP_TYPES` const with from/to type constraints. Return `ValidationError` with specific failure details for the C2.2 semantic-retry prompt reinforcement.

**Test scenarios:**
- Valid JSON with approved types → accepted.
- Unknown entity type → rejected with message naming the type.
- Relationship with wrong from/to types → rejected.
- Missing chunk_indices → rejected.
- Empty entities array → accepted (some docs may have no extractable entities).
- Malformed JSON → rejected, triggers retry.

**Verification:** `cargo test -p mika-agent -- subject_extractor::validation`

---

- [ ] **Unit 4: Core extraction logic — extract_document**

**Goal:** The single entry point: read doc, build prompt with chunk markers, LLM call with C2.2 retry, validate output, write to DB in single transaction.

**Requirements:** D1, D4, D5, D6, C2.2, C2.3.

**Dependencies:** Units 1-3.

**Files:** `crates/mika-agent/src/kg/subject_extractor.rs`.

**Approach:**
1. Read full doc from disk. Read `kg_chunks` for this (agent_id, doc_path) to get chunk boundaries (seq_id ordering).
2. Build prompt: system message with approved types/schemas, user message with doc text annotated by `[CHUNK N]` markers.
3. Call LLM. On transport/rate-limit failure: retry per C2.2 (up to 3 attempts with backoff). On semantic failure (malformed JSON): one retry with prompt reinforcement, then log-and-skip per C2.3.
4. Validate output (Unit 3).
5. If `previous_state` is Some: execute D5 three-phase reconciliation in single transaction.
6. If `previous_state` is None: write entities (UPSERT), relationships (UPSERT), provenance rows in single transaction.
7. Emit `llm_calls` row (C2.4).
8. Emit `audit_events` row (D9).
9. Return `ExtractionStats`.

**Test scenarios:**
- Fresh extraction: entities, relationships, provenance all written correctly.
- Re-extraction with unchanged content: UPSERT produces same rows, no orphans.
- Re-extraction with changed content: old-only entities deleted, new entities added, shared entities survive.
- LLM returns malformed JSON: retry fires, then log-and-skip. No partial writes.
- LLM returns empty entities: succeeds with zero entities written (doc has no extractable content).

**Verification:** Integration tests with `MockLlmProvider`.

---

- [ ] **Unit 5: Background extraction — extract_pending**

**Goal:** Enumerate pending docs per agent, extract each, log progress.

**Requirements:** D2, D7, D9.

**Dependencies:** Unit 4.

**Files:** `crates/mika-agent/src/kg/subject_extractor.rs`.

**Approach:** Run the pending-doc query (D7). Iterate docs, call `extract_document` for each. Log progress every 10 docs or 60 seconds (whichever comes first). Log completion summary. Failures per doc are logged and skipped (C2.3) — they remain pending for next startup.

**Test scenarios:**
- 0 pending docs → returns immediately, logs "nothing to extract."
- N pending docs → extracts all, logs progress and completion.
- Mid-run failure on doc K → docs 0..K-1 extracted, K skipped (logged), K+1..N continue.
- Provider entirely down → all docs fail, all remain pending.

**Verification:** Integration tests with mock.

---

- [ ] **Unit 6: Startup and compound hook integration**

**Goal:** Wire extraction into the server startup sequence (background spawn) and compound hook (synchronous inline).

**Requirements:** D2.

**Dependencies:** Unit 5.

**Files:** `crates/mika-agent/src/server/mod.rs` (startup spawn), `crates/mika-agent/src/kg/lexical_ingestor.rs` (compound hook integration).

**Approach:**
- Startup: after `LexicalIngestor.ingest_all()` completes, `tokio::spawn` one `extract_pending(agent_id)` task per agent.
- Compound hook: after `ingest_document(doc)`, call `extract_document(doc, None)` synchronously. Wrap in catch — extraction failure must not fail the compound write.

**Test scenarios:**
- Server starts, extraction tasks spawned for each agent.
- Compound doc written → both chunks and subject entities available immediately.
- Compound hook extraction fails → doc ingested successfully, extraction pending for next startup.

**Verification:** Integration test verifying startup sequence ordering; compound hook test with mock.

---

- [ ] **Unit 7: Re-extraction integration with #689**

**Goal:** Wire the three-phase re-extraction flow (D5) into #689's doc-change handler.

**Requirements:** D5.

**Dependencies:** Units 4, 6.

**Files:** `crates/mika-agent/src/kg/lexical_ingestor.rs` (capture previous state before re-ingest), `crates/mika-agent/src/kg/subject_extractor.rs` (reconciliation).

**Approach:**
- Before #689 deletes old chunks for a changed doc, capture `PreviousState` (entity_ids and relationship_ids linked to this doc's chunks).
- After #689 writes new chunks, call `extract_document(doc, Some(previous_state))`.
- The single-transaction reconciliation in Unit 4 handles the rest.

**Test scenarios:**
- Doc edited, entity removed → entity with no other provenance is deleted.
- Doc edited, entity still present → entity survives with updated provenance.
- Doc edited, new entity added → new entity and provenance created.
- Doc edited, shared entity between two docs → entity survives (provenance from other doc remains).

**Verification:** Integration test with two docs sharing an entity, editing one.

## Error Handling & Edge Cases

| Scenario | Expected behavior |
|----------|------------------|
| LLM provider not configured (`MIKA_KG_EXTRACTION_MODEL` unset, no fallback) | Extraction disabled at startup. Log `warn!(event="extraction_disabled", reason="no extraction model configured")`. Chunks still ingested, subject layer stays empty. |
| Provider rate-limited mid-extraction | Per-agent backoff per C2.2. Other agents' extraction continues. Rate-limited agent retries affected doc on next attempt. |
| Doc exceeds 30K tokens | Log warning, skip extraction for this doc. Flag for future section-level fallback. |
| LLM returns entities with names containing `:` | Validate `name` field does not contain `:` (would break `entity_key = type || ':' || name` CHECK constraint). Reject entity, log warning. |
| Duplicate entity_key from same doc (LLM extracts `problem_type:fabrication` twice) | UPSERT handles — second insertion updates properties. Single provenance row per (chunk, entity) via UNIQUE constraint. |
| Zero entities extracted from a doc | Valid outcome — some docs (e.g., pure configuration guides) may have no extractable entities. Write no rows, emit audit_event with `entities_extracted: 0`. Doc is marked as "extracted" by having been processed (no longer returned by pending query if we track extraction completion — see D7 note below). |

**D7 edge case — zero-entity docs and the pending query:**

The pending-doc query (D7) checks for absence of `kg_chunk_subjects` rows. A doc with zero extracted entities has no provenance rows — it would be re-extracted every startup. Two options:

1. Accept the re-extraction cost (~$0.0003 per doc). It's idempotent and the cost is noise. Simplest.
2. Write a sentinel `kg_chunk_subjects` row with a special `subject_entity_id` (e.g., pointing to a synthetic "no entities" sentinel entity).

Option 1 is recommended — re-extracting a zero-entity doc costs less than the engineering complexity of a sentinel. If the fraction of zero-entity docs becomes large enough to matter, add the sentinel then. YAGNI.
