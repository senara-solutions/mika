# Core Memory Promotion Protocol — Reflection-Pass Spec

**Status:** Draft (planning — implementation in sibling ticket)
**Created:** 2026-04-28
**Ticket:** senara-solutions/mika#868
**Companion docs:**
- [`docs/solutions/best-practices/core-memory-as-citation-not-accumulator-2026-04-28.md`](../solutions/best-practices/core-memory-as-citation-not-accumulator-2026-04-28.md) — the three-way filter policy this spec enforces
- [`docs/plans/2026-04-28-003-feat-promotion-protocol-prompts-and-reflection-spec-plan.md`](../plans/2026-04-28-003-feat-promotion-protocol-prompts-and-reflection-spec-plan.md) — the implementation plan

This document specifies the runtime enforcement of the core-memory accretion policy. A reflection-pass scan surfaces promotion candidates by bucket assignment during `SilentTrigger::Reflection` turns. The agent reads the surfaced candidates and acts on them. **The policy citation is included in the surfaced text — agents learn the protocol from the runtime surface, not from a static prompt section.**

Implementation is deferred to a sibling ticket. This doc is the design contract.

---

## C1. Purpose and Boundary

### What the reflection-pass enforces

The reflection-pass surfaces core-memory promotion candidates classified by the three-way filter defined in `core-memory-as-citation-not-accumulator-2026-04-28.md`:

- **Bucket 1 — Existing artifact:** The content has a durable artifact elsewhere (compound doc, ticket, system prompt). Surface with suggestion to drop in-line and replace with a one-line citation.
- **Bucket 2 — N≥2 recurrence:** The content documents a failure class that has recurred (N≥2 with distinct ticket references). Surface with suggestion to promote to a compound doc in `docs/solutions/`, then cite from core memory.
- **Bucket 3 — N=1 with recurrence-watch:** Single-incident content annotated `[recurrence-watch: N=1, <ticket-ref>]`. Surface only when a new occurrence has been detected (N has incremented), triggering re-evaluation to Bucket 2.

### What it does NOT enforce

- **Auto-promotion or auto-drop.** The policy compound doc (lines 130–132) explicitly prescribes: "Surface the candidate + bucket + suggested action; let the agent (or operator) confirm." The reflection-pass surfaces; the agent decides.
- **Write-time rejection.** The eventual structural guard at `update_core_memory` (see C8) is a separate, future layer. This spec covers the surface-and-suggest layer only.
- **LLM-driven classification.** Bucket assignment is engine-side and deterministic (see C4). The LLM reads the surfaced candidates, it does not classify them.

## C2. Trigger Gating

The `<core-memory-promotion-candidates>` block is injected during **`SilentTrigger::Reflection` only**. This block is **additive** — it sits alongside the existing Reflection context (HOUSEKEEPING / PROMOTION / INSIGHT subsections at `agent.rs:2647–2672`, task-health context, conversation/audit digests). It does not replace or displace any existing Reflection content.

Per the gating-rationale discipline from `callback-turn-work-item-context-injection.md` (prevention rule #1: justify each variant decision):

| Variant | Included | Rationale |
|---------|----------|-----------|
| **Reflection** | YES | Daily cadence matches "review accreted state" semantics. Reflection's existing 5-edit cap composes naturally with surface-don't-promote discipline. Promotion candidates are *consolidation context*, which is what Reflection's prompt budget is shaped for. |
| **Heartbeat** | NO | High-frequency (sub-hourly). Accretion is daily/weekly drift. Surfacing on every heartbeat burns token budget across many no-op turns. |
| **Callback** | NO | Mid-task continuations. Injecting promotion candidates breaks task focus. |
| **SkillRun** | NO | Skill-specific contexts. Promotion candidates are off-topic. |
| **Reminder** | NO | User-facing channel. Injecting internal-state candidates conflicts with user-channel framing. |

**Cadence lever:** If Reflection's daily cadence proves too slow to catch accretion before the next cap-hit, the variant set is the lever to revisit — not the surface format. Adding Heartbeat to the gated set is a knob change, not a redesign.

## C3. Three-Layer Architecture

Mirrors the `get_task_health_summary` pattern (`db.rs:264–285`, `agent.rs:2692–2704`, `prompt.rs:815–873`).

### C3.1 DB Layer

New function: `get_core_memory_promotion_candidates(agent_id: &str) -> Result<CoreMemoryPromotionCandidates>`

**Types** (analogous to `TaskHealthSummary` / `TaskHealthAnomaly`):

```rust
pub struct CoreMemoryPromotionCandidates {
    pub candidates: Vec<PromotionCandidate>,
}

pub struct PromotionCandidate {
    pub section: String,          // core memory block name (e.g., "self_model")
    pub content_excerpt: String,  // the accreted content (truncated for prompt budget)
    pub bucket: PromotionBucket,
    pub evidence: String,         // why this bucket (e.g., "matches docs/solutions/...")
    pub suggested_action: String, // what the agent should do
}

pub enum PromotionBucket {
    ExistingArtifact,    // Bucket 1
    RecurrencePromote,   // Bucket 2
    RecurrenceWatch,     // Bucket 3 (only surfaced when N has incremented)
}
```

**Candidate cap:** Analogous to `MAX_ANOMALIES = 10` in task-health, cap at `MAX_PROMOTION_CANDIDATES = 10`. Higher counts indicate a block overdue for manual audit — surface the top 10 by confidence/bucket priority (Bucket 1 first, then 2, then 3).

**Query shape:** Read all core memory blocks for the agent via existing `get_all_core_memory(agent_id)`. For each block, run the bucket-classification heuristics (C4) against each content segment. Return candidates only when the classifier has non-trivial confidence.

### C3.2 Engine Layer

Extend the silent-trigger gating block at `agent.rs:2692–2704`. Currently:

```rust
let (task_health, stored_preferences) = if matches!(
    &params.trigger,
    SilentTrigger::Heartbeat | SilentTrigger::Callback { .. } | SilentTrigger::Reminder { .. }
) { ... } else { (None, vec![]) };
```

Add a parallel fetch for Reflection only:

```rust
let promotion_candidates = if matches!(&params.trigger, SilentTrigger::Reflection) {
    db.get_core_memory_promotion_candidates(&agent_id).await.ok()
} else {
    None
};
```

Thread through `SilentPromptContext`:

```rust
pub struct SilentPromptContext<'a> {
    // ... existing fields ...
    pub promotion_candidates: Option<&'a CoreMemoryPromotionCandidates>,
}
```

### C3.3 Prompt Layer

New XML block emission in `build_silent_prompt()` (analogue of `prompt.rs:815–873`):

```xml
<core-memory-promotion-candidates trust="internal">
<candidates>
- [existing_artifact] self_model: "Fabrication risk rule..." → matches docs/solutions/741-grounding-fabrication-regression-scenarios.md. Suggested: drop in-line, replace with citation.
- [recurrence_promote] current_priorities: "Pre-commit discovery (N=4)..." → N≥2 with tickets #52, #636, #665, #663. Suggested: promote to compound doc, then cite.
</candidates>

<core-memory-promotion-instructions>
Review the promotion candidates above. For each candidate:
1. Read the candidate's bucket classification and evidence. The three-way filter is defined in docs/solutions/best-practices/core-memory-as-citation-not-accumulator-2026-04-28.md — use query_knowledge_graph (with include_context=true) to fetch the policy text at runtime; the agent has no direct file-read path to docs/solutions/. The lexical ingestor must have indexed the file (covered automatically when the agent's [kg].docs_roots includes the repo's docs/solutions/ — true for mika-arch by default; for mika-dev verify MIKA_KG_DOCS_ROOT or [kg].docs_roots covers the repo docs).
2. For Bucket 1 (existing_artifact): use update_core_memory action=replace to drop the in-line content and replace with a one-line citation to the existing artifact.
3. For Bucket 2 (recurrence_promote): if a compound doc does not already exist, file a work item via create_task (type=issue, label=promote-to-compound-doc). Once the doc exists, use update_core_memory to replace the in-line content with a citation. **send_message to operator is the secondary path when an interactive channel is attached; during autonomous reflection (no user channel), create_task is the primary surface — send_message returns NoChannel framing silently and the candidate would be lost.**
4. For Bucket 3 (recurrence_watch): check whether N has incremented since the annotation was written. If yes, re-classify as Bucket 2. If no, leave the item in place — it is correctly parked.
5. Do NOT use read_agent_file to read core_memory sections (engine-blocked). Do NOT use search_memory with category=core_memory (redirected). Core memory is already in your system prompt.
6. Surface your decisions via update_core_memory calls. Stay within the 5-edit cap for this reflection session.
</core-memory-promotion-instructions>
</core-memory-promotion-candidates>
```

**Trust tagging:** `trust="internal"` per the `<rewind_reversals trust="internal">` pattern from `rewind-context-marker-confabulation-prevention.md`. Signals to the agent that this content is system-generated, not from conversation.

**No-internal-tags update:** When implementation lands, add `<core-memory-promotion-candidates>` to the no-internal-tags-in-responses list at `prompt.rs:441–445`. The current list contains four tags (`<context>`, `<callback_result>`, `<task-health>`, `<rewind_reversals>`); the update preserves all four and appends the new tag:

```rust
"- **No internal tags in responses:** Never include internal XML tags like <context>, \
 <callback_result>, <task-health>, <rewind_reversals>, or \
 <core-memory-promotion-candidates> in your responses. \
 These are system metadata injected for your context — they are not for user display.\n",
```

**`INTERNAL_TAG_NAMES` registration:** Add `core-memory-promotion-candidates` to the `INTERNAL_TAG_NAMES` constant at `crates/mika-common/src/llm/mod.rs:23–31`. This drives the `strip_internal_tags()` regex pass that scrubs echoed internal tags from LLM response text — a separate stripping layer from the no-internal-tags prompt instruction above. Both layers must be updated at implementation time and both must be removed at retirement (see C7).

**Tag-naming convention:** This block uses kebab-case (`core-memory-promotion-candidates`), consistent with structural-block tags like `<task-health>`, `<task-health-instructions>`, and `<active-work-items>`. The atomic-marker tags `rewind_reversals` and `callback_result` use snake_case, but the `<rewind_reversals trust="internal">` precedent is cited only for the `trust=...` attribute pattern, not the case convention. Structural blocks that wrap candidate content + instructions follow the kebab-case shape of the `task-health` family.

**Omission rule:** When `promotion_candidates` is `None` or the candidates list is empty, the entire `<core-memory-promotion-candidates>` block is omitted from the prompt. Mirrors the task-health pattern at `prompt.rs:819` (`if has_items || has_anomalies`).

## C4. Bucket Classification Heuristics (Engine-Side)

Per `deterministic-skill-context-injection.md`: "If the LLM doesn't control the fetch, it can't skip it." Bucket classification runs in `get_core_memory_promotion_candidates()` as deterministic pattern-matching — no LLM calls.

**Classifier shape:**

For each content segment in a core memory block:

1. **Bucket 1 detection — citation-string matching.** Scan content for patterns indicating an existing durable artifact:
   - `docs/solutions/` path references (substring match against files on disk)
   - `docs/architecture/` path references
   - Ticket references matching `mika#NNN` or `#NNN` patterns where the ticket exists
   - `soul.md` / system prompt cross-references
   - Compound-doc filenames (`.md` paths matching known `docs/solutions/` entries)

2. **Bucket 2 detection — recurrence-count parsing.** Scan content for recurrence indicators:
   - Explicit `N=` annotations (e.g., `N=2`, `N=4`)
   - Multiple distinct ticket references (count unique `#NNN` patterns; ≥2 triggers Bucket 2)
   - `[recurrence-watch: N=1, ...]` annotations where a newer ticket reference is also present (N has incremented)

3. **Bucket 3 detection — recurrence-watch annotation parsing.** Scan content for `[recurrence-watch: N=1, ...]` annotations. Only surface when a new occurrence has been detected (heuristic: a newer ticket reference in the same block that wasn't in the original annotation).

**Heuristic tuning is deferred to implementation.** The implementation ticket will tune these patterns against real core-memory blocks from mika-dev and mika-arch. The spec commits to the algorithm shape (pattern-matching, not LLM-driven) and the bucket taxonomy (three buckets, engine-classified).

## C5. Surface Format

The XML block shape is specified in C3.3 above. Key design decisions:

- **Embedded policy citation.** The `<core-memory-promotion-instructions>` block references the compound doc by path (`docs/solutions/best-practices/core-memory-as-citation-not-accumulator-2026-04-28.md`). The agent learns the protocol from the runtime surface. No static prompt section duplicates this.
- **Per-candidate structure.** Each candidate line includes: bucket label, section name, content excerpt (truncated), evidence string, suggested action. This mirrors the `<anomalies>` shape in task-health (type + id + label + age).
- **Instruction count.** Six numbered instructions (mirroring the task-health-instructions 8-point shape, scoped to the promotion domain). Instruction 5 is the composability guard from C9.
- **Trust wrapping.** `trust="internal"` on the outer tag per `rewind-context-marker-confabulation-prevention.md`.

## C6. Test Fixture Pattern

**No fixtures ship in this PR.** Test scenarios are defined here for the implementation ticket.

**Harness:** `EvalHarness` + `MockLlmProvider` per `eval-harness-test-defaults-and-di-pattern.md`. Scenario directory: `crates/mika-agent/tests/eval/reflection_promotion_candidates/`.

**Scenario classes:**

1. **Positive — Bucket 1 candidate surfaced.** Seed core memory with content matching an existing `docs/solutions/` file. Assert `<core-memory-promotion-candidates>` block present in the system prompt with `existing_artifact` classification.

2. **Positive — Bucket 2 candidate surfaced.** Seed core memory with content containing N≥2 distinct ticket references. Assert `recurrence_promote` classification.

3. **Positive — Bucket 3 re-evaluation transitions to Bucket 2.** Seed core memory with `[recurrence-watch: N=1, #100]` annotation and a newer `#200` reference in the same block. Per C1 ("Surface only when a new occurrence has been detected … triggering re-evaluation to Bucket 2") and C4 Bucket 2 third bullet, the classifier MUST emit `recurrence_promote` for this seed — the watch annotation is the trigger condition, but the surfaced bucket is Bucket 2. Assert `recurrence_promote` classification.

   **Bucket 3 NOT-surfaced sub-case:** Seed `[recurrence-watch: N=1, #100]` with no newer ticket reference. Assert NO promotion candidate is emitted for this content (the watch is correctly parked).

4. **Negative — empty candidates, block omitted.** Seed core memory with only identity content (no accreted rules). Assert `<core-memory-promotion-candidates>` block is NOT present in the system prompt. Mirrors `test_silent_prompt_omits_task_health_when_none`.

5. **Negative — non-Reflection trigger, block omitted.** Run with `SilentTrigger::Heartbeat`. Assert block is NOT present regardless of core memory content. Heartbeat is a sufficient sample only if the implementation gates the fetch with a positive `matches!(trigger, SilentTrigger::Reflection)` check (whitelist). If the implementation uses an exclusion list, this scenario must be expanded to cover Callback, SkillRun, and Reminder as well.

6. **Boundary — `MAX_PROMOTION_CANDIDATES` cap and priority ordering.** Seed 11 distinct candidates spanning all three buckets (e.g., 4 Bucket 1, 5 Bucket 2, 2 Bucket 3-with-incremented-N). Assert exactly 10 candidate lines emitted in the surfaced block, with Bucket 1 entries first, then Bucket 2, then Bucket 3 — matching the priority sort defined in C3.1. Asserts both the cap value and the ordering rule.

7. **Boundary — Bucket 2 N=1 must not classify.** Seed core memory with content containing exactly one ticket reference (`#100`) and no `[recurrence-watch:]` annotation. Assert NO Bucket 2 candidate is emitted (the N≥2 threshold guards against false-positive promotion of single-incident content).

8. **Near-miss — false-positive prevention.** Seed near-miss content for each bucket and assert NO candidate is emitted:
   - **Bucket 1 near-miss:** content containing the substring `docs/solutions` inside a longer non-citation context (e.g., a sentence describing the directory naming convention rather than citing a specific file). Assert no Bucket 1 candidate.
   - **Bucket 2 near-miss:** content containing the same ticket reference repeated twice (`#100` … `#100`). Assert no Bucket 2 candidate (N=1 distinct, repeats don't count).
   - **Bucket 3 near-miss:** content containing the literal string `recurrence-watch` outside an annotation bracket. Assert no Bucket 3 candidate.

**Assertion style:** Hard assertions only (no LLM-judge). Frozen fixtures. Content assertions via substring match on the built system prompt string. **Co-occurrence requirement:** for positive scenarios, the bucket label string (e.g., `[recurrence_promote]`) must be asserted to appear on the SAME candidate line as the seeded section name — NOT anywhere in the prompt. The bucket label strings (`existing_artifact`, `recurrence_promote`, `recurrence_watch`) also appear in the static `<core-memory-promotion-instructions>` block per C3.3, so a substring-only assertion (`prompt.contains("recurrence_promote")`) passes regardless of classifier emission. Use line-level matching via the candidates block content (e.g., `extract_candidates_block(prompt).contains_line("[recurrence_promote] current_priorities:")`) or equivalent. **Frozen-fixture maintenance note:** when C4 heuristics are tuned in the implementation ticket or in subsequent maintenance, frozen-fixture seed content may need to be updated to remain a clean test case — distinguish deliberate fixture updates (heuristic tuning) from regressions by paired commits that update both the heuristic and its fixtures, and document the tuning rationale in the commit message per the KG eval `judge-deprecation-as-reset` pattern.

## C7. Cost, Latency Budget, and Retirement Criterion

### Cost and Latency

- **No LLM calls.** Engine-side classification (C4) is pure pattern-matching. Zero additional LLM cost per reflection.
- **DB query count:** Target ≤ 3 per scan. One `get_all_core_memory(agent_id)` call + up to 2 file-existence checks for Bucket 1 citation verification. Analogous to `get_task_health_summary` query profile.
- **Latency target:** < 100ms per scan. Core memory blocks are small (≤ 2500 tokens total across 5 blocks). Pattern-matching against them is sub-millisecond. File-existence checks are the latency floor.
- **Step budget:** The scan runs before the LLM turn, not during it. It consumes zero agent steps. The agent's responses to surfaced candidates use the existing 5-edit Reflection cap — no new step budget.
- **Max-steps interaction:** Per `conversation-mode-max-steps-too-low.md` (#386, 2026-04-03), `MAX_TOOL_STEPS` was unified to 20 across all silent-trigger modes — Reflection now runs with a 20-step budget. The promotion-candidates surface does not change this — it adds context to the system prompt, not additional tool calls. The agent decides how many of its 20 steps to spend on promotion actions vs. other reflection tasks. **Implementation note:** if a future ticket adds a new `SilentTrigger` variant, `SilentTrigger::max_steps()` MUST be matched exhaustively (no wildcard arm) — see `silent-callback-max-steps-exhaustion.md` and `reminder-trigger-max-steps-exhaustion.md` for the recurrence pattern this prevents.

### Retirement Criterion

**When the structural write-time guard at `update_core_memory` lands (future sibling ticket mirroring `is_core_memory_path()` shape from `core-memory-path-guard-read-agent-file.md`), the reflection-pass surface is removed.**

The write-time guard binds behavior structurally — it rejects accretion-shaped writes at the tool layer. Once that guard is active, the reflection-pass surface becomes dead context: it surfaces candidates that the write-time guard already prevents from accumulating. Keeping both surfaces running is redundant context that wastes Reflection's token budget.

The future PR landing the write-time guard MUST include deletion of:
1. The `get_core_memory_promotion_candidates()` DB function
2. The `promotion_candidates` field on `SilentPromptContext`
3. The `<core-memory-promotion-candidates>` XML emission block in `build_silent_prompt()`
4. The `<core-memory-promotion-candidates>` entry in the no-internal-tags list at `crates/mika-agent/src/prompt.rs:441–445`
5. The `core-memory-promotion-candidates` entry in `INTERNAL_TAG_NAMES` at `crates/mika-common/src/llm/mod.rs:23–31` (drives the response-text stripping regex; leaving it in place after the surface retires would silently scrub matching content from any future LLM output)
6. The associated test fixtures in `tests/eval/reflection_promotion_candidates/` and any `test_strip_core_memory_promotion_candidates_tag` unit test added to `mika-common/src/llm/mod.rs` at implementation time

This deletion path is pre-specified here so it is discoverable without archaeology.

## C8. Migration Path

This spec is bridge scaffolding toward a structural write-time guard at `update_core_memory`:

| Layer | Shape | Status |
|-------|-------|--------|
| **Layer 3 (surface)** | Reflection-pass scans core memory, surfaces promotion candidates by bucket | This spec (mika#868). Implementation in sibling ticket. |
| **Layer 2 (nudge)** | Prompt-level reminder in the surfaced block's instructions (C3.3 instruction text) | Ships with Layer 3 — embedded in the `<core-memory-promotion-instructions>` block. |
| **Layer 1 (guard)** | Structural write-time rejection at `update_core_memory` tool layer | Future ticket. Mirrors `is_core_memory_path()` guard shape. When this lands, Layer 3 retires (C7). |

The three-layer defense pattern from `core-memory-path-guard-read-agent-file.md` is the target architecture. This spec delivers Layers 2 and 3. Layer 1 is deferred until the bucket-classification heuristics (C4) have been tuned against real core-memory blocks — the heuristics inform what the write-time guard should reject.

## C9. Composability with Existing Guards

Per `pre-tool-context-redundancy-check.md`, the candidates block MUST NOT recommend actions that are blocked at the engine layer:

- **Do NOT suggest `read_agent_file core_memory/...`** — blocked by `is_core_memory_path()` guard (`tools/read_agent_file.rs`). Core memory is already in the system prompt.
- **Do NOT suggest `search_memory category=core_memory`** — hard-redirected by the context-redundancy guard (`tools/search_memory.rs`).

**Approved suggestions in the candidates block:**
- `update_core_memory action=replace` — to drop in-line content and replace with citation
- `store_fact` — to promote content to Layer 2 structured facts
- `create_task` (type=issue) — primary path for Bucket 2 items needing a new compound doc; works in autonomous reflection mode where there is no user channel
- `send_message` to operator — secondary path when an interactive channel is attached. **Important:** during `SilentTrigger::Reflection` runs there is typically no user channel; `GatewayMessageSender` returns `Ok(NoChannel)` silently and the candidate is lost. The implementation must ensure the surfaced suggestion routes to `create_task` when no channel is attached, and the `<core-memory-promotion-instructions>` block must reflect this (instruction 3 above). This pattern follows the `task-engine` autonomous-mode ticket-filing convention.

These approved suggestions are encoded in the `<core-memory-promotion-instructions>` block (C3.3, instructions 2–4) and in the `suggested_action` field of each `PromotionCandidate` (C3.1).

**Operator-equivalence gap (out of scope for this spec, surfaced for visibility):** No CLI command provides surgical core-memory replacement equivalent to the agent's `update_core_memory action=replace`. `mika memory reset <block>` is full-block-only. An operator who wants to apply a Bucket 1 promotion manually must either instruct the agent via `mika ask` or write directly to the SQLite store. The plan companion-docs lists `mika core-memory set --agent X --section Y --content "..."` as a separate enhancement ticket. The spec does not require this CLI to exist for the reflection-pass to function — the agent surface is sufficient — but reviewers and operators should know the surgical-replacement path is currently agent-only.

## C10. Implementation Deferred

Implementation is a sibling ticket filed post-merge of this PR. The implementation realizes:

1. **DB layer:** `get_core_memory_promotion_candidates()` with the `CoreMemoryPromotionCandidates` / `PromotionCandidate` / `PromotionBucket` types (C3.1).
2. **Engine layer:** Reflection-gated fetch in `run_silent_agent()`, threading through `SilentPromptContext` (C3.2).
3. **Prompt layer:** `<core-memory-promotion-candidates trust="internal">` XML block emission with `<core-memory-promotion-instructions>` (C3.3). No-internal-tags list update at `crates/mika-agent/src/prompt.rs:441–445` AND `INTERNAL_TAG_NAMES` registration at `crates/mika-common/src/llm/mod.rs:23–31` (with paired `test_strip_core_memory_promotion_candidates_tag` unit test).
4. **Heuristics:** Bucket-classification pattern-matching (C4), tuned against real core-memory blocks.
5. **Tests:** Eval harness scenarios per C6 (positive and negative cases, hard assertions, frozen fixtures).

The spec is reviewable independently of the runtime code. The implementation ticket cites this spec as the contract.

---

## Citations

- `docs/solutions/best-practices/core-memory-as-citation-not-accumulator-2026-04-28.md` — the three-way filter policy (lines 129–133 prescribe the surface mechanism)
- `docs/solutions/architecture-patterns/engine-guards-vs-prompt-rules-for-agent-behavior-2026-04-19.md` — frames why prompt-level rules are not the right shape
- `docs/solutions/best-practices/required-tools-gate-evasion-patterns-2026-04-28.md` — Rule 3: prompt-level catalogues don't bind
- `docs/solutions/architecture-patterns/task-health-awareness-heartbeat-injection.md` — the canonical injection blueprint mirrored by this spec
- `docs/solutions/architecture-patterns/callback-turn-work-item-context-injection.md` — gating-rationale discipline (justify each variant)
- `docs/solutions/architecture-patterns/pre-tool-context-redundancy-check.md` — composability with `is_active_skill_prompt()` and `search_memory` guards
- `docs/solutions/architecture-patterns/core-memory-path-guard-read-agent-file.md` — three-layer defense pattern; the structural-guard endpoint
- `docs/solutions/architecture-patterns/deterministic-skill-context-injection.md` — engine-owned fetch principle
- `docs/solutions/architecture/rewind-context-marker-confabulation-prevention.md` — `trust="internal"` wrapping pattern
- `docs/plans/2026-03-03-feat-periodic-memory-reflection-plan.md` — prior `SilentTrigger::Reflection` design that this spec extends
- `docs/memory-classification.md` — Layer 1/2/3 framework; the new injection extends the "Deterministic Operations" table
- `crates/mika-agent/src/db.rs:264–285` — `TaskHealthSummary` / `TaskHealthAnomaly` types (analogous to `CoreMemoryPromotionCandidates`)
- `crates/mika-agent/src/agent.rs:2459–2486` — `SilentTrigger` enum
- `crates/mika-agent/src/agent.rs:2647–2672` — existing Reflection trigger context (additive, not displaced)
- `crates/mika-agent/src/agent.rs:2692–2704` — silent-trigger gating block (the extension point)
- `crates/mika-agent/src/prompt.rs:441–445` — no-internal-tags-in-responses list (update required at implementation)
- `crates/mika-agent/src/prompt.rs:815–873` — task-health XML block emission (the template for the new block)
- senara-solutions/mika#866 — first application of the policy (foundational citation surface move)
- senara-solutions/mika#867 — review-applied fix on #866
- senara-solutions/mika#868 — this ticket
