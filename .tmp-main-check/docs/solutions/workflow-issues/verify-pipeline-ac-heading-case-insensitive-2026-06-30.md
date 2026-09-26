---
title: Structural gates over LLM-authored plan markdown must case-fold the heading match
tags:
  - mika-platform
  - workflow
  - ci
  - verify-pipeline
  - plan-quality
  - dev-groom
module: scripts/verify-pipeline.sh
problem_type: workflow_issue
category: workflow-issues
severity: medium
created: 2026-06-30
---

# Structural gates over LLM-authored plan markdown must case-fold the heading match

## Symptom

The mika#1600 Pipeline Artifacts CI gate fails a PR with:

```
FAIL: Plan 'docs/plans/....md' missing '## Acceptance criteria' section. See mika#1600.
```

— **even though the plan has a complete, non-empty Acceptance-criteria section** — when
the plan writes the heading in title case (`## Acceptance Criteria`, capital C) instead
of the sentence case the gate matched literally (`## Acceptance criteria`).

## Root cause

`scripts/verify-pipeline.sh` matched the heading **case-sensitively** at two sites:

- presence: `grep -q '^## Acceptance criteria'`
- non-empty-content: `sed -n '/^## Acceptance criteria/,/^## /{ ... }'`

The autonomous-loop groom command instructs lowercase, but heading text is
**LLM-authored natural language**, and the model frequently title-cases the noun
phrase out of habit. The lowercase instruction is a prompt, and prompt enforcement is
fragile (`feedback_prompt_enforcement_fragile`): ~50% of autonomous-loop plans came out
title-cased and hit this gate. Four PRs were hand-fixed in a single day
(#1623 commit 2bd622ea, #1626 c9643057, #1628 8a02bb2a, #1638 fe3a5b5c) — each a
one-character lowercase edit to the heading. The case-sensitivity added zero value: no
realistic plan misses the AC concept by writing it in title case.

## Fix (mika#1639 secondary half)

Fold case at both matchers — control over the structural gate, not another prompt rule:

- `grep -q` -> `grep -qi`
- sed start-address gets the GNU `I` flag: `/^## Acceptance criteria/I,/^## /{ ... }`.
  Only the **start** address needs it; the end address `/^## /` and the inner `/^## /d`
  already match any `## ` heading prefix case-agnostically.

Both GNU flags are available on the CI runner (GitHub Actions Ubuntu) and dev hosts
(Gentoo). Test it **behaviorally** through the script, not by re-encoding the regex:
`scripts/verify-pipeline-test.sh` adds a title-case-heading -> PASS case alongside the
existing present/missing/empty cases (run `bash scripts/verify-pipeline-test.sh`).

## The recurring class (n=2): structural matchers over LLM-authored plan markdown

This is the **same class** as
[`find-issue-plan-header-shape-widening-2026-06-27.md`](find-issue-plan-header-shape-widening-2026-06-27.md):
a structural matcher over plan markdown the pilot/groom *writes* was too literal, so a
legitimate plan went invisible/rejected. There the variation axis was header **shape**
(`**Ticket:**` vs `**Issue:**`); here it is **case**. Both recur because the matched
text is natural language the model generates, not a pinned token.

- **n=1 — header shape** (mika#1381/#771/#1600 -> fix #1602): widen the `_find_issue_plan`
  content-fallback alternation in `dispatch-lib.sh`.
- **n=2 — heading case** (mika#1639): case-fold the `verify-pipeline.sh` AC-heading match.

**Rule of thumb:** when a gate or discovery function matches a heading/label/marker that
an LLM authors, make the match tolerant of the natural-language variation the model will
produce (case, surrounding punctuation, the obvious synonym), and pin the behavior with a
fixture. Fix the matcher (control), not the prompt (documentation) — the prompt is the
wrong layer for a structural invariant.

## When NOT to over-widen

Tolerance is for *cosmetic* natural-language variation, not for accepting a malformed
artifact. The case-fold here still requires the literal heading text and a non-empty body
— a plan with no AC section, or an empty one, still FAILs (covered by the existing test
cases). Don't relax a gate past the point where it still proves the thing it exists to
prove.
