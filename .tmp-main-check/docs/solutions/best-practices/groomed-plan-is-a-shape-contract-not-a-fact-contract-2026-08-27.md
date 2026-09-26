---
module: dev-loop
tags: [grooming, architect-review, plan-premises, security-gate, regex, verification]
problem_type: bug-class-prevention
category: best-practices
---

# A groomed plan is a shape contract, not a fact contract

## Problem

mika#1957 shipped with an architect `Disposition: READY` on the first pass, no iteration
requested. Two of the plan's load-bearing claims were nonetheless wrong, and both would
have shipped had the implementer treated the READY verdict as covering facts:

**Instance 1 — a premise that contradicted the code.** The plan's Tier 1 proposed
removing `"shell-exec"` from `DEFAULT_AGENT_SKILL_ALLOWLIST`, reasoning that
"personal-tier agents never need arbitrary shell execution." Against the code:
`DEFAULT_AGENT_SKILL_ALLOWLIST` backs `AgentTier::Default`, documented in
`crates/mika-common/src/home.rs` as the *operator/platform-owner* persona, where
mika#1641 makes `shell-exec` load-bearing for the orchestrator seat. The family tier the
ticket meant to protect is a different constant that has never contained `shell-exec`,
with a test already asserting the exclusion (mika#1778). Executing Tier 1 would have
regressed a shipped design for zero added coverage.

The premise did not come from nowhere. The merged doctrine doc from the parent ticket
made the same conflation in its own prose — "Personal-tier agents ship with `shell-exec`
in `DEFAULT_AGENT_SKILL_ALLOWLIST`" — and the plan inherited it. **A wrong sentence in a
merged doc propagates into every plan that cites the doc, and survives architect review,
because the reviewer is checking the plan against the doc, not against the code.**

**Instance 2 — a mechanism that did not do what it claimed.** The plan specified a
concrete regex for the security gate and asserted, shape by shape, which bypasses it
caught — including `sh -c "gws gmail ..."`, the shape the whole ticket exists to close.
Measured against a case matrix, the regex failed 4 of 11 block cases, all of them quoted
subshells. Cause: the leading boundary class `[[:space:]|;&`$(]` omits `'` and `"`, and
in every `sh -c '...'` shape the character immediately before the token is a quote, so
the alternation never fires. The plan's own coverage table asserted the opposite, in
prose, with an explanation of why it worked.

## Solution

**Re-derive the plan's load-bearing premises against code before writing any of it.**
Grooming and architect review validate the *shape* of the work — scope, sequencing,
acceptance criteria, risk surface. They do not re-run the plan's factual claims against
the tree, and a `READY` verdict does not assert that they hold. The implementer is the
last position where a false premise is still cheap.

Concretely, before the first edit:

1. **Open every file and constant the plan names** and read the doc comment, not just
   the identifier. A constant called `DEFAULT_*` may be the operator tier.
2. **Check the plan's citations for drift.** Line numbers move; here `home.rs:367` had
   become `:372`. Cheap to fix, and the mismatch is a signal the premise is stale too.
3. **Distrust a premise sourced from prose.** When a plan's rationale traces to a doc
   rather than to code, verify at the code and correct the doc in the same PR.

**Measure a gate's matcher before shipping it.** Any regex, glob, or allowlist that
exists to *block* something gets a case matrix — the shapes it must block, plus the
shapes it must not — run as a script before it goes into the file. Prose reasoning about
a boundary class is not evidence; a boundary class is exactly the kind of detail that
reads correct and behaves wrong. This is the same discipline as
`feedback_verify_pipeline_passes_without_the_fix`, applied one step earlier: verify the
matcher matches before verifying the gate gates.

**Route the overturn, carry the intent.** Instance 1 changed what ships, so it went to
`mika-arch` with the evidence and three named resolutions before any code was written;
the architect chose, and the plan records the ratification. Instance 2 changed only how
an unchanged acceptance criterion is met, so the implementer fixed it and documented the
correction. The split is: **a finding that alters the deliverable routes; a finding that
alters the mechanism carries.**

## Evidence

- mika#1957 plan § Corrections post-grooming, C1 and C2 — the two corrections, with the
  architect session id for the ratified overturn.
- `todos/1957-injection-verification.md` inversion 2 — reverting to the plan's regex
  fails exactly the five quoted-subshell cases, retained as a standing guard.
- Architect first-pass verdict on mika#1957: `READY`, no iteration. Both defects were
  present in the reviewed text.

## Related

- `feedback_implementer_finds_contradiction_architect_chooses_which_resolution_yields` —
  the carry/route split this applies.
- `feedback_verify_pipeline_passes_without_the_fix` — the downstream half of the
  measurement discipline.
- `docs/solutions/best-practices/operator-db-evidence-disconfirmation-when-architect-cant-surface-premise-2026-04-30.md`
  — earlier instance of a premise the architect could not surface.
