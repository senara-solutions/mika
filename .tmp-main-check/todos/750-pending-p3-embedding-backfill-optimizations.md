---
status: pending
priority: p3
issue_id: 750
tags: [code-review, performance, embeddings]
dependencies: []
---

# Embedding backfill startup optimizations

## Problem Statement

The embedding backfill in `startup_cleanup()` works correctly but has several optimization opportunities for edge cases (large fact corpus, fleet-wide restarts with shared API key).

## Findings

From performance review of #389 fix:

1. **No inter-batch delay:** Batches fire back-to-back. For typical workloads (2-5 batches) this is fine. For large backfills or concurrent container startups, could trigger OpenAI 429 rate limiting.

2. **Break on batch failure:** If one batch fails (e.g., 429), the entire backfill aborts. A retry-after-delay would be more resilient.

3. **No partial index on `embedding_json IS NULL`:** The idempotent startup query scans all `search_content` rows for the agent. A partial index would make the "nothing to do" fast path free.

4. **Sequential DB writes per embedding:** Each `index_embedding()` is a separate channel round-trip. A batch transaction method would reduce overhead.

## Proposed Solutions

### Option A: Add inter-batch delay only (Minimal)
- Add `tokio::time::sleep(Duration::from_millis(200)).await` between batches
- Pros: Simple, prevents rate limit issues
- Cons: Slows down small backfills slightly
- Effort: Small

### Option B: Full optimization pass
- Inter-batch delay + retry on 429 + partial index + batch DB writes
- Pros: Optimal for all scenarios
- Cons: More code, premature optimization for current scale
- Effort: Medium

## Technical Details

- **Affected files:** `crates/mika-agent/src/server/mod.rs`, `crates/mika-agent/src/db.rs`
- **Components:** startup_cleanup, embedding backfill

## Acceptance Criteria

- [ ] Backfill does not trigger 429 rate limiting under normal load
- [ ] Failed batches are retried after delay rather than aborting
- [ ] Idempotent startup check is optimized for the common case (no unembedded rows)
