---
title: Recovery shim fallback writes wrong-plan body callout (Class C drift)
module: dev-groom
date: 2026-05-16
problem_type: workflow_issue
component: development_workflow
severity: medium
tags:
  - mika-platform
  - dev-groom
  - body-callout
  - recovery-shim
  - dispatch
related_components:
  - claude-pilot
  - dev-groom
  - dispatch-lib
applies_when:
  - "A grooming session ends without writing its own plan callout to the issue body"
  - "Post-flight recovery shim runs against a worktree branched from main"
  - "Prior PRs have left plan files in docs/plans/ matching the glob *-plan.md"
---

# Recovery shim fallback writes wrong-plan body callout (Class C drift)

## Symptom

A grooming session completes but does not write the canonical `## Plan` callout to the GitHub issue body before exit. The post-flight recovery shim (`_shared/dispatch-lib.sh`) detects the missing callout and tries to patch it. The shim cannot find a plan file scoped to the current issue number, falls through to a fallback that picks "any plan file in `docs/plans/`," and writes a callout whose path is **completely unrelated to the current issue**.

Reader of the issue follows the callout link, reads someone else's already-merged plan, and acts on stale context.

## Mechanism

`_shared/dispatch-lib.sh:185-196` (current shape on `main`):

```sh
# Try issue-scoped plan first
plan_path=$(find docs/plans -name "*-${issue_num}-*-plan.md" 2>/dev/null | head -1)

# Fallback: any plan in docs/plans/, lexicographically last
if [ -z "$plan_path" ]; then
    plan_path=$(find docs/plans -name "*-plan.md" 2>/dev/null | sort -r | head -1)
fi
```

When a worktree is branched fresh from `main` and the grooming session aborts before writing its plan file (or writes it under a different name), the fallback's `sort -r | head -1` returns the **lexicographically largest filename** across every plan ever merged. Recent plans dominate by date prefix (`2026-05-15-…` ranks above `2026-04-30-…`), so the callout reliably points at "the most recent merged plan" — a deterministic wrong answer.

## Canonical instance (2026-05-16)

mika#1142 grooming session. dev-groom recursed on a bug-report ticket about pilot drift, never wrote its own plan callout, exited. The post-flight recovery shim then ran:

1. `find docs/plans -name '*-1142-*-plan.md'` → empty (no plan written for #1142).
2. Fallback fired: `find docs/plans -name '*-plan.md' | sort -r | head -1` → `2026-05-15-923-fix-shared-dir-install-plan.md` (PR #923's plan, merged prior day).
3. Shim wrote a `## Plan` callout to mika#1142's body pointing at #923's plan path.

Body callout claimed mika#1142 was "ready, see plan at `docs/plans/2026-05-15-923-fix-shared-dir-install-plan.md`." That plan is about a completely unrelated shared-dir install fix.

## How to identify Class C drift

Class C is the third member of the body-callout drift family (see `feedback_body_callout_drift_two_classes` in operator memory). Identification recipe:

| Signal | Class A (none) | Class B (verdict-extra) | Class C (wrong-plan-path) |
|--------|---------------|-------------------------|----------------------------|
| Has any `## Plan` callout in body? | No | Yes | Yes |
| Grooming history line | absent | present | **present, and mentions "recovered by post-flight"** |
| Plan path in callout | n/a | matches issue number | **does NOT include the current issue number** |
| Disposition | dispatch dev-groom | perl-patch fix the callout | **strip callout + re-groom** |

The two Class C tells are conjunctive:
- Grooming history line says **"recovered by post-flight (mika#1123)"** or similar shim attribution, AND
- The `## Plan` callout's path filename does NOT contain `-<current-issue-num>-`.

Either tell alone can occur in Class A/B; only the conjunction is diagnostic for Class C.

## Fix (in-flight)

mika#1144 (filed 2026-05-16). Recommended fix: **strip the fallback entirely.** If the issue-scoped `*-<issue_num>-*-plan.md` lookup returns empty, the shim should NOT write a callout. A missing callout is more recoverable than a wrong one — the next operator pass detects "no callout" cleanly (Class A) and dispatches dev-groom for a real plan. A wrong callout looks valid and silently routes the next reader into a stale plan.

Alternative considered and rejected: have the shim leave a sentinel placeholder ("callout missing, run dev-groom again"). The shim already has no canonical contract for partial state; adding one is more surface than removing the fallback.

## Related

- Memory: `feedback_body_callout_drift_two_classes` (extended to 3 classes 2026-05-16).
- Doc: `docs/solutions/workflow-issues/dev-groom-zero-artifact-exit-2026-05-13.md` (sibling failure mode where dev-groom exits without writing anything — Class A trigger).
- Ticket: mika#1144 (fix in flight).
