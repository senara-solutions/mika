<!--
V1 — EXPECTED RED (mika#2201 AC3, rule L1).

`VERDICT_TOKEN_RE` is `\b(GROOMED|ESCALATE[DS]?)\b`, and a hyphen satisfies a
word boundary, so `ESCALATE` matches INSIDE `ESCALATE-divergence`. mika#2188
closed that by POSITION — a later completed marker outranks the escalation —
and it deliberately declined to tighten the regex, because that "would make the
verdict depend on the spelling of a compound word rather than on chronology".

That resolution is conditional on a later pass EXISTING. A callout carrying
`(ESCALATE-divergence, résolu par l'opérateur)` and NOTHING AFTER reads
`Escalated`: the prose says resolved, the machine says escalated. mika#2188 says
so itself — "claiming it distinguishes a resolved escalation from an open one
would lend it a semantic reading it does not have."

That residue is what a textual lint catches and a positional predicate cannot.
-->

- **Branch:** `feat/2201/lint-jetons-machine-canoniques-dans-les`
> - **Plan:** `docs/plans/2026-09-20-005-feat-2201-lint-jetons-machine-canoniques-plan.md` (committed on branch @ `4950de91`)
> - **Grooming history:** /ce:plan → checkpoint Phase 2.5 (ESCALATE-divergence, résolu par l'opérateur)
