---
ticket: mika issue#960
type: fix
component: kg
sequence: 2026-05-03-001
status: proposed
---

# fix(kg): domain-graph rebuild invalidates `no_match` resolutions for newly-added entity types

## Goal

When `domain_builder.rebuild()` adds N≥1 new entities of type T (skill / tool / agent / problem_type / concept), invalidate every `kg_resolutions_log` row where `outcome='no_match'` AND the subject's type is T, so the next resolver tick re-attempts those subjects against the expanded domain graph.

Acceptance criteria from the ticket — restated for self-containment:

- A1. After rebuild adds N entities of type T, the immediately-following resolver tick (or startup spawn) re-attempts all `outcome='no_match'` subjects where `kg_subject_entities.type = T`.
- A2. Re-attempt count is observable via a new field on `kg_resolver_tick.complete` — `reattempted_no_match: u32`.
- A3. Unit test: 1 `concept` domain entity added; 5 `concept`-typed subjects with prior `outcome='no_match'`; after invalidation+resolve, 5 are re-attempted with fresh `resolved_at`.
- A4. New structured log event `domain_rebuild_invalidated_resolutions` carries per-type counts: `{added_type: "concept", invalidated_no_match: N}`.

## Why this approach

Two viable design points; I'm taking the first and naming the alternative.

**Chosen — invalidate inside the rebuild transaction.** Right after entity UPSERT and before commit, run one `DELETE FROM kg_resolutions_log` per type with `entities_added_for_type[T] > 0`. The invalidation is atomic with the rebuild — if the transaction rolls back, the log rows survive. Resolutions and the domain graph stay consistent.

**Rejected — invalidate in server init between rebuild and resolver_tick spawn.** This would split the semantic invariant ("expanded domain graph implies stale `no_match` rows") across two transactions. If the rebuild commits but the server crashes before the invalidation step, we end up in the exact failure state #960 is trying to prevent. Atomicity wins.

## Files touched

- `crates/mika-agent/src/kg/domain_builder.rs` — invalidation step + `RebuildStats` extension + log event
- `crates/mika-agent/src/kg/resolver_tick.rs` — `reattempted_no_match` field on `kg_resolver_tick.complete`
- `crates/mika-agent/src/kg/entity_resolver.rs` — count rows deleted when stale-extraction-trace branch fires (for the `reattempted_no_match` observability — see KTD-2 below)
- `crates/mika-agent/src/db/kg_schema.rs` — no DDL change. The CHECK constraint on `outcome` already permits all values; we only DELETE.
- Tests — new unit test in `domain_builder.rs::tests`; new fixture in `tests/eval/kg_fixtures/` if needed.

No schema migration. No new env var. No public-API change to the `RebuildStats` struct fields beyond the additive `invalidated_no_match` HashMap.

## Implementation

### Step 1 — Track per-type adds in `RebuildStats`

`domain_builder.rs::rebuild()` already iterates types and emits the `domain_rebuild_entities` log event with `added` counts (line 443). Aggregate the per-type `added` into a `HashMap<String, usize>` field on `RebuildStats`:

```rust
pub struct RebuildStats {
    // existing fields...
    /// Per-type counts of `outcome='no_match'` log rows invalidated this rebuild.
    /// Populated after Step 2 runs. Empty when no type had `added > 0`.
    pub invalidated_no_match: HashMap<String, usize>,
}
```

### Step 2 — Invalidate `no_match` rows for types with new entities

Inside the same transaction, after entity UPSERT and edge rebuild but before `tx.commit()`, for each `type` in `KG_DOMAIN_ENTITY_TYPES`:

```rust
if entities_added_per_type[type_name] > 0 {
    let invalidated = tx.execute(
        "DELETE FROM kg_resolutions_log
         WHERE outcome = 'no_match'
           AND subject_entity_id IN (
             SELECT id FROM kg_subject_entities WHERE type = ?
           )",
        [type_name],
    )?;
    if invalidated > 0 {
        invalidated_no_match.insert(type_name.to_string(), invalidated);
    }
}
```

Why DELETE not UPDATE: the resolver's pending-detection query (`entity_resolver.rs::count_pending`, lines 879-918) treats `r.id IS NULL` as pending. DELETE is the simplest signal. Adding a `superseded_at` column or similar marker is out of scope for v1.

The CASCADE on `kg_subject_resolutions` is **not** triggered here because that table FKs to `kg_subject_entities`, not to `kg_resolutions_log`. The subject_resolution rows for these subjects don't exist yet (no_match never wrote them); only log rows exist.

### Step 3 — Emit the structured log event

After commit, emit one row per non-zero entry:

```rust
for (type_name, count) in &stats.invalidated_no_match {
    info!(
        target: "mika::otel",
        trace_id = %self.trace_id,
        added_type = %type_name,
        invalidated_no_match = count,
        event = "domain_rebuild_invalidated_resolutions",
    );
}
```

### Step 4 — Surface `reattempted_no_match` on resolver_tick

The resolver tick already counts outcomes in `ResolutionStats`. Add a new field that increments when a subject the resolver processes was previously `no_match` and was deleted in this run. Implementation note: the resolver doesn't currently know the prior state of each subject — the invalidation already happened at rebuild time. Two options:

**KTD-2a — count via post-tick query.** After the tick runs, query: how many subjects of types T (where T had invalidation this server-boot) were re-resolved this tick? This requires the rebuild-trace to persist somewhere queryable (currently it's only in logs).

**KTD-2b — emit `reattempted_no_match` from the rebuild side, not the tick side.** The rebuild already knows the count (it just deleted them). The `kg_resolver_tick.complete` event would carry the running cumulative count of subjects whose log row is fresh-after-rebuild-invalidation. But this requires a "this subject was invalidated by rebuild" marker that the resolver checks.

**KTD-2c — drop the field from acceptance.** The `domain_rebuild_invalidated_resolutions` event already gives operators the answer. The `reattempted_no_match` on `kg_resolver_tick.complete` is observability-only and partially redundant with `pending_before` jumping after a rebuild.

I propose **KTD-2c** for v1 — meet acceptance A1, A3, A4 and document A2 as a follow-up. Adding the marker column or post-tick query is non-trivial and the operational signal is already present via the rebuild event. Surface this trade-off to the architect in pass 1.

### Step 5 — Unit test

Add `tests` module test in `domain_builder.rs`:

```rust
#[tokio::test]
async fn rebuild_invalidates_no_match_when_type_adds_entities() {
    // 1. Seed: 5 concept-typed kg_subject_entities + 5 kg_resolutions_log rows
    //    with outcome='no_match' for those subject ids. Zero matched_* rows.
    // 2. Run rebuild with at least 1 new concept entity.
    // 3. Assert: kg_resolutions_log has 0 rows for those 5 subjects.
    // 4. Assert: RebuildStats.invalidated_no_match["concept"] == 5.
}

#[tokio::test]
async fn rebuild_does_not_invalidate_when_type_added_zero() {
    // 1. Seed: 3 problem_type subjects with outcome='no_match'.
    // 2. Run rebuild that adds 0 problem_type entities (steady state — all seeds present).
    // 3. Assert: kg_resolutions_log still has 3 rows.
    // 4. Assert: RebuildStats.invalidated_no_match.get("problem_type") is None.
}

#[tokio::test]
async fn rebuild_does_not_invalidate_matched_outcomes() {
    // 1. Seed: 2 concept subjects with outcome='matched_llm', 2 with outcome='no_match'.
    // 2. Run rebuild with 1 new concept entity.
    // 3. Assert: 2 matched_llm rows survive; 2 no_match rows are deleted.
    // 4. Assert: RebuildStats.invalidated_no_match["concept"] == 2.
}
```

The seed helpers in `tests/eval/kg_fixtures/mod.rs` (`seed_subject_entity`, `seed_resolution`) already cover what's needed.

### Step 6 — Update CLAUDE.md

`crates/mika-agent/CLAUDE.md` — append to the `## Knowledge Graph — Domain Graph Builder` section:

> **Resolution invalidation on type expansion (#960):** When `rebuild()` adds N≥1 entities of type T, all `kg_resolutions_log` rows with `outcome='no_match'` for subjects where `kg_subject_entities.type = T` are deleted in the same transaction. The next resolver tick re-attempts those subjects against the expanded domain graph. Per-type invalidation counts surface via the `domain_rebuild_invalidated_resolutions` log event. `matched_*` outcomes are NOT invalidated (re-ranking against a better-matched entity is a separate ranking concern).

## Risks and mitigations

**R1 — first-restart-after-deploy invalidation storm.** When this fix ships, every rebuild that adds entities (which is every rebuild on a freshly-deployed node) will invalidate matching `no_match` rows. The next resolver tick then re-attempts them. For the primary mika corpus this could mean re-attempting thousands of subjects, hitting the LLM budget cap and emitting `kg_budget_exhausted`.

**Mitigation:** The existing `MIKA_KG_BATCH_BUDGET` (default 500) caps the burst. Subsequent ticks drain the rest. Operators can temporarily raise the budget post-deploy if they want faster drain. No code change needed; document in the CLAUDE.md note.

**R2 — well-known-only filter.** The resolver's `count_pending` only considers types in `('skill','tool','agent','problem_type','concept')`. The invalidation query in Step 2 has no such filter — it would invalidate `no_match` rows for any subject type. That's a no-op for "discovered" types (`solution_path`, `failure_mode`, `pattern`) because they're never actually resolved. But it's wasteful work.

**Mitigation:** Add the same well-known-types filter to the DELETE WHERE clause. Single-line change.

**R3 — sole-writer contract.** `domain_builder.rs` is the sole writer of `skill:*`/`tool:*`/`agent:*`/`problem_type:*`/`concept:*` entity_keys. Adding a writer to `kg_resolutions_log` from `domain_builder` extends its responsibility. Per the C1 cross-cutting conventions in `kg-implementation-conventions.md`, this is acceptable because the invalidation is consequent to a domain-graph mutation, not an independent concern.

**Mitigation:** Document the new sole-writer relationship in the module docstring at the top of `domain_builder.rs`.

## Out of scope

- Invalidating non-`no_match` outcomes when domain graph expansion adds a potentially better entity (re-ranking concern; separate ticket).
- Forcing re-extraction of source documents when the extractor's `APPROVED_ENTITY_TYPES` is updated (compile-time const, hot-reload not in scope).
- Fan-out to per-agent corpus partitioning of the invalidation event — the DELETE is global by type because the domain graph is global.
- Migrating mika-arch's primary mika corpus to retroactively fix the 17.6% baseline — the operational wipe of the secondary corpora applied 2026-05-03 is sufficient for milestone#19; primary baseline is its own conversation.

## Sequencing

Single PR, single commit conceptually. No upstream blockers. Can ship behind no flag — the change is observability-positive (more log events) and behavior-positive (more rows resolved).

## Test plan

1. `cargo test -p mika-agent kg::domain_builder` — covers Step 5 unit tests.
2. `cargo test -p mika-agent kg::entity_resolver` — sanity check existing pending-detection still passes.
3. Operational smoke: deploy to local mika-server, observe `domain_rebuild_invalidated_resolutions` log on next restart, verify next resolver tick fires with `pending_before > 0` for concept type.

## Estimated diff size

~80–120 lines in domain_builder.rs (invalidation + RebuildStats field + log emission + 3 tests) + ~10 lines in CLAUDE.md. No migration. No new dependency.
