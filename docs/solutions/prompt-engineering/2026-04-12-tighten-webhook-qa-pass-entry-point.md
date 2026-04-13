---
title: "Tighten webhook QA pass entry point — merge before correlate"
category: prompt-engineering
date: 2026-04-12
tags: [webhook, qa-verdict, pr-merge, self-dev-webhook-qa, evidence-action, zero-narration, mika-dev]
repos: [mika-skills]
---

# Tighten webhook QA pass entry point — merge before correlate

## Problem

mika-dev received a `pull_request_review.submitted` webhook with `VERDICT: pass` visible in context but misclassified it as `pull_request.opened`, re-dispatched claude-pilot instead of merging, and left the PR stuck for 7 hours (incident: PR #522, 2026-04-11).

Three gaps in the `self-dev-webhook-qa/system_prompt.md` allowed this:

1. **No event-type fingerprint.** The prompt assumed the LLM would correctly identify `pull_request_review.submitted` from message text. The LLM misread it as `pull_request.opened`.
2. **Work item correlation before action.** Step 1 was "correlate to work item" — if that step confused the LLM, merge never happened. The PR URL was already in the webhook; merge should not depend on work item lookup.
3. **No zero-narration directive.** The prompt said "DO NOT end your turn without acting" but never prohibited narration *before* acting. The LLM filled silence with text instead of making a tool call.

## Solution

Three changes to `mika-skills/self-dev-webhook-qa/system_prompt.md`:

### 1. Event identity check at the top

Added explicit fingerprint block that names what the event IS and what it is NOT:

> **EVENT IDENTITY CHECK:** This message contains a PR review body with a `VERDICT:` line posted by mika-qa. This is a **QA verdict event** — NOT a new PR (`pull_request.opened`), NOT a CI event (`check_suite`), NOT an informational comment.

### 2. Reordered flow: parse → merge → correlate

Old order: correlate work item (Step 1) → parse verdict (Step 2) → act (Step 3)

New order: parse verdict (Step 1) → extract PR coords (Step 2) → act/merge (Step 3) → correlate work item (Step 4) → update status (Step 5)

The `pass` case now calls `pr_merge_with_gate` before any work item lookup. If correlation fails later, the merge already succeeded. Step 4 says: "If no work item found, skip work item updates — the merge/action in Step 3 already succeeded."

### 3. Zero-narration rule on pass verdict

> **ZERO-NARRATION RULE: On a `pass` verdict, your FIRST output MUST be a `pr_merge_with_gate` tool call. No text, no explanation, no questions, no status checks before the tool call. Narration before action is a workflow failure. Evidence → Action.**

### 4. soul.md reinforcement (minor)

Added to the "Evidence → Action" section:
- "On webhook events with clear verdicts: act first, correlate and update state after."
- "On a QA pass verdict, your first output is a tool call — not text."

## Prevention

- **When an external event should trigger a deterministic action, put the action first.** Correlation, state updates, and notifications are follow-up work that should not gate the primary action.
- **Event identity checks prevent misclassification.** Explicitly naming what an event IS and IS NOT reduces the chance of the LLM pattern-matching to the wrong event type.
- **Zero-narration directives close the "narrate instead of act" failure mode.** "DO NOT end without acting" is weaker than "your FIRST output MUST be a tool call."
- **This is a prompt-level fix.** The structural runtime handler (mika#524) is the deeper solution — it moves verdict→merge out of the LLM entirely. This prompt fix reduces failure probability; the structural fix eliminates it.

## Related

- [mika-dev verdict misclassification on PR #522](../agent-quality/2026-04-11-mika-dev-verdict-misclassification-pr-522.md) — the incident that motivated this fix
- [CI gate tool as structural backstop](../architecture-patterns/ci-gate-tool-structural-backstop-for-pr-merges.md) — the `pr_merge_with_gate` tool
- [Grounding rule](grounding-rule-downstream-state-hallucination.md) — related principle about evidence-based claims
- mika#524 — structural verdict→merge handler (separate, deeper fix)
- mika#553 — this issue
