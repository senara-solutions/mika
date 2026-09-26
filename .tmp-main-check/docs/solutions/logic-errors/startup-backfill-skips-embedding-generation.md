---
title: "startup backfill skips embedding generation for existing facts"
category: logic-errors
date: 2026-04-09
tags: [embeddings, startup, backfill, hybrid-search, vector-search, FTS5]
issue: "#389"
module: mika-agent/server
severity: medium
---

# startup backfill skips embedding generation for existing facts

## Problem

After setting `MIKA_OPENAI_API_KEY`, `search_memory` never uses vector search for existing facts. Only newly stored facts (via `store_fact`/`update_fact`) get embeddings. The `search_content.embedding_json` column is NULL for all pre-existing facts, and the `vec_search` table has no rows, so `hybrid_search()` falls back to FTS5-only.

## Root Cause

`startup_cleanup()` in `crates/mika-agent/src/server/mod.rs` received only `AsyncDatabase` — no `EmbeddingClient`. The FTS5 backfill (triggered when `count_search_content() == 0`) inserted rows into `search_content` but explicitly skipped embedding generation:

```rust
let _ = content_ids; // embeddings require embedding_client; skipped here
```

Two compounding issues:

1. **First-run case:** The FTS5 backfill populated `search_content` rows with `embedding_json = NULL` and moved on.
2. **Subsequent startups:** The `count_search_content() == 0` guard prevented re-running the backfill, so there was no code path to generate embeddings for those rows.

## Solution

1. **Pass `Option<EmbeddingClient>` into `startup_cleanup()`** — changed the function signature and the call site to pass `agent_state.embedding_client.clone()`.

2. **Added `get_unembedded_content(agent_id)` DB method** — queries `search_content` rows where `embedding_json IS NULL`, scoped per agent.

3. **Added embedding backfill as a separate pass** after the FTS5 backfill block. This runs on every startup (idempotent — no-op when all rows have embeddings):

```rust
if let Some(ref client) = embedding_client {
    match db.get_unembedded_content().await {
        Ok(unembedded) if !unembedded.is_empty() => {
            info!(count = unembedded.len(), "backfilling embeddings for search content");
            for chunk in unembedded.chunks(EMBEDDING_BACKFILL_BATCH_SIZE) {
                let texts: Vec<&str> = chunk.iter().map(|(_, c)| c.as_str()).collect();
                match client.embed_batch(&texts).await {
                    Ok(embeddings) => { /* store each via index_embedding() */ }
                    Err(e) => {
                        warn!(error = %e, "embedding batch failed, will retry on next startup");
                        break; // abort run, pick up remaining on next startup
                    }
                }
            }
        }
        // ...graceful error handling
    }
}
```

Key design decisions:
- **Two-phase approach:** FTS5 backfill runs first, embedding backfill queries NULL rows second. Simpler than inline embedding during FTS5 insertion — one code path, no duplication.
- **Batch size 100:** Conservative constant matching `MAX_COMPACTION_BATCH` precedent. OpenAI supports 2048 per request but token-per-minute limits vary by tier.
- **Break on batch failure:** If the OpenAI API is down, abort the run. On next startup, `get_unembedded_content` picks up where it left off — already-embedded rows are skipped.
- **No cross-agent serialization:** Each agent's backfill runs in its own `tokio::spawn`. Retry logic handles transient 429s from shared API key.

## Prevention

- **When adding a "skip for now" comment** (like `let _ = content_ids;`), create a tracking issue immediately. Deferred work without a tracking mechanism becomes permanent gaps.
- **When a function needs a new dependency to do its job fully,** pass it in rather than silently degrading. The original `startup_cleanup` could have accepted `Option<EmbeddingClient>` from the start — the skip was an intentional shortcut that became a persistent bug.
- **Test the full initialization path,** not just individual methods. The backfill logic was never tested end-to-end with an embedding client, so the gap was invisible.

## Related

- [ADR-003: Layer 3 hybrid vector search](../../adr/003-layer3-hybrid-vector-search.md) — design for the FTS5 + vector search system
- [Team engine ignores per-agent LLM config](team-engine-ignores-per-agent-llm-config.md) — documents how `EmbeddingClient` is shared across agents
- GitHub issue: #389
