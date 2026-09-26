---
title: "feat(server): structural check_suite.completed(success) handler"
type: feat
status: active
date: 2026-04-15
issue: 571
---

# feat(server): structural check_suite.completed(success) handler

## Overview

Add a structural handler that auto-merges PRs when CI turns green after a QA `VERDICT: pass` has already been recorded. Mirrors the existing `verdict_handler` pattern — server-side Rust, zero LLM involvement, deterministic state-machine transition.

## Problem Frame

Observed on PR #570: mika-qa posted `VERDICT: pass`, `verdict_handler` fired, but CI was red at that moment — merge correctly declined and `auto_merge_enabled` was set. A fix was pushed, CI turned green. Nothing re-evaluated the pending merge because `check_suite.completed(success)` events are silently dropped by the gateway. The PR sat approved + green + unmerged until a human intervened.

The gap is in `crates/mika-gateway/src/github.rs` — `route_event()` routes `check_suite.completed` only for `failure`/`timed_out` conclusions. Success falls through to `None`.

## Requirements Trace

- R1. Route `check_suite.completed(success)` webhooks to `mika-dev` (gateway)
- R2. New structural handler evaluates merge conditions without LLM involvement
- R3. Handler reuses existing `pr_merge_with_gate` helpers — no merge logic duplication
- R4. Stale-verdict gate: QA review `commit_id` must match current PR `head.sha`
- R5. CI aggregation gate: all required checks must pass (webhook is per-workflow, not per-PR)
- R6. Self-terminating on post-merge webhooks (no open PR for `main` branch)
- R7. Idempotent: concurrent/duplicate webhooks produce `AlreadyMerged`, no error logs

## Scope Boundaries

- No new crates, no schema changes, no LLM involvement
- No new HTTP endpoints, no new config fields
- No changes to the existing `verdict_handler` (it remains the `pull_request_review` handler)
- The handler uses `gh` CLI subprocess calls (same as `verdict_handler` and `pr_merge_with_gate`), not the GitHub REST API directly

## Context & Research

### Relevant Code and Patterns

- `crates/mika-agent/src/server/verdict_handler.rs` — direct structural parallel; `VerdictAction` enum, `Handled`/`Passthrough` return pattern, pre-digest formatting, 60s subprocess timeout
- `crates/mika-agent/src/server/handlers.rs:750-777` — `run_agent_for_message` structural handler interception point inside `if req.channel == "github"` block
- `crates/mika-agent/src/tools/pr_merge_with_gate.rs` — `pub(crate)` helpers: `CheckClassification`, `classify_checks`, `run_gh_checks`, `run_gh_merge`
- `crates/mika-gateway/src/github.rs:142-159` — `route_event()` function, `check_suite` routing gap
- `crates/mika-agent/src/server/verdict.rs` — parser module for PR review text format

### Institutional Learnings

- `docs/solutions/architecture-patterns/structural-verdict-handler-pr-review-auto-merge.md` — authoritative compound doc for the verdict handler pattern
- `docs/solutions/security-issues/verdict-handler-global-token-source.md` — **must** use `a.settings.resolve_github_token()`, never `s.github_token` (global AppState is empty in multi-agent)
- `docs/solutions/logic-errors/webhook-fallthrough-dispatches-unrelated-backlog-work.md` — structural handlers prevent webhook fallthrough to LLM which can trigger unrelated backlog dispatch
- `docs/solutions/architecture-patterns/webhook-deferral-queue-callback-sequencing.md` — check_suite events correlate by branch name (Tier 2), not PR URL

## Key Technical Decisions

- **Handler location: `mika-agent/src/server/`** not gateway: Gateway is transport; semantics belong to the agent. Keeps `route_event` content-agnostic. Direct same-crate reuse of `pr_merge_with_gate` helpers.
- **Re-query reviews via `gh api`** not DB lookup: Single source of truth for QA approval status. Matches `pr_merge_with_gate` philosophy.
- **Stale-SHA gate (strict)**: `review.commit_id == pr.head.sha`. A push after approval — even a mechanical fix — is unreviewed code. The cost is one extra QA cycle; the alternative is silently trusting that the push was safe.
- **CI aggregation is load-bearing, not defensive**: `check_suite.completed` event is scoped to one workflow. A PR with multiple required workflows can fire this event while another is pending or failed. The aggregation via `run_gh_checks` + `classify_checks` IS the gate.
- **Return type: reuse `VerdictAction`**: Same `Handled`/`Passthrough` enum from `verdict_handler` — no new return type needed. Handler results are processed identically in `handlers.rs`.
- **Non-matching events return `Passthrough { enrichment: None }`**: Order-independent — each handler self-selects on event type. Both handlers passthrough for non-matching events.

## Open Questions

### Resolved During Planning

- **Should `StaleVerdict` / `NoPassVerdict` return `Passthrough` or `Handled`?** → `Passthrough { enrichment: None }` — let the LLM see the webhook and decide follow-up (same as verdict handler's treatment of `block`/`hold` verdicts)
- **How to find the PR from the webhook?** → `gh pr list --repo {repo} --head {branch} --state open --json number,headRefName,headRefOid` — branch-based lookup since check_suite carries branch, not PR number

### Deferred to Implementation

- Exact pre-digest message wording (will mirror verdict_handler's XML-tag pattern and completion-claim-safe phrasing)
- Whether `CiSuccessAction` enum needs `Display` impl or just `Debug`

## Implementation Units

- [ ] **Unit 1: Gateway routing — route check_suite success to mika-dev**

  **Goal:** Forward `check_suite.completed(success)` webhooks to `mika-dev` agent instead of silently dropping them.

  **Requirements:** R1

  **Dependencies:** None

  **Files:**
  - Modify: `crates/mika-gateway/src/github.rs`

  **Approach:**
  - Add `"success"` to the existing `check_conclusion` match arm: `Some("failure" | "timed_out" | "success") => Some("mika-dev")`
  - Flip existing `test_route_event_check_suite_success` assertion from `None` to `Some("mika-dev")`

  **Patterns to follow:**
  - Existing `route_event()` match structure in `github.rs:147-158`

  **Test scenarios:**
  - Happy path: `route_event("check_suite", Some("completed"), Some("success"))` returns `Some("mika-dev")` (regression test flip)
  - Edge case: `route_event("check_suite", Some("completed"), Some("neutral"))` returns `None` (unchanged)
  - Edge case: `route_event("check_suite", Some("completed"), None)` returns `None` (unchanged)

  **Verification:**
  - `cargo test -p mika-gateway test_route_event` passes with updated assertion

- [ ] **Unit 2: CI success handler module — `CiSuccessAction` enum and `try_handle_ci_success` function**

  **Goal:** Create the structural handler that evaluates merge conditions for `check_suite.completed(success)` events and initiates merge when all conditions are met.

  **Requirements:** R2, R3, R4, R5, R6, R7

  **Dependencies:** Unit 1 (gateway must route the event for handler to ever fire; but handler code is independently testable)

  **Files:**
  - Create: `crates/mika-agent/src/server/ci_success_handler.rs`
  - Modify: `crates/mika-agent/src/server/mod.rs` (add `pub mod ci_success_handler;`)

  **Approach:**

  The handler follows the verdict_handler's "fail to passthrough" pattern:

  1. **Parse**: Extract `check_suite.conclusion`, `repository.full_name`, `check_suite.head_branch`, `check_suite.head_sha` from gateway-formatted text. Non-matching text → `Passthrough { enrichment: None }`.
  2. **Find PR**: `gh pr list --repo {repo} --head {branch} --state open --json number,headRefName,headRefOid`. No open PR → `NoPr` action, return `Passthrough` (self-terminates post-merge webhook on `main`).
  3. **Find QA verdict**: `gh api /repos/{repo}/pulls/{pr}/reviews`. Look for APPROVED review with `VERDICT: pass` in body (parse body, not state — per existing contract at `docs/skills.md`). Not found → `NoPassVerdict`, return `Passthrough`.
  4. **Stale-SHA gate**: `review.commit_id == pr.head.sha`. Mismatch → `StaleVerdict`, return `Passthrough`.
  5. **CI aggregation**: `run_gh_checks` + `classify_checks`. Not `AllPassed` → `ChecksNotAllGreen`, return `Passthrough`.
  6. **Merge**: `run_gh_merge(pr, repo, "squash", true, false, token)`. Map `already merged`/`Pull request is closed` stderr → `AlreadyMerged` (info log, not error). Update work item metadata, log audit event, send notification.
  7. **Pre-digest**: Format `<ci_success_handler>` XML block with "Do NOT call pr_merge_with_gate" instruction.

  Token resolution: `github_token: Option<&str>` parameter, resolved by caller via `a.settings.resolve_github_token()`.

  All subprocess calls wrapped in 60s `tokio::time::timeout`.

  **Patterns to follow:**
  - `verdict_handler.rs` — function signature, `VerdictAction` return type, error handling, pre-digest formatting, metadata update via `merge_metadata`, audit logging
  - `pr_merge_with_gate.rs` — `run_gh_checks`, `classify_checks`, `run_gh_merge` helper reuse

  **Test scenarios:**
  - Happy path: pre-digest message avoids completion-claim trigger words (`merged`, `deployed`, `completed`, `shipped`) — regex test matching `verdict_handler` test pattern
  - Happy path: pre-digest contains "Do NOT call pr_merge_with_gate" instruction
  - Happy path: pre-digest contains work item ID when present
  - Edge case: `AlreadyMerged` pre-digest avoids completion-claim trigger words
  - Error path: error pre-digest contains the error message
  - Edge case: non-matching text (not a check_suite success event) returns `Passthrough { enrichment: None }`

  **Verification:**
  - Module compiles: `cargo check -p mika-agent`
  - Unit tests pass: `cargo test -p mika-agent ci_success`

- [ ] **Unit 3: Wire handler into `handle_message`**

  **Goal:** Call `try_handle_ci_success` alongside the existing `try_handle_pr_review_verdict` in the `if req.channel == "github"` block.

  **Requirements:** R2

  **Dependencies:** Unit 2

  **Files:**
  - Modify: `crates/mika-agent/src/server/handlers.rs`

  **Approach:**
  - Add a second structural handler call after the verdict handler in `run_agent_for_message`, inside the existing `if req.channel == "github"` block (around line 777)
  - Share the already-resolved `verdict_github_token` — no second token resolution needed
  - Process `VerdictAction` identically: `Handled` → replace `req.text`, `Passthrough { enrichment }` → prepend, `Passthrough { None }` → no-op
  - Order between verdict_handler and ci_success_handler is irrelevant — each handler self-selects on event type and returns `Passthrough` for non-matching events. Only one will ever return `Handled` for a given webhook.

  **Patterns to follow:**
  - Existing verdict handler call at `handlers.rs:750-777`

  **Test scenarios:**

  Test expectation: none — wiring is structural glue code. The handler's logic is tested in Unit 2; the handlers.rs integration is covered by the existing `test_message_returns_202_accepted` test (proves the handler doesn't crash on non-matching events).

  **Verification:**
  - Full build compiles: `cargo build -p mika-agent`
  - Existing handler tests still pass: `cargo test -p mika-agent -- server`

- [ ] **Unit 4: Documentation updates**

  **Goal:** Document the new handler alongside the existing verdict_handler.

  **Requirements:** None (documentation)

  **Dependencies:** Units 1-3

  **Files:**
  - Modify: `crates/mika-agent/CLAUDE.md` (one-line entry for the new handler)
  - Modify: `docs/solutions/architecture-patterns/structural-verdict-handler-pr-review-auto-merge.md` (append CI success pattern, stale-verdict policy, CI-aggregation invariant)

  **Approach:**
  - Add a `### Structural CI Success Handler` subsection to `crates/mika-agent/CLAUDE.md` under the existing `### Structural Verdict Handler` entry
  - Append to the compound doc: document the `check_suite.completed(success)` pattern as a companion to the verdict handler, the stale-verdict policy decision (Option B — strict), and the CI-aggregation invariant (load-bearing, not defensive)

  **Test expectation:** none — documentation only

  **Verification:**
  - Files are well-formed markdown

## System-Wide Impact

- **Interaction graph:** Gateway `route_event()` → agent `handle_message` → `try_handle_ci_success` → `run_gh_checks`/`run_gh_merge` subprocesses. Webhook deferral queue already handles check_suite events by branch correlation (Tier 2) — no changes needed.
- **Error propagation:** Subprocess failures (gh CLI) → `Handled { pre_digest: error }` (same as verdict_handler). DB failures → `Passthrough` (warn log, let LLM handle).
- **State lifecycle risks:** Concurrent webhooks: `AlreadyMerged` graceful handling prevents duplicate merge attempts. Post-merge `check_suite` webhooks on `main`: `NoPr` self-terminates (no open PR for main branch).
- **API surface parity:** No new endpoints. No API changes.
- **Unchanged invariants:** `verdict_handler` is not modified. `pr_merge_with_gate` tool is not modified. Gateway's `format_event_text()` output format is not modified.

## Risks & Dependencies

| Risk | Mitigation |
|------|------------|
| check_suite webhook storm after CI fix (multiple workflows completing) | Handler is idempotent — first merge succeeds, subsequent → `AlreadyMerged`. Agent lock serializes concurrent `handle_message` calls (429 if busy). |
| QA bot login detection (finding the right review) | Use same bot-login detection as existing verdict handler — look for `VERDICT: pass` in review body text |
| gh CLI subprocess hangs | 60s timeout on all subprocess calls (same as verdict_handler) |

## Sources & References

- Related issue: #571
- Related PRs/issues: #524 (verdict_handler), #555 (PR merge gate), #570 (incident)
- Compound doc: `docs/solutions/architecture-patterns/structural-verdict-handler-pr-review-auto-merge.md`
- Security learning: `docs/solutions/security-issues/verdict-handler-global-token-source.md`
