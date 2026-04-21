---
title: "feat: entity resolution — bridge subject graph to domain graph"
type: feat
status: active
date: 2026-04-21
---

# Entity resolution — bridge subject graph to domain graph

## Overview

Resolve extracted subject entities to domain graph nodes (milestone mika#14, ticket mika#691). For each agent, after subject extraction (#690) produces `kg_subject_entities`, the resolver attempts to link well-known-type entities (skill, tool, agent, problem_type) to their corresponding `kg_entities` rows via `kg_subject_resolutions`. Two-stage pipeline: exact match (case-insensitive entity_key with sibling ambiguity check), then LLM disambiguation for unresolved or ambiguous cases. Per-agent scope.

This ticket reads from both sides of the graph (subject layer + domain layer) and writes only to its own edge table (`kg_subject_resolutions`) and tracking table (`kg_resolutions_log`). It does not modify `kg_entities`, `kg_subject_entities`, or any other table. This orthogonality is the architectural anchor — the resolver is a read-both-write-own-edges component.

## Problem Frame

#690 populates the subject graph with entities like `problem_type:fabrication` and relationships like `CAUSED_BY`. But these entities exist in a per-agent silo — they're named by what the LLM extracted from prose, not by canonical domain identifiers. An agent asking "which skills solve fabrication problems?" needs the bridge: subject `problem_type:fabrication` → domain `problem_type:fabrication` → domain edges `SOLVED_BY` → domain `skill:*`.

Without resolution, subject-layer queries can't compose with domain-layer traversals. The KG degrades to two disconnected graphs — one structural (domain, from manifests) and one semantic (subject, from prose) — with no cross-layer path.

## Requirements Trace

- R1. Two-stage resolution: exact match with sibling ambiguity check, then LLM disambiguation for unresolved/ambiguous well-known types.
- R2. Resolution runs as per-doc follow-on after extraction (same background task), with compound hook resolution spawned asynchronously.
- R3. Structured tracking via `kg_resolutions_log` — authoritative "has this been attempted?" and "what was the outcome?"
- R4. Confidence scoring: min(extraction_confidence, disambiguation_confidence). Exact match confidence = extraction confidence, not always 1.0.
- R5. Resolution LLM prompts must authorize "no_match" as a first-class response.
- R6. Observability per C2.4 (llm_calls for disambiguation) and C3.3 (per-doc audit_events).

## Scope Boundaries

- Subject → domain resolution via `kg_subject_resolutions`.
- Resolution tracking via `kg_resolutions_log`.
- No cross-agent subject → subject resolution (deferred — see D8).
- No mutation of domain graph or subject graph — read-both, write-own-edges only.
- No human-in-the-loop review queue — deferred, potentially surfaced via #692.
- No query tool: **mika#688**.

## Context & Research

### Cross-cutting conventions

- **C2 (non-interactive LLM call policy):** All of C2 applies to disambiguation calls. Model: `MIKA_KG_RESOLUTION_MODEL` (C2.1). Retry: C2.2 four-category taxonomy. Log-and-skip: C2.3. Observability: C2.4 llm_calls rows.
- **C3.3 (observability — subject extraction/resolution):** Per-doc audit_events for resolution, matching extraction's cadence.

### Dependencies

- **#686 schema:** `kg_subject_resolutions` table (existing), `kg_resolutions_log` table (D16, new).
- **#687:** Domain graph nodes must exist before resolution can match against them. Startup ordering: domain rebuild → chunk ingestion → extraction → resolution.
- **#690:** Subject entities must exist before resolution. Resolution runs as a per-doc follow-on after extraction, sharing the extraction task's context.

### Relevant Code and Patterns

- **`kg_subject_resolutions` schema:** `UNIQUE(agent_id, subject_entity_id, domain_entity_id)`, `confidence REAL NOT NULL CHECK(0.0..1.0)`, CASCADE on both FKs. Allows multiple resolutions per subject entity (one subject entity can resolve to multiple domain entities if genuinely ambiguous).
- **`kg_entities` query surface:** `entity_key TEXT UNIQUE`, `type TEXT`, `name TEXT`. Domain entities queried by `type` for candidate lists (e.g., all `skill:*` entities).
- **LlmProvider infrastructure:** `create_provider(ModelSpec, max_tokens)` for `MIKA_KG_RESOLUTION_MODEL`. Same pipeline as extraction model.
- **COLLATE NOCASE:** Not on `entity_key` in current schema. Case-insensitive exact match requires explicit `LOWER()` or `COLLATE NOCASE` in queries.

## Key Technical Decisions

### D1. Two-stage pipeline: exact match + LLM disambiguation

Resolved during planning. No fuzzy string matching stage (Levenshtein, FTS5 ranking) — the LLM subsumes fuzzy matching with better judgment and explicit confidence.

**Stage 1 — Exact match with sibling ambiguity check:**

For each subject entity with a well-known type (skill, tool, agent, problem_type):

1. Case-insensitive exact match: `SELECT id, entity_key FROM kg_entities WHERE LOWER(entity_key) = LOWER(?)`.
2. If exactly one match AND extraction confidence > 0.9 AND no sibling ambiguity → resolve at confidence = extraction_confidence. No LLM call.
3. **Sibling ambiguity check:** query `SELECT COUNT(*) FROM kg_entities WHERE type = ? AND (name LIKE ? || '%' OR ? LIKE name || '%')`. If siblings exist whose names share a prefix with the candidate, escalate to LLM disambiguation even on exact match — the extraction may have picked a close-but-wrong name from the domain graph.

**Stage 2 — LLM disambiguation:**

For entities that don't shortcircuit via stage 1:

1. Fetch top-N domain candidates of the same type (all entities of that type if N < 50, otherwise top-50 by name similarity).
2. Include chunk context: the prose from the source chunk(s) where this entity was extracted (via `kg_chunk_subjects` → `kg_chunks` → `search_content`).
3. LLM returns: `{"match": "skill:self-dev", "confidence": 0.85}` or `{"match": null, "confidence": 0.0}` (no_match).
4. Resolution confidence = min(extraction_confidence, LLM_confidence). See D4.

**Discovered types** (solution_path, failure_mode, pattern) skip resolution entirely — no domain counterpart exists. See D8.

### D2. Confidence scoring: min(extraction, disambiguation)

Resolved during planning. Resolution confidence is the minimum of the extraction confidence (from #690's `kg_subject_entities.confidence`) and the disambiguation confidence (from the LLM or 1.0 for unambiguous exact match).

- **Exact match, no ambiguity:** confidence = extraction_confidence (NOT always 1.0 — the extraction itself may have been uncertain).
- **LLM disambiguation:** confidence = min(extraction_confidence, llm_confidence).
- **No match:** no `kg_subject_resolutions` row. `kg_resolutions_log.outcome = 'no_match'`.

Rationale: the resolution chain is as confident as its weakest link. If extraction was uncertain (confidence 0.7), an exact match doesn't make it more certain — the extracted entity_key might itself be wrong.

### D3. LLM disambiguation prompt requirements

Resolved during planning. Resolution prompts must satisfy:

1. **"no_match" is a first-class response.** The prompt explicitly authorizes and provides examples of returning `{"match": null}`. Prompts that frame disambiguation as "which of these candidates is the match?" bias toward forced matching. Reframe as "does any candidate match? If not, return null."
2. **Chunk context included.** The disambiguation prompt includes the source prose (from the chunk where the entity was extracted) so the LLM can judge meaning-in-context, not just name similarity.
3. **Candidate list is bounded.** Max 50 candidates per call. For types with fewer than 50 domain entities (most cases — skill count is ~20, tool count is ~50), the full list is included.
4. **Output schema matches D1 stage 2:** `{"match": "<entity_key>" | null, "confidence": 0.0-1.0}`.
5. **Validation is structural.** Matched entity_key must exist in the candidate list. Confidence must be in range. Malformed JSON triggers C2.2 semantic retry.

Exact prompt wording deferred to implementation (empirical). The output contract and "no_match authorization" requirement are load-bearing.

### D4. `kg_resolutions_log` tracking table (schema amendment D16)

Resolved during planning. Dedicated tracking table, separate from `kg_extractions` — different keys (per-entity vs per-doc), different metadata, different invalidation triggers.

```sql
CREATE TABLE kg_resolutions_log (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    agent_id TEXT NOT NULL REFERENCES agents(id) ON DELETE CASCADE,
    subject_entity_id INTEGER NOT NULL REFERENCES kg_subject_entities(id) ON DELETE CASCADE,
    outcome TEXT NOT NULL CHECK (outcome IN (
        'matched_exact', 'matched_llm', 'no_match', 'skipped_discovered_type', 'error'
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

**Pending-resolution query (handles staleness from re-extraction):**

```sql
SELECT e.id, e.entity_key, e.type, e.confidence
FROM kg_subject_entities e
LEFT JOIN kg_resolutions_log r ON r.subject_entity_id = e.id AND r.agent_id = e.agent_id
WHERE e.agent_id = ?
  AND e.type IN ('skill', 'tool', 'agent', 'problem_type')
  AND (
    r.id IS NULL  -- never attempted
    OR r.source_extraction_trace_id != (
        SELECT cs.extraction_trace_id
        FROM kg_chunk_subjects cs
        WHERE cs.subject_entity_id = e.id
        ORDER BY cs.created_at DESC LIMIT 1
    )  -- extraction changed since resolution
  )
```

**Four staleness triggers:**

| Trigger | Detection | Handling |
|---------|-----------|----------|
| Re-extraction regenerates entity | `source_extraction_trace_id` mismatch | Pending query catches, re-resolve |
| Domain rebuild adds new entities | Previous `no_match` might now match | Not auto-detected; flag for periodic re-resolution |
| Resolution model upgrade | Previous `matched_llm`/`no_match` from different model | Operator action: `DELETE FROM kg_resolutions_log WHERE model != ?` |
| Domain entity deleted (CASCADE) | `kg_subject_resolutions` row deleted, `kg_resolutions_log` becomes stale | Pending query: outcome says `matched_*` but no resolution row → re-resolve |

Trigger 1 is automatically handled by the pending query. Triggers 2-4 require operator action or periodic re-resolution (deferred).

### D5. Per-doc follow-on execution, compound hook async

Resolved during planning.

**Background extraction+resolution flow (per doc):**

```
extract_document(doc) → writes entities
resolve_doc_entities(doc_entity_ids) → writes resolutions + tracking
(same tokio task, same trace_id)
```

**Compound hook:** extraction runs synchronously (~2-3s), resolution spawns as background task after extraction commits. The authoring agent gets immediate subject entities and FTS5 queryability; resolutions appear shortly after (~2-5s background). Matches C1.1's bounded-staleness pattern (writes commit, enrichment lags).

Rationale: compound hook resolution involves LLM calls for ~20-30% of entities. Synchronous resolution would add 2-5s latency for marginal immediate value — the authoring agent's most likely next query is "what did I just write about?" which is answered by FTS5/subject entities, not cross-layer traversal.

**Concurrent resolution LLM calls within a doc's resolve pass:** fire N concurrent disambiguation calls (N = entity count needing LLM, typically 1-5). Per-agent parallelism still bounds total server-wide concurrency.

### D6. Four-phase re-extraction flow (extends #690 D5)

Resolved during planning. When #690's three-phase re-extraction reconciliation runs (D5), resolution adds a fourth phase:

```
Phase 1: Capture previous_entity_ids, previous_relationship_ids
Phase 2: #689 reingest (delete old chunks, write new chunks)
Phase 3: #690 extract + reconcile subject graph (single transaction)
Phase 4: Re-resolve entities touched by re-extraction (outside transaction)
```

Phase 4: coarse re-resolution — all entities touched by re-extraction re-resolve, regardless of whether their content changed. The work is bounded (5-15 entities per doc) and the complexity of diffing isn't worth the savings.

Phase 4 failures follow C2.3 log-and-skip. Failed re-resolution leaves the entity without an updated resolution; pending query picks it up on next startup. Phase 3's commit does not depend on phase 4 succeeding.

### D7. Sole writer designation

Resolved during planning. The `SubjectEntityResolver` is the sole writer of:

- `kg_subject_resolutions` rows (subject → domain edges)
- `kg_resolutions_log` rows (resolution tracking)

No other code path writes to these tables. #690 (extraction) does not write resolutions. #687 (domain builder) does not write resolutions. #688 (query tool) is read-only.

### D8. Discovered types skip resolution — explicit deferral

Resolved during planning. Subject entities with discovered types (solution_path, failure_mode, pattern) skip resolution because no domain counterpart exists today.

This is an **explicit design choice**, not an inherent property. Cross-agent subject resolution (agent A's `solution_path:validate_before_write` = agent B's `solution_path:pre_persistence_validation`) is a real need that #692's self-knowledge queries will surface as a fragmentation limitation. Future work to address:

- Domain seeds for discovered types (add `solution_path:*`, `failure_mode:*`, `pattern:*` to the #687 ProblemType seed pattern)
- Cross-agent subject → subject clustering (LLM-based equivalence detection, new resolution type)

Both are deferred. #691 records `outcome = 'skipped_discovered_type'` in `kg_resolutions_log` so the deferral is visible in data, not silent.

### D9. LLM call budget: 20-30%, not 5%

Resolved during planning. The initial estimate of "<5% of subject entities need LLM" is optimistic. Subject extraction produces names from free prose; extraction naming won't always match domain canonical names, and sibling ambiguity (D1) escalates exact matches to LLM when similar domain entities exist.

Budget for 20-30% LLM disambiguation rate. At ~10-15 subject entities per doc across 200 docs per agent:
- 2000-3000 subject entities per agent
- 400-900 LLM calls per agent (20-30%)
- At `MIKA_KG_RESOLUTION_MODEL` pricing: ~$0.05-0.10 per agent

Measure actual rate in implementation. Tune the exact-match shortcircuit threshold (D1 step 2: extraction confidence > 0.9 AND no siblings) based on observed resolution quality.

## Open Questions

### Resolved During Planning

- Pipeline stages — two-stage, no fuzzy (see D1).
- Confidence scoring — min(extraction, disambiguation) (see D2).
- LLM prompt requirements — no_match authorized, chunk context, bounded candidates (see D3).
- Resolution tracking — `kg_resolutions_log` table, not column on `kg_subject_entities` (see D4).
- Execution model — per-doc follow-on, compound hook async (see D5).
- Re-extraction flow — four-phase extending #690 D5 (see D6).
- Sole writer — resolver owns `kg_subject_resolutions` + `kg_resolutions_log` (see D7).
- Discovered types — skip with explicit deferral (see D8).
- LLM call rate — budget 20-30% (see D9).

### Deferred to Implementation

- Exact LLM prompt wording for disambiguation (empirical).
- Resolution model default (`MIKA_KG_RESOLUTION_MODEL` default value).
- Sibling ambiguity threshold (prefix/suffix matching depth).
- Per-entity vs batched LLM calls (default per-entity; batching is an optimization if call rate is too high).
- Periodic re-resolution for staleness triggers 2-4 (deferred until observed).
- Cross-agent subject → subject clustering for discovered types (future ticket if #692 surfaces the need).
- Human-in-the-loop review queue for low-confidence resolutions (deferred — may surface via #692).
- Trace_id scheme: same trace_id across extraction+resolution in same pass, or separate per phase. Minor; pick one.

## Output Structure

```
crates/mika-agent/src/
├── db.rs                            # ADD: kg_resolutions_log migration (D16)
├── db/kg_schema.rs                  # ADD: column constants for kg_resolutions_log
└── kg/
    ├── mod.rs                       # MODIFY: add `pub mod entity_resolver;`
    ├── subject_extractor.rs         # MODIFY: call resolver after extraction
    └── entity_resolver.rs           # NEW: two-stage resolution pipeline

crates/mika-agent/tests/
└── kg/
    └── entity_resolver.rs           # NEW: resolution integration tests

docs/plans/
└── 2026-04-21-007-feat-entity-resolution-plan.md   # this file
```

## High-Level Technical Design

> *Directional guidance for review, not implementation specification.*

```rust
// crates/mika-agent/src/kg/entity_resolver.rs

/// Sole writer of kg_subject_resolutions and kg_resolutions_log.
///
/// Invariants:
/// - Reads from both kg_subject_entities and kg_entities.
/// - Writes only to kg_subject_resolutions and kg_resolutions_log.
/// - Does not modify kg_entities, kg_subject_entities, or any other table.
/// - All disambiguation calls use MIKA_KG_RESOLUTION_MODEL (per C2.1).
pub struct SubjectEntityResolver {
    db: AsyncDatabase,
    llm: Option<Arc<dyn LlmProvider>>,  // None if no resolution model configured
}

impl SubjectEntityResolver {
    /// Resolve entities produced by a single doc's extraction.
    /// Called as per-doc follow-on after extract_document().
    pub async fn resolve_doc_entities(
        &self,
        agent_id: &str,
        entity_ids: &[i64],
        trace_id: &str,
        extraction_trace_id: &str,
    ) -> Result<ResolutionStats> {
        // For each entity:
        //   1. Check type — skip discovered types (D8).
        //   2. Attempt exact match with sibling check (D1 stage 1).
        //   3. If ambiguous or unmatched → LLM disambiguation (D1 stage 2).
        //   4. Write kg_subject_resolutions (if matched).
        //   5. Write kg_resolutions_log (always — tracks outcome).
    }

    /// Resolve all pending entities for an agent (startup or re-resolution).
    pub async fn resolve_pending(&self, agent_id: &str) -> Result<BatchStats> {
        // Run pending query (D4).
        // Group by source_doc for context locality.
        // For each group: resolve_doc_entities().
    }
}
```

### Disambiguation prompt shape (directional, not final)

```
System: You are resolving an entity mention to a canonical knowledge graph node.
Given the entity extracted from prose and a list of candidate domain entities,
determine which candidate (if any) matches. Return null if NONE match —
do not force a pick.

Return JSON: {"match": "<entity_key>" | null, "confidence": 0.0-1.0}

User:
Extracted entity: skill:self_dev (confidence: 0.85)
Source prose: "...the self-dev skill handles autonomous implementation..."

Candidates:
- skill:self-dev — Main self-development orchestration
- skill:self-knowledge — Agent self-awareness queries
- skill:self-dev-webhook-qa — QA webhook handler for self-dev
- skill:self-dev-webhook-ci — CI webhook handler for self-dev
```

## Implementation Units

- [ ] **Unit 1: Schema amendment (D16 — kg_resolutions_log)**

**Goal:** Add `kg_resolutions_log` table to the KG schema migration.

**Requirements:** D4.

**Dependencies:** #686 schema (v25 base).

**Files:** `crates/mika-agent/src/db.rs`, `crates/mika-agent/src/db/kg_schema.rs`.

**Approach:** Fold into v25 (nothing shipped yet). Add to `migrate_v1()` for clean-slate convergence.

**Test scenarios:**
- Forward-test: migration applies cleanly.
- FK CASCADE: deleting a subject entity CASCADE-deletes its log row.
- UNIQUE constraint: second resolution attempt for same entity UPSERTs the log row.
- Outcome CHECK constraint rejects invalid values.

**Verification:** `cargo test -p mika-agent -- migrations`

---

- [ ] **Unit 2: Exact match with sibling ambiguity check**

**Goal:** Stage 1 of the resolution pipeline — case-insensitive exact match against `kg_entities`, with sibling ambiguity escalation.

**Requirements:** D1 stage 1.

**Dependencies:** Unit 1.

**Files:** `crates/mika-agent/src/kg/entity_resolver.rs`.

**Approach:**
1. `SELECT id, entity_key FROM kg_entities WHERE LOWER(entity_key) = LOWER(?)`.
2. If match: check sibling ambiguity — `SELECT COUNT(*) FROM kg_entities WHERE type = ? AND name != ? AND (name LIKE ? || '%' OR ? LIKE name || '%')`.
3. If match + extraction confidence > 0.9 + no siblings → resolve. Confidence = extraction_confidence.
4. If match + siblings or low confidence → escalate to LLM (Unit 3).
5. If no match → escalate to LLM (Unit 3).

**Test scenarios:**
- Exact match, no siblings → resolved at extraction confidence.
- Exact match, siblings exist → escalated to LLM.
- Exact match, extraction confidence < 0.9 → escalated to LLM.
- No match → escalated to LLM.
- Case difference (skill:Self-Dev vs skill:self-dev) → exact match succeeds.

**Verification:** Unit tests against test DB with seeded domain entities.

---

- [ ] **Unit 3: LLM disambiguation**

**Goal:** Stage 2 — send ambiguous/unmatched entities to the resolution LLM with chunk context and candidate list.

**Requirements:** D1 stage 2, D3, C2.1, C2.2.

**Dependencies:** Unit 2, LLM provider for resolution model.

**Files:** `crates/mika-agent/src/kg/entity_resolver.rs`.

**Approach:**
1. Fetch all domain entities of the same type as candidates (bounded to 50).
2. Fetch source chunk prose from `kg_chunk_subjects` → `kg_chunks` → `search_content`.
3. Build disambiguation prompt (D3 requirements: no_match authorized, chunk context, candidate descriptions).
4. LLM call with C2.2 retry. Validate response: matched entity_key must be in candidate list, confidence in range.
5. If match: write `kg_subject_resolutions` row. Confidence = min(extraction_confidence, llm_confidence).
6. If no_match: no resolution row. Log row with `outcome = 'no_match'`.
7. Write `kg_resolutions_log` row (always).

**Test scenarios:**
- LLM picks correct candidate → resolution written with combined confidence.
- LLM returns no_match → no resolution, log says no_match.
- LLM returns entity_key not in candidates → rejected, retry per C2.2.
- LLM returns malformed JSON → retry, then log-and-skip.
- No resolution model configured → skip LLM, exact-match-only mode.

**Verification:** Integration tests with `MockLlmProvider`.

---

- [ ] **Unit 4: Resolution tracking and pending query**

**Goal:** Write `kg_resolutions_log` rows, implement pending-resolution query with staleness detection.

**Requirements:** D4.

**Dependencies:** Units 2-3.

**Files:** `crates/mika-agent/src/kg/entity_resolver.rs`.

**Approach:**
- After each entity's resolution attempt (matched, no_match, skipped, or error), UPSERT `kg_resolutions_log` row.
- Pending query: LEFT JOIN against log, check for NULL (never attempted) or stale `source_extraction_trace_id`.
- Include `outcome` in log for observability queries.

**Test scenarios:**
- Fresh entity → no log row → pending.
- Resolved entity → log row with `matched_exact` → not pending.
- Re-extracted entity (new extraction_trace_id) → log stale → pending again.
- Zero-result resolution → log with `no_match` → not pending.
- Discovered type → log with `skipped_discovered_type` → not pending.

**Verification:** Integration tests covering all staleness triggers.

---

- [ ] **Unit 5: Per-doc follow-on integration with #690**

**Goal:** Wire resolution as a per-doc follow-on after extraction. Compound hook: spawn async. Background: inline.

**Requirements:** D5.

**Dependencies:** Units 2-4, #690 subject_extractor.

**Files:** `crates/mika-agent/src/kg/subject_extractor.rs` (call resolver), `crates/mika-agent/src/kg/entity_resolver.rs`.

**Approach:**
- Background path: after `extract_document()` writes entities, call `resolve_doc_entities(entity_ids)` inline. Same task, same trace_id.
- Compound hook path: after `extract_document()` commits, `tokio::spawn` resolution as background task. Compound write returns immediately.
- Pass `extraction_trace_id` to resolver for staleness tracking.

**Test scenarios:**
- Background extraction → resolution runs inline, both complete in same task.
- Compound hook → extraction sync, resolution async. Subject entities available immediately, resolutions appear shortly after.
- Resolution failure in compound hook → extraction committed, resolution pending for next startup.

**Verification:** Integration tests with mock.

---

- [ ] **Unit 6: Four-phase re-extraction integration (D6)**

**Goal:** Add phase 4 (re-resolution) to #690's three-phase re-extraction reconciliation.

**Requirements:** D6.

**Dependencies:** Units 4-5, #690 D5 reconciliation.

**Files:** `crates/mika-agent/src/kg/ingestion_orchestrator.rs` (add phase 4), `crates/mika-agent/src/kg/entity_resolver.rs`.

**Approach:** After phase 3 (extract + reconcile) commits, run `resolve_doc_entities()` on all entities touched by re-extraction. Coarse re-resolution — all touched entities, no diffing. Failures: C2.3 log-and-skip, pending for next startup.

**Test scenarios:**
- Doc edited, entity survives, resolution stale → re-resolved.
- Doc edited, new entity added → resolved for first time.
- Doc edited, entity orphaned → resolution CASCADE-deleted, log CASCADE-deleted.
- Phase 4 failure → phase 3 committed, resolution pending.

**Verification:** Integration test with doc edit triggering full four-phase flow.

## Error Handling & Edge Cases

| Scenario | Expected behavior |
|----------|------------------|
| No resolution model configured (`MIKA_KG_RESOLUTION_MODEL` unset, no fallback) | Exact-match-only mode. Entities that don't exact-match remain unresolved. Log `warn!(event="resolution_llm_disabled")`. |
| Domain graph empty (first boot, #687 hasn't run yet) | No candidates → all resolutions produce `no_match`. Tracked in log; re-resolved after next startup when domain graph exists. |
| Subject entity resolves to multiple domain entities | Valid — `UNIQUE(agent_id, subject_entity_id, domain_entity_id)` allows multiple rows. Example: `tool:search` could resolve to both `tool:search_memory` and `tool:search_skills` if LLM returns multiple matches. |
| Domain entity deleted after resolution | `kg_subject_resolutions` row CASCADE-deleted. `kg_resolutions_log` row persists with stale `matched_*` outcome. Pending query detects inconsistency on next run. |
| Concurrent resolution across agents | Each agent's resolution is independent (per-agent subject graph). No cross-agent locking needed. |
