---
title: "A proximity grep is not a behavioral pin — evaluate the real guard block with a spy"
date: 2026-09-21
category: best-practices
module: dispatch-lib, skills/bundled/_shared
problem_type: best_practice
component: development_workflow
severity: medium
applies_when:
  - Pinning a shell guard (an `if` around a callsite) whose position in a long function is itself the contract, so it cannot be extracted into a function
  - Writing or reviewing a structural assertion that locates a line by `grep -n` and compares line numbers
  - Running a negative control on a suite that reads the source file rather than executing it
tags: [shell-tests, structural-assertion, mutation-escape, negative-control, dispatch-lib, spy, awk-eval]
---

# A proximity grep is not a behavioral pin — evaluate the real guard block with a spy

## Context

mika#2155 added a three-line guard inside `_set_up_worktree`
(`skills/bundled/_shared/dispatch-lib.sh`): `if [ "$DRY_RUN" != "true" ] && [ "$DRY_RUN" != "1" ]`
around the `_stamp_issue_seat` callsite. Two guarantees hang on it — a real
dispatch claims the ticket before touching the worktree, a dry run leaves GitHub
untouched. The first suite pinned the callsite with **line-order** greps (after
the #2012 gate, before `git fetch origin main`) and pinned the guard with a
**proximity** grep: "some line containing `DRY_RUN` sits within 8 lines above
the callsite".

The `/ce:review` testing reviewer inverted the guard in a scratch copy
(`[ "$DRY_RUN" = "true" ] || [ "$DRY_RUN" = "1" ]` — stamp on dry run, skip on a
real dispatch) and the suite reported `PASS: 68 FAIL: 0`. The validator
reproduced it. A regression of exactly that shape would have merged clean.

The neighbouring learning
(`extract-the-block-to-make-the-net-testable-2026-08-30.md`) says: extract the
inline net into a function and test the function. That remedy does not reach
this case — **the guard's position is the contract** (after every no-dispatch
exit, before the first mutation), and moving it into a function would move the
thing the order test exists to pin.

## Guidance

**A grep over source text pins the presence of tokens, never what the code
decides.** It is the right tool for *order* ("the callsite is before the fetch")
and the wrong tool for *logic* ("the callsite runs only when not dry-run"). The
two look alike in a test file and fail differently under mutation: an inverted
condition keeps every token in place.

When the block cannot be extracted, **extract it at test time and evaluate it**:

```bash
# Pull the real block out of the production file, by its comment marker
# and its closing `fi` — not a hand-written copy of the condition.
guard_block=$(awk '/mika#2155: claim the ticket for the loop BEFORE/,/^        fi$/' "$DISPATCH_LIB")
assert_contains "extracted the guard block (positive control)" "$guard_block" \
    '_stamp_issue_seat "$REPO" "$ISSUE_NUM" "$LABELS"'

for tc in "true|skip" "1|skip" "false|stamp" "|stamp" "0|stamp"; do
    val="${tc%%|*}"; want="${tc#*|}"
    : > "$SPY_LOG"
    out=$(
        REPO=mika; ISSUE_NUM=2155; LABELS="bug,ready"; DRY_RUN="$val"; ISSUE_SEAT_CLAIMED=0
        _stamp_issue_seat() { printf 'spy %s %s %s\n' "$1" "$2" "$3" >> "$SPY_LOG"; return 0; }
        eval "$guard_block"
        printf 'claimed=%s' "$ISSUE_SEAT_CLAIMED"
    )
    # assert on the spy log and on `out`, per $want
done
```

Three properties make this a pin rather than a second copy of the condition:

1. **The source is the source.** The block comes out of `dispatch-lib.sh` by
   `awk` range; a hand-typed `if` in the test would be a duplicate that drifts
   independently of production.
2. **The extraction has its own positive control.** If the comment marker or
   the closing `fi` moves, `guard_block` is empty and the `assert_contains` on
   the callsite text goes red *before* the loop can pass vacuously on an
   empty `eval`.
3. **The spy replaces the side effect, not the decision.** `_stamp_issue_seat`
   is overridden in a subshell so nothing reaches `gh`; the `if` itself runs
   unchanged, with the real variable names the production block reads.

Keep the structural grep too — but **anchor it on the real line**, not on a
token: `grep -n '^ *if \[ "\$DRY_RUN" != "true" \] && ...'` compared to
`stamp_line - 1`, rather than "any line containing `DRY_RUN` within 8 lines".
The order test and the behavioral test then pin two different things and go
red for two different reasons.

**Run the mutation before believing the suite.** The inverted guard turned
ten assertions red after the change (`T14 DRY_RUN='true' skips the stamp`, …)
against zero before it. A suite whose negative control was never run is a
suite whose greenness has not been measured.

**And commit before mutating.** The mutate-then-restore idiom
(`sed -i …; run; git checkout -- file`) restores the *committed* file: on a
tree with uncommitted work in that file, the restore discards it. It did here,
once, on the very edit under test. Commit the green state first, mutate, run,
`git checkout` — in that order.

## Applicability

Applies to any shell test in this repo that reads `dispatch-lib.sh` (or another
long script) with `grep`/`awk` to assert a property: ask whether the property
is *where* a line sits (grep is right) or *what* a condition decides (grep is
vacuous — evaluate). It does not replace the extract-into-a-function pattern
where the block *can* move; it covers the block whose position is load-bearing.

## Related

- `docs/solutions/best-practices/extract-the-block-to-make-the-net-testable-2026-08-30.md` — the sibling remedy, for blocks that can be extracted
- `docs/solutions/best-practices/a-guard-must-observe-not-assert-2026-08-29.md` — see the guard's test fail before trusting it
- Orchestrator memory `feedback_a_probe_needs_both_controls_in_the_same_call` (mika-platform, not a file in this repo) — the positive control on the extraction is that rule applied to a test fixture
- `skills/bundled/_shared/tests/test_stamp_issue_seat.sh` — T14 and the structural block, mika#2155
