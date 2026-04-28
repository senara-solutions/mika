---
title: "feat: Reflection-pass spec for core-memory accretion enforcement"
type: feat
status: active
date: 2026-04-28
ticket: senara-solutions/mika#868
branch: feat/promotion-protocol-prompts-and-reflection-spec
origin: senara-solutions/mika#866 (parent ship — landed the policy compound doc)
---

# feat: Reflection-pass spec for core-memory accretion enforcement

## Process Note (load-bearing)

This plan was reviewed externally per the recursive-self-review carve-out (`docs/solutions/best-practices/recursive-self-review-carve-out-2026-04-26.md`). The original draft included a parallel sub-task — `## Memory promotion protocol` prompt additions to `MIKA_DEV_SOUL` and `MIKA_ARCH_SOUL` constants — which the external reviewer cut. The cut rationale is load-bearing for the audit trail and is recorded in § "Alternatives Rejected" below.

This is instance #2 of the recursive-self-review carve-out (mika#818 was instance #1). Per the carve-out's "When to revisit" criterion, formal codification in `docs/architecture/review-guide.md` is warranted at instance #3. The codification-prep ticket is filed as a separate issue (not a sub-task here) per the external reviewer's recommendation: pre-file at N=2 so that when N=3 hits, the codification language and prior-instance evidence are already assembled. See § "Documentation / Operational Notes" for the residual action item.

The audit trail callout for the issue body: `> - **Architect verdict:** GROOMED (Claude Chat / Mika project external review per the recursive-self-review carve-out — instance #2)`.

## Overview

Today (2026-04-28), mika#866 + mika#867 shipped two prior PRs in this thread:
- **mika#866** (merged) — moved mika-arch's foundational citation list from `current_priorities` core memory into the `MIKA_ARCH_SOUL` constant; added the policy compound doc `docs/solutions/best-practices/core-memory-as-citation-not-accumulator-2026-04-28.md` introducing the three-way filter (Bucket 1 existing-artifact / Bucket 2 N≥2-recurrence-promote / Bucket 3 N=1-keep-with-recurrence-watch).
- **mika#867** (merged) — /ce:review fixes on #866 (P0 broken citation `docs/architecture/north-star.md` → `docs/design/north-star.md`, mika-arch test additions, plan rename, missing citations).

mika#868 ships **a design spec for the runtime enforcement** of that three-way-filter policy: a new doc at `docs/architecture/core-memory-promotion-protocol.md` describing a reflection-pass that surfaces core-memory promotion candidates by bucket assignment during `SilentTrigger::Reflection` turns. **This plan SPECs the reflection-pass design — implementation is deferred to a sibling ticket.** No runtime code changes in this PR.

The original plan included a parallel prompt-level layer (a `## Memory promotion protocol` section added to `MIKA_DEV_SOUL` and `MIKA_ARCH_SOUL`) framed as bridge scaffolding. External review cut that layer entirely; the policy lives where the enforcement lives (the runtime surface), not duplicated as a static prompt section that drifts when the policy evolves. See § "Alternatives Rejected" for the full reasoning.

## Problem Frame

Today's audit (`core-memory-as-citation-not-accumulator-2026-04-28.md` § "Applied To Today's Audit", lines 91–110) found:

- **mika-dev `self_model`** at 471/500 tokens, with 7 incident-derived behavioral rules accreted over 13 days. 5 of 7 rules already had `soul.md` duplicates; 5 of 7 had compound-doc artifacts already. Block was hoarding paraphrased duplicates.
- **mika-arch `current_priorities`** at 372/500 tokens, with 5 items each tagged "promote to ticket on real pressure" or "Compound doc pending one more cycle" — explicit promotion triggers in the rule text itself, none of which fired.

Without runtime enforcement of the three-way filter, the next compaction cycle re-accretes. The 500-token cap surfaces the pressure, but compress-to-fit is the trained reflex; the agent doesn't promote. mika#866's audit was operator-driven (Vincent triggered it). The reflection-pass spec defines a deterministic engine-side scan that automates the surfacing so accretion gets surfaced, not just compressed.

The reflection-pass operates engine-side, surfacing candidates as a `<core-memory-promotion-candidates>` block during `SilentTrigger::Reflection` turns. The agent reads the surfaced candidates and acts on them. The policy citation (`core-memory-as-citation-not-accumulator-2026-04-28.md`) is included *in the surfaced block*, not duplicated as a prompt-level rule. That's the as-above-so-below pattern done right: policy lives where enforcement lives.

This plan defines the spec; implementation is a sibling ticket TBD.

## Requirements Trace

- **R1.** Reflection-pass spec doc lands in `docs/architecture/` describing trigger conditions, scan algorithm, surface mechanism, test fixtures. Doc explicitly notes implementation is a separate ticket.
- **R2.** Plan doc on the dispatch branch under `docs/plans/` covers WHY (citing this ticket and the compound doc), scope/out-of-scope, alternatives rejected, implementation steps, verification, risk + rollback. (This file.)
- **R3.** PR body links: this ticket, mika#866 (the policy ship), mika#867 (the bug-fix follow-up), the relevant compound doc, and the codification-prep ticket filed alongside this PR.

## Scope Boundaries

- No changes to `MIKA_DEV_SOUL`, `MIKA_ARCH_SOUL`, or any other agent soul/identity constant. The original draft's `## Memory promotion protocol` prompt sections are explicitly NOT part of this PR (see § "Alternatives Rejected").
- No runtime code changes. Spec-only PR.
- No new tests in this PR. Test scenarios are *defined* in the spec for the implementation ticket; no fixtures shipped here.

### Deferred to Separate Tasks

- **Reflection-pass implementation** — sibling ticket TBD (filed post-merge of this PR). Implementation realizes the spec this plan delivers: db.rs detector, agent.rs trigger gate, prompt.rs XML emission, paired tests. Specs first because the spec is reviewable independently of the runtime code; the carve-out path validates the design before dev cycles burn on the wrong shape.
- **Structural write-time guard at `update_core_memory`** — engine ticket TBD. The eventual layer-1 enforcement (per `core-memory-path-guard-read-agent-file.md`'s three-layer pattern). Mirrors `is_core_memory_path()` in shape: structural rejection when the content matches an "existing artifact elsewhere" pattern. Filed post-implementation when the bucket-classification heuristics have settled. **When this guard lands, the reflection-pass surface retires** — see spec § C7.
- **Codification-prep ticket for the recursive-self-review carve-out** — separate issue, filed alongside this PR. Scope: assemble prior-instance evidence (mika#818 + mika#868 review trails), draft codification language for `review-guide.md`, sit until N=3 promotes it from prep to ship. Pre-filing at N=2 is per the structural-check-replaces-human-discipline principle: codification *prep* is cheap and forward-loadable; codification *retroactively* makes the codification itself the rate-limit on closing the recurrence.
- **Operator CLI `mika core-memory set --agent X --section Y --content "..."`** — surfaced as a gap during today's audit. Separate enhancement ticket.
- **mika#862, #863, #864** — engine-side guards for required-tools-gate evasion + verdict-line ghosting. Filed today; orthogonal to mika#868's surface but methodologically aligned.

## Context & Research

### Relevant Code and Patterns

**Three-layer injection precedent** (research finding B — `get_task_health_summary` blueprint):
- DB layer: `crates/mika-agent/src/db.rs:264-285` (TaskHealthAnomaly + TaskHealthSummary types), `crates/mika-agent/src/db.rs:4319` (`get_task_health_summary`), `crates/mika-agent/src/async_db.rs:388-390` (async wrapper), `crates/mika-agent/src/task_engine/types.rs:34-49` (`health_thresholds` constants).
- Engine layer: `crates/mika-agent/src/agent.rs:2692-2704` (silent-trigger gating block — currently includes Heartbeat/Callback/Reminder, **excludes Reflection and SkillRun**), threading via `SilentPromptContext` at lines 2706-2721.
- Prompt layer: `crates/mika-agent/src/prompt.rs:710-711` (`task_health` field), `crates/mika-agent/src/prompt.rs:815-873` (XML block emission + 8-point `<task-health-instructions>`), `crates/mika-agent/src/prompt.rs:441-445` (no-internal-tags-in-responses list).
- Test pair: `crates/mika-agent/src/prompt.rs:1786-1909` (positive: `test_silent_prompt_includes_task_health`), `crates/mika-agent/src/prompt.rs:1911-1933` (negative: omitted-when-none).

**SilentTrigger entry points** (research finding C):
- Enum: `crates/mika-agent/src/agent.rs:2459-2486`. Variants: Heartbeat, Reflection, Callback, SkillRun, Reminder.
- Per-trigger context assembly: `crates/mika-agent/src/agent.rs:2627-2690`. Reflection arm at lines 2647-2672 — emits HOUSEKEEPING / PROMOTION / INSIGHT subsections + 5-edit cap.
- Pre-trigger digest assembly (Reflection-only): `crates/mika-agent/src/agent.rs:2572-2625`. The reflection-pass scan would extend this gathering to also pull `db.get_all_core_memory(agent_id)`.
- Dispatch sites: `crates/mika-agent/src/task_engine/dispatcher.rs:667` (Heartbeat), `crates/mika-agent/src/task_engine/dispatcher.rs:812` (Reflection, with five pre-filter gates).

**`docs/architecture/` convention** (research finding E):
- Three existing files: `kg-id-convention.md`, `kg-implementation-conventions.md`, `review-guide.md`.
- Pattern: plain `# Title` H1 + `**Status:** Active` + `**Created:** YYYY-MM-DD` metadata lines (no YAML frontmatter, unlike `docs/solutions/best-practices/`).
- Reflection-pass spec should match `kg-implementation-conventions.md` shape (cross-cutting prescriptive policy, multi-section with numbered subsections C1/C2/C3).

**Direct prior art** — `docs/plans/2026-03-03-feat-periodic-memory-reflection-plan.md` (status: completed): introduces `SilentTrigger::Reflection`, `ReflectionConfig` in identity.toml (`enabled`, `time`, `notify`), `reflection_runs` SQLite table. The mika#868 reflection-pass spec **extends** this by adding a memory-promotion-candidates sub-step to the existing reflection daily run; it does NOT introduce a new SilentTrigger variant.

### Institutional Learnings

**MUST-cite (load-bearing for grooming):**

1. `docs/solutions/best-practices/core-memory-as-citation-not-accumulator-2026-04-28.md` — the policy this spec enforces. Three buckets verbatim. Lines 129-133 prescribe the surface mechanism: surface candidates by bucket, don't auto-promote, Bucket-3 staleness triggers re-evaluation only.
2. `docs/solutions/architecture-patterns/engine-guards-vs-prompt-rules-for-agent-behavior-2026-04-19.md` — frames why prompt-level rules are not the right shape for this. Decisive in cutting sub-task 1 from the original draft.
3. `docs/solutions/best-practices/required-tools-gate-evasion-patterns-2026-04-28.md` Rule 3 — prompt-level catalogues don't bind even when actively read. Co-written yesterday; cited as decisive evidence by the external reviewer when cutting sub-task 1.
4. `docs/solutions/best-practices/recursive-self-review-carve-out-2026-04-26.md` — routes this ticket's review to Claude Chat. Audit trail callout naming the reviewer.
5. `docs/memory-classification.md` — Layer 1/2/3 framework. The new `<core-memory-promotion-candidates>` injection extends the "Deterministic Operations" table.
6. `docs/plans/2026-03-03-feat-periodic-memory-reflection-plan.md` — prior `SilentTrigger::Reflection` design; the new spec is a refinement.

**Strongly relevant:**

7. `docs/solutions/architecture-patterns/task-health-awareness-heartbeat-injection.md` — the canonical injection blueprint to mirror (silent-trigger gating + anomaly cap + helper extraction + named threshold constants).
8. `docs/solutions/architecture-patterns/callback-turn-work-item-context-injection.md` — gating-rationale precedent. Names the per-`SilentTrigger`-variant inclusion/exclusion discipline. Spec MUST justify each variant decision (prevention rule #1).
9. `docs/solutions/architecture-patterns/pre-tool-context-redundancy-check.md` — composability with `is_active_skill_prompt()` and `search_memory` core_memory guards. Candidates block must NOT recommend `read_agent_file core_memory/...` or `search_memory category=core_memory` (both blocked at engine layer).
10. `docs/solutions/architecture-patterns/core-memory-path-guard-read-agent-file.md` — three-layer defense pattern. The structural-guard endpoint mika#868 is transitioning toward.
11. `docs/solutions/architecture-patterns/deterministic-skill-context-injection.md` — engine-owned fetch principle. Bucket classification must be engine-side, not LLM-side.
12. `docs/solutions/architecture/rewind-context-marker-confabulation-prevention.md` — `<rewind_reversals trust="internal">` pattern. New block uses `trust="internal"` wrapping.

**Test fixture references** (deferred to implementation ticket; named in spec C6): `docs/solutions/best-practices/eval-harness-test-defaults-and-di-pattern.md`, `docs/solutions/741-grounding-fabrication-regression-scenarios.md`.

### External References

External research skipped (Phase 1.2). Codebase has strong local patterns (well-established three-layer injection precedent; documented `SilentTrigger` machinery; direct prior art at `docs/plans/2026-03-03-feat-periodic-memory-reflection-plan.md`). The technology layer is fully internal — no third-party APIs, no novel frameworks.

## Key Technical Decisions

**D1. Reflection-pass extends existing `SilentTrigger::Reflection`, not a new variant.** Per `docs/plans/2026-03-03-feat-periodic-memory-reflection-plan.md`, Reflection already exists with daily-cadence semantics. New scan is a sub-pass of existing reflection. Reuses dispatcher gates, agent-lock, trigger-context assembly. Avoids creating a parallel scheduling path. *Trade-off:* the existing reflection arm at `agent.rs:2647-2672` becomes denser; mitigated by extracting the new scan into a helper.

**D2. Bucket classification is engine-side (deterministic), not LLM-side.** Per `deterministic-skill-context-injection.md` ("If the LLM doesn't control the fetch, it can't skip it") + `silent-callback-max-steps-exhaustion.md` (Reflection's default 10 max steps). Engine-side classification fits within scan latency budgets; LLM-driven classification would burn agent steps and reintroduce prompt-level drift the spec is trying to bound. *Trade-off:* deterministic heuristics are coarser than LLM judgment for ambiguous cases (Bucket 2 vs Bucket 3 boundary). Mitigation: surface as a candidate, not a verdict; agent reviews and decides.

**D3. Surface mechanism is `<core-memory-promotion-candidates trust="internal">` XML block.** Mirrors `<task-health>` (research finding B) + `<rewind_reversals trust="internal">` (learning #12). Includes a `<core-memory-promotion-instructions>` sub-block listing 4-6 numbered behaviors agent should perform on the surfaced candidates (mirror task-health-instructions shape). The new tag must be added to `prompt.rs:441-445` no-internal-tags-in-responses list when implementation lands. **The block includes the policy citation (`core-memory-as-citation-not-accumulator-2026-04-28.md`) in its surfaced text** — the agent learns the protocol from the runtime surface, not from a static prompt section.

**D4. Trigger gating: Reflection only.** Per gating-rationale prevention rule from learning #8 (justify each variant):
- **Reflection:** YES. Daily cadence matches "review accreted state" semantics. Reflection's existing 5-edit cap composes naturally with surface-don't-promote discipline. Promotion candidates are *consolidation context*, which is exactly what Reflection's prompt budget is shaped for.
- **Heartbeat:** NO. Heartbeat is high-frequency (sub-hourly); accretion is daily/weekly drift. Surfacing on every heartbeat would be noisy and burn token budget across many no-op turns.
- **Callback:** NO. Callbacks are mid-task continuations; injecting promotion candidates breaks task focus.
- **SkillRun:** NO. Skill-specific contexts; promotion candidates are off-topic.
- **Reminder:** NO. Reminders are user-facing; injecting internal-state candidates conflicts with the user-channel framing.

The prior task-health and callback-context gating arcs went heartbeat-only → expanded to callback. mika#868 takes the inverse direction (Reflection only). External-review confirmation: the prior arcs were expanding *task-context* injection; promotion candidates are *consolidation context*, which lives on a different axis. The inverse-arc isn't an inverse; it's the other axis.

**D5. The reflection-pass surface is *additive* during Reflection, not displacing of existing reflection content.** Reflection already injects task-health context + reflection-specific PROMOTION/INSIGHT/HOUSEKEEPING subsections. The new `<core-memory-promotion-candidates>` block sits alongside them, doesn't replace them. Spec C2 must be explicit about this so an implementer doesn't read the gating section and assume Reflection's prompt becomes "promotion-candidates only."

**D6. The reflection-pass surface retires when the structural write-time guard lands.** Per `core-memory-path-guard-read-agent-file.md`'s three-layer defense pattern: when the eventual write-time guard at `update_core_memory` (mirroring `is_core_memory_path()` shape) ships, the reflection-pass surface becomes dead context — the structural layer binds behavior, the reflection layer is redundant. Spec C7 (migration path) names the retirement explicitly so the future PR landing the write-time guard has the deletion path pre-specified. Otherwise the runtime ends up with both surfaces firing.

## Open Questions

### Resolved During Planning

- **Q: Surface mechanism shape?** A: `<core-memory-promotion-candidates trust="internal">` XML block with embedded `<core-memory-promotion-instructions>` (D3), policy citation embedded in the surfaced text.
- **Q: Trigger gating?** A: Reflection only (D4).
- **Q: Engine-side or LLM-side bucket classification?** A: Engine-side (D2).
- **Q: New SilentTrigger variant or extend Reflection?** A: Extend existing Reflection (D1).
- **Q: Should the spec include a parallel prompt-level layer (`## Memory promotion protocol` in soul constants)?** A: NO. Cut by external review — see § "Alternatives Rejected".
- **Q: Should the spec doc include implementation skeleton (Rust code) or stay design-only?** A: Design-only. Implementation in sibling ticket. Spec uses references to file:line precedents from research, not code.
- **Q: Should the spec doc include skeleton test fixtures?** A: NO. Spec describes the *pattern* (EvalHarness + MockLlmProvider, frozen fixtures, hard assertions). Artifacts live in the implementation ticket. Front-loading helper extraction without an existing fixture is YAGNI.

### Deferred to Implementation

- **Bucket-classification heuristic.** Exact rules for matching content text against existing artifacts (citation-string detection, ticket-reference parsing, recurrence-watch annotation parsing). Spec describes the algorithm shape; implementation tunes the heuristics against real core-memory blocks.
- **Candidate-surfacing threshold.** Should the block render when zero candidates exist (negative test path), or be omitted entirely? Mirrors `test_silent_prompt_omits_task_health_when_none` pattern. Implementation decides based on prompt-budget impact.
- **Cost budget specifics.** Latency, query count, max-step impact. Spec's cost section names the constraints; implementation measures and tunes.
- **Optional `core_memory_promotion_log` table.** Analogous to existing `reflection_runs` table — track what was surfaced for audit / observability. Deferred to implementation; spec notes the option but doesn't commit to schema.
- **Per-agent `MemoryPromotionConfig` in identity.toml.** Disable the scan per-agent (mirroring `ReflectionConfig`)? Defer — implementation may add or skip based on observed need.

## Implementation Units

- [ ] **Unit 1: Reflection-pass design spec doc**

**Goal:** Write the design spec for the reflection-pass enforcement — the implementation that surfaces promotion candidates by bucket assignment during `SilentTrigger::Reflection` turns.

**Requirements:** R1.

**Dependencies:** None (doc-only; design references existing code).

**Files:**
- Create: `docs/architecture/core-memory-promotion-protocol.md`

**Approach:**

The spec follows `kg-implementation-conventions.md` shape (research finding E):
- Header with `**Status:** Draft (planning — implementation in sibling ticket)` + `**Created:** 2026-04-28` + `**Companion docs:**` line linking compound doc and this plan.
- Numbered sections (C1, C2, C3...) for cross-cutting policies.

Spec content sections:
1. **C1. Purpose & boundary.** Cites the policy compound doc as the source of buckets. Names what reflection-pass enforces (surface) vs. doesn't enforce (auto-action). Explicitly states: *the reflection-pass surface includes the policy citation in its surfaced text — agents learn the protocol from the runtime surface, not from a static prompt section*.
2. **C2. Trigger gating.** D4 verbatim — Reflection only, with per-variant exclusion rationale. **Explicitly state the surface is additive on Reflection (D5)**, not displacing of existing reflection content (task-health context + PROMOTION/INSIGHT/HOUSEKEEPING subsections from `agent.rs:2647-2672` continue to fire alongside).
3. **C3. Three-layer architecture.** Mirrors `get_task_health_summary` pattern (research finding B). Subsections:
   - C3.1 DB layer — new `get_core_memory_promotion_candidates(agent_id) -> CoreMemoryPromotionCandidates` function. Type analogous to `TaskHealthSummary`. Cap on candidates (analogous to `MAX_ANOMALIES = 10`).
   - C3.2 Engine layer — silent-trigger gating block at `agent.rs:2692-2704` extends to include Reflection for the new field. Threading via `SilentPromptContext`.
   - C3.3 Prompt layer — new XML block emission at `prompt.rs` analogue of lines 815-873. New tag `<core-memory-promotion-candidates trust="internal">` added to no-internal-tags list at `prompt.rs:441-445`.
4. **C4. Bucket classification heuristics (engine-side).** D2 verbatim. Names the classifier shape — pattern-matching content text against existing-artifact citations + recurrence-watch annotations + N≥2 ticket-reference detection. Specific heuristics deferred to implementation.
5. **C5. Surface format.** XML block shape with embedded `<core-memory-promotion-instructions>` (mirrors `<task-health-instructions>` 8-point shape). Trust-tagged per learning #12. Surfaced text includes the policy compound-doc citation so the agent reads the protocol at runtime, not from a static prompt section.
6. **C6. Test fixture pattern.** EvalHarness + MockLlmProvider per learnings #15, #16. New scenario directory: `crates/mika-agent/tests/eval/reflection_promotion_candidates/`. Hard assertions only (no LLM-judge), frozen fixtures. Pattern only — no fixtures shipped in this PR.
7. **C7. Cost & latency budget + retirement criterion.** Constraints per learning #18 (max-steps exhaustion). Engine-side classification fits within reflection's existing 10-step default. DB query count target: ≤ 3 per scan. Latency target: < 100ms per scan (analogous to `get_task_health_summary` profile). **Explicit retirement criterion (D6): when the structural write-time guard at `update_core_memory` lands, the reflection-pass surface is removed.** Future PR landing the write-time guard has the deletion path pre-specified by this section. *If reflection-cadence proves too slow to catch accretion before the next cap-hit, the variant set (D4) is the lever to revisit, not the surface format.*
8. **C8. Migration path.** This spec is bridge scaffolding to a future structural write-time guard at `update_core_memory`. Names the eventual layer-1 enforcement (mirroring `is_core_memory_path()` shape from `core-memory-path-guard-read-agent-file.md`).
9. **C9. Composability with existing guards.** Per learning #9 — the candidates block does NOT recommend `read_agent_file core_memory/...` or `search_memory category=core_memory` (both engine-blocked). Spec names approved suggestions: `update_core_memory action=replace` to drop, `store_fact` or file-an-issue to promote.
10. **C10. Implementation deferred.** Sibling ticket TBD. Spec is reviewable independently; implementation realizes the design.

**Patterns to follow:**
- `docs/architecture/kg-implementation-conventions.md` (header shape, numbered sections, prescriptive policy doc)
- `docs/architecture/review-guide.md` (citation-or-silence rigor — every spec claim cites code or doc precedent)

**Test scenarios:**
- *Test expectation: none — pure design doc, no executable behavior to test in this PR.* The test scenarios this spec defines are realized in the sibling implementation ticket. Verification for this unit is doc-presence + cross-reference resolution.

**Verification:**
- File exists at `docs/architecture/core-memory-promotion-protocol.md`.
- Cross-references resolve: cited file paths + doc paths (compound doc, three-layer precedents at `prompt.rs`, `agent.rs`, `db.rs` line numbers) all exist on this branch.
- Header metadata includes `**Status:** Draft (planning — implementation in sibling ticket)`.
- All ten sections (C1–C10) present and address their stated content per the Approach above.

## Alternatives Rejected

**A1. Parallel prompt-level layer in `MIKA_DEV_SOUL` and `MIKA_ARCH_SOUL` (the original draft's sub-task 1).** Cut by external review. Reasoning:

The original draft proposed adding a `## Memory promotion protocol` section (~15-20 lines per agent) to MIKA_DEV_SOUL and MIKA_ARCH_SOUL constants — a soft prompt-level nudge during `update_core_memory` writes, framed as "bridge scaffolding" until the reflection-pass and write-time guard ship.

External-review verdict: shipping a prompt-level rule while citing the doc that says prompt-level rules don't work is the irony Rule 3 of `required-tools-gate-evasion-patterns-2026-04-28.md` explicitly inoculates against. Rule 3's exact words: *"the response is to file a structural-enforcement ticket, not to add a stronger version of the same prompt-level catalogue."* The proposed soul edit is a stronger version of the same prompt-level catalogue.

The "cheap and documented" defense fails on its own terms: cheap-and-documented prompt rules are exactly what accreted into `self_model` in the first place. Five of mika-dev's seven accreted rules were "cheap and documented" when written. PR #866 extracted them. Adding a new one — even one named "the protocol for not accreting rules" — is the recurrence pattern.

The "bridge scaffolding" framing also fails: bridge scaffolding is justified when it connects two things the system needs to traverse. The reflection-pass spec does NOT need a soul.md prompt rule to function — it operates engine-side, surfacing candidates as a `<core-memory-promotion-candidates>` block. The agent reads the surfaced candidates and acts on them; it doesn't need a separate prompt section telling it the rules. The soul edit isn't bridging anything — it's parallel scaffolding that duplicates what the surfaced block already conveys.

The cleaner shape: the policy lives where the enforcement lives. The reflection-pass surface includes the policy citation in the surfaced text. The agent learns the protocol from the runtime surface. Operators who want documentation read `docs/architecture/core-memory-promotion-protocol.md` directly. Different audiences, different artifacts.

Secondary benefit of the cut: the soul edit would have forced a Post-deploy Step 0 reprovisioning step on every host (per the #866 pattern). Skipping it removes the deploy ceremony entirely.

**A2. New `SilentTrigger` variant for promotion-pass.** Rejected — the existing `SilentTrigger::Reflection` is the correct hook. Per `docs/plans/2026-03-03-feat-periodic-memory-reflection-plan.md`, Reflection already has daily-cadence semantics with five pre-filter gates. Creating a parallel scheduling path duplicates infrastructure for no gain.

**A3. LLM-side bucket classification.** Rejected — would burn agent steps (Reflection has default 10 max steps per `silent-callback-max-steps-exhaustion.md`) and reintroduce the prompt-level drift the spec is trying to bound. Per `deterministic-skill-context-injection.md`: if the LLM doesn't control the fetch, it can't skip it.

**A4. Auto-promotion (engine drops or promotes core-memory entries without agent confirmation).** Rejected by the policy compound doc itself, lines 130-131: "Don't auto-promote. Surface candidate + bucket + suggested action; let the agent (or operator) confirm."

**A5. Skeleton test fixtures shipped in this PR.** Rejected — adding fixture skeletons before the first actual fixture exists is a YAGNI violation. The `kg_fixtures/mod.rs` shared-helper analogy doesn't translate; that pattern emerged from existing fixtures being extracted to shared helpers. The spec C6 describes the *pattern* (EvalHarness + MockLlmProvider, frozen fixtures, hard assertions); implementation ticket builds the artifacts.

## System-Wide Impact

- **Interaction graph:** No code changes in this PR. Spec describes future engine integration (extending `agent.rs:2692-2704` silent-trigger gating + new `prompt.rs` XML block emission + new `db.rs` detector function); implementation lands in sibling ticket.
- **Error propagation:** N/A this PR. Spec section C7 names failure modes (DB query failure, classification heuristic miss); implementation handles them.
- **State lifecycle risks:** None for this PR (doc-only).
- **API surface parity:** No external API changes. The spec's eventual XML block has internal-only consumers (the agent's own LLM context).
- **Integration coverage:** N/A this PR. Spec section C6 specifies the integration test pattern for the implementation ticket.
- **Unchanged invariants:** No changes to soul constants; no changes to provisioning logic; no changes to `update_core_memory`'s tool semantics; no changes to `SilentTrigger` enum shape; no changes to existing `get_task_health_summary` pattern. The new spec doc is a new file at `docs/architecture/core-memory-promotion-protocol.md`.

## Risks & Dependencies

| Risk | Mitigation |
|------|------------|
| **Spec/implementation misalignment** — sibling ticket implementer might diverge from the spec. | Spec is reviewed externally before merge (instance #2 of carve-out); implementation ticket cites this spec as the contract. Match-ticket-to-spec discipline. |
| **Reflection cadence proves too slow** — daily surfacing might let accretion compound across multiple operator sessions before being caught. | Per spec C7: the variant set (D4) is the lever to revisit, not the surface format. Adding Heartbeat to the gated set is a knob change, not a redesign. Documented in the spec itself so future-implementer has the right lever. |
| **Candidates block conflicts with conversation history under load** — agent might confabulate when the surfaced candidates contradict content the agent itself just wrote. | Trust-tagged wrapping (`<core-memory-promotion-candidates trust="internal">`) per `rewind-context-marker-confabulation-prevention.md`. The wrapper signals to the agent that this content is internally generated, not from conversation. Implementation includes the same instruction-text shape as the rewind marker. |
| **Reflection-pass surface becomes dead context after structural guard lands** — runtime ends up with both surfaces firing. | D6 + spec C7 explicit retirement criterion: when the write-time guard at `update_core_memory` lands, the reflection-pass surface is removed. The future PR landing the write-time guard has the deletion path pre-specified. |
| **Carve-out instance count climbs without codification keeping up** — N=2 now; codification triggers at N=3. | Codification-prep ticket filed alongside this PR (separate issue, scoped: assemble prior-instance evidence, draft codification language, sit until N=3). Per the structural-check-replaces-human-discipline principle, prep at N=2 means the codification itself is not the rate-limit when N=3 hits. |
| **Carve-out divergence between instances** — mika#818 and mika#868 might have used the carve-out for different shapes of work, making codification harder than convergence assumes. | The codification-prep ticket includes a 60-minute review of mika#818's external-review trail to compare verdict shape. If converging, prep is small. If diverging, prep is larger and the codification language has to adjudicate. Knowing this at N=2 changes the prep-ticket framing. |

## Documentation / Operational Notes

- PR body must link: this ticket (mika#868), mika#866 (the policy ship), mika#867 (the bug-fix follow-up), the policy compound doc, and the codification-prep ticket filed alongside this PR.
- **Codification-prep ticket** (residual action item): file as a separate issue post-merge of this PR. Scope: "Assemble prior-instance evidence (mika#818 + mika#868 review trails), perform a 60-minute comparison read to assess convergence/divergence of carve-out usage, draft codification language for `docs/architecture/review-guide.md`. Sit until N=3 promotes prep to ship per recursive-self-review-carve-out-2026-04-26.md § 'When to revisit'."
- **Sibling implementation ticket** (residual action item): file post-merge of this PR. Implementation realizes the spec's three-layer design (db.rs detector, agent.rs trigger gate, prompt.rs XML block, paired tests). Scope determined by the spec; no scope-of-implementation decisions made here.
- No reprovisioning step required for this PR (no soul.md changes). The deploy ceremony from #866 does not apply.

## Sources & References

- **Origin / parent:** senara-solutions/mika#866 (merged) — landed the policy compound doc this enforces
- **Sibling:** senara-solutions/mika#867 (merged) — /ce:review fixes on #866
- **Direct prior art:** `docs/plans/2026-03-03-feat-periodic-memory-reflection-plan.md` — completed reflection-pass design that this spec extends
- **Policy compound doc:** `docs/solutions/best-practices/core-memory-as-citation-not-accumulator-2026-04-28.md`
- **Three-layer injection blueprint:** `docs/solutions/architecture-patterns/task-health-awareness-heartbeat-injection.md`
- **Carve-out routing:** `docs/solutions/best-practices/recursive-self-review-carve-out-2026-04-26.md` (instance #2)
- **Decisive in cutting sub-task 1:** `docs/solutions/architecture-patterns/engine-guards-vs-prompt-rules-for-agent-behavior-2026-04-19.md` + `docs/solutions/best-practices/required-tools-gate-evasion-patterns-2026-04-28.md` Rule 3
- **N=3+ → CI gate principle (applied at N=2 to codification-prep):** `docs/solutions/best-practices/structural-check-replaces-human-discipline-2026-04-27.md`
- **Memory framework:** `docs/memory-classification.md`
- **Engine integration precedent (deferred to implementation):** `crates/mika-agent/src/db.rs:264-285` + `:4319`, `crates/mika-agent/src/agent.rs:2459-2486` + `:2692-2704` + `:2627-2690`, `crates/mika-agent/src/prompt.rs:710-711` + `:815-873` + `:441-445`
- **Sibling reference plans:** `docs/plans/2026-04-28-001-refactor-dev-pilot-derive-scripts-companion-plan.md`, `docs/plans/2026-04-28-002-chore-extract-mika-arch-foundational-refs-plan.md`
- **Engine-guard tickets (orthogonal but methodologically aligned):** senara-solutions/mika#862, #863, #864
- **Operator-tool gap (deferred):** `mika core-memory set --agent X --section Y --content "..."` — separate enhancement ticket
