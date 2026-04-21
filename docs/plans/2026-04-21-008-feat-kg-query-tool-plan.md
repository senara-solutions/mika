---
title: "feat: query_knowledge_graph — KG traversal tool for agent self-awareness"
type: feat
status: active
date: 2026-04-21
---

# query_knowledge_graph — KG traversal tool for agent self-awareness

## Overview

Add a `query_knowledge_graph` builtin tool (milestone mika#14, ticket mika#688) that lets agents traverse the KG to discover capabilities, find solution paths, and reason about their environment. Hybrid retrieval: parallel entry paths (direct name match, subject name match, semantic search, optional LLM translation) find starting entities, then graph expansion via recursive CTE traverses relationships up to configurable depth. Returns entities, edges, and optional chunk provenance ranked by traversal distance and edge confidence.

Separate tool from `get_documentation` — KG traversal and static doc lookup are different operations with different intents. The self-knowledge skill (#692) orchestrates both.

## Problem Frame

The KG now has three populated layers: domain (structure from manifests), lexical (chunks from docs), and subject (extracted entities and relationships). Entity resolution (#691) bridges subject to domain. But no agent-facing tool exists to query this graph — agents can't ask "what solves CI failures?" and get a traversal from `problem_type:ci_failure` through `SOLVED_BY` to solution paths and skills.

Without a query tool, the KG is an infrastructure investment with no consumer. #688 is the read interface that makes the graph useful.

## Requirements Trace

- R1. Hybrid retrieval: parallel entry paths for query → entity mapping, then graph expansion.
- R2. Cross-layer traversal: domain, subject, and lexical layers compose via resolution edges.
- R3. Agent-scoped queries return entities enriched with `agent_context` metadata (enablement, availability). No filtering — metadata only.
- R4. Return shape distinguishes `starting_entity_missing` from `traversal_empty` for #692 fallback logic.
- R5. Result ranking: distance first, confidence second, provenance density third. Top-K per level.
- R6. Default compact return (entity keys, edge types, chunk IDs). Optional `include_context` for prose.
- R7. Read-only tool — no mutations to any KG table.

## Scope Boundaries

- Query tool implementation, input/output schema, retrieval algorithm, ranking.
- No self-knowledge orchestration (deferred to **mika#692**).
- No query-intent-aware traversal (deferred — MVP returns generic tree, LLM synthesizes).
- No KG mutations.

## Context & Research

### Cross-cutting conventions

- **C1.1 (async-embedding):** Query tool reads from `vec_search` via the existing `hybrid_search` pipeline. Freshly ingested chunks may not have embeddings yet — query gracefully degrades to FTS5-only.

### Dependencies

- **#686:** Schema (all tables).
- **#687:** Domain graph populated.
- **#689:** Lexical chunks populated.
- **#690:** Subject entities and relationships populated.
- **#691:** Subject → domain resolutions populated.

### Relevant Code and Patterns

- **`hybrid_search`:** `crates/mika-agent/src/db.rs:6286-6340`. FTS5 + vec0 with RRF k=60. Input: `(agent_id, query, limit, source_type_filter)`. Returns `SearchResult` with `source_type`, `source_id`, `content`, `score`. The `source_type="kg_chunk"` filter scopes to KG chunks.
- **Recursive CTE pattern:** Not in the codebase yet. Standard SQLite recursive CTE shape:
  ```sql
  WITH RECURSIVE traverse AS (
    SELECT ... FROM kg_relationships WHERE from_entity_id = ?
    UNION ALL
    SELECT ... FROM kg_relationships JOIN traverse ON ...
    WHERE depth < max_depth
  )
  ```
- **Tool registration:** `crates/mika-agent/src/tools/mod.rs` `default_tools()`. New tool registers here.
- **`skill_overrides` LEFT JOIN:** `crates/mika-agent/src/db.rs`. Tri-state: NULL=default enabled, 0=disabled, 1=explicitly enabled. `COALESCE(so.enabled, 1)` gives effective boolean.

## Key Technical Decisions

### D1. Parallel entry paths, not linear flow

Resolved during planning. A single "semantic search → chunks → entities → traverse" flow has a single-point-of-failure at the resolution boundary. Parallel entry paths find starting entities via multiple strategies, merge results, and use top-K as traversal starting points.

**Path A — Direct domain entity match:** Case-insensitive match of query terms against `kg_entities.name` and `entity_key`. "CI failures" → `problem_type:ci_failure`. Fast (indexed), no LLM cost. High precision when it works.

**Path B — Subject entity match:** Same match against `kg_subject_entities.name` for the querying agent. Captures entities that haven't resolved to domain yet, and discovered types (solution_path, failure_mode, pattern) that have no domain counterpart.

**Path C — Semantic search via chunks:** `hybrid_search(query, source_type="kg_chunk")` → chunks → `kg_chunk_subjects` → subject entities → `kg_subject_resolutions` → domain entities. Captures cases where the entity name doesn't lexically match but the surrounding prose does.

**Path D — LLM query translation (deferred):** If paths A-C all fail to find a confident entry point, ask a resolution-tier LLM to map the query to domain entities from a candidate list. Deferred to future enhancement — MVP relies on A-C.

Each path returns `(entity_id, layer: domain|subject, entry_confidence)` tuples. Merge, dedupe by entity_id, rank by entry_confidence, use top-K (default K=5) as traversal starting points.

### D2. Traversal algorithm

Resolved during planning.

**Edge policy:** Default set of all relationship types in both domain and subject layers. Caller can restrict via `follow` parameter in `traversal` input. Traversal follows both `kg_relationships` (domain) and `kg_subject_relationships` (subject).

**Max depth:** Default 2. Caller-overridable via `max_depth` parameter (cap at 4 to bound result size). Depth 2 covers the common query shape: problem → solution_path → skill/tool.

**Resolution strategy:** Pre-resolve at traversal start. Before expanding edges, resolve all subject entities in the starting set to their domain counterparts (via `kg_subject_resolutions`). Traversal then operates on domain entity IDs where possible. For subject entities without domain resolution (discovered types), traversal stays in the subject layer.

**Cross-layer hops:** When traversal reaches a subject entity that has a domain resolution, the traversal transparently follows into the domain layer. When domain traversal reaches a domain entity that has subject-layer edges (via reverse resolution), it can optionally follow those. Default: domain→subject hop disabled (keeps results clean). Caller can enable via `cross_layer: true`.

**Subject-only traversal:** Questions about discovered types (failure_mode, solution_path, pattern) traverse within the subject layer without requiring a domain anchor. The entry path (B) finds subject entities directly.

### D3. Result ranking

Resolved during planning.

1. **Traversal distance:** 0-hop (starting entity) > 1-hop > 2-hop. Entities closer to entry point rank higher.
2. **Cumulative edge confidence:** Product of confidence scores along the traversal path. A 2-hop path through two 0.9-confidence edges scores 0.81.
3. **Provenance density:** Count of `kg_chunk_subjects`/`kg_chunk_subject_relationships` rows supporting each entity/edge. More provenance = stronger signal.

Results are returned as a ranked list within each traversal level. Default top-K per level: 5 entities at hop 1, 3 per parent entity at hop 2. Caller can adjust via `result_limit`.

### D4. Return shape and status field

Resolved during planning.

**Input shape:**

```json
{
  "question": "what solves CI failures?",
  "traversal": {
    "start": "problem_type:ci_failure",
    "follow": ["SOLVED_BY", "USES", "PROVIDES"],
    "max_depth": 2,
    "cross_layer": false
  },
  "agent_id": "mika-dev",
  "include_context": false,
  "result_limit": 10
}
```

`question` and `traversal.start` are either/or — `question` triggers entry-path resolution (D1), `traversal.start` bypasses it with a known entity_key. `agent_id` makes the query agent-scoped (D5). `include_context` enables chunk prose in results.

**Return shape:**

```json
{
  "status": "ok",
  "entries": [
    {
      "entity_key": "problem_type:ci_failure",
      "name": "ci_failure",
      "type": "problem_type",
      "layer": "domain",
      "hop": 0,
      "confidence": 0.95,
      "agent_context": { "enabled": true },
      "edges_out": [
        {
          "type": "SOLVED_BY",
          "target": "solution_path:webhook_ci_handler",
          "confidence": 0.85
        }
      ]
    }
  ],
  "chunks": [],
  "entry_method": "path_a_direct_match"
}
```

**Status values:**

| Status | Meaning | #692 action |
|--------|---------|-------------|
| `ok` | Results found | Use results |
| `starting_entity_missing` | All entry paths failed to find an entity | Trigger registry/config fallback |
| `traversal_empty` | Entry entity found but no edges to follow | Trust the KG — graph says empty |

### D5. Agent-scoped queries: annotate, don't filter

Resolved during planning. When `agent_id` is provided, entities in results are enriched with `agent_context` metadata. The tool does NOT filter based on agent context — it returns all entities with their state, and the caller decides how to interpret.

**For skill entities:**
```sql
SELECT e.entity_key, e.name,
       COALESCE(so.enabled, 1) as agent_enabled
FROM kg_entities e
LEFT JOIN skill_overrides so
  ON so.skill_name = e.name AND so.agent_id = ?
WHERE e.type = 'skill'
```

`agent_context.enabled` is a boolean derived from the tri-state `skill_overrides.enabled` (NULL → true, 0 → false, 1 → true).

**Rationale:** Filtering is a question about intent, not data. "What skills do I have?" wants enabled only. "What skills could I enable?" wants disabled. "What skills solved fabrication?" is intent-independent. The tool can't guess intent; the LLM can. Annotate, let the LLM decide.

This extends the broader "combine KG with live state" pattern established across #692's staleness discussion: structural queries → KG; state → authoritative live sources; results combine both via metadata, not filtering.

### D6. Compact default, opt-in context

Resolved during planning. Default return includes entity keys, edge types, confidence, and chunk IDs — not chunk prose text. Chunk text is large and most traversals don't need it; the LLM can make a follow-up call with `include_context: true` or call `get_documentation` for specific docs.

Result budgets (hard caps, overridable):
- Max entities: 20
- Max edges: 30
- Max chunks (when `include_context: true`): 10

Keeps context window impact predictable for `always_on` self-knowledge skill.

## Open Questions

### Resolved During Planning

- Tool shape — two tools, `query_knowledge_graph` + `get_documentation` (separate responsibilities).
- Entry strategy — parallel paths A-C, path D deferred.
- Traversal — recursive CTE, default depth 2, cap 4, pre-resolve strategy.
- Ranking — distance > confidence > provenance density.
- Agent context — annotate as metadata, don't filter (D5).
- Status field — three values for #692 fallback logic (D4).
- Return compactness — default no chunk text, opt-in (D6).
- Query-intent-aware traversal — deferred (MVP: generic, LLM synthesizes).

### Deferred to Implementation

- Exact SQL for recursive CTE traversal (directional, not prescriptive in plan).
- Path D (LLM query translation for no-confidence entry points) — add when A-C prove insufficient.
- Query-intent classification (what/how/why → different edge types and depth emphasis).
- Cross-layer traversal detail (domain↔subject hop mechanics).
- Tool description wording (compact for context-window efficiency in `always_on` skill).
- Orphaned `skill_overrides` rows referencing deleted domain entities (outside #688 scope, flag for #687).

## Output Structure

```
crates/mika-agent/src/
├── tools/
│   └── query_knowledge_graph.rs   # NEW: the query tool
├── kg/
│   ├── mod.rs                     # MODIFY: add pub mod query;
│   └── query.rs                   # NEW: traversal engine, entry paths, ranking
└── db/
    └── kg_schema.rs               # MODIFY: add query-related column constants

crates/mika-agent/tests/
└── kg/
    └── query.rs                   # NEW: query integration tests

docs/plans/
└── 2026-04-21-008-feat-kg-query-tool-plan.md   # this file
```

## Implementation Units

- [ ] **Unit 1: Entry path resolution (paths A-C)**

**Goal:** Given a free-text query or explicit entity_key, find starting entities across domain and subject layers.

**Requirements:** D1.

**Files:** `crates/mika-agent/src/kg/query.rs`.

**Approach:**
- Path A: `SELECT id, entity_key FROM kg_entities WHERE LOWER(name) LIKE LOWER(?) OR LOWER(entity_key) = LOWER(?)`.
- Path B: `SELECT id, entity_key FROM kg_subject_entities WHERE agent_id = ? AND (LOWER(name) LIKE LOWER(?) OR LOWER(entity_key) = LOWER(?))`.
- Path C: `hybrid_search(query, source_type="kg_chunk")` → `kg_chunk_subjects` → entity_ids.
- Merge, dedupe, assign entry_confidence (A: 1.0 for exact, 0.8 for LIKE; B: 0.9 for exact; C: search score).

**Test scenarios:**
- Exact domain match → Path A, confidence 1.0.
- No domain match, subject match → Path B.
- No name match, chunk matches → Path C.
- All paths return same entity → deduped, highest confidence wins.

---

- [ ] **Unit 2: Graph traversal engine (recursive CTE)**

**Goal:** From starting entities, expand via relationship edges up to max_depth hops.

**Requirements:** D2.

**Files:** `crates/mika-agent/src/kg/query.rs`.

**Approach:** Recursive CTE over `kg_relationships` (domain) and optionally `kg_subject_relationships` (subject). Pre-resolve subject starting entities to domain via `kg_subject_resolutions`. Collect traversed entities and edges with hop count.

**Test scenarios:**
- Depth 1: starting entity → direct neighbors only.
- Depth 2: starting entity → neighbors → neighbors.
- Edge type filter: only follow specified types.
- Subject-only traversal: discovered type entity, subject edges only.
- Cross-layer: subject entity resolved to domain, traversal continues in domain.

---

- [ ] **Unit 3: Result ranking and budgeting**

**Goal:** Rank results by distance, confidence, provenance density. Apply result budgets.

**Requirements:** D3, D6.

**Files:** `crates/mika-agent/src/kg/query.rs`.

**Approach:** Score = `(max_depth - hop) * 100 + cumulative_confidence * 50 + provenance_count`. Top-K per hop level. Truncate to budget caps.

---

- [ ] **Unit 4: Agent context enrichment**

**Goal:** When `agent_id` provided, enrich skill/tool entities with `agent_context` metadata.

**Requirements:** D5.

**Files:** `crates/mika-agent/src/kg/query.rs`.

**Approach:** LEFT JOIN `skill_overrides` for skill entities. `COALESCE(so.enabled, 1)` for effective boolean. Attach as `agent_context` field on results.

---

- [ ] **Unit 5: Tool registration and integration**

**Goal:** Register `query_knowledge_graph` as a builtin tool. Wire input parsing, validation, and output formatting.

**Requirements:** R7 (read-only).

**Files:** `crates/mika-agent/src/tools/query_knowledge_graph.rs`, `crates/mika-agent/src/tools/mod.rs`.

**Approach:** Register in `default_tools()`. Input schema per D4. Parse `question` vs `traversal.start`. Call entry paths (Unit 1) or direct traversal (Unit 2). Enrich with agent_context (Unit 4) when `agent_id` provided. Format return per D4.

**Test scenarios:**
- Question-based query → entry paths → traversal → ranked results.
- Traversal-based query → direct start → traversal → results.
- Agent-scoped → entities have agent_context.
- Non-agent-scoped → no agent_context.
- `include_context: true` → chunks returned with prose.
- `include_context: false` → chunks returned as IDs only.
- Empty KG → status `starting_entity_missing`.
- Entity found, no edges → status `traversal_empty`.

## Error Handling & Edge Cases

| Scenario | Expected behavior |
|----------|------------------|
| KG not populated (first boot, extraction hasn't run) | All entry paths fail → `starting_entity_missing`. #692 fallback fires. |
| Question matches subject entity but not domain | Traversal starts from subject layer. Subject-only results returned. |
| Embedding not yet available for query (backfill lag) | Path C degrades to FTS5-only. Paths A-B unaffected. |
| Very broad query ("everything") | Entry paths return many entities → top-K limits (D1: K=5) bound traversal starting set. Result budgets (D6) cap output. |
| `max_depth: 0` | Returns starting entities only, no traversal. Valid for "does this entity exist?" queries. |
