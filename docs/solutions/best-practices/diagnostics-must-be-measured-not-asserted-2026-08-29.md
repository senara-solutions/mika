---
module: skills/bundled/_shared/dispatch-lib
tags: [diagnostics, error-reporting, autonomous-loop, dev-groom, dispatch-lib, honesty, failure-classification]
problem_type: best_practice
category: best-practices
date: 2026-08-29
ticket: mika#1772
applies_when:
  - Writing the message a failure path reports to an operator or an agent
  - Reviewing a caller that emits one sentence for a callee with several failure modes
  - Classifying why an autonomous session ended
  - A callback sends someone to investigate the wrong thing
root_cause: logic_error
resolution_type: code_fix
---

# A diagnostic that is asserted rather than measured is wrong exactly when it matters

## The shape

When a function has N failure modes and its caller emits one sentence about them, that sentence is a guess. It reads as a fact, it is the first thing the operator sees, and it is wrong on N-1 of the paths.

`_iterate_groom_loop` in `dispatch-lib.sh` has 18 `return 1` sites — guard trips, a missing plan, a failed architect call, a response with no content, three architect refusals, an unconverged revise pilot, an unparsable disposition. All 18 collapsed into one hardcoded line at the call site:

> `PIPELINE FAILURE: architect convergence did not complete (_iterate_groom_loop returned non-zero). Plan exists on branch but architect verdict is missing.`

On the 2026-08-28 dispatches of mika#2013 the loop never reached the architect and no plan existed. The sentence sent the operator hunting a missing architect verdict for a file that was never written — the wrong side of the problem, stated with total confidence, three times over 45-minute intervals.

The same callback carried two more claims of the same kind:

- *"no /ce:plan invocation detected in session log"* — the session log file did not exist. Nothing was detected either way; the code reported a search it never performed.
- *"plan already committed from prior run"* — the guard matched **any** `*-plan.md` in `docs/plans/`, of which `main` carries 769. It was true of every dev-groom dispatch and informative about none.

Three false statements, one true one, the false ones printed first.

## The rule

**Every claim about state is re-derived, at write time, from the thing that knows it.**

| Claim | Ask |
|---|---|
| why the loop failed | the loop — it records its own reason before each exit |
| whether a plan exists for this issue | `_find_issue_plan` (worktree) or `_committed_plan_on_branch` (remote) |
| whether the process exited zero | `$PILOT_EXIT` |
| why a session was terminated | the result's `.subtype` / `.termination_reason` |
| whether the branch carries work | `PRE_RUN_HEAD` vs `POST_RUN_HEAD`, plus `git status --porcelain` |

Two measurements of the same-sounding thing get two sentences, never one. `_find_issue_plan` answers about the worktree; `_committed_plan_on_branch` answers about the remote **and** needs a callout in the issue body. Collapsing them into "a plan exists" produces a claim neither one supports.

When nothing can answer, say that. `Halt: cause not recorded — no subtype on the result and no [guardrail] line in stderr` is a useful message. Naming a cause you did not read is not.

## Why this one is worth writing down

Code review caught the **fix** reintroducing the defect.

The replacement classifier stated *"Nothing was written to the branch and the architect was never invoked"* without measuring either. That reads as fine until you notice `status: terminated` covers two populations — a guardrail abort that usually kills a session at turn 1, and an SDK limit (`error_max_turns`, `error_max_budget_usd`) that often kills one at turn 40 with commits on the branch. For the second population the new sentence was false, and worse, the code path skipped the dirty-worktree rescue, so files the pilot had written were lost when the next dispatch force-removed the worktree.

The corrected shape gates the absolute claim on the measurement that licenses it:

```bash
if [ "$STATUS" = "terminated" ] && _pilot_left_no_work; then
    # earns "nothing was written": HEAD unmoved AND clean tree
else
    _post_flight_recovery              # preserve the work first
    # banner states the commit range instead of denying it
fi
```

Writing the honesty rule down does not install it. It was violated inside the change that established it, by an author who had just spent hours on the principle, and only an independent reviewer caught it. Assume the same of the next diagnostic you write.

## Prevention

For any message a failure path emits, take each sentence and name the expression that makes it true. A sentence with no such expression is either deleted or gated behind the measurement that licenses it. Applied to this file it deleted two claims, gated one, and turned one hardcoded phrase into a value the failing function records.

## Related

- `docs/solutions/workflow-issues/2026-06-14-dev-groom-drift-misdiagnosis-policy-deny-halt.md` — the same class, earlier: a policy-deny halt reported as LLM drift, sending the operator to the pilot's reasoning instead of the allow-list.
- `docs/solutions/test-failures/bash-assert-sigpipe-and-host-coupling-before-ci-gate-2026-08-29.md` — the regression coverage this rule needs, and why the suite had to become deterministic before it could hold it.
