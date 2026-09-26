---
module: kg/entity_resolver
date: 2026-05-02
problem_type: logic_error
component: database
severity: high
symptoms:
  - "Secondary corpora (mika-skills, mika-platform, mika-cloud) show ~50 cumulative resolution attempts after many ticks while primary corpus has 12k+"
  - "kg_resolver_tick.complete logs show resolution progress only on the primary corpus"
  - "Signal E pending_after never reaches 0 for secondary corpora within the expected 17-18 hour window"
root_cause: logic_error
resolution_type: code_fix
tags:
  - kg
  - entity-resolver
  - fairness
  - round-robin
  - multi-corpus
  - starvation
  - resolver-tick
related_components:
  - tooling
---

# KG resolver per-corpus starvation — primary corpus backlog starves secondaries

## Problem

`entity_resolver::get_pending_entities()` selected pending subject entities globally across all agent corpora without per-corpus partitioning or limits. With the default SQLite row ordering (insertion order), the primary corpus — which had the largest backlog (17,538 pending vs ~50 per secondary) — consumed the entire 500/tick Stage-2 LLM budget before any secondary corpus entities were reached in the iteration.

## Symptoms

- After deploying multi-corpus support (#798, #877), mika-arch's secondary corpora (mika-skills, mika-platform, mika-cloud) each had only ~50 cumulative resolution attempts while the primary corpus had 12,673
- The `kg_resolver_tick.complete` log showed resolution progress concentrated on one corpus
- Secondary corpora's resolution rates remained throughput-bound for the entire ~17-18 hour primary drain window

## What Didn't Work

The original implementation used a single SQL query with an `IN (?, ?, ...)` clause across all `docs_root_hash` values and no `ORDER BY` or `LIMIT`. SQLite's default row ordering (rowid, which follows insertion order) meant the primary corpus — populated first and with the most rows — always came first in the result set.

## Solution

### Two-pass budget allocation with round-robin interleaving

Refactored `get_pending_entities()` to accept `total_budget` and distribute it fairly:

**1. Per-corpus pending count query** — lightweight `COUNT(*)` per corpus to determine pool sizes.

**2. Two-pass allocation:**
- First pass: assign each corpus `min(pending_count, budget / N_corpora)`
- Second pass: redistribute unused budget to "hungry" corpora (those with more pending than their first-pass allocation) proportionally

**3. Per-corpus selection** — new `get_pending_entities_for_corpus(hash, limit)` helper with `ORDER BY e.id ASC LIMIT ?` for deterministic, bounded selection per corpus.

**4. Round-robin interleave** — `interleave_round_robin()` produces `[A₀, B₀, C₀, D₀, A₁, B₁, ...]` ordering so each corpus contributes a Stage-2 attempt within at most `N_corpora` iterations.

**5. Per-corpus observability** — `ResolutionStats.per_corpus_attempted: HashMap<String, u32>` tracks per-corpus attempt counts, emitted as JSON in the `kg_resolver_tick.complete` log event.

### Key design decisions

- **Selection limit uses 2x oversupply with floor of 50** (KTD-2): `per_corpus_limit = max(2 * assigned, 50)`. The oversupply accommodates Stage-1 exact matches (which are free but consume iteration slots).
- **Single-corpus fast path**: agents with one corpus skip all allocation overhead.
- **budget=0 preserves Stage-1**: uses `effective_budget = 50` so exact matches still proceed (documented invariant).

## Why This Works

The root cause was that the iteration order in `resolve_entities()` determined which corpora got Stage-2 LLM attempts. By partitioning the selection per corpus and interleaving results, every corpus with pending entities gets a proportional share of both selection and iteration position, regardless of backlog size ratios.

The real LLM budget enforcement stays in `resolve_entities()` (the `llm_calls < budget` guard at each iteration) — the fairness fix only controls *selection and ordering*, not the budget mechanism itself.

## Prevention

- **Test fairness invariants explicitly**: the regression test seeds 4 corpora with asymmetric pending pools (1000, 50, 50, 50) and asserts each contributes ≥25 attempts — this catches future regressions from query changes.
- **When adding multi-entity queries with budget caps, consider iteration-order bias**: if a budget cap can exhaust before all partitions are visited, the selection must be partition-aware.
- **Monitor `per_corpus_attempted` in production**: Signal F (`grep kg_resolver_tick.complete | jq '.per_corpus_attempted'`) confirms all corpora get non-zero attempts on every tick.

## References

- mika#927 — this fix
- mika#877 — milestone#19 verification that surfaced the starvation gap
- mika#906 — periodic resolver tick (the execution context this fix improves fairness of)
- mika#798 — multi-corpus support that introduced the per-agent corpora
- mika#757 — extraction budget guard (sibling pattern)
