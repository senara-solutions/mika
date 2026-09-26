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

## Pinned Algorithm: Two-Pass Fair Budget Allocation

The resolver's fairness algorithm at `entity_resolver.rs:980-1017` (verbatim from production):

```rust
// 2. First pass: assign each corpus min(pending_count, budget / N).
let base_share = budget / n;
let mut assigned: Vec<u32> = corpus_counts
    .iter()
    .map(|(_, count)| (*count).min(base_share))
    .collect();

// 3. Compute remaining budget.
let used: u32 = assigned.iter().sum();
let mut remaining = budget.saturating_sub(used);

// 4. Second pass: distribute remaining to corpora with surplus pending.
if remaining > 0 {
    let mut hungry: Vec<usize> = corpus_counts
        .iter()
        .enumerate()
        .filter(|(i, (_, count))| *count > assigned[*i])
        .map(|(i, _)| i)
        .collect();

    while remaining > 0 && !hungry.is_empty() {
        let share = (remaining / hungry.len() as u32).max(1);
        let mut next_hungry = Vec::new();
        for &idx in &hungry {
            if remaining == 0 { break; }
            let can_take = corpus_counts[idx].1.saturating_sub(assigned[idx]);
            let give = share.min(can_take).min(remaining);
            assigned[idx] += give;
            remaining -= give;
            if assigned[idx] < corpus_counts[idx].1 {
                next_hungry.push(idx);
            }
        }
        hungry = next_hungry;
    }
}
```

**Algorithm invariants:**
- **Postcondition:** `assigned.iter().sum() == budget` (when total pending ≥ budget) or `assigned.iter().sum() == total_pending` (when total pending < budget).
- **Floor guarantee:** Every corpus with pending > 0 gets at least `min(pending, budget/N)` (first pass).
- **Remainder redistribution:** Unused first-pass slots go proportionally to "hungry" corpora (those with pending > assigned).
- **Convergence:** The `while` loop terminates because either `remaining` decreases or `hungry` shrinks each iteration.
- **Empty-corpus handling:** Corpora with 0 pending get 0 allocation in pass 1 (via `.min(base_share)` where count=0), freeing their share for redistribution.

**Extraction adaptation:** The algorithm is identical — only the input type changes from `(docs_root_hash, pending_entity_count)` to `(corpus_index, pending_doc_count)`. This is why it belongs in a shared function (Step 1).

## Implementation Plan

### Step 1: Extract shared `allocate_fair_budget()` to `kg::budget` module

**New file:** `crates/mika-agent/src/kg/budget.rs`

Extract the two-pass allocation algorithm into a shared pure function. Both the resolver and the extraction caller use it.

```rust
//! Budget allocation for KG pipeline stages.
//!
//! Shared between extraction (startup) and resolution (startup + tick).
//! The algorithm distributes a total budget across N corpora with pending
//! work, using two-pass fair allocation (per mika#927, mika#962).

/// Per-corpus pending count and allocation result.
pub struct CorpusBudget {
    pub pending: u32,
    pub allocated: u32,
}

/// Distribute `total_budget` fairly across corpora with pending work.
///
/// Two-pass algorithm:
/// - Pass 1: Each corpus gets `min(pending, total_budget / N_active)`.
/// - Pass 2: Redistribute unused slots to "hungry" corpora (pending > allocated).
///
/// Postconditions:
/// - `sum(allocated) == min(total_budget, sum(pending))`
/// - Every corpus with `pending > 0` gets `allocated > 0` (when budget > 0 and N_active > 0)
///
/// Returns a Vec of allocated budgets parallel to the input `pending_counts`.
pub fn allocate_fair_budget(pending_counts: &[u32], total_budget: u32) -> Vec<u32> {
    let n = pending_counts.iter().filter(|&&c| c > 0).count() as u32;
    if n == 0 || total_budget == 0 {
        return vec![0; pending_counts.len()];
    }

    // Pass 1: floor allocation.
    let base_share = total_budget / n;
    let mut assigned: Vec<u32> = pending_counts
        .iter()
        .map(|&count| count.min(base_share))
        .collect();

    // Remainder after floor allocation.
    let used: u32 = assigned.iter().sum();
    let mut remaining = total_budget.saturating_sub(used);

    // Pass 2: redistribute remainder to hungry corpora.
    if remaining > 0 {
        let mut hungry: Vec<usize> = pending_counts
            .iter()
            .enumerate()
            .filter(|(i, &count)| count > assigned[*i])
            .map(|(i, _)| i)
            .collect();

        while remaining > 0 && !hungry.is_empty() {
            let share = (remaining / hungry.len() as u32).max(1);
            let mut next_hungry = Vec::new();
            for &idx in &hungry {
                if remaining == 0 { break; }
                let can_take = pending_counts[idx].saturating_sub(assigned[idx]);
                let give = share.min(can_take).min(remaining);
                assigned[idx] += give;
                remaining -= give;
                if assigned[idx] < pending_counts[idx] {
                    next_hungry.push(idx);
                }
            }
            hungry = next_hungry;
        }
    }

    assigned
}
```

Register the module in `crates/mika-agent/src/kg/mod.rs`:
```rust
pub mod budget;
```

### Step 2: Refactor the resolver to use `allocate_fair_budget()`

**File:** `crates/mika-agent/src/kg/entity_resolver.rs` (lines 980-1017)

Replace the inline two-pass allocation in `get_pending_entities()` with a call to the shared function:

```rust
use super::budget::allocate_fair_budget;

// Inside get_pending_entities():
let pending_counts: Vec<u32> = corpus_counts.iter().map(|(_, count)| *count).collect();
let assigned = allocate_fair_budget(&pending_counts, budget);
```

This is a pure refactor — no behavioral change to resolution. **Verification gate:** all existing resolver tests must pass unchanged after refactoring to use `allocate_fair_budget()`. If any fail, the shared function's behavior diverges from the inline original — stop and reconcile before proceeding to Step 4.

### Step 3: Add `count_pending_docs()` to `SubjectExtractor`

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

### Step 4: Refactor the extraction startup loop in `server/mod.rs`

**File:** `crates/mika-agent/src/server/mod.rs` (lines 949-993)

Replace the sequential drain loop with a three-phase fair-distribution pattern:

**Phase 1 — Count pending docs per corpus:**
```rust
let mut corpus_pending: Vec<u32> = Vec::new();
let mut extractors: Vec<SubjectExtractor> = Vec::new();
for docs_root in &corpora_clone {
    let extractor = SubjectExtractor::new(db.clone(), llm.clone(), docs_root.clone(), None);
    let count = extractor.count_pending_docs().await.unwrap_or(0);
    corpus_pending.push(count);
    extractors.push(extractor);
}
```

**Phase 2 — Fair budget allocation via shared function:**
```rust
use crate::kg::budget::allocate_fair_budget;
let allocated = allocate_fair_budget(&corpus_pending, budget);
```

**Phase 3 — Execute extractors with per-corpus budgets:**
```rust
let mut per_corpus_stats: BTreeMap<String, u32> = BTreeMap::new();
for (idx, (extractor, per_budget)) in extractors.into_iter().zip(allocated.iter()).enumerate() {
    if *per_budget == 0 { continue; }
    match extractor.extract_pending(*per_budget).await {
        Ok(stats) => {
            per_corpus_stats.insert(
                corpora_clone[idx].display().to_string(),
                stats.docs_extracted as u32,
            );
            // ... existing stats aggregation
        }
        Err(e) => { /* existing error handling */ }
    }
}
```

### Step 5: Add per-corpus logging to extraction stats

**File:** `crates/mika-agent/src/server/mod.rs`

Add `per_corpus_extracted` field to the extraction log event. `per_corpus_extracted` serialized via `BTreeMap` for deterministic log output (per mika#927 F6 convention):

```rust
info!(
    event = "subject_extraction_ready",
    agent_id = %agent_name_clone,
    per_corpus_extracted = %serde_json::to_string(&per_corpus_stats).unwrap_or_default(),
    total_docs_extracted = total_extracted,
    total_entities = total_entities,
    ...
);
```

### Step 6: Update CLAUDE.md signals

**File:** `crates/mika-agent/CLAUDE.md` (root CLAUDE.md, not crate-level)

Add Signal G for extraction per-corpus fairness (sibling of Signal F for resolution):

> **Signal G — extraction per-corpus fairness (#962).** `grep subject_extraction_ready server.log | jq '.per_corpus_extracted'` — the `per_corpus_extracted` JSON field shows doc-extraction counts per corpus per startup. After #962, all corpora with pending docs should show non-zero extractions, not just the primary. If a secondary corpus shows 0 extractions while having pending docs, the fairness allocation is broken. **Architectural note:** Extraction fairness is caller-side (`server/mod.rs` distributes budget to per-corpus `SubjectExtractor` instances); resolution fairness is internal (`SubjectEntityResolver` distributes budget across corpora within a single instance). This asymmetry is intentional — the extractor's provenance transaction is per-doc-per-corpus and would be invasively complex to multi-corpus-ify (see mika#962 plan).

### Step 7: Update the `MIKA_KG_BATCH_BUDGET` env var docs

**File:** `crates/mika-agent/CLAUDE.md` (root CLAUDE.md)

Update the `MIKA_KG_BATCH_BUDGET` docs to note that the budget is now distributed fairly across corpora for both extraction and resolution, not just resolution.

### Step 8: Tests

**File:** `crates/mika-agent/src/kg/budget.rs` (shared module tests)

Unit tests on the shared `allocate_fair_budget()` function:

1. **Single corpus** — all budget goes to the sole corpus.
2. **Equal pending** — budget split evenly.
3. **Unequal pending** — first pass gives floor, second pass redistributes to hungry corpora.
4. **Budget exceeds total pending** — each corpus gets exactly its pending count, no waste.
5. **Budget is zero** — all allocations are zero.
6. **Empty input** — returns empty vec.
7. **One corpus has zero pending** — its share redistributed to others.
8. **Three corpora (100, 50, 25), budget=60** — all three get non-zero allocation. First pass: 20 each (20, 20, 20) but corpus 3 only has 25 so gets 20. Second pass: 0 remaining → done. Actual: (20, 20, 20) with corpus 3 capped at 20 from its 25.

**File:** `crates/mika-agent/src/kg/subject_extractor.rs` (inline test module)

9. **`count_pending_docs` returns correct count** — seed chunks with and without extraction markers, verify count matches.

**File:** `crates/mika-agent/src/kg/entity_resolver.rs` (inline test module)

10. **Existing resolver tests still pass after refactor** — no behavioral change from using the shared function.

## Out of Scope

- Resolver-side changes (already fair per #927; refactored to use shared function but no behavioral change)
- Domain graph changes (already shipped per #928)
- Periodic extraction tick (sibling to #906's resolution tick) — follow-up ticket to be filed after #962 lands
- Changing array-order semantics — the fix makes array order irrelevant under normal budget conditions while preserving deterministic behavior when budget is insufficient for all corpora

## Acceptance Criteria (from issue, adjusted per architect NF4)

- mika-platform corpus subject entities ≥ 50
- mika-cloud corpus subject entities ≥ 50
- Entities/chunk ratio for secondary corpora within 0.3-1.0× the primary (mika) corpus's ratio
- After entities populate, allow 1-2 resolver-tick cycles, then re-evaluate milestone#19's R1/R2

## Risk Assessment

**Low risk.** The fairness algorithm is pinned from the resolver (#927) and has been running in production since 2026-05-02. The extraction path uses the identical algorithm via a shared function. The `SubjectExtractor` API is unchanged — only the caller's budget distribution logic changes. The resolver refactor to use the shared function is a pure extraction with no behavioral change.

## Files Changed

| File | Change |
|------|--------|
| `crates/mika-agent/src/kg/budget.rs` | **New** — shared `allocate_fair_budget()` function |
| `crates/mika-agent/src/kg/mod.rs` | Register `budget` module |
| `crates/mika-agent/src/kg/entity_resolver.rs` | Refactor to use `allocate_fair_budget()` (no behavioral change) |
| `crates/mika-agent/src/kg/subject_extractor.rs` | Add `count_pending_docs()` method |
| `crates/mika-agent/src/server/mod.rs` | Refactor extraction startup loop (lines 949-993) |
| `crates/mika-agent/CLAUDE.md` | Add Signal G (with asymmetry note), update budget docs |
