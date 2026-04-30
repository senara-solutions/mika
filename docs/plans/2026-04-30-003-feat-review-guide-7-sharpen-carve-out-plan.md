---
title: "feat(review-guide): sharpen §7 recursive-self-review carve-out — causation vs outcome-shape, isolation axis"
type: feat
status: active
date: 2026-04-30
---

# feat(review-guide): sharpen §7 recursive-self-review carve-out

## Overview

`mika/docs/architecture/review-guide.md` §7 fires the recursive-self-review carve-out on outcome-shape grounds alone — *"plan modifies mika-arch's surface"*. Two empirical drift signals from milestone#19 grooming on 2026-04-30: (1) mika#874 second-pass ESCALATEd citing F9 *"decisions reshaped to benefit reviewer's own consumption surface"* even though the reshaping was driven by external peer review + operator decision, not by reviewer pressure; (2) every milestone#19 sub-issue's second-pass now defaults to external routing when the fix touches mika-arch-adjacent code, bypassing the second-pass quality gate (Sonnet 4.6 mika-arch-second-review skill) entirely. The fix is two doc-level sharpenings: (a) §7 distinguishes causation-grounded carve-out triggers from outcome-shape triggers via an iteration-history test; (b) §7 specifies which isolation axis (identity / prompt / memory) the carve-out tests, naming memory-shared as the load-bearing axis with prompt+model split alone as insufficient. No code changes. Pure documentation.

## Problem Frame

Empirical evidence (from issue body + parallel-session handoff):

- **mika#874 second-pass ESCALATE:** Plan iterated F4 (outcome enum value) from "out-of-scope" to "schema bump." The flip improved mika-arch's own SQL-queryable observability surface. But: the flip was driven by external peer-review brief + operator decision routing through Signal C in `mika/CLAUDE.md` ("Post-restart safety check #757"), NOT by reviewer pressure. mika-arch's first-pass had asked for *explicitness* (commit OR mark out-of-scope-with-rationale), not for the flip itself. Second-pass nonetheless ESCALATEd citing *"when a plan's iteration history shows decisions reshaped to benefit the reviewer's own consumption surface, that IS the vested-interest signature, regardless of whether the reasoning is sound."* That trigger is invariant to who *drove* the reshaping.

- **Cumulative cost concern:** under the current strict reading, milestone#19's remaining sub-issues (#875 closed, #876, #877) each fire §7 carve-out on second-pass, routing through operator (Vincent) for external review. **5 external second-passes per milestone is the operational cost** of the unsharpened rule. mika#876 second-pass today produced GROOMED in-pipeline only because the F1 finding addressed Phase 0 verification (truncation rule-out) rather than substrate-modifying surface — but the carve-out boundary was uncomfortably close to firing.

- **Isolation-axis ambiguity (issue body):** mika-arch has TWO skill prompts (`mika-arch-groom-ticket` Opus 4.7 first-pass + `mika-arch-second-review` Sonnet 4.6 second-pass), ONE agent identity (mika-arch), and ONE shared memory store (`store_fact` rows from first-pass are readable by second-pass via `search_memory`). Empirically (per mika#874 grooming): second-pass cited F8/F9 findings extracted from `store_fact` rows the first-pass wrote. The two passes look structurally separated (different prompts, different models) but are memory-coupled. Today's §7 doesn't name which axis triggers the carve-out — strict reading says "agent-identity-shared," which fires for any second-pass on infrastructure-adjacent work.

## Requirements Trace

- **R1.** §7 (and the related compound doc) explicitly distinguish causation-grounded carve-out triggers from outcome-shape triggers (per ticket AC).
- **R2.** §7 (or sibling doc) explicitly specifies which isolation axis (identity / prompt / memory) the carve-out tests (per ticket AC).
- **R3.** A re-run of the mika#874 second-pass scenario under the sharpened rule reaches a deterministic verdict (either "F9 applies" or "F9 does not apply") with reasoning that doesn't reduce to "depends on how you read it" (per ticket AC).
- **R4.** mika#875/#876/#877 grooming dispatched after the sharpening lands does NOT route every second-pass external by default purely on infrastructure-adjacency grounds (per ticket AC).
- **R5.** Existing carve-out behavior on TRUE vested-interest cases (reviewer's first-pass critique drove the decision toward reviewer's benefit) is preserved — the sharpening narrows the trigger condition, not the consequences when triggered.
- **R6.** Companion compound doc `mika/docs/solutions/best-practices/recursive-self-review-carve-out-2026-04-26.md` updated in lockstep — `review-guide.md §7` and the compound doc cannot drift on the trigger definition.
- **R7.** No code changes. Doc-only. Memory-key namespacing implementation (if memory-shared is selected as trigger axis) is explicitly out-of-scope per issue body.

## Scope Boundaries

- **In scope:**
  - `mika/docs/architecture/review-guide.md` §7 — sharpenings to *What to flag* / *What not to flag* sections (Unit 1).
  - `mika/docs/solutions/best-practices/recursive-self-review-carve-out-2026-04-26.md` — lockstep update (Unit 2).
  - The mika#874 worked-example added as a test case in the compound doc to demonstrate the sharpened rule (Unit 3).
- **Out of scope:**
  - **Implementing memory-key-namespacing** between mika-arch-groom-ticket and mika-arch-second-review skills (per issue body's "Out of scope" — separate ticket if memory-shared is selected as the trigger axis).
  - **Engine-side enforcement** of the carve-out (e.g., a structural guard that detects substrate-adjacent reviews and forces external routing). This is doc/prompt-level discipline; structural enforcement is a different ticket.
  - **Retroactively re-reviewing prior milestone#19 sub-issues under the new rule** (per issue body — those land under whatever rule is in force at decision time).
  - Updating mika-arch skill prompt files (`skills/bundled/mika-arch-*`) directly. Per Pin 3 (below), the skill prompts inherit §7 discipline by reference, not by inline copy. Updating §7 propagates to the skills automatically. Inlining skill-prompt copy of the carve-out text would create the docs-skills drift class that `Maintenance` line of review-guide.md explicitly warns against.

## Phase 0 Pins (load-bearing source verification)

### Pin 1: §7 current text — outcome-shape-only trigger

`mika/docs/architecture/review-guide.md:172-186` (verbatim, 2026-04-30):

> *"When a plan modifies the reviewing agent's own surface — the skills, identity config, or permission surface that the agent reads at runtime — the agent is both reviewer and structural stakeholder in the outcome."*
>
> **What to flag:** *"Second-pass review routed to the agent whose surface is being modified. When a plan changes `skills/bundled/mika-arch-*` skill manifests or prompts, mika-arch's `identity.toml` blocks (`[tools]`, `[skills]`, `[kg]`), or the mika-arch entry in `well_known_agents.rs`, the second-pass review MUST route to an external reviewer..."*
>
> **What not to flag:** *"Changes to code mika-arch uses but that don't modify her configuration."*

The trigger is **surface-shape**: does the plan touch mika-arch's skill / identity / permission config? If yes, fire. If no, don't. There is **no causation test** — the rule fires regardless of whether reviewer pressure drove the design choice.

### Pin 2: Companion compound doc — INTERNALLY INCONSISTENT on precedent enumeration

`mika/docs/solutions/best-practices/recursive-self-review-carve-out-2026-04-26.md` — 9.4 KB, last modified 2026-04-29. **Per first-pass F2 + gh_read verification 2026-04-30, the doc is internally inconsistent on which tickets are the three precedent instances:**

- **Line 81** (When to revisit § 2): *"Three instances formed the evidence base: mika#788 (first instance), mika#872 (second instance — promotion protocol prompts), mika#879 (third instance — milestone grooming; triggered the 3-instance codification threshold)."*
- **Lines 87-91** (Sources & References enumeration):
  - mika#818 — first instance exercising the carve-out (drop memory-write tools from MIKA_ARCH_DISABLED_TOOLS)
  - mika#788 — first instance of mika-arch bundled skill surface change
  - mika#872 — second instance (promotion protocol prompts)
  - mika#879 — third instance (milestone grooming)
  - mika#817 — counter-example
- **§ 7 Evidence base** at `review-guide.md:189-193`: cites mika#788/#872/#879 (matching compound doc line 81 but contradicting compound doc lines 87-91).

**gh_read verification 2026-04-30:**
- mika#788 [OPEN] — `fix(tools): run_gh allowlist — milestone/project are hallucinated subcommands` — this is a `run_gh` tool allowlist fix, **NOT a §7 carve-out instance**. Zero recursive-self-review framing in the ticket.
- mika#818 [CLOSED] — `fix(mika-arch): drop memory-write tools from MIKA_ARCH_DISABLED_TOOLS` — substrate change to mika-arch's tool denylist. ✓ matches carve-out.
- mika#868 [CLOSED] — `feat(mika-arch, mika-dev): promotion-protocol system-prompt additions + reflection-pass spec` — substrate change to mika-arch's skill prompts. ✓ matches carve-out. **mika#868's PR body explicitly enumerates: instance #1 = mika#818, instance #2 = mika#868 itself, instance #3 = mika#879.**
- mika#872 [MERGED] — `feat(arch): core-memory promotion-protocol reflection-pass spec` — substrate change. ✓ matches carve-out, but appears to be the merged-PR companion to #868's grooming ticket, not a distinct instance.
- mika#879 [presumed CLOSED] — milestone grooming additions to mika-arch bundled skills. ✓ matches carve-out.

**Authoritative enumeration per #868's PR body (most recent codification ratification):** mika#818 (instance #1), mika#868 (instance #2), mika#879 (instance #3). #788 is wrong; #872 is the implementation PR for #868's grooming, not a distinct instance.

**Implication:** Both `review-guide.md:189-193` AND `recursive-self-review-carve-out-2026-04-26.md:81` need correction. Unit 2's lockstep update must reconcile to the authoritative enumeration. R5 (preserves fire on TRUE vested interest) requires walking each correctly-identified instance under the sharpened rule (Unit 2's expansion below).

### Pin 3: Skill prompts inherit §7 by reference — VERIFIED via grep

Verified 2026-04-30 per first-pass F3:

- `skills/bundled/mika-arch-groom-ticket/system_prompt.md:8` — *"`docs/architecture/review-guide.md` — the architectural principles reference"*
- `skills/bundled/mika-arch-groom-ticket/system_prompt.md:23` — *"4. **Review against principles.** Evaluate the plan against the principles in `docs/architecture/review-guide.md`:"*
- `skills/bundled/mika-arch-second-review/system_prompt.md:7` — *"**Citation or silence.** Same rule as first pass — flag concerns only with citations to `docs/architecture/review-guide.md`, ADRs, or compound docs."*

`grep -rn "include_str.*review\|review-guide\|review_guide" /data/workspace/mika-platform/mika/crates/` returns only:
- `bundled_skills.rs:532, 545` — `include_str!` for `skill-review/system_prompt.md` (different skill — ✗ not relevant)
- `well_known_agents.rs` — textual references to review-guide.md path in identity bootstrap (textual, not include_str!)

**Conclusion: skill prompts cite review-guide.md by path, NOT by inlined include_str!.** The LLM consults the doc at runtime via tool calls (likely `read_agent_file` against the worktree-resolved path) when actually evaluating principles. Updating §7 propagates to skill behavior automatically — no rebuild required.

**Implication for scope:** Unit 1 + Unit 2 doc-level changes are sufficient. Out-of-scope to inline skill-prompt copy of the carve-out text (would create the docs-skills drift class that `Maintenance` line of review-guide.md explicitly warns against).

### Pin 4: Memory-shared evidence — mika#874 cross-pass coupling

Per issue body: *"F8/F9 findings the second-pass cited were extracted from `store_fact` rows the first-pass wrote (verified via `sqlite3 ~/.mika/data/mika.db \"SELECT input FROM tool_calls WHERE session_id='...' AND tool_name='store_fact'\"`)."*

This evidence is load-bearing for selecting **memory-shared as the carve-out trigger axis**. The two skills have:
- Different system prompts (`mika-arch-groom-ticket` vs `mika-arch-second-review`)
- Different models (Opus 4.7 vs Sonnet 4.6)
- Same agent identity
- **Same memory store**

The prompt + model split provides genuine *structural* diversity (different reasoning surfaces). But shared memory means the second-pass reads the first-pass's persisted findings, defeating the diversity. **The memory-shared coupling is what makes the second-pass non-independent**, not the identity overlap.

## Context & Research

### Relevant Docs and Patterns

- **`review-guide.md` § 6 (citation-or-silence):** the discipline that makes the carve-out load-bearing in the first place. If reviewers could cite anything to justify any framing, vested interest wouldn't matter. Because §6 holds reviewers to citation-or-silence, the framing axis (which citations get emphasized) is where vested interest expresses itself.
- **`mika/docs/solutions/best-practices/recursive-self-review-carve-out-2026-04-26.md`:** the codification doc. Three instances enumerated. The doc currently describes the rule by surface-shape; needs lockstep update.
- **mika#788, mika#872, mika#879** (cited in Pin 2): the three precedent instances. The sharpening should not reverse those classifications — the rule narrows the trigger but preserves the consequence.

### Institutional Learnings

- **N=8 conditional-disclosure-evasion pattern** (architect persisted memory). The verbatim-emit-findings drift in mika#901 is the same family — discipline-rules drift unless structurally enforced. §7's current rule drifted on the same axis: a doc-level discipline that applies regardless of context held when first written, but the iteration-history test that distinguishes genuine vs coincidental vested interest only emerged from empirical observation (mika#874 second-pass).
- **`engine-guards-vs-prompt-rules-for-agent-behavior-2026-04-19.md`:** prompt-level fix is the cheapest first layer. This plan stays at the prompt-level (doc-level rule that mika-arch consults). Structural escalation (engine-side carve-out enforcement) is a separate concern.
- **`feedback_implementation_scope_bundling.md`:** memory-key namespacing if memory-shared is the trigger axis is a separate ticket per issue body. This plan does NOT bundle implementation into the rule clarification.

## Key Technical Decisions

- **Causation test: provenance — which party first introduced the position the plan now ratifies?** (Per first-pass F1, the load-bearing operational discriminator.) The naive enumeration of triggers ("reviewer's first-pass critique" / "reviewer-emitted pressure") doesn't decide ambiguous cases — every iterating ticket arguably emits some critique by design. The discriminator is **provenance**: trace which party FIRST introduced the position the plan now ratifies. If reviewer's first-pass introduced the position and plan ratifies it, fire. If operator/external evidence introduced the position and reviewer subsequently endorsed (or did not block), don't fire. Citation count alone doesn't ground the verdict; provenance does.

  Decidable test for ambiguous cases: read the iteration-history trace top-to-bottom. The first tool/message/comment that asserts the position the final plan ratifies is the introducer. If that introducer is the reviewer (first-pass critique authoring the position OR reviewer's persisted memory cited by second-pass), fire. If the introducer is anyone else (operator, external peer review brief, canonical workflow doc citation), don't fire on outcome-shape grounds alone.

  **Memory-cite transport boundary (per second-pass F8):** when second-pass cites reviewer's persisted memory, trace through the persistence to the original introducer. Reviewer-persisted-but-operator-introduced positions are still operator-introduced — persistence is transport, not position-authoring. Memory citation surfaces the position to second-pass; it doesn't author the position. Apply provenance test to the underlying introducer, not the persister.

- **§6/§7 division explicit (per first-pass F4).** Reviewer pressure on non-substrate plans is handled by §6 (citation-or-silence); §7 specifically defends substrate-reshaping. Both gates (surface-shape AND provenance) must hold for §7 to fire — pre-empts confusion about whether non-substrate cases need §7 review.

- **Isolation axis: memory-shared (per first-pass F5 confirmation).** Memory-shared is the load-bearing axis empirically grounded by mika#874's `store_fact`-cross-pass evidence. §7's *What to flag* gains explicit naming: *"The carve-out fires when the first-pass and second-pass share memory state on the same ticket — i.e., when persisted `store_fact` rows from first-pass are readable by the second-pass via `search_memory` or context-injection. Prompt + model split alone does NOT defuse the carve-out as long as memory is shared. Memory-shared coupling is what makes the second-pass non-independent, regardless of prompt-and-model diversity. Identity-shared (any review by an agent on its own substrate, regardless of memory state) is over-conservative — it fires on cases where prompt-and-memory split eliminates the recursive coupling. Memory-shared captures the actual coupling vector."*

- **Future Work threshold (per first-pass F6).** If sharpened §7 fails to discriminate (false-positives recur N times OR mika#874-class deterministic-verdict cases recur with iteration-history ambiguity), escalate to engine-layer substrate-adjacency detector. Threshold: 3 documented recurrences post-deploy of this fix.

- **Surface-shape stays as a NECESSARY condition.** The sharpening narrows by adding the causation test; it does not remove the surface-shape gate. Both must hold for the carve-out to fire: (a) plan modifies reviewer's surface AND (b) iteration history shows reviewer-driven reshaping. If only (a) holds, no fire (current default flips). If only (b) holds (reviewer pressured a non-surface change), §6 (citation-or-silence) handles it without §7.

- **Three-state taxonomy:** (1) NEITHER condition holds → review normally; (2) surface-shape only → review normally (carve-out narrowed); (3) BOTH conditions hold → fire carve-out, route external. The current default (surface-shape alone fires) becomes case (2) under the sharpening, which is the bulk of the milestone#19 cost-reduction win.

- **Worked example: mika#874 re-classified.** Under the sharpened rule, mika#874 second-pass's F9 ESCALATE was incorrect — the F4 flip was driven by external peer review + operator decision, not by reviewer pressure. The deterministic verdict (per R3) is *"F9 does not apply; mika-arch second-pass is appropriate quality gate."* Unit 3 codifies this as the worked example in the compound doc.

- **Future-proofing for memory-key namespacing.** §7 explicitly notes that if/when mika introduces memory-key namespacing between the two skills (separate ticket per issue body's Out of Scope), the memory-shared trigger axis would be defused and the carve-out would only fire on agent-identity-shared (which is structural and unavoidable while there's one mika-arch identity). This makes the eventual memory-namespacing ticket's value proposition explicit: it shrinks the carve-out trigger surface from "memory-shared" to "identity-shared."

## Implementation Units

- [ ] **Unit 1: Sharpen `review-guide.md` §7 with causation test + isolation-axis specification**

  **Goal:** §7 distinguishes causation vs outcome-shape; specifies memory-shared as the load-bearing isolation axis.

  **Requirements:** R1, R2, R3, R5

  **Dependencies:** None.

  **Files:**
  - Modify: `mika/docs/architecture/review-guide.md` § 7 (lines 172-194 per Pin 1).

  **Approach:**
  - **Modify intro paragraph (line 172):** add the causation framing. Replace *"the agent is both reviewer and structural stakeholder in the outcome"* with *"the agent is both reviewer and structural stakeholder in the outcome. The carve-out fires when **both** conditions hold: (a) the plan modifies the reviewer's surface AND (b) the iteration history shows reviewer-driven reshaping. Outcome-shape alone does not trigger the carve-out — coincidental alignment between an externally-driven decision and reviewer benefit is not vested interest."*
  - **Add new "How to test" sub-section before "What to flag":** specifies the iteration-history trace as the deterministic test. Three positive triggers (reviewer's first-pass critique, reviewer-emitted pressure, reviewer's persisted memory cited by second-pass) and three negative triggers (operator judgment, external peer review, citation of canonical operator workflow documents independent of the reviewer).
  - **Modify "What to flag" first bullet (line 178):** add the causation gate. *"Second-pass review routed to the agent whose surface is being modified — provided the iteration-history test (above) shows reviewer-driven reshaping. When both conditions hold, the second-pass review MUST route to an external reviewer..."*
  - **Add new "What to flag" bullet:** *"Memory-shared coupling between first-pass and second-pass on the same ticket. mika-arch's two skills (`mika-arch-groom-ticket` Opus 4.7 + `mika-arch-second-review` Sonnet 4.6) share an agent-scoped memory store. Persisted `store_fact` rows from first-pass are readable by second-pass via `search_memory`. The prompt+model split alone does NOT defuse the carve-out as long as memory is shared — second-pass reading first-pass's persisted reasoning IS the recursive-self-review boundary."*
  - **Modify "What not to flag" — add the coincidental-alignment case:** *"Coincidental outcome-shape alignment with no reviewer-driven causation. When a plan's iteration history shows the reshaping originated in operator judgment, external peer review, or citation of canonical operator workflow documents independent of the reviewer, the carve-out does not fire on outcome-shape grounds alone. Example: mika#874 F4 flip was driven by external peer review + operator decision through Signal C in `mika/CLAUDE.md` (Post-restart safety check #757); reviewer's contribution was a procedural ask (be explicit), not the design choice. F9 does not apply."*
  - **Add forward-pointer to memory-namespacing future work:** *"Future work: if memory-key namespacing between `mika-arch-groom-ticket` and `mika-arch-second-review` skills is introduced (separate ticket), the memory-shared trigger axis would be defused, narrowing the carve-out to agent-identity-shared cases."*

  **Patterns to follow:**
  - Existing § 7 structure (intro paragraph + What to flag / What not to flag / Evidence base).
  - Existing "Evidence base" footer at line 187 (lists three instances mika#788/#872/#879). Append fourth instance: mika#874 (worked example demonstrating the sharpened rule).

  **Test expectation:** None — doc-only change. Verification is the worked-example deterministic verdict per R3.

  **Verification:**
  - `grep -n "iteration history\|memory-shared\|coincidental" mika/docs/architecture/review-guide.md` returns the new text in §7.
  - Reading the sharpened §7 against mika#874's iteration history yields a deterministic "F9 does not apply" verdict (R3).

- [ ] **Unit 2: Lockstep update of companion compound doc + correct the precedent enumeration**

  **Goal:** `recursive-self-review-carve-out-2026-04-26.md` matches §7's sharpened trigger definition AND the precedent enumeration is corrected to match the authoritative #868-PR-body shape. R6 (no drift between guide and compound doc) plus pre-existing-error correction (per first-pass F2).

  **Requirements:** R5, R6

  **Dependencies:** Unit 1 (defines the new shape).

  **Files:**
  - Modify: `mika/docs/solutions/best-practices/recursive-self-review-carve-out-2026-04-26.md`.
  - Modify: `mika/docs/architecture/review-guide.md:189-193` § 7 Evidence base (lockstep with compound doc).

  **Approach:**
  - **Correct the precedent enumeration (per Pin 2's authoritative shape):** the actual three precedent instances are mika#818 (instance 1), mika#868 (instance 2), mika#879 (instance 3). Update both `review-guide.md:189-193` Evidence base AND `recursive-self-review-carve-out-2026-04-26.md:81` "When to revisit" enumeration AND lines 87-91 "Sources" enumeration to consistently cite #818/#868/#879. Drop #788 (gh_read confirmed: `run_gh` allowlist fix, NOT a §7 instance). Preserve mika#872 as the implementation PR for #868's grooming (cross-reference, not distinct instance) and mika#817 as counter-example.
  - Add a new section "Causation vs outcome-shape" after the corrected "Three instances" enumeration. Mirrors §7's new "How to test" sub-section + provenance discriminator.
  - Add a new section "Isolation axis (identity / prompt / memory)" naming memory-shared as load-bearing. Mirrors §7's new bullet.
  - **Walk each precedent under the sharpened rule (per first-pass F2 R5 verification):**
    - **mika#818 (instance 1, dropped memory-write tools from MIKA_ARCH_DISABLED_TOOLS):** Surface-shape gate ✓ (modifies mika-arch's tool denylist). Provenance gate: needs verification of the iteration history. If reviewer pressure introduced the change, fire. If operator-driven, don't fire.
    - **mika#868 (instance 2, promotion-protocol system-prompt additions):** Surface-shape gate ✓ (modifies mika-arch's skill prompt). Provenance gate: needs verification.
    - **mika#879 (instance 3, milestone grooming):** Surface-shape gate ✓ (adds to mika-arch bundled skills). Provenance gate: needs verification.
    - For each: document the verdict under the sharpened rule. R5 commits to "preserves fire on TRUE vested interest" — if any precedent's verdict flips from "fire" (current rule) to "don't fire" (sharpened rule), that's a regression that needs explicit operator opt-in.
    - **The walks may surface that one or more precedents were misclassified under the surface-shape-only rule.** That's a feature, not a bug — the sharpened rule is more accurate.
  - Append fourth instance (mika#874) to the corrected "Three instances" enumeration as the case-study that motivated the sharpening. Reference Unit 3's worked example for the reasoning.
  - Cross-reference §7 explicitly so future readers can see both surfaces are in lockstep.

  **Patterns to follow:**
  - Existing structure of the compound doc (Three instances + Why it matters + When to revisit).
  - The compound-doc voice (slightly more narrative than review-guide.md; review-guide.md is rule-prescriptive, the compound doc is incident-narrative).

  **Test expectation:** None — doc-only.

  **Verification:**
  - `diff` between §7 trigger definition and compound doc's trigger definition shows no semantic gap.
  - `grep -c "mika#788\|mika#818\|mika#868\|mika#879" review-guide.md compound-doc.md` returns consistent counts: #788 should appear 0 times (corrected), #818/#868/#879 each appear at expected counts.
  - Each of the three precedent walks produces a deterministic verdict (no "depends on how you read it" language).

- [ ] **Unit 3: mika#874 worked example added as a deterministic-verdict test case**

  **Goal:** Demonstrate the sharpened rule produces a deterministic verdict on the case that motivated the sharpening. R3 (verifiability).

  **Requirements:** R3

  **Dependencies:** Units 1 + 2.

  **Files:**
  - Modify: `mika/docs/solutions/best-practices/recursive-self-review-carve-out-2026-04-26.md` (add worked example as a sub-section).

  **Approach:**
  - New sub-section: "Worked example: mika#874 (provenance test in action)."
  - Walk through the iteration history: first-pass ITERATE asked for explicitness; operator chose schema-bump per Signal C citation; external peer-review brief contributed; second-pass ESCALATEd citing F9.
  - Apply the sharpened test (per first-pass F1 + F7):
    - **Surface-shape gate ✓** — plan modifies mika-arch's KG resolver observability surface (schema bump v28→v29).
    - **Provenance gate ✗** — schema-bump position introduced by Signal C citation (`mika/CLAUDE.md` § Post-restart safety check #757) AND external peer-review brief, NOT by reviewer's first-pass. First-pass F4 explicitly marked Out-of-Scope-with-rationale. The flip ratifies external-introduced position, not reviewer-introduced position. Provenance gate fails.
    - Both gates must hold → §7 does NOT fire.
  - **Deterministic verdict:** F9 does not apply; mika-arch second-pass is appropriate quality gate; second-pass should route in-pipeline (mika-arch-second-review skill) under the sharpened rule.

  **Patterns to follow:**
  - The "Three instances" section's narrative style.

  **Test expectation:** None.

  **Verification:** Reading the worked example produces the same verdict as the section header claims (deterministic — no "depends on how you read it" language).

## System-Wide Impact

- **Interaction graph:** §7 is consumed by mika-arch's two skill prompts (by reference). Sharpening §7 propagates to both skills automatically — the next mika-arch run that consults `review-guide.md` reads the sharpened rule. No skill-prompt edits needed.
- **Error propagation:** N/A — doc-only.
- **State lifecycle risks:** None.
- **API surface parity:** N/A.
- **Burst invariant / unchanged invariants:** Existing carve-out behavior on TRUE vested-interest cases (reviewer drove the reshaping) is preserved (R5). The sharpening narrows trigger conditions; consequences when triggered are unchanged.
- **Operational observability:** The sharpened rule produces deterministic verdicts on iteration-history evidence. Operator can audit any second-pass classification by reading the iteration history rather than inferring intent.

## Risks & Dependencies

| Risk | Mitigation |
|------|------------|
| Sharpened rule allows TRUE vested interest to slip through (e.g., reviewer's pressure was subtle and not captured by the iteration-history trace). | The iteration-history test is conservative on the trigger side — if any reviewer-driven causation appears in the trace, fire. Subtle pressure that doesn't appear in the trace is the same risk under the current rule (the current rule fires on outcome-shape, which doesn't capture pressure either). The sharpening doesn't increase this risk; it just adds a NECESSARY condition that shrinks false positives. |
| Memory-shared trigger fires too readily after the sharpening because mika-arch's two skills always share memory. | This is by design under the sharpened rule. The right defuse is memory-key-namespacing (separate ticket per issue body). Without that, the carve-out fires on substrate-modifying changes plus iteration-history evidence — narrower than today's "any substrate-modifying change," but still firing on the genuine cases. |
| Compound doc and guide drift over time. | R6 makes the lockstep update part of this plan. Future drift is a maintenance concern; same risk as any docs-pair. |
| The worked example (mika#874) is not authoritative; future readers might dispute the iteration-history reading. | Cite the actual session trace (operator's external peer-review session, the explicit Signal C citation, mika-arch first-pass's "be explicit" framing). The dispute surface is empirical evidence, not interpretation. |
| Per-instance precedent walk produces a verdict that contradicts current rule (regression: current rule fires; sharpened rule doesn't). | **Regression-opt-in mechanism (per second-pass F9):** (a) flag the regression in the per-instance walk's verdict block within Unit 2's compound doc update; (b) add a "Regression note" line to §7's Evidence base section naming the regressed instance + reasoning; (c) require explicit operator confirmation via PR review before merge — operator must comment "opt-in to regression on instance #X per [reasoning]" on the PR. Without operator opt-in comment, PR is blocked. Mirrors `review-guide.md § "Deliberate trade-offs documented in ADRs"` — regressions are deliberate trade-offs requiring documented operator endorsement. |

## Acceptance

Per ticket AC:

- ✅ R1: §7 (Unit 1) and compound doc (Unit 2) explicitly distinguish causation-grounded triggers from outcome-shape triggers.
- ✅ R2: §7 (Unit 1) and compound doc (Unit 2) explicitly specify memory-shared as the load-bearing isolation axis (with prompt+model split alone insufficient).
- ✅ R3: mika#874 worked example (Unit 3) reaches a deterministic "F9 does not apply" verdict under the sharpened rule.
- ✅ R4: mika#876/#877 grooming dispatched after this lands does NOT route every second-pass external by default; only fires when iteration-history shows reviewer-driven causation.

**Verification path:**

1. **Doc review:** Vincent reads §7 + compound doc post-Unit-1+2; confirms the rule reads as a single test (surface-shape AND iteration-history) rather than two competing surface-shape and outcome-shape readings.
2. **Empirical verification:** the next mika-arch second-pass after this ships either (a) routes in-pipeline if iteration-history is operator/external-driven OR (b) ESCALATEs externally if iteration-history shows reviewer-driven reshaping. Per R4, the default for mika#876/#877 should be (a).
3. **CI gate:** `docs-sync` job (per `mika/CLAUDE.md` Conventions) ensures `mika/docs/` updates propagate to the agent's embedded docs at build time.

## Future Work

- **Memory-key-namespacing between `mika-arch-groom-ticket` and `mika-arch-second-review`** (separate ticket per issue body's Out of Scope). Defuses the memory-shared trigger axis, narrowing the carve-out to agent-identity-shared cases. Worth doing if substrate-modifying changes become frequent enough that iteration-history-evidence collection is itself friction.
- **Engine-side enforcement of the carve-out** (e.g., a structural guard that detects substrate-adjacent reviews via `kg_extractions.docs_root_hash` matching the reviewer agent's corpora). Doc-level rule today; structural guard as the next ratchet if rule-drift recurs (per `engine-guards-vs-prompt-rules` precedent).

## Sources & References

- Related issue: mika#904
- Sibling ticket: mika#901 (verbatim findings emit) — drives operator-side recovery from architect thin-emission; cumulative cost concern shared.
- Earlier instances feeding the codification: mika#788, mika#872, mika#879 (per `recursive-self-review-carve-out-2026-04-26.md` "Three instances").
- Empirical case driving this sharpening: mika#874 second-pass ESCALATE on F9 (2026-04-30).
- Doc references:
  - `mika/docs/architecture/review-guide.md:172-194` — § 7 (Pin 1)
  - `mika/docs/solutions/best-practices/recursive-self-review-carve-out-2026-04-26.md` — companion (Pin 2)
- Institutional learnings:
  - `mika/docs/solutions/architecture-patterns/engine-guards-vs-prompt-rules-for-agent-behavior-2026-04-19.md` (informs why this stays at doc-level vs structural enforcement)
  - `feedback_implementation_scope_bundling.md` (memory) — memory-namespacing implementation deferred to separate ticket
