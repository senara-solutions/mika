---
title: "A guard that reads its evidence path out of the claim it is checking cannot refute that claim"
date: 2026-08-30
category: best-practices
module: skills/bundled/_shared/dispatch-lib.sh
tags: [loop-substrate, dispatch, grooming, guards, attestation, false-positive]
problem_type: silent-wrong-answer
issue: senara-solutions/mika#2034
---

## The shape

A guard decides whether a ticket has already been groomed. It reads the plan path
out of the ticket's own body callout, then checks that the path resolves on the
dispatch branch. If it does, the ticket is declared groomed and grooming is refused.

Both halves look like measurement. Neither is.

- The path came from the claim under test. Resolving it tests the *claim's syntax*,
  not the ticket's state.
- Every dispatch branch descends from `main`, and `main` carried 769 plan files. So
  **any** valid plan path resolved, whatever ticket it belonged to.

The guard could therefore only ever agree with the body. Two tickets were refused
grooming permanently: mika#1887, whose callout named a plan headed
`issue: senara-solutions/mika#1933`, and mika#2026, an observation ticket whose
callout named an April `rand` dependency-bump plan headed `**Issue:** #539`.

## Why it stayed invisible

The assertion suite covering this guard was green throughout — 461 assertions, five
fixture cases aimed squarely at it. Its fixture repository's `main` carried **no plan
files**. The production defect *cannot occur* in a tree with no inherited plans, so
the suite was testing a world in which the bug does not exist. Same class as
`a-stub-built-from-the-doc-cannot-falsify-the-doc`: the fixture inherited the
implementation's assumption instead of challenging it.

The tell is cheap to look for: **if the fixture cannot express the production shape,
a green suite is evidence about the fixture, not about the code.**

## The second-order cost

mika#2034 was filed to wait for a recurrence of a different grooming failure, then act
on what the new diagnostic named. The diagnostic was correct and deployed. It produced
nothing for 30 hours because **8 of 8 dev-groom dispatches were refused by this gate
before the diagnosed code ran at all**.

A ticket whose scope is "wait for evidence" is only as sound as the path that produces
the evidence. Before waiting, measure that the producing path still runs. Here the
measurement was one query: count dispatches since the fix merged, and how many reached
the instrumented function. Zero would have been visible on day one.

## What to do instead

- **Bind the candidate to the subject before believing it.** The instrument already
  existed in the same file — `_plan_header_refutes_issue`, built by mika#2038 for this
  exact `rand` plan. `_find_issue_plan` used it; the dispatch gate did not. When a
  refutation helper exists, every consumer of that evidence class should call it; grep
  for the helper's callers when you add a new one.
- **Refute, never confirm.** Reject only on positive evidence of a different owner.
  95 of 745 plans carry no issue marker; demanding a positive match would strand every
  one of them. Silence is not evidence in either direction.
- **Separate the decision from the description.** Whether the plan was *committed* to
  the branch or *inherited* from `main` is real, and worth saying — but it must not
  gate, because a legitimately groomed and merged ticket also has its plan on `main`.
  Measure it, print it, do not branch on it.
- **When a check cannot be performed, decline — do not skip.** The first draft of the
  fix silently skipped the binding when `mktemp` or `git show` failed, which fires the
  gate on an unbound candidate: the original defect, reached by another road. A guard's
  own failure modes need the same fail-direction discipline as its subject.
- **`cat-file -e` is satisfied by a directory.** A callout naming `docs/plans` resolved,
  and `git show` on a tree prints a listing — non-empty, no issue header, therefore
  "not refuted". Assert the object type, not merely that the path resolves.

## Verification that actually discriminates

Re-running the fixed guard against the two real tickets first *passed for the wrong
reason*: the branches had since been deleted, so the guard declined at its `git fetch`,
never reaching the refutation. A correct verdict produced by an unrelated path is not
verification. The reproduction was rebuilt from the real plan blobs — branch inheriting
from `main`, committing nothing — and only then did the refusal diagnostic name issues
1933 and 539.

Check *which* branch produced the answer, not just that the answer was right.

## See also

- `docs/solutions/best-practices/diagnostics-must-be-measured-not-asserted-2026-08-29.md` (mika#2028 — same class, the reporting site)
- `docs/solutions/best-practices/run-the-new-check-against-live-state-before-calling-it-done-2026-08-29.md`
- mika#2038 (`_plan_header_refutes_issue`), mika#2012 (the gate's origin)
