<!--
V2 — EXPECTED GREEN, and this fixture's SIGN IS THE POINT (mika#2201 § R1).

mika#2201's AC3 asked for `seconde passe` as an anti-vacuity fixture, i.e. as a
form the lint should REFUSE. Confronted to the code that matches — which is
Prime's own closing bound for this ticket — that is backwards.

`grooming_marker.rs`, sole reader of the verdict marker since mika#2158, is
bilingual BY WRITTEN DECISION:

    LATER_PASS_RE       = (?i)(second-pass|seconde passe|deuxième passe|deuxieme passe)
    FIRST_PASS_READY_RE = (?i:first-pass|première passe|premiere passe)\s*\(\s*(READY)

and its doc-comment settles the direction of the alignment: "it is the PREDICATE
that aligns on the spec, not the reverse. […] the spec does not have to impose
English to be machine-readable, in a repository that writes its tickets and its
plans in French." Prime says the same: "French did not bite — a textual boundary
bit, and it is now structural."

So the fixture stays, and CHANGES SIGN: it is a NON-REGRESSION guard. The day it
turns red, somebody has tightened the lint onto a population the reader accepts,
and this file is what says so — before a French-writing operator discovers it on
a ticket that sits invisible for days.

Note the callout below also carries `(READY)` in a form the FIRST-PASS reader
reads, and a lower-case French sentence around it. None of it may be accused.
-->

> - **Branch:** `fix/1772/seconde-passe`
> - **Plan:** `docs/plans/2026-08-30-001-fix-1772-seconde-passe-plan.md` (committed on branch @ `abc1234`)
> - **Grooming history:** première passe (READY) → seconde passe (GROOMED) — session-id: 550e8400-e29b-41d4-a716-446655440000

La prose du ticket est en français, et le lecteur la lit : c'est le bearing Prime,
et c'est une garde, pas une tolérance provisoire.
