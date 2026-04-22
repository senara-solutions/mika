---
title: "KG lexical ingestion: composed write contract and transactional atomicity"
module: kg
date: 2026-04-22
problem_type: best_practice
component: database
severity: medium
applies_when:
  - "Adding a new KG layer that writes to kg_chunks + search_content"
  - "Implementing content-hash idempotency with delete-before-reinsert"
  - "Writing startup ingestion hooks that run per-agent"
tags:
  - knowledge-graph
  - lexical-ingestion
  - composed-write
  - transactional-atomicity
  - search-content
  - idempotency
---

# KG lexical ingestion: composed write contract and transactional atomicity

## Context

The KG lexical layer (#689) ingests `docs/solutions/**/*.md` per agent into `kg_chunks` + `search_content` with content-hash idempotency. The implementation surfaced several architectural patterns that apply to any future KG layer writer (subject extraction #690, entity resolution #691).

## Guidance

### Single-transaction composed writes

The hash check, delete of old chunks, and insert of new chunks **must happen in a single `with_db` closure** to prevent a flash-of-empty window where concurrent readers see zero chunks for a doc that's being re-ingested.

Wrong (two separate transactions):
```
let deleted = self.delete_doc_chunks(&path).await?;  // Transaction 1
self.db.with_db(move |db| { /* insert */ }).await?;   // Transaction 2
// Reader between T1 and T2 sees zero chunks!
```

Right (single transaction):
```
self.db.with_db(move |db| {
    let tx = db.conn.unchecked_transaction()?;
    // 1. Hash check
    // 2. Delete old chunks + search_content cleanup
    // 3. Insert new chunks + index_content
    tx.commit()?;
    Ok(())
}).await?;
```

### Use `db.delete_search_content()` for symmetric cleanup

When deleting chunks, use the canonical `Database::delete_search_content()` method instead of manually replicating the SQL for `search_content` + `fts_search` + `vec_search` cleanup. This ensures future search table additions are automatically covered.

### Always delete when chunks exist (not just when hash matches)

When the DB has chunks for a doc but the hash check returns multiple distinct hashes (corruption/invariant violation), always delete before re-inserting. Skipping the delete causes UNIQUE constraint failures and chunk accumulation.

### `unchecked_transaction` + `db.conn` writes

`db.conn.unchecked_transaction()` returns a `Transaction` that wraps the same connection. Writes through `db.conn` (e.g., `db.index_content()`) are inside the transaction because SQLite uses a single-connection model. Document this invariant with a `// SAFETY:` comment when mixing `tx.execute()` and `db.method()` calls.

### Empty-doc cleanup

When a doc normalizes to empty content (whitespace-only file), still check for and delete any stale chunks from a previous run where the doc had content. Don't just return "skipped" without cleanup.

## Why This Matters

The lexical layer is the first per-agent KG writer. Subject extraction (#690) and entity resolution (#691) will follow the same patterns. Getting the transactional contract right here prevents each downstream ticket from re-discovering the same pitfalls.

The flash-of-empty bug is particularly subtle because it only manifests under concurrent access — startup ingestion of one agent while another agent's search query is in flight, or compound hook ingestion racing with a user query. It won't show up in serial unit tests.

## When to Apply

- Implementing any new KG layer writer (kg_chunks, kg_subject_entities, etc.)
- Adding composed writes that span `kg_*` tables and `search_content`
- Implementing content-hash idempotency with delete-before-reinsert patterns

## Examples

See `crates/mika-agent/src/kg/lexical_ingestor.rs` for the canonical implementation. Key methods:
- `ingest_single_doc_inner()` — single-transaction hash check + delete + insert
- `delete_doc_chunks()` — symmetric cleanup using `db.delete_search_content()`
- `normalize_content()` + `compute_hash()` — content normalization for idempotent hashing

## Related

- [`../workflow-issues/kg-milestone-14-autonomous-execution-retrospective-2026-04-22.md`](../workflow-issues/kg-milestone-14-autonomous-execution-retrospective-2026-04-22.md) — KG milestone retrospective.
- [`kg-subject-extraction-constrained-ner-2026-04-22.md`](kg-subject-extraction-constrained-ner-2026-04-22.md) — next KG layer, follows the same composed-write pattern.
- [`kg-entity-resolution-two-stage-pipeline.md`](kg-entity-resolution-two-stage-pipeline.md) — bridges subject graph to domain graph.
- `docs/architecture/kg-implementation-conventions.md` — cross-cutting KG conventions.
