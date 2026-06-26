---
issue: mika#1559
type: fix
branch: fix/1559/grooming-ce-plan-template-missing
title: "Grooming-gate guarantee for the plan `## Acceptance criteria` section"
date: 2026-06-26
---

# mika#1559 — Grooming-gate guarantee for the plan `## Acceptance criteria` section

## Summary

mika-qa hard-`block[pipeline]`s any plan that lacks a `## Acceptance criteria`
section, citing a non-existent "/ce:plan Phase 4.2." `/ce:plan` is a third-party
marketplace plugin that never produced that section. Four consecutive plans
(mika#1531, #1533, #1557, #1558) failed on this phantom contract. The ratified
fix (approach **(c)**, operator-confirmed 2026-06-26) enforces the acceptance-
criteria contract on a surface we control — the **mika-arch grooming gate** —
so every plan that reaches `Verdict: GROOMED` already carries the section, and
removes the fabricated citation from the qa-review gate.

## Problem Frame

**The bug is a contract mismatch between a plan-producer and a plan-validator.**

- **Validator (ours):** `skills/bundled/qa-review/system_prompt.md` § 2.5.2 reads
  the plan's `## Acceptance criteria` section and emits `block[pipeline]` when it
  is missing or empty — and attributes the requirement to "`/ce:plan` Phase 4.2 —
  the section is named explicitly."
- **Producer (third-party):** `/ce:plan` is the `compound-engineering` marketplace
  plugin (`~/.claude/plugins/marketplaces/compound-engineering-plugin/skills/ce-plan/`).
  Its hard-floor sections are Summary / Problem Frame / Requirements / KTDs /
  Implementation Units. Its native acceptance model is **optional** "Acceptance
  Examples" (AE-IDs) — a different concept. **It has no `## Acceptance criteria`
  section and no "Phase 4.2."** The citation is fabricated.
- **Why the producer cannot be fixed:** the plugin auto-updates from a marketplace
  and is not vendored in any senara repo. A local edit is clobbered on the next
  update — a non-durable fix that silently re-opens the bug.

The grooming pipeline sits between producer and validator. Making **grooming**
guarantee the section (and removing the fabricated citation) restores a real
contract entirely within our control surface. Because auto-groom-on-dispatch
(mika#996) grooms every dispatched ticket, this covers the dispatch path.

## Constraints / Key Technical Decisions (KTDs)

- **KTD-1 — mika-arch is read-only.** Both `mika-arch-groom-ticket` and
  `mika-arch-second-review` declare "No file write tools" (groom-ticket line 117,
  second-review line 112). The architect therefore **flags** a missing section; it
  does not write it. Injection is performed by the **groomer agent** acting on the
  architect's `ITERATE` finding (the groomer is an LLM session with file access).
- **KTD-2 — structural backstop, not prompt-trust.** The load-bearing guarantee is
  the **second-pass gate**: `Verdict: GROOMED` is structurally blocked when the
  plan lacks the section (absent ⇒ `ESCALATE`). qa-review's `block[pipeline]`
  remains the final backstop. The first-pass `ITERATE` + groomer-injection is the
  convergence mechanism, not the guarantee. Worst case is a visible `ESCALATE`,
  never a silent pass — consistent with `feedback_prompt_enforcement_fragile`
  (gates are structural; the prompt only handles the mechanical injection).
- **KTD-3 — single-repo scope.** Enforcing at the grooming gate keeps the entire
  fix inside the `mika` repo (three bundled-skill prompt files). The mika#1531/#1533/
  #1557/#1558 failures are satisfied without a cross-repo change, because a plan is
  only ever GROOMED with the section present — so it reaches qa-review already
  compliant (AC3 is satisfied at GROOMED time, not by an extra qa round).
- **KTD-4 — model the new gate on the existing one.** The Acceptance-Criteria Gate
  mirrors the "Unresolved-Decision Gate (mika#1244)" already present in both
  prompts: a named gate, a decision tree, and an F-list finding on the terminal
  disposition. DRY with the established pattern (review-guide.md § DRY).
- **KTD-5 — the injector is the existing groom iterate loop; no new code (mika-arch F1).**
  "The groomer" is whichever session is driving the groom: the autonomous
  `dev-groom` claude-pilot running `/mika-groom-plan-only` (auto-groom-on-dispatch,
  mika#996) or the orchestrator running `/mika-groom-ticket` interactively. When the
  first-pass gate returns `ITERATE`, that session adds the `## Acceptance criteria`
  section during its **existing** revise-and-resubmit step (`/mika-groom-ticket`
  Phase 4 steps 11–12: "Update the plan to address each architect concern"). This is
  the identical mechanism already used for Unresolved-Decision-Gate ITERATE findings
  — **no new groomer logic is implemented in this ticket.** The architect's F-finding
  carries the issue body's AC verbatim (via `gh_read` `issue_view`), so the groomer
  transcribes rather than invents. This keeps KTD-3's single-repo scope intact.

## Implementation Units

### U1 — Acceptance-Criteria Gate in `mika-arch-groom-ticket` (first pass)

**File:** `skills/bundled/mika-arch-groom-ticket/system_prompt.md`

Add a new gate section immediately after "### Unresolved-Decision Gate (mika#1244)"
(after line 55), titled **"### Acceptance-Criteria Gate (mika#1559)"**:

- State the rule: *A plan with no `## Acceptance criteria` section, or with that
  section present but empty, MUST return `ITERATE` — never `READY`.*
- Decision tree (complete — covers the source-absent branch per mika-arch F2):
  1. Plan has a non-empty `## Acceptance criteria` section ⇒ gate passes.
  2. Section missing/empty AND the issue body has an acceptance-criteria section ⇒
     return `ITERATE` with a BLOCKING F-finding instructing the author to add a
     `## Acceptance criteria` section sourced from the issue body's AC (the architect
     uses `gh_read` `issue_view` to quote the body's AC so the finding names the exact
     criteria to transcribe).
  3. Section missing/empty AND the issue body has NO acceptance-criteria section ⇒
     still return `ITERATE`, but the F-finding instructs the author to **derive**
     concrete, testable acceptance criteria from the issue body's requirements / the
     plan's own Implementation Units (the section is mandatory regardless of issue-body
     shape).
  4. Section missing/empty AND the ticket is so underspecified that no testable
     criteria can be derived from either the issue body or the plan ⇒ this is a genuine
     operator-input gap: return `ESCALATE` naming the missing acceptance definition
     (an unresolved decision outside architect authority).
- One-line rationale: the downstream qa-review gate hard-blocks on this section;
  guaranteeing it at groom time is what prevents the mika#1531/#1533/#1557/#1558
  `block[pipeline]` failure class.

### U2 — Acceptance-Criteria Gate in `mika-arch-second-review` (second pass)

**File:** `skills/bundled/mika-arch-second-review/system_prompt.md`

Add a parallel gate after "### Unresolved-Decision Gate (mika#1244)" (after line 58),
titled **"### Acceptance-Criteria Gate (mika#1559)"**:

- State the rule: *A revised plan with no `## Acceptance criteria` section, or an
  empty one, MUST return `ESCALATE` — never `GROOMED`.* (No ITERATE exists at second
  pass per the two-pass limit, line 76.)
- Decision tree:
  1. Non-empty `## Acceptance criteria` section present ⇒ gate passes.
  2. Missing/empty ⇒ `ESCALATE` with a BLOCKING F-finding (the section was required
     by the first-pass gate and remains absent).
- This is the structural guarantee (KTD-2): `GROOMED` cannot be emitted without the
  section, so every dispatched plan carries it.

### U3 — Remove the fabricated citation in `qa-review`

**File:** `skills/bundled/qa-review/system_prompt.md` § 2.5.2 (the line currently
reading "Read the plan's `## Acceptance criteria` section (per `/ce:plan` Phase 4.2
— the section is named explicitly; …)").

- Replace the parenthetical "per `/ce:plan` Phase 4.2 — the section is named
  explicitly" with a truthful reference to the grooming-gate guarantee, e.g.:
  "(guaranteed present by the mika-arch grooming Acceptance-Criteria Gate, mika#1559;
  bullets are markdown checkbox items: `- [ ] <criterion>` or `- [x] <criterion>`)".
- Do **not** weaken the gate itself: the `block[pipeline]`-on-missing-section
  behavior (§ 2.5.2, the "no `## Acceptance criteria` section OR the section is
  empty" branch) is retained — it is now the final backstop behind the grooming gate,
  no longer the primary (and falsely-attributed) enforcement point.

## Scope Boundaries

### Out of scope
- **Editing the third-party `/ce:plan` plugin template.** Non-durable (auto-updates,
  not vendored). The original AC1 is superseded by the grooming-gate guarantee.
- **Realigning qa-review to CE-native "Acceptance Examples"/AE-IDs (Alt-2).**
  Considered and not chosen; the `## Acceptance criteria` contract is kept.
- **Proactive injection in the meta-repo groom commands** (`/mika-groom-ticket`,
  `/mika-groom-plan-only`). An optimization that would let the *first*-pass plan
  carry the section without an iterate round. Not required for correctness — the
  first-pass ITERATE + groomer-injection already converges, and AC3 is met at
  GROOMED time. Deferred to follow-up only if measured grooming-round overhead
  justifies it.

### Deferred to Follow-Up Work
- If telemetry later shows the systematic extra first-pass `ITERATE` round is a
  meaningful grooming-latency cost (loop-speed tier), file a follow-up to add a
  post-`/ce:plan` "ensure `## Acceptance criteria` present, sourced from issue body"
  step to the meta-repo groom commands.

## System-Wide Impact

- Prompt-only changes to three engine-coupled bundled skills; they ship atomically
  with the engine via build-time discovery (`crates/mika-agent/build.rs`). No Rust,
  no schema, no migration.
- Behavioral effect: ungroomed-or-AC-less plans now surface `ITERATE`/`ESCALATE` at
  the architect gate instead of failing later at qa-review `block[pipeline]`. The
  failure moves earlier and names the exact fix.

## Acceptance criteria

- [ ] `mika-arch-groom-ticket/system_prompt.md` contains an "Acceptance-Criteria Gate
  (mika#1559)" that returns `ITERATE` when the plan lacks a non-empty
  `## Acceptance criteria` section, with an F-finding instructing the author to
  source it from the issue body's AC
- [ ] `mika-arch-second-review/system_prompt.md` contains an "Acceptance-Criteria Gate
  (mika#1559)" that returns `ESCALATE` (never `GROOMED`) when the revised plan lacks a
  non-empty `## Acceptance criteria` section
- [ ] `qa-review/system_prompt.md` § 2.5.2 no longer cites the non-existent "`/ce:plan`
  Phase 4.2"; the AC-section reference points at the mika#1559 grooming-gate guarantee,
  and the existing `block[pipeline]`-on-missing-section behavior is retained
- [ ] No change is made to the third-party `/ce:plan` plugin template, and the
  out-of-scope rationale is documented in the plan
- [ ] After this change, a plan groomed through the mika-arch two-pass flow reaches
  `Verdict: GROOMED` only with a non-empty `## Acceptance criteria` section present —
  so it passes the qa-review acceptance-criteria check on first submission

## Test Expectation

Prompt-only change to three bundled-skill system prompts — there is no Rust unit
to assert against. Verification is structural-by-inspection plus behavioral:
- **Inspection:** each of the three files contains the specified gate/citation text.
- **Behavioral (dogfood):** this very plan carries a `## Acceptance criteria` section,
  and the next ticket groomed after this ships should pass qa-review's AC check on
  first submission. If a bundled-skill prompt-structure test exists in the
  build/test suite, extend it to assert the gate heading is present; otherwise
  `Test expectation: none -- prompt-only skill change, verified by inspection + first
  post-merge groom`.

## Sources & Research

- `skills/bundled/qa-review/system_prompt.md` § 2.5.2 — the gate + fabricated citation
- `skills/bundled/mika-arch-groom-ticket/system_prompt.md` — first-pass gate pattern
  (Unresolved-Decision Gate, lines 37–55; read-only constraint, line 117)
- `skills/bundled/mika-arch-second-review/system_prompt.md` — second-pass gate pattern
  (lines 41–58; two-pass limit, line 76; read-only, line 112)
- `~/.claude/plugins/marketplaces/compound-engineering-plugin/skills/ce-plan/` — the
  third-party producer (hard-floor sections; optional Acceptance Examples)
- mika#996 — auto-groom-on-dispatch (every dispatched ticket is groomed)
- `feedback_prompt_enforcement_fragile` — structural gate vs. prompt-trust (KTD-2)
- Operator ruling 2026-06-26 — approach (c) over Alt-1 (vendor)/Alt-2 (realign)
