---
title: "Integration-test fixtures must use the canonical write path to preserve dual-write invariants"
date: 2026-04-23
category: best-practices
module: eval-harness
problem_type: best_practice
component: testing_framework
severity: medium
applies_when:
  - Seeding state for integration tests that exercise search or retrieval code
  - Writing fixture helpers for tables that participate in dual-write or transactional double-write invariants (FTS5, vec0, JOIN-linked sidecar tables)
  - Authoring eval scenarios where a broad assertion could be satisfied by multiple entry paths
  - Adding helpers to `tests/eval/kg_fixtures/` or similar crate-shared seeding modules
tags:
  - eval-harness
  - fixture-seeding
  - fts5
  - knowledge-graph
  - kg-chunks
  - hybrid-search
  - integration-test
  - invariant-preservation
---

# Integration-test fixtures must use the canonical write path to preserve dual-write invariants

## Context

The KG self-knowledge eval scenarios (#740) seed a known KG state — domain entities, subject entities, chunks, and resolutions — so Path A / Path B / Path C retrieval can be exercised against controlled input. The first version of `seed_chunk` in `tests/eval/kg_fixtures/mod.rs` inserted directly into `kg_chunks` and `search_content`:

```rust
db.execute_sql(
    "INSERT INTO kg_chunks (agent_id, seq_id, source_doc_path, source_doc_hash) \
     VALUES (?1, ?2, ?3, ?4)",
    /* ... */
)?;
let chunk_id = db.last_insert_rowid();

db.execute_sql(
    "INSERT INTO search_content (agent_id, content, source_type, source_id) \
     VALUES (?1, ?2, 'kg_chunk', ?3)",
    /* ... */
)?;
```

This looked right — the two tables touched by a `kg_chunk` ingest were both populated. But Path C (semantic via chunks) calls `hybrid_search(source_type="kg_chunk")`, which joins `fts_search` (FTS5 virtual table) back to `search_content`. The raw insert skipped `fts_search`, so no chunk was retrievable via Path C.

The scenarios still passed because the assertions were lenient enough that Path A (direct domain entity LIKE match) or Path B (subject entity LIKE match) could satisfy them. Path C was dead on the seed data — not in the production code. A prompt or ranking change that actually broke Path C would not have been caught.

## Guidance

**1. Route fixture seeds through the same public API the production code uses.** The `kg_chunks` write path in `lexical_ingestor.rs` calls `Database::index_content(agent_id, KG_CHUNK_SOURCE_TYPE, Some(chunk_id), text)` inside a transaction. That one call updates `search_content`, `fts_search`, and — when embeddings arrive — `vec_search`. A fixture that bypasses it silently drops whichever indexes it doesn't replicate by hand. The fix is one line:

```rust
db.index_content(
    &agent_id,
    mika_agent::db::kg_schema::KG_CHUNK_SOURCE_TYPE,
    Some(chunk_id),
    &text,
)?;
```

Now `search_content` and `fts_search` stay in parity on seed, matching the transactional double-write invariant that `kg_schema.rs` declares as hard.

**2. Assume every table with a non-obvious sidecar has a dual-write invariant.** FTS5 virtual tables, `vec0` virtual tables, materialized views, trigger-maintained summary rows — all of these have a second write that a raw `INSERT` will skip. Grep for `fts_search`, `vec_search`, or `CREATE TRIGGER` near the target table before writing a fixture. If a single write method (`Database::index_content`, `Database::delete_search_content`) encapsulates the invariant, use it from the fixture too.

**3. Tighten scenario assertions so only the target path can satisfy them.** If a Path C scenario asserts "entity `skill:self-dev` appears in results", Path A will satisfy it whenever a domain entity with that name exists. To actually test Path C, either (a) omit the matching-name domain/subject entity so Path A and B yield nothing, or (b) discriminate on `hop`, `layer`, `entry_method`, or a confidence signature that only Path C produces. The rule: **when a test has multiple valid code paths that could satisfy its assertions, the test doesn't discriminate between them.**

**4. Pin fixture modules to a schema version with an actionable failure message.** `kg_fixtures/mod.rs` asserts `PINNED_SCHEMA_VERSION == 25` at scenario setup and fails with a message naming the files to update and the plan's D5 section. On schema advance, the fixture breaks loudly, not silently — the next author has a checklist, not a mystery.

## Why This Matters

Silent fixture bugs in integration tests are worse than red tests. A red test gets fixed; a green-but-vacuous test rots until someone notices production is wrong. The CI signal is green, the regression gate thinks the path is covered, and the code you believed you were exercising isn't running.

Structural lesson: **the invariants a code path depends on are part of the code path.** A fixture that bypasses the invariants doesn't reproduce the path — it reproduces a different path that happens to share a name. The only way to be sure a test exercises the real path is to write the fixture through the same API production writes through.

Adjacent failure mode: hybrid / multi-path retrieval systems (Path A ∪ Path B ∪ Path C merged by union, FTS ∪ vector merged by RRF, etc.) paper over a broken individual path whenever another path also satisfies the assertion. Tighten the assertions, or shape the fixture so only one path can produce the asserted result.

## When to Apply

- Adding new helpers to `crates/mika-agent/tests/eval/kg_fixtures/` or any crate-shared fixture module
- Authoring scenarios under `tests/eval/kg_self_knowledge/` or `tests/eval/grounding/` (future #741)
- Writing integration tests that exercise Layer 3 hybrid search, RRF ranking, or any code that joins an FTS5/vec0 sidecar back to `search_content`
- Reviewing PRs that add raw `INSERT INTO kg_*`, `INSERT INTO search_content`, or `INSERT INTO fts_search` outside of `Database` methods
- Writing fixtures for dual-written tables in new subsystems (if the write path's doc comment says "dual" or "index", route the fixture through it)

## Examples

**Wrong — raw insert skips `fts_search`, masks Path C failures:**

```rust
pub async fn seed_chunk(db: &AsyncDatabase, spec: &ChunkSpec) -> i64 {
    db.with_db(move |db| {
        db.execute_sql("INSERT INTO kg_chunks ...", /* ... */)?;
        let chunk_id = db.last_insert_rowid();
        // BUG: writes search_content but bypasses fts_search.
        // hybrid_search(source_type="kg_chunk") never finds this chunk.
        db.execute_sql(
            "INSERT INTO search_content (agent_id, content, source_type, source_id) \
             VALUES (?1, ?2, 'kg_chunk', ?3)",
            /* ... */,
        )?;
        Ok(chunk_id)
    }).await.unwrap()
}
```

**Right — route through `Database::index_content`, the canonical write path:**

```rust
pub async fn seed_chunk(db: &AsyncDatabase, spec: &ChunkSpec) -> i64 {
    db.with_db(move |db| {
        db.execute_sql("INSERT INTO kg_chunks ...", /* ... */)?;
        let chunk_id = db.last_insert_rowid();
        // Canonical path: updates search_content + fts_search in one call,
        // mirrors lexical_ingestor.rs:309-335.
        db.index_content(
            &agent_id,
            mika_agent::db::kg_schema::KG_CHUNK_SOURCE_TYPE,
            Some(chunk_id),
            &text,
        )?;
        Ok(chunk_id)
    }).await.unwrap()
}
```

**Right — scenario that discriminates Path C from A/B by construction:**

```rust
// Seed ONLY a chunk + subject with a discovered type (no domain counterpart).
// Path A is dead (no matching domain entity), Path B yields only subject layer.
// A Path C hit via FTS5 on the chunk text is the only way this assertion passes.
seed_subject_entity(&db, &SubjectEntitySpec {
    entity_type: "solution_path",
    name: "webhook-ci-handler",
    confidence: 0.88,
    properties_json: None,
}).await;
let chunk_id = seed_chunk(&db, &ChunkSpec { /* text mentioning webhook-ci-handler */ }).await;
seed_chunk_subject(&db, chunk_id, subject_id).await;

let entry = result.entries.iter()
    .find(|e| e.entity_key == "solution_path:webhook-ci-handler")
    .expect("Path C must find the subject via chunk");
assert_eq!(entry.layer, "subject");
```

## References

- `crates/mika-agent/tests/eval/kg_fixtures/mod.rs` — fixture module
- `crates/mika-agent/src/kg/lexical_ingestor.rs:309-335` — production write path
- `crates/mika-agent/src/db.rs` — `index_content`, `hybrid_search`, `fts_search`, `vec_search_internal`
- `crates/mika-agent/src/db/kg_schema.rs` — dual-write invariant
- PR #758 / commit `bab112a7` — the fix
- mika#740 — KG-backed self-knowledge eval scenarios
