---
title: "Webhook deferral queue: sequencing webhooks against in-flight callbacks"
date: 2026-04-14
category: architecture-patterns
module: mika-agent (server::webhook_queue, server::handlers)
problem_type: best_practice
component: assistant
severity: high
applies_when:
  - A GitHub webhook arrives while a task has an in-flight run_claude_pilot callback
  - The webhook references a PR whose metadata (pr_url) has not been persisted yet
  - The structural verdict handler (#524) needs pr_url to correlate the webhook to a task
tags: [webhook, callback, race-condition, sequencing, queue, deferral, pr-url]
---

# Webhook deferral queue: sequencing webhooks against in-flight callbacks

## Context

On 2026-04-11, mika-dev experienced a webhook/callback ordering race on PR #522. A `pull_request_review.submitted` webhook arrived 13 seconds before the `claude-pilot completed` callback for the same task. At webhook arrival time, the task had no `pr_url` in metadata (written by the callback path), so the structural verdict handler (#524) could not correlate. The LLM misclassified and re-dispatched instead of merging.

Companion fixes #524 (structural verdict handler) and #525 (dispatch-readiness guard) reduce the impact of misclassification and double-dispatch, but the underlying race remained: webhooks arriving before callback metadata is persisted see stale state.

## Guidance

An in-memory webhook deferral queue (`server::webhook_queue`) holds inbound GitHub webhooks when the target task has an in-flight callback. The queue sits in `handle_message()` between agent resolution and lock acquisition — it intercepts before the webhook enters the processing pipeline.

**Correlation strategy (three tiers):**

1. **PR URL** — `parse_pr_review_event()` extracts PR URL from `pull_request_review` events; `find_active_work_item_by_pr_url()` looks up the task
2. **Branch** — `check_suite` events carry a branch name; `find_active_work_item_by_branch()` (new query on `metadata.claude_pilot.branch`) looks up the task
3. **Sole-inflight fallback** — if no task found by URL/branch, check if exactly one active task has an active callback child. Defers only when unambiguous (exactly one match)

**Active callback detection:** reuses the `get_child_tasks(task_id)` pattern from `validate_dispatch_readiness()` (#525), filtering for `trigger_type="callback" && status IN (pending, in_progress)`.

**Drain triggers:**
- Callback completion: in `handle_task_complete()`, after `dispatch_resume_agent()` returns `Ok(())` (not on `AgentBusy` — metadata hasn't been persisted yet)
- Non-resume_agent callback completion: same pattern in the `else` branch
- Timeout: per-webhook 60s `tokio::sleep` + `drain_expired()` for forced replay

**Replay path:** `replay_deferred_webhooks()` acquires the agent lock (blocking `.lock_owned().await`), then calls the shared `run_agent_for_message()` function — the same code path as normal `handle_message()` processing. No duplication.

## Why This Matters

Without the deferral queue, webhooks that arrive during the callback window see incomplete task state. The structural verdict handler cannot find the task by `pr_url` (not yet written), the LLM receives the webhook without task context, and the result is non-deterministic behavior — misclassification, double-dispatch, or missed merges.

The queue closes the race window at the source. Combined with the structural verdict handler (#524) and dispatch-readiness guard (#525), this provides three layers of defense:

1. **Queue (#528):** prevents the race from occurring
2. **Verdict handler (#524):** handles `pass` verdicts deterministically even if the race occurs
3. **Dispatch guard (#525):** prevents double-dispatch even if misclassification occurs

## When to Apply

- Any new webhook event type routed to the agent (e.g., if `pull_request.closed` is added to `route_event()`) should be evaluated for deferral eligibility by adding correlation parsing to `correlate_webhook()`
- The in-memory queue is acceptable for short-lived deferral windows (≤60s). If a future feature needs longer deferral, consider SQLite persistence
- The sole-inflight fallback is appropriate when the agent typically handles one task at a time. If multi-work-item concurrency increases, the fallback becomes less useful (returns `None` when ambiguous)

## Examples

**Before (race condition):**
```
02:40:48  gateway → agent: pull_request_review.submitted (PR #522)
          → handle_message() → verdict handler → no task found (pr_url missing)
          → LLM misclassifies → re-dispatches instead of merging
02:41:01  callback completes → pr_url written to metadata (too late)
```

**After (deferral queue):**
```
02:40:48  gateway → agent: pull_request_review.submitted (PR #522)
          → handle_message() → deferral check → active callback detected
          → webhook queued, 202 "deferred" returned
02:41:01  callback completes → dispatch_resume_agent() Ok
          → drain queue → replay webhook through run_agent_for_message()
          → verdict handler → finds task (pr_url now in metadata)
          → merge initiated
```

## Related

- [Structural verdict handler](structural-verdict-handler-pr-review-auto-merge.md) — companion fix #524
- [Dispatch readiness guard](dispatch-readiness-guard-long-running-status-validation.md) — companion fix #525
- [Callback metadata extraction](engine-level-callback-metadata-extraction.md) — explains what `try_extract_callback_metadata()` writes
- [Incident doc](../../solutions/agent-quality/2026-04-11-mika-dev-verdict-misclassification-pr-522.md) — the triggering incident
- GitHub issue: #528
