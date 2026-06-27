---
artifact_contract: ce-unified-plan/v1
artifact_readiness: implementation-ready
execution: code
product_contract_source: ce-plan-bootstrap
plan_type: fix
date: 2026-06-27
---

# fix(dispatch-lib): `_find_issue_plan` content-fallback regex misses `**Issue:**` header shape (n=3 bound)

**Issue:** mika#1602

**Target repo:** mika

> **Meta-poetic note (load-bearing).** This plan file deliberately uses the `**Issue:** mika#1602` header shape (not `**Ticket:**`). It is the live, on-branch end-to-end witness for AC1/AC5/AC6: after the regex widening lands, `_find_issue_plan` (with `WORKTREE_DIR` pointed at this branch and `ISSUE_NUM=1602`) must discover *this very file* via the content-fallback pass. On the pre-fix code it does not — that is the bug.

---

## Summary

`_find_issue_plan` in `skills/bundled/_shared/dispatch-lib.sh` is the discovery function the autonomous grooming loop uses to locate the plan a pilot wrote on its branch, so the architect verdict step can review it. Its content-fallback regex accepts only two plan-header shapes — `**Ticket:**` and `ticket:` (YAML). When a pilot legitimately writes `**Issue:** mika#N` (a synonym matching GitHub's own UI), both discovery passes miss, `_iterate_groom_loop` returns 1, the architect is never called, and dispatch reports a misleading `PIPELINE FAILURE: ...Session likely drifted into executor mode` even though the plan-on-branch is correct and complete.

This is the **third bound of one class** (the regex doesn't match a header shape the pilot legitimately produces): n=1 mika#1381 and n=2 mika#771 (both filename-shape gaps, the founding cases for mika#1421's content-fallback), and n=3 mika#1600 (this — filename has no `-1600-` *and* the header is `**Issue:**`, so both passes miss). The fix is a single additive regex widening plus a behavioral regression test. It is fully backward-compatible — the two existing header shapes continue to match.

---

## Problem Frame

**Where it lives.** `skills/bundled/_shared/dispatch-lib.sh::_find_issue_plan` (the fallback `grep -qE` at ~line 1457, inside the `while`-loop that scans `docs/plans/*-plan.md`).

**Current regex:**

```
^(\*\*Ticket:\*\*|ticket:)\s+mika[[:space:]]?(issue)?#${ISSUE_NUM}\b
```

**Two-pass discovery:**
1. **Primary (filename):** `find ... -name "*-${ISSUE_NUM}-*-plan.md"` — requires the issue number embedded in the filename (e.g. `-1600-`). Misses when the plan filename is `2026-06-27-006-fix-...` (the `-006-` is the daily counter, not the issue number).
2. **Fallback (content):** greps the first 20 lines of each plan for the regex above. Misses when the header is `**Issue:** mika#N` rather than `**Ticket:**`.

**Reproduced this session (hard evidence).** Sourcing `dispatch-lib.sh` (verified side-effect-free — function definitions only, no top-level execution) and calling `_find_issue_plan` with `WORKTREE_DIR` pointing at a temp fixture whose plan header is `**Issue:** mika#1600` returns *not found* on current code. The same call after the widening returns the plan path.

**Why the `**Issue:**` shape is reasonable, not a pilot bug.** "Issue" is a synonym for "Ticket" in mika ticket vernacular and matches GitHub's UI ("Issue #1600"). The structural fix — widen the discovery regex — is the load-bearing one; prescriptive prompt-only header standardization is fragile (`feedback_prompt_enforcement_fragile`) and is explicitly out of scope.

---

## Key Technical Decisions

**KTD1 — Widen the union, don't replace it with a loose heuristic (n<5).** Add `**Issue:**` and `issue:` as third and fourth alternation branches, preserving `**Ticket:**` and `ticket:` verbatim. This keeps the header-zone anchoring (`^...`) and the first-20-lines scope that prevent the body-prose false-positive mika#1421's v1 self-test hit (a plan quoting another ticket's header on line 49). A general "any line containing `mika#N`" heuristic is deferred to the Tier-2 escalation trigger at n≥5 (see Failure-disposition) — at n=3 the bounded union is correct and lower-risk.

**KTD2 — Keep the `(issue)?` token in place.** The existing `mika[[:space:]]?(issue)?#${ISSUE_NUM}` already tolerates `mika issue#N` and `mika#N` in the *reference* portion. The widening only adds *header-prefix* alternatives; the reference grammar is unchanged, so `**Issue:** mika issue#N`, `**Issue:** mika#N`, `issue: mika#N` all match.

**KTD3 — Test behaviorally by sourcing, not by grepping the script text.** The existing harness mixes structural assertions (grep the script) with behavioral ones (the `_parse_prompt` replication at the file tail). For `_find_issue_plan`, source `dispatch-lib.sh` and invoke the real function against temp fixtures — this is the faithful proof of AC4 ("fails on current code, passes after widening") and avoids re-encoding the regex in the test (which would let a regex typo pass both).

---

## Implementation Units

### U1. Widen `_find_issue_plan` content-fallback regex + refresh its comment

**Goal:** Accept `**Issue:**` and `issue:` header shapes in the content-fallback pass without regressing the two existing shapes.

**Requirements:** AC1, AC2, AC3 (AC3 is untouched-by-construction — the primary filename pass is not modified).

**Dependencies:** none.

**Files:**
- `skills/bundled/_shared/dispatch-lib.sh` (modify) — the `grep -qE` regex at ~line 1457 and the comment block at ~lines 1440-1444 documenting the accepted shapes.

**Approach:**
- Replace the alternation `(\*\*Ticket:\*\*|ticket:)` with `(\*\*Ticket:\*\*|\*\*Issue:\*\*|ticket:|issue:)`. Everything after the alternation (`\s+mika[[:space:]]?(issue)?#${ISSUE_NUM}\b`) is unchanged.
- Final regex:
  ```
  ^(\*\*Ticket:\*\*|\*\*Issue:\*\*|ticket:|issue:)\s+mika[[:space:]]?(issue)?#${ISSUE_NUM}\b
  ```
- Update the comment block (the "Pattern handles three shapes" prose) to list four shapes: `**Ticket:** mika#N`, `**Issue:** mika#N`, `ticket: mika#N`, `issue: mika#N`. Add a one-line provenance note citing mika#1602 (n=3) the way the existing comment cites mika#1421 (n=2).

**Patterns to follow:** the existing comment-then-code convention in the function; the mika#1421 provenance annotation style already present at lines ~1411-1422.

**Test scenarios:** behavioral coverage lives in U2 (the function is exercised end-to-end there). No standalone scenario for U1 beyond U2's fixtures.

**Verification:** `grep -n 'Issue:' skills/bundled/_shared/dispatch-lib.sh` shows the widened alternation; the U2 test suite passes.

### U2. Add behavioral regression tests for `_find_issue_plan` header-shape discovery

**Goal:** Prove AC1–AC4: the `**Issue:**` shape is now discoverable, the two legacy shapes still match, and the filename-primary pass still matches — via real function calls, not script-text greps.

**Requirements:** AC1, AC2, AC3, AC4.

**Dependencies:** U1 (tests must pass against the widened regex; they are authored to FAIL against pre-U1 code for the `**Issue:**` case).

**Files:**
- `skills/bundled/_shared/test-dispatch-lib.sh` (modify) — append a new test block ("Test N: `_find_issue_plan` header-shape discovery").

**Approach:**
- Source `dispatch-lib.sh` inside a subshell helper (the file is verified safe to source — no top-level execution). Build temp `docs/plans/` fixtures with `mktemp -d`; each fixture is a >500-byte plan (satisfy the mika#1033 size filter) with the target header on an early line and a filename slug chosen to control which pass fires.
- Use a small wrapper that sets `WORKTREE_DIR` and `ISSUE_NUM` per-case and captures `_find_issue_plan`'s stdout + exit code, asserting via the harness's existing `assert_eq` / `assert_contains`.
- Clean up each temp dir after its assertions.

**Patterns to follow:** the `_parse_prompt` behavioral block at the tail of `test-dispatch-lib.sh` (define-and-call a function, assert on its output) and the `assert_eq` / `assert_contains` helpers at the file head.

**Test scenarios:**
- **AC1 / happy path (the n=3 case):** fixture `2026-06-27-006-fix-unrelated-slug-plan.md` (no `-1602-`), header `**Issue:** mika#1602` on line 3, body padded >500 bytes. `WORKTREE_DIR`=tmp, `ISSUE_NUM=1602` → function returns the fixture path, exit 0. **This case must fail on pre-U1 code and pass after.**
- **AC1 variant — `issue:` YAML:** fixture with `issue: mika#1602` in frontmatter within the first 20 lines → found.
- **AC2 regression — `**Ticket:**`:** fixture with `**Ticket:** mika#771`, unrelated filename, `ISSUE_NUM=771` → found (no regression).
- **AC2 regression — `ticket:` YAML:** fixture with `ticket: mika#771` frontmatter → found.
- **AC3 regression — filename primary pass:** fixture `2026-06-06-003-fix-1407-something-plan.md` with NO matching content header, `ISSUE_NUM=1407` → found via the primary filename pass (proves U1 left the primary pass intact).
- **Negative — wrong issue number:** fixture `**Issue:** mika#9999`, `ISSUE_NUM=1602` → not found (exit non-zero), guarding against an over-broad match.
- **Negative — header below the 20-line zone:** fixture with `**Issue:** mika#1602` on line 30+ and unrelated filename → not found (proves the header-zone scope survived).

**Verification:** `bash skills/bundled/_shared/test-dispatch-lib.sh` exits 0 with all new assertions passing; checking out the pre-U1 `dispatch-lib.sh` makes the AC1 happy-path assertion fail (the FAIL-before / PASS-after witness for AC4).

---

## Scope Boundaries

**In scope:** the single regex widening (U1) and its behavioral regression tests (U2), both in `skills/bundled/_shared/`.

### Deferred to Follow-Up Work
- None for this PR.

### Outside this change
- **Pilot plan-header standardization** (making pilots always emit `**Ticket:**`). Prompt-only enforcement is fragile (`feedback_prompt_enforcement_fragile`); the structural regex widening is the load-bearing fix.
- **`/ce:plan` plugin template edits** — clobbered on marketplace update (established by mika#1585).
- **The misleading "executor mode" diagnostic wording** in dispatch-lib's `PIPELINE FAILURE` message — a real but separate readability fix; defer to a follow-up.

---

## Verification Contract

1. `bash skills/bundled/_shared/test-dispatch-lib.sh` → exits 0; all pre-existing assertions still pass; the new header-shape block passes.
2. FAIL-before witness: against the unmodified `dispatch-lib.sh`, the new AC1 happy-path assertion fails (proves the test has teeth — AC4).
3. End-to-end witness (AC1/AC5/AC6): sourcing the *widened* `dispatch-lib.sh` and calling `_find_issue_plan` with `WORKTREE_DIR` = this branch's checkout and `ISSUE_NUM=1602` returns this plan file's path — discovered via the content-fallback `**Issue:**` branch.
4. No regression to the primary filename pass or the two legacy content shapes (AC2, AC3) — covered by U2's regression fixtures.

---

## Definition of Done

- U1 and U2 landed; `test-dispatch-lib.sh` passes; the FAIL-before / PASS-after witness holds.
- The function comment documents all four accepted header shapes with mika#1602 provenance.
- PR opened against `senara-solutions/mika` from `fix/1602/dispatch-lib-find-issue-plan-content` with `Closes #1602`.

---

## Acceptance criteria

- [ ] **AC1.** `_find_issue_plan` returns the plan path when the plan header has `**Issue:** mika#${ISSUE_NUM}` on a line within the first 20 lines, regardless of filename shape.
- [ ] **AC2.** The existing two header shapes (`**Ticket:** mika#N` and `ticket: mika#N` YAML frontmatter) continue to match (no regression).
- [ ] **AC3.** A plan with the issue number in the **filename** continues to match via the primary pass (no regression to the mika#1421 founding case).
- [ ] **AC4.** `test-dispatch-lib.sh` gains a new test case asserting that a plan with `**Issue:** mika#NNN` on line 3 + an unrelated filename slug is found by `_find_issue_plan`. The test fails on current `dispatch-lib.sh` and passes after the regex widening.
- [ ] **AC5.** End-to-end: a fresh `ready` label on any substrate-shaped ticket where the pilot writes `**Issue:** mika#N` (rather than `**Ticket:**`) reaches the architect verdict step, not `PIPELINE FAILURE`.
- [ ] **AC6.** mika#1600 dev-groom can be re-dispatched (or its existing branch advanced) and completes the architect-verdict step without further code change in this PR — the existing plan at `8676c46d` is now findable.

---

## Failure-disposition

**Detector:** `_find_issue_plan`'s content-fallback regex (the widened union). When a pilot-written plan uses a header shape outside the union, the fallback misses → `PIPELINE FAILURE` with the "executor mode" diagnostic.

**Tier 1 remediation (n<5):** operator files an additive widening ticket (this shape — single regex edit + test fixture). Current iterative-widening doctrine; correct while the header-shape class is small and bounded.

**Tier 2 escalation trigger (n≥5):** if the header-shape class reaches n≥5 distinct shapes, replace the fixed union with a general heuristic (scan first 20 lines for any line containing `mika#${ISSUE_NUM}` regardless of prefix), accepting the body-quote false-positive risk with a compensating guard. The n≥5 threshold is the operator's judgment call — named here so the pattern is observable, not so it auto-fires.
