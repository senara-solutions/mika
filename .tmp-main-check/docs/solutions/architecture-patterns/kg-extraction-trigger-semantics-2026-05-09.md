---
module: kg
tags: [extraction, trigger, budget, drain-rate, periodic-tick]
problem_type: architecture-pattern
category: architecture-patterns
date: 2026-05-09
ticket: mika#1052
---

# KG Extraction Trigger Semantics

## Three Extraction Triggers

The KG subject extraction pipeline has three trigger contexts. Each runs the same `SubjectExtractor::extract_pending(budget)` method with the same fair budget allocation via `allocate_fair_budget()`.

| Trigger | Scope | Frequency | Budget source | Added in |
|---------|-------|-----------|---------------|----------|
| **Startup** | All pending docs per corpus | On deploy/restart | `MIKA_KG_BATCH_BUDGET` (default 500) | #690 |
| **Compound hook** | Single doc just written | On `ce:compound` doc write | 1 (single doc) | #690 |
| **Periodic tick** | All pending docs per corpus | Every 30 min | `MIKA_KG_BATCH_BUDGET` (default 500) | #1052 |

## Budget Allocation

Both startup and periodic tick use the same two-pass fair allocation algorithm (`kg::budget::allocate_fair_budget`):

1. **Pass 1 (floor):** Each corpus with pending > 0 gets `min(pending, budget / N_active)`.
2. **Pass 2 (redistribute):** Unused slots from corpora that hit their pending cap are redistributed to "hungry" corpora proportionally.

This ensures no corpus is starved regardless of array order (#962).

## Expected Drain Rate

With the default budget of 500 and a 30-min tick interval:

- **Per day:** 48 tick opportunities × budget/N_corpora per tick
- **Single corpus agent:** Up to 24,000 extraction LLM calls/day (will reach 0 pending quickly)
- **4-corpus agent (e.g., mika-arch):** ~125 docs per corpus per tick, ~6,000 per corpus per day

Once coverage reaches 100%, the tick's extraction phase is a no-op (zero pending = zero budget allocated, no LLM calls).

## Idempotency

The `kg_extractions` table serves as the idempotency marker with `UNIQUE(docs_root_hash, source_doc_path)`. Since #1052, the marker uses `ON CONFLICT ... DO UPDATE` (upsert) instead of `INSERT OR IGNORE`:

- **NULL-hash rows** (pre-v26 legacy) get updated on re-extraction
- **Content-changed docs** (hash mismatch) get re-extracted
- **Identical-content re-extractions** are no-ops (WHERE clause prevents update)

## Observability

| Event | Phase | Fields |
|-------|-------|--------|
| `subject_extraction_start` | Per-doc | `pending_docs`, `budget` |
| `subject_extraction_complete` | Per-doc batch | `completed`, `failed`, `llm_calls`, `aborted_budget` |
| `subject_extraction_ready` | Startup | `per_corpus_extracted`, `total_docs_extracted` |
| `kg_extraction_tick.complete` | Periodic tick | `per_corpus_extracted`, `total_pending` |
| `kg_extraction_coverage` | Periodic tick | `per_corpus_coverage` (total, extracted, null_hash, pct) |
| `kg_budget_exhausted` | Any | `scope="extraction"`, `calls_made`, `remaining` |

## Cost Model

With OpenRouter configured at cheap-tier pricing (~$0.0001/call):

- **Per startup:** `N_agents × budget` extraction calls max → ~$0.05
- **Per tick:** Same budget, but typically fewer pending docs → converges to $0
- **Steady state:** $0 (no pending docs = no LLM calls)
