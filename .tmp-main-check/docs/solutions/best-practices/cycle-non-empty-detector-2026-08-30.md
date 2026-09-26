---
title: "A successful cycle must mean a non-empty output — and the detector lives inside the loop"
module: dispatch-lib
date: 2026-08-30
problem_type: best_practice
component: loop-substrate
severity: high
tags: [dispatch-lib, claude-pilot, empty-completion, callback, control-must-be-unavoidable, false-red, measurement]
applies_when: "Any mechanism reports success or failure for work it did not measure — pilot cycles, batch jobs, scheduled repairs, rescue nets"
---

# A successful cycle must mean a non-empty output

## Context

Measured on 2026-08-29: of the **120 most recent claude-pilot sessions, 102 made zero tool calls** and none exceeded two. The last session above ten tool calls was **2026-07-29**. Throughout that month the loop reported those cycles as successes, because success was defined as `exit code 0` + `callback delivered` + `task_id returned` — three process signals, none of which looks at what the cycle produced.

The measurement that would have caught it already existed. `_pilot_left_no_work` in `dispatch-lib.sh` reads exactly the right thing — HEAD unmoved and a clean worktree — but it sat behind `if [ "$STATUS" = "terminated" ]`. The three other exit paths (`status: success`, exit 0 with unstructured output, non-zero exit) and the crash path in the EXIT trap delivered a verdict without anyone looking. Downstream the engine reaper carries an explicit groom-class filter, so an empty grooming cycle was never even eligible to be marked failed.

The rescue nets (`#1282` dirty-worktree, `#1383` auto-PR-create) recovered work **after the fact**. That is the defect this closes: the loop *discovered* the outage instead of *detecting* it. It is the most expensive version of the signal that answers something rather than nothing.

## Guidance

**1. Define non-empty as a trace of production observable outside the process, and write down what it excludes.**

A cycle is non-empty when it left at least one of: a PR that belongs to it; commits whose diff is non-empty; written files in the worktree; or a conclusive terminal disposition backed by at least one tool call. Everything else is excluded, and the exclusions are the half that carries:

- process signals — exit code, `status: success`, callback delivered, task id returned;
- output volume — turns, text length, cost, duration;
- files outside the repository (writing to your own log is not producing);
- commits with no content — an `--allow-empty` marker moves HEAD without producing anything, so require a non-empty diff, not a moved HEAD;
- a disposition on its own — an `Outcome:` line with zero tool calls is text about work, not work;
- reading — forty read-only tool calls that leave no trace are empty.

A definition whose exclusions are unwritten is not defensible; it is a feeling with a function name.

**2. Anti-vacuity must run both ways, and the positive direction is the load-bearing one.**

"A cycle that produces nothing fails" is satisfied by "always fail". Pair it with "a cycle that produces something still succeeds", and make the positive direction an *invariant*, not an intention: on a producing cycle the gate leaves the callback **byte for byte identical**, and a test asserts that equality. Without it, the gate's own regression is invisible.

**3. Never let the criterion manufacture a false red.**

A false red trains people to ignore red, which costs more than the silence it replaced. Three shapes of protection:

- **A third verdict.** `produced` / `empty` / **`undetermined`**. A detector forced to pick green or red when it has no ground to measure will always pick wrong: fail-closed invents failures, fail-open restores the silence. `undetermined` says so, changes no outcome, and is countable — so a frequent `undetermined` is itself a visible defect.
- **Distinguish "produced nothing" from "no cycle ran".** The dispatcher's own deliberate exits — a closed-issue auto-skip, an already-groomed refusal, a dry run — are decisions, not cycles. Gate them out on a flag written at the single point where the session actually launches, and assert that the flag has exactly one writer.
- **Never stack a second diagnosis.** A callback already carrying a terminal classification (terminated session, push violation, handler crash, operator cancel) keeps it. The first diagnosis names a cause; this gate can only describe an absence, and the more specific one wins. Stacking is how red becomes unreadable.

**4. The measurement that decides must not be the thing being judged.**

Keep the measurement side-effect free and separate from the gate that acts on it. It is what makes the positive-direction invariant provable rather than merely asserted. And keep the enriching signal out of the criterion: a tool-call count read from a file that can be missing may qualify a disposition, but must never on its own condemn a cycle that produced something or rescue one that did not.

**5. The control has to be unavoidable, and a guard has to keep it that way.**

CONTROL-MUST-BE-UNAVOIDABLE (Bearing Prime, 2026-08-26): a guarantee exists only if **every** path producing the guarded effect crosses the control point. Here the guarded effect is "a cycle verdict reaches mika-dev", and it had two producers — `_deliver_callback` and the EXIT trap, which sends its own callback rather than calling it. Both cross the gate, and a static test parses the file by function and fails if any function containing a delivery call lacks the gate. An invariant of this shape is lost by the next well-meaning patch unless something structural remembers it.

**6. Falsify against the world, not against the definition.**

A fixture built from the definition cannot refute the definition. Run the finished detector against real logs from the host: it classified `empty` for **40 of the 40 most recent real sessions**, its tool counts matched a direct `grep -c` on every one, and on a genuinely productive session from 2026-07-29 (33 tool calls) it returned `produced` through both the commit path and the disposition path — and `empty` when those 33 calls had left nothing behind.

## Reference

- Implementation: `skills/bundled/_shared/dispatch-lib.sh` — `_measure_cycle_output`, `_gate_non_empty_cycle`
- Tests (both directions + the bypass guard): `skills/bundled/_shared/test-dispatch-lib.sh`, section `mika#1996`
- Founding incident: mika#1910 (silent empty output at max_turns), mika#1996
- Sibling principle: [`no-substrate-on-open-failure-mode-2026-08-30.md`](no-substrate-on-open-failure-mode-2026-08-30.md)
