# Review brief (untrusted review data)

## Intent

Groomed plans produced by the autonomous loop could lack the `## Fire-Disposition`
section a doctrine requires when a plan ships a detector-class deliverable. The
architect then returns ITERATE — correctly — and the grooming loop, which has
exactly ONE iteration, spends it on a purely formal defect; at the second pass the
gate has no recourse, so the ticket ESCALATEs and implementation is never
dispatched. The change adds two things to the shell dispatch substrate: a
prescription injected into the grooming pilot's prompt, and a one-shot retry of
the revise pilot when the architect's first-pass findings asked for the section
and the revised plan still does not carry it. The architect call budget (two
passes) must be unchanged.

## Material risk divisions

1. **The retry guard's predicate and its termination.**
   `skills/bundled/_shared/dispatch-lib.sh`, function
   `_fd_retry_if_section_still_missing`, called from `_launch_revise_pilot`'s
   success branch. A conjunction of two greps plus a counter. The interesting
   failure is not "it does not fire" but "it fires when it should not", or fires
   more than once, or its counter can be reopened by a downstream failure. Note
   the counter is a global (no `local`) reset at the top of `_launch_revise_pilot`.

2. **The source of the predicate's first term.**
   Same function. The guard WRITES a targeted findings file and must never READ it
   back: that file contains the trigger string by construction, so a predicate
   reading it would be true always. Check that no path — including the pilot
   relaunch — can make the written file become the predicate's input on a later
   evaluation, and that no recursion into `_launch_revise_pilot` exists.

3. **Interaction with the surrounding error/exit discipline.**
   Same file, under `set -euo pipefail`. The new code runs greps in conditions,
   calls `mktemp`, runs a subprocess with `set +e`/`set -e` around it, and
   dereferences several globals (`CWD_ARGS`, `LOG_ID`, `REPO`, `ISSUE_NUM`).
   Consider what happens when any of those is unset, when `mktemp` fails, when the
   findings write fails, and whether any of it can change `_launch_revise_pilot`'s
   return value, which the caller uses to decide whether grooming converged.

4. **The prompt injection site.**
   Same file, inside `_set_up_worktree`, right after two pre-existing
   unconditional injections. The new one is conditional on `$SKILL`. Three
   documented position invariants hold there; the most consequential is that the
   FIRST LINE of the prompt must stay exactly `<repo>#<num>`, or an anchored regex
   upstream misses and the dispatch silently falls into free-text mode with no
   worktree created.

5. **The test harness as a silent-pass mechanism.**
   `skills/bundled/_shared/test-dispatch-lib.sh`, new section T1–T11 at the end.
   These are the detectors for everything above, so their own fidelity is the
   question: can any of them go green while the guard is broken? Two structural
   scans (no `_arch_ask` in the guard; the targeted findings file is never
   grepped) and one behavioural probe that shadows the pilot launch. The probe
   builds a fixture plan that must exceed a 500-byte filter applied upstream, and
   parses a `rc|count|stderr` triple. Both are places where a test can pass
   without exercising anything.

6. **Two neighbouring structural counters raised by this change.**
   Same test file: the census of `claude-pilot` launch sites (3 → 4) and of
   `--log-dir "$_PILOT_LOG_DIR"` occurrences (2 → 3). Raising a census is how a
   guard is legitimately updated and also how one is silently widened; the
   question is whether each increment corresponds to exactly one new real site.

## Cross-division interaction to test

Division 1 and division 5 are coupled: the retry guard leaves the caller's return
value untouched on purpose (the plan DID change on the first pass), so a guard
that silently did nothing on its second evaluation would be indistinguishable
from one that worked, unless a specific test asserts the failure event is
emitted exactly once and no third relaunch occurs.
