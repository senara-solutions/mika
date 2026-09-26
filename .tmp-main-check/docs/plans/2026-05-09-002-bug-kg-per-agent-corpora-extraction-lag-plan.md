# Plan: Fix KG Per-Agent Corpora Extraction Lag

**Ticket:** mika issue#1052
**Type:** bug
**Branch:** `bug/1052/kg-mika-arch-per-agent-corpora`
**Date:** 2026-05-09

## Problem

Three mika-arch-only KG corpora are at 16-71% extraction coverage after 2 weeks. The shared `34b8cf03` corpus is at 89%. The odds-engine corpus (100%) proves the pipeline can drain to completion.

## Root Cause Analysis

**Primary cause: No periodic extraction — only startup + compound hooks.**

The extraction pipeline has a structural gap:

| Trigger | Runs extraction? | Runs resolution? | Frequency |
|---------|-----------------|------------------|-----------|
| Server startup | Yes | Yes | On deploy/restart |
| Compound hook | Yes (1 doc) | Yes (async) | On doc write |
| 30-min resolver tick | **No** | Yes | Every 30 min |

When startup extraction hits budget limits (default 500 LLM calls) or encounters transient LLM errors, remaining docs stay pending until the next restart. The 30-min tick only resolves already-extracted entities — it never picks up unextracted docs.

**Why mika-arch-only corpora are worse:**
- The shared corpus (`34b8cf03`) benefits from extraction attempts by both `mika` and `mika-arch` agents at their respective startups
- mika-arch-only corpora (`98509090`, `ac0e96dc`, `d7107cd1`) only get extraction from mika-arch's startup
- Fewer restart cycles = fewer extraction opportunities = slower coverage convergence

**Secondary cause: NULL source_doc_hash rows from pre-v26 era.**
Three rows from 2026-04-22 have `source_doc_hash IS NULL`. The v27 backfill should have populated them but the pending-doc detection query uses equality (`e.source_doc_hash = c.source_doc_hash`), and `NULL = NULL` is `NULL` (falsy in SQL). These rows are perpetually "already extracted" from the query's perspective — they'll never be re-extracted because the `INSERT OR IGNORE` on the unique key `(docs_root_hash, source_doc_path)` sees the existing row and skips.

Wait — actually the pending query checks `NOT EXISTS (... AND e.source_doc_hash = c.source_doc_hash)`. If `e.source_doc_hash IS NULL`, the equality `NULL = <hash>` is NULL → the NOT EXISTS subquery finds no matching row → the doc IS treated as pending. So the NULL rows should trigger re-extraction. But re-extraction would hit `INSERT OR IGNORE` on the unique key and silently skip (the old NULL-hash row already occupies the slot). The extraction marker is never updated.

**This is the NULL-hash deadlock:** the pending query says "extract me" but the insert says "already done." The doc cycles between "pending" and "skip" on every startup, consuming a budget slot each time but never making progress.

## Implementation Plan

### Change 1: Add periodic extraction to the resolver tick

**File:** `crates/mika-agent/src/kg/resolver_tick.rs`
**File:** `crates/mika-agent/src/server/mod.rs`

Add extraction as a first phase of the periodic tick, before resolution. The tick becomes an "extraction + resolution" tick:

```
Every 30 min:
  1. Count pending docs per corpus
  2. If pending > 0: run extract_pending(budget) with fair allocation
  3. Run resolve_pending(budget) as before
```

Budget for extraction in the tick should use the same `MIKA_KG_BATCH_BUDGET` env var. The tick's extraction is structurally identical to the startup extraction — same `SubjectExtractor::extract_pending()` call, same fair budget allocation via `allocate_fair_budget()`.

**Observability:** Log `kg_extraction_tick.complete` event with `per_corpus_extracted` map, mirroring the startup extraction log shape.

**Why this fixes the primary cause:** Corpora that don't fully drain at startup get 48 more extraction opportunities per day (every 30 min). Even with conservative budgets, 48 × budget/N_corpora will drain any reasonable backlog within hours, not weeks.

### Change 2: Fix NULL source_doc_hash deadlock

**File:** `crates/mika-agent/src/kg/subject_extractor.rs`

In `extract_document()`, after successful extraction, change the marker write from `INSERT OR IGNORE` to `INSERT OR REPLACE` (or use an `UPDATE ... SET source_doc_hash = ?` followed by `INSERT OR IGNORE` for new rows).

More precisely, use an upsert pattern:
```sql
INSERT INTO kg_extractions
    (docs_root_hash, docs_root, source_doc_path, source_doc_hash,
     extraction_model, entities_extracted, relationships_extracted,
     extraction_trace_id)
VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
ON CONFLICT(docs_root_hash, source_doc_path) DO UPDATE SET
    source_doc_hash = excluded.source_doc_hash,
    extraction_model = excluded.extraction_model,
    entities_extracted = excluded.entities_extracted,
    relationships_extracted = excluded.relationships_extracted,
    extraction_trace_id = excluded.extraction_trace_id,
    created_at = excluded.created_at
WHERE kg_extractions.source_doc_hash IS NULL
   OR kg_extractions.source_doc_hash != excluded.source_doc_hash;
```

The `WHERE` clause ensures:
- NULL-hash rows get updated (fixes the deadlock)
- Content-changed docs get re-extracted (hash mismatch)
- Identical-content re-extractions are no-ops (hash matches, no update)

This preserves the first-writer-wins semantics for normal operation while fixing the NULL-hash edge case.

### Change 3: One-time backfill for existing NULL-hash rows

**File:** `crates/mika-agent/src/db/kg_schema.rs` (or a new migration)

Add a startup migration step that deletes the three known NULL-hash rows so they're cleanly re-extracted on the next cycle:

```sql
DELETE FROM kg_extractions WHERE source_doc_hash IS NULL;
```

This is safe because:
- The extracted entities/relationships are in separate tables and keyed differently
- The `kg_extractions` row is an idempotency marker, not the data itself
- Deleting it makes the doc "pending" again, which triggers a clean re-extraction with the upsert from Change 2

### Change 4: Extraction coverage observability

**File:** `crates/mika-agent/src/kg/subject_extractor.rs` (or new module)

Add a `coverage_report()` method that returns per-corpus extraction coverage stats:

```rust
pub struct CorpusCoverage {
    pub docs_root_hash: String,
    pub total_docs: u32,      // count from kg_chunks DISTINCT source_doc_path
    pub extracted_docs: u32,  // count from kg_extractions with non-NULL hash
    pub null_hash_docs: u32,  // count from kg_extractions with NULL hash
    pub coverage_pct: f64,
}
```

Log this at startup after extraction completes, and at the end of each tick. Format as the `kg_extraction_coverage` event with the corpus coverage map.

**Why:** The ticket's investigation required manual SQL queries. Structured logging makes coverage monitoring automatic and dashboardable.

### Change 5: Document extraction trigger semantics

**File:** `docs/solutions/architecture-patterns/kg-extraction-trigger-semantics-<date>.md`

Document the three extraction triggers (startup, compound hook, periodic tick), their budget allocation, and the expected drain rate. Per acceptance criteria.

## File Change Summary

| File | Change |
|------|--------|
| `crates/mika-agent/src/kg/resolver_tick.rs` | Add extraction phase before resolution in the periodic tick |
| `crates/mika-agent/src/server/mod.rs` | Extract shared extraction logic into reusable function callable from both startup and tick |
| `crates/mika-agent/src/kg/subject_extractor.rs` | Change `INSERT OR IGNORE` to `ON CONFLICT ... DO UPDATE` upsert; add `coverage_report()` |
| `crates/mika-agent/src/db/kg_schema.rs` | Add NULL-hash cleanup migration step |
| `docs/solutions/architecture-patterns/` | New doc on extraction trigger semantics |

## Testing

1. **Unit test for upsert behavior:** Create a NULL-hash extraction row, then extract the same doc. Verify the row is updated with the correct hash.
2. **Unit test for tick extraction:** Mock the tick and verify it calls `extract_pending()` before `resolve_pending()`.
3. **Integration test for coverage_report:** Seed a corpus with mixed extracted/pending docs, verify coverage percentages.
4. **Manual verification:** After deploy, query `kg_extractions` to confirm:
   - Zero rows with `source_doc_hash IS NULL`
   - All five corpora at >= 80% coverage within 1-2 tick cycles
   - PR #95 docs appear in `kg_extractions`

## Risks

- **Budget doubling:** Adding extraction to the tick means extraction runs more frequently. With the default 500 budget and 30-min ticks, that's up to 24,000 extraction LLM calls/day per agent. Mitigation: the budget is shared across extraction + resolution, and the fair allocation only allocates budget proportional to pending work. Once coverage reaches 100%, the tick's extraction phase is a no-op (zero pending = zero budget allocated).
- **Upsert vs first-writer-wins:** The upsert changes the semantics from "first writer wins forever" to "first writer wins unless hash changes." This is actually more correct — if a doc's content changes, re-extraction is desirable. The `WHERE` clause prevents unnecessary updates when content hasn't changed.

## Sequence

Changes 1-4 can be implemented in a single commit. Change 5 (docs) in a follow-up commit.

## Out of Scope

Per ticket:
- High `no_match` resolution rate (defer until extraction is healthy)
- New extraction features (parallelism, faster batch)
- Mempalace alternatives
