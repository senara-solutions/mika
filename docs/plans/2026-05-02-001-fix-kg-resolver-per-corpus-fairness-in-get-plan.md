---
title: Per-corpus fairness in entity_resolver pending selection
type: fix
status: active
date: 2026-05-02
ticket: mika#927
---

# Per-corpus fairness in entity_resolver pending selection

## Overview

Refactor `entity_resolver::get_pending_entities()` so the pending pool returned to `resolve_entities()` is interleaved across corpora rather than concatenated in `kg_subject_entities.id` order. With the current ordering, the corpus that has the largest pending backlog dominates the Stage-2 LLM-call budget on every tick, starving smaller corpora for many ticks. Per-corpus selection + round-robin interleaving lets each corpus contribute Stage-2 attempts on every tick (subject to per-tick LLM budget).

## Problem Frame

`entity_resolver::resolve_pending(budget)` calls `get_pending_entities()` (no args) which issues a single SQL query against `kg_subject_entities` filtered by `e.docs_root_hash IN (?, ?, ...)` — multi-corpus fan-out per #798 — but with **no ORDER BY** and **no LIMIT**. SQLite's default row order without `ORDER BY` is implementation-defined but in practice follows rowid, so older inserts (the primary corpus, populated first) come first.

`resolve_entities()` (the shared per-doc and batch loop at `entity_resolver.rs:294`) then iterates the returned Vec linearly. Stage-1 exact matches don't debit the budget; Stage-2 LLM disambiguation calls do. Once `stats.llm_calls >= budget`, `resolve_single_entity` short-circuits Stage-2 with `SkippedBudget` for any remaining entity — Stage-1 still proceeds for free, but Stage-2 attempts on later corpora never happen.

**Concrete reproduction (mika#877 verification, 2026-05-01, post-#874+#876 deploy):**

| Corpus | Subjects | Attempted | Resolved | Pending | %/attempted |
|---|---:|---:|---:|---:|---:|
| mika (primary) | 30,211 | 12,673 | 8,972 | 17,538 | 70.8% |
| mika-skills | 174 | 102 | 54 | 72 | 52.9% |
| mika-platform | 103 | 48 | 23 | 55 | 47.9% |
| mika-cloud | 95 | 48 | 15 | 47 | 31.2% |

The secondaries' "Attempted" counts are ~50 cumulative — i.e., one or two ticks' allocation in the past, then nothing. Per the resolver-tick `Signal E` math, primary takes ~17–18 hours to drain to pending=0; secondaries are starved on Stage-2 throughout that window.

**Note on premise:** the ticket says "`get_pending_entities(budget: u32)` selects via a single SQL query." The actual signature today is `get_pending_entities(&self) -> Result<Vec<PendingEntity>>` (no budget) — see `crates/mika-agent/src/kg/entity_resolver.rs:910`. The budget cap lives in `resolve_entities`'s Stage-2 guard at line 318. The fix still lives in the selection function (or a wrapper around it), but the signature change is more substantial than the ticket's wording implied.

## Requirements Trace

- **R1.** After fix, with mika-arch's 4 corpora and varying pending pool sizes (16k+ on primary, ~50 each on secondaries), one tick of `resolve_pending(500)` produces non-zero Stage-2 attempts on every corpus that has pending entities (ticket AC #1).
- **R2.** Existing `entity_resolver` tests still pass (ticket AC #2).
- **R3.** New regression test seeds 4 corpora with pending pool sizes (1000, 50, 50, 50), calls `get_pending_entities(per_corpus_limit=50)` (or `resolve_pending(200)` end-to-end), asserts each corpus contributed at least 25 entities (ticket AC #3).
- **R4.** mika#877's `R3a` (>50% on primary + mika-skills) continues to verify (ticket AC #4).
- **R5.** mika#877's `R3b` (>0 attempts on mika-platform + mika-cloud) continues to verify (ticket AC #5).
- **R6.** Per-corpus throughput observable in tick logs: `kg_resolver_tick.complete` events include a per-corpus breakdown of `attempted_count` (ticket AC #6).

## Scope Boundaries

- **Out:** Extraction-side pending-doc selection (`subject_extractor`'s loop). May have a similar fairness story but is a separate ticket.
- **Out:** Resolver tick cadence (mika#906's 30-min interval is unchanged).
- **Out:** Domain graph coverage / match rate (separate concern — that's *match-rate*, this is *attempt-rate*).

### Deferred to Separate Tasks

- **Per-corpus Stage-2 budget caps** (vs. current per-tick total cap): if observation post-merge shows that one corpus's high miss-rate against its domain graph still consumes most of the budget despite fair selection, a follow-up may add per-corpus Stage-2 caps. File only if observed in production.

## Context & Research

### Relevant Code and Patterns

- `crates/mika-agent/src/kg/entity_resolver.rs:910` — `get_pending_entities()` — current global selection.
- `crates/mika-agent/src/kg/entity_resolver.rs:867` — `count_pending()` — mirrors the WHERE clause; will need a parallel update if we want per-corpus pending counts.
- `crates/mika-agent/src/kg/entity_resolver.rs:241` — `resolve_pending(budget)` — entry point that calls `get_pending_entities` and `resolve_entities`.
- `crates/mika-agent/src/kg/entity_resolver.rs:294` — `resolve_entities()` — iteration loop with Stage-2 budget guard.
- `crates/mika-agent/src/kg/entity_resolver.rs:126` — `ResolutionStats` struct — currently flat counters; will gain a per-corpus map.
- `crates/mika-agent/src/kg/entity_resolver.rs:155` — `SubjectEntityResolver` struct — already carries `docs_root_hashes: Vec<String>` (per #798 multi-corpus).
- `crates/mika-agent/src/kg/resolver_tick.rs:142` — `kg_resolver_tick.complete` log emission — where new per-corpus field lands.
- `crates/mika-agent/src/db.rs:1089` — `agent_kg_corpora` schema (PRIMARY KEY `(agent_id, docs_root_hash)`).
- `crates/mika-agent/tests/eval/kg_fixtures/mod.rs:435` — existing KG fixture pattern for integration tests.
- Existing in-file unit tests at `crates/mika-agent/src/kg/entity_resolver.rs:1517` — pure parsing tests, no DB. The new fairness test will need a DB fixture and likely belongs in the integration test tree, not the in-file unit tests.

### Institutional Learnings

- `mika/docs/solutions/best-practices/operator-db-evidence-disconfirmation-when-architect-cant-surface-premise-2026-04-30.md` — verify premise against runtime DB state. Already done here: confirmed via `mika#877` post-deploy snapshot that secondaries have ~50 cumulative attempts and large primary backlog.
- mika#874 / mika#875 / mika#876 (milestone#19) — sibling resolver/extractor fixes; their patterns for `docs_root_hashes` IN-list multi-corpus reads are the precedent this fix builds on.
- mika#906 — the 30-min resolver tick that this fix gives fair throughput across corpora.

### External References

None required. Bug + fix are local to the resolver module; no external library changes, no new crate dependencies.

## Key Technical Decisions

### KTD-1. Round-robin per-corpus selection over SQL window function

The ticket offered two alternatives: (1) per-corpus selection loop unioned in Rust; (2) SQL `ROW_NUMBER() OVER (PARTITION BY docs_root_hash ORDER BY id)` window function. **Choose (1)** for these reasons:

- **Testability:** each per-corpus query is a one-arg helper that integration tests can invoke directly to verify per-corpus selection without setting up the full resolver pipeline.
- **Per-corpus instrumentation:** R6 requires per-corpus `attempted_count`. With explicit per-corpus selection, attribution is trivial — each iteration knows which corpus the entity came from. With a single window-function query, attribution still works (project `docs_root_hash` in the SELECT), but the implementation lives in two places (SQL + Rust) instead of one.
- **Pending semantics complexity:** the existing pending-detection clause uses a correlated subquery against `kg_chunk_subjects` (lines 928-935 of `entity_resolver.rs`). Wrapping that in a windowed CTE is doable but obscures the read. The loop-based shape keeps the original SQL untouched and adds the partition logic in Rust.
- **No measurable perf advantage to window function** at expected scales: 4–8 corpora × budget-sized rows per corpus = O(thousands) of rows. Each per-corpus query hits the same `idx_kg_subject_entities` index path (same WHERE clause, just a single-element IN-list).

If a future follow-up shows the per-corpus loop is bottlenecking on round-trip count, revisit window-function approach as Phase 2.

### KTD-2. Per-corpus selection limit = `2 * total_budget`, with a floor of 50

Per-corpus limit must be:
- **Large enough** that Stage-1 hits don't waste the corpus's selection (Stage-1 doesn't debit budget but does consume "attempted" slots in the iteration). With Stage-1 hit rates around 99% historically, an entity-budget of N produces ~N Stage-1 hits and ~0.01N Stage-2 calls. The total budget across all corpora is the Stage-2 LLM cap (currently 500/tick), so each corpus could productively iterate well beyond `budget/num_corpora` entities before saturating.
- **Bounded** so the result Vec doesn't balloon. With 4 corpora and limit-per-corpus = 1000, we materialize 4000 PendingEntity rows — fine for memory.

**Decision:** `per_corpus_limit = max(2 * total_budget / num_corpora, 50)`. For the canonical case (budget=500, 4 corpora), per-corpus limit = 250. With 4 corpora that's 1000 rows materialized. The factor-of-2 oversupply gives Stage-1 hits room to register without prematurely capping primary throughput. The 50-floor protects single-corpus and 2-corpus cases (where `2 * 500 / 1 = 1000` but `2 * 500 / 2 = 500` — both fine; the floor mostly guards against pathological large `num_corpora`).

### KTD-3. Round-robin (zip) interleave order in the result Vec

Concatenating per-corpus Vecs (`[A_0, A_1, ..., A_n, B_0, B_1, ..., B_m, ...]`) re-creates the original starvation under iteration order. Instead, **zip the per-corpus Vecs** so iteration order is `[A_0, B_0, C_0, D_0, A_1, B_1, ...]`. This guarantees each corpus contributes a Stage-2 attempt at most `num_corpora` iterations after the previous corpus did, so a budget of even N=4 still distributes one Stage-2 attempt per corpus.

Implementation: a simple `interleave` helper using `Vec::iter` and `Iterator::next` round-robin until all corpora drained.

### KTD-4. Plumb `docs_root_hash` through `PendingEntity`

Currently `PendingEntity { id, entity_key, entity_type, name, confidence }` does not carry `docs_root_hash`. To attribute per-corpus `attempted_count` in `resolve_entities`, the entity must know which corpus it came from. Options:

1. Add `pub docs_root_hash: String` field to `PendingEntity`.
2. Pass a parallel `Vec<String>` of corpus tags alongside the entity Vec.
3. Group entities by corpus at the call site and attribute via index.

**Decision:** Option 1 — add the field. Reason: PendingEntity is internal to `entity_resolver.rs` (per `pub(crate)` scope and grep results), so the change is contained. It's also a property genuinely belonging to the entity. Option 2 risks index-misalignment bugs. Option 3 would prevent simple linear iteration.

Update the SELECT in `get_pending_entities_for_corpus` to project `e.docs_root_hash` (already in the WHERE clause). Bind to the new field.

### KTD-5. `ResolutionStats.per_corpus_attempted: HashMap<String, u32>` for log emission

`ResolutionStats` currently has flat counters. Add `pub per_corpus_attempted: HashMap<String, u32>` (and `Default::default()` produces empty map). In `resolve_entities`, increment `stats.per_corpus_attempted.entry(entity.docs_root_hash.clone()).or_insert(0) += 1` once per iteration (regardless of Stage-1 vs Stage-2 outcome — "attempted" means "iteration reached this entity").

Tick log emission: serialize the map as a single tracing field. The `tracing::info!` macro doesn't have great structured-map support. Use a stable JSON-encoded string:

```rust
per_corpus_attempted = %serde_json::to_string(&stats.per_corpus_attempted).unwrap_or_default(),
```

Operator dashboard parsing: the field becomes a JSON string within the JSON log line. Downstream parsers (Loki, jq, audit tooling) can re-parse. Alternative considered: emit one log line per corpus — rejected because it changes the line cardinality of `kg_resolver_tick.complete` from 1 per tick to N per tick, breaking existing log-counting assumptions.

## Open Questions

### Resolved During Planning

- **Q: Does the budget cap currently bound entity load count?** A: No. `get_pending_entities()` has no LIMIT. The SubjectEntityResolver loads all pending into memory (verified via SQL inspection at line 922). For very large primary corpora (17k+ rows), this is already loading thousands of rows into a Vec; the new per-corpus limit will actually *reduce* memory.
- **Q: Should `count_pending` mirror per-corpus selection?** A: Not for this fix. `count_pending` is observability-only (drives `pending_before` log field). Per-corpus pending counts can be a follow-up if operator workflow needs them. The new `per_corpus_attempted` field gives per-tick visibility, which is the actual operator question.
- **Q: Is `docs_root_hash` on `kg_subject_entities` indexed?** A: Yes — implicitly via WHERE clause matching `e.docs_root_hash IN (...)`. Per-corpus single-value queries hit the same index path. Verified by reading the schema at `crates/mika-agent/src/db.rs:1089`.

### Deferred to Implementation

- **Exact `interleave` helper signature:** could be a free function, a method on `Vec<Vec<T>>`, or a simple inline loop in `get_pending_entities`. Implementer's call once they see the call site.
- **Test fixture seeding strategy:** existing fixtures at `tests/eval/kg_fixtures/mod.rs` use a real SQLite DB. Whether to extend that fixture or build a fresh integration test with a temp DB is a per-test choice.
- **Whether to add a `per_corpus_pending: HashMap<String, u64>`** alongside `per_corpus_attempted` for symmetry: deferred — can add post-implementation if operator dashboard wants both.

## Implementation Units

- [ ] **Unit 1: Add `docs_root_hash` field to `PendingEntity` and update existing SELECTs**

**Goal:** Plumb the corpus identity through `PendingEntity` so iteration knows which corpus each entity came from. Necessary precondition for per-corpus instrumentation.

**Requirements:** R6 (per-corpus attempted_count requires this attribution).

**Dependencies:** None — first change.

**Files:**
- Modify: `crates/mika-agent/src/kg/entity_resolver.rs` (PendingEntity struct definition; existing `get_pending_entities` and any other constructor sites)
- Test: `crates/mika-agent/src/kg/entity_resolver.rs` (existing in-file `tests` module if any test constructs PendingEntity directly)

**Approach:**
- Add `pub docs_root_hash: String` to `PendingEntity`.
- Update the SELECT in the existing `get_pending_entities` (line 921-936) to project `e.docs_root_hash` and bind it (this is a transitional state — will be replaced in Unit 3).
- Audit other PendingEntity construction sites via grep — there should be only one (the SQL query); fail loudly if there are more.

**Patterns to follow:**
- Existing struct field ordering and visibility (all fields `pub`, see `entity_resolver.rs:30-50` area).

**Test scenarios:**
- Test expectation: none for this unit alone — it's a struct-field plumb. Verification falls out of Unit 5's regression test, which asserts a specific `docs_root_hash` value on returned entities.

**Verification:**
- `cargo build -p mika-agent` succeeds.
- `cargo test -p mika-agent --lib kg::entity_resolver` passes (existing parse tests in the in-file `tests` module still build with the augmented struct).

---

- [ ] **Unit 2: Add `get_pending_entities_for_corpus(docs_root_hash, limit)` helper**

**Goal:** Provide a single-corpus, bounded variant of the pending query that powers the round-robin selection in Unit 3. Each corpus selection becomes independently testable.

**Requirements:** R1, R3 (per-corpus selection is the structural fix).

**Dependencies:** Unit 1 (PendingEntity carries `docs_root_hash`).

**Files:**
- Modify: `crates/mika-agent/src/kg/entity_resolver.rs`

**Approach:**
- Add new private async method `get_pending_entities_for_corpus(&self, docs_root_hash: &str, limit: u32) -> Result<Vec<PendingEntity>>`.
- Reuse the existing pending-WHERE clause (the LEFT JOIN + correlated subquery against `kg_chunk_subjects`) but with `e.docs_root_hash = ?` (single param) and a trailing `ORDER BY e.id ASC LIMIT ?`. The `ORDER BY` is **load-bearing** — without it the per-corpus selection is non-deterministic.
- Project all PendingEntity fields including the new `docs_root_hash` (Unit 1).

**Patterns to follow:**
- Existing `get_pending_entities` query structure (lines 910-960) — same `with_db` async closure pattern, same boxed-ToSql params.

**Test scenarios:**
- *Happy path:* seed 100 entities for corpus `hash-A` and 50 for corpus `hash-B`. Call `get_pending_entities_for_corpus("hash-A", 30)`. Assert returned Vec has length 30 and all entities have `docs_root_hash == "hash-A"`.
- *Edge case:* empty corpus — call against a `docs_root_hash` that has zero pending entities. Assert returns empty Vec, no error.
- *Edge case:* limit larger than pending pool — corpus has 10 pending, call with limit=100. Assert returns all 10.
- *Edge case:* `docs_root_hash` not in `agent_kg_corpora` — assert returns empty Vec (the WHERE filter naturally excludes; no special handling needed).

**Verification:**
- `cargo test -p mika-agent --test eval` (or equivalent integration test command) passes the new per-corpus selection tests.
- Existing tests untouched; no broader regression.

---

- [ ] **Unit 3: Refactor `get_pending_entities()` to round-robin across corpora**

**Goal:** Replace the global pending query with a per-corpus loop + interleave. After this unit, `resolve_entities` iterates a fairness-ordered Vec.

**Requirements:** R1, R3, R4, R5.

**Dependencies:** Unit 2 (helper exists), Unit 1 (PendingEntity carries `docs_root_hash`).

**Files:**
- Modify: `crates/mika-agent/src/kg/entity_resolver.rs`

**Approach:**
- Change `get_pending_entities` signature: `get_pending_entities(&self, total_budget: u32) -> Result<Vec<PendingEntity>>` (was no-arg). The caller (`resolve_pending`) already has `budget: u32` and passes it through.
- Compute `per_corpus_limit = std::cmp::max(2 * total_budget / num_corpora, 50)` where `num_corpora = self.docs_root_hashes.len()` (guard against zero — return empty Vec).
- For each `docs_root_hash` in `self.docs_root_hashes`, call `get_pending_entities_for_corpus(hash, per_corpus_limit)`. Collect into `Vec<Vec<PendingEntity>>`.
- Round-robin interleave the result: implement an `interleave_round_robin(buckets: Vec<Vec<PendingEntity>>) -> Vec<PendingEntity>` helper (private free function) that pops `buckets[0][0], buckets[1][0], ..., buckets[N-1][0], buckets[0][1], ...` until all buckets empty.
- Update the call site in `resolve_pending` to pass `budget` (line 245: `let pending = self.get_pending_entities().await?` → `let pending = self.get_pending_entities(budget).await?`).

**Patterns to follow:**
- Existing async `with_db` callback pattern is unchanged — only the orchestration layer is new.
- `docs_root_hashes` iteration: the field is already `Vec<String>` on `SubjectEntityResolver` per #798.

**Test scenarios:**
- *Happy path (R1, ticket AC #1, ticket AC #3):* seed 4 corpora with pending pool sizes (1000, 50, 50, 50). Call `get_pending_entities(200)`. Assert returned Vec has length ≥ 200, contains entries from all 4 corpora, and the first 4 entries have 4 distinct `docs_root_hash` values (round-robin interleave proof).
- *Edge case:* single corpus — `docs_root_hashes = ["only-hash"]`, 100 pending. Call `get_pending_entities(50)`. Assert returns ≤50 entities (per-corpus limit applied), all with the same hash. Iteration order = insertion order (stable).
- *Edge case:* zero corpora — `docs_root_hashes = []`. Assert returns empty Vec, no error.
- *Edge case:* corpora with empty pending — 4 corpora, only 1 has pending entities. Assert returns those entities and skips the empty ones.
- *Integration:* run `resolve_pending(500)` against the (1000,50,50,50) seeded fixture. Assert each corpus contributed at least 25 attempts (ticket AC #3).

**Verification:**
- All four secondary corpora produce non-zero attempts in the integration test.
- Existing `entity_resolver` tests pass (R2).

---

- [ ] **Unit 4: Add `per_corpus_attempted` to `ResolutionStats` and emit in tick log**

**Goal:** Make per-corpus throughput visible at `kg_resolver_tick.complete` so operators (and future audits) can confirm fairness in production.

**Requirements:** R6.

**Dependencies:** Unit 1 (PendingEntity has `docs_root_hash`).

**Files:**
- Modify: `crates/mika-agent/src/kg/entity_resolver.rs` (ResolutionStats struct, `resolve_entities` increment site)
- Modify: `crates/mika-agent/src/kg/resolver_tick.rs` (info!() at `kg_resolver_tick.complete`)

**Approach:**
- Add `pub per_corpus_attempted: HashMap<String, u32>` to `ResolutionStats`. Default to empty.
- In `resolve_entities` loop (line 310), at top of each iteration: `*stats.per_corpus_attempted.entry(entity.docs_root_hash.clone()).or_insert(0) += 1`. "Attempted" = iteration reached this entity, regardless of Stage-1 success or Stage-2 budget skip.
- In `resolver_tick.rs` info!() call at `kg_resolver_tick.complete` (line 142), add a field: `per_corpus_attempted = %serde_json::to_string(&stats.per_corpus_attempted).unwrap_or_default()`.
- Confirm `serde_json` is already a dep in `mika-agent`'s Cargo.toml; if not, add it (it almost certainly is — entity_resolver already does JSON parsing).

**Patterns to follow:**
- Existing tracing fields in `kg_resolver_tick.complete` (resolver_tick.rs:129-143) — additive new field, no removal.
- Existing serialization patterns — the file already uses `serde_json` for disambiguation parsing; reuse the dep.

**Test scenarios:**
- *Happy path:* Unit 5's integration test asserts `stats.per_corpus_attempted` contains all 4 expected `docs_root_hash` keys with non-zero values.
- *Edge case:* empty pending — `per_corpus_attempted` is an empty HashMap; serializes to `"{}"`. No log breakage.
- *Edge case:* single corpus — only one key in the map.

**Verification:**
- A real `kg_resolver_tick.complete` log line from a manual smoke test (after merge + deploy) shows the `per_corpus_attempted` field with non-empty values.
- `grep per_corpus_attempted /var/log/mika/server.log` returns hits for ticks across all KG-enabled agents.

---

- [ ] **Unit 5: Regression test — per-corpus fairness end-to-end**

**Goal:** Lock in R1, R3, R4, R5 with a self-contained integration test that future refactors must keep green.

**Requirements:** R3 (the explicit acceptance criterion).

**Dependencies:** Units 1–4 complete.

**Files:**
- Create or modify: `crates/mika-agent/tests/eval/per_corpus_fairness.rs` (new test file in the integration tree, or extend an existing file in `tests/eval/`)

**Approach:**
- Use the existing `kg_fixtures` pattern from `crates/mika-agent/tests/eval/kg_fixtures/mod.rs:435+` — temp DB, register agent, seed `agent_kg_corpora` rows for 4 corpora.
- Seed `kg_subject_entities` with pending pool sizes (1000, 50, 50, 50) — large primary, small secondaries, all with `type IN ('skill', 'tool', 'agent', 'problem_type')`.
- Construct `SubjectEntityResolver` with the 4 `docs_root_hashes`. Use exact-match-only mode (no LLM) so Stage-1 alone exercises the iteration; budget=200.
- Call `resolve_pending(200)`.
- **Assertion 1 (R1, R3):** `stats.per_corpus_attempted` has all 4 keys and each value ≥ 25.
- **Assertion 2:** total returned Vec from a separate `get_pending_entities(200)` call ≤ `4 * per_corpus_limit` (sanity check on selection bound).
- **Assertion 3 (R4 sanity):** if seeded with valid domain entities for primary, primary's resolution rate is non-zero (Stage-1 hits work).

**Patterns to follow:**
- `tests/eval/kg_fixtures/mod.rs:435+` schema setup pattern (CREATE TABLE statements verbatim from production schema).
- Existing eval-tree integration tests for resolver behavior (grep `tests/eval/` for resolver-related fixtures).

**Test scenarios:**
- *Happy path (the AC):* 4 corpora (1000, 50, 50, 50) → each contributes ≥25 attempts in one tick.
- *Edge case (R4 continued verification):* with valid domain corpus, primary hits >50% Stage-1 rate (mika#877 R3a — recreate via fixture).
- *Edge case (R5 continued verification):* mika-platform + mika-cloud secondaries (the smaller corpora) get >0 attempts (mika#877 R3b — recreate via fixture).

**Verification:**
- `cargo test -p mika-agent --test eval per_corpus_fairness` (or equivalent — existing eval test runner) passes.
- Test fails if Unit 3 is reverted (selectively reintroducing the global SQL query): the 4-corpus fairness assertion catches the regression.

## System-Wide Impact

- **Interaction graph:** the change is local to the `kg::entity_resolver` module. Callers of `resolve_pending` (the `resolver_tick`, the per-doc `IngestionOrchestrator` compound hook path, and any startup spawn) are unaffected — `resolve_pending`'s signature does not change. `get_pending_entities`'s signature changes from `()` to `(total_budget: u32)`, but it's a private method (`async fn`, no `pub`), so call sites are confined to this file.
- **Error propagation:** unchanged. Each per-corpus query can fail independently; the loop returns the first error (early-exit via `?`). This matches the existing single-query failure shape — `resolve_pending` propagates the error to the tick handler, which logs `kg_resolver_tick.error`.
- **State lifecycle risks:** none new. Selection is read-only; writes still go through `resolve_entities` as before. No new transactions, no new connections — each per-corpus query reuses the existing `with_db` async DB handle.
- **API surface parity:** `ResolutionStats` gains a public field. Anyone deserializing `ResolutionStats` (none in-tree per grep) would see an additive change. `kg_resolver_tick.complete` log line gains `per_corpus_attempted` field — additive.
- **Integration coverage:** per-corpus fairness is invariant-level. Unit 5's test is the canonical proof; manual production verification post-deploy via `grep per_corpus_attempted /var/log/mika/server.log` confirms.
- **Unchanged invariants:** writes to `kg_subject_resolutions` and `kg_resolutions_log` semantics; Stage-1 vs Stage-2 budget guard at `entity_resolver.rs:318`; tick cadence and lifecycle (mika#906); existing pending-detection WHERE clause (the LEFT JOIN against `kg_resolutions_log` and the correlated subquery against `kg_chunk_subjects`); the `MIKA_KG_RESOLUTION_MODEL` Stage-2 model (per C2.1).

## Risks & Dependencies

| Risk | Mitigation |
|------|------------|
| Per-corpus query loses determinism without `ORDER BY`. | Add `ORDER BY e.id ASC` in `get_pending_entities_for_corpus` SELECT. Existing test seeds verify selection order. |
| `PendingEntity` field addition breaks construction sites elsewhere. | Single producer (the SQL query); verified via grep. Compiler enforces. |
| `per_corpus_attempted` HashMap serialization adds noise to log lines. | Field is JSON-encoded — adds ~100 bytes per tick at 4 corpora. Negligible vs. existing log volume. |
| Floor on per-corpus limit (50) wastes budget on tiny secondaries. | 50 entities per corpus is small absolute cost; ensures each corpus gets fair access even when `2 * total_budget / num_corpora < 50`. Acceptable. |
| Round-robin interleave allocates intermediate `Vec<Vec<>>`. | At 4 corpora × ~250 limit = 1000 PendingEntity rows in flight. Memory cost negligible. |
| Existing `count_pending` does not gain per-corpus breakdown. | Out of scope; followup-able. `pending_before` log field still works (sums across corpora). |

## Documentation / Operational Notes

- **Operator-side smoke test post-merge:** after deploy, `grep '"event":"kg_resolver_tick.complete"' /var/log/mika/server.log | grep per_corpus_attempted | tail -5` should show the new field with all four mika-arch corpora present.
- **No schema migration:** the fix is pure code; no `schema_version` bump needed.
- **No config flag:** the change is behavior-preserving in the single-corpus case (single-element interleave is a no-op) so it can ship without a feature flag.

## Sources & References

- **Origin issue:** mika#927 — fix(kg/resolver): per-corpus fairness in get_pending_entities()
- **Parent milestone:** mika#19 — KG flawlessness — extraction + resolution defects
- **Sibling tickets:**
  - mika#874 — resolver Stage-2 candidate-list (closed in milestone#19)
  - mika#876 — subject_extractor parse-tolerance (closed in milestone#19)
  - mika#877 — mika-arch secondary corpora + CLI (closed in milestone#19; surfaced this gap)
  - mika#906 — periodic resolver tick
  - mika#928 — domain graph expansion (separate, match-rate vs attempt-rate)
- **Related code:** `crates/mika-agent/src/kg/entity_resolver.rs:910` (the function), `crates/mika-agent/src/kg/resolver_tick.rs:142` (log emission), `crates/mika-agent/src/db.rs:1089` (schema)
- **Prior solutions doc:** `mika/docs/solutions/best-practices/operator-db-evidence-disconfirmation-when-architect-cant-surface-premise-2026-04-30.md` — applied to verify premise pre-grooming.
