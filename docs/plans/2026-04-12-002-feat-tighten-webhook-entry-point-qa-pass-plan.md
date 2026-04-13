---
title: "feat: tighten webhook entry point — QA pass + open PR = immediate merge"
type: feat
status: completed
date: 2026-04-12
---

# feat: tighten webhook entry point — QA pass + open PR = immediate merge

## Overview

When a `pull_request_review.submitted` webhook arrives with verdict `pass` and the PR is open, the agent should immediately call `pr_merge_with_gate` without narrating, questioning, or waiting for confirmation. The PR #522 incident (2026-04-11) showed that despite having the correct decision tree in the prompt, the LLM misclassified the event and narrated instead of acting, leaving the PR stuck for 7 hours.

This issue tightens the prompt-level directives as an intermediate fix. The structural runtime handler (mika#524) is a separate, deeper fix.

## Problem Statement

Three gaps in the current `self-dev-webhook-qa/system_prompt.md` allowed the PR #522 failure:

1. **No explicit "zero narration" directive.** The prompt says "DO NOT end your turn without acting" and "MUST call pr_merge_with_gate" — but never says "DO NOT narrate, explain, or ask questions before acting." The LLM fills silence with narration.
2. **Work item correlation is a prerequisite for merge.** Step 1 requires correlating to a work item before parsing the verdict. If correlation fails or confuses the LLM, the merge never happens. The PR URL is in the webhook — merge should not depend on work item lookup.
3. **Event type identity is implicit.** The prompt assumes the LLM correctly identifies `pull_request_review.submitted` from the message text. PR #522 showed the LLM misread it as `pull_request.opened`. The prompt needs an explicit event-type fingerprint check.

## Proposed Solution

### Change 1: Tighten `self-dev-webhook-qa/system_prompt.md`

**File:** `mika-skills/self-dev-webhook-qa/system_prompt.md`

Add hard directives at the top of the pass verdict section:

1. **Zero-narration rule:** Add explicit directive: "On a `pass` verdict, your FIRST output MUST be a tool call. No text, no explanation, no questions before the tool call. Narration before action is a workflow failure."
2. **Decouple merge from work item correlation:** Restructure the pass flow to:
   - Step 1: Parse `VERDICT:` line from review body
   - Step 2: If `pass` → extract PR number and repo from PR URL → call `pr_merge_with_gate` immediately
   - Step 3: AFTER merge result, correlate to work item and update status
   - This ensures merge happens even if work item lookup fails or confuses the model
3. **Event-type fingerprint:** Add an explicit check at the top: "This event contains a review body with a `VERDICT:` line. This is a QA verdict event — NOT a new PR, NOT a CI event, NOT a comment. Your only job is to parse the verdict and act on it."
4. **Simplify the pass case preamble:** Remove multi-step extraction instructions that add cognitive load. The PR URL is right there in the message — make the instruction direct: "Extract pr_number and repo from the PR URL in the message, then call pr_merge_with_gate."

### Change 2: Reinforce `soul.md` (minor)

**File:** `~/.mika/agents/mika-dev/soul.md`

The "Evidence → Action" principle at lines 54-62 is already well-stated. Minor reinforcement:
- Add: "On webhook events with clear verdicts, act first — correlate and update state after."
- This aligns soul.md with the reordered flow (merge first, correlate second).

## Acceptance Criteria

- [x] `self-dev-webhook-qa/system_prompt.md` — pass verdict section has zero-narration directive
- [x] `self-dev-webhook-qa/system_prompt.md` — pass flow reordered: parse verdict → merge → correlate work item (not: correlate → parse → merge)
- [x] `self-dev-webhook-qa/system_prompt.md` — event-type fingerprint check at top prevents misclassification
- [x] `soul.md` — "Evidence → Action" section reinforced with "act first, correlate after" on webhook events
- [x] No narration or clarifying questions on QA pass events

## Files to Modify

| File | Change |
|------|--------|
| `mika-skills/self-dev-webhook-qa/system_prompt.md` | Tighten pass verdict flow: zero-narration, decouple merge from work item, event fingerprint |
| `~/.mika/agents/mika-dev/soul.md` | Minor reinforcement of Evidence → Action for webhook events |

## Sources

- **Incident:** [mika-dev verdict misclassification on PR #522](../solutions/agent-quality/2026-04-11-mika-dev-verdict-misclassification-pr-522.md) — LLM misclassified `pull_request_review.submitted` as `pull_request.opened`, narrated instead of merging
- **Structural fix (separate):** mika#524 — hard-wired verdict→merge handler in agent runtime
- **Related patterns:** [CI gate tool as structural backstop](../solutions/architecture-patterns/ci-gate-tool-structural-backstop-for-pr-merges.md), [grounding rule](../solutions/prompt-engineering/grounding-rule-downstream-state-hallucination.md)
- Issue: #553
