---
title: "Structural CI failure handler — dispatch focused fix to claude-pilot"
date: 2026-04-20
category: architecture-patterns
module: mika-agent
problem_type: best_practice
component: tooling
severity: high
applies_when:
  - Adding a new structural webhook handler for check_suite events
  - Debugging CI failure dispatch flow or circuit breaker behavior
  - Understanding the interaction between structural handlers and the LLM turn
tags:
  - check-suite
  - ci-failure
  - structural-handler
  - webhook
  - dispatch
  - circuit-breaker
  - claude-pilot
---

# Structural CI failure handler — dispatch focused fix to claude-pilot

## Context

`check_suite.completed/failure` events routed to mika-dev fell through to a regular LLM turn. The LLM burned the 5-minute engine wall-clock budget on diagnostic tool calls (`gh pr view`, `gh pr checks`, `gh run view --job`) without ever dispatching `run_claude_pilot` to fix the failure. Observed twice on PR #592 (mika#589) — both attempts hit the same timeout pattern.

The success-side handler (#571, `ci_success_handler.rs`) proved that structural webhook interception is the correct pattern for deterministic state-machine transitions. This is the failure-side companion.

## Guidance

`server::ci_failure_handler` intercepts `check_suite.completed(failure|timed_out)` webhook events **before** the LLM turn. The handler:

1. Parses the gateway-formatted event text (`[GitHub] Check suite {conclusion} on {repo} (branch: {branch})`)
2. Skips `main`/`master` branches (loop prevention)
3. Finds the open PR via `find_open_pr()` (shared with `ci_success_handler`)
4. Matches to an active work item by PR URL then branch fallback
5. Gates on task status (`in_progress` only) and active callback children (fix already in-flight)
6. Applies circuit breaker: `ci_fix_count >= 2` in task metadata → escalation instead of dispatch
7. Fetches failure context: failing checks via `run_gh_checks` + job logs via `gh run view --job <id> --log-failed` (max 3 jobs, 100 lines each)
8. Constructs a pre-digest in `<ci_failure_handler>` XML tags for the LLM to dispatch `run_claude_pilot`

Key design decisions:

- **Pre-digest, not direct dispatch.** The handler prepares context; the LLM invokes `run_claude_pilot` through the normal skill executor path. The dispatch-readiness guard in `executor.rs` is the authoritative gate.
- **Handler increments `ci_fix_count` deterministically.** Relying on the LLM to increment the counter is unreliable (compaction can drop the instruction). The handler writes the increment before returning the pre-digest, with "Do NOT re-increment" in the LLM instructions.
- **Fetch failures before incrementing.** The `ci_fix_count` increment happens AFTER `fetch_failure_context` confirms there are actual failing checks. If CI self-heals between the event and the handler's check fetch, the handler returns `Passthrough` without consuming a circuit-breaker slot.

## Why This Matters

Without the structural handler, CI failure events fall through to the LLM, which burns the 5-minute engine budget on diagnostic tools without reaching the dispatch step. This was observed twice on the same PR — the LLM consistently timed out before calling `run_claude_pilot`.

The pattern follows the established principle: when an external event should trigger a deterministic state transition, handle it structurally in the engine layer before the LLM turn (see `docs/solutions/architecture-patterns/engine-guards-vs-prompt-rules-for-agent-behavior-2026-04-19.md`).

## When to Apply

- Adding new structural handlers for GitHub webhook events
- Debugging why CI failures aren't being auto-fixed
- Understanding the `ci_fix_count` circuit breaker behavior
- Modifying the webhook handler chain in `handlers.rs`

## Examples

**Handler registration pattern** (in `handlers.rs`, inside `if req.channel == "github"` block):

```rust
// Order-independent — each handler self-selects on event type
let ci_failure_action = ci_failure_handler::try_handle_ci_failure(
    &req.text, &a.db, verdict_github_token.as_deref(),
    Some(&sender_arc), &session_id, &req.request_id,
).await;
match ci_failure_action {
    VerdictAction::Handled { pre_digest } => { req.text = pre_digest; }
    VerdictAction::Passthrough { enrichment: Some(e) } => { req.text = format!("{e}{}", req.text); }
    VerdictAction::Passthrough { enrichment: None } => {}
}
```

**Bug fix: `CHECK_SUITE_RE` regex in `webhook_queue.rs`:**

The deferral queue's regex expected `[GitHub] Check suite (failure) on ...` (parenthesized conclusion) but the gateway produces `[GitHub] Check suite failure on ...` (bare word). The regex `\S+` in the fix matches both, but the old test strings also needed updating.

**Bug fix: `fetch_job_log` missing `run_id` and `--repo`:**

The initial implementation extracted only `job_id` from the check link URL. `gh run view` requires a positional `run_id` argument AND a `--repo` flag (the server process runs outside a git checkout). Both were added via `parse_check_link()` helper.

## Related

- [Structural verdict handler](structural-verdict-handler-pr-review-auto-merge.md) — companion PR review handler
- [Engine guards vs prompt rules](engine-guards-vs-prompt-rules-for-agent-behavior-2026-04-19.md) — foundational principle
- [Webhook deferral queue](webhook-deferral-queue-callback-sequencing.md) — branch-based correlation for check_suite events
- [Dispatch readiness guard](dispatch-readiness-guard-long-running-status-validation.md) — authoritative gate for `run_claude_pilot` dispatch
- GitHub issue: #594
