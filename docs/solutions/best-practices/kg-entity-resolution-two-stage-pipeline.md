---
module: kg
date: 2026-04-22
last_updated: 2026-04-22
problem_type: best_practice
component: database
severity: medium
tags:
  - knowledge-graph
  - entity-resolution
  - llm-disambiguation
  - subject-graph
  - domain-graph
  - sqlite
applies_when:
  - Bridging extracted entities from one graph layer to canonical nodes in another
  - Resolving ambiguous LLM-extracted entity mentions to structured references
  - Implementing cross-layer graph traversal with confidence scoring
---

# Entity Resolution — Two-Stage Pipeline for Cross-Layer KG Bridging

## Context

After subject graph extraction (#690) populates `kg_subject_entities` with LLM-extracted entity mentions from prose, those entities exist in a per-agent silo — named by what the LLM extracted, not by canonical domain identifiers. Without resolution, subject-layer queries can't compose with domain-layer traversals, degrading the KG to two disconnected graphs.

The entity resolution module (#691) bridges subject graph entities to domain graph nodes via `kg_subject_resolutions`, enabling cross-layer queries like "which skills solve fabrication problems?" to traverse from subject mentions through domain edges.

## Guidance

### Two-stage pipeline: exact match + LLM disambiguation

**Stage 1 — Exact match** with confidence gating:
- Case-insensitive match: `SELECT id, entity_key FROM kg_entities WHERE LOWER(entity_key) = LOWER(?)`
- If match found AND extraction confidence > 0.9, resolve immediately at extraction confidence
- If no match or low confidence, escalate to LLM (stage 2)

**Stage 2 — LLM disambiguation** with bounded candidates:
- Fetch domain candidates via range query (`entity_key >= '{type}:' AND entity_key < '{type};'`) instead of LIKE to avoid underscore wildcard issues, bounded to 50 candidates
- Include source chunk prose for context (via `kg_chunk_subjects` -> `kg_chunks` -> `search_content`)
- LLM returns `{"match": "<entity_key>" | null, "confidence": 0.0-1.0}`
- Candidate validation uses case-insensitive comparison (`eq_ignore_ascii_case`) — LLMs may return different casing than the canonical entity_key
- `no_match` is an authorized first-class response — the prompt must not bias toward forced matching
- Combined confidence = `min(extraction_confidence, llm_confidence)` — the chain is only as confident as its weakest link

### Resolution tracking via `kg_resolutions_log`

Every resolution attempt writes a tracking row with outcome enum: `matched_exact`, `matched_llm`, `no_match`, `skipped_discovered_type`, `skipped_no_llm`, `error`. This enables:
- Pending-resolution detection (entities never attempted or stale from re-extraction)
- Staleness via `source_extraction_trace_id` mismatch
- Budget monitoring (actual LLM call rate vs expected 20-30%)

### Sole-writer contract

`SubjectEntityResolver` is the exclusive writer of `kg_subject_resolutions` and `kg_resolutions_log`. No other code path writes these tables. This orthogonality — read-both-write-own-edges — prevents race conditions and simplifies reasoning about data flow.

### Execution contexts

- **Startup:** `resolve_pending()` runs after extraction completes, resolving all pending entities
- **Compound hook:** resolution spawns as a `tokio::spawn` background task after extraction commits — the authoring agent gets immediate subject entities; resolutions appear shortly after
- **Re-extraction:** phase 4 of the four-phase flow re-resolves all entities touched by doc changes

### Semantic retry for LLM calls (C2.2)

When the disambiguation LLM returns malformed JSON, retry once with prompt reinforcement before log-and-skip. This matches the extraction module's retry pattern. Transport errors get 3 retries with exponential backoff; configuration errors (401/403) do not retry.

## Why This Matters

Without entity resolution, the knowledge graph has two disconnected subgraphs — a structural domain layer (from manifests) and a semantic subject layer (from prose). Resolution creates the cross-layer edges that make the KG queryable as a unified graph. The two-stage pipeline keeps LLM costs low (exact match handles 70-80% of entities) while preserving resolution quality for ambiguous cases.

## When to Apply

- When adding new graph layers that need cross-references to existing layers
- When implementing entity linking between LLM-extracted mentions and structured data
- When building confidence-scored resolution with staleness tracking
- When designing LLM-augmented data pipelines that need both fast-path and slow-path resolution

## Examples

**Exact match (high confidence):**
```
Subject entity: skill:self-dev (confidence: 0.95)
Domain entity:  skill:self-dev (exact match)
→ Resolution at confidence 0.95 (no LLM call)
```

**LLM disambiguation (low confidence or name mismatch):**
```
Subject entity: skill:self_dev (confidence: 0.85, underscore vs hyphen)
Domain candidates: skill:self-dev, skill:self-knowledge, skill:self-dev-webhook-qa
Source prose: "the self-dev skill handles autonomous implementation"
→ LLM returns: {"match": "skill:self-dev", "confidence": 0.92}
→ Resolution at confidence min(0.85, 0.92) = 0.85
```

**No match (discovered type):**
```
Subject entity: solution_path:validate_before_write (confidence: 0.90)
→ Skipped: discovered types have no domain counterpart (D8)
→ kg_resolutions_log.outcome = 'skipped_discovered_type'
```
