---
module: mika-agent
date: 2026-04-13
problem_type: best_practice
component: tooling
severity: high
tags:
  - webhook-handler
  - verdict-handler
  - ci-success-handler
  - check-suite
  - pr-merge
  - structural-handler
  - pre-digest
  - llm-bypass
related_issues:
  - 524
  - 522
  - 525
  - 571
last_updated: 2026-04-15
applies_when:
  - An external event should trigger a deterministic state transition
  - The LLM could misinterpret or improvise on a raw webhook payload
  - A time-sensitive action (like PR merge) cannot wait for LLM processing
---

# Structural PR Review Verdict Handler — Engine-Level Auto-Merge

## Context

On 2026-04-11, mika-dev (qwen3-coder) received a `pull_request_review.submitted(approved)` webhook for PR #522 with `VERDICT: pass` in the review body. Instead of calling `gh pr merge`, mika-dev misclassified the event as `pull_request.opened`, fabricated a task_id, and re-dispatched `run_claude_pilot` for an unrelated issue. PR #522 sat unmerged for ~7 hours until manually merged.

Root cause: the verdict-to-merge decision was left to LLM interpretation of raw webhook text. The LLM never parsed `VERDICT: pass`, never checked PR state, and improvised a completely wrong action. This is a state-machine transition, not a judgement call.

See also: `docs/solutions/agent-quality/2026-04-11-mika-dev-verdict-misclassification-pr-522.md` for the incident post-mortem.

## Guidance

**When an external event should trigger a deterministic action, handle it structurally in the engine layer before the LLM turn — not via prompt instructions.**

The verdict handler (`server::verdict_handler`) intercepts `pull_request_review.submitted` webhook events in `handle_message()`, after the agent lock is acquired but before `run_agent()` is called:

1. **Parse** the gateway-formatted text using `parse_pr_review_event()` (regex match on `[GitHub] PR review ({state}) on {repo}#{number}`)
2. **Extract verdict** from the review body using `parse_verdict()` — case-insensitive match on `VERDICT:` at start of line
3. **Act on verdict:**
   - `pass` → look up task by `metadata.claude_pilot.pr_url`, check status is `in_progress`, then call `run_gh_checks` + `classify_checks` + `run_gh_merge` (reused from `pr_merge_with_gate`)
   - `block[ac]` → look up task, dispatch claude-pilot with AC-fix prompt, bounded retry counter (max 3), escalate on limit (#889)
   - `block[ci]` → look up task, dispatch claude-pilot with CI-fix prompt, bounded retry counter (max 3), escalate on limit (#889)
   - `block[security]` / `block[pipeline]` → mark task blocked, notify operator, NO auto-dispatch (#889)
   - `hold[review]` → notify operator, leave task in_progress (#889)
   - Missing → safe-default hold[review] semantics + `verdict_classification_failed` structured log event (#889)
4. **Pre-digest** the result for the LLM as a fait accompli — the LLM receives "merge initiated" rather than the raw review text

Key design decisions:
- **Reuse `pr_merge_with_gate` internals** (`run_gh_checks`, `classify_checks`, `run_gh_merge`) — made `pub(crate)` for shared access. Same CI gate classification logic applies.
- **No new task statuses** — gates on existing `in_progress` + `metadata.claude_pilot.pr_url` presence. Tracks merge state in `metadata.verdict_merge` to avoid schema migration.
- **Pre-digest avoids completion-claim guard trigger words** — uses "merge initiated" / "auto-merge enabled" instead of "merged" / "completed" since the engine action happens outside the LLM's tool calls.
- **60-second timeout** on subprocess calls to prevent agent lock starvation.

## Why This Matters

- **LLMs improvise on state-machine transitions.** When the correct action is deterministic (approved + VERDICT: pass → merge), leaving it to the LLM introduces unnecessary failure modes: misclassification, hallucinated task IDs, re-dispatch of unrelated work.
- **Pre-digestion eliminates LLM step waste.** Without the handler, the LLM burns 10+ tool steps discovering the PR state, parsing the verdict, and deciding to merge. The handler does it in one structural pass.
- **Prompt-only enforcement is unreliable after compaction.** The system prompt can instruct "parse VERDICT: and merge" but the LLM ignores instructions after context compaction. Code-level handlers survive compaction.

## When to Apply

Use this pattern when:
- An external event (webhook, callback, timer) should trigger a deterministic action
- The action has a clear state-machine transition (status X + event Y → action Z)
- The LLM interpretation of the raw event has historically been unreliable
- The action is time-sensitive (PR merges should happen immediately, not after LLM deliberation)

Do NOT use this pattern when:
- The action requires judgement (which retry strategy? what error message to send?)
- The event is ambiguous and benefits from LLM reasoning
- The action needs access to conversation history or memory context

## Examples

**Before (LLM-driven, unreliable):**
The LLM receives raw webhook text `[GitHub] PR review (approved) on repo#522 by @mika-qa` and must independently: (1) recognize this is an approval, (2) find `VERDICT: pass` in the body, (3) look up the task, (4) call `pr_merge_with_gate`. Any step can fail or be improvised incorrectly.

**After (structural handler):**
The engine intercepts the event, parses the verdict, looks up the task, and calls merge before the LLM sees the message. The LLM receives a pre-digested `<verdict_handler>` block describing what the engine did.

## Related

- `docs/solutions/architecture-patterns/ci-gate-tool-structural-backstop-for-pr-merges.md` — the PR merge gate tool that the verdict handler reuses
- `docs/solutions/architecture-patterns/engine-level-callback-metadata-extraction.md` — same pattern of engine-level extraction before LLM turn
- `docs/solutions/architecture-patterns/merge-two-step-llm-tool-contracts.md` — anti-pattern the handler avoids (atomic, not two-step)
- `docs/solutions/architecture-patterns/deterministic-skill-context-injection.md` — related pattern of engine-owned pre-fetch
- mika#524 — implementation issue
- mika#525 — companion: tool-level refusal for `run_claude_pilot` on invalid task states

## Companion: CI Success Handler (`check_suite.completed/success`)

Added in #571. When `verdict_handler` fires on a `pull_request_review.submitted` event and CI is still pending, it enables `--auto` merge via GitHub. However, if a force-push (e.g., CI fix) lands after `--auto` was set, GitHub's native auto-merge is cancelled. The `ci_success_handler` fills this gap.

**Trigger:** `check_suite.completed` with conclusion `success`, routed to `mika-dev` by the gateway.

**Logic:**
1. Parse `[GitHub] Check suite success on {repo} (branch: {branch})` from gateway-formatted text
2. `gh pr list` — find open PR for the branch (`NoPr` if none — self-terminates post-merge webhooks)
3. `gh api` — find APPROVED review with `VERDICT: pass` in body
4. **Stale-SHA gate (strict):** `review.commit_id == pr.head.sha`. If QA approved a different SHA than the current HEAD, do NOT merge. A push after approval — even a mechanical CI fix — is unreviewed code. The cost is one extra QA cycle; the alternative is silently trusting that the push was safe.
5. **CI aggregation (load-bearing, not defensive):** `run_gh_checks` + `classify_checks` must return `AllPassed`. The `check_suite.completed/success` webhook is scoped to ONE workflow — a PR with multiple required workflows can fire this event while another is still pending or already failed. The aggregation IS the gate, not a belt-and-suspenders re-check.
6. `run_gh_merge` — squash merge with branch deletion. Maps `already merged` / `Pull request is closed` to `AlreadyMerged` (info log, not error).

**Key design decisions:**
- Same location (`mika-agent/src/server/`), same return type (`VerdictAction`), same `pr_merge_with_gate` helper reuse
- Order-independent with `verdict_handler` — each handler self-selects on event type, returns `Passthrough` for non-matching events
- No LLM involvement — structural state-machine transition
- Pre-digest uses `<ci_success_handler>` XML tag with completion-claim-safe phrasing
