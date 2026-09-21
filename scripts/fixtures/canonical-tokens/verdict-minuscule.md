<!--
V4 — EXPECTED RED (rule L2).

`VERDICT_TOKEN_RE` is case-SENSITIVE, and `grooming_marker.rs` states why in as
many words: "case matters: `GROOMED` is a token the pipeline produces, 'groomed'
in French or English prose is not one."

So a callout carrying `second-pass (groomed)` reads `Absent`, not `Groomed` —
the ticket is never promoted, and nothing says so. Note `second-pass` itself is
lower-case here too and that is FINE: `LATER_PASS_RE` carries `(?i)`. Two tokens
on one line, two tolerances, and only one of them may be accused. That is the
distinction the `tolérance` column of the TSV exists to carry, and this fixture
is what proves the lint consults it instead of accusing the whole line.
-->

> - **Branch:** `feat/9998/casse`
> - **Plan:** `docs/plans/2026-09-20-998-feat-9998-casse-plan.md` (committed on branch @ `cafe123`)
> - **Grooming history:** first-pass (READY) → second-pass (groomed) — session-id: 99999999-8888-7777-6666-555555555555
