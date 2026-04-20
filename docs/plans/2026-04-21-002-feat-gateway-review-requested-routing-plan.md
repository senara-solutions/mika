---
title: "feat(gateway): route pull_request.review_requested to mika-qa + machine user assignee support"
type: feat
status: active
date: 2026-04-21
---

# feat(gateway): route pull_request.review_requested to mika-qa + machine user assignee support

## Overview

Add `review_requested` to the gateway's pull_request routing so requesting `mika-platform-qa` as a PR reviewer triggers mika-qa's qa-review skill. Enrich the formatted message with the requested reviewer login. Update the qa-review skill prompt to document the new trigger. Verify machine user assignee filtering works for `mika-platform-dev`.

## Problem Frame

Machine user accounts (`mika-platform-dev`, `mika-platform-qa`) are now in the org. Requesting `mika-platform-qa` as a PR reviewer fires a `pull_request` + `review_requested` webhook, but the gateway's `route_event()` does not match this action — it falls through to `None` and is silently dropped. The qa-review skill never fires.

For assignee filtering, `MIKA_GITHUB_APP_LOGIN` is a single `Option<String>` in config. The filtering logic lives in the self-dev skill prompt (in `mika-skills/` repo, not in Rust code). The agent simply passes the configured login through. Setting `MIKA_GITHUB_APP_LOGIN=mika-platform-dev` in the per-agent `.env` is a configuration concern, not a code change in this repo.

## Requirements Trace

- R1. `pull_request.review_requested` webhooks route to mika-qa
- R2. Formatted message includes the requested reviewer's login for context
- R3. Existing `opened`/`synchronize`/`closed` routing unchanged
- R4. qa-review skill prompt documents the new trigger event
- R5. Gateway CLAUDE.md routing table updated
- R6. Machine user assignee filtering verified (configuration, not code — documented)

## Scope Boundaries

- No changes to the `GitHubWebhookEvent` struct beyond adding the optional `requested_reviewer` field
- No changes to assignee filtering Rust code — `MIKA_GITHUB_APP_LOGIN` is already a single string config; the self-dev skill prompt in `mika-skills/` handles the comparison
- No changes to the verdict handler — `pull_request_review.submitted` routing to mika-dev is unchanged
- No feedback loop risk — mika-qa receives `review_requested` but does not generate `review_requested` events (it submits reviews, which produce `pull_request_review.submitted` routed to mika-dev)

### Deferred to Separate Tasks

- Self-dev skill prompt updates for machine user login matching: `mika-skills/` repo
- Per-agent `.env` configuration for `MIKA_GITHUB_APP_LOGIN=mika-platform-dev`: operational task

## Context & Research

### Relevant Code and Patterns

- `crates/mika-gateway/src/github.rs` line 148-160: `route_event()` match table
- `crates/mika-gateway/src/github.rs` line 225-247: `format_event_text()` pull_request branch
- `crates/mika-gateway/src/github.rs` line 1021-1050: existing routing test pattern (`test_route_event_pr_*`)
- `crates/mika-gateway/src/github.rs` line 1124-1152: existing format test pattern (`test_format_event_text_pr_opened`)
- `skills/bundled/qa-review/system_prompt.md` line 5: trigger documentation

### Institutional Learnings

- `docs/solutions/integration-issues/gateway-pr-closed-webhook-routing.md`: Same gap pattern — `pull_request.closed` was silently dropped until a match arm was added. Confirms the one-line routing fix approach.
- `docs/solutions/architecture-patterns/github-webhook-endpoint-gateway.md`: Bot self-event filtering was intentionally removed (#401). Loop prevention relies on routing table partitioning. `review_requested` → mika-qa is safe because mika-qa submits reviews (→ mika-dev), not review requests.
- `docs/solutions/runtime-errors/github-webhook-parse-fails-missing-app-id.md`: New struct fields must be `Option<T>` to avoid parse failures for events that omit them.
- `docs/solutions/logic-errors/webhook-fallthrough-dispatches-unrelated-backlog-work.md`: The receiving skill's keywords must match the event type. qa-review already matches `pull_request` keywords; updating the prompt ensures the skill knows it handles `review_requested`.

## Key Technical Decisions

- **Add `requested_reviewer` to struct:** The `review_requested` webhook includes a top-level `requested_reviewer` object. Adding it as `Option<GitHubUser>` enriches the formatted message with who was requested, giving mika-qa better context. Uses the existing `GitHubUser` type. `Option` ensures other event types that lack this field still parse correctly.
- **Enrich format text for `review_requested` action:** Add a conditional line `Requested reviewer: @{login}` in the `pull_request` format branch when the action is `review_requested` and the field is present. This follows the same pattern as the `closed` action's `Merged: {bool}` enrichment (line 239-242).
- **No Rust code for Part B (assignee filtering):** `MIKA_GITHUB_APP_LOGIN` is already a single-string config. The filtering logic lives in the self-dev skill prompt. Setting the config to `mika-platform-dev` is an operational step, not a code change.

## Open Questions

### Resolved During Planning

- **Does `format_event_text` need changes?** Yes — adding `requested_reviewer` info enriches the message for mika-qa. The format branch already handles arbitrary PR actions generically, so `review_requested` will produce `[GitHub] PR review_requested: ...`. The enrichment adds reviewer identity.
- **Feedback loop risk?** None. mika-qa receives `review_requested`, performs its review, and submits a `pull_request_review.submitted` event which routes to mika-dev. The routing table partitions events cleanly.

### Deferred to Implementation

- None — this is a well-understood, pattern-following change.

## Implementation Units

- [x] **Unit 1: Add `review_requested` routing and `requested_reviewer` enrichment**

**Goal:** Route `pull_request.review_requested` to mika-qa and enrich the formatted message with the requested reviewer's login.

**Requirements:** R1, R2, R3

**Dependencies:** None

**Files:**
- Modify: `crates/mika-gateway/src/github.rs` (struct, routing, formatting, tests)

**Approach:**
- Add `pub requested_reviewer: Option<GitHubUser>` field to `GitHubWebhookEvent` struct (after `review` field, line ~69)
- Add `"review_requested"` to the or-pattern on line 151: `("pull_request", Some("opened" | "synchronize" | "review_requested")) => Some("mika-qa")`
- In `format_event_text()` pull_request branch (line 238), add a conditional after the `closed`/merged block: if `action == "review_requested"`, append `Requested reviewer: @{login}` from `event.requested_reviewer`
- Add routing test: `test_route_event_pr_review_requested` asserting `Some("mika-qa")`
- Add format test: `test_format_event_text_pr_review_requested` asserting output contains `[GitHub] PR review_requested` and `Requested reviewer: @`

**Patterns to follow:**
- Routing test pattern: `test_route_event_pr_opened` (line 1021)
- Format test pattern: `test_format_event_text_pr_opened` (line 1124)
- Conditional enrichment pattern: `closed` action's `Merged: {bool}` (line 239-242)
- Optional struct field pattern: all existing fields on `GitHubWebhookEvent` are `Option<T>`

**Test scenarios:**
- Happy path: `route_event("pull_request", Some("review_requested"), None)` returns `Some("mika-qa")`
- Happy path: `format_event_text("pull_request", event)` with `action=review_requested` and `requested_reviewer` set contains `[GitHub] PR review_requested` and `Requested reviewer: @{login}`
- Edge case: `format_event_text` with `action=review_requested` but `requested_reviewer=None` still produces valid output without the reviewer line
- Happy path: Existing `opened` and `synchronize` routing unchanged (existing tests cover this — run and confirm they pass)

**Verification:**
- `cargo test -p mika-gateway` passes with new and existing tests
- `cargo clippy -p mika-gateway` clean

- [x] **Unit 2: Update qa-review skill prompt and gateway CLAUDE.md**

**Goal:** Document the new `review_requested` trigger in the qa-review skill prompt and update the gateway routing table in CLAUDE.md.

**Requirements:** R4, R5

**Dependencies:** Unit 1

**Files:**
- Modify: `skills/bundled/qa-review/system_prompt.md`
- Modify: `crates/mika-gateway/CLAUDE.md`

**Approach:**
- In `skills/bundled/qa-review/system_prompt.md` line 5, add `pull_request.review_requested` to the trigger list
- In `crates/mika-gateway/CLAUDE.md` line 24, add `pull_request.review_requested` to the mika-qa routing entry

**Patterns to follow:**
- Existing trigger documentation format in qa-review prompt
- Existing routing table format in CLAUDE.md

**Test expectation: none** — documentation-only changes

**Verification:**
- qa-review prompt lists all three trigger events
- CLAUDE.md routing table reflects the new route

- [x] **Unit 3: Document machine user assignee configuration**

**Goal:** Document the machine user assignee configuration requirements for Part B of the issue.

**Requirements:** R6

**Dependencies:** None

**Files:**
- Modify: `crates/mika-gateway/CLAUDE.md` (add note about machine user configuration)

**Approach:**
- Add a brief note in the GitHub Webhook Integration section explaining that `MIKA_GITHUB_APP_LOGIN` in per-agent `.env` should be set to the machine user login (e.g., `mika-platform-dev`) for autonomous issue pickup. Note that the filtering logic lives in the self-dev skill prompt.

**Test expectation: none** — documentation-only change

**Verification:**
- CLAUDE.md mentions machine user login configuration

## System-Wide Impact

- **Interaction graph:** Gateway `route_event()` → agent container `/message` → qa-review skill activation. No new interaction paths — follows the existing `opened`/`synchronize` flow.
- **Error propagation:** Unchanged — `review_requested` follows the same retry/DLQ path as other routed events.
- **State lifecycle risks:** None — no new state is introduced.
- **API surface parity:** The `requested_reviewer` struct field is additive and optional. All existing webhook payloads continue to deserialize correctly.
- **Unchanged invariants:** `pull_request_review.submitted` → mika-dev routing is unchanged. The verdict handler and auto-merge path are unaffected. `pull_request.closed` → mika-dev is unchanged.

## Risks & Dependencies

| Risk | Mitigation |
|------|------------|
| `requested_reviewer` field name differs from GitHub's actual payload key | GitHub docs confirm `requested_reviewer` is the top-level field name for `review_requested` action. serde will silently ignore if absent. |
| qa-review skill does not keyword-match `review_requested` | The skill matches on `pull_request` keyword which is present in the formatted text `[GitHub] PR review_requested:...`. Updating the prompt is documentation, not routing logic. |

## Sources & References

- Related issue: #506
- Related code: `crates/mika-gateway/src/github.rs` (routing, formatting, tests)
- Related solution: `docs/solutions/integration-issues/gateway-pr-closed-webhook-routing.md`
- Related plan: `docs/plans/2026-04-03-006-feat-github-app-identity-agent-infrastructure-plan.md` (#416)
- GitHub docs: [pull_request webhook event](https://docs.github.com/en/webhooks/webhook-events-and-payloads#pull_request)
