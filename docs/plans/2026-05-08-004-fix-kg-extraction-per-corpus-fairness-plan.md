# fix(kg/extraction): per-corpus fairness for subject-entity extraction

- **Issue:** mika issue#962
- **Type:** fix
- **Branch:** `fix/962/kg-extraction-subject-entity-extractor`

## Problem

Subject-entity extraction stalls on secondary corpora (mika-platform: 6 entities from 437 chunks, mika-cloud: 6 entities from 199 chunks). The extraction startup loop in `server/mod.rs:949-993` uses a **sequential left-to-right drain** with a shared budget:

```rust
let mut remaining_budget = budget;
for (corpus_idx, docs_root) in corpora_clone.into_iter().enumerate() {
    if remaining_budget == 0 { break; }  // ← starves remaining corpora
    let extractor = SubjectExtractor::new(db.clone(), llm.clone(), docs_root, None);
    match extractor.extract_pending(remaining_budget).await { ... }
}
```

The primary corpus (first in the `docs_roots` array) consumes the entire budget before secondary corpora get a turn. This is the **extraction-side sibling** of the resolution starvation bug fixed in #927.

## Root Cause

Architectural asymmetry between extraction and resolution:

| Aspect | Resolution (#927 — fixed) | Extraction (#962 — broken) |
|--------|--------------------------|---------------------------|
| Instance | Single `SubjectEntityResolver` with all `docs_root_hash` values | One `SubjectExtractor` per corpus, sequential |
| Budget distribution | Two-pass fairness allocation + round-robin interleaving | Left-to-right drain, first corpus wins |
| Code location | `server/mod.rs:1087-1103` → `entity_resolver.rs:944-1035` | `server/mod.rs:949-993` → `subject_extractor.rs:556+` |

## Fix Strategy

Apply the same fairness pattern proven in the resolver to the extraction path. Two options:

**Option A (chosen): Refactor the server-side startup loop to distribute budget fairly before calling per-corpus extractors.**

This keeps `SubjectExtractor` as a single-corpus abstraction (consistent with its sole-writer contract on per-`docs_root_hash` tables) and moves the fairness logic into the caller — the same separation the resolver uses between `server/mod.rs` (which gathers hashes) and `entity_resolver.rs` (which distributes internally).

**Option B (rejected): Refactor `SubjectExtractor` to accept multiple corpora internally.**

This would mirror the resolver's internal design but would break the extractor's simpler single-corpus abstraction and require more invasive changes to `get_pending_docs()`, `extract_doc()`, and the provenance-tracking transaction.

## Implementation Plan

### Step 1: Add `count_pending_docs()` to `SubjectExtractor`

**File:** `crates/mika-agent/src/kg/subject_extractor.rs`

Add a public method that returns the number of pending docs for this extractor's corpus without fetching them all:

```rust
pub async fn count_pending_docs(&self) -> Result<u32> {
    let docs_root_hash = self.docs_root_hash.clone();
    self.db.with_db(move |db| {
        let count: u32 = db.conn.query_row(
            "SELECT COUNT(DISTINCT c.source_doc_path)
             FROM kg_chunks c
             WHERE c.docs_root_hash = ?1
               AND NOT EXISTS (
                 SELECT 1 FROM kg_extractions e
                 WHERE e.docs_root_hash  = c.docs_root_hash
                   AND e.source_doc_path = c.source_doc_path
                   AND e.source_doc_hash = c.source_doc_hash
               )",
            rusqlite::params![docs_root_hash],
            |row| row.get(0),
        )?;
        Ok(count)
    }).await
}
```

This mirrors `count_pending_for_corpus()` in `entity_resolver.rs:952-975`.

### Step 2: Refactor the extraction startup loop in `server/mod.rs`

**File:** `crates/mika-agent/src/server/mod.rs` (lines 949-993)

Replace the sequential drain loop with a three-phase fair-distribution pattern:

**Phase 1 — Count pending docs per corpus:**
```rust
let mut corpus_counts: Vec<(usize, PathBuf, u32)> = Vec::new();
for (idx, docs_root) in corpora_clone.iter().enumerate() {
    let extractor = SubjectExtractor::new(db.clone(), llm.clone(), docs_root.clone(), None);
    let count = extractor.count_pending_docs().await.unwrap_or(0);
    corpus_counts.push((idx, docs_root.clone(), count));
}
```

**Phase 2 — Two-pass budget allocation** (same algorithm as `entity_resolver.rs:980-1017`):
```rust
let n = corpus_counts.iter().filter(|(_, _, c)| *c > 0).count() as u32;
if n == 0 { return; }
let base_share = budget / n;
// First pass: min(pending, base_share) per corpus
// Second pass: distribute remainder to hungry corpora
```

**Phase 3 — Execute extractors with per-corpus budgets:**
```rust
for (idx, docs_root, per_corpus_budget) in allocated.iter() {
    if *per_corpus_budget == 0 { continue; }
    let extractor = SubjectExtractor::new(db.clone(), llm.clone(), docs_root.clone(), None);
    match extractor.extract_pending(*per_corpus_budget).await { ... }
}
```

### Step 3: Add per-corpus logging to extraction stats

**File:** `crates/mika-agent/src/server/mod.rs`

Add `per_corpus_extracted` field to the extraction log event (mirrors `per_corpus_attempted` in the resolver tick):

```rust
info!(
    event = "subject_extraction_ready",
    agent_id = %agent_name_clone,
    per_corpus_extracted = ?per_corpus_stats, // HashMap<String, u32>
    total_docs_extracted = total_extracted,
    total_entities = total_entities,
    ...
);
```

### Step 4: Update CLAUDE.md signals

**File:** `crates/mika-agent/CLAUDE.md` (root)

Add Signal G for extraction per-corpus fairness (sibling of Signal F for resolution):

> **Signal G — extraction per-corpus fairness (#962).** `grep subject_extraction_ready server.log | jq '.per_corpus_extracted'` — the `per_corpus_extracted` JSON field shows doc-extraction counts per corpus per startup. After #962, all corpora with pending docs should show non-zero extractions, not just the primary.

### Step 5: Update the `MIKA_KG_BATCH_BUDGET` env var docs

**File:** `crates/mika-agent/CLAUDE.md` (root)

Update the `MIKA_KG_BATCH_BUDGET` docs to note that the budget is now distributed fairly across corpora for both extraction and resolution, not just resolution.

### Step 6: Tests

**File:** `crates/mika-agent/src/kg/subject_extractor.rs` (inline test module)

1. **Unit test: `count_pending_docs` returns correct count** — seed chunks with and without extraction markers, verify count matches.
2. **Integration test: fair distribution under constrained budget** — seed 3 corpora with 100, 50, and 25 pending docs respectively. Set budget=60. Verify all three corpora get non-zero allocation (≈20 each from first pass, then remainder redistributed).

The integration test should exercise the allocation algorithm directly (extract it into a helper function for testability) rather than requiring full LLM mocking.

## Out of Scope

- Resolver-side changes (already fair per #927)
- Domain graph changes (already shipped per #928)
- Changing the extraction trigger from startup-only to periodic tick (separate concern, would be a follow-up like #906 was for resolution)
- Changing array-order semantics — the fix makes array order irrelevant under normal budget conditions while preserving deterministic behavior when budget is insufficient for all corpora

## Acceptance Criteria (from issue)

- mika-platform corpus subject entities ≥ 50
- mika-cloud corpus subject entities ≥ 50
- Entities/chunk ratio for secondaries within 0.3-1.0× mika-skills' ratio
- After entities populate, allow 1-2 resolver-tick cycles, then re-evaluate milestone#19's R1/R2

## Risk Assessment

**Low risk.** The fairness algorithm is proven in the resolver (#927) and has been running in production since 2026-05-02. The extraction path is a direct port of the same pattern. The `SubjectExtractor` API is unchanged — only the caller's budget distribution logic changes.

## Files Changed

| File | Change |
|------|--------|
| `crates/mika-agent/src/kg/subject_extractor.rs` | Add `count_pending_docs()` method |
| `crates/mika-agent/src/server/mod.rs` | Refactor extraction startup loop (lines 949-993) |
| `crates/mika-agent/CLAUDE.md` | Add Signal G, update budget docs |
