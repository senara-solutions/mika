---
title: "Sibling skill for aggregate scope — don't branch on input shape inside a single skill"
date: 2026-04-29
category: best-practices
module: mika-arch
problem_type: best_practice
component: tooling
severity: medium
applies_when:
  - An existing skill handles one input shape (e.g., single ticket) and needs to handle an aggregate shape (e.g., milestone with sub-issues)
  - The aggregate shape produces a different output contract (additional sections, different disposition semantics)
  - Prompt-based input-shape branching would add complexity the LLM can ghost under load
tags:
  - mika-arch
  - bundled-skill
  - sibling-skill
  - milestone-grooming
  - output-contract
  - prompt-discipline
---

# Sibling skill for aggregate scope — don't branch on input shape inside a single skill

## Context

mika-arch's `mika-arch-groom-ticket` skill handles single-issue plan review: it reads one plan, fetches one issue, and emits `Disposition: READY|ITERATE|ESCALATE` as the literal final line. When milestone#19 required grooming all four sub-issues as a unit, the natural question was: extend the existing skill to detect `milestone#N` input, or add a sibling?

Three compound docs converged against extending:
- `prompt-rule-cheapness-bias-toward-wrong-layer-2026-04-28.md` (N=9): adding input-shape detection is the cheap-but-wrong default
- `required-tools-gate-evasion-patterns-2026-04-28.md` (N=2): mika-arch ghosts its own catalogue under load — shape-detection rules are where drift concentrates
- `operator-only-bundled-skill-structural-enforcement-2026-04-28.md`: the sibling-skill template is already in production (dev-groom)

## Guidance

**One skill = one output contract.** When a new scope (milestone, project, sprint) produces a structurally different output (per-sub-issue summaries + sequencing record + cross-cutting concerns + aggregate disposition), create a sibling skill rather than branching inside the existing prompt.

The implementation checklist for a new mika-arch sibling skill:

1. **Skill scaffold** — `skills/bundled/<name>/` with `skill.toml`, `system_prompt.md`, `tools.json`. Mirror the existing sibling's structure. Keep `required_suffix_lines` consistent with the disposition vocabulary.

2. **4 allowlist sites in `well_known_agents.rs`** — per `rename-bundled-skill-touchpoints-and-gates-2026-04-28.md`:
   - `MIKA_DEV.disabled_skills`
   - `MIKA_QA.disabled_skills`
   - `MIKA_RELAY.disabled_skills`
   - `build_mika_arch_identity()` allowlist array

3. **LLM override** — add a `LlmOverrideSpec` entry matching the review-class model (Opus 4.7 for first-pass review skills).

4. **MIKA_ARCH_SOUL** — one-line addition referencing the new scope. Don't restructure; minimal citation per `core-memory-as-citation-not-accumulator-2026-04-28.md`.

5. **Test updates** — update all assertions on counts: `test_mika_arch_has_llm_overrides`, `test_mika_arch_identity_toml_has_allowlist_and_disabled_tools`, `test_seed_skill_overrides_mika_arch`, `test_well_known_agent_specs_dev_qa_no_overlap` (allowed_overlap list).

6. **Doc comment on `MIKA_ARCH`** — update the Rust doc comment listing enabled skills.

7. **CLAUDE.md updates** — bump bundled skill count, add to directory listing.

## Why This Matters

The load-bearing argument is about **output contracts, not input shapes**. Per-ticket grooming emits one plan + one disposition. Milestone grooming emits N plans + a sequencing record + cross-cutting concerns + an aggregate disposition. These are two distinct output contracts. A single skill emitting two contracts under input-shape branching is the failure mode — the LLM has N=2 evidence of ghosting its own catalogue boundaries under load, and the boundary between two output contracts is exactly where this drift concentrates.

The sibling approach also preserves the `required_suffix_lines` engine guard (mika#864): each skill declares its own accepted set, and the guard enforces it per-skill. A single skill with two output shapes would need the guard to be context-aware of which shape was active — complexity the engine doesn't support and shouldn't need to.

## When to Apply

- Adding milestone/project/sprint scope to an existing per-ticket skill
- Adding any aggregate scope where the output includes sections the single-item flow doesn't produce
- When the LLM's output contract under the new scope has different required sections than the existing scope

Do **not** apply when:
- The new input is just a format variant of the same scope (e.g., issue URL vs issue number — same output contract)
- The output contract is identical regardless of input shape

## Examples

**mika#879 (this PR):** Added `mika-arch-groom-milestone` as a sibling to `mika-arch-groom-ticket`. The milestone skill receives a consolidated brief of all sub-issue plans and produces: per-sub-issue disposition summary, sequencing assessment, cross-cutting concerns, and aggregate `Disposition:` — a different output contract from the per-ticket skill's single-plan review.

**dev-groom (mika#845):** Added as a sibling to self-dev rather than extending self-dev with operator-triggered grooming. Same pattern: different output contract (grooming phases) from self-dev's dispatch contract.

## Related

- `docs/solutions/best-practices/operator-only-bundled-skill-structural-enforcement-2026-04-28.md` — the sibling-skill template precedent
- `docs/solutions/best-practices/rename-bundled-skill-touchpoints-and-gates-2026-04-28.md` — the 4-site propagation checklist
- `docs/solutions/best-practices/prompt-rule-cheapness-bias-toward-wrong-layer-2026-04-28.md` — why input-shape branching is the wrong layer
- `docs/solutions/best-practices/required-tools-gate-evasion-patterns-2026-04-28.md` — evidence of ghosting under load
- senara-solutions/mika#879 — implementation PR
- senara-solutions/mika#845 — dev-groom precedent
