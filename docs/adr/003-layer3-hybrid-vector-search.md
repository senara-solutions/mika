---
title: "ADR-003: Hybrid Vector Search"
---

# ADR-003: Layer 3 Hybrid Vector Search with FTS5 and sqlite-vec

**Date:** 2026-02-25
**Status:** Accepted
**Component:** mika-agent/search

## Context

Mika's `search_memory` tool relied on SQL LIKE substring matching. This meant
no semantic understanding ("meetings with alice" wouldn't find "Alice is my
co-founder"), exact phrase dependence, and degraded UX at scale.

## Decision

Implement a 3-tier hybrid search with graceful degradation:

1. **Tier 1 (Hybrid)**: FTS5 BM25 keyword ranking + sqlite-vec cosine similarity,
   merged via Reciprocal Rank Fusion (RRF, k=60). Requires OpenAI API key.
2. **Tier 2 (FTS5-only)**: Full-text keyword search when embedding client is
   unavailable or query embedding fails.
3. **Tier 3 (LIKE fallback)**: SQL substring matching when the search index is not
   yet populated.

### Schema (v8)

Three new tables:
- `search_content` — canonical index: `(id, source_type, source_id, content)`
- `fts_search` — FTS5 virtual table for BM25 ranking
- `vec_search` — sqlite-vec vec0 table: `(content_id, embedding float[512])`

### Key Components

- **EmbeddingClient**: OpenAI `text-embedding-3-small` at 512 dimensions with
  exponential backoff retry on 429/500/503
- **RRF Merge**: `score = sum(1/(k + rank + 1))` from each ranking source
- **Best-effort indexing**: Called by `store_fact` and `update_fact` after DB writes;
  logs warnings but never propagates errors to tool responses
- **Startup backfill**: Detects empty index, indexes all existing facts, batches
  embedding generation in chunks of 100
- **FTS5 sanitization**: User input wrapped in double quotes to prevent FTS5 operator
  parse errors

## Consequences

- No OpenAI key = FTS5-only search (still much better than LIKE)
- Empty index = LIKE fallback (first-run experience unchanged)
- ~2.5MB per 1000 facts (2KB per 512-dim embedding + FTS5 overhead)
- Backfill runs at startup before accepting requests (~2 seconds per 100 facts)
- Indexing errors are silent — search degrades gracefully but never fails

### Patterns Established

- Idempotent initialization with `std::sync::Once` internally, not caller-managed
- Manual `Debug` impl with redaction for credential-holding types
- Typed HTTP errors with status code downcast for retry logic
- Input sanitization per engine's escaping rules (FTS5: double quotes)
- Subquery bulk deletion (`DELETE WHERE id IN (SELECT ...)`) instead of per-row loops
