---
title: "A guard must read substance, not the shape its producer happens to emit"
date: 2026-09-04
category: architecture-patterns
module: agent-core
problem_type: architecture_pattern
component: auto-pull
severity: high
applies_when:
  - Writing a gate that classifies an artifact produced by another part of the system
  - The classifier keys off a count, a length, a filename, or any other cheap proxy
  - The producer is a documented pipeline whose output shape can legitimately vary
  - Reviewing a guard whose comment says a separation comes "for free"
symptoms:
  - The gate refuses precisely the items that got the most work
  - A manual lift of the gate's label survives minutes to hours, then re-fires
  - The refusal message states a category ("partial work") without naming evidence
  - The gate's own tests are green because they encode the same false assumption
root_cause: incorrect_assumption
resolution_type: code_fix
related_components:
  - auto_pull promotion gate
  - mika-groom-ticket
related_issues:
  - mika#2140
  - mika#2120
  - mika#2123
---

# A guard must read substance, not the shape its producer happens to emit

## The recurring shape

Three defects in four days, in three different files, all the same shape:
**a guard encoded an assumption about what its producer produces, and the
producer legitimately produced something else.**

| # | guard | assumption | reality |
|---|---|---|---|
| 1 | `is_groomed` (mika#2120) | the plan callout reads `docs/plans/` | the grooming spec writes `mika/docs/plans/` |
| 2 | `dispatch-lib.sh:4405` `PLAN_PATH` extraction | same | same |
| 3 | `classify_promotion` (mika#2140) | a groomed branch carries **one** plan commit | the grooming spec commits the plan at **three** sites |

The first two are prefix blindness and were compounded as
[a guard parser must be as permissive as the downstream consumer it protects](guard-parser-must-be-as-permissive-as-downstream-consumer-2026-08-29.md).
The third is the same class one level up, and it is the one worth naming
separately, because the proxy it used was not a string — it was a **count**.

## The concrete defect

`auto_pull.rs` separated "a branch carrying only its plan" from "a branch
carrying a dead pilot's partial work" like this:

```rust
if staleness.ahead_by > 1 {
    return PromotionGate::Refuse(RefusalReason::SalvageWorkOnStaleBranch { … });
}
```

The module said so out loud, and the sentence is the defect in one line:

> `ahead_by` separates the two populations **for free**: a branch carrying only
> its plan has `ahead_by == 1`.

It is not free. `/mika-groom-ticket` commits the plan at Phase 3 step 10,
Phase 4 step 12 and Phase 5 step 17 — **by design**, so the lineage between
"the architect signed" and "the operator wrote" stays readable. So every ticket
whose architect asked for one iteration carried `ahead_by ∈ {2,3}` without a
pilot ever touching it.

**The predicate punished exactly the property it should have rewarded.** The
more a plan was reworked, the more commits it carried, the more certainly the
gate called it a dead pilot's leftovers, labelled it `operator-gated`, and
removed it from the pool that feeds the loop.

## Three signs that generalize

**1. "For free" in a comment is a smell.** A separation that costs nothing is
usually a proxy standing in for the property you actually mean. Write down the
property (*"this branch carries work that is not grooming"*), then ask what
directly measures it (*the files the branch touches*) — not what correlates with
it today.

**2. The blast radius grows silently, because the proxy tracks activity.**
Measured on the open backlog: **2** false positives on 2026-09-02, **10** on
2026-09-04 — a fivefold rise in two days with no code change. A count-based
proxy drifts with how busy the system is. Among the ten was the branch of the
ticket filed to fix the gate: *the gate refused its own repair.*

**3. A manual remedy with a half-life is not a remedy.** Both tickets were
hand-rebased and un-labelled on 2026-09-02. `#2120` was re-gated **63 minutes**
later, `#2118` after ~73. The mechanism is worth stating because it recurs: the
rebase set `behind_by` to `0`, which promoted via an *upstream short-circuit*,
not because the faulty predicate had stopped being true. The next merge on
`main` restored `behind_by > 0` and the predicate took over again.

> **The test for a manual remedy: name the event that undoes it, and measure the
> time to that event.** If the remedy is only reachable through a short-circuit
> that a routine event closes, it is Sisyphus wearing a fix's clothes.
> See [[feedback_un_remede_manuel_a_une_demi_vie_mesurez_la]].

## The fix, and the part that is reusable

`ahead_by` stops being a discriminator and goes back to being a distance —
logged, never interpreted. The predicate becomes: *the branch modifies at least
one file outside `docs/plans/`.*

Two properties of the rewrite are worth copying:

**The fail-open path is built by construction, not by a branch.** The
partitioning helper is total: an unreadable file list yields an empty
non-plan vector, so the caller finds nothing to salvage and falls through. API
truncation needs no special case either, and the reasoning is worth reusing
verbatim: if the truncated list already shows a non-plan file, the fact sought
is established and truncation cannot retract it; if everything visible is a plan
file, the only possible ignorance is "there might be code further down", which
promotes. **A gate whose fail-open is a separate `if` will eventually have a
path that misses it.**

**The refusal names its evidence.** The old message said "partial work from an
earlier pilot" and made the operator redo the investigation by hand — twice, on
2026-09-02. The new one names the offending files (bounded, so a comment cannot
become a `git diff --stat`). A category without evidence is not a reason; it is
a verdict.

## What the fix does not close, said out loud

A pilot killed before its first commit leaves **uncommitted** work. That is
invisible to a commit count and equally invisible to a file list — both read
only what was committed. Closing it needs a local git state this module does not
have. The predicate fixes one direction of a two-directional error and says so
in the module rather than implying completeness.

## The check to run before shipping a guard

1. **Name the property**, not the proxy. If the comment says a proxy separates
   the populations "for free", that is the line to delete.
2. **Read the producer's spec** and enumerate every legal output shape. For a
   pipeline, that means every commit site, not the happy path.
3. **Evaluate both predicates on the live population** and count the
   flips — in *both* directions. The dangerous direction (something that
   passed now fails) needs a named disposition per item, not a summary.
4. **Re-run that measurement at implementation time.** It has a date, and the
   backlog moves: this one went from 11 branches to 18, and from 2 false
   positives to 10, in one day. It also surfaced a flip the original
   measurement had not seen, because the original counted only *open* tickets
   and the new case was a closed one frozen in a fixture.
5. **Freeze the boundary case with a self-cleaning assertion.** Where a
   refusal is overdetermined (another rule would refuse it anyway), assert the
   overdetermination. The day someone freezes a case where the narrow rule is
   the *sole* cause of a refusal that would otherwise have promoted, that
   assertion fails and reopens the question instead of letting it answer itself
   in silence.
