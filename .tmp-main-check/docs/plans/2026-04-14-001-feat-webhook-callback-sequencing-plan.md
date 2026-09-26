---
title: "feat: Queue PR webhooks until in-flight callback completes"
type: feat
status: active
date: 2026-04-14
issue: 528
---

# feat: Queue PR webhooks until in-flight callback completes

## Overview

Add a webhook deferral layer in the agent HTTP server that holds inbound GitHub webhooks targeting a work item with an in-flight `run_claude_pilot` callback. Queued webhooks are replayed in arrival order after the callback's `dispatch_resume_agent` completes (lock released), ensuring the verdict handler and LLM see consistent work item metadata (especially `pr_url`).

## Problem Frame

On 2026-04-11, mika-dev experienced a webhook/callback ordering race on PR #522. A `pull_request_review.submitted` webhook arrived 13 seconds before the `claude-pilot completed` callback for the same work item. At webhook arrival time, the work item had no `pr_url` in metadata (written by the callback path), so the structural verdict handler (#524) could not correlate. The LLM misclassified and re-dispatched instead of merging. Companion fixes #524 (verdict handler) and #525 (dispatch-readiness guard) reduce impact but don't close the underlying race window.

## Requirements Trace

- R1. Track in-flight `run_claude_pilot` callbacks per work item
- R2. When a webhook arrives that correlates to a work item with an in-flight callback, defer it instead of processing immediately
- R3. On callback completion (after `dispatch_resume_agent` releases the agent lock), replay deferred webhooks in arrival order
- R4. Maximum deferral time: 60 seconds from webhook arrival. After timeout, replay anyway
- R5. Emit `webhook_deferred` audit event with `(webhook_event, work_item_id, deferral_ms)` when deferral fires
- R6. Integration test: webhook delivered before callback completion -> processed after callback metadata visible
- R7. Integration test: webhook for unrelated work item -> NOT deferred
- R8. Integration test: hung callback (no completion within 60s) -> webhook replayed after timeout

## Scope Boundaries

- Only GitHub channel webhooks are eligible for deferral (not Telegram, not A2A)
- Only work items with active callback child tasks (trigger_type="callback", status IN pending/in_progress) trigger deferral
- The queue is in-memory only -- acceptable given the short 60s window and GitHub's webhook redelivery capability
- Does not change the gateway's webhook delivery behavior -- the gateway still gets a 202 response
- Does not modify the verdict handler or dispatch-readiness guard -- those are companion fixes already landed

### Deferred to Separate Tasks

- Persistent (SQLite-backed) webhook queue: only if production observation shows the in-memory queue is insufficient
- Webhook deduplication (matching gateway's LRU cache): not needed at the agent level since the gateway already deduplicates

## Context & Research

### Relevant Code and Patterns

- `crates/mika-agent/src/server/handlers.rs` — `handle_message()` is the interception point; `handle_task_complete()` is where callbacks arrive
- `crates/mika-agent/src/server/state.rs` — `AgentState` holds per-agent lock, DB, dispatcher; `AppState` is the Axum shared state
- `crates/mika-agent/src/server/verdict_handler.rs` — `try_handle_pr_review_verdict()` uses `find_active_work_item_by_pr_url()` for correlation
- `crates/mika-agent/src/server/verdict.rs` — `parse_pr_review_event()` extracts `PrReviewEvent` with repo + pr_number from gateway text
- `crates/mika-agent/src/task_engine/dispatcher.rs` — `dispatch_resume_agent()` acquires agent lock, calls `try_extract_callback_metadata()` (writes cost/session/duration to work item), then `run_silent_agent()`, then `mark_task_delivered()`
- `crates/mika-agent/src/skills/executor.rs` — `validate_dispatch_readiness()` checks for active callback children via `get_child_tasks()`
- `crates/mika-agent/src/db.rs` — `find_active_work_item_by_pr_url()`, `get_child_tasks()`, `log_audit_event()`

### Institutional Learnings

- **Verdict misclassification incident** (`docs/solutions/agent-quality/2026-04-11-mika-dev-verdict-misclassification-pr-522.md`): documents the exact race and recommends this sequencing guard
- **Callback metadata extraction** (`docs/solutions/architecture-patterns/engine-level-callback-metadata-extraction.md`): `try_extract_callback_metadata()` writes `session_id`, `cost_usd`, `duration_ms`, `turns` -- but NOT `pr_url`. The `pr_url` is written by the silent agent during the callback turn via `update_work_item_status`. This means the replay point must be **after the entire `dispatch_resume_agent()` completes**, not just after `try_extract_callback_metadata()`
- **Failed callback tasks** (`docs/solutions/logic-errors/failed-callback-tasks-silently-dropped.md`): both `completed` and `failed` are deliverable terminal states. Queue release must fire on both outcomes
- **Callback loop prevention** (`docs/solutions/architecture-patterns/callback-task-loop-prevention.md`): queued webhook replay must go through the same handler chain as direct delivery
- **Out-of-scope PR verdicts** (`docs/solutions/workflow-patterns/2026-04-13-mika-dev-ignores-out-of-scope-pr-verdicts.md`): only queue webhooks that correlate to a work item -- uncorrelated webhooks pass through immediately

## Key Technical Decisions

- **In-memory queue on `AgentState`:** A `tokio::sync::Mutex<Vec<DeferredWebhook>>` on `AgentState`. Simple, per-agent scoped, no schema migration. Lost on restart, but 60s window + GitHub redelivery makes this acceptable. The `dashmap` crate is already a dependency (used in `AppState::a2a_broadcasters`)
- **Correlation via PR URL parsed from webhook text:** Reuse `parse_pr_review_event()` to extract PR URL from `pull_request_review` events. For `check_suite` events, parse branch from the formatted text and match against `metadata.claude_pilot.branch`. For events without PR/branch context (issues, issue_comment), skip deferral entirely
- **Active callback detection via DB query:** Query `get_child_tasks(work_item_id)` and filter for `trigger_type="callback" && status IN (pending, in_progress)`. This reuses the exact pattern from `validate_dispatch_readiness()` in #525
- **Replay after agent lock release:** The `dispatch_resume_agent()` holds the agent lock for the entire callback turn (metadata extraction + silent agent run). The replay fires AFTER the lock is released via a `tokio::spawn` that watches a `tokio::sync::Notify`. This ensures `pr_url` is available (written by the silent agent during the callback turn) and the agent lock is free for the replayed webhook
- **Pre-pr_url deferral via any active callback on the agent:** When no work item is found by PR URL (because `pr_url` hasn't been written yet), fall back to checking if ANY work item for this agent has an active callback child. If exactly one does, defer. If zero or multiple, don't defer (ambiguous). This handles the primary race where the callback hasn't completed at all yet
- **Return 202 for deferred webhooks:** The gateway already expects 202 from `/message`. Deferred webhooks return 202 with `"status": "deferred"` so the gateway treats them identically
- **60s timeout is per-webhook, not per-callback:** Each deferred webhook has its own deadline. A tokio timer fires at deadline and replays regardless of callback state. This matches the issue's acceptance criteria

## Open Questions

### Resolved During Planning

- **Which event types should be eligible?** PR review events (`pull_request_review.submitted`) and check suite events (`check_suite.completed`) are the only event types routed to mika-dev that reference PRs. Issue events (`issues.assigned`, `issue_comment.created`) lack PR context and cannot correlate -- they pass through unconditionally
- **Where should queued webhooks be replayed?** After `dispatch_resume_agent()` releases the agent lock, not during the callback turn. This is the only point where (a) `pr_url` is guaranteed to be in metadata (written by the silent agent), (b) the agent lock is available for a new turn, and (c) the callback's full lifecycle (including `mark_task_delivered`) has completed
- **Should the queue use SQLite?** No. The 60s window is short, GitHub supports redelivery, and an in-memory queue avoids schema migration complexity. If production shows issues, a persistent queue can be added later

### Deferred to Implementation

- Exact field names and struct layout for `DeferredWebhook` -- depends on what `handle_message` needs to reconstruct the full processing path
- Whether `check_suite` text parsing needs a new regex or can reuse an existing one -- will be determined when implementing the parser

## High-Level Technical Design

> *This illustrates the intended approach and is directional guidance for review, not implementation specification. The implementing agent should treat it as context, not code to reproduce.*

```
                    POST /message (channel: "github")
                              │
                              ▼
                    ┌─────────────────────┐
                    │  Parse webhook text  │
                    │  (PR URL / branch)   │
                    └──────────┬──────────┘
                               │
                    ┌──────────▼──────────┐    no
                    │  Correlates to a    │────────► Normal handle_message()
                    │  work item?         │
                    └──────────┬──────────┘
                               │ yes
                    ┌──────────▼──────────┐    no
                    │  Work item has      │────────► Normal handle_message()
                    │  active callback?   │
                    └──────────┬──────────┘
                               │ yes
                    ┌──────────▼──────────┐
                    │  DEFER: push to     │
                    │  in-memory queue,   │
                    │  start 60s timer    │
                    │  Return 202         │
                    └──────────┬──────────┘
                               │
              ┌────────────────┼────────────────┐
              │                                 │
    ┌─────────▼──────────┐           ┌─────────▼──────────┐
    │ Callback completes │           │ 60s timeout fires  │
    │ (lock released)    │           │                    │
    │ Notify fires       │           │                    │
    └─────────┬──────────┘           └─────────┬──────────┘
              │                                 │
              └────────────┬────────────────────┘
                           │
                ┌──────────▼──────────┐
                │  Drain queue:       │
                │  replay each via    │
                │  handle_message()   │
                │  internal path      │
                └─────────────────────┘
```

## Implementation Units

- [ ] **Unit 1: Webhook deferral data structures and queue on AgentState**

  **Goal:** Define the `DeferredWebhook` struct and add the in-memory queue to `AgentState`. Add a `tokio::sync::Notify` for callback-completion signaling.

  **Requirements:** R1

  **Dependencies:** None

  **Files:**
  - Create: `crates/mika-agent/src/server/webhook_queue.rs`
  - Modify: `crates/mika-agent/src/server/state.rs`
  - Modify: `crates/mika-agent/src/server/mod.rs` (add module)
  - Test: `crates/mika-agent/src/server/webhook_queue.rs` (inline `#[cfg(test)]`)

  **Approach:**
  - `DeferredWebhook` holds: `MessageRequest`, `received_at: Instant`, `work_item_id: String`, `webhook_event_desc: String` (for audit), `deadline: Instant`
  - Queue on `AgentState`: `webhook_queue: Arc<tokio::sync::Mutex<Vec<DeferredWebhook>>>` and `callback_complete_notify: Arc<tokio::sync::Notify>`
  - Expose `should_defer_webhook()` as the pure decision function: takes DB handle + parsed webhook info, returns `Option<(work_item_id, webhook_event_desc)>`
  - Expose `drain_deferred_webhooks()` to extract all items from queue, filtered by work_item_id

  **Patterns to follow:**
  - `AgentState` in `state.rs` — add fields alongside existing `agent_lock`, `dispatcher`
  - `a2a_broadcasters: Arc<DashMap<...>>` on `AppState` as precedent for Arc-wrapped concurrent state

  **Test scenarios:**
  - Happy path: `DeferredWebhook` can be constructed and pushed/drained from the queue
  - Edge case: drain with no matching work_item_id returns empty vec
  - Edge case: drain with mixed work_item_ids only returns matching ones

  **Verification:** Compiles, tests pass, no clippy warnings

- [ ] **Unit 2: Webhook correlation — parse PR context from gateway-formatted text**

  **Goal:** Extract PR URL or branch from gateway-formatted webhook text for all deferrable event types. Determine if a webhook can be correlated to a work item.

  **Requirements:** R2

  **Dependencies:** Unit 1

  **Files:**
  - Modify: `crates/mika-agent/src/server/webhook_queue.rs`
  - Test: `crates/mika-agent/src/server/webhook_queue.rs` (inline tests)

  **Approach:**
  - For `pull_request_review` events: reuse existing `parse_pr_review_event()` to get `PrReviewEvent.pr_url()`
  - For `check_suite` events: new regex to parse `[GitHub] Check suite (conclusion) on repo (branch: branch_name)` format from `format_event_text()`
  - `correlate_webhook()` function: given webhook text, returns `Option<WebhookCorrelation>` with `{ pr_url: Option<String>, branch: Option<String> }`
  - Work item lookup: try `find_active_work_item_by_pr_url()` first; if no match AND branch is available, add new `find_active_work_item_by_branch()` DB query
  - Fallback for pre-pr_url state: if no work item found by URL/branch, check if the agent has exactly one active work item with an active callback child (unambiguous deferral)

  **Patterns to follow:**
  - `parse_pr_review_event()` in `verdict.rs` — regex parsing of gateway text format
  - `find_active_work_item_by_pr_url()` in `db.rs` — JSON metadata extraction in SQL

  **Test scenarios:**
  - Happy path: `pull_request_review` text -> correct PR URL extracted
  - Happy path: `check_suite` text -> correct branch extracted
  - Edge case: `issues.assigned` text -> no correlation (returns None)
  - Edge case: `issue_comment.created` text -> no correlation
  - Happy path: work item found by pr_url with active callback -> deferral recommended
  - Happy path: work item found by pr_url with NO active callback -> no deferral
  - Edge case: no work item found by pr_url, one active callback work item exists -> deferral (fallback)
  - Edge case: no work item found, zero or multiple active callback work items -> no deferral

  **Verification:** All correlation logic has unit test coverage

- [ ] **Unit 3: `find_active_work_item_by_branch()` DB query**

  **Goal:** Add a new DB query to find active work items by the `metadata.claude_pilot.branch` field.

  **Requirements:** R2

  **Dependencies:** None (can be done in parallel with Unit 1)

  **Files:**
  - Modify: `crates/mika-agent/src/db.rs`
  - Modify: `crates/mika-agent/src/async_db.rs`
  - Test: `crates/mika-agent/src/db.rs` (inline `#[cfg(test)]`)

  **Approach:**
  - Mirror `find_active_work_item_by_pr_url()` but query `json_extract(metadata, '$.claude_pilot.branch')` instead
  - Same status filter: `NOT IN ('completed', 'cancelled', 'failed', 'delivered')`
  - Async wrapper in `async_db.rs` following the same pattern

  **Patterns to follow:**
  - `find_active_work_item_by_pr_url()` in `db.rs` line 3184 — exact pattern to mirror

  **Test scenarios:**
  - Happy path: work item with matching branch found
  - Edge case: no work item with that branch -> returns None
  - Edge case: completed work item with matching branch -> not returned
  - Edge case: work item with branch in different metadata path -> not returned

  **Verification:** Tests pass, async wrapper works

- [ ] **Unit 4: Deferral interception in `handle_message()`**

  **Goal:** Add the deferral check at the top of `handle_message()` for GitHub channel webhooks. If deferral is needed, push to queue, spawn timeout timer, return 202.

  **Requirements:** R2, R4, R5

  **Dependencies:** Units 1, 2, 3

  **Files:**
  - Modify: `crates/mika-agent/src/server/handlers.rs`
  - Test: `crates/mika-agent/tests/eval/test_webhook_queue.rs`

  **Approach:**
  - After agent resolution but BEFORE agent lock acquisition: check if this is a GitHub webhook that should be deferred
  - Call `correlate_webhook()` to get PR URL/branch, then `should_defer_webhook()` to check for active callback
  - If deferral: push `DeferredWebhook` to queue, emit `webhook_deferred` audit event, spawn a `tokio::spawn` with `tokio::time::sleep(Duration::from_secs(60))` that drains and replays on timeout, return 202 with `"status": "deferred"`
  - The timeout task: after sleep, drain any remaining webhooks for this work_item_id and replay them by re-entering the processing path (POST to self or call an internal function)
  - Audit event: use `db.log_audit_event()` with tool_name `"webhook_deferred"`, target_key `"task:{work_item_id}"`, after_value with the webhook event description

  **Patterns to follow:**
  - `handle_message()` in `handlers.rs` — existing flow for validation, agent resolution, lock acquisition
  - `log_audit_event()` usage in `verdict_handler.rs` line 273

  **Test scenarios:**
  - Happy path: GitHub webhook for work item with active callback -> 202 with status "deferred", webhook in queue
  - Happy path: GitHub webhook for work item with NO active callback -> normal 202 processing
  - Happy path: non-GitHub webhook -> never deferred regardless of work item state
  - Edge case: webhook for unknown PR -> not deferred
  - Edge case: audit event emitted with correct fields on deferral

  **Verification:** Deferral path returns 202 with "deferred" status, webhook is in queue, audit event logged

- [ ] **Unit 5: Callback completion triggers queue drain**

  **Goal:** After `dispatch_resume_agent()` completes (including silent agent run and `mark_task_delivered`), notify the queue to drain deferred webhooks for the completed work item.

  **Requirements:** R3

  **Dependencies:** Units 1, 4

  **Files:**
  - Modify: `crates/mika-agent/src/server/handlers.rs` (the `handle_task_complete` spawn block)
  - Modify: `crates/mika-agent/src/server/webhook_queue.rs` (drain + replay logic)

  **Approach:**
  - In `handle_task_complete()`, after `dispatcher.dispatch_resume_agent()` returns (success or error): call `drain_and_replay_webhooks()` for the parent work item ID
  - `drain_and_replay_webhooks()`: lock queue, drain matching items, for each item call an internal processing function that mirrors `handle_message()` flow (acquire lock, verdict handler, run_agent)
  - The drain happens in the same `tokio::spawn` as the dispatch, after the lock is released (dispatcher drops guard). Each replayed webhook gets its own agent lock acquisition via `try_lock_owned()` -- if busy (e.g., another webhook is replaying), use `lock().await` (blocking wait, since we know the queue is draining and the agent should be free shortly)
  - Handle `dispatch_resume_agent()` failure (agent busy retry): still drain after the final attempt completes or fails. The callback's result is persisted to DB regardless of dispatch outcome, so `pr_url` may or may not be available -- this is acceptable since the 60s timeout provides a fallback

  **Patterns to follow:**
  - `handle_task_complete()` spawn block at line 451 in `handlers.rs`
  - `dispatch_resume_agent()` retry logic (reset to pending for tick loop)

  **Test scenarios:**
  - Integration: callback completes -> queued webhook drained and processed
  - Integration: callback fails -> queued webhook still drained
  - Edge case: no queued webhooks when callback completes -> no-op, no errors
  - Edge case: multiple queued webhooks -> replayed in arrival order

  **Verification:** Queued webhooks are processed after callback completion with correct metadata state

- [ ] **Unit 6: Integration tests**

  **Goal:** Add integration tests covering the three acceptance criteria scenarios.

  **Requirements:** R6, R7, R8

  **Dependencies:** Units 1-5

  **Files:**
  - Create: `crates/mika-agent/tests/eval/test_webhook_queue.rs`
  - Modify: `crates/mika-agent/tests/eval/mod.rs` (add module)

  **Approach:**
  - Use in-memory `AsyncDatabase` + real `AgentState` construction (following `test_verdict_handler.rs` patterns)
  - Test 1 (R6): Create work item, create active callback child task, send webhook via `handle_message()` -> assert 202 deferred -> complete callback via `handle_task_complete()` -> assert webhook processed after (check audit events or message count)
  - Test 2 (R7): Create work item with NO active callback, send webhook -> assert normal processing (not deferred)
  - Test 3 (R8): Create work item with active callback, send webhook -> assert deferred -> wait >60s (use `tokio::time::pause()` + `advance()` for deterministic time control) -> assert webhook replayed despite callback not completing

  **Execution note:** These are integration-level tests. Use `tokio::time::pause()` for deterministic time control in the timeout test to avoid actual 60s waits.

  **Patterns to follow:**
  - `tests/eval/test_verdict_handler.rs` — test setup with in-memory DB, work item creation, webhook text formatting
  - `tokio::time::pause()` / `advance()` for time-dependent tests

  **Test scenarios:**
  - Happy path (R6): webhook before callback -> deferred -> callback completes -> webhook processed with pr_url visible
  - Happy path (R7): webhook for unrelated work item -> not deferred, processed immediately
  - Happy path (R8): webhook before callback -> deferred -> 60s timeout -> webhook replayed regardless
  - Edge case: webhook for PR with no work item at all -> not deferred
  - Edge case: two webhooks queued for same work item -> both replayed in order after callback

  **Verification:** All three acceptance criteria scenarios pass as automated tests

## System-Wide Impact

- **Interaction graph:** `handle_message()` gains a new early-exit path before lock acquisition. `handle_task_complete()` gains a post-dispatch drain step. The verdict handler is unchanged but benefits from seeing consistent metadata on replayed webhooks
- **Error propagation:** Deferral failures (DB query errors) fall through to normal processing -- fail-open, not fail-closed. This is intentional: a failed deferral check is less dangerous than a dropped webhook
- **State lifecycle risks:** In-memory queue is lost on server restart. Mitigated by the 60s window (short exposure) and GitHub's webhook redelivery. The timeout timer ensures no webhook sits in queue indefinitely
- **API surface parity:** The `/message` endpoint's 202 response gains an optional `"status": "deferred"` field. The gateway does not inspect response bodies beyond status codes, so this is backward-compatible
- **Integration coverage:** The core race condition (webhook before callback) requires an integration test that exercises both `handle_message` and `handle_task_complete` in sequence with time control
- **Unchanged invariants:** The agent lock model (non-blocking `try_lock_owned` in `handle_message`, `try_lock` in `dispatch_resume_agent`) is preserved. The verdict handler's `find_active_work_item_by_pr_url` lookup is unchanged. The dispatch-readiness guard (#525) is unchanged

## Risks & Dependencies

| Risk | Mitigation |
|------|------------|
| In-memory queue lost on restart | 60s window is short; GitHub supports webhook redelivery; gateway LRU dedup resets on restart too |
| Race between deferral check and callback completion | Fail-open: if callback completes during the check, webhook is deferred but immediately drained. Worst case: ~30s delay (tick loop retry) |
| Timeout replay without pr_url (callback hung) | This matches current behavior (webhook processed without metadata). The timeout is a safety valve, not a correctness guarantee |
| Queue memory growth | Bounded by 60s timeout + webhook arrival rate. At worst, a few KB per deferred webhook (text-only, no images from GitHub) |
| Deferral check adds latency to every GitHub webhook | The DB query (two queries: find work item + get child tasks) adds ~1-2ms for SQLite. Acceptable for webhook processing |

## Documentation / Operational Notes

- The `webhook_deferred` audit event provides observability into how often the race occurs in production
- If the audit events show frequent deferrals, consider the persistent queue follow-up
- Dashboard `query_timeline` will surface `webhook_deferred` events automatically via the `unified_timeline` VIEW

## Sources & References

- **GitHub issue:** [#528](https://github.com/senara-solutions/mika/issues/528)
- **Incident doc:** `docs/solutions/agent-quality/2026-04-11-mika-dev-verdict-misclassification-pr-522.md`
- **Companion fixes:** #524 (structural verdict handler), #525 (dispatch-readiness guard)
- **Callback metadata extraction:** `docs/solutions/architecture-patterns/engine-level-callback-metadata-extraction.md`
- **Dispatch readiness guard:** `docs/solutions/architecture-patterns/dispatch-readiness-guard-long-running-status-validation.md`
