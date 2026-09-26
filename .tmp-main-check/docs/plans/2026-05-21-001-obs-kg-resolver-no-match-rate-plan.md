# Plan: obs(kg) — instrument resolver no_match rate (#1077)

**Issue:** mika#1077
**Type:** obs (observability enhancement)
**Branch:** `obs/1077/kg-instrument-resolver-no-match-rate`

## Problem

The KG entity resolver's `no_match` outcome accounts for ~82% of lifetime `kg_resolutions_log` rows (mika-arch, 10,666/13,033). This could indicate graph fragmentation or naturally novel corpus content. There is no rolling-window metric to distinguish the two, detect regressions, or compare across agents/corpora.

## Acceptance Criteria Tie-backs

- **AC1:** `mika kg status` surfaces a rolling 7-day `no_match` rate per agent and per corpus.
- **AC2:** Per-corpus breakdown available (not just per-agent).
- **AC3:** Alert mechanism (log WARN) triggers when 7-day rate >60% sustained.
- **AC4:** Metric persists across restarts (DB-backed, not in-memory).

## Design Decisions

### D1: No new table — query `kg_resolutions_log` directly

The `kg_resolutions_log` table already has `resolved_at TEXT` (ISO 8601) and `outcome TEXT` per row, with an index `idx_kg_res_log_pending ON (agent_id, outcome)`. A rolling 7-day window query is a simple `WHERE resolved_at >= datetime('now', '-7 days')` filter. No materialized metric table is needed — SQLite is fast enough for the expected row counts (<100K per agent). This satisfies AC4 inherently — the source data is already DB-backed.

### D2: Per-corpus breakdown via JOIN through `kg_subject_entities`

`kg_resolutions_log` has `subject_entity_id` FK to `kg_subject_entities`, which carries `docs_root_hash`. A JOIN gives per-corpus breakdown without schema changes. This satisfies AC2.

### D3: Alert via resolver tick structured log

The resolver tick (`resolver_tick.rs`) already runs every 30 minutes per KG-enabled agent. After each resolution batch, compute the 7-day `no_match` rate and emit a WARN log if >60%. This is zero-cost when not firing and reuses the existing periodic execution context. The structured log event name: `kg_no_match_rate_high`. Fields: `agent_id`, `no_match_rate` (f64, 0.0–1.0), `no_match_count` (u64), `total_count` (u64), `window_days` (u64), `per_corpus` (JSON object mapping `docs_root_hash` to `{no_match_count, total, rate}`). This satisfies AC3.

### D4: CLI surface via `mika kg status` extension

Extend `AgentKgState` with `no_match_rate_7d: Option<f64>` and `no_match_count_7d: Option<u64>`, `total_resolutions_7d: Option<u64>`. The text table gains a `7d_no_match` column showing the rate as a percentage (e.g., `82.1%`). JSON output includes the raw fields. Per-corpus rows already exist in the status table (#877). This satisfies AC1.

### D5: DB query as a reusable Database method

Add `kg_resolution_outcome_stats(agent_id, docs_root_hash_filter, window_days) -> Result<ResolutionOutcomeStats>` to `Database`. The method is reusable by both the CLI (always computes per-corpus — AC2 requires per-corpus to be "available", not conditional) and the resolver tick alert (per-corpus detail only computed when agent-wide threshold exceeded — optimization for the alert path only).

### D6: Denominator excludes structural skips and errors

The `no_match_rate()` denominator counts only **attempted resolution** outcomes: `matched_exact`, `matched_llm`, `matched_llm_db_fallback`, `no_match`, `no_candidate_of_type`. Structural skips (`skipped_no_llm`, `skipped_discovered_type`, `skipped_discovered_subject`) and `error` are excluded. Rationale: structural skips are entities that never entered the resolution pipeline (no LLM configured, discovered type with no domain counterpart); including them would dilute the rate and mask fragmentation. `error` represents infrastructure failures, not resolution outcomes. The struct carries both `attempted` (denominator) and `total` (all rows in window, informational only) for full transparency.

## Implementation Steps

### Phase 1: DB query method

**File: `crates/mika-agent/src/db/kg_schema.rs`** — struct definition (centralizes KG types).
**File: `crates/mika-agent/src/db.rs`** — query method (alongside other `kg_*` methods).

**Step 1.** Add `ResolutionOutcomeStats` struct to `crates/mika-agent/src/db/kg_schema.rs`:
```rust
/// Rolling-window outcome breakdown from `kg_resolutions_log`.
/// Used by `mika kg status` (AC1/AC2) and the resolver tick alert (AC3).
pub struct ResolutionOutcomeStats {
    /// All rows in window (informational — includes skips and errors).
    pub total: u64,
    /// Attempted resolution outcomes only (denominator for rate calculation).
    /// = matched_exact + matched_llm + matched_llm_db_fallback + no_match + no_candidate_of_type
    pub attempted: u64,
    pub no_match: u64,
    pub no_candidate_of_type: u64,
    pub matched_exact: u64,
    pub matched_llm: u64,
    pub matched_llm_db_fallback: u64,
    /// Structural skips (excluded from rate denominator).
    pub skipped: u64,
    /// Infrastructure failures (excluded from rate denominator).
    pub errors: u64,
}

impl ResolutionOutcomeStats {
    /// no_match / attempted. Returns 0.0 when no attempted resolutions exist.
    pub fn no_match_rate(&self) -> f64 {
        if self.attempted == 0 { 0.0 } else { self.no_match as f64 / self.attempted as f64 }
    }
}
```

**Step 2.** Add `Database::kg_resolution_outcome_stats(&self, agent_id: &str, docs_root_hash: Option<&str>, window_days: u32) -> Result<ResolutionOutcomeStats>` to `crates/mika-agent/src/db.rs`.

SQL (per-corpus variant when `docs_root_hash` is `Some`):
```sql
SELECT
    COUNT(*) as total,
    SUM(CASE WHEN rl.outcome IN ('matched_exact','matched_llm','matched_llm_db_fallback','no_match','no_candidate_of_type') THEN 1 ELSE 0 END) as attempted,
    SUM(CASE WHEN rl.outcome = 'no_match' THEN 1 ELSE 0 END) as no_match,
    SUM(CASE WHEN rl.outcome = 'no_candidate_of_type' THEN 1 ELSE 0 END) as no_candidate_of_type,
    SUM(CASE WHEN rl.outcome = 'matched_exact' THEN 1 ELSE 0 END) as matched_exact,
    SUM(CASE WHEN rl.outcome = 'matched_llm' THEN 1 ELSE 0 END) as matched_llm,
    SUM(CASE WHEN rl.outcome = 'matched_llm_db_fallback' THEN 1 ELSE 0 END) as matched_llm_db_fallback,
    SUM(CASE WHEN rl.outcome IN ('skipped_no_llm','skipped_discovered_type','skipped_discovered_subject') THEN 1 ELSE 0 END) as skipped,
    SUM(CASE WHEN rl.outcome = 'error' THEN 1 ELSE 0 END) as errors
FROM kg_resolutions_log rl
JOIN kg_subject_entities se ON se.id = rl.subject_entity_id
WHERE rl.agent_id = ?1
  AND se.docs_root_hash = ?2
  AND rl.resolved_at >= datetime('now', ?3)
```

The `?3` parameter is `-N days` string (e.g., `'-7 days'`). When `docs_root_hash` is `None`, drop the JOIN and `se.docs_root_hash` filter for agent-wide stats.

**Step 3.** Add `AsyncDatabase::kg_resolution_outcome_stats(...)` async wrapper to `crates/mika-agent/src/async_db.rs` (same pattern as other `with_db` wrappers).

### Phase 2: Resolver tick alert

**File: `crates/mika-agent/src/kg/resolver_tick.rs`**

**Step 4.** After the resolution phase completes in the tick loop, call `db.kg_resolution_outcome_stats(agent_id, None, 7)` for the agent-wide rate. If `stats.no_match_rate() > 0.60 && stats.attempted >= 50` (minimum sample size to avoid false alarms on cold start), emit:
```rust
warn!(
    agent_id = %agent_id,
    no_match_rate = stats.no_match_rate(),
    no_match_count = stats.no_match,
    attempted_count = stats.attempted,
    window_days = 7,
    "kg_no_match_rate_high"
);
```

**Step 5.** Per-corpus breakdown in the WARN (only when agent-wide threshold exceeded — optimization for the alert path; CLI always computes per-corpus per AC2): iterate `docs_root_hashes` and call `db.kg_resolution_outcome_stats(agent_id, Some(hash), 7)` for each. Serialize as JSON in the log event's `per_corpus` field.

### Phase 3: CLI extension

**File: `crates/mika-cli/src/commands/kg.rs`**

**Step 6.** Extend `AgentKgState` with three new fields:
```rust
#[serde(skip_serializing_if = "Option::is_none")]
no_match_rate_7d: Option<f64>,
#[serde(skip_serializing_if = "Option::is_none")]
no_match_count_7d: Option<u64>,
#[serde(skip_serializing_if = "Option::is_none")]
total_resolutions_7d: Option<u64>,
```

**Step 7.** In `build_agent_kg_states()`, after computing `resolved`/`pending`, call `db.kg_resolution_outcome_stats(agent_name, Some(&corpus.docs_root_hash), 7)` and populate the new fields. On error, leave as `None` (fail-open — existing status output is not degraded).

**Step 8.** Update the text table format: add a `7d_no_match` column between `pending` and `last_extraction`. Format as `"82.1%"` when present, `"N/A"` when `None` or `total_resolutions_7d == 0`. Highlight with `[!]` suffix when rate >60%.

**Step 9.** JSON output: the new fields are included automatically via `serde::Serialize`.

### Phase 4: Tests

**Step 10.** Unit test for `ResolutionOutcomeStats::no_match_rate()` — zero total returns 0.0, normal calculation correct.

**Step 11.** Integration test for `Database::kg_resolution_outcome_stats()`:
- Seed `kg_resolutions_log` with known outcomes and `resolved_at` timestamps spanning >7 days.
- Seed `kg_subject_entities` with two different `docs_root_hash` values.
- Assert agent-wide stats match expected counts.
- Assert per-corpus filter returns correct subset.
- Assert 8-day-old rows are excluded from 7-day window.

**Step 12.** No eval-harness changes needed — this is pure observability, no agent loop behavior change.

## Files Changed

| File | Change |
|------|--------|
| `crates/mika-agent/src/db/kg_schema.rs` | `ResolutionOutcomeStats` struct definition |
| `crates/mika-agent/src/db.rs` | `kg_resolution_outcome_stats()` query method |
| `crates/mika-agent/src/async_db.rs` | Async wrapper for new DB method |
| `crates/mika-agent/src/kg/resolver_tick.rs` | 7-day no_match rate check + WARN log after resolution phase |
| `crates/mika-cli/src/commands/kg.rs` | `AgentKgState` extension, `build_agent_kg_states()` population, text table format update |

## Scope

**In scope:** Rolling 7-day `no_match` rate metric via DB query, per-agent and per-corpus, CLI surface, structured WARN log alert.

**Out of scope:**
- Dashboard widget (future — this provides the DB query; the dashboard can consume it later)
- Deduplication/canonicalization pass (separate concern — this surfaces the signal to decide whether to invest)
- Configurable threshold (hardcoded 60% per AC3; tunability deferred)
- Configurable window (hardcoded 7 days per AC1; the DB method takes `window_days` as a parameter for future flexibility)

## Risks

1. **Performance on large `kg_resolutions_log`.** Current mika-arch has ~13K rows. The JOIN through `kg_subject_entities` adds cost, but both tables have indexes on the join columns. For O(100K) rows this is sub-millisecond on SQLite. If it grows to O(1M), a covering index on `(agent_id, resolved_at, outcome)` would help, but that's a premature optimization now.

2. **Minimum sample size.** The 50-attempted-resolution minimum in Step 4 prevents false WARN alerts on cold-start or newly-provisioned agents. This is a conservative threshold — a fresh corpus with 50 attempted resolutions at >60% `no_match` is still a meaningful signal.

## Architect Review History

- **First-pass (ITERATE):** session `bc7f0fdd-fefe-4102-9412-4ae853157263`. F1: Phase 0 file placement pinned (4 sites). F2: denominator definition — resolved via D6 (exclude structural skips and errors, use `attempted` as denominator). F3: error outcome — excluded from denominator per D6. F4: D5 per-corpus gating vs AC2 — CLI always computes per-corpus; gating only in WARN alert path.
