---
title: "fix: startup backfill skips embedding generation"
type: fix
status: completed
date: 2026-04-09
issue: 389
---

# fix: startup backfill skips embedding generation

`startup_cleanup()` in `server/mod.rs` receives only `AsyncDatabase` — no `EmbeddingClient`. The FTS5 backfill inserts into `search_content` but explicitly skips embeddings (`let _ = content_ids;`). Once FTS5 entries exist from a previous run, the `count_search_content() == 0` gate prevents re-running, so there's no path to generate embeddings for pre-existing facts. Result: `search_content.embedding_json` is NULL for all pre-existing facts, `vec_search` has no rows, `hybrid_search()` falls back to FTS5-only.

## Acceptance Criteria

- [x] `startup_cleanup()` accepts `Option<EmbeddingClient>` and generates embeddings during FTS5 backfill (first-run case)
- [x] On subsequent startups, rows with `embedding_json IS NULL` are detected and batch-embedded (incremental backfill)
- [x] New `get_unembedded_content(agent_id)` DB method queries rows missing embeddings
- [x] Backfill is idempotent — safe to re-run on every startup, no-op when all rows have embeddings
- [x] Graceful degradation when `embedding_client` is `None` (no API key) — FTS5 backfill still works, embedding step is skipped
- [x] Batch size constant (100 items) prevents hitting OpenAI API limits
- [x] Progress logging at `info!` level: count of rows to embed, completion message
- [x] `cargo test` and `cargo clippy` pass

## Implementation

### 1. Add `get_unembedded_content()` to `db.rs`

```rust
// crates/mika-agent/src/db.rs — near other search methods (~line 6012)
pub fn get_unembedded_content(&self, agent_id: &str) -> Result<Vec<(i64, String)>> {
    let mut stmt = self.conn.prepare(
        "SELECT id, content FROM search_content
         WHERE agent_id = ?1 AND embedding_json IS NULL"
    )?;
    let rows = stmt.query_map(params![agent_id], |row| {
        Ok((row.get(0)?, row.get(1)?))
    })?;
    rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
}
```

### 2. Add async wrapper in `async_db.rs`

```rust
// crates/mika-agent/src/async_db.rs — in "Layer 3: Search Indexing" section
pub async fn get_unembedded_content(&self) -> Result<Vec<(i64, String)>> {
    let a = self.agent_id.clone();
    self.with_db(move |db| db.get_unembedded_content(&a)).await
}
```

### 3. Modify `startup_cleanup()` in `server/mod.rs`

Change signature from `async fn startup_cleanup(db: AsyncDatabase)` to:
```rust
async fn startup_cleanup(db: AsyncDatabase, embedding_client: Option<EmbeddingClient>)
```

After FTS5 backfill, replace `let _ = content_ids;` with inline embedding generation using the collected `content_ids` when `embedding_client` is `Some`.

Add a second pass after the FTS5 block for incremental embedding backfill:
```rust
// Backfill embeddings for rows missing them (idempotent, runs every startup)
if let Some(ref client) = embedding_client {
    if let Ok(unembedded) = db.get_unembedded_content().await {
        if !unembedded.is_empty() {
            info!(count = unembedded.len(), "backfilling embeddings for search content");
            // Batch in chunks of EMBEDDING_BACKFILL_BATCH_SIZE
            for chunk in unembedded.chunks(EMBEDDING_BACKFILL_BATCH_SIZE) {
                let texts: Vec<&str> = chunk.iter().map(|(_, c)| c.as_str()).collect();
                match client.embed_batch(&texts).await {
                    Ok(embeddings) => {
                        for ((id, _), emb) in chunk.iter().zip(embeddings) {
                            if let Err(e) = db.index_embedding(*id, emb).await {
                                warn!(content_id = id, error = %e, "failed to store embedding");
                            }
                        }
                    }
                    Err(e) => {
                        warn!(error = %e, "embedding batch failed, will retry on next startup");
                        break;
                    }
                }
            }
            info!("embedding backfill complete");
        }
    }
}
```

### 4. Update call site

```rust
// Line ~706 in server/mod.rs
tokio::spawn(startup_cleanup(db, agent_state.embedding_client.clone()));
```

### Files to modify

| File | Change |
|------|--------|
| `crates/mika-agent/src/db.rs` | Add `get_unembedded_content()` |
| `crates/mika-agent/src/async_db.rs` | Add async wrapper |
| `crates/mika-agent/src/server/mod.rs` | Update `startup_cleanup()` signature + implementation + call site |

## Design Decisions

- **Batch size 100:** Conservative default matching `MAX_COMPACTION_BATCH` precedent. OpenAI supports 2048 per request but token-per-minute limits vary by tier.
- **Break on batch failure:** If one `embed_batch()` call fails, abort the run. On next startup, `get_unembedded_content` picks up where it left off — already-embedded rows are skipped.
- **Single code path for embeddings:** Rather than embedding inline during FTS5 backfill AND as a second pass, the FTS5 backfill runs first, then the embedding pass queries all NULL rows. Simpler, no duplication, same result.
- **No cross-agent serialization:** Each agent's backfill runs independently in its own `tokio::spawn`. Shared API key may hit rate limits, but retry logic + next-startup catch-up handles it.
- **Server-only scope:** CLI startup path is out of scope for this fix (separate concern, separate entry point). Can be addressed in a follow-up.
- **No config flag:** Backfill is gated on `embedding_client.is_some()` which is gated on `MIKA_OPENAI_API_KEY`. Users who don't want embeddings don't set the key.
