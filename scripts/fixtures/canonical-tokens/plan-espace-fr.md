<!--
V3 — EXPECTED RED (rule L3).

The French typographic space before a colon. This is the most probable variant
in a repository that writes its tickets in French, and NEITHER of the two bites
mika#2201 cites has produced it yet — which is the whole prospective value of
the lint rather than a retrospective one.

All three readers of the `Plan` callout are line-anchored and literal:

    auto_pull.rs::PLAN_CALLOUT_RE            (?m)^> - \*\*Plan:\*\* `…`
    dispatch-lib.sh::_extract_plan_path      grep -oP '^> - \*\*Plan:\*\* `\K…'
    dispatch-lib.sh::_committed_plan_on_branch  grep -qE '^> - \*\*Plan:\*\*'

One of these forms makes the ticket invisible to the feeder. mika#2120 measured
what that costs on a neighbouring axis: `auto_feeder_no_backlog` on every
ten-minute tick with SIX groomed candidates in front of it, an empty queue for
over fifteen hours, and the loop stopping precisely when grooming SUCCEEDED.
-->

> - **Branch:** `feat/9999/espace-typographique`
> - **Plan :** `docs/plans/2026-09-20-999-feat-9999-espace-plan.md` (committed on branch @ `deadbee`)
> - **Grooming history:** first-pass (READY) → second-pass (GROOMED) — session-id: 11111111-2222-3333-4444-555555555555
