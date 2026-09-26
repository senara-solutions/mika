---
title: "qa-review docs-only block fabricates Pipeline-Exempt escape that doesn't exist"
date: 2026-05-10
category: prompt-engineering
module: qa-review
problem_type: logic_error
component: tooling
symptoms:
  - "mika-qa emits VERDICT: block[pipeline] on docs-only PRs citing 'Pipeline-Exempt label' escape that isn't implemented"
  - "Adding pipeline-exempt label to PR does not change the verdict — still blocked"
  - "Autonomous loop fires correctly on ready label but verdict has no honored escape for legitimate docs-only PRs"
root_cause: missing_workflow_step
resolution_type: documentation_update
severity: medium
tags:
  - qa-review
  - pipeline-exempt
  - docs-only
  - fabrication
  - skill-prompt
  - label-drift
---

# qa-review docs-only block fabricates Pipeline-Exempt escape that doesn't exist

## Problem

mika-qa's verdict on docs-only PRs cites a `pipeline-exempt` label escape mechanism that the skill prompt doesn't implement. Operators who follow the fabricated advice (adding the label) still get `block[pipeline]`. Three docs-only PRs (mika#1062, mika#1063, mika-platform#99) were stuck at REVIEW_REQUIRED with no autonomous path to approval.

## Symptoms

- `VERDICT: block[pipeline]` with reason mentioning "Pipeline-Exempt label" on docs-only PRs
- Toggling `ready` label off+on with `pipeline-exempt` already present produces the same block verdict
- `gh pr view --json labels` confirms the label IS present, but the verdict ignores it

## What Didn't Work

- Adding `pipeline-exempt` label to the PR — the skill prompt's Step 2 check 2 is a hardcoded structural block with no label check
- Toggling `ready` label to force re-review — same result because the gate code doesn't honor any label
- The model's suggested workaround ("add Pipeline-Exempt label") is fabricated from the label's GitHub description text, not from any implemented logic

## Solution

Two coupled fixes:

1. **Added `pipeline-exempt` label gate to qa-review skill prompt** — inserted at the top of Step 2, before check 1. The gate has two conditions (both must be true):
   - PR labels contain `pipeline-exempt` (exact lowercase-kebab match)
   - The diff contains only documentation files (reuses the same `grep -v` pattern from check 2, plus `.github/` exclusion)
   
   When both conditions are met, Steps 2.1–2.3 and Step 2.5 (plan-AC verification) are skipped. The review proceeds directly to Step 3 (diff review) for security checks only. A source-change guard prevents misapplied labels from bypassing pipeline checks on code PRs.

2. **Canonicalized `pipeline-exempt` label in `.github/labels.yml`** — the label existed on the repo as drift (not in the canonical taxonomy). Added it under the State section so EndBug/label-sync recreates it on wipe.

3. **Updated pre-termination self-check invariant 3** — the invariant only accepted two states (PLAN-AC VERIFICATION with AC bullets, or block[pipeline] from Step 2.5). Added a third accepted state for the pipeline-exempt bypass path to prevent the LLM from stalling or downgrading the verdict.

Key files changed:
- `skills/bundled/qa-review/system_prompt.md` — pipeline-exempt gate + verdict template + invariant 3
- `.github/labels.yml` — canonicalized label

## Why This Works

The root cause was two coupled defects:

1. **Label drift** — `pipeline-exempt` existed on GitHub but not in `.github/labels.yml`. Origin unknown (likely created manually with intent that the implementation didn't follow through).
2. **No escape mechanism** — Step 2 check 2 is a hardcoded `block[pipeline]` for all docs-only PRs with zero conditional paths.

The LLM read the orphaned label's description text ("Docs-only or non-code PR exempt from pipeline gates") and inferred an escape route that doesn't exist — a fabrication from environmental signal rather than implemented logic. This is the same class as `feedback_mika_dev_llm_fabricates_tool_errors`: an LLM offering a workaround that doesn't actually work.

The fix makes the escape structural: the label check is in the prompt text, the source-change guard prevents misuse, and the label is canonicalized so it survives label-sync wipes.

## Prevention

- **Structural gates need structural escapes.** When adding a hardcoded block to a skill prompt, also implement the escape mechanism if one should exist. Don't leave it to the model to infer.
- **Canonicalize labels before referencing them.** Any label used in automated logic must be in `.github/labels.yml`. Orphaned labels invite fabrication.
- **Source-change guards on label bypasses.** The `pipeline-exempt` gate confirms the diff is docs-only before honoring the label, preventing misapplied labels from skipping plan-AC verification on code PRs.

## Related Issues

- mika#1064 — this ticket
- mika#1062, mika#1063, mika-platform#99 — the stuck docs-only PRs
- mika#861 — prior label-driven CI exemption for docs-only PRs
- `docs/solutions/best-practices/verify-which-script-ci-actually-invokes-2026-04-28.md` — same class: fabricated escape mechanism
- `docs/solutions/best-practices/prompt-rule-cheapness-bias-toward-wrong-layer-2026-04-28.md` — established label-driven checks as the structural correlate
- `docs/solutions/best-practices/citation-fabrication-prompt-anchoring-2026-05-02.md` — fabrication family pattern
