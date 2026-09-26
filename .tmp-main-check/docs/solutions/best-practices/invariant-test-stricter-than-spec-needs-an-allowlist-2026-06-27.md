---
module: skills
tags: [invariant-test, allowlist, scope-discipline, false-positive, collision-detection, handler-type, bundled-skills]
problem_type: bug-class-prevention
category: best-practices
---

# An invariant test authored stricter than its spec needs a suppression allowlist — relax the test, don't grow the allowlist

## Context

mika#1326 AC2 specified a build-time invariant: no two bundled skills may declare the same
tool name with **different handler types** — the production-risk class where divergent
handlers silently shadow each other via `HashMap` last-write-wins (the 2026-05-28
`run_gh` Builtin-vs-Exec incident).

The dispatched implementer of mika#1569 wrote `test_bundled_skills_no_cross_skill_tool_name_collision`
**stricter than the spec**: it flagged ALL same-name declarations regardless of handler
type, with the rationale "even same-handler-type collisions indicate manifest hygiene."
That stricter framing immediately false-positived on four skills (`gh-read-only` plus the
three `mika-arch-*` skills) that each legitimately declare `gh_read` with an **identical**
Builtin handler — the skill-scoped tool surface model working as designed (same name +
same handler = no last-write-wins risk). To ship mika#1569 green, the author added a
`KNOWN_PRE_EXISTING_COLLISIONS` allowlist with a self-cleaning assertion to suppress the
false-positive, and filed mika#1573 to resolve it.

mika#1573 deleted the allowlist and relaxed the test to AC2's actual spec: group
declarations by `tool_name -> handler_type -> {skills}` and report only tool names with
more than one distinct handler type. A fixture regression test (`run_gh` Builtin-vs-Exec)
proves the original incident class still trips after the relaxation.

## Guidance

**The suppression allowlist was the smell, not the fix.** When a freshly-authored invariant
test needs an allowlist of "known benign exceptions" to pass on the *current, healthy*
tree, the test is almost always stricter than the invariant it was scoped to enforce. The
allowlist is suppressing the test's own over-reach, not a real pre-existing defect. The
correct move is to relax the test to its spec — not to institutionalize the over-strict
version behind a curated exception list that future engineers must maintain.

Diagnostics that distinguish "test is too strict" from "real exception":

- **The exception is the design working as intended.** Four skills sharing an identical
  Builtin `gh_read` handler is the skill-scoped tool surface model, not a bug. If the
  "violation" is a pattern the system *wants*, the test contract is wrong.
- **The allowlist appears in the same PR that introduces the test.** A genuine
  pre-existing exception predates the gate. An exception born alongside the gate is the
  gate mis-scoped on day one.
- **The spec named a narrower class than the test enforces.** AC2 said "different handler
  types"; the test flagged "all same names." Re-read the AC: enforce what it specified,
  not a superset the implementer rationalized as "free extra coverage."

**When you do relax, prove the spec'd class still trips.** Relaxing a too-strict test
risks over-correcting into a vacuous one. Factor the detection into a pure helper
(`detect_divergent_handler_collisions(&[&BundledSkill]) -> Vec<String>`) and add a
fixture-based regression test that feeds it the original incident class (two skills, same
tool name, Builtin vs Exec handlers) and asserts it still fires — plus a negative
companion asserting the benign same-handler case does not. The helper makes the real
invariant test and the regression fixture exercise the *same* code path, so the regression
test is a genuine guard rather than a parallel re-implementation that can drift.

**Keep the detection logic in `#[cfg(test)]`.** mika#1326's freeze-safe subset reserved an
additions-only scope; the collision helper lives inside `#[cfg(test)] mod tests` so the
production skill loader cannot consult it at runtime. Relaxing the test must not leak a
mod-scope re-export.

## Evidence

- `crates/mika-agent/src/bundled_skills.rs` — `test_bundled_skills_no_cross_skill_tool_name_collision`
  + `detect_divergent_handler_collisions` helper + `test_collision_detector_catches_divergent_run_gh_handlers`.
- mika#1573 (this fix); mika#1569 (introduced the over-strict test + allowlist);
  mika#1326 AC2 (the spec); the 2026-05-28 `run_gh` Builtin-vs-Exec incident (the class
  the invariant exists to catch).

## Related

- `detector-built-on-filtered-set-is-blind-to-the-filter.md` — sibling failure mode on the
  same mika#1326/mika#1575 invariant surface (a detector that *under*-fires because its
  input set presupposes the invariant). This doc is the *over*-fires counterpart.
