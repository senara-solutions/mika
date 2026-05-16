---
module: kg/lexical_ingestor
tags: [kg, search_content, multi-corpus, backfill, mika-arch]
problem_type: data_gap
category: bug-fix
---

# KG search_content index gap on multi-corpus agents (#1155)

## Problem

After the v27 shared-corpus migration, `kg_chunks` became keyed by
`docs_root_hash` and the lexical ingestor's content-hash idempotency
check skips re-chunking when another agent has already ingested the same
doc. The skip path returned early **without writing per-agent
`search_content` rows** — but `search_content` is per-agent (keyed on
`agent_id`). Multi-corpus agents arriving late to the race (mika-arch
with 6 `docs_roots`) ended up with shared `kg_chunks` but no
corresponding `search_content`, causing the resolver's
`get_chunk_context()` join to miss on ~76% of subjects.

## Root Cause

`ingest_single_doc_inner` line 284: `return Ok((0, 0, true))` when hash
matches — exits before `db.index_content(agent_id, ...)` is called.

The skip optimization correctly avoids re-chunking shared content, but
collapsed two orthogonal scopes:
- **Shared chunk content** — `(docs_root_hash, source_doc_path)` in `kg_chunks`
- **Per-agent search index** — `(agent_id, source_type, source_id)` in `search_content`

## Solution

Split the skip path into a two-axis check:

1. Hash matches → chunks unchanged (correct, keep optimization).
2. Check if **this agent** has `search_content` rows for all existing
   chunk IDs via a membership query.
3. If any are missing → backfill via `index_content()` in a transaction.
4. If all present → true no-op (original optimization fires).

New `DocOutcome::IndexBackfilled` variant and `IngestStats` fields
(`docs_index_backfilled`, `chunks_indexed_backfill`) surface the work.

Self-healing: each startup `ingest_all` discovers and fills the gap.
No schema change, no explicit migration.

## Key Design Decisions

- **D3a mismatch guard:** If DB chunk count ≠ in-memory chunker count
  (chunker version drift), logs a WARN and falls through to the
  delete+insert path instead of backfilling with potentially wrong text.
- **Text source:** In-memory chunker output (deterministic for same
  content hash) paired with DB chunk IDs ordered by `seq_id`. The
  `kg_chunks` table has no `text` column.
- **Transaction scope:** Backfill wrapped in `unchecked_transaction` for
  per-doc atomicity of `search_content` + `fts_search` writes.
- **No resolution invalidation:** Prior `no_match` outcomes are NOT
  invalidated. Filed as separate follow-up scope.

## Signals

- **Signal I** in root `CLAUDE.md`: `grep lexical_ingest_complete
  server.log | jq 'select(.docs_index_backfilled > 0)'`
- First restart: `docs_index_backfilled` ≈ shared-corpus doc count.
- Subsequent restarts: `docs_index_backfilled = 0` (self-healing complete).

## Affected Files

- `crates/mika-agent/src/kg/lexical_ingestor.rs` — core fix + 7 tests
- `CLAUDE.md` — Signal I documentation
