---
title: "KG subject-entity extraction stalls on secondary corpora due to sequential budget drain"
module: kg
date: 2026-05-08
problem_type: logic_error
component: tooling
severity: high
symptoms:
  - "Secondary corpora (mika-platform, mika-cloud) have 6 entities from 200+ chunks while primary (mika) has 30K+"
  - "Entities/chunk ratio two orders of magnitude lower on secondaries vs primary"
  - "Extraction on secondaries shows only 1 cluster in kg_subject_entities.created_at, then nothing"
  - "kg_budget_exhausted WARN fires on extraction scope with roots_remaining > 0"
root_cause: logic_error
resolution_type: code_fix
tags:
  - kg
  - subject-extraction
  - budget
  - fairness
  - multi-corpus
  - starvation
related_components:
  - database
---

# KG subject-entity extraction stalls on secondary corpora due to sequential budget drain

## Problem

Subject-entity extraction stalled on secondary corpora (mika-platform: 6 entities from 437 chunks, mika-cloud: 6 entities from 199 chunks) while the primary corpus (mika) had 30,289 entities from 3,155 chunks. The extraction startup loop in `server/mod.rs` used a sequential left-to-right drain with a shared budget — the primary corpus consumed the entire `MIKA_KG_BATCH_BUDGET` before secondary corpora got a turn.

This was the **extraction-side sibling** of the resolution starvation bug fixed in #927.

## Symptoms

- Secondary corpora show entities/chunk ratio of 0.014–0.030 vs 9.6 for the primary
- `kg_budget_exhausted` WARN with `scope="extraction"` and `roots_remaining > 0`
- Only one `created_at` cluster per secondary corpus in `kg_subject_entities`
- Extraction stops on secondaries after the first batch, never resumes

## Root Cause

Architectural asymmetry between extraction and resolution:

| Aspect | Resolution (#927 — fixed) | Extraction (#962 — broken) |
|--------|--------------------------|---------------------------|
| Instance | Single resolver with all `docs_root_hash` values | One `SubjectExtractor` per corpus, sequential |
| Budget distribution | Two-pass fairness allocation + round-robin interleaving | Left-to-right drain, first corpus wins |

The extraction startup loop iterated corpora sequentially, subtracting `llm_calls` from a shared `remaining_budget`. The primary corpus (first in the array) exhausted the budget, causing the loop to `break` before reaching secondary corpora.

```rust
// BEFORE — sequential drain, primary starves secondaries
let mut remaining_budget = budget;
for (corpus_idx, docs_root) in corpora_clone.into_iter().enumerate() {
    if remaining_budget == 0 { break; }  // ← starves remaining corpora
    let extractor = SubjectExtractor::new(db.clone(), llm.clone(), docs_root, None);
    match extractor.extract_pending(remaining_budget).await { ... }
}
```

## Solution

Extract the two-pass fair budget allocation algorithm (proven in the resolver since #927) into a shared `kg::budget::allocate_fair_budget()` function, then apply it to both extraction and resolution.

### 1. Shared budget allocation module (`kg/budget.rs`)

```rust
pub(crate) fn allocate_fair_budget(pending_counts: &[u32], total_budget: u32) -> Vec<u32> {
    let n = pending_counts.iter().filter(|&&c| c > 0).count() as u32;
    if n == 0 || total_budget == 0 {
        return vec![0; pending_counts.len()];
    }
    // Pass 1: floor allocation — each corpus gets min(pending, budget/N_active)
    let base_share = total_budget / n;
    let mut assigned: Vec<u32> = pending_counts.iter().map(|&c| c.min(base_share)).collect();
    // Pass 2: redistribute remainder to hungry corpora
    let mut remaining = total_budget.saturating_sub(assigned.iter().sum::<u32>());
    // ... (round-robin distribution to corpora with pending > assigned)
    assigned
}
```

### 2. Three-phase extraction startup loop (`server/mod.rs`)

```rust
// AFTER — fair distribution
// Phase 1: Count pending docs per corpus
let mut corpus_pending: Vec<u32> = Vec::new();
let mut extractors: Vec<SubjectExtractor> = Vec::new();
for docs_root in &corpora_clone {
    let extractor = SubjectExtractor::new(db.clone(), llm.clone(), docs_root.clone(), None);
    let count = extractor.count_pending_docs().await.unwrap_or(0);
    corpus_pending.push(count);
    extractors.push(extractor);
}
// Phase 2: Fair budget allocation
let allocated = kg::budget::allocate_fair_budget(&corpus_pending, budget);
// Phase 3: Execute with per-corpus budgets
for (idx, (extractor, per_budget)) in extractors.into_iter().zip(allocated.iter()).enumerate() {
    if *per_budget == 0 { continue; }
    extractor.extract_pending(*per_budget).await ...
}
```

### 3. Refactored resolver (`entity_resolver.rs`)

Replaced 40 lines of inline two-pass allocation with a 3-line call to the shared function. No behavioral change — verified by all 17 existing resolver tests passing unchanged.

## Why This Works

The two-pass algorithm guarantees proportional allocation:
- **Pass 1:** Each active corpus gets `min(pending, budget/N_active)` — the floor share
- **Pass 2:** Unused slots (from corpora with fewer pending than their floor share) are redistributed to "hungry" corpora that need more

This eliminates array-order dependence. When budget >= N_active, every corpus with pending work gets a non-zero allocation. The algorithm was already proven in production for resolution since #927.

## Prevention

1. **Pattern: shared budget functions for multi-corpus pipelines.** When adding new KG pipeline stages that operate across multiple corpora, use `kg::budget::allocate_fair_budget()` instead of implementing sequential drain loops. The function is in `crates/mika-agent/src/kg/budget.rs` with 12 unit tests.

2. **Signal G for monitoring.** After deploy, verify extraction fairness via:
   ```bash
   grep subject_extraction_ready server.log | jq '.per_corpus_extracted'
   ```
   All corpora with pending docs should show non-zero extractions.

3. **Architectural note.** Extraction fairness is caller-side (server/mod.rs distributes budget to per-corpus SubjectExtractor instances); resolution fairness is internal (SubjectEntityResolver distributes within a single instance). This asymmetry is intentional — the extractor's provenance transaction is per-doc-per-corpus.

## Related

- mika#962 — This issue
- mika#927 — Resolution-side fairness fix (sibling)
- mika#928 — Domain graph concept expansion (prerequisite for meaningful resolution)
- `docs/solutions/best-practices/kg-single-consumer-topology-shared-corpus-race-2026-05-07.md` — Related shared-corpus pattern
- `docs/solutions/best-practices/post-restart-kg-extraction-resolution-audit-2026-04-29.md` — Post-restart audit signals (now includes Signal G)
