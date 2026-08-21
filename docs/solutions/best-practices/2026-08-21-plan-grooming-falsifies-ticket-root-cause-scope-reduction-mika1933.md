---
module: milestone_manager
tags: [plan-grooming, empirical-grounding, scope-reduction, regression-lock-in, agent-milestone, prime-bearing]
problem_type: workflow-discipline
category: best-practices
related_issues: [mika#1933, mika#1932]
related_docs:
  - docs/solutions/workflow-issues/dev-groom-zero-artifact-exit-2026-05-13.md
---

# Plan grooming FALSIFIED ticket root cause; scope reduced from filter-fix to rendering-fix (mika#1933)

## Signal

Ticket mika#1933 was filed with a concrete-looking bug hypothesis rooted in an
empirical mismatch:

- GitHub API for `senara-solutions/mika` milestone #31 returned
  `open_issues:4, closed_issues:1` → ground truth = 5 total.
- `mika milestone report senara-solutions/mika#31` output showed
  `Progress: 0/2 sub-issues complete (0% done)` — the wrong denominator
  AND the wrong numerator, plus the closed sub-issue absent from the enumeration.

The ticket body proposed a root cause: `crates/mika-agent/src/milestone_manager/reader.rs`
likely invokes `gh issue list --milestone <N>` with default `--state open` filter and/or
missing pagination. AC1 through AC6 were scoped against that hypothesis, from
"Reader fetches ALL sub-issues (closed + open)" through "sed-inject that re-adds
`--state open` → tests fail".

## Finding

**Grooming pass with mika-arch empirically FALSIFIED the ticket's root cause hypothesis.**
The actual state of `Reader::read_with_runner` post-PR#1932 already contained
`"--state", "all"` and `"--limit", "100"` as adjacent-pair args in the `gh issue list`
invocation. Live re-probe on milestone #31 returned the correct
`Progress: 1/5 sub-issues complete (20% done)` — the empirical symptom that founded the
ticket did not reproduce.

The ONLY genuinely missing surface was AC3: the Reporter had `write_in_flight`,
`write_blocked`, `write_unstarted` siblings but NO `write_completed` — so the CLOSED
sub-issue (the "established brick", per Prime's bearing) never appeared in the
rendered `### État` section even though the denominator was correct.

## Pattern

**When a ticket ships a root-cause hypothesis, grooming's first move is to
falsify it against the current tree.** The falsification cost is one live probe
+ symbol-grep + read; the alternative (implementing the hypothesized fix on top
of already-shipped code) is a duplicated ship + confused git history + false
signal to the next agent reading the ticket.

Concretely, the grooming discipline that saved this ticket:

1. **Live probe against the currently-deployed binary before planning.** The
   ticket said `Progress: 0/2`; the probe said `Progress: 1/5`. That mismatch
   is the wedge — either the ticket author saw a pre-fix state or the current
   fix is masking a subtler intermittent, but in either case the plan can't
   proceed on the ticket's word.
2. **Grep the suspect symbol before writing the fix.** The plan called for
   `--state all` to be added to the `gh issue list` call. `grep -n '"--state"'
   crates/mika-agent/src/milestone_manager/reader.rs` shows it's already there.
   That one grep collapses AC1 from "implement" to "regression-lock via test".
3. **Rebase the AC set explicitly.** Per-AC disposition in the plan document,
   with evidence:
   - AC1: already satisfied by PR#1932 → **regression-lock via arg-capture test**
   - AC2: already satisfied → **regression-lock via extended compose_end_to_end**
   - AC3: genuine gap → **implement `write_completed` + 4 Reporter tests**
   - AC4: unaffected by AC3 → **invariance test proving closed count doesn't shift semantics**
   - AC5+AC6: scope-consistent → **arg-capture guards + injection-verified evidence**

The Prime bearing (2026-08-21) reframed the semantic reading of the bug
correctly: *« Le Reader doit distinguer « état d'avancement » (closed compte)
de « reste-à-faire » (open seulement). Un brick CLOSED n'est pas du bruit,
c'est le signal le plus fort. »* — the WHY that survived falsification.

## What to do

- **First pass of any grooming that ships a root-cause hypothesis:** run one
  live probe + one `grep` against the named symbol. If either falsifies the
  hypothesis, halt planning until the AC set is explicitly rebased with evidence.
- **When AC drops from "implement" to "regression-lock", KEEP it in the plan
  as a regression-lock task** — don't silently drop it. The ticket's AC list
  is a contract with the operator; each AC needs a disposition (implemented /
  locked-in-via-test / re-scoped / withdrawn), not silent omission.
- **Preserve the Prime bearing verbatim in the plan and PR body** even when
  the mechanical fix is smaller than the ticket implied. The bearing is what
  survives falsification; the hypothesis is what got falsified.

## Anti-pattern

**Don't implement the hypothesized fix "just to be safe" on top of already-shipped
code.** The result is a duplicate `--state all` add, git blame confusion, and
the wrong signal (the AC1 test would pass whether or not the fix landed).

**Don't scope-reduce silently.** A grooming pass that shrinks AC5's fixture from
"open + closed + PR + blocker mix" to "closed-only sanity check" without
recording the rationale forfeits the ticket's regression-locking discipline.

## Evidence

- Plan doc: `docs/plans/2026-08-21-002-fix-1933-reader-completed-section-avancement-plan.md`
  § 0 (Empirical grounding before scope) documents the live probe + symbol
  greps + per-AC disposition rebase.
- Implementation: `crates/mika-agent/src/milestone_manager/reporter.rs`
  gains `write_completed` (~50 lines source + 6 tests) as the sole real code
  change. The remaining ~200 lines (Reader arg-capture guards + Assessor
  invariance) are regression-lock scaffolding for the falsified-hypothesis ACs.
- Injection-verified pre-commit run (plan § 7): sed-inject `--state open` → the
  `RecordingGhRunner`-based tests fail with the exact adjacent-pair mismatch
  panic; restore → all tests green.
