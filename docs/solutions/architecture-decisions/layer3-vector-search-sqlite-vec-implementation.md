---
title: Implement Layer 3 Hybrid Vector Search with FTS5 and sqlite-vec
date: 2026-02-25
type: feature_implementation
severity: medium
component: mika-agent/search
tags: [sqlite-vec, fts5, vector-search, embeddings, hybrid-search, semantic-search, reciprocal-rank-fusion]
status: resolved
---

# Layer 3 Hybrid Vector Search Implementation

## Problem Statement

Mika's `search_memory` tool relied exclusively on SQL LIKE-based substring matching to find stored facts. This meant:

- **No semantic understanding**: "meetings with alice" wouldn't find "Alice is my co-founder" (no keyword overlap)
- **Exact phrase dependence**: Users had to remember the exact wording used when facts were stored
- **Missed contextual relationships**: "schedule" and "reminder" are semantically related but produce no LIKE matches
- **Degraded UX at scale**: As memory grew, substring search became increasingly unhelpful

The assistant's core value is remembering context intelligently. Without semantic search, it was a keyword index.

## Solution

### Architecture: 3-Tier Hybrid Search with Graceful Degradation

1. **Tier 1 (Hybrid)**: FTS5 BM25 keyword ranking + sqlite-vec cosine distance, merged via Reciprocal Rank Fusion (RRF, k=60). Requires OpenAI API key.
2. **Tier 2 (FTS5-only)**: Full-text keyword search when embedding client unavailable or query embedding fails.
3. **Tier 3 (LIKE fallback)**: SQL substring matching when search index not yet populated. Also used for non-indexed categories (core_memory, reminders).

### Schema (v8)

Three new tables:
- `search_content` — canonical index: `(id, source_type, source_id, content)`
- `fts_search` — FTS5 virtual table for BM25 ranking: `(content_id, content, source_type)`
- `vec_search` — sqlite-vec vec0 table: `(content_id, embedding float[512])`

### Key Components

**EmbeddingClient** (`crates/mika-common/src/embedding.rs`):
- OpenAI `text-embedding-3-small` at 512 dimensions (Matryoshka truncation)
- Retry with exponential backoff on 429/500/503
- Typed `EmbeddingApiError` with status code downcast for retry decisions

**RRF Merge** (`crates/mika-agent/src/db.rs:hybrid_search`):
```rust
const RRF_K: f64 = 60.0;
// Score = sum of 1/(k + rank + 1) from each ranking source
for (rank, result) in fts_results.iter().enumerate() {
    *scores.entry(result.id).or_default() += 1.0 / (RRF_K + rank as f64 + 1.0);
}
for (rank, (content_id, _)) in vec_results.iter().enumerate() {
    *scores.entry(*content_id).or_default() += 1.0 / (RRF_K + rank as f64 + 1.0);
}
```

**Best-effort indexing** (`crates/mika-agent/src/tools/mod.rs:index_fact`):
- Called by `store_fact` and `update_fact` after successful DB writes
- Logs warnings but never propagates errors to tool responses
- Deletes old entries before re-indexing (handles upserts)

**Startup backfill** (`crates/mika-agent/src/scheduler.rs:recover`):
- Detects empty search index via `count_search_content() == 0`
- Indexes all existing facts into FTS5
- Batch-generates embeddings in chunks of 100 (prevents API timeouts)
- Tracks content_ids from `index_content()` return values (not FTS re-query)

**FTS5 sanitization** (`crates/mika-agent/src/db.rs:sanitize_fts5_query`):
```rust
fn sanitize_fts5_query(query: &str) -> String {
    let escaped = query.replace('"', "\"\"");
    format!("\"{escaped}\"")
}
```
Wraps user input in double quotes to prevent FTS5 operator parse errors (AND, OR, NOT, etc.).

## Code Review Fixes Applied

Eight findings from architecture, security, and simplicity reviews:

1. **Once guard** in `init_sqlite_vec()` — callers no longer manage their own guards
2. **Debug impl** on `EmbeddingClient` redacts `api_key` as `[REDACTED]`
3. **Typed error downcast** for `is_retryable_status` instead of string matching
4. **Backfill ID tracking** from `index_content()` return values, not FTS re-query
5. **Batch size limit** of 100 for embedding API calls during backfill
6. **FTS5 query sanitization** via double-quote wrapping
7. **Subquery-based deletion** instead of loop for `delete_search_content`
8. **Error body truncation** to 500 chars in embedding API error logging

## Prevention & Best Practices

### Patterns to Replicate

- **Idempotent initialization**: Wrap global resource registration in `std::sync::Once` internally, not caller-managed
- **Manual Debug for credential types**: Any struct holding API keys must implement `Debug` with redaction
- **Typed HTTP errors**: Parse status codes to typed enums, use `downcast_ref` for retry logic
- **Input sanitization for search engines**: Quote user input per the engine's escaping rules (FTS5: double quotes)
- **Tracked IDs during bulk operations**: Collect return values from insert operations for later use
- **Subquery bulk deletion**: `DELETE WHERE id IN (SELECT ...)` instead of per-row loops

### Anti-patterns Avoided

- String matching on error messages for retry logic
- Caller-managed once guards for shared initialization
- Full error response bodies in logs (truncate to 500 chars)
- Unbounded batch API calls (chunk to 100)
- FTS re-query to find IDs that were just inserted

### Testing Considerations

- Test FTS5 with adversarial queries: `AND OR NOT`, quotes, parentheses, null bytes
- Test graceful degradation: hybrid without embedding client, FTS5 without index, LIKE fallback
- Test backfill idempotence: call twice, verify no duplicates
- Test bulk deletion scales: verify subquery cleans up all related tables

## Operational Notes

- **Graceful degradation**: No OpenAI key = FTS5-only. Empty index = LIKE fallback. Both logged at WARN.
- **Storage**: ~2.5MB per 1000 facts (2KB per 512-dim embedding + FTS5 overhead)
- **Backfill timing**: Runs at startup before accepting requests. 100 facts = ~2 seconds.
- **Monitoring**: Track `search_content` count vs total facts. Alert if hybrid search latency exceeds 1s.

## Related Documentation

- **Plan**: `docs/plans/2026-02-25-feat-layer3-vector-search-memory-plan.md`
- **PR**: [#11 — feat: add Layer 3 vector search memory](https://github.com/senara-solutions/mika/pull/11)
- **Related**: `docs/solutions/logic-errors/broken-preference-substring-search.md` (motivating limitation)
- **External**: [sqlite-vec docs](https://alexgarcia.xyz/sqlite-vec/), [SQLite FTS5 docs](https://www.sqlite.org/fts5.html)

## Files Changed

| File | Changes |
|------|---------|
| `crates/mika-common/src/embedding.rs` | New: EmbeddingClient with retry, Debug redaction, typed errors |
| `crates/mika-common/src/config.rs` | Added: openai_api_key, embedding_model, embedding_dimensions |
| `crates/mika-agent/src/db.rs` | Schema v8, search methods (index, FTS5, vec, hybrid, RRF) |
| `crates/mika-agent/src/async_db.rs` | Async wrappers for all search methods |
| `crates/mika-agent/src/tools/mod.rs` | index_fact helper, embedding_client in ToolContext |
| `crates/mika-agent/src/tools/search_memory.rs` | Hybrid search integration with fallback chain |
| `crates/mika-agent/src/tools/store_fact.rs` | Hook indexing into all 4 store functions |
| `crates/mika-agent/src/tools/update_fact.rs` | Hook re-indexing on commitment status change |
| `crates/mika-agent/src/scheduler.rs` | Backfill logic with batched embeddings |
| `crates/mika-agent/src/agent.rs` | Thread embedding_client through AgentParams |
