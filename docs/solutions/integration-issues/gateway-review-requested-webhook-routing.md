---
title: Route pull_request.review_requested webhook to mika-qa
date: 2026-04-21
module: mika-gateway
problem_type: integration_issue
component: tooling
severity: medium
symptoms:
  - Requesting mika-platform-qa as PR reviewer produces no QA review
  - pull_request.review_requested webhook silently dropped by gateway
root_cause: missing_workflow_step
resolution_type: code_fix
tags: [gateway, webhook, github, review-requested, routing, machine-user]
---

# Gateway drops pull_request.review_requested — QA review not triggered

## Problem

With machine user accounts (`mika-platform-dev`, `mika-platform-qa`) in the org, requesting `mika-platform-qa` as a PR reviewer fires a `pull_request` + `review_requested` GitHub webhook. The gateway's `route_event()` only matched `opened | synchronize` for pull_request events routed to mika-qa — `review_requested` fell through to `_ => None` and was silently dropped. The qa-review skill never activated.

## Symptoms

- Requesting a reviewer on a PR in the GitHub UI produces no mika-qa response
- Gateway logs show `warn` for unroutable `pull_request.review_requested` event (observability added in #401)
- The `review_requested` delivery appears in gateway traces but no forwarding occurs

## What Didn't Work

This was identified directly from the routing table — no failed investigation paths. The exact same gap pattern was documented previously for `pull_request.closed` (see Related Issues).

## Solution

Three changes in `crates/mika-gateway/src/github.rs`:

**1. Add `review_requested` to routing match (one-line change):**

```rust
// Before:
("pull_request", Some("opened" | "synchronize")) => Some("mika-qa"),

// After:
("pull_request", Some("opened" | "synchronize" | "review_requested")) => Some("mika-qa"),
```

**2. Add `requested_reviewer` field to `GitHubWebhookEvent` struct:**

```rust
/// Requested reviewer (present in pull_request.review_requested events).
pub requested_reviewer: Option<GitHubUser>,
```

GitHub's `review_requested` payload includes a top-level `requested_reviewer` object. Adding it as `Option<GitHubUser>` allows enriching the formatted message. The `Option` wrapper ensures other event types (which lack this field) still deserialize correctly — serde ignores unknown fields by default, but having the field explicitly modeled enables type-safe access.

**3. Enrich formatted message with reviewer identity:**

```rust
if action == "review_requested"
    && let Some(reviewer) = &event.requested_reviewer
{
    text.push_str(&format!("\nRequested reviewer: @{}", reviewer.login));
}
```

This follows the same conditional enrichment pattern as the `closed` action's `Merged: {bool}` line.

## Why This Works

The `route_event()` function is a static match table — the single gate for which events are routable. Adding `"review_requested"` to the or-pattern is the only change needed for routing. The `format_event_text()` function already handles arbitrary `pull_request` actions generically (`[GitHub] PR {action}: ...`), so `review_requested` produces valid output even without the enrichment. The enrichment adds reviewer context for mika-qa.

No feedback loop risk: mika-qa receives `review_requested`, performs its review, and submits a `pull_request_review.submitted` event — which routes to mika-dev (different agent). Routing table partitioning prevents loops.

## Prevention

- When adding new GitHub webhook workflows, trace the full lifecycle: event source (GitHub) -> gateway routing (`route_event()`) -> skill activation -> task state transition. Any gap means events get dropped silently.
- The `route_event()` function is the single gate — `agent_mapping` only remaps names, not event eligibility.
- New struct fields for webhook payloads must use `Option<T>` to avoid parse failures for events that omit them (#403 precedent).
- Update all routing table references when adding new routes: `crates/mika-gateway/CLAUDE.md`, `docs/solutions/architecture-patterns/github-webhook-endpoint-gateway.md`, skill prompts that mention triggers.

## Related Issues

- #506 — This issue
- `docs/solutions/integration-issues/gateway-pr-closed-webhook-routing.md` — Same gap pattern for `pull_request.closed`
- `docs/solutions/architecture-patterns/github-webhook-endpoint-gateway.md` — Canonical routing table reference
- `docs/solutions/architecture/github-app-identity-and-agent-infrastructure.md` — `MIKA_GITHUB_APP_LOGIN` and per-agent identity
- `docs/solutions/runtime-errors/github-webhook-parse-fails-missing-app-id.md` — Why new fields must be `Option<T>`
