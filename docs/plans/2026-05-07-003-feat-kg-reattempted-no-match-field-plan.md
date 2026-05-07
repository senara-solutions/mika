---
title: "feat(kg): emit reattempted_no_match field on kg_resolver_tick.complete"
type: feat
status: active
date: 2026-05-07
---

# feat(kg): emit reattempted_no_match field on kg_resolver_tick.complete

## Overview

Add a `reattempted_no_match: u32` field to the `kg_resolver_tick.complete` structured log event (and `resolution_pending_complete`) so operators can answer: "of this tick's resolved entities, how many were re-attempts from prior `no_match` outcomes invalidated by #960's domain-graph rebuild?"

## Problem Frame

Issue #960 invalidates `no_match` resolution log rows when domain graph rebuilds add new entities of a type. After invalidation (DELETE from `kg_resolutions_log`), those entities re-enter the pending pool via the `r.id IS NULL` path. However, they are indistinguishable from never-resolved entities. Operators need per-tick visibility into how many pending entities are reattempts from prior `no_match` outcomes vs genuinely new entities.

## Requirements Trace

- R1. `kg_resolver_tick.complete` log event includes new field `reattempted_no_match: u32`
- R2. Field is zero on ticks following a rebuild with no invalidation (steady state)
- R3. Field is non-zero on the first tick after a rebuild that invalidated rows for an extant type
- R4. Unit test in `entity_resolver.rs::tests` covers both zero and non-zero paths
- R5. Field is also emitted on `resolution_pending_complete` (startup resolver path) for immediate post-rebuild visibility

## Scope Boundaries

- No changes to the rebuild-side invalidation event (`domain_rebuild_invalidated_resolutions`) — covered by #960
- No re-resolution of non-`no_match` outcomes (e.g., `matched_llm` when better candidates appear)
- No surfacing the count on the tick start event (start-time count is misleading)

### Deferred to Separate Tasks

- Integration with #960's domain builder (one-line call to `record_invalidated_no_match`): done when #960 merges. Resolver-side is complete and independently testable.

## Context & Research

### Relevant Code and Patterns

- `crates/mika-agent/src/kg/entity_resolver.rs` — `ResolutionStats` struct (line 128-147), `resolve_entities` loop (line 299-398), `apply_result` (line 400+)
- `crates/mika-agent/src/kg/resolver_tick.rs` — `kg_resolver_tick.complete` event (line 131-146)
- `crates/mika-agent/src/kg/domain_builder.rs` — #960's invalidation step (on branch `fix/960/kg-domain-graph-rebuild-must-invalidate`, commit `6ed6cbbf`)
- `crates/mika-agent/src/db.rs` — schema migrations, `CURRENT_SCHEMA_VERSION = 31`
- `crates/mika-agent/src/db/kg_schema.rs` — KG-specific DB helpers
- Pattern: `per_corpus_attempted` field added in #927 — most recent example of adding a counter to `ResolutionStats` + emitting on tick log

### Institutional Learnings

- `docs/solutions/874-kg-resolver-candidate-list-db-fallback.md` — Resolution outcome taxonomy and `ResolutionStats` counter pattern
- `docs/solutions/kg/906-resolver-tick-periodic-drain-2026-04-30.md` — Three execution contexts share `resolve_pending(budget)` path; any new field must work across all three
- `docs/solutions/best-practices/kg-resolver-tick-visibility-audit-2026-05-06.md` — Counter scope must match the 5-type allowlist (`skill`, `tool`, `agent`, `problem_type`, `concept`)
- `docs/solutions/logic-errors/kg-resolver-per-corpus-starvation-2026-05-02.md` — `per_corpus_attempted` pattern for adding observability fields

## Key Technical Decisions

- **Sidecar table `kg_invalidated_no_match` over in-memory tracker:** A DB table survives crashes between domain rebuild and resolver tick, works across all three execution contexts (startup, compound-hook, periodic tick), and avoids threading `Arc<Mutex<HashSet>>` through the application. The table is ephemeral (rows cleaned up after resolution) and tiny (bounded by the number of previously-`no_match` entities per rebuild).

- **Record + DELETE pattern over soft-delete on `kg_resolutions_log`:** Adding an `invalidated_at` column to `kg_resolutions_log` would require expanding the CHECK constraint (another table rebuild migration) and modifying the pending query. A separate sidecar table is less invasive and keeps #960's clean DELETE semantics.

- **Detection at query time (pending-entity fetch) over per-entity check:** Extend `get_pending_entities_for_corpus` to LEFT JOIN the sidecar table and return `was_invalidated: bool` on `PendingEntity`. This avoids N+1 queries during the resolution loop.

- **Cleanup in `write_log` over cleanup in `apply_result`:** Adding a defensive DELETE to `write_log()` ensures markers are cleaned up from ANY code path that writes a resolution log row (startup, compound-hook, tick), not just the `resolve_pending` path. Prevents orphan markers from compound-hook races (flow analysis gap #3).

- **Helper function `record_invalidated_no_match` in `kg_schema.rs`:** Isolates the sidecar write into a reusable function that the domain builder calls. Preserves the entity resolver's sole-writer contract for `kg_resolutions_log` — the domain builder uses a separate module for the marker table. The `kg_resolutions_log` DELETE stays in `domain_builder.rs` (#960's code).

## Open Questions

### Resolved During Planning

- **Module ownership for invalidation markers:** The sidecar table `kg_invalidated_no_match` is written by `domain_builder` (via helper in `kg_schema.rs`) and read/cleaned by `entity_resolver`. Both modules import the shared helper. The entity resolver's sole-writer contract for `kg_resolutions_log` is unchanged — `domain_builder` deletes from `kg_resolutions_log` (per #960), the entity resolver writes to it. The sidecar table has no sole-writer contract (write-once-read-once lifecycle).

- **Table schema PK:** `(subject_entity_id, agent_id)` composite PK, matching `kg_resolutions_log`'s per-agent granularity. A single subject entity shared across agents (via `docs_root_hash`) can have separate `no_match` log rows per agent.

- **Race between startup resolver and periodic tick:** The first tick is skipped by design (`interval.tick().await` on resolver_tick.rs line 69), guaranteeing the startup resolver runs before any tick sees invalidated entities. Counter inflation is impossible under this invariant.

- **`INSERT OR IGNORE` for crash-restart-crash-restart:** If the server restarts twice and the first rebuild already inserted markers for the same entities, `INSERT OR IGNORE` handles the UNIQUE constraint gracefully.

### Deferred to Implementation

- Exact column ordering in the new migration DDL

## Implementation Units

- [ ] **Unit 1: Schema v32 — add `kg_invalidated_no_match` table**

**Goal:** Create the sidecar table that records which subject entities had their `no_match` resolution log rows deleted by #960's domain-graph rebuild invalidation.

**Requirements:** R1, R2, R3 (foundational for all)

**Dependencies:** None

**Files:**
- Modify: `crates/mika-agent/src/db.rs` (migration v31→v32, bump `CURRENT_SCHEMA_VERSION`, AND add table to `migrate_v1` clean-install DDL)
- Modify: `crates/mika-agent/src/db/kg_schema.rs` (add `record_invalidated_no_match` and `clear_invalidated_no_match` helpers)
- Modify: `CLAUDE.md` and `crates/mika-agent/CLAUDE.md` (schema version reference)

**Approach:**
- Table DDL: `kg_invalidated_no_match(subject_entity_id INTEGER NOT NULL, agent_id TEXT NOT NULL, invalidated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')), PRIMARY KEY (subject_entity_id, agent_id))`. No FK constraints (lightweight marker, not relational integrity).
- Migration: `ALTER TABLE` is not needed — just `CREATE TABLE IF NOT EXISTS` in the v31→v32 migration block. Also add the CREATE TABLE to `migrate_v1`'s clean-install DDL (fresh databases skip incremental migrations).
- Helper `record_invalidated_no_match(conn, agent_id, subject_entity_ids: &[i64])`: batch `INSERT OR IGNORE INTO kg_invalidated_no_match` for each ID.
- Helper `clear_invalidated_no_match(conn, agent_id, subject_entity_id)`: single-row DELETE.
- Both helpers take `&rusqlite::Connection` (called inside `with_db` closures).

**Patterns to follow:**
- v30→v31 migration pattern in `db.rs` (additive CREATE TABLE)
- `write_log` helper pattern in `kg_schema.rs` for DB helpers

**Test scenarios:**
- Happy path: `record_invalidated_no_match` inserts rows, `clear_invalidated_no_match` removes them
- Edge case: `INSERT OR IGNORE` — calling `record_invalidated_no_match` twice with the same IDs does not error
- Edge case: `clear_invalidated_no_match` on non-existent row is a no-op (no error)

**Verification:**
- `cargo test -p mika-agent` passes with the new schema version
- Fresh DB creation reaches v32

- [ ] **Unit 2: Extend `PendingEntity` with `was_invalidated` flag and detection query**

**Goal:** Modify the pending-entity query to detect which entities were invalidated by a prior domain rebuild, and carry that information through the resolution loop.

**Requirements:** R1, R3

**Dependencies:** Unit 1

**Files:**
- Modify: `crates/mika-agent/src/kg/entity_resolver.rs` (extend `PendingEntity` struct, modify `get_pending_entities_for_corpus` query, update `get_entities_by_ids`, add `reattempted_no_match: u32` to `ResolutionStats`)

**Approach:**
- Add `pub was_invalidated: bool` to `PendingEntity` struct.
- Modify `get_pending_entities_for_corpus` SQL to LEFT JOIN `kg_invalidated_no_match inv ON inv.subject_entity_id = e.id AND inv.agent_id = ?1`. Return `(inv.subject_entity_id IS NOT NULL) AS was_invalidated` as a new SELECT column.
- Update `get_entities_by_ids` (line ~826, used by compound-hook `resolve_doc_entities`) to hardcode `was_invalidated: false` — compound-hook resolves freshly extracted entities, not invalidation reattempts.
- Add `pub reattempted_no_match: u32` to `ResolutionStats` (derives `Default` → starts at 0).

**Patterns to follow:**
- `per_corpus_attempted` field addition in #927 (counter on `ResolutionStats`)
- Existing `PendingEntity` struct and query mapping pattern

**Test scenarios:**
- Happy path: pending entity with a marker in `kg_invalidated_no_match` has `was_invalidated = true`
- Happy path: pending entity without a marker has `was_invalidated = false`
- Edge case: entity is pending via trace-id mismatch (re-extraction) AND has an invalidation marker — `was_invalidated` should still be true (both conditions can co-occur)

**Verification:**
- `PendingEntity` struct correctly carries the flag from the query
- `ResolutionStats` has the new field defaulting to 0

- [ ] **Unit 3: Increment counter in resolution loop and clean up markers**

**Goal:** Wire the `was_invalidated` flag to the `reattempted_no_match` counter and clean up sidecar markers after resolution.

**Requirements:** R1, R2, R3

**Dependencies:** Unit 2

**Files:**
- Modify: `crates/mika-agent/src/kg/entity_resolver.rs` (`resolve_entities` loop, `write_log` method)

**Approach:**
- In `resolve_entities`, after `apply_result` for each entity: if `entity.was_invalidated` and the result wrote a log row (any outcome except `SkippedBudget` — i.e., `ExactMatch`, `LlmMatch`, `LlmMatchDbFallback`, `NoMatch`, `SkippedDiscoveredType`, `SkippedNoLlm`, `Error`), increment `stats.reattempted_no_match`. `SkippedBudget` entities stay pending with their marker intact for the next tick.
- In `write_log` (the shared log-row writer), add a defensive `DELETE FROM kg_invalidated_no_match WHERE subject_entity_id = ?1 AND agent_id = ?2` after the UPSERT. This ensures cleanup from ALL code paths (startup, compound-hook, tick) — not just `resolve_pending`.
- The cleanup DELETE uses `clear_invalidated_no_match` helper from Unit 1.

**Patterns to follow:**
- Existing counter increment pattern in `resolve_entities` (e.g., `stats.llm_calls`)
- `write_log` method's existing UPSERT + audit event pattern

**Test scenarios:**
- Happy path: 3 pending entities with `was_invalidated=true`, all resolve → `reattempted_no_match=3`
- Happy path: 3 pending entities with `was_invalidated=false` → `reattempted_no_match=0`
- Happy path: mix of 2 invalidated + 3 non-invalidated → `reattempted_no_match=2`
- Edge case: invalidated entity hits `SkippedBudget` → NOT counted in `reattempted_no_match` (entity stays pending, marker stays)
- Edge case: `write_log` called from compound-hook path cleans up sidecar marker even though no counter was incremented (cleanup is unconditional in `write_log`)
- Integration: after resolution, `kg_invalidated_no_match` table is empty for processed entities

**Verification:**
- `reattempted_no_match` counter accurately reflects invalidated entities that were actually re-resolved
- Sidecar table rows are cleaned up after resolution

- [ ] **Unit 4: Emit field on log events**

**Goal:** Surface `reattempted_no_match` on both `kg_resolver_tick.complete` and `resolution_pending_complete` structured log events.

**Requirements:** R1, R5

**Dependencies:** Unit 3

**Files:**
- Modify: `crates/mika-agent/src/kg/resolver_tick.rs` (add field to `kg_resolver_tick.complete` event)
- Modify: `crates/mika-agent/src/kg/entity_resolver.rs` (add field to `resolution_pending_complete` event)

**Approach:**
- In `resolver_tick.rs` line 131-146: add `reattempted_no_match = stats.reattempted_no_match` to the `info!` macro.
- In `entity_resolver.rs` line 381-395: add `reattempted_no_match = stats.reattempted_no_match` to the `resolution_pending_complete` `info!` macro.

**Patterns to follow:**
- Existing field emissions in both log events (e.g., `per_corpus_attempted`, `llm_calls`)

**Test scenarios:**
Test expectation: none — log emission is verified by inspection and covered by the integration tests in Unit 5 that assert `ResolutionStats` field values.

**Verification:**
- `reattempted_no_match` appears in both log events
- Field is 0 when no invalidation occurred, non-zero when it did

- [ ] **Unit 5: Unit tests for both paths**

**Goal:** Cover both the zero and non-zero `reattempted_no_match` paths with deterministic tests.

**Requirements:** R4

**Dependencies:** Units 1-4

**Files:**
- Modify: `crates/mika-agent/src/kg/entity_resolver.rs` (add tests in `#[cfg(test)] mod tests`)

**Approach:**
- Test 1 (non-zero path): Seed subject entities with provenance, seed `kg_invalidated_no_match` markers, seed domain entities for exact match. Run `resolve_pending`. Assert `stats.reattempted_no_match > 0` and sidecar table is empty after.
- Test 2 (zero path): Seed subject entities with provenance but NO sidecar markers. Run `resolve_pending`. Assert `stats.reattempted_no_match == 0`.
- Both tests use `MockLlmProvider` (no network) — exact-match resolution only.
- Follow existing test patterns (e.g., `f3_path1_in_list_accept` test setup).

**Patterns to follow:**
- Existing entity resolver tests at line 1912+ (DB seeding, `MockLlmProvider`, assertion on resolution outcomes)
- `seed_subjects_with_resolutions` helper from #960's domain_builder tests for seeding subject entities

**Test scenarios:**
- Happy path: 5 concept-typed subjects with invalidation markers, 1 matching domain entity → `reattempted_no_match=5`, markers cleaned up
- Happy path: 5 concept-typed subjects without invalidation markers → `reattempted_no_match=0`
- Edge case: mix of invalidated and non-invalidated pending entities → counter reflects only invalidated ones
- Edge case: invalidated entity that resolves as `no_match` again (no domain match) — still counted in `reattempted_no_match`, marker cleaned up, new `no_match` log row written

**Verification:**
- `cargo test -p mika-agent` passes
- Both zero and non-zero paths have explicit assertions

## System-Wide Impact

- **Interaction graph:** Domain builder (writes markers) → entity resolver (reads markers, increments counter, cleans up) → resolver tick (emits field on log event). Compound-hook path also cleans up markers via `write_log` but does not increment counter.
- **Error propagation:** Sidecar table operations are non-critical. If `record_invalidated_no_match` fails, the invalidation still works (entities become pending); the counter just reports 0. If `clear_invalidated_no_match` fails, the marker lingers but is harmless (next resolution of the same entity will see it again).
- **State lifecycle risks:** Markers accumulate if entities are never re-resolved (e.g., perpetually budget-skipped). Bounded by the number of `no_match` entities invalidated per rebuild — typically < 100 per mika-arch corpus. No cleanup sweep needed for v1.
- **API surface parity:** No dashboard or API changes — this is log-only observability.
- **Unchanged invariants:** `kg_resolutions_log` schema unchanged. Entity resolver's sole-writer contract for `kg_resolutions_log` and `kg_subject_resolutions` unchanged. Domain builder's sole-writer contract for `kg_entities` and `kg_relationships` unchanged. The new `kg_invalidated_no_match` table has no sole-writer contract (write-once-read-once lifecycle). The entity resolver module docstring should be updated to note it reads/deletes from `kg_invalidated_no_match`.

## Risks & Dependencies

| Risk | Mitigation |
|------|------------|
| #960 not merged — domain builder doesn't write markers yet | Resolver-side is complete and testable. Counter is 0 until #960 integration (one-line call). Tests seed the sidecar table directly. |
| Schema v32 conflicts with another in-flight migration | Current schema is v31. No other v32 migration in progress. |
| Counter double-counts if startup resolver and tick race | First tick is skipped by design (`interval.tick().await`). Documented as "at least N" semantics. |

## Documentation / Operational Notes

- `CLAUDE.md` Signal F observation instructions should be updated to include the `reattempted_no_match` field once #960 is merged: `grep kg_resolver_tick.complete server.log | jq '.reattempted_no_match'`
- The field is zero on all ticks until #960 merges and the domain builder calls `record_invalidated_no_match`

## Sources & References

- Related issues: #960 (parent — domain-graph rebuild invalidation), #906 (periodic resolver tick), #927 (per-corpus fairness — most recent `ResolutionStats` addition)
- Related code: `crates/mika-agent/src/kg/entity_resolver.rs`, `crates/mika-agent/src/kg/resolver_tick.rs`, `crates/mika-agent/src/kg/domain_builder.rs`
- #960 implementation: branch `fix/960/kg-domain-graph-rebuild-must-invalidate`, commit `6ed6cbbf`
