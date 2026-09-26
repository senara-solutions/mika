# Plan: What's Next Engine — Sub-Issue 2a: Scoring Engine + Status Re-Derivation

**Ticket:** mika#1263 (Layer 2 of Operational Partner Foundation)  
**Sub-issue scope:** 2a — Scoring engine + calibration constants + status re-derivation  
**Depends on:** Layer 1 (mika#1262, merged — `operational_items` table at schema v39)

## Decomposition

Per review-guide.md § Single Responsibility and the #1259 decomposition precedent, mika#1263 is decomposed into five sub-issues. Each gets its own grooming, architect review, and PR.

| Sub-issue | Scope | Depends on |
|-----------|-------|------------|
| **2a (this plan)** | Scoring engine (`scoring.rs`, `calibration.rs`) + status re-derivation (`status.rs`) + `AsyncDatabase` canonical read path. Pure Rust, no LLM, no IO beyond DB. Foundation for everything else. | Layer 1 only |
| **2b** | CLI `mika next` + agent prompt injection. First consumer surfaces — proves the engine end-to-end. | 2a |
| **2c** | LLM explainer (`explainer.rs`) + daily brief integration. Adds narration on top of working scoring. | 2a |
| **2d** | Dashboard `OperationalInbox` page + API endpoint extensions. Largest frontend surface, ships last. | 2a, 2c |
| **2e** | Memory rule encoding + documentation updates. Structural tests and docs. | 2a, 2c |

**This plan covers sub-issue 2a only.** Sub-issues 2b–2e will be filed as separate tickets with their own plans after 2a merges.

## Summary

Implement the deterministic scoring engine and status re-derivation pass for `OperationalItem`s. This is the computational core of the What's Next engine — all read surfaces (CLI, dashboard, prompt injection, daily brief) depend on it. No LLM involvement, no HTTP endpoints, no frontend. Pure Rust scoring and status derivation with batch-optimized DB access.

The core principle: **the scoring formula is deterministic, calibratable, and debuggable.** The LLM's role (sub-issue 2c) is downstream narration only.

## Phase 0 — Pin Load-Bearing Sources

Before coding, verify:

1. `crates/mika-agent/src/operational/mod.rs` — confirm module structure and existing exports from Layer 1.
2. `crates/mika-agent/src/operational/types.rs` — confirm `OperationalItem`, `OperationalKind`, `OperationalStatus`, `Owner` struct shapes match foundation doc §3.
3. `crates/mika-agent/src/operational/query.rs` — confirm existing query methods and their signatures.
4. `crates/mika-agent/src/db.rs` — confirm `AsyncDatabase` wrapper patterns for operational items.
5. `docs/architecture/operational-partner-frame.md` §4 (status taxonomy) and §5 (scoring formula) — pin exact formula terms.
6. `crates/mika-agent/src/db/operational.rs` — confirm `complete_operational_item()` body. **Pre-pinned (second-pass review):** the function only UPDATEs the single target row to `status = 'done'`; it does **NOT** cascade clear `blocked_by` on dependent items. `resolve_cleared_blockers()` (§2.3) is the primary cascade mechanism, not defense-in-depth. The Layer 1 write path is unchanged by this plan.
7. `crates/mika-agent/src/db.rs` `operational_items` index list — confirm 5 indexes exist: `idx_operational_items_agent_status`, `idx_operational_items_agent_kind`, `idx_operational_items_agent_priority`, `idx_operational_items_source`, `idx_operational_items_source_unique`. **Pre-pinned (second-pass review):** there is **NO** `idx_operational_items_blocked_by` index. The Phase 1.3 GROUP BY query runs as a full table scan, which is acceptable at N≤50 (per the bounded-cost analysis below); no v39→v40 migration is added by this plan to introduce a new index.

## Phase 1 — Scoring Engine (`crates/mika-agent/src/operational/scoring.rs`)

### 1.1 Create `scoring.rs` module

New file at `crates/mika-agent/src/operational/scoring.rs`, exported from `mod.rs`.

**Functions:**

```rust
/// Compute composite priority score for a single item.
/// Returns a ScoreBreakdown with each term's contribution.
/// `blocked_counts` is a pre-loaded map of item_id -> count of items blocked by it.
pub fn priority(
    item: &OperationalItem,
    now: DateTime<Utc>,
    blocked_counts: &HashMap<String, u32>,
) -> ScoreBreakdown

/// Score and rank a list of items. Returns items sorted by priority DESC.
/// Batch-loads dependency counts in a single query before scoring.
pub fn rank(items: &[OperationalItem], now: DateTime<Utc>, db: &Database) -> Vec<ScoredItem>
```

**`ScoreBreakdown` struct:**

```rust
pub struct ScoreBreakdown {
    pub total: f32,
    pub urgency: f32,
    pub commitment_weight: f32,
    pub user_importance: f32,
    pub stale_time: f32,
    pub dependency_risk: f32,
    pub confidence_penalty: f32,
}
```

**`ScoredItem` struct:**

```rust
pub struct ScoredItem {
    pub item: OperationalItem,
    pub breakdown: ScoreBreakdown,
}
```

### 1.2 Implement each scoring term

Per foundation doc §5, using constants from `calibration.rs`:

| Term | Implementation |
|------|----------------|
| `urgency` | Parse `item.due_at` as ISO 8601. `None` → 0.0. Past-due → `URGENCY_MAX` (100.0). Otherwise `URGENCY_MAX / (hours_until_due + 1.0)`. |
| `commitment_weight` | `item.kind == Commitment` → weight by `item.owner`: `User` → `COMMITMENT_WEIGHT_USER` (50.0), `Mika` → `COMMITMENT_WEIGHT_MIKA` (35.0), `Person(_)`/`Agent(_)` → `COMMITMENT_WEIGHT_THIRD_PARTY` (20.0). Non-commitments → 0.0. |
| `user_importance` | `item.user_importance.min(USER_IMPORTANCE_MAX)` (clamped at 50.0). |
| `stale_time` | Hours since `item.updated_at`. `log(hours + 1.0) * STALE_TIME_MULTIPLIER` capped at `STALE_TIME_CAP` (30.0). Uses `f32::ln()`. |
| `dependency_risk` | Lookup `blocked_counts[&item.id]` × `DEPENDENCY_RISK_PER_BLOCKED` (10.0), capped at `DEPENDENCY_RISK_CAP` (40.0). **No per-item DB query** — counts are batch-loaded (see §1.3). |
| `confidence_penalty` | `(1.0 - item.confidence) * CONFIDENCE_PENALTY_MULTIPLIER` (50.0). Subtracted from total. |

**Composite:** `urgency + commitment_weight + user_importance + stale_time + dependency_risk - confidence_penalty`.

### 1.3 Batch-load dependency counts (addresses F3: N+1 query elimination)

The `dependency_risk` term requires knowing how many items are blocked by each item. Instead of per-item `COUNT(*)` queries (N+1 pattern), `rank()` batch-loads all blocked counts in a single query before entering the scoring loop:

```sql
SELECT blocked_by, COUNT(*) as blocked_count
FROM operational_items
WHERE agent_id = ?1 AND blocked_by IS NOT NULL AND status != 'done'
GROUP BY blocked_by
```

This returns a `HashMap<String, u32>` passed to each `priority()` call. Cost: **one indexed GROUP BY query per rank invocation**, regardless of item count.

The `priority()` function takes `blocked_counts: &HashMap<String, u32>` instead of a `&Database` reference, making it a pure function (no IO) — testable without DB fixtures.

**Bounded cost:** For the maximum realistic item set (50 items per agent, per the `MIKA_OPERATIONAL_PARTNER` design), this is a single query returning at most 50 rows. **No `idx_operational_items_blocked_by` index exists** (verified at Phase 0 pin 7) — the GROUP BY runs as a full table scan over the agent's operational_items rows. This is acceptable at N≤50 (sub-millisecond on SQLite); no migration is added to introduce a new index. Should item counts grow significantly beyond 50, a v39→v40 migration adding `CREATE INDEX idx_operational_items_blocked_by ON operational_items(agent_id, blocked_by) WHERE blocked_by IS NOT NULL` is the natural follow-up — out of scope for this plan.

Citation: review-guide.md § KISS (batch query eliminates hidden N+1 complexity); review-guide.md § DRY (batch query pattern already exists in the codebase for similar count aggregations).

### 1.4 Priority cache write-back

After scoring, call `db.update_operational_item_priority(id, total)` to persist the computed score. This keeps the `priority` column in sync for dashboard queries that sort by priority without re-computing.

### 1.5 Calibration constants (`calibration.rs`)

New file at `crates/mika-agent/src/operational/calibration.rs`:

```rust
pub const URGENCY_MAX: f32 = 100.0;
pub const COMMITMENT_WEIGHT_USER: f32 = 50.0;
pub const COMMITMENT_WEIGHT_MIKA: f32 = 35.0;
pub const COMMITMENT_WEIGHT_THIRD_PARTY: f32 = 20.0;
pub const USER_IMPORTANCE_MAX: f32 = 50.0;
pub const STALE_TIME_MULTIPLIER: f32 = 5.0;
pub const STALE_TIME_CAP: f32 = 30.0;
pub const DEPENDENCY_RISK_PER_BLOCKED: f32 = 10.0;
pub const DEPENDENCY_RISK_CAP: f32 = 40.0;
pub const CONFIDENCE_PENALTY_MULTIPLIER: f32 = 50.0;
pub const TODAY_WINDOW_HOURS: f64 = 24.0;
pub const STALE_NEAR_DEADLINE_HOURS: f64 = 48.0;
```

### 1.6 Tests

- Unit tests for each term in isolation with synthetic `OperationalItem` fixtures. `priority()` is a pure function (no DB) — tests pass `HashMap` directly.
- Deterministic ranking test: 5+ items with known field values, assert exact ordering.
- Edge cases: `due_at = None`, `confidence = 0.0`, `confidence = 1.0`, past-due item, commitment vs non-commitment, item with high dependency fan-out.
- Calibration suite: `#[cfg(test)] mod calibration_tests` with fixtures that produce expected rankings deterministically (no LLM in the loop).

## Phase 2 — Status Re-Derivation (`crates/mika-agent/src/operational/status.rs`)

### 2.1 Create `status.rs` module

Per foundation §4, status is **derivable, not authoritative** (except `Done` which is terminal). On every read, the engine re-derives status from item properties and updates the cache if it changed.

```rust
/// Re-derive the status of a non-Done item based on its current properties.
/// Returns the derived status. Does NOT write to DB — caller decides.
pub fn derive_status(item: &OperationalItem, now: DateTime<Utc>) -> OperationalStatus
```

**Derivation rules (precedence per Decision G: AtRisk > Now > Waiting > Delegated > Scheduled):**

1. **Done** → skip (terminal, excluded from re-derivation).
2. **AtRisk** — any of: (a) `due_at` within `TODAY_WINDOW_HOURS` AND `updated_at` > `STALE_NEAR_DEADLINE_HOURS` ago (stale near deadline); (b) callback failure evidence in `evidence_refs`. AtRisk overrides all non-terminal statuses.
3. **Now** — `due_at` is within the today window (`TODAY_WINDOW_HOURS` = 24h) AND `blocked_by IS NULL`. Or: `due_at IS NULL` AND `blocked_by IS NULL` AND `status` was explicitly set to `Now`.
4. **Waiting** — `blocked_by IS NOT NULL`.
5. **Delegated** — `owner` is `Mika` or `Agent(_)` AND there's an active source task.
6. **Scheduled** — `due_at` is set AND beyond the today window.

### 2.2 Read-path re-derivation with bounded cost analysis (addresses F2)

`OperationalItem::query()` (in `query.rs`) calls `derive_status()` on each returned non-Done item. If the derived status differs from the cached status, issue a `db.update_operational_item_status()` call to update the cache.

**Cost analysis — worst-case write amplification:**

The re-derivation runs at read time on every non-Done item returned by a query. The bounded costs per read surface:

| Surface | Max items per read | Max status-change writes | Frequency |
|---------|-------------------|--------------------------|-----------|
| Agent prompt injection | 20 (hardcoded limit) | 20 | Every agent turn (~30s–5min intervals) |
| CLI `mika next` | 5 | 5 | On-demand (human-initiated) |
| Dashboard list page | 50 (paginated) | 50 | On-demand (human-initiated) |
| Daily brief | 10 | 10 | Once per day |

**Worst case:** 20 items × status change probability. In practice, status changes are rare events — they require a condition change (due_at entering the today window, a blocker resolving, staleness crossing the threshold). On a typical read, 0–2 items change status. The write is a single `UPDATE operational_items SET status = ?2 WHERE id = ?1` per changed item — indexed, sub-millisecond on SQLite.

**Accepted trade-off:** The synchronous write-back on the read path is acceptable because: (a) N ≤ 20 for the hottest path (prompt injection); (b) each write is a single indexed UPDATE, not a transaction; (c) status changes are infrequent (condition changes, not every read); (d) the alternative (periodic sweep) adds complexity and staleness without meaningful performance gain at this scale.

This is an explicit acceptance of the write amplification pattern, per foundation doc §4 Decision G which establishes derivation as the authority.

Citation: review-guide.md § KISS (explicit cost model over hidden complexity); foundation doc §4 Decision G (establishes derivation principle).

### 2.3 `Blocked_by` cascade — **primary mechanism** (not defense-in-depth)

When the query returns items, check for transitive unblocks: if a `Waiting` item's `blocked_by` ID points to an item that is now `Done`, the blocked item should re-derive (likely to `Now`).

**Cascade ownership (verified at Phase 0 pin 6):** Layer 1's `complete_operational_item()` only UPDATEs the single target row — it does **NOT** clear `blocked_by` on dependent items. `resolve_cleared_blockers()` is therefore the **primary cascade mechanism**, not a belt-and-suspenders check. This makes it critical-path code: the read path is the only place where transitive unblocks happen.

The helper:

```rust
/// Primary cascade mechanism for transitive unblocks.
/// For each Waiting item in the slice, check if its blocked_by ID
/// points to a Done item; if so, clear blocked_by in the DB.
/// Layer 1's complete_operational_item does NOT cascade — this is
/// where the blocker→waiting unblock happens.
pub fn resolve_cleared_blockers(items: &[OperationalItem], db: &Database) -> Result<()>
```

**Critical-path implications:**
- Must run before `derive_status()` so the cleared `blocked_by` is reflected in the next status derivation pass.
- A read with N=20 items has at most 20 cascade-check queries (each is `SELECT status FROM operational_items WHERE id = ?` — indexed by primary key, sub-millisecond). The bounded cost analysis in §2.2 covers this — the write-amplification table's "20 items × status change probability" includes any cascade-induced status changes.
- Tests must assert that completing a blocker results in dependent items unblocking on the next `query()` call, not on the `complete_operational_item()` call itself.

### 2.4 Tests

- Derive each status from fixture properties.
- Precedence: item with both AtRisk and Now conditions → AtRisk wins.
- Done exclusion: Done item with past-due `due_at` stays Done.
- Blocker cascade: Waiting item unblocks when blocker moves to Done.

## Phase 3 — `AsyncDatabase` Canonical Read Path

### 3.1 `AsyncDatabase` wrapper

Add `async fn score_and_rank_items(&self, agent_id: &str, limit: u32) -> Result<Vec<ScoredItem>>` to `AsyncDatabase` that:

1. Queries non-Done items for the agent.
2. Batch-loads blocked counts (single GROUP BY query — §1.3).
3. Runs `resolve_cleared_blockers()` to clear stale `blocked_by` references.
4. Runs `derive_status()` on each non-Done item, updating cache if changed.
5. Runs `priority()` scoring on each item using the batch-loaded counts.
6. Writes back updated priority scores.
7. Sorts by priority DESC.
8. Returns top `limit` items with breakdowns.

This is the **canonical read-path entry point** for all surfaces (sub-issues 2b–2d). The method encapsulates the full score-derive-rank pipeline so consumers don't need to orchestrate the steps.

### 3.2 Feature gate placement (addresses F6)

The `MIKA_OPERATIONAL_PARTNER` gate check happens at each **read surface's entry point** — not inside `score_and_rank_items()`. This follows the Layer 1 pattern (PR #1266: "Writes always-on; reads gated behind `MIKA_OPERATIONAL_PARTNER=1`") where the gate lives at the surface, not the DB method.

Concretely (for sub-issues 2b–2d):
- CLI: early return with guidance message before calling `score_and_rank_items()`.
- Dashboard endpoint: return 404 before calling `score_and_rank_items()`.
- Agent prompt injection: skip the `score_and_rank_items()` call entirely when gate is off. The gate check happens once in the caller (`run_agent()` / `run_silent_agent()` / `run_team_agent()`), before the DB query — not after.
- Daily brief: skip operational section before calling `score_and_rank_items()`.

This means the DB method has no knowledge of the feature flag. The flag is a surface concern.

Citation: PR #1266 — established pattern for `MIKA_OPERATIONAL_PARTNER` gating; review-guide.md § Orthogonality (gate check placement affects which layer knows about the feature flag).

### 3.3 Integration test

- Full-stack test: synthetic items → `score_and_rank_items()` → verify correct ordering, status re-derivation, and batch-loaded dependency counts.
- Feature gate is not tested here (it's a surface concern for sub-issue 2b–2d tests).

## Decisions

- **No LLM in the ranking loop.** The scoring formula is the sole ranking authority. The LLM narrates after-the-fact (sub-issue 2c).
- **Re-derivation at read time with bounded cost.** Status is a cache of derivable state. Every read re-derives and updates. Worst case: 20 single-row UPDATEs per prompt-injection read, with 0–2 actual changes in practice. The alternative (periodic sweep) adds complexity without meaningful performance gain at N ≤ 50.
- **Batch-loaded dependency counts.** A single `GROUP BY blocked_by` query replaces per-item `COUNT(*)` calls. `priority()` is a pure function taking a pre-loaded HashMap — no DB access in the scoring loop.
- **Score persistence.** The computed `priority` score is written back to the DB column. Dashboard sorting uses the cached score. The re-computation on read ensures freshness for the specific surface that triggered it.
- **Today window = 24 hours.** Defined as `TODAY_WINDOW_HOURS` constant in `calibration.rs`. The foundation doc §4 says "today window" without defining exact hours; 24h is the default, tunable via the constant. This is a decision, not an open question.
- **AtRisk heuristics (initial set).** Two triggers: (a) stale near deadline (`updated_at` > 48h ago AND `due_at` within 24h); (b) callback failure evidence in `evidence_refs`. Expand heuristics based on operational experience in future sub-issues. This is a decision with a minimal-viable starting set — not an open question.
- **Feature gate at the surface, not the DB method.** Follows the Layer 1 pattern (PR #1266). `score_and_rank_items()` has no knowledge of `MIKA_OPERATIONAL_PARTNER`.

## Acceptance Criteria

1. `scoring.rs` implements the foundation §5 formula with all 6 terms, using batch-loaded dependency counts (no N+1 queries).
2. `calibration.rs` defines all scoring constants as named constants.
3. `status.rs` implements the 6-status derivation rules with Decision G precedence.
4. `score_and_rank_items()` on `AsyncDatabase` is the canonical read-path entry point.
5. Read-path re-derivation updates cached status with bounded worst-case: ≤ N single-row UPDATEs where N is the query limit (20 for prompt injection, 50 for dashboard pagination).
6. Unit tests cover each scoring term in isolation, deterministic ranking, status derivation precedence, and blocker cascade.
7. Integration test exercises the full `score_and_rank_items()` pipeline.

## Out of Scope (deferred to sub-issues 2b–2e)

- **CLI `mika next` command** → sub-issue 2b
- **Agent prompt injection** → sub-issue 2b
- **LLM explainer** → sub-issue 2c (F4 sharpening — test concerns — addressed there)
- **Daily brief integration** → sub-issue 2c
- **Dashboard `OperationalInbox`** → sub-issue 2d (F5 sharpening — component availability pin — addressed there)
- **HTTP API endpoint extensions** → sub-issue 2d
- **Memory rule encoding** → sub-issue 2e
- **Documentation updates** → sub-issue 2e

## Revision History

- rev 1 (2026-06-11): Initial plan — full 7-phase scope covering all of mika#1263.
- rev 2 (2026-06-11): Addressed architect first-pass findings:
  - **F1** (BLOCKING): Decomposed into 5 sub-issues (2a–2e) per review-guide.md § Single Responsibility and the #1259 decomposition precedent. This plan now covers sub-issue 2a only (scoring engine + status re-derivation + AsyncDatabase canonical read path).
  - **F2** (BLOCKING): Added explicit cost analysis table for read-path write amplification (§2.2). Worst case: ≤20 single-row UPDATEs per prompt-injection read; 0–2 actual changes in practice. Explicitly accepted with rationale.
  - **F3** (BLOCKING): Replaced per-item `db.count_blocked_items()` with batch-loaded `GROUP BY blocked_by` query (§1.3). `priority()` is now a pure function taking `&HashMap<String, u32>` — no DB access in the scoring loop.
  - **F4** (sharpening): Explainer test concerns deferred to sub-issue 2c plan. Noted in Out of Scope.
  - **F5** (sharpening): Dashboard component availability pin deferred to sub-issue 2d plan. Noted in Out of Scope.
  - **F6** (sharpening): Specified feature gate placement — gate at the surface, not the DB method, following PR #1266 pattern (§3.2).
  - **F7** (sharpening): Converted all 3 open questions to decisions with defaults and rationale (see Decisions section). No open questions remain.
- rev 3 (2026-06-11): Addressed architect second-pass findings (escalate-second-pass-after-iterate.md):
  - **F1** (sharpening): Verified against current code — `idx_operational_items_blocked_by` does **NOT** exist (5 indexes on `operational_items`: agent_status, agent_kind, agent_priority, source, source_unique). Removed the "uses existing index" claim in §1.3. Reframed as: full-scan acceptable at N≤50, sub-millisecond on SQLite. Added Phase 0 pin 7 with explicit index list verification and the v40-migration deferral note. No new migration added by this plan.
  - **F2** (sharpening): Verified against current code — `complete_operational_item()` (`crates/mika-agent/src/db/operational.rs:239`) only UPDATEs the single target row to `status = 'done'`; it does **NOT** cascade clear `blocked_by`. Reframed §2.3 `resolve_cleared_blockers()` as **primary cascade mechanism** (critical-path code), not defense-in-depth. Added critical-path implications (ordering requirement, bounded cost, test assertion shape). Added Phase 0 pin 6 with the verified `complete_operational_item` behavior pinned. Layer 1 write path is unchanged by this plan.

  Resolution methodology: orchestrator-CC verified both findings directly against the codebase (grep on index list, sed read on function body) before editing. Both findings resolved by plan-text revision — no code change required to address them; the verification is the work.
