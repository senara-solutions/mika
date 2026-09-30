<!--
V3 — EXPECTED RED (rule L3).

The French typographic space before a colon. This is the most probable variant
in a repository that writes its tickets in French, and NEITHER of the two bites
mika#2201 cites has produced it yet — which is the whole prospective value of
the lint rather than a retrospective one.

Since mika#2194 (phase 1) and mika#2608 (phase 2) there is exactly ONE reader of
the `Plan` callout, and it is line-anchored and literal:

    plan_callout.rs::PLAN_CALLOUT_RE   (?m)^> - \*\*Plan:\*\* `…`

The three bash and Rust readers this fixture used to name — `auto_pull`'s own
regex, and the two in `dispatch-lib.sh` — all delegate to it now and carry no
copy. They are deliberately NOT named here any more: a fixture that prescribes a
dead site is what a future editor copies (class mika#2050, measured twice in this
repo), and this file's whole job is to be read by someone repairing a lint hit.

The form below makes the ticket invisible to the feeder. mika#2120 measured
what that costs on a neighbouring axis: `auto_feeder_no_backlog` on every
ten-minute tick with SIX groomed candidates in front of it, an empty queue for
over fifteen hours, and the loop stopping precisely when grooming SUCCEEDED.
-->

> - **Branch:** `feat/9999/espace-typographique`
> - **Plan :** `docs/plans/2026-09-20-999-feat-9999-espace-plan.md` (committed on branch @ `deadbee`)
> - **Grooming history:** first-pass (READY) → second-pass (GROOMED) — session-id: 11111111-2222-3333-4444-555555555555
