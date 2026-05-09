---
module: kg
tags: [extraction, null-hash, deadlock, upsert, periodic-tick, idempotency]
problem_type: bug
category: database-issues
date: 2026-05-09
ticket: mika#1052
---

# KG NULL-Hash Deadlock and Extraction Lag

## Problem

Three mika-arch-only KG corpora were at 16-71% extraction coverage after 2 weeks, while the shared corpus (used by mika/mika-dev/mika-qa/mika-arch) was at 89% and a control corpus (odds-engine) was at 100%.

Two root causes:

### 1. No periodic extraction — only startup + compound hooks

The extraction pipeline had a structural gap: the 30-min resolver tick (#906) only ran resolution, not extraction. Corpora that didn't fully drain at startup had to wait for the next restart. mika-arch-only corpora got fewer restart cycles (only mika-arch restarts, not mika/mika-dev/mika-qa), leading to slower coverage convergence.

### 2. NULL source_doc_hash deadlock

Three pre-v26 `kg_extractions` rows had `source_doc_hash IS NULL`. The pending-doc detection query uses `e.source_doc_hash = c.source_doc_hash`, and `NULL = NULL` evaluates to `NULL` (falsy in SQL), so the `NOT EXISTS` subquery treated these docs as perpetually "pending." But the `INSERT OR IGNORE` on the `UNIQUE(docs_root_hash, source_doc_path)` key saw the existing row and skipped silently.

**Result:** Each startup consumed a budget slot for these 3 docs (LLM call + extraction) but the marker was never updated. The docs cycled between "pending" and "skip" on every restart.

## Solution

### Change 1: Periodic extraction in the tick

Added extraction as Phase 1 of the 30-min tick (before resolution in Phase 2). Uses the same `allocate_fair_budget()` for per-corpus fairness. Corpora get 48 more extraction opportunities per day.

### Change 2: INSERT OR IGNORE → ON CONFLICT DO UPDATE upsert

Changed the idempotency marker write from:
```sql
INSERT OR IGNORE INTO kg_extractions ... VALUES (...)
```
to:
```sql
INSERT INTO kg_extractions ... VALUES (...)
ON CONFLICT(docs_root_hash, source_doc_path) DO UPDATE SET
    source_doc_hash = excluded.source_doc_hash,
    ...
WHERE kg_extractions.source_doc_hash IS NULL
   OR kg_extractions.source_doc_hash != excluded.source_doc_hash
```

The WHERE clause ensures:
- NULL-hash rows get updated (fixes the deadlock)
- Content-changed docs get re-extracted (hash mismatch)
- Identical-content re-extractions are no-ops (hash matches → no update)

### Change 3: v32→v33 migration

Deletes existing NULL-hash rows so they're cleanly pending for re-extraction with the new upsert. Wrapped in a transaction per the migration pattern.

### Change 4: Coverage observability

Added `coverage_report()` method and `kg_extraction_coverage` structured log event. Per-corpus coverage (total, extracted, null_hash, pct) is logged at the end of each tick.

## Lessons Learned

1. **NULL equality is a SQL trap.** `NULL = NULL` is `NULL`, not `TRUE`. Any SQL pattern using equality predicates on nullable columns must account for this. The pending-doc detection query was correct (it correctly treated NULL-hash rows as "pending"), but the INSERT OR IGNORE marker write was the other half of the deadlock — it silently skipped the row because the UNIQUE constraint was already satisfied.

2. **Startup-only triggers create restart-dependent convergence.** When a pipeline only runs at startup, the drain rate is coupled to the restart frequency. Agents that restart less often (mika-arch, which has no direct user traffic triggering restarts) fall behind. Adding the pipeline to a periodic tick decouples drain rate from restart cadence.

3. **Migration atomicity matters even for "safe" operations.** The initial migration ran DELETE and INSERT INTO schema_version as separate statements. Review caught this — even though the DELETE is idempotent (safe on re-run), wrapping in a transaction is the correct pattern for consistency with all other migrations. A crash between the two operations would leave the DB in an inconsistent state that, while recoverable, would confuse monitoring.

4. **Budget double-spend is acceptable when bounded.** The tick runs both extraction and resolution at the same budget cap (up to 2× budget per tick). This is the same pattern as startup. Once extraction coverage reaches 100%, the extraction phase becomes a no-op, so the steady-state cost is just resolution.
