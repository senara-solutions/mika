---
title: A completion-gate that sets a global PR_URL shadows the correct rescue path
module: skills/bundled/_shared/dispatch-lib.sh
date: 2026-07-01
problem_type: bug
category: workflow-issues
component: development_workflow
severity: high
ticket: mika#1679
resolution_type: dead_path_removal
tags:
  - dispatch-lib
  - rescue-pr
  - shadowing
  - single-source-of-truth
  - recovery-guards
  - autonomous-loop
related_components:
  - dev-pilot
  - self-dev-webhook-qa
  - qa-review
applies_when:
  - "Two code paths fire on the same trigger and the first sets a global the second is gated on being empty"
  - "A rescue/recovery PR opens with the wrong shape (non-draft, missing marker) despite a correct handler existing"
  - "A defense-in-depth marker commit must reach the remote for a downstream guard to arm"
---

# A completion-gate that sets a global PR_URL shadows the correct rescue path (mika#1679)

## Problem

`dispatch-lib.sh` had **two** PR-creation paths firing on the *same* trigger — "the dev-pilot committed and pushed but ended its turn before opening a PR." The first path pre-empted the second by setting a shared global, so the second (correct) path never ran. The visible symptom: rescue PRs opened **non-draft with no recovery marker**, bypassing the mika#1613 recovery guards and becoming eligible for autonomous un-draft + auto-merge without operator review (evidence: mika#PR1678, mika#PR1683).

## Symptoms

- A "commit-pushed-no-pr" rescue PR opens with `isDraft: false`, no `## Auto-rescued PR (dispatch-lib recovery, class: ...)` body header, and the RESULT line `auto-created PR ...` (not `Draft PR (dispatch-lib recovery): ...`).
- `RECOVERY_PENDING: true` is absent from the callback RESULT, so `tasks.metadata.unpushed_recovery_pending` is never set and the qa-webhook recovery-skip guard (Guard 1) never fires.
- qa-review approves the PR (Step 1.5 finds no rescue header → treats it as a normal PR).

## Root cause

Two paths, identical trigger condition, in the same dispatch flow:

- **Path A** — the mika#1383 "structural completion gate" inside `_post_flight_recovery()`. Guard: `SKILL=dev-pilot && POST_RUN_HEAD set && PRE_RUN_HEAD != POST_RUN_HEAD && WORKTREE_DIR && BRANCH`. It opened a **non-draft** PR with a plain body, then a later re-query block (`Issue #138: Discover actual PR URL`) found that PR and **set the global `PR_URL`**.
- **Path B** — the mika#1396 `commit-pushed-no-pr` rescue inside `dispatch_claude_pilot()`, guarded by `[ -z "$PR_URL" ]`. It already did the fully-correct thing: `--draft` + rescue header + `RECOVERY_PENDING: true` + `wip-rescue` label + canonical `PR:` line + a class-specific PR title.

Because Path A ran first (inside `_run_claude_pilot` → `_post_flight_recovery`) and set the global `PR_URL`, Path B's `[ -z "$PR_URL" ]` guard was false on every commit-pushed-no-pr run. **Path B's `commit-pushed-no-pr` branch was effectively dead code** — it only survived as a fallback when Path A's own `gh pr create` errored. The grooming that produced the original fix plan referenced mika#1282/#1383 but not mika#1396, so the shadowing was invisible at plan time.

## What didn't work

The first (groomed) plan proposed adding four coordinated edits to **Path A** to make *it* emit `--draft` + the marker + the header + a `wip()` commit. That would have worked functionally but **institutionalized a two-implementations-one-contract divergence trap**: two ~40-line rescue blocks producing the same PR shape, one of them shadowed dead code, on a contract that had churned four times in one week (draft, marker, header, label added across mika#1282/#1396/#1613/#1618). The next contract change would update one path and not the other.

## Solution

Delete Path A's PR-creation entirely; let the already-correct Path B own it (single source of truth).

- **Keep** Path A's *Phase A* (trailing-dirty-content rescue) — it commits leftover dirty content with a `wip()` prefix and advances `POST_RUN_HEAD` so Path B sees the latest commits.
- **Delete** Path A's *Phase B* (the `gh pr list` existence check + `gh pr create` + result append). With no PR created there, the `Issue #138` re-query returns empty, `PR_URL` stays empty, and Path B's `[ -z "$PR_URL" ]` guard fires.
- **Add** an empty `wip(mika#1383)` marker commit to Path B's `commit-pushed-no-pr` branch (before `gh pr create`, scoped to that class only) so the PR head-commit headline matches Guard 2's `^wip\(` regex — the pilot's own head commit is a conventional `fix(/feat(` subject. The dirty-worktree class is already `wip()`-prefixed by its mika#1282 rescue, so it is excluded.

The "PR already exists" case (re-dispatch) stays safe: the `Issue #138` re-query finds the existing PR → `PR_URL` set → Path B no-ops → no double-create.

## Why this works

`PR_URL` is the single arbiter of "does a rescue PR already exist." When only one path can create the PR, the global cleanly serializes the decision. The rescue contract now lives in exactly one block, so the next contract change has one edit site.

## Prevention

Three reusable diagnostics, in order of how often they bite:

1. **When an early "completion gate" sets a global, grep for *other* consumers gated on that global being empty.** A path that sets `X` shadows every later path guarded by `[ -z "$X" ]`. The shadowed path can be more correct than the one that wins. The fix is usually to stop the early path from setting the global, not to duplicate the late path's behavior.

2. **"A shadows B" is NOT the same claim as "B runs when A is deleted."** Before deleting the shadowing path, *verify reachability* of the path you're deferring to — trace every early-return between them. Here, the only early-return between `_post_flight_recovery` and Path B's guard was `_check_pilot_force_push`, which returns `0` unconditionally for `dev-pilot` (`[ "$SKILL" = "dev-groom" ] || return 0`), so Path B was reachable. Skipping this check risks converting a fail-quiet bug (wrong-shaped PR) into a fail-silent bug (no PR at all).

3. **A defense-in-depth marker that depends on a push must surface push failure, not swallow it.** `gh pr create --head "$BRANCH"` opens the PR from the *origin* branch, so a marker commit whose push fails silently (`|| true`) yields a PR head that doesn't match the guard regex — the guard never arms and nothing says so. Emit an observable signal (`rescue_marker_push.failed` to stderr, matching the file's `pilot_push_guard.*` convention) so operator/telemetry sees the belt is missing. Keep the rescue going — the other guards still hold the draft.

### Process note (orchestration)

The shadowing was found during an interactive `/mika` confidence-check of an already-GROOMED plan. Because the cleaner fix (delete Path A) *overturned* the architect's signed-off design and contradicted its explicit "refactor only if further drift surfaces" out-of-scope note, the design-overturn fork was routed to **mika-arch** (Disposition: ESCALATE) and then to **Mika Prime** for a bearing ruling (R2: single-source-of-truth on a churning safety contract), then revised via `/mika-revise-plan` and re-reviewed (Verdict: GROOMED) before implementation. Pattern: an implementer who finds a groomed resolution contradicts code reality *carries* a fix that completes the resolution's intent, but *routes* a fix that overturns the design.

## References

- mika#1679 — this fix
- mika#1396 — the correct `commit-pushed-no-pr` rescue (Path B) that was shadowed
- mika#1383 — the structural completion gate (Path A) whose PR-creation was removed
- mika#1613 — the recovery guards the non-draft PR bypassed
- mika#1618 — qa-review Step 1.5 rescue-header detection
- mika#PR1678, mika#PR1683 — the bypassed-case evidence
- `skills/bundled/_shared/dispatch-lib.sh` — `_post_flight_recovery()` (Path A) and `dispatch_claude_pilot()` Unit 2 (Path B)
