---
module: skills/registry, prompt-assembly, tool-execution
tags: [doctrine, non-transit, structural-enforcement, defense-in-depth, prompt-fragility, testimony-grade]
problem_type: behavioral-invariant
category: best-practices
issue: mika#1798
date: 2026-08-22
---

# Baking a behavioral invariant: four composable structural layers, prompt-only never enough

## Context

When Mika's cloud family-tier proposed granting itself Gmail / Calendar / Drive
OAuth access during a family interaction on 2026-07-18, Vincent + Prime ratified
a non-transit data-grade doctrine: **the grade of the data determines Mika's
access, not the convenience of the moment.** Mika may NEVER access nor propose
accessing testimony-grade data (Gmail, full Drive, journals, confessional).

The naive fix — add rules to the system prompt — has a well-documented
substrate failure mode: prompt-only enforcement is empirically fragile (n≥3
substrate hits in `feedback_prompt_enforcement_fragile` and
`feedback_prompt_enforcement_empirically_confirmed_at_loop_substrate`). Under
prompt-injection pressure, model quirk, or future refactor of the prompt
assembly path, a prompt-only invariant silently opens.

## Pattern: four composable structural layers

For behavioral invariants that must hold across every LLM turn / every agent /
every future refactor, ship **all four** of these layers so any single layer
failing does not silently open the invariant surface. This is defense in depth
where **each layer is Rust code**, not prose.

1. **Prompt template** — rendered on every turn at a load-bearing position
   (before core memory, before `## Instructions`). Grounds the *propose*
   surface for every LLM-driven turn. Sole layer that reaches "the model
   didn't verbalize" outcomes; useless for "the model called the tool
   anyway" outcomes.

2. **Skill registry ban (Phase 2)** — evict skills declaring the forbidden
   trait at load time, at every skill-loading callsite. Runs AFTER identity
   allowlist and DB overrides so any policy re-enable is still overridden by
   the ban. No per-agent override surface in v1 — the ban is structural,
   matching "set once" contracts. Wire at every callsite: server init, hot-
   reload handlers (server + a2a), CLI (chat + skills), team engine,
   `delegate_task`, `list_skills` tool, and any test parity site.

3. **Per-tool subcommand ban** — for tools that mix operational + testimony
   surfaces (e.g., `gws` covers Calendar-operational AND Gmail-testimony
   under one skill), the ban lives inside the tool handler where the
   granularity is. Refusal returns structured JSON with a stable
   discriminator (`error = "testimony_grade_forbidden"`) that any consumer
   can pattern-match. Fail-closed on malformed input.

4. **Execute-time guardrail** — pre-dispatch check in the tool dispatcher
   (before builtin/skill/MCP routing). Reads a stateless map at execute
   time from the current registry snapshot, so it closes hot-reload race
   windows, is forward-compatible with dynamic MCP registration, and
   catches DB-override paths that Phase-2 eviction cannot cover. O(1)
   lookup, cloned into per-step dispatch context.

## Why all four (coverage-honesty)

- **Layer 1 alone** — model refuses to propose but happily calls the tool
  anyway if the prompt slips.
- **Layer 2 alone** — silent open in any hot-reload window; forward-
  incompatible with dynamic tool registration.
- **Layer 3 alone** — one-off per tool; every future testimony-touching
  tool needs a new subcommand-ban entry, easy to forget.
- **Layer 4 alone** — depends on the manifest tag being present; a skill
  that touches testimony data but forgets the tag bypasses the check.

The four together cover every "one of them misses" scenario. The doctrine
doc names this explicitly as the **single axis of vigilance** for future
changes: any new testimony-adjacent path MUST either declare
`data_grade = "testimony"` at manifest time (so Layers 2/4 fire) OR add its
own subcommand-ban entry (Layer 3 pattern) OR both. If a future change does
NEITHER, only Layer 1 catches it — the fragile layer the doctrine explicitly
distrusts.

## Structural gates over per-agent policy

The Phase-2 ban runs after both identity allowlist (Phase -1) and DB
overrides (Phase 0/1). This ordering is load-bearing: it means an operator
who explicitly allowlists a testimony skill AND explicitly enables it via DB
override STILL cannot open the doctrine surface. Same as `apply_load_safety_check`
runs after overrides to prevent DB rows from resurrecting broken skills.
"Set once" via a code change is the only opening; runtime override does not
exist in v1.

## Tests that prove the shape

- **Composition test:** `apply_testimony_grade_ban_composes_with_allowlist_and_overrides` —
  fixture with a testimony skill that identity allowlist explicitly includes
  AND DB override explicitly enables. Assert that after all three phases,
  Layer 2 still evicts. This is the load-bearing regression that makes the
  "structural, not policy-configurable" claim testable.
- **Ordering test in Layer 3:** the existing flag-smuggling test on
  `["gmail", ..., "--token", "evil"]` must still return the flag-smuggle
  error, not the doctrine error — this proves the doctrine ban runs AFTER
  flag-smuggling (defense-in-depth ordering: catch smugglers first).
- **Position-stable test in Layer 1:** the doctrine section MUST appear
  before `## Instructions` and before core memory (grounding-first per
  Context priority rule).
- **Compile-time budget assertion:** the compact-prompt variant uses
  `const _: () = assert!(BLOCK.len() < 400)` so any future edit that busts
  the compact budget fails to compile, not at runtime.

## Doctrine doc, not just tests

Ship a doctrine doc (~300 lines) that names:
- Grade taxonomy with worked boundary cases.
- Each layer with file:line pointers.
- Current ban list AND operational carve-outs.
- **Operator override path** ("there is none in v1" — say so explicitly).
- **Vigilance surface** — the single axis of "what to check on every future
  change of this class".
- Cross-refs to sibling tickets / anchoring memories.

The doc is the artifact that survives the LLM's context window across
future work. A test suite proves the current shape holds; the doc explains
which future changes threaten to break it.

## What NOT to do

- **Do not use a runtime env var to disable the ban** ("emergency override").
  Any runtime toggle re-introduces the class of failure the doctrine was
  built against.
- **Do not persist the evicted-skill list on the registry** unless a
  consumer exists (dashboard, CLI subcommand, audit export). Absent a
  reader, the field is API surface without meaning; ship it when the
  reader lands, not speculatively.
- **Do not conflate skill-level ban with subcommand-level ban.** For a
  tool that mixes grades (e.g., `gws` covers calendar-operational AND
  gmail-testimony), the skill-level tag would kill the operational path
  too. Ban at the subcommand handler where the granularity is.

## References

- Founding incident: 2026-07-18 Prime ratification via samidarko relay
  (Vincent-ratified).
- Doctrine doc: `crates/mika-agent/docs/non-transit-data-grade.md`.
- Plan: `docs/plans/2026-08-22-004-invariant-1798-non-transit-doctrine-bake-plan.md`.
- Fragile-prompt memories: `feedback_prompt_enforcement_fragile`,
  `feedback_prompt_enforcement_empirically_confirmed_at_loop_substrate`.
- Prior structural-over-prompt patterns:
  `docs/solutions/best-practices/agent-tool-must-call-apply-load-safety-check-on-skill-registry.md`.
