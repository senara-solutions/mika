# Plan: fix verify-pipeline.sh AC-heading case-sensitivity (mika#1639 secondary)

## Problem frame (WHY)

`scripts/verify-pipeline.sh` enforces the mika#1600 plan-quality gate with a
**case-sensitive** match on the Acceptance-criteria heading:

- Line ~112: `grep -q '^## Acceptance criteria'` (presence check)
- Line ~117: `sed -n '/^## Acceptance criteria/,/^## /{ ... }'` (non-empty-content check)

A plan that writes the heading in title case (`## Acceptance Criteria`, capital C)
fails the gate even though it has a valid, non-empty AC section. The autonomous-loop
groom command instructs lowercase, but the LLM frequently title-cases the noun
phrase out of natural English habit — so ~50% of autonomous-loop plans fail
Pipeline Artifacts CI on this exact heading-case mismatch.

**Hard evidence (4 PRs hand-fixed today):** #1623 (commit 2bd622ea), #1626
(c9643057), #1628 (8a02bb2a), #1638 (fe3a5b5c) — each required a manual one-character
lowercase edit to the plan heading to pass the gate. That's repeated operator labor
for a one-character regression class with zero value added by the case-sensitivity
(no realistic plan misses the AC concept by writing it in title case).

This is the **secondary** half of mika#1639. The primary half (tier1 policy-deny on
`make verify-bundled-skills`) lives in claude-pilot-py and shipped via
senara-solutions/claude-pilot-py#48.

## Scope boundaries

**In scope:** make the two AC-heading matchers in `scripts/verify-pipeline.sh`
case-insensitive, plus a regression test.

**Out of scope:** anything tier1/permission-policy (that's the cpp primary, done);
the groom command's lowercase instruction (it stays — this fix makes the gate
*tolerant*, it does not change what grooming emits). No other behavior of
verify-pipeline.sh changes.

## Definition of Done

- [ ] `scripts/verify-pipeline.sh` accepts `## Acceptance Criteria` (and any case) at both the presence check and the content-emptiness extraction.
- [ ] A regression test in `scripts/verify-pipeline-test.sh` covers the title-case heading → PASS case.
- [ ] `bash scripts/verify-pipeline-test.sh` is all-green; `bash scripts/verify-pipeline.sh` passes on this branch's own plan.

## Implementation units

### U1. Case-insensitive AC-heading matching in verify-pipeline.sh

**Goal:** title-case `## Acceptance Criteria` passes the mika#1600 gate.

**Files:** `scripts/verify-pipeline.sh` (modify)

**Approach:** two surgical changes in the mika#1600 block (~lines 106-124):
- Presence check: `grep -q '^## Acceptance criteria'` → `grep -qi '^## Acceptance criteria'`.
- Content extraction: add the GNU `I` (case-insensitive) flag to the **start** address only — `sed -n '/^## Acceptance criteria/I,/^## /{ ... }'`. The end address `/^## /` and the inner `/^## /d` are already case-agnostic (they match any `## ` heading prefix, including a title-case AC heading), so only the start address needs `I`.

GNU `grep -i` and GNU `sed` address-`I` are both available on the CI runner (GitHub
Actions Ubuntu) and dev hosts (Gentoo) — verified at plan time. Keep the literal
heading text `Acceptance criteria` unchanged; only the match becomes case-folding.

**Patterns to follow:** the existing block; no structural change.

**Test scenarios:** covered by U2.

**Verification:** `bash scripts/verify-pipeline.sh` passes on this branch (its own
plan uses lowercase, still accepted); a title-case plan also passes (U2).

### U2. Regression test for the title-case heading

**Goal:** lock the case-insensitive behavior.

**Files:** `scripts/verify-pipeline-test.sh` (modify)

**Approach:** add one case after the existing mika#1600 AC cases (~line 450),
mirroring `=== AC check: plan with AC section present → PASS ===` but with a
title-case heading `## Acceptance Criteria` and a non-empty body. Assert PASS
(`Pipeline verification passed`). This exercises both the presence check and the
content-emptiness extraction through the title-case path.

**Test scenarios:**
- A plan with `## Acceptance Criteria` (capital C) + non-empty checkbox body → `verify-pipeline.sh` exits 0 (PASS). *(Covers AC3, AC4)*

**Verification:** `bash scripts/verify-pipeline-test.sh` reports the new case PASS
and zero failures overall.

## Acceptance criteria

- [ ] AC3 — `scripts/verify-pipeline.sh` accepts BOTH `## Acceptance Criteria` and `## Acceptance criteria` as the heading (case-insensitive), for both the presence check and the content-emptiness extraction.
- [ ] AC4 — A plan with title-case `## Acceptance Criteria` and a non-empty body passes the gate (would have unblocked #1623/#1626/#1628/#1638 without the manual lowercase edit), proven by a green regression test.

## Verification contract

1. `bash scripts/verify-pipeline-test.sh` — all cases PASS, including the new title-case case.
2. `bash scripts/verify-pipeline.sh` — passes on this branch (sanity: the gate still accepts a valid plan).

## Sources & research

- mika#1639 — coupled parent ticket (secondary half).
- mika#1600 — the AC-heading gate this fix relaxes.
- mika#1627 — propagated AC-injection to the autonomous groom command (did not eliminate the title-case failures — see evidence).
- senara-solutions/claude-pilot-py#48 — the primary half (tier1 make allowlist).
- `scripts/verify-pipeline.sh:112,117` — the two case-sensitive matchers.
- `scripts/verify-pipeline-test.sh:377-450` — existing mika#1600 AC test cases (pattern to mirror).
