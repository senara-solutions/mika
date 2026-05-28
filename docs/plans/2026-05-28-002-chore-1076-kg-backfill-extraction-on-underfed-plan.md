# Plan: chore(kg): backfill extraction on underfed corpora

**Issue:** mika#1076
**Type:** chore (bug-class behavior)
**Branch:** `chore/1076/kg-backfill-extraction-on-underfed`

## Problem

Two of mika-arch's four KG corpora sit at ~30% extraction coverage despite 16+ days of continuous extraction ticks (since 2026-04-25). The other two corpora (92%, 75%) are healthy. Budget fairness (#962) is correct — `allocate_fair_budget` distributes budget proportionally. The total pending count (~110 docs) is well within the default budget (500), so budget exhaustion is not the cause.

## Root Cause Analysis

`extract_document()` in `subject_extractor.rs` has four early-exit paths that return `Ok(ExtractionStats::default())` **without writing a `kg_extractions` idempotency marker**:

| Exit path | Line | LLM call? | Marker written? | Doc stays pending? |
|-----------|------|-----------|-----------------|-------------------|
| Empty doc content | 683-685 | No | **No** | **Yes — forever** |
| No chunks found | 689-698 | No | **No** | **Yes — forever** |
| LLM returned nothing usable | 710-716 | Yes | **No** | **Yes — forever** |
| Validation failed | 726-736 | Yes | **No** | **Yes — forever** |

The caller `extract_pending()` (line 944-960) counts these as `docs_extracted += 1` and debits the budget (line 965), but since no `kg_extractions` row is written, the doc remains pending on the next tick. This creates **zombie docs** — permanently pending, permanently consuming budget, permanently blocking coverage convergence.

### Why Underfed Corpora Are Specifically Affected

The smaller corpora (likely mika-skills and mika-cloud `docs/solutions/`) may have:
- Higher proportion of short/structural docs that produce empty LLM output
- Docs whose content doesn't match the extraction prompt's expected patterns
- Path resolution mismatches between what `kg_chunks.source_doc_path` stores and what `resolve_doc_path()` constructs

The larger primary corpus (mika/docs/solutions with 427 docs) has enough "good" docs to reach 92% even if some zombie docs exist. The smaller corpora can't absorb the same zombie ratio — 30% of 63 docs is only 19 successfully extracted.

### Evidence Required to Confirm

Before implementing, confirm the hypothesis with these diagnostic queries (run against mika-arch's SQLite DB):

```sql
-- 1. Per-corpus pending breakdown (should show the underfed corpora)
SELECT c.docs_root_hash,
       COUNT(DISTINCT c.source_doc_path) as pending
FROM kg_chunks c
WHERE NOT EXISTS (
    SELECT 1 FROM kg_extractions e
    WHERE e.docs_root_hash = c.docs_root_hash
      AND e.source_doc_path = c.source_doc_path
      AND e.source_doc_hash = c.source_doc_hash
)
GROUP BY c.docs_root_hash;

-- 2. Cross-check: are any pending docs also in kg_extractions with a stale hash?
SELECT e.docs_root_hash, e.source_doc_path, e.source_doc_hash as extraction_hash,
       c.source_doc_hash as chunk_hash
FROM kg_extractions e
JOIN kg_chunks c ON e.docs_root_hash = c.docs_root_hash
                AND e.source_doc_path = c.source_doc_path
WHERE e.source_doc_hash != c.source_doc_hash
GROUP BY e.docs_root_hash, e.source_doc_path;

-- 3. Log analysis: look for extraction_doc_failed, extraction_no_chunks,
--    extraction_validation_failed events for the underfed corpus hashes
```

Also check server logs:
```bash
grep -E 'extraction_(doc_failed|no_chunks|validation_failed)' $MIKA_SERVER_LOG_FILE | \
  jq 'select(.agent_id == "mika-arch")' | \
  jq -r '.doc' | sort | uniq -c | sort -rn | head -20
```

## Implementation

### Step 1: Write skip markers for non-extractable docs

**File:** `crates/mika-agent/src/kg/subject_extractor.rs`

Add a `record_skip_marker()` method that writes a `kg_extractions` row for docs that can't be extracted, using the current `source_doc_hash` so they won't be retried until content changes:

```rust
/// Write a skip marker for a doc that cannot be extracted.
///
/// Uses the same `kg_extractions` upsert as `write_extraction_results`
/// but with zero entities/relationships. The doc becomes non-pending
/// (hash matches) and will only re-enter the pending set if the
/// content changes (new hash from re-ingestion).
async fn record_skip_marker(
    &self,
    doc_path: &str,
    reason: &str,
) -> Result<()>
```

This method:
1. Reads the current `source_doc_hash` from `kg_chunks` via `get_doc_hash()`
2. Writes an `kg_extractions` row with `entities_extracted=0, relationships_extracted=0`
3. Uses the same `ON CONFLICT DO UPDATE` upsert pattern
4. Includes the skip reason in `extraction_trace_id` or a new field for diagnostics

### Step 2: Call `record_skip_marker` from early-exit paths

Modify the four early-exit paths in `extract_document()`:

**Empty doc (line 683-685):**
```rust
if doc_text.trim().is_empty() {
    self.record_skip_marker(doc_path, "empty_content").await
        .unwrap_or_else(|e| warn!(/* ... */));
    return Ok(ExtractionStats::default());
}
```

**No chunks (line 689-698):**
```rust
if chunks.is_empty() {
    // No chunks = doc won't appear in pending query (kg_chunks is the source),
    // so no skip marker needed. But log for diagnostics.
    warn!(/* existing log */);
    return Ok(ExtractionStats::default());
}
```

Note: The no-chunks path doesn't need a skip marker because `get_pending_docs` queries `kg_chunks` — if there are no chunks, the doc won't appear in the pending set. This path only fires in a race condition where chunks were deleted between the pending query and the extraction attempt.

**LLM returned nothing usable (line 710-716):**
```rust
None => {
    self.record_skip_marker(doc_path, "llm_empty_response").await
        .unwrap_or_else(|e| warn!(/* ... */));
    return Ok(ExtractionStats::default());
}
```

**Validation failed (line 726-736):**
```rust
Err(e) => {
    self.record_skip_marker(doc_path, &format!("validation_failed: {e}")).await
        .unwrap_or_else(|e2| warn!(/* ... */));
    warn!(/* existing log */);
    return Ok(ExtractionStats::default());
}
```

### Step 3: Fix budget accounting for no-LLM-call paths

**File:** `crates/mika-agent/src/kg/subject_extractor.rs`

The current budget accounting (line 965) increments `llm_calls` for EVERY doc, even those that exit before making an LLM call. This over-debits the budget. Fix by tracking whether an LLM call was actually made:

Move the `llm_calls` increment inside the match block, after the extraction attempt, and only increment when the doc actually made an LLM call (i.e., didn't hit the empty-doc or no-chunks early exits). The simplest approach:

Change `extract_document` to return a richer result type that indicates whether an LLM call was made, or add a `made_llm_call: bool` field to `ExtractionStats`. Then only debit budget for docs that actually consumed LLM capacity.

```rust
pub struct ExtractionStats {
    // ... existing fields ...
    /// Whether this extraction made at least one LLM API call.
    pub made_llm_call: bool,
}
```

Update `extract_pending` budget accounting:
```rust
if doc_stats.made_llm_call {
    stats.llm_calls = stats.llm_calls.saturating_add(1);
}
```

### Step 4: Add structured logging for skip-marker writes

Add an `extraction_skipped` log event for each skip marker written, including the corpus hash, doc path, and reason. This enables post-deploy verification via:

```bash
grep extraction_skipped $MIKA_SERVER_LOG_FILE | \
  jq -r '[.docs_root_hash, .reason] | @tsv' | sort | uniq -c
```

### Step 5: Add observability for zombie-doc detection

**File:** `crates/mika-agent/src/kg/resolver_tick.rs` (in `tick_coverage`)

Extend the coverage report to include the count of docs that are "covered but with zero entities" — this helps operators distinguish between genuinely-empty docs and docs that were skip-marked:

```sql
SELECT COUNT(*)
FROM kg_extractions
WHERE docs_root_hash = ?1
  AND source_doc_hash IS NOT NULL
  AND entities_extracted = 0
```

Emit as `zero_entity_docs` in the `kg_extraction_coverage` log event.

### Step 6: Tests

1. **Unit test: skip marker written on empty doc** — Create a `SubjectExtractor` with a test DB containing chunks for an empty doc. Call `extract_document`. Verify `kg_extractions` row exists with `entities_extracted=0`.

2. **Unit test: skip marker written on validation failure** — Mock an LLM that returns invalid JSON. Call `extract_document`. Verify `kg_extractions` row exists.

3. **Unit test: skip-marked doc is no longer pending** — Write a skip marker via `record_skip_marker`. Call `count_pending_docs`. Verify count is 0 for that doc.

4. **Unit test: content change re-triggers extraction** — Write a skip marker. Update `kg_chunks.source_doc_hash` to a new value. Call `count_pending_docs`. Verify count is 1.

5. **Unit test: budget not debited for no-LLM-call paths** — Extract a batch where the first doc is empty. Verify `BatchStats.llm_calls` is 0 for that doc.

6. **Integration test (eval harness):** Add a scenario to `tests/eval/` that seeds chunks for three corpora with different failure profiles and verifies extraction converges to 100% within a bounded number of ticks.

## Files Changed

| File | Change |
|------|--------|
| `crates/mika-agent/src/kg/subject_extractor.rs` | Add `record_skip_marker()`, call from early-exit paths, add `made_llm_call` to `ExtractionStats`, fix budget accounting |
| `crates/mika-agent/src/kg/resolver_tick.rs` | Extend `tick_coverage` with zero-entity-docs count |
| `crates/mika-agent/src/kg/subject_extractor.rs` (tests) | Unit tests for skip marker and budget accounting |

## Verification

### Pre-deploy diagnostic
Run the SQL queries from the "Evidence Required" section to confirm the zombie-doc hypothesis before deploying.

### Post-deploy signals
1. **Signal H convergence:** `grep kg_extraction_tick.complete server.log | jq 'select(.total_pending == 0)'` — `total_pending` should reach 0 within 1-2 ticks (skip markers written for all zombie docs).
2. **Extraction coverage:** `grep kg_extraction_coverage server.log | jq '.per_corpus_coverage'` — all four corpora should show `pct >= 85` (per AC) within 1-2 ticks.
3. **Skip-marker audit:** `grep extraction_skipped server.log` — should show the skip reasons for previously-zombie docs. Count should match the gap between current coverage and 100%.
4. **No regression on healthy corpora:** The primary corpus (92%) should not see its coverage drop or its extraction behavior change.

### Acceptance Criteria Verification
- [ ] ≥85% extraction coverage on `ac0e96dc51b85b80` and `d7107cd14e544043` — verified via `kg_extraction_coverage` log or `mika kg status`
- [ ] Root cause identified: zombie docs cycling through early-exit paths without writing idempotency markers
- [ ] Fix shipped: skip markers prevent infinite cycling; budget accounting fixed for no-LLM paths

## Risk Assessment

**Low risk.** Changes are additive (new skip-marker writes, new log events) and don't alter the extraction flow for docs that currently succeed. The `kg_extractions` upsert uses the same `ON CONFLICT` pattern already proven in production. Skip-marked docs automatically re-enter the pending set if content changes (hash-mismatch detection preserved).

**Edge case:** If a doc's LLM extraction fails transiently (network error), skip-marking it would mean it doesn't retry until content changes. This is acceptable because: (a) the C2.2 retry taxonomy already retries transport errors 3 times within a single `extract_document` call, (b) truly transient failures that survive 3 retries are rare, and (c) the doc will be re-extracted on any content update. For the LLM-empty-response and validation-failure paths, the failure is likely content-dependent and won't resolve without a content change anyway.
