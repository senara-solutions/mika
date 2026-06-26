---
title: Enforce a validator/producer contract at the gate you control, not the third-party producer
date: 2026-06-26
category: best-practices
module: skills
problem_type: best_practice
component: grooming
severity: high
tags:
  - grooming-gate
  - qa-review
  - mika-arch
  - third-party-plugin
  - contract-mismatch
  - acceptance-criteria
applies_when:
  - A downstream validator hard-fails on a clause an upstream producer never agreed to emit
  - The upstream producer is a third-party / auto-updating dependency you cannot durably edit
  - A live enforcement gate cites an authority (a phase, a section, a spec) that does not exist
  - You are tempted to "fix" a contract mismatch by editing the producer instead of the surface between them
---

# Enforce a validator/producer contract at the gate you control, not the third-party producer

## Context

The mika dev loop has three surfaces in sequence on every dispatched ticket:

- **Producer** — `/ce:plan`, the third-party `compound-engineering` marketplace plugin, writes the plan.
- **Gate** — the `mika-arch` two-pass grooming review (`mika-arch-groom-ticket` → `mika-arch-second-review`) grooms every dispatched ticket (auto-groom-on-dispatch, mika#996).
- **Validator** — `mika-qa` (`qa-review`) reviews the PR and hard-`block[pipeline]`s a plan that lacks a `## Acceptance criteria` section.

The validator demanded a `## Acceptance criteria` section and attributed the requirement to "`/ce:plan` Phase 4.2 — the section is named explicitly." But `/ce:plan` has no Phase 4.2 and never produced that section — its native acceptance model is the *optional* "Acceptance Examples" (AE-IDs). The citation was fabricated, and the producer never agreed to the clause. Four consecutive plans (mika#1531, mika#1533, mika#1557, mika#1558) failed on this phantom contract (mika#1559).

The instinct is to fix the producer: add the section to the `/ce:plan` template. That fix is non-durable — the plugin auto-updates from a marketplace and is not vendored in any senara repo, so a local edit is clobbered on the next update and the bug silently re-opens.

## Guidance

When a downstream validator requires a clause the upstream producer never agreed to provide, **enforce the contract on the surface you control that sits between them** — here, the grooming gate — rather than editing the producer or weakening the validator.

The shape that worked for mika#1559:

1. **First-pass gate = convergence.** `mika-arch-groom-ticket` returns `ITERATE` when the plan lacks a non-empty `## Acceptance criteria` section, with a BLOCKING F-finding telling the author to add it (sourced verbatim from the issue body's AC via `gh_read issue_view`, or derived from requirements when the body has none). The architect is read-only — it *flags*; the existing groomer revise-and-resubmit step *injects*. No new groomer code.
2. **Second-pass gate = structural guarantee.** `mika-arch-second-review` returns `ESCALATE` (never `GROOMED`) when the section is still absent. Because `GROOMED` is the dispatch precondition, every plan that reaches the validator carries the section. The guarantee is the *gate that cannot emit the good verdict*, not prompt-trust in the convergence step — consistent with `feedback_prompt_enforcement_fragile`.
3. **Remove the fabricated authority.** The validator's citation was corrected to reference the real grooming-gate guarantee (mika#1559). The `block[pipeline]`-on-missing-section behavior is *retained* — it is now the final backstop behind the gate, no longer the primary (and falsely-attributed) enforcement point. A live enforcement gate must never cite an authority that does not exist.

Model the new gate on the existing one (DRY): the Acceptance-Criteria Gate mirrors the established Unresolved-Decision Gate (mika#1244) — a named gate, a decision tree, an F-list finding on the terminal disposition. When two such gates can demand different dispositions on the same plan, state a precedence rule (most-blocking disposition wins; emit the union of F-findings).

## Why This Matters

- **Durability.** A fix on a surface you own survives third-party updates; a fix on an auto-updating dependency is a fragment that silently regresses.
- **No fabricated authority in live gates.** A gate that cites a non-existent phase/section trains every downstream reader on a false fact and is impossible to verify. Cite the real enforcement surface.
- **Earlier, named failure.** The failure moves from a late qa-review `block[pipeline]` (after a full pilot run + PR) to an architect `ITERATE`/`ESCALATE` at groom time that names the exact fix — cheaper and more actionable.
- **Structural over prompt-trust.** The load-bearing guarantee is a gate that *cannot* emit the passing verdict without the clause. The prompt only handles the mechanical injection in the convergence step; worst case is a visible `ESCALATE`, never a silent pass.

## When to Apply

- Any time a validator and a producer disagree on a contract and the producer is third-party, auto-updating, or otherwise outside your durable edit surface.
- Any time you find a live enforcement gate citing an authority (a phase number, a spec section, a "named explicitly" clause) — verify the authority exists; if it does not, relocate enforcement to a real surface and correct the citation.
- Prefer enforcing at a gate that already runs on every item in the pipeline (here, auto-groom-on-dispatch), so coverage is automatic rather than opt-in.

## Examples

Before (qa-review § 2.5.2 — fabricated authority):

```
Read the plan's `## Acceptance criteria` section (per `/ce:plan` Phase 4.2 —
the section is named explicitly; ...).
```

After (references the real grooming-gate guarantee, backstop retained):

```
Read the plan's `## Acceptance criteria` section (guaranteed present by the
mika-arch grooming Acceptance-Criteria Gate, mika#1559 — a plan reaches
`Verdict: GROOMED` only with a non-empty section, so this gate is the final
backstop, not the primary enforcement point; ...).
```

The structural guarantee (mika-arch-second-review):

```
A revised plan with no `## Acceptance criteria` section, or an empty one,
MUST return `ESCALATE` — never `GROOMED`.
```

Files changed: `skills/bundled/mika-arch-groom-ticket/system_prompt.md`,
`skills/bundled/mika-arch-second-review/system_prompt.md`,
`skills/bundled/qa-review/system_prompt.md` (plus the prompt-structure test in
`crates/mika-agent/src/calibration/roles/mika_arch.rs`).
