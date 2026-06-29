# PR Review: wip(#1234): partial implementation of webhook retry

## PR Metadata
- **Title:** wip(#1234): partial implementation of webhook retry
- **State:** DRAFT
- **Draft:** true
- **Base:** main
- **Head:** feat/1234/webhook-retry
- **Files changed:** 4
- **Additions:** 89
- **Deletions:** 3
- **Latest commit:** `wip(#1234): partial implementation`

## Auto-rescued PR (dispatch-lib recovery, class:dirty-worktree)

This PR was auto-rescued by dispatch-lib's post-flight dirty-worktree recovery.
The pilot session exited with uncommitted changes and zero commits.
dispatch-lib auto-committed with `wip()` prefix and opened this draft PR.

<!-- rescue-pipeline-verified: no -->

## Plan Path
`docs/plans/1234-webhook-retry-plan.md`

## Acceptance Criteria
- AC1: Failed webhook deliveries are retried up to 3 times with exponential backoff
- AC2: Retry state persisted in `webhook_delivery_attempts` table
- AC3: Dead-letter queue populated after max retries exhausted
- AC4: Integration test covers the full retry→DLQ flow

## Diff Summary

### `crates/mika-gateway/src/webhook_queue.rs`
- Added `RetryPolicy` struct with `max_attempts: u32` and `base_delay_ms: u64`
- Added `schedule_retry()` function — incomplete, contains `todo!()` on line 45

### `crates/mika-gateway/src/db.rs`
- Added `webhook_delivery_attempts` table creation in migration
- Added `record_delivery_attempt()` — saves attempt with timestamp and error

### `crates/mika-gateway/src/webhook_queue.rs` (partial)
- `process_webhook()` has retry loop skeleton but the DLQ branch is commented out
- `// TODO: implement DLQ population` on line 78

### `crates/mika-gateway/tests/webhook_retry_test.rs`
- File does not exist in the diff — no test added
