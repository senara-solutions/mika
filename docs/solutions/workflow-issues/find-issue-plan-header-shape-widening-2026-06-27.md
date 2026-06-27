---
title: _find_issue_plan content-fallback regex must accept every header shape the pilot legitimately writes
tags:
  - mika-platform
  - dispatch
  - workflow
  - claude-pilot
  - dev-groom
module: skills/bundled/_shared/dispatch-lib.sh
problem_type: workflow_issue
category: dispatch
severity: medium
created: 2026-06-27
---

# `_find_issue_plan` content-fallback regex must accept every header shape the pilot legitimately writes

## Symptom

An autonomous dev-groom dispatch fails with:

```
PIPELINE FAILURE: dev-groom produced no valid plan file (no issue-scoped plan >500 bytes
found via _find_issue_plan for mika#NNNN). Session likely drifted into executor mode.
```

— **even though the pilot wrote a complete, well-structured plan and committed it to the branch.** The "executor mode" diagnostic is misleading: the session did not drift; the plan exists on the branch but is invisible to the discovery function, so `_iterate_groom_loop` returns 1 and the architect verdict step never runs.

## Root cause

`_find_issue_plan` (in `skills/bundled/_shared/dispatch-lib.sh`) discovers the plan-on-branch in two passes:

1. **Primary (filename):** `find ... -name "*-${ISSUE_NUM}-*-plan.md"` — requires the issue number embedded in the filename (e.g. `-1600-`). Misses when the plan filename is `YYYY-MM-DD-NNN-<slug>-plan.md` where `-NNN-` is the daily counter, not the issue number.
2. **Fallback (content):** greps the first 20 lines of each `docs/plans/*-plan.md` for a header referencing the issue.

The content-fallback regex was a **closed union of the exact header shapes observed so far**. When a pilot writes a header shape outside that union, *both* passes miss. This recurs because "what header shape does the pilot write" is not pinned — `**Ticket:**`, `**Issue:**`, `ticket:`, and `issue:` are all reasonable and all appear in practice ("Issue" matches GitHub's own UI).

### The recurrence (one class, bound at n=3)

- **n=1, mika#1381** (2026-06-06) — plan filename had no `-${ISSUE_NUM}-` token. Founding incident for the content-fallback itself.
- **n=2, mika#771** (2026-06-06) — same filename-shape gap. Bound the class; added the content-fallback (mika#1421).
- **n=3, mika#1600** (2026-06-27) — filename had no `-1600-` token **AND** the header was `**Issue:** mika#1600` rather than `**Ticket:**`. Both passes missed. Fixed by mika#1602.

## Fix

Widen the content-fallback alternation additively — never replace a shape, only add. After mika#1602 the regex is:

```
^(\*\*Ticket:\*\*|\*\*Issue:\*\*|ticket:|issue:)\s+mika[[:space:]]?(issue)?#${ISSUE_NUM}\b
```

The shared tail (`\s+mika[[:space:]]?(issue)?#${ISSUE_NUM}\b`) and the first-20-lines header-zone scope are unchanged — the `^` anchor + header-zone keep body-prose quotes of *other* tickets' headers from false-matching (the failure mode mika#1421's v1 self-test hit), and the `\b` after `${ISSUE_NUM}` prevents prefix collision (`ISSUE_NUM=160` does not match `#1600`).

Test it **behaviorally**, not by re-encoding the regex: source `dispatch-lib.sh` (it is side-effect-free — function definitions only) and call `_find_issue_plan` against temp `docs/plans/` fixtures with the target header on an early line and a deliberately unrelated filename slug. See `test-dispatch-lib.sh` Test 16. Re-encoding the regex in the test would let a regex typo pass both.

## When to stop widening (Tier-2 escalation, n≥5)

Iterative additive widening is the correct remediation **while the header-shape class is small and bounded (n<5)** — cost is ~30 min including grooming, and each shape is a one-line union extension plus a fixture. If the class reaches **n≥5 distinct shapes**, the diminishing-returns point is crossed: file a design ticket to replace the fixed union with a general heuristic (scan the first 20 lines for any line containing `mika#${ISSUE_NUM}` regardless of prefix), accepting the body-quote false-positive risk with a compensating guard. The n≥5 threshold is an operator judgment call — named here so the pattern is observable, not so it auto-fires.

## Why not fix the pilot prompt instead

Standardizing the pilot's plan header via prompt prose is fragile (`feedback_prompt_enforcement_fragile`) and was explicitly rejected in mika#1602's scope. Widening the structural discovery regex is the load-bearing fix; the prompt is documentation, the regex is control.

## Deployment note

`dispatch-lib.sh` is copy-deployed to `~/.mika/agents/<role>/skills/_shared/` and re-seeded from the source tree by `seed_bundled_skills_if_needed()` on startup (and by `make deploy`). A source-tree edit + deploy is sufficient; main-merged is **not** live until deployed.
