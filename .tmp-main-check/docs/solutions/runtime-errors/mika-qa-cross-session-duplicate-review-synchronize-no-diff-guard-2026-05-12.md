---
title: "mika-qa posts cross-session duplicate APPROVED review on no-op synchronize push"
date: 2026-05-12
category: runtime-errors
module: mika-gateway
problem_type: runtime_error
component: tooling
severity: medium
symptoms:
  - "Duplicate APPROVED reviews from mika-qa on the same PR after a force-push that only changes the commit message (trailer-only amend)"
  - "Two separate mika-qa sessions created ~9 minutes apart for the same PR, both posting identical review bodies"
  - "webhook_synchronize event fires for a push with zero file changes"
root_cause: scope_issue
resolution_type: code_fix
tags:
  - mika-qa
  - pr-review
  - duplicate-submission
  - cross-session
  - gateway
  - synchronize-webhook
  - github-compare-api
  - fail-open
---

# mika-qa posts cross-session duplicate APPROVED review on no-op synchronize push

## Problem

When a `pull_request.synchronize` webhook fires for a no-op push (trailer-only amend, commit-message-only change), the gateway unconditionally routes the event to mika-qa, which creates a new session and posts a duplicate APPROVED review with identical body. The session-scope `pr_reviews_posted` DashMap (mika#821/#822) only dedupes within a single session, so cross-session duplicates pass through.

## Symptoms

- PR #885 received two identical APPROVED reviews from mika-qa, ~9 minutes apart
- Two separate sessions (`835faa7b` and `fbc21a06`) created by two separate `synchronize` webhooks
- The amended commit differed from the original only by a `Pipeline-Exempt:` commit-message trailer; the file diff was byte-identical
- Each session independently satisfied the required-tools gate and posted an identical review

## What Didn't Work

The session-scope DashMap defense (mika#821/#822) was designed for within-session dedup (required-tools gate retry). It works correctly for that case. But cross-session duplicates bypass it entirely because each `synchronize` webhook spawns a fresh session with its own DashMap instance. The compound doc (`mika-qa-duplicate-pr-review-required-tools-gate-2026-04-26.md`, lines 130-132) explicitly named this as a future risk: "Per-trace might be a useful middle-ground scope."

## Solution

Gateway-level suppression: for `synchronize` events, compare `before` and `after` commit SHAs (from the webhook payload) via the GitHub Compare API before dispatching to mika-qa. If zero files changed, suppress the event.

### Key implementation details

1. **New fields on `GitHubWebhookEvent`:** `before: Option<String>` and `after: Option<String>` — present on GitHub's `pull_request.synchronize` payloads.

2. **`commits_have_file_changes()` helper:** Calls `GET /repos/{repo}/compare/{before}...{after}` and checks `files.is_empty()`. For trailer-only amends, the compare returns `files: []` with `status: "ahead"` (ahead by 1 commit, 0 file changes).

3. **Guard position:** Between the skill denylist check (step 9b) and semaphore acquisition (step 10). Suppressed events don't consume a semaphore permit or incur formatting cost.

4. **`GitHubApp` wired into gateway:** `AppState.github_app: Option<Arc<GitHubApp>>` constructed from `MIKA_GITHUB_APP_*` env vars. Uses `GitHubApp::from_credentials()` (new public constructor on mika-common) since the gateway has its own `GatewaySettings` type, not the mika-common `Settings`.

5. **Span guard refactor:** The handler previously used `.entered()` (which is `!Send`) with no await points. Adding the Compare API `.await` required switching to explicit `drop(_entered)` / re-enter around the await to keep the future `Send`.

### Fail-open on all error paths

| Scenario | Behavior |
|----------|----------|
| GitHub App not configured | Guard skipped entirely |
| `before`/`after` absent in payload | Pass through |
| API timeout (5s) | Dispatch proceeds |
| HTTP error (4xx/5xx) | Dispatch proceeds |
| Token refresh failure | Dispatch proceeds |
| Response parse failure | Dispatch proceeds |

## Why This Works

The bug is "we triggered work that shouldn't have been triggered." Suppressing at the gateway saves a full session's compute (LLM call + GitHub API calls + tool executions). The Compare API reliably returns `files: []` when two commits share the same tree SHA (trailer-only amend), and returns non-empty `files` when actual code changes exist (including rebases).

## Prevention

- **Scope defense layers to their threat model:** Per-session DashMap prevents within-session duplicates; gateway-level suppression prevents cross-session duplicates from no-op pushes. Neither alone covers both cases.
- **Kill the trigger, don't dedupe the work:** When the root cause is "unnecessary work was triggered," the cheapest fix is preventing the trigger. Agent-level dedup still creates a session and makes API calls before detecting the duplicate.
- **Fail-open by default for routing guards:** A suppressed-but-legitimate review is worse than a duplicate review. All error paths in the no-diff guard fail open.
- **Watch for `!Send` span guards in async handlers:** `tracing::Span::entered()` returns an `Entered` guard that is `!Send`. This is fine in purely synchronous handlers but becomes a compilation error when `.await` is introduced. Use explicit `drop()` / re-enter or `Instrument` when adding await points to previously-sync handlers.

## Related

- [mika#886](https://github.com/senara-solutions/mika/issues/886) — This issue
- [mika#821](https://github.com/senara-solutions/mika/issues/821) / [mika#822](https://github.com/senara-solutions/mika/issues/822) — Within-session DashMap fix
- [mika#695](https://github.com/senara-solutions/mika/issues/695) — Original within-session duplicate review
- [mika-qa-duplicate-pr-review-required-tools-gate-2026-04-26.md](mika-qa-duplicate-pr-review-required-tools-gate-2026-04-26.md) — Companion compound doc covering the within-session case; lines 130-132 named this cross-session gap
- PR #885 — Canonical in-the-wild reproduction
