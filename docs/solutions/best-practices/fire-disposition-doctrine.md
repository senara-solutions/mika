---
module: dev-groom
tags: [doctrine, scope-bind, detector, fire-disposition, architect-gate]
problem_type: scope-underspecification
category: best-practices
date: 2026-06-28
ticket: mika#1574
---

# Fire-Disposition Doctrine

## Problem

When a dispatch plan includes a detector-class deliverable (test, assertion, lint, invariant, validation), the plan must specify what happens when the detector fires on **existing** data — pre-existing violations that the new detector now surfaces. Without this specification, the implementing pilot either breaks CI (the detector fires and the test suite goes red) or makes an undirected scope decision (silently allowing the violation).

## Founding incident

mika#1326 → mika#1569 → mika#1573: AC2's `verify_bundled_skills` invariant test caught a benign cross-skill `gh_read` collision on existing bundled-skill data. The scope-bind said "additions-only, don't mutate existing dispatch paths" but did not name the fire-disposition for when the detector caught existing violations. The pilot's strict interpretation produced a failing test.

## Doctrine

Every plan with a detector-class deliverable MUST include a `## Fire-Disposition` section naming one of three canonical options:

### Option (a): Named allowlist exception (default)

The detector enforces for new cases. Each existing violation gets a grep-visible named exception with:
1. **Specific data name** — the exact entity/path/value triggering the exception, not a blanket allowance
2. **Follow-up tracker reference** — a filed issue to fix the underlying violation
3. **Self-cleaning assertion** — the exception entry itself has a test that fires when the follow-up resolves and the exception becomes stale ("remove this entry")

When the allowlist is structural (a data const + test logic), scope it inside `#[cfg(test)] mod tests` so the production loader cannot consult it at runtime.

### Option (b): Land disabled

The detector lands with `#[ignore]`, `#[cfg(skip)]`, or equivalent, plus a tracked follow-up to enable it. Use only when the existing violation is itself dangerous to leave un-flagged and the detector's CI-red state would mask the danger.

### Option (c): Halt-and-surface

The implementation stops and surfaces to the operator for scoping. Use only when the existing violation's resolution shape is itself the operator-scoping question — the plan cannot pre-decide.

## Structural enforcement

The fire-disposition rule is enforced by the **Fire-Disposition Gate** in `mika-arch-groom-ticket` (first-pass) and `mika-arch-second-review` (second-pass). The gate returns ITERATE (first pass) or ESCALATE (second pass) when a plan with detector deliverables lacks the `## Fire-Disposition` section. This gate is the third architect gate, alongside the Unresolved-Decision Gate (mika#1244) and the Acceptance-Criteria Gate (mika#1559).

## Provenance

- `feedback_scope_bind_must_name_fire_disposition` (orchestrator-CC memory, 2026-06-26)
- mika#1574 (this ticket)
- Mika Prime bearing-read 2026-06-26
